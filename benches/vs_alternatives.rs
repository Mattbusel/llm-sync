//! Merge speed against the `crdts` crate and llm-sync 0.1.1, and wire size of
//! JSON versus MessagePack.
//!
//! - GCounter with 100 agents, merge two replicas: llm-sync 0.2, 0.1.1, crdts 7.3
//! - add-wins set with 1,000 elements (10% removed), merge two replicas:
//!   llm-sync `ORSet` vs crdts `Orswot`
//! - `AgentState` with 50 counters and 50 registers: encode + decode as JSON
//!   and as MessagePack (feature `msgpack`)
//!
//! Run: `cargo bench --bench vs_alternatives --features msgpack`
#![allow(clippy::unwrap_used)]

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use crdts::{CmRDT, CvRDT};

fn counters(c: &mut Criterion) {
    let mut g = c.benchmark_group("GCounter merge, 100 agents");
    let (mut a, mut b) = (llm_sync::GCounter::new(), llm_sync::GCounter::new());
    let (mut a1, mut b1) = (llm_sync_011::GCounter::new(), llm_sync_011::GCounter::new());
    let (mut ca, mut cb) = (crdts::GCounter::<String>::new(), crdts::GCounter::<String>::new());
    for i in 0..100u64 {
        let n = format!("agent-{i}");
        a.increment(n.clone(), i);
        b.increment(n.clone(), 100 - i);
        a1.increment(n.clone(), i);
        b1.increment(n.clone(), 100 - i);
        let d = ca.inc_many(n.clone(), i + 1);
        ca.apply(d);
        let d = cb.inc_many(n, 101 - i);
        cb.apply(d);
    }
    g.bench_function("llm-sync 0.2", |bch| bch.iter(|| black_box(a.merge(&b))));
    g.bench_function("llm-sync 0.1.1", |bch| bch.iter(|| black_box(a1.merge(&b1))));
    g.bench_function("crdts 7.3", |bch| {
        bch.iter(|| {
            let mut x = ca.clone();
            x.merge(cb.clone());
            black_box(x)
        })
    });
    g.finish();
}

fn sets(c: &mut Criterion) {
    let mut g = c.benchmark_group("add-wins set merge, 1000 elements");
    let (mut a, mut b) = (llm_sync::ORSet::new(), llm_sync::ORSet::new());
    let (mut ca, mut cb) = (crdts::Orswot::<String, &str>::new(), crdts::Orswot::<String, &str>::new());
    for i in 0..1_000 {
        let item = format!("https://example.com/{i}");
        let (s, cs, actor) = if i % 2 == 0 { (&mut a, &mut ca, "a") } else { (&mut b, &mut cb, "b") };
        s.add(item.clone(), actor);
        let ctx = cs.read_ctx().derive_add_ctx(actor);
        let op = cs.add(item.clone(), ctx);
        cs.apply(op);
        if i % 10 == 0 {
            s.remove(&item);
            let ctx = cs.contains(&item).derive_rm_ctx();
            let op = cs.rm(item, ctx);
            cs.apply(op);
        }
    }
    assert_eq!(a.merge(&b).len(), 900);
    g.bench_function("llm-sync ORSet", |bch| bch.iter(|| black_box(a.merge(&b))));
    g.bench_function("crdts Orswot", |bch| {
        bch.iter(|| {
            let mut x = ca.clone();
            x.merge(cb.clone());
            black_box(x)
        })
    });
    g.finish();
}

fn wire(c: &mut Criterion) {
    let mut s = llm_sync::AgentState::new();
    for i in 0..50u64 {
        let mut g = llm_sync::GCounter::new();
        g.increment(format!("agent-{}", i % 7), i);
        s.counters.insert(format!("tasks_{i}"), g);
        let mut r = llm_sync::LWWRegister::new();
        r.write(format!("value number {i}"), i, format!("agent-{}", i % 7));
        s.registers.insert(format!("reg_{i}"), r);
    }
    let json = s.to_json().unwrap();
    println!("AgentState wire size: JSON {} bytes", json.len());
    let mut g = c.benchmark_group("AgentState encode+decode");
    g.bench_function("JSON", |b| b.iter(|| black_box(llm_sync::AgentState::from_json(&s.to_json().unwrap()).unwrap())));
    #[cfg(feature = "msgpack")]
    {
        let mp = s.to_msgpack().unwrap();
        println!("AgentState wire size: MessagePack {} bytes", mp.len());
        g.bench_function("MessagePack", |b| {
            b.iter(|| black_box(llm_sync::AgentState::from_msgpack(&s.to_msgpack().unwrap()).unwrap()))
        });
    }
    g.finish();
}

criterion_group!(benches, counters, sets, wire);
criterion_main!(benches);
