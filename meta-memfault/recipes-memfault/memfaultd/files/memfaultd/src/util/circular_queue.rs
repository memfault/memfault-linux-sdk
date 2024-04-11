//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! A simple circular queue implementation
//!
//! This module provides a simple circular queue implementation. The buffer has a
//! fixed capacity and will overwrite the oldest item when it is full.

use std::collections::VecDeque;

#[derive(Debug, Default, Clone)]
pub struct CircularQueue<T> {
    buffer: VecDeque<T>,
    capacity: usize,
}

impl<T> CircularQueue<T> {
    /// Create a new circular queue with the given capacity
    pub fn new(capacity: usize) -> Self {
        CircularQueue {
            buffer: VecDeque::with_capacity(capacity + 1),
            capacity,
        }
    }

    /// Push an item onto the buffer.
    ///
    /// If the buffer is full, the oldest item will be removed.
    pub fn push(&mut self, item: T) {
        self.buffer.push_back(item);

        // If the buffer is full, remove the oldest item
        if self.buffer.len() > self.capacity {
            self.buffer.pop_front();
        }
    }

    /// Pop an item from the buffer.
    pub fn pop(&mut self) -> Option<T> {
        self.buffer.pop_front()
    }

    /// Return true if the buffer is empty
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Return true if the buffer is full
    pub fn is_full(&self) -> bool {
        self.buffer.len() == self.capacity
    }

    /// Return the number of items in the buffer
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Return the buffer capacity
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Return a reference to the back item in the buffer
    pub fn back(&self) -> Option<&T> {
        self.buffer.back()
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.buffer.iter()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_ring_buffer() {
        let mut rb = CircularQueue::new(3);
        assert_eq!(rb.len(), 0);
        assert_eq!(rb.capacity(), 3);
        assert!(rb.is_empty());

        rb.push(1);
        assert_eq!(rb.len(), 1);
        assert_eq!(rb.pop(), Some(1));
        assert!(rb.is_empty());

        rb.push(2);
        rb.push(3);
        rb.push(4);
        assert_eq!(rb.len(), 3);
        assert_eq!(rb.pop(), Some(2));
        assert_eq!(rb.pop(), Some(3));
        assert_eq!(rb.pop(), Some(4));
        assert!(rb.is_empty());
    }

    #[test]
    fn test_wrap_around() {
        let mut rb = CircularQueue::new(3);
        rb.push(1);
        rb.push(2);
        rb.push(3);
        assert_eq!(rb.len(), 3);

        rb.push(4);
        rb.push(5);
        assert_eq!(rb.len(), 3);
        assert_eq!(rb.pop(), Some(3));
        assert_eq!(rb.pop(), Some(4));
        assert_eq!(rb.pop(), Some(5));
        assert!(rb.is_empty());
    }

    #[test]
    fn empty_pop() {
        let mut rb: CircularQueue<u32> = CircularQueue::new(3);
        assert_eq!(rb.pop(), None);
    }
}
