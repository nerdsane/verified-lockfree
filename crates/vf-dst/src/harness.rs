//! DST test harness for running reproducible concurrent tests.
//!
//! The harness provides a structured way to run DST tests with:
//! - Configurable number of threads
//! - Deterministic scheduling
//! - Fault injection
//! - Invariant checking at each step

use crate::{DstEnv, FaultConfig, ScheduleDecision};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Configuration for DST test harness.
#[derive(Debug, Clone)]
pub struct HarnessConfig {
    /// Number of threads to simulate
    pub threads_count: usize,
    /// Number of operations per thread
    pub operations_per_thread: u64,
    /// Probability of context switch at yield points
    pub yield_probability: f64,
    /// Fault injection configuration
    pub fault_config: FaultConfig,
    /// Check invariants after every N operations (0 = never)
    pub invariant_check_interval: u64,
}

impl Default for HarnessConfig {
    fn default() -> Self {
        Self {
            threads_count: 4,
            operations_per_thread: 100,
            yield_probability: 0.2,
            fault_config: FaultConfig::default(),
            invariant_check_interval: 10,
        }
    }
}

impl HarnessConfig {
    /// Configuration for stress testing.
    pub fn stress() -> Self {
        Self {
            threads_count: 8,
            operations_per_thread: 1000,
            yield_probability: 0.3,
            fault_config: FaultConfig::aggressive(),
            invariant_check_interval: 100,
        }
    }

    /// Configuration for quick testing.
    pub fn quick() -> Self {
        Self {
            threads_count: 2,
            operations_per_thread: 50,
            yield_probability: 0.1,
            fault_config: FaultConfig::none(),
            invariant_check_interval: 10,
        }
    }
}

/// Operation that a thread can perform.
#[derive(Debug, Clone)]
pub enum ThreadOp<T> {
    /// Custom operation with data
    Custom(T),
    /// Yield point (potential context switch)
    Yield,
    /// Check invariants
    CheckInvariants,
}

/// Result of running the harness.
#[derive(Debug, Clone)]
pub struct HarnessResult {
    /// Seed used for reproduction
    pub seed: u64,
    /// Total operations executed
    pub operations_count: u64,
    /// Context switches that occurred
    pub context_switches_count: u64,
    /// Faults injected
    pub faults_injected_count: u64,
    /// Invariant checks performed
    pub invariant_checks_count: u64,
    /// Whether all invariants held
    pub all_invariants_held: bool,
    /// First violation (if any)
    pub first_violation: Option<String>,
}

/// DST test harness for concurrent testing.
///
/// The harness simulates concurrent execution with deterministic scheduling.
/// Given the same seed, the same interleaving is produced.
pub struct DstHarness {
    env: DstEnv,
    config: HarnessConfig,
    operations_count: AtomicU64,
    context_switches_count: AtomicU64,
    invariant_checks_count: AtomicU64,
    violation: std::sync::Mutex<Option<String>>,
    stopped: AtomicBool,
}

impl DstHarness {
    /// Create a new harness with the given seed and config.
    pub fn new(seed: u64, config: HarnessConfig) -> Self {
        debug_assert!(seed != 0, "Seed should not be zero");
        debug_assert!(config.threads_count > 0, "Must have at least one thread");
        debug_assert!(
            config.threads_count <= 16,
            "Too many threads for DST: {}",
            config.threads_count
        );

        let env = DstEnv::with_scheduler(seed, config.threads_count);

        Self {
            env,
            config,
            operations_count: AtomicU64::new(0),
            context_switches_count: AtomicU64::new(0),
            invariant_checks_count: AtomicU64::new(0),
            violation: std::sync::Mutex::new(None),
            stopped: AtomicBool::new(false),
        }
    }

    /// Get the seed for reproduction.
    pub fn seed(&self) -> u64 {
        self.env.seed()
    }

    /// Get the environment for custom operations.
    pub fn env(&mut self) -> &mut DstEnv {
        &mut self.env
    }

    /// Check if the harness has been stopped due to a violation.
    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }

    /// Stop the harness with a violation message.
    pub fn stop_with_violation(&self, message: String) {
        let mut guard = self.violation.lock().unwrap();
        if guard.is_none() {
            *guard = Some(message);
        }
        self.stopped.store(true, Ordering::Release);
    }

    /// Record an operation.
    pub fn record_operation(&self) {
        self.operations_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Record an invariant check.
    pub fn record_invariant_check(&self) {
        self.invariant_checks_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Yield point - potentially switch to another thread.
    pub fn yield_point(&mut self) -> ScheduleDecision {
        if let Some(scheduler) = self.env.scheduler() {
            let decision = scheduler.decide();
            if decision != ScheduleDecision::Continue {
                self.context_switches_count.fetch_add(1, Ordering::Relaxed);
            }
            decision
        } else {
            ScheduleDecision::Continue
        }
    }

    /// Get the current thread ID.
    pub fn current_thread(&mut self) -> Option<usize> {
        self.env.scheduler().map(|s| s.current_thread())
    }

    /// Should we check invariants now?
    pub fn should_check_invariants(&self) -> bool {
        if self.config.invariant_check_interval == 0 {
            return false;
        }
        let count = self.operations_count.load(Ordering::Relaxed);
        count % self.config.invariant_check_interval == 0
    }

    /// Run a single-threaded test with the given operation generator.
    ///
    /// The generator receives the current step and returns an operation.
    pub fn run_single_threaded<F, T, R>(&mut self, mut generate_op: F, mut execute: R) -> HarnessResult
    where
        F: FnMut(&mut DstEnv, u64) -> Option<T>,
        R: FnMut(&mut DstEnv, T) -> Result<(), String>,
    {
        let total_ops = self.config.operations_per_thread;
        let mut step = 0u64;

        while step < total_ops && !self.is_stopped() {
            // Generate operation
            if let Some(op) = generate_op(&mut self.env, step) {
                // Execute operation
                if let Err(e) = execute(&mut self.env, op) {
                    self.stop_with_violation(e);
                    break;
                }
                self.record_operation();
            }

            // Maybe inject fault
            self.env.maybe_delay();

            step += 1;
        }

        self.build_result()
    }

    /// Run a simulated concurrent test.
    ///
    /// Operations are interleaved according to the scheduler.
    pub fn run_concurrent<F, T, R, I>(
        &mut self,
        mut generate_op: F,
        mut execute: R,
        mut check_invariants: I,
    ) -> HarnessResult
    where
        F: FnMut(&mut DstEnv, usize, u64) -> Option<T>,
        R: FnMut(&mut DstEnv, usize, T) -> Result<(), String>,
        I: FnMut() -> Result<(), String>,
    {
        let threads_count = self.config.threads_count;
        let ops_per_thread = self.config.operations_per_thread;

        // Track progress per thread
        let mut thread_steps: Vec<u64> = vec![0; threads_count];

        while !self.is_stopped() {
            // Get current thread from scheduler
            let current = self.current_thread().unwrap_or(0);

            // Check if this thread is done
            if thread_steps[current] >= ops_per_thread {
                // Check if all threads are done
                if thread_steps.iter().all(|&s| s >= ops_per_thread) {
                    break;
                }
                // Force switch to another thread
                if let Some(scheduler) = self.env.scheduler() {
                    scheduler.force_switch();
                }
                continue;
            }

            // Generate operation for this thread
            if let Some(op) = generate_op(&mut self.env, current, thread_steps[current]) {
                // Execute operation
                if let Err(e) = execute(&mut self.env, current, op) {
                    self.stop_with_violation(format!("Thread {}: {}", current, e));
                    break;
                }
                self.record_operation();
            }

            thread_steps[current] += 1;

            // Maybe check invariants
            if self.should_check_invariants() {
                self.record_invariant_check();
                if let Err(e) = check_invariants() {
                    self.stop_with_violation(e);
                    break;
                }
            }

            // Yield point
            self.yield_point();
        }

        // Final invariant check
        if !self.is_stopped() {
            self.record_invariant_check();
            if let Err(e) = check_invariants() {
                self.stop_with_violation(e);
            }
        }

        self.build_result()
    }

    fn build_result(&mut self) -> HarnessResult {
        let violation = self.violation.lock().unwrap().clone();

        HarnessResult {
            seed: self.env.seed(),
            operations_count: self.operations_count.load(Ordering::Relaxed),
            context_switches_count: self.context_switches_count.load(Ordering::Relaxed),
            faults_injected_count: self.env.fault().stats().faults_count,
            invariant_checks_count: self.invariant_checks_count.load(Ordering::Relaxed),
            all_invariants_held: violation.is_none(),
            first_violation: violation,
        }
    }
}

impl HarnessResult {
    /// Format for display.
    pub fn format(&self) -> String {
        let status = if self.all_invariants_held {
            "PASS"
        } else {
            "FAIL"
        };

        let mut result = format!(
            "[{}] DST_SEED={} ops={} switches={} faults={} checks={}",
            status,
            self.seed,
            self.operations_count,
            self.context_switches_count,
            self.faults_injected_count,
            self.invariant_checks_count
        );

        if let Some(ref violation) = self.first_violation {
            result.push_str(&format!("\n  Violation: {}", violation));
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_harness_single_threaded() {
        let mut harness = DstHarness::new(12345, HarnessConfig::quick());

        let mut counter = 0u64;

        let result = harness.run_single_threaded(
            |_env, step| {
                if step < 10 {
                    Some(step)
                } else {
                    None
                }
            },
            |_env, op| {
                counter += op;
                Ok(())
            },
        );

        assert!(result.all_invariants_held);
        assert_eq!(counter, 45); // 0+1+2+...+9
    }

    #[test]
    fn test_harness_stops_on_violation() {
        let config = HarnessConfig {
            operations_per_thread: 100,
            ..HarnessConfig::quick()
        };
        let mut harness = DstHarness::new(12345, config);

        let result = harness.run_single_threaded(
            |_env, step| Some(step),
            |_env, op| {
                if op == 5 {
                    Err("Intentional failure at step 5".to_string())
                } else {
                    Ok(())
                }
            },
        );

        assert!(!result.all_invariants_held);
        assert!(result.first_violation.is_some());
        assert!(result.operations_count < 100);
    }

    #[test]
    fn test_harness_concurrent() {
        let config = HarnessConfig {
            threads_count: 2,
            operations_per_thread: 10,
            yield_probability: 0.5,
            invariant_check_interval: 5,
            ..HarnessConfig::quick()
        };
        let mut harness = DstHarness::new(12345, config);

        let mut thread_counters = vec![0u64; 2];

        let result = harness.run_concurrent(
            |_env, _thread, step| Some(step),
            |_env, thread, _op| {
                thread_counters[thread] += 1;
                Ok(())
            },
            || Ok(()),
        );

        assert!(result.all_invariants_held);
        assert_eq!(thread_counters[0], 10);
        assert_eq!(thread_counters[1], 10);
        assert!(result.context_switches_count > 0);
    }
}
