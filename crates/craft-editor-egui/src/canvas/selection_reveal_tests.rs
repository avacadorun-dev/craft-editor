//! Measured native scrolling regressions; hosted widgets do not establish OS UX acceptance.
use super::*;
use craft_editor_model::NodePath;
use egui::{RawInput, TextEdit};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn point(block: &str, offset: usize) -> DocPoint {
    DocPoint {
        block_id: block.into(),
        offset: ScalarOffset(offset),
    }
}

fn document() -> craft_editor_model::Result<Document> {
    let paragraphs: Vec<_> = (0..40)
        .map(|index| {
            serde_json::json!({
                "type":"paragraph","attrs":{"blockId":format!("p{index}")},
                "content":[{"type":"text","text":"A signal waits."}]
            })
        })
        .collect();
    Document::from_json(
        serde_json::json!({"type":"doc","attrs":{"schemaVersion":1},"content":paragraphs}),
    )
}

struct Frame {
    response: egui::Response,
    viewport: Rect,
    selected: Option<Rect>,
    offset: f32,
    host: egui::Response,
}

struct Harness {
    session: EditorSession,
    ctx: egui::Context,
    time: f64,
    host_text: String,
}

impl Harness {
    fn new(document: Document) -> Self {
        Self {
            session: EditorSession::new(document, Revision(1)),
            ctx: egui::Context::default(),
            time: 0.0,
            host_text: "Host toolbar field".into(),
        }
    }

    fn frame(
        &mut self,
        width: f32,
        height: f32,
        offset: Option<f32>,
        events: Vec<Event>,
    ) -> TestResult<Frame> {
        self.time += 0.1;
        let ctx = self.ctx.clone();
        let mut result = None;
        let mut output = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, height))),
                time: Some(self.time),
                focused: true,
                events,
                ..Default::default()
            },
            |root| {
                egui::CentralPanel::default().show(root, |ui| {
                    let host = ui.add(
                        TextEdit::singleline(&mut self.host_text).id_salt("host-toolbar-field"),
                    );
                    ui.separator();
                    ui.add_space(8.0);
                    let mut area = egui::ScrollArea::vertical()
                        .id_salt("selection-reveal-scroll")
                        .auto_shrink([false, false]);
                    if let Some(offset) = offset {
                        area = area.vertical_scroll_offset(offset);
                    }
                    let scrolled = area.show(ui, |ui| -> Result<_> {
                        let view = self.session.show_canvas(ui, &CanvasStyle::default());
                        if let Some(error) = view.errors.into_iter().next() {
                            return Err(error);
                        }
                        assert!(
                            view.changes.is_empty(),
                            "presentation must not emit model changes"
                        );
                        let selected = self.session.canvas_selection_rect(&view.response)?;
                        let viewport = view.response.rect.intersect(ui.clip_rect());
                        Ok((view.response, viewport, selected))
                    });
                    result = Some(scrolled.inner.map(|(response, viewport, selected)| Frame {
                        response,
                        viewport,
                        selected,
                        offset: scrolled.state.offset.y,
                        host,
                    }));
                });
            },
        );
        output.textures_delta.clear();
        output.platform_output.commands.clear();
        Ok(result.ok_or("canvas frame did not run")??)
    }

    fn settle(&mut self, width: f32, height: f32) -> TestResult<Frame> {
        let mut previous = None;
        let mut stable = 0;
        for _ in 0..16 {
            let frame = self.frame(width, height, None, vec![])?;
            if previous == Some(frame.offset) {
                stable += 1;
                if stable == 3 {
                    return Ok(frame);
                }
            } else {
                stable = 0;
            }
            previous = Some(frame.offset);
        }
        Err("native scroll did not settle within the frame budget".into())
    }
}

#[test]
fn forward_and_reverse_two_line_reveal_fits_below_toolbar_without_changing_author_state()
-> TestResult {
    for reverse in [false, true] {
        let original = document()?;
        let mut host = Harness::new(original.clone());
        host.session.set_selection(collapsed(point("p0", 0)))?;
        host.session.insert_plain_text("Preface. ")?;
        host.session.take_pending_changes();
        let before = host.session.document().clone();
        let revision = host.session.revision();
        let selection = if reverse {
            Selection {
                anchor: point("p21", 15),
                head: point("p20", 0),
            }
        } else {
            Selection {
                anchor: point("p20", 0),
                head: point("p21", 15),
            }
        };
        host.session.set_selection(selection.clone())?;
        host.session.cancel_scroll_request();
        let initial = host.frame(480.0, 280.0, None, vec![])?;
        // Start below the measured range. Revealing only the forward head would put its
        // second line at the top and leave the first line clipped above the viewport.
        let selected = initial.selected.ok_or("initial measured range")?;
        let below_offset =
            selected.bottom() - initial.response.rect.top() + initial.viewport.height() * 0.5;
        host.frame(480.0, 280.0, Some(below_offset), vec![])?;
        let below = host.settle(480.0, 280.0)?;
        assert!(below.selected.ok_or("offscreen range")?.bottom() < below.viewport.top());
        host.ctx
            .memory_mut(|memory| memory.request_focus(initial.host.id));
        host.session.request_scroll_to_selection();
        let shown = host.settle(480.0, 280.0)?;
        let selected = shown.selected.ok_or("measured selected range")?;
        let lines = host
            .session
            .canvas_state
            .geometry
            .as_ref()
            .ok_or("measured geometry")?
            .selection_rects(&selection)?;
        assert_eq!(
            lines.len(),
            2,
            "fixture must cover two actual measured lines"
        );
        assert!(shown.viewport.top() > shown.host.rect.bottom());
        assert!(
            shown.viewport.contains_rect(selected),
            "{selected:?} not in {:?}",
            shown.viewport
        );
        assert_eq!(host.session.selection(), Some(&selection));
        assert_eq!(host.session.document(), &before);
        assert_eq!(host.session.revision(), revision);
        assert!(host.session.can_undo());
        assert_eq!(
            host.ctx.memory(|memory| memory.focused()),
            Some(initial.host.id)
        );

        host.frame(
            480.0,
            280.0,
            None,
            vec![
                Event::PointerMoved(shown.viewport.center()),
                Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: Vec2::new(0.0, 160.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: Modifiers::NONE,
                },
            ],
        )?;
        let manually_scrolled = host.settle(480.0, 280.0)?;
        assert!(
            manually_scrolled.offset < shown.offset,
            "a consumed reveal must not snap a later wheel scroll back"
        );
        assert_eq!(host.session.selection(), Some(&selection));
        host.session.undo()?;
        assert_eq!(
            host.session.document(),
            &original,
            "presentation must retain the actual model Undo entry"
        );
    }
    Ok(())
}

#[test]
fn explicit_reveal_after_width_reflow_uses_the_new_measured_range() -> TestResult {
    let mut value = document()?.to_json()?;
    let text = "Nina finds the missing signal log while Asya waits beside the lighthouse door.";
    value["content"][20]["content"][0]["text"] = serde_json::json!(text);
    let source = Document::from_json(value)?;
    let mut host = Harness::new(source.clone());
    let selection = Selection {
        anchor: point("p20", 0),
        head: point("p20", text.chars().count()),
    };
    host.session.set_selection(selection.clone())?;
    host.session.request_scroll_to_selection();
    let wide = host.settle(680.0, 330.0)?;
    host.session.request_scroll_to_selection();
    let narrow = host.settle(280.0, 330.0)?;
    let selected = narrow.selected.ok_or("reflowed selection")?;
    assert!(selected.height() > wide.selected.ok_or("wide selection")?.height());
    assert!(
        narrow.viewport.contains_rect(selected),
        "reflowed start and end must both remain visible"
    );
    assert_eq!(host.session.selection(), Some(&selection));
    assert_eq!(host.session.document(), &source);
    assert!(!host.session.can_undo());
    Ok(())
}

#[test]
fn oversized_forward_and_reverse_ranges_reveal_the_same_normalized_start() -> TestResult {
    let mut offsets = Vec::new();
    for reverse in [false, true] {
        let mut host = Harness::new(document()?);
        let selection = if reverse {
            Selection {
                anchor: point("p30", 15),
                head: point("p10", 0),
            }
        } else {
            Selection {
                anchor: point("p10", 0),
                head: point("p30", 15),
            }
        };
        host.session.set_selection(selection.clone())?;
        host.session.request_scroll_to_selection();
        let frame = host.settle(480.0, 230.0)?;
        assert!(frame.selected.ok_or("large selection")?.height() > frame.viewport.height());
        let start = host
            .session
            .canvas_state
            .geometry
            .as_ref()
            .ok_or("measured geometry")?
            .caret(&point("p10", 0))?
            .ok_or("normalized start caret")?;
        assert!(
            frame
                .viewport
                .contains_rect(on_screen(frame.response.rect.min, start))
        );
        assert_eq!(host.session.selection(), Some(&selection));
        offsets.push(frame.offset);
    }
    assert_eq!(offsets[0], offsets[1]);
    Ok(())
}

#[test]
fn ordinary_caret_navigation_supersedes_an_explicit_range_request() -> TestResult {
    let source = document()?;
    let mut host = Harness::new(source.clone());
    host.session.set_selection(Selection {
        anchor: point("p10", 0),
        head: point("p30", 14),
    })?;
    host.session.request_scroll_to_selection();
    host.settle(480.0, 230.0)?;
    host.session.request_scroll_to_selection();
    host.session.request_canvas_focus(&host.ctx);
    host.frame(
        480.0,
        230.0,
        None,
        vec![Event::Key {
            key: Key::ArrowRight,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::SHIFT,
        }],
    )?;
    let frame = host.settle(480.0, 230.0)?;
    let caret = host
        .session
        .canvas_caret_rect(&frame.response)?
        .ok_or("navigation caret")?;
    assert!(frame.viewport.contains_rect(caret));
    assert!(frame.selected.ok_or("range after navigation")?.top() < frame.viewport.top());
    assert_eq!(host.session.document(), &source);
    assert!(!host.session.can_undo());
    Ok(())
}

#[test]
fn measured_node_reveal_and_cancellation_preserve_an_explicit_host_return_offset() -> TestResult {
    let mut value = document()?.to_json()?;
    value["content"][20] = serde_json::json!({"type":"horizontalRule"});
    let source = Document::from_json(value)?;
    let mut host = Harness::new(source.clone());
    let node = host.session.document().node_ref(&NodePath(vec![20]))?;
    host.session.select_node(node.clone())?;
    host.session.request_scroll_to_selection();
    let shown = host.settle(480.0, 280.0)?;
    assert!(
        shown
            .viewport
            .contains_rect(shown.selected.ok_or("measured node")?)
    );
    host.session.request_scroll_to_caret();
    host.session.request_scroll_to_selection();
    assert!(!host.session.canvas_state.scroll_to_caret);
    host.session.cancel_scroll_request();
    assert!(
        !host.session.canvas_state.scroll_to_caret
            && !host.session.canvas_state.scroll_to_selection
    );
    // The host owns its already-submitted animations; here the previous reveal is settled.
    host.frame(480.0, 280.0, Some(0.0), vec![])?;
    let returned = host.settle(480.0, 280.0)?;
    assert_eq!(returned.offset, 0.0);
    assert!(
        !returned
            .viewport
            .intersects(returned.selected.ok_or("offscreen node")?)
    );
    assert_eq!(host.session.node_selection(), Some(&node));
    assert_eq!(host.session.document(), &source);
    Ok(())
}

#[test]
fn stale_geometry_is_not_exposed_and_failed_measurement_consumes_the_request() -> TestResult {
    let mut host = Harness::new(document()?);
    host.session.set_selection(Selection {
        anchor: point("p0", 0),
        head: point("p0", 8),
    })?;
    let frame = host.frame(480.0, 280.0, None, vec![])?;
    assert!(
        host.session
            .canvas_selection_rect(&frame.response)?
            .is_some()
    );
    // A new shadow epoch with no successful remeasurement must not expose the old geometry,
    // even though native composition has not committed a document revision.
    host.session.canvas_state.preedit_epoch =
        host.session.canvas_state.preedit_epoch.wrapping_add(1);
    assert!(
        host.session
            .canvas_selection_rect(&frame.response)?
            .is_none()
    );
    let frame = host.frame(480.0, 280.0, None, vec![])?;
    assert!(
        host.session
            .canvas_selection_rect(&frame.response)?
            .is_some()
    );
    host.session.insert_plain_text("Changed")?;
    host.session.take_pending_changes();
    assert!(
        host.session
            .canvas_selection_rect(&frame.response)?
            .is_none()
    );
    host.session.request_scroll_to_selection();
    let content = host.session.document().clone();
    let selection = host.session.selection_state()?;
    let mut errors = Vec::new();
    let mut output = host.ctx.run_ui(RawInput::default(), |ui| {
        let response = host.session.show_canvas(
            ui,
            &CanvasStyle {
                font_size: f32::NAN,
                ..Default::default()
            },
        );
        errors = response.errors;
    });
    output.textures_delta.clear();
    assert!(!errors.is_empty());
    assert!(!host.session.canvas_state.scroll_to_selection);
    assert_eq!(host.session.document(), &content);
    assert_eq!(host.session.selection_state()?, selection);
    host.session.restore_selection_state(None)?;
    assert!(
        host.session
            .canvas_state
            .selection_rect(&host.session, Pos2::ZERO)?
            .is_none()
    );
    host.session.request_scroll_to_selection();
    host.frame(480.0, 280.0, None, vec![])?;
    assert!(!host.session.canvas_state.scroll_to_selection);
    Ok(())
}
