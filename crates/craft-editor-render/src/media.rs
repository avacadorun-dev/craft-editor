use std::{
    io::Cursor,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use craft_editor_egui::layout::AtomSize;
use craft_editor_model::Node;
use image::{ImageFormat, ImageReader};

use super::{AssetSource, Error, PreparedImage, RenderLimits, Renderer, Result};

impl Renderer {
    pub(super) fn check_size(&self, width: f32, height: f32) -> Result<(u32, u32)> {
        if !width.is_finite()
            || !height.is_finite()
            || width <= 0.0
            || height <= 0.0
            || width.ceil() > self.options.limits.dimension as f32
            || height.ceil() > self.options.limits.dimension as f32
        {
            return Err(Error::ResourceLimit);
        }
        let width = width.ceil() as u32;
        let height = height.ceil() as u32;
        if u64::from(width) * u64::from(height) > self.options.limits.pixels {
            return Err(Error::ResourceLimit);
        }
        Ok((width, height))
    }

    pub(super) fn dimensions(&self, bytes: &[u8]) -> Result<(u32, u32)> {
        raster_dimensions(bytes, &self.options.limits)
    }

    pub(super) fn image(&self, node: &Node, assets: &dyn AssetSource) -> Result<PreparedImage> {
        let bytes = assets.load(node)?;
        if bytes.len() > self.options.limits.asset_bytes {
            return Err(Error::ResourceLimit);
        }
        let mut image = if image::guess_format(&bytes).is_ok() {
            let (width, height) = self.dimensions(&bytes)?;
            let mut reader = ImageReader::new(Cursor::new(bytes.as_slice()))
                .with_guessed_format()
                .map_err(|_| Error::ImageDecode)?;
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(self.options.limits.dimension);
            limits.max_image_height = Some(self.options.limits.dimension);
            limits.max_alloc = Some(self.options.limits.pixels.saturating_mul(8));
            reader.limits(limits);
            let decoded = reader.decode().map_err(|_| Error::ImageDecode)?;
            let mut output = Cursor::new(Vec::new());
            decoded
                .write_to(&mut output, ImageFormat::Png)
                .map_err(|_| Error::ImageDecode)?;
            PreparedImage {
                size: AtomSize {
                    width: width as f32,
                    height: height as f32,
                },
                baseline: None,
                encoded: Arc::new(output.into_inner()),
            }
        } else {
            self.svg_image(&bytes)?
        };
        let width = positive_attr(node, "width")?;
        let height = positive_attr(node, "height")?;
        image.size = match (width, height) {
            (Some(width), Some(height)) => AtomSize { width, height },
            (Some(width), None) => AtomSize {
                width,
                height: image.size.height * width / image.size.width,
            },
            (None, Some(height)) => AtomSize {
                width: image.size.width * height / image.size.height,
                height,
            },
            (None, None) => image.size,
        };
        if !image.size.width.is_finite()
            || !image.size.height.is_finite()
            || image.size.width <= 0.0
            || image.size.height <= 0.0
        {
            return Err(Error::InvalidOptions);
        }
        Ok(image)
    }

    pub(super) fn svg_image(&self, bytes: &[u8]) -> Result<PreparedImage> {
        if bytes.len() > self.options.limits.asset_bytes {
            return Err(Error::ResourceLimit);
        }
        let text = std::str::from_utf8(bytes).map_err(|_| Error::ImageDecode)?;
        let xml = roxmltree::Document::parse_with_options(
            text,
            roxmltree::ParsingOptions {
                nodes_limit: 100_000,
                ..Default::default()
            },
        )
        .map_err(|_| Error::ImageDecode)?;
        if xml.root_element().tag_name().name() != "svg" {
            return Err(Error::ImageDecode);
        }
        for element in xml.descendants().filter(roxmltree::Node::is_element) {
            if matches!(element.tag_name().name(), "script" | "foreignObject") {
                return Err(Error::ImageDecode);
            }
            for attr in element.attributes() {
                if attr.name() == "href"
                    && element.tag_name().name() != "a"
                    && !attr.value().starts_with('#')
                    && !attr.value().starts_with("data:")
                {
                    return Err(Error::ExternalResource);
                }
            }
        }
        let failed = Arc::new(AtomicBool::new(false));
        let failed_data = Arc::clone(&failed);
        let limits = self.options.limits.clone();
        let default_data = usvg::ImageHrefResolver::default_data_resolver();
        let options = usvg::Options {
            font_family: self.svg.font_family.clone(),
            fontdb: Arc::clone(&self.svg.fontdb),
            image_href_resolver: usvg::ImageHrefResolver {
                resolve_string: Box::new(|_, _| None),
                resolve_data: Box::new(move |mime, data, options| {
                    if data.len() > limits.asset_bytes || raster_dimensions(&data, &limits).is_err()
                    {
                        failed_data.store(true, Ordering::Relaxed);
                        None
                    } else {
                        default_data(mime, data, options)
                    }
                }),
            },
            ..Default::default()
        };
        let tree = usvg::Tree::from_data(bytes, &options).map_err(|_| Error::ImageDecode)?;
        if failed.load(Ordering::Relaxed) {
            return Err(Error::ImageDecode);
        }
        let size = tree.size();
        let (width, height) = self.check_size(
            size.width() * self.options.pixel_scale,
            size.height() * self.options.pixel_scale,
        )?;
        let mut pixmap =
            resvg::tiny_skia::Pixmap::new(width, height).ok_or(Error::ResourceLimit)?;
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(
                self.options.pixel_scale,
                self.options.pixel_scale,
            ),
            &mut pixmap.as_mut(),
        );
        let encoded = pixmap.encode_png().map_err(|_| Error::ImageDecode)?;
        Ok(PreparedImage {
            size: AtomSize {
                width: size.width(),
                height: size.height(),
            },
            baseline: None,
            encoded: Arc::new(encoded),
        })
    }

    pub(super) fn fallback(&self, node: &Node) -> Result<PreparedImage> {
        let (label, source) = match node.node_type.as_str() {
            "mermaidBlock" => (
                &self.options.fallback_labels.diagram,
                node.string_attr("code").unwrap_or_default(),
            ),
            "inlineMath" | "mathBlock" => (
                &self.options.fallback_labels.formula,
                node.string_attr("latex").unwrap_or_default(),
            ),
            "imageBlock" => (
                &self.options.fallback_labels.image,
                node.string_attr("alt").unwrap_or_default(),
            ),
            _ => return Err(Error::UnsupportedAtom),
        };
        // A bounded visible preview is explicitly marked with an ellipsis; the complete source
        // remains in the unchanged canonical node and the host's source editor.
        let mut lines = Vec::new();
        let mut line = String::new();
        let mut count = 0;
        let mut overflow = false;
        for character in source.chars() {
            if lines.len() >= 12 {
                overflow = true;
                break;
            }
            if character == '\n' || count >= 76 {
                lines.push(std::mem::take(&mut line));
                count = 0;
                if character == '\n' {
                    continue;
                }
            }
            line.push(character);
            count += 1;
        }
        if !line.is_empty() && lines.len() < 12 {
            lines.push(line);
        }
        if overflow {
            lines.push("…".into());
        }
        let height = 60 + lines.len().max(1) * 21;
        let foreground = rgba_css(self.options.foreground);
        let background = rgba_css(self.options.background);
        let mut svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="640" height="{height}"><rect x="0.5" y="0.5" width="639" height="{}" rx="5" fill="{background}" stroke="#a77c3f"/><text x="16" y="28" font-family="Source Sans 3" font-size="15" font-weight="600" fill="{foreground}">{}</text>"##,
            height - 1,
            escape_xml(label)
        );
        for (index, line) in lines.iter().enumerate() {
            svg.push_str(&format!(r#"<text xml:space="preserve" x="16" y="{}" font-family="JetBrains Mono" font-size="13" fill="{foreground}">{}</text>"#, 54 + index * 21, escape_xml(line)));
        }
        svg.push_str("</svg>");
        self.svg_image(svg.as_bytes())
    }
}

fn positive_attr(node: &Node, key: &str) -> Result<Option<f32>> {
    let Some(value) = node.attrs.as_ref().and_then(|attrs| attrs.get(key)) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let number = value
        .as_f64()
        .filter(|value| value.is_finite() && *value > 0.0 && *value <= 20_000.0)
        .ok_or(Error::InvalidOptions)?;
    Ok(Some(number as f32))
}

fn raster_dimensions(bytes: &[u8], limits: &RenderLimits) -> Result<(u32, u32)> {
    let (width, height) = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| Error::ImageDecode)?
        .into_dimensions()
        .map_err(|_| Error::ImageDecode)?;
    if width == 0
        || height == 0
        || width > limits.dimension
        || height > limits.dimension
        || u64::from(width) * u64::from(height) > limits.pixels
    {
        return Err(Error::ResourceLimit);
    }
    Ok((width, height))
}

fn rgba_css([r, g, b, a]: [u8; 4]) -> String {
    format!("rgba({r},{g},{b},{})", f32::from(a) / 255.0)
}
fn escape_xml(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&apos;"),
            _ => output.push(character),
        }
    }
    output
}
