// SPDX-License-Identifier: MIT
//! Vector clock for causal ordering in distributed agent systems.
//!
//! ## Responsibility
//! Provide a per-node logical timestamp that allows agents to establish causal
//! ordering of events without relying on synchronized wall clocks.
//!
//! ## Guarantees
//! - Merge is commutative, associative, and idempotent.
//! - `tick` is non-blocking and O(1).
//! - `compare` is O(n) in the number of distinct nodes.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// A vector clock: maps node_id to logical timestamp.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct VectorClock {
    clock: HashMap<String, u64>,
}

/// The causal ordering relationship between two vector clocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockOrder {
    /// `self` happened before `other` (self ≤ other, strict on at least one component).
    Before,
    /// `self` happened after `other`.
    After,
    /// Neither dominates; events are concurrent.
    Concurrent,
    /// The clocks are identical.
    Equal,
}

impl VectorClock {
    /// Create a new, empty vector clock.
    pub fn new() -> Self { Self::default() }

    /// Increment this node's logical timestamp by 1.
    ///
    /// # Arguments
    /// * `node` — Identifier for the node performing the event.
    pub fn tick(&mut self, node: impl Into<String>) {
        let n = node.into();
        *self.clock.entry(n).or_insert(0) += 1;
    }

    /// Get a node's current timestamp. Returns 0 for unknown nodes.
    ///
    /// # Arguments
    /// * `node` — Node identifier to query.
    pub fn get(&self, node: &str) -> u64 {
        *self.clock.get(node).unwrap_or(&0)
    }

    /// Merge two clocks, taking the component-wise maximum.
    ///
    /// The operation is commutative, associative, and idempotent.
    ///
    /// # Returns
    /// A new `VectorClock` representing the merged state.
    pub fn merge(&self, other: &VectorClock) -> VectorClock {
        let mut result = self.clock.clone();
        for (node, &ts) in &other.clock {
            let entry = result.entry(node.clone()).or_insert(0);
            if ts > *entry { *entry = ts; }
        }
        VectorClock { clock: result }
    }

    /// Compare two clocks to determine causal ordering.
    ///
    /// # Returns
    /// A [`ClockOrder`] variant describing the relationship.
    pub fn compare(&self, other: &VectorClock) -> ClockOrder {
        let self_nodes: std::collections::HashSet<&str> =
            self.clock.keys().map(|s| s.as_str()).collect();
        let other_nodes: std::collections::HashSet<&str> =
            other.clock.keys().map(|s| s.as_str()).collect();
        let all_nodes: std::collections::HashSet<&str> =
            self_nodes.union(&other_nodes).copied().collect();

        let mut self_less = false;
        let mut other_less = false;

        for node in all_nodes {
            let s = self.get(node);
            let o = other.get(node);
            if s < o { self_less = true; }
            if s > o { other_less = true; }
        }

        match (self_less, other_less) {
            (false, false) => ClockOrder::Equal,
            (true, false) => ClockOrder::Before,
            (false, true) => ClockOrder::After,
            (true, true) => ClockOrder::Concurrent,
        }
    }

    /// Return the number of distinct nodes tracked by this clock.
    pub fn node_count(&self) -> usize { self.clock.len() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vclock_tick_increments_counter() {
        let mut c = VectorClock::new();
        c.tick("a");
        c.tick("a");
        assert_eq!(c.get("a"), 2);
    }

    #[test]
    fn test_vclock_get_unknown_node_returns_zero() {
        let c = VectorClock::new();
        assert_eq!(c.get("nobody"), 0);
    }

    #[test]
    fn test_vclock_merge_takes_max_component() {
        let mut c1 = VectorClock::new();
        c1.tick("a"); c1.tick("a"); // a=2
        let mut c2 = VectorClock::new();
        c2.tick("a"); c2.tick("b"); // a=1, b=1
        let merged = c1.merge(&c2);
        assert_eq!(merged.get("a"), 2);
        assert_eq!(merged.get("b"), 1);
    }

    #[test]
    fn test_vclock_merge_is_commutative() {
        let mut c1 = VectorClock::new();
        c1.tick("x"); c1.tick("x");
        let mut c2 = VectorClock::new();
        c2.tick("y");
        let m1 = c1.merge(&c2);
        let m2 = c2.merge(&c1);
        assert_eq!(m1, m2);
    }

    #[test]
    fn test_vclock_merge_is_idempotent() {
        let mut c = VectorClock::new();
        c.tick("a");
        let m = c.merge(&c.clone());
        assert_eq!(m, c);
    }

    #[test]
    fn test_vclock_merge_is_associative() {
        let mut a = VectorClock::new(); a.tick("a");
        let mut b = VectorClock::new(); b.tick("b");
        let mut c = VectorClock::new(); c.tick("c");
        let lhs = a.merge(&b).merge(&c);
        let rhs = a.merge(&b.merge(&c));
        assert_eq!(lhs, rhs);
    }

    #[test]
    fn test_vclock_compare_before() {
        let c1 = VectorClock::new();
        let mut c2 = VectorClock::new();
        c2.tick("a");
        assert_eq!(c1.compare(&c2), ClockOrder::Before);
    }

    #[test]
    fn test_vclock_compare_after() {
        let mut c1 = VectorClock::new();
        c1.tick("a");
        let c2 = VectorClock::new();
        assert_eq!(c1.compare(&c2), ClockOrder::After);
    }

    #[test]
    fn test_vclock_compare_concurrent() {
        let mut c1 = VectorClock::new();
        c1.tick("a");
        let mut c2 = VectorClock::new();
        c2.tick("b");
        assert_eq!(c1.compare(&c2), ClockOrder::Concurrent);
    }

    #[test]
    fn test_vclock_compare_equal_empty() {
        let c1 = VectorClock::new();
        let c2 = VectorClock::new();
        assert_eq!(c1.compare(&c2), ClockOrder::Equal);
    }

    #[test]
    fn test_vclock_compare_equal_nonempty() {
        let mut c1 = VectorClock::new();
        c1.tick("x"); c1.tick("y");
        let c2 = c1.clone();
        assert_eq!(c1.compare(&c2), ClockOrder::Equal);
    }

    #[test]
    fn test_vclock_node_count() {
        let mut c = VectorClock::new();
        c.tick("a"); c.tick("b");
        assert_eq!(c.node_count(), 2);
    }
}
