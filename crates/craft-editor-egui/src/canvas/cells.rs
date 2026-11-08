//! Native rectangular table interaction over canonical paths, never transient layout indices.
use super::*;
use crate::TableSelection;
use craft_editor_model::NodePath;

impl CanvasState {
    pub(super) fn hit_cell(
        &self,
        session: &EditorSession,
        local: Vec2,
    ) -> Result<Option<TableSelection>> {
        let geometry = self.geometry.as_ref().ok_or(Error::InvalidSelection)?;
        // Nested cells occupy a subset of their parent's box; the deepest canonical path wins.
        let cell = geometry
            .cells()?
            .into_iter()
            .filter(|cell| {
                let rect = cell.rect;
                rect.page == 0
                    && local.x >= rect.x
                    && local.x <= rect.x + rect.width
                    && local.y >= rect.y
                    && local.y <= rect.y + rect.height
            })
            .max_by_key(|cell| cell.canonical_path.0.len());
        match cell {
            Some(cell) => TableSelection::from_path(session.document(), &cell.canonical_path),
            None => Ok(None),
        }
    }
    pub(super) fn cell_pointer(
        &mut self,
        session: &mut EditorSession,
        local: Vec2,
        shift: bool,
        dragging: bool,
        triple: bool,
    ) -> Result<bool> {
        let hit = self.hit_cell(session, local)?;
        if !dragging {
            self.cell_drag = hit.clone();
            if let Some(mut selected) = hit {
                if triple {
                    session.set_cell_selection(selected)?;
                    return Ok(true);
                }
                if shift {
                    let anchor = if let Some(cells) = session.cell_selection() {
                        Some(cells.clone())
                    } else if let Some(selection) = session.selection() {
                        session.text_cell(&selection.anchor)?
                    } else {
                        None
                    };
                    if let Some(anchor) = anchor
                        && anchor.table_path == selected.table_path
                        && (session.cell_selection().is_some() || anchor.anchor != selected.head)
                    {
                        selected.anchor = anchor.anchor;
                        self.cell_drag = Some(selected.clone());
                        session.set_cell_selection(selected)?;
                        return Ok(true);
                    }
                }
            }
            return Ok(false);
        }
        let Some(mut anchor) = self.cell_drag.clone() else {
            return Ok(false);
        };
        if let Some(head) = hit
            && head.table_path == anchor.table_path
        {
            anchor.head = head.head;
            if session.cell_selection().is_some() || anchor.head != anchor.anchor {
                session.set_cell_selection(anchor)?;
                return Ok(true);
            }
        } else if session.cell_selection().is_some() {
            // Once the table owns a drag, leaving it must not turn it into a cross-document range.
            return Ok(true);
        }
        Ok(false)
    }
    pub(super) fn cell_arrow(
        &mut self,
        session: &mut EditorSession,
        key: Key,
        modifiers: Modifiers,
    ) -> Result<bool> {
        if !matches!(
            key,
            Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown
        ) {
            return Ok(false);
        }
        let horizontal = matches!(key, Key::ArrowLeft | Key::ArrowRight);
        let backward = matches!(key, Key::ArrowLeft | Key::ArrowUp);
        if let Some(mut selected) = session.cell_selection().cloned() {
            if modifiers.shift {
                if let Some(head) = selected.neighbor(session.document(), horizontal, backward)? {
                    selected.head = head;
                    session.set_cell_selection(selected)?;
                }
            } else {
                let path = selected.head_path(session.document())?;
                let projection = Projection::new(session.document())?;
                let span = if backward {
                    projection
                        .spans
                        .iter()
                        .rev()
                        .find(|span| span.path < path.0)
                        .or_else(|| {
                            projection
                                .spans
                                .iter()
                                .find(|span| span.path.starts_with(&path.0))
                        })
                } else {
                    projection
                        .spans
                        .iter()
                        .find(|span| span.path.starts_with(&path.0))
                }
                .ok_or(Error::InvalidSelection)?;
                if span.atom {
                    session
                        .select_node(session.document().node_ref(&NodePath(span.path.clone()))?)?;
                } else {
                    let point = DocPoint {
                        block_id: span.id.clone(),
                        offset: ScalarOffset(if backward && span.path < path.0 {
                            span.end - span.start
                        } else {
                            0
                        }),
                    };
                    session.set_selection(collapsed(point))?;
                }
            }
            self.goal_x = None;
            self.scroll_to_caret = true;
            return Ok(true);
        }
        let Some(selection) = session.selection() else {
            return Ok(false);
        };
        let Some(mut cell) = session.text_cell(&selection.head)? else {
            return Ok(false);
        };
        if !horizontal && selection.anchor != selection.head {
            return Ok(false);
        }
        let path = cell.head_path(session.document())?;
        let projection = Projection::new(session.document())?;
        let mut spans = projection
            .spans
            .iter()
            .filter(|span| span.path.starts_with(&path.0));
        let edge = if backward {
            spans.next()
        } else {
            spans.next_back()
        }
        .ok_or(Error::InvalidSelection)?;
        if edge.atom || edge.id != selection.head.block_id {
            return Ok(false);
        }
        let geometry = self.geometry.as_ref().ok_or(Error::InvalidSelection)?;
        if horizontal {
            if selection.head.offset.0 != if backward { 0 } else { edge.end - edge.start } {
                return Ok(false);
            }
        } else if let Some((point, _)) =
            geometry.vertical(&selection.head, None, if backward { -1 } else { 1 })?
            && session
                .text_cell(&point)?
                .is_some_and(|next| next.table_path == cell.table_path && next.head == cell.head)
        {
            return Ok(false);
        }
        if modifiers.shift {
            if let Some(head) = cell.neighbor(session.document(), horizontal, backward)? {
                cell.head = head;
                session.set_cell_selection(cell)?;
                self.scroll_to_caret = true;
                return Ok(true);
            }
        } else if !horizontal
            && let Some(head) = cell.neighbor(session.document(), false, backward)?
        {
            cell.head = head;
            let target = cell.head_path(session.document())?;
            let span = projection
                .spans
                .iter()
                .find(|span| span.path.starts_with(&target.0))
                .ok_or(Error::InvalidSelection)?;
            if span.atom {
                session.select_node(session.document().node_ref(&NodePath(span.path.clone()))?)?;
            } else {
                session.set_selection(collapsed(DocPoint {
                    block_id: span.id.clone(),
                    offset: ScalarOffset(0),
                }))?;
            }
            self.scroll_to_caret = true;
            return Ok(true);
        }
        Ok(false)
    }
    pub(super) fn paint_cells(
        &self,
        session: &EditorSession,
        ui: &egui::Ui,
        origin: Pos2,
        color: Color32,
    ) -> Result<()> {
        let Some(selected) = session.cell_selection() else {
            return Ok(());
        };
        let paths = selected.origin_paths(session.document())?;
        let geometry = self.geometry.as_ref().ok_or(Error::InvalidSelection)?;
        let tint =
            Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), color.a().min(65));
        for cell in geometry.cells()? {
            if paths.contains(&cell.canonical_path) {
                ui.painter()
                    .rect_filled(on_screen(origin, cell.rect).shrink(1.0), 0.0, tint);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CellPoint;
    use craft_editor_model::Node;
    use serde_json::json;
    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
    fn fixture() -> Result<Document> {
        Ok(Document::from_json(
            json!({"type":"doc","attrs":{"schemaVersion":4},"content":[
            {"type":"paragraph","attrs":{"blockId":"before"},"content":[{"type":"text","text":"Before"}]},
            {"type":"table","attrs":{"blockId":"t"},"content":[
                {"type":"tableRow","content":[{"type":"tableHeader","content":[{"type":"paragraph","attrs":{"blockId":"a"},"content":[{"type":"text","text":"Alpha"}]}]},{"type":"tableHeader","content":[{"type":"paragraph","attrs":{"blockId":"b"},"content":[{"type":"text","text":"Bravo"}]}]}]},
                {"type":"tableRow","content":[{"type":"tableCell","content":[{"type":"paragraph","attrs":{"blockId":"c"},"content":[{"type":"text","text":"Charlie"}]}]},{"type":"tableCell","content":[{"type":"paragraph","attrs":{"blockId":"d"},"content":[{"type":"text","text":"Delta"}]}]}]}]},
            {"type":"paragraph","attrs":{"blockId":"after"},"content":[{"type":"text","text":"After"}]}]}),
        )?)
    }
    fn rectangle() -> TableSelection {
        TableSelection {
            table_path: NodePath(vec![1]),
            anchor: CellPoint { row: 0, column: 0 },
            head: CellPoint { row: 1, column: 1 },
        }
    }
    fn frame(
        session: &mut EditorSession,
        ctx: &egui::Context,
        events: Vec<Event>,
        style: &CanvasStyle,
    ) -> (ViewResponse, egui::FullOutput) {
        let modifiers = events
            .iter()
            .rev()
            .find_map(|event| match event {
                Event::Key { modifiers, .. } | Event::PointerButton { modifiers, .. } => {
                    Some(*modifiers)
                }
                _ => None,
            })
            .unwrap_or_else(|| ctx.input(|input| input.modifiers));
        let mut result = None;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(700.0, 650.0))),
                events,
                modifiers,
                focused: true,
                ..Default::default()
            },
            |root| {
                egui::CentralPanel::default().show(root, |ui| {
                    result = Some(session.show_canvas(ui, style));
                });
            },
        );
        output.textures_delta.clear();
        (result.expect("canvas frame"), output)
    }
    fn key(key: Key, modifiers: Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }
    fn focus(
        session: &mut EditorSession,
        ctx: &egui::Context,
        style: &CanvasStyle,
    ) -> Result<Rect> {
        let (response, _) = frame(session, ctx, vec![], style);
        assert!(response.errors.is_empty(), "{:?}", response.errors);
        ctx.memory_mut(|memory| memory.request_focus(session.options.id));
        Ok(response.response.rect)
    }
    fn cell_center(session: &EditorSession, canvas: Rect, path: &[usize]) -> Result<Pos2> {
        let cell = session
            .canvas_state
            .geometry
            .as_ref()
            .ok_or(Error::InvalidSelection)?
            .cells()?
            .into_iter()
            .find(|cell| !cell.covered && cell.canonical_path.0 == path)
            .ok_or(Error::InvalidSelection)?;
        Ok(on_screen(canvas.min, cell.rect).center())
    }
    #[test]
    fn compiled_canvas_drag_owns_rectangle_paints_cells_and_types_once_with_undo() -> TestResult {
        let original = fixture()?;
        let mut session = EditorSession::new(original.clone(), Revision(0));
        let ctx = egui::Context::default();
        let style = CanvasStyle::default();
        let canvas = focus(&mut session, &ctx, &style)?;
        let a = cell_center(&session, canvas, &[1, 0, 0])?;
        let d = cell_center(&session, canvas, &[1, 1, 1])?;
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let (response, _) = frame(
            &mut session,
            &ctx,
            vec![Event::PointerMoved(a), button(a, true)],
            &style,
        );
        assert!(response.errors.is_empty());
        let (response, output) = frame(&mut session, &ctx, vec![Event::PointerMoved(d)], &style);
        assert!(response.errors.is_empty(), "{:?}", response.errors);
        assert_eq!(session.cell_selection(), Some(&rectangle()));
        let tint = Color32::from_rgba_unmultiplied(
            style.selection.r(),
            style.selection.g(),
            style.selection.b(),
            style.selection.a().min(65),
        );
        assert!(output.shapes.iter().filter(|shape|matches!(&shape.shape,egui::epaint::Shape::Rect(rect) if rect.fill==tint)).count()>=4);
        frame(&mut session, &ctx, vec![button(d, false)], &style);
        let (response, _) = frame(
            &mut session,
            &ctx,
            vec![Event::Text("新しい 😀".into())],
            &style,
        );
        assert!(response.errors.is_empty(), "{:?}", response.errors);
        let written = session.document().blocks()[1]
            .children()
            .iter()
            .flat_map(Node::children)
            .flat_map(Node::children)
            .map(Node::inline_text)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>();
        assert_eq!(written, vec!["新しい 😀"]);
        assert_eq!(session.revision(), Revision(1));
        session.undo()?;
        assert_eq!(session.document(), &original);
        assert_eq!(session.cell_selection(), Some(&rectangle()));
        Ok(())
    }
    #[test]
    fn compiled_canvas_shift_click_extends_cells_and_regular_click_returns_to_text() -> TestResult {
        let mut session = EditorSession::new(fixture()?, Revision(0));
        let ctx = egui::Context::default();
        let style = CanvasStyle::default();
        let canvas = focus(&mut session, &ctx, &style)?;
        session.set_selection(collapsed(DocPoint {
            block_id: "a".into(),
            offset: ScalarOffset(2),
        }))?;
        let d = cell_center(&session, canvas, &[1, 1, 1])?;
        let button = |pressed, modifiers| Event::PointerButton {
            pos: d,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        };
        let (response, _) = frame(
            &mut session,
            &ctx,
            vec![Event::PointerMoved(d), button(true, Modifiers::SHIFT)],
            &style,
        );
        assert!(response.errors.is_empty(), "{:?}", response.errors);
        assert_eq!(session.cell_selection(), Some(&rectangle()));
        frame(
            &mut session,
            &ctx,
            vec![button(false, Modifiers::SHIFT)],
            &style,
        );
        frame(
            &mut session,
            &ctx,
            vec![button(true, Modifiers::NONE)],
            &style,
        );
        assert!(session.cell_selection().is_none());
        assert_eq!(
            session
                .selection()
                .ok_or("missing text caret")?
                .head
                .block_id,
            "d"
        );
        Ok(())
    }
    #[test]
    fn cell_arrow_canvas_commands_match_installed_source_directional_endpoints() -> TestResult {
        let corpus: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/source-cell-keys.json"))?;
        let original = fixture()?;
        for case in corpus["cases"].as_array().ok_or("missing cases")? {
            let key = case["key"].as_str().ok_or("missing key")?;
            if !key.contains("Arrow") {
                continue;
            }
            let (anchor, head) = match case["name"].as_str().ok_or("missing name")? {
                "all" => (
                    CellPoint { row: 0, column: 0 },
                    CellPoint { row: 1, column: 1 },
                ),
                "partial" => (
                    CellPoint { row: 0, column: 0 },
                    CellPoint { row: 0, column: 1 },
                ),
                "reversed" => (
                    CellPoint { row: 1, column: 1 },
                    CellPoint { row: 0, column: 0 },
                ),
                _ => (
                    CellPoint { row: 1, column: 1 },
                    CellPoint { row: 1, column: 1 },
                ),
            };
            let mut session = EditorSession::new(original.clone(), Revision(0));
            let ctx = egui::Context::default();
            let style = CanvasStyle::default();
            focus(&mut session, &ctx, &style)?;
            session.set_cell_selection(TableSelection {
                table_path: NodePath(vec![1]),
                anchor,
                head,
            })?;
            let arrow = match key.trim_start_matches("Shift-") {
                "ArrowLeft" => Key::ArrowLeft,
                "ArrowRight" => Key::ArrowRight,
                "ArrowUp" => Key::ArrowUp,
                _ => Key::ArrowDown,
            };
            let (response, _) = frame(
                &mut session,
                &ctx,
                vec![self::key(
                    arrow,
                    if key.starts_with("Shift-") {
                        Modifiers::SHIFT
                    } else {
                        Modifiers::NONE
                    },
                )],
                &style,
            );
            assert!(response.errors.is_empty(), "{case}: {:?}", response.errors);
            if case["selection"]["type"] == "text" {
                let point = &session.selection().ok_or("missing text caret")?.head;
                assert_eq!(
                    point.block_id,
                    case["from"]["blockId"]
                        .as_str()
                        .ok_or("missing source id")?,
                    "{case}"
                );
                assert_eq!(
                    point.offset.0,
                    case["from"]["offset"].as_u64().ok_or("missing offset")? as usize,
                    "{case}"
                );
            } else {
                let selected = session.cell_selection().ok_or("missing rectangle")?;
                assert_eq!(selected.anchor, anchor);
                if case["handled"] == false {
                    assert_eq!(selected.head, head);
                } else {
                    assert_ne!(selected.head, head);
                }
            }
            assert_eq!(session.document(), &original);
            assert!(!session.can_undo());
        }
        Ok(())
    }
    #[test]
    fn rectangular_ime_shadow_commit_and_read_only_commands_preserve_atomic_history() -> TestResult
    {
        for read_only in [false, true] {
            let original = fixture()?;
            let mut session = EditorSession::new(original.clone(), Revision(0));
            let ctx = egui::Context::default();
            let style = CanvasStyle {
                read_only,
                ..Default::default()
            };
            focus(&mut session, &ctx, &style)?;
            session.set_cell_selection(rectangle())?;
            let (response, _) = frame(
                &mut session,
                &ctx,
                vec![
                    Event::Ime(egui::ImeEvent::Enabled),
                    Event::Ime(egui::ImeEvent::Preedit {
                        text: "候補".into(),
                        active_range_chars: Some(0..2),
                    }),
                ],
                &style,
            );
            assert!(response.errors.is_empty(), "{:?}", response.errors);
            assert_eq!(session.document(), &original);
            assert!(!session.can_undo());
            let (response, _) = frame(
                &mut session,
                &ctx,
                vec![Event::Ime(egui::ImeEvent::Commit("確定".into()))],
                &style,
            );
            assert!(response.errors.is_empty(), "{:?}", response.errors);
            if read_only {
                let (response, _) = frame(
                    &mut session,
                    &ctx,
                    vec![
                        key(Key::Delete, Modifiers::NONE),
                        Event::Text("wrong".into()),
                    ],
                    &style,
                );
                assert!(response.errors.is_empty());
                assert_eq!(session.document(), &original);
                assert_eq!(session.cell_selection(), Some(&rectangle()));
            } else {
                assert_eq!(session.revision(), Revision(1));
                let point = &session.selection().ok_or("missing IME caret")?.head;
                assert_eq!(session.document().block_text(&point.block_id)?, "確定");
                session.undo()?;
                assert_eq!(session.document(), &original);
                assert_eq!(session.cell_selection(), Some(&rectangle()));
            }
        }
        Ok(())
    }
}
