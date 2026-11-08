//! Source-compatible rectangular clipboard normalization, without host project policy.
use crate::{Error, Node, Result, TableGrid};
use serde_json::{Map, json};

/// Canonical rectangular cells recognized by the original table-paste plugin.
#[derive(Debug, Clone, PartialEq)]
pub struct PastedCells {
    /// Logical columns, independent of a native renderer's capacity.
    pub width: usize,
    /// Physical rows, including rows covered completely by a rowspan.
    pub height: usize,
    /// Canonical row envelopes and cells; opaque data is not serialized through a renderer.
    pub rows: Vec<Node>,
    /// Consumed outer table/row context, retained for explicit destination metadata merging.
    pub context: Vec<Node>,
}
pub(crate) fn span(cell: &Node, name: &str) -> Result<usize> {
    match cell.attrs.as_ref().and_then(|attrs| attrs.get(name)) {
        None => Ok(1),
        Some(value) => crate::document::integer(value)
            .filter(|value| (1..=100).contains(value))
            .map(|value| value as usize)
            .ok_or_else(|| Error::InvalidDocument("table_grid_span_limit".into())),
    }
}
impl PastedCells {
    /// Fit ordinary rich writing into the source plugin's one-body-cell clipboard form.
    /// This is used only for an explicit cell rectangle, not for ordinary prose caret paste.
    pub fn from_writing<F: FnMut() -> Result<String>>(
        content: &[Node],
        open_start: usize,
        open_end: usize,
        mut allocate: F,
    ) -> Result<Self> {
        let cell = crate::fitting::fit_clipboard_container(
            "tableCell",
            content,
            open_start,
            open_end,
            &mut allocate,
        )?;
        Ok(Self {
            width: 1,
            height: 1,
            rows: vec![Node::block("tableRow", Map::new(), vec![cell])],
            context: vec![],
        })
    }
}
fn empty_cell<F: FnMut() -> Result<String>>(allocate: &mut F) -> Result<Node> {
    let mut paragraph = Node::paragraph(&allocate()?, "");
    paragraph
        .attrs
        .get_or_insert_default()
        .insert("sourceRefs".into(), json!([]));
    Ok(Node::block(
        "tableCell",
        Map::from_iter([
            ("colspan".into(), json!(1)),
            ("rowspan".into(), json!(1)),
            ("colwidth".into(), json!(null)),
            ("align".into(), json!(null)),
        ]),
        vec![paragraph],
    ))
}
/// Recognize source table/row/cell slices and fill ragged rows without flattening rich cells.
/// Returns None for ordinary prose, including open single-cell fragments the source unwraps
/// into prose. Fresh filler paragraph IDs are supplied explicitly; no OS resource is accessed.
pub fn table_cells_from_slice<F: FnMut() -> Result<String>>(
    content: &[Node],
    open_start: usize,
    open_end: usize,
    mut allocate: F,
) -> Result<Option<PastedCells>> {
    Node::decode_open_fragment(serde_json::to_value(content)?)?;
    if open_start > 64
        || open_end > 64
        || open_start > crate::fitting::edge_depth(content, false)
        || open_end > crate::fitting::edge_depth(content, true)
    {
        return Err(Error::InvalidPosition);
    }

    if content.is_empty() {
        return Ok(None);
    }
    let mut content = content;
    let mut open_start = isize::try_from(open_start).map_err(|_| Error::ResourceLimit)?;
    let mut open_end = isize::try_from(open_end).map_err(|_| Error::ResourceLimit)?;
    let mut context = Vec::new();
    while content.len() == 1
        && ((open_start > 0 && open_end > 0) || content[0].node_type == "table")
    {
        context.push(content[0].content_envelope());
        content = content[0].children();
        open_start -= 1;
        open_end -= 1;
    }
    let Some(first) = content.first() else {
        return Ok(None);
    };
    let mut rows = Vec::new();
    match first.node_type.as_str() {
        "tableRow" => {
            for (index, row) in content.iter().enumerate() {
                if row.node_type != "tableRow" {
                    return Err(Error::InvalidDocument("table_clipboard_row_type".into()));
                }
                let left = if index == 0 {
                    (open_start - 1).max(0) as usize
                } else {
                    0
                };
                let right = if index + 1 == content.len() {
                    (open_end - 1).max(0) as usize
                } else {
                    0
                };
                let mut rebuilt = row.clone();
                if left > 0 || right > 0 {
                    rebuilt.content = Some(
                        crate::fitting::fit_clipboard_container(
                            "tableRow",
                            row.children(),
                            left,
                            right,
                            &mut allocate,
                        )?
                        .children()
                        .to_vec(),
                    );
                }
                rows.push(rebuilt);
            }
        }
        "tableCell" | "tableHeader" => {
            if content
                .iter()
                .any(|node| !matches!(node.node_type.as_str(), "tableCell" | "tableHeader"))
            {
                return Err(Error::InvalidDocument("table_clipboard_cell_type".into()));
            }
            let row = if open_start > 0 || open_end > 0 {
                crate::fitting::fit_clipboard_container(
                    "tableRow",
                    content,
                    open_start.max(0) as usize,
                    open_end.max(0) as usize,
                    &mut allocate,
                )?
            } else {
                Node::block("tableRow", Map::new(), content.to_vec())
            };
            rows.push(row);
        }
        _ => return Ok(None),
    }
    let mut widths = vec![0usize; rows.len()];
    let mut node_budget = 100_000usize;
    let mut stack = rows.iter().collect::<Vec<_>>();
    while let Some(node) = stack.pop() {
        node_budget = node_budget.checked_sub(1).ok_or(Error::ResourceLimit)?;
        stack.extend(node.children());
    }
    for (index, row) in rows.iter().enumerate() {
        for cell in row.children() {
            let bottom = index
                .checked_add(span(cell, "rowspan")?)
                .ok_or(Error::ResourceLimit)?;
            if bottom > 100_000 {
                return Err(Error::ResourceLimit);
            }
            widths.resize(widths.len().max(bottom), 0);
            for width in &mut widths[index..bottom] {
                *width = width
                    .checked_add(span(cell, "colspan")?)
                    .ok_or(Error::ResourceLimit)?;
            }
        }
    }
    let width = widths.iter().copied().max().unwrap_or(0);
    if width == 0 {
        return Ok(None);
    }
    for (index, current) in widths.iter().copied().enumerate() {
        if index >= rows.len() {
            node_budget = node_budget.checked_sub(1).ok_or(Error::ResourceLimit)?;
            rows.push(Node::block("tableRow", Map::new(), vec![]));
        }
        for _ in current..width {
            node_budget = node_budget.checked_sub(2).ok_or(Error::ResourceLimit)?;
            rows[index]
                .content
                .get_or_insert_default()
                .push(empty_cell(&mut allocate)?);
        }
    }
    Node::decode_open_fragment(serde_json::to_value(&rows)?)?;
    let table = Node::block("table", Map::new(), rows.clone());
    let grid = TableGrid::new(&table)?;
    Ok(Some(PastedCells {
        width: grid.columns,
        height: grid.rows,
        rows,
        context,
    }))
}
