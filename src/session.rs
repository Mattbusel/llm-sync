// SPDX-License-Identifier: MIT
//! Sync session management for agent-to-agent state exchange.
//!
//! ## Responsibility
//! Encapsulate the full shared CRDT state for a sync session, including
//! vector clocks, counters, sets, registers, and maps. Provide a single
//! `merge` entry point that delegates to each CRDT's merge operation.
//!
//! ## Guarantees
//! - All merges are commutative, associative, and idempotent.
//! - JSON round-trip is lossless.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::crdt::{GCounter, GSet, LWWRegister, ORMap, ORSet, PNCounter};
use crate::error::SyncError;
use crate::vclock::VectorClock;

/// Unique identifier for a sync session.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub String);

impl SessionId {
    /// Generate a fresh random session id.
    pub fn new() -> Self { Self(Uuid::new_v4().to_string()) }
}

impl Default for SessionId {
    fn default() -> Self { Self::new() }
}

/// The full CRDT-backed shared state for a sync session.
///
/// Each field is a named map of CRDTs, allowing arbitrary named slots
/// without requiring schema changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    /// Unique session identifier.
    pub session_id: SessionId,
    /// Vector clock tracking causal history across agents.
    pub vector_clock: VectorClock,
    /// Named grow-only counters.
    pub counters: HashMap<String, GCounter>,
    /// Named counters that can go up and down.
    #[serde(default)]
    pub pn_counters: HashMap<String, PNCounter>,
    /// Named grow-only sets.
    pub sets: HashMap<String, GSet>,
    /// Named sets with removal (add wins).
    #[serde(default)]
    pub or_sets: HashMap<String, ORSet>,
    /// Named last-write-wins string registers.
    pub registers: HashMap<String, LWWRegister<String>>,
    /// Named observed-remove maps.
    pub maps: HashMap<String, ORMap>,
}

impl Default for AgentState {
    fn default() -> Self {
        Self {
            session_id: SessionId::new(),
            vector_clock: VectorClock::new(),
            counters: HashMap::new(),
            pn_counters: HashMap::new(),
            sets: HashMap::new(),
            or_sets: HashMap::new(),
            registers: HashMap::new(),
            maps: HashMap::new(),
        }
    }
}

impl AgentState {
    /// Create a new AgentState with a fresh random session id.
    pub fn new() -> Self { Self::default() }

    /// Override the session id (builder pattern).
    pub fn with_session(mut self, id: SessionId) -> Self {
        self.session_id = id;
        self
    }

    /// Merge another agent's state into this one.
    ///
    /// All merges are CRDT-compliant: commutative, associative, idempotent.
    ///
    /// # Returns
    /// A new `AgentState` representing the merged view.
    pub fn merge(&self, other: &AgentState) -> AgentState {
        let mut result = self.clone();
        result.vector_clock = self.vector_clock.merge(&other.vector_clock);

        for (k, v) in &other.counters {
            result.counters
                .entry(k.clone())
                .and_modify(|existing| *existing = existing.merge(v))
                .or_insert_with(|| v.clone());
        }
        for (k, v) in &other.pn_counters {
            result.pn_counters
                .entry(k.clone())
                .and_modify(|existing| *existing = existing.merge(v))
                .or_insert_with(|| v.clone());
        }
        for (k, v) in &other.sets {
            result.sets
                .entry(k.clone())
                .and_modify(|existing| *existing = existing.merge(v))
                .or_insert_with(|| v.clone());
        }
        for (k, v) in &other.or_sets {
            result.or_sets
                .entry(k.clone())
                .and_modify(|existing| *existing = existing.merge(v))
                .or_insert_with(|| v.clone());
        }
        for (k, v) in &other.registers {
            result.registers
                .entry(k.clone())
                .and_modify(|existing| *existing = existing.merge(v))
                .or_insert_with(|| v.clone());
        }
        for (k, v) in &other.maps {
            result.maps
                .entry(k.clone())
                .and_modify(|existing| *existing = existing.merge(v))
                .or_insert_with(|| v.clone());
        }
        result
    }

    /// Serialize this state to JSON.
    ///
    /// # Returns
    /// - `Ok(String)`: JSON representation.
    /// - `Err(SyncError::Serialization)`: on failure.
    pub fn to_json(&self) -> Result<String, SyncError> {
        serde_json::to_string(self).map_err(SyncError::Serialization)
    }

    /// Serialize this state to MessagePack (feature `msgpack`): the same data
    /// as [`to_json`](Self::to_json) in a binary format that is usually
    /// smaller and faster to parse. Field names are kept, so states written by
    /// an older version still load.
    ///
    /// # Errors
    /// [`SyncError::InvalidState`] if encoding fails.
    #[cfg(feature = "msgpack")]
    #[cfg_attr(docsrs, doc(cfg(feature = "msgpack")))]
    pub fn to_msgpack(&self) -> Result<Vec<u8>, SyncError> {
        rmp_serde::to_vec_named(self).map_err(|e| SyncError::InvalidState(format!("msgpack encode: {e}")))
    }

    /// Load a state written by [`to_msgpack`](Self::to_msgpack).
    ///
    /// # Errors
    /// [`SyncError::InvalidState`] if the bytes are not a valid state.
    #[cfg(feature = "msgpack")]
    #[cfg_attr(docsrs, doc(cfg(feature = "msgpack")))]
    pub fn from_msgpack(bytes: &[u8]) -> Result<Self, SyncError> {
        rmp_serde::from_slice(bytes).map_err(|e| SyncError::InvalidState(format!("msgpack decode: {e}")))
    }

    /// The largest LWW timestamp in this state (registers and map entries).
    /// Pass it to a hybrid logical clock (`AgentClock::observe`, feature
    /// `hlc`) after merging remote state, so your next write sorts after
    /// everything you have seen.
    pub fn max_timestamp(&self) -> u64 {
        let regs = self.registers.values().map(|r| r.timestamp());
        let maps = self.maps.values().map(|m| m.max_timestamp());
        regs.chain(maps).max().unwrap_or(0)
    }

    /// Deserialize state from JSON.
    ///
    /// # Returns
    /// - `Ok(AgentState)` on success.
    /// - `Err(SyncError::Serialization)` on parse failure.
    pub fn from_json(s: &str) -> Result<Self, SyncError> {
        serde_json::from_str(s).map_err(SyncError::Serialization)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_state_merge_counters_commutative() {
        let mut s1 = AgentState::new();
        let mut g1 = GCounter::new(); g1.increment("a1", 5);
        s1.counters.insert("req_count".into(), g1);

        let mut s2 = AgentState::new();
        let mut g2 = GCounter::new(); g2.increment("a2", 3);
        s2.counters.insert("req_count".into(), g2);

        let m1 = s1.merge(&s2);
        let m2 = s2.merge(&s1);
        assert_eq!(m1.counters["req_count"].value(), m2.counters["req_count"].value());
        assert_eq!(m1.counters["req_count"].value(), 8);
    }

    #[test]
    fn test_agent_state_merge_is_idempotent() {
        let mut s = AgentState::new();
        let mut g = GCounter::new(); g.increment("x", 10);
        s.counters.insert("c".into(), g);
        let m = s.merge(&s.clone());
        assert_eq!(m.counters["c"].value(), 10);
    }

    #[test]
    fn test_agent_state_json_roundtrip_preserves_session_id() {
        let s = AgentState::new();
        let json = s.to_json().unwrap();
        let decoded = AgentState::from_json(&json).unwrap();
        assert_eq!(decoded.session_id, s.session_id);
    }

    #[test]
    fn test_agent_state_from_json_invalid_returns_error() {
        let err = AgentState::from_json("not json").unwrap_err();
        assert!(matches!(err, SyncError::Serialization(_)));
    }

    #[test]
    fn test_agent_state_merge_sets_union() {
        let mut s1 = AgentState::new();
        let mut set1 = GSet::new(); set1.insert("capability-a");
        s1.sets.insert("capabilities".into(), set1);

        let mut s2 = AgentState::new();
        let mut set2 = GSet::new(); set2.insert("capability-b");
        s2.sets.insert("capabilities".into(), set2);

        let merged = s1.merge(&s2);
        let caps = &merged.sets["capabilities"];
        assert!(caps.contains("capability-a"));
        assert!(caps.contains("capability-b"));
    }

    #[test]
    fn test_agent_state_merge_registers_lww() {
        let mut s1 = AgentState::new();
        let mut r1: LWWRegister<String> = LWWRegister::new();
        r1.write("model-a".into(), 1, "agent1");
        s1.registers.insert("current_model".into(), r1);

        let mut s2 = AgentState::new();
        let mut r2: LWWRegister<String> = LWWRegister::new();
        r2.write("model-b".into(), 10, "agent2");
        s2.registers.insert("current_model".into(), r2);

        let merged = s1.merge(&s2);
        assert_eq!(merged.registers["current_model"].read().unwrap(), "model-b");
    }

    #[test]
    fn test_agent_state_merge_propagates_new_keys_from_other() {
        let s1 = AgentState::new();
        let mut s2 = AgentState::new();
        let mut g = GCounter::new(); g.increment("n", 7);
        s2.counters.insert("new_key".into(), g);

        let merged = s1.merge(&s2);
        assert_eq!(merged.counters["new_key"].value(), 7);
    }

    #[test]
    fn test_agent_state_with_session_overrides_id() {
        let id = SessionId("custom".into());
        let s = AgentState::new().with_session(id.clone());
        assert_eq!(s.session_id, id);
    }

    #[test]
    fn test_agent_state_merge_maps_lww() {
        let mut s1 = AgentState::new();
        let mut m1 = ORMap::new(); m1.set("config", "v1", 1, "a");
        s1.maps.insert("cfg".into(), m1);

        let mut s2 = AgentState::new();
        let mut m2 = ORMap::new(); m2.set("config", "v2", 5, "b");
        s2.maps.insert("cfg".into(), m2);

        let merged = s1.merge(&s2);
        assert_eq!(merged.maps["cfg"].get("config").unwrap(), "v2");
    }

    #[test]
    fn test_session_id_default_is_unique() {
        let a = SessionId::new();
        let b = SessionId::new();
        assert_ne!(a.0, b.0);
    }

    #[test]
    fn test_agent_state_merges_pn_counters_and_reads_old_json() {
        let mut s1 = AgentState::new();
        let mut p = PNCounter::new();
        p.increment("a", 5);
        s1.pn_counters.insert("open_tasks".into(), p);
        let mut s2 = AgentState::new();
        let mut q = PNCounter::new();
        q.decrement("b", 2);
        s2.pn_counters.insert("open_tasks".into(), q);
        assert_eq!(s1.merge(&s2).pn_counters["open_tasks"].value(), 3);
        assert_eq!(s2.merge(&s1).pn_counters["open_tasks"].value(), 3);

        // JSON from 0.1.x has no pn_counters field.
        let mut json: serde_json::Value = serde_json::from_str(&s1.to_json().unwrap()).unwrap();
        json.as_object_mut().unwrap().remove("pn_counters");
        let old = AgentState::from_json(&json.to_string()).unwrap();
        assert!(old.pn_counters.is_empty());
    }

    #[test]
    fn test_agent_state_merges_or_sets() {
        let mut a = AgentState::new();
        let mut s = ORSet::new();
        s.add("blocked:task-1", "a");
        a.or_sets.insert("flags".into(), s.clone());
        let mut b = AgentState::new();
        s.remove("blocked:task-1");
        b.or_sets.insert("flags".into(), s);
        assert!(!a.merge(&b).or_sets["flags"].contains("blocked:task-1"));
        assert!(!b.merge(&a).or_sets["flags"].contains("blocked:task-1"));
    }

    #[test]
    fn test_max_timestamp() {
        let mut a = AgentState::new();
        assert_eq!(a.max_timestamp(), 0);
        let mut r = LWWRegister::new();
        r.write("x".to_string(), 7, "n");
        a.registers.insert("r".into(), r);
        let mut m = ORMap::new();
        m.set("k", "v", 9, "n");
        a.maps.insert("m".into(), m);
        assert_eq!(a.max_timestamp(), 9);
    }

    #[cfg(feature = "msgpack")]
    #[test]
    fn test_msgpack_roundtrip_is_smaller_than_json() {
        let mut a = AgentState::new();
        for i in 0..20 {
            let mut g = GCounter::new();
            g.increment(format!("agent-{i}"), i);
            a.counters.insert(format!("c{i}"), g);
        }
        let bytes = a.to_msgpack().unwrap();
        let back = AgentState::from_msgpack(&bytes).unwrap();
        assert_eq!(back.counters["c7"].value(), 7);
        assert!(bytes.len() < a.to_json().unwrap().len());
        assert!(AgentState::from_msgpack(&[0xc1, 0x00]).is_err());
    }
}
