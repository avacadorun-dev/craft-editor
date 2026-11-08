//! Continuous native semantic editing checkpoint over canonical rich documents.
//!
//! One document-level egui input/selection surface maps edits into revision-guarded model
//! transactions. This is not full editor parity: table layout is currently a linear editable
//! cell traversal; complex blocks/atoms and rich system clipboard still need their real renderers.
#![deny(missing_docs)]

/// Optional hosted qualification of a pinned native flow/table geometry backend.
#[cfg(feature = "wordcraft-geometry")]
pub mod layout;

/// Native raster canvas with a single semantic caret and selection.
#[cfg(feature = "wordcraft-canvas")]
pub mod canvas;

mod projection;
use craft_editor_model::{
    Change, Command, Document, Editor, Mark, MarkMode, Revision, ScalarOffset, TextRange,
    Transaction,
};
use egui::{
    Color32, FontFamily, Id, Key, Modifiers, TextBuffer, Vec2,
    text::{CCursor, CCursorRange},
};
use projection::Projection;
use std::ops::Range;

/// Content-free view diagnostic; hosts localize the error code.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The native flow backend rejected a geometry or raster operation.
    #[cfg(feature = "wordcraft-geometry")]
    #[error(transparent)]
    Geometry(#[from] layout::Error),
    /// The semantic model refused the requested operation.
    #[error(transparent)]
    Model(#[from] craft_editor_model::Error),
    /// Editable text blocks need durable identities; missing IDs are not invented while painting.
    #[error("missing_block_identity")]
    MissingBlockIdentity,
    /// The supplied document-level selection cannot be resolved against this revision.
    #[error("invalid_selection")]
    InvalidSelection,
    /// The operation crosses a structural boundary whose transaction is not implemented yet.
    #[error("unsupported_structural_edit")]
    UnsupportedStructuralEdit,
    /// The current checkpoint has no qualified renderer/input contract for this block.
    #[error("unsupported_block_renderer: {0}")]
    UnsupportedBlockKind(String),
    /// A canonical command cannot interrupt an uncommitted native IME composition.
    #[error("composition_active")]
    CompositionActive,
}
/// Result returned by semantic view commands.
pub type Result<T> = std::result::Result<T, Error>;

/// One canonical block-relative Unicode-scalar point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocPoint {
    /// Stable canonical text-block identity.
    pub block_id: String,
    /// Unicode scalar offset, not UTF-16 units or UTF-8 bytes.
    pub offset: ScalarOffset,
}
/// Document-level directed selection, including ranges across paragraphs and table cells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// Fixed endpoint of a drag or shifted keyboard selection.
    pub anchor: DocPoint,
    /// Moving endpoint/caret.
    pub head: DocPoint,
}
/// Host-owned visual options; this library does not install fonts or replace the host theme.
#[derive(Debug, Clone)]
pub struct ViewOptions {
    /// Text size in logical points.
    pub font_size: f32,
    /// Font family already installed by the host.
    pub font_family: FontFamily,
    /// Optional real bold face; no fake bold font is synthesized.
    pub bold_family: Option<FontFamily>,
    /// Background identifying editable table-cell text in the current linear traversal.
    pub table_background: Color32,
    /// Stable editor identity, independent of the document's text or UI translation.
    pub id: Id,
}
impl Default for ViewOptions {
    fn default() -> Self {
        Self {
            font_size: 18.0,
            font_family: FontFamily::Proportional,
            bold_family: None,
            table_background: Color32::from_rgba_unmultiplied(90, 120, 160, 25),
            id: Id::new("craft-rich-editor"),
        }
    }
}
/// A frame's acknowledged rich changes and diagnostics. Hosts persist the structured changes.
pub struct ViewResponse {
    /// The single native input widget's response, usable for focus/context menus.
    pub response: egui::Response,
    /// Revision-guarded changes, never newly flattened paragraphs.
    pub changes: Vec<Change>,
    /// Rejected input or unsupported operations; canonical data remains unchanged on failure.
    pub errors: Vec<Error>,
}
#[derive(Clone)]
struct SelectionHistory {
    before: Option<Selection>,
    after: Option<Selection>,
}
struct Composition {
    document: Document,
    commands: Vec<Command>,
    before_selection: Option<Selection>,
}

/// One canonical document, semantic selection and rich Undo/Redo history.
pub struct EditorSession {
    editor: Editor,
    selection: Option<Selection>,
    undo_selection: Vec<SelectionHistory>,
    redo_selection: Vec<SelectionHistory>,
    pending: Vec<Change>,
    restore_cursor: bool,
    next_id: u64,
    composition: Option<Composition>,
    #[cfg(feature = "wordcraft-canvas")]
    canvas_state: canvas::CanvasState,
    #[cfg(feature = "wordcraft-canvas")]
    canvas_composing: bool,
    /// Host-owned presentation configuration.
    pub options: ViewOptions,
}
impl EditorSession {
    /// Open validated canonical state at its host revision.
    pub fn new(document: Document, revision: Revision) -> Self {
        Self {
            editor: Editor::new(document, revision),
            selection: None,
            undo_selection: vec![],
            redo_selection: vec![],
            pending: vec![],
            restore_cursor: false,
            next_id: 0,
            composition: None,
            #[cfg(feature = "wordcraft-canvas")]
            canvas_state: canvas::CanvasState::default(),
            #[cfg(feature = "wordcraft-canvas")]
            canvas_composing: false,
            options: ViewOptions::default(),
        }
    }
    /// Borrow the current canonical structured tree.
    pub fn document(&self) -> &Document {
        self.editor.document()
    }
    /// Current acknowledged semantic revision.
    pub fn revision(&self) -> Revision {
        self.editor.revision()
    }
    /// Whether one acknowledged canonical transaction can be undone now.
    pub fn can_undo(&self) -> bool {
        !self.is_composing() && self.editor.can_undo()
    }
    /// Whether one undone canonical transaction can be reapplied now.
    pub fn can_redo(&self) -> bool {
        !self.is_composing() && self.editor.can_redo()
    }
    /// Return keyboard focus from a host toolbar without changing semantic selection.
    #[cfg(feature = "wordcraft-canvas")]
    pub fn request_canvas_focus(&self, context: &egui::Context) {
        context.memory_mut(|memory| memory.request_focus(self.options.id));
    }
    /// Whether the native IME has an uncommitted display-only composition.
    pub fn is_composing(&self) -> bool {
        self.composition.is_some() || self.canvas_is_composing()
    }
    #[cfg(feature = "wordcraft-canvas")]
    fn canvas_is_composing(&self) -> bool {
        self.canvas_composing
    }
    #[cfg(not(feature = "wordcraft-canvas"))]
    fn canvas_is_composing(&self) -> bool {
        false
    }
    /// Borrow the directed document selection.
    pub fn selection(&self) -> Option<&Selection> {
        self.selection.as_ref()
    }
    /// Set an exact semantic selection; rejects points not present in this revision.
    pub fn set_selection(&mut self, selection: Selection) -> Result<()> {
        if self.is_composing() {
            return Err(Error::CompositionActive);
        }
        let projection = Projection::new(self.document())?;
        projection.scalar(&selection.anchor)?;
        projection.scalar(&selection.head)?;
        self.selection = Some(selection);
        self.restore_cursor = true;
        Ok(())
    }
    /// Plain clipboard projection of the exact selected occurrence, including block separators.
    pub fn selected_text(&self) -> Result<String> {
        if self.is_composing() {
            return Err(Error::CompositionActive);
        }
        let Some(selection) = &self.selection else {
            return Ok(String::new());
        };
        let projection = Projection::new(self.document())?;
        let a = projection.scalar(&selection.anchor)?;
        let b = projection.scalar(&selection.head)?;
        Ok(projection
            .text
            .chars()
            .skip(a.min(b))
            .take(a.abs_diff(b))
            .collect())
    }
    /// Apply explicit semantic commands through the same history used by native input.
    pub fn apply_commands(&mut self, commands: Vec<Command>) -> Result<Change> {
        if self.is_composing() {
            return Err(Error::CompositionActive);
        }
        let before = self.selection.clone();
        let change = self.editor.apply(Transaction {
            base_revision: self.revision(),
            commands,
        })?;
        if change.before_revision != change.after_revision {
            if let Some(selection) = &mut self.selection {
                for map in &change.mappings {
                    let (id, offset) =
                        map.map_caret(&selection.anchor.block_id, selection.anchor.offset, false);
                    selection.anchor = DocPoint {
                        block_id: id,
                        offset,
                    };
                    let (id, offset) =
                        map.map_caret(&selection.head.block_id, selection.head.offset, true);
                    selection.head = DocPoint {
                        block_id: id,
                        offset,
                    };
                }
            }
            if self.selection.as_ref().is_some_and(|selection| {
                [&selection.anchor, &selection.head]
                    .into_iter()
                    .any(|point| {
                        !self
                            .document()
                            .block_text(&point.block_id)
                            .is_ok_and(|text| point.offset.0 <= text.chars().count())
                    })
            }) {
                self.selection = None;
            }
            if self.undo_selection.len() >= 100 {
                self.undo_selection.remove(0);
            }
            self.undo_selection.push(SelectionHistory {
                before,
                after: self.selection.clone(),
            });
            self.redo_selection.clear();
            self.pending.push(change.clone());
            self.restore_cursor = true;
        }
        Ok(change)
    }
    /// Add or remove a mark across the exact selection's text-block ranges atomically.
    pub fn format_selection(&mut self, mark: Mark, mode: MarkMode) -> Result<Change> {
        let projection = Projection::new(self.document())?;
        let selection = self.selection.as_ref().ok_or(Error::InvalidSelection)?;
        let a = projection.scalar(&selection.anchor)?;
        let b = projection.scalar(&selection.head)?;
        let mut commands = vec![];
        for span in &projection.spans {
            let start = a.min(b).max(span.start);
            let end = a.max(b).min(span.end);
            if start < end {
                commands.push(Command::ApplyMarks {
                    range: TextRange {
                        block_id: span.id.clone(),
                        start: ScalarOffset(start - span.start),
                        end: ScalarOffset(end - span.start),
                    },
                    marks: vec![mark.clone()],
                    mode,
                });
            }
        }
        self.apply_commands(commands)
    }
    /// Undo a rich transaction and restore its semantic selection.
    pub fn undo(&mut self) -> Result<Change> {
        if self.is_composing() {
            return Err(Error::CompositionActive);
        }
        let change = self.editor.undo(self.revision())?;
        if let Some(entry) = self.undo_selection.pop() {
            self.selection = entry.before.clone();
            self.redo_selection.push(entry);
        }
        self.pending.push(change.clone());
        self.restore_cursor = true;
        Ok(change)
    }
    /// Redo a rich transaction and restore its semantic selection.
    pub fn redo(&mut self) -> Result<Change> {
        if self.is_composing() {
            return Err(Error::CompositionActive);
        }
        let change = self.editor.redo(self.revision())?;
        if let Some(entry) = self.redo_selection.pop() {
            self.selection = entry.after.clone();
            self.undo_selection.push(entry);
        }
        self.pending.push(change.clone());
        self.restore_cursor = true;
        Ok(change)
    }
    /// Display a real continuous-flow raster canvas with one document-level selection/history.
    ///
    /// The host supplies its colors and places this widget inside a vertical scroll area.
    /// Unsupported document kinds return a diagnostic without modifying canonical content.
    #[cfg(feature = "wordcraft-canvas")]
    pub fn show_canvas(&mut self, ui: &mut egui::Ui, style: &canvas::CanvasStyle) -> ViewResponse {
        let mut state = std::mem::take(&mut self.canvas_state);
        let response = state.show(self, ui, style);
        self.canvas_state = state;
        response
    }
    /// Display one native input surface for the complete editable traversal.
    ///
    /// Native egui caret, hit testing, shifted selection, IME and plain clipboard are reused.
    /// Complex block renderers, rich clipboard and platform IME acceptance remain qualification work.
    pub fn show(&mut self, ui: &mut egui::Ui) -> ViewResponse {
        let mut errors = vec![];
        let focused = ui.memory(|m| m.has_focus(self.options.id));
        if focused {
            let redo = ui.input_mut(|i| {
                i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)
                    || i.consume_key(Modifiers::COMMAND, Key::Y)
            });
            let undo = ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Z));
            let result = if redo {
                self.redo().map(|_| ())
            } else if undo {
                self.undo().map(|_| ())
            } else {
                Ok(())
            };
            if let Err(error) = result {
                errors.push(error);
            }
        }
        let (starts_composition, ends_composition) = ui.input(|input| {
            let mut starts = false;
            let mut ends = false;
            for event in &input.events {
                match event {
                    egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => {
                        starts |= !text.is_empty();
                        ends |= text.is_empty();
                    }
                    egui::Event::Ime(egui::ImeEvent::Commit(_)) => ends = true,
                    _ => {}
                }
            }
            (focused && starts, focused && ends)
        });
        let working_document = self
            .composition
            .as_ref()
            .map(|composition| &composition.document)
            .unwrap_or_else(|| self.document())
            .clone();
        let projection = match Projection::new(&working_document) {
            Ok(projection) => projection,
            Err(error) => {
                return ViewResponse {
                    response: ui.allocate_response(Vec2::ZERO, egui::Sense::hover()),
                    changes: std::mem::take(&mut self.pending),
                    errors: vec![error],
                };
            }
        };
        if self.restore_cursor {
            if let Some(selection) = &self.selection
                && let (Ok(a), Ok(b)) = (
                    projection.scalar(&selection.anchor),
                    projection.scalar(&selection.head),
                )
            {
                let mut state =
                    egui::TextEdit::load_state(ui.ctx(), self.options.id).unwrap_or_default();
                state
                    .cursor
                    .set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
                state.clear_undoer();
                state.store(ui.ctx(), self.options.id);
            }
            self.restore_cursor = false;
        }
        let mut buffer =
            SemanticBuffer::new(working_document, self.revision(), projection, self.next_id);
        let options = self.options.clone();
        let color = ui.visuals().text_color();
        let initial_projection = buffer.initial_projection.clone();
        let mut layouter = |ui: &egui::Ui, _text: &dyn TextBuffer, width: f32| {
            // A fresh canonical projection is installed below after accepted input. During an
            // intermediate IME mutation use its exact current text, never stale byte ranges.
            let mut job = buffer_layout(_text.as_str(), &options, color, width);
            if _text.as_str() == initial_projection.text {
                job = initial_projection.job(&options, color, width);
            }
            ui.fonts_mut(|fonts| fonts.layout_job(job))
        };
        let mut output = egui::TextEdit::multiline(&mut buffer)
            .id(self.options.id)
            .desired_width(f32::INFINITY)
            .desired_rows(12)
            .font(egui::FontId::new(
                options.font_size,
                options.font_family.clone(),
            ))
            .layouter(&mut layouter)
            .show(ui);
        output.state.clear_undoer();
        output.state.clone().store(ui.ctx(), self.options.id);
        self.next_id = buffer.next_id;
        let after_range = output.cursor_range;
        if let Some(error) = buffer.error.take() {
            errors.push(error);
            self.restore_cursor = true;
            if let Some(composition) = self.composition.take() {
                self.selection = composition.before_selection;
            }
        } else if (self.composition.is_some() || starts_composition) && !ends_composition {
            let composition = self.composition.get_or_insert_with(|| Composition {
                document: self.editor.document().clone(),
                commands: vec![],
                before_selection: self.selection.clone(),
            });
            composition.document = buffer.editor.document().clone();
            composition.commands.append(&mut buffer.commands);
        } else if !buffer.commands.is_empty() || self.composition.is_some() {
            let mut commands = if let Some(composition) = self.composition.take() {
                self.selection = composition.before_selection;
                composition.commands
            } else {
                vec![]
            };
            commands.append(&mut buffer.commands);
            match self.apply_commands(commands) {
                Ok(_) => {
                    if let Some(range) = after_range {
                        self.selection = selection_from_range(&buffer.projection, range).ok();
                    }
                    if let Some(entry) = self.undo_selection.last_mut() {
                        entry.after = self.selection.clone();
                    }
                    self.restore_cursor = false;
                }
                Err(error) => {
                    errors.push(error);
                    self.restore_cursor = true;
                }
            }
        } else if let Some(range) = after_range {
            self.selection = selection_from_range(&buffer.projection, range).ok();
        }
        ViewResponse {
            response: output.response.response,
            changes: std::mem::take(&mut self.pending),
            errors,
        }
    }
}
fn selection_from_range(projection: &Projection, range: CCursorRange) -> Result<Selection> {
    Ok(Selection {
        anchor: projection.point(usize::from(range.secondary.index), false)?,
        head: projection.point(usize::from(range.primary.index), true)?,
    })
}
fn buffer_layout(
    text: &str,
    options: &ViewOptions,
    color: Color32,
    width: f32,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = width;
    job.append(
        text,
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::new(options.font_size, options.font_family.clone()),
            color,
            ..Default::default()
        },
    );
    job
}

struct SemanticBuffer {
    editor: Editor,
    projection: Projection,
    initial_projection: Projection,
    commands: Vec<Command>,
    error: Option<Error>,
    next_id: u64,
}
impl SemanticBuffer {
    fn new(document: Document, revision: Revision, projection: Projection, next_id: u64) -> Self {
        Self {
            editor: Editor::new(document, revision),
            initial_projection: projection.clone(),
            projection,
            commands: vec![],
            error: None,
            next_id,
        }
    }
    fn execute(&mut self, commands: Vec<Command>) -> Result<()> {
        self.editor.apply(Transaction {
            base_revision: self.editor.revision(),
            commands: commands.clone(),
        })?;
        self.projection = Projection::new(self.editor.document())?;
        self.commands.extend(commands);
        Ok(())
    }
    fn insert(&mut self, text: &str, at: usize) -> Result<usize> {
        let point = self.projection.point(at, true)?;
        let mut commands = vec![];
        let lines: Vec<_> = text.split('\n').collect();
        let mut current = point.block_id;
        let mut offset = point.offset.0;
        for (index, line) in lines.iter().enumerate() {
            if !line.is_empty() {
                commands.push(Command::ReplaceText {
                    range: TextRange {
                        block_id: current.clone(),
                        start: ScalarOffset(offset),
                        end: ScalarOffset(offset),
                    },
                    expected: String::new(),
                    replacement: (*line).into(),
                });
                offset += line.chars().count();
            }
            if index + 1 < lines.len() {
                self.next_id = self.next_id.saturating_add(1);
                let next = format!("craft-view-{}-{}", self.editor.revision().0, self.next_id);
                commands.push(Command::SplitBlock {
                    block_id: current,
                    at: ScalarOffset(offset),
                    new_block_id: next.clone(),
                });
                current = next;
                offset = 0;
            }
        }
        self.execute(commands)?;
        Ok(text.chars().count())
    }
    fn delete(&mut self, range: Range<usize>) -> Result<()> {
        if range.is_empty() {
            return Ok(());
        }
        let first = self.projection.point(range.start, true)?;
        let last = self.projection.point(range.end, false)?;
        let first_index = self
            .projection
            .spans
            .iter()
            .position(|s| s.id == first.block_id)
            .ok_or(Error::InvalidSelection)?;
        let last_index = self
            .projection
            .spans
            .iter()
            .position(|s| s.id == last.block_id)
            .ok_or(Error::InvalidSelection)?;
        let spans = &self.projection.spans[first_index..=last_index];
        if spans.len() > 1 && spans.iter().any(|s| s.table || s.path.len() != 1) {
            return Err(Error::UnsupportedStructuralEdit);
        }
        let mut commands = vec![];
        for span in spans {
            let start = range.start.max(span.start) - span.start;
            let end = range.end.min(span.end).saturating_sub(span.start);
            if start < end {
                commands.push(Command::ReplaceText {
                    range: TextRange {
                        block_id: span.id.clone(),
                        start: ScalarOffset(start),
                        end: ScalarOffset(end),
                    },
                    expected: span
                        .node
                        .inline_text()
                        .chars()
                        .skip(start)
                        .take(end - start)
                        .collect(),
                    replacement: String::new(),
                });
            }
        }
        if spans.len() > 1 {
            for span in spans.iter().skip(1) {
                commands.push(Command::JoinBlocks {
                    left_id: first.block_id.clone(),
                    right_id: span.id.clone(),
                });
            }
        }
        self.execute(commands)
    }
}
impl TextBuffer for SemanticBuffer {
    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<Self>()
    }
    fn is_mutable(&self) -> bool {
        true
    }
    fn as_str(&self) -> &str {
        &self.projection.text
    }
    fn insert_text(&mut self, text: &str, char_index: egui::epaint::text::CharIndex) -> usize {
        if self.error.is_some() {
            return 0;
        }
        match self.insert(text, usize::from(char_index)) {
            Ok(count) => count,
            Err(error) => {
                self.error = Some(error);
                0
            }
        }
    }
    fn delete_char_range(&mut self, range: Range<egui::epaint::text::CharIndex>) {
        if self.error.is_none()
            && let Err(error) = self.delete(usize::from(range.start)..usize::from(range.end))
        {
            self.error = Some(error);
        }
    }
    fn replace_with(&mut self, _text: &str) {
        self.error = Some(Error::UnsupportedStructuralEdit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

    fn fixture() -> craft_editor_model::Result<Document> {
        Document::from_json(json!({"type":"doc","attrs":{"schemaVersion":4},"content":[
            {"type":"paragraph","attrs":{"blockId":"a","sourceRefs":["original"],"vendor":{"keep":true}},"content":[{"type":"text","text":"А🪶Б","marks":[{"type":"italic"}]}]},
            {"type":"table","attrs":{"blockId":"table"},"content":[{"type":"tableRow","content":[
                {"type":"tableCell","attrs":{"colspan":1,"rowspan":1},"content":[{"type":"paragraph","attrs":{"blockId":"cell-a"},"content":[{"type":"text","text":"Left"}]}]},
                {"type":"tableCell","attrs":{"colspan":1,"rowspan":1},"content":[{"type":"paragraph","attrs":{"blockId":"cell-b"},"content":[{"type":"text","text":"Right"}]}]}
            ]}]},
            {"type":"paragraph","attrs":{"blockId":"b"},"content":[{"type":"text","text":"End"}]}
        ]}))
    }
    fn point(id: &str, offset: usize) -> DocPoint {
        DocPoint {
            block_id: id.into(),
            offset: ScalarOffset(offset),
        }
    }
    fn frame(
        session: &mut EditorSession,
        context: &egui::Context,
        events: Vec<egui::Event>,
    ) -> ViewResponse {
        let mut result = None;
        let mut frame_output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(700.0, 500.0),
                )),
                events,
                focused: true,
                ..Default::default()
            },
            |root| {
                egui::CentralPanel::default().show(root, |ui| result = Some(session.show(ui)));
            },
        );
        frame_output.textures_delta.clear();
        match result {
            Some(result) => result,
            None => panic!("headless frame did not run"),
        }
    }
    #[test]
    fn one_selection_crosses_paragraph_and_table_cell_boundaries() -> TestResult {
        let mut session = EditorSession::new(fixture()?, Revision(7));
        session.set_selection(Selection {
            anchor: point("a", 1),
            head: point("cell-b", 3),
        })?;
        assert_eq!(session.selected_text()?, "🪶Б\nLeft\nRig");
        assert_eq!(session.revision(), Revision(7));
        Ok(())
    }
    #[test]
    fn table_cell_typing_preserves_grid_and_rich_source_metadata() -> TestResult {
        let original = fixture()?;
        let projection = Projection::new(&original)?;
        let mut buffer = SemanticBuffer::new(original.clone(), Revision(1), projection, 0);
        let at = buffer.projection.scalar(&point("cell-a", 2))?;
        buffer.insert("🪶", at)?;
        assert_eq!(buffer.editor.document().block_text("cell-a")?, "Le🪶ft");
        assert_eq!(buffer.editor.document().block("a")?, original.block("a")?);
        assert_eq!(
            buffer.editor.document().block("table")?.children()[0]
                .children()
                .len(),
            2
        );
        Ok(())
    }
    #[test]
    fn unsupported_cross_cell_delete_is_rejected_without_canonical_mutation() -> TestResult {
        let original = fixture()?;
        let projection = Projection::new(&original)?;
        let mut buffer = SemanticBuffer::new(original.clone(), Revision(1), projection, 0);
        let start = buffer.projection.scalar(&point("cell-a", 2))?;
        let end = buffer.projection.scalar(&point("cell-b", 2))?;
        assert!(matches!(
            buffer.delete(start..end),
            Err(Error::UnsupportedStructuralEdit)
        ));
        assert_eq!(buffer.editor.document(), &original);
        Ok(())
    }
    #[test]
    fn native_text_event_commits_semantic_table_cell_edit() -> TestResult {
        let context = egui::Context::default();
        let mut session = EditorSession::new(fixture()?, Revision(1));
        frame(&mut session, &context, vec![]);
        session.set_selection(Selection {
            anchor: point("cell-a", 1),
            head: point("cell-a", 1),
        })?;
        context.memory_mut(|memory| memory.request_focus(session.options.id));
        let result = frame(&mut session, &context, vec![egui::Event::Text("字".into())]);
        assert!(
            result.errors.is_empty(),
            "input errors: {:?}",
            result.errors
        );
        assert_eq!(session.document().block_text("cell-a")?, "L字eft");
        assert_eq!(result.changes.len(), 1);
        Ok(())
    }
    #[test]
    fn formatting_and_undo_restore_exact_rich_snapshot_and_selection() -> TestResult {
        let original = fixture()?;
        let mut session = EditorSession::new(original.clone(), Revision(1));
        let selection = Selection {
            anchor: point("a", 0),
            head: point("cell-a", 2),
        };
        session.set_selection(selection.clone())?;
        session.format_selection(Mark::new("bold"), MarkMode::Add)?;
        assert_ne!(session.document(), &original);
        session.undo()?;
        assert_eq!(session.document(), &original);
        assert_eq!(session.selection(), Some(&selection));
        session.redo()?;
        assert_ne!(session.document(), &original);
        Ok(())
    }
    #[test]
    fn newline_is_a_real_split_and_undo_restores_provenance() -> TestResult {
        let original = Document::plain("p", "Привет🪶")?;
        let projection = Projection::new(&original)?;
        let mut buffer = SemanticBuffer::new(original.clone(), Revision(4), projection, 0);
        buffer.insert("\nМир", 6)?;
        assert_eq!(buffer.editor.document().blocks().len(), 2);
        assert_eq!(buffer.editor.document().blocks()[0].inline_text(), "Привет");
        assert_eq!(buffer.editor.document().blocks()[1].inline_text(), "Мир🪶");
        let mut session = EditorSession::new(original.clone(), Revision(4));
        session.apply_commands(buffer.commands)?;
        session.undo()?;
        assert_eq!(session.document(), &original);
        Ok(())
    }
    #[test]
    fn headless_layout_uses_one_input_id_and_maps_every_scalar_boundary() -> TestResult {
        let document = fixture()?;
        let projection = Projection::new(&document)?;
        for span in &projection.spans {
            for offset in 0..=span.end - span.start {
                let p = point(&span.id, offset);
                assert_eq!(projection.point(projection.scalar(&p)?, true)?, p);
            }
        }
        let context = egui::Context::default();
        let mut session = EditorSession::new(document, Revision(1));
        let response = frame(&mut session, &context, vec![]);
        assert_eq!(response.response.id, session.options.id);
        assert!(response.response.rect.height() > 0.0);
        Ok(())
    }
    #[test]
    fn ime_preedit_is_display_only_until_one_atomic_commit() -> TestResult {
        let context = egui::Context::default();
        let original = Document::plain("p", "AB")?;
        let mut session = EditorSession::new(original.clone(), Revision(3));
        frame(&mut session, &context, vec![]);
        session.set_selection(Selection {
            anchor: point("p", 1),
            head: point("p", 1),
        })?;
        context.memory_mut(|memory| memory.request_focus(session.options.id));
        let interim = frame(
            &mut session,
            &context,
            vec![egui::Event::Ime(egui::ImeEvent::Preedit {
                text: "に".into(),
                active_range_chars: Some(0..1),
            })],
        );
        assert!(
            interim.errors.is_empty(),
            "IME errors: {:?}",
            interim.errors
        );
        assert!(interim.changes.is_empty());
        assert!(session.is_composing());
        assert_eq!(session.document(), &original);
        assert_eq!(session.revision(), Revision(3));
        let committed = frame(
            &mut session,
            &context,
            vec![egui::Event::Ime(egui::ImeEvent::Commit("日".into()))],
        );
        assert!(
            committed.errors.is_empty(),
            "commit errors: {:?}",
            committed.errors
        );
        assert!(!session.is_composing());
        assert_eq!(committed.changes.len(), 1);
        assert_eq!(session.document().block_text("p")?, "A日B");
        session.undo()?;
        assert_eq!(session.document(), &original);
        Ok(())
    }
    #[test]
    fn shifted_keyboard_selection_crosses_into_table_cell() -> TestResult {
        let context = egui::Context::default();
        let mut session = EditorSession::new(fixture()?, Revision(1));
        frame(&mut session, &context, vec![]);
        session.set_selection(Selection {
            anchor: point("a", 3),
            head: point("a", 3),
        })?;
        context.memory_mut(|memory| memory.request_focus(session.options.id));
        let event = egui::Event::Key {
            key: Key::ArrowRight,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::SHIFT,
        };
        let result = frame(&mut session, &context, vec![event.clone(), event]);
        assert!(result.errors.is_empty());
        assert_eq!(session.selected_text()?, "\nL");
        assert_eq!(
            session.selection().map(|s| &s.head),
            Some(&point("cell-a", 1))
        );
        Ok(())
    }
    #[test]
    fn complex_blocks_fail_explicitly_instead_of_silently_disappearing() -> TestResult {
        let document = Document::from_json(
            json!({"type":"doc","attrs":{"schemaVersion":4},"content":[
                {"type":"paragraph","attrs":{"blockId":"p"},"content":[{"type":"text","text":"Before"}]},
                {"type":"mathBlock","attrs":{"blockId":"math","latex":"x^2"}}
            ]}),
        )?;
        assert!(
            matches!(Projection::new(&document),Err(Error::UnsupportedBlockKind(kind)) if kind=="mathBlock")
        );
        Ok(())
    }
}
