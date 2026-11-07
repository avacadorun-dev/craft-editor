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
        if !width.is_finite() || width <= 0.0 {
            return Err(Error::InvalidSize);
        }
        let mut builder = Builder {
            blocks: Vec::new(),
            cell_paths: HashMap::new(),
            atom_size,
        };
        let body = builder.blocks(source.root().children(), &[], &[], width)?;
        let mut upstream = wordcraft_doc::Document::new();
        upstream.body = body;
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
                    "paragraph" => self.paragraph(node, Path(native_path)),
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
        for inline in node.children() {
            let mut props = CharProps::default();
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
                    paragraph.insert_text(paragraph.len(), text, &props)?;
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
        Ok(table)
    }
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
        "colspan_100":{"supported":false,"backend_limit":wordcraft_doc::table::MAX_COLS,"blocks_full_parity":true},
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
