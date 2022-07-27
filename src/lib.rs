//! A fast single-producer single-consumer ring buffer.
//! For performance reasons, the capacity of the buffer is determined
//! at compile time via a const generic and it is required to be a
//! power of two for a more efficient index handling.
//!
//! # Example
//! ```
//! use ringbuffer_spsc::RingBuffer;
//! use std::sync::atomic::{AtomicUsize, Ordering};
//! use std::sync::Arc;
//!
//! fn main() {
//!     const N: usize = 1_000_000;
//!     let (mut tx, mut rx) = RingBuffer::<usize, 16>::new();
//!
//!     let p = std::thread::spawn(move || {
//!         let mut current: usize = 0;
//!         while current < N {
//!             if tx.push(current).is_none() {
//!                 current = current.wrapping_add(1);
//!             } else {
//!                 std::thread::yield_now();
//!             }
//!         }
//!     });
//!
//!     let c = std::thread::spawn(move || {
//!         let mut current: usize = 0;
//!         while current < N {
//!             if let Some(c) = rx.pull() {
//!                 assert_eq!(c, current);
//!                 current = current.wrapping_add(1);
//!             } else {
//!                 std::thread::yield_now();
//!             }
//!         }
//!     });
//!
//!     p.join().unwrap();
//!     c.join().unwrap();
//! }
//! ```
use cache_padded::CachePadded;
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub struct RingBuffer<T, const N: usize> {
    buffer: UnsafeCell<[Option<T>; N]>,
    idx_r: CachePadded<AtomicUsize>,
    idx_w: CachePadded<AtomicUsize>,
}

unsafe impl<T, const N: usize> Send for RingBuffer<T, N> {}
unsafe impl<T, const N: usize> Sync for RingBuffer<T, N> {}

impl<T, const N: usize> RingBuffer<T, N> {
    pub fn new() -> (RingBufferWriter<T, N>, RingBufferReader<T, N>) {
        assert!(
            N.is_power_of_two(),
            "RingBuffer requires the capacity to be a power of 2. {} is not.",
            N
        );
        let rb = Arc::new(RingBuffer {
            buffer: UnsafeCell::new(array_init::array_init(|_| None)),
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

    #[inline]
    unsafe fn get_mut(&self, idx: usize) -> &mut Option<T> {
        // Since N is a power of two, N-1 is a mask covering N
        // elements overflowing when N elements have been added.
        // Indexes are left growing indefinetely and naturally wraps
        // around once the index increment reaches usize::MAX.
        &mut (*self.buffer.get())[idx & (N - 1)]
    }
}

pub struct RingBufferWriter<T, const N: usize> {
    inner: Arc<RingBuffer<T, N>>,
    cached_idx_r: usize,
    local_idx_w: usize,
}

impl<T, const N: usize> RingBufferWriter<T, N> {
    #[inline]
    pub fn push(&mut self, t: T) -> Option<T> {
        // Check if the ring buffer is potentially full.
        // This happens when the difference between the write and read indexes equals
        // the ring buffer capacity. Note that the write and read indexes are left growing
        // indefinitely, so we need to compute the difference by accounting for any eventual
        // overflow. This requires wrapping the subtraction operation.
        if self.local_idx_w.wrapping_sub(self.cached_idx_r) == N {
            self.cached_idx_r = self.inner.idx_r.load(Ordering::Acquire);
            // Check if the ring buffer is really full
            if self.local_idx_w.wrapping_sub(self.cached_idx_r) == N {
                return Some(t);
            }
        }

        // Insert the element in the ring buffer
        *unsafe { self.inner.get_mut(self.local_idx_w) } = Some(t);
        // Let's increment the counter and let it grow indefinitely
        // and potentially overflow resetting it to 0.
        self.local_idx_w = self.local_idx_w.wrapping_add(1);
        self.inner.idx_w.store(self.local_idx_w, Ordering::Release);

        None
    }
}

pub struct RingBufferReader<T, const N: usize> {
    inner: Arc<RingBuffer<T, N>>,
    local_idx_r: usize,
    cached_idx_w: usize,
}

impl<T, const N: usize> RingBufferReader<T, N> {
    #[inline]
    pub fn pull(&mut self) -> Option<T> {
        // Check if the ring buffer is potentially empty
        if self.local_idx_r == self.cached_idx_w {
            // Update the write index
            self.cached_idx_w = self.inner.idx_w.load(Ordering::Acquire);
            // Check if the ring buffer is really empty
            if self.local_idx_r == self.cached_idx_w {
                return None;
            }
        }
        // Remove the element from the ring buffer
        let t = unsafe { self.inner.get_mut(self.local_idx_r) }.take();
        // Let's increment the counter and let it grow indefinitely
        // and potentially overflow resetting it to 0.
        self.local_idx_r = self.local_idx_r.wrapping_add(1);
        self.inner.idx_r.store(self.local_idx_r, Ordering::Release);

        t
    }
}

#[cfg(test)]
mod tests {
    use super::RingBuffer;

    #[test]
    fn it_works() {
        const N: usize = 1_000_000;
        let (mut tx, mut rx) = RingBuffer::<usize, 16>::new();

        let p = std::thread::spawn(move || {
            let mut current: usize = 0;
            while current < N {
                if tx.push(current).is_none() {
                    current = current.wrapping_add(1);
                } else {
                    std::thread::yield_now();
                }
            }
        });

        let c = std::thread::spawn(move || {
            let mut current: usize = 0;
            while current < N {
                if let Some(c) = rx.pull() {
                    assert_eq!(c, current);
                    current = current.wrapping_add(1);
                } else {
                    std::thread::yield_now();
                }
            }
        });

        p.join().unwrap();
        c.join().unwrap();
    }
}