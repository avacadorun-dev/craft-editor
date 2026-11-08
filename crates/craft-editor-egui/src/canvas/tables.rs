//! Native local scrolling over the shared table viewport projection.
use super::*;

impl CanvasState {
    pub(super) fn table_scroll_input(&mut self, ui: &mut egui::Ui, canvas: Rect) -> Result<bool> {
        let pointer = ui.input(|input| input.pointer.interact_pos());
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
                    response.widget_info(|| {
                        egui::WidgetInfo::slider(true, f64::from(offset), &style.table_scroll_label)
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
