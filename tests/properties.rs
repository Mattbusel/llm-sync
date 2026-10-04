//! CRDT laws for every type, and cross-checks against the `crdts` crate.
#![allow(clippy::unwrap_used)]

use llm_sync::{AgentState, GCounter, GSet, LWWRegister, ORMap, ORSet, PNCounter, VectorClock};
use proptest::prelude::*;

fn node() -> impl Strategy<Value = String> {
    prop::sample::select(vec!["a", "b", "c"]).prop_map(String::from)
}
fn item() -> impl Strategy<Value = String> {
    prop::sample::select(vec!["x", "y", "z", "w"]).prop_map(String::from)
}

fn gcounter() -> impl Strategy<Value = GCounter> {
    prop::collection::vec((node(), 0u64..1_000), 0..6).prop_map(|ops| {
        let mut c = GCounter::new();
        for (n, k) in ops {
            c.increment(n, k);
        }
        c
    })
}
fn pncounter() -> impl Strategy<Value = PNCounter> {
    prop::collection::vec((node(), 0u64..1_000, any::<bool>()), 0..6).prop_map(|ops| {
        let mut c = PNCounter::new();
        for (n, k, up) in ops {
            if up { c.increment(n, k) } else { c.decrement(n, k) }
        }
        c
    })
}
fn gset() -> impl Strategy<Value = GSet> {
    prop::collection::vec(item(), 0..5).prop_map(|v| {
        let mut s = GSet::new();
        for i in v {
            s.insert(i);
        }
        s
    })
}
fn orset() -> impl Strategy<Value = ORSet> {
    prop::collection::vec((item(), node(), any::<bool>()), 0..8).prop_map(|ops| {
        let mut s = ORSet::new();
        for (i, n, add) in ops {
            if add { s.add(i, n) } else { s.remove(&i); }
        }
        s
    })
}
fn lww() -> impl Strategy<Value = LWWRegister<String>> {
    prop::collection::vec((item(), 0u64..4, node()), 0..4).prop_map(|ops| {
        let mut r = LWWRegister::new();
        for (v, t, n) in ops {
            r.write(v, t, n);
        }
        r
    })
}
fn ormap() -> impl Strategy<Value = ORMap> {
    prop::collection::vec((item(), item(), 0u64..4, node(), any::<bool>()), 0..6).prop_map(|ops| {
        let mut m = ORMap::new();
        for (k, v, t, n, set) in ops {
            if set { m.set(k, v, t, n) } else { m.remove(k, t, n) }
        }
        m
    })
}
fn vclock() -> impl Strategy<Value = VectorClock> {
    prop::collection::vec(node(), 0..6).prop_map(|ticks| {
        let mut c = VectorClock::new();
        for n in ticks {
            c.tick(n);
        }
        c
    })
}

/// Observable state of a map (so we compare what users can read).
fn map_view(m: &ORMap) -> Vec<(String, String)> {
    let mut v: Vec<_> = m.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    v.sort();
    v
}
fn set_view(s: &ORSet) -> Vec<String> {
    s.iter().map(str::to_string).collect()
}

macro_rules! laws {
    ($name:ident, $strat:expr, $view:expr) => {
        proptest! {
            #![proptest_config(ProptestConfig::with_cases(300))]
            #[test]
            fn $name(a in $strat, b in $strat, c in $strat) {
                let view = $view;
                prop_assert_eq!(view(&a.merge(&b)), view(&b.merge(&a)), "commutative");
                prop_assert_eq!(view(&a.merge(&b).merge(&c)), view(&a.merge(&b.merge(&c))), "associative");
                prop_assert_eq!(view(&a.merge(&a)), view(&a), "idempotent");
            }
        }
    };
}

laws!(gcounter_laws, gcounter(), |x: &GCounter| x.value());
laws!(pncounter_laws, pncounter(), |x: &PNCounter| x.value());
laws!(gset_laws, gset(), |x: &GSet| { let mut v: Vec<String> = x.iter().map(str::to_string).collect(); v.sort(); v });
laws!(orset_laws, orset(), set_view);
laws!(lww_laws, lww(), |x: &LWWRegister<String>| x.read().cloned());
laws!(ormap_laws, ormap(), map_view);
laws!(vclock_laws, vclock(), |x: &VectorClock| x.clone());

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// AgentState merge is order-independent too, and survives JSON.
    #[test]
    fn agent_state_laws(c1 in gcounter(), c2 in gcounter(), s1 in orset(), s2 in orset(), m1 in ormap(), m2 in ormap()) {
        let mut a = AgentState::new();
        a.counters.insert("n".into(), c1);
        a.or_sets.insert("s".into(), s1);
        a.maps.insert("m".into(), m1);
        let mut b = AgentState::new();
        b.counters.insert("n".into(), c2);
        b.or_sets.insert("s".into(), s2);
        b.maps.insert("m".into(), m2);
        let ab = a.merge(&b);
        let ba = b.merge(&a);
        prop_assert_eq!(ab.counters["n"].value(), ba.counters["n"].value());
        prop_assert_eq!(set_view(&ab.or_sets["s"]), set_view(&ba.or_sets["s"]));
        prop_assert_eq!(map_view(&ab.maps["m"]), map_view(&ba.maps["m"]));
        let back = AgentState::from_json(&ab.to_json().unwrap()).unwrap();
        prop_assert_eq!(set_view(&back.or_sets["s"]), set_view(&ab.or_sets["s"]));
    }
}

// ── cross-checks against the crdts crate ─────────────────────────────────────

#[derive(Debug, Clone)]
enum Op {
    Add(usize, String),
    Remove(usize, String),
    Sync(usize, usize),
}

fn ops() -> impl Strategy<Value = Vec<Op>> {
    let op = prop_oneof![
        (0usize..3, item()).prop_map(|(r, i)| Op::Add(r, i)),
        (0usize..3, item()).prop_map(|(r, i)| Op::Remove(r, i)),
        (0usize..3, 0usize..3).prop_map(|(a, b)| Op::Sync(a, b)),
    ];
    prop::collection::vec(op, 0..40)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// Same random history of adds, removes and partial syncs on three
    /// replicas: ORSet and crdts::Orswot (both add-wins) agree on every replica.
    #[test]
    fn orset_matches_crdts_orswot(history in ops()) {
        use crdts::{CmRDT, CvRDT};
        let actors = ["a", "b", "c"];
        let mut ours = vec![ORSet::new(); 3];
        let mut theirs: Vec<crdts::Orswot<String, &str>> = vec![crdts::Orswot::new(); 3];
        for op in history {
            match op {
                Op::Add(r, i) => {
                    ours[r].add(i.clone(), actors[r]);
                    let ctx = theirs[r].read_ctx().derive_add_ctx(actors[r]);
                    let o = theirs[r].add(i, ctx);
                    theirs[r].apply(o);
                }
                Op::Remove(r, i) => {
                    ours[r].remove(&i);
                    let ctx = theirs[r].contains(&i).derive_rm_ctx();
                    let o = theirs[r].rm(i, ctx);
                    theirs[r].apply(o);
                }
                Op::Sync(to, from) => {
                    ours[to] = ours[to].merge(&ours[from]);
                    let other = theirs[from].clone();
                    theirs[to].merge(other);
                }
            }
            for r in 0..3 {
                let mut t: Vec<String> = theirs[r].read().val.into_iter().collect();
                t.sort();
                prop_assert_eq!(set_view(&ours[r]), t, "replica {}", r);
            }
        }
    }

    #[test]
    fn gcounter_matches_crdts(incs in prop::collection::vec((0usize..3, 1u64..100), 0..30)) {
        use crdts::{CmRDT, CvRDT};
        let actors = ["a", "b", "c"];
        let mut ours = vec![GCounter::new(); 3];
        let mut theirs: Vec<crdts::GCounter<&str>> = vec![crdts::GCounter::new(); 3];
        for (r, k) in incs {
            ours[r].increment(actors[r], k);
            let d = theirs[r].inc_many(actors[r], k);
            theirs[r].apply(d);
        }
        let all = ours.iter().fold(GCounter::new(), |acc, c| acc.merge(c));
        let mut t = crdts::GCounter::new();
        for c in theirs { t.merge(c); }
        prop_assert_eq!(all.value().to_string(), t.read().to_string());
    }
}
