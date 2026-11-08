//! Explicit container projections without conflating parent offsets and native leaf carets.
use crate::{Document, Error, Node, Result, ScalarOffset};

/// An exact scalar boundary in one identified native text leaf.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextPoint {
    /// Stable paragraph, heading or code-block identity.
    pub block_id: String,
    /// Boundary in the common native inline projection.
    pub offset: ScalarOffset,
}
/// Explicit projected spelling of an inline math atom.
#[derive(Clone, Copy, Debug)]
pub enum InlineAtomText<'a> {
    /// One U+FFFC scalar, matching native caret coordinates.
    ReplacementCharacter,
    /// No text; atom boundaries are mapped with explicit affinity.
    Omit,
    /// Authored LaTeX with caller-supplied compatibility delimiters.
    WithAffixes {
        /// Text preceding the original LaTeX.
        prefix: &'a str,
        /// Text following the original LaTeX.
        suffix: &'a str,
    },
}
/// Explicit, read-only text projection policy; never a canonical schema migration.
#[derive(Clone, Copy, Debug)]
pub struct TextProjectionOptions<'a> {
    /// Text between adjacent text blocks and included opaque block atoms.
    pub block_separator: &'a str,
    /// Projected text of a native hard-break scalar.
    pub hard_break: &'a str,
    /// Projected text of inline math.
    pub inline_math: InlineAtomText<'a>,
    /// Projected text of an opaque block atom. Empty means intentionally omitted.
    pub block_atom: &'a str,
}
impl Default for TextProjectionOptions<'_> {
    fn default() -> Self {
        Self {
            block_separator: "\n",
            hard_break: "\n",
            inline_math: InlineAtomText::ReplacementCharacter,
            block_atom: "\u{fffc}",
        }
    }
}
#[derive(Clone, Debug)]
struct Segment {
    start: usize,
    end: usize,
    native_start: usize,
    native_end: usize,
}
#[derive(Clone, Debug)]
struct Leaf {
    id: String,
    start: usize,
    end: usize,
    native_len: usize,
    segments: Vec<Segment>,
}
/// Read-only projection with exact parent-offset ↔ native-leaf boundary conversion.
///
/// The default adds one newline between native leaves, retains hard breaks and
/// represents inline/opaque block atoms with U+FFFC. Compatibility policies are
/// explicit; they may omit atoms or spell them out without changing native carets.
/// Interior positions in an expanded atom spelling have no exact native caret
/// and are rejected. Canonical JSON and `Node::inline_text` remain unchanged.
#[derive(Clone, Debug)]
pub struct BlockTextProjection {
    text: String,
    leaves: Vec<Leaf>,
    scalar_len: usize,
    blocks: usize,
}
impl BlockTextProjection {
    /// Build the native projection of an identified text block or container.
    ///
    /// # Errors
    /// Returns an error for unknown IDs or missing text-leaf identities.
    pub fn new(document: &Document, block_id: &str) -> Result<Self> {
        Self::with_options(document, block_id, TextProjectionOptions::default())
    }
    /// Build a projection under an explicit compatibility text policy.
    ///
    /// # Errors
    /// Returns an error for unknown IDs or missing text-leaf identities.
    pub fn with_options(
        document: &Document,
        block_id: &str,
        options: TextProjectionOptions<'_>,
    ) -> Result<Self> {
        let mut result = Self {
            text: String::new(),
            leaves: vec![],
            scalar_len: 0,
            blocks: 0,
        };
        result.visit(document.block(block_id)?, options)?;
        Ok(result)
    }
    fn append(&mut self, text: &str) {
        self.text.push_str(text);
        self.scalar_len += text.chars().count();
    }
    fn begin_block(&mut self, options: TextProjectionOptions<'_>) {
        if self.blocks > 0 {
            self.append(options.block_separator);
        }
        self.blocks += 1;
    }
    fn visit(&mut self, node: &Node, options: TextProjectionOptions<'_>) -> Result<()> {
        if matches!(
            node.node_type.as_str(),
            "paragraph" | "heading" | "codeBlock"
        ) {
            self.begin_block(options);
            let mut leaf = Leaf {
                id: node.block_id().ok_or(Error::BlockNotFound)?.to_owned(),
                start: self.scalar_len,
                end: self.scalar_len,
                native_len: 0,
                segments: vec![],
            };
            self.inline(node, options, &mut leaf);
            leaf.end = self.scalar_len;
            self.leaves.push(leaf);
            return Ok(());
        }
        if matches!(
            node.node_type.as_str(),
            "imageBlock" | "mathBlock" | "mermaidBlock" | "horizontalRule" | "sceneBreak"
        ) {
            if !options.block_atom.is_empty() {
                self.begin_block(options);
                self.append(options.block_atom);
            }
            return Ok(());
        }
        for child in node.children() {
            self.visit(child, options)?;
        }
        Ok(())
    }
    fn inline(&mut self, node: &Node, options: TextProjectionOptions<'_>, leaf: &mut Leaf) {
        let (text, native_len) = match node.node_type.as_str() {
            "text" => (
                node.text.clone().unwrap_or_default(),
                node.text.as_deref().unwrap_or_default().chars().count(),
            ),
            "hardBreak" => (options.hard_break.to_owned(), 1),
            "inlineMath" => (
                match options.inline_math {
                    InlineAtomText::ReplacementCharacter => "\u{fffc}".to_owned(),
                    InlineAtomText::Omit => String::new(),
                    InlineAtomText::WithAffixes { prefix, suffix } => format!(
                        "{prefix}{}{suffix}",
                        node.string_attr("latex").unwrap_or_default()
                    ),
                },
                1,
            ),
            _ => {
                for child in node.children() {
                    self.inline(child, options, leaf);
                }
                return;
            }
        };
        let start = self.scalar_len;
        self.append(&text);
        leaf.segments.push(Segment {
            start,
            end: self.scalar_len,
            native_start: leaf.native_len,
            native_end: leaf.native_len + native_len,
        });
        leaf.native_len += native_len;
    }
    /// Exact projected text including explicitly configured separators/atom spellings.
    pub fn text(&self) -> &str {
        &self.text
    }
    /// Convert a projected boundary to a native text-leaf caret.
    /// Affinity chooses before/after omitted atoms or concatenated leaf boundaries.
    ///
    /// # Errors
    /// Rejects out-of-bounds offsets, opaque-block boundaries without a leaf,
    /// and the interior of expanded inline-atom labels.
    pub fn point(&self, offset: ScalarOffset, right_bias: bool) -> Result<TextPoint> {
        if offset.0 > self.scalar_len {
            return Err(Error::InvalidPosition);
        }
        let mut result = None;
        for leaf in &self.leaves {
            if offset.0 < leaf.start || offset.0 > leaf.end {
                continue;
            }
            if leaf.segments.is_empty() {
                let point = TextPoint {
                    block_id: leaf.id.clone(),
                    offset: ScalarOffset(0),
                };
                if !right_bias {
                    return Ok(point);
                }
                result = Some(point);
                continue;
            }
            for segment in &leaf.segments {
                if offset.0 < segment.start || offset.0 > segment.end {
                    continue;
                }
                let projected = segment.end - segment.start;
                let native = segment.native_end - segment.native_start;
                let mapped = if projected == native {
                    Some(segment.native_start + offset.0 - segment.start)
                } else if projected == 0 {
                    Some(if right_bias {
                        segment.native_end
                    } else {
                        segment.native_start
                    })
                } else if offset.0 == segment.start {
                    Some(segment.native_start)
                } else if offset.0 == segment.end {
                    Some(segment.native_end)
                } else {
                    None
                };
                if let Some(mapped) = mapped {
                    let point = TextPoint {
                        block_id: leaf.id.clone(),
                        offset: ScalarOffset(mapped),
                    };
                    if !right_bias {
                        return Ok(point);
                    }
                    result = Some(point);
                }
            }
        }
        result.ok_or(Error::InvalidPosition)
    }
    /// Convert a native leaf boundary into this explicit container projection.
    ///
    /// # Errors
    /// Rejects foreign leaf identities or invalid native offsets.
    pub fn offset(&self, point: &TextPoint) -> Result<ScalarOffset> {
        let leaf = self
            .leaves
            .iter()
            .find(|leaf| leaf.id == point.block_id)
            .ok_or(Error::InvalidPosition)?;
        if point.offset.0 > leaf.native_len {
            return Err(Error::InvalidPosition);
        }
        if leaf.segments.is_empty() {
            return Ok(ScalarOffset(leaf.start));
        }
        for segment in &leaf.segments {
            if point.offset.0 < segment.native_start || point.offset.0 > segment.native_end {
                continue;
            }
            let projected = segment.end - segment.start;
            let native = segment.native_end - segment.native_start;
            if projected == native {
                return Ok(ScalarOffset(
                    segment.start + point.offset.0 - segment.native_start,
                ));
            }
            if point.offset.0 == segment.native_start {
                return Ok(ScalarOffset(segment.start));
            }
            if point.offset.0 == segment.native_end {
                return Ok(ScalarOffset(segment.end));
            }
        }
        Err(Error::InvalidPosition)
    }
    /// Whether this projection contains the identified native text leaf.
    pub fn contains_leaf(&self, block_id: &str) -> bool {
        self.leaves.iter().any(|leaf| leaf.id == block_id)
    }
}
