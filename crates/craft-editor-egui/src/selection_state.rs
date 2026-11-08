//! One revision-local host bookmark over mutually exclusive native selection forms.
use crate::{EditorSession, Error, Result, Selection, TableSelection};
use craft_editor_model::NodeRef;

/// One semantic selection form. This value carries positions, not authority to edit another session.
#[derive(Debug, Clone, PartialEq)]
pub enum SelectionState {
    /// Directional text range or collapsed insertion caret.
    Text(Selection),
    /// Exact guarded opaque atom or complete table.
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
                self.table_node_return = None;
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

/// Sealed positional bookmark for one native session/revision, including pending typing marks.
///
/// This is not database, clipboard-provider or project authority. Hosts retain their storage guards.
/// It cannot be forged or deserialized from external data.
/// ```compile_fail
/// use craft_editor_egui::SelectionBookmark;
/// let forged = SelectionBookmark { revision: craft_editor_model::Revision(0) };
/// ```
#[derive(Debug, Clone)]
pub struct SelectionBookmark {
    pub(crate) session: std::sync::Arc<()>,
    pub(crate) revision: craft_editor_model::Revision,
    pub(crate) selection: Option<SelectionState>,
    pub(crate) stored_marks: Option<Vec<craft_editor_model::Mark>>,
}
impl EditorSession {
    /// Freeze one selection and its stored marks without cloning unrelated manuscript content.
    pub fn capture_selection_bookmark(&self) -> Result<SelectionBookmark> {
        if self.is_composing() {
            return Err(Error::CompositionActive);
        }
        Ok(SelectionBookmark {
            session: self.clipboard_state.session_identity(),
            revision: self.revision(),
            selection: self.selection_state()?,
            stored_marks: self.stored_marks.clone(),
        })
    }
    /// Restore only within the captured session/revision, after validating the full target.
    /// IME, foreign/stale identity and invalid positions preserve the current selection and marks.
    pub fn restore_selection_bookmark(&mut self, bookmark: &SelectionBookmark) -> Result<()> {
        if self.is_composing() {
            return Err(Error::CompositionActive);
        }
        if !std::sync::Arc::ptr_eq(&self.clipboard_state.session_identity(), &bookmark.session)
            || self.revision() != bookmark.revision
        {
            return Err(Error::StaleSelectionBookmark);
        }
        self.restore_selection_state(bookmark.selection.clone())?;
        self.stored_marks = bookmark.stored_marks.clone();
        Ok(())
    }
}
#[cfg(test)]
mod bookmark_tests {
    use super::*;
    use crate::{DocPoint, MarkState};
    use craft_editor_model::{Document, Mark, Revision, ScalarOffset};
    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
    fn selected(session: &mut EditorSession, offset: usize) -> Result<()> {
        let point = DocPoint {
            block_id: "p".into(),
            offset: ScalarOffset(offset),
        };
        session.set_selection(Selection {
            anchor: point.clone(),
            head: point,
        })
    }
    #[test]
    fn sealed_bookmark_restores_marks_but_rejects_foreign_same_revision_and_stale_history()
    -> TestResult {
        let original = Document::plain("p", "ABC")?;
        let mut session = EditorSession::new(original.clone(), Revision(0));
        selected(&mut session, 1)?;
        session.toggle_mark(Mark::new("bold"))?;
        let bookmark = session.capture_selection_bookmark()?;
        let saved = session.selection_state()?;
        selected(&mut session, 2)?;
        session.restore_selection_bookmark(&bookmark)?;
        assert_eq!(session.selection_state()?, saved);
        assert_eq!(session.mark_state("bold")?, MarkState::On);
        assert_eq!(session.document(), &original);
        assert!(!session.can_undo());
        let mut other = EditorSession::new(original.clone(), Revision(0));
        selected(&mut other, 2)?;
        let before = other.selection_state()?;
        assert!(matches!(
            other.restore_selection_bookmark(&bookmark),
            Err(Error::StaleSelectionBookmark)
        ));
        assert_eq!(other.selection_state()?, before);
        assert_eq!(other.document(), &original);
        session.insert_plain_text("X")?;
        let before = session.selection_state()?;
        let written = session.document().clone();
        assert!(matches!(
            session.restore_selection_bookmark(&bookmark),
            Err(Error::StaleSelectionBookmark)
        ));
        assert_eq!(session.selection_state()?, before);
        assert_eq!(session.document(), &written);
        session.undo()?;
        assert!(matches!(
            session.restore_selection_bookmark(&bookmark),
            Err(Error::StaleSelectionBookmark)
        ));
        assert_eq!(session.document(), &original);
        Ok(())
    }
    #[test]
    fn sealed_none_and_invalid_bookmarks_do_not_clear_live_selection_during_ime_or_failure()
    -> TestResult {
        let mut session = EditorSession::new(Document::plain("p", "ABC")?, Revision(0));
        let empty = session.capture_selection_bookmark()?;
        selected(&mut session, 1)?;
        let before = session.selection_state()?;
        session.composition = Some(crate::Composition {
            document: session.document().clone(),
            commands: vec![],
            before_selection: session.selection.clone(),
        });
        assert!(session.capture_selection_bookmark().is_err());
        assert!(matches!(
            session.restore_selection_bookmark(&empty),
            Err(Error::CompositionActive)
        ));
        assert_eq!(session.selection_state()?, before);
        session.composition = None;
        let mut invalid = session.capture_selection_bookmark()?;
        invalid.selection = Some(SelectionState::Text(Selection {
            anchor: DocPoint {
                block_id: "missing".into(),
                offset: ScalarOffset(0),
            },
            head: DocPoint {
                block_id: "missing".into(),
                offset: ScalarOffset(0),
            },
        }));
        assert!(session.restore_selection_bookmark(&invalid).is_err());
        assert_eq!(session.selection_state()?, before);
        assert!(!session.can_undo());
        session.restore_selection_bookmark(&empty)?;
        assert_eq!(session.selection_state()?, None);
        Ok(())
    }
}
