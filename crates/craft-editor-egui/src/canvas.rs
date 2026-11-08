//! Experimental continuous-flow native document canvas.
//!
//! WordCraft supplies measured glyphs, real table tracks and viewport rasterization. Canonical
//! commands, selection and history remain owned by [`EditorSession`]. This is a qualification
//! checkpoint: unsupported nodes fail explicitly; rich clipboard, full accessibility and native
//! platform IME acceptance are not implied by the headless tests.

use craft_editor_model::{Command, Document, Mark, Revision, ScalarOffset, TextRange};
use egui::{Color32, Event, Key, Modifiers, Pos2, Rect, Sense, TextureHandle, Vec2};

use crate::{
    DocPoint, EditorSession, Error, Result, Selection, SemanticBuffer, ViewResponse,
    layout::{FlowGeometry, GeometryRect, GeometryStyle},
    projection::Projection,
};

/// Host-owned canvas colors and typography, independent of canonical document attributes.
#[derive(Debug, Clone)]
pub struct CanvasStyle {
    /// Body text color.
    pub foreground: Color32,
    /// Continuous document panel color; no inner paper frame is painted.
    pub background: Color32,
    /// Selection tint; opaque theme colors receive a translucent overlay.
    pub selection: Color32,
    /// Active insertion caret color.
    pub caret: Color32,
    /// Table grid color.
    pub table_border: Color32,
    /// Header-cell fill color.
    pub table_header: Color32,
    /// Body text size in logical points.
    pub font_size: f32,
    /// Font family registered with the layout backend, not an egui family identifier.
    pub font_family: Option<String>,
    /// Line-height multiple.
    pub line_height: f32,
    /// Space following paragraphs in logical points.
    pub paragraph_spacing: f32,
    /// Minimum allocated canvas height; the host owns the enclosing scroll area.
    pub min_height: f32,
    /// Allow selection and copying while refusing all native edits.
    pub read_only: bool,
}
impl Default for CanvasStyle {
    fn default() -> Self {
        Self {
            foreground: Color32::from_rgb(35, 39, 47),
            background: Color32::from_rgb(250, 250, 248),
            selection: Color32::from_rgb(70, 135, 220),
            caret: Color32::from_rgb(35, 39, 47),
            table_border: Color32::from_rgb(139, 148, 163),
            table_header: Color32::from_rgb(234, 238, 244),
            font_size: 18.0,
            font_family: None,
            line_height: 1.5,
            paragraph_spacing: 10.0,
            min_height: 320.0,
            read_only: false,
        }
    }
}
impl CanvasStyle {
    fn geometry(&self) -> GeometryStyle {
        GeometryStyle {
            font_size: self.font_size,
            font_family: self.font_family.clone(),
            text_color: rgb(self.foreground),
            table_border: rgb(self.table_border),
            table_header: rgb(self.table_header),
            line_height: self.line_height,
            paragraph_spacing: self.paragraph_spacing,
        }
    }
}

struct Preedit {
    document: Document,
    caret: DocPoint,
    text: String,
}
#[derive(PartialEq)]
struct MeasurementKey {
    revision: Revision,
    width: u32,
    style: GeometryStyle,
    preedit_epoch: u64,
}
struct Tile {
    column: i32,
    row: i32,
    scale: u32,
    paper: [u8; 3],
    rect: GeometryRect,
    texture: TextureHandle,
}
#[derive(Default)]
pub(crate) struct CanvasState {
    geometry: Option<FlowGeometry>,
    measured: Option<MeasurementKey>,
    tiles: Vec<Tile>,
    goal_x: Option<f32>,
    preedit: Option<Preedit>,
    preedit_epoch: u64,
    scroll_to_caret: bool,
}

impl CanvasState {
    fn measure(&mut self, session: &EditorSession, width: f32, style: &CanvasStyle) -> Result<()> {
        let key = MeasurementKey {
            revision: session.revision(),
            width: width.to_bits(),
            style: style.geometry(),
            preedit_epoch: self.preedit_epoch,
        };
        if self.measured.as_ref() != Some(&key) {
            let document = self
                .preedit
                .as_ref()
                .map_or_else(|| session.document(), |p| &p.document);
            let geometry = FlowGeometry::with_style(document, width, &|_| None, Some(&key.style))?;
            self.geometry = Some(geometry);
            self.measured = Some(key);
            self.tiles.clear();
        }
        Ok(())
    }

    pub(crate) fn show(
        &mut self,
        session: &mut EditorSession,
        ui: &mut egui::Ui,
        style: &CanvasStyle,
    ) -> ViewResponse {
        let mut errors = Vec::new();
        let width = ui.available_width().max(144.0);
        if let Err(error) = self.measure(session, width, style) {
            return ViewResponse {
                response: ui
                    .allocate_response(Vec2::new(width, style.min_height.max(0.0)), Sense::hover()),
                changes: std::mem::take(&mut session.pending),
                errors: vec![error],
            };
        }
        let extent = self.geometry.as_ref().and_then(FlowGeometry::extent);
        let height = extent.map_or(style.min_height, |rect| rect.height.max(style.min_height));
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
        let response = ui.interact(rect, session.options.id, Sense::click_and_drag());
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        }
        let pressed = response.hovered()
            && ui.input(|input| input.pointer.button_pressed(egui::PointerButton::Primary));
        if (pressed || response.clicked() || response.drag_started() || response.dragged())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            response.request_focus();
            self.cancel_preedit(session);
            if let Err(error) = self.measure(session, width, style) {
                errors.push(error);
            }
            if let Err(error) = self.pointer(
                session,
                pointer - rect.min,
                ui.input(|i| i.modifiers.shift),
                !pressed && response.dragged(),
                response.double_clicked(),
                response.triple_clicked(),
            ) {
                errors.push(error);
            }
        }
        let focused = ui.memory(|memory| memory.has_focus(session.options.id));
        if focused {
            ui.memory_mut(|memory| {
                memory.set_focus_lock_filter(
                    session.options.id,
                    egui::EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                    },
                )
            });
            let events = ui.input_mut(|input| {
                let mut accepted = Vec::new();
                input.events.retain(|event| {
                    if editor_event(event) {
                        accepted.push(event.clone());
                        false
                    } else {
                        true
                    }
                });
                accepted
            });
            for event in events {
                if let Err(error) = self.event(session, ui, event, style.read_only) {
                    errors.push(error);
                }
                if let Err(error) = self.measure(session, width, style) {
                    errors.push(error);
                    break;
                }
            }
        }
        if let Err(error) = self.measure(session, width, style) {
            errors.push(error);
        }
        ui.painter().rect_filled(rect, 0.0, style.background);
        if let Err(error) = self.paint(ui, rect, style) {
            errors.push(error);
        }
        if let Some(geometry) = &self.geometry {
            let selection = session.selection.as_ref();
            if self.preedit.is_none()
                && let Some(selection) = selection
            {
                match geometry.selection_rects(selection) {
                    Ok(rectangles) => {
                        let tint = Color32::from_rgba_unmultiplied(
                            style.selection.r(),
                            style.selection.g(),
                            style.selection.b(),
                            style.selection.a().min(85),
                        );
                        for selected in rectangles {
                            ui.painter()
                                .rect_filled(on_screen(rect.min, selected), 1.0, tint);
                        }
                    }
                    Err(error) => errors.push(error.into()),
                }
            }
            let caret = self
                .preedit
                .as_ref()
                .map(|preedit| &preedit.caret)
                .or_else(|| selection.map(|s| &s.head));
            if let Some(caret) = caret {
                match geometry.caret(caret) {
                    Ok(Some(caret)) => {
                        let caret_rect = on_screen(rect.min, caret);
                        if focused {
                            let x = caret_rect.left();
                            ui.painter().line_segment(
                                [
                                    Pos2::new(x, caret_rect.top()),
                                    Pos2::new(x, caret_rect.bottom()),
                                ],
                                egui::Stroke::new(1.5, style.caret),
                            );
                            ui.output_mut(|output| {
                                output.ime = Some(egui::output::IMEOutput {
                                    purpose: egui::IMEPurpose::Normal,
                                    rect,
                                    cursor_rect: caret_rect,
                                    should_interrupt_composition: false,
                                })
                            });
                        }
                        if self.scroll_to_caret {
                            ui.scroll_to_rect(caret_rect.expand(12.0), None);
                            self.scroll_to_caret = false;
                        }
                        if let Some(preedit) = &self.preedit
                            && !preedit.text.is_empty()
                        {
                            ui.painter().line_segment(
                                [
                                    caret_rect.left_bottom() - Vec2::new(10.0, 0.0),
                                    caret_rect.left_bottom(),
                                ],
                                egui::Stroke::new(1.0, style.caret),
                            );
                        }
                    }
                    Ok(None) => {}
                    Err(error) => errors.push(error.into()),
                }
            }
        }
        response.widget_info(|| egui::WidgetInfo::text_edit(!style.read_only, "", "", ""));
        ViewResponse {
            response,
            changes: std::mem::take(&mut session.pending),
            errors,
        }
    }

    fn pointer(
        &mut self,
        session: &mut EditorSession,
        local: Vec2,
        shift: bool,
        dragging: bool,
        double: bool,
        triple: bool,
    ) -> Result<()> {
        let geometry = self.geometry.as_ref().ok_or(Error::InvalidSelection)?;
        let Some(point) = geometry.hit(0, local.x, local.y)? else {
            return Ok(());
        };
        let selection = if triple {
            Selection {
                anchor: DocPoint {
                    block_id: point.block_id.clone(),
                    offset: ScalarOffset(0),
                },
                head: DocPoint {
                    block_id: point.block_id.clone(),
                    offset: ScalarOffset(
                        session
                            .document()
                            .block_text(&point.block_id)?
                            .chars()
                            .count(),
                    ),
                },
            }
        } else if double {
            geometry.word_at(&point)?
        } else if (shift || dragging)
            && let Some(selection) = session.selection.as_ref()
        {
            Selection {
                anchor: selection.anchor.clone(),
                head: point,
            }
        } else {
            collapsed(point)
        };
        session.set_selection(selection)?;
        self.goal_x = None;
        Ok(())
    }

    fn event(
        &mut self,
        session: &mut EditorSession,
        ui: &egui::Ui,
        event: Event,
        read_only: bool,
    ) -> Result<()> {
        match event {
            Event::Copy => {
                ui.ctx().copy_text(session.selected_text()?);
            }
            Event::Cut if !read_only => {
                let text = session.selected_text()?;
                replace(session, "")?;
                ui.ctx().copy_text(text);
                self.scroll_to_caret = true;
            }
            Event::Text(text) | Event::Paste(text) if !read_only && self.preedit.is_none() => {
                replace(session, &text.replace("\r\n", "\n"))?;
                self.scroll_to_caret = true;
            }
            Event::Ime(egui::ImeEvent::Preedit { text, .. }) if !read_only => {
                self.preedit_epoch = self.preedit_epoch.wrapping_add(1);
                if text.is_empty() {
                    self.cancel_preedit(session);
                } else {
                    let (buffer, caret) = replacement_buffer(session, &text)?;
                    self.preedit = Some(Preedit {
                        document: buffer.editor.document().clone(),
                        caret,
                        text,
                    });
                    session.canvas_composing = true;
                }
            }
            Event::Ime(egui::ImeEvent::Commit(text)) if !read_only => {
                self.cancel_preedit(session);
                if !text.is_empty() {
                    replace(session, &text)?;
                }
                self.scroll_to_caret = true;
            }
            Event::Ime(egui::ImeEvent::DeleteSurrounding {
                before_chars,
                after_chars,
            }) if !read_only => {
                self.cancel_preedit(session);
                if let Some(selection) = session.selection.as_ref() {
                    let point = &selection.head;
                    let len = session
                        .document()
                        .block_text(&point.block_id)?
                        .chars()
                        .count();
                    session.selection = Some(Selection {
                        anchor: DocPoint {
                            block_id: point.block_id.clone(),
                            offset: ScalarOffset(point.offset.0.saturating_sub(before_chars)),
                        },
                        head: DocPoint {
                            block_id: point.block_id.clone(),
                            offset: ScalarOffset(
                                point.offset.0.saturating_add(after_chars).min(len),
                            ),
                        },
                    });
                    replace(session, "")?;
                }
            }
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                self.key(session, key, modifiers, read_only)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn key(
        &mut self,
        session: &mut EditorSession,
        key: Key,
        modifiers: Modifiers,
        read_only: bool,
    ) -> Result<()> {
        if key == Key::Escape {
            self.cancel_preedit(session);
            return Ok(());
        }
        if self.preedit.is_some() {
            return Ok(());
        }
        if modifiers.command {
            match key {
                Key::Z if !read_only => {
                    if modifiers.shift {
                        session.redo()?;
                    } else {
                        session.undo()?;
                    }
                    self.scroll_to_caret = true;
                    return Ok(());
                }
                Key::Y if !read_only => {
                    session.redo()?;
                    self.scroll_to_caret = true;
                    return Ok(());
                }
                Key::B | Key::I | Key::U if !read_only => {
                    let kind = match key {
                        Key::B => "bold",
                        Key::I => "italic",
                        _ => "underline",
                    };
                    session.toggle_mark(Mark::new(kind))?;
                    return Ok(());
                }
                _ => {}
            }
        }
        let geometry = self.geometry.as_ref().ok_or(Error::InvalidSelection)?;
        let selection = session
            .selection
            .clone()
            .unwrap_or(collapsed(geometry.edge(false)?));
        if modifiers.command && key == Key::A {
            session.set_selection(Selection {
                anchor: geometry.edge(false)?,
                head: geometry.edge(true)?,
            })?;
            return Ok(());
        }
        if !read_only && matches!(key, Key::Backspace | Key::Delete) {
            if selection.anchor == selection.head {
                let other = geometry.deletion_boundary(&selection.head, key == Key::Backspace)?;
                session.selection = Some(Selection {
                    anchor: other,
                    head: selection.head,
                });
            }
            replace(session, "")?;
            self.goal_x = None;
            self.scroll_to_caret = true;
            return Ok(());
        }
        if !read_only && key == Key::Enter {
            replace(session, "\n")?;
            self.scroll_to_caret = true;
            return Ok(());
        }
        let word = modifiers.alt || (modifiers.ctrl && !modifiers.mac_cmd);
        let moved = match key {
            Key::ArrowLeft | Key::ArrowRight => {
                let left = key == Key::ArrowLeft;
                if modifiers.mac_cmd {
                    Some(geometry.line_edge(&selection.head, !left)?)
                } else if !modifiers.shift && selection.anchor != selection.head {
                    let projection = Projection::new(session.document())?;
                    let a = projection.scalar(&selection.anchor)?;
                    let b = projection.scalar(&selection.head)?;
                    Some(if left == (a <= b) {
                        selection.anchor.clone()
                    } else {
                        selection.head.clone()
                    })
                } else {
                    Some(geometry.horizontal(&selection.head, left, word)?)
                }
            }
            Key::ArrowUp | Key::ArrowDown => {
                if modifiers.mac_cmd {
                    Some(geometry.edge(key == Key::ArrowDown)?)
                } else {
                    geometry
                        .vertical(
                            &selection.head,
                            self.goal_x,
                            if key == Key::ArrowUp { -1 } else { 1 },
                        )?
                        .map(|(point, x)| {
                            self.goal_x = Some(x);
                            point
                        })
                }
            }
            Key::Home | Key::End => Some(if modifiers.command {
                geometry.edge(key == Key::End)?
            } else {
                geometry.line_edge(&selection.head, key == Key::End)?
            }),
            Key::Tab => geometry.adjacent_cell(&selection.head, modifiers.shift)?,
            _ => None,
        };
        if let Some(point) = moved {
            session.set_selection(if modifiers.shift && key != Key::Tab {
                Selection {
                    anchor: selection.anchor,
                    head: point,
                }
            } else {
                collapsed(point)
            })?;
            if !matches!(key, Key::ArrowUp | Key::ArrowDown) {
                self.goal_x = None;
            }
            self.scroll_to_caret = true;
        }
        Ok(())
    }

    fn cancel_preedit(&mut self, session: &mut EditorSession) {
        if self.preedit.take().is_some() {
            self.preedit_epoch = self.preedit_epoch.wrapping_add(1);
        }
        session.canvas_composing = false;
    }

    fn paint(&mut self, ui: &egui::Ui, canvas: Rect, style: &CanvasStyle) -> Result<()> {
        const TILE: f32 = 256.0;
        const MAX_TILES: usize = 32;
        let geometry = self.geometry.as_ref().ok_or(Error::InvalidSelection)?;
        let Some(extent) = geometry.extent() else {
            return Ok(());
        };
        let visible = canvas.intersect(ui.clip_rect());
        if !visible.is_positive() {
            return Ok(());
        }
        let local_min = visible.min - canvas.min;
        let local_max = visible.max - canvas.min;
        let scale = ui.ctx().pixels_per_point().clamp(1.0, 3.0);
        let paper = rgb(style.background);
        let columns = (local_min.x / TILE).floor() as i32
            ..=((local_max.x.min(extent.width) / TILE).floor() as i32);
        let rows = (local_min.y / TILE).floor() as i32
            ..=((local_max.y.min(extent.height) / TILE).floor() as i32);
        for row in rows {
            for column in columns.clone() {
                let area = GeometryRect {
                    page: 0,
                    x: column as f32 * TILE,
                    y: row as f32 * TILE,
                    width: TILE.min(extent.width - column as f32 * TILE),
                    height: TILE.min(extent.height - row as f32 * TILE),
                };
                if area.width <= 0.0 || area.height <= 0.0 {
                    continue;
                }
                let cached = self.tiles.iter().position(|tile| {
                    tile.column == column
                        && tile.row == row
                        && tile.scale == scale.to_bits()
                        && tile.paper == paper
                });
                let index = if let Some(index) = cached {
                    index
                } else {
                    let raster = geometry.rasterize(area, scale, paper)?;
                    let image = egui::ColorImage::from_rgba_premultiplied(
                        [raster.width as usize, raster.height as usize],
                        &raster.pixels,
                    );
                    let texture = ui.ctx().load_texture(
                        format!("craft-canvas-{}-{column}-{row}", ui.id().value()),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    if self.tiles.len() >= MAX_TILES {
                        self.tiles.remove(0);
                    }
                    self.tiles.push(Tile {
                        column,
                        row,
                        scale: scale.to_bits(),
                        paper,
                        rect: area,
                        texture,
                    });
                    self.tiles.len() - 1
                };
                let tile = &self.tiles[index];
                ui.painter().image(
                    tile.texture.id(),
                    on_screen(canvas.min, tile.rect),
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        }
        Ok(())
    }
}

fn rgb(color: Color32) -> [u8; 3] {
    [color.r(), color.g(), color.b()]
}
fn collapsed(point: DocPoint) -> Selection {
    Selection {
        anchor: point.clone(),
        head: point,
    }
}
fn on_screen(origin: Pos2, rect: GeometryRect) -> Rect {
    Rect::from_min_size(
        origin + Vec2::new(rect.x, rect.y),
        Vec2::new(rect.width.max(1.0), rect.height),
    )
}
fn editor_event(event: &Event) -> bool {
    match event {
        Event::Text(_) | Event::Paste(_) | Event::Copy | Event::Cut | Event::Ime(_) => true,
        Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } => {
            matches!(
                key,
                Key::ArrowLeft
                    | Key::ArrowRight
                    | Key::ArrowUp
                    | Key::ArrowDown
                    | Key::Home
                    | Key::End
                    | Key::Tab
                    | Key::Enter
                    | Key::Backspace
                    | Key::Delete
                    | Key::Escape
            ) || (modifiers.command
                && matches!(key, Key::A | Key::B | Key::I | Key::U | Key::Z | Key::Y))
        }
        _ => false,
    }
}

fn replacement_buffer(session: &EditorSession, text: &str) -> Result<(SemanticBuffer, DocPoint)> {
    let projection = Projection::new(session.document())?;
    let selection = session
        .selection
        .clone()
        .unwrap_or(collapsed(projection.point(0, true)?));
    let a = projection.scalar(&selection.anchor)?;
    let b = projection.scalar(&selection.head)?;
    let at = a.min(b);
    let mut buffer = SemanticBuffer::new(
        session.document().clone(),
        session.revision(),
        projection,
        session.next_id,
    );
    buffer.typing_marks.clone_from(&session.stored_marks);
    if a != b {
        let spans: Vec<_> = buffer
            .projection
            .spans
            .iter()
            .filter(|span| span.end >= at && span.start <= a.max(b))
            .cloned()
            .collect();
        if spans.len() > 1 && spans.iter().any(|span| span.table || span.path.len() != 1) {
            // Text-range deletion through cells clears their content, preserving the grid and
            // durable cell/block identities. Joining cells is a separate structural command.
            let mut commands = Vec::new();
            for span in spans {
                let from = at.max(span.start) - span.start;
                let to = a.max(b).min(span.end).saturating_sub(span.start);
                if from < to {
                    commands.push(Command::ReplaceText {
                        range: TextRange {
                            block_id: span.id,
                            start: ScalarOffset(from),
                            end: ScalarOffset(to),
                        },
                        expected: span
                            .node
                            .inline_text()
                            .chars()
                            .skip(from)
                            .take(to - from)
                            .collect(),
                        replacement: String::new(),
                    });
                }
            }
            buffer.execute(commands)?;
        } else {
            buffer.delete(at..a.max(b))?;
        }
    }
    let insertion = buffer.projection.point(at, true)?;
    if !text.is_empty() {
        buffer.insert(text, at)?;
    }
    // Resolve the final caret from the command sequence, not by adding flattened offsets
    // through preserved cell separators or freshly split paragraph identities.
    let mut caret = insertion;
    for command in &buffer.commands {
        match command {
            Command::SplitBlock {
                block_id,
                new_block_id,
                ..
            } if *block_id == caret.block_id => {
                caret.block_id.clone_from(new_block_id);
                caret.offset = ScalarOffset(0);
            }
            Command::ReplaceText {
                range, replacement, ..
            } if range.block_id == caret.block_id
                && range.start == range.end
                && !replacement.is_empty() =>
            {
                caret.offset = ScalarOffset(range.start.0 + replacement.chars().count());
            }
            _ => {}
        }
    }
    Ok((buffer, caret))
}
fn replace(session: &mut EditorSession, text: &str) -> Result<()> {
    let stored_marks = session.stored_marks.clone();
    let (buffer, caret) = replacement_buffer(session, text)?;
    let next_id = buffer.next_id;
    let change = session.apply_commands(buffer.commands)?;
    session.next_id = next_id;
    session.stored_marks = stored_marks;
    session.selection = Some(collapsed(caret));
    if change.before_revision != change.after_revision
        && let Some(history) = session.undo_selection.last_mut()
    {
        history.after = session.selection.clone();
    }
    session.restore_cursor = false;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_editor_model::MarkMode;
    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

    fn fixture() -> craft_editor_model::Result<Document> {
        Document::from_json(serde_json::from_str(include_str!(
            "../examples/canvas_fixture.json"
        ))?)
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
        events: Vec<Event>,
        style: &CanvasStyle,
    ) -> ViewResponse {
        let mut result = None;
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(700.0, 650.0))),
                events,
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
        match result {
            Some(result) => result,
            None => panic!("canvas frame did not run"),
        }
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

    #[test]
    fn real_grid_wraps_spans_within_the_host_column() -> TestResult {
        let source = fixture()?;
        let geometry = FlowGeometry::with_style(
            &source,
            500.0,
            &|_| None,
            Some(&CanvasStyle::default().geometry()),
        )?;
        let cells = geometry.cells()?;
        assert_eq!(cells.len(), 6);
        assert_eq!(cells.iter().filter(|cell| cell.covered).count(), 1);
        assert!(
            cells
                .iter()
                .all(|cell| cell.rect.x >= 17.0 && cell.rect.x + cell.rect.width <= 483.0)
        );
        let first = geometry
            .caret(&point("merged", 0))?
            .ok_or(Error::InvalidSelection)?;
        let last = geometry
            .caret(&point("merged", 85))?
            .ok_or(Error::InvalidSelection)?;
        assert!(
            last.y > first.y,
            "cell text must wrap within real grid tracks"
        );
        Ok(())
    }
    #[test]
    fn raster_uses_host_paper_and_premultiplied_rgba() -> TestResult {
        let source = fixture()?;
        let style = CanvasStyle {
            foreground: Color32::from_rgb(220, 225, 232),
            background: Color32::from_rgb(29, 34, 42),
            table_border: Color32::from_rgb(90, 105, 123),
            table_header: Color32::from_rgb(39, 48, 61),
            ..Default::default()
        };
        let geometry =
            FlowGeometry::with_style(&source, 600.0, &|_| None, Some(&style.geometry()))?;
        let raster = geometry.rasterize(
            GeometryRect {
                page: 0,
                x: 0.0,
                y: 0.0,
                width: 256.0,
                height: 256.0,
            },
            2.0,
            rgb(style.background),
        )?;
        assert_eq!((raster.width, raster.height), (512, 512));
        assert_eq!(&raster.pixels[..4], &[29, 34, 42, 255]);
        assert!(
            raster
                .pixels
                .chunks_exact(4)
                .any(|pixel| pixel[..3] != [29, 34, 42])
        );
        assert!(
            raster
                .pixels
                .chunks_exact(4)
                .all(|pixel| pixel[0] <= pixel[3] && pixel[1] <= pixel[3] && pixel[2] <= pixel[3])
        );
        Ok(())
    }
    #[test]
    fn canvas_pointer_hit_then_unicode_typing_preserves_merged_grid() -> TestResult {
        let original = fixture()?;
        let mut session = EditorSession::new(original.clone(), Revision(2));
        let mut state = CanvasState::default();
        state.measure(&session, 600.0, &CanvasStyle::default())?;
        let caret = state
            .geometry
            .as_ref()
            .ok_or(Error::InvalidSelection)?
            .caret(&point("merged", 2))?
            .ok_or(Error::InvalidSelection)?;
        state.pointer(
            &mut session,
            Vec2::new(caret.x + 0.01, caret.y + caret.height / 2.0),
            false,
            false,
            false,
            false,
        )?;
        assert_eq!(
            session.selection().map(|s| &s.head),
            Some(&point("merged", 2))
        );
        replace(&mut session, "字")?;
        assert!(
            session
                .document()
                .block_text("merged")?
                .starts_with("Я😀字")
        );
        assert_eq!(
            session.document().block("grid")?.attrs,
            original.block("grid")?.attrs
        );
        session.undo()?;
        assert_eq!(session.document(), &original);
        Ok(())
    }
    #[test]
    fn reverse_cross_cell_selection_formats_and_undo_restores_direction() -> TestResult {
        let original = fixture()?;
        let mut session = EditorSession::new(original.clone(), Revision(1));
        let selection = Selection {
            anchor: point("corner", 3),
            head: point("merged", 2),
        };
        session.set_selection(selection.clone())?;
        let context = egui::Context::default();
        context.memory_mut(|memory| memory.request_focus(session.options.id));
        let response = frame(
            &mut session,
            &context,
            vec![key(Key::B, Modifiers::COMMAND)],
            &CanvasStyle::default(),
        );
        assert!(response.errors.is_empty(), "{:?}", response.errors);
        assert_eq!(response.changes.len(), 1);
        assert_eq!(session.selection(), Some(&selection));
        assert_eq!(session.mark_state("bold")?, crate::MarkState::On);
        session.undo()?;
        assert_eq!(session.document(), &original);
        assert_eq!(session.selection(), Some(&selection));
        Ok(())
    }
    #[test]
    fn canvas_text_event_and_keyboard_undo_use_canonical_history() -> TestResult {
        let original = fixture()?;
        let mut session = EditorSession::new(original.clone(), Revision(8));
        session.set_selection(collapsed(point("right", 2)))?;
        let context = egui::Context::default();
        context.memory_mut(|memory| memory.request_focus(session.options.id));
        let result = frame(
            &mut session,
            &context,
            vec![Event::Text("🪶".into())],
            &CanvasStyle::default(),
        );
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(session.document().block_text("right")?, "Ri🪶ght header");
        assert_eq!(result.changes.len(), 1);
        let undone = frame(
            &mut session,
            &context,
            vec![key(Key::Z, Modifiers::COMMAND)],
            &CanvasStyle::default(),
        );
        assert!(undone.errors.is_empty(), "{:?}", undone.errors);
        assert_eq!(session.document(), &original);
        Ok(())
    }
    #[test]
    fn canvas_preedit_never_changes_canonical_state_before_commit() -> TestResult {
        let original = fixture()?;
        let mut session = EditorSession::new(original.clone(), Revision(3));
        session.set_selection(collapsed(point("right", 1)))?;
        let context = egui::Context::default();
        context.memory_mut(|memory| memory.request_focus(session.options.id));
        let interim = frame(
            &mut session,
            &context,
            vec![Event::Ime(egui::ImeEvent::Preedit {
                text: "ni".into(),
                active_range_chars: Some(0..2),
            })],
            &CanvasStyle::default(),
        );
        assert!(interim.errors.is_empty(), "{:?}", interim.errors);
        assert!(interim.changes.is_empty());
        assert!(session.is_composing());
        assert_eq!(session.document(), &original);
        assert!(matches!(
            session.format_selection(Mark::new("bold"), MarkMode::Add),
            Err(Error::CompositionActive)
        ));
        let committed = frame(
            &mut session,
            &context,
            vec![Event::Ime(egui::ImeEvent::Commit("字".into()))],
            &CanvasStyle::default(),
        );
        assert!(committed.errors.is_empty(), "{:?}", committed.errors);
        assert_eq!(committed.changes.len(), 1);
        assert!(!session.is_composing());
        assert_eq!(session.document().block_text("right")?, "R字ight header");
        session.undo()?;
        assert_eq!(session.document(), &original);
        Ok(())
    }
    #[test]
    fn cross_cell_text_deletion_clears_text_without_joining_cells() -> TestResult {
        let original = fixture()?;
        let mut session = EditorSession::new(original.clone(), Revision(1));
        session.set_selection(Selection {
            anchor: point("right", 2),
            head: point("merged", 2),
        })?;
        replace(&mut session, "X")?;
        assert_eq!(session.document().block_text("merged")?, "Я😀X");
        assert_eq!(session.document().block_text("right")?, "ght header");
        assert_eq!(session.document().block("grid")?.children().len(), 3);
        session.undo()?;
        assert_eq!(session.document(), &original);
        Ok(())
    }
    #[test]
    fn read_only_canvas_allows_selection_but_rejects_native_text() -> TestResult {
        let original = fixture()?;
        let mut session = EditorSession::new(original.clone(), Revision(1));
        session.set_selection(collapsed(point("right", 1)))?;
        let context = egui::Context::default();
        context.memory_mut(|memory| memory.request_focus(session.options.id));
        let response = frame(
            &mut session,
            &context,
            vec![
                Event::Text("ignored".into()),
                key(Key::B, Modifiers::COMMAND),
            ],
            &CanvasStyle {
                read_only: true,
                ..Default::default()
            },
        );
        assert!(response.errors.is_empty(), "{:?}", response.errors);
        assert!(response.changes.is_empty());
        assert_eq!(session.document(), &original);
        Ok(())
    }
    #[test]
    fn collapsed_toolbar_mark_changes_next_typing_without_empty_history() -> TestResult {
        let original = Document::plain("p", "AB")?;
        let mut session = EditorSession::new(original.clone(), Revision(1));
        session.set_selection(collapsed(point("p", 1)))?;
        session.toggle_mark(Mark::new("bold"))?;
        assert_eq!(session.mark_state("bold")?, crate::MarkState::On);
        assert_eq!(session.document(), &original);
        assert!(!session.can_undo());
        replace(&mut session, "X")?;
        assert_eq!(session.document().block_text("p")?, "AXB");
        assert_eq!(session.mark_state("bold")?, crate::MarkState::On);
        session.toggle_mark(Mark::new("bold"))?;
        replace(&mut session, "Y")?;
        session.set_selection(Selection {
            anchor: point("p", 1),
            head: point("p", 2),
        })?;
        assert_eq!(session.mark_state("bold")?, crate::MarkState::On);
        session.set_selection(Selection {
            anchor: point("p", 2),
            head: point("p", 3),
        })?;
        assert_eq!(session.mark_state("bold")?, crate::MarkState::Off);
        session.set_selection(Selection {
            anchor: point("p", 1),
            head: point("p", 3),
        })?;
        assert_eq!(session.mark_state("bold")?, crate::MarkState::Mixed);
        session.undo()?;
        session.undo()?;
        assert_eq!(session.document(), &original);
        Ok(())
    }
    #[test]
    fn pending_marks_follow_enter_to_a_new_unique_paragraph_and_atomic_undo() -> TestResult {
        let original = Document::plain("p", "AB")?;
        let mut session = EditorSession::new(original.clone(), Revision(7));
        session.set_selection(collapsed(point("p", 1)))?;
        session.toggle_mark(Mark::new("italic"))?;
        replace(&mut session, "X\nY")?;
        let blocks = session.document().blocks();
        assert_eq!(blocks.len(), 2);
        assert_ne!(blocks[0].block_id(), blocks[1].block_id());
        assert_eq!(blocks[0].inline_text(), "AX");
        assert_eq!(blocks[1].inline_text(), "YB");
        assert_eq!(
            session
                .selection()
                .map(|selection| selection.head.block_id.as_str()),
            blocks[1].block_id()
        );
        assert_eq!(session.mark_state("italic")?, crate::MarkState::On);
        session.undo()?;
        assert_eq!(session.document(), &original);
        assert!(!session.can_undo());
        Ok(())
    }
    #[test]
    fn headings_and_multiline_code_have_real_typography_and_lossless_caret_maps() -> TestResult {
        let source = Document::from_json(
            serde_json::json!({"type":"doc","attrs":{"schemaVersion":4},"content":[
                {"type":"heading","attrs":{"blockId":"h1","level":1,"vendor":{"keep":true}},"content":[{"type":"text","text":"A heading"}]},
                {"type":"heading","attrs":{"blockId":"h2","level":2},"content":[{"type":"text","text":"Second heading"}]},
                {"type":"heading","attrs":{"blockId":"h3","level":3},"content":[{"type":"text","text":"Third heading"}]},
                {"type":"paragraph","attrs":{"blockId":"p"},"content":[{"type":"text","text":"Body text"}]},
                {"type":"codeBlock","attrs":{"blockId":"code","language":"rust"},"content":[{"type":"text","text":"let a = 1;\nlet b = 2;"}]}
            ]}),
        )?;
        let before = source.to_json()?;
        let geometry = FlowGeometry::with_style(
            &source,
            600.0,
            &|_| None,
            Some(&CanvasStyle::default().geometry()),
        )?;
        let h1 = geometry
            .caret(&point("h1", 1))?
            .ok_or(Error::InvalidSelection)?;
        let h2 = geometry
            .caret(&point("h2", 1))?
            .ok_or(Error::InvalidSelection)?;
        let h3 = geometry
            .caret(&point("h3", 1))?
            .ok_or(Error::InvalidSelection)?;
        let body = geometry
            .caret(&point("p", 1))?
            .ok_or(Error::InvalidSelection)?;
        assert!(h1.height > h2.height && h2.height > h3.height && h3.height > body.height);
        let code_start = geometry
            .caret(&point("code", 0))?
            .ok_or(Error::InvalidSelection)?;
        let code_end = geometry
            .caret(&point("code", 13))?
            .ok_or(Error::InvalidSelection)?;
        assert!(code_end.y > code_start.y);
        assert_eq!(
            geometry.hit(0, code_end.x + 0.01, code_end.y + code_end.height / 2.0)?,
            Some(point("code", 13))
        );
        assert_eq!(source.to_json()?, before);
        Ok(())
    }
    #[test]
    fn code_enter_keeps_one_code_block_and_exact_undo() -> TestResult {
        let original = Document::from_json(
            serde_json::json!({"type":"doc","attrs":{"schemaVersion":4},"content":[{"type":"codeBlock","attrs":{"blockId":"code","language":"rust"},"content":[{"type":"text","text":"ab"}]}]}),
        )?;
        let mut session = EditorSession::new(original.clone(), Revision(1));
        session.set_selection(collapsed(point("code", 1)))?;
        replace(&mut session, "\n")?;
        assert_eq!(session.document().blocks().len(), 1);
        assert_eq!(session.document().block_text("code")?, "a\nb");
        assert_eq!(
            session.selection().map(|selection| selection.head.offset),
            Some(ScalarOffset(2))
        );
        session.undo()?;
        assert_eq!(session.document(), &original);
        Ok(())
    }
}
