//! Explicit container-text projection without conflating parent offsets and leaf carets.
use crate::{Document, Error, Node, Result, ScalarOffset};

/// An exact scalar boundary in one identified native text leaf.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextPoint {
    /// Stable paragraph, heading or code-block identity.
    pub block_id: String,
    /// Boundary in the common native inline projection.
    pub offset: ScalarOffset,
}

#[derive(Clone, Debug)]
struct Leaf {
    id: String,
    start: usize,
    end: usize,
}

/// Read-only scalar projection of an identified text block or nested text container.
///
/// Adjacent text leaves are separated by one explicit newline. Hard breaks are
/// newlines and inline math occupies U+FFFC, exactly as native leaf carets do.
/// This is deliberately distinct from `Node::inline_text`, which concatenates
/// child blocks, and from legacy exports that omit or spell out inline atoms.
/// No canonical JSON or scalar address in an existing text leaf is rewritten.
#[derive(Clone, Debug)]
pub struct BlockTextProjection {
    text: String,
    leaves: Vec<Leaf>,
    scalar_len: usize,
}
impl BlockTextProjection {
    /// Resolve one unique identified subtree and build its explicit text projection.
    /// Non-text block atoms are rejected rather than omitted from a passage silently.
    ///
    /// # Errors
    /// Returns an error for unknown IDs, unsupported block atoms or missing leaf IDs.
    pub fn new(document: &Document, block_id: &str) -> Result<Self> {
        let mut result = Self {
            text: String::new(),
            leaves: vec![],
            scalar_len: 0,
        };
        result.visit(document.block(block_id)?)?;
        Ok(result)
    }
    fn visit(&mut self, node: &Node) -> Result<()> {
        if matches!(
            node.node_type.as_str(),
            "paragraph" | "heading" | "codeBlock"
        ) {
            let id = node.block_id().ok_or(Error::BlockNotFound)?.to_owned();
            if !self.leaves.is_empty() {
                self.text.push('\n');
                self.scalar_len += 1;
            }
            let start = self.scalar_len;
            let text = node.inline_text();
            self.scalar_len += text.chars().count();
            self.text.push_str(&text);
            self.leaves.push(Leaf {
                id,
                start,
                end: self.scalar_len,
            });
            return Ok(());
        }
        if matches!(
            node.node_type.as_str(),
            "imageBlock" | "mathBlock" | "mermaidBlock" | "horizontalRule" | "sceneBreak"
        ) {
            return Err(Error::UnsupportedEdit);
        }
        for child in node.children() {
            self.visit(child)?;
        }
        Ok(())
    }
    /// Exact projected text, including explicit leaf-separator newlines.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Convert a projected boundary into its exact native leaf caret.
    /// `right_bias` disambiguates equal boundaries of empty leaves; it never skips
    /// a real newline or changes a scalar inside a text leaf.
    ///
    /// # Errors
    /// Returns an error for out-of-bounds offsets or a subtree with no text leaf.
    pub fn point(&self, offset: ScalarOffset, right_bias: bool) -> Result<TextPoint> {
        if offset.0 > self.scalar_len {
            return Err(Error::InvalidPosition);
        }
        let matching = |leaf: &&Leaf| leaf.start <= offset.0 && offset.0 <= leaf.end;
        let leaf = if right_bias {
            self.leaves.iter().rev().find(matching)
        } else {
            self.leaves.iter().find(matching)
        }
        .ok_or(Error::InvalidPosition)?;
        Ok(TextPoint {
            block_id: leaf.id.clone(),
            offset: ScalarOffset(offset.0 - leaf.start),
        })
    }

    /// Convert a native leaf boundary into the container projection.
    ///
    /// # Errors
    /// Returns an error if the leaf is outside this subtree or the caret is invalid.
    pub fn offset(&self, point: &TextPoint) -> Result<ScalarOffset> {
        let leaf = self
            .leaves
            .iter()
            .find(|leaf| leaf.id == point.block_id)
            .ok_or(Error::InvalidPosition)?;
        if point.offset.0 > leaf.end - leaf.start {
            return Err(Error::InvalidPosition);
        }
        Ok(ScalarOffset(leaf.start + point.offset.0))
    }

    /// Whether this projection includes the identified native text leaf.
    pub fn contains_leaf(&self, block_id: &str) -> bool {
        self.leaves.iter().any(|leaf| leaf.id == block_id)
    }
}
