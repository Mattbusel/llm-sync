//! The README quickstart. Run with `cargo run --example quickstart`.

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
