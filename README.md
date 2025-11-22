# ringbuffer-spsc

[![CI](https://img.shields.io/github/actions/workflow/status/Mallets/ringbuffer-spsc/ci.yaml?branch=main)](https://github.com/Mallets/ringbuffer-spsc/actions?query=workflow:CI+branch:main)
[![docs.rs](https://img.shields.io/docsrs/ringbuffer-spsc)](https://docs.rs/ringbuffer-spsc/latest/ringbuffer_spsc/)
[![License](https://img.shields.io/badge/License-EPL%202.0-blue)](https://choosealicense.com/licenses/epl-2.0/)
[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)

A fast `#[no_std]` single-producer single-consumer (SPSC) ring buffer designed for low-latency and high-throughput scenarios.

For performance reasons, the buffer's capacity must be a power of two. Using a power-of-two capacity lets the implementation wrap indices with a simple bitmask instead of using a slower modulo operation. This reduces computational overhead and improves throughput.

## Example

A minimal example showing a simple producer–consumer (cross-thread) pattern.

```rust
use ringbuffer_spsc::ringbuffer;

fn main() {
    // Create a writer/reader pair
    let (mut writer, mut reader) = ringbuffer::<usize>(16);

    // Thread that pushes elements on the ringbuffer
    std::thread::spawn(move || for i in 0..usize::MAX {
        // Attempt to push an element
        if writer.push(i).is_some() {
            // The ringbuffer is full, yield the thread
            std::thread::yield_now();
        }
    });

    // Loop that pulls elements from the ringbuffer
    loop {
        match reader.pull() {
            // We have got an element, do something
            Some(t) => std::hint::blackbox(t),
            // The ringbuffer is empty, yield the thread
            None => std::thread::yield_now(),
        }
    }
}
```

## Performance

The repository includes a [throughput example](https://github.com/Mallets/ringbuffer-spsc/blob/main/examples/throughput.rs).
To run it locally:

```sh
cargo run --release --example throughput
```

Provides `~520M elem/s` of sustained throughput when benchmarking the example on an Apple M4, 32 GB of RAM:

```sh
531933452 elem/s
531134948 elem/s
528573235 elem/s
529276820 elem/s
```
