use crossbeam_epoch::{self as epoch, Atomic, Guard, Owned, Shared};
use std::cmp::Ordering;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

const B: usize = 6;

#[derive(Debug)]
struct Node {
    keys: Vec<u64>,
    vals: Vec<u64>,
    children: Vec<Atomic<Node>>,
    next: Atomic<Node>,
    is_leaf: bool,
}

impl Node {
    fn new_leaf() -> Self {
        Node {
            keys: Vec::with_capacity(2 * B - 1),
            vals: Vec::with_capacity(2 * B - 1),
            children: Vec::new(),
            next: Atomic::null(),
            is_leaf: true,
        }
    }

    fn new_internal() -> Self {
        Node {
            keys: Vec::with_capacity(2 * B - 1),
            vals: Vec::new(),
            children: Vec::with_capacity(2 * B),
            next: Atomic::null(),
            is_leaf: false,
        }
    }
}

pub struct ConcurrentBTree {
    root: Atomic<Node>,
    len: AtomicUsize,
}

impl ConcurrentBTree {
    pub fn new() -> Self {
        ConcurrentBTree {
            root: Atomic::new(Node::new_leaf()),
            len: AtomicUsize::new(0),
        }
    }

    pub fn insert(&self, key: u64, value: u64) {
        let guard = &epoch::pin();
        loop {
            let root_ptr = self.root.load(AtomicOrdering::Acquire, guard);
            if root_ptr.is_null() {
                let new_root = Owned::new(Node::new_leaf());
                match self.root.compare_exchange(
                    Shared::null(),
                    new_root,
                    AtomicOrdering::Release,
                    AtomicOrdering::Acquire,
                    guard,
                ) {
                    Ok(_) => continue,
                    Err(_) => continue,
                }
            }

            let root_ref = unsafe { root_ptr.deref() };
            
            if root_ref.is_leaf {
                if self.insert_into_leaf(key, value, root_ptr, guard) {
                    self.len.fetch_add(1, AtomicOrdering::Relaxed);
                    return;
                }
            } else {
                if self.insert_into_internal(key, value, root_ptr, guard) {
                    self.len.fetch_add(1, AtomicOrdering::Relaxed);
                    return;
                }
            }
        }
    }

    fn insert_into_leaf(&self, key: u64, value: u64, node_ptr: Shared<Node>, guard: &Guard) -> bool {
        let node = unsafe { node_ptr.deref() };
        
        let mut new_node = Node::new_leaf();
        new_node.keys = node.keys.clone();
        new_node.vals = node.vals.clone();
        
        match new_node.keys.binary_search(&key) {
            Ok(pos) => {
                new_node.vals[pos] = value;
            }
            Err(pos) => {
                new_node.keys.insert(pos, key);
                new_node.vals.insert(pos, value);
            }
        }

        if new_node.keys.len() >= 2 * B {
            return false;
        }

        let new_ptr = Owned::new(new_node).into_shared(guard);
        
        if node_ptr == self.root.load(AtomicOrdering::Acquire, guard) {
            match self.root.compare_exchange(
                node_ptr,
                new_ptr,
                AtomicOrdering::Release,
                AtomicOrdering::Acquire,
                guard,
            ) {
                Ok(_) => {
                    unsafe { guard.defer_destroy(node_ptr) };
                    true
                }
                Err(_) => {
                    unsafe { new_ptr.into_owned() };
                    false
                }
            }
        } else {
            false
        }
    }

    fn insert_into_internal(&self, key: u64, value: u64, node_ptr: Shared<Node>, guard: &Guard) -> bool {
        let node = unsafe { node_ptr.deref() };
        
        let pos = node.keys.binary_search(&key).unwrap_or_else(|p| p);
        let child_idx = if pos < node.keys.len() && key >= node.keys[pos] {
            pos + 1
        } else {
            pos
        };

        if child_idx < node.children.len() {
            let child_ptr = node.children[child_idx].load(AtomicOrdering::Acquire, guard);
            if !child_ptr.is_null() {
                let child = unsafe { child_ptr.deref() };
                if child.is_leaf {
                    return self.insert_into_leaf(key, value, child_ptr, guard);
                } else {
                    return self.insert_into_internal(key, value, child_ptr, guard);
                }
            }
        }
        
        false
    }

    pub fn get(&self, key: &u64) -> Option<u64> {
        let guard = &epoch::pin();
        let mut current = self.root.load(AtomicOrdering::Acquire, guard);
        
        while !current.is_null() {
            let node = unsafe { current.deref() };
            
            if node.is_leaf {
                match node.keys.binary_search(key) {
                    Ok(pos) => return Some(node.vals[pos]),
                    Err(_) => return None,
                }
            } else {
                let pos = node.keys.binary_search(key).unwrap_or_else(|p| p);
                let child_idx = if pos < node.keys.len() && *key >= node.keys[pos] {
                    pos + 1
                } else {
                    pos
                };
                
                if child_idx < node.children.len() {
                    current = node.children[child_idx].load(AtomicOrdering::Acquire, guard);
                } else {
                    return None;
                }
            }
        }
        
        None
    }

    pub fn remove(&self, key: &u64) -> bool {
        let guard = &epoch::pin();
        let root_ptr = self.root.load(AtomicOrdering::Acquire, guard);
        
        if root_ptr.is_null() {
            return false;
        }
        
        let root = unsafe { root_ptr.deref() };
        
        if root.is_leaf {
            match root.keys.binary_search(key) {
                Ok(pos) => {
                    let mut new_node = Node::new_leaf();
                    new_node.keys = root.keys.clone();
                    new_node.vals = root.vals.clone();
                    new_node.keys.remove(pos);
                    new_node.vals.remove(pos);
                    
                    let new_ptr = Owned::new(new_node).into_shared(guard);
                    
                    match self.root.compare_exchange(
                        root_ptr,
                        new_ptr,
                        AtomicOrdering::Release,
                        AtomicOrdering::Acquire,
                        guard,
                    ) {
                        Ok(_) => {
                            unsafe { guard.defer_destroy(root_ptr) };
                            self.len.fetch_sub(1, AtomicOrdering::Relaxed);
                            true
                        }
                        Err(_) => {
                            unsafe { new_ptr.into_owned() };
                            false
                        }
                    }
                }
                Err(_) => false,
            }
        } else {
            false
        }
    }

    pub fn range(&self, start: u64, end: u64) -> Vec<(u64, u64)> {
        let guard = &epoch::pin();
        let mut result = Vec::new();
        
        let root_ptr = self.root.load(AtomicOrdering::Acquire, guard);
        if root_ptr.is_null() {
            return result;
        }
        
        self.collect_range(root_ptr, start, end, &mut result, guard);
        result
    }

    fn collect_range(&self, node_ptr: Shared<Node>, start: u64, end: u64, result: &mut Vec<(u64, u64)>, guard: &Guard) {
        let node = unsafe { node_ptr.deref() };
        
        if node.is_leaf {
            for (i, &k) in node.keys.iter().enumerate() {
                if k >= start && k < end {
                    result.push((k, node.vals[i]));
                }
            }
        } else {
            for (i, &k) in node.keys.iter().enumerate() {
                if k >= end {
                    break;
                }
                
                if i < node.children.len() {
                    let child = node.children[i].load(AtomicOrdering::Acquire, guard);
                    if !child.is_null() {
                        self.collect_range(child, start, end, result, guard);
                    }
                }
                
                if k >= start && k < end {
                    // Internal nodes don't store values in this implementation
                }
            }
            
            if let Some(last_child) = node.children.last() {
                let child = last_child.load(AtomicOrdering::Acquire, guard);
                if !child.is_null() {
                    self.collect_range(child, start, end, result, guard);
                }
            }
        }
    }

    pub fn len(&self) -> usize {
        self.len.load(AtomicOrdering::Relaxed)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Drop for ConcurrentBTree {
    fn drop(&mut self) {
        let guard = &epoch::pin();
        let root = self.root.load(AtomicOrdering::Acquire, guard);
        if !root.is_null() {
            unsafe {
                guard.defer_destroy(root);
            }
        }
    }
}