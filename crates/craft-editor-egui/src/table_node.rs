//! Whole-table selection stays distinct from rectangular cells and opaque atom hit targets.
use crate::{DocPoint, EditorSession, Error, Result, Selection, projection::Projection};
use craft_editor_model::{
    Command, Document, Editor, Node, NodePath, NodeRef, ScalarOffset, Transaction,
};

impl EditorSession {
    pub(crate) fn selected_table_node(&self) -> Result<Option<&NodeRef>> {
        let Some(reference) = self
            .node_selection
            .as_ref()
            .filter(|node| node.expected.node_type == "table")
        else {
            return Ok(None);
        };
        if self.document().node(&reference.path)? != reference.expected.as_ref() {
            return Err(craft_editor_model::Error::NodeReferenceConflict.into());
        }
        Ok(Some(reference))
    }
    /// Select the complete innermost current table, preserving its exact prior native occurrence.
    /// This never converts the node into a rectangular cell range or changes canonical data.
    pub fn select_current_table(&mut self) -> Result<()> {
        if self.selected_table_node()?.is_some() {
            return Ok(());
        }
        let path = self
            .current_table_cell()?
            .ok_or(Error::InvalidSelection)?
            .table_path;
        self.select_node(self.document().node_ref(&path)?)
    }
    /// Leave a whole-table selection only through its sealed same-session/revision return target.
    /// A stale target keeps the selected table and history unchanged rather than guessing a caret.
    pub fn leave_table_node_selection(&mut self) -> Result<bool> {
        if self.selected_table_node()?.is_none() {
            return Ok(false);
        }
        let bookmark = self
            .table_node_return
            .clone()
            .ok_or(Error::StaleSelectionBookmark)?;
        self.restore_selection_bookmark(&bookmark)?;
        self.table_node_return = None;
        Ok(true)
    }
    /// Move before/after the complete selected table using the source block-selection boundary.
    /// A sole table has no outside semantic target and stays selected, without creating content.
    pub fn navigate_selected_table(&mut self, backward: bool) -> Result<bool> {
        let Some(reference) = self.selected_table_node()? else {
            return Ok(false);
        };
        let path = reference.path.clone();
        let projection = Projection::new(self.document())?;
        let target = if backward {
            projection
                .spans
                .iter()
                .rev()
                .find(|span| span.path < path.0 && !span.path.starts_with(&path.0))
        } else {
            projection
                .spans
                .iter()
                .find(|span| span.path > path.0 && !span.path.starts_with(&path.0))
        };
        let Some(target) = target else {
            return Ok(true);
        };
        if target.atom {
            self.select_node(self.document().node_ref(&NodePath(target.path.clone()))?)?;
        } else {
            let point = DocPoint {
                block_id: target.id.clone(),
                offset: ScalarOffset(if backward {
                    target.end - target.start
                } else {
                    0
                }),
            };
            self.set_selection(Selection {
                anchor: point.clone(),
                head: point,
            })?;
        }
        Ok(true)
    }
    pub(crate) fn commit_selected_table_node(
        &mut self,
        commands: Vec<Command>,
        reference: NodeRef,
    ) -> Result<craft_editor_model::Change> {
        if self.document().node(&reference.path)? != reference.expected.as_ref() {
            return Err(craft_editor_model::Error::NodeReferenceConflict.into());
        }
        let stable_slot = commands
            .iter()
            .all(|command| keeps_table_slot(command, &reference.path));
        let mut candidate = Editor::new(self.document().clone(), self.revision());
        candidate.apply(Transaction {
            base_revision: self.revision(),
            commands: commands.clone(),
        })?;
        let path = retained_table_path(candidate.document(), &reference, stable_slot)?;
        let after = candidate.document().node_ref(&path)?;
        self.apply_commands_with_node_selection(commands, after)
    }
}

// Source formatting preserves the selected table occurrence. Prefer its stable identity; an
// anonymous wrapper operation must prove one exact retained subtree and never choose a duplicate.
fn keeps_table_slot(command: &Command, table: &NodePath) -> bool {
    match command {
        Command::ApplyMarks { .. }
        | Command::ReplaceText { .. }
        | Command::SetBlockAttrs { .. } => true,
        Command::ReplaceNode { target, .. } | Command::ReplaceInline { target, .. } => {
            target.path.0.starts_with(&table.0)
        }
        Command::InsertChildren { parent, .. }
        | Command::ReplaceChildren { parent, .. }
        | Command::DeleteChildren { parent, .. } => parent.path.0.starts_with(&table.0),
        Command::MoveChildren {
            source,
            destination,
            ..
        } => source.path.0.starts_with(&table.0) && destination.path.0.starts_with(&table.0),
        _ => false,
    }
}
fn retained_table_path(
    document: &Document,
    reference: &NodeRef,
    stable_slot: bool,
) -> Result<NodePath> {
    fn collect(node: &Node, path: NodePath, reference: &NodeRef, output: &mut Vec<NodePath>) {
        let matches = if let Some(id) = reference.expected.block_id() {
            node.node_type == "table" && node.block_id() == Some(id)
        } else {
            node == reference.expected.as_ref()
        };
        if matches {
            output.push(path.clone());
        }
        for (index, child) in node.children().iter().enumerate() {
            collect(child, path.child(index), reference, output);
        }
    }
    if stable_slot
        && reference.expected.block_id().is_none()
        && document
            .node(&reference.path)
            .is_ok_and(|node| node.node_type == "table")
    {
        return Ok(reference.path.clone());
    }
    let mut paths = Vec::new();
    collect(document.root(), NodePath::root(), reference, &mut paths);
    if paths.len() == 1 {
        return Ok(paths.remove(0));
    }
    // A direct wrapper around this one selected slot is still an exact controlled occurrence.
    if reference.expected.block_id().is_none()
        && let Ok(slot) = document.node(&reference.path)
    {
        let mut local = Vec::new();
        collect(slot, reference.path.clone(), reference, &mut local);
        if local.len() == 1 {
            return Ok(local.remove(0));
        }
    }
    Err(Error::InvalidSelection)
}
