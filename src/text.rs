// SPDX-License-Identifier: MIT
//! Shared text that several agents can edit at the same time (feature `text`).
//!
//! The counters, sets and registers in [`crate::crdt`] cannot merge two edits
//! to the same string: a last-write-wins register keeps one version and drops
//! the other. [`SharedText`] is a sequence CRDT backed by
//! [Yrs](https://crates.io/crates/yrs), the Rust port of
//! [Yjs](https://yjs.dev). Concurrent inserts and deletes from different
//! agents are all kept, and every replica converges to the same text no matter
//! in which order updates arrive. Updates are compact binary messages that are
//! wire-compatible with Yjs, so a browser editor using Yjs can join in.
//!
//! ```
//! use llm_sync::text::SharedText;
//!
//! let mut planner = SharedText::new(1);
//! planner.push("1. research\n");
//! let mut writer = SharedText::from_update(2, &planner.encode_state())?;
//!
//! // Both edit at the same time, without talking to each other.
//! planner.push("2. outline\n");
//! writer.insert(0, "PLAN\n")?;
//!
//! // Exchange only what the other side is missing, in either order.
//! let to_writer = planner.encode_diff(&writer.state_vector())?;
//! let to_planner = writer.encode_diff(&planner.state_vector())?;
//! writer.apply_update(&to_writer)?;
//! planner.apply_update(&to_planner)?;
//!
//! assert_eq!(planner.text(), "PLAN\n1. research\n2. outline\n");
//! assert_eq!(planner.text(), writer.text());
//! # Ok::<(), llm_sync::SyncError>(())
//! ```

use crate::error::SyncError;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{ClientID, Doc, GetString, OffsetKind, Options, ReadTxn, StateVector, Text, TextRef, Transact, Update};

const FIELD: &str = "text";
const MAX_CLIENT_ID: u64 = (1 << 53) - 1;

/// A text document that merges concurrent edits from many agents.
///
/// Positions are byte offsets into the UTF-8 text and must fall on character
/// boundaries; [`insert`](Self::insert) and [`delete`](Self::delete) check this.
pub struct SharedText {
    doc: Doc,
    text: TextRef,
}

impl std::fmt::Debug for SharedText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedText")
            .field("client_id", &self.client_id())
            .field("text", &self.text())
            .finish()
    }
}

fn decode_err(e: impl std::fmt::Display) -> SyncError {
    SyncError::InvalidState(format!("invalid update: {e}"))
}

impl SharedText {
    /// An empty document for one replica. `client_id` must be unique among the
    /// replicas that edit the same document (for example a number per agent)
    /// and below 2^53, the largest integer JavaScript (and so Yjs) can hold;
    /// higher bits are dropped.
    pub fn new(client_id: u64) -> Self {
        let mut options = Options::with_client_id(ClientID::new(client_id & MAX_CLIENT_ID));
        options.offset_kind = OffsetKind::Bytes;
        let doc = Doc::with_options(options);
        let text = doc.get_or_insert_text(FIELD);
        Self { doc, text }
    }

    /// A new replica that starts from another replica's full state
    /// (from [`encode_state`](Self::encode_state)).
    ///
    /// # Errors
    /// [`SyncError::InvalidState`] if `update` is not a valid update.
    pub fn from_update(client_id: u64, update: &[u8]) -> Result<Self, SyncError> {
        let mut s = Self::new(client_id);
        s.apply_update(update)?;
        Ok(s)
    }

    /// This replica's client id.
    pub fn client_id(&self) -> u64 {
        self.doc.client_id().get()
    }

    /// The current text.
    pub fn text(&self) -> String {
        self.text.get_string(&self.doc.transact())
    }

    /// Length of the text in bytes.
    pub fn len(&self) -> usize {
        self.text.len(&self.doc.transact()) as usize
    }

    /// True if the text is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn check_boundary(&self, current: &str, index: usize) -> Result<(), SyncError> {
        if index > current.len() || !current.is_char_boundary(index) {
            return Err(SyncError::InvalidState(format!(
                "position {index} is not a character boundary in a {}-byte text",
                current.len()
            )));
        }
        Ok(())
    }

    /// Insert `chunk` at byte position `index`.
    ///
    /// # Errors
    /// [`SyncError::InvalidState`] if `index` is past the end or inside a character.
    pub fn insert(&mut self, index: usize, chunk: &str) -> Result<(), SyncError> {
        let current = self.text();
        self.check_boundary(&current, index)?;
        let pos = u32::try_from(index).map_err(|_| SyncError::InvalidState("text too long".into()))?;
        let mut txn = self.doc.transact_mut();
        self.text.insert(&mut txn, pos, chunk);
        Ok(())
    }

    /// Append `chunk` at the end.
    pub fn push(&mut self, chunk: &str) {
        let mut txn = self.doc.transact_mut();
        self.text.push(&mut txn, chunk);
    }

    /// Delete `len` bytes starting at byte position `index`.
    ///
    /// # Errors
    /// [`SyncError::InvalidState`] if the range is out of bounds or splits a character.
    pub fn delete(&mut self, index: usize, len: usize) -> Result<(), SyncError> {
        let current = self.text();
        let end = index
            .checked_add(len)
            .ok_or_else(|| SyncError::InvalidState("range overflows".into()))?;
        self.check_boundary(&current, index)?;
        self.check_boundary(&current, end)?;
        let (pos, n) = (
            u32::try_from(index).map_err(|_| SyncError::InvalidState("text too long".into()))?,
            u32::try_from(len).map_err(|_| SyncError::InvalidState("text too long".into()))?,
        );
        let mut txn = self.doc.transact_mut();
        self.text.remove_range(&mut txn, pos, n);
        Ok(())
    }

    /// What this replica has seen, as a compact binary summary. Send it to a
    /// peer so it can reply with only the edits you are missing.
    pub fn state_vector(&self) -> Vec<u8> {
        self.doc.transact().state_vector().encode_v1()
    }

    /// Everything in this replica, as one update another replica can apply.
    pub fn encode_state(&self) -> Vec<u8> {
        self.doc.transact().encode_state_as_update_v1(&StateVector::default())
    }

    /// The edits a peer with state vector `remote_state_vector` has not seen yet.
    ///
    /// # Errors
    /// [`SyncError::InvalidState`] if the state vector does not decode.
    pub fn encode_diff(&self, remote_state_vector: &[u8]) -> Result<Vec<u8>, SyncError> {
        let sv = StateVector::decode_v1(remote_state_vector).map_err(decode_err)?;
        Ok(self.doc.transact().encode_diff_v1(&sv))
    }

    /// Apply an update from another replica. Applying the same update twice,
    /// or updates out of order, is safe.
    ///
    /// # Errors
    /// [`SyncError::InvalidState`] if the update does not decode or apply.
    pub fn apply_update(&mut self, update: &[u8]) -> Result<(), SyncError> {
        let update = Update::decode_v1(update).map_err(decode_err)?;
        self.doc.transact_mut().apply_update(update).map_err(decode_err)
    }

    /// Pull in everything `other` has that this replica does not.
    ///
    /// # Errors
    /// [`SyncError::InvalidState`] if the exchange fails to decode.
    pub fn merge_from(&mut self, other: &SharedText) -> Result<(), SyncError> {
        let diff = other.encode_diff(&self.state_vector())?;
        self.apply_update(&diff)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_inserts_at_same_spot_keep_both_and_converge() {
        let mut a = SharedText::new(1);
        a.push("hello");
        let mut b = SharedText::from_update(2, &a.encode_state()).unwrap();
        a.insert(5, " world").unwrap();
        b.insert(5, " there").unwrap();
        let (a0, b0) = (a.encode_state(), b.encode_state());
        a.apply_update(&b0).unwrap();
        b.apply_update(&a0).unwrap();
        assert_eq!(a.text(), b.text());
        assert!(a.text().contains(" world") && a.text().contains(" there"));
        assert_eq!(a.len(), "hello world there".len());
    }

    #[test]
    fn lww_register_loses_one_edit_but_shared_text_does_not() {
        // The register keeps one of two concurrent edits; SharedText keeps both.
        let mut r1 = crate::LWWRegister::new();
        r1.write("draft + intro".to_string(), 2, "a");
        let mut r2 = crate::LWWRegister::new();
        r2.write("draft + summary".to_string(), 2, "b");
        let merged = r1.merge(&r2);
        let kept = merged.read().cloned().unwrap_or_default();
        assert!(!(kept.contains("intro") && kept.contains("summary")));

        let mut a = SharedText::new(1);
        a.push("draft");
        let mut b = SharedText::from_update(2, &a.encode_state()).unwrap();
        a.push(" + intro");
        b.push(" + summary");
        a.merge_from(&b).unwrap();
        assert!(a.text().contains("intro") && a.text().contains("summary"));
    }

    #[test]
    fn concurrent_delete_and_insert() {
        let mut a = SharedText::new(1);
        a.push("The quick brown fox");
        let mut b = SharedText::from_update(2, &a.encode_state()).unwrap();
        a.delete(4, 6).unwrap(); // "quick "
        b.insert(19, " jumps").unwrap();
        a.merge_from(&b).unwrap();
        b.merge_from(&a).unwrap();
        assert_eq!(a.text(), "The brown fox jumps");
        assert_eq!(a.text(), b.text());
    }

    #[test]
    fn updates_are_idempotent_and_order_free() {
        let mut a = SharedText::new(1);
        a.push("x");
        let u1 = a.encode_state();
        a.push("y");
        let u2 = a.encode_diff(&SharedText::from_update(9, &u1).unwrap().state_vector()).unwrap();
        let mut c = SharedText::new(3);
        c.apply_update(&u2).unwrap(); // later edit first
        c.apply_update(&u1).unwrap();
        c.apply_update(&u1).unwrap(); // twice
        assert_eq!(c.text(), "xy");
    }

    #[test]
    fn rejects_positions_inside_a_character() {
        let mut a = SharedText::new(1);
        a.push("h\u{e9}llo"); // the accented e is 2 bytes
        assert!(a.insert(2, "x").is_err());
        assert!(a.insert(99, "x").is_err());
        assert!(a.delete(1, 1).is_err());
        a.delete(1, 2).unwrap();
        assert_eq!(a.text(), "hllo");
    }

    #[test]
    fn garbage_updates_are_errors_not_panics() {
        let mut a = SharedText::new(1);
        assert!(a.apply_update(&[0xff, 0xfe, 0x01]).is_err());
        assert!(a.encode_diff(&[0xff, 0xff, 0xff, 0xff, 0xff]).is_err());
        assert!(a.is_empty());
    }

    #[test]
    fn client_id_round_trips() {
        assert_eq!(SharedText::new(42).client_id(), 42);
        assert_eq!(SharedText::new(u64::MAX).client_id(), MAX_CLIENT_ID);
    }
}
