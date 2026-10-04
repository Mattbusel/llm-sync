# llm-sync

Let several LLM agents share counters, sets, notes and even a text document without a central server: each agent edits its own copy, and the copies merge into the same result whatever order they are exchanged in.

[![crates.io](https://img.shields.io/crates/v/llm-sync.svg)](https://crates.io/crates/llm-sync)
[![docs.rs](https://img.shields.io/docsrs/llm-sync)](https://docs.rs/llm-sync)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://gitlab.com/mattbusel/llm-sync/-/blob/main/LICENSE)

Multi-agent systems often need a little shared state: how many tasks are done, which URLs were visited, the current plan, a shared draft. The data types here are CRDTs (conflict-free replicated data types): merging is order-independent and safe to repeat, so you can send state over whatever you already have (HTTP, Redis, a queue, files) without locks or a coordinator. No async runtime and no I/O.

## Install

```bash
cargo add llm-sync
# optional: shared text, clock-safe timestamps, compact wire format
cargo add llm-sync --features text,hlc,msgpack
```

## In ten lines

```rust
use llm_sync::ORSet;

let mut a = ORSet::new();
a.add("task-7", "agent-a");
let mut b = a.clone();               // b received a's state
a.remove("task-7");                  // a closes the task...
b.add("task-7", "agent-b");          // ...while b re-opens it
let (ab, ba) = (a.merge(&b), b.merge(&a));
assert_eq!(ab, ba);                  // same result in any order
assert!(ab.contains("task-7"));      // the concurrent re-open wins
```

## Quickstart

```rust
use llm_sync::{AgentState, ClockOrder, GCounter, GSet, LWWRegister, SyncError};

fn main() -> Result<(), SyncError> {
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

    // Neither saw the other's work: their clocks are concurrent.
    assert_eq!(a.vector_clock.compare(&b.vector_clock), ClockOrder::Concurrent);

    // Ship B's state as JSON and merge it on A. The order does not matter.
    let b_on_the_wire = b.to_json()?;
    let merged = a.merge(&AgentState::from_json(&b_on_the_wire)?);

    assert_eq!(merged.counters["tasks_done"].value(), 5);
    assert!(merged.sets["visited"].contains("https://example.com/a"));
    assert_eq!(merged.registers["plan"].read().map(String::as_str), Some("summarize findings"));
    println!("merged: {} tasks done", merged.counters["tasks_done"].value());
    Ok(())
}
```

The same program is `examples/quickstart.rs`: `cargo run --example quickstart`.

### Agents editing one document (feature `text`)

A register keeps only one of two concurrent edits. `SharedText` keeps both:

```rust,ignore
use llm_sync::SharedText;

let mut planner = SharedText::new(1);           // one id per agent
planner.push("1. research\n");
let mut writer = SharedText::from_update(2, &planner.encode_state())?;

planner.push("2. outline\n");                    // both edit at once
writer.insert(0, "PLAN\n")?;

// Send each side only what the other is missing.
writer.apply_update(&planner.encode_diff(&writer.state_vector())?)?;
planner.apply_update(&writer.encode_diff(&planner.state_vector())?)?;

assert_eq!(planner.text(), "PLAN\n1. research\n2. outline\n");
assert_eq!(planner.text(), writer.text());
```

Run it with `cargo run --example shared_text --features text`. The updates are Yjs binary updates, so a browser editor built on [Yjs](https://yjs.dev) can join the same document.

### Timestamps that survive clock skew (feature `hlc`)

Last-write-wins needs timestamps. Wall clocks disagree between machines: an agent whose clock is 300 ms slow can read a value, change it, and still lose to the value it just read. `AgentClock` is a hybrid logical clock (from [uhlc](https://crates.io/crates/uhlc)): after `observe`-ing what it has seen, its next timestamp is always later, and a timestamp more than 500 ms in the future is refused instead of dragging every clock forward.

```rust,ignore
let clock = AgentClock::new();
let merged = mine.merge(&theirs);
clock.observe(merged.max_timestamp())?;              // learn from remote writes
plan.write("v2".to_string(), clock.now(), "agent-a"); // sorts after everything seen
```

On `wasm32-unknown-unknown` there is no system clock: use `AgentClock::with_physical_clock(|| clock::physical_time(date_now_ms()), max_drift)`.

## Why this and not a CRDT framework

- **[yrs](https://crates.io/crates/yrs) / [automerge](https://crates.io/crates/automerge) / [loro](https://crates.io/crates/loro)** model a whole collaborative document with operation history. They are the right tool for rich text and nested JSON documents; `llm-sync` uses yrs for exactly that (`SharedText`). For "a few counters, sets and settings shared by agents" they bring a large dependency and a document model you have to learn.
- **[crdts](https://crates.io/crates/crdts)** has well-tested building blocks with generic actor types and operation-based APIs (last release August 2023). `llm-sync` keeps a smaller state-based API with string agent ids, bundles the types into one `AgentState` that round-trips through JSON or MessagePack, and adds causality-safe timestamps. Its `ORSet` and `GCounter` are property-tested against crdts' `Orswot` and `GCounter` on random histories and agree on every replica.

## Already using another crate?

- **crdts**: the same add-wins and counter semantics (cross-checked in `tests/properties.rs`), so state can be mirrored between the two.
- **Yjs (JavaScript) / yrs**: `SharedText` updates are Yjs binary updates; a browser editor using Yjs can apply them directly.
- **serde**: everything derives `Serialize`/`Deserialize`; `AgentState` has `to_json`/`from_json` and, with `msgpack`, `to_msgpack`/`from_msgpack` (rmp-serde, field names kept).
- **Transports** (HTTP, Redis, NATS, files): send the bytes; merging is order-independent and safe to repeat.

## Features

| Feature | Default | What it adds | Extra dependencies |
|---|---|---|---|
| (none) | yes | `VectorClock`, `GCounter`, `PNCounter`, `GSet`, `ORSet`, `LWWRegister`, `ORMap`, `AgentState` with JSON | serde, serde_json, uuid, thiserror |
| `text` | no | `SharedText`: agents typing into one document (Yjs-compatible updates) | [yrs](https://crates.io/crates/yrs) |
| `hlc` | no | `AgentClock`: hybrid logical clock timestamps for LWW writes | [uhlc](https://crates.io/crates/uhlc) |
| `msgpack` | no | `AgentState::to_msgpack` / `from_msgpack` | [rmp-serde](https://crates.io/crates/rmp-serde) |

## Examples

| Example | Shows | Run |
|---|---|---|
| `quickstart` | counters, sets, registers, vector clocks, JSON round trip | `cargo run --example quickstart` |
| `shared_text` | two agents typing into one document | `cargo run --example shared_text --features text` |
| `task_board` | three agents, an add-wins task list, HLC-timestamped plan, MessagePack wire format, two sync orders that agree | `cargo run --example task_board --features hlc,msgpack` |

## The data types

| Type | Use it for | Merge rule |
|---|---|---|
| `VectorClock` | "did A see B's work?" (`compare` gives `Before`, `After`, `Concurrent`, `Equal`) | per-agent maximum |
| `GCounter` | counts that only go up | per-agent maximum, then sum (saturating) |
| `PNCounter` | counts that go up and down | two `GCounter`s |
| `GSet` | sets you only add to (visited URLs, seen ids) | union |
| `ORSet` | sets with removal: open tasks, claimed work, flags | add wins over a concurrent remove; a remove cancels only the adds it saw |
| `LWWRegister<T>` | one value, latest write wins | higher timestamp; ties go to the higher writer id, then the larger value, so every replica picks the same winner |
| `ORMap` | string-to-string map with `set` and `remove` | per-key last-write-wins; a removal is a timestamped tombstone |
| `AgentState` | a bundle of all of the above plus a clock | merges each part |
| `SharedText` | a document several agents type into | Yjs sequence CRDT |

Every merge is commutative, associative and idempotent: `tests/properties.rs` checks all three laws for every type on random states, and a deliberately broken `ORSet::merge` makes those tests fail.

## Performance

`benches/vs_alternatives.rs` (Intel i7-13700KF, Windows 11, release build, other work running, so rough):

| Benchmark | llm-sync 0.2 | other |
|---|---|---|
| merge two `GCounter`s with 100 agents | 7.2 µs | 0.1.1: 7.0 µs; crdts 7.3: 14.1 µs (includes cloning both, its merge consumes the other replica) |
| merge two add-wins sets, 1,000 elements, 10% removed | 181 µs | crdts `Orswot`: 159 µs |
| `AgentState` (50 counters, 50 registers) encode + decode | JSON 28.5 µs, 5,508 bytes | MessagePack 28.3 µs, 4,250 bytes (23% smaller) |

crdts' `Orswot` is faster and stores no tombstones; `ORSet` keeps removed tags, so it grows with the number of removes. Choose `Orswot` if your sets churn heavily. Reproduce with `cargo bench --bench vs_alternatives --features msgpack`.

## Limitations

- `GSet` cannot remove entries; use `ORSet`.
- `ORSet` and `ORMap` keep tombstones for removed entries forever.
- `AgentState` sends whole CRDTs, not deltas, so its size grows with the number of agents and keys. `SharedText` does send deltas (`encode_diff`).
- `SharedText` positions are byte offsets and must fall on character boundaries (checked, returns an error otherwise).
- `AgentClock::new` reads the system clock, which does not exist on `wasm32-unknown-unknown`; use `with_physical_clock` there.

Run the tests with `cargo test --all-features`. Contributions are welcome, see [CONTRIBUTING.md](https://gitlab.com/mattbusel/llm-sync/-/blob/main/CONTRIBUTING.md).

## License

MIT, see [LICENSE](https://gitlab.com/mattbusel/llm-sync/-/blob/main/LICENSE).
