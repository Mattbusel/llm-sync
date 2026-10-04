//! Three agents share a task board without a coordinator: an add-wins set of
//! open tasks, a counter of finished work, and a "current plan" register
//! timestamped by hybrid logical clocks. They sync in a random order and
//! still agree.
//!
//! Run with `cargo run --example task_board --features hlc,msgpack`.

use llm_sync::{AgentClock, AgentState, GCounter, LWWRegister, ORSet, SyncError};

fn main() -> Result<(), SyncError> {
    let names = ["planner", "coder", "tester"];
    let clocks: Vec<AgentClock> = names.iter().map(|_| AgentClock::new()).collect();
    let mut states: Vec<AgentState> = names.iter().map(|_| AgentState::new()).collect();

    // planner opens three tasks and writes a plan
    let mut open = ORSet::new();
    for t in ["parse input", "write tests", "ship"] {
        open.add(t, names[0]);
    }
    states[0].or_sets.insert("open".into(), open);
    let mut plan = LWWRegister::new();
    plan.write("v1: tests first".to_string(), clocks[0].now(), names[0]);
    states[0].registers.insert("plan".into(), plan);

    // everyone receives the planner's state (as compact MessagePack)
    let wire = states[0].to_msgpack()?;
    println!("planner's state on the wire: {} bytes MessagePack, {} bytes JSON", wire.len(), states[0].to_json()?.len());
    for i in 1..3 {
        let received = AgentState::from_msgpack(&wire)?;
        clocks[i].observe(received.max_timestamp())?;
        states[i] = states[i].merge(&received);
    }

    // coder finishes a task; tester re-opens it concurrently (add wins)
    if let Some(s) = states[1].or_sets.get_mut("open") {
        s.remove("parse input");
    }
    let mut done = GCounter::new();
    done.increment(names[1], 1);
    states[1].counters.insert("done".into(), done);
    if let Some(s) = states[2].or_sets.get_mut("open") {
        s.add("parse input", names[2]);
    }

    // tester changes the plan after having seen v1
    if let Some(r) = states[2].registers.get_mut("plan") {
        r.write("v2: fuzz the parser too".to_string(), clocks[2].now(), names[2]);
    }

    // sync in two different orders
    let a = states[0].merge(&states[1]).merge(&states[2]);
    let b = states[2].merge(&states[0]).merge(&states[1]);
    let open_a: Vec<&str> = a.or_sets["open"].iter().collect();
    let open_b: Vec<&str> = b.or_sets["open"].iter().collect();
    assert_eq!(open_a, open_b);
    assert_eq!(a.registers["plan"].read(), b.registers["plan"].read());

    println!("open tasks: {open_a:?}");
    println!("done: {}", a.counters["done"].value());
    println!("plan: {:?}", a.registers["plan"].read());
    Ok(())
}
