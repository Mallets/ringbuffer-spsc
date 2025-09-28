use ringbuffer_spsc::RingBuffer;
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

fn main() {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let (mut tx, mut rx) = RingBuffer::<usize>::new(1_024);

    std::thread::spawn(move || loop {
        if tx.push(1).is_some() {
            std::thread::yield_now();
        }
    });

    std::thread::spawn(move || loop {
        if rx.pull().is_some() {
            COUNTER.fetch_add(1, Ordering::Relaxed);
        } else {
            std::thread::yield_now();
        }
    });

    static STEP: Duration = Duration::from_secs(1);
    let start = Instant::now();
    for i in 1..=u32::MAX {
        std::thread::sleep(start + i * STEP - Instant::now());
        println!("{} elem/s", COUNTER.swap(0, Ordering::Relaxed));
    }
}
