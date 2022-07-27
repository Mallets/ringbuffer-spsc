# ringbuffer-spsc

A fast single-producer single-consumer ring buffer.
For performance reasons, the capacity of the buffer is determined
at compile time via a const generic and it is required to be a
power of two for a more efficient index handling.

# Example
```
use ringbuffer_spsc::RingBuffer;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn main() {
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
```