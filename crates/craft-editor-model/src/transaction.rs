use std::collections::VecDeque;

use prosemirror::transform::{Mappable, StepMap};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    Document, Error, Mark, Node, Result, ScalarOffset, TextRange, document::validate_mark,
    scalar_to_byte,
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
}

/// One document-level editing session with guarded, bounded snapshot history.
/// Host annotation/selection snapshots must be committed alongside [`Change`].
#[derive(Debug)]
pub struct Editor {
    document: Document,
    revision: Revision,
    undo: VecDeque<Document>,
    redo: Vec<Document>,
    history_limit: usize,
}

impl Editor {
    /// Create a session from validated canonical state and its host revision.
    pub fn new(document: Document, revision: Revision) -> Self {
        Self {
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
        for command in transaction.commands {
            apply_command(&mut candidate, command, &mut mappings)?;
            candidate.promote_schema()?;
            candidate.validate()?;
        }
        let before = self.document.clone();
        let before_revision = self.revision;
        if candidate != self.document {
            let revision = self.next_revision()?;
            if self.history_limit > 0 {
                self.undo.push_back(self.document.clone());
                while self.undo.len() > self.history_limit {
                    self.undo.pop_front();
                }
            }
            self.redo.clear();
            self.document = candidate;
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
        })
    }
    /// Undo by creating a new revision; stale calls never consume history.
    pub fn undo(&mut self, expected_revision: Revision) -> Result<Change> {
        self.check_revision(expected_revision)?;
        let revision = self.next_revision()?;
        let previous = self.undo.pop_back().ok_or(Error::HistoryEmpty)?;
        let before = std::mem::replace(&mut self.document, previous);
        self.redo.push(before.clone());
        let before_revision = std::mem::replace(&mut self.revision, revision);
        Ok(Change {
            before_revision,
            after_revision: revision,
            before,
            after: self.document.clone(),
            mappings: vec![],
        })
    }
    /// Redo a previously undone batch as another new revision.
    pub fn redo(&mut self, expected_revision: Revision) -> Result<Change> {
        self.check_revision(expected_revision)?;
        let revision = self.next_revision()?;
        let next = self.redo.pop().ok_or(Error::HistoryEmpty)?;
        let before = std::mem::replace(&mut self.document, next);
        if self.history_limit > 0 {
            self.undo.push_back(before.clone());
        }
        let before_revision = std::mem::replace(&mut self.revision, revision);
        Ok(Change {
            before_revision,
            after_revision: revision,
            before,
            after: self.document.clone(),
            mappings: vec![],
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
            node.content = Some(content);
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
            node.content = Some(prefix.into_iter().chain(selected).chain(suffix).collect());
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
            left.content
                .get_or_insert_default()
                .extend(right.content.unwrap_or_default());
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
    if left.node_type != right.node_type || left.extra != right.extra || left.marks != right.marks {
        return false;
    }
    let stripped = |attrs: &Option<Map<String, Value>>| {
        let mut result = attrs.clone().unwrap_or_default();
        result.remove("blockId");
        result.remove("sourceRefs");
        result
    };
    stripped(&left.attrs) == stripped(&right.attrs)
}
