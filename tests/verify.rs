use ringbuffer_spsc::ringbuffer;

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

#[test]
fn memcheck() {
    const N: u32 = 4;
    let (mut tx, rx) = ringbuffer::<Box<usize>>(N);
    for i in 0..2_usize.pow(N) {
        assert!(tx.push(Box::new(i)).is_none());
    }
    drop(tx);
    drop(rx);
}
