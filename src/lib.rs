// SPDX-License-Identifier: MIT
//! # llm-sync
//!
//! CRDT and vector clock primitives for distributed LLM agent state synchronization.
//! Every merge operation is commutative, associative, and idempotent.
//!
//! ## Example
//! ```rust
//! use llm_sync::{AgentState, GCounter};
//!
//! let mut s1 = AgentState::new();
//! let mut g = GCounter::new();
//! g.increment("agent-1", 10);
//! s1.counters.insert("requests".into(), g);
//!
//! let s2 = AgentState::new();
//! let merged = s1.merge(&s2);
//! assert_eq!(merged.counters["requests"].value(), 10);
//! ```

pub mod crdt;
pub mod error;
pub mod session;
pub mod vclock;

pub use error::SyncError;
pub use vclock::VectorClock;
pub use crdt::{GCounter, GSet, LWWRegister, ORMap, PNCounter};
pub use session::{AgentState, SessionId};
