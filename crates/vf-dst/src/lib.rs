//! # vf-dst
//!
//! Deterministic Simulation Testing framework for verified concurrent structures.
//!
//! Inspired by FoundationDB and TigerBeetle, this crate provides deterministic
//! simulation of time, randomness, and faults. All behavior is reproducible
//! via a seed.
//!
//! ## Harnesses
//!
//! - `fault_injection`: Lock-free structures (Treiber Stack)
//! - `ssi_harness`: Lock-based protocols (SSI transactions)
//!
//! ## Usage
//!
//! ```rust
//! use vf_dst::DstEnv;
//!
//! let seed = 12345;
//! let mut env = DstEnv::new(seed);
//!
//! // Deterministic time
//! let now = env.clock().now_ns();
//! env.clock().advance_ns(1_000_000); // 1ms
//!
//! // Deterministic randomness
//! let value: u64 = env.rng().gen();
//! let choice = env.rng().gen_range(0..10);
//!
//! // Deterministic fault injection
//! if env.fault().should_fail() {
//!     // Simulate failure
//! }
//! ```
//!
//! ## Reproducibility
//!
//! To reproduce a failing test:
//! ```bash
//! DST_SEED=12345 cargo test
//! ```

pub mod clock;
pub mod env;
pub mod fault;
pub mod fault_injection;
pub mod harness;
pub mod loom_oracle;
pub mod oracle_scheduler;
pub mod random;
pub mod scheduler;
pub mod ssi_harness;
pub mod ssi_oracle;

// Deprecated - violates "code is disposable" principle
#[doc(hidden)]
pub mod instrumented;

pub use clock::SimClock;
pub use env::DstEnv;
pub use fault::{FaultConfig, FaultInjector};
pub use fault_injection::{DstRunner, DstTestableStack, DstOp, FaultPoint, FaultType, run_dst_scenario};
pub use harness::{DstHarness, HarnessConfig, HarnessResult};
pub use loom_oracle::{LoomScenario, LoomOp, LoomTestableStack, generate_loom_test};
pub use oracle_scheduler::{OracleScheduler, OracleSchedulerStats, OracleTrace};
pub use random::DeterministicRng;
pub use scheduler::{ScheduleDecision, Scheduler};
pub use ssi_harness::{DstTestableSsi, SsiDstRunner, SsiDstStats, SsiFaultPoint, SsiFaultType, DstSsiOp, run_ssi_scenario};
pub use ssi_oracle::{SsiOracleTrace, SsiOracleAction, SsiExpectedOutcome, SsiOracleReplayResult, replay_ssi_oracle, run_all_ssi_oracles};

/// Get DST seed from environment or generate random one.
///
/// Prints the seed for reproduction. Use `DST_SEED=<seed>` to reproduce.
#[must_use]
pub fn get_or_generate_seed() -> u64 {
    match std::env::var("DST_SEED") {
        Ok(s) => {
            let seed: u64 = s.parse().expect("DST_SEED must be a valid u64");
            println!("DST_SEED={} (from environment)", seed);
            seed
        }
        Err(_) => {
            let seed = rand::random::<u64>();
            println!("DST_SEED={} (randomly generated)", seed);
            seed
        }
    }
}
