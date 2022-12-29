use ringbuffer_spsc::RingBuffer;

#[test]
fn it_works() {
    const N: usize = 1_000_000;
    let (mut tx, mut rx) = RingBuffer::<usize, 16>::init();

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
