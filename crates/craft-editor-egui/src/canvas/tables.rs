//! Native local scrolling over the shared table viewport projection.
use super::*;

impl CanvasState {
    pub(super) fn table_scroll_input(&mut self, ui: &mut egui::Ui, canvas: Rect) -> Result<bool> {
        let pointer = ui
            .input(|input| input.pointer.interact_pos())
            .filter(|_| ui.rect_contains_pointer(canvas));
        let Some(geometry) = self.geometry.as_mut() else {
            return Ok(false);
        };
        if self.table_scroll_pending {
            geometry.restore_table_offsets(&self.table_offsets);
            self.table_scroll_pending = false;
            self.tiles.clear();
        }
        let viewports = geometry.table_viewports();
        let bar = pointer.is_some_and(|pointer| {
            viewports
                .iter()
                .any(|viewport| on_screen(canvas.min, viewport.scrollbar).contains(pointer))
        });
        if self.column_resize.is_none()
            && let Some(pointer) = pointer
            && let Some(viewport) = viewports
                .iter()
                .filter(|viewport| {
                    on_screen(canvas.min, viewport.rect).contains(pointer)
                        || on_screen(canvas.min, viewport.scrollbar).contains(pointer)
                })
                .max_by_key(|viewport| viewport.table_path.0.len())
        {
            let delta = ui.input_mut(|input| {
                if input.smooth_scroll_delta.x != 0.0 {
                    std::mem::take(&mut input.smooth_scroll_delta.x)
                } else if input.modifiers.shift {
                    std::mem::take(&mut input.smooth_scroll_delta.y)
                } else {
                    0.0
                }
            });
            if delta != 0.0
                && geometry.scroll_table(&viewport.table_path, viewport.offset - delta)?
            {
                self.table_offsets = geometry.table_offsets();
                self.tiles.clear();
                ui.ctx().request_repaint();
            }
        }
        Ok(bar || self.table_bar_dragging)
    }
    pub(super) fn table_scrollbars(
        &mut self,
        ui: &mut egui::Ui,
        canvas: Rect,
        style: &CanvasStyle,
    ) -> Result<()> {
        let Some(geometry) = self.geometry.as_mut() else {
            return Ok(());
        };
        let viewports = geometry.table_viewports();
        self.table_bar_dragging = false;
        for viewport in viewports {
            let rect = on_screen(canvas.min, viewport.scrollbar);
            if !rect.intersects(ui.clip_rect()) {
                continue;
            }
            let mut offset = viewport.offset;
            let mut dragged = false;
            ui.push_id(("table-scroll", &viewport.identity), |ui| {
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    ui.spacing_mut().slider_width = rect.width();
                    let thumb =
                        (viewport.rect.width / viewport.content_width * viewport.rect.width / 16.0)
                            .max(1.0);
                    let slider = egui::Slider::new(&mut offset, 0.0..=viewport.maximum)
                        .show_value(false)
                        .step_by(1.0)
                        .trailing_fill(false)
                        .handle_shape(egui::style::HandleShape::Rect {
                            aspect_ratio: thumb,
                        });
                    let response = ui.add_enabled(self.column_resize.is_none(), slider);
                    if response.drag_started() || response.is_pointer_button_down_on() {
                        response.request_focus();
                    }
                    response.widget_info(|| {
                        egui::WidgetInfo::slider(
                            response.enabled(),
                            f64::from(offset),
                            &style.table_scroll_label,
                        )
                    });
                    dragged = response.dragged();
                    response.on_hover_text(&style.table_scroll_label);
                });
            });
            self.table_bar_dragging |= dragged;
            if (offset - viewport.offset).abs() > 0.01 {
                // The table pixels and semantic overlays for this frame are already painted.
                // Publish the control's target on the next frame, keeping one coherent projection.
                self.table_offsets.insert(viewport.identity, offset);
                self.table_scroll_pending = true;
                ui.ctx().request_repaint();
            }
        }
        Ok(())
    }
    pub(super) fn reveal_table_target(&mut self, session: &EditorSession) -> Result<()> {
        if !self.scroll_to_caret {
            return Ok(());
        }
        let cell_point = if session.cell_selection().is_some() {
            session.head_cell_text_point(false)?
        } else {
            None
        };
        let point = self
            .preedit
            .as_ref()
            .map(|preedit| &preedit.caret)
            .or_else(|| session.selection().map(|selection| &selection.head))
            .or(cell_point.as_ref());
        let Some(geometry) = self.geometry.as_mut() else {
            return Ok(());
        };
        let changed = if let Some(point) = point {
            geometry.reveal_caret(point)?
        } else if let Some(node) = session.node_selection() {
            geometry.reveal_node(&node.path)
        } else {
            false
        };
        if changed {
            self.table_offsets = geometry.table_offsets();
            self.tiles.clear();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_editor_model::{NodePath, TableGrid};
    use serde_json::json;
    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
    fn source() -> Result<Document> {
        let cells:Vec<_>=(0..63).map(|column|json!({"type":"tableCell","attrs":{"colwidth":[80]},"content":[
            {"type":"paragraph","attrs":{"blockId":format!("cell-{column}")},"content":[{"type":"text","text":format!("Cell {column} 😀")}]}
        ]})).collect();
        Ok(Document::from_json(
            json!({"type":"doc","attrs":{"schemaVersion":4},"content":[
                {"type":"paragraph","attrs":{"blockId":"before"},"content":[{"type":"text","text":"Before"}]},
                {"type":"table","attrs":{"blockId":"wide"},"content":[{"type":"tableRow","content":cells}]},
                {"type":"paragraph","attrs":{"blockId":"after"},"content":[{"type":"text","text":"After"}]}
            ]}),
        )?)
    }
    fn frame(
        session: &mut EditorSession,
        ctx: &egui::Context,
        events: Vec<Event>,
    ) -> (ViewResponse, egui::FullOutput) {
        frame_with_clip(session, ctx, events, None)
    }
    fn frame_with_clip(
        session: &mut EditorSession,
        ctx: &egui::Context,
        events: Vec<Event>,
        clip: Option<Rect>,
    ) -> (ViewResponse, egui::FullOutput) {
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
                    if let Some(clip) = clip {
                        ui.set_clip_rect(clip.intersect(ui.clip_rect()));
                    }
                    response = Some(session.show_canvas(ui, &CanvasStyle::default()));
                });
            },
        );
        output.textures_delta.clear();
        let response = match response {
            Some(response) => response,
            None => panic!("native table frame"),
        };
        (response, output)
    }
    fn press(position: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }
    #[test]
    fn native_horizontal_wheel_and_scrollbar_keyboard_leave_canonical_writing_and_selection_unchanged()
    -> TestResult {
        let original = source()?;
        let mut session = EditorSession::new(original.clone(), Revision(0));
        let ctx = egui::Context::default();
        let (first, _) = frame(&mut session, &ctx, vec![]);
        assert!(first.errors.is_empty(), "{:?}", first.errors);
        let selection = session.selection_state()?;
        let viewport = session
            .canvas_state
            .geometry
            .as_ref()
            .ok_or(Error::InvalidSelection)?
            .table_viewports()
            .remove(0);
        let at = on_screen(first.response.rect.min, viewport.rect).center();
        let (hidden, _) = frame_with_clip(
            &mut session,
            &ctx,
            vec![
                Event::PointerMoved(at),
                Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: Vec2::new(-150.0, 0.0),
                    modifiers: Modifiers::NONE,
                },
            ],
            Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(1.0))),
        );
        assert!(hidden.errors.is_empty(), "{:?}", hidden.errors);
        assert_eq!(
            session
                .canvas_state
                .geometry
                .as_ref()
                .ok_or(Error::InvalidSelection)?
                .table_viewports()[0]
                .offset,
            0.0
        );
        let (wheel, _) = frame(
            &mut session,
            &ctx,
            vec![
                Event::PointerMoved(at),
                Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: Vec2::new(-150.0, 0.0),
                    modifiers: Modifiers::NONE,
                },
            ],
        );
        assert!(wheel.errors.is_empty(), "{:?}", wheel.errors);
        let viewport = session
            .canvas_state
            .geometry
            .as_ref()
            .ok_or(Error::InvalidSelection)?
            .table_viewports()
            .remove(0);
        assert!(viewport.offset > 0.0);
        // Drain wheel smoothing before testing the scrollbar's independent keyboard focus.
        for _ in 0..64 {
            frame(&mut session, &ctx, vec![]);
        }
        let bar = on_screen(first.response.rect.min, viewport.scrollbar).center();
        frame(
            &mut session,
            &ctx,
            vec![Event::PointerMoved(bar), press(bar, true)],
        );
        frame(&mut session, &ctx, vec![press(bar, false)]);
        frame(&mut session, &ctx, vec![]);
        let prior = session
            .canvas_state
            .geometry
            .as_ref()
            .ok_or(Error::InvalidSelection)?
            .table_viewports()[0]
            .offset;
        frame(
            &mut session,
            &ctx,
            vec![Event::Key {
                key: Key::ArrowRight,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
        );
        frame(&mut session, &ctx, vec![]);
        let current = session
            .canvas_state
            .geometry
            .as_ref()
            .ok_or(Error::InvalidSelection)?
            .table_viewports()[0]
            .offset;
        assert!(
            current > prior,
            "scrollbar keyboard: {current} after {prior}"
        );
        assert_eq!(session.selection_state()?, selection);
        assert_eq!(session.document(), &original);
        assert_eq!(session.revision(), Revision(0));
        assert!(!session.can_undo());
        Ok(())
    }
    #[test]
    fn last_column_reveal_pointer_ime_and_resize_share_one_visible_semantic_target() -> TestResult {
        let original = source()?;
        let mut session = EditorSession::new(original.clone(), Revision(0));
        session.set_table_column_limit(Some(63))?;
        let point = DocPoint {
            block_id: "cell-62".into(),
            offset: ScalarOffset(0),
        };
        session.set_selection(collapsed(point.clone()))?;
        session.request_scroll_to_caret();
        let ctx = egui::Context::default();
        ctx.memory_mut(|memory| memory.request_focus(session.options.id));
        let (first, _) = frame(&mut session, &ctx, vec![]);
        assert!(first.errors.is_empty(), "{:?}", first.errors);
        let geometry = session
            .canvas_state
            .geometry
            .as_ref()
            .ok_or(Error::InvalidSelection)?;
        assert!(geometry.table_viewports()[0].offset > 0.0);
        let caret = geometry.caret(&point)?.ok_or(Error::InvalidSelection)?;
        let at = on_screen(first.response.rect.min, caret).center() + Vec2::new(0.1, 0.0);
        frame(
            &mut session,
            &ctx,
            vec![Event::PointerMoved(at), press(at, true)],
        );
        frame(&mut session, &ctx, vec![press(at, false)]);
        assert!(ctx.memory(|memory| memory.has_focus(session.options.id)));
        assert_eq!(
            session
                .selection()
                .map(|selection| selection.head.block_id.as_str()),
            Some("cell-62")
        );
        let (preedit, output) = frame(
            &mut session,
            &ctx,
            vec![Event::Ime(egui::ImeEvent::Preedit {
                text: "界".into(),
                active_range_chars: Some(0..1),
            })],
        );
        assert!(preedit.errors.is_empty(), "{:?}", preedit.errors);
        assert!(session.is_composing());
        assert_eq!(session.document(), &original);
        let ime = output
            .platform_output
            .ime
            .as_ref()
            .ok_or("missing native IME geometry")?;
        let viewport = session
            .canvas_state
            .geometry
            .as_ref()
            .ok_or(Error::InvalidSelection)?
            .table_viewports()
            .remove(0);
        let visible = on_screen(first.response.rect.min, viewport.rect);
        assert!(visible.contains(ime.cursor_rect.center()));
        let (commit, _) = frame(
            &mut session,
            &ctx,
            vec![Event::Ime(egui::ImeEvent::Commit("界".into()))],
        );
        assert!(commit.errors.is_empty(), "{:?}", commit.errors);
        assert!(session.document().block_text("cell-62")?.contains('界'));
        session.undo()?;
        assert_eq!(session.document(), &original);
        let cells = crate::TableSelection {
            table_path: NodePath(vec![1]),
            anchor: crate::CellPoint { row: 0, column: 62 },
            head: crate::CellPoint { row: 0, column: 61 },
        };
        session.set_cell_selection(cells.clone())?;
        frame(&mut session, &ctx, vec![]);
        // A caret reveal exposes its exact position. Scroll to the far end to expose the
        // last track's right border, then resize through the real pointer route.
        let viewport = session
            .canvas_state
            .geometry
            .as_ref()
            .ok_or(Error::InvalidSelection)?
            .table_viewports()
            .remove(0);
        let table_center = on_screen(first.response.rect.min, viewport.rect).center();
        frame(
            &mut session,
            &ctx,
            vec![
                Event::PointerMoved(table_center),
                Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: Vec2::new(-100_000.0, 0.0),
                    modifiers: Modifiers::NONE,
                },
            ],
        );
        let geometry = session
            .canvas_state
            .geometry
            .as_ref()
            .ok_or(Error::InvalidSelection)?;
        let column = geometry
            .columns(session.document())?
            .into_iter()
            .find(|column| column.column == 62)
            .ok_or("missing final column")?;
        let edge = egui::pos2(
            first.response.rect.left() + column.rect.x + column.rect.width,
            first.response.rect.top() + column.rect.y + column.rect.height * 0.5,
        );
        frame(
            &mut session,
            &ctx,
            vec![Event::PointerMoved(edge), press(edge, true)],
        );
        let destination = edge + Vec2::new(40.0, 0.0);
        let (resize, _) = frame(
            &mut session,
            &ctx,
            vec![Event::PointerMoved(destination), press(destination, false)],
        );
        assert!(resize.errors.is_empty(), "{:?}", resize.errors);
        let grid = TableGrid::new(session.document().node(&NodePath(vec![1]))?)?;
        assert_eq!(grid.column_widths.get(&62), Some(&120));
        assert_eq!(session.cell_selection(), Some(&cells));
        session.undo()?;
        assert_eq!(session.document(), &original);
        assert_eq!(session.cell_selection(), Some(&cells));
        Ok(())
    }
}
