//! Guarded table-scoped replacement after source-compatible cell normalization.
use crate::table_paste::span;
use crate::{
    CellSelection, Command, Document, Error, Node, NodeRef, PastedCells, Result, TableGrid,
};
use serde_json::{Map, Value, json};
use std::collections::HashSet;

/// Explicit repetition and expansion policy supplied by the embedding clipboard boundary.
#[derive(Debug, Clone, Copy)]
pub struct TablePasteOptions {
    /// Tile/clip source cells to the selected rectangle. False pastes at its top-left and grows.
    pub repeat: bool,
    /// Bound serialized expanded incoming/split cell bytes before retaining their clones.
    pub max_expanded_bytes: usize,
}
struct PlacedCell {
    row: usize,
    column: usize,
    node: Node,
}
struct Fresh<'a, F: FnMut() -> Result<String>> {
    allocate: &'a mut F,
    reserved: HashSet<String>,
    remaining: usize,
}
impl<F: FnMut() -> Result<String>> Fresh<'_, F> {
    fn id(&mut self) -> Result<String> {
        let id = (self.allocate)()?;
        if id.is_empty() || id.len() > 160 || !self.reserved.insert(id.clone()) {
            return Err(Error::InvalidDocument("table_paste_fresh_identity".into()));
        }
        Ok(id)
    }
    fn cloned(&mut self, node: &Node, renew: bool) -> Result<Node> {
        let size = wire_size(node, self.remaining)?;
        self.remaining = self
            .remaining
            .checked_sub(size)
            .ok_or(Error::ResourceLimit)?;
        let mut node = node.clone();
        if renew {
            self.renew(&mut node)?;
        }
        let actual = wire_size(&node, self.remaining.saturating_add(size))?;
        self.remaining = self
            .remaining
            .checked_add(size)
            .and_then(|budget| budget.checked_sub(actual))
            .ok_or(Error::ResourceLimit)?;
        Ok(node)
    }
    fn renew(&mut self, node: &mut Node) -> Result<()> {
        if node.block_id().is_some()
            || matches!(
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
            )
        {
            node.attrs
                .get_or_insert_default()
                .insert("blockId".into(), json!(self.id()?));
        }
        if let Some(children) = node.content.as_mut() {
            for child in children {
                self.renew(child)?;
            }
        }
        Ok(())
    }
    fn empty(&mut self, header: bool) -> Result<Node> {
        let paragraph = Node::paragraph(self.id()?, "");
        let node = Node::block(
            if header { "tableHeader" } else { "tableCell" },
            Map::from_iter([
                ("colspan".into(), json!(1)),
                ("rowspan".into(), json!(1)),
                ("colwidth".into(), Value::Null),
                ("align".into(), Value::Null),
            ]),
            vec![paragraph],
        );
        self.cloned(&node, false)
    }
    fn split_empty(&mut self, template: &Node) -> Result<Node> {
        let mut node = template.content_envelope();
        if node.block_id().is_some() {
            node.attrs
                .get_or_insert_default()
                .insert("blockId".into(), json!(self.id()?));
        }
        node.content = Some(vec![Node::paragraph(self.id()?, "")]);
        self.cloned(&node, false)
    }
}
struct Counter {
    bytes: usize,
    limit: usize,
    exceeded: bool,
}
impl std::io::Write for Counter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let Some(next) = self
            .bytes
            .checked_add(buf.len())
            .filter(|size| *size <= self.limit)
        else {
            self.exceeded = true;
            return Err(std::io::Error::other("table_paste_budget"));
        };
        self.bytes = next;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn wire_size<T: serde::Serialize>(node: &T, limit: usize) -> Result<usize> {
    let mut counter = Counter {
        bytes: 0,
        limit,
        exceeded: false,
    };
    let result = serde_json::to_writer(&mut counter, node);
    if counter.exceeded {
        return Err(Error::ResourceLimit);
    }
    result?;
    Ok(counter.bytes)
}
fn ids(node: &Node, out: &mut HashSet<String>) {
    if let Some(id) = node.block_id() {
        out.insert(id.into());
    }
    for child in node.children() {
        ids(child, out);
    }
}
fn set_span(node: &mut Node, key: &str, value: usize) -> Result<()> {
    if !(1..=100).contains(&value) {
        return Err(Error::InvalidDocument("table_grid_span_limit".into()));
    }
    node.attrs
        .get_or_insert_default()
        .insert(key.into(), json!(value));
    Ok(())
}
fn columns(node: &mut Node, from: usize, count: usize) -> Result<()> {
    let before = span(node, "colspan")?;
    if from.checked_add(count).is_none_or(|end| end > before) {
        return Err(Error::InvalidPosition);
    }
    if let Some(Value::Array(widths)) = node.attrs.as_ref().and_then(|attrs| attrs.get("colwidth"))
    {
        let widths = widths
            .get(from..from + count)
            .ok_or(Error::InvalidPosition)?
            .to_vec();
        node.attrs
            .get_or_insert_default()
            .insert("colwidth".into(), json!(widths));
    }
    set_span(node, "colspan", count)
}
fn isolate_horizontal<F: FnMut() -> Result<String>>(
    cells: &mut Vec<PlacedCell>,
    edge: usize,
    left: usize,
    right: usize,
    fresh: &mut Fresh<'_, F>,
) -> Result<()> {
    let length = cells.len();
    for index in 0..length {
        let cell = &cells[index];
        let bottom = cell.row + span(&cell.node, "rowspan")?;
        let cell_right = cell.column + span(&cell.node, "colspan")?;
        if cell.row < edge && edge < bottom && cell.column < right && left < cell_right {
            let mut lower = fresh.split_empty(&cell.node)?;
            let column = cell.column;
            let top = cell.row;
            set_span(&mut lower, "rowspan", bottom - edge)?;
            set_span(&mut cells[index].node, "rowspan", edge - top)?;
            cells.push(PlacedCell {
                row: edge,
                column,
                node: lower,
            });
        }
    }
    Ok(())
}
fn isolate_vertical<F: FnMut() -> Result<String>>(
    cells: &mut Vec<PlacedCell>,
    edge: usize,
    top: usize,
    bottom: usize,
    fresh: &mut Fresh<'_, F>,
) -> Result<()> {
    let length = cells.len();
    for index in 0..length {
        let cell = &cells[index];
        let right = cell.column + span(&cell.node, "colspan")?;
        let cell_bottom = cell.row + span(&cell.node, "rowspan")?;
        if cell.column < edge && edge < right && cell.row < bottom && top < cell_bottom {
            let mut other = fresh.split_empty(&cell.node)?;
            let row = cell.row;
            let left = cell.column;
            columns(&mut other, edge - left, right - edge)?;
            columns(&mut cells[index].node, 0, edge - left)?;
            cells.push(PlacedCell {
                row,
                column: edge,
                node: other,
            });
        }
    }
    Ok(())
}
fn merge_context<F: FnMut() -> Result<String>>(
    target: &mut Node,
    source: &Node,
    fresh: &mut Fresh<'_, F>,
) -> Result<()> {
    if target.node_type != source.node_type {
        return Err(Error::UnsupportedEdit);
    }
    let before = wire_size(&target.content_envelope(), usize::MAX)?;
    let mut wire = serde_json::to_value(target.content_envelope())?;
    let incoming = serde_json::to_value(source.content_envelope())?;
    let object = wire.as_object_mut().ok_or(Error::UnsupportedEdit)?;
    for (key, value) in incoming.as_object().ok_or(Error::UnsupportedEdit)? {
        if matches!(key.as_str(), "type" | "content") {
            continue;
        }
        if key == "attrs" {
            if value.is_null() {
                if object.get(key).is_some_and(|old| !old.is_null()) {
                    return Err(Error::UnsupportedEdit);
                }
                object.insert(key.clone(), Value::Null);
                continue;
            }
            let attrs = object
                .entry(key.clone())
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .ok_or(Error::UnsupportedEdit)?;
            for (name, value) in value.as_object().ok_or(Error::UnsupportedEdit)? {
                if name == "blockId" {
                    continue;
                }
                if name == "sourceRefs" {
                    let references = attrs
                        .entry(name.clone())
                        .or_insert_with(|| json!([]))
                        .as_array_mut()
                        .ok_or(Error::UnsupportedEdit)?;
                    for item in value.as_array().ok_or(Error::UnsupportedEdit)? {
                        if !references.contains(item) {
                            references.push(item.clone());
                        }
                    }
                } else {
                    if attrs.get(name).is_some_and(|old| old != value) {
                        return Err(Error::UnsupportedEdit);
                    }
                    attrs.insert(name.clone(), value.clone());
                }
            }
        } else {
            if object.get(key).is_some_and(|old| old != value) {
                return Err(Error::UnsupportedEdit);
            }
            object.insert(key.clone(), value.clone());
        }
    }
    let mut merged: Node = serde_json::from_value(wire)?;
    if target.block_id().is_none()
        && source
            .attrs
            .as_ref()
            .is_some_and(|attrs| attrs.contains_key("blockId"))
    {
        merged
            .attrs
            .get_or_insert_default()
            .insert("blockId".into(), json!(fresh.id()?));
    }
    let after = wire_size(&merged, fresh.remaining.saturating_add(before))?;
    fresh.remaining = fresh
        .remaining
        .checked_sub(after.saturating_sub(before))
        .ok_or(Error::ResourceLimit)?;
    merged.content = target.content.take();
    *target = merged;
    Ok(())
}
fn add_refs(node: &mut Node, refs: &[Value], budget: &mut usize) -> Result<()> {
    if refs.is_empty() {
        return Ok(());
    }
    if matches!(
        node.node_type.as_str(),
        "paragraph" | "heading" | "codeBlock"
    ) {
        let present = node
            .attrs
            .as_ref()
            .is_some_and(|attrs| attrs.contains_key("sourceRefs"));
        if !present {
            *budget = budget.checked_sub(32).ok_or(Error::ResourceLimit)?;
        }
        let value = node
            .attrs
            .get_or_insert_default()
            .entry("sourceRefs")
            .or_insert_with(|| json!([]));
        let destination = value.as_array_mut().ok_or(Error::UnsupportedEdit)?;
        for reference in refs {
            if !destination.contains(reference) {
                let bytes = wire_size(reference, *budget)?
                    .checked_add(1)
                    .ok_or(Error::ResourceLimit)?;
                *budget = budget.checked_sub(bytes).ok_or(Error::ResourceLimit)?;
                destination.push(reference.clone());
            }
        }
    }
    if let Some(children) = node.content.as_mut() {
        for child in children {
            add_refs(child, refs, budget)?;
        }
    }
    Ok(())
}
fn context_refs(source: &PastedCells) -> Result<Vec<Value>> {
    let mut refs = Vec::new();
    for context in &source.context {
        if !matches!(context.node_type.as_str(), "table" | "tableRow") {
            let wire = serde_json::to_value(context)?;
            if !context.extra.is_empty()
                || wire.get("marks").is_some()
                || wire.get("text").is_some()
                || context.attrs.as_ref().is_some_and(|attrs| {
                    attrs
                        .keys()
                        .any(|key| !matches!(key.as_str(), "blockId" | "sourceRefs"))
                })
            {
                return Err(Error::UnsupportedEdit);
            }
        }
        if let Some(references) = context
            .attrs
            .as_ref()
            .and_then(|attrs| attrs.get("sourceRefs"))
            .and_then(Value::as_array)
        {
            for reference in references {
                if !refs.contains(reference) {
                    refs.push(reference.clone());
                }
            }
        }
    }
    Ok(refs)
}
/// Paste already normalized cells into an exact table as one candidate-validated transaction.
/// Unaffected cell text keeps its IDs and scalar positions; history owns full rich Undo/Redo.
/// Renderer column limits are a separate host policy and never lower the generic parser.
pub fn paste_table_cells<F: FnMut() -> Result<String>>(
    document: &Document,
    target: &NodeRef,
    selection: CellSelection,
    source: &PastedCells,
    options: TablePasteOptions,
    mut allocate: F,
) -> Result<(Command, CellSelection)> {
    crate::structural::check_ref(document, target)?;
    let table = target.expected.as_ref();
    let old_grid = TableGrid::new(table)?;
    if selection.top >= selection.bottom
        || selection.left >= selection.right
        || selection.bottom > old_grid.rows
        || selection.right > old_grid.columns
    {
        return Err(Error::InvalidPosition);
    }
    Node::decode_open_fragment(serde_json::to_value(&source.rows)?)?;
    Node::decode_open_fragment(serde_json::to_value(&source.context)?)?;
    let references = context_refs(source)?;
    let source_table = Node::block("table", Map::new(), source.rows.clone());
    let grid = TableGrid::new(&source_table)?;
    if source.width != grid.columns || source.height != grid.rows {
        return Err(Error::InvalidPosition);
    }
    let width = if options.repeat {
        selection.right - selection.left
    } else {
        grid.columns
    };
    let height = if options.repeat {
        selection.bottom - selection.top
    } else {
        grid.rows
    };
    let right = selection
        .left
        .checked_add(width)
        .ok_or(Error::ResourceLimit)?;
    let bottom = selection
        .top
        .checked_add(height)
        .ok_or(Error::ResourceLimit)?;
    if bottom.max(old_grid.rows) > 100_000 {
        return Err(Error::ResourceLimit);
    }
    let mut reserved = HashSet::new();
    ids(document.root(), &mut reserved);
    ids(&source_table, &mut reserved);
    for context in &source.context {
        ids(context, &mut reserved);
    }
    let mut fresh = Fresh {
        allocate: &mut allocate,
        reserved,
        remaining: options.max_expanded_bytes,
    };
    let mut cells = Vec::new();
    for cell in &old_grid.cells {
        cells.push(PlacedCell {
            row: cell.row,
            column: cell.column,
            node: table.children()[cell.row].children()[cell.cell_index].clone(),
        });
    }
    let total_columns = right.max(old_grid.columns);
    let rows = bottom.max(old_grid.rows);
    // Grow only unit cells outside the pasted rectangle. Source cells already cover
    // its inside; materializing and immediately deleting a dense temporary grid would
    // make a valid sparse/span paste consume unbounded intermediate work.
    let added_columns = total_columns - old_grid.columns;
    let added_rows = rows - old_grid.rows;
    let overlap_rows = bottom.min(old_grid.rows) - selection.top;
    let fillers = (old_grid.rows - overlap_rows)
        .checked_mul(added_columns)
        .and_then(|outside| {
            added_rows
                .checked_mul(selection.left + total_columns - right)
                .and_then(|extra| outside.checked_add(extra))
        })
        .ok_or(Error::ResourceLimit)?;
    if fillers > 50_000 {
        return Err(Error::ResourceLimit);
    }
    for row in 0..old_grid.rows {
        let header = table.children()[row]
            .children()
            .last()
            .is_some_and(|node| node.node_type == "tableHeader");
        let start = if selection.top <= row && row < bottom {
            old_grid.columns.max(right)
        } else {
            old_grid.columns
        };
        for column in start..total_columns {
            cells.push(PlacedCell {
                row,
                column,
                node: fresh.empty(header)?,
            });
        }
    }
    for row in old_grid.rows..rows {
        for column in (0..selection.left).chain(right..total_columns) {
            let header = old_grid
                .cell_at(old_grid.rows - 1, column)
                .is_some_and(|rect| {
                    table.children()[rect.row].children()[rect.cell_index].node_type
                        == "tableHeader"
                });
            cells.push(PlacedCell {
                row,
                column,
                node: fresh.empty(header)?,
            });
        }
    }
    isolate_horizontal(&mut cells, selection.top, selection.left, right, &mut fresh)?;
    isolate_horizontal(&mut cells, bottom, selection.left, right, &mut fresh)?;
    isolate_vertical(
        &mut cells,
        selection.left,
        selection.top,
        bottom,
        &mut fresh,
    )?;
    isolate_vertical(&mut cells, right, selection.top, bottom, &mut fresh)?;
    cells.retain(|cell| {
        !(selection.top <= cell.row
            && cell.row < bottom
            && selection.left <= cell.column
            && cell.column < right)
    });
    for tile_row in (0..height).step_by(grid.rows) {
        for tile_column in (0..width).step_by(grid.columns) {
            for rect in &grid.cells {
                let row = tile_row + rect.row;
                let column = tile_column + rect.column;
                if row >= height || column >= width {
                    continue;
                }
                let mut node =
                    fresh.cloned(&source.rows[rect.row].children()[rect.cell_index], true)?;
                let colspan = rect.colspan.min(width - column);
                let rowspan = rect.rowspan.min(height - row);
                add_refs(&mut node, &references, &mut fresh.remaining)?;
                if colspan != rect.colspan {
                    columns(&mut node, 0, colspan)?;
                }
                if rowspan != rect.rowspan {
                    set_span(&mut node, "rowspan", rowspan)?;
                }
                cells.push(PlacedCell {
                    row: selection.top + row,
                    column: selection.left + column,
                    node,
                });
                if cells.len() > 100_000 {
                    return Err(Error::ResourceLimit);
                }
            }
        }
    }
    cells.sort_by_key(|cell| (cell.row, cell.column));
    let mut replacement = table.clone();
    for context in &source.context {
        if context.node_type == "table" {
            merge_context(&mut replacement, context, &mut fresh)?;
        }
    }
    let mut output = Vec::with_capacity(rows);
    for row in 0..rows {
        let mut envelope = table
            .children()
            .get(row)
            .map(Node::content_envelope)
            .unwrap_or_else(|| Node::block("tableRow", Map::new(), vec![]));
        if selection.top <= row && row < bottom {
            merge_context(
                &mut envelope,
                &source.rows[(row - selection.top) % grid.rows],
                &mut fresh,
            )?;
            for context in &source.context {
                if context.node_type == "tableRow" {
                    merge_context(&mut envelope, context, &mut fresh)?;
                }
            }
        }
        envelope.content = Some(Vec::new());
        output.push(envelope);
    }
    for cell in cells {
        output[cell.row]
            .content
            .get_or_insert_default()
            .push(cell.node);
    }
    replacement.content = Some(output);
    TableGrid::new(&replacement)?;
    let command = document.replace_node(target, replacement)?;
    Ok((
        command,
        CellSelection {
            top: selection.top,
            left: selection.left,
            bottom,
            right,
        },
    ))
}

/// Copy a logical rectangle without including writing from a cell whose origin lies outside it.
/// The returned table envelope preserves context metadata; the clipboard layer owns open depths.
/// Clipped empty cells receive explicit fresh IDs while the source document stays immutable.
pub fn copy_table_fragment<F: FnMut() -> Result<String>>(
    document: &Document,
    target: &NodeRef,
    selection: CellSelection,
    max_expanded_bytes: usize,
    mut allocate: F,
) -> Result<Node> {
    crate::structural::check_ref(document, target)?;
    let table = target.expected.as_ref();
    let grid = TableGrid::new(table)?;
    if selection.top >= selection.bottom
        || selection.left >= selection.right
        || selection.bottom > grid.rows
        || selection.right > grid.columns
    {
        return Err(Error::InvalidPosition);
    }
    let mut reserved = HashSet::new();
    ids(document.root(), &mut reserved);
    let mut fresh = Fresh {
        allocate: &mut allocate,
        reserved,
        remaining: max_expanded_bytes,
    };
    let mut copied = fresh.cloned(&table.content_envelope(), false)?;
    let mut rows = Vec::with_capacity(selection.bottom - selection.top);
    for row in selection.top..selection.bottom {
        let mut envelope = fresh.cloned(&table.children()[row].content_envelope(), false)?;
        let mut cells = grid
            .cells
            .iter()
            .filter(|cell| {
                let origin = cell.row.max(selection.top);
                origin == row
                    && cell.row < selection.bottom
                    && selection.top < cell.bottom()
                    && cell.column < selection.right
                    && selection.left < cell.right()
            })
            .collect::<Vec<_>>();
        cells.sort_by_key(|cell| cell.column);
        let mut content = Vec::with_capacity(cells.len());
        for rect in cells {
            let original = &table.children()[rect.row].children()[rect.cell_index];
            let mut node = if rect.row < selection.top || rect.column < selection.left {
                fresh.split_empty(original)?
            } else {
                fresh.cloned(original, false)?
            };
            let left = rect.column.max(selection.left);
            let right = rect.right().min(selection.right);
            if left != rect.column || right != rect.right() {
                columns(&mut node, left - rect.column, right - left)?;
            }
            let top = rect.row.max(selection.top);
            let bottom = rect.bottom().min(selection.bottom);
            if top != rect.row || bottom != rect.bottom() {
                set_span(&mut node, "rowspan", bottom - top)?;
            }
            content.push(node);
        }
        envelope.content = Some(content);
        rows.push(envelope);
    }
    copied.content = Some(rows);
    TableGrid::new(&copied)?;
    crate::structural::check_subtree_bounds(&copied)?;
    Ok(copied)
}
