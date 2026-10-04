# Changelog

## 0.2.0 (2026-10-03)

### Added
- Feature `text`: `SharedText`, a text CRDT for agents editing one document at the same time, built on Yrs (the Rust port of Yjs). Concurrent inserts and deletes are all kept and every replica converges; updates are Yjs-compatible binary deltas (`state_vector`, `encode_diff`, `apply_update`, `merge_from`). Positions are checked to fall on character boundaries.
- `ORSet`: an add-wins observed-remove set (open tasks, claimed work, flags), also in `AgentState::or_sets` (serde default, old JSON loads). Property-tested against crdts' `Orswot` on random histories with partial syncs: same contents on every replica.
- Feature `hlc`: `AgentClock`, a hybrid logical clock (uhlc) for LWW timestamps. A machine whose clock is 300 ms slow no longer loses a write made after reading the newer value (tested), and remote timestamps more than the allowed drift in the future are refused. `AgentState::max_timestamp`, `ORMap::max_timestamp`, `clock::physical_time` for wasm32 clocks.
- Feature `msgpack`: `AgentState::to_msgpack` / `from_msgpack` (rmp-serde, field names kept): 23% smaller than JSON on a 100-entry state, same speed (measured).
- Property tests for the CRDT laws (commutative, associative, idempotent) of every type and of `AgentState`; a deliberately broken `ORSet::merge` fails them. `benches/vs_alternatives.rs` against crdts 7.3 and llm-sync 0.1.1.
- `task_board` example; CONTRIBUTING.md, issue templates, MSRV job in CI.
- `ORMap::remove` (timestamped tombstones; a newer `set` brings a key back, an older one arriving late does not) and `ORMap::iter`.
- `AgentState::pn_counters` (counters that go up and down), read as empty from 0.1.x JSON.
- `GCounter::node_value`, `LWWRegister::writer`; `ClockOrder` is re-exported at the crate root.
- `examples/quickstart.rs`, `examples/shared_text.rs`; the README is the crate docs and its quickstart is a doctest.

### Fixed
- `LWWRegister::merge` was not commutative: on a timestamp tie each replica kept its own value, so two agents that merged each other's state disagreed forever. Ties now go to the higher writer id, then the larger value, and local writes use the same rule.
- `GCounter` overflowed (panic in debug builds, wrap-around in release) near `u64::MAX`; it now saturates. `PNCounter::value` cast `u64` to `i64` and could flip sign; it is now computed exactly and clamped.
- Builds for `wasm32-unknown-unknown` (uuid's `js` feature is enabled for that target; 0.1.x failed to compile there).

### Changed
- `ORMap` stores `Option<String>` per key to represent removals. JSON written by 0.1.x still loads.
- `ORMap::len` counts live keys only.
- Removed unused dependencies (chrono, proptest).
- MSRV is 1.85.

## 0.1.1

- First published version.
