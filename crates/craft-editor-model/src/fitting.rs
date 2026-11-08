//! Native structural fitting with private canonical envelope transport.
use crate::{
    Command, Document, Error, Mark, Node, NodePath, NodeRef, Result, ScalarOffset, TextPoint,
};
use prosemirror::{
    dynamic::{DynamicSchema, schema::SchemaSpec, types::Dyn},
    model::{Fragment, Node as _, NodeType as _, Slice},
    transform::{Mappable, StepMap, Transform},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
const NODE_TOKEN: &str = "__craftEnvelope";
const LEAF_TOKEN: &str = "__craftLeafEnvelope";
const MARK_TOKEN: &str = "__craftMarkEnvelope";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct BlockCoordinate {
    id: String,
    start: usize,
    length: usize,
    surrogates: Vec<usize>,
}
impl BlockCoordinate {
    fn units(&self, offset: usize) -> Option<usize> {
        (offset <= self.length)
            .then(|| offset + self.surrogates.partition_point(|index| *index < offset))
    }
    fn scalar(&self, units: usize) -> Option<usize> {
        let mut scalar = units;
        for (extra, index) in self.surrogates.iter().enumerate() {
            let boundary = index + extra;
            if units == boundary + 1 {
                return None;
            }
            if units > boundary {
                scalar = scalar.checked_sub(1)?;
            } else {
                break;
            }
        }
        (scalar <= self.length).then_some(scalar)
    }
    fn end(&self) -> usize {
        self.start + self.length + self.surrogates.len()
    }
}
/// Exact block-local mapping computed from a native fit, never supplied as command authority.
/// Compact surrogate indexes retain explicit UTF-16/scalar conversion without copying writing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FittedMap {
    before: Vec<BlockCoordinate>,
    after: Vec<BlockCoordinate>,
    steps: Vec<StepMap>,
    paths: Vec<(NodePath, NodePath)>,
    atoms: Vec<(usize, NodePath)>,
    inserted_end: usize,
    inserted_right: bool,
}
impl FittedMap {
    fn global(&self, point: &TextPoint) -> Option<usize> {
        let block = self
            .before
            .iter()
            .find(|block| block.id == point.block_id)?;
        block
            .units(point.offset.0)
            .map(|offset| block.start + offset)
    }
    fn mapped_global(&self, point: &TextPoint, right: bool, detach: bool) -> Option<usize> {
        let mut pos = self.global(point)?;
        for step in &self.steps {
            let mapped = step.map_result(pos, if right { 1 } else { -1 });
            if detach && mapped.deleted() {
                return None;
            }
            pos = mapped.pos;
        }
        Some(pos)
    }
    fn resolve(&self, pos: usize, right: bool) -> Option<TextPoint> {
        let exact = self.after.iter().find_map(|block| {
            if (block.start..=block.end()).contains(&pos) {
                block.scalar(pos - block.start).map(|offset| TextPoint {
                    block_id: block.id.clone(),
                    offset: ScalarOffset(offset),
                })
            } else {
                None
            }
        });
        if exact.is_some() {
            return exact;
        }
        let next = self
            .after
            .iter()
            .find(|block| block.start >= pos)
            .map(|block| TextPoint {
                block_id: block.id.clone(),
                offset: ScalarOffset(0),
            });
        let previous = self
            .after
            .iter()
            .rev()
            .find(|block| block.end() <= pos)
            .map(|block| TextPoint {
                block_id: block.id.clone(),
                offset: ScalarOffset(block.length),
            });
        if right {
            next.or(previous)
        } else {
            previous.or(next)
        }
    }
    /// Map a surviving existing caret, explicitly detaching deleted boundaries.
    pub fn map_existing_point(&self, point: &TextPoint, right: bool) -> Option<TextPoint> {
        self.resolve(self.mapped_global(point, right, true)?, right)
    }
    /// Resolve a post-insertion caret with boundary affinity, including structural fitting.
    pub fn insertion_point(&self, point: &TextPoint, right: bool) -> Option<TextPoint> {
        self.resolve(self.mapped_global(point, right, false)?, right)
    }
    /// Resolve the nearest opaque node when a fitted document has no text caret.
    pub fn insertion_node(&self, point: &TextPoint) -> Option<NodePath> {
        let pos = self.mapped_global(point, true, false)?;
        self.atoms
            .iter()
            .rev()
            .find(|(at, _)| *at <= pos)
            .or(self.atoms.first())
            .map(|(_, path)| path.clone())
    }
    fn inserted_selection(&self) -> Option<FittedSelection> {
        let pos = self.inserted_end;
        if let Some(block) = self
            .after
            .iter()
            .find(|block| (block.start..=block.end()).contains(&pos))
        {
            return block.scalar(pos - block.start).map(|offset| {
                FittedSelection::Text(TextPoint {
                    block_id: block.id.clone(),
                    offset: ScalarOffset(offset),
                })
            });
        }
        for right in [self.inserted_right, !self.inserted_right] {
            let text = if right {
                self.after
                    .iter()
                    .find(|block| block.start >= pos)
                    .map(|block| (block.start, block, 0))
            } else {
                self.after
                    .iter()
                    .rev()
                    .find(|block| block.end() <= pos)
                    .map(|block| (block.end(), block, block.length))
            };
            let atom = if right {
                self.atoms
                    .iter()
                    .find(|(at, _)| *at >= pos)
                    .map(|(at, path)| (*at, path))
            } else {
                self.atoms
                    .iter()
                    .rev()
                    .find(|(at, _)| *at < pos)
                    .map(|(at, path)| (*at + 1, path))
            };
            if let Some((at, path)) = atom
                && text
                    .is_none_or(|(text_at, _, _)| if right { at <= text_at } else { at >= text_at })
            {
                return Some(FittedSelection::Node(path.clone()));
            }
            if let Some((_, block, offset)) = text {
                return Some(FittedSelection::Text(TextPoint {
                    block_id: block.id.clone(),
                    offset: ScalarOffset(offset),
                }));
            }
        }
        None
    }
    /// Caret at the actual fitted insertion end, independent of original selection direction.
    pub fn inserted_caret(&self) -> Option<TextPoint> {
        match self.inserted_selection()? {
            FittedSelection::Text(point) => Some(point),
            FittedSelection::Node(_) => None,
        }
    }
    /// Opaque-node selection when the source-compatible insertion search reaches an atom first.
    pub fn inserted_node(&self) -> Option<NodePath> {
        match self.inserted_selection()? {
            FittedSelection::Node(path) => Some(path),
            FittedSelection::Text(_) => None,
        }
    }
    pub(crate) fn map_path(&self, path: &NodePath) -> Option<NodePath> {
        self.paths
            .iter()
            .find(|(before, _)| before == path)
            .map(|(_, after)| after.clone())
    }
}
enum FittedSelection {
    Text(TextPoint),
    Node(NodePath),
}
fn insertion_search_right(content: &[Node], open_end: usize) -> bool {
    let mut last = content.last();
    let mut parent = None;
    for _ in 0..open_end {
        parent = last;
        last = last.and_then(|node| node.children().last());
    }
    !last.map_or_else(
        || parent.is_some_and(is_text_block),
        |node| matches!(node.node_type.as_str(), "text" | "hardBreak" | "inlineMath"),
    )
}
fn is_text_block(node: &Node) -> bool {
    matches!(
        node.node_type.as_str(),
        "paragraph" | "heading" | "codeBlock"
    )
}
fn is_atom(node: &Node) -> bool {
    matches!(
        node.node_type.as_str(),
        "hardBreak"
            | "inlineMath"
            | "horizontalRule"
            | "sceneBreak"
            | "imageBlock"
            | "mermaidBlock"
            | "mathBlock"
    )
}
fn size(node: &Node) -> usize {
    if node.node_type == "text" {
        node.text
            .as_deref()
            .unwrap_or_default()
            .encode_utf16()
            .count()
    } else if is_atom(node) {
        1
    } else {
        node.children().iter().map(size).sum::<usize>() + 2
    }
}
fn coordinates(root: &Node) -> Vec<BlockCoordinate> {
    fn visit(node: &Node, pos: usize, out: &mut Vec<BlockCoordinate>) {
        if is_text_block(node)
            && let Some(id) = node.block_id()
        {
            let text = node.inline_text();
            out.push(BlockCoordinate {
                id: id.into(),
                start: pos + 1,
                length: text.chars().count(),
                surrogates: text
                    .chars()
                    .enumerate()
                    .filter_map(|(index, ch)| (ch.len_utf16() == 2).then_some(index))
                    .collect(),
            });
        }
        let mut cursor = pos + 1;
        for child in node.children() {
            visit(child, cursor, out);
            cursor += size(child);
        }
    }
    let mut out = Vec::new();
    let mut pos = 0;
    for node in root.children() {
        visit(node, pos, &mut out);
        pos += size(node);
    }
    out
}
fn global(blocks: &[BlockCoordinate], point: &TextPoint) -> Result<usize> {
    let block = blocks
        .iter()
        .find(|block| block.id == point.block_id)
        .ok_or(Error::BlockNotFound)?;
    Ok(block.start + block.units(point.offset.0).ok_or(Error::InvalidPosition)?)
}
struct Envelope<'a> {
    node: &'a Node,
    path: Option<NodePath>,
    start: usize,
    incoming: bool,
    mark_indices: Vec<usize>,
}
struct Bridge<'a> {
    nodes: Vec<Envelope<'a>>,
    marks: Vec<&'a Mark>,
    spec: Value,
    node_groups: HashMap<Vec<u8>, usize>,
    mark_groups: HashMap<Vec<u8>, usize>,
    node_tokens: Vec<usize>,
}
impl<'a> Bridge<'a> {
    fn new() -> Result<Self> {
        let mut spec: Value = serde_json::from_str(include_str!("fitting_schema.json"))?;
        spec["marks"][LEAF_TOKEN] = json!({"attrs":{"token":{"default":null}},"excludes":""});
        // Private metadata must coexist with code while the structural planner filters real marks.
        spec["marks"]["code"]["excludes"] = json!("");
        spec["nodes"]["codeBlock"]["marks"] = json!(LEAF_TOKEN);
        Ok(Self {
            nodes: Vec::new(),
            marks: Vec::new(),
            spec,
            node_groups: HashMap::new(),
            mark_groups: HashMap::new(),
            node_tokens: Vec::new(),
        })
    }
    fn schema(&self, top_node: &str) -> Result<DynamicSchema> {
        // Filler preference follows the actual source's ordered schema, not Value's
        // lexicographically sorted map. Paragraph is the source's default block.
        let mut spec: SchemaSpec = serde_json::from_str(include_str!("fitting_schema.json"))?;
        spec.top_node = top_node.into();
        for (name, node) in &mut spec.nodes {
            *node = serde_json::from_value(self.spec["nodes"][name].clone())?;
        }
        for (name, mark) in &mut spec.marks {
            *mark = serde_json::from_value(self.spec["marks"][name].clone())?;
        }
        spec.marks.insert(
            LEAF_TOKEN.into(),
            serde_json::from_value(self.spec["marks"][LEAF_TOKEN].clone())?,
        );
        DynamicSchema::from_spec(spec).map_err(|_| Error::InvalidDocument("fitting_schema".into()))
    }
    fn encode(
        &mut self,
        node: &'a Node,
        path: Option<NodePath>,
        start: usize,
        incoming: bool,
    ) -> Result<Value> {
        let token = self.nodes.len();
        if token >= 200_000 {
            return Err(Error::ResourceLimit);
        }
        self.nodes.push(Envelope {
            node,
            path: path.clone(),
            start,
            incoming,
            mark_indices: Vec::new(),
        });
        let marker = if node.node_type == "text" || is_atom(node) {
            token
        } else {
            let key = serde_json::to_vec(&node.content_envelope())?;
            *self.node_groups.entry(key).or_insert(token)
        };
        self.node_tokens.push(marker);
        if node.node_type != "text" {
            self.spec["nodes"][&node.node_type]["attrs"][NODE_TOKEN] = json!({"default":null});
        }
        let mut attrs = serde_json::Map::new();
        for (key, value) in node.attrs.as_ref().into_iter().flatten() {
            if self.spec["nodes"][&node.node_type]["attrs"]
                .get(key)
                .is_some()
            {
                attrs.insert(key.clone(), value.clone());
            }
        }
        if node.node_type != "text" {
            attrs.insert(NODE_TOKEN.into(), json!(marker));
        }
        let mut value = if node.node_type == "text" {
            json!({"type":node.node_type})
        } else {
            json!({"type":node.node_type,"attrs":attrs})
        };
        if let Some(text) = &node.text {
            value["text"] = json!(text);
        }
        let mut marks = Vec::new();
        for mark in node.marks.as_deref().unwrap_or_default() {
            let key = serde_json::to_vec(mark)?;
            let index = if let Some(index) = self.mark_groups.get(&key) {
                *index
            } else {
                let index = self.marks.len();
                self.marks.push(mark);
                self.mark_groups.insert(key, index);
                index
            };
            self.nodes[token].mark_indices.push(index);
            let mut attrs = serde_json::Map::new();
            for (key, value) in mark.attrs.as_ref().into_iter().flatten() {
                if self.spec["marks"][&mark.mark_type]["attrs"]
                    .get(key)
                    .is_some()
                {
                    attrs.insert(key.clone(), value.clone());
                }
            }
            attrs.insert(MARK_TOKEN.into(), json!(index));
            self.spec["marks"][&mark.mark_type]["attrs"][MARK_TOKEN] = json!({"default":null});
            marks.push(json!({"type":mark.mark_type,"attrs":attrs}));
        }
        if node.node_type == "text" || matches!(node.node_type.as_str(), "hardBreak" | "inlineMath")
        {
            marks.push(json!({"type":LEAF_TOKEN,"attrs":{"token":token}}));
        }
        if !marks.is_empty() {
            value["marks"] = json!(marks);
        }
        if !node.children().is_empty() {
            let mut children = Vec::new();
            let mut cursor = if node.node_type == "doc" {
                0
            } else {
                start + 1
            };
            for (index, child) in node.children().iter().enumerate() {
                children.push(self.encode(
                    child,
                    path.as_ref().map(|path| path.child(index)),
                    cursor,
                    incoming,
                )?);
                cursor += size(child);
            }
            value["content"] = json!(children);
        }
        Ok(value)
    }
}
struct Reconstruct<'a, 'b, F: FnMut() -> Result<String>> {
    bridge: &'b Bridge<'a>,
    allocate: F,
    used: HashSet<String>,
    reserved: HashSet<String>,
    seen: Vec<usize>,
    text: HashMap<usize, String>,
    paths: Vec<(NodePath, NodePath)>,
    atoms: Vec<(usize, NodePath)>,
}
impl<F: FnMut() -> Result<String>> Reconstruct<'_, '_, F> {
    fn node(&mut self, value: &Value, path: NodePath, pos: usize) -> Result<Node> {
        let kind = value["type"].as_str().ok_or(Error::UnsupportedEdit)?;
        let leaf = value["marks"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|mark| mark["type"] == LEAF_TOKEN)
            .and_then(|mark| mark["attrs"]["token"].as_u64());
        let token = leaf
            .or_else(|| value["attrs"][NODE_TOKEN].as_u64())
            .map(|index| usize::try_from(index).map_err(|_| Error::ResourceLimit))
            .transpose()?;
        let bridge = self.bridge;
        let original = token
            .map(|index| bridge.nodes.get(index).ok_or(Error::UnsupportedEdit))
            .transpose()?;
        let mut node = if let Some(original) = original {
            original.node.content_envelope()
        } else {
            let mut attrs = value["attrs"].as_object().cloned().unwrap_or_default();
            attrs.remove(NODE_TOKEN);
            Node::block(kind, attrs, vec![])
        };
        node.node_type = kind.into();
        if let Some(original) = original {
            let index = token.ok_or(Error::UnsupportedEdit)?;
            self.seen[index] += 1;
            if original.node.node_type == "text" {
                let fragment = if kind == "hardBreak" {
                    "\n"
                } else {
                    value["text"].as_str().ok_or(Error::UnsupportedEdit)?
                };
                self.text.entry(index).or_default().push_str(fragment);
            }
        }
        if kind == "text" {
            node.text = Some(value["text"].as_str().ok_or(Error::UnsupportedEdit)?.into());
        } else {
            node.text = None;
        }
        let indices = value["marks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|mark| mark["type"] != LEAF_TOKEN)
            .map(|mark| {
                mark["attrs"][MARK_TOKEN]
                    .as_u64()
                    .and_then(|index| usize::try_from(index).ok())
                    .ok_or(Error::UnsupportedEdit)
            })
            .collect::<Result<HashSet<_>>>()?;
        let mut marks = Vec::new();
        if let Some(original) = original {
            for (index, mark) in original
                .mark_indices
                .iter()
                .zip(original.node.marks.as_deref().unwrap_or_default())
            {
                if indices.contains(index) {
                    marks.push(mark.clone());
                }
            }
            if original.node.marks.is_some() || !marks.is_empty() {
                node.marks = Some(marks);
            } else {
                node.marks = None;
            }
        } else if !indices.is_empty() {
            for index in indices {
                marks.push((*self.bridge.marks.get(index).ok_or(Error::UnsupportedEdit)?).clone());
            }
            node.marks = Some(marks);
        }
        let mut children = Vec::new();
        let mut cursor = if kind == "doc" { 0 } else { pos + 1 };
        for (index, child) in value["content"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            let child = self.node(child, path.child(index), cursor)?;
            cursor += size(&child);
            children.push(child);
        }
        if node.content.is_some() || !children.is_empty() {
            let mut merged: Vec<Node> = Vec::with_capacity(children.len());
            let mut indices = Vec::with_capacity(children.len());
            let mut combined = HashSet::new();
            for child in children {
                if let Some(previous) = merged.last_mut()
                    && previous.can_coalesce_text(&child)
                    && let (Some(left), Some(right)) = (&mut previous.text, &child.text)
                {
                    left.push_str(right);
                    let index = merged.len() - 1;
                    combined.insert(index);
                    indices.push(index);
                } else {
                    indices.push(merged.len());
                    merged.push(child);
                }
            }
            self.paths.retain_mut(|(_, after)| {
                if after.0.starts_with(&path.0) && after.0.len() > path.0.len() {
                    let index = &mut after.0[path.0.len()];
                    if let Some(mapped) = indices.get(*index) {
                        if combined.contains(mapped) {
                            return false;
                        }
                        *index = *mapped;
                    }
                }
                true
            });
            node.content = Some(merged);
        }
        let identified = matches!(
            kind,
            "paragraph"
                | "heading"
                | "blockquote"
                | "codeBlock"
                | "callout"
                | "table"
                | "imageBlock"
                | "mermaidBlock"
                | "mathBlock"
        );
        if identified || node.block_id().is_some() {
            let keep = node
                .block_id()
                .map(str::to_owned)
                .filter(|id| self.used.insert(id.clone()));
            if keep.is_none() {
                let id = (self.allocate)()?;
                if self.reserved.contains(&id) || !self.used.insert(id.clone()) {
                    return Err(Error::InvalidDocument("fitting_fresh_identity".into()));
                }
                self.reserved.insert(id.clone());
                node.attrs
                    .get_or_insert_default()
                    .insert("blockId".into(), json!(id));
            }
        }
        if is_atom(&node) && !matches!(kind, "hardBreak" | "inlineMath") {
            self.atoms.push((pos, path.clone()));
        }
        if let Some(original) = original
            && let Some(old_path) = &original.path
            && original.node == &node
        {
            self.paths.push((old_path.clone(), path));
        }
        Ok(node)
    }
}
/// Plan a typed, replayable rich slice command with explicit caller-allocated fresh identities.
/// The model computes all structural/Unicode maps itself. No resource is fetched or opened.
pub fn plan_slice_replacement<F: FnMut() -> Result<String>>(
    document: &Document,
    from: TextPoint,
    to: TextPoint,
    content: Vec<Node>,
    open_start: usize,
    open_end: usize,
    mut allocate: F,
) -> Result<(Command, FittedMap)> {
    let mut fresh_ids = Vec::new();
    let (_, map) = fit(
        document,
        &from,
        &to,
        &content,
        open_start,
        open_end,
        &mut || {
            let id = allocate()?;
            fresh_ids.push(id.clone());
            Ok(id)
        },
    )?;
    Ok((
        Command::ReplaceSlice {
            from_block_id: from.block_id,
            from: from.offset,
            to_block_id: to.block_id,
            to: to.offset,
            content,
            open_start,
            open_end,
            fresh_ids,
        },
        map,
    ))
}
/// Plan an open or closed rich slice replacing one exact guarded node.
/// Structural mapping retains unaffected text identities, including notes outside that node.
pub fn plan_node_slice_replacement<F: FnMut() -> Result<String>>(
    document: &Document,
    target: NodeRef,
    content: Vec<Node>,
    open_start: usize,
    open_end: usize,
    mut allocate: F,
) -> Result<(Command, FittedMap)> {
    let mut fresh_ids = Vec::new();
    let (_, map) = fit_node(
        document,
        &target,
        &content,
        open_start,
        open_end,
        &mut || {
            let id = allocate()?;
            fresh_ids.push(id.clone());
            Ok(id)
        },
    )?;
    Ok((
        Command::ReplaceNodeSlice {
            target,
            content,
            open_start,
            open_end,
            fresh_ids,
        },
        map,
    ))
}
fn node_range(document: &Document, target: &NodeRef) -> Result<std::ops::Range<usize>> {
    crate::structural::check_ref(document, target)?;
    if target.path.0.is_empty()
        || matches!(target.expected.node_type.as_str(), "text" | "hardBreak")
    {
        return Err(Error::InvalidPosition);
    }
    let mut node = document.root();
    let mut pos = 0;
    for (depth, index) in target.path.0.iter().copied().enumerate() {
        if depth > 0 {
            pos += 1;
        }
        pos += node.children().iter().take(index).map(size).sum::<usize>();
        node = node.children().get(index).ok_or(Error::InvalidPosition)?;
    }
    Ok(pos..pos + size(node))
}
pub(crate) fn fit_node<F: FnMut() -> Result<String>>(
    document: &Document,
    target: &NodeRef,
    content: &[Node],
    open_start: usize,
    open_end: usize,
    allocate: &mut F,
) -> Result<(Document, FittedMap)> {
    let range = node_range(document, target)?;
    if open_start == 0
        && open_end == 0
        && !content.is_empty()
        && target.expected.node_type != "inlineMath"
        && content
            .iter()
            .all(|node| matches!(node.node_type.as_str(), "text" | "hardBreak" | "inlineMath"))
    {
        // The pinned native fitter cannot synthesize the source's default textblock
        // when bare inline content replaces a whole block. Make that proven source
        // wrapper explicit, retaining each original inline envelope and fresh ID.
        let paragraph = Node::block(
            "paragraph",
            serde_json::Map::from_iter([
                ("blockId".into(), json!(allocate()?)),
                ("sourceRefs".into(), json!([])),
            ]),
            content.to_vec(),
        );
        return fit_at(document, range, &[paragraph], 0, 0, allocate);
    }
    fit_at(document, range, content, open_start, open_end, allocate)
}
pub(crate) fn edge_depth(nodes: &[Node], last: bool) -> usize {
    let node = if last { nodes.last() } else { nodes.first() };
    match node {
        Some(node) if node.node_type != "text" && !is_atom(node) => {
            1 + edge_depth(node.children(), last)
        }
        _ => 0,
    }
}
pub(crate) fn fit<F: FnMut() -> Result<String>>(
    document: &Document,
    from: &TextPoint,
    to: &TextPoint,
    content: &[Node],
    open_start: usize,
    open_end: usize,
    allocate: &mut F,
) -> Result<(Document, FittedMap)> {
    let before = coordinates(document.root());
    let from = global(&before, from)?;
    let to = global(&before, to)?;
    fit_at(
        document,
        from.min(to)..from.max(to),
        content,
        open_start,
        open_end,
        allocate,
    )
}
fn fit_at<F: FnMut() -> Result<String>>(
    document: &Document,
    range: std::ops::Range<usize>,
    content: &[Node],
    open_start: usize,
    open_end: usize,
    allocate: &mut F,
) -> Result<(Document, FittedMap)> {
    if open_start > 64
        || open_end > 64
        || open_start > edge_depth(content, false)
        || open_end > edge_depth(content, true)
    {
        return Err(Error::InvalidPosition);
    }
    Node::decode_open_fragment(serde_json::to_value(content)?)?;
    let before = coordinates(document.root());
    let (from_pos, to_pos) = (range.start, range.end);
    fn ids(node: &Node, out: &mut HashSet<String>) {
        if let Some(id) = node.block_id() {
            out.insert(id.into());
        }
        for child in node.children() {
            ids(child, out);
        }
    }
    let mut old_ids = HashSet::new();
    ids(document.root(), &mut old_ids);
    let mut incoming_ids = HashSet::new();
    for node in content {
        ids(node, &mut incoming_ids);
    }
    if incoming_ids.iter().any(|id| old_ids.contains(id)) {
        return Err(Error::InvalidDocument("fitting_identity_conflict".into()));
    }
    let mut bridge = Bridge::new()?;
    let native_doc = bridge.encode(document.root(), Some(NodePath::root()), 0, false)?;
    let mut native_content = Vec::new();
    for node in content {
        native_content.push(bridge.encode(node, None, 0, true)?);
    }
    let schema = bridge.schema("doc")?;
    let (value, steps, incoming_size) = schema.with_types(|| -> Result<_> {
        let native = schema
            .node_from_json(&native_doc)
            .map_err(|_| Error::UnsupportedEdit)?;
        let incoming = native_content
            .iter()
            .map(|node| {
                schema
                    .node_from_json(node)
                    .map_err(|_| Error::UnsupportedEdit)
            })
            .collect::<Result<Vec<_>>>()?;
        let slice = Slice::new(Fragment::from(incoming), open_start, open_end);
        let incoming_size = slice.size();
        let mut transform = Transform::<Dyn>::new(native);
        transform.replace_range(from_pos, to_pos, slice);
        if transform.steps.is_empty() && (incoming_size > 0 || from_pos != to_pos) {
            return Err(Error::UnsupportedEdit);
        }
        let value = serde_json::to_value(&transform.doc)?;
        Ok((value, transform.mapping.maps, incoming_size))
    })?;
    if incoming_size == 0 && from_pos == to_pos {
        let paths = bridge
            .nodes
            .iter()
            .filter_map(|entry| entry.path.as_ref().map(|path| (path.clone(), path.clone())))
            .collect();
        let atoms = bridge
            .nodes
            .iter()
            .filter(|entry| {
                !entry.incoming
                    && is_atom(entry.node)
                    && !matches!(entry.node.node_type.as_str(), "hardBreak" | "inlineMath")
            })
            .filter_map(|entry| entry.path.as_ref().map(|path| (entry.start, path.clone())))
            .collect();
        return Ok((
            document.clone(),
            FittedMap {
                before: before.clone(),
                after: before,
                steps: Vec::new(),
                paths,
                atoms,
                inserted_end: from_pos,
                inserted_right: insertion_search_right(content, open_end),
            },
        ));
    }
    let mut reserved = HashSet::new();
    for entry in &bridge.nodes {
        if let Some(id) = entry.node.block_id() {
            reserved.insert(id.to_owned());
        }
    }
    let mut rebuilt = Reconstruct {
        bridge: &bridge,
        allocate,
        used: HashSet::new(),
        reserved,
        seen: vec![0; bridge.nodes.len()],
        text: HashMap::new(),
        paths: Vec::new(),
        atoms: Vec::new(),
    };
    let mut root = rebuilt.node(&value, NodePath::root(), 0)?;
    for (index, entry) in bridge.nodes.iter().enumerate() {
        if entry.node.node_type == "text" {
            let text = entry.node.text.as_deref().unwrap_or_default();
            let expected = if entry.incoming {
                text.to_owned()
            } else {
                let mut kept = String::new();
                let mut units = entry.start;
                for ch in text.chars() {
                    let end = units + ch.len_utf16();
                    if end <= from_pos || units >= to_pos {
                        kept.push(ch);
                    }
                    units = end;
                }
                kept
            };
            if rebuilt
                .text
                .get(&index)
                .map(String::as_str)
                .unwrap_or_default()
                != expected
            {
                return Err(Error::UnsupportedEdit);
            }
        } else if entry.incoming && is_atom(entry.node) && rebuilt.seen[index] != 1 {
            return Err(Error::UnsupportedEdit);
        }
    }
    let required = root.required_schema_version()?;
    let current = root
        .attrs
        .as_ref()
        .and_then(|attrs| attrs.get("schemaVersion"))
        .and_then(Value::as_f64)
        .unwrap_or(1.0);
    if current < required as f64 {
        root.attrs
            .get_or_insert_default()
            .insert("schemaVersion".into(), json!(required));
    }
    let mut candidate = Document::from_root(root)?;
    let after = coordinates(candidate.root());
    // Source selectionToInsertionEnd uses the first changed range in the final
    // replacement step, rather than mapping the old selection start through it.
    let mut inserted_end = None;
    if let Some(step) = steps.last() {
        step.for_each(|_, _, _, end| {
            inserted_end.get_or_insert(end);
        });
    }
    let mut map = FittedMap {
        before,
        after,
        steps,
        paths: rebuilt.paths,
        atoms: rebuilt.atoms,
        inserted_end: inserted_end.unwrap_or(from_pos),
        inserted_right: insertion_search_right(content, open_end),
    };
    let known: Value = serde_json::from_str(include_str!("fitting_schema.json"))?;
    let mut incoming_refs = Vec::new();
    for (index, entry) in bridge.nodes.iter().enumerate() {
        if entry.node.node_type == "text" {
            continue;
        }
        let survived = rebuilt.seen[bridge.node_tokens[index]] > 0;
        let opaque = !entry.node.extra.is_empty()
            || entry.node.attrs.as_ref().is_some_and(|attrs| {
                attrs.keys().any(|key| {
                    !matches!(key.as_str(), "blockId" | "sourceRefs")
                        && known["nodes"][&entry.node.node_type]["attrs"]
                            .get(key)
                            .is_none()
                })
            })
            || entry
                .node
                .marks
                .as_ref()
                .is_some_and(|marks| !marks.is_empty());
        let deleted =
            !entry.incoming && from_pos <= entry.start && to_pos >= entry.start + size(entry.node);
        if !survived && opaque && !deleted {
            let position = if entry.incoming {
                from_pos
            } else {
                entry.start + 1
            };
            let mapped = map
                .steps
                .iter()
                .fold(position, |pos, step| step.map(pos, -1));
            let destination = map.resolve(mapped, false).ok_or(Error::UnsupportedEdit)?;
            merge_context(&mut candidate, &destination.block_id, entry.node, &known)?;
        }
        let refs = entry
            .node
            .attrs
            .as_ref()
            .and_then(|attrs| attrs.get("sourceRefs"))
            .and_then(Value::as_array);
        if let Some(refs) = refs {
            if entry.incoming && !survived {
                for reference in refs {
                    if !incoming_refs.contains(reference) {
                        incoming_refs.push(reference.clone());
                    }
                }
            } else if !entry.incoming && !deleted {
                let mut left = entry.start + 1;
                let mut right = entry.start + size(entry.node) - 1;
                for step in &map.steps {
                    left = step.map(left, -1);
                    right = step.map(right, 1);
                }
                for block in &map.after {
                    if block.start <= right && block.end() >= left {
                        merge_refs(&mut candidate.root, &block.id, refs)?;
                    }
                }
            }
        }
    }
    if !incoming_refs.is_empty()
        && let Some(point) = map.resolve(
            map.steps
                .iter()
                .fold(from_pos, |pos, step| step.map(pos, -1)),
            false,
        )
    {
        merge_refs(&mut candidate.root, &point.block_id, &incoming_refs)?;
    }
    candidate.validate()?;
    map.paths.retain(|(before,after)|matches!((document.node(before),candidate.node(after)),(Ok(left),Ok(right)) if left==right));
    Ok((candidate, map))
}

/// Source table paste fits open fragments into a row/cell independently of the destination.
/// This private planner returns canonical envelopes; no native serialization becomes authority.
pub(crate) fn fit_clipboard_container<F: FnMut() -> Result<String>>(
    kind: &str,
    content: &[Node],
    open_start: usize,
    open_end: usize,
    allocate: &mut F,
) -> Result<Node> {
    Node::decode_open_fragment(serde_json::to_value(content)?)?;
    if !matches!(kind, "tableRow" | "tableCell")
        || open_start > 64
        || open_end > 64
        || open_start > edge_depth(content, false)
        || open_end > edge_depth(content, true)
    {
        return Err(Error::InvalidPosition);
    }
    let mut bridge = Bridge::new()?;
    let incoming = content
        .iter()
        .map(|node| bridge.encode(node, None, 0, true))
        .collect::<Result<Vec<_>>>()?;
    let schema = bridge.schema(kind)?;
    let value = schema.with_types(|| -> Result<Value> {
        let root = schema
            .node_type(kind)
            .and_then(|node_type| node_type.create_and_fill(Value::Null, None, None))
            .ok_or(Error::UnsupportedEdit)?;
        let end = root.content().map_or(0, Fragment::size);
        let nodes = incoming
            .iter()
            .map(|node| {
                schema
                    .node_from_json(node)
                    .map_err(|_| Error::UnsupportedEdit)
            })
            .collect::<Result<Vec<_>>>()?;
        let slice = Slice::new(Fragment::from(nodes), open_start, open_end);
        let mut transform = Transform::<Dyn>::new(root);
        transform.replace(0, Some(end), Some(slice));
        Ok(serde_json::to_value(&transform.doc)?)
    })?;
    let mut reserved = HashSet::new();
    for entry in &bridge.nodes {
        if let Some(id) = entry.node.block_id() {
            reserved.insert(id.to_owned());
        }
    }
    let mut rebuilt = Reconstruct {
        bridge: &bridge,
        allocate,
        used: HashSet::new(),
        reserved,
        seen: vec![0; bridge.nodes.len()],
        text: HashMap::new(),
        paths: Vec::new(),
        atoms: Vec::new(),
    };
    let root = rebuilt.node(&value, NodePath::root(), 0)?;
    for (index, entry) in bridge.nodes.iter().enumerate() {
        if entry.node.node_type == "text" {
            if rebuilt
                .text
                .get(&index)
                .map(String::as_str)
                .unwrap_or_default()
                != entry.node.text.as_deref().unwrap_or_default()
            {
                return Err(Error::UnsupportedEdit);
            }
            continue;
        }
        let missing_atom = is_atom(entry.node) && rebuilt.seen[index] != 1;
        let missing_context = rebuilt.seen[bridge.node_tokens[index]] == 0
            && (!entry.node.extra.is_empty()
                || entry.node.attrs.as_ref().is_some_and(|attrs| {
                    attrs.keys().any(|key| {
                        !matches!(key.as_str(), "blockId" | "sourceRefs")
                            && bridge.spec["nodes"][&entry.node.node_type]["attrs"]
                                .get(key)
                                .is_none()
                    })
                }));
        if missing_atom || missing_context {
            return Err(Error::UnsupportedEdit);
        }
    }
    crate::structural::check_subtree_bounds(&root)?;
    Ok(root)
}

fn merge_refs(node: &mut Node, id: &str, refs: &[Value]) -> Result<()> {
    if refs.is_empty() {
        return Ok(());
    }
    if node.block_id() == Some(id) {
        let attrs = node.attrs.get_or_insert_default();
        let mut merged = attrs
            .get("sourceRefs")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for reference in refs {
            if !merged.contains(reference) {
                merged.push(reference.clone());
            }
        }
        attrs.insert("sourceRefs".into(), json!(merged));
        return Ok(());
    }
    if let Some(children) = &mut node.content {
        for child in children {
            merge_refs(child, id, refs)?;
        }
    }
    Ok(())
}

fn merge_context(document: &mut Document, id: &str, source: &Node, spec: &Value) -> Result<()> {
    fn path(node: &Node, id: &str, at: NodePath) -> Option<NodePath> {
        if node.block_id() == Some(id) {
            return Some(at);
        }
        node.children()
            .iter()
            .enumerate()
            .find_map(|(index, child)| path(child, id, at.child(index)))
    }
    let path = path(document.root(), id, NodePath::root()).ok_or(Error::BlockNotFound)?;
    let mut destination = None;
    for depth in (0..=path.0.len()).rev() {
        let candidate = NodePath(path.0[..depth].to_vec());
        if document.node(&candidate)?.node_type == source.node_type {
            destination = Some(candidate);
            break;
        }
    }
    let target = crate::structural::at_path_mut(
        &mut document.root,
        &destination.ok_or(Error::UnsupportedEdit)?,
    )
    .ok_or(Error::InvalidPosition)?;
    for (key, value) in &source.extra {
        if target
            .extra
            .get(key)
            .is_some_and(|existing| existing != value)
        {
            return Err(Error::UnsupportedEdit);
        }
        target.extra.insert(key.clone(), value.clone());
    }
    for (key, value) in source.attrs.as_ref().into_iter().flatten() {
        if matches!(key.as_str(), "blockId" | "sourceRefs")
            || spec["nodes"][&source.node_type]["attrs"].get(key).is_some()
        {
            continue;
        }
        let attrs = target.attrs.get_or_insert_default();
        if attrs.get(key).is_some_and(|existing| existing != value) {
            return Err(Error::UnsupportedEdit);
        }
        attrs.insert(key.clone(), value.clone());
    }
    if source.marks.as_ref().is_some_and(|marks| !marks.is_empty()) {
        if target
            .marks
            .as_ref()
            .is_some_and(|marks| !marks.is_empty() && target.marks != source.marks)
        {
            return Err(Error::UnsupportedEdit);
        }
        target.marks = source.marks.clone();
    }
    Ok(())
}
