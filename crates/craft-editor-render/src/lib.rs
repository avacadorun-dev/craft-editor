//! Native Mermaid, KaTeX-compatible formula and image rendering over canonical atoms.
//!
//! Prepare a catalog on a host-owned worker, then give the immutable catalog to the editor.
//! Rendering never rewrites source nodes or performs hidden network/filesystem media access.
//! A renderer fixture is not full native editor or upstream syntax-parity acceptance.
#![deny(missing_docs)]

mod media;

use craft_editor_egui::{
    layout::AtomSize,
    renderers::{AtomResolver, ResolvedAtom},
};
use craft_editor_model::{Document, Node};
use merman::{Engine, MermaidConfig, OperationControl, RenderOutput, RenderRequest, SvgRequest};
use ratex_types::{color::Color, math_style::MathStyle};
use serde_json::json;
use std::{collections::HashMap, sync::Arc, time::Duration};

/// Content-free diagnostics; hosts supply localized explanations and retain source.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// Render configuration is not finite, positive or internally consistent.
    #[error("invalid_render_options")]
    InvalidOptions,
    /// Input, output or work exceeded the configured bounds.
    #[error("render_resource_limit")]
    ResourceLimit,
    /// No host-authorized media bytes match this image identity.
    #[error("asset_unavailable")]
    AssetUnavailable,
    /// The supplied SVG references a resource outside the supplied bytes.
    #[error("external_svg_resource")]
    ExternalResource,
    /// Raster bytes or SVG could not be decoded.
    #[error("image_decode_failed")]
    ImageDecode,
    /// Native SVG text has no usable bundled or supplied font face.
    #[error("font_unavailable")]
    FontUnavailable,
    /// Mermaid rejected source, an unavailable capability or rendering work.
    #[error("diagram_render_failed")]
    DiagramRender,
    /// The source has no diagram after parsing.
    #[error("diagram_missing")]
    DiagramMissing,
    /// Formula parsing or layout failed.
    #[error("formula_render_failed")]
    FormulaRender,
    /// The node is not a supported media/formula/diagram atom.
    #[error("unsupported_atom")]
    UnsupportedAtom,
    /// Node identity could not be encoded for the immutable catalog.
    #[error("atom_identity_failed")]
    Identity,
}
/// Renderer result with no manuscript-bearing error strings.
pub type Result<T> = std::result::Result<T, Error>;

/// Explicit bounds for input, output, and a document's atom collection.
#[derive(Debug, Clone)]
pub struct RenderLimits {
    /// Maximum Mermaid input bytes; the source schema allows 100,000.
    pub diagram_bytes: usize,
    /// Maximum formula input bytes; the source schema allows 10,000.
    pub formula_bytes: usize,
    /// Maximum bytes supplied for one raster/SVG asset.
    pub asset_bytes: usize,
    /// Maximum width/height of any decoded or generated raster.
    pub dimension: u32,
    /// Maximum pixels in one decoded/generated raster, before allocating RGBA.
    pub pixels: u64,
    /// Maximum distinct atom nodes prepared in one catalog.
    pub atoms: usize,
    /// Maximum aggregate encoded output bytes held by a catalog.
    pub catalog_bytes: usize,
    /// Cooperative native Mermaid deadline for one atom.
    pub diagram_timeout: Duration,
}
impl Default for RenderLimits {
    fn default() -> Self {
        Self {
            diagram_bytes: 100_000,
            formula_bytes: 10_000,
            asset_bytes: 16 * 1024 * 1024,
            dimension: 20_000,
            pixels: 16 * 1024 * 1024,
            atoms: 256,
            catalog_bytes: 64 * 1024 * 1024,
            diagram_timeout: Duration::from_secs(3),
        }
    }
}

/// Host-localized labels used only for explicitly failed source renderings.
#[derive(Debug, Clone)]
pub struct FallbackLabels {
    /// Diagram error label, followed by the preserved diagram source.
    pub diagram: String,
    /// Formula error label, followed by the preserved LaTeX source.
    pub formula: String,
    /// Image error label, followed by its authored alternative text.
    pub image: String,
}
impl Default for FallbackLabels {
    fn default() -> Self {
        Self {
            diagram: "Diagram could not be rendered".into(),
            formula: "Formula could not be rendered".into(),
            image: "Image could not be loaded".into(),
        }
    }
}

/// Source-independent presentation and resource policy; changes require a fresh catalog generation.
#[derive(Debug, Clone)]
pub struct RenderOptions {
    /// Formula size in logical document points.
    pub font_size: f32,
    /// Raster pixels per logical point, normally the display's scale factor.
    pub pixel_scale: f32,
    /// Formula and fallback text RGBA color.
    pub foreground: [u8; 4],
    /// Fallback canvas RGBA color. Successful formulas retain transparency.
    pub background: [u8; 4],
    /// Select the native Mermaid dark or default theme.
    pub dark: bool,
    /// Requested native SVG label family.
    pub font_family: String,
    /// Additional authorized font bytes. Defaults already include WordCraft's 13 OFL faces.
    pub font_faces: Vec<Arc<Vec<u8>>>,
    /// Input/output limits.
    pub limits: RenderLimits,
    /// Host-localized failure labels; valid source never uses these fallbacks.
    pub fallback_labels: FallbackLabels,
}
impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            font_size: 20.0,
            pixel_scale: 2.0,
            foreground: [35, 39, 47, 255],
            background: [250, 250, 248, 255],
            dark: false,
            font_family: "Source Sans 3".into(),
            font_faces: vec![],
            limits: RenderLimits::default(),
            fallback_labels: FallbackLabels::default(),
        }
    }
}

/// Host-owned bytes for exact canonical image nodes; no ambient path or URL lookup is allowed.
pub trait AssetSource: Send + Sync {
    /// Supply bytes already authorized by the application/media owner.
    ///
    /// # Errors
    /// Return `AssetUnavailable` when the referenced bytes are absent or inaccessible.
    fn load(&self, node: &Node) -> Result<Arc<Vec<u8>>>;
}

/// In-memory asset source, suitable for a host's acknowledged media snapshot.
#[derive(Debug, Clone, Default)]
pub struct AssetMap {
    assets: HashMap<String, Arc<Vec<u8>>>,
}
impl AssetMap {
    /// Store bytes under an asset ID or an exact source key; no I/O is performed.
    pub fn insert(
        &mut self,
        identity: impl Into<String>,
        bytes: Arc<Vec<u8>>,
    ) -> Option<Arc<Vec<u8>>> {
        self.assets.insert(identity.into(), bytes)
    }
}
impl AssetSource for AssetMap {
    fn load(&self, node: &Node) -> Result<Arc<Vec<u8>>> {
        let key = node
            .string_attr("assetId")
            .or_else(|| node.string_attr("src"))
            .ok_or(Error::AssetUnavailable)?;
        self.assets.get(key).cloned().ok_or(Error::AssetUnavailable)
    }
}

/// A real raster and its measured logical geometry, independent of a particular viewport.
#[derive(Debug, Clone)]
pub struct PreparedImage {
    /// Native logical bounds before fitting to the editor width.
    pub size: AtomSize,
    /// Inline baseline measured from the top, including formula padding.
    pub baseline: Option<f32>,
    /// Encoded PNG from the native renderer/decoder.
    pub encoded: Arc<Vec<u8>>,
}
impl PreparedImage {
    /// Fit down to a viewport without enlarging or rerendering the cached bitmap.
    ///
    /// # Errors
    /// Reject a nonfinite or nonpositive available width.
    pub fn fit(&self, available_width: f32) -> Result<ResolvedAtom> {
        if !available_width.is_finite() || available_width <= 0.0 {
            return Err(Error::InvalidOptions);
        }
        let scale = (available_width / self.size.width).min(1.0);
        Ok(ResolvedAtom {
            size: AtomSize {
                width: self.size.width * scale,
                height: self.size.height * scale,
            },
            baseline: self.baseline.map(|baseline| baseline * scale),
            encoded: Arc::clone(&self.encoded),
        })
    }
}

/// One explicit source rendering failure; authored source remains in the host's canonical node.
#[derive(Debug, Clone)]
pub struct RenderFailure {
    /// Exact child ordinals of the failing node in the prepared snapshot.
    pub path: Vec<usize>,
    /// Canonical atom kind.
    pub node_type: String,
    /// Existing durable block identity, when the schema supplies one.
    pub block_id: Option<String>,
    /// Content-free reason. The catalog's image is a clearly labelled source fallback.
    pub error: Error,
}

/// Immutable worker-prepared output. Resolution never parses, decodes, opens files or uses a network.
#[derive(Debug, Clone)]
pub struct PreparedCatalog {
    generation: u64,
    images: HashMap<String, PreparedImage>,
    failures: Vec<RenderFailure>,
    errors: HashMap<String, Error>,
}
impl PreparedCatalog {
    /// Per-node failures, including the explicit source fallback entries.
    pub fn failures(&self) -> &[RenderFailure] {
        &self.failures
    }
    /// Number of distinct prepared atoms, including labelled failures.
    pub fn len(&self) -> usize {
        self.images.len()
    }
    /// Whether no atoms were present in the snapshot.
    pub fn is_empty(&self) -> bool {
        self.images.is_empty()
    }
}
impl AtomResolver for PreparedCatalog {
    fn resolve(&self, node: &Node, width: f32) -> craft_editor_egui::Result<ResolvedAtom> {
        let key = identity(node).map_err(view_error)?;
        self.images
            .get(&key)
            .ok_or_else(|| view_error(Error::AssetUnavailable))?
            .fit(width)
            .map_err(view_error)
    }
    fn generation(&self) -> u64 {
        self.generation
    }
}

/// Reusable CPU renderer. Construct and prepare on the host's bounded worker, not in a paint callback.
pub struct Renderer {
    options: RenderOptions,
    svg: usvg::Options<'static>,
    diagrams: merman::Renderer,
}
impl Renderer {
    /// Build a deterministic native font/config environment.
    ///
    /// # Errors
    /// Reject invalid options or an unusable native SVG font database.
    pub fn new(options: RenderOptions) -> Result<Self> {
        if !options.font_size.is_finite()
            || options.font_size <= 0.0
            || options.font_size > 256.0
            || !options.pixel_scale.is_finite()
            || options.pixel_scale <= 0.0
            || options.pixel_scale > 4.0
            || options.limits.dimension == 0
            || options.limits.pixels == 0
            || options.limits.atoms == 0
            || options.limits.catalog_bytes == 0
        {
            return Err(Error::InvalidOptions);
        }
        let mut svg = usvg::Options {
            font_family: options.font_family.clone(),
            ..Default::default()
        };
        for face in wordcraft_fonts::bundled() {
            svg.fontdb_mut().load_font_data(face.to_vec());
        }
        for face in &options.font_faces {
            svg.fontdb_mut().load_font_data(face.as_ref().clone());
        }
        svg.fontdb_mut()
            .set_sans_serif_family(options.font_family.clone());
        svg.fontdb_mut().set_serif_family("Source Serif 4");
        svg.fontdb_mut().set_monospace_family("JetBrains Mono");
        if svg.fontdb.faces().next().is_none() {
            return Err(Error::FontUnavailable);
        }
        let config = MermaidConfig::from_value(
            json!({"theme": if options.dark { "dark" } else { "default" },
            "fontFamily": options.font_family, "securityLevel":"strict"}),
        );
        let diagrams = merman::Renderer::new().with_engine(Engine::new().with_site_config(config));
        Ok(Self {
            options,
            svg,
            diagrams,
        })
    }

    /// Render one actual diagram/formula/media atom, without a source fallback.
    ///
    /// # Errors
    /// Return the precise content-free renderer/resource error; never mutate the node.
    pub fn render_atom(&self, node: &Node, assets: &dyn AssetSource) -> Result<PreparedImage> {
        match node.node_type.as_str() {
            "mermaidBlock" => self.diagram(node),
            "inlineMath" | "mathBlock" => self.formula(node),
            "imageBlock" => self.image(node, assets),
            _ => Err(Error::UnsupportedAtom),
        }
    }

    /// Prepare all canonical atoms. Failed source becomes an explicit labelled native fallback.
    ///
    /// # Errors
    /// Reject aggregate limits, unavailable fallback rendering, or unencodable node identity.
    /// Per-node render failures are retained in `PreparedCatalog::failures` instead of being hidden.
    pub fn prepare_document(
        &self,
        document: &Document,
        assets: &dyn AssetSource,
        generation: u64,
    ) -> Result<PreparedCatalog> {
        let mut catalog = PreparedCatalog {
            generation,
            images: HashMap::new(),
            failures: vec![],
            errors: HashMap::new(),
        };
        let mut total = 0;
        self.visit(
            document.root(),
            &mut vec![],
            assets,
            &mut catalog,
            &mut total,
        )?;
        Ok(catalog)
    }

    fn visit(
        &self,
        node: &Node,
        path: &mut Vec<usize>,
        assets: &dyn AssetSource,
        catalog: &mut PreparedCatalog,
        total: &mut usize,
    ) -> Result<()> {
        if matches!(
            node.node_type.as_str(),
            "mermaidBlock" | "mathBlock" | "inlineMath" | "imageBlock"
        ) {
            let key = identity(node)?;
            if !catalog.images.contains_key(&key) {
                if catalog.images.len() >= self.options.limits.atoms {
                    return Err(Error::ResourceLimit);
                }
                let image = match self.render_atom(node, assets) {
                    Ok(image) => image,
                    Err(error) => {
                        catalog.errors.insert(key.clone(), error);
                        self.fallback(node)?
                    }
                };
                *total = total
                    .checked_add(image.encoded.len())
                    .ok_or(Error::ResourceLimit)?;
                if *total > self.options.limits.catalog_bytes {
                    return Err(Error::ResourceLimit);
                }
                catalog.images.insert(key.clone(), image);
            }
            if let Some(error) = catalog.errors.get(&key) {
                catalog.failures.push(RenderFailure {
                    path: path.clone(),
                    node_type: node.node_type.clone(),
                    block_id: node.block_id().map(str::to_owned),
                    error: error.clone(),
                });
            }
        }
        for (index, child) in node.children().iter().enumerate() {
            path.push(index);
            self.visit(child, path, assets, catalog, total)?;
            path.pop();
        }
        Ok(())
    }

    fn diagram(&self, node: &Node) -> Result<PreparedImage> {
        let source = node.string_attr("code").ok_or(Error::DiagramMissing)?;
        if source.len() > self.options.limits.diagram_bytes {
            return Err(Error::ResourceLimit);
        }
        let request = SvgRequest {
            pipeline: Some(merman::svg::SvgPipeline::resvg_safe()),
            ..Default::default()
        };
        let control = OperationControl::new().with_deadline(self.options.limits.diagram_timeout);
        let output = self
            .diagrams
            .render(RenderRequest::svg(source, control, request))
            .map_err(|_| Error::DiagramRender)?;
        let RenderOutput::Svg(Some(svg)) = output else {
            return Err(Error::DiagramMissing);
        };
        self.svg_image(svg.svg().as_bytes())
    }

    fn formula(&self, node: &Node) -> Result<PreparedImage> {
        let source = node.string_attr("latex").ok_or(Error::FormulaRender)?;
        if source.len() > self.options.limits.formula_bytes {
            return Err(Error::ResourceLimit);
        }
        if source.trim().is_empty() {
            return Err(Error::FormulaRender);
        }
        let ast = ratex_parser::parser::parse(source).map_err(|_| Error::FormulaRender)?;
        let [r, g, b, a] = self
            .options
            .foreground
            .map(|channel| f32::from(channel) / 255.0);
        let style = if node.node_type == "inlineMath" {
            MathStyle::Text
        } else {
            MathStyle::Display
        };
        let options = ratex_layout::LayoutOptions::default()
            .with_style(style)
            .with_color(Color::new(r, g, b, a));
        let layout = ratex_layout::layout(&ast, &options);
        ratex_layout::validate_layout(&layout).map_err(|_| Error::FormulaRender)?;
        let list = ratex_layout::to_display_list(&layout);
        let padding = if node.node_type == "inlineMath" {
            1.0
        } else {
            8.0
        };
        let em = self.options.font_size;
        let width = (list.width as f32 * em + padding * 2.0).max(1.0);
        let height = ((list.height + list.depth) as f32 * em + padding * 2.0).max(1.0);
        self.check_size(
            width * self.options.pixel_scale,
            height * self.options.pixel_scale,
        )?;
        let encoded = ratex_render::render_to_png(
            &list,
            &ratex_render::RenderOptions {
                font_size: em,
                padding,
                background_color: Color::new(0.0, 0.0, 0.0, 0.0),
                device_pixel_ratio: self.options.pixel_scale,
                ..Default::default()
            },
        )
        .map_err(|_| Error::FormulaRender)?;
        let (pixel_width, pixel_height) = self.dimensions(&encoded)?;
        Ok(PreparedImage {
            size: AtomSize {
                width: pixel_width as f32 / self.options.pixel_scale,
                height: pixel_height as f32 / self.options.pixel_scale,
            },
            baseline: Some(padding + list.height as f32 * em),
            encoded: Arc::new(encoded),
        })
    }
}

fn identity(node: &Node) -> Result<String> {
    serde_json::to_string(node).map_err(|_| Error::Identity)
}
fn view_error(error: Error) -> craft_editor_egui::Error {
    craft_editor_egui::Error::AtomRendering(error.to_string())
}
