use crate::{DocPoint, Error, Result, ViewOptions};
use craft_editor_model::{Document, Node, ScalarOffset};
use egui::{Color32, FontFamily, FontId, Stroke, TextFormat, text::LayoutJob};

#[derive(Debug, Clone)]
pub(crate) struct Span {
    pub id: String,
    pub start: usize,
    pub end: usize,
    pub path: Vec<usize>,
    pub node: Node,
    pub table: bool,
    pub atom: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Projection {
    pub text: String,
    pub spans: Vec<Span>,
    scalar_len: usize,
}

impl Projection {
    pub fn new(document: &Document) -> Result<Self> {
        let mut output = Self::default();
        output.visit(document.root(), &mut vec![], false)?;
        Ok(output)
    }
    fn visit(&mut self, node: &Node, path: &mut Vec<usize>, table: bool) -> Result<()> {
        let table = table || matches!(node.node_type.as_str(), "tableCell" | "tableHeader");
        if matches!(
            node.node_type.as_str(),
            "paragraph" | "heading" | "codeBlock"
        ) {
            let id = node
                .block_id()
                .ok_or(Error::MissingBlockIdentity)?
                .to_owned();
            if !self.spans.is_empty() {
                self.text.push('\n');
                self.scalar_len += 1;
            }
            let start = self.scalar_len;
            let text = node.inline_text();
            self.scalar_len += text.chars().count();
            self.text.push_str(&text);
            self.spans.push(Span {
                id,
                start,
                end: self.scalar_len,
                path: path.clone(),
                node: node.clone(),
                table,
                atom: false,
            });
        } else if matches!(
            node.node_type.as_str(),
            "imageBlock" | "mathBlock" | "mermaidBlock" | "horizontalRule" | "sceneBreak"
        ) {
            if !self.spans.is_empty() {
                self.text.push('\n');
                self.scalar_len += 1;
            }
            let start = self.scalar_len;
            self.text.push('\u{fffc}');
            self.scalar_len += 1;
            self.spans.push(Span {
                id: node
                    .block_id()
                    .map_or_else(|| format!("craft-view-atom:{path:?}"), str::to_owned),
                start,
                end: self.scalar_len,
                path: path.clone(),
                node: node.clone(),
                table,
                atom: true,
            });
        } else {
            for (index, child) in node.children().iter().enumerate() {
                path.push(index);
                self.visit(child, path, table)?;
                path.pop();
            }
        }
        Ok(())
    }
    pub fn point(&self, scalar: usize, right_bias: bool) -> Result<DocPoint> {
        let mut previous: Option<&Span> = None;
        for span in self.spans.iter().filter(|span| !span.atom) {
            if (span.start..=span.end).contains(&scalar) {
                return Ok(DocPoint {
                    block_id: span.id.clone(),
                    offset: ScalarOffset(scalar - span.start),
                });
            }
            if scalar < span.start {
                let (block, offset) = if right_bias || previous.is_none() {
                    (span, 0)
                } else {
                    let block = previous.ok_or(Error::InvalidSelection)?;
                    (block, block.end - block.start)
                };
                return Ok(DocPoint {
                    block_id: block.id.clone(),
                    offset: ScalarOffset(offset),
                });
            }
            previous = Some(span);
        }
        if scalar <= self.scalar_len
            && let Some(block) = previous
        {
            return Ok(DocPoint {
                block_id: block.id.clone(),
                offset: ScalarOffset(block.end - block.start),
            });
        }
        Err(Error::InvalidSelection)
    }
    pub fn linear_checkpoint(document: &Document) -> Result<Self> {
        let projection = Self::new(document)?;
        if let Some(span) = projection.spans.iter().find(|span| span.atom) {
            return Err(Error::UnsupportedBlockKind(span.node.node_type.clone()));
        }
        Ok(projection)
    }
    pub fn scalar(&self, point: &DocPoint) -> Result<usize> {
        let span = self
            .spans
            .iter()
            .find(|span| !span.atom && span.id == point.block_id)
            .ok_or(Error::InvalidSelection)?;
        if point.offset.0 > span.end - span.start {
            return Err(Error::InvalidSelection);
        }
        Ok(span.start + point.offset.0)
    }
    pub fn job(&self, options: &ViewOptions, color: Color32, width: f32) -> LayoutJob {
        let mut job = LayoutJob::default();
        job.wrap.max_width = width;
        for (index, span) in self.spans.iter().enumerate() {
            if index > 0 {
                job.append(
                    "\n",
                    0.0,
                    TextFormat {
                        font_id: FontId::new(options.font_size, options.font_family.clone()),
                        color,
                        ..Default::default()
                    },
                );
            }
            let mut format = TextFormat {
                font_id: FontId::new(options.font_size, options.font_family.clone()),
                color,
                ..Default::default()
            };
            if span.node.node_type == "heading" {
                let level = span
                    .node
                    .attrs
                    .as_ref()
                    .and_then(|a| a.get("level"))
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(1);
                format.font_id.size *= (1.65 - level as f32 * 0.1).max(1.05);
            }
            if span.node.node_type == "codeBlock" {
                format.font_id.family = FontFamily::Monospace;
            }
            if span.table {
                format.background = options.table_background;
            }
            append_inline(&mut job, &span.node, &format, options);
        }
        job
    }
}

fn append_inline(job: &mut LayoutJob, node: &Node, inherited: &TextFormat, options: &ViewOptions) {
    match node.node_type.as_str() {
        "text" => {
            let mut format = inherited.clone();
            for mark in node.marks.as_deref().unwrap_or_default() {
                match mark.mark_type.as_str() {
                    "bold" => {
                        if let Some(family) = &options.bold_family {
                            format.font_id.family = family.clone();
                        }
                    }
                    "italic" => format.italics = true,
                    "underline" | "link" => format.underline = Stroke::new(1.0, format.color),
                    "strike" => format.strikethrough = Stroke::new(1.0, format.color),
                    "code" => format.font_id.family = FontFamily::Monospace,
                    _ => {}
                }
            }
            job.append(node.text.as_deref().unwrap_or_default(), 0.0, format);
        }
        "hardBreak" => job.append("\n", 0.0, inherited.clone()),
        "inlineMath" => job.append("\u{fffc}", 0.0, inherited.clone()),
        _ => {
            for child in node.children() {
                append_inline(job, child, inherited, options);
            }
        }
    }
}
