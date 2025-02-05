use std::{
    cell::OnceCell,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::SyncSender,
        Arc,
    },
};

use dashmap::DashMap;

use crate::{
    executor::{BlockingFn, ExecutorFlavour, ExecutorId, ExecutorTask},
    task::Task,
    utils::ring_buffer::LockFreeBoundedRingBuffer,
};

/// Appox. 1 million tasks
pub const MAX_TASKS: usize = 1024 * 1024;

static TASK_MANAGER_ID: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// Singleton instance of the runtime.
    pub static TASK_MANAGER: OnceCell<Arc<TaskManager>> = const { OnceCell::new() };
}
/// A static structure used to distribute tasks to executors
pub struct TaskManager {
    /// ID is purely used for debugging
    id: usize,
    waiting_non_blocking_tasks: LockFreeBoundedRingBuffer<Arc<Task>>,
    waiting_blocking_tasks: LockFreeBoundedRingBuffer<Box<BlockingFn>>,
    /// Index into task that contains a task
    // used_slots: LockFreeBoundedRingBuffer<usize>,
    // waiting_tasks: LockFreeBoundedRingBuffer<usize>,
    waiting_executors: LockFreeBoundedRingBuffer<ExecutorId>,
    executors: DashMap<ExecutorId, SyncSender<ExecutorTask>>,
    // unavailable_executors: Vec<SyncSender<ExecutorTask>>,
}

/// For debugging
fn generate_task_manager_id() -> usize {
    TASK_MANAGER_ID.fetch_add(1, Ordering::Relaxed)
}

unsafe impl Sync for TaskManager {}

impl TaskManager {
    pub fn get() -> Arc<TaskManager> {
        let mut arc = None;
        TASK_MANAGER.with(|cell| {
            let arc_inner = cell.get_or_init(|| {
                Arc::new(TaskManager {
                    id: generate_task_manager_id(),
                    waiting_non_blocking_tasks: LockFreeBoundedRingBuffer::new(128),
                    waiting_blocking_tasks: LockFreeBoundedRingBuffer::new(128),
                    // used_slots: LockFreeBoundedRingBuffer::new(MAX_TASKS),
                    // waiting_tasks: LockFreeBoundedRingBuffer::new(MAX_TASKS),
                    // How many threads are you really going to be using?
                    waiting_executors: LockFreeBoundedRingBuffer::new(128),
                    executors: DashMap::with_capacity(128),
                })
            });
            arc = Some(Arc::clone(arc_inner));
        });
        arc.unwrap()
    }

    /// For debugging
    pub fn id(&self) -> usize {
        self.id
    }

    /// Register an executor to the task manager
    pub fn register_executor(
        &self,
        executor_id: ExecutorId,
        executor_sender: SyncSender<ExecutorTask>,
    ) {
        self.executors.insert(executor_id, executor_sender);
        self.waiting_executors.push(executor_id).unwrap();
    }

    /// Executor is ready for another task, if a task is immediately available then execute it,
    /// otherwise wait for a task to be executed
    pub fn executor_ready(&self, executor_id: ExecutorId) {
        while let Some(task) = self.waiting_non_blocking_tasks.pop() {
            if task.has_aborted() {
                continue;
            }
            let sender = self.executors.get(&executor_id).unwrap();
            sender.send(ExecutorTask::Task(task)).unwrap();
            return;
        }

        if let Some(f) = self.waiting_blocking_tasks.pop() {
            let sender = self.executors.get(&executor_id).unwrap();
            sender.send(ExecutorTask::Block(f)).unwrap();
            return;
        }
        self.waiting_executors.push(executor_id).unwrap();
    }

    /// If no executor is ready, register the task as waiting otherwise execute immediately.
    pub fn register_or_execute_non_blocking_task(&self, task: Arc<Task>) {
        println!("########### WAITING 1");
        if task.has_aborted() {
            return;
        }

        println!("########### WAITING 2");
        if let Some(exec) = self.waiting_executors.pop() {
            let sender = self.executors.get(&exec).unwrap();
            sender.send(ExecutorTask::Task(task)).unwrap();
            return;
        }

        println!("########### WAITING 4");
        self.waiting_non_blocking_tasks.push(task).unwrap();
    }
}
