// SPDX-License-Identifier: MIT
#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]

#[cfg(feature = "hlc")]
#[cfg_attr(docsrs, doc(cfg(feature = "hlc")))]
pub mod clock;
pub mod crdt;
/// The error type.
pub mod error;
pub mod session;
#[cfg(feature = "text")]
#[cfg_attr(docsrs, doc(cfg(feature = "text")))]
pub mod text;
pub mod vclock;

#[cfg(feature = "hlc")]
pub use clock::AgentClock;
pub use crdt::{GCounter, GSet, LWWRegister, ORMap, ORSet, PNCounter};
pub use error::SyncError;
pub use session::{AgentState, SessionId};
#[cfg(feature = "text")]
pub use text::SharedText;
pub use vclock::{ClockOrder, VectorClock};
