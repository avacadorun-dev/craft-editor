//! Revision-local, exact-node-guarded child-tree operations.

use serde::{Deserialize, Serialize};

use crate::{Command, Document, Error, Node, PositionMap, Result, ScalarOffset};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// A path of child ordinals from the document root; an empty path is the root.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodePath(#[doc = "Child ordinals from the root."] pub Vec<usize>);

impl NodePath {
    /// The document root path.
    pub fn root() -> Self {
        Self::default()
    }
    /// A child path without changing this path.
    pub fn child(&self, index: usize) -> Self {
        let mut path = self.0.clone();
        path.push(index);
        Self(path)
    }
    /// Parent path, or none for the root.
    pub fn parent(&self) -> Option<Self> {
        self.0.split_last().map(|(_, parent)| Self(parent.to_vec()))
    }
}

/// An exact expected node at a revision-local path, including unknown metadata.
/// Capture from the current document for each sequential intermediate command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeRef {
    /// Revision-local location; never silently retargeted by type or quote.
    pub path: NodePath,
    /// Complete expected node. Different content/attrs/presence fails the guard.
    pub expected: Box<Node>,
}

impl Document {
    /// Build one guarded replacement after validating the complete resulting tree.
    /// The caller still supplies the expected document revision when applying it.
    pub fn replace_node(&self, target: &NodeRef, replacement: Node) -> Result<Command> {
        check_subtree_bounds(&replacement)?;
        let mut candidate = self.clone();
        replace_node(
            &mut candidate,
            target.clone(),
            replacement.clone(),
            &mut vec![],
        )?;
        candidate.promote_schema()?;
        candidate.validate()?;
        Ok(Command::ReplaceNode {
            target: target.clone(),
            replacement: Box::new(replacement),
        })
    }
    /// Patch known or extension attributes without rebuilding or dropping fields.
    pub fn patch_node_attrs(&self, target: &NodeRef, attrs: Map<String, Value>) -> Result<Command> {
        check_ref(self, target)?;
        let mut replacement = target.expected.as_ref().clone();
        replacement.attrs.get_or_insert_default().extend(attrs);
        self.replace_node(target, replacement)
    }
    /// Build a schema-checked inline rich slice replacement against an exact node.
    pub fn replace_inline(
        &self,
        target: &NodeRef,
        from: ScalarOffset,
        to: ScalarOffset,
        children: Vec<Node>,
    ) -> Result<Command> {
        block_ids(&children)?;
        let command = Command::ReplaceInline {
            target: target.clone(),
            from,
            to,
            children,
        };
        let mut editor = crate::Editor::new(self.clone(), crate::Revision(0));
        editor.apply(crate::Transaction {
            base_revision: crate::Revision(0),
            commands: vec![command.clone()],
        })?;
        Ok(command)
    }

    /// Read a node by revision-local path, with checked bounds at every level.
    pub fn node(&self, path: &NodePath) -> Result<&Node> {
        at_path(&self.root, path).ok_or(Error::InvalidPosition)
    }
    /// Freeze exact node identity and metadata for a structural command guard.
    pub fn node_ref(&self, path: &NodePath) -> Result<NodeRef> {
        Ok(NodeRef {
            path: path.clone(),
            expected: Box::new(self.node(path)?.clone()),
        })
    }
}

pub(crate) fn splice_children(
    document: &mut Document,
    reference: NodeRef,
    from: usize,
    to: usize,
    inserted: Vec<Node>,
    mappings: &mut Vec<PositionMap>,
) -> Result<()> {
    check_ref(document, &reference)?;
    let parent = at_path_mut(&mut document.root, &reference.path).ok_or(Error::InvalidPosition)?;
    let removed = parent
        .children()
        .get(from..to)
        .ok_or(Error::InvalidPosition)?;
    if removed.is_empty() && inserted.is_empty() {
        return Ok(());
    }
    let removed_ids = block_ids(removed)?;
    let inserted_ids = block_ids(&inserted)?;
    let inserted_len = inserted.len();
    parent
        .content
        .get_or_insert_default()
        .splice(from..to, inserted);
    mappings.push(PositionMap::Children {
        parent: reference.path,
        from,
        to,
        inserted: inserted_len,
        removed_ids,
        inserted_ids,
    });
    Ok(())
}

pub(crate) fn move_children(
    document: &mut Document,
    source: NodeRef,
    from: usize,
    to: usize,
    destination: NodeRef,
    index: usize,
    mappings: &mut Vec<PositionMap>,
) -> Result<()> {
    check_ref(document, &source)?;
    check_ref(document, &destination)?;
    let source_node = document.node(&source.path)?;
    let selected = source_node
        .children()
        .get(from..to)
        .ok_or(Error::InvalidPosition)?;
    if index > document.node(&destination.path)?.children().len() {
        return Err(Error::InvalidPosition);
    }
    if selected.is_empty() {
        return Ok(());
    }
    if source.path == destination.path && (from..=to).contains(&index) {
        return Ok(());
    }
    if destination.path.0.starts_with(&source.path.0)
        && let Some(child) = destination.path.0.get(source.path.0.len())
        && (from..to).contains(child)
    {
        return Err(Error::InvalidDocument("move_cycle".into()));
    }
    let moved_ids = block_ids(selected)?;
    let adjusted_destination = map_splice_path(&destination.path, &source.path, from, to, 0)
        .ok_or(Error::InvalidPosition)?;
    let adjusted_index = if source.path == destination.path && index > to {
        index - (to - from)
    } else {
        index
    };
    let source_node =
        at_path_mut(&mut document.root, &source.path).ok_or(Error::InvalidPosition)?;
    let moved = source_node
        .content
        .get_or_insert_default()
        .drain(from..to)
        .collect::<Vec<_>>();
    let target =
        at_path_mut(&mut document.root, &adjusted_destination).ok_or(Error::InvalidPosition)?;
    let children = target.content.get_or_insert_default();
    if adjusted_index > children.len() {
        return Err(Error::InvalidPosition);
    }
    children.splice(adjusted_index..adjusted_index, moved);
    mappings.push(PositionMap::MoveChildren {
        source_parent: source.path,
        from,
        to,
        destination_parent: destination.path,
        index,
        moved_ids,
    });
    Ok(())
}

pub(crate) fn check_ref(document: &Document, reference: &NodeRef) -> Result<()> {
    if document.node(&reference.path)? == reference.expected.as_ref() {
        Ok(())
    } else {
        Err(Error::NodeReferenceConflict)
    }
}
fn at_path<'a>(root: &'a Node, path: &NodePath) -> Option<&'a Node> {
    let mut node = root;
    for index in &path.0 {
        node = node.children().get(*index)?;
    }
    Some(node)
}
pub(crate) fn at_path_mut<'a>(root: &'a mut Node, path: &NodePath) -> Option<&'a mut Node> {
    let mut node = root;
    for index in &path.0 {
        node = node.content.as_mut()?.get_mut(*index)?;
    }
    Some(node)
}
pub(crate) fn block_ids(nodes: &[Node]) -> Result<Vec<String>> {
    let mut stack = nodes.iter().map(|node| (node, 0)).collect::<Vec<_>>();
    let mut ids = Vec::new();
    let mut count = 0;
    while let Some((node, depth)) = stack.pop() {
        count += 1;
        if depth > 64 || count > 100_000 {
            return Err(Error::ResourceLimit);
        }
        if let Some(id) = node.block_id() {
            ids.push(id.to_owned());
        }
        stack.extend(node.children().iter().map(|child| (child, depth + 1)));
    }
    ids.sort();
    Ok(ids)
}

pub(crate) fn map_splice_path(
    path: &NodePath,
    parent: &NodePath,
    from: usize,
    to: usize,
    inserted: usize,
) -> Option<NodePath> {
    if from > to {
        return None;
    }
    if !path.0.starts_with(&parent.0) || path.0.len() <= parent.0.len() {
        return Some(path.clone());
    }
    let child = path.0[parent.0.len()];
    if (from..to).contains(&child) {
        return None;
    }
    if child < from {
        return Some(path.clone());
    }
    let mut mapped = path.clone();
    mapped.0[parent.0.len()] = child.checked_sub(to - from)?.checked_add(inserted)?;
    Some(mapped)
}

pub(crate) fn map_move_path(
    path: &NodePath,
    source: &NodePath,
    from: usize,
    to: usize,
    destination: &NodePath,
    index: usize,
) -> Option<NodePath> {
    if from > to {
        return None;
    }
    if source == destination && (from..=to).contains(&index) {
        return Some(path.clone());
    }
    let adjusted_destination = map_splice_path(destination, source, from, to, 0)?;
    let adjusted_index = if source == destination && index > to {
        index.checked_sub(to - from)?
    } else {
        index
    };
    if path.0.starts_with(&source.0)
        && let Some(child) = path.0.get(source.0.len())
        && (from..to).contains(child)
    {
        let mut result = adjusted_destination.0;
        result.push(adjusted_index.checked_add(child - from)?);
        result.extend_from_slice(&path.0[source.0.len() + 1..]);
        return Some(NodePath(result));
    }
    let after_removal = map_splice_path(path, source, from, to, 0)?;
    map_splice_path(
        &after_removal,
        &adjusted_destination,
        adjusted_index,
        adjusted_index,
        to - from,
    )
}

pub(crate) fn replace_node(
    document: &mut Document,
    target: NodeRef,
    replacement: Node,
    mappings: &mut Vec<PositionMap>,
) -> Result<()> {
    check_ref(document, &target)?;
    let before = document.node(&target.path)?;
    if before == &replacement {
        return Ok(());
    }
    check_subtree_bounds(&replacement)?;
    let mut old = BTreeMap::new();
    let mut new = BTreeMap::new();
    collect_texts(before, &mut old);
    collect_texts(&replacement, &mut new);
    let removed_ids = old
        .keys()
        .filter(|id| !new.contains_key(*id))
        .cloned()
        .collect();
    let inserted_ids = new
        .keys()
        .filter(|id| !old.contains_key(*id))
        .cloned()
        .collect();
    let invalidated_ids = old
        .iter()
        .filter(|(id, text)| new.get(*id).is_some_and(|new| new != *text))
        .map(|(id, _)| id.clone())
        .collect();
    *at_path_mut(&mut document.root, &target.path).ok_or(Error::InvalidPosition)? = replacement;
    mappings.push(PositionMap::NodeReplacement {
        path: target.path,
        removed_ids,
        inserted_ids,
        invalidated_ids,
    });
    Ok(())
}
pub(crate) fn check_subtree_bounds(node: &Node) -> Result<()> {
    let mut stack = vec![(node, 0)];
    let mut count = 0;
    while let Some((node, depth)) = stack.pop() {
        count += 1;
        if depth > 64 || count > 100_000 {
            return Err(Error::ResourceLimit);
        }
        stack.extend(node.children().iter().map(|child| (child, depth + 1)));
    }
    Ok(())
}
fn collect_texts(node: &Node, result: &mut BTreeMap<String, String>) {
    if let Some(id) = node.block_id() {
        result.insert(id.into(), node.inline_text());
    }
    for child in node.children() {
        collect_texts(child, result);
    }
}
