//! Evaluator cascade orchestration.
//!
//! Runs evaluators in order, stopping at the first failure.

use std::path::Path;
use std::time::Duration;

use crate::level0_rustc;
use crate::level1_miri;
use crate::level2_loom;
use crate::level3_dst;
use crate::level4_stateright;
use crate::level5_kani;
use crate::level6_verus;
use crate::result::{CascadeResult, EvaluatorResult};

/// Evaluator levels in the cascade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum EvaluatorLevel {
    /// Level 0: rustc - type checking, lifetime analysis
    Rustc = 0,
    /// Level 1: miri - undefined behavior detection
    Miri = 1,
    /// Level 2: loom - thread interleaving exploration
    Loom = 2,
    /// Level 3: DST - deterministic simulation testing
    Dst = 3,
    /// Level 4: stateright - model checking against TLA+ spec
    Stateright = 4,
    /// Level 5: kani - bounded model checking / proof
    Kani = 5,
    /// Level 6: verus - SMT-based theorem proving (proofs for ALL inputs)
    Verus = 6,
}

impl EvaluatorLevel {
    /// Get all levels up to and including this one.
    pub fn levels_up_to(self) -> Vec<EvaluatorLevel> {
        let max = self as u8;
        (0..=max)
            .map(|i| match i {
                0 => EvaluatorLevel::Rustc,
                1 => EvaluatorLevel::Miri,
                2 => EvaluatorLevel::Loom,
                3 => EvaluatorLevel::Dst,
                4 => EvaluatorLevel::Stateright,
                5 => EvaluatorLevel::Kani,
                6 => EvaluatorLevel::Verus,
                _ => unreachable!(),
            })
            .collect()
    }

    /// Get the name of this level.
    pub fn name(&self) -> &'static str {
        match self {
            EvaluatorLevel::Rustc => "rustc",
            EvaluatorLevel::Miri => "miri",
            EvaluatorLevel::Loom => "loom",
            EvaluatorLevel::Dst => "DST",
            EvaluatorLevel::Stateright => "stateright",
            EvaluatorLevel::Kani => "kani",
            EvaluatorLevel::Verus => "verus",
        }
    }
}

/// Configuration for the cascade.
#[derive(Debug, Clone)]
pub struct CascadeConfig {
    /// Maximum level to run (inclusive)
    pub max_level: EvaluatorLevel,
    /// Stop on first failure
    pub fail_fast: bool,
    /// Timeout per evaluator
    pub timeout: Duration,
    /// Loom preemption bound (higher = more thorough, slower)
    pub loom_preemption_bound: usize,
    /// Stateright max depth
    pub stateright_depth_max: usize,
    /// Kani unwind bound
    pub kani_unwind: usize,
    /// DST seed (if None, generates random)
    pub dst_seed: Option<u64>,
    /// Number of DST iterations
    pub dst_iterations: u64,
    /// Verus thread count
    pub verus_threads: usize,
}

impl Default for CascadeConfig {
    fn default() -> Self {
        Self {
            max_level: EvaluatorLevel::Dst, // Default to DST (fast, thorough)
            fail_fast: true,
            timeout: Duration::from_secs(300), // 5 minutes
            loom_preemption_bound: 3,
            stateright_depth_max: 100,
            kani_unwind: 10,
            dst_seed: None,
            dst_iterations: 1000,
            verus_threads: num_cpus::get(),
        }
    }
}

impl CascadeConfig {
    /// Fast config for quick iteration.
    pub fn fast() -> Self {
        Self {
            max_level: EvaluatorLevel::Miri,
            timeout: Duration::from_secs(30),
            dst_iterations: 100,
            ..Default::default()
        }
    }

    /// Thorough config for CI.
    pub fn thorough() -> Self {
        Self {
            max_level: EvaluatorLevel::Stateright,
            timeout: Duration::from_secs(600),
            loom_preemption_bound: 4,
            stateright_depth_max: 200,
            dst_iterations: 10000,
            ..Default::default()
        }
    }

    /// Maximum verification (includes Kani).
    pub fn maximum() -> Self {
        Self {
            max_level: EvaluatorLevel::Kani,
            timeout: Duration::from_secs(1800), // 30 minutes
            loom_preemption_bound: 5,
            stateright_depth_max: 500,
            kani_unwind: 20,
            dst_iterations: 100000,
            ..Default::default()
        }
    }

    /// Formal verification (includes Verus SMT proofs).
    ///
    /// This goes beyond bounded model checking to prove properties
    /// for ALL inputs, not just explored states.
    pub fn formal() -> Self {
        Self {
            max_level: EvaluatorLevel::Verus,
            timeout: Duration::from_secs(3600), // 1 hour
            loom_preemption_bound: 5,
            stateright_depth_max: 500,
            kani_unwind: 20,
            dst_iterations: 100000,
            verus_threads: num_cpus::get(),
            ..Default::default()
        }
    }
}

/// The evaluator cascade.
///
/// Runs evaluators in order from fastest to slowest, stopping
/// at the first failure if configured to do so.
pub struct EvaluatorCascade {
    config: CascadeConfig,
}

impl EvaluatorCascade {
    /// Create a new cascade with the given config.
    pub fn new(config: CascadeConfig) -> Self {
        Self { config }
    }

    /// Create with default config.
    pub fn with_defaults() -> Self {
        Self::new(CascadeConfig::default())
    }

    /// Run the cascade on a crate.
    ///
    /// # Arguments
    /// - `crate_path`: Path to the crate directory (containing Cargo.toml)
    pub async fn run(&self, crate_path: &Path) -> CascadeResult {
        let mut results = Vec::new();
        let levels = self.config.max_level.levels_up_to();

        for level in levels {
            let result = match level {
                EvaluatorLevel::Rustc => level0_rustc::run(crate_path, self.config.timeout).await,
                EvaluatorLevel::Miri => level1_miri::run(crate_path, self.config.timeout).await,
                EvaluatorLevel::Loom => {
                    level2_loom::run(crate_path, self.config.timeout, self.config.loom_preemption_bound).await
                }
                EvaluatorLevel::Dst => {
                    level3_dst::run(
                        crate_path,
                        self.config.timeout,
                        self.config.dst_seed,
                        self.config.dst_iterations,
                    )
                    .await
                }
                EvaluatorLevel::Stateright => {
                    level4_stateright::run(
                        crate_path,
                        self.config.timeout,
                        self.config.stateright_depth_max,
                    )
                    .await
                }
                EvaluatorLevel::Kani => {
                    level5_kani::run(crate_path, self.config.timeout, self.config.kani_unwind).await
                }
                EvaluatorLevel::Verus => {
                    let verus_config = level6_verus::VerusConfig {
                        timeout: self.config.timeout,
                        threads: self.config.verus_threads,
                        ..Default::default()
                    };
                    // Look for verus proof files (*.verus.rs or verify.rs)
                    let verus_file = crate_path.join("src").join("verify.rs");
                    if verus_file.exists() {
                        level6_verus::run(&verus_file, &verus_config).await
                    } else {
                        // Skip if no verus file present
                        EvaluatorResult::skip(
                            "verus",
                            "No Verus proof file found (src/verify.rs)",
                            Duration::ZERO,
                        )
                    }
                }
            };

            let failed = !result.passed;
            results.push(result);

            if failed && self.config.fail_fast {
                break;
            }
        }

        CascadeResult::from_results(results)
    }

    /// Run the cascade on source code directly (for generated code).
    ///
    /// Creates a temporary crate and runs the cascade on it.
    pub async fn run_on_code(&self, code: &str, test_code: &str) -> CascadeResult {
        // Create temporary directory with Cargo.toml and source
        let temp_dir = std::env::temp_dir().join(format!("vf-cascade-{}", rand::random::<u64>()));
        let src_dir = temp_dir.join("src");

        // Create directory structure
        if let Err(e) = tokio::fs::create_dir_all(&src_dir).await {
            return CascadeResult::from_results(vec![EvaluatorResult::fail(
                "setup",
                format!("Failed to create temp directory: {}", e),
                Duration::ZERO,
                String::new(),
            )]);
        }

        // Write Cargo.toml
        let cargo_toml = r#"
[package]
name = "vf-temp-crate"
version = "0.1.0"
edition = "2021"

[dependencies]
crossbeam-epoch = "0.9"

[dev-dependencies]
loom = "0.7"

[features]
default = []
"#;

        if let Err(e) = tokio::fs::write(temp_dir.join("Cargo.toml"), cargo_toml).await {
            return CascadeResult::from_results(vec![EvaluatorResult::fail(
                "setup",
                format!("Failed to write Cargo.toml: {}", e),
                Duration::ZERO,
                String::new(),
            )]);
        }

        // Write lib.rs - strip any existing tests module from generated code to avoid duplicates
        let code_without_tests = strip_tests_module(code);
        let lib_content = format!("{}\n\n#[cfg(test)]\nmod tests {{\n    use super::*;\n{}\n}}", code_without_tests, test_code);
        if let Err(e) = tokio::fs::write(src_dir.join("lib.rs"), lib_content).await {
            return CascadeResult::from_results(vec![EvaluatorResult::fail(
                "setup",
                format!("Failed to write lib.rs: {}", e),
                Duration::ZERO,
                String::new(),
            )]);
        }

        // Run cascade
        let result = self.run(&temp_dir).await;

        // Cleanup
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;

        result
    }

    /// Get the current config.
    pub fn config(&self) -> &CascadeConfig {
        &self.config
    }
}

/// Strip any existing #[cfg(test)] mod tests { ... } block from generated code.
///
/// This prevents duplicate module errors when the cascade adds its own test module.
/// Uses simple heuristics that work for well-formatted Rust code.
fn strip_tests_module(code: &str) -> String {
    // Look for #[cfg(test)] followed by mod tests
    let lines: Vec<&str> = code.lines().collect();
    let mut result = Vec::new();
    let mut in_tests_module = false;
    let mut brace_depth = 0;
    let mut skip_next_mod = false;

    for line in lines {
        let trimmed = line.trim();

        // Detect #[cfg(test)] attribute
        if trimmed == "#[cfg(test)]" {
            skip_next_mod = true;
            continue;
        }

        // Detect mod tests { following #[cfg(test)]
        if skip_next_mod && trimmed.starts_with("mod tests") {
            in_tests_module = true;
            brace_depth = 0;
            // Count opening braces on this line
            brace_depth += line.chars().filter(|&c| c == '{').count() as i32;
            brace_depth -= line.chars().filter(|&c| c == '}').count() as i32;
            skip_next_mod = false;
            continue;
        }

        skip_next_mod = false;

        // If in tests module, track brace depth
        if in_tests_module {
            brace_depth += line.chars().filter(|&c| c == '{').count() as i32;
            brace_depth -= line.chars().filter(|&c| c == '}').count() as i32;
            if brace_depth <= 0 {
                in_tests_module = false;
            }
            continue;
        }

        result.push(line);
    }

    result.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_levels_up_to() {
        let levels = EvaluatorLevel::Loom.levels_up_to();
        assert_eq!(levels.len(), 3);
        assert_eq!(levels[0], EvaluatorLevel::Rustc);
        assert_eq!(levels[1], EvaluatorLevel::Miri);
        assert_eq!(levels[2], EvaluatorLevel::Loom);

        // Test all levels including Verus
        let all_levels = EvaluatorLevel::Verus.levels_up_to();
        assert_eq!(all_levels.len(), 7);
        assert_eq!(all_levels[6], EvaluatorLevel::Verus);
    }

    #[test]
    fn test_config_presets() {
        let fast = CascadeConfig::fast();
        assert_eq!(fast.max_level, EvaluatorLevel::Miri);

        let thorough = CascadeConfig::thorough();
        assert_eq!(thorough.max_level, EvaluatorLevel::Stateright);

        let max = CascadeConfig::maximum();
        assert_eq!(max.max_level, EvaluatorLevel::Kani);

        let formal = CascadeConfig::formal();
        assert_eq!(formal.max_level, EvaluatorLevel::Verus);
    }

    #[test]
    fn test_level_names() {
        assert_eq!(EvaluatorLevel::Rustc.name(), "rustc");
        assert_eq!(EvaluatorLevel::Miri.name(), "miri");
        assert_eq!(EvaluatorLevel::Loom.name(), "loom");
        assert_eq!(EvaluatorLevel::Dst.name(), "DST");
        assert_eq!(EvaluatorLevel::Stateright.name(), "stateright");
        assert_eq!(EvaluatorLevel::Kani.name(), "kani");
        assert_eq!(EvaluatorLevel::Verus.name(), "verus");
    }

    #[test]
    fn test_strip_tests_module() {
        let code = r#"
pub struct Foo {
    value: u32,
}

impl Foo {
    pub fn new() -> Self {
        Self { value: 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_foo() {
        let f = Foo::new();
        assert_eq!(f.value, 0);
    }
}
"#;
        let stripped = strip_tests_module(code);
        assert!(!stripped.contains("mod tests"));
        assert!(!stripped.contains("#[cfg(test)]"));
        assert!(stripped.contains("pub struct Foo"));
        assert!(stripped.contains("pub fn new()"));
    }

    #[test]
    fn test_strip_tests_module_no_tests() {
        let code = r#"
pub struct Bar {
    x: i32,
}
"#;
        let stripped = strip_tests_module(code);
        assert!(stripped.contains("pub struct Bar"));
    }
}
