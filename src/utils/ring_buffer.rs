use std::{
    fmt::Debug,
    mem::MaybeUninit,
    sync::atomic::{AtomicUsize, Ordering},
};

#[derive(Debug)]
pub struct LockFreeBoundedRingBuffer<T> {
    buffer: Vec<MaybeUninit<T>>,
    start: AtomicUsize,
    end: AtomicUsize,
    count: AtomicUsize,
}

impl<T> LockFreeBoundedRingBuffer<T> {
    /// Default buffer size of `1 Mb * std::mem::size_of::<T>()`
    const DEFAULT_BUFFER_SIZE: usize = 1024 * 1024;
    /// Lock free bounded ring buffer of size `bound * std::mem::size_of::<T>()`
    pub fn new(bound: usize) -> Self {
        Self {
            buffer: (0..bound).map(|_| MaybeUninit::uninit()).collect(),
            start: AtomicUsize::new(0),
            end: AtomicUsize::new(0),
            count: AtomicUsize::new(0),
        }
    }

    fn insert_value(&self, idx: usize, value: T) {
        unsafe {
            let buffer_ptr = self.buffer.as_ptr() as *mut MaybeUninit<T>;
            buffer_ptr.add(idx).drop_in_place();
            buffer_ptr.add(idx).write(MaybeUninit::new(value));
        }
    }

    pub fn capacity(&self) -> usize {
        self.buffer.capacity()
    }

    pub fn len(&self) -> usize {
        self.count.load(Ordering::Relaxed)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Push a value into the buffer
    pub fn push(&self, value: T) -> Result<(), String> {
        // Check if buffer is full
        if self.len() == self.capacity() {
            return Err("Buffer is full".into());
        }
        let current_end = self.end.load(Ordering::Acquire);

        let new_end = if current_end + 1 < self.buffer.capacity() {
            current_end + 1
        } else {
            0
        };

        self.insert_value(current_end, value);
        self.count.fetch_add(1, Ordering::Relaxed);
        self.end.store(new_end, Ordering::Release);

        Ok(())
    }

    // fn get_value_ref(&self, idx: usize) -> &T {
    //     unsafe {
    //         let buffer_ptr = self.buffer.as_ptr() as *mut MaybeUninit<T>;
    //         let value = &*buffer_ptr;
    //         value.assume_init_ref()
    //     }
    // }

    fn get_value(&self, idx: usize) -> T {
        unsafe {
            let buffer_ptr = self.buffer.as_ptr() as *mut MaybeUninit<T>;
            let value = std::ptr::replace(buffer_ptr.add(idx), MaybeUninit::uninit());
            value.assume_init()
        }
    }

    /// Pop a value from the buffer
    pub fn pop(&self) -> Option<T> {
        let current_start = self.start.load(Ordering::Acquire);
        let current_end = self.end.load(Ordering::Acquire);

        // Buffer is empty if start and end are equal
        if current_start == current_end && self.count.load(Ordering::Relaxed) == 0 {
            return None;
        }

        let value = self.get_value(current_start);
        self.count.fetch_sub(1, Ordering::Relaxed);

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
    /// Default buffer size of 1 Mb * sizeof::<T>()
    fn default() -> Self {
        Self::new(Self::DEFAULT_BUFFER_SIZE)
    }
}

// TODO create a display for this struct
// impl<T> Display for LockFreeBoundedRingBuffer<T>
// where
//     T: std::fmt::Debug,
// {
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         let start = self.start.load(Ordering::Relaxed);
//         let end = self.end.load(Ordering::Relaxed);
//
//
//
//         write!(f, "[")
//
//
//
//     }
// }

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
        assert!(buffer.push(4).is_err());
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
        assert!(buffer.push(16).is_err());
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
