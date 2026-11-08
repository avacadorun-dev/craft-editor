use super::*;
use craft_editor_model::{NodePath, TableGrid};
use serde_json::{Value, json};
type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
fn frame(
    session: &mut EditorSession,
    ctx: &egui::Context,
    events: Vec<Event>,
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
                response = Some(session.show_canvas(ui, &CanvasStyle::default()));
            });
        },
    );
    output.textures_delta.clear();
    (
        match response {
            Some(value) => value,
            None => panic!("native node selection frame"),
        },
        output,
    )
}
fn key(key: Key) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}
fn button(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}
fn ordinary() -> Result<Document> {
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/source-table-node.json"))
            .map_err(craft_editor_model::Error::from)?;
    Ok(Document::from_json(fixture["cases"][0]["input"].clone())?)
}
fn text(block: &str, offset: usize) -> Selection {
    collapsed(DocPoint {
        block_id: block.into(),
        offset: ScalarOffset(offset),
    })
}
fn outlined(output: &egui::FullOutput, rect: Rect) -> bool {
    output.shapes.iter().any(|shape|matches!(&shape.shape,egui::epaint::Shape::Rect(selected) if selected.rect==rect.expand(2.0) && selected.stroke.width==2.0 && selected.stroke.color==CanvasStyle::default().selection))
}
#[test]
fn whole_table_outline_escape_arrows_and_regular_text_hits_use_distinct_native_forms() -> TestResult
{
    let original = ordinary()?;
    let mut session = EditorSession::new(original.clone(), Revision(0));
    session.set_selection(text("a", 1))?;
    let ctx = egui::Context::default();
    frame(&mut session, &ctx, vec![]);
    ctx.memory_mut(|memory| memory.request_focus(session.options.id));
    session.select_current_table()?;
    let table = session.node_selection().cloned().ok_or("node")?;
    let (paint, output) = frame(&mut session, &ctx, vec![]);
    assert!(paint.errors.is_empty());
    let geometry = session
        .canvas_state
        .geometry
        .as_ref()
        .ok_or(Error::InvalidSelection)?;
    assert!(
        !geometry
            .nodes()
            .iter()
            .any(|node| node.canonical_path == table.path)
    );
    let bounds = geometry
        .node_bounds(&table.path)
        .ok_or("missing table selection frame")?;
    assert!(outlined(
        &output,
        on_screen(paint.response.rect.min, bounds)
    ));
    let caret = geometry
        .caret(&DocPoint {
            block_id: "a".into(),
            offset: ScalarOffset(1),
        })?
        .ok_or("text caret")?;
    let at = on_screen(paint.response.rect.min, caret).center() + Vec2::new(0.1, 0.0);
    frame(
        &mut session,
        &ctx,
        vec![Event::PointerMoved(at), button(at, true)],
    );
    frame(&mut session, &ctx, vec![button(at, false)]);
    assert!(session.node_selection().is_none());
    assert_eq!(session.selection().ok_or("text click")?.head.block_id, "a");
    let prior = session.selection_state()?;
    session.select_current_table()?;
    let (escaped, _) = frame(&mut session, &ctx, vec![key(Key::Escape)]);
    assert!(escaped.errors.is_empty());
    assert_eq!(session.selection_state()?, prior);
    for (key, block, offset) in [(Key::ArrowLeft, "before", 8), (Key::ArrowRight, "after", 0)] {
        session.select_node(table.clone())?;
        let (moved, _) = frame(&mut session, &ctx, vec![self::key(key)]);
        assert!(moved.errors.is_empty(), "{:?}", moved.errors);
        assert_eq!(session.selection(), Some(&text(block, offset)));
    }
    session.select_node(table)?;
    let (preedit, output) = frame(
        &mut session,
        &ctx,
        vec![Event::Ime(egui::ImeEvent::Preedit {
            text: "候補".into(),
            active_range_chars: Some(0..2),
        })],
    );
    assert!(preedit.errors.is_empty(), "{:?}", preedit.errors);
    assert!(session.is_composing());
    assert_eq!(session.document(), &original);
    assert!(output.platform_output.ime.is_some());
    frame(&mut session, &ctx, vec![key(Key::Escape)]);
    assert!(!session.is_composing());
    assert!(session.node_selection().is_some());
    assert_eq!(session.document(), &original);
    assert!(!session.can_undo());
    Ok(())
}
#[test]
fn sole_table_arrow_retains_a_visible_outline_and_never_invents_content() -> TestResult {
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/source-table-node.json"))?;
    let original = Document::from_json(fixture["cases"][1]["input"].clone())?;
    let mut session = EditorSession::new(original.clone(), Revision(0));
    session.select_node(session.document().node_ref(&NodePath(vec![0]))?)?;
    let selected = session.node_selection().cloned().ok_or("node")?;
    let ctx = egui::Context::default();
    frame(&mut session, &ctx, vec![]);
    ctx.memory_mut(|memory| memory.request_focus(session.options.id));
    for key in [
        Key::ArrowLeft,
        Key::ArrowRight,
        Key::ArrowUp,
        Key::ArrowDown,
    ] {
        let (response, output) = frame(&mut session, &ctx, vec![self::key(key)]);
        assert!(response.errors.is_empty(), "{:?}", response.errors);
        let bounds = session
            .canvas_state
            .geometry
            .as_ref()
            .ok_or(Error::InvalidSelection)?
            .node_bounds(&selected.path)
            .ok_or("table bounds")?;
        assert!(outlined(
            &output,
            on_screen(response.response.rect.min, bounds)
        ));
        assert_eq!(session.node_selection(), Some(&selected));
        assert_eq!(session.document(), &original);
        assert!(!session.can_undo());
    }
    Ok(())
}
#[test]
fn wide_table_node_selection_has_a_bounded_frame_and_survives_final_track_resize_undo() -> TestResult
{
    let cells:Vec<_>=(0..63).map(|i|json!({"type":"tableCell","attrs":{"colwidth":[80]},"content":[{"type":"paragraph","attrs":{"blockId":format!("c-{i}")},"content":[{"type":"text","text":format!("Cell {i}")}]}]})).collect();
    let original = Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":4},"content":[{"type":"table","attrs":{"blockId":"table"},"content":[{"type":"tableRow","content":cells}]}]}),
    )?;
    let mut session = EditorSession::new(original.clone(), Revision(0));
    session.set_table_column_limit(Some(63))?;
    session.set_selection(text("c-62", 0))?;
    session.request_scroll_to_caret();
    let ctx = egui::Context::default();
    let (first, _) = frame(&mut session, &ctx, vec![]);
    session.select_current_table()?;
    let selected = session.node_selection().cloned().ok_or("node")?;
    let (paint, output) = frame(&mut session, &ctx, vec![]);
    assert!(paint.errors.is_empty());
    let geometry = session
        .canvas_state
        .geometry
        .as_ref()
        .ok_or(Error::InvalidSelection)?;
    let viewport = geometry.table_viewports().remove(0);
    assert!(viewport.offset > 0.0);
    let bounds = geometry
        .node_bounds(&selected.path)
        .ok_or("bounded selection frame")?;
    assert!(bounds.width <= 684.0);
    assert!(outlined(
        &output,
        on_screen(paint.response.rect.min, bounds)
    ));
    let middle = on_screen(first.response.rect.min, viewport.rect).center();
    frame(
        &mut session,
        &ctx,
        vec![
            Event::PointerMoved(middle),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                phase: egui::TouchPhase::Move,
                delta: Vec2::new(-100_000.0, 0.0),
                modifiers: Modifiers::NONE,
            },
        ],
    );
    let column = session
        .canvas_state
        .geometry
        .as_ref()
        .ok_or(Error::InvalidSelection)?
        .columns(session.document())?
        .into_iter()
        .find(|column| column.column == 62)
        .ok_or("last track")?;
    let edge = egui::pos2(
        first.response.rect.left() + column.rect.x + column.rect.width,
        first.response.rect.top() + column.rect.y + column.rect.height * 0.5,
    );
    frame(
        &mut session,
        &ctx,
        vec![Event::PointerMoved(edge), button(edge, true)],
    );
    let end = edge + Vec2::new(40.0, 0.0);
    let (resize, _) = frame(
        &mut session,
        &ctx,
        vec![Event::PointerMoved(end), button(end, false)],
    );
    assert!(resize.errors.is_empty(), "{:?}", resize.errors);
    assert_eq!(
        TableGrid::new(session.document().node(&selected.path)?)?
            .column_widths
            .get(&62),
        Some(&120)
    );
    assert!(session.node_selection().is_some());
    assert!(session.selection().is_none() && session.cell_selection().is_none());
    session.undo()?;
    assert_eq!(session.document(), &original);
    assert_eq!(session.node_selection(), Some(&selected));
    Ok(())
}
