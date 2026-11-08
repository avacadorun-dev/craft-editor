use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use prosemirror::transform::{Mappable, StepMap};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    Document, Error, Mark, Node, NodePath, NodeRef, Result, ScalarOffset, TextRange,
    document::validate_mark, scalar_to_byte,
};

/// Monotonic document revision identity, independent of history depth.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Revision(#[doc = "Monotonic revision number."] pub u64);

/// Explicit formatting semantics, independent of the UI or MCP caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarkMode {
    /// Add missing mark types, preserving existing same-type attributes.
    Add,
    /// Remove supplied mark types; their attributes are irrelevant.
    Remove,
    /// Replace the selected text's entire mark set.
    Replace,
}

/// A generic operation applied through one atomic canonical engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Command {
    /// Replace one exact guarded node, including the root, as one canonical edit.
    ReplaceNode {
        /// Complete expected node at its current path.
        target: NodeRef,
        /// Complete replacement retaining every host-required field.
        replacement: Box<Node>,
    },
    /// Replace an inline scalar range with canonical inline nodes, not Markdown.
    ReplaceInline {
        /// Exact paragraph/heading/code-block snapshot.
        target: NodeRef,
        /// Inclusive scalar boundary in the shared inline projection.
        from: ScalarOffset,
        /// Exclusive scalar boundary.
        to: ScalarOffset,
        /// Canonical inline slice, with explicit marks and atom attributes.
        children: Vec<Node>,
    },
    /// Insert complete children at a guarded parent, including nested/inline atoms.
    InsertChildren {
        /// Exact expected parent in the sequential intermediate state.
        parent: NodeRef,
        /// Insertion child ordinal, including the end.
        index: usize,
        /// Complete canonical child trees.
        children: Vec<Node>,
    },
    /// Replace a half-open child range at a guarded parent.
    ReplaceChildren {
        /// Exact expected parent.
        parent: NodeRef,
        /// Inclusive child ordinal.
        from: usize,
        /// Exclusive child ordinal.
        to: usize,
        /// Complete replacement child trees.
        children: Vec<Node>,
    },
    /// Delete a half-open child range, subject to full schema validation.
    DeleteChildren {
        /// Exact expected parent.
        parent: NodeRef,
        /// Inclusive child ordinal.
        from: usize,
        /// Exclusive child ordinal.
        to: usize,
    },
    /// Move children preserving identities, metadata and original relative order.
    MoveChildren {
        /// Exact expected source parent.
        source: NodeRef,
        /// Inclusive source child ordinal.
        from: usize,
        /// Exclusive source child ordinal.
        to: usize,
        /// Exact destination parent before removal, checked with source together.
        destination: NodeRef,
        /// Destination ordinal before source removal; inside the same range is a no-op.
        index: usize,
    },
    /// Insert ordered canonical blocks at a top-level block ordinal.
    InsertBlocks {
        /// Zero-based insertion ordinal, including the end of the document.
        index: usize,
        /// Complete canonical blocks with application-supplied stable IDs.
        blocks: Vec<Node>,
    },
    /// Replace literal text at a revision-pinned scalar range in a text block.
    ReplaceText {
        /// Exact half-open scalar range.
        range: TextRange,
        /// Text that must currently occupy that range.
        expected: String,
        /// Literal replacement without implicit Markdown conversion.
        replacement: String,
    },
    /// Format a nonempty scalar range in one text block.
    ApplyMarks {
        /// Exact half-open scalar range.
        range: TextRange,
        /// Marks or mark types, interpreted by mode.
        marks: Vec<Mark>,
        /// Explicit additive, removal or replacement behavior.
        mode: MarkMode,
    },
    /// Patch supplied block attributes, preserving every unmentioned field.
    SetBlockAttrs {
        /// Stable block identity.
        block_id: String,
        /// Attribute fields to insert or replace; null is an explicit value.
        attrs: Map<String, Value>,
    },
    /// Split a text block, retaining left identity and inherited provenance.
    SplitBlock {
        /// Stable identity of the block to split.
        block_id: String,
        /// Scalar boundary in that block's projection.
        at: ScalarOffset,
        /// Caller-supplied independent identity for the right block.
        new_block_id: String,
    },
    /// Join adjacent sibling text blocks without dropping conflicting metadata.
    JoinBlocks {
        /// Identity retained by the joined block.
        left_id: String,
        /// Adjacent right sibling whose content is appended.
        right_id: String,
    },
}

/// Commands use sequential intermediate states; the entire batch commits once.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transaction {
    /// Exact revision read by the caller.
    pub base_revision: Revision,
    /// Ordered editing operations; any error leaves document/history unchanged.
    pub commands: Vec<Command>,
}

/// A scalar-space mapping produced by an actual successful operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PositionMap {
    /// Whole-node replacement invalidates paths and reports exact block survival.
    NodeReplacement {
        /// Replaced node path; paths inside this subtree require recapture.
        path: NodePath,
        /// Stable IDs no longer present in the replaced subtree.
        removed_ids: Vec<String>,
        /// Newly present stable IDs.
        inserted_ids: Vec<String>,
        /// Retained IDs whose inline text changed without an exact scalar map.
        invalidated_ids: Vec<String>,
    },
    /// Inline runs were rebuilt without changing scalar positions.
    InlineRebuild {
        /// Identified text block whose marks/run boundaries changed.
        block_id: String,
    },
    /// Structural child replacement, using paths from before this operation.
    Children {
        /// Parent path, unchanged by its own child replacement.
        parent: NodePath,
        /// Inclusive former child ordinal.
        from: usize,
        /// Exclusive former child ordinal.
        to: usize,
        /// Number of inserted children.
        inserted: usize,
        /// Stable IDs in all removed subtrees, including descendants.
        removed_ids: Vec<String>,
        /// Stable IDs in all inserted subtrees, including descendants.
        inserted_ids: Vec<String>,
    },
    /// Structural move; moved node paths follow their actual new location.
    MoveChildren {
        /// Source parent path before the operation.
        source_parent: NodePath,
        /// Inclusive source ordinal.
        from: usize,
        /// Exclusive source ordinal.
        to: usize,
        /// Destination parent before removal.
        destination_parent: NodePath,
        /// Destination ordinal before removal.
        index: usize,
        /// Stable IDs preserved by this move, including descendant blocks.
        moved_ids: Vec<String>,
    },
    /// Inline replacement in one block; maps are expressed in Unicode scalars.
    Text {
        /// Block identity.
        block_id: String,
        /// Inclusive original start.
        start: usize,
        /// Original scalar count.
        old_len: usize,
        /// Replacement scalar count.
        new_len: usize,
    },
    /// A right suffix moves to a newly identified sibling.
    Split {
        /// Original/left identity.
        block_id: String,
        /// Split scalar boundary.
        at: usize,
        /// New right identity.
        new_block_id: String,
    },
    /// Right content moves into the surviving left block.
    Join {
        /// Surviving left identity.
        left_id: String,
        /// Former right identity.
        right_id: String,
        /// Former left scalar length.
        left_len: usize,
    },
}

impl PositionMap {
    /// Map a node path. Deleted/replaced targets detach; moved targets follow.
    /// Scalar/run/split/join maps conservatively invalidate paths; recapture refs.
    /// Block-local scalar carets use map_existing_caret instead.
    pub fn map_path(&self, path: &NodePath) -> Option<NodePath> {
        match self {
            Self::NodeReplacement { path: replaced, .. } => {
                if path.0.starts_with(&replaced.0) {
                    None
                } else {
                    Some(path.clone())
                }
            }
            Self::Children {
                parent,
                from,
                to,
                inserted,
                ..
            } => crate::structural::map_splice_path(path, parent, *from, *to, *inserted),
            Self::MoveChildren {
                source_parent,
                from,
                to,
                destination_parent,
                index,
                ..
            } => crate::structural::map_move_path(
                path,
                source_parent,
                *from,
                *to,
                destination_parent,
                *index,
            ),
            _ => None,
        }
    }
    /// Map a caret, explicitly detaching blocks removed by structural replacement.
    /// A surviving mapped range still needs current-text and revision validation.
    pub fn map_existing_caret(
        &self,
        block_id: &str,
        offset: ScalarOffset,
        right_bias: bool,
    ) -> Option<(String, ScalarOffset)> {
        if let Self::NodeReplacement {
            removed_ids,
            invalidated_ids,
            ..
        } = self
            && (removed_ids.iter().any(|id| id == block_id)
                || invalidated_ids.iter().any(|id| id == block_id))
        {
            return None;
        }
        if let Self::Children { removed_ids, .. } = self
            && removed_ids.iter().any(|id| id == block_id)
        {
            return None;
        }
        Some(self.map_caret(block_id, offset, right_bias))
    }

    /// Map a block-local caret with explicit left(-1)/right(1) boundary bias.
    /// This does not prove an attached quote's semantic validity after edits.
    pub fn map_caret(
        &self,
        block_id: &str,
        offset: ScalarOffset,
        right_bias: bool,
    ) -> (String, ScalarOffset) {
        match self {
            Self::Text {
                block_id: changed,
                start,
                old_len,
                new_len,
            } if changed == block_id => {
                let map = StepMap::new(vec![*start, *old_len, *new_len]);
                (
                    block_id.to_owned(),
                    ScalarOffset(map.map(offset.0, if right_bias { 1 } else { -1 })),
                )
            }
            Self::Split {
                block_id: changed,
                at,
                new_block_id,
            } if changed == block_id && (offset.0 > *at || (offset.0 == *at && right_bias)) => {
                (new_block_id.clone(), ScalarOffset(offset.0 - at))
            }
            Self::Join {
                left_id,
                right_id,
                left_len,
            } if right_id == block_id => (
                left_id.clone(),
                ScalarOffset(left_len.saturating_add(offset.0)),
            ),
            _ => (block_id.to_owned(), offset),
        }
    }
}

/// An acknowledged atomic state transition, including exact before/after trees.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    /// Previously acknowledged revision.
    pub before_revision: Revision,
    /// New monotonic revision; unchanged for a no-op.
    pub after_revision: Revision,
    /// Canonical state before the operation, available for host transactions.
    pub before: Document,
    /// Canonical state after the operation, available for host transactions.
    pub after: Document,
    /// Ordered scalar maps; Undo/Redo return none and provide exact snapshots.
    pub mappings: Vec<PositionMap>,
    replay: ReplayProof,
}

impl Change {
    /// Immutable engine provenance, independent of the public display snapshots
    /// and maps. It cannot be deserialized or constructed by an external caller.
    pub fn replay_proof(&self) -> ReplayProof {
        self.replay.clone()
    }
}

/// Exact in-process history-entry identity, distinct even for identical JSON.
/// Hosts retain clones while attachment history is reachable. Receipts are not
/// portable identifiers and deliberately have no serialization implementation.
#[derive(Debug, Clone)]
pub struct HistoryReceipt(Arc<Document>);

impl PartialEq for HistoryReceipt {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for HistoryReceipt {}
impl Hash for HistoryReceipt {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).hash(state);
    }
}
impl HistoryReceipt {
    /// Immutable canonical tree belonging to this exact history entry.
    pub fn document(&self) -> &Document {
        &self.0
    }
    /// Ownership count for bounded host receipt-cache reclamation. This is a
    /// liveness hint only, never an authorization or snapshot-validity check.
    pub fn reference_count(&self) -> usize {
        Arc::strong_count(&self.0)
    }
}

/// Provenance category generated only by the canonical editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayAction {
    /// Recompute maps by replaying private, validated typed commands.
    Apply,
    /// Restore a specific previously retained history receipt.
    Undo,
    /// Reapply a specific previously undone history receipt.
    Redo,
}

/// Native command provenance. Public Change fields are not trusted authority.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayProof {
    before: HistoryReceipt,
    after: HistoryReceipt,
    before_revision: Revision,
    action: ReplayAction,
    commands: Vec<Command>,
}
impl ReplayProof {
    /// Exact immutable input state/history identity.
    pub fn before(&self) -> &HistoryReceipt {
        &self.before
    }
    /// Exact immutable output state/history identity.
    pub fn after(&self) -> &HistoryReceipt {
        &self.after
    }
    /// Whether this is forward editing or a specific history traversal.
    pub fn action(&self) -> ReplayAction {
        self.action
    }
    /// Validate against the actual host canonical base and recompute forward maps.
    /// Undo/Redo return no guessed inverse maps: the host must restore the exact
    /// target receipt's attachment snapshot, refusing an unknown history receipt.
    pub fn verify(&self, expected: &Document) -> Result<(Document, Vec<PositionMap>)> {
        if expected != self.before.document() {
            return Err(Error::NodeReferenceConflict);
        }
        if self.action != ReplayAction::Apply {
            return Ok((self.after.document().clone(), vec![]));
        }
        let mut editor = Editor::new(expected.clone(), self.before_revision).with_history_limit(0);
        let change = editor.apply(Transaction {
            base_revision: self.before_revision,
            commands: self.commands.clone(),
        })?;
        if &change.after != self.after.document() {
            return Err(Error::NodeReferenceConflict);
        }
        Ok((change.after, change.mappings))
    }
}

/// One document-level editing session with guarded, bounded snapshot history.
/// Host annotation/selection snapshots must be committed alongside [`Change`].
#[derive(Debug)]
pub struct Editor {
    document: Document,
    receipt: HistoryReceipt,
    revision: Revision,
    undo: VecDeque<HistoryReceipt>,
    redo: Vec<HistoryReceipt>,
    history_limit: usize,
}

impl Editor {
    /// Create a session from validated canonical state and its host revision.
    pub fn new(document: Document, revision: Revision) -> Self {
        Self {
            receipt: HistoryReceipt(Arc::new(document.clone())),
            document,
            revision,
            undo: VecDeque::new(),
            redo: Vec::new(),
            history_limit: 100,
        }
    }
    /// Limit retained Undo entries. Zero disables history without disabling edits.
    pub fn with_history_limit(mut self, limit: usize) -> Self {
        self.history_limit = limit;
        self
    }
    /// Borrow current acknowledged canonical state.
    pub fn document(&self) -> &Document {
        &self.document
    }
    /// Current monotonic revision.
    pub fn revision(&self) -> Revision {
        self.revision
    }
    /// Whether a manual Undo is available.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    /// Whether a manual Redo is available.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    /// Apply a complete guarded batch, leaving canonical/history untouched on error.
    pub fn apply(&mut self, transaction: Transaction) -> Result<Change> {
        self.check_revision(transaction.base_revision)?;
        if transaction.commands.len() > 1_000 {
            return Err(Error::ResourceLimit);
        }
        let mut candidate = self.document.clone();
        let mut mappings = Vec::new();
        let commands = transaction.commands;
        for command in commands.iter().cloned() {
            apply_command(&mut candidate, command, &mut mappings)?;
            candidate.promote_schema()?;
            candidate.validate()?;
        }
        let before = self.document.clone();
        let before_receipt = self.receipt.clone();
        let before_revision = self.revision;
        if candidate != self.document {
            let revision = self.next_revision()?;
            if self.history_limit > 0 {
                self.undo.push_back(self.receipt.clone());
                while self.undo.len() > self.history_limit {
                    self.undo.pop_front();
                }
            }
            self.redo.clear();
            self.document = candidate;
            self.receipt = HistoryReceipt(Arc::new(self.document.clone()));
            self.revision = revision;
        } else {
            mappings.clear();
        }
        Ok(Change {
            before_revision,
            after_revision: self.revision,
            before,
            after: self.document.clone(),
            mappings,
            replay: ReplayProof {
                before: before_receipt,
                after: self.receipt.clone(),
                before_revision,
                action: ReplayAction::Apply,
                commands,
            },
        })
    }
    /// Undo by creating a new revision; stale calls never consume history.
    pub fn undo(&mut self, expected_revision: Revision) -> Result<Change> {
        self.check_revision(expected_revision)?;
        let revision = self.next_revision()?;
        let previous = self.undo.pop_back().ok_or(Error::HistoryEmpty)?;
        let before = std::mem::replace(&mut self.document, previous.document().clone());
        let before_receipt = std::mem::replace(&mut self.receipt, previous);
        self.redo.push(before_receipt.clone());
        let before_revision = std::mem::replace(&mut self.revision, revision);
        Ok(Change {
            before_revision,
            after_revision: revision,
            before,
            after: self.document.clone(),
            mappings: vec![],
            replay: ReplayProof {
                before: before_receipt,
                after: self.receipt.clone(),
                before_revision,
                action: ReplayAction::Undo,
                commands: vec![],
            },
        })
    }
    /// Redo a previously undone batch as another new revision.
    pub fn redo(&mut self, expected_revision: Revision) -> Result<Change> {
        self.check_revision(expected_revision)?;
        let revision = self.next_revision()?;
        let next = self.redo.pop().ok_or(Error::HistoryEmpty)?;
        let before = std::mem::replace(&mut self.document, next.document().clone());
        let before_receipt = std::mem::replace(&mut self.receipt, next);
        if self.history_limit > 0 {
            self.undo.push_back(before_receipt.clone());
        }
        let before_revision = std::mem::replace(&mut self.revision, revision);
        Ok(Change {
            before_revision,
            after_revision: revision,
            before,
            after: self.document.clone(),
            mappings: vec![],
            replay: ReplayProof {
                before: before_receipt,
                after: self.receipt.clone(),
                before_revision,
                action: ReplayAction::Redo,
                commands: vec![],
            },
        })
    }
    fn check_revision(&self, revision: Revision) -> Result<()> {
        if revision == self.revision {
            Ok(())
        } else {
            Err(Error::StaleRevision)
        }
    }
    fn next_revision(&self) -> Result<Revision> {
        self.revision
            .0
            .checked_add(1)
            .map(Revision)
            .ok_or(Error::RevisionExhausted)
    }
}

fn apply_command(
    document: &mut Document,
    command: Command,
    mappings: &mut Vec<PositionMap>,
) -> Result<()> {
    match command {
        Command::ReplaceNode {
            target,
            replacement,
        } => crate::structural::replace_node(document, target, *replacement, mappings)?,
        Command::ReplaceInline {
            target,
            from,
            to,
            children,
        } => {
            crate::structural::check_ref(document, &target)?;
            let node = crate::structural::at_path_mut(&mut document.root, &target.path)
                .ok_or(Error::InvalidPosition)?;
            require_text_block(node)?;
            if from > to {
                return Err(Error::InvalidPosition);
            }
            let old_ids = crate::structural::block_ids(node.children())?;
            let (prefix, rest) = split_inline(node.children(), from.0)?;
            let (_, suffix) = split_inline(&rest, to.0 - from.0)?;
            let new_len = children.iter().try_fold(0usize, |length, node| {
                length
                    .checked_add(inline_len(node)?)
                    .ok_or(Error::ResourceLimit)
            })?;
            node.content = Some(coalesce_text(
                prefix.into_iter().chain(children).chain(suffix).collect(),
            ));
            let new_ids = crate::structural::block_ids(node.children())?;
            let removed_ids = old_ids
                .iter()
                .filter(|id| !new_ids.contains(id))
                .cloned()
                .collect::<Vec<_>>();
            let inserted_ids = new_ids
                .iter()
                .filter(|id| !old_ids.contains(id))
                .cloned()
                .collect::<Vec<_>>();
            if let Some(id) = node.block_id() {
                mappings.push(PositionMap::Text {
                    block_id: id.into(),
                    start: from.0,
                    old_len: to.0 - from.0,
                    new_len,
                });
            } else {
                mappings.push(PositionMap::InlineRebuild {
                    block_id: String::new(),
                });
            }
            if !removed_ids.is_empty() || !inserted_ids.is_empty() {
                mappings.push(PositionMap::NodeReplacement {
                    path: target.path,
                    removed_ids,
                    inserted_ids,
                    invalidated_ids: vec![],
                });
            }
        }
        Command::InsertChildren {
            parent,
            index,
            children,
        } => {
            crate::structural::splice_children(document, parent, index, index, children, mappings)?
        }
        Command::ReplaceChildren {
            parent,
            from,
            to,
            children,
        } => crate::structural::splice_children(document, parent, from, to, children, mappings)?,
        Command::DeleteChildren { parent, from, to } => {
            crate::structural::splice_children(document, parent, from, to, vec![], mappings)?
        }
        Command::MoveChildren {
            source,
            from,
            to,
            destination,
            index,
        } => crate::structural::move_children(
            document,
            source,
            from,
            to,
            destination,
            index,
            mappings,
        )?,
        Command::InsertBlocks { index, blocks } => {
            let existing = document.root.content.get_or_insert_default();
            if index > existing.len() {
                return Err(Error::InvalidPosition);
            }
            existing.splice(index..index, blocks);
        }
        Command::ReplaceText {
            range,
            expected,
            replacement,
        } => {
            let node = document.block_mut(&range.block_id)?;
            require_text_block(node)?;
            let projection = node.inline_text();
            let (start, end) = byte_range(&projection, &range)?;
            if projection[start..end] != expected {
                return Err(Error::ExpectedTextMismatch);
            }
            if expected == replacement {
                return Ok(());
            }
            let (prefix, rest) = split_inline(node.children(), range.start.0)?;
            let (_, suffix) = split_inline(&rest, range.end.0 - range.start.0)?;
            let inherited = insertion_leaf(node.children(), range.start.0);
            let mut content = prefix;
            if !replacement.is_empty() {
                let mut leaf = inherited.unwrap_or_else(|| Node::text(""));
                leaf.text = Some(replacement.clone());
                content.push(leaf);
            }
            content.extend(suffix);
            node.content = Some(coalesce_text(content));
            mappings.push(PositionMap::Text {
                block_id: range.block_id,
                start: range.start.0,
                old_len: range.end.0 - range.start.0,
                new_len: replacement.chars().count(),
            });
        }
        Command::ApplyMarks { range, marks, mode } => {
            if range.end <= range.start {
                return Err(Error::InvalidPosition);
            }
            for mark in &marks {
                if mode != MarkMode::Remove {
                    validate_mark(mark)?;
                } else if !["bold", "italic", "underline", "strike", "link", "code"]
                    .contains(&mark.mark_type.as_str())
                {
                    return Err(Error::InvalidDocument("mark_type".into()));
                }
            }
            let node = document.block_mut(&range.block_id)?;
            require_text_block(node)?;
            byte_range(&node.inline_text(), &range)?;
            let (prefix, rest) = split_inline(node.children(), range.start.0)?;
            let (mut selected, suffix) = split_inline(&rest, range.end.0 - range.start.0)?;
            for leaf in &mut selected {
                if leaf.node_type != "text" {
                    return Err(Error::UnsupportedEdit);
                }
                let current = leaf.marks.get_or_insert_default();
                match mode {
                    MarkMode::Add => {
                        for mark in &marks {
                            if !current
                                .iter()
                                .any(|existing| existing.mark_type == mark.mark_type)
                            {
                                current.push(mark.clone());
                            }
                        }
                    }
                    MarkMode::Remove => current.retain(|existing| {
                        !marks
                            .iter()
                            .any(|mark| existing.mark_type == mark.mark_type)
                    }),
                    MarkMode::Replace => *current = marks.clone(),
                }
            }
            node.content = Some(coalesce_text(
                prefix.into_iter().chain(selected).chain(suffix).collect(),
            ));
            mappings.push(PositionMap::InlineRebuild {
                block_id: range.block_id,
            });
        }
        Command::SetBlockAttrs { block_id, attrs } => {
            document
                .block_mut(&block_id)?
                .attrs
                .get_or_insert_default()
                .extend(attrs);
        }
        Command::SplitBlock {
            block_id,
            at,
            new_block_id,
        } => {
            if new_block_id.is_empty() || document.block(&new_block_id).is_ok() {
                return Err(Error::InvalidDocument("duplicate_block_id".into()));
            }
            let children =
                sibling_children(&mut document.root, &block_id).ok_or(Error::BlockNotFound)?;
            let index = children
                .iter()
                .position(|node| node.block_id() == Some(&block_id))
                .ok_or(Error::BlockNotFound)?;
            let node = &children[index];
            require_text_block(node)?;
            let (left, right) = split_inline(node.children(), at.0)?;
            let mut sibling = node.clone();
            sibling.content = Some(right);
            sibling
                .attrs
                .get_or_insert_default()
                .insert("blockId".into(), Value::String(new_block_id.clone()));
            children[index].content = Some(left);
            children.insert(index + 1, sibling);
            mappings.push(PositionMap::Split {
                block_id,
                at: at.0,
                new_block_id,
            });
        }
        Command::JoinBlocks { left_id, right_id } => {
            let children =
                sibling_children(&mut document.root, &left_id).ok_or(Error::BlockNotFound)?;
            let index = children
                .iter()
                .position(|node| node.block_id() == Some(&left_id))
                .ok_or(Error::BlockNotFound)?;
            let right = children.get(index + 1).ok_or(Error::InvalidPosition)?;
            if right.block_id() != Some(right_id.as_str()) {
                return Err(Error::InvalidPosition);
            }
            require_text_block(&children[index])?;
            require_text_block(right)?;
            if !join_compatible(&children[index], right) {
                return Err(Error::UnsupportedEdit);
            }
            let left_len = children[index].inline_text().chars().count();
            let right = children.remove(index + 1);
            let left = &mut children[index];
            if let Some(refs) = right
                .attrs
                .as_ref()
                .and_then(|attrs| attrs.get("sourceRefs"))
                .and_then(Value::as_array)
            {
                let existing = left
                    .attrs
                    .get_or_insert_default()
                    .entry("sourceRefs")
                    .or_insert(Value::Array(vec![]));
                let array = existing
                    .as_array_mut()
                    .ok_or_else(|| Error::InvalidDocument("source_refs".into()))?;
                for reference in refs {
                    if !array.contains(reference) {
                        array.push(reference.clone());
                    }
                }
            }
            let content = left.content.get_or_insert_default();
            content.extend(right.content.unwrap_or_default());
            *content = coalesce_text(std::mem::take(content));
            mappings.push(PositionMap::Join {
                left_id,
                right_id,
                left_len,
            });
        }
    }
    Ok(())
}
fn require_text_block(node: &Node) -> Result<()> {
    if ["paragraph", "heading", "codeBlock"].contains(&node.node_type.as_str()) {
        Ok(())
    } else {
        Err(Error::UnsupportedEdit)
    }
}
fn byte_range(text: &str, range: &TextRange) -> Result<(usize, usize)> {
    if range.start > range.end {
        return Err(Error::InvalidPosition);
    }
    Ok((
        scalar_to_byte(text, range.start)?.0,
        scalar_to_byte(text, range.end)?.0,
    ))
}
fn inline_len(node: &Node) -> Result<usize> {
    match node.node_type.as_str() {
        "text" => Ok(node.text.as_deref().unwrap_or_default().chars().count()),
        "hardBreak" | "inlineMath" => Ok(1),
        _ => Err(Error::UnsupportedEdit),
    }
}
fn split_inline(nodes: &[Node], at: usize) -> Result<(Vec<Node>, Vec<Node>)> {
    let mut left = Vec::new();
    let mut right = Vec::new();
    let mut cursor = 0;
    for node in nodes {
        let length = inline_len(node)?;
        let end = cursor + length;
        if at <= cursor {
            right.push(node.clone());
        } else if at >= end {
            left.push(node.clone());
        } else {
            if node.node_type != "text" {
                return Err(Error::InvalidPosition);
            }
            let text = node.text.as_deref().ok_or(Error::UnsupportedEdit)?;
            let byte = scalar_to_byte(text, ScalarOffset(at - cursor))?.0;
            let mut prefix = node.clone();
            prefix.text = Some(text[..byte].to_owned());
            let mut suffix = node.clone();
            suffix.text = Some(text[byte..].to_owned());
            left.push(prefix);
            right.push(suffix);
        }
        cursor = end;
    }
    if at > cursor {
        return Err(Error::InvalidPosition);
    }
    Ok((left, right))
}
fn insertion_leaf(nodes: &[Node], at: usize) -> Option<Node> {
    let mut cursor = 0;
    let mut previous = None;
    for node in nodes {
        let length = inline_len(node).ok()?;
        if at < cursor + length && node.node_type == "text" {
            return Some(node.clone());
        }
        if node.node_type == "text" {
            previous = Some(node.clone());
        }
        cursor += length;
    }
    previous
}
fn sibling_children<'a>(node: &'a mut Node, id: &str) -> Option<&'a mut Vec<Node>> {
    let children = node.content.as_mut()?;
    if children.iter().any(|child| child.block_id() == Some(id)) {
        return Some(children);
    }
    children
        .iter_mut()
        .find_map(|child| sibling_children(child, id))
}
fn join_compatible(left: &Node, right: &Node) -> bool {
    left.node_type == right.node_type && left.compatible_envelope(right, &["blockId", "sourceRefs"])
}

fn coalesce_text(nodes: Vec<Node>) -> Vec<Node> {
    let mut result: Vec<Node> = Vec::with_capacity(nodes.len());
    for node in nodes {
        if let Some(previous) = result.last_mut()
            && previous.can_coalesce_text(&node)
            && let Some(previous_text) = previous.text.as_mut()
            && let Some(text) = node.text.as_deref()
        {
            previous_text.push_str(text);
        } else {
            result.push(node);
        }
    }
    result
}
