//! Counterexample representation and rendering.
//!
//! When a property violation is detected, a counterexample shows
//! the exact sequence of operations that led to the failure.

use std::fmt;

/// A counterexample showing the failure path.
///
/// Contains the sequence of states and thread actions that led
/// to an invariant violation. Can be rendered as a human-readable
/// thread diagram.
#[derive(Debug, Clone)]
pub struct Counterexample {
    /// Sequence of state snapshots
    pub states: Vec<StateSnapshot>,
    /// Thread interleaving that caused the failure
    pub interleaving: Vec<ThreadAction>,
    /// Memory-related issues detected
    pub memory_issues: Vec<MemoryIssue>,
    /// DST seed for reproduction (if applicable)
    pub dst_seed: Option<u64>,
    /// Human-readable description of the failure (invariant violations, etc.)
    pub description: Option<String>,
}

/// Snapshot of system state at a point in time.
#[derive(Debug, Clone)]
pub struct StateSnapshot {
    /// Step number in the execution
    pub step: u64,
    /// Description of the state
    pub description: String,
    /// Variable values at this point
    pub variables: Vec<(String, String)>,
}

/// Action taken by a thread.
#[derive(Debug, Clone)]
pub struct ThreadAction {
    /// Thread identifier
    pub thread_id: u64,
    /// Step number when this action occurred
    pub step: u64,
    /// Description of the action
    pub action: String,
    /// Whether this action succeeded
    pub success: bool,
}

/// Memory-related issue detected.
#[derive(Debug, Clone)]
pub enum MemoryIssue {
    /// Use-after-free detected
    UseAfterFree {
        address: u64,
        freed_at_step: u64,
        used_at_step: u64,
    },
    /// Data race detected
    DataRace {
        address: u64,
        thread_a: u64,
        thread_b: u64,
        step: u64,
    },
    /// ABA problem detected
    AbaProblem {
        address: u64,
        original_value: u64,
        intermediate_value: u64,
        final_value: u64,
        step: u64,
    },
    /// Memory leak detected
    MemoryLeak {
        address: u64,
        allocated_at_step: u64,
    },
}

impl Counterexample {
    /// Create a new empty counterexample.
    #[must_use]
    pub fn new() -> Self {
        Self {
            states: Vec::new(),
            interleaving: Vec::new(),
            memory_issues: Vec::new(),
            dst_seed: None,
            description: None,
        }
    }

    /// Create a counterexample with DST seed for reproduction.
    #[must_use]
    pub fn with_seed(seed: u64) -> Self {
        debug_assert!(seed != 0, "DST seed should not be zero");
        Self {
            states: Vec::new(),
            interleaving: Vec::new(),
            memory_issues: Vec::new(),
            dst_seed: Some(seed),
            description: None,
        }
    }

    /// Set the description for this counterexample.
    pub fn with_description(mut self, description: String) -> Self {
        self.description = Some(description);
        self
    }

    /// Add a state snapshot.
    pub fn add_state(&mut self, state: StateSnapshot) {
        debug_assert!(
            self.states.is_empty() || state.step > self.states.last().unwrap().step,
            "States must be added in order"
        );
        self.states.push(state);
    }

    /// Add a thread action.
    pub fn add_action(&mut self, action: ThreadAction) {
        self.interleaving.push(action);
    }

    /// Add a memory issue.
    pub fn add_memory_issue(&mut self, issue: MemoryIssue) {
        self.memory_issues.push(issue);
    }

    /// Render the counterexample as a human-readable thread diagram.
    ///
    /// Format:
    /// ```text
    /// DST_SEED=12345
    ///
    /// Step | Thread 0     | Thread 1     | State
    /// -----|--------------|--------------|-------
    ///    1 | push(42)     |              | head=N1
    ///    2 |              | pop() start  | head=N1
    ///    3 | push(43)     |              | head=N2
    ///    4 |              | CAS fail     | head=N2
    /// ```
    #[must_use]
    pub fn render_diagram(&self) -> String {
        let mut output = String::new();

        // DST seed for reproduction
        if let Some(seed) = self.dst_seed {
            output.push_str(&format!("DST_SEED={}\n\n", seed));
        }

        // Description (invariant violations, etc.)
        if let Some(ref desc) = self.description {
            output.push_str("Failure: ");
            output.push_str(desc);
            output.push_str("\n\n");
        }

        // Find all threads involved
        let mut threads: Vec<u64> = self
            .interleaving
            .iter()
            .map(|a| a.thread_id)
            .collect();
        threads.sort_unstable();
        threads.dedup();

        if threads.is_empty() {
            output.push_str("(no thread actions recorded)\n");
            return output;
        }

        // Header
        output.push_str("Step |");
        for tid in &threads {
            output.push_str(&format!(" Thread {} |", tid));
        }
        output.push_str(" State\n");

        output.push_str("-----|");
        for _ in &threads {
            output.push_str("----------|");
        }
        output.push_str("------\n");

        // Build step-by-step view
        let max_step = self
            .interleaving
            .iter()
            .map(|a| a.step)
            .max()
            .unwrap_or(0);

        for step in 1..=max_step {
            output.push_str(&format!("{:4} |", step));

            for tid in &threads {
                let action = self
                    .interleaving
                    .iter()
                    .find(|a| a.step == step && a.thread_id == *tid);

                match action {
                    Some(a) => {
                        let status = if a.success { "" } else { " [FAIL]" };
                        output.push_str(&format!(" {}{} |", a.action, status));
                    }
                    None => output.push_str("          |"),
                }
            }

            // State at this step
            if let Some(state) = self.states.iter().find(|s| s.step == step) {
                output.push_str(&format!(" {}", state.description));
            }

            output.push('\n');
        }

        // Memory issues
        if !self.memory_issues.is_empty() {
            output.push_str("\nMemory Issues:\n");
            for issue in &self.memory_issues {
                output.push_str(&format!("  - {}\n", issue));
            }
        }

        output
    }
}

impl Default for Counterexample {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for MemoryIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemoryIssue::UseAfterFree {
                address,
                freed_at_step,
                used_at_step,
            } => write!(
                f,
                "Use-after-free: address 0x{:x} freed at step {}, used at step {}",
                address, freed_at_step, used_at_step
            ),
            MemoryIssue::DataRace {
                address,
                thread_a,
                thread_b,
                step,
            } => write!(
                f,
                "Data race: address 0x{:x} accessed by threads {} and {} at step {}",
                address, thread_a, thread_b, step
            ),
            MemoryIssue::AbaProblem {
                address,
                original_value,
                intermediate_value,
                final_value,
                step,
            } => write!(
                f,
                "ABA problem: address 0x{:x} changed {} -> {} -> {} at step {}",
                address, original_value, intermediate_value, final_value, step
            ),
            MemoryIssue::MemoryLeak {
                address,
                allocated_at_step,
            } => write!(
                f,
                "Memory leak: address 0x{:x} allocated at step {}, never freed",
                address, allocated_at_step
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_counterexample_creation() {
        let ce = Counterexample::new();
        assert!(ce.states.is_empty());
        assert!(ce.interleaving.is_empty());
        assert!(ce.dst_seed.is_none());
    }

    #[test]
    fn test_counterexample_with_seed() {
        let ce = Counterexample::with_seed(12345);
        assert_eq!(ce.dst_seed, Some(12345));
    }

    #[test]
    fn test_render_diagram() {
        let mut ce = Counterexample::with_seed(42);

        ce.add_action(ThreadAction {
            thread_id: 0,
            step: 1,
            action: "push(1)".to_string(),
            success: true,
        });

        ce.add_action(ThreadAction {
            thread_id: 1,
            step: 2,
            action: "pop()".to_string(),
            success: true,
        });

        ce.add_state(StateSnapshot {
            step: 1,
            description: "head=N1".to_string(),
            variables: vec![],
        });

        let diagram = ce.render_diagram();
        assert!(diagram.contains("DST_SEED=42"));
        assert!(diagram.contains("Thread 0"));
        assert!(diagram.contains("push(1)"));
    }
}
