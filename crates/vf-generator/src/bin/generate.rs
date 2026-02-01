//! CLI for generating verified lock-free implementations from TLA+ specs.
//!
//! # Usage
//!
//! ```bash
//! # Generate from spec file
//! cargo run -p vf-generator --bin vf-generate -- --spec specs/lockfree/treiber_stack.tla
//!
//! # With custom max attempts
//! cargo run -p vf-generator --bin vf-generate -- --spec specs/lockfree/treiber_stack.tla --max-attempts 10
//!
//! # Quick mode (faster verification)
//! cargo run -p vf-generator --bin vf-generate -- --spec specs/lockfree/treiber_stack.tla --quick
//!
//! # Save output to file
//! cargo run -p vf-generator --bin vf-generate -- --spec specs/lockfree/treiber_stack.tla --output generated.rs
//! ```

use std::path::PathBuf;
use std::process::ExitCode;

use vf_evaluators::EvaluatorLevel;
use vf_generator::{CodeGenerator, GeneratorConfig};

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();

    if args.help {
        print_help();
        return ExitCode::SUCCESS;
    }

    let spec_path = match args.spec {
        Some(path) => path,
        None => {
            eprintln!("Error: --spec is required");
            eprintln!("Run with --help for usage information");
            return ExitCode::FAILURE;
        }
    };

    // Build config
    let mut config = if args.quick {
        GeneratorConfig::quick()
    } else if args.thorough {
        GeneratorConfig::thorough()
    } else {
        GeneratorConfig::default()
    };

    if let Some(max) = args.max_attempts {
        config.max_attempts = max;
    }

    if let Some(ref level) = args.max_level {
        config.cascade_config.max_level = parse_level(level);
    }

    config.verbose = !args.quiet;

    // Create generator
    let generator = match CodeGenerator::from_env(config) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("Error creating generator: {}", e);
            eprintln!();
            eprintln!("Make sure ANTHROPIC_API_KEY is set:");
            eprintln!("  export ANTHROPIC_API_KEY=sk-ant-...");
            return ExitCode::FAILURE;
        }
    };

    println!("Verified Lock-Free Code Generator");
    println!("==================================");
    println!();
    println!("Spec: {}", spec_path.display());
    println!("Max level: {:?}", generator.max_level());
    println!();

    // Generate
    match generator.generate_from_file(&spec_path).await {
        Ok(result) => {
            println!();
            println!("{}", result.format_summary());

            if result.success {
                if let Some(ref code) = result.code {
                    // Output to file or stdout
                    if let Some(output_path) = args.output {
                        match std::fs::write(&output_path, code) {
                            Ok(()) => {
                                println!("Code written to: {}", output_path.display());
                            }
                            Err(e) => {
                                eprintln!("Failed to write output: {}", e);
                                return ExitCode::FAILURE;
                            }
                        }
                    } else {
                        println!();
                        println!("Generated Code:");
                        println!("===============");
                        println!();
                        println!("{}", code);
                    }
                }
                ExitCode::SUCCESS
            } else {
                eprintln!("Generation failed after {} attempts", result.attempts);
                ExitCode::FAILURE
            }
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            ExitCode::FAILURE
        }
    }
}

/// Simple argument parsing (no external deps).
struct Args {
    spec: Option<PathBuf>,
    output: Option<PathBuf>,
    max_attempts: Option<u32>,
    max_level: Option<String>,
    quick: bool,
    thorough: bool,
    quiet: bool,
    help: bool,
}

impl Args {
    fn parse() -> Self {
        let mut args = Args {
            spec: None,
            output: None,
            max_attempts: None,
            max_level: None,
            quick: false,
            thorough: false,
            quiet: false,
            help: false,
        };

        let mut iter = std::env::args().skip(1);
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--spec" | "-s" => {
                    args.spec = iter.next().map(PathBuf::from);
                }
                "--output" | "-o" => {
                    args.output = iter.next().map(PathBuf::from);
                }
                "--max-attempts" | "-n" => {
                    args.max_attempts = iter.next().and_then(|s| s.parse().ok());
                }
                "--max-level" | "-l" => {
                    args.max_level = iter.next();
                }
                "--quick" => {
                    args.quick = true;
                }
                "--thorough" => {
                    args.thorough = true;
                }
                "--quiet" | "-q" => {
                    args.quiet = true;
                }
                "--help" | "-h" => {
                    args.help = true;
                }
                other => {
                    // Treat as spec path if no flag
                    if !other.starts_with('-') && args.spec.is_none() {
                        args.spec = Some(PathBuf::from(other));
                    }
                }
            }
        }

        args
    }
}

fn parse_level(s: &str) -> EvaluatorLevel {
    match s.to_lowercase().as_str() {
        "rustc" | "0" => EvaluatorLevel::Rustc,
        "miri" | "1" => EvaluatorLevel::Miri,
        "loom" | "2" => EvaluatorLevel::Loom,
        "dst" | "3" => EvaluatorLevel::Dst,
        "stateright" | "4" => EvaluatorLevel::Stateright,
        "kani" | "5" => EvaluatorLevel::Kani,
        _ => EvaluatorLevel::Dst, // Default
    }
}

fn print_help() {
    println!(
        r#"vf-generate - Generate verified lock-free code from TLA+ specs

USAGE:
    vf-generate --spec <SPEC_FILE> [OPTIONS]

OPTIONS:
    -s, --spec <FILE>        TLA+ specification file (required)
    -o, --output <FILE>      Output file for generated code (default: stdout)
    -n, --max-attempts <N>   Maximum generation attempts (default: 5)
    -l, --max-level <LEVEL>  Maximum evaluator level (rustc, miri, loom, dst, stateright, kani)
    --quick                  Use quick verification (fewer iterations)
    --thorough               Use thorough verification (more iterations)
    -q, --quiet              Suppress progress output
    -h, --help               Show this help message

EXAMPLES:
    # Generate from TLA+ spec
    vf-generate --spec specs/lockfree/treiber_stack.tla

    # Quick generation with output file
    vf-generate --spec specs/lockfree/treiber_stack.tla --quick --output stack.rs

    # Thorough verification up to loom level
    vf-generate --spec specs/lockfree/treiber_stack.tla --thorough --max-level loom

ENVIRONMENT:
    ANTHROPIC_API_KEY    Required. Your Anthropic API key.

EVALUATOR CASCADE:
    Level 0: rustc      - Type checking, lifetime analysis
    Level 1: miri       - Undefined behavior detection
    Level 2: loom       - Thread interleaving exploration
    Level 3: DST        - Deterministic simulation testing
    Level 4: stateright - Model checking against TLA+ spec
    Level 5: kani       - Bounded model checking / proofs
"#
    );
}
