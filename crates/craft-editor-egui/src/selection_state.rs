//! One revision-local host bookmark over mutually exclusive native selection forms.
use crate::{EditorSession, Error, Result, Selection, TableSelection};
use craft_editor_model::NodeRef;

/// One semantic selection form. This value carries positions, not authority to edit another session.
#[derive(Debug, Clone, PartialEq)]
pub enum SelectionState {
    /// Directional text range or collapsed insertion caret.
    Text(Selection),
    /// Exact guarded opaque block/inline atom.
    Node(NodeRef),
    /// Directional originating table cells.
    Cells(TableSelection),
}
impl EditorSession {
    /// Capture one selection form, rejecting an impossible mixed internal state.
    /// The host must independently bind the bookmark to the same session/document/base revision.
    pub fn selection_state(&self) -> Result<Option<SelectionState>> {
        match (&self.selection, &self.node_selection, &self.table_selection) {
            (None, None, None) => Ok(None),
            (Some(text), None, None) => Ok(Some(SelectionState::Text(text.clone()))),
            (None, Some(node), None) => Ok(Some(SelectionState::Node(node.clone()))),
            (None, None, Some(cells)) => Ok(Some(SelectionState::Cells(cells.clone()))),
            _ => Err(Error::InvalidSelection),
        }
    }
    /// Restore a revision-local selection through the existing complete validation path.
    /// Rejected targets and active IME, including a request to clear selection, change nothing.
    pub fn restore_selection_state(&mut self, state: Option<SelectionState>) -> Result<()> {
        if self.is_composing() {
            return Err(Error::CompositionActive);
        }
        match state {
            Some(SelectionState::Text(selection)) => self.set_selection(selection),
            Some(SelectionState::Node(node)) => self.select_node(node),
            Some(SelectionState::Cells(cells)) => self.set_cell_selection(cells),
            None => {
                self.selection = None;
                self.node_selection = None;
                self.table_selection = None;
                self.stored_marks = None;
                self.restore_cursor = true;
                Ok(())
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CellPoint, DocPoint};
    use craft_editor_model::{Document, NodePath, Revision, ScalarOffset};
    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
    #[test]
    fn state_bookmark_rejects_mixed_invalid_and_ime_targets_without_partial_clear() -> TestResult {
        let mut session = EditorSession::new(Document::plain("p", "ABC")?, Revision(0));
        let selection = Selection {
            anchor: DocPoint {
                block_id: "p".into(),
                offset: ScalarOffset(0),
            },
            head: DocPoint {
                block_id: "p".into(),
                offset: ScalarOffset(2),
            },
        };
        session.set_selection(selection.clone())?;
        let before = session.selection_state()?;
        let invalid = SelectionState::Cells(TableSelection {
            table_path: NodePath(vec![0]),
            anchor: CellPoint { row: 0, column: 0 },
            head: CellPoint { row: 0, column: 0 },
        });
        assert!(
            session
                .restore_selection_state(Some(invalid.clone()))
                .is_err()
        );
        assert_eq!(session.selection_state()?, before);
        session.composition = Some(crate::Composition {
            document: session.document().clone(),
            commands: vec![],
            before_selection: session.selection.clone(),
        });
        for target in [None, before.clone(), Some(invalid)] {
            assert!(matches!(
                session.restore_selection_state(target),
                Err(Error::CompositionActive)
            ));
            assert_eq!(session.selection_state()?, before);
        }
        session.composition = None;
        session.table_selection = Some(TableSelection {
            table_path: NodePath(vec![0]),
            anchor: CellPoint { row: 0, column: 0 },
            head: CellPoint { row: 0, column: 0 },
        });
        assert!(session.selection_state().is_err());
        assert_eq!(session.selection(), Some(&selection));
        session.table_selection = None;
        session.restore_selection_state(None)?;
        assert_eq!(session.selection_state()?, None);
        assert!(!session.can_undo());
        Ok(())
    }
}
