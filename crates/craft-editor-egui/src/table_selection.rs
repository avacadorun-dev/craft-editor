//! Directional rectangular selection over the existing one-document rich session.
use crate::{EditorSession, Error, Result};
use craft_editor_model::{CellSelection, Document, NodePath, TableGrid};

/// One logical table coordinate. Covered coordinates resolve to their originating spanned cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CellPoint {
    /// Physical row, starting at zero.
    pub row: usize,
    /// Logical grid column, starting at zero.
    pub column: usize,
}
/// Directional table selection without duplicating canonical table writing in view/history state.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TableSelection {
    /// Revision-local path to the table. Commands map this path or invalidate the selection.
    pub table_path: NodePath,
    /// Original drag/keyboard endpoint.
    pub anchor: CellPoint,
    /// Current drag/keyboard endpoint; direction survives Undo/Redo.
    pub head: CellPoint,
}
impl TableSelection {
    /// Resolve the source-compatible rectangle between both originating cells.
    pub fn rectangle(&self, document: &Document) -> Result<CellSelection> {
        let grid = TableGrid::new(document.node(&self.table_path)?)?;
        let anchor = grid
            .cell_at(self.anchor.row, self.anchor.column)
            .ok_or(Error::InvalidSelection)?;
        let head = grid
            .cell_at(self.head.row, self.head.column)
            .ok_or(Error::InvalidSelection)?;
        Ok(CellSelection {
            top: anchor.row.min(head.row),
            left: anchor.column.min(head.column),
            bottom: anchor.bottom().max(head.bottom()),
            right: anchor.right().max(head.right()),
        })
    }
    pub(crate) fn canonicalized(mut self, document: &Document) -> Result<Self> {
        let grid = TableGrid::new(document.node(&self.table_path)?)?;
        for point in [&mut self.anchor, &mut self.head] {
            let rect = grid
                .cell_at(point.row, point.column)
                .ok_or(Error::InvalidSelection)?;
            *point = CellPoint {
                row: rect.row,
                column: rect.column,
            };
        }
        Ok(self)
    }
    pub(crate) fn origin_paths(&self, document: &Document) -> Result<Vec<NodePath>> {
        let grid = TableGrid::new(document.node(&self.table_path)?)?;
        let rect = self.rectangle(document)?;
        Ok(grid
            .cells
            .iter()
            .filter(|cell| {
                rect.top <= cell.row
                    && cell.row < rect.bottom
                    && rect.left <= cell.column
                    && cell.column < rect.right
            })
            .map(|cell| self.table_path.child(cell.row).child(cell.cell_index))
            .collect())
    }
}
impl EditorSession {
    pub(crate) fn validate_table_capacity(&self, document: &Document) -> Result<()> {
        let Some(maximum) = self.max_table_columns else {
            return Ok(());
        };
        let mut stack = vec![document.root()];
        while let Some(node) = stack.pop() {
            if node.node_type == "table" {
                let requested = TableGrid::new(node)?.columns;
                if requested > maximum {
                    return Err(Error::TableColumnLimit { requested, maximum });
                }
            }
            stack.extend(node.children());
        }
        Ok(())
    }
    /// Current directional cell rectangle; text/node selections are mutually exclusive with it.
    pub fn cell_selection(&self) -> Option<&TableSelection> {
        self.table_selection.as_ref()
    }
    /// Select validated originating cells without changing canonical content or history.
    pub fn set_cell_selection(&mut self, selection: TableSelection) -> Result<()> {
        if self.is_composing() {
            return Err(Error::CompositionActive);
        }
        let selection = selection.canonicalized(self.document())?;
        self.selection = None;
        self.node_selection = None;
        self.stored_marks = None;
        self.table_selection = Some(selection);
        self.restore_cursor = true;
        #[cfg(feature = "wordcraft-canvas")]
        self.request_scroll_to_caret();
        Ok(())
    }
    /// Set an optional host-native edit capacity. This never changes generic canonical parsing.
    /// A limit affects future table edits; callers retain responsibility for read-only wider views.
    pub fn set_table_column_limit(&mut self, limit: Option<usize>) -> Result<()> {
        if limit == Some(0) {
            return Err(Error::InvalidSelection);
        }
        self.max_table_columns = limit;
        Ok(())
    }
    /// Current optional native table edit capacity.
    pub fn table_column_limit(&self) -> Option<usize> {
        self.max_table_columns
    }
    /// Commit a guarded batch with its rectangular post-selection in the same history entry.
    pub fn apply_commands_with_cell_selection(
        &mut self,
        commands: Vec<craft_editor_model::Command>,
        selection: TableSelection,
    ) -> Result<craft_editor_model::Change> {
        if self.is_composing() {
            return Err(Error::CompositionActive);
        }
        let mut candidate =
            craft_editor_model::Editor::new(self.document().clone(), self.revision());
        candidate.apply(craft_editor_model::Transaction {
            base_revision: self.revision(),
            commands: commands.clone(),
        })?;
        self.validate_table_capacity(candidate.document())?;
        let selection = selection.canonicalized(candidate.document())?;
        let change = self.apply_commands(commands)?;
        self.table_selection = Some(selection);
        self.selection = None;
        self.node_selection = None;
        self.stored_marks = None;
        self.restore_cursor = true;
        if change.before_revision != change.after_revision
            && let Some(history) = self.undo_selection.last_mut()
        {
            history.after = None;
            history.after_node = None;
            history.after_cells = self.table_selection.clone();
        }
        #[cfg(feature = "wordcraft-canvas")]
        self.request_scroll_to_caret();
        Ok(change)
    }
}

impl EditorSession {
    fn table_paste_target(
        &self,
    ) -> Result<Option<(craft_editor_model::NodeRef, CellSelection, bool)>> {
        if let Some(selection) = &self.table_selection {
            return Ok(Some((
                self.document().node_ref(&selection.table_path)?,
                selection.rectangle(self.document())?,
                true,
            )));
        }
        let path = if let Some(reference) = &self.node_selection {
            reference.path.clone()
        } else if let Some(selection) = &self.selection {
            let projection = crate::projection::Projection::new(self.document())?;
            let span = projection
                .spans
                .iter()
                .find(|span| span.id == selection.head.block_id && !span.atom)
                .ok_or(Error::InvalidSelection)?;
            NodePath(span.path.clone())
        } else {
            return Ok(None);
        };
        for depth in (1..=path.0.len()).rev() {
            let cell_path = NodePath(path.0[..depth].to_vec());
            if !matches!(
                self.document().node(&cell_path)?.node_type.as_str(),
                "tableCell" | "tableHeader"
            ) {
                continue;
            }
            if depth < 2 {
                return Err(Error::InvalidSelection);
            }
            let table_path = NodePath(path.0[..depth - 2].to_vec());
            let target = self.document().node_ref(&table_path)?;
            let grid = TableGrid::new(&target.expected)?;
            let row = path.0[depth - 2];
            let index = path.0[depth - 1];
            let rect = grid
                .cells
                .iter()
                .find(|rect| rect.row == row && rect.cell_index == index)
                .ok_or(Error::InvalidSelection)?;
            return Ok(Some((
                target,
                CellSelection {
                    top: rect.row,
                    left: rect.column,
                    bottom: rect.bottom(),
                    right: rect.right(),
                },
                false,
            )));
        }
        Ok(None)
    }
    pub(crate) fn try_paste_table_slice(
        &mut self,
        slice: &crate::clipboard::RichSlice,
        max_bytes: usize,
    ) -> Result<Option<craft_editor_model::Change>> {
        if self.is_composing() {
            return Err(Error::CompositionActive);
        }
        let Some((target, selection, repeat)) = self.table_paste_target()? else {
            return Ok(None);
        };
        let mut next = self.next_id;
        let prefix = self.revision().0;
        let mut reserved = crate::insertion::block_id_reservations(self.document());
        fn reserve(
            nodes: &[craft_editor_model::Node],
            reserved: &mut std::collections::HashSet<String>,
        ) {
            for node in nodes {
                if let Some(id) = node.block_id() {
                    reserved.insert(id.into());
                }
                reserve(node.children(), reserved);
            }
        }
        reserve(&slice.content, &mut reserved);
        let mut allocate = || {
            crate::insertion::allocate_block_id(prefix, &mut next, &mut reserved).map_err(|error| {
                match error {
                    Error::Model(error) => error,
                    _ => craft_editor_model::Error::UnsupportedEdit,
                }
            })
        };
        let source = match craft_editor_model::table_cells_from_slice(
            &slice.content,
            slice.open_start,
            slice.open_end,
            &mut allocate,
        )? {
            Some(cells) => cells,
            None if repeat => craft_editor_model::PastedCells::from_writing(
                &slice.content,
                slice.open_start,
                slice.open_end,
                &mut allocate,
            )?,
            None => return Ok(None),
        };
        let current = TableGrid::new(&target.expected)?.columns;
        let requested = if repeat {
            current
        } else {
            selection
                .left
                .checked_add(source.width)
                .ok_or(craft_editor_model::Error::ResourceLimit)?
                .max(current)
        };
        if let Some(maximum) = self.max_table_columns
            && requested > maximum
        {
            return Err(Error::TableColumnLimit { requested, maximum });
        }
        let (command, after) = craft_editor_model::paste_table_cells(
            self.document(),
            &target,
            selection,
            &source,
            craft_editor_model::TablePasteOptions {
                repeat,
                max_expanded_bytes: max_bytes,
            },
            &mut allocate,
        )?;
        let next_id = next;
        let change = self.apply_commands_with_cell_selection(
            vec![command],
            TableSelection {
                table_path: target.path,
                anchor: CellPoint {
                    row: after.top,
                    column: after.left,
                },
                head: CellPoint {
                    row: after.bottom - 1,
                    column: after.right - 1,
                },
            },
        )?;
        self.next_id = next_id;
        Ok(Some(change))
    }
}

impl EditorSession {
    /// Clear selected cell contents while retaining their headers, spans and rectangular selection.
    /// This matches native Backspace/Delete. Cut instead collapses to the head cell's new caret.
    pub fn clear_selected_cells(&mut self) -> Result<craft_editor_model::Change> {
        self.replace_selected_cells("", true)
    }
    pub(crate) fn replace_selected_cells(
        &mut self,
        text: &str,
        preserve_rectangle: bool,
    ) -> Result<craft_editor_model::Change> {
        if self.is_composing() {
            return Err(Error::CompositionActive);
        }
        let selected = self
            .table_selection
            .clone()
            .ok_or(Error::InvalidSelection)?;
        let table = self.document().node(&selected.table_path)?;
        let grid = TableGrid::new(table)?;
        if let Some(maximum) = self.max_table_columns
            && grid.columns > maximum
        {
            return Err(Error::TableColumnLimit {
                requested: grid.columns,
                maximum,
            });
        }
        let head = grid
            .cell_at(selected.head.row, selected.head.column)
            .ok_or(Error::InvalidSelection)?;
        let head_path = selected.table_path.child(head.row).child(head.cell_index);
        let paths = selected.origin_paths(self.document())?;
        let mut plan = crate::insertion::Plan::new(self, &[])?;
        let mut caret = None;
        for path in paths {
            let target = plan.doc().node_ref(&path)?;
            let mut cell = target.expected.as_ref().clone();
            let id = plan.id()?;
            cell.content = Some(vec![craft_editor_model::Node::paragraph(&id, "")]);
            let command = plan.doc().replace_node(&target, cell)?;
            plan.execute(vec![command])?;
            if path == head_path {
                caret = Some(crate::DocPoint {
                    block_id: id,
                    offset: craft_editor_model::ScalarOffset(0),
                });
            }
        }
        let mut caret = caret.ok_or(Error::InvalidSelection)?;
        if !text.is_empty() {
            caret = plan.insert_text_fragment(caret, &craft_editor_model::Node::text(""), text)?;
        }
        let next = plan.next_id;
        let change = if preserve_rectangle {
            self.apply_commands_with_cell_selection(plan.commands, selected)?
        } else {
            self.apply_commands_with_selection(
                plan.commands,
                crate::Selection {
                    anchor: caret.clone(),
                    head: caret,
                },
            )?
        };
        self.next_id = next;
        Ok(change)
    }
}
