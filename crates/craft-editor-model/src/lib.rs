//! Lossless rich-document data and revision-guarded editing commands.
//!
//! The original JSON tree is authoritative. ProseMirror is used for structural
//! validation and position maps, never to serialize away unrecognized metadata.
//! This model checkpoint is not a native editing surface or full editor parity.

#![deny(missing_docs)]

mod document;
mod position;
mod transaction;

pub use document::{Document, Mark, Node};
pub use position::{
    ByteOffset, ScalarOffset, TextRange, Utf16Offset, byte_to_scalar, scalar_to_byte,
    scalar_to_utf16, utf16_to_scalar,
};
pub use transaction::{Change, Command, Editor, MarkMode, PositionMap, Revision, Transaction};

/// A stable, content-free model diagnostic. Hosts provide localized UI messages.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Input is not valid canonical JSON.
    #[error("invalid_json")]
    Json(#[from] serde_json::Error),
    /// Schema or node attributes are invalid; details contain no manuscript text.
    #[error("invalid_document: {0}")]
    InvalidDocument(String),
    /// A requested character boundary does not exist.
    #[error("invalid_position")]
    InvalidPosition,
    /// A stable block identifier could not be resolved.
    #[error("block_not_found")]
    BlockNotFound,
    /// A requested operation cannot safely preserve the selected structure.
    #[error("unsupported_edit")]
    UnsupportedEdit,
    /// Exact expected text differs at the supplied range.
    #[error("expected_text_mismatch")]
    ExpectedTextMismatch,
    /// The transaction was prepared against another revision.
    #[error("stale_revision")]
    StaleRevision,
    /// Revision identity cannot advance further.
    #[error("revision_exhausted")]
    RevisionExhausted,
    /// There is no Undo or Redo entry.
    #[error("history_empty")]
    HistoryEmpty,
    /// A bounded resource limit would be exceeded.
    #[error("resource_limit")]
    ResourceLimit,
}

/// Model operation result.
pub type Result<T> = std::result::Result<T, Error>;
