use std::{
    sync::{mpsc::SyncSender, Arc},
    task::{Context, Waker},
};

use crate::spawner::Task;

pub enum ExecutorTask {
    Task(Arc<Task>),
    Finished,
}

#[derive(Debug)]
pub enum Status {
    Awaited(Waker),
    Happened,
}

pub struct Executor {
    ready_queue: std::sync::mpsc::Receiver<ExecutorTask>,
    panic_tx: SyncSender<()>,
}

impl Executor {
    pub fn new(rx: std::sync::mpsc::Receiver<ExecutorTask>, panic_tx: SyncSender<()>) -> Self {
        Self {
            ready_queue: rx,
            panic_tx,
        }
    }

    pub fn run(&self) {
        while let Ok(task) = self.ready_queue.recv() {
            let task = match task {
                ExecutorTask::Finished => return,
                ExecutorTask::Task(task) => task,
            };
            let mut future = task.future.lock().unwrap();

            // make a context
            let waker = Arc::clone(&task).waker();
            let mut context = Context::from_waker(&waker);
            // Alow the future some CPU time to make progress
            if let Err(e) = std::panic::catch_unwind(move || future.as_mut().poll(&mut context)) {
                println!("EXECUTOR PANIC FUNCTION. ERROR: {:?}", e);
                self.panic_tx.send(()).unwrap();
            }
        }
    }
}
