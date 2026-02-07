use crossbeam_epoch::{self as epoch, Atomic, Owned, Shared};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::cmp;

const FANOUT: usize = 256;

#[derive(Debug)]
struct Node {
    key_prefix: Vec<u8>,
    value: Option<u64>,
    children: [Atomic<Node>; FANOUT],
}

impl Node {
    fn new(key_prefix: Vec<u8>, value: Option<u64>) -> Self {
        const INIT: Atomic<Node> = Atomic::null();
        Self {
            key_prefix,
            value,
            children: [INIT; FANOUT],
        }
    }

    fn new_leaf(key: Vec<u8>, value: u64) -> Self {
        Self::new(key, Some(value))
    }

    fn new_internal(key_prefix: Vec<u8>) -> Self {
        Self::new(key_prefix, None)
    }
}

pub struct RadixTree {
    root: Atomic<Node>,
    len: AtomicUsize,
}

impl RadixTree {
    pub fn new() -> Self {
        Self {
            root: Atomic::null(),
            len: AtomicUsize::new(0),
        }
    }

    pub fn insert(&self, key: &[u8], value: u64) {
        let guard = &epoch::pin();
        
        loop {
            let root = self.root.load(Ordering::Acquire, guard);
            
            if root.is_null() {
                let new_root = Owned::new(Node::new_leaf(key.to_vec(), value));
                match self.root.compare_exchange_weak(
                    root,
                    new_root,
                    Ordering::Release,
                    Ordering::Relaxed,
                    guard,
                ) {
                    Ok(_) => {
                        self.len.fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                    Err(_) => continue,
                }
            } else {
                if self.insert_recursive(root, key, value, guard) {
                    return;
                }
            }
        }
    }

    fn insert_recursive(&self, node: Shared<Node>, key: &[u8], value: u64, guard: &epoch::Guard) -> bool {
        let node_ref = unsafe { node.deref() };
        let common_len = common_prefix_length(&node_ref.key_prefix, key);
        
        if common_len == node_ref.key_prefix.len() {
            if common_len == key.len() {
                // Exact match - this is a simplified approach
                return true;
            } else {
                // Insert into child
                let next_byte = key[common_len] as usize;
                let child = node_ref.children[next_byte].load(Ordering::Acquire, guard);
                
                if child.is_null() {
                    let new_child = Owned::new(Node::new_leaf(key[common_len + 1..].to_vec(), value));
                    match node_ref.children[next_byte].compare_exchange_weak(
                        child,
                        new_child,
                        Ordering::Release,
                        Ordering::Relaxed,
                        guard,
                    ) {
                        Ok(_) => {
                            self.len.fetch_add(1, Ordering::Relaxed);
                            return true;
                        }
                        Err(_) => return false,
                    }
                } else {
                    return self.insert_recursive(child, &key[common_len + 1..], value, guard);
                }
            }
        } else {
            // Need to split node - simplified for now
            return true;
        }
    }

    pub fn get(&self, key: &[u8]) -> Option<u64> {
        let guard = &epoch::pin();
        let root = self.root.load(Ordering::Acquire, guard);
        
        if root.is_null() {
            return None;
        }
        
        self.get_recursive(root, key, guard)
    }

    fn get_recursive(&self, node: Shared<Node>, key: &[u8], guard: &epoch::Guard) -> Option<u64> {
        let node_ref = unsafe { node.deref() };
        
        if key.len() < node_ref.key_prefix.len() {
            return None;
        }
        
        if !key.starts_with(&node_ref.key_prefix) {
            return None;
        }
        
        if key.len() == node_ref.key_prefix.len() {
            return node_ref.value;
        }
        
        let remaining = &key[node_ref.key_prefix.len()..];
        let next_byte = remaining[0] as usize;
        let child = node_ref.children[next_byte].load(Ordering::Acquire, guard);
        
        if child.is_null() {
            None
        } else {
            self.get_recursive(child, &remaining[1..], guard)
        }
    }

    pub fn remove(&self, key: &[u8]) -> bool {
        let guard = &epoch::pin();
        let root = self.root.load(Ordering::Acquire, guard);
        
        if root.is_null() {
            return false;
        }
        
        if self.remove_recursive(root, key, guard) {
            self.len.fetch_sub(1, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    fn remove_recursive(&self, node: Shared<Node>, key: &[u8], guard: &epoch::Guard) -> bool {
        let node_ref = unsafe { node.deref() };
        
        if key.len() < node_ref.key_prefix.len() {
            return false;
        }
        
        if !key.starts_with(&node_ref.key_prefix) {
            return false;
        }
        
        if key.len() == node_ref.key_prefix.len() {
            // This is simplified - in a full implementation we'd need to handle node removal
            return node_ref.value.is_some();
        }
        
        let remaining = &key[node_ref.key_prefix.len()..];
        let next_byte = remaining[0] as usize;
        let child = node_ref.children[next_byte].load(Ordering::Acquire, guard);
        
        if child.is_null() {
            false
        } else {
            self.remove_recursive(child, &remaining[1..], guard)
        }
    }

    pub fn len(&self) -> usize {
        self.len.load(Ordering::Relaxed)
    }
}

fn common_prefix_length(a: &[u8], b: &[u8]) -> usize {
    let max_len = cmp::min(a.len(), b.len());
    for i in 0..max_len {
        if a[i] != b[i] {
            return i;
        }
    }
    max_len
}