//! Hosted raster/input qualification; a headless checkpoint does not approve native UX.

use craft_editor_egui::{
    DocPoint, EditorSession, Selection,
    canvas::CanvasStyle,
    layout::{FlowGeometry, GeometryRect},
};
use craft_editor_model::{Document, Mark, MarkMode, Revision, ScalarOffset};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source = Document::from_json(serde_json::from_str(include_str!("canvas_fixture.json"))?)?;
    let before = source.to_json()?;
    let geometry = FlowGeometry::new(&source, 600.0)?;
    let cells = geometry.cells()?;
    let raster = geometry.rasterize(
        GeometryRect {
            page: 0,
            x: 0.0,
            y: 0.0,
            width: 600.0,
            height: geometry.extent().ok_or("missing extent")?.height.min(800.0),
        },
        2.0,
        [29, 34, 42],
    )?;
    let selection = Selection {
        anchor: DocPoint {
            block_id: "corner".into(),
            offset: ScalarOffset(3),
        },
        head: DocPoint {
            block_id: "merged".into(),
            offset: ScalarOffset(2),
        },
    };
    let rectangles = geometry.selection_rects(&selection)?;
    let mut session = EditorSession::new(source, Revision(1));
    session.set_selection(selection.clone())?;
    session.format_selection(Mark::new("bold"), MarkMode::Add)?;
    session.undo()?;
    if session.document().to_json()? != before || session.selection() != Some(&selection) {
        return Err("semantic undo mismatch".into());
    }
    session.set_selection(Selection {
        anchor: DocPoint {
            block_id: "right".into(),
            offset: ScalarOffset(2),
        },
        head: DocPoint {
            block_id: "right".into(),
            offset: ScalarOffset(2),
        },
    })?;
    let context = egui::Context::default();
    context.memory_mut(|memory| memory.request_focus(session.options.id));
    let mut errors = Vec::new();
    let mut changes = 0;
    let mut frame = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::Vec2::new(700.0, 650.0),
            )),
            events: vec![egui::Event::Text("🪶".into())],
            focused: true,
            ..Default::default()
        },
        |root| {
            egui::CentralPanel::default().show(root, |ui| {
                let response = session.show_canvas(ui, &CanvasStyle::default());
                changes = response.changes.len();
                errors.extend(response.errors.into_iter().map(|error| error.to_string()));
            });
        },
    );
    frame.textures_delta.clear();
    if !errors.is_empty() || session.document().block_text("right")? != "Ri🪶ght header" {
        return Err(format!("native input checkpoint: {errors:?}").into());
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "checkpoint":"native-grid-canvas", "real_cell_boxes":cells.len(), "covered_merge_boxes":cells.iter().filter(|cell|cell.covered).count(),
            "selection_rectangles":rectangles.len(), "raster_width":raster.width, "raster_height":raster.height,
            "raster_rgba_bytes":raster.pixels.len(), "semantic_format_undo_preserved":true,
            "native_text_event_committed":true, "frame_changes_including_queued_history":changes,
            "editor_acceptance":false, "platform_ime_acceptance":false, "full_schema_acceptance":false
        }))?
    );
    Ok(())
}
