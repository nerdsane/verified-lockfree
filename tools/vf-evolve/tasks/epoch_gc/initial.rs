use std::sync::{Arc, Mutex};

pub struct EpochGc {
    inner: Arc<Mutex<GcState>>,
}

struct GcState {
    deferred: Vec<Box<dyn FnOnce() + Send + 'static>>,
    pinned: usize,
}

pub struct Guard {
    gc: Arc<Mutex<GcState>>,
}

impl EpochGc {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(GcState {
                deferred: Vec::new(),
                pinned: 0,
            })),
        }
    }

    pub fn pin(&self) -> Guard {
        let mut state = self.inner.lock().unwrap();
        state.pinned += 1;
        Guard {
            gc: Arc::clone(&self.inner),
        }
    }

    pub fn retire<T>(&self, ptr: *mut T)
    where
        T: Send + 'static,
    {
        // Convert pointer to usize for Send-ability
        let addr = ptr as usize;

        let mut state = self.inner.lock().unwrap();

        // Store as closure that converts back
        // Safety: T is Send, ptr is valid, will be dropped on correct thread
        state.deferred.push(Box::new(move || unsafe {
            let ptr = addr as *mut T;
            let _ = Box::from_raw(ptr);
        }));
    }

    pub fn collect(&self) {
        let mut state = self.inner.lock().unwrap();

        // Only collect if no guards are pinned
        if state.pinned == 0 {
            let deferred = std::mem::take(&mut state.deferred);

            // Drop the lock before running deferred functions
            drop(state);

            for f in deferred {
                f();
            }
        }
    }
}

impl Guard {
    pub fn defer(&self, f: impl FnOnce() + Send + 'static) {
        let mut state = self.gc.lock().unwrap();
        state.deferred.push(Box::new(f));
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        let mut state = self.gc.lock().unwrap();
        state.pinned = state.pinned.saturating_sub(1);
    }
}

// Safety: GcState contains only Send types
unsafe impl Send for GcState {}
