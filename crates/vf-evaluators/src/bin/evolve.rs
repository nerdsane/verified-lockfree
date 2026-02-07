//! vf-evolve — Rust-native evolutionary code synthesis engine.
//!
//! ShinkaEvolve-inspired design: island model with multi-model LLM ensemble,
//! power-law selection, verification cascade scoring.
//!
//! Each island is assigned a different LLM model. Models that produce better
//! candidates get higher selection weight (bandit-style adaptation).
//!
//! # Usage
//!
//! ```bash
//! vf-evolve --task treiber_stack --generations 50
//! vf-evolve --task treiber_stack --generations 5 --islands 3
//! ```

use std::path::{Path, PathBuf};
use std::process;
use std::time::{Duration, Instant};

use clap::Parser;
use rand::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

use vf_evaluators::{CascadeConfig, EvaluatorCascade, EvaluatorLevel};

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

/// Evolutionary code synthesis with formal verification cascade.
#[derive(Parser, Debug)]
#[command(name = "vf-evolve")]
#[command(about = "Evolve concurrent Rust code guided by verification cascade")]
struct Cli {
    /// Task name (maps to tools/vf-evolve/tasks/<name>/).
    #[arg(long)]
    task: String,

    /// Number of generations to evolve.
    #[arg(long, default_value_t = 50)]
    generations: u64,

    /// Number of islands (independent populations). Each island uses a
    /// different LLM model from the ensemble for diversity.
    /// Default 4 maps to: opus, sonnet, haiku, sonnet-hot.
    #[arg(long, default_value_t = 4)]
    islands: usize,

    /// Population size per island.
    #[arg(long, default_value_t = 10)]
    population: usize,

    /// Migration interval (generations between migrations).
    #[arg(long, default_value_t = 5)]
    migration_interval: u64,

    /// Maximum cascade level to evaluate.
    #[arg(long, default_value = "dst")]
    max_level: String,

    /// Cascade timeout per level in seconds.
    #[arg(long, default_value_t = 300)]
    timeout: u64,

    /// LLM temperature for mutation.
    #[arg(long, default_value_t = 0.8)]
    temperature: f64,

    /// Output directory.
    #[arg(long, default_value = "/tmp/vf-evolve-output")]
    output_dir: String,

    /// Project root (for finding tasks/).
    #[arg(long)]
    project_root: Option<String>,

    /// Enable stepping-stone prompts that adapt guidance to score band.
    #[arg(long, default_value_t = false)]
    stepping_stones: bool,

    /// Override seed file (default: initial.rs in task directory).
    #[arg(long)]
    seed: Option<String>,

    /// Task category: lockfree (cascade), distributed (cascade), adrs (simulator).
    #[arg(long, default_value = "lockfree")]
    category: String,

    /// Inject reference patterns into the system prompt.
    #[arg(long, default_value_t = false)]
    inject_patterns: bool,
}

// ---------------------------------------------------------------------------
// Multi-model ensemble
// ---------------------------------------------------------------------------

/// An LLM model in the ensemble with bandit-style scoring.
#[derive(Debug, Clone)]
struct LlmModel {
    name: String,
    /// Display label for logging.
    label: String,
    /// Temperature override (None = use default).
    temperature: Option<f64>,
    /// Bandit: total score produced by this model's candidates.
    total_score: f64,
    /// Bandit: number of evaluations from this model.
    eval_count: u64,
    /// Bandit: best score ever from this model.
    best_score: f64,
}

impl LlmModel {
    fn new(name: &str, label: &str, temperature: Option<f64>) -> Self {
        Self {
            name: name.to_string(),
            label: label.to_string(),
            temperature,
            total_score: 0.0,
            eval_count: 0,
            best_score: 0.0,
        }
    }

    /// Average score for bandit selection.
    fn avg_score(&self) -> f64 {
        if self.eval_count == 0 {
            f64::MAX // Optimistic initialization: untried models get priority
        } else {
            self.total_score / self.eval_count as f64
        }
    }

    /// UCB1 score for bandit selection.
    fn ucb1(&self, total_evals: u64) -> f64 {
        if self.eval_count == 0 {
            return f64::MAX;
        }
        let exploitation = self.total_score / self.eval_count as f64;
        let exploration = (2.0 * (total_evals as f64).ln() / self.eval_count as f64).sqrt();
        exploitation + exploration * 100.0 // Scale exploration to score range
    }

    fn record(&mut self, score: f64) {
        self.total_score += score;
        self.eval_count += 1;
        if score > self.best_score {
            self.best_score = score;
        }
    }
}

/// Build the default model ensemble.
fn default_ensemble() -> Vec<LlmModel> {
    vec![
        LlmModel::new(
            "claude-opus-4-20250514",
            "opus",
            Some(0.7), // Opus: highest quality, lower temp for precision
        ),
        LlmModel::new(
            "claude-sonnet-4-20250514",
            "sonnet",
            None, // Sonnet: fast + capable, default temp
        ),
        LlmModel::new(
            "claude-haiku-4-5-20251001",
            "haiku",
            Some(1.0), // Haiku: cheapest, high temp for maximum diversity
        ),
        LlmModel::new(
            "claude-sonnet-4-20250514",
            "sonnet-hot",
            Some(1.0), // Sonnet with high temp for exploration
        ),
    ]
}

/// Select which model to use for this island/generation.
/// Round-robin by island index, with bandit override for exploitation.
fn select_model<'a>(
    ensemble: &'a [LlmModel],
    island_idx: usize,
    generation: u64,
    total_evals: u64,
    rng: &mut impl Rng,
) -> usize {
    debug_assert!(!ensemble.is_empty());

    // First few generations: round-robin to ensure all models get tried
    if generation <= ensemble.len() as u64 {
        return island_idx % ensemble.len();
    }

    // After warmup: 70% exploit (UCB1 best), 30% explore (round-robin)
    if rng.gen::<f64>() < 0.3 {
        island_idx % ensemble.len()
    } else {
        // UCB1 selection
        ensemble
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.ucb1(total_evals).partial_cmp(&b.ucb1(total_evals)).unwrap())
            .map(|(i, _)| i)
            .unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Candidate {
    id: String,
    code: String,
    score: f64,
    correct: bool,
    generation: u64,
    parent_id: Option<String>,
    island: usize,
    level_reached: i64,
    progress: String,
    feedback: String,
    /// Which model produced this candidate.
    model: String,
}

impl Candidate {
    fn new_from_code(code: String, generation: u64, island: usize, model: &str) -> Self {
        let hash = fnv_hash(&code);
        let id = format!("{hash:016x}");
        Self {
            id,
            code,
            score: 0.0,
            correct: false,
            generation,
            parent_id: None,
            island,
            level_reached: -1,
            progress: "Unknown".to_string(),
            feedback: String::new(),
            model: model.to_string(),
        }
    }
}

#[derive(Debug, Serialize)]
struct GenerationLog {
    generation: u64,
    best_score: f64,
    best_progress: String,
    best_model: String,
    island_bests: Vec<f64>,
    evaluations: u64,
    duration_secs: f64,
    model_stats: Vec<ModelStat>,
}

#[derive(Debug, Clone, Serialize)]
struct ModelStat {
    label: String,
    evals: u64,
    avg_score: f64,
    best_score: f64,
}

// ---------------------------------------------------------------------------
// LLM client
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<ContentBlock>,
}

#[derive(Deserialize)]
struct ContentBlock {
    text: Option<String>,
}

async fn call_anthropic(
    client: &reqwest::Client,
    api_key: &str,
    model: &str,
    system: &str,
    prompt: &str,
    temperature: f64,
    max_tokens: u64,
) -> Result<String, String> {
    let body = json!({
        "model": model,
        "max_tokens": max_tokens,
        "temperature": temperature,
        "system": system,
        "messages": [{"role": "user", "content": prompt}],
    });

    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("content-type", "application/json")
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("HTTP error: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("API error {status}: {}", &text[..text.len().min(500)]));
    }

    let data: AnthropicResponse = resp
        .json()
        .await
        .map_err(|e| format!("Parse error: {e}"))?;

    data.content
        .first()
        .and_then(|b| b.text.clone())
        .ok_or_else(|| "Empty response".to_string())
}

fn extract_rust_code(response: &str) -> String {
    if let Some(start) = response.find("```rust") {
        let code_start = start + "```rust".len();
        if let Some(end) = response[code_start..].find("```") {
            return response[code_start..code_start + end].trim().to_string();
        }
    }
    if let Some(start) = response.find("```") {
        let after_ticks = start + 3;
        let code_start = response[after_ticks..]
            .find('\n')
            .map(|i| after_ticks + i + 1)
            .unwrap_or(after_ticks);
        if let Some(end) = response[code_start..].find("```") {
            return response[code_start..code_start + end].trim().to_string();
        }
    }
    response.trim().to_string()
}

// ---------------------------------------------------------------------------
// Evolution engine
// ---------------------------------------------------------------------------

/// Structured feedback from ALL cascade levels, not just first failure.
fn build_structured_feedback(result: &vf_evaluators::CascadeResult) -> String {
    const EXCERPT_LIMIT: usize = 2000;
    let mut feedback = String::with_capacity(4096);

    feedback.push_str("## Cascade Results:\n");
    for r in &result.results {
        let status = if r.passed { "PASS" } else { "FAIL" };
        feedback.push_str(&format!(
            "- {}: {} ({:.1}s)",
            r.evaluator,
            status,
            r.duration.as_secs_f64()
        ));

        if !r.passed {
            if let Some(ref error) = r.error {
                let excerpt = &error[..error.len().min(EXCERPT_LIMIT)];
                feedback.push_str(&format!(" — error: {:?}", excerpt));
            }
            // Include raw output excerpt for Miri/Loom diagnostics
            if !r.output.is_empty() {
                let out_excerpt = &r.output[..r.output.len().min(EXCERPT_LIMIT)];
                feedback.push_str(&format!("\n  output: {:?}", out_excerpt));
            }
        }
        feedback.push('\n');
    }

    feedback
}

async fn evaluate_code(
    cascade: &EvaluatorCascade,
    code: &str,
    test_code: &str,
) -> (f64, bool, i64, String, String) {
    let result = cascade.run_on_code(code, test_code).await;

    let level_reached: i64 = result
        .results
        .iter()
        .enumerate()
        .rev()
        .find(|(_, r)| r.passed)
        .map(|(i, _)| i as i64)
        .unwrap_or(-1);

    let invariants_passed = result.results.iter().filter(|r| r.passed).count();
    let progress = vf_perf::analyze_progress_guarantee(code);
    let progress_name = match progress {
        vf_perf::ProgressGuarantee::Blocking => "Blocking",
        vf_perf::ProgressGuarantee::ObstructionFree => "ObstructionFree",
        vf_perf::ProgressGuarantee::LockFree => "LockFree",
        vf_perf::ProgressGuarantee::WaitFree => "WaitFree",
    };
    let progress_ordinal = match progress {
        vf_perf::ProgressGuarantee::Blocking => 0,
        vf_perf::ProgressGuarantee::ObstructionFree => 1,
        vf_perf::ProgressGuarantee::LockFree => 2,
        vf_perf::ProgressGuarantee::WaitFree => 3,
    };

    let score = ((level_reached + 1) * 100) as f64
        + (invariants_passed as f64) * 10.0
        + (progress_ordinal as f64) * 25.0;

    // Rich structured feedback from ALL levels
    let feedback = build_structured_feedback(&result);

    (score.max(0.0), result.all_passed, level_reached, progress_name.to_string(), feedback)
}

fn select_parent<'a>(island: &'a [Candidate], rng: &mut impl Rng) -> &'a Candidate {
    debug_assert!(!island.is_empty(), "Island must not be empty");

    let mut sorted: Vec<&Candidate> = island.iter().collect();
    sorted.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());

    let weights: Vec<f64> = (0..sorted.len())
        .map(|i| ((i + 1) as f64).powf(-1.5))
        .collect();
    let total: f64 = weights.iter().sum();

    let mut pick = rng.gen::<f64>() * total;
    for (i, w) in weights.iter().enumerate() {
        pick -= w;
        if pick <= 0.0 {
            return sorted[i];
        }
    }
    sorted.last().unwrap()
}

/// Score-band-specific guidance for stepping stones approach.
fn stepping_stone_guidance(score: f64) -> &'static str {
    if score < 100.0 {
        "Fix compilation errors. The code does not compile. \
         Focus on type errors, missing imports, and syntax issues."
    } else if score < 200.0 {
        "Code compiles but has undefined behavior. \
         Fix memory safety issues flagged by Miri. Common problems: \
         use-after-free, dangling pointers, uninitialized memory, \
         aliasing violations. Use safe abstractions or proper unsafe blocks."
    } else if score < 300.0 {
        "Code is memory-safe but has concurrency bugs. \
         Fix data races and ordering issues flagged by Loom. \
         Ensure all atomic operations use appropriate memory orderings \
         (Acquire/Release for load/store pairs, AcqRel for RMW)."
    } else if score < 400.0 {
        "Concurrency is correct under normal conditions. \
         Add fault tolerance for DST (deterministic simulation testing). \
         Handle spurious failures, retries, and crash recovery."
    } else {
        "All verification levels pass. Optimize throughput and \
         progress guarantees. Aim for WaitFree if currently LockFree."
    }
}

fn build_mutation_prompt(parent: &Candidate, stepping_stones: bool, patterns: &str) -> String {
    // Rich structured feedback (up to 3000 chars)
    let feedback_section = if parent.feedback.is_empty() {
        String::new()
    } else {
        let len = parent.feedback.len().min(3000);
        format!("\n{}\n", &parent.feedback[..len])
    };

    let stepping_section = if stepping_stones {
        format!(
            "\n## Priority Guidance (score band {:.0}):\n{}\n",
            parent.score,
            stepping_stone_guidance(parent.score)
        )
    } else {
        String::new()
    };

    let patterns_section = if patterns.is_empty() {
        String::new()
    } else {
        format!(
            "\n## Reference Pattern (known-correct CAS approach):\n```rust\n{}\n```\n",
            patterns
        )
    };

    format!(
        "Here is a Rust implementation scoring {:.0} points.\n\
         Progress guarantee: {}\n\
         Highest cascade level passed: {}\n\n\
         Current code:\n```rust\n{}\n```\n\
         {}{}{}\n\
         Generate an improved version. Aim for:\n\
         1. If it doesn't compile, fix syntax/type errors\n\
         2. If Blocking (uses Mutex), replace with lock-free CAS operations using std::sync::atomic\n\
         3. If LockFree, optimize throughput or try WaitFree\n\
         4. Use crossbeam-epoch for safe memory reclamation\n\n\
         Return ONLY the complete Rust source code, no explanations.",
        parent.score,
        parent.progress,
        parent.level_reached,
        parent.code,
        feedback_section,
        stepping_section,
        patterns_section,
    )
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Resolve project root
    let project_root = cli
        .project_root
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let mut dir = std::env::current_exe()
                .unwrap_or_default()
                .parent()
                .unwrap_or(Path::new("."))
                .to_path_buf();
            for _ in 0..5 {
                if dir.join("Cargo.toml").exists() && dir.join("crates").exists() {
                    return dir;
                }
                dir = dir.parent().unwrap_or(Path::new(".")).to_path_buf();
            }
            std::env::current_dir().unwrap_or_default()
        });

    let tasks_dir = project_root.join("tools/vf-evolve/tasks").join(&cli.task);

    // Support --seed override for intermediate seeds (e.g., initial_atomic.rs)
    let initial_path = if let Some(ref seed_file) = cli.seed {
        tasks_dir.join(seed_file)
    } else {
        tasks_dir.join("initial.rs")
    };
    let trait_spec_path = tasks_dir.join("trait_spec.rs");

    if !initial_path.exists() {
        eprintln!("Error: {} not found", initial_path.display());
        process::exit(1);
    }
    if !trait_spec_path.exists() {
        eprintln!("Error: {} not found", trait_spec_path.display());
        process::exit(1);
    }

    let initial_code = std::fs::read_to_string(&initial_path).unwrap();
    let test_code = std::fs::read_to_string(&trait_spec_path).unwrap();

    // Load reference patterns if requested
    let patterns = if cli.inject_patterns {
        let patterns_path = tasks_dir.join("patterns.rs");
        if patterns_path.exists() {
            std::fs::read_to_string(&patterns_path).unwrap_or_default()
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    let max_level = match cli.max_level.to_lowercase().as_str() {
        "rustc" => EvaluatorLevel::Rustc,
        "miri" => EvaluatorLevel::Miri,
        "loom" => EvaluatorLevel::Loom,
        "dst" => EvaluatorLevel::Dst,
        "stateright" => EvaluatorLevel::Stateright,
        "kani" => EvaluatorLevel::Kani,
        "verus" => EvaluatorLevel::Verus,
        other => {
            eprintln!("Unknown level: {other}");
            process::exit(1);
        }
    };

    let api_key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_else(|_| {
        eprintln!("Error: ANTHROPIC_API_KEY not set");
        process::exit(1);
    });

    let cascade = EvaluatorCascade::new(CascadeConfig {
        max_level,
        fail_fast: true,
        timeout: Duration::from_secs(cli.timeout),
        ..CascadeConfig::default()
    });

    let http_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap();

    let category_desc = match cli.category.as_str() {
        "distributed" => "a distributed systems protocol",
        "adrs" => "an algorithmic domain problem (optimization/scheduling)",
        _ => "a concurrent data structure",
    };
    let system_prompt = format!(
        "You are evolving a Rust implementation of {} ({}). \
         The code must compile and pass the test harness. \
         Among correct implementations, prefer higher progress guarantees: \
         WaitFree > LockFree > ObstructionFree > Blocking. \
         Use std::sync::atomic and crossbeam-epoch for lock-free designs.",
        category_desc, cli.task
    );

    let output_dir = PathBuf::from(&cli.output_dir).join(&cli.task);
    std::fs::create_dir_all(&output_dir).unwrap();

    let mut rng = rand::thread_rng();

    // Multi-model ensemble
    let mut ensemble = default_ensemble();
    // Ensure we have at least as many models as islands (repeat if needed)
    while ensemble.len() < cli.islands {
        let idx = ensemble.len() % default_ensemble().len();
        let mut extra = default_ensemble()[idx].clone();
        extra.label = format!("{}-{}", extra.label, ensemble.len());
        ensemble.push(extra);
    }

    let mut islands: Vec<Vec<Candidate>> = vec![Vec::new(); cli.islands];
    let mut best_ever: Option<Candidate> = None;
    let mut total_evals: u64 = 0;
    let mut history: Vec<GenerationLog> = Vec::new();

    // Print header
    println!("\n{}", "=".repeat(70));
    println!("vf-evolve: {} ({} gens, {} islands, {} models)",
        cli.task, cli.generations, cli.islands, ensemble.len());
    println!("Category: {} | Stepping stones: {} | Patterns: {}",
        cli.category,
        cli.stepping_stones,
        !patterns.is_empty());
    if let Some(ref seed_file) = cli.seed {
        println!("Seed: {}", seed_file);
    }
    println!("Models:");
    for (i, m) in ensemble.iter().enumerate() {
        println!("  island {} -> {} ({})", i, m.label, m.name);
    }
    println!("{}", "=".repeat(70));

    // Evaluate seed
    let (score, correct, level, progress, feedback) =
        evaluate_code(&cascade, &initial_code, &test_code).await;
    total_evals += 1;

    let mut seed = Candidate::new_from_code(initial_code.clone(), 0, 0, "seed");
    seed.score = score;
    seed.correct = correct;
    seed.level_reached = level;
    seed.progress = progress.clone();
    seed.feedback = feedback;

    println!("\nSeed: score={:.1} correct={} level={} progress={}", score, correct, level, progress);

    for island in &mut islands {
        island.push(seed.clone());
    }
    best_ever = Some(seed.clone());

    // Evolution loop
    println!();
    for gen in 1..=cli.generations {
        let gen_start = Instant::now();

        for island_idx in 0..cli.islands {
            let parent = select_parent(&islands[island_idx], &mut rng).clone();

            // Select model for this mutation (bandit-style)
            let model_idx = select_model(&ensemble, island_idx, gen, total_evals, &mut rng);
            let model_name = ensemble[model_idx].name.clone();
            let model_label = ensemble[model_idx].label.clone();
            let temp = ensemble[model_idx].temperature.unwrap_or(cli.temperature);

            let prompt = build_mutation_prompt(&parent, cli.stepping_stones, &patterns);
            let mutated = match call_anthropic(
                &http_client,
                &api_key,
                &model_name,
                &system_prompt,
                &prompt,
                temp,
                4096,
            )
            .await
            {
                Ok(response) => extract_rust_code(&response),
                Err(e) => {
                    eprintln!("  [{}] LLM error (island {}): {}", model_label, island_idx, e);
                    continue;
                }
            };

            if mutated == parent.code {
                continue;
            }

            // Evaluate
            let (score, correct, level, progress, feedback) =
                evaluate_code(&cascade, &mutated, &test_code).await;
            total_evals += 1;

            // Record in bandit
            ensemble[model_idx].record(score);

            let mut candidate = Candidate::new_from_code(mutated, gen, island_idx, &model_label);
            candidate.score = score;
            candidate.correct = correct;
            candidate.level_reached = level;
            candidate.progress = progress;
            candidate.feedback = feedback;
            candidate.parent_id = Some(parent.id.clone());

            islands[island_idx].push(candidate.clone());

            // Update best
            if best_ever.as_ref().map_or(true, |b| candidate.score > b.score) {
                best_ever = Some(candidate.clone());
                let _ = std::fs::write(output_dir.join("best.rs"), &candidate.code);
                let _ = std::fs::write(
                    output_dir.join("best_info.json"),
                    serde_json::to_string_pretty(&candidate).unwrap(),
                );
            }

            // Trim island
            islands[island_idx].sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
            islands[island_idx].truncate(cli.population);
        }

        // Migration
        if gen % cli.migration_interval == 0 && cli.islands > 1 {
            for i in 0..cli.islands {
                let next = (i + 1) % cli.islands;
                if let Some(best) = islands[i].iter().max_by(|a, b| a.score.partial_cmp(&b.score).unwrap()) {
                    let migrant = best.clone();
                    if islands[next]
                        .iter()
                        .min_by(|a, b| a.score.partial_cmp(&b.score).unwrap())
                        .map_or(true, |worst| migrant.score > worst.score)
                    {
                        islands[next].push(migrant);
                        islands[next].sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
                        islands[next].truncate(cli.population);
                    }
                }
            }
            // Log migration
            eprintln!("  [migration] gen {gen}");
        }

        let gen_time = gen_start.elapsed().as_secs_f64();
        let island_bests: Vec<f64> = islands
            .iter()
            .map(|isl| isl.iter().map(|c| c.score).fold(0.0f64, f64::max))
            .collect();

        let best = best_ever.as_ref().unwrap();

        let model_stats: Vec<ModelStat> = ensemble
            .iter()
            .map(|m| ModelStat {
                label: m.label.clone(),
                evals: m.eval_count,
                avg_score: if m.eval_count > 0 { m.total_score / m.eval_count as f64 } else { 0.0 },
                best_score: m.best_score,
            })
            .collect();

        history.push(GenerationLog {
            generation: gen,
            best_score: best.score,
            best_progress: best.progress.clone(),
            best_model: best.model.clone(),
            island_bests: island_bests.clone(),
            evaluations: total_evals,
            duration_secs: gen_time,
            model_stats: model_stats.clone(),
        });

        // Compact log line
        let model_summary: String = model_stats
            .iter()
            .map(|m| format!("{}:{}/{:.0}", m.label, m.evals, m.best_score))
            .collect::<Vec<_>>()
            .join(" ");

        println!(
            "  Gen {:3} | best={:.1} ({}) | islands=[{}] | {} | {:.1}s",
            gen,
            best.score,
            best.progress,
            island_bests
                .iter()
                .map(|s| format!("{:.0}", s))
                .collect::<Vec<_>>()
                .join(", "),
            model_summary,
            gen_time,
        );
    }

    // Save history
    let _ = std::fs::write(
        output_dir.join("history.json"),
        serde_json::to_string_pretty(&history).unwrap(),
    );

    // Final summary
    println!("\n{}", "=".repeat(70));
    println!("Evolution Complete");
    println!("{}", "=".repeat(70));
    if let Some(ref best) = best_ever {
        println!("  Best score:    {:.1}", best.score);
        println!("  Best progress: {}", best.progress);
        println!("  Best level:    {}", best.level_reached);
        println!("  Best correct:  {}", best.correct);
        println!("  Best model:    {}", best.model);
        println!("  Generation:    {}", best.generation);
    }
    println!("  Total evals:   {}", total_evals);
    println!();
    println!("  Model Performance:");
    for m in &ensemble {
        let avg = if m.eval_count > 0 { m.total_score / m.eval_count as f64 } else { 0.0 };
        println!(
            "    {:<12} evals={:<4} avg={:<8.1} best={:.1}",
            m.label, m.eval_count, avg, m.best_score
        );
    }
    println!("  Output:        {}", output_dir.display());

    let summary = json!({
        "task": cli.task,
        "best_score": best_ever.as_ref().map(|b| b.score).unwrap_or(0.0),
        "best_progress": best_ever.as_ref().map(|b| b.progress.as_str()).unwrap_or("Unknown"),
        "best_level": best_ever.as_ref().map(|b| b.level_reached).unwrap_or(-1),
        "best_correct": best_ever.as_ref().map(|b| b.correct).unwrap_or(false),
        "best_model": best_ever.as_ref().map(|b| b.model.as_str()).unwrap_or("none"),
        "best_generation": best_ever.as_ref().map(|b| b.generation).unwrap_or(0),
        "generations_run": cli.generations,
        "total_evaluations": total_evals,
        "output_dir": output_dir.display().to_string(),
        "model_stats": ensemble.iter().map(|m| json!({
            "label": m.label,
            "model": m.name,
            "evals": m.eval_count,
            "avg_score": if m.eval_count > 0 { m.total_score / m.eval_count as f64 } else { 0.0 },
            "best_score": m.best_score,
        })).collect::<Vec<_>>(),
    });
    println!("\n{}", serde_json::to_string_pretty(&summary).unwrap());
}

fn fnv_hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}
