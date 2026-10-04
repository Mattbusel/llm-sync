// SPDX-License-Identifier: MIT
use criterion::{criterion_group, criterion_main, Criterion};
use llm_sync::{AgentState, GCounter, VectorClock};

fn bench_gcounter_merge(c: &mut Criterion) {
    let mut c1 = GCounter::new();
    let mut c2 = GCounter::new();
    for i in 0..100u64 {
        c1.increment(format!("node{i}"), i);
        c2.increment(format!("node{}", i + 100), i);
    }
    c.bench_function("gcounter_merge_100_nodes", |b| b.iter(|| c1.merge(&c2)));
}

fn bench_vclock_compare(c: &mut Criterion) {
    let mut v1 = VectorClock::new();
    let mut v2 = VectorClock::new();
    for i in 0..50 {
        v1.tick(format!("a{i}"));
        v2.tick(format!("b{i}"));
    }
    c.bench_function("vclock_compare_50_nodes", |b| b.iter(|| v1.compare(&v2)));
}

fn bench_agent_state_merge(c: &mut Criterion) {
    let mut s1 = AgentState::new();
    let mut s2 = AgentState::new();
    let mut g = GCounter::new();
    g.increment("x", 100);
    s1.counters.insert("c".into(), g.clone());
    s2.counters.insert("c".into(), g);
    c.bench_function("agent_state_merge", |b| b.iter(|| s1.merge(&s2)));
}

criterion_group!(benches, bench_gcounter_merge, bench_vclock_compare, bench_agent_state_merge);
criterion_main!(benches);
