//! # vf-generator
//!
//! LLM-powered code generation with spec-guided verification.
//!
//! This crate provides:
//! - Claude API integration for code generation
//! - TLA+ spec to prompt transformation
//! - Counterexample-to-fix feedback loop
//! - Generate → verify → fix cycle
//!
//! # Usage
//!
//! ```bash
//! # Generate implementation from TLA+ spec
//! cargo run -p vf-generator -- --spec specs/lockfree/treiber_stack.tla
//!
//! # With custom API key
//! ANTHROPIC_API_KEY=sk-... cargo run -p vf-generator -- --spec specs/lockfree/treiber_stack.tla
//!
//! # With max retry attempts
//! cargo run -p vf-generator -- --spec specs/lockfree/treiber_stack.tla --max-attempts 5
//! ```
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────┐     ┌─────────────┐     ┌─────────────┐
//! │  TLA+ Spec  │ ──> │   Prompt    │ ──> │   Claude    │
//! │             │     │  Generator  │     │     API     │
//! └─────────────┘     └─────────────┘     └──────┬──────┘
//!                                                │
//!                                                ▼
//! ┌─────────────┐     ┌─────────────┐     ┌─────────────┐
//! │   Output    │ <── │  Cascade    │ <── │  Generated  │
//! │    Code     │     │  Verifier   │     │    Code     │
//! └─────────────┘     └──────┬──────┘     └─────────────┘
//!                            │
//!                     (if fails)
//!                            │
//!                            ▼
//!                     ┌─────────────┐
//!                     │    Fix      │ ──> (retry)
//!                     │   Prompt    │
//!                     └─────────────┘
//! ```

pub mod client;
pub mod generator;
pub mod prompt;
pub mod templates;

pub use client::{ClaudeClient, ClaudeConfig, Message, Role};
pub use generator::{CodeGenerator, GeneratorConfig, GeneratorResult};
pub use prompt::{PromptBuilder, PromptTemplate, SpecType};
pub use templates::dst_instrumented::{generate_dst_instrumented_code, DST_INSTRUMENTED_TEMPLATE};
