use ringbuffer_spsc::ringbuffer;
use std::sync::atomic::{AtomicUsize, Ordering};

// Elements arrive in order
#[test]
fn it_works() {
    const N: usize = 1_000_000;

    let (mut tx, mut rx) = ringbuffer::<usize>(16);

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
            if let Some(c) = rx.peek() {
                assert_eq!(*c, current);
                let c = rx.peek_mut().unwrap();
                assert_eq!(*c, current);
                let c = rx.pull().unwrap();
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

// Memory drop check
static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct DropCounter;

impl DropCounter {
    fn new() -> Self {
        COUNTER.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for DropCounter {
    fn drop(&mut self) {
        COUNTER.fetch_sub(1, Ordering::SeqCst);
    }
}

#[test]
fn memcheck() {
    const N: usize = 1_024;

    let (mut tx, rx) = ringbuffer::<DropCounter>(N);
    for _ in 0..N {
        assert!(tx.push(DropCounter::new()).is_none());
    }
    assert!(tx.push(DropCounter::new()).is_some());

    assert_eq!(
        COUNTER.load(Ordering::SeqCst),
        N,
        "There should be as many counters as ringbuffer capacity"
    );

    // Drop both reader and writer
    drop(tx);
    drop(rx);

    assert_eq!(
        COUNTER.load(Ordering::SeqCst),
        0,
        "All the drop counters should have been dropped"
    );
}
