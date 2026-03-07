// SPDX-License-Identifier: MIT
use thiserror::Error;

/// All errors that can occur in the llm-sync crate.
#[derive(Debug, Error)]
pub enum SyncError {
    /// A node id was not found in the vector clock.
    #[error("Node '{0}' not found in vector clock")]
    NodeNotFound(String),

    /// A CRDT merge conflict that cannot be automatically resolved.
    #[error("Merge conflict: cannot automatically resolve for key '{key}'")]
    MergeConflict { key: String },

    /// JSON serialization or deserialization failed.
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// The system is in an invalid state.
    #[error("Invalid state: {0}")]
    InvalidState(String),

    /// A sync session was not found or has expired.
    #[error("Session '{0}' expired or not found")]
    SessionNotFound(String),
}
