//! Sparse canonical table geometry and atomic editing builders.
//!
//! Semantics were inspected in the installed ProseMirror tables commands; this
//! implementation independently edits guarded JSON trees, not ProseMirror steps.
//! Cell spans are 1..=100; total grid width is not the Word/DOCX column limit.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{Command, Document, Error, Node, NodeRef, Result};

/// One originating cell's half-open rectangle in the logical table grid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellRect {
    /// Physical row containing the originating cell.
    pub row: usize,
    /// Child ordinal within that physical row.
    pub cell_index: usize,
    /// First logical grid column.
    pub column: usize,
    /// Number of covered columns, from 1 through 100.
    pub colspan: usize,
    /// Number of covered rows, from 1 through 100.
    pub rowspan: usize,
}
impl CellRect {
    /// Exclusive logical right edge; construction checks arithmetic.
    pub fn right(&self) -> usize {
        self.column.saturating_add(self.colspan)
    }
    /// Exclusive physical bottom edge.
    pub fn bottom(&self) -> usize {
        self.row.saturating_add(self.rowspan)
    }
}

/// A nonempty rectangular selection of logical grid cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellSelection {
    /// Inclusive top row.
    pub top: usize,
    /// Inclusive left column.
    pub left: usize,
    /// Exclusive bottom row.
    pub bottom: usize,
    /// Exclusive right column.
    pub right: usize,
}

/// Cell alignment, independent of the native view toolkit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CellAlignment {
    /// Align to the left.
    Left,
    /// Center content.
    Center,
    /// Align to the right.
    Right,
}

/// Validated sparse table geometry, storing one rectangle per originating cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableGrid {
    /// Number of physical rows, including wholly covered continuation rows.
    pub rows: usize,
    /// Logical column count; no dense rows-by-columns buffer is allocated.
    pub columns: usize,
    /// Originating cells in row/child order.
    pub cells: Vec<CellRect>,
    /// Consistent nonzero widths per logical column; zero/omission means automatic.
    pub column_widths: BTreeMap<usize, u64>,
}
impl TableGrid {
    /// Validate rectangular coverage, spans, bounds, widths and cell types.
    /// Unknown attributes are retained by the canonical tree, never normalized here.
    pub fn new(table: &Node) -> Result<Self> {
        if table.node_type != "table" || table.children().is_empty() {
            return grid_error("table_required");
        }
        let rows = table.children().len();
        if rows > 100_000 {
            return Err(Error::ResourceLimit);
        }
        let mut cells = Vec::new();
        let mut active: Vec<CellRect> = Vec::new();
        let mut columns = None;
        let mut column_widths = BTreeMap::new();
        for (row_index, row) in table.children().iter().enumerate() {
            if row.node_type != "tableRow" {
                return grid_error("row_type");
            }
            active.retain(|cell| cell.bottom() > row_index);
            let mut carried = active.clone();
            carried.sort_by_key(|cell| cell.column);
            let mut occupied = carried.clone();
            let mut active_cursor = 0;
            let mut cursor = 0usize;
            for (cell_index, cell) in row.children().iter().enumerate() {
                require_cell(cell)?;
                while let Some(cell) = carried.get(active_cursor) {
                    if cell.right() <= cursor {
                        active_cursor += 1;
                    } else if cell.column <= cursor {
                        cursor = cell.right();
                        active_cursor += 1;
                    } else {
                        break;
                    }
                }
                let colspan = span(cell, "colspan")?;
                let rowspan = span(cell, "rowspan")?;
                let right = cursor.checked_add(colspan).ok_or(Error::ResourceLimit)?;
                let bottom = row_index.checked_add(rowspan).ok_or(Error::ResourceLimit)?;
                if bottom > rows {
                    return grid_error("rowspan_bounds");
                }
                if carried
                    .get(active_cursor)
                    .is_some_and(|other| other.column < right)
                {
                    return grid_error("overlap");
                }
                let rect = CellRect {
                    row: row_index,
                    cell_index,
                    column: cursor,
                    colspan,
                    rowspan,
                };
                if let Some(widths) = widths(cell, colspan)? {
                    for (offset, width) in widths
                        .into_iter()
                        .enumerate()
                        .filter(|(_, width)| *width > 0)
                    {
                        match column_widths.insert(cursor + offset, width) {
                            Some(old) if old != width => return grid_error("inconsistent_width"),
                            _ => {}
                        }
                    }
                }
                occupied.push(rect.clone());
                if rowspan > 1 {
                    active.push(rect.clone());
                }
                cells.push(rect);
                if cells.len() > 100_000 {
                    return Err(Error::ResourceLimit);
                }
                cursor = right;
            }
            occupied.sort_by_key(|cell| cell.column);
            let mut end = 0;
            for cell in occupied {
                if cell.column != end {
                    return grid_error("gap_or_overlap");
                }
                end = cell.right();
            }
            if end == 0 {
                return grid_error("empty_grid_row");
            }
            if let Some(width) = columns {
                if width != end {
                    return grid_error("ragged_rows");
                }
            } else {
                columns = Some(end);
            }
        }
        Ok(Self {
            rows,
            columns: columns.ok_or_else(|| Error::InvalidDocument("table_grid_empty".into()))?,
            cells,
            column_widths,
        })
    }
    /// Resolve the originating cell covering a logical grid coordinate.
    pub fn cell_at(&self, row: usize, column: usize) -> Option<&CellRect> {
        self.cells.iter().find(|cell| {
            cell.row <= row && row < cell.bottom() && cell.column <= column && column < cell.right()
        })
    }
}

/// Create a unit cell with an identified empty paragraph; no ID is invented.
pub fn empty_table_cell(paragraph_id: &str, header: bool) -> Node {
    Node::block(
        if header { "tableHeader" } else { "tableCell" },
        Map::from_iter([("colspan".into(), json!(1)), ("rowspan".into(), json!(1))]),
        vec![Node::paragraph(paragraph_id, "")],
    )
}

/// Insert a physical row, extending cells spanning the insertion boundary.
/// The supplied row contains unit cells for every uncovered column; IDs/header
/// types are explicit host choices. A fully covered new row has no child cells.
pub fn insert_table_row(
    document: &Document,
    target: &NodeRef,
    index: usize,
    row: Node,
) -> Result<Command> {
    let (mut table, grid) = table_snapshot(document, target)?;
    if index > grid.rows || row.node_type != "tableRow" {
        return Err(Error::InvalidPosition);
    }
    for cell in row.children() {
        require_unit_cell(cell)?;
    }
    for rect in &grid.cells {
        if rect.row < index && index < rect.bottom() {
            set_span(origin_mut(&mut table, rect)?, "rowspan", rect.rowspan + 1)?;
        }
    }
    table.content.get_or_insert_default().insert(index, row);
    finish(document, target, table)
}

/// Delete one row, preserving spanning-cell content by moving its origin down.
/// Removing the last row is a separate whole-table deletion, not an empty table.
pub fn delete_table_row(document: &Document, target: &NodeRef, index: usize) -> Result<Command> {
    let (mut table, grid) = table_snapshot(document, target)?;
    if index >= grid.rows {
        return Err(Error::InvalidPosition);
    }
    if grid.rows == 1 {
        return Err(Error::UnsupportedEdit);
    }
    let mut origins = vec![Vec::new(); grid.rows - 1];
    for rect in &grid.cells {
        let mut cell = origin(&table, rect)?.clone();
        let row = if rect.row == index {
            if rect.rowspan == 1 {
                continue;
            }
            set_span(&mut cell, "rowspan", rect.rowspan - 1)?;
            index
        } else {
            if rect.row < index && index < rect.bottom() {
                set_span(&mut cell, "rowspan", rect.rowspan - 1)?;
            }
            rect.row - usize::from(rect.row > index)
        };
        origins
            .get_mut(row)
            .ok_or(Error::InvalidPosition)?
            .push((rect.column, cell));
    }
    table
        .content
        .as_mut()
        .ok_or(Error::InvalidPosition)?
        .remove(index);
    rebuild_rows(&mut table, origins)?;
    finish(document, target, table)
}

/// Insert one logical column, widening cells crossing its interior.
/// New unit cells are consumed in physical-row order only for uncovered rows.
pub fn insert_table_column(
    document: &Document,
    target: &NodeRef,
    index: usize,
    new_cells: Vec<Node>,
) -> Result<Command> {
    let (mut table, grid) = table_snapshot(document, target)?;
    if index > grid.columns {
        return Err(Error::InvalidPosition);
    }
    let mut origins = vec![Vec::new(); grid.rows];
    let mut crossed = vec![false; grid.rows];
    for rect in &grid.cells {
        let mut cell = origin(&table, rect)?.clone();
        if rect.column < index && index < rect.right() {
            set_span(&mut cell, "colspan", rect.colspan + 1)?;
            insert_width(&mut cell, index - rect.column)?;
            for row in crossed
                .get_mut(rect.row..rect.bottom())
                .ok_or(Error::InvalidPosition)?
            {
                *row = true;
            }
        }
        origins[rect.row].push((rect.column + usize::from(rect.column >= index), cell));
    }
    let mut additions = new_cells.into_iter();
    for (row, is_crossed) in crossed.into_iter().enumerate() {
        if !is_crossed {
            let cell = additions
                .next()
                .ok_or_else(|| Error::InvalidDocument("table_new_cells_missing".into()))?;
            require_unit_cell(&cell)?;
            origins[row].push((index, cell));
        }
    }
    if additions.next().is_some() {
        return grid_error("new_cells_extra");
    }
    rebuild_rows(&mut table, origins)?;
    finish(document, target, table)
}

/// Delete one logical column, shrinking spans and preserving remaining widths.
pub fn delete_table_column(document: &Document, target: &NodeRef, index: usize) -> Result<Command> {
    let (mut table, grid) = table_snapshot(document, target)?;
    if index >= grid.columns {
        return Err(Error::InvalidPosition);
    }
    if grid.columns == 1 {
        return Err(Error::UnsupportedEdit);
    }
    let mut origins = vec![Vec::new(); grid.rows];
    for rect in &grid.cells {
        let mut cell = origin(&table, rect)?.clone();
        if rect.column <= index && index < rect.right() {
            if rect.colspan == 1 {
                continue;
            }
            set_span(&mut cell, "colspan", rect.colspan - 1)?;
            remove_width(&mut cell, index - rect.column)?;
        }
        origins[rect.row].push((rect.column - usize::from(rect.column > index), cell));
    }
    rebuild_rows(&mut table, origins)?;
    finish(document, target, table)
}

/// Set a logical column width consistently across all cells spanning that column.
/// Zero restores automatic width. Widths are bounded to the existing 20000 limit.
pub fn set_table_column_width(
    document: &Document,
    target: &NodeRef,
    column: usize,
    width: u64,
) -> Result<Command> {
    let (mut table, grid) = table_snapshot(document, target)?;
    if column >= grid.columns || width > 20_000 {
        return Err(Error::InvalidPosition);
    }
    for rect in &grid.cells {
        if rect.column <= column && column < rect.right() {
            let cell = origin_mut(&mut table, rect)?;
            let mut values = widths(cell, rect.colspan)?.unwrap_or_else(|| vec![0; rect.colspan]);
            values[column - rect.column] = width;
            set_widths(cell, values);
        }
    }
    finish(document, target, table)
}

/// Set one originating cell's alignment, preserving every other attribute/field.
pub fn set_table_cell_alignment(
    document: &Document,
    target: &NodeRef,
    row: usize,
    cell_index: usize,
    alignment: Option<CellAlignment>,
) -> Result<Command> {
    let (mut table, grid) = table_snapshot(document, target)?;
    let rect = grid
        .cells
        .iter()
        .find(|cell| cell.row == row && cell.cell_index == cell_index)
        .ok_or(Error::InvalidPosition)?;
    origin_mut(&mut table, rect)?
        .attrs
        .get_or_insert_default()
        .insert(
            "align".into(),
            match alignment {
                Some(value) => serde_json::to_value(value)?,
                None => Value::Null,
            },
        );
    finish(document, target, table)
}

/// Merge a closed grid rectangle, retaining all block content/IDs in row order.
/// Differing opaque cell metadata or discarded cell IDs are explicitly refused.
/// Known layout attributes follow the top-left cell, as in the original editor.
pub fn merge_table_cells(
    document: &Document,
    target: &NodeRef,
    selection: CellSelection,
) -> Result<Command> {
    let (mut table, grid) = table_snapshot(document, target)?;
    if selection.top >= selection.bottom
        || selection.left >= selection.right
        || selection.bottom > grid.rows
        || selection.right > grid.columns
    {
        return Err(Error::InvalidPosition);
    }
    if selection.bottom - selection.top > 100 || selection.right - selection.left > 100 {
        return grid_error("span_limit");
    }
    let selected = grid
        .cells
        .iter()
        .filter(|cell| {
            cell.row < selection.bottom
                && selection.top < cell.bottom()
                && cell.column < selection.right
                && selection.left < cell.right()
        })
        .collect::<Vec<_>>();
    if selected.iter().any(|cell| {
        cell.row < selection.top
            || cell.bottom() > selection.bottom
            || cell.column < selection.left
            || cell.right() > selection.right
    }) {
        return grid_error("partial_cell_selection");
    }
    let first = grid
        .cell_at(selection.top, selection.left)
        .ok_or(Error::InvalidPosition)?;
    if selected.len() == 1 {
        return document.replace_node(target, table);
    }
    let mut merged = origin(&table, first)?.clone();
    let mut content = Vec::new();
    let mut references = Vec::new();
    for rect in &selected {
        let cell = origin(&table, rect)?;
        if !std::ptr::eq(*rect, first) && !merge_metadata_compatible(&merged, cell) {
            return Err(Error::UnsupportedEdit);
        }
        content.extend_from_slice(cell.children());
        if let Some(refs) = cell
            .attrs
            .as_ref()
            .and_then(|attrs| attrs.get("sourceRefs"))
            .and_then(Value::as_array)
        {
            for reference in refs {
                if !references.contains(reference) {
                    references.push(reference.clone());
                }
            }
        }
    }
    merged.content = Some(content);
    set_span(&mut merged, "colspan", selection.right - selection.left)?;
    set_span(&mut merged, "rowspan", selection.bottom - selection.top)?;
    if !references.is_empty() {
        merged
            .attrs
            .get_or_insert_default()
            .insert("sourceRefs".into(), Value::Array(references));
    }
    let values = (selection.left..selection.right)
        .map(|column| grid.column_widths.get(&column).copied().unwrap_or(0))
        .collect::<Vec<_>>();
    if values.iter().any(|width| *width > 0)
        || merged
            .attrs
            .as_ref()
            .is_some_and(|attrs| attrs.contains_key("colwidth"))
    {
        set_widths(&mut merged, values);
    }
    let selected_origins = selected
        .iter()
        .map(|cell| (cell.row, cell.cell_index))
        .collect::<BTreeSet<_>>();
    let mut origins = vec![Vec::new(); grid.rows];
    for rect in &grid.cells {
        if rect.row == first.row && rect.cell_index == first.cell_index {
            origins[rect.row].push((rect.column, merged.clone()));
        } else if !selected_origins.contains(&(rect.row, rect.cell_index)) {
            origins[rect.row].push((rect.column, origin(&table, rect)?.clone()));
        }
    }
    rebuild_rows(&mut table, origins)?;
    finish(document, target, table)
}

/// Split a spanning cell into unit cells. Original content/identity remain first.
/// Templates provide new content and optional fresh cell IDs, while source cell
/// metadata/formatting is inherited; conflicting opaque template values refuse.
pub fn split_table_cell(
    document: &Document,
    target: &NodeRef,
    row: usize,
    cell_index: usize,
    new_cells: Vec<Node>,
) -> Result<Command> {
    let (mut table, grid) = table_snapshot(document, target)?;
    let rect = grid
        .cells
        .iter()
        .find(|cell| cell.row == row && cell.cell_index == cell_index)
        .ok_or(Error::InvalidPosition)?;
    let count = rect
        .rowspan
        .checked_mul(rect.colspan)
        .ok_or(Error::ResourceLimit)?;
    if new_cells.len() != count - 1 {
        return grid_error("split_cell_count");
    }
    if count == 1 {
        return document.replace_node(target, table);
    }
    let original = origin(&table, rect)?.clone();
    let original_widths = widths(&original, rect.colspan)?;
    let mut origins = vec![Vec::new(); grid.rows];
    for cell in &grid.cells {
        if cell.row != row || cell.cell_index != cell_index {
            origins[cell.row].push((cell.column, origin(&table, cell)?.clone()));
        }
    }
    let mut additions = new_cells.into_iter();
    for physical_row in rect.row..rect.bottom() {
        for offset in 0..rect.colspan {
            let mut part = if physical_row == rect.row && offset == 0 {
                original.clone()
            } else {
                let template = additions.next().ok_or(Error::InvalidPosition)?;
                require_unit_cell(&template)?;
                split_part(&original, template)?
            };
            set_span(&mut part, "colspan", 1)?;
            set_span(&mut part, "rowspan", 1)?;
            if let Some(values) = &original_widths {
                set_widths(&mut part, vec![values[offset]]);
            }
            origins[physical_row].push((rect.column + offset, part));
        }
    }
    rebuild_rows(&mut table, origins)?;
    finish(document, target, table)
}

fn grid_error<T>(code: &str) -> Result<T> {
    Err(Error::InvalidDocument(format!("table_grid_{code}")))
}
fn require_cell(cell: &Node) -> Result<()> {
    if matches!(cell.node_type.as_str(), "tableCell" | "tableHeader") {
        Ok(())
    } else {
        grid_error("cell_type")
    }
}
fn require_unit_cell(cell: &Node) -> Result<()> {
    require_cell(cell)?;
    if span(cell, "colspan")? == 1 && span(cell, "rowspan")? == 1 {
        Ok(())
    } else {
        grid_error("new_cell_must_be_unit")
    }
}
fn span(cell: &Node, name: &str) -> Result<usize> {
    match cell.attrs.as_ref().and_then(|attrs| attrs.get(name)) {
        None => Ok(1),
        Some(value) => crate::document::integer(value)
            .filter(|value| (1..=100).contains(value))
            .map(|value| value as usize)
            .ok_or_else(|| Error::InvalidDocument("table_grid_span_limit".into())),
    }
}
fn set_span(cell: &mut Node, name: &str, value: usize) -> Result<()> {
    if !(1..=100).contains(&value) {
        return grid_error("span_limit");
    }
    cell.attrs
        .get_or_insert_default()
        .insert(name.into(), json!(value));
    Ok(())
}
fn widths(cell: &Node, colspan: usize) -> Result<Option<Vec<u64>>> {
    match cell.attrs.as_ref().and_then(|attrs| attrs.get("colwidth")) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(values)) if values.len() == colspan => values
            .iter()
            .map(|value| {
                crate::document::integer(value)
                    .filter(|value| *value <= 20_000)
                    .ok_or_else(|| Error::InvalidDocument("table_grid_column_width".into()))
            })
            .collect::<Result<Vec<_>>>()
            .map(Some),
        _ => grid_error("width_count"),
    }
}
fn set_widths(cell: &mut Node, values: Vec<u64>) {
    cell.attrs.get_or_insert_default().insert(
        "colwidth".into(),
        if values.iter().any(|width| *width > 0) {
            json!(values)
        } else {
            Value::Null
        },
    );
}
fn insert_width(cell: &mut Node, offset: usize) -> Result<()> {
    let old_span = span(cell, "colspan")?
        .checked_sub(1)
        .ok_or(Error::InvalidPosition)?;
    if let Some(mut values) = widths(cell, old_span)? {
        if offset > values.len() {
            return Err(Error::InvalidPosition);
        }
        values.insert(offset, 0);
        set_widths(cell, values);
    }
    Ok(())
}
fn remove_width(cell: &mut Node, offset: usize) -> Result<()> {
    let old_span = span(cell, "colspan")?
        .checked_add(1)
        .ok_or(Error::InvalidPosition)?;
    if let Some(mut values) = widths(cell, old_span)? {
        if offset >= values.len() {
            return Err(Error::InvalidPosition);
        }
        values.remove(offset);
        set_widths(cell, values);
    }
    Ok(())
}
fn origin<'a>(table: &'a Node, rect: &CellRect) -> Result<&'a Node> {
    table
        .children()
        .get(rect.row)
        .and_then(|row| row.children().get(rect.cell_index))
        .ok_or(Error::InvalidPosition)
}
fn origin_mut<'a>(table: &'a mut Node, rect: &CellRect) -> Result<&'a mut Node> {
    table
        .content
        .as_mut()
        .and_then(|rows| rows.get_mut(rect.row))
        .and_then(|row| row.content.as_mut())
        .and_then(|cells| cells.get_mut(rect.cell_index))
        .ok_or(Error::InvalidPosition)
}
fn table_snapshot(document: &Document, target: &NodeRef) -> Result<(Node, TableGrid)> {
    crate::structural::check_ref(document, target)?;
    let table = target.expected.as_ref().clone();
    let grid = TableGrid::new(&table)?;
    Ok((table, grid))
}
fn finish(document: &Document, target: &NodeRef, table: Node) -> Result<Command> {
    TableGrid::new(&table)?;
    document.replace_node(target, table)
}
fn rebuild_rows(table: &mut Node, origins: Vec<Vec<(usize, Node)>>) -> Result<()> {
    let rows = table.content.as_mut().ok_or(Error::InvalidPosition)?;
    if rows.len() != origins.len() {
        return Err(Error::InvalidPosition);
    }
    for (row, mut children) in rows.iter_mut().zip(origins) {
        children.sort_by_key(|(column, _)| *column);
        let content = children
            .into_iter()
            .map(|(_, cell)| cell)
            .collect::<Vec<_>>();
        if row.children() != content.as_slice() {
            row.content = Some(content);
        }
    }
    Ok(())
}
fn merge_metadata_compatible(first: &Node, other: &Node) -> bool {
    other.block_id().is_none()
        && first.compatible_envelope(
            other,
            &[
                "colspan",
                "rowspan",
                "colwidth",
                "align",
                "blockId",
                "sourceRefs",
            ],
        )
}
fn split_part(original: &Node, template: Node) -> Result<Node> {
    if original.node_type != template.node_type {
        return grid_error("split_cell_type");
    }
    let mut result = original.clone();
    result.merge_template_leaf_fields(&template)?;
    result.content = template.content;
    if let Some(id) = original.block_id() {
        let new_id = template
            .attrs
            .as_ref()
            .and_then(|attrs| attrs.get("blockId"))
            .and_then(Value::as_str)
            .filter(|new| *new != id)
            .ok_or(Error::UnsupportedEdit)?;
        result
            .attrs
            .get_or_insert_default()
            .insert("blockId".into(), json!(new_id));
    } else if let Some(id) = template
        .attrs
        .as_ref()
        .and_then(|attrs| attrs.get("blockId"))
    {
        result
            .attrs
            .get_or_insert_default()
            .insert("blockId".into(), id.clone());
    }
    for (key, value) in template.extra {
        if result.extra.get(&key).is_some_and(|old| old != &value) {
            return Err(Error::UnsupportedEdit);
        }
        result.extra.insert(key, value);
    }
    if let Some(attrs) = template.attrs {
        for (key, value) in attrs {
            if ["colspan", "rowspan", "colwidth", "align", "blockId"].contains(&key.as_str()) {
                continue;
            }
            let destination = result.attrs.get_or_insert_default();
            if destination.get(&key).is_some_and(|old| old != &value) {
                return Err(Error::UnsupportedEdit);
            }
            destination.insert(key, value);
        }
    }
    Ok(result)
}
