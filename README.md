# Verified Lock-Free

**Correctness-by-construction verification cascade.**

Code is disposable. Specs, invariants, and state machines are the intent.

## The Paradigm Shift

| Traditional | This Project |
|-------------|--------------|
| Code is precious, tests verify it | **Specs are precious, code is disposable** |
| Tests catch bugs | **Evaluators prove correctness** |
| Implementation defines behavior | **TLA+ specs define truth** |
| Fix bugs in code | **Regenerate code from spec** |

Any implementation that passes the evaluator cascade is **correct by construction**.

## The Verification Cascade

Code must pass each level in order:

```
┌─────────────────────────────────────────────────────────────┐
│                  EVALUATOR CASCADE                           │
│                                                              │
│  Level 0: rustc        [instant]   Types, lifetimes         │
│      ↓ pass                                                  │
│  Level 1: miri         [seconds]   Undefined behavior        │
│      ↓ pass                                                  │
│  Level 2: loom         [seconds]   Thread interleavings      │
│      ↓ pass                                                  │
│  Level 3: DST          [seconds]   Faults, crashes, delays   │
│      ↓ pass                                                  │
│  Level 4: stateright   [seconds]   Spec conformance          │
│      ↓ pass                                                  │
│  Level 5: kani         [minutes]   Bounded proofs            │
│      ↓ pass                                                  │
│  Level 6: verus        [minutes]   SMT theorem proving       │
│      ↓ pass                                                  │
│  ✅ CORRECT BY CONSTRUCTION                                  │
└─────────────────────────────────────────────────────────────┘
```

## Core Principle: Code is Disposable

Generated code is **pure** - just the algorithm, nothing else.

```rust
impl TreiberStack {
    fn push(&self, value: u64) {
        let node = Node::new(value);
        loop {
            let head = self.head.load(Acquire);
            node.next.store(head, Relaxed);
            if self.head.compare_exchange(head, node, Release, Relaxed).is_ok() {
                break;
            }
        }
    }
}
```

Verification happens externally. The code doesn't know it's being verified.

## What Defines Correctness

The **intent** lives in specifications, not implementations:

| Artifact | Role | Lifespan |
|----------|------|----------|
| TLA+ Specs | Define truth | Permanent |
| Invariants | Properties that must hold | Permanent |
| State Machines | Valid transitions | Permanent |
| Oracles | Interesting interleavings | Derived |
| **Code** | **One possible implementation** | **Disposable** |

## Three Pillars

### 1. Correctness (Evaluator Cascade)

Seven levels from fast type-checking to SMT theorem proving.

### 2. Quality (TigerStyle)

Full implementation of [TigerStyle](https://tigerstyle.dev):
- Explicit limits with `_MAX` suffix
- Big-endian naming: `segment_size_bytes_max`
- 2+ assertions per function
- u64 not usize

### 3. Performance

- Progress guarantees: wait-free > lock-free > obstruction-free
- Memory overhead analysis

## Quick Start

```bash
# Run all tests
cargo test --workspace

# Run with specific DST seed (for reproduction)
DST_SEED=12345 cargo test -p vf-examples

# Run stateright model checking
cargo test -p vf-stateright
```

## Crate Map

| Crate | Purpose |
|-------|---------|
| `vf-core` | Invariants, properties, counterexamples |
| `vf-dst` | Deterministic simulation (clock, RNG, faults) |
| `vf-evaluators` | Cascade orchestration (7 levels) |
| `vf-stateright` | State machine models, oracle extraction |
| `vf-generator` | LLM code generation from specs |
| `vf-examples` | Reference implementations |

## Oracle Flow

Stateright discovers interesting interleavings, which feed both Loom and DST:

```
TLA+ Spec → Stateright Model → Oracle Extractor
                                      │
                    ┌─────────────────┴─────────────────┐
                    ↓                                   ↓
              LoomScenario                        OracleTrace
           (atomic interleaving)               (fault injection)
```

## DST: Deterministic Simulation Testing

All behavior reproducible via seed:

```rust
use vf_dst::{DstEnv, get_or_generate_seed};

let seed = get_or_generate_seed();
let mut env = DstEnv::new(seed);

env.clock().advance_ms(100);           // Deterministic time
let value: u64 = env.rng().gen();      // Deterministic randomness
if env.fault().should_fail() { ... }   // Deterministic faults
```

Reproduce any failure: `DST_SEED=12345 cargo test`

## License

MIT OR Apache-2.0
