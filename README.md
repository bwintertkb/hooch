# Hooch

## Overview

Hooch is a custom async runtime written in Rust. The runtime serves as a lightweight, experimental alternative to existing asynchronous frameworks, allowing for a deeper understanding of async runtime internals.

## Features

- **Custom Executor and Reactor**: Implements basic building blocks for async task scheduling and polling.
- **Minimal Dependencies**: Lightweight and tailored specifically to async task handling.

# Usage

Below are examples demonstrating basic use cases for Hooch's async runtime.

## Example 1: Spawning and Awaiting a Task with `JoinHandle`

In this example, we create a task within the runtime and use a `JoinHandle` to await its completion and retrieve the result. The `JoinHandle` allows you to asynchronously wait for the result of a spawned task, making it useful for concurrent operations.

```rust
use hooch::runtime::RuntimeBuilder;

fn main() {
    let runtime_handle = RuntimeBuilder::default().build();

    runtime_handle.run_blocking(async {
        // Creating a runtime instance for this thread
        RuntimeBuilder::default().build();
        let handle = hooch::spawner::Spawner::spawn(async { 3 });
        let value = handle.await.unwrap();

        assert_eq!(value, 3);
        println!("Received value from task: {}", value);
    });
}
```

## Example 2: Using Bounded Channels for Communication between Threads

This example demonstrates using a bounded channel to send data between threads. Here, we create a sender thread that transmits a value through a bounded channel, and a receiver in the async runtime awaits and receives it.

```rust
use std::sync::{Arc, Mutex};
use hooch::{runtime::RuntimeBuilder, sync::mpsc::bounded_channel};

fn main() {
    let (tx, rx) = bounded_channel::<i32>(1000);
    let runtime_handle = RuntimeBuilder::default().build();

    // Spawning a thread that sends a value through the channel
    std::thread::Builder::new()
        .name("sender".into())
        .spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            let _ = tx.send(1);
        })
        .unwrap();

    // Receiving the value in the async runtime
    let result = Arc::new(Mutex::new(0));
    let result_clone = Arc::clone(&result);
    runtime_handle.run_blocking(async move {
        let rx_res = rx.recv().await.unwrap();
        *result_clone.lock().unwrap() = rx_res;
    });

    assert_eq!(*result.lock().unwrap(), 1);
    println!("Received value from channel: {}", *result.lock().unwrap());
}
```

### Time
Sleep functionality is included with hooch.

```rust
use std::time::Duration;
use hooch::{runtime::RuntimeBuilder, time::sleep};

fn main() {
    let runtime_handle = RuntimeBuilder::default().build();
    runtime_handle.run_blocking(async move {
        sleep(Duration::from_millis(sleep_milliseconds)).await;
    });
}
```

## Hooch macros

`#[hooch_main]` is a `proc_macro_attribute` that conveniently wraps your `main` function in the runtime. The only argument is `workers`, which defines the number of workers used. If the argument is not present, the default number of workers from the `RuntimeBuilder` will be used.

### Examples

Using `RuntimeBuilder` default number of workers
```rust
use hooch::hooch_main;

#[hooch_main]
async fn main() {
    println!("Default number of workers in Runtime builder");
}
```

Using 4 workers
```rust
use hooch::hooch_main;

#[hooch_main(workers = 4)]
async fn main() {
    println!("Use 4 workers in the runtime");
}
```
