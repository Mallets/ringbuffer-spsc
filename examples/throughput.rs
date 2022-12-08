use ringbuffer_spsc::RingBuffer;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn main() {
    let (mut tx, mut rx) = RingBuffer::<usize, 16>::init();
    let counter = Arc::new(AtomicUsize::new(0));

    std::thread::spawn(move || {
        let mut current: usize = 0;
        loop {
            if tx.push(current).is_none() {
                current = current.wrapping_add(1);
            } else {
                std::thread::yield_now();
            }
        }
    });

    let c_counter = counter.clone();
    std::thread::spawn(move || {
        let mut current: usize = 0;
        loop {
            if let Some(c) = rx.pull() {
                assert_eq!(c, current);
                current = current.wrapping_add(1);
                c_counter.fetch_add(1, Ordering::Relaxed);
            } else {
                std::thread::yield_now();
            }
        }
    });

    loop {
        std::thread::sleep(Duration::from_secs(1));
        println!("{} elem/s", counter.swap(0, Ordering::Relaxed));
    }
}
