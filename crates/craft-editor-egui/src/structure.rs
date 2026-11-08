//! Selection-aware host formatting commands over lossless canonical structure.

use craft_editor_model::{
    Change, Command, Document, Editor, ListKind, ListStyle, Node, NodePath, NodeRef, Transaction,
    indent_list_items, outdent_list_items_with_ids, toggle_list_with_ids, wrap_list,
};
use serde_json::{Map, Value};

use crate::{
    EditorSession, Error, MarkState, Result, Selection, insertion, projection::Projection,
};

enum Bookmark {
    Text(Selection),
    Cells(crate::TableSelection),
}
impl Bookmark {
    fn commit(self, session: &mut EditorSession, commands: Vec<Command>) -> Result<Change> {
        match self {
            Self::Text(selection) => session.apply_commands_with_selection(commands, selection),
            Self::Cells(selection) => {
                session.apply_commands_with_cell_selection(commands, selection)
            }
        }
    }
}
struct Bounds {
    first: NodePath,
    last: NodePath,
    selection: Bookmark,
}
impl Bounds {
    fn new(session: &EditorSession) -> Result<Self> {
        if session.is_composing() {
            return Err(Error::CompositionActive);
        }
        if let Some(cells) = session.cell_selection() {
            let path = cells.head_path(session.document())?;
            let count = session.document().node(&path)?.children().len();
            if count == 0 {
                return Err(Error::InvalidSelection);
            }
            return Ok(Self {
                first: path.child(0),
                last: path.child(count - 1),
                selection: Bookmark::Cells(cells.clone()),
            });
        }
        let selection = session
            .selection()
            .cloned()
            .ok_or(Error::InvalidSelection)?;
        let projection = Projection::new(session.document())?;
        let a = projection.scalar(&selection.anchor)?;
        let b = projection.scalar(&selection.head)?;
        let start = a.min(b);
        let end = a.max(b);
        let first = projection
            .spans
            .iter()
            .find(|span| {
                !span.atom
                    && span.id.as_str()
                        == if a <= b {
                            selection.anchor.block_id.as_str()
                        } else {
                            selection.head.block_id.as_str()
                        }
            })
            .ok_or(Error::InvalidSelection)?;
        // Original block-format commands include the block containing the final text point,
        // even at its offset zero. Clipboard character ranges remain end-exclusive.
        let last = projection
            .spans
            .iter()
            .rev()
            .find(|span| {
                if start == end {
                    span.path == first.path
                } else {
                    span.start <= end && span.end >= start
                }
            })
            .ok_or(Error::InvalidSelection)?;
        Ok(Self {
            first: NodePath(first.path.clone()),
            last: NodePath(last.path.clone()),
            selection: Bookmark::Text(selection),
        })
    }
    fn ancestor(&self, document: &Document, kind: impl Fn(&str) -> bool) -> Option<NodePath> {
        (0..=self.first.0.len()).rev().find_map(|length| {
            let path = NodePath(self.first.0[..length].to_vec());
            (self.last.0.starts_with(&path.0)
                && document.node(&path).is_ok_and(|node| kind(&node.node_type)))
            .then_some(path)
        })
    }
    fn sibling_range(&self) -> Result<(NodePath, usize, usize)> {
        if self.first == self.last {
            let (index, parent) = self.first.0.split_last().ok_or(Error::InvalidSelection)?;
            return Ok((NodePath(parent.to_vec()), *index, index + 1));
        }
        let length = self
            .first
            .0
            .iter()
            .zip(&self.last.0)
            .take_while(|(a, b)| a == b)
            .count();
        let from = *self.first.0.get(length).ok_or(Error::InvalidSelection)?;
        let to = self.last.0.get(length).ok_or(Error::InvalidSelection)? + 1;
        Ok((NodePath(self.first.0[..length].to_vec()), from, to))
    }
    fn child_range(&self, parent: &NodePath, document: &Document) -> Result<(usize, usize)> {
        if self.first == *parent && self.last == *parent {
            return Ok((0, document.node(parent)?.children().len()));
        }
        let from = *self
            .first
            .0
            .get(parent.0.len())
            .ok_or(Error::InvalidSelection)?;
        let to = self
            .last
            .0
            .get(parent.0.len())
            .ok_or(Error::InvalidSelection)?
            + 1;
        Ok((from, to))
    }
}

struct IdPool {
    prefix: u64,
    next: u64,
    reserved: std::collections::HashSet<String>,
}
impl IdPool {
    fn new(session: &EditorSession) -> Self {
        Self {
            prefix: session.revision().0,
            next: session.next_id,
            reserved: insertion::block_id_reservations(session.document()),
        }
    }
    fn id(&mut self) -> Result<String> {
        insertion::allocate_block_id(self.prefix, &mut self.next, &mut self.reserved)
    }
    fn additional_portions(&mut self, node: &Node, portions: usize) -> Result<Vec<String>> {
        if node.block_id().is_none() {
            return Ok(vec![]);
        }
        (0..portions.saturating_sub(1)).map(|_| self.id()).collect()
    }
}
fn is_list(kind: &str) -> bool {
    matches!(kind, "bulletList" | "orderedList")
}

impl EditorSession {
    /// Report whether all, some or none of the selected blocks belong to a list of this kind.
    pub fn list_state(&self, kind: ListKind) -> Result<MarkState> {
        self.container_state(kind.node_type())
    }
    /// Report the quotation state over the current semantic text selection.
    pub fn blockquote_state(&self) -> Result<MarkState> {
        self.container_state("blockquote")
    }
    fn container_state(&self, kind: &str) -> Result<MarkState> {
        let bounds = Bounds::new(self)?;
        let projection = Projection::new(self.document())?;
        let mut selected = projection.spans.iter().filter(|span| {
            span.path >= bounds.first.0
                && (span.path <= bounds.last.0 || span.path.starts_with(&bounds.last.0))
        });
        let mut any = false;
        let mut all = true;
        let mut found = false;
        for span in &mut selected {
            let active = (0..span.path.len()).any(|length| {
                self.document()
                    .node(&NodePath(span.path[..length].to_vec()))
                    .is_ok_and(|node| node.node_type == kind)
            });
            any |= active;
            all &= active;
            found = true;
            if span.path == bounds.last.0 {
                break;
            }
        }
        if !found {
            return Err(Error::InvalidSelection);
        }
        Ok(if all {
            MarkState::On
        } else if any {
            MarkState::Mixed
        } else {
            MarkState::Off
        })
    }
    /// Toggle the selected list items, or wrap sibling blocks, without flattening their content.
    ///
    /// Existing IDs, source references and rich runs remain canonical. Splitting an identified
    /// list allocates fresh IDs only for additional retained wrapper portions.
    pub fn toggle_list(&mut self, kind: ListKind) -> Result<Change> {
        let bounds = Bounds::new(self)?;
        let mut ids = IdPool::new(self);
        let commands = if let Some(path) = bounds.ancestor(self.document(), is_list) {
            let list = self.document().node_ref(&path)?;
            // Tiptap changes the complete active list when switching kind; only toggling the
            // current kind lifts the selected item range. Generic model builders also support
            // partial type changes, but that is not the original toolbar's interaction.
            let (from, to) = if list.expected.node_type == kind.node_type() {
                bounds.child_range(&path, self.document())?
            } else {
                (0, list.expected.children().len())
            };
            let portions = usize::from(from > 0)
                + usize::from(to < list.expected.children().len())
                + usize::from(list.expected.node_type != kind.node_type());
            let new_ids = ids.additional_portions(&list.expected, portions)?;
            vec![toggle_list_with_ids(
                self.document(),
                list,
                from,
                to,
                ListStyle::new(kind),
                &new_ids,
            )?]
        } else {
            let (parent_path, from, to) = bounds.sibling_range()?;
            let parent = self.document().node_ref(&parent_path)?;
            let selected = parent
                .expected
                .children()
                .get(from..to)
                .ok_or(Error::InvalidSelection)?;
            let mut commands = Vec::new();
            let mut candidate = Editor::new(self.document().clone(), self.revision());
            if selected
                .iter()
                .any(|node| matches!(node.node_type.as_str(), "heading" | "codeBlock"))
            {
                let normalized = selected
                    .iter()
                    .map(|node| {
                        let mut node = node.clone();
                        if matches!(node.node_type.as_str(), "heading" | "codeBlock") {
                            node.node_type = "paragraph".into();
                        }
                        node
                    })
                    .collect();
                let command = Command::ReplaceChildren {
                    parent,
                    from,
                    to,
                    children: normalized,
                };
                candidate.apply(Transaction {
                    base_revision: candidate.revision(),
                    commands: vec![command.clone()],
                })?;
                commands.push(command);
            }
            let parent = candidate.document().node_ref(&parent_path)?;
            let leading = parent
                .expected
                .children()
                .get(from..to)
                .ok_or(Error::InvalidSelection)?
                .iter()
                .filter(|node| node.node_type != "paragraph")
                .map(|_| ids.id())
                .collect::<Result<Vec<_>>>()?;
            let mut style = ListStyle::new(kind);
            if kind == ListKind::Ordered {
                style.attrs.extend([
                    ("start".into(), serde_json::json!(1)),
                    ("type".into(), Value::Null),
                ]);
            }
            commands.push(wrap_list(
                candidate.document(),
                parent,
                from,
                to,
                style,
                &leading,
            )?);
            commands
        };
        let change = bounds.selection.commit(self, commands)?;
        self.next_id = ids.next;
        Ok(change)
    }
    /// Indent the selected list items into the preceding item's nested list.
    pub fn indent_list(&mut self) -> Result<Change> {
        let bounds = Bounds::new(self)?;
        let path = bounds
            .ancestor(self.document(), is_list)
            .ok_or(Error::UnsupportedStructuralEdit)?;
        let (from, to) = bounds.child_range(&path, self.document())?;
        let command =
            indent_list_items(self.document(), self.document().node_ref(&path)?, from, to)?;
        bounds.selection.commit(self, vec![command])
    }
    /// Outdent the selected items by one nesting level, retaining their exact text selection.
    pub fn outdent_list(&mut self) -> Result<Change> {
        let bounds = Bounds::new(self)?;
        let path = bounds
            .ancestor(self.document(), is_list)
            .ok_or(Error::UnsupportedStructuralEdit)?;
        let (from, to) = bounds.child_range(&path, self.document())?;
        let list = self.document().node_ref(&path)?;
        let mut ids = IdPool::new(self);
        let portions = usize::from(from > 0) + usize::from(to < list.expected.children().len());
        let new_ids = ids.additional_portions(&list.expected, portions)?;
        let command = outdent_list_items_with_ids(self.document(), list, from, to, &new_ids)?;
        let change = bounds.selection.commit(self, vec![command])?;
        self.next_id = ids.next;
        Ok(change)
    }
    /// Toggle quotation for the selected sibling blocks or lift their enclosing quotation.
    ///
    /// Wrapper source references are distributed to retained provenance-bearing descendants.
    /// Unknown metadata without a retained wrapper destination rejects instead of disappearing.
    pub fn toggle_blockquote(&mut self) -> Result<Change> {
        let bounds = Bounds::new(self)?;
        let mut ids = IdPool::new(self);
        let command =
            if let Some(path) = bounds.ancestor(self.document(), |kind| kind == "blockquote") {
                let quote = self.document().node_ref(&path)?;
                let (from, to) = bounds.child_range(&path, self.document())?;
                lift_quote(self.document(), quote, from, to, &mut ids)?
            } else {
                let (path, from, to) = bounds.sibling_range()?;
                let parent = self.document().node_ref(&path)?;
                let selected = parent
                    .expected
                    .children()
                    .get(from..to)
                    .ok_or(Error::InvalidSelection)?
                    .to_vec();
                let quote = Node::block(
                    "blockquote",
                    Map::from_iter([
                        ("blockId".into(), Value::String(ids.id()?)),
                        ("sourceRefs".into(), serde_json::json!([])),
                    ]),
                    selected,
                );
                Command::ReplaceChildren {
                    parent,
                    from,
                    to,
                    children: vec![quote],
                }
            };
        let change = bounds.selection.commit(self, vec![command])?;
        self.next_id = ids.next;
        Ok(change)
    }
    /// Change the enclosing callout's source-defined kind while preserving its content and attrs.
    pub fn set_callout_kind(&mut self, kind: &str) -> Result<Change> {
        if !matches!(kind, "info" | "idea" | "warning" | "todo") {
            return Err(craft_editor_model::Error::InvalidDocument("callout_kind".into()).into());
        }
        let bounds = Bounds::new(self)?;
        let path = bounds
            .ancestor(self.document(), |kind| kind == "callout")
            .ok_or(Error::UnsupportedStructuralEdit)?;
        let target = self.document().node_ref(&path)?;
        let command = self.document().patch_node_attrs(
            &target,
            Map::from_iter([("kind".into(), Value::String(kind.into()))]),
        )?;
        bounds.selection.commit(self, vec![command])
    }
}

fn lift_quote(
    document: &Document,
    quote: NodeRef,
    from: usize,
    to: usize,
    ids: &mut IdPool,
) -> Result<Command> {
    let node = &quote.expected;
    let selected = node
        .children()
        .get(from..to)
        .filter(|nodes| !nodes.is_empty())
        .ok_or(Error::InvalidSelection)?;
    let before = from > 0;
    let after = to < node.children().len();
    let removable = node.extra.is_empty()
        && node.marks.as_ref().is_none_or(Vec::is_empty)
        && node.attrs.as_ref().is_none_or(|attrs| {
            attrs
                .keys()
                .all(|key| matches!(key.as_str(), "blockId" | "sourceRefs"))
        });
    if !before && !after && !removable {
        return Err(Error::UnsupportedStructuralEdit);
    }
    let mut children = Vec::new();
    if before {
        let mut prefix = node.as_ref().clone();
        prefix.content = Some(node.children()[..from].to_vec());
        children.push(prefix);
    }
    let mut lifted = selected.to_vec();
    if let Some(source_refs) = node
        .attrs
        .as_ref()
        .and_then(|attrs| attrs.get("sourceRefs"))
    {
        let refs = if source_refs.is_null() {
            &[][..]
        } else {
            source_refs
                .as_array()
                .ok_or(Error::UnsupportedStructuralEdit)?
                .as_slice()
        };
        if !refs.is_empty() {
            for child in &mut lifted {
                if !distribute_refs(child, refs)? {
                    return Err(Error::UnsupportedStructuralEdit);
                }
            }
        }
    }
    children.extend(lifted);
    if after {
        let mut suffix = node.as_ref().clone();
        suffix.content = Some(node.children()[to..].to_vec());
        if before && node.block_id().is_some() {
            suffix
                .attrs
                .get_or_insert_default()
                .insert("blockId".into(), Value::String(ids.id()?));
        }
        children.push(suffix);
    }
    let parent_path = quote.path.parent().ok_or(Error::InvalidSelection)?;
    let index = *quote.path.0.last().ok_or(Error::InvalidSelection)?;
    Ok(Command::ReplaceChildren {
        parent: document.node_ref(&parent_path)?,
        from: index,
        to: index + 1,
        children,
    })
}
fn distribute_refs(node: &mut Node, refs: &[Value]) -> Result<bool> {
    if matches!(
        node.node_type.as_str(),
        "paragraph" | "heading" | "blockquote"
    ) {
        let attrs = node.attrs.get_or_insert_default();
        let value = attrs
            .entry("sourceRefs")
            .or_insert_with(|| Value::Array(vec![]));
        if value.is_null() {
            *value = Value::Array(vec![]);
        }
        let existing = value
            .as_array_mut()
            .ok_or(Error::UnsupportedStructuralEdit)?;
        for reference in refs {
            if !existing.contains(reference) {
                existing.push(reference.clone());
            }
        }
        return Ok(true);
    }
    let mut retained = false;
    if let Some(children) = &mut node.content {
        for child in children {
            retained |= distribute_refs(child, refs)?;
        }
    }
    Ok(retained)
}

/// Canonical prose block kind offered by the source editor's format selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextBlockKind {
    /// Ordinary paragraph, retaining identity, writing and opaque metadata.
    Paragraph,
    /// Semantic heading level one through six.
    Heading {
        /// Heading level, validated before mutation.
        level: u8,
    },
}
/// Read-only formatting selector state for the exact native text/cell selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextBlockState {
    /// Ordinary paragraphs throughout the selected writing.
    Paragraph,
    /// One semantic heading level throughout the selected writing.
    Heading {
        /// Canonical heading level one through six.
        level: u8,
    },
    /// Code blocks throughout the selected writing.
    Code,
    /// More than one text block kind or heading level is selected.
    Mixed,
    /// No eligible text or cell selection is active.
    Unavailable,
}
impl EditorSession {
    fn selected_text_blocks(&self) -> Result<Vec<&Node>> {
        if self.cell_selection().is_some() {
            return self.selected_cell_blocks();
        }
        let Some(selected) = self.selection() else {
            return Ok(vec![]);
        };
        let projection = Projection::new(self.document())?;
        let a = projection
            .spans
            .iter()
            .position(|span| !span.atom && span.id == selected.anchor.block_id)
            .ok_or(Error::InvalidSelection)?;
        let b = projection
            .spans
            .iter()
            .position(|span| !span.atom && span.id == selected.head.block_id)
            .ok_or(Error::InvalidSelection)?;
        projection.spans[a.min(b)..=a.max(b)]
            .iter()
            .filter(|span| !span.atom)
            .map(|span| {
                self.document()
                    .node(&NodePath(span.path.clone()))
                    .map_err(Into::into)
            })
            .collect()
    }
    /// Summarize the complete eligible selection without moving it or changing stored marks.
    /// A mixed cell/paragraph range never masquerades as an ordinary paragraph.
    pub fn text_block_state(&self) -> Result<TextBlockState> {
        let mut result = TextBlockState::Unavailable;
        for node in self.selected_text_blocks()? {
            let next = match node.node_type.as_str() {
                "paragraph" => TextBlockState::Paragraph,
                "codeBlock" => TextBlockState::Code,
                "heading" => TextBlockState::Heading {
                    level: node
                        .attrs
                        .as_ref()
                        .and_then(|attrs| attrs.get("level"))
                        .and_then(serde_json::Value::as_f64)
                        .filter(|value| value.fract() == 0.0 && (1.0..=6.0).contains(value))
                        .ok_or(Error::InvalidSelection)? as u8,
                },
                _ => return Err(Error::InvalidSelection),
            };
            if result == TextBlockState::Unavailable {
                result = next;
            } else if result != next {
                return Ok(TextBlockState::Mixed);
            }
        }
        Ok(result)
    }
    /// Format selected textblocks, including every selected cell range, in one guarded history entry.
    /// Lists/quotes use the source head-cell policy; this command follows setBlockType's all-range policy.
    pub fn set_text_block_kind(&mut self, kind: TextBlockKind) -> Result<Change> {
        if self.is_composing() {
            return Err(Error::CompositionActive);
        }
        if let TextBlockKind::Heading { level } = kind
            && !(1..=6).contains(&level)
        {
            return Err(craft_editor_model::Error::InvalidDocument("heading_level".into()).into());
        }
        let ids = self
            .selected_text_blocks()?
            .into_iter()
            .map(|node| {
                node.block_id()
                    .map(str::to_owned)
                    .ok_or(Error::MissingBlockIdentity)
            })
            .collect::<Result<Vec<_>>>()?;
        let bookmark = if let Some(cells) = self.cell_selection() {
            Bookmark::Cells(cells.clone())
        } else {
            Bookmark::Text(self.selection().cloned().ok_or(Error::InvalidSelection)?)
        };
        let ids = ids.into_iter().collect::<std::collections::HashSet<_>>();
        let projection = Projection::new(self.document())?;
        let mut commands = Vec::new();
        for span in projection
            .spans
            .iter()
            .filter(|span| !span.atom && ids.contains(&span.id))
        {
            let target = self.document().node_ref(&NodePath(span.path.clone()))?;
            let mut replacement = target.expected.as_ref().clone();
            let from_code = replacement.node_type == "codeBlock";
            if from_code {
                if let Some(attrs) = &mut replacement.attrs {
                    attrs.remove("language");
                }
                let mut children = Vec::new();
                for child in replacement.children() {
                    if child.node_type == "text"
                        && child
                            .text
                            .as_deref()
                            .is_some_and(|text| text.contains('\n'))
                    {
                        for (index, line) in child
                            .text
                            .as_deref()
                            .unwrap_or_default()
                            .split('\n')
                            .enumerate()
                        {
                            if index > 0 {
                                children.push(Node::block("hardBreak", Map::new(), vec![]));
                            }
                            if !line.is_empty() {
                                let mut leaf = child.clone();
                                leaf.text = Some(line.into());
                                children.push(leaf);
                            }
                        }
                    } else {
                        children.push(child.clone());
                    }
                }
                replacement.content = Some(children);
            }
            if from_code {
                replacement
                    .attrs
                    .get_or_insert_default()
                    .entry("sourceRefs")
                    .or_insert_with(|| serde_json::json!([]));
            }
            match kind {
                TextBlockKind::Paragraph => {
                    replacement.node_type = "paragraph".into();
                    if let Some(attrs) = &mut replacement.attrs {
                        attrs.remove("level");
                    }
                }
                TextBlockKind::Heading { level } => {
                    replacement.node_type = "heading".into();
                    replacement
                        .attrs
                        .get_or_insert_default()
                        .insert("level".into(), serde_json::json!(level));
                }
            }
            commands.push(self.document().replace_node(&target, replacement)?);
        }
        bookmark.commit(self, commands)
    }
}
