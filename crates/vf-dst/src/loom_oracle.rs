//! Oracle-guided loom exploration.
//!
//! This module connects oracles (from stateright) to loom WITHOUT
//! requiring instrumentation in generated code.
//!
//! # Design Principle: Code is Disposable
//!
//! Generated code should be PURE - just the algorithm. No DST hooks,
//! no special traits beyond what's needed for invariant checking.
//!
//! Loom already intercepts atomic operations automatically. Oracles
//! guide WHICH interleavings loom should prioritize, not how code
//! is written.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────┐
//! │  Pure Generated     │  ← No DST knowledge
//! │  Code               │  ← Just uses std::sync::atomic
//! │  (or loom::sync     │  ← #[cfg(loom)] swap only
//! │   under test)       │
//! └──────────┬──────────┘
//!            │
//!            │ loom intercepts atomics
//!            ▼
//! ┌─────────────────────┐
//! │  Loom Runtime       │
//! │  ├── Intercepts CAS │
//! │  ├── Controls yield │
//! │  └── Explores       │
//! │      interleavings  │
//! └──────────┬──────────┘
//!            │
//!            │ oracles suggest interesting schedules
//!            ▼
//! ┌─────────────────────┐
//! │  Oracle Hints       │
//! │  (from stateright)  │
//! └─────────────────────┘
//! ```
//!
//! # How Oracles Help Loom
//!
//! Loom explores interleavings exhaustively (up to preemption bound).
//! Oracles from stateright tell us which patterns are interesting:
//!
//! 1. **CAS contention**: Two threads racing on same CAS
//! 2. **Retry scenarios**: Thread fails CAS multiple times
//! 3. **ABA patterns**: Value changes A→B→A
//!
//! We encode these as loom test scenarios that exercise the patterns.

use std::collections::HashSet;

/// Oracle-derived test scenario for loom.
///
/// These aren't "schedules" (loom controls that) but rather
/// "operation sequences" that tend to trigger interesting interleavings.
#[derive(Debug, Clone)]
pub struct LoomScenario {
    /// Name for debugging
    pub name: String,
    /// Number of threads
    pub threads: usize,
    /// Operations per thread (thread_id -> operations)
    pub operations: Vec<Vec<LoomOp>>,
    /// Description of what this tests
    pub description: String,
}

/// Operation in a loom scenario.
#[derive(Debug, Clone)]
pub enum LoomOp {
    Push(u64),
    Pop,
}

impl LoomScenario {
    /// Concurrent push contention - derived from stateright oracle.
    ///
    /// Two threads push simultaneously. Loom will explore all interleavings
    /// including the one where T0's CAS fails.
    pub fn concurrent_push(v0: u64, v1: u64) -> Self {
        Self {
            name: "concurrent_push".into(),
            threads: 2,
            operations: vec![
                vec![LoomOp::Push(v0)], // T0
                vec![LoomOp::Push(v1)], // T1
            ],
            description: "Two threads push - loom explores CAS contention".into(),
        }
    }

    /// Concurrent pop contention.
    pub fn concurrent_pop() -> Self {
        Self {
            name: "concurrent_pop".into(),
            threads: 2,
            operations: vec![
                vec![LoomOp::Pop], // T0
                vec![LoomOp::Pop], // T1
            ],
            description: "Two threads pop - loom explores CAS contention".into(),
        }
    }

    /// Push-pop race.
    pub fn push_pop_race(value: u64) -> Self {
        Self {
            name: "push_pop_race".into(),
            threads: 2,
            operations: vec![
                vec![LoomOp::Push(value)], // T0 pushes
                vec![LoomOp::Pop],          // T1 pops
            ],
            description: "Push and pop racing - tests visibility".into(),
        }
    }

    /// Multiple operations causing repeated CAS failures.
    pub fn cas_storm(values: &[u64]) -> Self {
        Self {
            name: "cas_storm".into(),
            threads: values.len(),
            operations: values.iter().map(|&v| vec![LoomOp::Push(v)]).collect(),
            description: "Many threads pushing - maximum CAS contention".into(),
        }
    }

    /// All standard scenarios derived from stateright oracles.
    pub fn all_scenarios() -> Vec<Self> {
        vec![
            Self::concurrent_push(100, 200),
            Self::concurrent_pop(),
            Self::push_pop_race(100),
            Self::cas_storm(&[1, 2, 3]),
        ]
    }
}

/// Trait for stacks that can be tested with loom.
///
/// This is MINIMAL - just what's needed for invariant checking.
/// No DST instrumentation required.
pub trait LoomTestableStack: Send + Sync + 'static {
    fn new() -> Self;
    fn push(&self, value: u64);
    fn pop(&self) -> Option<u64>;

    // For invariant checking only
    fn pushed_elements(&self) -> HashSet<u64>;
    fn popped_elements(&self) -> HashSet<u64>;
    fn get_contents(&self) -> Vec<u64>;
}

/// Generate loom test code for a scenario.
///
/// This produces a loom::model! block that tests the scenario.
/// The generated code is pure - no instrumentation.
pub fn generate_loom_test(scenario: &LoomScenario) -> String {
    let mut code = format!(
        r#"
#[test]
#[cfg(loom)]
fn loom_test_{}() {{
    use loom::thread;
    use std::sync::Arc;

    loom::model(|| {{
        let stack = Arc::new(TreiberStack::new());
"#,
        scenario.name
    );

    // Spawn threads
    for (tid, ops) in scenario.operations.iter().enumerate() {
        code.push_str(&format!(
            r#"
        let stack_{} = Arc::clone(&stack);
        let t{} = thread::spawn(move || {{
"#,
            tid, tid
        ));

        for op in ops {
            match op {
                LoomOp::Push(v) => {
                    code.push_str(&format!("            stack_{}.push({});\n", tid, v));
                }
                LoomOp::Pop => {
                    code.push_str(&format!("            stack_{}.pop();\n", tid));
                }
            }
        }

        code.push_str("        });\n");
    }

    // Join threads
    for tid in 0..scenario.threads {
        code.push_str(&format!("        t{}.join().unwrap();\n", tid));
    }

    // Check invariants
    code.push_str(
        r#"
        // Check invariants
        assert!(stack.check_no_lost_elements(), "NoLostElements violated");
        assert!(stack.check_no_duplicates(), "NoDuplicates violated");
    });
}
"#,
    );

    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scenario_creation() {
        let scenario = LoomScenario::concurrent_push(1, 2);
        assert_eq!(scenario.threads, 2);
        assert_eq!(scenario.operations.len(), 2);
    }

    #[test]
    fn test_generate_loom_test() {
        let scenario = LoomScenario::concurrent_push(100, 200);
        let code = generate_loom_test(&scenario);

        assert!(code.contains("loom::model"));
        assert!(code.contains("push(100)"));
        assert!(code.contains("push(200)"));
        assert!(code.contains("check_no_lost_elements"));
    }

    #[test]
    fn test_all_scenarios() {
        let scenarios = LoomScenario::all_scenarios();
        assert!(scenarios.len() >= 4);
    }
}
