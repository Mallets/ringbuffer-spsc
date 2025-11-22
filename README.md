[![CI](https://img.shields.io/github/actions/workflow/status/Mallets/ringbuffer-spsc/ci.yaml?branch=main)](https://github.com/Mallets/ringbuffer-spsc/actions?query=workflow:CI+branch:main)
[![docs.rs](https://img.shields.io/docsrs/ringbuffer-spsc)](https://docs.rs/ringbuffer-spsc/latest/ringbuffer_spsc/)
[![License](https://img.shields.io/badge/License-EPL%202.0-blue)](https://choosealicense.com/licenses/epl-2.0/)
[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)

# ringbuffer-spsc

A fast single-producer single-consumer ring buffer.
For performance reasons, the capacity of the buffer is determined
at compile time via a const generic and it is required to be a
power of two for a more efficient index handling.

# Example

Here below is reported the throughput example for convenience.

```Rust
use ringbuffer_spsc::RingBuffer;
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

fn main() {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let (mut tx, mut rx) = RingBuffer::<usize, 1_024>::init();

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
```

# Performance

Tests run on an Apple M4, 32 GB of RAM.

```sh
cargo run --release --example throughput
```

Provides `~520M elem/s` of sustained throughput.

```sh
531933452 elem/s
531134948 elem/s
528573235 elem/s
529276820 elem/s
523433161 elem/s
522780695 elem/s
527332139 elem/s
523041589 elem/s
520486274 elem/s
522570696 elem/s
...
```
