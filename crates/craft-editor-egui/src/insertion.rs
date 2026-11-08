//! Lossless canonical content insertion at the current semantic selection.

use std::collections::HashSet;

use craft_editor_model::{
    Change, Command, Document, Editor, Node, NodePath, NodeRef, ScalarOffset, Transaction,
};

use crate::{DocPoint, EditorSession, Error, Result, Selection, projection::Projection};

/// A closed canonical content slice, with no implicit Markdown or application-root insertion.
#[derive(Debug, Clone)]
pub enum ContentSlice {
    /// Inline text, hard breaks and formulas with their supplied canonical marks/attributes.
    Inline(Vec<Node>),
    /// Complete canonical blocks inserted at the current parent boundary.
    Blocks(Vec<Node>),
}

pub(crate) fn block_id_reservations(document: &Document) -> HashSet<String> {
    fn collect(node: &Node, output: &mut HashSet<String>) {
        if let Some(id) = node.block_id() {
            output.insert(id.to_owned());
        }
        for child in node.children() {
            collect(child, output);
        }
    }
    let mut output = HashSet::new();
    collect(document.root(), &mut output);
    output
}

pub(crate) fn allocate_block_id(
    prefix: u64,
    next_id: &mut u64,
    reserved: &mut HashSet<String>,
) -> Result<String> {
    loop {
        *next_id = next_id
            .checked_add(1)
            .ok_or(craft_editor_model::Error::ResourceLimit)?;
        let id = format!("craft-view-{prefix}-{next_id}");
        if reserved.insert(id.clone()) {
            return Ok(id);
        }
    }
}

pub(crate) struct Plan {
    editor: Editor,
    pub(crate) commands: Vec<Command>,
    pub(crate) next_id: u64,
    prefix: u64,
    reserved: HashSet<String>,
}
impl Plan {
    pub(crate) fn new(session: &EditorSession, incoming: &[Node]) -> Result<Self> {
        let mut reserved = block_id_reservations(session.document());
        fn reserve(nodes: &[Node], reserved: &mut HashSet<String>) -> Result<()> {
            for node in nodes {
                node.required_schema_version()?;
                if let Some(id) = node.block_id()
                    && !reserved.insert(id.to_owned())
                {
                    return Err(craft_editor_model::Error::InvalidDocument(
                        "duplicate_block_id".into(),
                    )
                    .into());
                }
                reserve(node.children(), reserved)?;
            }
            Ok(())
        }
        reserve(incoming, &mut reserved)?;
        Ok(Self {
            editor: Editor::new(session.document().clone(), session.revision()),
            commands: Vec::new(),
            next_id: session.next_id,
            prefix: session.revision().0,
            reserved,
        })
    }
    fn identify_incoming(&mut self, nodes: &mut [Node]) -> Result<()> {
        for node in nodes {
            if matches!(
                node.node_type.as_str(),
                "paragraph"
                    | "heading"
                    | "blockquote"
                    | "codeBlock"
                    | "callout"
                    | "table"
                    | "imageBlock"
                    | "mermaidBlock"
                    | "mathBlock"
            ) && node
                .attrs
                .as_ref()
                .and_then(|attrs| attrs.get("blockId"))
                .is_none_or(serde_json::Value::is_null)
            {
                let id = self.id()?;
                node.attrs
                    .get_or_insert_default()
                    .insert("blockId".into(), serde_json::Value::String(id));
            }
            if let Some(children) = &mut node.content {
                self.identify_incoming(children)?;
            }
        }
        Ok(())
    }
    pub(crate) fn doc(&self) -> &Document {
        self.editor.document()
    }
    pub(crate) fn execute(&mut self, commands: Vec<Command>) -> Result<()> {
        self.editor.apply(Transaction {
            base_revision: self.editor.revision(),
            commands: commands.clone(),
        })?;
        self.commands.extend(commands);
        Ok(())
    }
    pub(crate) fn id(&mut self) -> Result<String> {
        allocate_block_id(self.prefix, &mut self.next_id, &mut self.reserved)
    }
    fn path(&self, id: &str) -> Result<NodePath> {
        fn find(node: &Node, path: NodePath, id: &str) -> Option<NodePath> {
            if node.block_id() == Some(id) {
                return Some(path);
            }
            node.children()
                .iter()
                .enumerate()
                .find_map(|(index, node)| find(node, path.child(index), id))
        }
        find(self.doc().root(), NodePath::root(), id).ok_or(Error::InvalidSelection)
    }
    fn replace_inline(
        &mut self,
        id: &str,
        from: usize,
        to: usize,
        children: Vec<Node>,
    ) -> Result<()> {
        let target = self.doc().node_ref(&self.path(id)?)?;
        self.execute(vec![Command::ReplaceInline {
            target,
            from: ScalarOffset(from),
            to: ScalarOffset(to),
            children,
        }])
    }
    pub(crate) fn insert_text_fragment(
        &mut self,
        point: DocPoint,
        template: &Node,
        text: &str,
    ) -> Result<DocPoint> {
        let prototype = self.doc().block(&point.block_id)?.clone();
        let leaf = |value: &str| {
            let mut node = template.clone();
            node.text = Some(value.into());
            node
        };
        if prototype.node_type == "codeBlock" || !text.contains('\n') {
            if !text.is_empty() {
                self.replace_inline(
                    &point.block_id,
                    point.offset.0,
                    point.offset.0,
                    vec![leaf(text)],
                )?;
            }
            return Ok(DocPoint {
                block_id: point.block_id,
                offset: ScalarOffset(point.offset.0 + text.chars().count()),
            });
        }
        let lines: Vec<_> = text.split('\n').collect();
        let tail_id = self.id()?;
        self.execute(vec![Command::SplitBlock {
            block_id: point.block_id.clone(),
            at: point.offset,
            new_block_id: tail_id.clone(),
        }])?;
        if !lines[0].is_empty() {
            self.replace_inline(
                &point.block_id,
                point.offset.0,
                point.offset.0,
                vec![leaf(lines[0])],
            )?;
        }
        if lines.len() > 2 {
            let mut middle = Vec::new();
            for line in &lines[1..lines.len() - 1] {
                let mut block = prototype.clone();
                block
                    .attrs
                    .get_or_insert_default()
                    .insert("blockId".into(), serde_json::Value::String(self.id()?));
                block.content = Some(if line.is_empty() {
                    vec![]
                } else {
                    vec![leaf(line)]
                });
                middle.push(block);
            }
            let tail_path = self.path(&tail_id)?;
            let parent = self
                .doc()
                .node_ref(&tail_path.parent().ok_or(Error::InvalidSelection)?)?;
            let index = *tail_path.0.last().ok_or(Error::InvalidSelection)?;
            self.execute(vec![Command::InsertChildren {
                parent,
                index,
                children: middle,
            }])?;
        }
        let last = lines.last().ok_or(Error::InvalidSelection)?;
        if !last.is_empty() {
            self.replace_inline(&tail_id, 0, 0, vec![leaf(last)])?;
        }
        Ok(DocPoint {
            block_id: tail_id,
            offset: ScalarOffset(last.chars().count()),
        })
    }
    fn delete_range(&mut self, selection: &Selection) -> Result<DocPoint> {
        let projection = Projection::new(self.doc())?;
        let a = projection.scalar(&selection.anchor)?;
        let b = projection.scalar(&selection.head)?;
        let first = if a <= b {
            selection.anchor.clone()
        } else {
            selection.head.clone()
        };
        let last = if a <= b {
            selection.head.clone()
        } else {
            selection.anchor.clone()
        };
        if a == b {
            return Ok(first);
        }
        if first.block_id == last.block_id {
            self.replace_inline(&first.block_id, first.offset.0, last.offset.0, vec![])?;
            return Ok(first);
        }
        let left_path = self.path(&first.block_id)?;
        let right_path = self.path(&last.block_id)?;
        let left_parent = left_path.parent().ok_or(Error::InvalidSelection)?;
        let right_parent = right_path.parent().ok_or(Error::InvalidSelection)?;
        let left_len = self.doc().block_text(&first.block_id)?.chars().count();
        self.replace_inline(&first.block_id, first.offset.0, left_len, vec![])?;
        self.replace_inline(&last.block_id, 0, last.offset.0, vec![])?;
        if left_parent == right_parent {
            let left_index = *left_path.0.last().ok_or(Error::InvalidSelection)?;
            let right_index = *right_path.0.last().ok_or(Error::InvalidSelection)?;
            if right_index > left_index + 1 {
                let parent = self.doc().node_ref(&left_parent)?;
                self.execute(vec![Command::DeleteChildren {
                    parent,
                    from: left_index + 1,
                    to: right_index,
                }])?;
            }
            self.execute(vec![Command::JoinBlocks {
                left_id: first.block_id.clone(),
                right_id: last.block_id.clone(),
            }])?;
            return Ok(first);
        }
        // Cell text ranges clear content while retaining the canonical grid. More complex
        // wrapper joins are prepared separately rather than guessing a destructive lift.
        let left_table = table_ancestor(self.doc(), &left_path);
        let right_table = table_ancestor(self.doc(), &right_path);
        if left_table.is_some() && left_table == right_table {
            for span in projection.spans.iter().filter(|span| {
                !span.atom
                    && span.id != first.block_id
                    && span.id != last.block_id
                    && span.start >= a.min(b)
                    && span.end <= a.max(b)
            }) {
                self.replace_inline(&span.id, 0, span.node.inline_text().chars().count(), vec![])?;
            }
            for span in projection
                .spans
                .iter()
                .rev()
                .filter(|span| span.atom && span.start >= a.min(b) && span.end <= a.max(b))
            {
                let path = NodePath(span.path.clone());
                let parent_path = path.parent().ok_or(Error::InvalidSelection)?;
                let index = *path.0.last().ok_or(Error::InvalidSelection)?;
                let parent = self.doc().node_ref(&parent_path)?;
                if parent.expected.children().len() == 1 {
                    let paragraph = Node::paragraph(self.id()?, "");
                    self.execute(vec![Command::ReplaceChildren {
                        parent,
                        from: index,
                        to: index + 1,
                        children: vec![paragraph],
                    }])?;
                } else {
                    self.execute(vec![Command::DeleteChildren {
                        parent,
                        from: index,
                        to: index + 1,
                    }])?;
                }
            }
            return Ok(first);
        }
        Err(Error::UnsupportedStructuralEdit)
    }
}
fn table_ancestor(document: &Document, path: &NodePath) -> Option<NodePath> {
    (0..path.0.len()).rev().find_map(|length| {
        let ancestor = NodePath(path.0[..length].to_vec());
        document
            .node(&ancestor)
            .ok()
            .filter(|node| node.node_type == "table")
            .map(|_| ancestor)
    })
}
fn inline_len(nodes: &[Node]) -> usize {
    nodes
        .iter()
        .map(|node| node.inline_text().chars().count())
        .sum()
}
fn text_point_in(node: &Node, end: bool) -> Option<DocPoint> {
    if matches!(
        node.node_type.as_str(),
        "paragraph" | "heading" | "codeBlock"
    ) {
        return node.block_id().map(|id| DocPoint {
            block_id: id.into(),
            offset: ScalarOffset(if end {
                node.inline_text().chars().count()
            } else {
                0
            }),
        });
    }
    if end {
        node.children()
            .iter()
            .rev()
            .find_map(|node| text_point_in(node, end))
    } else {
        node.children()
            .iter()
            .find_map(|node| text_point_in(node, end))
    }
}
fn disposable_empty_paragraph(node: &Node) -> bool {
    node.node_type == "paragraph"
        && node.children().is_empty()
        && node.extra.is_empty()
        && node.marks.is_none()
        && node.attrs.as_ref().is_none_or(|attrs| {
            attrs.iter().all(|(key, value)| {
                key == "blockId"
                    || (key == "sourceRefs" && value.as_array().is_some_and(Vec::is_empty))
            })
        })
}
enum After {
    Text(DocPoint),
    Node(NodePath),
    Selection(Selection),
    IdentifiedNode(String),
}
impl EditorSession {
    /// Insert native plain text through the same atomic range, schema and selection planner.
    /// Newlines create real paragraph splits except inside code blocks, where they remain text.
    pub fn insert_plain_text(&mut self, text: &str) -> Result<Change> {
        if self.is_composing() {
            return Err(Error::CompositionActive);
        }
        if self.table_selection.is_some() {
            return self.replace_selected_cells(text, false);
        }
        if self.node_selection.is_some() {
            return self.replace_selected_node(text);
        }
        let selection = self.selection.clone().ok_or(Error::InvalidSelection)?;
        let mut plan = Plan::new(self, &[])?;
        let point = plan.delete_range(&selection)?;
        let marks = match &self.stored_marks {
            Some(marks) => marks.clone(),
            None => self.marks_at(&point)?,
        };
        let mut template = self.typing_leaf_at(&point)?;
        if template.marks.as_deref().unwrap_or_default() != marks.as_slice() {
            template.marks = Some(marks.clone());
        }
        let caret = plan.insert_text_fragment(point, &template, text)?;
        let next_id = plan.next_id;
        let change = self.apply_commands_with_selection(
            plan.commands,
            Selection {
                anchor: caret.clone(),
                head: caret,
            },
        )?;
        self.next_id = next_id;
        self.stored_marks = Some(marks);
        Ok(change)
    }
    /// Insert a paragraph break, splitting a list item or leaving an empty item as appropriate.
    ///
    /// Code blocks keep a literal newline. A selected opaque block is retained and followed by a
    /// new paragraph. The resulting tree and caret form one revision-guarded Undo/Redo entry.
    pub fn insert_paragraph(&mut self) -> Result<Change> {
        if self.is_composing() {
            return Err(Error::CompositionActive);
        }
        let mut plan = Plan::new(self, &[])?;
        let caret = if let Some(reference) = &self.node_selection {
            if reference.expected.node_type == "inlineMath" {
                return self.insert_plain_text("\n");
            }
            if plan.doc().node(&reference.path)? != reference.expected.as_ref() {
                return Err(craft_editor_model::Error::NodeReferenceConflict.into());
            }
            let parent_path = reference.path.parent().ok_or(Error::InvalidSelection)?;
            let index = *reference.path.0.last().ok_or(Error::InvalidSelection)?;
            let id = plan.id()?;
            let paragraph = Node::paragraph(&id, "");
            plan.execute(vec![Command::InsertChildren {
                parent: plan.doc().node_ref(&parent_path)?,
                index: index + 1,
                children: vec![paragraph],
            }])?;
            DocPoint {
                block_id: id,
                offset: ScalarOffset(0),
            }
        } else {
            let selection = self.selection.clone().ok_or(Error::InvalidSelection)?;
            let point = plan.delete_range(&selection)?;
            let path = plan.path(&point.block_id)?;
            let block = plan.doc().block(&point.block_id)?;
            if block.node_type == "codeBlock" {
                return self.insert_plain_text("\n");
            }
            let paragraph_index = *path.0.last().ok_or(Error::InvalidSelection)?;
            let item_path = path.parent().ok_or(Error::InvalidSelection)?;
            if block.node_type == "paragraph"
                && plan.doc().node(&item_path)?.node_type == "listItem"
            {
                let list_path = item_path.parent().ok_or(Error::InvalidSelection)?;
                let item_index = *item_path.0.last().ok_or(Error::InvalidSelection)?;
                let list = plan.doc().node_ref(&list_path)?;
                let item = plan.doc().node(&item_path)?;
                let empty_exit = item.children().len() == 1
                    && block.inline_text().is_empty()
                    && point.offset.0 == 0;
                let identified_item = item.block_id().is_some();
                let new_block_id = plan.id()?;
                let new_item_id = if identified_item {
                    Some(plan.id()?)
                } else {
                    None
                };
                let command = craft_editor_model::split_list_item(
                    plan.doc(),
                    list,
                    item_index,
                    paragraph_index,
                    point.offset,
                    &new_block_id,
                    new_item_id.as_deref(),
                )?;
                plan.execute(vec![command])?;
                if empty_exit {
                    point
                } else {
                    DocPoint {
                        block_id: new_block_id,
                        offset: ScalarOffset(0),
                    }
                }
            } else {
                return self.insert_plain_text("\n");
            }
        };
        let next_id = plan.next_id;
        let change = self.apply_commands_with_selection(
            plan.commands,
            Selection {
                anchor: caret.clone(),
                head: caret,
            },
        )?;
        self.next_id = next_id;
        Ok(change)
    }
    /// Insert a canonical hard break, retaining a literal newline inside code blocks.
    pub fn insert_hard_break(&mut self) -> Result<Change> {
        if self.selection.as_ref().is_some_and(|selection| {
            self.document()
                .block(&selection.head.block_id)
                .is_ok_and(|node| node.node_type == "codeBlock")
        }) {
            self.insert_plain_text("\n")
        } else {
            self.insert_content(ContentSlice::Inline(vec![Node::block(
                "hardBreak",
                serde_json::Map::new(),
                vec![],
            )]))
        }
    }
    /// Insert a closed canonical slice at the current selection as one rich transaction.
    ///
    /// Preserves supplied nodes and the unselected tail; schema/identity/placement failures do
    /// not change canonical content or history. Open clipboard slices require their own adapter.
    pub fn insert_content(&mut self, slice: ContentSlice) -> Result<Change> {
        self.insert_content_with_after(slice, None)
    }
    /// Insert a closed slice and record an exact post-edit text selection in the same history.
    ///
    /// The selection is checked against the prepared candidate before any live mutation. This
    /// supports commands such as table insertion that enter the first cell rather than the tail.
    pub fn insert_content_with_selection(
        &mut self,
        slice: ContentSlice,
        selection: Selection,
    ) -> Result<Change> {
        self.insert_content_with_after(slice, Some(After::Selection(selection)))
    }
    /// Insert a closed slice and select an identified opaque atom after resolving its actual path.
    ///
    /// A missing identity or a non-opaque destination rejects the entire insertion. Hosts do not
    /// need to predict how splitting a paragraph or replacing an empty block changes node paths.
    pub fn insert_content_selecting_node(
        &mut self,
        slice: ContentSlice,
        block_id: &str,
    ) -> Result<Change> {
        self.insert_content_with_after(slice, Some(After::IdentifiedNode(block_id.to_owned())))
    }
    fn insert_content_with_after(
        &mut self,
        slice: ContentSlice,
        explicit_after: Option<After>,
    ) -> Result<Change> {
        if self.is_composing() {
            return Err(Error::CompositionActive);
        }
        let (inline, mut nodes) = match slice {
            ContentSlice::Inline(nodes) => (true, nodes),
            ContentSlice::Blocks(nodes) => (false, nodes),
        };
        let mut plan = Plan::new(self, &nodes)?;
        plan.identify_incoming(&mut nodes)?;
        let mut selected_slot: Option<(NodePath, usize)> = None;
        let selection = if let Some(reference) = &self.node_selection {
            if plan.doc().node(&reference.path)? != reference.expected.as_ref() {
                return Err(craft_editor_model::Error::NodeReferenceConflict.into());
            }
            let parent_path = reference.path.parent().ok_or(Error::InvalidSelection)?;
            let index = *reference.path.0.last().ok_or(Error::InvalidSelection)?;
            if reference.expected.node_type == "inlineMath" {
                let parent = plan.doc().node(&parent_path)?;
                let id = parent
                    .block_id()
                    .ok_or(Error::MissingBlockIdentity)?
                    .to_owned();
                let from = inline_len(&parent.children()[..index]);
                Some(Selection {
                    anchor: DocPoint {
                        block_id: id.clone(),
                        offset: ScalarOffset(from),
                    },
                    head: DocPoint {
                        block_id: id,
                        offset: ScalarOffset(from + 1),
                    },
                })
            } else {
                selected_slot = Some((parent_path, index));
                None
            }
        } else {
            self.selection.clone()
        };
        let after = if let Some((parent_path, index)) = selected_slot {
            let mut children = if inline {
                let id = plan.id()?;
                let mut paragraph = Node::paragraph(id, "");
                paragraph.content = Some(nodes);
                vec![paragraph]
            } else {
                nodes
            };
            if children.is_empty() && plan.doc().node(&parent_path)?.children().len() == 1 {
                children.push(Node::paragraph(plan.id()?, ""));
            }
            let parent = plan.doc().node_ref(&parent_path)?;
            let selected = children.last().cloned();
            let count = children.len();
            plan.execute(vec![Command::ReplaceChildren {
                parent,
                from: index,
                to: index + 1,
                children,
            }])?;
            if let Some(node) = selected {
                if let Some(point) = text_point_in(&node, true) {
                    After::Text(point)
                } else {
                    After::Node(parent_path.child(index + count - 1))
                }
            } else {
                let projection = Projection::new(plan.doc())?;
                After::Text(projection.point(0, true)?)
            }
        } else {
            let selection = selection.ok_or(Error::InvalidSelection)?;
            let point = plan.delete_range(&selection)?;
            if inline {
                let length = inline_len(&nodes);
                plan.replace_inline(&point.block_id, point.offset.0, point.offset.0, nodes)?;
                After::Text(DocPoint {
                    block_id: point.block_id,
                    offset: ScalarOffset(point.offset.0 + length),
                })
            } else if nodes.is_empty() {
                After::Text(point)
            } else {
                let path = plan.path(&point.block_id)?;
                let parent_path = path.parent().ok_or(Error::InvalidSelection)?;
                let index = *path.0.last().ok_or(Error::InvalidSelection)?;
                let block = plan.doc().block(&point.block_id)?.clone();
                let length = block.inline_text().chars().count();
                let mandatory_list_lead =
                    index == 0 && plan.doc().node(&parent_path)?.node_type == "listItem";
                let mut right_tail: Option<DocPoint> = None;
                let first_is_paragraph = nodes
                    .first()
                    .is_some_and(|node| node.node_type == "paragraph");
                let (at, replace) = if disposable_empty_paragraph(&block)
                    && (!mandatory_list_lead || first_is_paragraph)
                {
                    (index, 1)
                } else if point.offset.0 == 0 && !mandatory_list_lead {
                    right_tail = Some(DocPoint {
                        block_id: point.block_id.clone(),
                        offset: ScalarOffset(0),
                    });
                    (index, 0)
                } else if point.offset.0 == length {
                    (index + 1, 0)
                } else {
                    let tail_id = plan.id()?;
                    plan.execute(vec![Command::SplitBlock {
                        block_id: point.block_id.clone(),
                        at: point.offset,
                        new_block_id: tail_id.clone(),
                    }])?;
                    right_tail = Some(DocPoint {
                        block_id: tail_id,
                        offset: ScalarOffset(0),
                    });
                    (index + 1, 0)
                };
                let last = nodes.last().cloned().ok_or(Error::InvalidSelection)?;
                let count = nodes.len();
                let parent = plan.doc().node_ref(&parent_path)?;
                plan.execute(vec![Command::ReplaceChildren {
                    parent,
                    from: at,
                    to: at + replace,
                    children: nodes,
                }])?;
                if let Some(point) = right_tail.or_else(|| text_point_in(&last, true)) {
                    After::Text(point)
                } else {
                    After::Node(parent_path.child(at + count - 1))
                }
            }
        };
        let next_id = plan.next_id;
        let change = match explicit_after.unwrap_or(after) {
            After::Selection(selection) => {
                self.apply_commands_with_selection(plan.commands, selection)?
            }
            After::IdentifiedNode(id) => {
                let reference = plan.doc().node_ref(&plan.path(&id)?)?;
                self.apply_commands_with_node_selection(plan.commands, reference)?
            }
            After::Text(point) => self.apply_commands_with_selection(
                plan.commands,
                Selection {
                    anchor: point.clone(),
                    head: point,
                },
            )?,
            After::Node(path) => {
                let reference: NodeRef = plan.doc().node_ref(&path)?;
                self.apply_commands_with_node_selection(plan.commands, reference)?
            }
        };
        self.next_id = next_id;
        Ok(change)
    }
}

impl EditorSession {
    pub(crate) fn insert_open_slice(
        &mut self,
        slice: crate::clipboard::RichSlice,
    ) -> Result<Change> {
        self.insert_fitted_slice(slice)
    }
}

impl EditorSession {
    fn insert_fitted_slice(&mut self, mut slice: crate::clipboard::RichSlice) -> Result<Change> {
        if self.is_composing() {
            return Err(Error::CompositionActive);
        }
        let mut plan = Plan::new(self, &slice.content)?;
        plan.identify_incoming(&mut slice.content)?;
        let mut allocate = || {
            plan.id().map_err(|error| match error {
                Error::Model(error) => error,
                _ => craft_editor_model::Error::UnsupportedEdit,
            })
        };
        let (command, map) = if let Some(target) = self.node_selection.clone() {
            craft_editor_model::plan_node_slice_replacement(
                self.document(),
                target,
                slice.content,
                slice.open_start,
                slice.open_end,
                &mut allocate,
            )?
        } else {
            let selection = self.selection.as_ref().ok_or(Error::InvalidSelection)?;
            craft_editor_model::plan_slice_replacement(
                self.document(),
                craft_editor_model::TextPoint {
                    block_id: selection.anchor.block_id.clone(),
                    offset: selection.anchor.offset,
                },
                craft_editor_model::TextPoint {
                    block_id: selection.head.block_id.clone(),
                    offset: selection.head.offset,
                },
                slice.content,
                slice.open_start,
                slice.open_end,
                &mut allocate,
            )?
        };
        let next = plan.next_id;
        let change = if let Some(point) = map.inserted_caret() {
            let caret = DocPoint {
                block_id: point.block_id,
                offset: point.offset,
            };
            self.apply_commands_with_selection(
                vec![command],
                Selection {
                    anchor: caret.clone(),
                    head: caret,
                },
            )?
        } else if let Some(path) = map.inserted_node() {
            let mut candidate = Editor::new(self.document().clone(), self.revision());
            candidate.apply(Transaction {
                base_revision: self.revision(),
                commands: vec![command.clone()],
            })?;
            let reference = candidate.document().node_ref(&path)?;
            self.apply_commands_with_node_selection(vec![command], reference)?
        } else {
            return Err(Error::InvalidSelection);
        };
        self.next_id = next;
        Ok(change)
    }
}
