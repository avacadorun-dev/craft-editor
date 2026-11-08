//! Hosted qualification of a pinned native continuous-flow geometry backend.
//!
//! This optional module is a layout experiment, not an editor or accepted renderer. The
//! canonical model remains authoritative; the upstream document is a disposable projection.
//! Source: WordCraft `da914ce674f18ebc4e08cca425cc8f8acca3372f` (MIT OR Apache-2.0).
//! No upstream source is copied here. Git dependencies retain their own font/license files.

use std::{collections::HashMap, sync::Arc};

use craft_editor_model::{
    ByteOffset, Document, Node, NodePath, ScalarOffset, byte_to_scalar, scalar_to_byte,
};
use serde_json::{Value, json};
use wordcraft_doc::{
    Block, Blocks, CharProps, InlineObject, Paragraph, Path, Pos, StoryRef, Table, para_block,
};
use wordcraft_layout::{DocLayout, LayoutCache, LayoutOptions, Placed, ViewMode};

use crate::{DocPoint, Selection};

/// Upstream revision used by the hosted geometry experiment.
pub const UPSTREAM_REVISION: &str = "da914ce674f18ebc4e08cca425cc8f8acca3372f";

/// Content-free geometry diagnostic. A failure never rewrites the canonical document.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The canonical model rejected a position or document.
    #[error(transparent)]
    Model(#[from] craft_editor_model::Error),
    /// WordCraft refused the projected paragraph operation.
    #[error(transparent)]
    Upstream(#[from] wordcraft_doc::DocError),
    /// The geometry probe has no projection for this semantic node.
    #[error("unsupported_geometry_node: {0}")]
    UnsupportedNode(String),
    /// A stable block identity is needed for reverse coordinate mapping.
    #[error("missing_geometry_identity")]
    MissingIdentity,
    /// A geometry position does not resolve to an exact canonical scalar boundary.
    #[error("unmapped_geometry_position")]
    UnmappedPosition,
    /// Invalid track overlap or unsupported rowspan extends outside its table.
    #[error("invalid_geometry_grid")]
    InvalidGrid,
    /// The backend cannot faithfully lay out this valid canonical table.
    #[error("backend_column_limit: requested={requested}, maximum={maximum}")]
    BackendColumnLimit {
        /// Effective grid columns required by the canonical document.
        requested: usize,
        /// The pinned upstream engine's actual limit.
        maximum: usize,
    },
    /// Width or host-supplied atom geometry is nonfinite or nonpositive.
    #[error("invalid_geometry_size")]
    InvalidSize,
    /// A native geometry invariant failed; the string names the invariant, not manuscript text.
    #[error("geometry_qualification_failed: {0}")]
    Qualification(&'static str),
}
/// Fallible geometry experiment result.
pub type Result<T> = std::result::Result<T, Error>;

/// Dimensions supplied by an actual atom renderer, in document points.
///
/// The geometry backend only reserves this rectangle. It does not render TeX/Mermaid/images.
#[derive(Debug, Clone, Copy)]
pub struct AtomSize {
    /// Reserved width.
    pub width: f32,
    /// Reserved height.
    pub height: f32,
}

/// Host presentation applied only to the disposable layout projection.
#[derive(Debug, Clone, PartialEq)]
pub struct GeometryStyle {
    /// Body text size in logical document points.
    pub font_size: f32,
    /// Backend-registered font family; an absent value uses the backend's font defaults.
    pub font_family: Option<String>,
    /// Text color for source marks without their own color, as RGB bytes.
    pub text_color: [u8; 3],
    /// Host table grid color.
    pub table_border: [u8; 3],
    /// Host header-cell fill.
    pub table_header: [u8; 3],
    /// Multiple of single line spacing.
    pub line_height: f32,
    /// Space after a paragraph, in logical document points.
    pub paragraph_spacing: f32,
}
impl Default for GeometryStyle {
    fn default() -> Self {
        Self {
            font_size: 18.0,
            font_family: None,
            text_color: [35, 39, 47],
            table_border: [139, 148, 163],
            table_header: [234, 238, 244],
            line_height: 1.5,
            paragraph_spacing: 10.0,
        }
    }
}

/// A native geometry rectangle in document points, with continuous-flow page identity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeometryRect {
    /// Upstream page index; Web view should produce one continuous page.
    pub page: usize,
    /// Left coordinate.
    pub x: f32,
    /// Top coordinate.
    pub y: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}
impl GeometryRect {
    fn from_rect(page: usize, rect: wordcraft_geom::Rect) -> Self {
        Self {
            page,
            x: rect.x,
            y: rect.y,
            width: rect.w,
            height: rect.h,
        }
    }
}

/// A real laid-out table-cell box, including covered vertical-merge continuation boxes.
#[derive(Debug, Clone)]
pub struct CellGeometry {
    /// Canonical path to the originating cell. Covered rows map to the original merged cell.
    pub canonical_path: NodePath,
    /// True for an upstream vertical-merge continuation, not a separately editable cell.
    pub covered: bool,
    /// Actual table-cell rectangle.
    pub rect: GeometryRect,
}

struct BlockMap {
    id: String,
    path: Path,
    text: String,
}
#[derive(Clone)]
struct ActiveSpan {
    remaining: usize,
    width: usize,
    origin: NodePath,
}

/// A disposable, measured WordCraft projection with explicit canonical scalar/UTF-8 maps.
///
/// Experimental coverage is limited to the probe's styled text paragraphs,
/// table tracks/colspans/rowspans and host-measured inline atoms. Other blocks return errors.
/// This type does not provide native input, painting, semantic editing or full schema parity.
pub struct FlowGeometry {
    upstream: wordcraft_doc::Document,
    measured: DocLayout,
    blocks: Vec<BlockMap>,
    cell_paths: HashMap<(Path, usize, usize), NodePath>,
}

impl FlowGeometry {
    /// Measure supported content without changing the canonical model.
    /// Inline atoms require [`Self::with_atom_sizes`] and otherwise fail explicitly.
    pub fn new(source: &Document, width: f32) -> Result<Self> {
        Self::with_atom_sizes(source, width, &|_| None)
    }

    /// Measure content using host-resolved inline atom sizes; no atom renderer is invented.
    pub fn with_atom_sizes(
        source: &Document,
        width: f32,
        atom_size: &impl Fn(&Node) -> Option<AtomSize>,
    ) -> Result<Self> {
        Self::with_style(source, width, atom_size, None)
    }

    /// Measure a disposable view with host typography, never changing canonical attributes.
    pub fn with_style(
        source: &Document,
        width: f32,
        atom_size: &impl Fn(&Node) -> Option<AtomSize>,
        style: Option<&GeometryStyle>,
    ) -> Result<Self> {
        if !width.is_finite() || width <= 0.0 {
            return Err(Error::InvalidSize);
        }
        if style.is_some_and(|s| {
            !s.font_size.is_finite()
                || s.font_size <= 0.0
                || !s.line_height.is_finite()
                || s.line_height <= 0.0
                || !s.paragraph_spacing.is_finite()
                || s.paragraph_spacing < 0.0
        }) {
            return Err(Error::InvalidSize);
        }
        let mut builder = Builder {
            blocks: Vec::new(),
            cell_paths: HashMap::new(),
            atom_size,
            style,
        };
        // Web flow reserves 18 points on each side; tracks occupy the actual text column.
        let body = builder.blocks(
            source.root().children(),
            &[],
            &[],
            (width.clamp(144.0, 20000.0) - 36.0).max(1.0),
        )?;
        let mut upstream = wordcraft_doc::Document::new();
        upstream.body = body;
        if let Some(style) = style {
            let sheet = Arc::make_mut(&mut upstream.styles);
            sheet.default_chr.size = Some(style.font_size);
            sheet.default_chr.font.clone_from(&style.font_family);
            sheet.default_chr.color = Some(wordcraft_doc::props::TextColor::Rgb(
                wordcraft_doc::props::Rgb(
                    style.text_color[0],
                    style.text_color[1],
                    style.text_color[2],
                ),
            ));
            sheet.default_para.line_spacing = Some(wordcraft_doc::props::LineSpacing::Multiple(
                style.line_height,
            ));
            sheet.default_para.space_after = Some(style.paragraph_spacing);
        }
        let options = LayoutOptions {
            view: ViewMode::Web,
            web_width: width,
            proofing: false,
            ..Default::default()
        };
        let measured = wordcraft_layout::layout(&upstream, &mut LayoutCache::new(), &options);
        Ok(Self {
            upstream,
            measured,
            blocks: builder.blocks,
            cell_paths: builder.cell_paths,
        })
    }

    /// Complete continuous-flow extent in document points.
    pub fn extent(&self) -> Option<GeometryRect> {
        self.measured.pages.first().map(|page| GeometryRect {
            page: 0,
            x: 0.0,
            y: 0.0,
            width: page.w,
            height: page.h,
        })
    }

    /// First or last exact canonical text position in document order.
    pub fn edge(&self, end: bool) -> Result<DocPoint> {
        let native = if end {
            self.upstream.end_of(StoryRef::Body)
        } else {
            self.upstream.start_of(StoryRef::Body)
        };
        self.resolve_native(&native)
    }

    /// Logical grapheme/word motion with shaped visual movement inside a line.
    pub fn horizontal(&self, point: &DocPoint, left: bool, word: bool) -> Result<DocPoint> {
        let mut pos = self.to_native(point)?;
        let paragraph = self.upstream.para_at(&pos).ok_or(Error::UnmappedPosition)?;
        if !word
            && let Some(wordcraft_layout::hit::VisualStep::Moved(moved)) =
                self.measured.visual_step(&pos, left, 0)
        {
            return self.resolve_native(&moved);
        }
        let next = if word {
            if left {
                paragraph.word_start(pos.off)
            } else {
                paragraph.word_end(pos.off)
            }
        } else if left {
            paragraph.prev_boundary(pos.off)
        } else {
            paragraph.next_boundary(pos.off)
        };
        if next == pos.off {
            let neighbor = if left {
                self.upstream.prev_para(pos.story, &pos.path)
            } else {
                self.upstream.next_para(pos.story, &pos.path)
            };
            if let Some(path) = neighbor {
                pos.path = path;
                pos.off = if left {
                    self.upstream
                        .para_at(&pos)
                        .ok_or(Error::UnmappedPosition)?
                        .len()
                } else {
                    0
                };
            }
        } else {
            pos.off = next;
        }
        self.resolve_native(&pos)
    }

    /// Previous native Backspace or next Delete boundary, respecting shaped grapheme stops.
    pub fn deletion_boundary(&self, point: &DocPoint, backward: bool) -> Result<DocPoint> {
        let mut pos = self.to_native(point)?;
        let paragraph = self.upstream.para_at(&pos).ok_or(Error::UnmappedPosition)?;
        let off = if backward {
            paragraph.backspace_from(pos.off)
        } else {
            paragraph.next_boundary(pos.off)
        };
        if off == pos.off {
            return self.horizontal(point, backward, false);
        }
        pos.off = off;
        self.resolve_native(&pos)
    }

    /// Physical previous/next line, preserving a goal x across table-cell geometry.
    pub fn vertical(
        &self,
        point: &DocPoint,
        goal_x: Option<f32>,
        direction: i32,
    ) -> Result<Option<(DocPoint, f32)>> {
        self.measured
            .vertical(&self.to_native(point)?, goal_x, direction, 0)
            .map(|(pos, x)| Ok((self.resolve_native(&pos)?, x)))
            .transpose()
    }

    /// Beginning or end of the physical line containing the caret.
    pub fn line_edge(&self, point: &DocPoint, end: bool) -> Result<DocPoint> {
        let (first, last) = self
            .measured
            .line_bounds(&self.to_native(point)?, 0)
            .ok_or(Error::UnmappedPosition)?;
        self.resolve_native(if end { &last } else { &first })
    }

    /// Word under a point, used for a native double-click selection.
    pub fn word_at(&self, point: &DocPoint) -> Result<Selection> {
        let pos = self.to_native(point)?;
        let paragraph = self.upstream.para_at(&pos).ok_or(Error::UnmappedPosition)?;
        let (start, end) = paragraph.word_at(pos.off);
        Ok(Selection {
            anchor: self.resolve_native(&Pos {
                off: start,
                ..pos.clone()
            })?,
            head: self.resolve_native(&Pos { off: end, ..pos })?,
        })
    }

    /// Start of the previous/next distinct table cell, skipping covered continuations.
    pub fn adjacent_cell(&self, point: &DocPoint, previous: bool) -> Result<Option<DocPoint>> {
        let pos = self.to_native(point)?;
        if pos.path.0.len() < 4 {
            return Ok(None);
        }
        let cell = &pos.path.0[..3];
        let index = self
            .blocks
            .iter()
            .position(|block| block.path == pos.path)
            .ok_or(Error::UnmappedPosition)?;
        let block = if previous {
            self.blocks[..index].iter().rev().find(|block| {
                block.path.0.len() >= 4 && block.path.0[0] == cell[0] && block.path.0[..3] != *cell
            })
        } else {
            self.blocks[index + 1..].iter().find(|block| {
                block.path.0.len() >= 4 && block.path.0[0] == cell[0] && block.path.0[..3] != *cell
            })
        };
        Ok(block.map(|block| DocPoint {
            block_id: block.id.clone(),
            offset: ScalarOffset(0),
        }))
    }

    /// Rasterize only a bounded viewport using the same measured glyphs and table geometry.
    #[cfg(feature = "wordcraft-canvas")]
    pub fn rasterize(
        &self,
        area: GeometryRect,
        scale: f32,
        paper: [u8; 3],
    ) -> Result<wordcraft_render::Rendered> {
        if area.page >= self.measured.pages.len()
            || !area.x.is_finite()
            || !area.y.is_finite()
            || !area.width.is_finite()
            || !area.height.is_finite()
            || area.width <= 0.0
            || area.height <= 0.0
            || !scale.is_finite()
            || scale <= 0.0
            || area.width * scale > 4096.0
            || area.height * scale > 4096.0
        {
            return Err(Error::InvalidSize);
        }
        let options = wordcraft_render::RenderOptions {
            paper: wordcraft_doc::props::Rgb(paper[0], paper[1], paper[2]),
            ..Default::default()
        };
        let rendered = wordcraft_render::render_area(
            &self.upstream,
            &self.measured.pages[area.page],
            wordcraft_geom::Rect::new(area.x, area.y, area.width, area.height),
            scale,
            &options,
        );
        if rendered.width == 0
            || rendered.height == 0
            || rendered.pixels.len() != rendered.width as usize * rendered.height as usize * 4
        {
            return Err(Error::Qualification("raster_buffer"));
        }
        Ok(rendered)
    }

    /// Number of upstream pages; continuous Web flow is qualified as one page.
    pub fn page_count(&self) -> usize {
        self.measured.pages.len()
    }

    /// Convert an exact semantic point to a native byte position and caret rectangle.
    pub fn caret(&self, point: &DocPoint) -> Result<Option<GeometryRect>> {
        Ok(self
            .measured
            .caret(&self.to_native(point)?)
            .map(|caret| GeometryRect {
                page: caret.page,
                x: caret.x,
                y: caret.top,
                width: caret.width,
                height: caret.height,
            }))
    }

    /// Resolve native hit testing to an exact canonical block/scalar point.
    pub fn hit(&self, page: usize, x: f32, y: f32) -> Result<Option<DocPoint>> {
        self.measured
            .hit(page, x, y, StoryRef::Body)
            .map(|pos| self.resolve_native(&pos))
            .transpose()
    }

    /// Resolve cross-block and cross-table selection geometry in one document coordinate space.
    pub fn selection_rects(&self, selection: &Selection) -> Result<Vec<GeometryRect>> {
        let anchor = self.to_native(&selection.anchor)?;
        let head = self.to_native(&selection.head)?;
        Ok(self
            .measured
            .selection_rects(&self.upstream, &anchor, &head, 0)
            .into_iter()
            .map(|(page, rect)| GeometryRect::from_rect(page, rect))
            .collect())
    }

    /// Return actual cell boxes emitted by the table layout engine.
    pub fn cells(&self) -> Result<Vec<CellGeometry>> {
        let mut cells = Vec::new();
        for (page, layout) in self.measured.pages.iter().enumerate() {
            for item in &layout.items {
                if let Placed::Cell {
                    table,
                    row,
                    cell,
                    rect,
                    covered,
                    ..
                } = item
                {
                    let canonical_path = self
                        .cell_paths
                        .get(&(table.clone(), *row, *cell))
                        .cloned()
                        .ok_or(Error::UnmappedPosition)?;
                    cells.push(CellGeometry {
                        canonical_path,
                        covered: *covered,
                        rect: GeometryRect::from_rect(page, *rect),
                    });
                }
            }
        }
        Ok(cells)
    }

    /// Reserved inline atom rectangles emitted by the real native display-list pass.
    /// Only untransformed probe atoms are qualified here; painting is a separate host service.
    pub fn atom_rects(&self) -> Vec<GeometryRect> {
        use wordcraft_layout::display::{DisplayOptions, Draw, page_display};
        self.measured
            .pages
            .iter()
            .enumerate()
            .flat_map(|(page, layout)| {
                page_display(&self.upstream, layout, &DisplayOptions::default())
                    .into_iter()
                    .filter_map(move |draw| match draw {
                        Draw::Image { rect, media, .. }
                            if media.starts_with("craft-geometry-atom:") =>
                        {
                            Some(GeometryRect::from_rect(page, rect))
                        }
                        _ => None,
                    })
            })
            .collect()
    }

    fn to_native(&self, point: &DocPoint) -> Result<Pos> {
        let block = self
            .blocks
            .iter()
            .find(|block| block.id == point.block_id)
            .ok_or(Error::UnmappedPosition)?;
        let offset = scalar_to_byte(&block.text, point.offset)?;
        Ok(Pos::new(StoryRef::Body, block.path.clone(), offset.0))
    }

    fn resolve_native(&self, pos: &Pos) -> Result<DocPoint> {
        if pos.story != StoryRef::Body {
            return Err(Error::UnmappedPosition);
        }
        let block = self
            .blocks
            .iter()
            .find(|block| block.path == pos.path)
            .ok_or(Error::UnmappedPosition)?;
        let offset = byte_to_scalar(&block.text, ByteOffset(pos.off))?;
        Ok(DocPoint {
            block_id: block.id.clone(),
            offset,
        })
    }
}

struct Builder<'a, F> {
    blocks: Vec<BlockMap>,
    cell_paths: HashMap<(Path, usize, usize), NodePath>,
    atom_size: &'a F,
    style: Option<&'a GeometryStyle>,
}
impl<F: Fn(&Node) -> Option<AtomSize>> Builder<'_, F> {
    fn blocks(
        &mut self,
        nodes: &[Node],
        native_parent: &[u32],
        canonical_parent: &[usize],
        width: f32,
    ) -> Result<Blocks> {
        nodes
            .iter()
            .enumerate()
            .map(|(index, node)| {
                let mut native_path = native_parent.to_vec();
                native_path.push(u32::try_from(index).map_err(|_| Error::InvalidGrid)?);
                let mut canonical_path = canonical_parent.to_vec();
                canonical_path.push(index);
                match node.node_type.as_str() {
                    "paragraph" | "heading" | "codeBlock" => {
                        self.paragraph(node, Path(native_path))
                    }
                    "table" if native_path.len() == 1 => Ok(Arc::new(Block::Table(self.table(
                        node,
                        &native_path,
                        &canonical_path,
                        width,
                    )?))),
                    _ => Err(Error::UnsupportedNode(node.node_type.clone())),
                }
            })
            .collect()
    }

    fn paragraph(&mut self, node: &Node, path: Path) -> Result<Arc<Block>> {
        let id = node.block_id().ok_or(Error::MissingIdentity)?.to_owned();
        let mut paragraph = Paragraph::new();
        let body_size = self.style.map_or(18.0, |style| style.font_size);
        let mut inherited = CharProps::default();
        if node.node_type == "heading" {
            let level = integer_attr(node, "level").clamp(1, 6);
            inherited.size = Some(
                body_size
                    * match level {
                        1 => 1.75,
                        2 => 1.45,
                        3 => 1.25,
                        _ => 1.1,
                    },
            );
            inherited.bold = Some(true);
            paragraph.props.outline_level = Some((level - 1) as u8);
            paragraph.props.space_before = Some(body_size * 0.6);
            paragraph.props.keep_next = Some(true);
        } else if node.node_type == "codeBlock" {
            inherited.font = Some("JetBrains Mono".into());
            inherited.size = Some(body_size * 0.9);
            paragraph.props.line_spacing = Some(wordcraft_doc::props::LineSpacing::Multiple(1.4));
        }
        paragraph.props.align = match node.string_attr("textAlign") {
            Some("left") => Some(wordcraft_doc::props::Align::Left),
            Some("center") => Some(wordcraft_doc::props::Align::Center),
            Some("right") => Some(wordcraft_doc::props::Align::Right),
            Some("justify") => Some(wordcraft_doc::props::Align::Justify),
            _ => None,
        };
        for inline in node.children() {
            let mut props = inherited.clone();
            for mark in inline.marks.as_deref().unwrap_or_default() {
                match mark.mark_type.as_str() {
                    "bold" => props.bold = Some(true),
                    "italic" => props.italic = Some(true),
                    "underline" => props.underline = Some(wordcraft_doc::props::Underline::Single),
                    "strike" => props.strike = Some(true),
                    "code" => props.font = Some("JetBrains Mono".into()),
                    // Annotation/link interaction and decoration remain host duties. Their
                    // canonical attributes are never serialized through WordCraft.
                    _ => return Err(Error::UnsupportedNode(format!("mark:{}", mark.mark_type))),
                }
            }
            match inline.node_type.as_str() {
                "text" => {
                    let text = inline.text.as_deref().unwrap_or_default();
                    if text.contains('\u{fffc}') {
                        return Err(Error::UnsupportedNode("literal_object_scalar".into()));
                    }
                    let display = text.replace('\n', "\u{000b}");
                    paragraph.insert_text(paragraph.len(), &display, &props)?;
                }
                "hardBreak" => {
                    paragraph.insert_text(paragraph.len(), "\u{000b}", &props)?;
                }
                "inlineMath" => {
                    let size = (self.atom_size)(inline)
                        .ok_or_else(|| Error::UnsupportedNode("unmeasured_inline_atom".into()))?;
                    if !size.width.is_finite()
                        || !size.height.is_finite()
                        || size.width <= 0.0
                        || size.height <= 0.0
                    {
                        return Err(Error::InvalidSize);
                    }
                    let object = InlineObject::Image {
                        media: format!("craft-geometry-atom:{}:{}", id, paragraph.len()),
                        w: size.width,
                        h: size.height,
                        alt: String::new(),
                        float: Default::default(),
                        crop: [0.0; 4],
                        fx: Default::default(),
                        ole: None,
                    };
                    paragraph.insert_object(paragraph.len(), object, &props)?;
                }
                _ => return Err(Error::UnsupportedNode(inline.node_type.clone())),
            }
        }
        self.blocks.push(BlockMap {
            id,
            path,
            text: node.inline_text(),
        });
        Ok(para_block(paragraph))
    }

    fn table(
        &mut self,
        node: &Node,
        native_path: &[u32],
        canonical_path: &[usize],
        width: f32,
    ) -> Result<Table> {
        let maximum = wordcraft_doc::table::MAX_COLS;
        let mut active: Vec<Option<ActiveSpan>> = vec![None; maximum];
        let mut rows = Vec::new();
        let mut column_count = 0;
        for (row_index, source_row) in node.children().iter().enumerate() {
            let mut cells = Vec::new();
            let mut source_cells = source_row.children().iter().enumerate().peekable();
            let mut column = 0;
            while source_cells.peek().is_some() || active.iter().skip(column).any(Option::is_some) {
                if column >= maximum {
                    return Err(Error::BackendColumnLimit {
                        requested: column + 1,
                        maximum,
                    });
                }
                if let Some(span) = active[column].take() {
                    let mut covered = wordcraft_doc::Cell::empty();
                    covered.props.span = span.width as u32;
                    covered.props.vmerge = wordcraft_doc::props::VMerge::Continue;
                    self.cell_paths.insert(
                        (Path(native_path.to_vec()), row_index, cells.len()),
                        span.origin.clone(),
                    );
                    cells.push(covered);
                    if span.remaining > 1 {
                        active[column] = Some(ActiveSpan {
                            remaining: span.remaining - 1,
                            ..span.clone()
                        });
                    }
                    column += span.width;
                    continue;
                }
                let Some((cell_index, source_cell)) = source_cells.next() else {
                    return Err(Error::InvalidGrid);
                };
                let span = integer_attr(source_cell, "colspan");
                let rowspan = integer_attr(source_cell, "rowspan");
                let end = column.checked_add(span).ok_or(Error::InvalidGrid)?;
                if end > maximum {
                    return Err(Error::BackendColumnLimit {
                        requested: end,
                        maximum,
                    });
                }
                if active[column..end].iter().any(Option::is_some)
                    || row_index + rowspan > node.children().len()
                {
                    return Err(Error::InvalidGrid);
                }
                let mut source_path = canonical_path.to_vec();
                source_path.extend([row_index, cell_index]);
                let mut word_path = native_path.to_vec();
                word_path.extend([row_index as u32, cells.len() as u32]);
                let mut cell = wordcraft_doc::Cell::empty();
                cell.props.span = span as u32;
                if source_cell.node_type == "tableHeader"
                    && let Some(style) = self.style
                {
                    cell.props.shading = Some(rgb(style.table_header));
                }
                cell.blocks =
                    self.blocks(source_cell.children(), &word_path, &source_path, width)?;
                if rowspan > 1 {
                    cell.props.vmerge = wordcraft_doc::props::VMerge::Restart;
                    active[column] = Some(ActiveSpan {
                        remaining: rowspan - 1,
                        width: span,
                        origin: NodePath(source_path.clone()),
                    });
                }
                self.cell_paths.insert(
                    (Path(native_path.to_vec()), row_index, cells.len()),
                    NodePath(source_path),
                );
                cells.push(cell);
                column = end;
            }
            column_count = column_count.max(column);
            rows.push(wordcraft_doc::Row {
                cells,
                ..Default::default()
            });
        }
        if column_count == 0 || active.iter().any(Option::is_some) {
            return Err(Error::InvalidGrid);
        }
        let mut table = Table::new(1, column_count, width);
        table.rows = rows;
        table.props.fixed = true;
        table.props.width = Some(width);
        if let Some(style) = self.style {
            let mut border = wordcraft_doc::props::Border::single(0.75);
            border.color = Some(rgb(style.table_border));
            table.props.borders = Some(wordcraft_doc::props::Borders::all(border));
            table.props.cell_margins = Some([7.0, 9.0, 7.0, 9.0]);
        }
        Ok(table)
    }
}

fn rgb(value: [u8; 3]) -> wordcraft_doc::props::Rgb {
    wordcraft_doc::props::Rgb(value[0], value[1], value[2])
}

fn integer_attr(node: &Node, key: &str) -> usize {
    node.attrs
        .as_ref()
        .and_then(|attrs| attrs.get(key))
        .and_then(Value::as_u64)
        .unwrap_or(1) as usize
}

fn point(id: &str, offset: usize) -> DocPoint {
    DocPoint {
        block_id: id.into(),
        offset: ScalarOffset(offset),
    }
}
fn require(value: bool, invariant: &'static str) -> Result<()> {
    if value {
        Ok(())
    } else {
        Err(Error::Qualification(invariant))
    }
}

/// Execute real Web layout, merged table, Unicode caret/hit and cross-table selection checks.
///
/// Returns evidence suitable for a hosted-runner JSON artifact. Successful geometry does not
/// qualify native input, painting, clipboard, accessibility, or full editor compatibility.
/// The known colspan-100 backend incompatibility is reported, never called supported.
pub fn qualification_report() -> Result<Value> {
    let source = Document::from_json(qualification_fixture())?;
    let before = source.to_json()?;
    let geometry = FlowGeometry::with_atom_sizes(&source, 600.0, &|_| {
        Some(AtomSize {
            width: 48.0,
            height: 24.0,
        })
    })?;
    require(geometry.page_count() == 1, "continuous_web_page")?;
    let cells = geometry.cells()?;
    require(cells.len() == 6, "real_table_cell_boxes")?;
    require(
        cells.iter().filter(|cell| cell.covered).count() == 1,
        "vertical_merge_covered_box",
    )?;
    let first = cells
        .iter()
        .find(|cell| cell.canonical_path == NodePath(vec![1, 0, 0]) && !cell.covered)
        .ok_or(Error::Qualification("first_merged_cell"))?;
    let other = cells
        .iter()
        .find(|cell| cell.canonical_path == NodePath(vec![1, 0, 1]))
        .ok_or(Error::Qualification("second_cell"))?;
    require(
        first.rect.width > other.rect.width * 1.8,
        "horizontal_span_width",
    )?;
    require(
        first.rect.height > other.rect.height * 1.5,
        "vertical_span_height",
    )?;
    require(
        other.rect.x >= first.rect.x + first.rect.width - 0.1,
        "adjacent_grid_tracks",
    )?;
    for point in [
        point("before", 1),
        point("merged", 1),
        point("right", 2),
        point("after", 2),
    ] {
        let caret = geometry
            .caret(&point)?
            .ok_or(Error::Qualification("caret_exists"))?;
        let hit = geometry
            .hit(caret.page, caret.x + 0.01, caret.y + caret.height * 0.5)?
            .ok_or(Error::Qualification("caret_hit_exists"))?;
        require(hit == point, "scalar_byte_hit_roundtrip")?;
    }
    let selected = geometry.selection_rects(&Selection {
        anchor: point("before", 1),
        head: point("after", 2),
    })?;
    require(selected.len() >= 5, "cross_table_selection_rects")?;
    let atoms = geometry.atom_rects();
    require(atoms.len() == 1, "measured_inline_object")?;
    require(
        (atoms[0].width - 48.0).abs() < 0.1 && (atoms[0].height - 24.0).abs() < 0.1,
        "host_atom_size_preserved",
    )?;
    require(source.to_json()? == before, "canonical_json_unchanged")?;
    let wide = Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":2},"content":[{"type":"table","content":[{"type":"tableRow","content":[{"type":"tableCell","attrs":{"colspan":100,"rowspan":1},"content":[{"type":"paragraph","attrs":{"blockId":"wide"},"content":[{"type":"text","text":"wide"}]}]}]}]}]}),
    )?;
    require(
        matches!(
            FlowGeometry::new(&wide, 600.0),
            Err(Error::BackendColumnLimit {
                requested: 100,
                maximum: 63
            })
        ),
        "span100_reported_unsupported",
    )?;
    Ok(json!({
        "checkpoint":"native-flow-geometry", "upstream_revision":UPSTREAM_REVISION,
        "continuous_pages":geometry.page_count(), "cell_boxes":cells.len(),
        "covered_cell_boxes":cells.iter().filter(|cell| cell.covered).count(),
        "cross_table_selection_rectangles":selected.len(), "inline_object_boxes":atoms.len(),
        "canonical_json_preserved":true, "unicode_caret_hit_roundtrips":4,
        "colspan_100":{"supported":false,"backend_limit":wordcraft_doc::table::MAX_COLS,"exceeds_backend_capacity":true,"requires_read_only":true},
        "editor_acceptance":false,"native_input_acceptance":false,"rendering_acceptance":false
    }))
}

fn qualification_fixture() -> Value {
    json!({"type":"doc","attrs":{"schemaVersion":4},"vendorRoot":{"keep":true},"content":[
        {"type":"paragraph","attrs":{"blockId":"before","sourceRefs":["source-a"]},"content":[{"type":"text","text":"Д😀 before","marks":[{"type":"bold"}]}]},
        {"type":"table","attrs":{"blockId":"grid","vendor":{"keep":true}},"content":[
            {"type":"tableRow","content":[
                {"type":"tableCell","attrs":{"colspan":2,"rowspan":2,"vendor":5},"content":[{"type":"paragraph","attrs":{"blockId":"merged"},"content":[{"type":"text","text":"Я😀 merged"}]}]},
                {"type":"tableCell","attrs":{"colspan":1,"rowspan":1},"content":[{"type":"paragraph","attrs":{"blockId":"right"},"content":[{"type":"text","text":"R😀 right"}]}]}
            ]},
            {"type":"tableRow","content":[{"type":"tableCell","attrs":{"colspan":1,"rowspan":1},"content":[{"type":"paragraph","attrs":{"blockId":"bottom"},"content":[{"type":"text","text":"Bottom"}]}]}]},
            {"type":"tableRow","content":[
                {"type":"tableCell","attrs":{"colspan":2,"rowspan":1},"content":[{"type":"paragraph","attrs":{"blockId":"wide-bottom"},"content":[{"type":"text","text":"Wide bottom"}]}]},
                {"type":"tableCell","attrs":{"colspan":1,"rowspan":1},"content":[{"type":"paragraph","attrs":{"blockId":"corner"},"content":[{"type":"text","text":"Corner"}]}]}
            ]}
        ]},
        {"type":"paragraph","attrs":{"blockId":"after"},"content":[{"type":"text","text":"А😀 after "},{"type":"inlineMath","attrs":{"latex":"x^2"}},{"type":"hardBreak"},{"type":"text","text":"next line"}]}
    ]})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_web_grid_and_canonical_position_invariants() -> Result<()> {
        let report = qualification_report()?;
        assert_eq!(report["canonical_json_preserved"], true);
        assert_eq!(report["editor_acceptance"], false);
        Ok(())
    }

    #[test]
    fn native_byte_mapping_rejects_inside_cyrillic_scalar() -> Result<()> {
        let source = Document::plain("unicode", "Я😀")?;
        let geometry = FlowGeometry::new(&source, 500.0)?;
        assert!(matches!(
            geometry.resolve_native(&Pos::body(0, 1)),
            Err(Error::Model(craft_editor_model::Error::InvalidPosition))
        ));
        assert_eq!(geometry.to_native(&point("unicode", 2))?.off, "Я😀".len());
        Ok(())
    }

    #[test]
    fn unmeasured_atom_and_unimplemented_blocks_fail_without_data_loss() -> Result<()> {
        let source = Document::from_json(qualification_fixture())?;
        let before = source.to_json()?;
        assert!(matches!(
            FlowGeometry::new(&source, 500.0),
            Err(Error::UnsupportedNode(_))
        ));
        assert_eq!(source.to_json()?, before);
        Ok(())
    }
}
