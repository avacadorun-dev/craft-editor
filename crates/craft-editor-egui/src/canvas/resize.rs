//! Revision-bound native column resizing. A drag is presentation until its release transaction.
use super::*;
use crate::{SelectionBookmark, layout::ColumnGeometry};
use craft_editor_model::{NodeRef, TableGrid};

pub(super) struct ColumnDrag {
    target: NodeRef,
    revision: Revision,
    bookmark: SelectionBookmark,
    column: usize,
    initial: f32,
    pointer_x: f32,
    width: u64,
    track: GeometryRect,
    clip: Option<GeometryRect>,
    moved: bool,
}
impl CanvasState {
    fn resize_hit(&self, session: &EditorSession, local: Vec2) -> Result<Option<ColumnGeometry>> {
        let geometry = self.geometry.as_ref().ok_or(Error::InvalidSelection)?;
        let cell = geometry
            .cells()?
            .into_iter()
            .filter(|cell| {
                let rect = cell.rect;
                local.y >= rect.y
                    && local.y <= rect.y + rect.height
                    && (local.x - rect.x - rect.width).abs() <= 4.0
            })
            .max_by_key(|cell| cell.canonical_path.0.len());
        let Some(cell) = cell else {
            return Ok(None);
        };
        let Some(depth) = cell.canonical_path.0.len().checked_sub(2) else {
            return Ok(None);
        };
        let path = craft_editor_model::NodePath(cell.canonical_path.0[..depth].to_vec());
        Ok(geometry
            .columns(session.document())?
            .into_iter()
            .find(|column| {
                column.table_path == path
                    && column.clip.is_none_or(|clip| {
                        local.x >= clip.x
                            && local.x <= clip.x + clip.width
                            && local.y >= clip.y
                            && local.y <= clip.y + clip.height
                    })
                    && (column.rect.x + column.rect.width - cell.rect.x - cell.rect.width).abs()
                        <= 0.5
            }))
    }
    pub(super) fn resize_input(
        &mut self,
        session: &mut EditorSession,
        ui: &mut egui::Ui,
        response: &egui::Response,
        read_only: bool,
    ) -> Result<bool> {
        if read_only || session.is_composing() {
            return Ok(self.column_resize.take().is_some());
        }
        if self.column_resize.is_some()
            && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.column_resize = None;
            return Ok(true);
        }
        if let Some(mut drag) = self.column_resize.take() {
            if session.revision() != drag.revision {
                return Err(Error::StaleSelectionBookmark);
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            let (pointer, down, released) = ui.input(|input| {
                (
                    input.pointer.interact_pos(),
                    input.pointer.primary_down(),
                    input.pointer.button_released(egui::PointerButton::Primary),
                )
            });
            if let Some(pointer) = pointer {
                let delta = pointer.x - response.rect.left() - drag.pointer_x;
                drag.moved |= delta.abs() >= 0.5;
                drag.width = (drag.initial + delta).round().clamp(25.0, 20_000.0) as u64;
            } else {
                return Ok(true);
            }
            if released {
                if drag.moved && (drag.width as f32 - drag.initial).abs() >= 0.5 {
                    let command = craft_editor_model::set_table_column_width(
                        session.document(),
                        &drag.target,
                        drag.column,
                        drag.width,
                    )?;
                    session.restore_selection_bookmark(&drag.bookmark)?;
                    if let Some(cells) = session.cell_selection().cloned() {
                        session.apply_commands_with_cell_selection(vec![command], cells)?;
                    } else {
                        session.apply_commands(vec![command])?;
                    }
                }
            } else if down {
                self.column_resize = Some(drag);
            }
            return Ok(true);
        }
        if response.hovered()
            && let Some(pointer) = response.hover_pos()
            && let Some(column) = self.resize_hit(session, pointer - response.rect.min)?
        {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            if ui.input(|input| input.pointer.button_pressed(egui::PointerButton::Primary)) {
                let target = session.document().node_ref(&column.table_path)?;
                // Wider read-only policy belongs to the host; still apply its native edit bound here.
                let grid = TableGrid::new(&target.expected)?;
                if let Some(maximum) = session.table_column_limit()
                    && grid.columns > maximum
                {
                    return Err(Error::TableColumnLimit {
                        requested: grid.columns,
                        maximum,
                    });
                }
                self.column_resize = Some(ColumnDrag {
                    target,
                    revision: session.revision(),
                    bookmark: session.capture_selection_bookmark()?,
                    column: column.column,
                    initial: column.rect.width,
                    pointer_x: pointer.x - response.rect.left(),
                    width: column.rect.width.round().clamp(25.0, 20_000.0) as u64,
                    track: column.rect,
                    clip: column.clip,
                    moved: false,
                });
                self.cell_drag = None;
                response.request_focus();
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub(super) fn paint_resize(&self, ui: &egui::Ui, origin: Pos2, color: Color32) {
        if let Some(drag) = &self.column_resize {
            let x = origin.x + drag.track.x + drag.width as f32;
            let painter = if let Some(clip) = drag.clip {
                ui.painter()
                    .with_clip_rect(on_screen(origin, clip).intersect(ui.clip_rect()))
            } else {
                ui.painter().clone()
            };
            painter.line_segment(
                [
                    egui::pos2(x, origin.y + drag.track.y),
                    egui::pos2(x, origin.y + drag.track.y + drag.track.height),
                ],
                egui::Stroke::new(1.5, color),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_editor_model::{NodePath, ScalarOffset};
    use serde_json::json;
    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
    fn source() -> Result<Document> {
        Ok(Document::from_json(
            json!({"type":"doc","attrs":{"schemaVersion":4},"content":[
                {"type":"paragraph","attrs":{"blockId":"before"},"content":[{"type":"text","text":"Before"}]},
                {"type":"table","attrs":{"blockId":"table","vendor":{"keep":true}},"content":[
                    {"type":"tableRow","content":[
                        {"type":"tableHeader","attrs":{"colspan":2,"colwidth":[120,240]},"content":[{"type":"paragraph","attrs":{"blockId":"merged"},"content":[{"type":"text","text":"Merged"}]}]}
                    ]},
                    {"type":"tableRow","content":[
                        {"type":"tableCell","attrs":{"colwidth":[120]},"content":[{"type":"paragraph","attrs":{"blockId":"a","sourceRefs":["segment"]},"content":[{"type":"text","text":"Alpha 😀"}]}]},
                        {"type":"tableCell","attrs":{"colwidth":[240]},"content":[{"type":"paragraph","attrs":{"blockId":"b"},"content":[{"type":"text","text":"Bravo"}]}]}
                    ]}
                ]},
                {"type":"paragraph","attrs":{"blockId":"after"},"content":[{"type":"text","text":"After"}]}
            ]}),
        )?)
    }
    fn frame(
        session: &mut EditorSession,
        ctx: &egui::Context,
        events: Vec<Event>,
        style: &CanvasStyle,
    ) -> ViewResponse {
        let mut response = None;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(700.0, 650.0))),
                events,
                focused: true,
                ..Default::default()
            },
            |root| {
                egui::CentralPanel::default().show(root, |ui| {
                    response = Some(session.show_canvas(ui, style));
                });
            },
        );
        // This event harness has no GPU backend; explicitly acknowledge discarded texture work.
        output.textures_delta.clear();
        match response {
            Some(response) => response,
            None => panic!("native canvas frame"),
        }
    }
    fn down(position: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }
    fn boundary(session: &EditorSession, canvas: Rect) -> Result<Pos2> {
        let cell = session
            .canvas_state
            .geometry
            .as_ref()
            .ok_or(Error::InvalidSelection)?
            .cells()?
            .into_iter()
            .find(|cell| cell.canonical_path == NodePath(vec![1, 1, 0]))
            .ok_or(Error::InvalidSelection)?;
        Ok(egui::pos2(
            canvas.left() + cell.rect.x + cell.rect.width,
            canvas.top() + cell.rect.y + cell.rect.height * 0.5,
        ))
    }
    #[test]
    fn pointer_column_resize_commits_once_on_release_preserves_spans_and_undo_selection()
    -> TestResult {
        let original = source()?;
        let mut session = EditorSession::new(original.clone(), Revision(0));
        session.set_table_column_limit(Some(63))?;
        let selection = collapsed(DocPoint {
            block_id: "a".into(),
            offset: ScalarOffset(3),
        });
        session.set_selection(selection.clone())?;
        let ctx = egui::Context::default();
        let style = CanvasStyle::default();
        let first = frame(&mut session, &ctx, vec![], &style);
        assert!(first.errors.is_empty(), "{:?}", first.errors);
        let at = boundary(&session, first.response.rect)?;
        let pressed = frame(
            &mut session,
            &ctx,
            vec![Event::PointerMoved(at), down(at, true)],
            &style,
        );
        assert!(pressed.errors.is_empty(), "{:?}", pressed.errors);
        assert!(session.canvas_state.column_resize.is_some());
        let destination = at + egui::vec2(80.0, 0.0);
        frame(
            &mut session,
            &ctx,
            vec![Event::PointerMoved(destination)],
            &style,
        );
        assert_eq!(session.document(), &original);
        assert!(!session.can_undo());
        let released = frame(&mut session, &ctx, vec![down(destination, false)], &style);
        assert!(released.errors.is_empty(), "{:?}", released.errors);
        assert_eq!(released.changes.len(), 1);
        assert_eq!(session.revision(), Revision(1));
        let grid = TableGrid::new(session.document().node(&NodePath(vec![1]))?)?;
        assert_eq!(grid.column_widths.get(&0), Some(&200));
        assert_eq!(grid.column_widths.get(&1), Some(&240));
        assert_eq!(
            session.document().to_json()?["content"][1]["attrs"]["vendor"],
            json!({"keep":true})
        );
        assert_eq!(session.selection(), Some(&selection));
        let written = session.document().to_json()?;
        session.undo()?;
        assert_eq!(session.document(), &original);
        assert_eq!(session.selection(), Some(&selection));
        assert!(!session.can_undo());
        session.redo()?;
        assert_eq!(session.document().to_json()?, written);
        Ok(())
    }
    #[test]
    fn resize_escape_stale_source_and_read_only_never_commit_the_drag() -> TestResult {
        for mode in ["escape", "stale", "read-only"] {
            let original = source()?;
            let mut session = EditorSession::new(original.clone(), Revision(0));
            session.set_selection(collapsed(DocPoint {
                block_id: "before".into(),
                offset: ScalarOffset(0),
            }))?;
            let ctx = egui::Context::default();
            let mut style = CanvasStyle::default();
            let first = frame(&mut session, &ctx, vec![], &style);
            let at = boundary(&session, first.response.rect)?;
            if mode == "read-only" {
                style.read_only = true;
            }
            frame(
                &mut session,
                &ctx,
                vec![Event::PointerMoved(at), down(at, true)],
                &style,
            );
            if mode == "stale" {
                session.insert_plain_text("New ")?;
            }
            let before = session.document().to_json()?;
            let revision = session.revision();
            let events = if mode == "escape" {
                vec![Event::Key {
                    key: Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }]
            } else {
                vec![Event::PointerMoved(at + egui::vec2(100.0, 0.0))]
            };
            let output = frame(&mut session, &ctx, events, &style);
            if mode == "stale" {
                assert!(
                    output
                        .errors
                        .iter()
                        .any(|error| matches!(error, Error::StaleSelectionBookmark))
                );
            }
            frame(
                &mut session,
                &ctx,
                vec![down(at + egui::vec2(100.0, 0.0), false)],
                &style,
            );
            assert!(session.canvas_state.column_resize.is_none());
            assert_eq!(session.document().to_json()?, before);
            assert_eq!(session.revision(), revision);
        }
        Ok(())
    }

    #[test]
    fn a_cell_rectangle_survives_native_resize_release_and_undo() -> TestResult {
        let original = source()?;
        let mut session = EditorSession::new(original.clone(), Revision(0));
        let cells = crate::TableSelection {
            table_path: NodePath(vec![1]),
            anchor: crate::CellPoint { row: 1, column: 1 },
            head: crate::CellPoint { row: 1, column: 0 },
        };
        session.set_cell_selection(cells.clone())?;
        let ctx = egui::Context::default();
        let style = CanvasStyle::default();
        let first = frame(&mut session, &ctx, vec![], &style);
        assert!(first.errors.is_empty(), "{:?}", first.errors);
        let at = boundary(&session, first.response.rect)?;
        let pressed = frame(
            &mut session,
            &ctx,
            vec![Event::PointerMoved(at), down(at, true)],
            &style,
        );
        assert!(pressed.errors.is_empty(), "{:?}", pressed.errors);
        let destination = at + egui::vec2(60.0, 0.0);
        let released = frame(
            &mut session,
            &ctx,
            vec![Event::PointerMoved(destination), down(destination, false)],
            &style,
        );
        assert!(released.errors.is_empty(), "{:?}", released.errors);
        assert_eq!(session.revision(), Revision(1));
        assert_eq!(session.cell_selection(), Some(&cells));
        let after = session.document().to_json()?;
        session.undo()?;
        assert_eq!(session.document(), &original);
        assert_eq!(session.cell_selection(), Some(&cells));
        assert!(!session.can_undo());
        session.redo()?;
        assert_eq!(session.document().to_json()?, after);
        assert_eq!(session.cell_selection(), Some(&cells));
        Ok(())
    }
}
