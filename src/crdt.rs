// SPDX-License-Identifier: MIT
//! CRDT implementations: GCounter, PNCounter, GSet, ORSet, LWWRegister, ORMap (an LWW map with deletes).
//!
//! Every merge operation is commutative, associative, and idempotent.
//! No merge operation ever blocks.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use serde::{Deserialize, Serialize};

// ── GCounter (grow-only counter) ─────────────────────────────────────────────

/// A grow-only counter CRDT.
///
/// Per-node counters are merged by taking the component-wise maximum.
/// The aggregate value is the sum of all per-node counters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GCounter {
    counts: HashMap<String, u64>,
}

impl GCounter {
    /// Create a new, empty GCounter.
    pub fn new() -> Self { Self::default() }

    /// Increment this node's counter by `amount`.
    ///
    /// # Arguments
    /// * `node`: identifier of the node performing the increment.
    /// * `amount`: value to add. The per-node count saturates at `u64::MAX`.
    pub fn increment(&mut self, node: impl Into<String>, amount: u64) {
        let e = self.counts.entry(node.into()).or_insert(0);
        *e = e.saturating_add(amount);
    }

    /// Return the aggregate value across all nodes (saturates at `u64::MAX`).
    pub fn value(&self) -> u64 {
        self.counts.values().fold(0u64, |acc, v| acc.saturating_add(*v))
    }

    /// The count recorded for one node.
    pub fn node_value(&self, node: &str) -> u64 {
        self.counts.get(node).copied().unwrap_or(0)
    }

    /// Merge with another GCounter, taking the component-wise maximum.
    ///
    /// The operation is commutative, associative, and idempotent.
    pub fn merge(&self, other: &GCounter) -> GCounter {
        let mut result = self.counts.clone();
        for (k, &v) in &other.counts {
            let e = result.entry(k.clone()).or_insert(0);
            if v > *e { *e = v; }
        }
        GCounter { counts: result }
    }
}

// ── PNCounter (increment/decrement counter) ──────────────────────────────────

/// A positive-negative counter CRDT that supports both increment and decrement.
///
/// Internally composed of two GCounters: one for increments, one for decrements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PNCounter {
    increments: GCounter,
    decrements: GCounter,
}

impl PNCounter {
    /// Create a new, zeroed PNCounter.
    pub fn new() -> Self { Self::default() }

    /// Increment by `amount` for `node`.
    pub fn increment(&mut self, node: impl Into<String>, amount: u64) {
        self.increments.increment(node, amount);
    }

    /// Decrement by `amount` for `node`.
    pub fn decrement(&mut self, node: impl Into<String>, amount: u64) {
        self.decrements.increment(node, amount);
    }

    /// Return the net value (increments minus decrements), clamped to the `i64` range.
    pub fn value(&self) -> i64 {
        let net = i128::from(self.increments.value()) - i128::from(self.decrements.value());
        net.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
    }

    /// Merge with another PNCounter.
    pub fn merge(&self, other: &PNCounter) -> PNCounter {
        PNCounter {
            increments: self.increments.merge(&other.increments),
            decrements: self.decrements.merge(&other.decrements),
        }
    }
}

// ── GSet (grow-only set) ─────────────────────────────────────────────────────

/// A grow-only set CRDT. Elements can be added but never removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GSet {
    items: HashSet<String>,
}

impl GSet {
    /// Create a new, empty GSet.
    pub fn new() -> Self { Self::default() }

    /// Insert an element.
    pub fn insert(&mut self, item: impl Into<String>) { self.items.insert(item.into()); }

    /// Return true if the element is present.
    pub fn contains(&self, item: &str) -> bool { self.items.contains(item) }

    /// Return the number of elements.
    pub fn len(&self) -> usize { self.items.len() }

    /// Return true if the set is empty.
    pub fn is_empty(&self) -> bool { self.items.is_empty() }

    /// Merge with another GSet (union).
    pub fn merge(&self, other: &GSet) -> GSet {
        GSet { items: self.items.union(&other.items).cloned().collect() }
    }

    /// Iterate over elements.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.items.iter().map(|s| s.as_str())
    }
}

// ── LWWRegister (last-write-wins register) ───────────────────────────────────

/// A last-write-wins register CRDT.
///
/// Every write carries a logical timestamp and a writer id. The write with the
/// higher timestamp wins; ties go to the higher writer id, then to the larger
/// value (compared as JSON). That total order is what makes `merge`
/// commutative: every replica picks the same winner whatever the merge order.
/// Give each writer a distinct id and use increasing timestamps.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(deserialize = "T: serde::de::DeserializeOwned"))]
pub struct LWWRegister<T: Clone + Serialize + serde::de::DeserializeOwned> {
    value: Option<T>,
    timestamp: u64,
    writer: String,
}

impl<T: Clone + Serialize + serde::de::DeserializeOwned> LWWRegister<T> {
    /// Create a new, empty register.
    pub fn new() -> Self {
        Self { value: None, timestamp: 0, writer: String::new() }
    }

    /// Write a value with a logical timestamp.
    ///
    /// # Arguments
    /// * `value`: the value to write.
    /// * `timestamp`: logical timestamp; higher values win.
    /// * `writer`: identifier of the writing node.
    ///
    /// The write is ignored if the current value wins under the register's
    /// ordering (see the type docs), exactly as it would be on merge.
    pub fn write(&mut self, value: T, timestamp: u64, writer: impl Into<String>) {
        let candidate = LWWRegister { value: Some(value), timestamp, writer: writer.into() };
        let never_written = self.value.is_none() && self.timestamp == 0 && self.writer.is_empty();
        if never_written || candidate.beats(self) {
            *self = candidate;
        }
    }

    fn value_key(&self) -> String {
        self.value
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok())
            .unwrap_or_default()
    }

    /// True if `self` wins over `other`.
    fn beats(&self, other: &Self) -> bool {
        (self.timestamp, &self.writer)
            .cmp(&(other.timestamp, &other.writer))
            .then_with(|| self.value_key().cmp(&other.value_key()))
            == std::cmp::Ordering::Greater
    }

    /// Read the current value. Returns `None` if never written.
    pub fn read(&self) -> Option<&T> { self.value.as_ref() }

    /// Merge with another register: the higher timestamp wins, ties are broken
    /// by writer id and then by value, so `a.merge(&b)` equals `b.merge(&a)`.
    pub fn merge(&self, other: &LWWRegister<T>) -> LWWRegister<T> {
        if other.beats(self) {
            other.clone()
        } else {
            self.clone()
        }
    }

    /// Return the current logical timestamp.
    pub fn timestamp(&self) -> u64 { self.timestamp }

    /// Return the id of the writer of the current value.
    pub fn writer(&self) -> &str { &self.writer }
}

impl<T: Clone + Serialize + serde::de::DeserializeOwned> Default for LWWRegister<T> {
    fn default() -> Self { Self::new() }
}

// ── ORMap (last-write-wins map with deletes) ─────────────────────────────────

/// A string-to-string map CRDT where each key is a last-write-wins register.
///
/// Keys can be set, updated and removed. A removal is a timestamped tombstone,
/// so a later `set` brings the key back and an older `set` arriving after the
/// removal does not. (Despite the historical name this is an LWW map, not an
/// add-wins observed-remove map.)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ORMap {
    entries: HashMap<String, LWWRegister<Option<String>>>,
}

impl ORMap {
    /// Create a new, empty ORMap.
    pub fn new() -> Self { Self::default() }

    /// Set a key to a value at the given logical timestamp.
    ///
    /// # Arguments
    /// * `key`: the map key.
    /// * `value`: the string value to store.
    /// * `timestamp`: logical timestamp for LWW resolution.
    /// * `writer`: identifier of the writing agent.
    pub fn set(
        &mut self,
        key: impl Into<String>,
        value: impl Into<String>,
        timestamp: u64,
        writer: impl Into<String>,
    ) {
        let k = key.into();
        self.entries.entry(k).or_default().write(Some(value.into()), timestamp, writer);
    }

    /// Remove a key as of `timestamp`. A `set` with a higher timestamp (from
    /// any replica) brings it back; older sets stay removed.
    pub fn remove(&mut self, key: impl Into<String>, timestamp: u64, writer: impl Into<String>) {
        self.entries.entry(key.into()).or_default().write(None, timestamp, writer);
    }

    /// Get a value by key. Returns `None` if never set or removed.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key)?.read()?.as_deref()
    }

    /// Iterate over live `(key, value)` pairs (in no particular order).
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries
            .iter()
            .filter_map(|(k, r)| r.read()?.as_deref().map(|v| (k.as_str(), v)))
    }

    /// Merge with another ORMap.
    pub fn merge(&self, other: &ORMap) -> ORMap {
        let mut result = self.entries.clone();
        for (k, reg) in &other.entries {
            let entry = result.entry(k.clone()).or_default();
            *entry = entry.merge(reg);
        }
        ORMap { entries: result }
    }

    /// The largest timestamp of any entry (set or removed), 0 if empty.
    pub fn max_timestamp(&self) -> u64 {
        self.entries.values().map(LWWRegister::timestamp).max().unwrap_or(0)
    }

    /// Return the number of live (not removed) keys.
    pub fn len(&self) -> usize { self.iter().count() }

    /// Return true if the map has no live keys.
    pub fn is_empty(&self) -> bool { self.len() == 0 }
}

// ── ORSet (observed-remove set, add wins) ────────────────────────────────────

/// A unique tag for one `add`: the node that added and that node's add count.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Tag {
    /// Node that performed the add.
    pub node: String,
    /// That node's add counter at the time.
    pub seq: u64,
}

/// An observed-remove set: elements can be added and removed, and a remove
/// only cancels the adds it has seen. If one agent removes an element while
/// another adds it again at the same time, the element stays (add wins), which
/// is what you want for "visited", "in progress" or "blocked" lists.
///
/// Each add gets a unique [`Tag`]; a remove records the tags it observed. Removed
/// tags are kept as tombstones so late-arriving state cannot bring them back,
/// which means the set's size grows with the number of adds and removes ever
/// made, not just with its current contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ORSet {
    adds: BTreeMap<String, BTreeSet<Tag>>,
    removed: BTreeSet<Tag>,
    counters: BTreeMap<String, u64>,
}

impl ORSet {
    /// Create a new, empty set.
    pub fn new() -> Self { Self::default() }

    /// Add `item` on behalf of `node` (use one id per agent).
    pub fn add(&mut self, item: impl Into<String>, node: impl Into<String>) {
        let node = node.into();
        let seq = self.counters.entry(node.clone()).or_insert(0);
        *seq = seq.saturating_add(1);
        let tag = Tag { node, seq: *seq };
        self.adds.entry(item.into()).or_default().insert(tag);
    }

    /// Remove `item`: cancels every add of it this replica has seen.
    /// Returns `false` if the item was not present.
    pub fn remove(&mut self, item: &str) -> bool {
        let Some(tags) = self.adds.get(item) else { return false };
        let live: Vec<Tag> = tags.iter().filter(|t| !self.removed.contains(*t)).cloned().collect();
        let was_present = !live.is_empty();
        self.removed.extend(live);
        was_present
    }

    /// True if the element has at least one add that was not removed.
    pub fn contains(&self, item: &str) -> bool {
        self.adds.get(item).is_some_and(|tags| tags.iter().any(|t| !self.removed.contains(t)))
    }

    /// Iterate over present elements, in sorted order.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.adds
            .iter()
            .filter(|(_, tags)| tags.iter().any(|t| !self.removed.contains(t)))
            .map(|(k, _)| k.as_str())
    }

    /// Number of present elements.
    pub fn len(&self) -> usize { self.iter().count() }

    /// True if no element is present.
    pub fn is_empty(&self) -> bool { self.len() == 0 }

    /// Merge with another replica: union of adds, union of removes, and the
    /// per-node maximum of the add counters.
    pub fn merge(&self, other: &ORSet) -> ORSet {
        let mut out = self.clone();
        for (item, tags) in &other.adds {
            out.adds.entry(item.clone()).or_default().extend(tags.iter().cloned());
        }
        out.removed.extend(other.removed.iter().cloned());
        for (node, &n) in &other.counters {
            let e = out.counters.entry(node.clone()).or_insert(0);
            *e = (*e).max(n);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── GCounter ──────────────────────────────────────────────────────────────

    #[test]
    fn test_gcounter_increment_and_value() {
        let mut c = GCounter::new();
        c.increment("a", 3);
        c.increment("b", 2);
        assert_eq!(c.value(), 5);
    }

    #[test]
    fn test_gcounter_merge_is_commutative() {
        let mut c1 = GCounter::new(); c1.increment("a", 5);
        let mut c2 = GCounter::new(); c2.increment("b", 3);
        assert_eq!(c1.merge(&c2).value(), c2.merge(&c1).value());
    }

    #[test]
    fn test_gcounter_merge_is_idempotent() {
        let mut c = GCounter::new(); c.increment("a", 10);
        assert_eq!(c.merge(&c.clone()).value(), c.value());
    }

    #[test]
    fn test_gcounter_merge_is_associative() {
        let mut a = GCounter::new(); a.increment("x", 1);
        let mut b = GCounter::new(); b.increment("y", 2);
        let mut c = GCounter::new(); c.increment("z", 3);
        assert_eq!(a.merge(&b).merge(&c).value(), a.merge(&b.merge(&c)).value());
    }

    #[test]
    fn test_gcounter_merge_takes_max_per_node() {
        let mut c1 = GCounter::new(); c1.increment("a", 10);
        let mut c2 = GCounter::new(); c2.increment("a", 5);
        assert_eq!(c1.merge(&c2).value(), 10);
    }

    #[test]
    fn test_gcounter_new_is_zero() {
        assert_eq!(GCounter::new().value(), 0);
    }

    // ── PNCounter ─────────────────────────────────────────────────────────────

    #[test]
    fn test_pncounter_increment_decrement() {
        let mut c = PNCounter::new();
        c.increment("a", 10);
        c.decrement("a", 3);
        assert_eq!(c.value(), 7);
    }

    #[test]
    fn test_pncounter_merge_is_commutative() {
        let mut c1 = PNCounter::new(); c1.increment("a", 5);
        let mut c2 = PNCounter::new(); c2.decrement("b", 2);
        assert_eq!(c1.merge(&c2).value(), c2.merge(&c1).value());
    }

    #[test]
    fn test_pncounter_merge_is_idempotent() {
        let mut c = PNCounter::new(); c.increment("x", 7);
        assert_eq!(c.merge(&c.clone()).value(), c.value());
    }

    #[test]
    fn test_pncounter_new_is_zero() {
        assert_eq!(PNCounter::new().value(), 0);
    }

    #[test]
    fn test_pncounter_can_go_negative() {
        let mut c = PNCounter::new();
        c.decrement("a", 5);
        assert_eq!(c.value(), -5);
    }

    // ── GSet ──────────────────────────────────────────────────────────────────

    #[test]
    fn test_gset_insert_and_contains() {
        let mut s = GSet::new();
        s.insert("alpha");
        assert!(s.contains("alpha"));
        assert!(!s.contains("beta"));
    }

    #[test]
    fn test_gset_merge_is_commutative() {
        let mut s1 = GSet::new(); s1.insert("a");
        let mut s2 = GSet::new(); s2.insert("b");
        assert_eq!(s1.merge(&s2).len(), s2.merge(&s1).len());
    }

    #[test]
    fn test_gset_merge_is_idempotent() {
        let mut s = GSet::new(); s.insert("x");
        assert_eq!(s.merge(&s.clone()).len(), s.len());
    }

    #[test]
    fn test_gset_merge_union() {
        let mut s1 = GSet::new(); s1.insert("a"); s1.insert("b");
        let mut s2 = GSet::new(); s2.insert("b"); s2.insert("c");
        assert_eq!(s1.merge(&s2).len(), 3);
    }

    #[test]
    fn test_gset_is_empty_on_new() {
        assert!(GSet::new().is_empty());
    }

    #[test]
    fn test_gset_iter_yields_all_elements() {
        let mut s = GSet::new(); s.insert("a"); s.insert("b");
        let mut items: Vec<&str> = s.iter().collect();
        items.sort();
        assert_eq!(items, vec!["a", "b"]);
    }

    // ── LWWRegister ───────────────────────────────────────────────────────────

    #[test]
    fn test_lww_register_write_and_read() {
        let mut r: LWWRegister<String> = LWWRegister::new();
        r.write("hello".into(), 1, "agent-1");
        assert_eq!(r.read().unwrap(), "hello");
    }

    #[test]
    fn test_lww_register_higher_timestamp_wins() {
        let mut r: LWWRegister<String> = LWWRegister::new();
        r.write("first".into(), 1, "a");
        r.write("second".into(), 2, "b");
        assert_eq!(r.read().unwrap(), "second");
    }

    #[test]
    fn test_lww_register_lower_timestamp_ignored() {
        let mut r: LWWRegister<String> = LWWRegister::new();
        r.write("latest".into(), 10, "a");
        r.write("old".into(), 5, "b");
        assert_eq!(r.read().unwrap(), "latest");
    }

    #[test]
    fn test_lww_register_merge_picks_higher_ts() {
        let mut r1: LWWRegister<String> = LWWRegister::new();
        r1.write("old".into(), 1, "a");
        let mut r2: LWWRegister<String> = LWWRegister::new();
        r2.write("new".into(), 5, "b");
        assert_eq!(r1.merge(&r2).read().unwrap(), "new");
    }

    #[test]
    fn test_lww_register_merge_is_commutative() {
        let mut r1: LWWRegister<String> = LWWRegister::new();
        r1.write("v1".into(), 3, "a");
        let mut r2: LWWRegister<String> = LWWRegister::new();
        r2.write("v2".into(), 7, "b");
        assert_eq!(r1.merge(&r2).read(), r2.merge(&r1).read());
    }

    #[test]
    fn test_lww_register_new_is_empty() {
        let r: LWWRegister<String> = LWWRegister::new();
        assert!(r.read().is_none());
        assert_eq!(r.timestamp(), 0);
    }

    // ── ORMap ─────────────────────────────────────────────────────────────────

    #[test]
    fn test_ormap_set_and_get() {
        let mut m = ORMap::new();
        m.set("key", "value", 1, "a");
        assert_eq!(m.get("key").unwrap(), "value");
    }

    #[test]
    fn test_ormap_get_missing_key_returns_none() {
        let m = ORMap::new();
        assert!(m.get("absent").is_none());
    }

    #[test]
    fn test_ormap_merge_lww_semantics() {
        let mut m1 = ORMap::new(); m1.set("k", "v1", 1, "a");
        let mut m2 = ORMap::new(); m2.set("k", "v2", 2, "b");
        assert_eq!(m1.merge(&m2).get("k").unwrap(), "v2");
    }

    #[test]
    fn test_ormap_merge_is_commutative() {
        let mut m1 = ORMap::new(); m1.set("x", "val1", 3, "a");
        let mut m2 = ORMap::new(); m2.set("y", "val2", 1, "b");
        let merged1 = m1.merge(&m2);
        let merged2 = m2.merge(&m1);
        assert_eq!(merged1.get("x"), merged2.get("x"));
        assert_eq!(merged1.get("y"), merged2.get("y"));
    }

    #[test]
    fn test_ormap_merge_is_idempotent() {
        let mut m = ORMap::new(); m.set("k", "v", 5, "a");
        let m2 = m.merge(&m.clone());
        assert_eq!(m2.get("k"), m.get("k"));
    }

    #[test]
    fn test_ormap_is_empty_on_new() {
        assert!(ORMap::new().is_empty());
    }

    #[test]
    fn test_ormap_len_counts_unique_keys() {
        let mut m = ORMap::new();
        m.set("a", "1", 1, "x");
        m.set("b", "2", 2, "x");
        assert_eq!(m.len(), 2);
    }

    // ── 0.2.0 regression tests ───────────────────────────────────────────────

    #[test]
    fn test_lww_merge_tie_is_commutative() {
        // 0.1.x kept the local value on a timestamp tie, so two replicas that
        // merged each other ended up with different values.
        let mut r1: LWWRegister<String> = LWWRegister::new();
        r1.write("from a".into(), 5, "agent-a");
        let mut r2: LWWRegister<String> = LWWRegister::new();
        r2.write("from b".into(), 5, "agent-b");
        assert_eq!(r1.merge(&r2).read(), r2.merge(&r1).read());
        assert_eq!(r1.merge(&r2).read().map(String::as_str), Some("from b"));
        // Same writer and timestamp, different values: still deterministic.
        let mut r3: LWWRegister<String> = LWWRegister::new();
        r3.write("x".into(), 5, "w");
        let mut r4: LWWRegister<String> = LWWRegister::new();
        r4.write("y".into(), 5, "w");
        assert_eq!(r3.merge(&r4).read(), r4.merge(&r3).read());
    }

    #[test]
    fn test_lww_local_write_agrees_with_merge() {
        // A local write must lose exactly when it would lose on merge,
        // otherwise replicas diverge.
        let mut a: LWWRegister<String> = LWWRegister::new();
        a.write("z-value".into(), 5, "z");
        let b = a.clone();
        a.write("a-value".into(), 5, "a"); // loses the tie to writer "z"
        assert_eq!(a.read(), b.merge(&a).read());
        assert_eq!(a.read().map(String::as_str), Some("z-value"));
        assert_eq!(a.writer(), "z");
    }

    #[test]
    fn test_gcounter_does_not_overflow() {
        let mut c = GCounter::new();
        c.increment("a", u64::MAX);
        c.increment("a", 1);
        c.increment("b", 5);
        assert_eq!(c.node_value("a"), u64::MAX);
        assert_eq!(c.value(), u64::MAX);
    }

    #[test]
    fn test_pncounter_value_clamps() {
        let mut c = PNCounter::new();
        c.decrement("a", u64::MAX);
        assert_eq!(c.value(), i64::MIN);
        let mut d = PNCounter::new();
        d.increment("a", u64::MAX);
        assert_eq!(d.value(), i64::MAX);
    }

    #[test]
    fn test_ormap_remove_and_readd() {
        let mut m = ORMap::new();
        m.set("k", "v1", 1, "a");
        m.remove("k", 2, "a");
        assert_eq!(m.get("k"), None);
        assert!(m.is_empty());
        // An older set arriving late does not resurrect the key.
        let mut late = ORMap::new();
        late.set("k", "stale", 1, "b");
        assert_eq!(m.merge(&late).get("k"), None);
        assert_eq!(late.merge(&m).get("k"), None);
        // A newer set does.
        let mut newer = ORMap::new();
        newer.set("k", "v3", 3, "b");
        assert_eq!(m.merge(&newer).get("k"), Some("v3"));
        assert_eq!(m.merge(&newer).iter().collect::<Vec<_>>(), vec![("k", "v3")]);
    }

    #[test]
    fn test_ormap_reads_0_1_json() {
        // JSON written by 0.1.x (plain string values) still loads.
        let old = r#"{"entries":{"k":{"value":"v","timestamp":3,"writer":"a"}}}"#;
        let m: ORMap = serde_json::from_str(old).unwrap();
        assert_eq!(m.get("k"), Some("v"));
    }

    #[test]
    fn test_ormap_tombstone_survives_json_roundtrip() {
        let mut m = ORMap::new();
        m.set("k", "v", 1, "a");
        m.remove("k", 5, "a");
        let mut back: ORMap = serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        back.set("k", "older", 3, "b"); // older than the removal
        assert_eq!(back.get("k"), None);
    }

    #[test]
    fn test_orset_add_remove_readd() {
        let mut s = ORSet::new();
        s.add("url-a", "n1");
        assert!(s.contains("url-a"));
        assert!(s.remove("url-a"));
        assert!(!s.contains("url-a"));
        assert!(!s.remove("url-a"));
        s.add("url-a", "n1");
        assert!(s.contains("url-a"));
        assert_eq!(s.iter().collect::<Vec<_>>(), vec!["url-a"]);
    }

    #[test]
    fn test_orset_concurrent_add_wins_over_remove() {
        let mut a = ORSet::new();
        a.add("task-7", "a");
        let mut b = a.clone();
        a.remove("task-7"); // a finishes the task
        b.add("task-7", "b"); // b re-opens it at the same time
        let m1 = a.merge(&b);
        let m2 = b.merge(&a);
        assert_eq!(m1, m2);
        assert!(m1.contains("task-7"));
    }

    #[test]
    fn test_orset_remove_is_not_undone_by_stale_state() {
        let mut a = ORSet::new();
        a.add("x", "a");
        let stale = a.clone();
        a.remove("x");
        assert!(!a.merge(&stale).contains("x"));
        assert!(!stale.merge(&a).contains("x"));
    }
}
