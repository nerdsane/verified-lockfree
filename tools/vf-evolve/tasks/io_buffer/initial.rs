use std::sync::Mutex;

pub struct IoBuffer {
    inner: Mutex<Vec<u8>>,
    capacity: usize,
}

impl IoBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(Vec::with_capacity(capacity)),
            capacity,
        }
    }

    pub fn append(&self, data: &[u8]) -> usize {
        let mut buf = self.inner.lock().unwrap();
        let offset = buf.len();
        buf.extend_from_slice(data);
        offset
    }

    pub fn flush(&self) -> Vec<u8> {
        let mut buf = self.inner.lock().unwrap();
        std::mem::replace(&mut *buf, Vec::with_capacity(self.capacity))
    }

    pub fn len(&self) -> usize {
        let buf = self.inner.lock().unwrap();
        buf.len()
    }
}
