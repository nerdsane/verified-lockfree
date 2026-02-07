# Verified Evolution: What If Formal Specs Were Fitness Functions?

*From BitsEvolve to Self-Optimizing Distributed Systems*

## Part 1: The Puzzle

[BitsEvolve](https://www.datadoghq.com/blog/engineering/self-optimizing-system/) taught Go code to optimize itself — 25-90% speedups in bit manipulation, tag normalization, secure hashing. But it had no correctness guarantee. The fitness function was "run faster," not "run correctly."

[ADRS](https://arxiv.org/abs/2512.14806) (AI-Driven Research for Systems) went further: LLMs discovered algorithms that beat human SOTA — 13x improvement in transaction scheduling, 35% in TCP congestion control. But these algorithms were validated by simulators, not formally proven.

The gap: no system evolves code with *both* performance optimization *and* formal correctness guarantees.

**What if formal specifications — TLA+, model checkers, property-based tests — were the fitness function?**

## Part 2: The Insight — Specifications as Landscapes

[Alp Keles argues](https://alperenkeles.com/posts/llms-could-be-but-shouldnt-be-compilers/): "If you can specify, you can build." We go further: **if you can specify formally, you can evolve.**

A TLA+ specification defines *what* must be true, not *how* to achieve it. A verification cascade — rustc → Miri → Loom → DST → Stateright → Kani → Verus — creates a gradient from "doesn't compile" to "formally proven." That gradient is exactly what population-based evolution needs.

```
Score = 0        "doesn't compile"
Score = 100      "compiles (rustc pass)"
Score = 200      "no undefined behavior (Miri pass)"
Score = 300      "correct under all interleavings (Loom pass)"
Score = 400      "fault-tolerant (DST pass)"
Score = 500      "spec-conformant (Stateright pass)"
Score = 600      "bounded proof (Kani pass)"
Score = 700      "universal proof (Verus pass)"
Score = 700+     "correct AND fast (throughput + progress bonus)"
```

The scoring function:

```
score(c) = (level_reached + 1) * 100
         + invariants_passed * 10
         + progress_ordinal * 25       // WaitFree=3, LockFree=2, Blocking=0
```

A Mutex-based Treiber stack that passes all levels scores 440 (Blocking). A CAS-based lock-free stack that also passes all levels scores 490 (+50 from LockFree progress). Evolution has a clear gradient.

## Part 3: The System — Rust-Native, Multi-Model

We built `vf-evolve` as a Rust-native binary — no Python orchestration layer. The evolution engine, verification cascade, and LLM client are all compiled Rust, calling the Anthropic API directly via `reqwest`.

### Multi-Model Bandit Ensemble

The key architectural choice: **each island in the population uses a different LLM model**, and a UCB1 bandit algorithm adapts model selection based on which models produce higher-scoring candidates.

```
┌─────────────────────────────────────────────────────────┐
│                    vf-evolve (Rust)                      │
│                                                         │
│  Island 0        Island 1        Island 2    Island 3   │
│  ┌──────┐       ┌──────┐       ┌──────┐    ┌──────┐   │
│  │ Opus │       │Sonnet│       │Haiku │    │Sonnet│   │
│  │ t=0.7│       │ t=0.8│       │ t=1.0│    │ t=1.0│   │
│  └──┬───┘       └──┬───┘       └──┬───┘    └──┬───┘   │
│     │  mutate      │  mutate      │  mutate    │       │
│     ▼              ▼              ▼            ▼       │
│  ┌────────────────────────────────────────────────┐    │
│  │     Verification Cascade (rustc→miri→loom→DST) │    │
│  └────────────────────────────────────────────────┘    │
│     │              │              │            │       │
│     ▼  score       ▼  score       ▼  score     ▼      │
│  ┌────────────────────────────────────────────────┐    │
│  │  UCB1 Bandit: track avg score per model        │    │
│  │  70% exploit best model, 30% explore others    │    │
│  └────────────────────────────────────────────────┘    │
│                                                         │
│  Migration: best candidates move between islands        │
│  every N generations (ring topology)                    │
└─────────────────────────────────────────────────────────┘
```

**Why multiple models matter:** Different models have different failure modes. Opus produces higher-quality code but is slower and more expensive. Haiku is cheap and fast but generates more compile errors. Sonnet at high temperature explores wild designs that occasionally discover novel patterns. The bandit learns which model is most productive for each task.

### The Default Ensemble

| Island | Model | Temperature | Role |
|--------|-------|------------|------|
| 0 | Claude Opus 4 | 0.7 | Highest quality, precision mutations |
| 1 | Claude Sonnet 4 | 0.8 | Balanced speed + capability |
| 2 | Claude Haiku 4.5 | 1.0 | Cheapest, maximum diversity |
| 3 | Claude Sonnet 4 | 1.0 | Exploration via high temperature |

### Task Specifications

Every evolution task starts with a Lamport-style prose problem statement (PROBLEM.md) defining safety properties, liveness properties, performance dimensions, and what is NOT specified. This follows [Lamport's advice](https://lamport.azurewebsites.net/pubs/state-the-problem.pdf): "State the problem before describing the solution."

The pipeline:
```
Prose problem statement (PROBLEM.md)
    ↓ formalize
TLA+ specification (specs/distributed/*.tla)
    ↓ compile to tests
Trait specification (trait_spec.rs)
    ↓ guides
Evolution (vf-evolve binary)
    ↓ produces
Verified implementation (best.rs)
```

### Evolution Targets

We built 17 tasks across four categories:

| Category | Tasks | Seed | What Evolves |
|----------|-------|------|-------------|
| Lock-free (8) | treiber_stack, ring_buffer, linked_list, radix_tree, io_buffer, epoch_gc, pagecache, btree_plus | Mutex-based | CAS strategy, memory reclamation, backoff |
| SSI (1) | cross_shard_ssi | Mutex-based | Conflict detection, snapshot isolation |
| Distributed (3) | raft_election, two_phase_commit, **raft_consensus** | Mutex-based | Timeout strategy, recovery logic, log replication |
| ADRS (5) | txn_scheduling, tcp_congestion, load_balancing, cloud_scheduling, llm_sql_cache | Greedy baseline (now Rust) | Algorithm parameters |

New since the initial post:
- **raft_consensus**: Full Raft capstone with election + log replication + commit safety + all 5 Raft paper invariants (TLA+ spec: 350 lines, 12 tests)
- **All 5 ADRS tasks ported from Python to Rust**: now run through the same verification cascade as lock-free tasks
- Three new `vf-evolve` flags: `--stepping-stones`, `--seed`, `--inject-patterns`

Each Rust task has a TLA+ spec with EVALUATOR MAPPING comments linking invariants to verification levels.

## Part 4: Results — Full Problem Set (17 Tasks, 18 Runs)

We ran **all 17 tasks** through the cascade: 8 lock-free, 4 distributed (including the new Raft consensus capstone), and 5 ADRS domain problems ported from Python to Rust. Total: ~350 LLM evaluations, ~45 minutes wall clock.

### Complete Results Table

| Category | Task | Score | Progress | Best Model | Gen |
|----------|------|------:|----------|-----------|----:|
| Lock-free | treiber_stack | 160 | LockFree | opus | 1 |
| Lock-free | linked_list | 160 | LockFree | opus | 2 |
| Lock-free | ring_buffer | 160 | LockFree | opus | 1 |
| Lock-free | **epoch_gc** | **270** | **LockFree** | **opus** | **7** |
| Lock-free | btree_plus | 160 | LockFree | opus | 1 |
| Lock-free | radix_tree | 160 | LockFree | sonnet | 1 |
| Lock-free | pagecache | 160 | LockFree | sonnet | 4 |
| Lock-free | io_buffer | 220 | Blocking | seed | 0 |
| Distributed | cross_shard_ssi | 160 | LockFree | opus | 3 |
| Distributed | raft_election | 160 | LockFree | sonnet | 1 |
| Distributed | two_phase_commit | 160 | LockFree | sonnet | 2 |
| Distributed | raft_consensus | 50 | LockFree | opus | 1 |
| **ADRS** | **txn_scheduling** | **270** | **LockFree** | **haiku** | **6** |
| **ADRS** | **tcp_congestion** | **270** | **LockFree** | **sonnet** | **2** |
| **ADRS** | **load_balancing** | **270** | **LockFree** | **sonnet** | **3** |
| **ADRS** | **cloud_scheduling** | **270** | **LockFree** | **haiku** | **6** |
| ADRS | llm_sql_cache | 160 | LockFree | sonnet | 4 |

### The Star: epoch_gc → Lock-Free Epoch GC (Score 270)

The evolution produced a fully lock-free epoch-based garbage collector with CAS-based deferred destruction queue. Starting from a Mutex seed, by generation 7 Opus produced code that:
- Uses `AtomicPtr` CAS loops for the deferred destruction linked list
- Tracks pinned guards with `AtomicUsize` (no Mutex!)
- Safely collects garbage only when pin count reaches zero
- Passes Miri's UB checks completely

This is the strongest evidence that the cascade *can* guide evolution past the Miri barrier — given the right task structure.

### ADRS: Domain Problems Are Highly Accessible

All 5 Python ADRS simulators were ported to Rust and 4/5 hit score 270 within 10 generations. The cascade's Miri check is trivial for pure algorithmic code (no unsafe, no raw pointers). Key insight: **Haiku won 2 ADRS tasks** (txn_scheduling, cloud_scheduling) — the $0.005/eval model beats the $0.04/eval model on simpler domain optimization.

### Raft Consensus Capstone (Score 50)

The most ambitious task: full Raft with election + log replication + commit safety. 6 API methods, 4 message types, 12 tests covering all 5 Raft paper invariants. Score 50 means LLM-generated code compiles with LockFree progress but fails the test suite. The complex protocol API is a different barrier than memory safety — the LLM can write CAS code but can't coordinate 5 interacting invariants simultaneously. More generations needed.

### The Valley Crossing Attempt: Still Not Crossed

We tested three improvements to cross the Mutex→CAS valley on treiber_stack:

1. **Stepping stones** (`--stepping-stones`): score-band-specific guidance
2. **Pattern seeding** (`--inject-patterns`): reference crossbeam-epoch CAS snippet
3. **CAS seed** (`--seed initial_atomic.rs`): AtomicPtr stack with known UB, starting at 160

```
$ vf-evolve --task treiber_stack --generations 50 --stepping-stones \
    --inject-patterns --seed initial_atomic.rs --islands 2

Seed: score=160.0 correct=false level=0 progress=LockFree
                                   ↑ starts on CAS side of valley

Gen  1 | best=160.0 (LockFree) | all 4 models at 160
Gen 10 | best=160.0 (LockFree) | all 4 models at 160
Gen 25 | best=160.0 (LockFree) | all 4 models at 160
Gen 50 | best=160.0 (LockFree) | all 4 models at 160

Valley NOT crossed. 101 evaluations, all stuck at 160.
```

**Why?** Every LLM-generated CAS variant has subtle memory reclamation UB that Miri catches. The models can write correct-looking `compare_exchange` loops but can't produce Miri-clean `unsafe` deferred destruction. The `patterns.rs` template helped the LLM generate crossbeam-epoch style code, but the epoch GC *usage* still had UB.

The valley is real but **not universal** — epoch_gc crossed it. The difference: epoch_gc's API is *about* memory reclamation, so the LLM is forced to focus on exactly the right problem.

### Model Performance Across All Tasks

| Model | Wins (best on task) | Strengths |
|-------|-------------------:|-----------|
| Opus | 6/17 (35%) | Complex lock-free tasks (epoch_gc, btree_plus, linked_list) |
| Sonnet | 7/17 (41%) | Most consistent across all categories |
| Haiku | 2/17 (12%) | ADRS domain optimization (cheapest model!) |
| Seed | 2/17 (12%) | io_buffer, raft_consensus (unbeaten) |

### Cost Analysis (full problem set)

| Resource | Amount |
|----------|--------|
| Total runs | 18 |
| Total evaluations | ~350 |
| Wall time | ~45 minutes |
| Cascade time (avg) | ~35s/eval |

## Part 5: Making It Scale (Bitter Lesson Alignment)

The system is designed for compute scaling:

**More generations** = more design space explored. The valley between Mutex and CAS likely needs 20-50 generations to cross, based on the complexity of producing correct lock-free code.

**More islands** = more diversity. Each island can explore a different region of the design space (different CAS strategies, different memory reclamation, different backoff).

**UCB1 bandit** = automatic model allocation. If Opus consistently produces better mutations, it gets more of the compute budget. If Haiku discovers a surprising pattern, the bandit adapts.

**Property-based testing** scales with compute: `VF_PROPTEST_CASES=10` for quick runs, `VF_PROPTEST_CASES=10000` for thorough verification. Proptest modules are gated with `#[cfg(not(miri))]` so they don't slow down the Miri level.

**Counterexample accumulation** (designed, not yet triggered): when Stateright finds an invariant violation, the counterexample trace becomes a regression test. Future generations must pass all accumulated tests. The fitness landscape sharpens monotonically.

## Part 6: The Arc — Self-Optimizing Distributed Systems

| System | What It Evolves | Correctness | Performance | LLM Role |
|--------|----------------|-------------|-------------|----------|
| **BitsEvolve** | Go bit manipulation | None (tests only) | 25-90% speedup | Single model |
| **ADRS** | Python algorithms | Simulator validation | 13x, 35% improvement | Single model |
| **Verified Evolution** | Rust concurrent code | TLA+ → 7-level cascade | Throughput benchmark | Multi-model bandit |

The progression: performance-only → algorithm discovery → formally verified evolution with model diversity.

### What We Built

- `vf-evolve`: Rust-native evolution binary with island-model population, multi-model LLM bandit ensemble (Opus/Sonnet/Haiku), power-law selection, and 7-level verification cascade
- `vf-cascade-runner`: Standalone cascade evaluator with adaptive start-level and counterexample extraction
- **17 evolution tasks** with TLA+ specs, trait_spec test harnesses, and Lamport-style problem statements
- 3 distributed protocol specs (Raft election, Two-phase commit, **Raft consensus**) with full TLA+ invariant definitions
- **5 ADRS domain simulators** ported from Python to Rust with scoring functions
- **Stepping stones + pattern injection** for guided evolution across fitness valleys

### What's Next

1. **Cross the Miri barrier** — seed with partial epoch GC scaffolding, or 200+ generation runs
2. **Loom-level evaluation** — run full cascade through Loom to test concurrency correctness
3. **Raft consensus deep run** — 50+ generations to evolve working log replication
4. **Cross-task pattern transfer** — epoch_gc's CAS patterns seeding treiber_stack/ring_buffer
5. **ADRS domain simulators** — integrate `adrs.rs` scoring into the cascade for domain-specific fitness
6. **Production integration** — production telemetry → evolve → verify → deploy loop

The self-optimizing loop:
```
Production metrics → identify bottleneck → evolve algorithm under TLA+ constraints
    → Stateright verifies invariants → benchmark confirms perf improvement
    → deploy → observe → repeat
```

Every TLA+ spec in your codebase becomes an evolution target. The will to specify is the bottleneck, not the will to implement.

---

*Sesh Nalla. February 2026.*
*Code: [github.com/verified-concurrent](https://github.com/TODO/verified-concurrent)*
