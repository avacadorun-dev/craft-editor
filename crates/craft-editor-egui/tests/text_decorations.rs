#![cfg(feature = "wordcraft-canvas")]
#![expect(
    clippy::expect_used,
    reason = "A synthetic native frame must emit a view response"
)]
use craft_editor_egui::{
    DocPoint, EditorSession, Selection, ViewResponse,
    canvas::CanvasStyle,
    decorations::{DecorationSource, TextDecoration, TextUnderline},
    layout::{FlowGeometry, GeometryStyle},
};
use craft_editor_model::{Document, ReplayProof, Revision, ScalarOffset};
use egui::{Color32, Event, Pos2, Rect, Vec2};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
const NOTE: Color32 = Color32::from_rgb(211, 143, 31);
fn point(offset: usize) -> DocPoint {
    DocPoint {
        block_id: "p".into(),
        offset: ScalarOffset(offset),
    }
}
struct Source {
    base: Document,
    start: ScalarOffset,
    end: ScalarOffset,
    calls: usize,
    seen_pending: usize,
    fail: bool,
}
impl DecorationSource for Source {
    fn decorations(
        &mut self,
        document: &Document,
        _: Revision,
        pending: &[ReplayProof],
    ) -> craft_editor_egui::Result<Vec<TextDecoration>> {
        self.calls += 1;
        self.seen_pending = pending.len();
        if self.fail {
            return Err(craft_editor_egui::Error::DecorationSource);
        }
        let mut tree = self.base.clone();
        let (mut start, mut end) = (self.start, self.end);
        for proof in pending {
            let (updated, maps) = proof.verify(&tree)?;
            for map in maps {
                let a = map
                    .map_existing_caret("p", start, true)
                    .ok_or(craft_editor_egui::Error::InvalidDecoration)?;
                let b = map
                    .map_existing_caret("p", end, false)
                    .ok_or(craft_editor_egui::Error::InvalidDecoration)?;
                if a.0 != "p" || b.0 != "p" {
                    return Err(craft_editor_egui::Error::InvalidDecoration);
                }
                start = a.1;
                end = b.1;
            }
            tree = updated;
        }
        if &tree != document {
            return Err(craft_editor_egui::Error::DecorationSource);
        }
        Ok(vec![TextDecoration {
            id: "author-note".into(),
            range: Selection {
                anchor: point(start.0),
                head: point(end.0),
            },
            color: NOTE,
            label: "Synthetic author note".into(),
        }])
    }
}
fn frame(
    session: &mut EditorSession,
    ctx: &egui::Context,
    source: &mut impl DecorationSource,
    events: Vec<Event>,
) -> (ViewResponse, egui::FullOutput) {
    let mut result = None;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(720.0, 520.0))),
            events,
            focused: true,
            ..Default::default()
        },
        |root| {
            egui::CentralPanel::default().show(root, |ui| {
                result = Some(session.show_canvas_with_sources(
                    ui,
                    &CanvasStyle::default(),
                    None,
                    Some(source),
                ));
            });
        },
    );
    output.textures_delta.clear();
    (result.expect("frame emits the native view"), output)
}
fn fixture() -> TestResult<(EditorSession, Source, egui::Context)> {
    let tree = Document::plain("p", "Я😀 echo echo")?;
    Ok((
        EditorSession::new(tree.clone(), Revision(0)),
        Source {
            base: tree,
            start: ScalarOffset(8),
            end: ScalarOffset(12),
            calls: 0,
            seen_pending: 0,
            fail: false,
        },
        egui::Context::default(),
    ))
}
fn has_note_ink(output: &egui::FullOutput) -> bool {
    let tint = Color32::from_rgba_unmultiplied(NOTE.r(), NOTE.g(), NOTE.b(), 65);
    output
        .shapes
        .iter()
        .any(|shape| matches!(&shape.shape,egui::Shape::Rect(rect) if rect.fill==tint))
}

#[test]
fn persistent_host_decoration_remains_painted_after_the_caret_moves() -> TestResult {
    let (mut session, mut source, ctx) = fixture()?;
    let original = session.document().clone();
    session.set_selection(Selection {
        anchor: point(8),
        head: point(12),
    })?;
    let (first, painted) = frame(&mut session, &ctx, &mut source, vec![]);
    assert!(first.errors.is_empty(), "{:?}", first.errors);
    assert!(has_note_ink(&painted), "persistent note tint missing");
    session.set_selection(Selection {
        anchor: point(0),
        head: point(0),
    })?;
    let (second, painted) = frame(&mut session, &ctx, &mut source, vec![]);
    assert!(second.errors.is_empty(), "{:?}", second.errors);
    assert!(has_note_ink(&painted), "note tint missing after caret move");
    assert_eq!(session.document(), &original);
    assert!(second.changes.is_empty());
    Ok(())
}

#[test]
fn source_receives_new_input_proofs_before_same_frame_decoration_paint() -> TestResult {
    let (mut session, mut source, ctx) = fixture()?;
    let (first, _) = frame(&mut session, &ctx, &mut source, vec![]);
    session.set_selection(Selection {
        anchor: point(0),
        head: point(0),
    })?;
    first.response.request_focus();
    let (output, painted) = frame(
        &mut session,
        &ctx,
        &mut source,
        vec![Event::Text("New ".into())],
    );
    assert!(output.errors.is_empty(), "{:?}", output.errors);
    assert_eq!(source.seen_pending, 1);
    assert_eq!(session.document().block_text("p")?, "New Я😀 echo echo");
    assert!(has_note_ink(&painted));
    Ok(())
}

#[test]
fn decoration_source_failure_does_not_block_unrelated_native_writing() -> TestResult {
    let (mut session, mut source, ctx) = fixture()?;
    let (first, _) = frame(&mut session, &ctx, &mut source, vec![]);
    session.set_selection(Selection {
        anchor: point(0),
        head: point(0),
    })?;
    first.response.request_focus();
    source.fail = true;
    let (output, painted) = frame(
        &mut session,
        &ctx,
        &mut source,
        vec![Event::Text("X".into())],
    );
    assert!(
        output
            .errors
            .iter()
            .any(|error| matches!(error, craft_editor_egui::Error::DecorationSource))
    );
    assert_eq!(session.document().block_text("p")?, "XЯ😀 echo echo");
    assert!(!has_note_ink(&painted));
    Ok(())
}

#[test]
fn clicking_a_persistent_mark_returns_opaque_identity_without_mutating_prose() -> TestResult {
    let (mut session, mut source, ctx) = fixture()?;
    let (original, _) = frame(&mut session, &ctx, &mut source, vec![]);
    let style = CanvasStyle::default();
    let geometry = FlowGeometry::with_style(
        session.document(),
        original.response.rect.width(),
        &|_| None,
        Some(&GeometryStyle {
            font_size: style.font_size,
            font_family: style.font_family,
            line_height: style.line_height,
            paragraph_spacing: style.paragraph_spacing,
            ..Default::default()
        }),
    )?;
    let marked = geometry.selection_rects(&Selection {
        anchor: point(8),
        head: point(12),
    })?;
    let range = marked.first().ok_or("missing marked glyph bounds")?;
    let position = original.response.rect.min
        + Vec2::new(range.x + range.width * 0.5, range.y + range.height * 0.5);
    frame(
        &mut session,
        &ctx,
        &mut source,
        vec![
            Event::PointerMoved(position),
            Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    let (output, _) = frame(
        &mut session,
        &ctx,
        &mut source,
        vec![
            Event::PointerMoved(position),
            Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    assert_eq!(output.activated_decorations, vec!["author-note"]);
    assert_eq!(session.document().block_text("p")?, "Я😀 echo echo");
    assert!(output.changes.is_empty());
    Ok(())
}
#[test]
fn ime_shadow_keeps_persistent_marks_visible_without_changing_canonical_authority() -> TestResult {
    let (mut session, mut source, ctx) = fixture()?;
    let original = session.document().clone();
    let (first, _) = frame(&mut session, &ctx, &mut source, vec![]);
    session.set_selection(Selection {
        anchor: point(0),
        head: point(0),
    })?;
    first.response.request_focus();
    let (preedit, painted) = frame(
        &mut session,
        &ctx,
        &mut source,
        vec![Event::Ime(egui::ImeEvent::Preedit {
            text: "文字".into(),
            active_range_chars: Some(0..2),
        })],
    );
    assert!(preedit.errors.is_empty(), "{:?}", preedit.errors);
    assert!(has_note_ink(&painted));
    assert!(preedit.changes.is_empty());
    assert_eq!(session.document(), &original);
    assert_eq!(source.seen_pending, 0);
    let caret = session
        .canvas_caret_rect(&preedit.response)?
        .ok_or("missing measured preedit caret")?;
    assert!(preedit.response.rect.intersects(caret));
    let (cancelled, painted) = frame(
        &mut session,
        &ctx,
        &mut source,
        vec![Event::Ime(egui::ImeEvent::Preedit {
            text: String::new(),
            active_range_chars: None,
        })],
    );
    assert!(cancelled.errors.is_empty());
    assert!(has_note_ink(&painted));
    assert_eq!(session.document(), &original);
    assert!(!session.is_composing());
    Ok(())
}

struct PassiveSource {
    range: Selection,
}
impl DecorationSource for PassiveSource {
    fn decorations(
        &mut self,
        _: &Document,
        _: Revision,
        _: &[ReplayProof],
    ) -> craft_editor_egui::Result<Vec<TextDecoration>> {
        Ok(vec![])
    }
    fn underlines(
        &mut self,
        _: &Document,
        _: Revision,
        _: &[ReplayProof],
    ) -> craft_editor_egui::Result<Vec<TextUnderline>> {
        Ok(vec![TextUnderline {
            range: self.range.clone(),
            color: Color32::RED,
            label: "Synthetic spelling concern".into(),
        }])
    }
}

#[test]
fn passive_proofing_paints_without_activating_or_replacing_native_selection() -> TestResult {
    let (mut session, _, ctx) = fixture()?;
    let selection = Selection {
        anchor: point(1),
        head: point(2),
    };
    session.set_selection(selection.clone())?;
    let canonical = session.document().clone();
    let revision = session.revision();
    let mut source = PassiveSource {
        range: Selection {
            anchor: point(8),
            head: point(12),
        },
    };
    let (view, painted) = frame(&mut session, &ctx, &mut source, vec![]);
    assert!(view.errors.is_empty(), "{:?}", view.errors);
    assert!(painted.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Path(path) if path.stroke.color == egui::epaint::ColorMode::Solid(Color32::RED))));
    assert!(view.activated_decorations.is_empty());
    assert!(view.changes.is_empty());
    assert_eq!(session.selection(), Some(&selection));
    assert_eq!(session.document(), &canonical);
    assert_eq!(session.revision(), revision);
    Ok(())
}

#[test]
fn invalid_passive_range_is_not_painted_and_never_blocks_prose() -> TestResult {
    let (mut session, _, ctx) = fixture()?;
    let mut source = PassiveSource {
        range: Selection {
            anchor: point(8),
            head: point(999),
        },
    };
    let canonical = session.document().clone();
    let (view, painted) = frame(&mut session, &ctx, &mut source, vec![]);
    assert!(!view.errors.is_empty());
    assert!(!painted.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Path(path) if path.stroke.color == egui::epaint::ColorMode::Solid(Color32::RED))));
    assert_eq!(session.document(), &canonical);
    assert!(view.activated_decorations.is_empty());
    Ok(())
}
