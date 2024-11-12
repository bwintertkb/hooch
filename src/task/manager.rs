use std::{
    cell::OnceCell,
    mem::MaybeUninit,
    sync::{mpsc::SyncSender, Arc, OnceLock},
};

use crate::{
    executor::ExecutorTask, spawner::BoxedFutureResult,
    utils::ring_buffer::LockFreeBoundedRingBuffer,
};

/// Appox. 10 million tasks
pub const MAX_TASKS: usize = 1024 * 1024 * 10;

thread_local! {
    /// Singleton instance of the runtime.
    pub static TASK_MANAGER: OnceCell<Arc<TaskManager>> = const { OnceCell::new() };
}
/// A static structure used to distribute tasks to executors
pub struct TaskManager {
    tasks: Vec<MaybeUninit<BoxedFutureResult>>,
    /// Index into task that contains a task
    used_slots: LockFreeBoundedRingBuffer<usize>,
    waiting_tasks: LockFreeBoundedRingBuffer<usize>,
    waiting_executors: LockFreeBoundedRingBuffer<(ExecutorId, SyncSender<ExecutorTask>)>,
    unavailable_executors: Vec<SyncSender<ExecutorTask>>,
}

unsafe impl Sync for TaskManager {}

impl TaskManager {
    pub fn get() -> &'static TaskManager {
        static TASK_MANAGER: OnceLock<TaskManager> = OnceLock::new();

        TASK_MANAGER.get_or_init(|| TaskManager {
            tasks: (0..MAX_TASKS).map(|_| MaybeUninit::uninit()).collect(),
            used_slots: LockFreeBoundedRingBuffer::new(MAX_TASKS),
            waiting_tasks: LockFreeBoundedRingBuffer::new(MAX_TASKS),
            // How many threads are you really going to be using?
            waiting_executors: LockFreeBoundedRingBuffer::new(512),
        })
    }
}
