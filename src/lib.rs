//! A fast thread-safe `no_std` single-producer single-consumer ring buffer.
//! For performance reasons, the capacity of the buffer is determined
//! at compile time via a const generic and it is required to be a
//! power of two for a more efficient index handling.
//!
//! # Example
//! ```
//! use ringbuffer_spsc::RingBuffer;
//!
//! const N: usize = 1_000_000;
//! let (mut tx, mut rx) = RingBuffer::<usize, 16>::init();
//!
//! let p = std::thread::spawn(move || {
//!     let mut current: usize = 0;
//!     while current < N {
//!         if tx.push(current).is_none() {
//!             current = current.wrapping_add(1);
//!         } else {
//!             std::thread::yield_now();
//!         }
//!     }
//! });
//!
//! let c = std::thread::spawn(move || {
//!     let mut current: usize = 0;
//!     while current < N {
//!         if let Some(c) = rx.pull() {
//!             assert_eq!(c, current);
//!             current = current.wrapping_add(1);
//!         } else {
//!             std::thread::yield_now();
//!         }
//!     }
//! });
//!
//! p.join().unwrap();
//! c.join().unwrap();
//! ```
#![no_std]
extern crate alloc;

use alloc::sync::Arc;
use core::{
    cell::UnsafeCell,
    mem::{self, MaybeUninit},
    sync::atomic::{AtomicUsize, Ordering},
};
use crossbeam::utils::CachePadded;

pub struct RingBuffer<T, const N: usize> {
    buffer: UnsafeCell<[MaybeUninit<T>; N]>,
    idx_r: CachePadded<AtomicUsize>,
    idx_w: CachePadded<AtomicUsize>,
}

impl<T, const N: usize> RingBuffer<T, N> {
    #[allow(clippy::new_ret_no_self)]
    #[deprecated(since = "0.1.8", note = "please use `init()` instead.")]
    pub fn new() -> (RingBufferWriter<T, N>, RingBufferReader<T, N>) {
        Self::init()
    }

    /// Initialize the RingBuffer with the given capacity
    pub fn init() -> (RingBufferWriter<T, N>, RingBufferReader<T, N>) {
        assert!(
            N.is_power_of_two(),
            "RingBuffer requires the capacity to be a power of 2. {N} is not."
        );
        let rb = Arc::new(RingBuffer {
            buffer: UnsafeCell::new(array_init::array_init(|_| MaybeUninit::uninit())),
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

    #[allow(clippy::mut_from_ref)]
    #[inline]
    unsafe fn get_mut(&self, idx: usize) -> &mut MaybeUninit<T> {
        // Since N is a power of two, N-1 is a mask covering N
        // elements overflowing when N elements have been added.
        // Indexes are left growing indefinetely and naturally wraps
        // around once the index increment reaches usize::MAX.
        &mut (*self.buffer.get())[idx & (N - 1)]
    }
}

impl<T, const N: usize> Drop for RingBuffer<T, N> {
    fn drop(&mut self) {
        let mut idx_r = self.idx_r.load(Ordering::Acquire);
        let idx_w = self.idx_w.load(Ordering::Acquire);

        while idx_r != idx_w {
            let t =
                unsafe { mem::replace(self.get_mut(idx_r), MaybeUninit::uninit()).assume_init() };
            mem::drop(t);
            idx_r = idx_r.wrapping_add(1);
        }
    }
}

pub struct RingBufferWriter<T, const N: usize> {
    inner: Arc<RingBuffer<T, N>>,
    cached_idx_r: usize,
    local_idx_w: usize,
}

unsafe impl<T: Send, const N: usize> Send for RingBufferWriter<T, N> {}
unsafe impl<T: Sync, const N: usize> Sync for RingBufferWriter<T, N> {}

impl<T, const N: usize> RingBufferWriter<T, N> {
    /// Push an element into the RingBuffer.
    /// It returns `Some(T)` if the RingBuffer is full, giving back the ownership of `T`.
    #[inline]
    pub fn push(&mut self, t: T) -> Option<T> {
        // Check if the ring buffer is full.
        if self.is_full() {
            return Some(t);
        }

        // Insert the element in the ring buffer
        let _ = unsafe { mem::replace(self.inner.get_mut(self.local_idx_w), MaybeUninit::new(t)) };
        // Let's increment the counter and let it grow indefinitely and potentially overflow resetting it to 0.
        self.local_idx_w = self.local_idx_w.wrapping_add(1);
        self.inner.idx_w.store(self.local_idx_w, Ordering::Release);

        None
    }

    /// Check if the RingBuffer is full and evenutally updates the internal cached indexes.
    #[inline]
    pub fn is_full(&mut self) -> bool {
        // Check if the ring buffer is potentially full.
        // This happens when the difference between the write and read indexes equals
        // the ring buffer capacity. Note that the write and read indexes are left growing
        // indefinitely, so we need to compute the difference by accounting for any eventual
        // overflow. This requires wrapping the subtraction operation.
        if self.local_idx_w.wrapping_sub(self.cached_idx_r) == N {
            self.cached_idx_r = self.inner.idx_r.load(Ordering::Acquire);
            // Check if the ring buffer is really full
            self.local_idx_w.wrapping_sub(self.cached_idx_r) == N
        } else {
            false
        }
    }
}

pub struct RingBufferReader<T, const N: usize> {
    inner: Arc<RingBuffer<T, N>>,
    local_idx_r: usize,
    cached_idx_w: usize,
}

unsafe impl<T: Send, const N: usize> Send for RingBufferReader<T, N> {}
unsafe impl<T: Sync, const N: usize> Sync for RingBufferReader<T, N> {}

impl<T, const N: usize> RingBufferReader<T, N> {
    /// Pull an element from the RingBuffer.
    /// It returns `None` if the RingBuffer is empty.
    #[inline]
    pub fn pull(&mut self) -> Option<T> {
        // Check if the ring buffer is potentially empty
        if self.is_empty() {
            return None;
        }

        // Remove the element from the ring buffer
        let t = unsafe {
            mem::replace(self.inner.get_mut(self.local_idx_r), MaybeUninit::uninit()).assume_init()
        };
        // Let's increment the counter and let it grow indefinitely
        // and potentially overflow resetting it to 0.
        self.local_idx_r = self.local_idx_r.wrapping_add(1);
        self.inner.idx_r.store(self.local_idx_r, Ordering::Release);

        Some(t)
    }

    /// Check if the RingBuffer is empty and evenutally updates the internal cached indexes.
    #[inline]
    pub fn is_empty(&mut self) -> bool {
        // Check if the ring buffer is potentially empty
        if self.local_idx_r == self.cached_idx_w {
            // Update the write index
            self.cached_idx_w = self.inner.idx_w.load(Ordering::Acquire);
            // Check if the ring buffer is really empty
            self.local_idx_r == self.cached_idx_w
        } else {
            false
        }
    }
}
