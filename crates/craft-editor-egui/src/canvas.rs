//! Experimental continuous-flow native document canvas.
//!
//! WordCraft supplies measured glyphs, real table tracks and viewport rasterization. Canonical
//! commands, selection and history remain owned by [`EditorSession`]. This is a qualification
//! checkpoint: unsupported nodes fail explicitly; rich clipboard, full accessibility and native
//! platform IME acceptance are not implied by the headless tests.

use craft_editor_model::{Document, Mark, PositionMap, Revision, ScalarOffset};
use egui::{Color32, Event, Key, Modifiers, Pos2, Rect, Sense, TextureHandle, Vec2};

use crate::{
    DocPoint, EditorSession, Error, Result, Selection, ViewResponse,
    layout::{FlowGeometry, GeometryRect, GeometryStyle},
    projection::Projection,
};

mod cells;

pub use crate::layout::LineHeightBasis;

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
    /// Reference used by the line-height multiple; the default retains backend metrics.
    pub line_height_basis: LineHeightBasis,
    /// Optional spacing between list items in logical points.
    pub list_item_spacing: Option<f32>,
    /// Optional standard-ligature override, independent of canonical marks.
    pub standard_ligatures: Option<bool>,
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
            line_height_basis: LineHeightBasis::FontMetrics,
            list_item_spacing: None,
            standard_ligatures: None,
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
            line_height_basis: self.line_height_basis,
            list_item_spacing: self.list_item_spacing,
            standard_ligatures: self.standard_ligatures,
            paragraph_spacing: self.paragraph_spacing,
            syntax_palette: Some(crate::syntax::Palette::for_surface(
                rgb(self.foreground),
                rgb(self.background),
            )),
        }
    }
}

struct Preedit {
    document: Document,
    caret: DocPoint,
    text: String,
    mappings: Vec<PositionMap>,
}
#[derive(PartialEq)]
struct MeasurementKey {
    revision: Revision,
    width: u32,
    style: GeometryStyle,
    preedit_epoch: u64,
    resolver_generation: Option<u64>,
    resolver_identity: Option<usize>,
    font_generation: u64,
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
    cell_drag: Option<crate::TableSelection>,
}

impl CanvasState {
    pub(crate) fn request_scroll_to_caret(&mut self) {
        self.scroll_to_caret = true;
    }
    pub(crate) fn caret_rect(&self, session: &EditorSession, origin: Pos2) -> Result<Option<Rect>> {
        let Some(geometry) = &self.geometry else {
            return Ok(None);
        };
        let cell_point = if session.cell_selection().is_some() {
            session.head_cell_text_point(false)?
        } else {
            None
        };
        let point = self
            .preedit
            .as_ref()
            .map(|preedit| &preedit.caret)
            .or_else(|| session.selection.as_ref().map(|selection| &selection.head))
            .or(cell_point.as_ref());
        let Some(point) = point else {
            return Ok(None);
        };
        Ok(geometry.caret(point)?.map(|rect| on_screen(origin, rect)))
    }
    #[cfg(test)]
    fn measure(&mut self, session: &EditorSession, width: f32, style: &CanvasStyle) -> Result<()> {
        self.measure_with_resolver(session, width, style, None)
    }
    fn measure_with_resolver(
        &mut self,
        session: &EditorSession,
        width: f32,
        style: &CanvasStyle,
        resolver: Option<&dyn crate::renderers::AtomResolver>,
    ) -> Result<()> {
        let key = MeasurementKey {
            revision: session.revision(),
            width: width.to_bits(),
            style: style.geometry(),
            preedit_epoch: self.preedit_epoch,
            resolver_generation: resolver.map(crate::renderers::AtomResolver::generation),
            resolver_identity: resolver.map(|resolver| {
                (resolver as *const dyn crate::renderers::AtomResolver).cast::<()>() as usize
            }),
            font_generation: crate::fonts::generation(),
        };
        if self.measured.as_ref() != Some(&key) {
            let document = self
                .preedit
                .as_ref()
                .map_or_else(|| session.document(), |p| &p.document);
            let geometry = if let Some(resolver) = resolver {
                FlowGeometry::with_resolver(document, width, &key.style, resolver)?
            } else {
                FlowGeometry::with_style(document, width, &|_| None, Some(&key.style))?
            };
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
        resolver: Option<&dyn crate::renderers::AtomResolver>,
        decorations: Option<&mut dyn crate::decorations::DecorationSource>,
    ) -> ViewResponse {
        let mut errors = Vec::new();
        let width = ui.available_width().max(144.0);
        if let Err(error) = self.measure_with_resolver(session, width, style, resolver) {
            return ViewResponse {
                response: ui
                    .allocate_response(Vec2::new(width, style.min_height.max(0.0)), Sense::hover()),
                changes: session.take_pending_changes(),
                errors: vec![error],
                activated_decorations: vec![],
            };
        }
        if session.selection.is_none()
            && session.node_selection.is_none()
            && session.table_selection.is_none()
            && let Some(geometry) = &self.geometry
        {
            match geometry.edge(false) {
                Ok(point) => {
                    if let Err(error) = session.set_selection(collapsed(point)) {
                        errors.push(error);
                    }
                }
                Err(error) => {
                    if let Some(node) = geometry.nodes().first() {
                        match session
                            .document()
                            .node_ref(&node.canonical_path)
                            .map_err(Error::from)
                            .and_then(|reference| session.select_node(reference))
                        {
                            Ok(()) => {}
                            Err(error) => errors.push(error),
                        }
                    } else {
                        errors.push(error.into());
                    }
                }
            }
        }
        let extent = self.geometry.as_ref().and_then(FlowGeometry::extent);
        let height = extent.map_or(style.min_height, |rect| rect.height.max(style.min_height));
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
        if let (Some(resolver), Some(geometry)) = (resolver, self.geometry.as_ref()) {
            let visible = rect.intersect(ui.clip_rect());
            let paths: Vec<_> = geometry
                .nodes()
                .into_iter()
                .filter(|node| on_screen(rect.min, node.rect).intersects(visible))
                .map(|node| node.canonical_path)
                .collect();
            resolver.prioritize(&paths);
            if geometry.placeholders().iter().any(|atom| {
                atom.placeholder.state == crate::renderers::AtomPlaceholderState::Pending
                    && on_screen(rect.min, atom.rect).intersects(visible)
            }) {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(50));
            }
        }
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
            if let Err(error) = self.measure_with_resolver(session, width, style, resolver) {
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
                if let Err(error) = self.measure_with_resolver(session, width, style, resolver) {
                    errors.push(error);
                    break;
                }
            }
        }
        if let Err(error) = self.measure_with_resolver(session, width, style, resolver) {
            errors.push(error);
        }
        ui.painter().rect_filled(rect, 0.0, style.background);
        if let Err(error) = self.paint(ui, rect, style, &response) {
            errors.push(error);
        }
        let mut activated_decorations = Vec::new();
        if let Some(source) = decorations {
            let proofs: Vec<_> = session
                .pending
                .iter()
                .map(|change| change.replay_proof())
                .collect();
            match source.decorations(session.document(), session.revision(), &proofs) {
                Ok(decorations) => {
                    if let Err(error) =
                        crate::decorations::validate(session.document(), &decorations)
                    {
                        errors.push(error);
                    } else if let Some(geometry) = &self.geometry {
                        for decoration in decorations {
                            let Some(range) = transient_decoration_range(
                                &decoration.range,
                                self.preedit.as_ref(),
                            ) else {
                                continue;
                            };
                            match geometry.selection_rects(&range) {
                                Ok(rectangles) => {
                                    let tint = Color32::from_rgba_unmultiplied(
                                        decoration.color.r(),
                                        decoration.color.g(),
                                        decoration.color.b(),
                                        decoration.color.a().min(65),
                                    );
                                    let hovered = response.hover_pos().is_some_and(|pointer| {
                                        rectangles.iter().any(|range| {
                                            on_screen(rect.min, *range).contains(pointer)
                                        })
                                    });
                                    for range in rectangles {
                                        let range = on_screen(rect.min, range);
                                        ui.painter().rect_filled(range, 1.0, tint);
                                        ui.painter().line_segment(
                                            [range.left_bottom(), range.right_bottom()],
                                            egui::Stroke::new(1.0, decoration.color),
                                        );
                                    }
                                    if hovered {
                                        response.clone().on_hover_text(&decoration.label);
                                        if response.clicked()
                                            && !ui.input(|input| input.modifiers.any())
                                        {
                                            activated_decorations.push(decoration.id);
                                        }
                                    }
                                }
                                Err(error) => errors.push(error.into()),
                            }
                        }
                    }
                }
                Err(error) => errors.push(error),
            }
            let visible = if self.preedit.is_none() {
                self.geometry.as_ref().map_or_else(Vec::new, |geometry| {
                    let clip = rect.intersect(ui.clip_rect());
                    geometry.visible_text_ranges(GeometryRect {
                        page: 0,
                        x: clip.left() - rect.left(),
                        y: clip.top() - rect.top(),
                        width: clip.width(),
                        height: clip.height(),
                    })
                })
            } else {
                vec![]
            };
            match source.underlines_in_view(
                session.document(),
                session.revision(),
                &proofs,
                &visible,
            ) {
                Ok(underlines) => {
                    if let Err(error) =
                        crate::decorations::validate_underlines(session.document(), &underlines)
                    {
                        errors.push(error);
                    } else if let Some(geometry) = &self.geometry {
                        for underline in underlines {
                            let Some(range) =
                                transient_decoration_range(&underline.range, self.preedit.as_ref())
                            else {
                                continue;
                            };
                            match geometry.selection_rects(&range) {
                                Ok(rectangles) => {
                                    let hovered = response.hover_pos().is_some_and(|pointer| {
                                        rectangles.iter().any(|range| {
                                            on_screen(rect.min, *range).contains(pointer)
                                        })
                                    });
                                    for range in rectangles {
                                        paint_passive_underline(
                                            ui,
                                            on_screen(rect.min, range),
                                            underline.color,
                                        );
                                    }
                                    if hovered && !underline.label.is_empty() {
                                        response.clone().on_hover_text(&underline.label);
                                    }
                                }
                                Err(error) => errors.push(error.into()),
                            }
                        }
                    }
                }
                Err(error) => errors.push(error),
            }
        }
        if self.preedit.is_none()
            && let Err(error) = self.paint_cells(session, ui, rect.min, style.selection)
        {
            errors.push(error);
        }
        if let Some(geometry) = &self.geometry {
            let selection = session.selection.as_ref();
            if let Some(reference) = &session.node_selection
                && let Some(node) = geometry
                    .nodes()
                    .iter()
                    .find(|node| node.canonical_path == reference.path)
            {
                let selected = on_screen(rect.min, node.rect);
                ui.painter().rect_stroke(
                    selected.expand(2.0),
                    3.0,
                    egui::Stroke::new(2.0, style.selection),
                    egui::StrokeKind::Outside,
                );
                if self.scroll_to_caret {
                    ui.scroll_to_rect(selected.expand(12.0), None);
                    self.scroll_to_caret = false;
                }
            }
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
            let cell_point = if session.cell_selection().is_some() {
                match session.head_cell_text_point(false) {
                    Ok(point) => point,
                    Err(error) => {
                        errors.push(error);
                        None
                    }
                }
            } else {
                None
            };
            let caret = self
                .preedit
                .as_ref()
                .map(|preedit| &preedit.caret)
                .or_else(|| selection.map(|s| &s.head))
                .or(cell_point.as_ref());
            if let Some(caret) = caret {
                match geometry.caret(caret) {
                    Ok(Some(caret)) => {
                        let caret_rect = on_screen(rect.min, caret);
                        if focused {
                            if session.cell_selection().is_none() || self.preedit.is_some() {
                                let x = caret_rect.left();
                                ui.painter().line_segment(
                                    [
                                        Pos2::new(x, caret_rect.top()),
                                        Pos2::new(x, caret_rect.bottom()),
                                    ],
                                    egui::Stroke::new(1.5, style.caret),
                                );
                            }
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
            changes: session.take_pending_changes(),
            errors,
            activated_decorations,
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
        if self.cell_pointer(session, local, shift, dragging, triple)? {
            self.goal_x = None;
            return Ok(());
        }
        let geometry = self.geometry.as_ref().ok_or(Error::InvalidSelection)?;
        if !shift
            && !dragging
            && let Some(path) = geometry.hit_node(0, local.x, local.y)
        {
            let reference = session.document().node_ref(&path)?;
            session.select_node(reference)?;
            self.goal_x = None;
            return Ok(());
        }
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
            Event::Copy
                if session.clipboard_mode() == crate::clipboard::ClipboardMode::Deferred =>
            {
                session.queue_clipboard_copy(false)?;
            }
            Event::Cut
                if !read_only
                    && session.clipboard_mode() == crate::clipboard::ClipboardMode::Deferred =>
            {
                session.queue_clipboard_copy(true)?;
            }
            Event::Paste(_)
                if !read_only
                    && self.preedit.is_none()
                    && session.clipboard_mode() == crate::clipboard::ClipboardMode::Deferred =>
            {
                session.queue_clipboard_paste()?;
            }
            Event::Copy => {
                if let Some(bundle) = session.copy_selection()? {
                    if let Some(provider) = &session.clipboard_provider {
                        provider.write(&bundle)?;
                    } else {
                        ui.ctx().copy_text(bundle.plain);
                    }
                }
            }
            Event::Cut if !read_only => {
                session.cut_to_clipboard()?;
                self.scroll_to_caret = true;
            }
            Event::Paste(text) if !read_only && self.preedit.is_none() => {
                if session.clipboard_provider.is_some() {
                    session.paste_from_clipboard()?;
                } else {
                    let max_bytes = text.len();
                    session.paste_bundle(
                        &crate::clipboard::ClipboardBundle {
                            plain: text,
                            ..Default::default()
                        },
                        max_bytes,
                    )?;
                }
                self.scroll_to_caret = true;
            }
            Event::Text(text) if !read_only && self.preedit.is_none() => {
                replace(session, &text.replace("\r\n", "\n"))?;
                self.scroll_to_caret = true;
            }
            Event::Ime(egui::ImeEvent::Preedit { text, .. }) if !read_only => {
                self.preedit_epoch = self.preedit_epoch.wrapping_add(1);
                if text.is_empty() {
                    self.cancel_preedit(session);
                } else {
                    let (document, caret, mappings) = preedit_document(session, &text)?;
                    self.preedit = Some(Preedit {
                        document,
                        caret,
                        text,
                        mappings,
                    });
                    session.canvas_composing = true;
                }
            }
            Event::Ime(egui::ImeEvent::Commit(text)) if !read_only => {
                self.cancel_preedit(session);
                if !text.is_empty() {
                    if session.node_selection.is_some() {
                        session.replace_selected_node(&text)?;
                    } else {
                        replace(session, &text)?;
                    }
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
                    if modifiers.shift && session.can_redo() {
                        session.redo()?;
                    } else if !modifiers.shift && session.can_undo() {
                        session.undo()?;
                    }
                    self.scroll_to_caret = true;
                    return Ok(());
                }
                Key::Y if !read_only => {
                    if session.can_redo() {
                        session.redo()?;
                    }
                    self.scroll_to_caret = true;
                    return Ok(());
                }
                Key::B | Key::I | Key::U if !read_only && session.node_selection.is_none() => {
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
        if session.cell_selection().is_some()
            && !read_only
            && matches!(key, Key::Backspace | Key::Delete)
        {
            session.delete_cell_selection()?;
            self.scroll_to_caret = true;
            return Ok(());
        }
        if session.cell_selection().is_some() && key == Key::Enter {
            if !read_only && !modifiers.shift && !modifiers.command {
                session.append_paragraph_in_head_cell()?;
                self.scroll_to_caret = true;
            }
            return Ok(());
        }
        if key == Key::Tab && !read_only && session.advance_table_cell(modifiers.shift)? {
            self.scroll_to_caret = true;
            return Ok(());
        }
        if self.cell_arrow(session, key, modifiers)? {
            return Ok(());
        }
        if session.node_selection.is_some()
            && !read_only
            && matches!(key, Key::Backspace | Key::Delete)
        {
            session.delete_selected_node()?;
            self.scroll_to_caret = true;
            return Ok(());
        }
        if !read_only && key == Key::Enter && session.node_selection.is_some() {
            session.insert_paragraph()?;
            self.scroll_to_caret = true;
            return Ok(());
        }
        let geometry = self.geometry.as_ref().ok_or(Error::InvalidSelection)?;
        if let Some(reference) = &session.node_selection {
            if let Some(node) = geometry
                .nodes()
                .iter()
                .find(|node| node.canonical_path == reference.path)
            {
                let before = matches!(key, Key::ArrowLeft | Key::ArrowUp | Key::Home);
                if matches!(
                    key,
                    Key::ArrowLeft
                        | Key::ArrowUp
                        | Key::Home
                        | Key::ArrowRight
                        | Key::ArrowDown
                        | Key::End
                ) {
                    let y = if before {
                        node.rect.y - 1.0
                    } else {
                        node.rect.y + node.rect.height + 1.0
                    };
                    if let Some(point) = geometry.hit(node.rect.page, node.rect.x, y)? {
                        session.set_selection(collapsed(point))?;
                        self.scroll_to_caret = true;
                    }
                }
            }
            return Ok(());
        }
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
            if modifiers.shift {
                session.insert_hard_break()?;
            } else {
                session.insert_paragraph()?;
            }
            self.scroll_to_caret = true;
            return Ok(());
        }
        if !read_only
            && key == Key::Tab
            && !Projection::new(session.document())?
                .spans
                .iter()
                .any(|span| span.id == selection.head.block_id && span.table)
        {
            let in_list = session.list_state(crate::ListKind::Bullet)? != crate::MarkState::Off
                || session.list_state(crate::ListKind::Ordered)? != crate::MarkState::Off;
            if in_list {
                if modifiers.shift {
                    session.outdent_list()?;
                } else {
                    session.indent_list()?;
                }
                self.scroll_to_caret = true;
                return Ok(());
            }
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

    fn paint(
        &mut self,
        ui: &egui::Ui,
        canvas: Rect,
        style: &CanvasStyle,
        canvas_response: &egui::Response,
    ) -> Result<()> {
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
        for pending in geometry.placeholders() {
            let bounds = on_screen(canvas.min, pending.rect);
            if !bounds.intersects(visible) {
                continue;
            }
            let painter = ui
                .painter()
                .with_clip_rect(bounds.intersect(ui.clip_rect()));
            let failed =
                pending.placeholder.state == crate::renderers::AtomPlaceholderState::Failed;
            let border = if failed {
                style.foreground
            } else {
                style.table_border
            };
            painter.rect_filled(bounds, 4.0, style.table_header.gamma_multiply(0.65));
            painter.rect_stroke(
                bounds,
                4.0,
                egui::Stroke::new(1.0, border),
                egui::StrokeKind::Inside,
            );
            let compact = bounds.height() < 48.0;
            let inset = if compact { 5.0 } else { 12.0 };
            let font_size = (style.font_size * 0.72).clamp(11.0, 14.0);
            if ui
                .ctx()
                .pointer_hover_pos()
                .is_some_and(|pointer| bounds.contains(pointer))
            {
                let tooltip = if pending.placeholder.detail.is_empty() {
                    pending.placeholder.label.clone()
                } else {
                    format!(
                        "{}\n{}",
                        pending.placeholder.label, pending.placeholder.detail
                    )
                };
                // Reuse the editor's existing response; no overlay widget may steal its selection
                // or pointer input. Compact status still has the full host-localized explanation.
                let _ = canvas_response.clone().on_hover_text(tooltip);
            }
            let label_text = if compact && bounds.width() < 96.0 {
                if failed { "!".into() } else { "…".into() }
            } else {
                pending.placeholder.label
            };
            let label = painter.layout(
                label_text,
                egui::FontId::proportional(font_size),
                style.foreground,
                (bounds.width() - inset * 2.0).max(1.0),
            );
            let top = bounds.top()
                + if compact {
                    (bounds.height() - label.size().y) * 0.5
                } else {
                    inset
                };
            let detail_top = top + label.size().y + 6.0;
            painter.galley(
                egui::pos2(bounds.left() + inset, top),
                label,
                style.foreground,
            );
            if !compact && !pending.placeholder.detail.is_empty() {
                let detail = painter.layout(
                    pending.placeholder.detail,
                    egui::FontId::monospace((font_size - 1.0).max(10.0)),
                    style.foreground,
                    (bounds.width() - inset * 2.0).max(1.0),
                );
                painter.galley(
                    egui::pos2(bounds.left() + inset, detail_top),
                    detail,
                    style.foreground,
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

// Purely visual mapping through the private IME shadow transaction. This never
// changes host metadata, publishes arbitrary maps or treats preedit as a save.
fn paint_passive_underline(ui: &egui::Ui, range: Rect, color: Color32) {
    let visible = range.intersect(ui.clip_rect());
    let baseline = range.bottom() - 1.0;
    if visible.width() <= 0.0
        || baseline < ui.clip_rect().top()
        || baseline > ui.clip_rect().bottom()
    {
        return;
    }
    let steps = (visible.width() / 2.0).ceil() as usize;
    let points = (0..=steps)
        .map(|step| {
            egui::pos2(
                (visible.left() + step as f32 * 2.0).min(visible.right()),
                baseline - if step % 2 == 0 { 0.0 } else { 1.5 },
            )
        })
        .collect::<Vec<_>>();
    ui.painter()
        .add(egui::Shape::line(points, egui::Stroke::new(1.0, color)));
}

fn transient_decoration_range(range: &Selection, preedit: Option<&Preedit>) -> Option<Selection> {
    let mut result = range.clone();
    if let Some(preedit) = preedit {
        for mapping in &preedit.mappings {
            let (id, offset) =
                mapping.map_existing_caret(&result.anchor.block_id, result.anchor.offset, true)?;
            result.anchor = DocPoint {
                block_id: id,
                offset,
            };
            let (id, offset) =
                mapping.map_existing_caret(&result.head.block_id, result.head.offset, false)?;
            result.head = DocPoint {
                block_id: id,
                offset,
            };
        }
    }
    (result.anchor != result.head).then_some(result)
}
fn preedit_document(
    session: &EditorSession,
    text: &str,
) -> Result<(Document, DocPoint, Vec<PositionMap>)> {
    let mut shadow = EditorSession::new(session.document().clone(), session.revision());
    shadow.selection.clone_from(&session.selection);
    shadow.node_selection.clone_from(&session.node_selection);
    shadow.table_selection.clone_from(&session.table_selection);
    shadow.max_table_columns = session.max_table_columns;
    shadow.stored_marks.clone_from(&session.stored_marks);
    shadow.next_id = session.next_id;
    shadow.insert_plain_text(text)?;
    let caret = shadow
        .selection
        .as_ref()
        .ok_or(Error::InvalidSelection)?
        .head
        .clone();
    let mappings = shadow
        .take_pending_changes()
        .into_iter()
        .flat_map(|change| change.mappings)
        .collect();
    Ok((shadow.document().clone(), caret, mappings))
}
fn replace(session: &mut EditorSession, text: &str) -> Result<()> {
    session.insert_plain_text(text).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_editor_model::{Command, MarkMode};
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
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[..3] != [29, 34, 42])
        );
        assert!(
            raster
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
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
    #[test]
    fn first_focus_command_b_and_typing_share_real_pending_mark_state() -> TestResult {
        let mut session = EditorSession::new(Document::plain("p", "")?, Revision(1));
        let context = egui::Context::default();
        context.memory_mut(|memory| memory.request_focus(session.options.id));
        let response = frame(
            &mut session,
            &context,
            vec![key(Key::B, Modifiers::COMMAND), Event::Text("Bold".into())],
            &CanvasStyle::default(),
        );
        assert!(response.errors.is_empty(), "{:?}", response.errors);
        assert_eq!(response.changes.len(), 1);
        assert_eq!(session.document().block_text("p")?, "Bold");
        assert_eq!(session.mark_state("bold")?, crate::MarkState::On);
        session.undo()?;
        assert_eq!(session.document().block_text("p")?, "");
        let empty_undo = frame(
            &mut session,
            &context,
            vec![key(Key::Z, Modifiers::COMMAND)],
            &CanvasStyle::default(),
        );
        assert!(empty_undo.errors.is_empty(), "{:?}", empty_undo.errors);
        Ok(())
    }
    #[test]
    fn canonical_hard_break_uses_native_line_break_and_exact_unicode_positions() -> TestResult {
        let source = Document::from_json(
            serde_json::json!({"type":"doc","attrs":{"schemaVersion":4},"content":[{"type":"paragraph","attrs":{"blockId":"p"},"content":[{"type":"text","text":"Я"},{"type":"hardBreak"},{"type":"text","text":"😀Z"}]}]}),
        )?;
        let geometry = FlowGeometry::new(&source, 500.0)?;
        let first = geometry
            .caret(&point("p", 0))?
            .ok_or(Error::InvalidSelection)?;
        let after = geometry
            .caret(&point("p", 3))?
            .ok_or(Error::InvalidSelection)?;
        assert!(after.y > first.y);
        assert_eq!(
            geometry.hit(0, after.x + 0.01, after.y + after.height / 2.0)?,
            Some(point("p", 3))
        );
        assert_eq!(source.block_text("p")?, "Я\n😀Z");
        Ok(())
    }
    #[test]
    fn structural_delete_records_surviving_caret_for_undo_and_redo() -> TestResult {
        let original = fixture()?;
        let mut session = EditorSession::new(original.clone(), Revision(3));
        let before = collapsed(point("corner", 1));
        let after = collapsed(point("bottom", 2));
        session.set_selection(before.clone())?;
        let target = session
            .document()
            .node_ref(&craft_editor_model::NodePath(vec![1]))?;
        let command = craft_editor_model::delete_table_row(session.document(), &target, 2)?;
        session.apply_commands_with_selection(vec![command], after.clone())?;
        assert_eq!(session.selection(), Some(&after));
        assert!(session.document().block("corner").is_err());
        assert!(session.canvas_state.scroll_to_caret);
        session.undo()?;
        assert_eq!(session.document(), &original);
        assert_eq!(session.selection(), Some(&before));
        session.redo()?;
        assert_eq!(session.selection(), Some(&after));
        Ok(())
    }
    #[test]
    fn invalid_post_edit_caret_rejects_structural_transaction_atomically() -> TestResult {
        let original = fixture()?;
        let mut session = EditorSession::new(original.clone(), Revision(3));
        let selection = collapsed(point("corner", 1));
        session.set_selection(selection.clone())?;
        let target = session
            .document()
            .node_ref(&craft_editor_model::NodePath(vec![1]))?;
        let command = craft_editor_model::delete_table_row(session.document(), &target, 2)?;
        assert!(matches!(
            session.apply_commands_with_selection(vec![command], selection.clone()),
            Err(Error::InvalidSelection)
        ));
        assert_eq!(session.document(), &original);
        assert_eq!(session.revision(), Revision(3));
        assert_eq!(session.selection(), Some(&selection));
        assert!(!session.can_undo());
        assert!(session.pending.is_empty());
        Ok(())
    }
    #[test]
    fn inserted_table_enters_new_cell_and_redo_restores_that_caret() -> TestResult {
        let original = fixture()?;
        let mut session = EditorSession::new(original.clone(), Revision(3));
        let before = collapsed(point("intro", 3));
        let after = collapsed(point("new-cell", 0));
        session.set_selection(before.clone())?;
        let table = craft_editor_model::Node::block(
            "table",
            serde_json::Map::new(),
            vec![craft_editor_model::Node::block(
                "tableRow",
                serde_json::Map::new(),
                vec![craft_editor_model::empty_table_cell("new-cell", false)],
            )],
        );
        let parent = session
            .document()
            .node_ref(&craft_editor_model::NodePath::root())?;
        session.apply_commands_with_selection(
            vec![Command::InsertChildren {
                parent,
                index: 2,
                children: vec![table],
            }],
            after.clone(),
        )?;
        assert_eq!(session.selection(), Some(&after));
        session.undo()?;
        assert_eq!(session.document(), &original);
        assert_eq!(session.selection(), Some(&before));
        session.redo()?;
        assert_eq!(session.selection(), Some(&after));
        Ok(())
    }
    struct ContractResolver;
    impl crate::renderers::AtomResolver for ContractResolver {
        fn resolve(
            &self,
            node: &craft_editor_model::Node,
            _: f32,
        ) -> Result<crate::renderers::ResolvedAtom> {
            Ok(crate::renderers::ResolvedAtom {
                size: crate::layout::AtomSize {
                    width: 48.0,
                    height: 24.0,
                },
                baseline: (node.node_type == "inlineMath").then_some(18.0),
                encoded: std::sync::Arc::new(
                    include_bytes!("../examples/atom_contract_pixel.png").to_vec(),
                ),
            })
        }
        fn generation(&self) -> u64 {
            1
        }
    }
    #[test]
    fn all_schema_nodes_have_native_geometry_with_explicit_contract_atom_pixels() -> TestResult {
        // This deterministic pixel fixture verifies the resolver boundary, not TeX/Mermaid
        // correctness. The separate renderer crate qualifies actual diagram/formula output.
        let source = Document::from_json(serde_json::from_str(include_str!(
            "../examples/all_nodes_fixture.json"
        ))?)?;
        let before = source.to_json()?;
        let geometry = FlowGeometry::with_resolver(
            &source,
            650.0,
            &CanvasStyle::default().geometry(),
            &ContractResolver,
        )?;
        assert_eq!(geometry.page_count(), 1);
        assert_eq!(geometry.nodes().len(), 6); // inline math, rule, scene, image, diagram, block math
        assert_eq!(geometry.cells()?.len(), 6);
        let base = geometry
            .caret(&point("last", 0))?
            .ok_or(Error::InvalidSelection)?;
        let quote = geometry
            .caret(&point("quoted", 0))?
            .ok_or(Error::InvalidSelection)?;
        let nested = geometry
            .caret(&point("nested", 0))?
            .ok_or(Error::InvalidSelection)?;
        assert!(quote.x > base.x && nested.x > quote.x);
        assert_eq!(source.to_json()?, before);
        Ok(())
    }
    #[test]
    fn anonymous_node_delete_and_redo_preserve_exact_node_selection() -> TestResult {
        let original = Document::from_json(
            serde_json::json!({"type":"doc","attrs":{"schemaVersion":4},"content":[{"type":"horizontalRule"}]}),
        )?;
        let mut session = EditorSession::new(original.clone(), Revision(3));
        let reference = session
            .document()
            .node_ref(&craft_editor_model::NodePath(vec![0]))?;
        session.select_node(reference.clone())?;
        session.delete_selected_node()?;
        assert_eq!(session.document().blocks()[0].node_type, "paragraph");
        assert!(session.node_selection().is_none());
        assert!(session.selection().is_some());
        session.undo()?;
        assert_eq!(session.document(), &original);
        assert_eq!(session.node_selection(), Some(&reference));
        session.redo()?;
        assert!(session.node_selection().is_none());
        Ok(())
    }
    #[test]
    fn selected_inline_formula_type_replacement_is_one_rich_transaction() -> TestResult {
        let original = Document::from_json(
            serde_json::json!({"type":"doc","attrs":{"schemaVersion":4},"content":[{"type":"paragraph","attrs":{"blockId":"p"},"content":[{"type":"text","text":"A"},{"type":"inlineMath","attrs":{"latex":"x^2","vendor":9}},{"type":"text","text":"B"}]}]}),
        )?;
        let mut session = EditorSession::new(original.clone(), Revision(3));
        let reference = session
            .document()
            .node_ref(&craft_editor_model::NodePath(vec![0, 1]))?;
        session.select_node(reference.clone())?;
        session.replace_selected_node("字")?;
        assert_eq!(session.document().block_text("p")?, "A字B");
        assert_eq!(session.revision(), Revision(4));
        session.undo()?;
        assert_eq!(session.document(), &original);
        assert_eq!(session.node_selection(), Some(&reference));
        Ok(())
    }
    #[test]
    fn selected_atom_preedit_is_transient_and_commits_one_exact_replacement() -> TestResult {
        let original = Document::from_json(
            serde_json::json!({"type":"doc","attrs":{"schemaVersion":4},"content":[{"type":"paragraph","attrs":{"blockId":"p"},"content":[{"type":"text","text":"A"},{"type":"inlineMath","attrs":{"latex":"x^2"}},{"type":"text","text":"B"}]}]}),
        )?;
        let mut session = EditorSession::new(original.clone(), Revision(0));
        let selected = session
            .document()
            .node_ref(&craft_editor_model::NodePath(vec![0, 1]))?;
        session.select_node(selected.clone())?;
        let context = egui::Context::default();
        context.memory_mut(|memory| memory.request_focus(session.options.id));
        let run = |session: &mut EditorSession, event: Event| {
            let mut result = None;
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(700.0, 650.0))),
                    events: vec![event],
                    focused: true,
                    ..Default::default()
                },
                |root| {
                    egui::CentralPanel::default().show(root, |ui| {
                        result = Some(session.show_canvas_with_resolver(
                            ui,
                            &CanvasStyle::default(),
                            &ContractResolver,
                        ));
                    });
                },
            );
            output.textures_delta.clear();
            result.ok_or("missing frame")
        };
        let preedit = run(
            &mut session,
            Event::Ime(egui::ImeEvent::Preedit {
                text: "ni".into(),
                active_range_chars: Some(0..2),
            }),
        )?;
        assert!(preedit.errors.is_empty(), "{:?}", preedit.errors);
        assert!(preedit.changes.is_empty());
        assert!(session.is_composing());
        assert_eq!(session.document(), &original);
        assert_eq!(session.node_selection(), Some(&selected));
        let committed = run(
            &mut session,
            Event::Ime(egui::ImeEvent::Commit("字".into())),
        )?;
        assert!(committed.errors.is_empty(), "{:?}", committed.errors);
        assert_eq!(committed.changes.len(), 1);
        assert!(!session.is_composing());
        assert_eq!(session.document().block_text("p")?, "A字B");
        session.undo()?;
        assert_eq!(session.document(), &original);
        assert_eq!(session.node_selection(), Some(&selected));
        Ok(())
    }
}
