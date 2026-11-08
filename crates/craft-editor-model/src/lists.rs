//! Pure list-edit builders over the guarded canonical command engine.
//!
//! Paragraph-first item behavior follows ProseMirror schema-list and Tiptap list
//! commands. This independent tree implementation copies none of their code.
//! Unknown wrapper metadata is retained, or unwrapping is refused when it cannot
//! be retained at an unambiguous canonical location.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{
    Command, Document, Error, Node, NodePath, NodeRef, Result, ScalarOffset, scalar_to_byte,
};

/// Canonical ordered or bullet list kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ListKind {
    /// Unnumbered `bulletList`.
    Bullet,
    /// Numbered `orderedList`.
    Ordered,
}
impl ListKind {
    /// Canonical wire name.
    pub fn node_type(self) -> &'static str {
        match self {
            Self::Bullet => "bulletList",
            Self::Ordered => "orderedList",
        }
    }
}
/// Explicit list formatting; supplied unknown fields remain canonical.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListStyle {
    /// Canonical list kind.
    pub kind: ListKind,
    /// Attributes to set, including ordered-list start/type.
    pub attrs: Map<String, Value>,
}
impl ListStyle {
    /// Construct a default-style list.
    pub fn new(kind: ListKind) -> Self {
        Self {
            kind,
            attrs: Map::new(),
        }
    }
}

/// Wrap sibling blocks without converting or flattening them. Every selected
/// non-paragraph block needs one caller-supplied leading paragraph ID, consumed
/// in selection order; list items must begin with paragraphs. Selected existing
/// paragraphs keep their IDs, provenance, marks and complete metadata.
pub fn wrap_list(
    document: &Document,
    parent: NodeRef,
    from: usize,
    to: usize,
    style: ListStyle,
    leading_paragraph_ids: &[String],
) -> Result<Command> {
    checked_node(document, &parent)?;
    let selected = selected_children(&parent.expected, from, to)?;
    let mut ids = leading_paragraph_ids.iter();
    let mut items = Vec::with_capacity(selected.len());
    for block in selected {
        let mut content = Vec::new();
        if block.node_type != "paragraph" {
            let id = ids.next().filter(|id| !id.is_empty()).ok_or_else(|| {
                Error::InvalidDocument("leading_paragraph_identity_required".into())
            })?;
            content.push(Node::paragraph(id, ""));
        }
        content.push(block.clone());
        items.push(Node::block("listItem", Map::new(), content));
    }
    if ids.next().is_some() {
        return Err(Error::InvalidDocument(
            "unused_leading_paragraph_identity".into(),
        ));
    }
    let list = Node::block(style.kind.node_type(), style.attrs, items);
    let mut root = document.root().clone();
    let parent_node = node_mut(&mut root, &parent.path)?;
    parent_node
        .content
        .get_or_insert_default()
        .splice(from..to, [list]);
    join_around(parent_node, from, 1)?;
    finish(document, root)
}

/// Matching kind lifts selected items; another kind changes only the selected
/// list portion, retaining other items and wrapper metadata. A wrapper with a
/// stable blockId cannot be duplicated across multiple portions.
pub fn toggle_list(
    document: &Document,
    list: NodeRef,
    from: usize,
    to: usize,
    style: ListStyle,
) -> Result<Command> {
    toggle_impl(document, list, from, to, style, None)
}

/// Toggle with explicit fresh IDs for additional portions of an identified list.
/// The first retained portion keeps the original ID; further portions consume
/// these IDs in document order. Missing, duplicate or unused IDs are rejected.
pub fn toggle_list_with_ids(
    document: &Document,
    list: NodeRef,
    from: usize,
    to: usize,
    style: ListStyle,
    new_wrapper_ids: &[String],
) -> Result<Command> {
    toggle_impl(document, list, from, to, style, Some(new_wrapper_ids))
}

fn toggle_impl(
    document: &Document,
    list: NodeRef,
    from: usize,
    to: usize,
    style: ListStyle,
    new_wrapper_ids: Option<&[String]>,
) -> Result<Command> {
    checked_list(document, &list)?;
    selected_children(&list.expected, from, to)?;
    if list.expected.node_type == style.kind.node_type() {
        return lift_impl(document, list, from, to, new_wrapper_ids);
    }
    let count = usize::from(from > 0) + 1 + usize::from(to < list.expected.children().len());
    let mut parts = ListParts::new(&list.expected, new_wrapper_ids);
    let mut replacements = Vec::with_capacity(count);
    if from > 0 {
        replacements.push(parts.portion(0, from)?);
    }
    let mut selected = parts.portion(from, to)?;
    selected.node_type = style.kind.node_type().to_owned();
    if let Some(new_id) = style.attrs.get("blockId")
        && selected
            .attrs
            .as_ref()
            .and_then(|attrs| attrs.get("blockId"))
            .is_some_and(|old| old != new_id)
    {
        return Err(Error::InvalidDocument("list_identity_change".into()));
    }
    selected.attrs.get_or_insert_default().extend(style.attrs);
    replacements.push(selected);
    if to < list.expected.children().len() {
        replacements.push(parts.portion(to, list.expected.children().len())?);
    }
    parts.finish()?;
    replace_list_in_parent(document, &list.path, replacements, true)
}

/// Indent into the preceding item. A matching final nested list is extended;
/// otherwise a new default-style nested list is created. The first item cannot
/// be indented without inventing a preceding sibling.
pub fn indent_list_items(
    document: &Document,
    list: NodeRef,
    from: usize,
    to: usize,
) -> Result<Command> {
    checked_list(document, &list)?;
    selected_children(&list.expected, from, to)?;
    if from == 0 {
        return Err(Error::UnsupportedEdit);
    }
    let mut replacement = list.expected.as_ref().clone();
    let kind = replacement.node_type.clone();
    let items = replacement.content.get_or_insert_default();
    let moved = items.drain(from..to).collect::<Vec<_>>();
    let previous = items.get_mut(from - 1).ok_or(Error::InvalidPosition)?;
    let blocks = previous.content.get_or_insert_default();
    if let Some(last) = blocks.last_mut()
        && last.node_type == kind
    {
        last.content.get_or_insert_default().extend(moved);
    } else {
        blocks.push(Node::block(kind, Map::new(), moved));
    }
    document.replace_node(&list, replacement)
}

/// Lift by one level, sharing exactly the explicit lift operation.
pub fn outdent_list_items(
    document: &Document,
    list: NodeRef,
    from: usize,
    to: usize,
) -> Result<Command> {
    lift_list_items(document, list, from, to)
}

/// Lift items one level. Top-level items become their original blocks; nested
/// items remain intact items in the outer list. Trailing inner items follow the
/// last lifted item as a nested list. Unknown metadata on removed wrappers must
/// have a retained wrapper destination, otherwise the operation is refused.
pub fn lift_list_items(
    document: &Document,
    list: NodeRef,
    from: usize,
    to: usize,
) -> Result<Command> {
    lift_impl(document, list, from, to, None)
}

/// Lift with caller-supplied IDs when a stable-ID wrapper must become multiple
/// retained list portions. This does not authorize dropping an entire identified
/// or metadata-bearing wrapper that has no remaining canonical destination.
pub fn lift_list_items_with_ids(
    document: &Document,
    list: NodeRef,
    from: usize,
    to: usize,
    new_wrapper_ids: &[String],
) -> Result<Command> {
    lift_impl(document, list, from, to, Some(new_wrapper_ids))
}

/// Outdent with the same explicit wrapper-identity allocation as lifting.
pub fn outdent_list_items_with_ids(
    document: &Document,
    list: NodeRef,
    from: usize,
    to: usize,
    new_wrapper_ids: &[String],
) -> Result<Command> {
    lift_list_items_with_ids(document, list, from, to, new_wrapper_ids)
}

fn lift_impl(
    document: &Document,
    list: NodeRef,
    from: usize,
    to: usize,
    new_wrapper_ids: Option<&[String]>,
) -> Result<Command> {
    checked_list(document, &list)?;
    let selected = selected_children(&list.expected, from, to)?;
    let parent_path = list.path.parent().ok_or(Error::UnsupportedEdit)?;
    if document.node(&parent_path)?.node_type == "listItem" {
        return lift_nested(document, list, from, to, new_wrapper_ids);
    }
    if selected.iter().any(|item| !removable_item_wrapper(item)) {
        return Err(Error::UnsupportedEdit);
    }
    let before = from > 0;
    let after = to < list.expected.children().len();
    if !before && !after && !removable_list_wrapper(&list.expected) {
        return Err(Error::UnsupportedEdit);
    }
    let mut parts = ListParts::new(&list.expected, new_wrapper_ids);
    let mut replacements = Vec::new();
    if before {
        replacements.push(parts.portion(0, from)?);
    }
    replacements.extend(
        selected
            .iter()
            .flat_map(|item| item.children().iter().cloned()),
    );
    if after {
        replacements.push(parts.portion(to, list.expected.children().len())?);
    }
    parts.finish()?;
    replace_list_in_parent(document, &list.path, replacements, false)
}

/// Split a direct paragraph in an item, retaining marks/sourceRefs and the left
/// identity. New right paragraph identity is supplied by the caller. An item
/// with a blockId also requires a fresh new_item_id. A sole empty paragraph
/// exits/lifts the item instead of producing another empty item.
pub fn split_list_item(
    document: &Document,
    list: NodeRef,
    item_index: usize,
    paragraph_index: usize,
    at: ScalarOffset,
    new_block_id: &str,
    new_item_id: Option<&str>,
) -> Result<Command> {
    checked_list(document, &list)?;
    let item = list
        .expected
        .children()
        .get(item_index)
        .ok_or(Error::InvalidPosition)?;
    let paragraph = item
        .children()
        .get(paragraph_index)
        .ok_or(Error::InvalidPosition)?;
    if paragraph.node_type != "paragraph" || new_block_id.is_empty() {
        return Err(Error::UnsupportedEdit);
    }
    if item.children().len() == 1 && paragraph.inline_text().is_empty() && at.0 == 0 {
        return exit_empty_list_item(document, list, item_index);
    }
    let (left, right) = split_inline(paragraph.children(), at.0)?;
    let mut left_item = item.clone();
    let mut right_item = item.clone();
    if item.block_id().is_some() && new_item_id.is_none() {
        return Err(Error::InvalidDocument("new_item_identity_required".into()));
    }
    if let Some(id) = new_item_id {
        if id.is_empty() {
            return Err(Error::InvalidDocument("new_item_identity_required".into()));
        }
        right_item
            .attrs
            .get_or_insert_default()
            .insert("blockId".into(), json!(id));
    }
    let mut left_paragraph = paragraph.clone();
    left_paragraph.content = Some(left);
    let mut right_paragraph = paragraph.clone();
    right_paragraph.content = Some(right);
    right_paragraph
        .attrs
        .get_or_insert_default()
        .insert("blockId".into(), json!(new_block_id));
    let mut left_blocks = item.children()[..paragraph_index].to_vec();
    left_blocks.push(left_paragraph);
    let mut right_blocks = vec![right_paragraph];
    right_blocks.extend_from_slice(&item.children()[paragraph_index + 1..]);
    left_item.content = Some(left_blocks);
    right_item.content = Some(right_blocks);
    let mut replacement = list.expected.as_ref().clone();
    replacement
        .content
        .get_or_insert_default()
        .splice(item_index..item_index + 1, [left_item, right_item]);
    document.replace_node(&list, replacement)
}

/// Exit a truly empty item. Extra nested/rich blocks prevent this shortcut so
/// an Enter action cannot discard hidden content.
pub fn exit_empty_list_item(
    document: &Document,
    list: NodeRef,
    item_index: usize,
) -> Result<Command> {
    checked_list(document, &list)?;
    let item = list
        .expected
        .children()
        .get(item_index)
        .ok_or(Error::InvalidPosition)?;
    if item.children().len() != 1
        || item.children()[0].node_type != "paragraph"
        || !item.children()[0].inline_text().is_empty()
    {
        return Err(Error::UnsupportedEdit);
    }
    lift_list_items(document, list, item_index, item_index + 1)
}

fn checked_node<'a>(document: &'a Document, reference: &NodeRef) -> Result<&'a Node> {
    let node = document.node(&reference.path)?;
    if node != reference.expected.as_ref() {
        return Err(Error::NodeReferenceConflict);
    }
    Ok(node)
}
fn checked_list<'a>(document: &'a Document, reference: &NodeRef) -> Result<&'a Node> {
    let node = checked_node(document, reference)?;
    if !matches!(node.node_type.as_str(), "bulletList" | "orderedList")
        || node
            .children()
            .iter()
            .any(|item| item.node_type != "listItem")
    {
        return Err(Error::UnsupportedEdit);
    }
    Ok(node)
}
fn selected_children(parent: &Node, from: usize, to: usize) -> Result<&[Node]> {
    parent
        .children()
        .get(from..to)
        .filter(|selected| !selected.is_empty())
        .ok_or(Error::InvalidPosition)
}
fn finish(document: &Document, root: Node) -> Result<Command> {
    document.replace_node(&document.node_ref(&NodePath::root())?, root)
}
fn node_mut<'a>(root: &'a mut Node, path: &NodePath) -> Result<&'a mut Node> {
    let mut node = root;
    for index in &path.0 {
        node = node
            .content
            .as_mut()
            .and_then(|children| children.get_mut(*index))
            .ok_or(Error::InvalidPosition)?;
    }
    Ok(node)
}
fn replace_list_in_parent(
    document: &Document,
    path: &NodePath,
    replacement: Vec<Node>,
    auto_join: bool,
) -> Result<Command> {
    let (index, parent) = path.0.split_last().ok_or(Error::UnsupportedEdit)?;
    let mut root = document.root().clone();
    let parent_node = node_mut(&mut root, &NodePath(parent.to_vec()))?;
    let inserted = replacement.len();
    parent_node
        .content
        .get_or_insert_default()
        .splice(*index..*index + 1, replacement);
    if auto_join {
        join_around(parent_node, *index, inserted)?;
    }
    finish(document, root)
}
fn removable_list_wrapper(list: &Node) -> bool {
    list.extra.is_empty()
        && list.marks.as_ref().is_none_or(Vec::is_empty)
        && list.attrs.as_ref().is_none_or(|attrs| {
            attrs
                .keys()
                .all(|key| matches!(key.as_str(), "start" | "type"))
        })
}
fn removable_item_wrapper(item: &Node) -> bool {
    item.extra.is_empty()
        && item.marks.as_ref().is_none_or(Vec::is_empty)
        && item.attrs.as_ref().is_none_or(Map::is_empty)
}
fn list_portion(list: &Node, from: usize, to: usize) -> Result<Node> {
    let mut portion = list.clone();
    portion.content = Some(
        list.children()
            .get(from..to)
            .ok_or(Error::InvalidPosition)?
            .to_vec(),
    );
    if from > 0 && list.node_type == "orderedList" {
        let start = match list.attrs.as_ref().and_then(|attrs| attrs.get("start")) {
            None | Some(Value::Null) => 1,
            Some(value) => value
                .as_i64()
                .ok_or_else(|| Error::InvalidDocument("ordered_list_start".into()))?,
        };
        let offset = i64::try_from(from).map_err(|_| Error::ResourceLimit)?;
        let start = start.checked_add(offset).ok_or(Error::ResourceLimit)?;
        portion
            .attrs
            .get_or_insert_default()
            .insert("start".into(), json!(start));
    }
    Ok(portion)
}
fn lift_nested(
    document: &Document,
    list: NodeRef,
    from: usize,
    to: usize,
    new_wrapper_ids: Option<&[String]>,
) -> Result<Command> {
    let (inner_index, parent_path) = list.path.0.split_last().ok_or(Error::UnsupportedEdit)?;
    let (outer_item_index, outer_path) = parent_path.split_last().ok_or(Error::UnsupportedEdit)?;
    let outer_path = NodePath(outer_path.to_vec());
    let outer = document.node(&outer_path)?;
    if !matches!(outer.node_type.as_str(), "bulletList" | "orderedList") {
        return Err(Error::UnsupportedEdit);
    }
    let parent_item = document.node(&NodePath(parent_path.to_vec()))?;
    let mut lifted = list.expected.children()[from..to].to_vec();
    let before = from > 0;
    let after = to < list.expected.children().len();
    if !before && !after && !removable_list_wrapper(&list.expected) {
        return Err(Error::UnsupportedEdit);
    }
    let mut parts = ListParts::new(&list.expected, new_wrapper_ids);
    let mut retained_parent = parent_item.clone();
    let mut retained_blocks = parent_item.children()[..*inner_index].to_vec();
    if before {
        retained_blocks.push(parts.portion(0, from)?);
    }
    retained_parent.content = Some(retained_blocks);
    let last = lifted.last_mut().ok_or(Error::InvalidPosition)?;
    if after {
        last.content
            .get_or_insert_default()
            .push(parts.portion(to, list.expected.children().len())?);
    }
    last.content
        .get_or_insert_default()
        .extend_from_slice(&parent_item.children()[*inner_index + 1..]);
    parts.finish()?;
    let mut outer_replacement = outer.clone();
    let mut replacements = vec![retained_parent];
    replacements.extend(lifted);
    outer_replacement
        .content
        .get_or_insert_default()
        .splice(*outer_item_index..*outer_item_index + 1, replacements);
    document.replace_node(&document.node_ref(&outer_path)?, outer_replacement)
}
fn split_inline(nodes: &[Node], at: usize) -> Result<(Vec<Node>, Vec<Node>)> {
    let mut left = Vec::new();
    let mut right = Vec::new();
    let mut cursor = 0;
    for node in nodes {
        let len = match node.node_type.as_str() {
            "text" => node
                .text
                .as_deref()
                .ok_or(Error::UnsupportedEdit)?
                .chars()
                .count(),
            "hardBreak" | "inlineMath" => 1,
            _ => return Err(Error::UnsupportedEdit),
        };
        let end = cursor + len;
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

/// Join two exact adjacent sibling lists only when their numbering and complete
/// non-content metadata are compatible. Stable wrapper IDs are never discarded.
/// Ordered lists must already be a numeric continuation, not a restarted list.
pub fn join_adjacent_lists(document: &Document, left: NodeRef, right: NodeRef) -> Result<Command> {
    checked_list(document, &left)?;
    checked_list(document, &right)?;
    let (left_index, parent_path) = left.path.0.split_last().ok_or(Error::UnsupportedEdit)?;
    let (right_index, right_parent) = right.path.0.split_last().ok_or(Error::UnsupportedEdit)?;
    if parent_path != right_parent
        || left_index.checked_add(1) != Some(*right_index)
        || !join_compatible(&left.expected, &right.expected)
    {
        return Err(Error::UnsupportedEdit);
    }
    let mut joined = left.expected.as_ref().clone();
    joined
        .content
        .get_or_insert_default()
        .extend_from_slice(right.expected.children());
    let mut root = document.root().clone();
    node_mut(&mut root, &NodePath(parent_path.to_vec()))?
        .content
        .get_or_insert_default()
        .splice(*left_index..*right_index + 1, [joined]);
    finish(document, root)
}

struct ListParts<'a> {
    list: &'a Node,
    identities: Option<std::slice::Iter<'a, String>>,
    count: usize,
}
impl<'a> ListParts<'a> {
    fn new(list: &'a Node, identities: Option<&'a [String]>) -> Self {
        Self {
            list,
            identities: identities.map(|ids| ids.iter()),
            count: 0,
        }
    }
    fn portion(&mut self, from: usize, to: usize) -> Result<Node> {
        let mut portion = list_portion(self.list, from, to)?;
        if self.list.block_id().is_some() && self.count > 0 {
            let Some(identities) = self.identities.as_mut() else {
                return Err(Error::UnsupportedEdit);
            };
            let id = identities
                .next()
                .filter(|id| !id.is_empty())
                .ok_or_else(|| Error::InvalidDocument("wrapper_identity_required".into()))?;
            portion
                .attrs
                .get_or_insert_default()
                .insert("blockId".into(), json!(id));
        }
        self.count += 1;
        Ok(portion)
    }
    fn finish(mut self) -> Result<()> {
        if self
            .identities
            .as_mut()
            .and_then(|ids| ids.next())
            .is_some()
        {
            return Err(Error::InvalidDocument("unused_wrapper_identity".into()));
        }
        Ok(())
    }
}

fn ordered_start(list: &Node) -> Option<i64> {
    match list.attrs.as_ref().and_then(|attrs| attrs.get("start")) {
        None | Some(Value::Null) => Some(1),
        Some(value) => value.as_i64(),
    }
}
fn join_compatible(left: &Node, right: &Node) -> bool {
    if left.node_type != right.node_type
        || !matches!(left.node_type.as_str(), "bulletList" | "orderedList")
        || left.block_id().is_some()
        || right.block_id().is_some()
        || !left.same_null_presence(right)
    {
        return false;
    }
    let mut left_shell = left.clone();
    let mut right_shell = right.clone();
    left_shell.content = Some(Vec::new());
    right_shell.content = Some(Vec::new());
    if left.node_type == "orderedList" {
        let Some(expected_right) = ordered_start(left).and_then(|start| {
            i64::try_from(left.children().len())
                .ok()
                .and_then(|count| start.checked_add(count))
        }) else {
            return false;
        };
        if ordered_start(right) != Some(expected_right) {
            return false;
        }
        // Start differs because the second list is a continuation. All other
        // fields and presence metadata still have to match before joining.
        left_shell
            .attrs
            .get_or_insert_default()
            .insert("start".into(), json!(1));
        right_shell
            .attrs
            .get_or_insert_default()
            .insert("start".into(), json!(1));
    } else {
        if left_shell.attrs.as_ref().is_none_or(Map::is_empty) {
            left_shell.attrs = Some(Map::new());
        }
        if right_shell.attrs.as_ref().is_none_or(Map::is_empty) {
            right_shell.attrs = Some(Map::new());
        }
    }
    left_shell == right_shell
}
fn join_around(parent: &mut Node, from: usize, inserted: usize) -> Result<()> {
    let children = parent.content.get_or_insert_default();
    let mut first = from;
    let mut end = from.checked_add(inserted).ok_or(Error::ResourceLimit)?;
    if end > children.len() {
        return Err(Error::InvalidPosition);
    }
    let mut index = from.saturating_sub(1);
    while index + 1 < children.len() && index < end {
        let right_index = index + 1;
        let left_touched = (first..end).contains(&index);
        let right_touched = (first..end).contains(&right_index);
        if (left_touched || right_touched)
            && join_compatible(&children[index], &children[right_index])
        {
            let right = children.remove(right_index);
            children[index]
                .content
                .get_or_insert_default()
                .extend(right.content.unwrap_or_default());
            if right_touched {
                end -= 1;
            }
            first = first.min(index);
            index = index.saturating_sub(1);
        } else {
            index += 1;
        }
    }
    Ok(())
}
