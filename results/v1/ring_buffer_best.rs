use std::sync::atomic::{AtomicUsize, AtomicPtr, Ordering};
use std::ptr;
use std::mem::MaybeUninit;
use std::cell::UnsafeCell;
use crossbeam_epoch::{self as epoch, Atomic, Owned, Shared, Guard};

pub struct RingBuffer<T> {
    buffer: Box<[UnsafeCell<MaybeUninit<T>>]>,
    capacity: usize,
    head: AtomicUsize,
    tail: AtomicUsize,
    size: AtomicUsize,
}

unsafe impl<T: Send> Send for RingBuffer<T> {}
unsafe impl<T: Send> Sync for RingBuffer<T> {}

impl<T> RingBuffer<T> {
    pub fn new(capacity: usize) -> Self {
        debug_assert!(capacity > 0, "capacity must be positive");
        
        let mut buffer = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            buffer.push(UnsafeCell::new(MaybeUninit::uninit()));
        }
        
        RingBuffer {
            buffer: buffer.into_boxed_slice(),
            capacity,
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            size: AtomicUsize::new(0),
        }
    }

    pub fn push(&self, value: T) -> bool {
        loop {
            let current_size = self.size.load(Ordering::Acquire);
            if current_size >= self.capacity {
                return false;
            }

            let tail = self.tail.load(Ordering::Acquire);
            let new_tail = (tail + 1) % self.capacity;

            if self.tail.compare_exchange_weak(
                tail,
                new_tail,
                Ordering::Release,
                Ordering::Relaxed
            ).is_ok() {
                unsafe {
                    (*self.buffer[tail].get()).write(value);
                }
                
                loop {
                    let old_size = self.size.load(Ordering::Acquire);
                    if self.size.compare_exchange_weak(
                        old_size,
                        old_size + 1,
                        Ordering::Release,
                        Ordering::Relaxed
                    ).is_ok() {
                        break;
                    }
                }
                
                return true;
            }
        }
    }

    pub fn pop(&self) -> Option<T> {
        loop {
            let current_size = self.size.load(Ordering::Acquire);
            if current_size == 0 {
                return None;
            }

            let head = self.head.load(Ordering::Acquire);
            let new_head = (head + 1) % self.capacity;

            if self.head.compare_exchange_weak(
                head,
                new_head,
                Ordering::Release,
                Ordering::Relaxed
            ).is_ok() {
                let value = unsafe {
                    (*self.buffer[head].get()).assume_init_read()
                };
                
                loop {
                    let old_size = self.size.load(Ordering::Acquire);
                    if self.size.compare_exchange_weak(
                        old_size,
                        old_size - 1,
                        Ordering::Release,
                        Ordering::Relaxed
                    ).is_ok() {
                        break;
                    }
                }
                
                return Some(value);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.size.load(Ordering::Acquire)
    }

    pub fn is_empty(&self) -> bool {
        self.size.load(Ordering::Acquire) == 0
    }

    pub fn is_full(&self) -> bool {
        self.size.load(Ordering::Acquire) >= self.capacity
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

impl<T> Drop for RingBuffer<T> {
    fn drop(&mut self) {
        while self.pop().is_some() {}
    }
}