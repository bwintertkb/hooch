//! This module defines a basic executor for managing asynchronous tasks.
//! The executor utilizes a ready queue for receiving tasks, and a panic
//! channel to signal any panics that may occur while polling tasks.
//!
//! It includes the `Executor`, `ExecutorTask`, and `Status` types, which
//! handle task scheduling, task states, and task completion respectively.

use std::{
    sync::{mpsc::SyncSender, Arc},
    task::{Context, Waker},
};

use crate::spawner::Task;

/// Enum representing a task that the executor can handle.
/// It can either be a `Task` to execute, or a `Finished` signal to stop the executor.
pub enum ExecutorTask {
    /// Represents a task to be executed, wrapped in an `Arc` for shared ownership.
    Task(Arc<Task>),
    /// Represents a signal that indicates the executor should stop.
    Finished,
}

/// Enum representing the status of a task.
/// This is used to track whether a task has already completed or if it is still awaited.
#[derive(Debug)]
pub enum Status {
    /// The task is awaited by a specific `Waker`, which will be used to notify when it can proceed.
    Awaited(Waker),
    /// The task has already happened or completed.
    Happened,
}

/// A basic executor that runs tasks from a ready queue and catches any panics
/// that may occur while polling tasks. It also sends a message on `panic_tx`
/// if a panic is encountered.
///
/// The executor is designed to run tasks until it receives an `ExecutorTask::Finished`
/// signal.
pub struct Executor {
    /// A receiver for the ready queue that provides tasks to be executed.
    ready_queue: std::sync::mpsc::Receiver<ExecutorTask>,
    /// A sender for notifying about panics that occur while executing tasks.
    panic_tx: SyncSender<()>,
}

impl Executor {
    /// Creates a new `Executor` with the given ready queue receiver and panic notification sender.
    ///
    /// # Parameters
    /// - `rx`: The receiver end of a sync channel that provides tasks for the executor.
    /// - `panic_tx`: A sender to signal if a panic occurs while polling a task.
    ///
    /// # Returns
    /// A new instance of `Executor`.
    pub fn new(rx: std::sync::mpsc::Receiver<ExecutorTask>, panic_tx: SyncSender<()>) -> Self {
        Self {
            ready_queue: rx,
            panic_tx,
        }
    }

    /// Starts the executor loop, processing tasks from the ready queue.
    /// The loop continues until a `Finished` signal is received.
    ///
    /// Each task's future is polled, and if a panic occurs, it is caught
    /// and logged, with a notification sent via `panic_tx`.
    pub fn run(&self) {
        while let Ok(task) = self.ready_queue.recv() {
            let task = match task {
                ExecutorTask::Finished => return, // Stop the executor
                ExecutorTask::Task(task) => task, // Retrieve the task
            };

            let mut future = task.future.lock().unwrap();

            // Create a waker from the task to construct the context
            let waker = Arc::clone(&task).waker();
            let mut context = Context::from_waker(&waker);

            // Allow the future to make progress by polling it
            if let Err(e) = std::panic::catch_unwind(move || future.as_mut().poll(&mut context)) {
                println!("EXECUTOR PANIC FUNCTION. ERROR: {:?}", e);
                self.panic_tx.send(()).unwrap(); // Send panic signal
            }
        }
    }
}
