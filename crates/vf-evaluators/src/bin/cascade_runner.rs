//! vf-cascade-runner — CLI for running the evaluator cascade on evolved code.
//!
//! Used by ShinkaEvolve's evaluate.py to score candidate implementations.
//!
//! # Usage
//!
//! ```bash
//! vf-cascade-runner --code evolved.rs --trait-spec trait_spec.rs --max-level dst
//! ```
//!
//! Outputs JSON to stdout with cascade results and progress guarantee analysis.

use std::path::PathBuf;
use std::process;
use std::time::Duration;

use clap::Parser;
use serde_json::json;

use vf_evaluators::{CascadeConfig, EvaluatorCascade, EvaluatorLevel};

/// Maximum timeout per evaluator level (seconds).
const TIMEOUT_SECONDS_MAX: u64 = 3600;

/// Default timeout per evaluator level (seconds).
const TIMEOUT_SECONDS_DEFAULT: u64 = 300;

/// Run the evaluator cascade on evolved code and output JSON results.
#[derive(Parser, Debug)]
#[command(name = "vf-cascade-runner")]
#[command(about = "Evaluator cascade runner for ShinkaEvolve")]
struct Cli {
    /// Path to the evolved .rs implementation file.
    #[arg(long)]
    code: PathBuf,

    /// Path to the fixed trait_spec.rs (test harness).
    #[arg(long)]
    trait_spec: PathBuf,

    /// Maximum evaluator level to run.
    #[arg(long, default_value = "dst")]
    max_level: String,

    /// Starting evaluator level (skip levels below this).
    ///
    /// Used for adaptive cascade: if a checkpoint knows code passes
    /// miri, start from loom to save time.
    #[arg(long, default_value = "rustc")]
    start_level: String,

    /// Timeout per evaluator in seconds.
    #[arg(long, default_value_t = TIMEOUT_SECONDS_DEFAULT)]
    timeout: u64,

    /// DST seed (random if not set).
    #[arg(long)]
    dst_seed: Option<u64>,

    /// Number of DST iterations.
    #[arg(long, default_value_t = 1000)]
    dst_iterations: u64,

    /// Include throughput benchmark in output.
    ///
    /// Runs a quick benchmark after cascade passes and includes
    /// ops_per_sec in the JSON output.
    #[arg(long)]
    benchmark: bool,
}

fn parse_level(s: &str) -> Result<EvaluatorLevel, String> {
    match s.to_lowercase().as_str() {
        "rustc" => Ok(EvaluatorLevel::Rustc),
        "miri" => Ok(EvaluatorLevel::Miri),
        "loom" => Ok(EvaluatorLevel::Loom),
        "dst" => Ok(EvaluatorLevel::Dst),
        "stateright" => Ok(EvaluatorLevel::Stateright),
        "kani" => Ok(EvaluatorLevel::Kani),
        "verus" => Ok(EvaluatorLevel::Verus),
        _ => Err(format!("Unknown level: {s}. Expected: rustc, miri, loom, dst, stateright, kani, verus")),
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Parse max level
    let max_level = match parse_level(&cli.max_level) {
        Ok(level) => level,
        Err(e) => {
            eprintln!("Error: {e}");
            process::exit(1);
        }
    };

    // Parse start level (for adaptive cascade)
    let start_level = match parse_level(&cli.start_level) {
        Ok(level) => level,
        Err(e) => {
            eprintln!("Error: {e}");
            process::exit(1);
        }
    };

    // Validate timeout
    debug_assert!(cli.timeout > 0, "Timeout must be positive");
    let timeout_secs = cli.timeout.min(TIMEOUT_SECONDS_MAX);

    // Read evolved code
    let code = match std::fs::read_to_string(&cli.code) {
        Ok(c) => c,
        Err(e) => {
            let result = json!({
                "level_reached": -1,
                "all_passed": false,
                "results": [],
                "invariants_passed": 0,
                "invariants_total": 0,
                "progress_guarantee": "Unknown",
                "text_feedback": format!("Failed to read code file {}: {}", cli.code.display(), e),
            });
            println!("{}", serde_json::to_string_pretty(&result).unwrap());
            process::exit(0);
        }
    };

    // Read trait spec (test harness)
    let test_code = match std::fs::read_to_string(&cli.trait_spec) {
        Ok(c) => c,
        Err(e) => {
            let result = json!({
                "level_reached": -1,
                "all_passed": false,
                "results": [],
                "invariants_passed": 0,
                "invariants_total": 0,
                "progress_guarantee": "Unknown",
                "text_feedback": format!("Failed to read trait spec {}: {}", cli.trait_spec.display(), e),
            });
            println!("{}", serde_json::to_string_pretty(&result).unwrap());
            process::exit(0);
        }
    };

    // Configure cascade
    let config = CascadeConfig {
        max_level,
        fail_fast: true,
        timeout: Duration::from_secs(timeout_secs),
        dst_seed: cli.dst_seed,
        dst_iterations: cli.dst_iterations,
        ..CascadeConfig::default()
    };

    let cascade = EvaluatorCascade::new(config);

    // Run cascade (with adaptive start level)
    let cascade_result = if start_level > EvaluatorLevel::Rustc {
        // Skip levels below start_level
        eprintln!("Adaptive cascade: skipping to {}", start_level.name());
        cascade.run_on_code_from_level(&code, &test_code, start_level).await
    } else {
        cascade.run_on_code(&code, &test_code).await
    };

    // Analyze progress guarantee
    let progress = vf_perf::analyze_progress_guarantee(&code);

    // Compute level reached (highest passing level)
    let level_reached: i64 = cascade_result
        .results
        .iter()
        .enumerate()
        .rev()
        .find(|(_, r)| r.passed)
        .map(|(i, _)| i as i64)
        .unwrap_or(-1);

    // Count invariant-style checks passed
    let invariants_passed = cascade_result.results.iter().filter(|r| r.passed).count();
    let invariants_total = cascade_result.results.len();

    // Build text feedback
    let mut feedback_lines: Vec<String> = Vec::new();
    for r in &cascade_result.results {
        feedback_lines.push(r.format_status());
    }
    if let Some(ref failure) = cascade_result.first_failure {
        if let Some(ref error) = failure.error {
            feedback_lines.push(format!("\nFirst failure at {}: {}", failure.evaluator, error));
        }
        if let Some(ref ce) = failure.counterexample {
            feedback_lines.push(format!("\nCounterexample:\n{}", ce.render_diagram()));
        }
    }

    // Build per-result JSON
    let results_json: Vec<serde_json::Value> = cascade_result
        .results
        .iter()
        .map(|r| {
            let mut entry = json!({
                "evaluator": r.evaluator,
                "passed": r.passed,
                "duration_ms": r.duration.as_millis() as u64,
            });
            if let Some(ref error) = r.error {
                entry["error"] = json!(error);
            }
            if let Some(ref ce) = r.counterexample {
                entry["counterexample"] = json!(ce.render_diagram());
            }
            entry
        })
        .collect();

    let progress_name = match progress {
        vf_perf::ProgressGuarantee::Blocking => "Blocking",
        vf_perf::ProgressGuarantee::ObstructionFree => "ObstructionFree",
        vf_perf::ProgressGuarantee::LockFree => "LockFree",
        vf_perf::ProgressGuarantee::WaitFree => "WaitFree",
    };

    // Extract counterexample traces as structured data for oracle accumulation
    let mut counterexample_traces: Vec<serde_json::Value> = Vec::new();
    for r in &cascade_result.results {
        if !r.passed {
            if let Some(ref ce) = r.counterexample {
                counterexample_traces.push(json!({
                    "evaluator": r.evaluator,
                    "invariant": ce.description.as_deref().unwrap_or("unknown"),
                    "actions": ce.interleaving.iter().map(|a| json!({
                        "thread": a.thread_id,
                        "action": a.action,
                        "value": null,
                    })).collect::<Vec<_>>(),
                    "description": ce.render_diagram(),
                }));
            }
        }
    }

    let mut output = json!({
        "level_reached": level_reached,
        "all_passed": cascade_result.all_passed,
        "results": results_json,
        "invariants_passed": invariants_passed,
        "invariants_total": invariants_total,
        "progress_guarantee": progress_name,
        "text_feedback": feedback_lines.join("\n"),
        "start_level": cli.start_level,
    });

    // Include structured counterexample traces for oracle accumulation
    if !counterexample_traces.is_empty() {
        output["counterexample_traces"] = json!(counterexample_traces);
    }

    println!("{}", serde_json::to_string_pretty(&output).unwrap());
}
