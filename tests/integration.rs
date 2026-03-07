// SPDX-License-Identifier: MIT
use llm_sync::{AgentState, GCounter, GSet, VectorClock};
use llm_sync::crdt::{LWWRegister, ORMap, PNCounter};
use llm_sync::vclock::ClockOrder;

#[test]
fn test_24_agent_counter_merge_total() {
    let mut states: Vec<AgentState> = (0..24).map(|_| AgentState::new()).collect();
    for (i, state) in states.iter_mut().enumerate() {
        let mut g = GCounter::new();
        g.increment(format!("agent{i}"), (i + 1) as u64);
        state.counters.insert("work_units".into(), g);
    }
    let merged = states.iter().skip(1).fold(states[0].clone(), |acc, s| acc.merge(s));
    let total: u64 = (1..=24).sum();
    assert_eq!(merged.counters["work_units"].value(), total);
}

#[test]
fn test_gcounter_merge_associativity() {
    let mut a = GCounter::new(); a.increment("x", 1);
    let mut b = GCounter::new(); b.increment("y", 2);
    let mut c = GCounter::new(); c.increment("z", 3);
    let lhs = a.merge(&b).merge(&c);
    let rhs = a.merge(&b.merge(&c));
    assert_eq!(lhs.value(), rhs.value());
    assert_eq!(lhs.value(), 6);
}

#[test]
fn test_vector_clock_causal_ordering_after_sync() {
    let mut agent1 = VectorClock::new();
    let mut agent2 = VectorClock::new();
    agent1.tick("agent1");
    // agent2 syncs agent1's state then advances
    agent2 = agent2.merge(&agent1);
    agent2.tick("agent2");
    // agent1 < agent2
    assert_eq!(agent1.compare(&agent2), ClockOrder::Before);
}

#[test]
fn test_state_json_roundtrip_preserves_counter_value() {
    let mut state = AgentState::new();
    let mut g = GCounter::new(); g.increment("n1", 42);
    state.counters.insert("calls".into(), g);
    let json = state.to_json().unwrap();
    let restored = AgentState::from_json(&json).unwrap();
    assert_eq!(restored.counters["calls"].value(), 42);
}

#[test]
fn test_pncounter_distributed_merge() {
    let mut a = PNCounter::new(); a.increment("node1", 10); a.decrement("node1", 3);
    let mut b = PNCounter::new(); b.increment("node2", 5);
    let merged = a.merge(&b);
    assert_eq!(merged.value(), 12); // 10 - 3 + 5
}

#[test]
fn test_gset_capabilities_merge_across_agents() {
    let mut s1 = AgentState::new();
    let mut set1 = GSet::new(); set1.insert("rag"); set1.insert("reasoning");
    s1.sets.insert("capabilities".into(), set1);

    let mut s2 = AgentState::new();
    let mut set2 = GSet::new(); set2.insert("reasoning"); set2.insert("tool-use");
    s2.sets.insert("capabilities".into(), set2);

    let merged = s1.merge(&s2);
    let caps = &merged.sets["capabilities"];
    assert!(caps.contains("rag"));
    assert!(caps.contains("reasoning"));
    assert!(caps.contains("tool-use"));
    assert_eq!(caps.len(), 3);
}

#[test]
fn test_ormap_distributed_config_lww() {
    let mut m1 = ORMap::new(); m1.set("model", "gpt-4", 1, "coordinator");
    let mut m2 = ORMap::new(); m2.set("model", "claude-3", 10, "supervisor");
    let merged = m1.merge(&m2);
    assert_eq!(merged.get("model").unwrap(), "claude-3");
}

#[test]
fn test_lww_register_concurrent_write_higher_ts_wins() {
    let mut r1: LWWRegister<String> = LWWRegister::new();
    let mut r2: LWWRegister<String> = LWWRegister::new();
    r1.write("replica-1-value".into(), 5, "r1");
    r2.write("replica-2-value".into(), 8, "r2");
    let merged = r1.merge(&r2);
    assert_eq!(merged.read().unwrap(), "replica-2-value");
}

#[test]
fn test_vclock_concurrent_agents_detected() {
    let mut c1 = VectorClock::new(); c1.tick("agent-a");
    let mut c2 = VectorClock::new(); c2.tick("agent-b");
    assert_eq!(c1.compare(&c2), ClockOrder::Concurrent);
}

#[test]
fn test_agent_state_triple_merge_associativity() {
    let mut s1 = AgentState::new();
    let mut g1 = GCounter::new(); g1.increment("a", 1);
    s1.counters.insert("c".into(), g1);

    let mut s2 = AgentState::new();
    let mut g2 = GCounter::new(); g2.increment("b", 2);
    s2.counters.insert("c".into(), g2);

    let mut s3 = AgentState::new();
    let mut g3 = GCounter::new(); g3.increment("c_node", 3);
    s3.counters.insert("c".into(), g3);

    let lhs = s1.merge(&s2).merge(&s3);
    let rhs = s1.merge(&s2.merge(&s3));
    assert_eq!(lhs.counters["c"].value(), rhs.counters["c"].value());
    assert_eq!(lhs.counters["c"].value(), 6);
}
