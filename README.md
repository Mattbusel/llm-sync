# llm-sync

CRDT and vector clock primitives for distributed LLM agent state synchronization.

Coordinate shared state across multiple LLM agents without a central coordinator. Merge concurrent updates safely using conflict-free replicated data types.

## What's inside

- **VectorClock** — happens-before ordering for distributed agent events
- **GCounter / PNCounter** — grow-only and increment/decrement CRDTs for agent metrics
- **LWWRegister** — last-write-wins register for agent configuration and shared flags
- **ORSet** — observed-remove set for shared agent capability registries
- **StateMerge** — merge any two CRDT states deterministically

## Use cases

- Shared agent configuration that multiple instances can update concurrently
- Distributed rate-limit counters across an agent fleet
- Consensus-free agent state replication over NATS or Redis pubsub
- Conflict-free merging of agent memory segments

## Quick start

```rust
use llm_sync::{VectorClock, LWWRegister};

let mut clock_a = VectorClock::new("agent-a");
let mut clock_b = VectorClock::new("agent-b");

clock_a.increment();
clock_b.increment();

// Merge — safe under concurrent updates
clock_a.merge(&clock_b);
println!("Merged clock: {:?}", clock_a);
```

## Add to your project

```toml
[dependencies]
llm-sync = { git = "https://github.com/Mattbusel/llm-sync" }
```

## Test coverage

```bash
cargo test
```
