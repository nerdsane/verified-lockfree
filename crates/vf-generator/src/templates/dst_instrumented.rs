//! Template for DST-instrumented code generation.
//!
//! This shows the pattern that the generator should produce so that
//! DST can replay oracle scenarios against actual lock-free code.

/// Template for DST-instrumented Treiber stack.
///
/// The generator uses this pattern to produce code that:
/// 1. Implements the lock-free algorithm correctly
/// 2. Has yield points at CAS operations for DST control
/// 3. Tracks pushed/popped elements for invariant checking
pub const DST_INSTRUMENTED_TEMPLATE: &str = r#"
//! DST-instrumented Treiber Stack
//!
//! Generated from TLA+ spec: treiber_stack.tla
//! Invariants: NoLostElements (line 45), NoDuplicates (line 58)

use std::collections::HashSet;
use std::sync::atomic::{AtomicPtr, AtomicU64, Ordering};
use std::sync::Mutex;
use crossbeam_epoch::{self as epoch, Atomic, Owned, Shared};

use vf_dst::{DstContext, InstrumentedStack, YieldPoint};

/// Node in the stack.
struct Node {
    value: u64,
    next: Atomic<Node>,
}

/// DST-instrumented Treiber Stack.
pub struct TreiberStackDst {
    head: Atomic<Node>,
    // Tracking for invariant verification
    pushed: Mutex<HashSet<u64>>,
    popped: Mutex<HashSet<u64>>,
}

impl InstrumentedStack for TreiberStackDst {
    fn new() -> Self {
        Self {
            head: Atomic::null(),
            pushed: Mutex::new(HashSet::new()),
            popped: Mutex::new(HashSet::new()),
        }
    }

    fn push_instrumented(&self, value: u64, ctx: &mut DstContext) {
        let guard = epoch::pin();

        // Phase 1: Allocate node
        let node = Owned::new(Node {
            value,
            next: Atomic::null(),
        });
        ctx.set_value(value);
        ctx.yield_point(YieldPoint::PushAlloc);  // DST yield point

        let node = node.into_shared(&guard);

        loop {
            // Phase 2: Read current head
            let head = self.head.load(Ordering::Acquire, &guard);
            unsafe {
                (*node.as_raw()).next.store(head, Ordering::Relaxed);
            }
            ctx.yield_point(YieldPoint::PushReadHead);  // DST yield point

            // Phase 3: CAS to update head
            match self.head.compare_exchange(
                head,
                node,
                Ordering::Release,
                Ordering::Relaxed,
                &guard,
            ) {
                Ok(_) => {
                    // CAS succeeded
                    ctx.record_cas_success();
                    ctx.yield_point(YieldPoint::PushCas);  // DST yield point
                    self.pushed.lock().unwrap().insert(value);
                    break;
                }
                Err(_) => {
                    // CAS failed - retry
                    ctx.record_cas_failure();
                    ctx.yield_point(YieldPoint::PushCas);  // DST yield point (retry)
                }
            }
        }
    }

    fn pop_instrumented(&self, ctx: &mut DstContext) -> Option<u64> {
        let guard = epoch::pin();

        loop {
            // Phase 1: Read head
            let head = self.head.load(Ordering::Acquire, &guard);
            ctx.yield_point(YieldPoint::PopReadHead);  // DST yield point

            match unsafe { head.as_ref() } {
                None => {
                    // Stack is empty
                    return None;
                }
                Some(head_node) => {
                    let next = head_node.next.load(Ordering::Acquire, &guard);
                    let value = head_node.value;

                    // Phase 2: CAS to update head
                    match self.head.compare_exchange(
                        head,
                        next,
                        Ordering::Release,
                        Ordering::Relaxed,
                        &guard,
                    ) {
                        Ok(_) => {
                            // CAS succeeded
                            ctx.record_cas_success();
                            ctx.yield_point(YieldPoint::PopCas);  // DST yield point

                            // Safe to defer drop due to epoch GC
                            unsafe {
                                guard.defer_destroy(head);
                            }

                            self.popped.lock().unwrap().insert(value);
                            return Some(value);
                        }
                        Err(_) => {
                            // CAS failed - retry
                            ctx.record_cas_failure();
                            ctx.yield_point(YieldPoint::PopCas);  // DST yield point (retry)
                        }
                    }
                }
            }
        }
    }

    fn is_empty(&self) -> bool {
        let guard = epoch::pin();
        self.head.load(Ordering::Acquire, &guard).is_null()
    }

    fn pushed_elements(&self) -> HashSet<u64> {
        self.pushed.lock().unwrap().clone()
    }

    fn popped_elements(&self) -> HashSet<u64> {
        self.popped.lock().unwrap().clone()
    }

    fn get_contents(&self) -> Vec<u64> {
        let guard = epoch::pin();
        let mut result = Vec::new();
        let mut current = self.head.load(Ordering::Acquire, &guard);

        while let Some(node) = unsafe { current.as_ref() } {
            result.push(node.value);
            current = node.next.load(Ordering::Acquire, &guard);
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vf_dst::{OracleTrace, run_oracle_dst, DstOperation};

    #[test]
    fn test_with_oracle() {
        // Create oracle for concurrent push contention
        let oracle = OracleTrace::concurrent_push_contention(0, 1, 100, 200);

        // Operations for each thread
        let ops = vec![
            DstOperation::Push(100),  // T0
            DstOperation::Push(200),  // T1
        ];

        // Run DST with oracle
        let result = run_oracle_dst::<TreiberStackDst>(oracle, 2, ops);

        assert!(result.passed(), "DST failed: {}", result.format());
    }
}
"#;

/// Generate DST-instrumented code from spec.
pub fn generate_dst_instrumented_code(spec_name: &str) -> String {
    // In real implementation, this would:
    // 1. Parse TLA+ spec
    // 2. Extract invariants
    // 3. Generate instrumented Rust code

    format!(
        "// Generated DST-instrumented code for: {}\n\n{}",
        spec_name,
        DST_INSTRUMENTED_TEMPLATE
    )
}
