//! A fast, small single-producer single-consumer (SPSC) ringbuffer designed
//! for low-latency and high-throughput exchange between a single writer and
//! a single reader. This crate is `#![no_std]` and uses `alloc` internally; it provides a
//! minimal, ergonomic API that works well from `no_std` contexts that supply
//! an allocator as well as from normal `std` programs and examples.
//!
//! Important design points:
//! - The ringbuffer capacity is specified at runtime via the [`ringbuffer`] constructor
//!   and **must be a power of two**. The implementation uses a bitmask to wrap
//!   indices which is much faster than a modulo operation and reduces runtime
//!   overhead in hot paths.
//! - The API is non-blocking: [`RingBufferWriter::push`] returns `Some(T)` immediately when the
//!   buffer is full (giving back ownership of the value), and [`RingBufferReader::pull`] returns
//!   `None` immediately when the buffer is empty. Typical usage is to yield or
//!   retry in those cases.
//!
//! *NOTE:* elements remaining in the buffer are dropped when the internal storage is deallocated.
//! This happens when both [`RingBufferReader`] and [`RingBufferWriter`] are dropped.
//!
//! ## Example
//! ```rust
//! use ringbuffer_spsc::ringbuffer;
//!
//! const N: usize = 8;
//!
//! // Create a ringbuffer with capacity 16 (must be power of two)
//! let (mut writer, mut reader) = ringbuffer::<usize>(16);
//! // Producer
//! std::thread::spawn(move || for i in 0..N {
//!     if writer.push(i).is_some() {
//!         std::thread::yield_now();
//!     }
//! });
//! // Consumer
//! let mut i = 0;
//! while i < N {
//!     match reader.pull() {
//!         Some(_) => i += 1,
//!         None => std::thread::yield_now(),
//!     }
//! }
//! ```
#![no_std]
extern crate alloc;

use alloc::{boxed::Box, sync::Arc, vec::Vec};
use core::{
    mem::{self, MaybeUninit},
    sync::atomic::{AtomicUsize, Ordering},
};
use crossbeam_utils::CachePadded;

/// Create a new ringbuffer with a fixed capacity.
///
/// # Panics
///
/// Panics if *capacity* is not a power of two.
///
/// This requirement enables a fast bitmask wrap for indices which avoids the cost of `mod`
/// in the hot path.
///
/// # Returns
///
/// A `(`[`RingBufferWriter<T>`]`, `[`RingBufferReader<T>`]`)` pair where the writer is
/// intended for the single producer and the reader for the single consumer.
pub fn ringbuffer<T>(capacity: usize) -> (RingBufferWriter<T>, RingBufferReader<T>) {
    assert!(capacity.is_power_of_two(), "Capacity must be a power of 2");

    // Inner container
    let v = (0..capacity)
        .map(|_| MaybeUninit::uninit())
        .collect::<Vec<_>>()
        .into_boxed_slice();

    let rb = Arc::new(RingBuffer {
        // Keep the pointer to the boxed slice
        ptr: Box::into_raw(v),
        // Since capacity is a power of two, capacity-1 is a mask covering N elements overflowing when N elements have been added.
        // Indexes are left growing indefinitely and naturally wrap around once the index increment reaches usize::MAX.
        mask: capacity - 1,
        idx_r: CachePadded::new(AtomicUsize::new(0)),
        idx_w: CachePadded::new(AtomicUsize::new(0)),
    });
    (
        RingBufferWriter {
            inner: rb.clone(),
            cached_idx_r: 0,
            local_idx_w: 0,
        },
        RingBufferReader {
            inner: rb,
            local_idx_r: 0,
            cached_idx_w: 0,
        },
    )
}

/// Internal ringbuffer storage. This type is private to the crate.
///
/// It stores the raw boxed slice pointer and the atomic indices used for
/// synchronization. The implementation uses monotonically increasing indices
/// (wrapping on overflow) and a power-of-two mask to convert indices to
/// positions inside the buffer.
struct RingBuffer<T> {
    ptr: *mut [MaybeUninit<T>],
    mask: usize,
    idx_r: CachePadded<AtomicUsize>,
    idx_w: CachePadded<AtomicUsize>,
}

impl<T> RingBuffer<T> {
    #[allow(clippy::mut_from_ref)]
    unsafe fn get_unchecked_mut(&self, idx: usize) -> &mut MaybeUninit<T> {
        // Safety: caller must ensure that `idx` is in a range that refers to
        // an initialized slot when reading, or to a slot that may be written
        // when writing. This helper performs unchecked indexing into the
        // backing slice using the internal mask.
        unsafe { (&mut (*self.ptr)).get_unchecked_mut(idx & self.mask) }
    }
}

// The internal `RingBuffer` is stored inside an `Arc` and will be deallocated
// when the last writer or reader handle is dropped (i.e., when the `Arc`
// reference count reaches zero).
impl<T> Drop for RingBuffer<T> {
    fn drop(&mut self) {
        let mut idx_r = self.idx_r.load(Ordering::Acquire);
        let idx_w = self.idx_w.load(Ordering::Acquire);

        while idx_r != idx_w {
            // SAFETY: we are in Drop and we must clean up any elements still
            // present in the buffer. Since only one producer and one
            // consumer exist, and we're dropping the entire buffer, it is
            // safe to assume we can take ownership of remaining initialized
            // elements between `idx_r` and `idx_w`.
            let t = unsafe {
                mem::replace(self.get_unchecked_mut(idx_r), MaybeUninit::uninit()).assume_init()
            };
            mem::drop(t);
            idx_r = idx_r.wrapping_add(1);
        }

        // At this point we've taken ownership of and dropped every
        // initialized element that was still present in the buffer. It is
        // important to drop all elements before freeing the backing storage
        // so that each element's destructor runs exactly once. Converting
        // the raw pointer back into a `Box` will free the allocation for
        // the slice itself.
        let ptr = unsafe { Box::from_raw(self.ptr) };
        mem::drop(ptr);
    }
}

/// Writer handle of the ringbuffer.
pub struct RingBufferWriter<T> {
    inner: Arc<RingBuffer<T>>,
    cached_idx_r: usize,
    local_idx_w: usize,
}

unsafe impl<T: Send> Send for RingBufferWriter<T> {}
unsafe impl<T: Sync> Sync for RingBufferWriter<T> {}

impl<T> RingBufferWriter<T> {
    /// Returns the capacity (number of slots) of the ringbuffer.
    pub fn capacity(&self) -> usize {
        self.inner.ptr.len()
    }

    /// Push an element into the RingBuffer.
    ///
    /// Returns `Some(T)` when the buffer is full (giving back ownership of the value), otherwise returns `None` on success.
    #[inline]
    pub fn push(&mut self, t: T) -> Option<T> {
        // Check if the ringbuffer is full.
        if self.is_full() {
            return Some(t);
        }

        // Insert the element in the ringbuffer
        let _ = mem::replace(
            unsafe { self.inner.get_unchecked_mut(self.local_idx_w) },
            MaybeUninit::new(t),
        );

        // Let's increment the counter and let it grow indefinitely and potentially overflow resetting it to 0.
        self.local_idx_w = self.local_idx_w.wrapping_add(1);
        self.inner.idx_w.store(self.local_idx_w, Ordering::Release);

        None
    }

    /// Check if the RingBuffer is full.
    #[inline]
    pub fn is_full(&mut self) -> bool {
        // Check if the ringbuffer is potentially full.
        // This happens when the difference between the write and read indexes equals
        // the ringbuffer capacity. Note that the write and read indexes are left growing
        // indefinitely, so we need to compute the difference by accounting for any eventual
        // overflow. This requires wrapping the subtraction operation.
        if self.local_idx_w.wrapping_sub(self.cached_idx_r) == self.inner.ptr.len() {
            self.cached_idx_r = self.inner.idx_r.load(Ordering::Acquire);
            // Check if the ringbuffer is really full
            self.local_idx_w.wrapping_sub(self.cached_idx_r) == self.inner.ptr.len()
        } else {
            false
        }
    }
}

/// Reader handle of the ringbuffer.
pub struct RingBufferReader<T> {
    inner: Arc<RingBuffer<T>>,
    local_idx_r: usize,
    cached_idx_w: usize,
}

unsafe impl<T: Send> Send for RingBufferReader<T> {}
unsafe impl<T: Sync> Sync for RingBufferReader<T> {}

impl<T> RingBufferReader<T> {
    /// Returns the capacity (number of slots) of the ringbuffer.
    pub fn capacity(&self) -> usize {
        self.inner.ptr.len()
    }

    /// Pull an element from the ringbuffer.
    ///
    /// Returns `Some(T)` if an element is available, otherwise `None` when the buffer is empty.
    #[inline]
    pub fn pull(&mut self) -> Option<T> {
        // Check if the ringbuffer is potentially empty
        if self.is_empty() {
            return None;
        }

        // Remove the element from the ringbuffer
        let t = unsafe {
            mem::replace(
                self.inner.get_unchecked_mut(self.local_idx_r),
                MaybeUninit::uninit(),
            )
            .assume_init()
        };
        // Let's increment the counter and let it grow indefinitely
        // and potentially overflow resetting it to 0.
        self.local_idx_r = self.local_idx_r.wrapping_add(1);
        self.inner.idx_r.store(self.local_idx_r, Ordering::Release);

        Some(t)
    }

    /// Peek an element from the ringbuffer without pulling it out.
    ///
    /// Returns `Some(&T)` when at lease one element is present, or `None` when the buffer is empty.
    #[inline]
    pub fn peek(&mut self) -> Option<&T> {
        // Check if the ringbuffer is potentially empty
        if self.is_empty() {
            return None;
        }

        Some(unsafe {
            self.inner
                .get_unchecked_mut(self.local_idx_r)
                .assume_init_ref()
        })
    }

    /// Peek a mutable element from the ringbuffer without pulling it out.
    ///
    /// Returns `Some(&mut T)` when at lease one element is present, or `None` when the buffer is empty.
    #[inline]
    pub fn peek_mut(&mut self) -> Option<&mut T> {
        // Check if the ringbuffer is potentially empty
        if self.is_empty() {
            return None;
        }

        Some(unsafe {
            self.inner
                .get_unchecked_mut(self.local_idx_r)
                .assume_init_mut()
        })
    }

    /// Check if the ringbuffer is empty.
    #[inline]
    pub fn is_empty(&mut self) -> bool {
        // Check if the ringbuffer is potentially empty
        if self.local_idx_r == self.cached_idx_w {
            // Update the write index
            self.cached_idx_w = self.inner.idx_w.load(Ordering::Acquire);
            // Check if the ringbuffer is really empty
            self.local_idx_r == self.cached_idx_w
        } else {
            false
        }
    }
}
