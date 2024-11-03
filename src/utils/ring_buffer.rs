//! A lock-free bounded ring buffer implementation for high-performance,
//! multi-threaded environments. This buffer stores elements in a circular manner,
//! allowing concurrent push and pop operations without the need for locks.
//!
//! The buffer is bounded, meaning it has a fixed capacity, and once full,
//! further pushes will return an error. This implementation leverages atomic
//! operations to achieve lock-free behavior, making it suitable for low-latency
//! applications where contention is expected.
use std::{
    fmt::Debug,
    mem::MaybeUninit,
    sync::atomic::{AtomicUsize, Ordering},
};

/// A lock-free bounded ring buffer for storing elements in a circular manner.
/// The ring buffer allows for concurrent push and pop operations without locks,
/// making it suitable for high-performance and multi-threaded environments.
#[derive(Debug)]
pub struct LockFreeBoundedRingBuffer<T> {
    /// Internal buffer for storing elements in `MaybeUninit<T>` to avoid unnecessary initializations.
    buffer: Vec<MaybeUninit<T>>,
    /// Atomic index of the start of the buffer, used for pop operations.
    start: AtomicUsize,
    /// Atomic index of the end of the buffer, used for push operations.
    end: AtomicUsize,
    /// Atomic counter for the number of elements in the buffer.
    count: AtomicUsize,
}

impl<T> LockFreeBoundedRingBuffer<T> {
    /// Default buffer size of 1 MB * size of T
    const DEFAULT_BUFFER_SIZE: usize = 1024 * 1024;

    /// Creates a new ring buffer with the given bound, initializing the internal buffer.
    ///
    /// # Parameters
    /// - `bound`: The maximum number of elements that the buffer can hold.
    ///
    /// # Returns
    /// A new instance of `LockFreeBoundedRingBuffer<T>`.
    pub fn new(bound: usize) -> Self {
        Self {
            buffer: (0..bound).map(|_| MaybeUninit::uninit()).collect(),
            start: AtomicUsize::new(0),
            end: AtomicUsize::new(0),
            count: AtomicUsize::new(0),
        }
    }

    /// Inserts a value into the buffer at the specified index.
    /// This function is unsafe due to manual memory management.
    ///
    /// # Safety
    /// Caller must ensure the index is within bounds and the buffer slot
    /// is not being accessed concurrently.
    fn insert_value(&self, idx: usize, value: T) {
        unsafe {
            let buffer_ptr = self.buffer.as_ptr() as *mut MaybeUninit<T>;
            buffer_ptr.add(idx).drop_in_place(); // Clear existing value
            buffer_ptr.add(idx).write(MaybeUninit::new(value)); // Write new value
        }
    }

    /// Returns the capacity of the buffer.
    pub fn capacity(&self) -> usize {
        self.buffer.capacity()
    }

    /// Returns the current number of elements in the buffer.
    pub fn len(&self) -> usize {
        self.count.load(Ordering::Relaxed)
    }

    /// Checks if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Pushes a value into the buffer. Returns an error if the buffer is full.
    ///
    /// # Parameters
    /// - `value`: The value to be added to the buffer.
    ///
    /// # Returns
    /// - `Ok(())` on success.
    /// - `Err(String)` if the buffer is full.
    pub fn push(&self, value: T) -> Result<(), String> {
        // Check if the buffer is full
        if self.len() == self.capacity() {
            return Err("Buffer is full".into());
        }
        let current_end = self.end.load(Ordering::Acquire);

        // Calculate the next end position in a circular manner
        let new_end = if current_end + 1 < self.buffer.capacity() {
            current_end + 1
        } else {
            0
        };

        // Insert the value and update end index and count
        self.insert_value(current_end, value);
        self.count.fetch_add(1, Ordering::Relaxed);
        self.end.store(new_end, Ordering::Release);

        Ok(())
    }

    /// Retrieves a value from the buffer at the specified index and clears that position.
    ///
    /// # Safety
    /// The caller must ensure the index is within bounds and the slot is being accessed correctly.
    fn get_value(&self, idx: usize) -> T {
        unsafe {
            let buffer_ptr = self.buffer.as_ptr() as *mut MaybeUninit<T>;
            let value = std::ptr::replace(buffer_ptr.add(idx), MaybeUninit::uninit());
            value.assume_init()
        }
    }

    /// Pops a value from the buffer. Returns `None` if the buffer is empty.
    pub fn pop(&self) -> Option<T> {
        let current_start = self.start.load(Ordering::Acquire);
        let current_end = self.end.load(Ordering::Acquire);

        // Check if the buffer is empty
        if current_start == current_end && self.count.load(Ordering::Relaxed) == 0 {
            return None;
        }

        // Retrieve the value and update start index and count
        let value = self.get_value(current_start);
        self.count.fetch_sub(1, Ordering::Relaxed);

        // Calculate the next start position in a circular manner
        let new_start = if current_start + 1 >= self.buffer.capacity() {
            0
        } else {
            current_start + 1
        };

        self.start.store(new_start, Ordering::Release);

        Some(value)
    }
}

impl<T> Default for LockFreeBoundedRingBuffer<T> {
    /// Creates a new ring buffer with the default buffer size of 1 MB * size of T.
    fn default() -> Self {
        Self::new(Self::DEFAULT_BUFFER_SIZE)
    }
}

// Unit tests for the ring buffer implementation
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default() {
        let buffer = LockFreeBoundedRingBuffer::<i32>::default();
        assert!(buffer.buffer.capacity() == LockFreeBoundedRingBuffer::<i32>::DEFAULT_BUFFER_SIZE);
        assert!(buffer.is_empty());
    }

    #[test]
    fn test_push() {
        let buffer = LockFreeBoundedRingBuffer::<i32>::new(2);
        assert!(buffer.push(2).is_ok());
        assert!(buffer.len() == 1);
        assert!(buffer.push(3).is_ok());
        assert!(buffer.len() == 2);
        assert!(buffer.push(4).is_err()); // Buffer should be full
        assert!(buffer.len() == 2);
    }

    #[test]
    fn test_pop() {
        let buffer = LockFreeBoundedRingBuffer::<i32>::new(1);
        assert!(buffer.push(9).is_ok());
        let pop_res = buffer.pop();
        assert!(pop_res.is_some());
        assert!(pop_res.unwrap() == 9);
        assert!(buffer.is_empty());
    }

    #[test]
    fn test_multi_insert_multi_pop() {
        let buffer = LockFreeBoundedRingBuffer::<i32>::new(3);
        assert!(buffer.capacity() == 3);
        assert!(buffer.push(10).is_ok());
        assert!(buffer.len() == 1);
        assert!(buffer.push(12).is_ok());
        assert!(buffer.len() == 2);
        let pop_res = buffer.pop();
        assert!(pop_res.is_some());
        assert!(pop_res.unwrap() == 10);
        assert!(buffer.len() == 1);
        assert!(buffer.push(14).is_ok());
        assert!(buffer.len() == 2);
        assert!(buffer.push(15).is_ok());
        assert!(buffer.len() == 3);
        assert!(buffer.push(16).is_err()); // Buffer should be full
        assert!(buffer.len() == 3);
        let pop_res = buffer.pop();
        assert!(pop_res.is_some());
        assert!(pop_res.unwrap() == 12);
        assert!(buffer.len() == 2);
        let pop_res = buffer.pop();
        assert!(pop_res.is_some());
        assert!(pop_res.unwrap() == 14);
        assert!(buffer.len() == 1);
        let pop_res = buffer.pop();
        assert!(pop_res.is_some());
        assert!(pop_res.unwrap() == 15);
        assert!(buffer.is_empty());
        assert!(buffer.push(101).is_ok());
        assert!(buffer.len() == 1);
        let pop_res = buffer.pop();
        assert!(pop_res.is_some());
        assert!(pop_res.unwrap() == 101);
        assert!(buffer.is_empty());
        assert!(buffer.capacity() == 3);
    }
}
