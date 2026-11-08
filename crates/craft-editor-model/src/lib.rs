//! Lossless rich-document data and revision-guarded editing commands.
//!
//! The original JSON tree is authoritative. ProseMirror is used for structural
//! validation and position maps, never to serialize away unrecognized metadata.
//! This model checkpoint is not a native editing surface or full editor parity.

#![deny(missing_docs)]

mod document;
mod fitting;
mod lists;
mod position;
mod structural;
mod table_paste;
mod tables;
mod text_projection;
mod transaction;

pub use document::{Document, Mark, Node, is_allowed_link_uri};
pub use fitting::{FittedMap, plan_node_slice_replacement, plan_slice_replacement};
pub use lists::{
    ListKind, ListStyle, exit_empty_list_item, indent_list_items, join_adjacent_lists,
    lift_list_items, lift_list_items_with_ids, outdent_list_items, outdent_list_items_with_ids,
    split_list_item, toggle_list, toggle_list_with_ids, wrap_list,
};
pub use position::{
    ByteOffset, ScalarOffset, TextRange, Utf16Offset, byte_to_scalar, scalar_to_byte,
    scalar_to_utf16, utf16_to_scalar,
};
pub use structural::{NodePath, NodeRef};
pub use table_paste::{PastedCells, table_cells_from_slice};
pub use tables::{
    CellAlignment, CellRect, CellSelection, TableGrid, delete_table_column, delete_table_row,
    empty_table_cell, insert_table_column, insert_table_row, merge_table_cells,
    set_table_cell_alignment, set_table_column_width, split_table_cell,
};
pub use text_projection::{BlockTextProjection, InlineAtomText, TextPoint, TextProjectionOptions};
pub use transaction::{
    Change, Command, Editor, HistoryReceipt, MarkMode, PositionMap, ReplayAction, ReplayProof,
    Revision, Transaction,
};

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
    /// A revision-local structural reference no longer matches its exact node.
    #[error("node_reference_conflict")]
    NodeReferenceConflict,
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
