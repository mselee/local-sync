#![allow(unused)]

use std::borrow::Borrow;
use std::cell::UnsafeCell;
use std::collections::VecDeque;
use std::mem::MaybeUninit;
use std::ptr::{null, null_mut, NonNull};

const BLOCK_CAP: usize = 32;
pub(crate) struct Block<T> {
    /// The next block in the linked list.
    next: UnsafeCell<*mut Block<T>>,

    /// Array containing values pushed into the block.
    values: [UnsafeCell<MaybeUninit<T>>; BLOCK_CAP],

    /// Head index.
    begin: usize,

    /// Tail index.
    end: usize,
}

impl<T> Block<T> {
    pub(crate) fn new() -> Self {
        let vals = unsafe { MaybeUninit::uninit() };
        Self {
            next: UnsafeCell::new(null_mut()),
            values: unsafe { vals.assume_init() },
            begin: 0,
            end: 0,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.end - self.begin
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.end == self.begin
    }

    pub(crate) fn next(&self) -> Option<NonNull<Block<T>>> {
        let ptr = unsafe { *self.next.get() };
        NonNull::new(ptr)
    }

    pub(crate) unsafe fn reset(&mut self) {
        *self.next.get_mut() = null_mut();
        self.begin = 0;
        self.end = 0;
    }
}

pub(crate) struct Queue<T> {
    /// The block to read data from.
    head: NonNull<Block<T>>,
    /// The block to write data to. It must be a valid block that has space.
    tail: NonNull<Block<T>>,
    /// Data length
    len: usize,
    /// Capacity(0 means unbounded)
    cap: usize,
}

impl<T> Queue<T> {
    pub(crate) fn new(cap: Option<usize>) -> Self {
        let block = Box::new(Block::new());
        let ptr = unsafe { NonNull::new_unchecked(Box::into_raw(block)) };
        Self {
            head: ptr,
            tail: ptr,
            len: 0,
            cap: cap.unwrap_or_default(),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// is_full always returns false for unbounded queue.
    pub(crate) fn is_full(&self) -> bool {
        self.cap == 0 || self.len < self.cap
    }

    /// Push data into queue.
    /// Safety: Make sure the current capacity is allowed.
    pub(crate) unsafe fn push_unchecked(&mut self, value: T) {
        // Write data and update block end index
        let blk = self.tail.as_mut();
        let offset = blk.end;
        blk.end += 1;
        let ptr = blk.values[offset].get();
        ptr.write(MaybeUninit::new(value));

        // Update queue length and make sure tail point to a valid block(not full)
        self.len += 1;
        if blk.end == BLOCK_CAP {
            if let Some(ptr) = blk.next() {
                // just move the tail ptr
                self.tail = ptr;
            } else {
                // alloc a new block
                let block = Box::new(Block::new());
                let ptr = unsafe { NonNull::new_unchecked(Box::into_raw(block)) };
                *blk.next.get_mut() = ptr.as_ptr();
                // move the tail ptr
                self.tail = ptr;
            }
        }
    }

    /// Pop data out.
    /// Safety: Make sure there is still some data inside.
    pub(crate) unsafe fn pop_unchecked(&mut self) -> T {
        // Read data and update block read index
        let blk = self.head.as_mut();
        let offset = blk.begin;
        blk.begin += 1;
        let ptr = blk.values[offset].get();
        let value = ptr.read().assume_init();

        // Update queue length and try to recycle the head block if its empty.
        self.len -= 1;
        if blk.begin == BLOCK_CAP {
            // Update head of queue.
            self.head = blk.next().expect("internal error");
            // Move block to the tail and reset it.
            let tail = self.tail.as_mut();
            let free_blocks = *tail.next.get_mut();
            blk.reset();
            *blk.next.get_mut() = free_blocks;
            *tail.next.get_mut() = blk;
        }
        value
    }

    /// Free all blocks.
    /// # Safety: Free blocks and drop.
    pub(crate) unsafe fn free_blocks(&mut self) {
        debug_assert_ne!(self.head, NonNull::dangling());
        let mut cur = Some(self.head);

        #[cfg(debug_assertions)]
        {
            // to trigger the debug assert above so as to catch that we
            // don't call `free_blocks` more than once.
            self.head = NonNull::dangling();
        }

        while let Some(block) = cur {
            cur = block.as_ref().next();
            drop(Box::from_raw(block.as_ptr()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Queue;

    #[test]
    fn test_simple_push_pop() {
        let mut queue = Queue::new(Some(12));
        unsafe {
            queue.push_unchecked(1);
            queue.push_unchecked(2);
            queue.push_unchecked(3);
            assert_eq!(queue.len(), 3);
            assert_eq!(queue.pop_unchecked(), 1);
            assert_eq!(queue.pop_unchecked(), 2);
            assert_eq!(queue.pop_unchecked(), 3);
            assert_eq!(queue.len(), 0);
        }
    }

    #[test]
    fn test_across_block_push_pop() {
        let mut queue = Queue::new(Some(1024));
        unsafe {
            for i in 0..4 {
                for idx in 0..1024 {
                    queue.push_unchecked(idx);
                    assert_eq!(queue.len(), idx + 1);
                }
                for idx in 0..1024 {
                    assert_eq!(queue.pop_unchecked(), idx);
                    assert_eq!(queue.len(), 1023 - idx);
                }
            }
        }
    }
}
