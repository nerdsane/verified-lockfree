use std::sync::atomic::{AtomicUsize, AtomicPtr, Ordering};
use std::sync::Arc;
use std::ptr;

pub struct EpochGc {
    inner: Arc<GcState>,
}

struct GcState {
    deferred: AtomicPtr<DeferredNode>,
    pinned: AtomicUsize,
}

struct DeferredNode {
    next: *mut DeferredNode,
    func: Box<dyn FnOnce() + Send + 'static>,
}

pub struct Guard {
    gc: Arc<GcState>,
}

impl EpochGc {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(GcState {
                deferred: AtomicPtr::new(ptr::null_mut()),
                pinned: AtomicUsize::new(0),
            }),
        }
    }

    pub fn pin(&self) -> Guard {
        self.inner.pinned.fetch_add(1, Ordering::AcqRel);
        Guard {
            gc: Arc::clone(&self.inner),
        }
    }

    pub fn retire<T>(&self, ptr: *mut T)
    where
        T: Send + 'static,
    {
        let addr = ptr as usize;
        let func = Box::new(move || unsafe {
            let ptr = addr as *mut T;
            let _ = Box::from_raw(ptr);
        });

        let node = Box::into_raw(Box::new(DeferredNode {
            next: ptr::null_mut(),
            func,
        }));

        loop {
            let head = self.inner.deferred.load(Ordering::Acquire);
            unsafe {
                (*node).next = head;
            }
            match self.inner.deferred.compare_exchange(
                head,
                node,
                Ordering::Release,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(_) => continue,
            }
        }
    }

    pub fn collect(&self) {
        if self.inner.pinned.load(Ordering::Acquire) == 0 {
            let head = self.inner.deferred.swap(ptr::null_mut(), Ordering::AcqRel);
            
            if !head.is_null() {
                let mut current = head;
                let mut nodes = Vec::new();
                
                unsafe {
                    while !current.is_null() {
                        let node = Box::from_raw(current);
                        current = node.next;
                        nodes.push(node);
                    }
                }
                
                for node in nodes {
                    (node.func)();
                }
            }
        }
    }
}

impl Guard {
    pub fn defer(&self, f: impl FnOnce() + Send + 'static) {
        let node = Box::into_raw(Box::new(DeferredNode {
            next: ptr::null_mut(),
            func: Box::new(f),
        }));

        loop {
            let head = self.gc.deferred.load(Ordering::Acquire);
            unsafe {
                (*node).next = head;
            }
            match self.gc.deferred.compare_exchange(
                head,
                node,
                Ordering::Release,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(_) => continue,
            }
        }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.gc.pinned.fetch_sub(1, Ordering::AcqRel);
    }
}

unsafe impl Send for GcState {}
unsafe impl Sync for GcState {}