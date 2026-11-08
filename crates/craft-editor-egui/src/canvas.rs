//! Experimental continuous-flow native document canvas.
//!
//! WordCraft supplies measured glyphs, real table tracks and viewport rasterization. Canonical
//! commands, selection and history remain owned by [`EditorSession`]. This is a qualification
//! checkpoint: unsupported nodes fail explicitly; rich clipboard, full accessibility and native
//! platform IME acceptance are not implied by the headless tests.

use craft_editor_model::{Command, Document, Mark, MarkMode, Revision, ScalarOffset, TextRange};
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
        if (response.clicked() || response.drag_started() || response.dragged())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            response.request_focus();
            self.cancel_preedit(session);
            if let Err(error) = self.pointer(
                session,
                pointer - rect.min,
                ui.input(|i| i.modifiers.shift),
                response.dragged(),
                response.double_clicked(),
                response.triple_clicked(),
            ) {
                errors.push(error);
            }
        }
        let focused = ui.memory(|memory| memory.has_focus(session.options.id));
        if focused {
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
                anchor: geometry.line_edge(&point, false)?,
                head: geometry.line_edge(&point, true)?,
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
                    let mode = if selection_has_mark(session, kind)? {
                        MarkMode::Remove
                    } else {
                        MarkMode::Add
                    };
                    session.format_selection(Mark::new(kind), mode)?;
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

fn selection_has_mark(session: &EditorSession, kind: &str) -> Result<bool> {
    let Some(selection) = &session.selection else {
        return Ok(false);
    };
    let projection = Projection::new(session.document())?;
    let a = projection.scalar(&selection.anchor)?;
    let b = projection.scalar(&selection.head)?;
    let mut any = false;
    for span in &projection.spans {
        let from = a.min(b).max(span.start).saturating_sub(span.start);
        let to = a.max(b).min(span.end).saturating_sub(span.start);
        let mut offset = 0;
        for inline in span.node.children() {
            let end = offset + inline.inline_text().chars().count();
            if offset < to && end > from {
                any = true;
                if !inline
                    .marks
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .any(|mark| mark.mark_type == kind)
                {
                    return Ok(false);
                }
            }
            offset = end;
        }
    }
    Ok(any)
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
    let (buffer, caret) = replacement_buffer(session, text)?;
    let next_id = buffer.next_id;
    session.apply_commands(buffer.commands)?;
    session.next_id = next_id;
    session.selection = Some(collapsed(caret));
    if let Some(history) = session.undo_selection.last_mut() {
        history.after = session.selection.clone();
    }
    session.restore_cursor = false;
    Ok(())
}
