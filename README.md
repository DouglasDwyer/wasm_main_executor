# wasm_main_executor
### Run futures on the main browser thread

[![Crates.io](https://img.shields.io/crates/v/wasm_main_executor.svg)](https://crates.io/crates/wasm_main_executor)
[![Docs.rs](https://docs.rs/wasm_main_executor/badge.svg)](https://docs.rs/wasm_main_executor)

Certain tasks, like creating an `AudioContext` or `RtcPeerConnection`, can only be performed on the main browser thread.
`wasm_main_executor` provides an easy way to send futures to the main browser thread from any context. This allows
web workers to spawn main-threaded tasks and await their completion. As such, the `wasm_main_executor` crate is primarily
intended for the implementation of polyfills/shims that allow cross-threaded functionality.

## Usage

The following is a simple example of executor usage:

```rust
async fn test_future() -> i32 {
    // Do some async or main-threaded work...
    2
}

// Start the executor. This must be called from the main browser thread.
wasm_main_executor::initialize().unwrap();

// Futures may be spawned on background threads using the executor.
// The future runs on the main thread.
let fut = wasm_main_executor::spawn(test_future());

// A future is returned which may be awaited on the background thread.
assert_eq!(2, futures::executor::block_on(fut));
```