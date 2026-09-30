# llm-sync

[![CI](https://github.com/Mattbusel/llm-sync/actions/workflows/ci.yml/badge.svg)](https://github.com/Mattbusel/llm-sync/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/llm-sync.svg)](https://crates.io/crates/llm-sync)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

CRDTs and vector clocks for sharing state between LLM agents in Rust, so several agents can update the same counters, sets and key-value data independently and merge without a central coordinator or conflicts.

Multi-agent systems often need a little shared state: how many tasks are done, which URLs have been visited, the current plan. `llm-sync` gives you small, serializable CRDTs where every merge is commutative, associative and idempotent, so it does not matter in which order, or how many times, agents exchange state.

## Features

- **`VectorClock`**: `tick`, `merge`, and `compare` returning `ClockOrder::Before / After / Concurrent / Equal` for causal ordering of agent events.
- **`GCounter`** (grow-only) and **`PNCounter`** (increment and decrement) per-node counters.
- **`GSet`**: grow-only set of strings.
- **`LWWRegister<T>`**: last-write-wins register for any `Clone + Serialize + Deserialize` value, resolved by logical timestamp.
- **`ORMap`**: string-to-string map where each key is an LWW register.
- **`AgentState`**: one bundle of named counters, sets, registers and maps plus a vector clock and `SessionId`, with `merge`, `to_json` and `from_json` for sending it over any transport.
- No async runtime and no I/O: bring your own transport (HTTP, Redis, NATS, files).

## Quick start

```bash
cargo add llm-sync
```

```rust
use llm_sync::{AgentState, GCounter, GSet, LWWRegister};
use llm_sync::vclock::ClockOrder;

fn main() -> Result<(), llm_sync::SyncError> {
    // Agent A records its work.
    let mut a = AgentState::new();
    let mut done = GCounter::new();
    done.increment("agent-a", 3);
    a.counters.insert("tasks_done".into(), done);
    let mut seen = GSet::new();
    seen.insert("https://example.com/a");
    a.sets.insert("visited".into(), seen);
    a.vector_clock.tick("agent-a");

    // Agent B, independently.
    let mut b = AgentState::new();
    let mut done = GCounter::new();
    done.increment("agent-b", 2);
    b.counters.insert("tasks_done".into(), done);
    let mut plan = LWWRegister::new();
    plan.write("summarize findings".to_string(), 7, "agent-b");
    b.registers.insert("plan".into(), plan);
    b.vector_clock.tick("agent-b");

    assert_eq!(a.vector_clock.compare(&b.vector_clock), ClockOrder::Concurrent);

    // Ship B's state as JSON, merge on A. Order does not matter.
    let b_on_wire = b.to_json()?;
    let merged = a.merge(&AgentState::from_json(&b_on_wire)?);

    assert_eq!(merged.counters["tasks_done"].value(), 5);
    assert!(merged.sets["visited"].contains("https://example.com/a"));
    assert_eq!(merged.registers["plan"].read().map(String::as_str), Some("summarize findings"));
    Ok(())
}
```

## How it works

| File | What it holds |
|---|---|
| `src/vclock.rs` | `VectorClock`, `ClockOrder` |
| `src/crdt.rs` | `GCounter`, `PNCounter`, `GSet`, `LWWRegister`, `ORMap` |
| `src/session.rs` | `AgentState`, `SessionId`, JSON round-trip |
| `src/error.rs` | `SyncError` |

All `merge` methods take `&self` and return a new value, so replicas are never mutated by a merge. The test suite includes associativity checks and a 24-agent counter merge.

## Status and limitations

Version 0.1, state-based CRDTs only.

- There is no removal: `GSet` and `ORMap` cannot delete entries (use a tombstone value in your application if you need that).
- On `merge`, `LWWRegister` ties keep the local value, so give writers distinct timestamps.
- `AgentState` stores whole CRDTs, so the JSON grows with the number of agents and keys; there are no deltas.

```bash
cargo test
cargo bench
```

## License

MIT, see [LICENSE](LICENSE).

---

Part of a set of Rust crates for LLM agents, see [rust-crates](https://github.com/Mattbusel/rust-crates).
