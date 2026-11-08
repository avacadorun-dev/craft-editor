//! One captured paste context, one off-thread parse, and correlated prepared application.
use super::{AssetReference, ClipboardBundle, ClipboardError, RichSlice, html, plain, rules};
use crate::{EditorSession, Result, projection::Projection};
use craft_editor_model::Node;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};

/// Immutable local preparation context bound to one captured Paste request.
///
/// Fields are private and deliberately not serializable. The inherited text envelope omits the
/// original writing; no full manuscript or reusable authorization token is captured.
#[derive(Clone)]
pub struct PasteContext {
    identity: Arc<()>,
    source: Arc<ClipboardBundle>,
    template: Node,
    literal_code: bool,
}
/// Exact image-only binding change supplied by the host after scoped acquisition/import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageBinding {
    /// Original canonical source and optional identity, matched exactly.
    pub original: AssetReference,
    /// Host-authorized destination source and identity.
    pub destination: AssetReference,
}
/// Validated prepared input bound to its original captured context, not to equal plain text.
/// The host can patch only image bindings; source policy and original URL intent stay sealed.
pub struct PreparedPaste {
    identity: Arc<()>,
    source: Arc<ClipboardBundle>,
    source_assets: Vec<AssetReference>,
    content: Content,
    original_url: Option<String>,
    max_bytes: usize,
}
enum Content {
    Empty,
    Literal(String),
    Rich(RichSlice),
}
#[derive(Clone, Copy)]
enum Origin {
    Private,
    InternalHtml,
    ExternalHtml,
    Special,
    Plain,
}
impl PasteContext {
    pub(super) fn capture(session: &EditorSession, source: Arc<ClipboardBundle>) -> Result<Self> {
        let selection = session
            .selection
            .clone()
            .or(session.inline_node_text_selection()?);
        let (mut template, literal_code) = if let Some(selection) = selection {
            let projection = Projection::new(session.document())?;
            let first =
                if projection.scalar(&selection.anchor)? <= projection.scalar(&selection.head)? {
                    selection.anchor
                } else {
                    selection.head
                };
            let mut template = session.typing_leaf_at(&first)?;
            let marks = match &session.stored_marks {
                Some(marks) => marks.clone(),
                None => session.marks_at(&first)?,
            };
            if template.marks.as_deref().unwrap_or_default() != marks.as_slice() {
                template.marks = Some(marks);
            }
            (
                template,
                session.document().block(&first.block_id)?.node_type == "codeBlock",
            )
        } else {
            (Node::text(""), false)
        };
        template.text = None;
        Ok(Self {
            identity: Arc::new(()),
            source,
            template,
            literal_code,
        })
    }
    /// Borrow the exact captured representations; this method never rereads an OS clipboard.
    pub fn source(&self) -> &ClipboardBundle {
        &self.source
    }
    /// Parse/transform once on the host worker. Original whole-URL intent is recorded before
    /// marker conversion or image binding changes. No resource is fetched or opened.
    pub fn prepare(&self, max_bytes: usize) -> Result<PreparedPaste> {
        self.source.check_budget(max_bytes)?;
        let (mut slice, origin) = if let Some(bytes) = &self.source.rich {
            (Some(RichSlice::decode(bytes, max_bytes)?), Origin::Private)
        } else if let Some(slice) = plain::special_slice(&self.source.plain)? {
            (Some(slice), Origin::Special)
        } else if let Some(html) = self
            .source
            .html
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            (
                Some(html::parse(html)?),
                if html.contains("data-pm-slice") {
                    Origin::InternalHtml
                } else {
                    Origin::ExternalHtml
                },
            )
        } else {
            (None, Origin::Plain)
        };
        let source_assets = slice
            .as_ref()
            .map_or_else(Vec::new, RichSlice::asset_references);
        let opaque_content = slice
            .as_ref()
            .is_some_and(|slice| contains_atom(&slice.content));
        let original_text = if matches!(origin, Origin::Special) || opaque_content {
            String::new()
        } else if self.literal_code && !self.source.plain.is_empty() {
            self.source.plain.replace("\r\n", "\n").replace('\r', "\n")
        } else if let Some(slice) = &slice {
            text_content(&slice.content)
        } else {
            self.source.plain.replace(['\r', '\n'], "")
        };
        let links = rules::link_spans(&original_text)?;
        let original_url = links
            .first()
            .filter(|link| {
                links.len() == 1 && link.bytes.start == 0 && link.bytes.end == original_text.len()
            })
            .map(|link| link.href.clone());
        let text = self.source.plain.replace("\r\n", "\n").replace('\r', "\n");
        let content = if self.literal_code
            && !text.is_empty()
            && !matches!(origin, Origin::Special)
            && !(matches!(origin, Origin::Private | Origin::InternalHtml) && opaque_content)
        {
            let literal = if matches!(origin, Origin::Private | Origin::InternalHtml) {
                slice
                    .as_ref()
                    .map_or_else(|| text.clone(), |slice| literal_text(&slice.content))
            } else {
                text
            };
            Content::Literal(literal)
        } else if let Some(mut incoming) = slice.take() {
            if matches!(origin, Origin::ExternalHtml) {
                rules::transform_external(&mut incoming, max_bytes)?;
            }
            Content::Rich(incoming)
        } else if text.is_empty() {
            Content::Empty
        } else {
            Content::Rich(rules::plain_slice(&text, &self.template, max_bytes)?)
        };
        Ok(PreparedPaste {
            identity: self.identity.clone(),
            source: self.source.clone(),
            source_assets,
            content,
            original_url,
            max_bytes,
        })
    }
}
fn contains_atom(nodes: &[Node]) -> bool {
    nodes.iter().any(|node| {
        matches!(
            node.node_type.as_str(),
            "imageBlock"
                | "mermaidBlock"
                | "mathBlock"
                | "inlineMath"
                | "horizontalRule"
                | "sceneBreak"
        ) || contains_atom(node.children())
    })
}
fn text_content(nodes: &[Node]) -> String {
    let mut output = String::new();
    for node in nodes {
        if let Some(text) = &node.text {
            output.push_str(text);
        }
        output.push_str(&text_content(node.children()));
    }
    output
}
fn literal_text(nodes: &[Node]) -> String {
    nodes
        .iter()
        .map(|node| {
            if matches!(
                node.node_type.as_str(),
                "paragraph" | "heading" | "codeBlock"
            ) {
                node.inline_text()
            } else if node.node_type == "text" {
                node.text.clone().unwrap_or_default()
            } else {
                literal_text(node.children())
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
impl PreparedPaste {
    pub(super) fn matches(&self, context: &PasteContext) -> bool {
        Arc::ptr_eq(&self.identity, &context.identity)
    }
    /// Exact original representations, retained for capsule validation and explicit recovery.
    pub fn source(&self) -> &ClipboardBundle {
        &self.source
    }
    /// References in the original captured rich representation, before literal/code conversion.
    pub fn source_asset_references(&self) -> &[AssetReference] {
        &self.source_assets
    }
    /// References that the prepared insertion still needs. Literal code input has none.
    pub fn asset_references(&self) -> Vec<AssetReference> {
        match &self.content {
            Content::Rich(slice) => slice.asset_references(),
            _ => vec![],
        }
    }
    /// Borrow prepared canonical data without exposing mutable node/mark/source-policy fields.
    pub fn rich_slice(&self) -> Option<&RichSlice> {
        match &self.content {
            Content::Rich(slice) => Some(slice),
            _ => None,
        }
    }
    /// Patch only exact image src/assetId pairs after the host has authorized/imported them.
    /// Duplicate/conflicting or unused mappings fail atomically, preserving all other fields.
    pub fn bind_images(&mut self, bindings: &[ImageBinding]) -> Result<()> {
        let Content::Rich(slice) = &self.content else {
            return if bindings.is_empty() {
                Ok(())
            } else {
                Err(ClipboardError::InvalidSlice.into())
            };
        };
        if bindings.is_empty() {
            return Ok(());
        }
        let mut by_source = HashMap::with_capacity(bindings.len());
        for (index, binding) in bindings.iter().enumerate() {
            if binding.destination.source.len() > self.max_bytes
                || binding
                    .destination
                    .asset_id
                    .as_ref()
                    .is_some_and(|id| id.len() > self.max_bytes)
                || by_source.insert(binding.original.clone(), index).is_some()
            {
                return Err(ClipboardError::InvalidSlice.into());
            }
        }
        // Validate destination expansion before cloning canonical envelopes or destination strings.
        let mut projected = rules::serialized_bytes(&slice.content)?
            .checked_add(64)
            .ok_or(ClipboardError::ResourceLimit)?;
        for reference in slice.asset_references() {
            if let Some(index) = by_source.get(&reference) {
                let destination = &bindings[*index].destination;
                let cost = rules::serialized_bytes(&destination.source)?
                    .checked_add(rules::serialized_bytes(&destination.asset_id)?)
                    .and_then(|sum| sum.checked_add(32))
                    .ok_or(ClipboardError::ResourceLimit)?;
                projected = projected
                    .checked_add(cost)
                    .filter(|sum| *sum <= self.max_bytes)
                    .ok_or(ClipboardError::ResourceLimit)?;
            }
        }
        let mut candidate = slice.clone();
        let mut used = vec![false; bindings.len()];
        fn visit(
            nodes: &mut [Node],
            bindings: &[ImageBinding],
            by_source: &HashMap<AssetReference, usize>,
            used: &mut [bool],
        ) -> Result<()> {
            for node in nodes {
                if node.node_type == "imageBlock" {
                    let original = AssetReference {
                        asset_id: node.string_attr("assetId").map(str::to_owned),
                        source: node.string_attr("src").unwrap_or_default().into(),
                    };
                    if let Some(index) = by_source.get(&original).copied() {
                        let binding = &bindings[index];
                        let attrs = node.attrs.as_mut().ok_or(ClipboardError::InvalidSlice)?;
                        attrs.insert("src".into(), json!(binding.destination.source));
                        attrs.insert(
                            "assetId".into(),
                            binding
                                .destination
                                .asset_id
                                .as_ref()
                                .map_or(Value::Null, |value| json!(value)),
                        );
                        used[index] = true;
                    }
                }
                if let Some(children) = &mut node.content {
                    visit(children, bindings, by_source, used)?;
                }
            }
            Ok(())
        }
        visit(&mut candidate.content, bindings, &by_source, &mut used)?;
        if used.iter().any(|used| !used) {
            return Err(ClipboardError::InvalidSlice.into());
        }
        let encoded = candidate.encode()?;
        if encoded.len() > self.max_bytes {
            return Err(ClipboardError::ResourceLimit.into());
        }
        self.content = Content::Rich(candidate);
        Ok(())
    }
}
impl EditorSession {
    pub(super) fn apply_prepared_paste(
        &mut self,
        prepared: PreparedPaste,
    ) -> Result<craft_editor_model::Change> {
        if let Some(href) = prepared.original_url
            && self.can_link_paste_selection()?
        {
            let mut mark = craft_editor_model::Mark::new("link");
            mark.attrs = Some(serde_json::Map::from_iter([("href".into(), json!(href))]));
            return self.format_selection(mark, craft_editor_model::MarkMode::Patch);
        }
        match prepared.content {
            Content::Empty => self.apply_commands(vec![]),
            Content::Literal(text) => self.insert_plain_text(&text),
            Content::Rich(slice) => self.paste_rich_slice(slice),
        }
    }
    fn can_link_paste_selection(&self) -> Result<bool> {
        if let Some(node) = &self.node_selection {
            return Ok(node.expected.node_type == "inlineMath");
        }
        let Some(selection) = &self.selection else {
            return Ok(false);
        };
        if selection.anchor == selection.head {
            return Ok(false);
        }
        let projection = Projection::new(self.document())?;
        let a = projection.scalar(&selection.anchor)?;
        let b = projection.scalar(&selection.head)?;
        Ok(projection.spans.iter().any(|span| {
            !span.atom
                && span.node.node_type != "codeBlock"
                && span.start < a.max(b)
                && span.end > a.min(b)
        }))
    }
}
