//! This module defines a basic executor for managing asynchronous tasks.
//! The executor utilizes a ready queue for receiving tasks, and a panic
//! channel to signal any panics that may occur while polling tasks.
//!
//! It includes the `Executor`, `ExecutorTask`, and `Status` types, which
//! handle task scheduling, task states, and task completion respectively.
//!
//! Executors are designed to handle truly asynchronous lightweight tasks.
//! Blocking tasks are handled by `Blockers`

use std::{
    fmt::Display,
    sync::{mpsc::SyncSender, Arc},
    task::{Context, Waker},
};

use crate::task::{manager::TaskManager, Task};

pub type BlockingFn = dyn FnOnce() + Send + 'static;

/// Flavour of the executor - i.e. blocking or non-blocking
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub enum ExecutorFlavour {
    NonBlocking,
    Blocking,
}

/// Id to identify executors
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct ExecutorId(usize);

impl ExecutorId {
    pub fn get(&self) -> usize {
        self.0
    }
}

/// Enum representing a task that the executor can handle.
/// It can either be a `Task` to execute, or a `Finished` signal to stop the executor.
pub enum ExecutorTask {
    /// Represents a task to be executed, wrapped in an `Arc` for shared ownership. This task
    /// can be seen as a lightweight which will not block the executor.
    Task(Arc<Task>),
    /// Represents a function exection that will block the executor.
    Block(Box<BlockingFn>),
    /// Represents a signal that indicates the executor should stop.
    Finished,
}

impl Display for ExecutorTask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecutorTask::Task(_) => write!(f, "ExecutorTask::Task"),
            ExecutorTask::Block(_) => write!(f, "ExecutorTask::Block"),
            ExecutorTask::Finished => write!(f, "ExecutorTask::Finished"),
        }
    }
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
    /// Unique executor identifier
    id: ExecutorId,
    /// Flavour defines the type of tasks the executor can handle
    flavour: ExecutorFlavour,
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
    pub fn new(
        rx: std::sync::mpsc::Receiver<ExecutorTask>,
        executor_id: usize,
        flavour: ExecutorFlavour,
        panic_tx: SyncSender<()>,
    ) -> Self {
        Self {
            ready_queue: rx,
            panic_tx,
            id: ExecutorId(executor_id),
            flavour,
        }
    }

    /// Starts the executor loop, processing tasks from the ready queue.
    /// The loop continues until a `Finished` signal is received.
    ///
    /// Each task's future is polled, and if a panic occurs, it is caught
    /// and logged, with a notification sent via `panic_tx`.
    pub fn run(&self) {
        while let Ok(task) = self.ready_queue.recv() {
            match task {
                ExecutorTask::Finished => return, // Stop the executor
                ExecutorTask::Task(task) => self.forward_task(task), // Retrieve the task
                ExecutorTask::Block(f) => f(),
            };

            let tm = TaskManager::get();
            tm.executor_ready(self.id(), self.flavour());
        }
    }

    fn forward_task(&self, task: Arc<Task>) {
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

    pub fn id(&self) -> ExecutorId {
        self.id
    }

    pub fn flavour(&self) -> ExecutorFlavour {
        self.flavour
    }
}

/// Gets the executor flavours based on the number of workers. If the number of workers is less than 3,
/// then all the executors will be non-blocking. Otherwise, the first `num_workers - blocking_count` executors
/// will be non-blocking, and the rest will be blocking.
pub fn get_executor_flavours(num_workers: usize) -> Vec<ExecutorFlavour> {
    if num_workers < 3 {
        return (0..num_workers)
            .map(|_| ExecutorFlavour::NonBlocking)
            .collect();
    }

    // Only use blocking executors if you have 3 or more workers
    let mut blocking_count = 1;
    let mut range_increment = 6;
    let mut range_start = 3;
    while range_start + range_increment <= num_workers {
        range_start += range_increment;
        range_increment += 1;
        blocking_count += 1;
    }

    (0..num_workers)
        .map(|idx| {
            if idx < num_workers - blocking_count {
                ExecutorFlavour::NonBlocking
            } else {
                ExecutorFlavour::Blocking
            }
        })
        .collect()
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn test_exexcutor_flavours_from_num_workers() {
        let num_workers = 1;
        let actual = get_executor_flavours(num_workers);
        let expected = vec![ExecutorFlavour::NonBlocking];
        assert_eq!(actual, expected);

        let num_workers = 2;
        let actual = get_executor_flavours(num_workers);
        let expected = vec![ExecutorFlavour::NonBlocking, ExecutorFlavour::NonBlocking];
        assert_eq!(actual, expected);

        let num_workers = 3;
        let actual = get_executor_flavours(num_workers);
        let expected = vec![
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::Blocking,
        ];
        assert_eq!(actual, expected);

        let num_workers = 4;
        let actual = get_executor_flavours(num_workers);
        let expected = vec![
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::Blocking,
        ];
        assert_eq!(actual, expected);

        let num_workers = 9;
        let actual = get_executor_flavours(num_workers);
        let expected = vec![
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::Blocking,
            ExecutorFlavour::Blocking,
        ];
        assert_eq!(actual, expected);

        let num_workers = 10;
        let actual = get_executor_flavours(num_workers);
        let expected = vec![
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::Blocking,
            ExecutorFlavour::Blocking,
        ];
        assert_eq!(actual, expected);

        let num_workers = 16;
        let actual = get_executor_flavours(num_workers);
        let expected = vec![
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::Blocking,
            ExecutorFlavour::Blocking,
            ExecutorFlavour::Blocking,
        ];
        assert_eq!(actual, expected);

        let num_workers = 17;
        let actual = get_executor_flavours(num_workers);
        let expected = vec![
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::NonBlocking,
            ExecutorFlavour::Blocking,
            ExecutorFlavour::Blocking,
            ExecutorFlavour::Blocking,
        ];
        assert_eq!(actual, expected);
    }
}
