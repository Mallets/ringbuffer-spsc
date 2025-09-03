use ringbuffer_spsc::RingBuffer;
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

fn main() {
    let (mut tx, mut rx) = RingBuffer::<usize, 1_024>::init();
    static COUNTER: AtomicUsize = AtomicUsize::new(0);

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

    std::thread::spawn(move || {
        let mut current: usize = 0;
        loop {
            if let Some(c) = rx.pull() {
                debug_assert_eq!(c, current);
                current = current.wrapping_add(1);
                COUNTER.fetch_add(1, Ordering::Relaxed);
            } else {
                std::thread::yield_now();
            }
        }
    });

    let start = Instant::now();
    let step = Duration::from_secs(1);
    for i in 1..=u32::MAX {
        std::thread::sleep(start + i * step - Instant::now());
        println!("{} elem/s", COUNTER.swap(0, Ordering::Relaxed));
    }
}
