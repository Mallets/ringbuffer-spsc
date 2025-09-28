//! A fast thread-safe `no_std` single-producer single-consumer ring buffer.
//! For performance reasons, the capacity of the buffer is determined
//! at compile time via a const generic and it is required to be a
//! power of two for a more efficient index handling.
//!
//! # Example
//! ```rust
//! use ringbuffer_spsc::RingBuffer;
//!
//! const N: usize = 1_000_000;
//! let (mut tx, mut rx) = RingBuffer::<usize>::new(16);
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

use alloc::{boxed::Box, sync::Arc, vec::Vec};
use core::{
    mem::{self, MaybeUninit},
    sync::atomic::{AtomicUsize, Ordering},
};
use crossbeam::utils::CachePadded;

pub struct RingBuffer<T> {
    ptr: *const MaybeUninit<T>,
    capacity: usize,
    mask: usize,
    idx_r: CachePadded<AtomicUsize>,
    idx_w: CachePadded<AtomicUsize>,
}

impl<T> RingBuffer<T> {
    pub fn new(capacity: usize) -> (RingBufferWriter<T>, RingBufferReader<T>) {
        assert!(
            capacity.is_power_of_two(),
            "RingBuffer requires the capacity to be a power of 2: {capacity} is not."
        );
        let v = (0..capacity)
            .map(|_| MaybeUninit::uninit())
            .collect::<Vec<_>>();
        let ptr = Box::leak(v.into_boxed_slice()).as_ptr();

        // Since capacity is a power of two, capacity-1 is a mask covering N elements overflowing when N elements have been added.
        // Indexes are left growing indefinetely and naturally wraps around once the index increment reaches usize::MAX.
        let mask = capacity - 1;

        let rb = Arc::new(RingBuffer {
            ptr,
            capacity,
            mask,
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
    fn get_mut(&self, idx: usize) -> &mut MaybeUninit<T> {
        unsafe { &mut (*(self.ptr.add(idx & self.mask) as *mut MaybeUninit<T>)) }
    }
}

impl<T> Drop for RingBuffer<T> {
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

pub struct RingBufferWriter<T> {
    inner: Arc<RingBuffer<T>>,
    cached_idx_r: usize,
    local_idx_w: usize,
}

unsafe impl<T: Send> Send for RingBufferWriter<T> {}
unsafe impl<T: Sync> Sync for RingBufferWriter<T> {}

impl<T> RingBufferWriter<T> {
    /// Push an element into the RingBuffer.
    /// It returns `Some(T)` if the RingBuffer is full, giving back the ownership of `T`.
    #[inline]
    pub fn push(&mut self, t: T) -> Option<T> {
        // Check if the ring buffer is full.
        if self.is_full() {
            return Some(t);
        }

        // Insert the element in the ring buffer
        let _ = mem::replace(self.inner.get_mut(self.local_idx_w), MaybeUninit::new(t));
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
        if self.local_idx_w.wrapping_sub(self.cached_idx_r) == self.inner.capacity {
            self.cached_idx_r = self.inner.idx_r.load(Ordering::Acquire);
            // Check if the ring buffer is really full
            self.local_idx_w.wrapping_sub(self.cached_idx_r) == self.inner.capacity
        } else {
            false
        }
    }
}

pub struct RingBufferReader<T> {
    inner: Arc<RingBuffer<T>>,
    local_idx_r: usize,
    cached_idx_w: usize,
}

unsafe impl<T: Send> Send for RingBufferReader<T> {}
unsafe impl<T: Sync> Sync for RingBufferReader<T> {}

impl<T> RingBufferReader<T> {
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

    /// Peek an element from the RingBuffer without pulling it out.
    /// It returns `None` if the RingBuffer is empty.
    #[inline]
    pub fn peek<'a>(&'a mut self) -> Option<&'a T> {
        // Check if the ring buffer is potentially empty
        if self.is_empty() {
            return None;
        }

        Some(unsafe { self.inner.get_mut(self.local_idx_r).assume_init_ref() })
    }

    /// Peek a mutable element from the RingBuffer without pulling it out.
    /// It returns `None` if the RingBuffer is empty.
    #[inline]
    pub fn peek_mut<'a>(&'a mut self) -> Option<&'a mut T> {
        // Check if the ring buffer is potentially empty
        if self.is_empty() {
            return None;
        }

        Some(unsafe { self.inner.get_mut(self.local_idx_r).assume_init_mut() })
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
