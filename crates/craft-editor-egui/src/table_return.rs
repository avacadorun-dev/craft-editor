//! Only private, causally owned table-mode returns may cross a proven transaction/history step.
use crate::{EditorSession, Result, SelectionBookmark, SelectionState};
use craft_editor_model::{Document, Node, NodePath, NodeRef, Revision, TableGrid};

impl EditorSession {
    pub(crate) fn current_table_return(&self) -> Option<SelectionBookmark> {
        self.table_node_return
            .as_ref()
            .filter(|bookmark| {
                bookmark.revision == self.revision()
                    && std::sync::Arc::ptr_eq(
                        &bookmark.session,
                        &self.clipboard_state.session_identity(),
                    )
            })
            .cloned()
    }
    pub(crate) fn retained_table_return(
        &self,
        after: &Document,
        revision: Revision,
        selected: &NodeRef,
    ) -> Result<Option<SelectionBookmark>> {
        let Some(before) = self.selected_table_node()? else {
            return Ok(None);
        };
        let Some(mut bookmark) = self.current_table_return() else {
            return Ok(None);
        };
        if selected.expected.node_type != "table" {
            return Ok(None);
        }
        let retained = match &mut bookmark.selection {
            Some(SelectionState::Text(text)) => {
                [&text.anchor, &text.head].into_iter().all(|point| {
                    self.document()
                        .block_text(&point.block_id)
                        .is_ok_and(|old| {
                            after
                                .block_text(&point.block_id)
                                .is_ok_and(|new| old == new)
                        })
                        && valid_point(after, point)
                })
            }
            Some(SelectionState::Cells(cells)) => {
                if cells.table_path != before.path
                    || !same_cell_occurrences(before.expected.as_ref(), selected.expected.as_ref())?
                {
                    false
                } else {
                    cells.table_path = selected.path.clone();
                    cells
                        .clone()
                        .canonicalized(after)
                        .is_ok_and(|target| target == *cells)
                }
            }
            Some(SelectionState::Node(node)) => {
                // Repeated anonymous atoms still have a proven relative occurrence when the
                // selected table's complete cell topology/ordered content is unchanged.
                let local = node
                    .path
                    .0
                    .strip_prefix(before.path.0.as_slice())
                    .and_then(|suffix| {
                        let mut path = selected.path.0.clone();
                        path.extend_from_slice(suffix);
                        let path = NodePath(path);
                        after
                            .node(&path)
                            .is_ok_and(|actual| actual == node.expected.as_ref())
                            .then_some(path)
                    });
                let path = if local.is_some()
                    && same_cell_occurrences(before.expected.as_ref(), selected.expected.as_ref())?
                {
                    local
                } else {
                    exact_retained_node(after, node.expected.as_ref())
                };
                if let Some(path) = path {
                    *node = after.node_ref(&path)?;
                    true
                } else {
                    false
                }
            }
            None => false,
        };
        if !retained {
            return Ok(None);
        }
        bookmark.revision = revision;
        Ok(Some(bookmark))
    }
    pub(crate) fn history_table_return(
        &self,
        bookmark: Option<&SelectionBookmark>,
    ) -> Option<SelectionBookmark> {
        let bookmark = bookmark?;
        if !std::sync::Arc::ptr_eq(&bookmark.session, &self.clipboard_state.session_identity())
            || !self.selected_table_node().is_ok_and(|node| node.is_some())
        {
            return None;
        }
        let valid = match &bookmark.selection {
            Some(SelectionState::Text(text)) => [&text.anchor, &text.head]
                .into_iter()
                .all(|point| valid_point(self.document(), point)),
            Some(SelectionState::Cells(cells)) => cells
                .clone()
                .canonicalized(self.document())
                .is_ok_and(|target| target == *cells),
            Some(SelectionState::Node(node)) => self
                .document()
                .node(&node.path)
                .is_ok_and(|actual| actual == node.expected.as_ref()),
            None => false,
        };
        valid.then(|| {
            let mut bookmark = bookmark.clone();
            bookmark.revision = self.revision();
            bookmark
        })
    }
}
fn valid_point(document: &Document, point: &crate::DocPoint) -> bool {
    document.block(&point.block_id).is_ok_and(|node| {
        matches!(
            node.node_type.as_str(),
            "paragraph" | "heading" | "codeBlock"
        ) && point.offset.0 <= node.inline_text().chars().count()
    })
}
fn exact_retained_node(document: &Document, expected: &Node) -> Option<NodePath> {
    fn collect(node: &Node, path: NodePath, expected: &Node, paths: &mut Vec<NodePath>) {
        if node == expected {
            paths.push(path.clone());
        }
        for (index, child) in node.children().iter().enumerate() {
            collect(child, path.child(index), expected, paths);
        }
    }
    let mut paths = Vec::new();
    collect(document.root(), NodePath::root(), expected, &mut paths);
    (paths.len() == 1).then(|| paths.remove(0))
}
fn same_cell_occurrences(before: &Node, after: &Node) -> Result<bool> {
    let old = TableGrid::new(before)?;
    let new = TableGrid::new(after)?;
    if old.rows != new.rows || old.columns != new.columns || old.cells.len() != new.cells.len() {
        return Ok(false);
    }
    for (a, b) in old.cells.iter().zip(&new.cells) {
        if (a.row, a.cell_index, a.column, a.rowspan, a.colspan)
            != (b.row, b.cell_index, b.column, b.rowspan, b.colspan)
        {
            return Ok(false);
        }
        let a = &before.children()[a.row].children()[a.cell_index];
        let b = &after.children()[b.row].children()[b.cell_index];
        if occurrence(a)? != occurrence(b)? {
            return Ok(false);
        }
    }
    Ok(true)
}
fn occurrence(node: &Node) -> Result<Vec<serde_json::Value>> {
    if matches!(
        node.node_type.as_str(),
        "paragraph" | "heading" | "codeBlock"
    ) {
        return Ok(vec![
            serde_json::json!({"id":node.block_id(),"text":node.inline_text()}),
        ]);
    }
    if matches!(
        node.node_type.as_str(),
        "imageBlock" | "mermaidBlock" | "mathBlock" | "horizontalRule" | "sceneBreak"
    ) {
        return Ok(vec![
            serde_json::to_value(node).map_err(craft_editor_model::Error::from)?,
        ]);
    }
    node.children()
        .iter()
        .try_fold(Vec::new(), |mut output, child| {
            output.extend(occurrence(child)?);
            Ok(output)
        })
}
