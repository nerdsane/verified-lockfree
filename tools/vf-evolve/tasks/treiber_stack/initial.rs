/// Treiber Stack - Initial Seed (Mutex-based blocking implementation)
/// ShinkaEvolve will evolve this from Blocking -> LockFree -> WaitFree
use std::sync::Mutex;

pub struct TreiberStack<T> {
    inner: Mutex<Vec<T>>,
}

impl<T> TreiberStack<T> {
    pub fn new() -> Self {
        TreiberStack {
            inner: Mutex::new(Vec::new()),
        }
    }

    pub fn push(&self, value: T) {
        let mut guard = self.inner.lock().unwrap();
        guard.push(value);
    }

    pub fn pop(&self) -> Option<T> {
        let mut guard = self.inner.lock().unwrap();
        guard.pop()
    }

    pub fn is_empty(&self) -> bool {
        let guard = self.inner.lock().unwrap();
        guard.is_empty()
    }

    pub fn len(&self) -> usize {
        let guard = self.inner.lock().unwrap();
        guard.len()
    }
}
