//! Hosted qualification of a pinned native continuous-flow geometry backend.
//!
//! This optional module is a layout experiment, not an editor or accepted renderer. The
//! canonical model remains authoritative; the upstream document is a disposable projection.
//! Source: WordCraft `da914ce674f18ebc4e08cca425cc8f8acca3372f` (MIT OR Apache-2.0),
//! with the maintained generic nested-table margin repair pinned below.
//! No upstream source is copied here. Git dependencies retain their own font/license files.

use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use craft_editor_model::{
    ByteOffset, Document, Node, NodePath, ScalarOffset, byte_to_scalar, scalar_to_byte,
};
use serde_json::{Value, json};
use wordcraft_doc::{
    Block, Blocks, CharProps, InlineObject, Paragraph, Path, Pos, StoryRef, Table, para_block,
};
use wordcraft_layout::{DocLayout, LayoutCache, LayoutOptions, Placed, ViewMode};

use crate::{DocPoint, Selection, syntax};

mod table_viewport;
pub use table_viewport::{TABLE_SCROLL_GUTTER, TableViewport};

/// Upstream revision used by the hosted geometry experiment.
pub const UPSTREAM_REVISION: &str = "82ace77a17d71f8fce770227624319203e74cc7b";
/// Original upstream base retained by the minimal maintained backend fork.
pub const UPSTREAM_BASE_REVISION: &str = "da914ce674f18ebc4e08cca425cc8f8acca3372f";
/// Repository carrying only the generic bounded nested-table margin repair.
pub const BACKEND_REPOSITORY: &str = "https://github.com/avacadorun-dev/wordcraft";

/// Content-free geometry diagnostic. A failure never rewrites the canonical document.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Native syntax tokenization failed without changing canonical source.
    #[error(transparent)]
    Syntax(#[from] syntax::Error),
    /// Host atom rendering failed with a content-free code.
    #[error("atom_rendering: {0}")]
    AtomRendering(String),
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

/// Reference used by a host-supplied line-height multiple.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LineHeightBasis {
    /// Preserve the layout backend's single-line font-metric multiple.
    #[default]
    FontMetrics,
    /// Multiply the paragraph's nominal font size, matching a CSS unitless line height.
    FontSize,
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
    /// Whether the line-height multiple uses font metrics or the nominal font size.
    pub line_height_basis: LineHeightBasis,
    /// Optional independent gap between list items; None preserves inherited paragraph spacing.
    pub list_item_spacing: Option<f32>,
    /// Override standard ligatures without changing contextual/required shaping.
    pub standard_ligatures: Option<bool>,
    /// Space after a paragraph, in logical document points.
    pub paragraph_spacing: f32,
    /// Optional host syntax colors. None keeps code plain while preserving its language/source.
    pub syntax_palette: Option<syntax::Palette>,
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
            line_height_basis: LineHeightBasis::FontMetrics,
            list_item_spacing: None,
            standard_ligatures: None,
            paragraph_spacing: 10.0,
            syntax_palette: Some(syntax::Palette::for_surface([35, 39, 47], [250, 250, 248])),
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

/// One actual native logical-column boundary, including a merged cell's internal tracks.
#[derive(Debug, Clone)]
pub struct ColumnGeometry {
    /// Exact canonical table location in this measured document.
    pub table_path: NodePath,
    /// Logical column index, starting at zero.
    pub column: usize,
    /// Ancestor/own viewport clip; a hidden boundary cannot intercept a pointer.
    pub clip: Option<GeometryRect>,
    /// Track's actual native rectangle in document points.
    pub rect: GeometryRect,
}

/// Bounds of a canonical opaque atom, including its native caption where present.
#[derive(Debug, Clone)]
pub struct NodeGeometry {
    /// Exact source tree location; no transient view ID is written into canonical data.
    pub canonical_path: NodePath,
    /// Native logical document rectangle.
    pub rect: GeometryRect,
}

/// Measured native bounds and host-localized status for one nonfinal opaque atom.
#[derive(Debug, Clone)]
pub struct AtomPlaceholderGeometry {
    /// Exact unchanged canonical atom location.
    pub canonical_path: NodePath,
    /// Actual native object rectangle; inline atoms still occupy one canonical scalar.
    pub rect: GeometryRect,
    /// Explicit pending/failure status; never a replacement for canonical source.
    pub placeholder: crate::renderers::AtomPlaceholder,
}

/// Actual grammar availability for one code block in the unchanged canonical document.
#[derive(Debug, Clone)]
pub struct SyntaxReport {
    /// Exact canonical code-block location.
    pub canonical_path: NodePath,
    /// Stored language attribute, retained exactly rather than normalized in canonical data.
    pub language: String,
    /// Actual native grammar selection; required gaps remain visible to acceptance tooling.
    pub coverage: syntax::Coverage,
}

struct BlockMap {
    canonical: NodePath,
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
    blocks_by_id: HashMap<String, usize>,
    blocks_by_path: HashMap<Path, usize>,
    cell_paths: HashMap<(Path, usize, usize), NodePath>,
    table_tracks: HashMap<NodePath, Vec<f32>>,
    table_available: HashMap<NodePath, f32>,
    atom_paths: HashMap<String, NodePath>,
    node_paths: HashMap<Path, NodePath>,
    syntax_reports: Vec<SyntaxReport>,
    placeholders: BTreeMap<String, crate::renderers::AtomPlaceholder>,
    node_geometry: Vec<NodeGeometry>,
    placeholder_geometry: Vec<AtomPlaceholderGeometry>,
    text_windows: Vec<(GeometryRect, Selection)>,
    text_window_max_height: f32,
    raw_cells: Vec<CellGeometry>,
    raw_columns: Vec<ColumnGeometry>,
    viewports: table_viewport::Viewports,
    #[cfg(feature = "wordcraft-canvas")]
    view_paint: table_viewport::ViewPaint,
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
        Self::build(source, width, atom_size, style, None)
    }

    /// Measure actual host-resolved native image/diagram/formula output.
    pub fn with_resolver(
        source: &Document,
        width: f32,
        style: &GeometryStyle,
        resolver: &dyn crate::renderers::AtomResolver,
    ) -> Result<Self> {
        Self::build(source, width, &|_| None, Some(style), Some(resolver))
    }

    fn build(
        source: &Document,
        width: f32,
        atom_size: &impl Fn(&Node) -> Option<AtomSize>,
        style: Option<&GeometryStyle>,
        resolver: Option<&dyn crate::renderers::AtomResolver>,
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
                || s.list_item_spacing
                    .is_some_and(|gap| !gap.is_finite() || gap < 0.0)
        }) {
            return Err(Error::InvalidSize);
        }
        let mut builder = Builder {
            blocks: Vec::new(),
            cell_paths: HashMap::new(),
            table_tracks: HashMap::new(),
            table_available: HashMap::new(),
            atom_size,
            style,
            resolver,
            media: BTreeMap::new(),
            numbering: wordcraft_doc::numbering::Numbering::default(),
            atom_paths: HashMap::new(),
            node_paths: HashMap::new(),
            syntax_reports: vec![],
            placeholders: BTreeMap::new(),
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
        upstream.media = builder.media;
        upstream.numbering = Arc::new(builder.numbering);
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
            sheet.default_para.line_spacing = Some(match style.line_height_basis {
                LineHeightBasis::FontMetrics => {
                    wordcraft_doc::props::LineSpacing::Multiple(style.line_height)
                }
                LineHeightBasis::FontSize => {
                    wordcraft_doc::props::LineSpacing::Exactly(style.font_size * style.line_height)
                }
            });
            if let Some(standard) = style.standard_ligatures {
                sheet.default_chr.ligatures = Some(wordcraft_doc::props::Ligatures {
                    standard,
                    contextual: true,
                    historical: false,
                    discretional: false,
                });
            }
            sheet.default_para.space_after = Some(style.paragraph_spacing);
        }
        let options = LayoutOptions {
            view: ViewMode::Web,
            web_width: width,
            proofing: false,
            ..Default::default()
        };
        let measured = wordcraft_layout::layout(&upstream, &mut LayoutCache::new(), &options);
        let blocks_by_id = builder
            .blocks
            .iter()
            .enumerate()
            .map(|(index, block)| (block.id.clone(), index))
            .collect();
        let blocks_by_path = builder
            .blocks
            .iter()
            .enumerate()
            .map(|(index, block)| (block.path.clone(), index))
            .collect();
        let mut geometry = Self {
            blocks_by_id,
            blocks_by_path,
            upstream,
            measured,
            blocks: builder.blocks,
            cell_paths: builder.cell_paths,
            table_tracks: builder.table_tracks,
            table_available: builder.table_available,
            atom_paths: builder.atom_paths,
            node_paths: builder.node_paths,
            syntax_reports: builder.syntax_reports,
            placeholders: builder.placeholders,
            node_geometry: vec![],
            placeholder_geometry: vec![],
            text_windows: vec![],
            text_window_max_height: 0.0,
            raw_cells: vec![],
            raw_columns: vec![],
            viewports: table_viewport::Viewports::default(),
            #[cfg(feature = "wordcraft-canvas")]
            view_paint: table_viewport::ViewPaint::default(),
        };
        // Viewport scheduling/overlays read these every native frame. The immutable measured
        // layout changes only with a new projection; do not rebuild its full display list on hover.
        geometry.raw_cells = geometry.measure_cells()?;
        geometry.raw_columns = geometry.measure_columns(source)?;
        geometry.viewports =
            table_viewport::Viewports::new(source, &geometry.raw_cells, &geometry.table_available)?;
        #[cfg(feature = "wordcraft-canvas")]
        if !geometry.viewports.is_empty() {
            geometry.view_paint = table_viewport::ViewPaint::new(&geometry);
        }
        geometry.node_geometry = geometry.measure_nodes();
        geometry.placeholder_geometry = geometry.measure_placeholders();
        geometry.text_windows = geometry.measure_text_windows()?;
        geometry.text_window_max_height = geometry
            .text_windows
            .iter()
            .map(|(rect, _)| rect.height)
            .fold(0.0_f32, f32::max);
        Ok(geometry)
    }

    /// Actual grammar coverage for projected code blocks, including explicit required gaps.
    pub fn syntax_reports(&self) -> &[SyntaxReport] {
        &self.syntax_reports
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
        let block = if end {
            self.blocks.last()
        } else {
            self.blocks.first()
        }
        .ok_or(Error::UnmappedPosition)?;
        Ok(DocPoint {
            block_id: block.id.clone(),
            offset: ScalarOffset(if end { block.text.chars().count() } else { 0 }),
        })
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
        let native = self.to_native(point)?;
        let block = self
            .block_for_id(&point.block_id)
            .ok_or(Error::UnmappedPosition)?;
        let shift = self.viewports.shift(&block.canonical);
        let Some((candidate, x)) =
            self.measured
                .vertical(&native, goal_x.map(|x| x + shift), direction, 0)
        else {
            return Ok(None);
        };
        let goal = goal_x.unwrap_or(x - shift);
        let resolved = self.resolve_native(&candidate)?;
        let target = if let Some(caret) = self.measured.caret(&candidate) {
            self.hit(caret.page, goal, caret.top + caret.height * 0.5)?
                .unwrap_or(resolved)
        } else {
            resolved
        };
        Ok(Some((target, goal)))
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
        let mut rendered = if self.viewports.is_empty() {
            wordcraft_render::render_area(
                &self.upstream,
                &self.measured.pages[area.page],
                wordcraft_geom::Rect::new(area.x, area.y, area.width, area.height),
                scale,
                &options,
            )
        } else {
            wordcraft_render::render_items(
                &self.upstream,
                &self.view_paint.pages[area.page],
                wordcraft_geom::Rect::new(area.x, area.y, area.width, area.height),
                scale,
                &options,
            )
        };
        if !self.viewports.is_empty() {
            // render_items is transparent. Composite its premultiplied pixels on the host paper.
            for pixel in rendered.pixels.chunks_exact_mut(4) {
                let remaining = 255 - u32::from(pixel[3]);
                for channel in 0..3 {
                    pixel[channel] = (u32::from(pixel[channel])
                        + (u32::from(paper[channel]) * remaining + 127) / 255)
                        .min(255) as u8;
                }
                pixel[3] = 255;
            }
        }
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
        let block = self
            .block_for_id(&point.block_id)
            .ok_or(Error::UnmappedPosition)?;
        Ok(self
            .measured
            .caret(&self.to_native(point)?)
            .and_then(|caret| {
                self.viewports.rect(
                    &block.canonical,
                    GeometryRect {
                        page: caret.page,
                        x: caret.x,
                        y: caret.top,
                        width: caret.width.max(1.0),
                        height: caret.height,
                    },
                )
            }))
    }

    /// Resolve native hit testing to an exact canonical block/scalar point.
    pub fn hit(&self, page: usize, x: f32, y: f32) -> Result<Option<DocPoint>> {
        let (raw_x, raw_y) = self.viewports.inverse(page, x, y);
        let Some(pos) = self.measured.hit(page, raw_x, raw_y, StoryRef::Body) else {
            return Ok(None);
        };
        let point = self.resolve_native(&pos)?;
        let block = self
            .block_for_id(&point.block_id)
            .ok_or(Error::UnmappedPosition)?;
        if self.viewports.clip(&block.canonical).is_some_and(|clip| {
            x < clip.x || x > clip.x + clip.width || y < clip.y || y > clip.y + clip.height
        }) {
            return Ok(None);
        }
        Ok(Some(point))
    }

    /// Resolve cross-block and cross-table selection geometry in one document coordinate space.
    pub fn selection_rects(&self, selection: &Selection) -> Result<Vec<GeometryRect>> {
        let a = self.to_native(&selection.anchor)?;
        let b = self.to_native(&selection.head)?;
        let (first, last) = if a.path.0 < b.path.0 || (a.path == b.path && a.off <= b.off) {
            (a, b)
        } else {
            (b, a)
        };
        let mut rectangles = Vec::new();
        for block in self
            .blocks
            .iter()
            .filter(|block| first.path.0 <= block.path.0 && block.path.0 <= last.path.0)
        {
            let start = Pos::new(
                StoryRef::Body,
                block.path.clone(),
                if block.path == first.path {
                    first.off
                } else {
                    0
                },
            );
            let end = Pos::new(
                StoryRef::Body,
                block.path.clone(),
                if block.path == last.path {
                    last.off
                } else {
                    block.text.len()
                },
            );
            for (page, rect) in self
                .measured
                .selection_rects(&self.upstream, &start, &end, 0)
            {
                if let Some(rect) = self
                    .viewports
                    .rect(&block.canonical, GeometryRect::from_rect(page, rect))
                {
                    rectangles.push(rect);
                }
            }
        }
        Ok(rectangles)
    }

    /// Exact semantic ranges of measured text lines intersecting a viewport.
    /// The immutable line index is built with geometry, not by rescanning manuscript
    /// text on every repaint. Empty lines and opaque atom captions have no text range.
    pub fn visible_text_ranges(&self, area: GeometryRect) -> Vec<Selection> {
        if !area.x.is_finite()
            || !area.y.is_finite()
            || !area.width.is_finite()
            || !area.height.is_finite()
            || area.width <= 0.0
            || area.height <= 0.0
        {
            return vec![];
        }
        let lower = area.y - self.text_window_max_height;
        let start = self.text_windows.partition_point(|(rect, _)| {
            rect.page < area.page || (rect.page == area.page && rect.y < lower)
        });
        self.text_windows[start..]
            .iter()
            .take_while(|(rect, _)| rect.page == area.page && rect.y <= area.y + area.height)
            .filter(|(rect, range)| {
                self.block_for_id(&range.anchor.block_id)
                    .and_then(|block| self.viewports.rect(&block.canonical, *rect))
                    .is_some_and(|rect| {
                        rect.y + rect.height >= area.y
                            && rect.x < area.x + area.width
                            && rect.x + rect.width > area.x
                    })
            })
            .map(|(_, range)| range.clone())
            .collect()
    }

    fn measure_text_windows(&self) -> Result<Vec<(GeometryRect, Selection)>> {
        let blocks: HashMap<_, _> = self
            .blocks
            .iter()
            .map(|block| (&block.path, block))
            .collect();
        let mut windows = Vec::new();
        for (page, layout) in self.measured.pages.iter().enumerate() {
            for item in &layout.items {
                if let Placed::Lines {
                    path,
                    para,
                    l0,
                    l1,
                    x,
                    y,
                    ..
                } = item
                    && let Some(block) = blocks.get(path)
                {
                    let first_top = para.lines.get(*l0).map_or(0.0, |line| line.top);
                    for line in para.lines.iter().take(*l1).skip(*l0) {
                        let start = byte_to_scalar(&block.text, ByteOffset(line.start))?;
                        let end = byte_to_scalar(&block.text, ByteOffset(line.stop))?;
                        if start == end {
                            continue;
                        }
                        windows.push((
                            GeometryRect {
                                page,
                                x: *x + line.left,
                                y: *y + line.top - first_top,
                                width: (line.right - line.left).max(1.0),
                                height: line.height.max(1.0),
                            },
                            Selection {
                                anchor: DocPoint {
                                    block_id: block.id.clone(),
                                    offset: start,
                                },
                                head: DocPoint {
                                    block_id: block.id.clone(),
                                    offset: end,
                                },
                            },
                        ));
                    }
                }
            }
        }
        windows.sort_by(|(a, _), (b, _)| a.page.cmp(&b.page).then_with(|| a.y.total_cmp(&b.y)));
        Ok(windows)
    }

    /// Return actual cell boxes emitted by the table layout engine.
    pub fn cells(&self) -> Result<Vec<CellGeometry>> {
        Ok(self
            .raw_cells
            .iter()
            .filter_map(|cell| {
                self.viewports
                    .rect(&cell.canonical_path, cell.rect)
                    .map(|rect| CellGeometry {
                        rect,
                        ..cell.clone()
                    })
            })
            .collect())
    }
    fn measure_cells(&self) -> Result<Vec<CellGeometry>> {
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

    /// Resolve real column tracks from measured cell boxes, including invisible merged boundaries.
    pub fn columns(&self, _source: &Document) -> Result<Vec<ColumnGeometry>> {
        Ok(self
            .raw_columns
            .iter()
            .map(|column| {
                let path = column.table_path.child(0);
                ColumnGeometry {
                    rect: self.viewports.unbounded(&path, column.rect),
                    clip: self.viewports.clip(&path),
                    ..column.clone()
                }
            })
            .collect())
    }
    fn measure_columns(&self, source: &Document) -> Result<Vec<ColumnGeometry>> {
        let mut grouped: HashMap<NodePath, Vec<&CellGeometry>> = HashMap::new();
        for cell in &self.raw_cells {
            if cell.canonical_path.0.len() >= 2 {
                grouped
                    .entry(NodePath(
                        cell.canonical_path.0[..cell.canonical_path.0.len() - 2].to_vec(),
                    ))
                    .or_default()
                    .push(cell);
            }
        }
        let mut columns = Vec::new();
        for (path, tracks) in &self.table_tracks {
            let grid = craft_editor_model::TableGrid::new(source.node(path)?)?;
            let Some(matching) = grouped.get(path) else {
                continue;
            };
            let Some(first) = matching.first() else {
                continue;
            };
            let origin = grid
                .cells
                .iter()
                .find(|cell| first.canonical_path == path.child(cell.row).child(cell.cell_index))
                .ok_or(Error::UnmappedPosition)?;
            let x = first.rect.x - tracks.iter().take(origin.column).sum::<f32>();
            let top = matching
                .iter()
                .map(|cell| cell.rect.y)
                .fold(f32::INFINITY, f32::min);
            let bottom = matching
                .iter()
                .map(|cell| cell.rect.y + cell.rect.height)
                .fold(0.0_f32, f32::max);
            let mut left = x;
            for (column, width) in tracks.iter().copied().enumerate() {
                columns.push(ColumnGeometry {
                    table_path: path.clone(),
                    column,
                    clip: None,
                    rect: GeometryRect {
                        page: 0,
                        x: left,
                        y: top,
                        width,
                        height: bottom - top,
                    },
                });
                left += width;
            }
        }
        Ok(columns)
    }

    /// Actual selectable opaque-node bounds, with native caption and separator line geometry.
    pub fn nodes(&self) -> Vec<NodeGeometry> {
        self.node_geometry
            .iter()
            .filter_map(|node| {
                self.viewports
                    .rect(&node.canonical_path, node.rect)
                    .map(|rect| NodeGeometry {
                        rect,
                        ..node.clone()
                    })
            })
            .collect()
    }

    fn measure_nodes(&self) -> Vec<NodeGeometry> {
        if self.atom_paths.is_empty() && self.node_paths.is_empty() {
            return vec![];
        }
        use wordcraft_layout::display::{DisplayOptions, Draw, page_display};
        let mut output: Vec<NodeGeometry> = Vec::new();
        for (page, layout) in self.measured.pages.iter().enumerate() {
            for draw in page_display(&self.upstream, layout, &DisplayOptions::default()) {
                if let Draw::Image { rect, media, .. } = draw
                    && let Some(path) = self.atom_paths.get(&media)
                {
                    merge_node_rect(&mut output, path, GeometryRect::from_rect(page, rect));
                }
            }
            for item in &layout.items {
                if let Placed::Lines {
                    path,
                    para,
                    l0,
                    l1,
                    x,
                    y,
                    ..
                } = item
                    && let Some(canonical) = self.node_paths.get(path)
                {
                    let first_top = para.lines.get(*l0).map_or(0.0, |line| line.top);
                    for line in para.lines.iter().take(*l1).skip(*l0) {
                        merge_node_rect(
                            &mut output,
                            canonical,
                            GeometryRect {
                                page,
                                x: *x + line.left,
                                y: *y + line.top - first_top,
                                width: (line.right - line.left).max(1.0),
                                height: line.height.max(1.0),
                            },
                        );
                    }
                }
            }
        }
        output
    }
    /// Nonfinal atom status boxes at their actual native object bounds.
    /// The canvas paints these over transparent reservation pixels without changing source text.
    pub fn placeholders(&self) -> Vec<AtomPlaceholderGeometry> {
        self.placeholder_geometry
            .iter()
            .filter_map(|node| {
                self.viewports
                    .rect(&node.canonical_path, node.rect)
                    .map(|rect| AtomPlaceholderGeometry {
                        rect,
                        ..node.clone()
                    })
            })
            .collect()
    }

    fn measure_placeholders(&self) -> Vec<AtomPlaceholderGeometry> {
        if self.placeholders.is_empty() {
            return vec![];
        }
        use wordcraft_layout::display::{DisplayOptions, Draw, page_display};
        let mut output = Vec::new();
        for (page, layout) in self.measured.pages.iter().enumerate() {
            for draw in page_display(&self.upstream, layout, &DisplayOptions::default()) {
                if let Draw::Image { rect, media, .. } = draw
                    && let Some(placeholder) = self.placeholders.get(&media)
                    && let Some(path) = self.atom_paths.get(&media)
                {
                    output.push(AtomPlaceholderGeometry {
                        canonical_path: path.clone(),
                        rect: GeometryRect::from_rect(page, rect),
                        placeholder: placeholder.clone(),
                    });
                }
            }
        }
        output
    }

    /// Resolve a pointer inside a real atom/caption/separator to its exact canonical tree path.
    pub fn hit_node(&self, page: usize, x: f32, y: f32) -> Option<NodePath> {
        self.nodes()
            .into_iter()
            .find(|node| {
                node.rect.page == page
                    && x >= node.rect.x
                    && x <= node.rect.x + node.rect.width
                    && y >= node.rect.y
                    && y <= node.rect.y + node.rect.height
            })
            .map(|node| node.canonical_path)
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

    /// Bounded native table components in current presentation coordinates.
    pub fn table_viewports(&self) -> Vec<TableViewport> {
        self.viewports.list()
    }
    /// Current session-local table offsets, containing no edit authority.
    pub fn table_offsets(&self) -> HashMap<String, f32> {
        self.viewports.offsets()
    }
    /// Reapply/clamp presentation offsets after source or typography changes without canonical edits.
    pub fn restore_table_offsets(&mut self, offsets: &HashMap<String, f32>) {
        self.viewports.restore(offsets);
        #[cfg(feature = "wordcraft-canvas")]
        self.view_paint.update(&self.viewports);
    }
    /// Scroll one exact measured table, preserving source, semantic positions and history.
    pub fn scroll_table(&mut self, path: &NodePath, offset: f32) -> Result<bool> {
        let changed = self.viewports.set(path, offset)?;
        #[cfg(feature = "wordcraft-canvas")]
        if changed {
            self.view_paint.update(&self.viewports);
        }
        Ok(changed)
    }
    /// Reveal an exact caret through nested table viewports without reshaping or rewriting content.
    pub fn reveal_caret(&mut self, point: &DocPoint) -> Result<bool> {
        let canonical = self
            .block_for_id(&point.block_id)
            .ok_or(Error::UnmappedPosition)?
            .canonical
            .clone();
        let Some(caret) = self.measured.caret(&self.to_native(point)?) else {
            return Ok(false);
        };
        let changed = self.viewports.reveal(
            &canonical,
            GeometryRect {
                page: caret.page,
                x: caret.x,
                y: caret.top,
                width: caret.width.max(1.0),
                height: caret.height,
            },
        );
        #[cfg(feature = "wordcraft-canvas")]
        if changed {
            self.view_paint.update(&self.viewports);
        }
        Ok(changed)
    }
    /// Reveal a measured opaque block through its enclosing table components.
    pub fn reveal_node(&mut self, path: &NodePath) -> bool {
        let Some(node) = self
            .node_geometry
            .iter()
            .find(|node| node.canonical_path == *path)
        else {
            return false;
        };
        let changed = self.viewports.reveal(path, node.rect);
        #[cfg(feature = "wordcraft-canvas")]
        if changed {
            self.view_paint.update(&self.viewports);
        }
        changed
    }
    #[cfg(feature = "wordcraft-canvas")]
    fn canonical_item_path(&self, item: &Placed) -> Option<NodePath> {
        let from_path = |path: &Path| {
            self.block_for_path(path)
                .map(|block| block.canonical.clone())
                .or_else(|| self.node_paths.get(path).cloned())
        };
        let bounds = match item {
            Placed::Lines { path, .. } => return from_path(path),
            Placed::Cell {
                table, row, cell, ..
            } => return self.cell_paths.get(&(table.clone(), *row, *cell)).cloned(),
            Placed::Image {
                rect, media, path, ..
            } => {
                return self
                    .atom_paths
                    .get(media)
                    .cloned()
                    .or_else(|| from_path(path))
                    .or_else(|| {
                        self.viewports
                            .scope_for_rect(GeometryRect::from_rect(0, *rect))
                    });
            }
            Placed::Graphic { rect, path, .. } | Placed::Object { rect, path, .. } => {
                return from_path(path).or_else(|| {
                    self.viewports
                        .scope_for_rect(GeometryRect::from_rect(0, *rect))
                });
            }
            Placed::Fill { rect, .. } | Placed::Shape { rect, .. } | Placed::Clip { rect, .. } => {
                GeometryRect::from_rect(0, *rect)
            }
            Placed::Rule { x0, y0, x1, y1, .. } => GeometryRect {
                page: 0,
                x: x0.min(*x1),
                y: y0.min(*y1),
                width: (x1 - x0).abs().max(0.01),
                height: (y1 - y0).abs().max(0.01),
            },
        };
        self.viewports.scope_for_rect(bounds)
    }

    fn block_for_id(&self, id: &str) -> Option<&BlockMap> {
        self.blocks_by_id
            .get(id)
            .and_then(|index| self.blocks.get(*index))
    }
    fn block_for_path(&self, path: &Path) -> Option<&BlockMap> {
        self.blocks_by_path
            .get(path)
            .and_then(|index| self.blocks.get(*index))
    }
    fn to_native(&self, point: &DocPoint) -> Result<Pos> {
        let block = self
            .block_for_id(&point.block_id)
            .ok_or(Error::UnmappedPosition)?;
        let offset = scalar_to_byte(&block.text, point.offset)?;
        Ok(Pos::new(StoryRef::Body, block.path.clone(), offset.0))
    }

    fn resolve_native(&self, pos: &Pos) -> Result<DocPoint> {
        if pos.story != StoryRef::Body {
            return Err(Error::UnmappedPosition);
        }
        let Some(block) = self.block_for_path(&pos.path) else {
            let previous = self
                .blocks
                .iter()
                .rev()
                .find(|block| block.path.0 < pos.path.0);
            let next = self.blocks.iter().find(|block| block.path.0 > pos.path.0);
            let (block, end) = if pos.off == 0 {
                previous
                    .map(|block| (block, true))
                    .or_else(|| next.map(|block| (block, false)))
            } else {
                next.map(|block| (block, false))
                    .or_else(|| previous.map(|block| (block, true)))
            }
            .ok_or(Error::UnmappedPosition)?;
            return Ok(DocPoint {
                block_id: block.id.clone(),
                offset: ScalarOffset(if end { block.text.chars().count() } else { 0 }),
            });
        };
        let offset = byte_to_scalar(&block.text, ByteOffset(pos.off))?;
        Ok(DocPoint {
            block_id: block.id.clone(),
            offset,
        })
    }
}

#[derive(Clone, Default)]
struct ParagraphContext {
    indent: f32,
    list_depth: usize,
    list_space_before: f32,
    label: Option<String>,
    quote: bool,
    callout: bool,
}

struct Builder<'a, F> {
    blocks: Vec<BlockMap>,
    cell_paths: HashMap<(Path, usize, usize), NodePath>,
    table_tracks: HashMap<NodePath, Vec<f32>>,
    table_available: HashMap<NodePath, f32>,
    atom_size: &'a F,
    style: Option<&'a GeometryStyle>,
    resolver: Option<&'a dyn crate::renderers::AtomResolver>,
    media: BTreeMap<String, Arc<Vec<u8>>>,
    numbering: wordcraft_doc::numbering::Numbering,
    atom_paths: HashMap<String, NodePath>,
    node_paths: HashMap<Path, NodePath>,
    syntax_reports: Vec<SyntaxReport>,
    placeholders: BTreeMap<String, crate::renderers::AtomPlaceholder>,
}
impl<F: Fn(&Node) -> Option<AtomSize>> Builder<'_, F> {
    fn blocks(
        &mut self,
        nodes: &[Node],
        native_parent: &[u32],
        canonical_parent: &[usize],
        width: f32,
    ) -> Result<Blocks> {
        let mut output = Vec::new();
        self.append_nodes(
            nodes,
            native_parent,
            canonical_parent,
            width,
            &ParagraphContext::default(),
            &mut output,
        )?;
        Ok(output)
    }

    fn append_nodes(
        &mut self,
        nodes: &[Node],
        native_parent: &[u32],
        canonical_parent: &[usize],
        width: f32,
        context: &ParagraphContext,
        output: &mut Blocks,
    ) -> Result<()> {
        for (index, node) in nodes.iter().enumerate() {
            let mut canonical = canonical_parent.to_vec();
            canonical.push(index);
            let mut current = context.clone();
            if index > 0 {
                current.label = None;
                current.list_space_before = 0.0;
            }
            match node.node_type.as_str() {
                "blockquote" | "callout" => {
                    current.indent += 18.0;
                    current.quote |= node.node_type == "blockquote";
                    current.callout |= node.node_type == "callout";
                    self.append_nodes(
                        node.children(),
                        native_parent,
                        &canonical,
                        width,
                        &current,
                        output,
                    )?;
                }
                "bulletList" | "orderedList" => {
                    let start = node
                        .attrs
                        .as_ref()
                        .and_then(|attrs| attrs.get("start"))
                        .and_then(Value::as_i64)
                        .unwrap_or(1);
                    for (item_index, item) in node.children().iter().enumerate() {
                        if item.node_type != "listItem" {
                            return Err(Error::UnsupportedNode(item.node_type.clone()));
                        }
                        let mut item_path = canonical.clone();
                        item_path.push(item_index);
                        let mut item_context = current.clone();
                        item_context.indent += 24.0;
                        item_context.list_depth += 1;
                        if let Some(gap) = self.style.and_then(|style| style.list_item_spacing) {
                            item_context.list_space_before =
                                if item_index > 0 || current.list_depth > 0 {
                                    gap
                                } else {
                                    current.list_space_before
                                };
                        }
                        item_context.label = Some(if node.node_type == "bulletList" {
                            wordcraft_doc::numbering::default_bullet(current.list_depth).to_string()
                        } else {
                            let value = start
                                .checked_add(
                                    i64::try_from(item_index).map_err(|_| Error::InvalidGrid)?,
                                )
                                .ok_or(Error::InvalidGrid)?;
                            ordered_label(value, node.string_attr("type"))
                        });
                        self.append_nodes(
                            item.children(),
                            native_parent,
                            &item_path,
                            width,
                            &item_context,
                            output,
                        )?;
                    }
                }
                "paragraph" | "heading" | "codeBlock" => {
                    let mut native = native_parent.to_vec();
                    native.push(u32::try_from(output.len()).map_err(|_| Error::InvalidGrid)?);
                    output.push(self.paragraph(node, Path(native), width, &current, &canonical)?);
                }
                "table" => {
                    let mut native = native_parent.to_vec();
                    native.push(u32::try_from(output.len()).map_err(|_| Error::InvalidGrid)?);
                    let mut table =
                        self.table(node, &native, &canonical, (width - current.indent).max(1.0))?;
                    if current.indent > 0.0 {
                        table.props.indent = Some(current.indent);
                    }
                    output.push(Arc::new(Block::Table(table)));
                }
                "imageBlock" | "mathBlock" | "mermaidBlock" | "horizontalRule" | "sceneBreak" => {
                    let mut native = native_parent.to_vec();
                    native.push(u32::try_from(output.len()).map_err(|_| Error::InvalidGrid)?);
                    self.block_atom(node, Path(native), NodePath(canonical), width, output)?;
                }
                _ => return Err(Error::UnsupportedNode(node.node_type.clone())),
            }
        }
        Ok(())
    }

    fn paragraph(
        &mut self,
        node: &Node,
        path: Path,
        width: f32,
        context: &ParagraphContext,
        canonical_path: &[usize],
    ) -> Result<Arc<Block>> {
        let id = node.block_id().ok_or(Error::MissingIdentity)?.to_owned();
        let mut paragraph = Paragraph::new();
        let body_size = self.style.map_or(18.0, |style| style.font_size);
        if context.list_depth > 0
            && self
                .style
                .is_some_and(|style| style.list_item_spacing.is_some())
        {
            paragraph.props.space_after = Some(0.0);
            if context.list_space_before > 0.0 {
                paragraph.props.space_before = Some(context.list_space_before);
            }
        }
        if context.indent > 0.0 {
            paragraph.props.indent_left = Some(context.indent);
        }
        if context.quote || context.callout {
            let mut border =
                wordcraft_doc::props::Border::single(if context.callout { 2.0 } else { 1.5 });
            border.color = Some(rgb(self
                .style
                .map_or([139, 148, 163], |style| style.table_border)));
            border.space = 8.0;
            paragraph.props.borders = Some(wordcraft_doc::props::Borders {
                left: Some(border),
                ..Default::default()
            });
            if context.callout {
                paragraph.props.shading = Some(rgb(self
                    .style
                    .map_or([234, 238, 244], |style| style.table_header)));
            }
        }
        if let Some(label) = &context.label {
            let level = wordcraft_doc::numbering::Level {
                format: wordcraft_doc::section::NumFormat::Bullet,
                text: label.clone(),
                indent: context.indent,
                hanging: 18.0,
                chr: CharProps {
                    size: Some(body_size),
                    font: self.style.and_then(|style| style.font_family.clone()),
                    color: self
                        .style
                        .map(|style| wordcraft_doc::props::TextColor::Rgb(rgb(style.text_color))),
                    ..Default::default()
                },
                ..Default::default()
            };
            let num = self.numbering.try_add_levels(None, vec![level])?;
            paragraph.props.numbering = Some(wordcraft_doc::props::NumRef { num, level: 0 });
        }

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
        if node
            .children()
            .iter()
            .any(|inline| inline.node_type == "inlineMath")
            && let Some(style) = self
                .style
                .filter(|style| style.line_height_basis == LineHeightBasis::FontSize)
        {
            // Exact CSS prose advance must not clip replaced inline-object line boxes.
            paragraph.props.line_spacing = Some(wordcraft_doc::props::LineSpacing::AtLeast(
                style.font_size * style.line_height,
            ));
        }
        paragraph.props.align = match node.string_attr("textAlign") {
            Some("left") => Some(wordcraft_doc::props::Align::Left),
            Some("center") => Some(wordcraft_doc::props::Align::Center),
            Some("right") => Some(wordcraft_doc::props::Align::Right),
            Some("justify") => Some(wordcraft_doc::props::Align::Justify),
            _ => None,
        };
        for (inline_index, inline) in node.children().iter().enumerate() {
            let mut props = inherited.clone();
            for mark in inline.marks.as_deref().unwrap_or_default() {
                match mark.mark_type.as_str() {
                    "bold" => props.bold = Some(true),
                    "italic" => props.italic = Some(true),
                    "underline" | "link" => {
                        props.underline = Some(wordcraft_doc::props::Underline::Single)
                    }
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
                    paragraph.insert_text(paragraph.len(), text, &props)?;
                }
                "hardBreak" => {
                    paragraph.insert_text(
                        paragraph.len(),
                        &wordcraft_doc::para::LINE_BREAK.to_string(),
                        &props,
                    )?;
                }
                "inlineMath" => {
                    let media = format!("craft-geometry-atom:{}:{}", id, paragraph.len());
                    let size = if let Some(resolver) = self.resolver {
                        let resolved = self.resolve_atom(resolver, inline, width, &media)?;
                        if resolved.encoded.is_empty() || resolved.encoded.len() > 16 * 1024 * 1024
                        {
                            return Err(Error::AtomRendering("atom_encoded_size".into()));
                        }
                        if let Some(baseline) = resolved.baseline {
                            if !baseline.is_finite()
                                || baseline < 0.0
                                || baseline > resolved.size.height
                            {
                                return Err(Error::InvalidSize);
                            }
                            props.position = Some(baseline - resolved.size.height);
                        }
                        self.media.insert(media.clone(), resolved.encoded);
                        resolved.size
                    } else {
                        (self.atom_size)(inline).ok_or_else(|| {
                            Error::UnsupportedNode("unmeasured_inline_atom".into())
                        })?
                    };
                    if !size.width.is_finite()
                        || !size.height.is_finite()
                        || size.width <= 0.0
                        || size.height <= 0.0
                    {
                        return Err(Error::InvalidSize);
                    }
                    let mut atom_path = canonical_path.to_vec();
                    atom_path.push(inline_index);
                    self.atom_paths.insert(media.clone(), NodePath(atom_path));
                    let object = InlineObject::Image {
                        media,
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
        if node.node_type == "codeBlock" {
            let language = node.string_attr("language").unwrap_or_default();
            let highlighted = syntax::highlight(&paragraph.text, language)?;
            self.syntax_reports.push(SyntaxReport {
                canonical_path: NodePath(canonical_path.to_vec()),
                language: language.into(),
                coverage: highlighted.coverage.clone(),
            });
            let palette = self.style.map_or_else(
                || Some(syntax::Palette::for_surface([35, 39, 47], [250, 250, 248])),
                |style| style.syntax_palette.clone(),
            );
            if let Some(palette) = palette {
                apply_syntax_colors(&mut paragraph, &highlighted, &palette)?;
            }
        }
        self.blocks.push(BlockMap {
            canonical: NodePath(canonical_path.to_vec()),
            id,
            path,
            text: node.inline_text(),
        });
        Ok(para_block(paragraph))
    }

    fn block_atom(
        &mut self,
        node: &Node,
        path: Path,
        canonical: NodePath,
        width: f32,
        output: &mut Blocks,
    ) -> Result<()> {
        let mut paragraph = Paragraph::new();
        self.node_paths.insert(path.clone(), canonical.clone());
        let body_size = self.style.map_or(18.0, |style| style.font_size);
        if matches!(node.node_type.as_str(), "horizontalRule" | "sceneBreak") {
            paragraph.props.space_before = Some(12.0);
            paragraph.props.space_after = Some(12.0);
            if node.node_type == "horizontalRule" {
                let mut border = wordcraft_doc::props::Border::single(1.0);
                border.color = Some(rgb(self
                    .style
                    .map_or([139, 148, 163], |style| style.table_border)));
                paragraph.props.borders = Some(wordcraft_doc::props::Borders {
                    bottom: Some(border),
                    ..Default::default()
                });
            } else {
                paragraph.props.align = Some(wordcraft_doc::props::Align::Center);
                paragraph.insert_text(
                    0,
                    "* * *",
                    &CharProps {
                        size: Some(body_size),
                        ..Default::default()
                    },
                )?;
            }
        } else {
            let resolver = self
                .resolver
                .ok_or_else(|| Error::UnsupportedNode("unresolved_block_atom".into()))?;
            let media = format!("craft-geometry-atom:block:{:?}", canonical.0);
            let resolved = self.resolve_atom(resolver, node, width, &media)?;
            if !resolved.size.width.is_finite()
                || !resolved.size.height.is_finite()
                || resolved.size.width <= 0.0
                || resolved.size.height <= 0.0
                || resolved.encoded.is_empty()
                || resolved.encoded.len() > 16 * 1024 * 1024
            {
                return Err(Error::InvalidSize);
            }
            if let Some(style) = self
                .style
                .filter(|style| style.line_height_basis == LineHeightBasis::FontSize)
            {
                // WordCraft's Exactly deliberately ignores tall inline-object metrics. An
                // opaque block is projected as an image paragraph and needs its full bounds.
                paragraph.props.line_spacing = Some(wordcraft_doc::props::LineSpacing::AtLeast(
                    style.font_size * style.line_height,
                ));
            }
            self.media.insert(media.clone(), resolved.encoded);
            self.atom_paths.insert(media.clone(), canonical.clone());
            paragraph.props.align = Some(wordcraft_doc::props::Align::Center);
            paragraph.insert_object(
                0,
                InlineObject::Image {
                    media,
                    w: resolved.size.width,
                    h: resolved.size.height,
                    alt: node
                        .string_attr("alt")
                        .or_else(|| node.string_attr("description"))
                        .unwrap_or_default()
                        .into(),
                    float: Default::default(),
                    crop: [0.0; 4],
                    fx: Default::default(),
                    ole: None,
                },
                &CharProps::default(),
            )?;
        }
        output.push(para_block(paragraph));
        if let Some(caption) = node
            .string_attr("caption")
            .filter(|caption| !caption.is_empty())
        {
            let mut caption_path = path.0;
            if let Some(index) = caption_path.last_mut() {
                *index = index.checked_add(1).ok_or(Error::InvalidGrid)?;
            }
            self.node_paths.insert(Path(caption_path), canonical);
            let mut paragraph = Paragraph::new();
            paragraph.props.align = Some(wordcraft_doc::props::Align::Center);
            paragraph.props.space_after = Some(14.0);
            paragraph.insert_text(
                0,
                caption,
                &CharProps {
                    size: Some(body_size * 0.82),
                    italic: Some(true),
                    ..Default::default()
                },
            )?;
            output.push(para_block(paragraph));
        }
        Ok(())
    }

    fn resolve_atom(
        &mut self,
        resolver: &dyn crate::renderers::AtomResolver,
        node: &Node,
        width: f32,
        media: &str,
    ) -> Result<crate::renderers::ResolvedAtom> {
        match resolver.resolve(node, width) {
            Ok(resolved) => Ok(resolved),
            Err(error) => {
                if let Some(placeholder) = resolver.placeholder(node, width) {
                    let image = placeholder.reserved_image();
                    self.placeholders.insert(media.into(), placeholder);
                    Ok(image)
                } else {
                    Err(match error {
                        crate::Error::AtomRendering(code) => Error::AtomRendering(code),
                        _ => Error::AtomRendering("atom_resolution_failed".into()),
                    })
                }
            }
        }
    }

    fn table(
        &mut self,
        node: &Node,
        native_path: &[u32],
        canonical_path: &[usize],
        width: f32,
    ) -> Result<Table> {
        let maximum = wordcraft_doc::table::MAX_COLS;
        let canonical_grid = craft_editor_model::TableGrid::new(node)?;
        if canonical_grid.columns > maximum {
            return Err(Error::BackendColumnLimit {
                requested: canonical_grid.columns,
                maximum,
            });
        }
        let explicit: f32 = canonical_grid
            .column_widths
            .values()
            .map(|width| *width as f32)
            .sum();
        let automatic = canonical_grid.columns - canonical_grid.column_widths.len();
        let auto_width = if automatic == 0 {
            0.0
        } else {
            ((width - explicit) / automatic as f32).max(25.0)
        };
        let tracks: Vec<f32> = (0..canonical_grid.columns)
            .map(|column| {
                canonical_grid
                    .column_widths
                    .get(&column)
                    .map_or(auto_width, |value| *value as f32)
            })
            .collect();
        self.table_tracks
            .insert(NodePath(canonical_path.to_vec()), tracks.clone());
        self.table_available
            .insert(NodePath(canonical_path.to_vec()), width);
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
                let cell_width = tracks[column..end].iter().sum::<f32>();
                cell.blocks = self.blocks(
                    source_cell.children(),
                    &word_path,
                    &source_path,
                    (cell_width - 18.0).max(1.0),
                )?;
                if let Some(align) = source_cell.string_attr("align") {
                    align_cell_paragraphs(&mut cell.blocks, align);
                }
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
        table.grid = tracks;
        table.props.fixed = true;
        table.props.width = Some(table.grid.iter().sum());
        if table.grid.iter().sum::<f32>() > width + 0.5 {
            table.props.space_after = Some(TABLE_SCROLL_GUTTER);
        }
        if let Some(style) = self.style {
            let mut border = wordcraft_doc::props::Border::single(0.75);
            border.color = Some(rgb(style.table_border));
            table.props.borders = Some(wordcraft_doc::props::Borders::all(border));
            table.props.cell_margins = Some([7.0, 9.0, 7.0, 9.0]);
        }
        Ok(table)
    }
}

fn align_cell_paragraphs(blocks: &mut Blocks, alignment: &str) {
    let align = match alignment {
        "left" => wordcraft_doc::props::Align::Left,
        "center" => wordcraft_doc::props::Align::Center,
        "right" => wordcraft_doc::props::Align::Right,
        _ => return,
    };
    for block in blocks {
        if let Block::Para(paragraph) = Arc::make_mut(block) {
            paragraph.props.align = Some(align);
        }
    }
}

fn merge_node_rect(output: &mut Vec<NodeGeometry>, path: &NodePath, rect: GeometryRect) {
    if let Some(existing) = output
        .iter_mut()
        .find(|node| node.canonical_path == *path && node.rect.page == rect.page)
    {
        let right = (existing.rect.x + existing.rect.width).max(rect.x + rect.width);
        let bottom = (existing.rect.y + existing.rect.height).max(rect.y + rect.height);
        existing.rect.x = existing.rect.x.min(rect.x);
        existing.rect.y = existing.rect.y.min(rect.y);
        existing.rect.width = right - existing.rect.x;
        existing.rect.height = bottom - existing.rect.y;
    } else {
        output.push(NodeGeometry {
            canonical_path: path.clone(),
            rect,
        });
    }
}

fn ordered_label(value: i64, kind: Option<&str>) -> String {
    if value <= 0 {
        return format!("{value}.");
    }
    let label = match kind {
        Some("a" | "A") => {
            let mut number = value as u64;
            let mut letters = Vec::new();
            while number > 0 {
                number -= 1;
                letters.push(b'a' + (number % 26) as u8);
                number /= 26;
            }
            let text: String = letters.into_iter().rev().map(char::from).collect();
            if kind == Some("A") {
                text.to_uppercase()
            } else {
                text
            }
        }
        Some("i" | "I") if value <= 3999 => {
            let mut number = value;
            let mut text = String::new();
            for (amount, label) in [
                (1000, "M"),
                (900, "CM"),
                (500, "D"),
                (400, "CD"),
                (100, "C"),
                (90, "XC"),
                (50, "L"),
                (40, "XL"),
                (10, "X"),
                (9, "IX"),
                (5, "V"),
                (4, "IV"),
                (1, "I"),
            ] {
                while number >= amount {
                    text.push_str(label);
                    number -= amount;
                }
            }
            if kind == Some("i") {
                text.to_lowercase()
            } else {
                text
            }
        }
        _ => value.to_string(),
    };
    format!("{label}.")
}

fn rgb(value: [u8; 3]) -> wordcraft_doc::props::Rgb {
    wordcraft_doc::props::Rgb(value[0], value[1], value[2])
}

fn integer_attr(node: &Node, key: &str) -> usize {
    node.attrs
        .as_ref()
        .and_then(|attrs| attrs.get(key))
        .and_then(|value| {
            value
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .or_else(|| {
                    value
                        .as_f64()
                        .filter(|value| {
                            value.is_finite()
                                && value.fract() == 0.0
                                && *value >= 1.0
                                && *value <= 100.0
                        })
                        .map(|value| value as usize)
                })
        })
        .unwrap_or(1)
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

/// Split disposable formatting runs against token boundaries in one linear pass. Applying only
/// color preserves source bytes, inherited marks, shaping/font metrics and canonical caret maps.
fn apply_syntax_colors(
    paragraph: &mut Paragraph,
    highlighted: &syntax::Highlighted,
    palette: &syntax::Palette,
) -> Result<()> {
    highlighted.validate(&paragraph.text)?;
    let mut runs = Vec::with_capacity(paragraph.runs.len() + highlighted.tokens.len());
    let mut offset = 0;
    let mut token_index = 0;
    for original in &paragraph.runs {
        let end = offset + original.len;
        while offset < end {
            let token = highlighted
                .tokens
                .get(token_index)
                .ok_or(syntax::Error::InvalidTokenRange)?;
            let to = end.min(token.to.0);
            if offset < token.from.0 || to <= offset {
                return Err(syntax::Error::InvalidTokenRange.into());
            }
            let mut props = original.props.clone();
            props.color = Some(wordcraft_doc::props::TextColor::Rgb(rgb(
                palette.color(token.kind)
            )));
            runs.push(wordcraft_doc::para::Run {
                len: to - offset,
                props,
            });
            offset = to;
            if to == token.to.0 {
                token_index += 1;
            }
        }
    }
    if offset != paragraph.text.len() {
        return Err(syntax::Error::InvalidTokenRange.into());
    }
    paragraph.runs = runs;
    paragraph.normalize();
    paragraph.rev = wordcraft_doc::para::next_rev();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_size_line_height_uses_nominal_advance_without_mutating_source() -> Result<()> {
        let source = Document::from_json(
            json!({"type":"doc","attrs":{"schemaVersion":1},"content":[
                {"type":"paragraph","attrs":{"blockId":"p"},"content":[
                    {"type":"text","text":"first"},{"type":"hardBreak"},{"type":"text","text":"second"}
                ]}
            ]}),
        )?;
        let before = source.to_json()?;
        let style = GeometryStyle {
            font_size: 20.0,
            line_height: 1.75,
            line_height_basis: LineHeightBasis::FontSize,
            standard_ligatures: Some(false),
            ..Default::default()
        };
        let geometry = FlowGeometry::with_style(&source, 800.0, &|_| None, Some(&style))?;
        assert_eq!(
            geometry.upstream.styles.default_para.line_spacing,
            Some(wordcraft_doc::props::LineSpacing::Exactly(35.0))
        );
        let first = geometry
            .caret(&point("p", 0))?
            .ok_or(Error::UnmappedPosition)?;
        let second = geometry
            .caret(&point("p", 6))?
            .ok_or(Error::UnmappedPosition)?;
        assert!(
            (second.y - first.y - 35.0).abs() < 0.05,
            "actual line advance differs from 35 points"
        );
        assert_eq!(
            geometry.upstream.styles.default_chr.ligatures,
            Some(wordcraft_doc::props::Ligatures {
                standard: false,
                contextual: true,
                historical: false,
                discretional: false,
            })
        );
        assert_eq!(source.to_json()?, before);
        Ok(())
    }

    #[test]
    fn list_item_gap_is_independent_from_ordinary_paragraph_gap() -> Result<()> {
        let paragraph = |id: &str, text: &str| json!({"type":"paragraph","attrs":{"blockId":id},"content":[{"type":"text","text":text}]});
        let source =
            Document::from_json(json!({"type":"doc","attrs":{"schemaVersion":1},"content":[
                {"type":"bulletList","attrs":{"blockId":"list"},"content":[
                    {"type":"listItem","attrs":{"blockId":"one"},"content":[paragraph("p1","one")]},
                    {"type":"listItem","attrs":{"blockId":"two"},"content":[paragraph("p2","two")]}
                ]}, paragraph("after","after list")
            ]}))?;
        let mut style = GeometryStyle {
            paragraph_spacing: 40.0,
            list_item_spacing: Some(0.0),
            ..Default::default()
        };
        let tight = FlowGeometry::with_style(&source, 800.0, &|_| None, Some(&style))?;
        style.list_item_spacing = Some(8.0);
        let loose = FlowGeometry::with_style(&source, 800.0, &|_| None, Some(&style))?;
        let first = loose
            .upstream
            .para_at(&Pos::body(0, 0))
            .ok_or(Error::UnmappedPosition)?;
        let second = loose
            .upstream
            .para_at(&Pos::body(1, 0))
            .ok_or(Error::UnmappedPosition)?;
        assert_eq!(first.props.space_after, Some(0.0));
        assert_eq!(first.props.space_before, None);
        assert_eq!(second.props.space_after, Some(0.0));
        assert_eq!(second.props.space_before, Some(8.0));
        let tight_y = tight
            .caret(&point("p2", 0))?
            .ok_or(Error::UnmappedPosition)?
            .y;
        let loose_y = loose
            .caret(&point("p2", 0))?
            .ok_or(Error::UnmappedPosition)?
            .y;
        assert!((loose_y - tight_y - 8.0).abs() < 0.05);
        assert_eq!(loose.upstream.styles.default_para.space_after, Some(40.0));
        Ok(())
    }

    #[test]
    fn visible_text_index_tracks_scrolled_unicode_lines_without_reader_rewrite() -> Result<()> {
        let paragraphs: Vec<_> = (0..100)
            .map(|index| {
                json!({
                    "type":"paragraph","attrs":{"blockId":format!("p{index}")},
                    "content":[{"type":"text","text":format!("Paragraph {index} — Я😀 e\u{301}")}]
                })
            })
            .collect();
        let source = Document::from_json(
            json!({"type":"doc","attrs":{"schemaVersion":1},"content":paragraphs}),
        )?;
        let before = source.to_json()?;
        let geometry = FlowGeometry::new(&source, 700.0)?;
        let first = geometry.visible_text_ranges(GeometryRect {
            page: 0,
            x: 0.0,
            y: 0.0,
            width: 700.0,
            height: 80.0,
        });
        assert!(first.iter().any(|range| range.anchor.block_id == "p0"));
        assert!(!first.iter().any(|range| range.anchor.block_id == "p99"));
        let last = geometry
            .caret(&point("p99", 0))?
            .ok_or(Error::UnmappedPosition)?;
        let scrolled = geometry.visible_text_ranges(GeometryRect {
            page: 0,
            x: 0.0,
            y: last.y,
            width: 700.0,
            height: 80.0,
        });
        assert!(scrolled.iter().any(|range| range.anchor.block_id == "p99"));
        assert!(!scrolled.iter().any(|range| range.anchor.block_id == "p0"));
        for range in first.iter().chain(&scrolled) {
            assert!(range.anchor.offset < range.head.offset);
            geometry.to_native(&range.anchor)?;
            geometry.to_native(&range.head)?;
        }
        assert_eq!(source.to_json()?, before);
        Ok(())
    }

    #[test]
    fn invalid_list_gap_never_changes_the_canonical_document() -> Result<()> {
        let source = Document::plain("p", "unchanged")?;
        let before = source.to_json()?;
        for gap in [-1.0, f32::NAN, f32::INFINITY] {
            let style = GeometryStyle {
                list_item_spacing: Some(gap),
                ..Default::default()
            };
            assert!(matches!(
                FlowGeometry::with_style(&source, 600.0, &|_| None, Some(&style)),
                Err(Error::InvalidSize)
            ));
        }
        assert_eq!(source.to_json()?, before);
        Ok(())
    }

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
    #[test]
    fn legacy_integral_json_spans_keep_their_real_geometry() -> Result<()> {
        let mut fixture = qualification_fixture();
        fixture["content"][1]["content"][0]["content"][0]["attrs"]["colspan"] = json!(2.0);
        fixture["content"][1]["content"][0]["content"][0]["attrs"]["rowspan"] = json!(2.0);
        let source = Document::from_json(fixture)?;
        let before = source.to_json()?;
        let geometry = FlowGeometry::with_atom_sizes(&source, 600.0, &|_| {
            Some(AtomSize {
                width: 48.0,
                height: 24.0,
            })
        })?;
        let cells = geometry.cells()?;
        let merged = cells
            .iter()
            .find(|cell| cell.canonical_path == NodePath(vec![1, 0, 0]) && !cell.covered)
            .ok_or(Error::InvalidGrid)?;
        let right = cells
            .iter()
            .find(|cell| cell.canonical_path == NodePath(vec![1, 0, 1]))
            .ok_or(Error::InvalidGrid)?;
        assert!(merged.rect.width > right.rect.width * 1.8);
        assert!(merged.rect.height > right.rect.height * 1.5);
        assert_eq!(source.to_json()?, before);
        Ok(())
    }
    fn code_document(language: &str, text: &str) -> Result<Document> {
        Ok(Document::from_json(
            json!({"type":"doc","attrs":{"schemaVersion":4},"content":[
                {"type":"codeBlock","attrs":{"blockId":"code","language":language,"vendor":{"preserve":true}},"content":[{"type":"text","text":text}]}
            ]}),
        )?)
    }
    #[test]
    fn syntax_colors_leave_every_canonical_byte_mark_and_caret_position_unchanged() -> Result<()> {
        let text = "// Русский 日本語 🪶\nlet 名 = \"e\u{301}\";\n\t名 += 42;";
        let source = code_document("rust", text)?;
        let canonical = source.to_json()?;
        let highlighted_style = GeometryStyle::default();
        let mut plain_style = highlighted_style.clone();
        plain_style.syntax_palette = None;
        let highlighted =
            FlowGeometry::with_style(&source, 700.0, &|_| None, Some(&highlighted_style))?;
        let plain = FlowGeometry::with_style(&source, 700.0, &|_| None, Some(&plain_style))?;
        for offset in 0..=text.chars().count() {
            let point = point("code", offset);
            let a = highlighted.caret(&point)?.ok_or(Error::UnmappedPosition)?;
            let b = plain.caret(&point)?.ok_or(Error::UnmappedPosition)?;
            assert!(
                (a.x - b.x).abs() < 0.001
                    && (a.y - b.y).abs() < 0.001
                    && (a.height - b.height).abs() < 0.001,
                "syntax changed caret at {offset}"
            );
        }
        let pos = Pos::body(0, 0);
        let paragraph = highlighted
            .upstream
            .para_at(&pos)
            .ok_or(Error::UnmappedPosition)?;
        assert_eq!(paragraph.text, text);
        assert!(
            paragraph
                .runs
                .iter()
                .filter_map(|run| run.props.color.as_ref())
                .collect::<std::collections::HashSet<_>>()
                .len()
                > 2
        );
        assert_eq!(source.to_json()?, canonical);
        Ok(())
    }
    #[test]
    fn imported_unknown_code_language_is_preserved_and_reported_as_unknown() -> Result<()> {
        let source = code_document("private-author-format", "unchanged source")?;
        let before = source.to_json()?;
        let geometry = FlowGeometry::new(&source, 600.0)?;
        assert!(matches!(
            geometry.syntax_reports()[0].coverage,
            syntax::Coverage::UnknownLanguage { .. }
        ));
        assert_eq!(
            geometry.syntax_reports()[0].language,
            "private-author-format"
        );
        assert_eq!(source.to_json()?, before);
        Ok(())
    }
    #[test]
    fn host_token_palette_changes_colors_without_changing_font_or_source() -> Result<()> {
        let source = code_document("rust", "let value = 42;")?;
        let mut style = GeometryStyle::default();
        let mut palette = syntax::Palette::for_surface([230, 234, 240], [29, 34, 42]);
        palette.keyword = [190, 130, 240];
        style.syntax_palette = Some(palette);
        let geometry = FlowGeometry::with_style(&source, 600.0, &|_| None, Some(&style))?;
        let paragraph = geometry
            .upstream
            .para_at(&Pos::body(0, 0))
            .ok_or(Error::UnmappedPosition)?;
        assert!(paragraph.runs.iter().any(|run| run.props.color
            == Some(wordcraft_doc::props::TextColor::Rgb(rgb([190, 130, 240])))));
        assert!(
            paragraph
                .runs
                .iter()
                .all(|run| run.props.font.as_deref() == Some("JetBrains Mono"))
        );
        Ok(())
    }
    #[test]
    fn canonical_widths_merged_boundaries_and_cell_alignment_match_actual_native_tracks()
    -> Result<()> {
        let source = Document::from_json(
            json!({"type":"doc","attrs":{"schemaVersion":4},"content":[
                {"type":"table","attrs":{"blockId":"table"},"content":[
                    {"type":"tableRow","content":[
                        {"type":"tableHeader","attrs":{"colspan":2,"rowspan":1,"colwidth":[120,240],"align":"right"},"content":[{"type":"paragraph","attrs":{"blockId":"merged"},"content":[{"type":"text","text":"Retained merged text"}]}]}
                    ]},
                    {"type":"tableRow","content":[
                        {"type":"tableCell","attrs":{"colwidth":[120]},"content":[{"type":"paragraph","attrs":{"blockId":"left"},"content":[{"type":"text","text":"Left"}]}]},
                        {"type":"tableCell","attrs":{"colwidth":[240],"align":"center"},"content":[{"type":"paragraph","attrs":{"blockId":"right"},"content":[{"type":"text","text":"Right"}]}]}
                    ]}
                ]}
            ]}),
        )?;
        let before = source.to_json()?;
        let geometry = FlowGeometry::new(&source, 600.0)?;
        let columns = geometry.columns(&source)?;
        assert_eq!(columns.len(), 2);
        let first = columns
            .iter()
            .find(|column| column.column == 0)
            .ok_or(Error::InvalidGrid)?;
        let second = columns
            .iter()
            .find(|column| column.column == 1)
            .ok_or(Error::InvalidGrid)?;
        assert!((first.rect.width - 120.0).abs() < 0.1);
        assert!((second.rect.width - 240.0).abs() < 0.1);
        assert!((second.rect.x - first.rect.x - 120.0).abs() < 0.1);
        let cells = geometry.cells()?;
        let left = cells
            .iter()
            .find(|cell| cell.canonical_path == NodePath(vec![0, 1, 0]))
            .ok_or(Error::InvalidGrid)?;
        let right = cells
            .iter()
            .find(|cell| cell.canonical_path == NodePath(vec![0, 1, 1]))
            .ok_or(Error::InvalidGrid)?;
        assert!((left.rect.width - 120.0).abs() < 0.1);
        assert!((right.rect.width - 240.0).abs() < 0.1);
        let table = geometry.upstream.body[0]
            .as_table()
            .ok_or(Error::InvalidGrid)?;
        let merged = table.rows[0].cells[0].blocks[0]
            .as_para()
            .ok_or(Error::InvalidGrid)?;
        let right = table.rows[1].cells[1].blocks[0]
            .as_para()
            .ok_or(Error::InvalidGrid)?;
        assert_eq!(merged.props.align, Some(wordcraft_doc::props::Align::Right));
        assert_eq!(right.props.align, Some(wordcraft_doc::props::Align::Center));
        assert_eq!(source.to_json()?, before);
        Ok(())
    }

    #[test]
    fn automatic_columns_use_remaining_space_and_nested_tables_stay_inside_their_cell() -> Result<()>
    {
        let source = Document::from_json(
            json!({"type":"doc","attrs":{"schemaVersion":4},"content":[
                {"type":"table","attrs":{"blockId":"outer"},"content":[{"type":"tableRow","content":[
                    {"type":"tableCell","attrs":{"colwidth":[180]},"content":[{"type":"table","attrs":{"blockId":"nested"},"content":[{"type":"tableRow","content":[
                        {"type":"tableCell","content":[{"type":"paragraph","attrs":{"blockId":"inside"},"content":[{"type":"text","text":"Nested"}]}]}
                    ]}]}]},
                    {"type":"tableCell","content":[{"type":"paragraph","attrs":{"blockId":"right"},"content":[{"type":"text","text":"Auto"}]}]}
                ]}]}
            ]}),
        )?;
        let before = source.to_json()?;
        let geometry = FlowGeometry::new(&source, 600.0)?;
        let table = geometry.upstream.body[0]
            .as_table()
            .ok_or(Error::InvalidGrid)?;
        assert_eq!(table.grid, vec![180.0, 384.0]);
        let nested = table.rows[0].cells[0].blocks[0]
            .as_table()
            .ok_or(Error::InvalidGrid)?;
        assert!(nested.grid.iter().sum::<f32>() <= 162.0);
        let columns = geometry.columns(&source)?;
        assert_eq!(
            columns
                .iter()
                .filter(|column| column.table_path == NodePath(vec![0, 0, 0, 0]))
                .count(),
            1
        );
        assert_eq!(source.to_json()?, before);
        Ok(())
    }
}

#[cfg(test)]
mod table_viewport_tests;
