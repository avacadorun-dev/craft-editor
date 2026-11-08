//! Lossless canonical clipboard slices, meaningful fallbacks and explicit native transport.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use craft_editor_model::{Change, Node, NodePath};
use serde_json::{Value, json};

use crate::{EditorSession, Error, Result, insertion, projection::Projection};

mod deferred;
pub(crate) use deferred::DeferredState;
pub use deferred::{
    ClipboardIntentGate, ClipboardMode, ClipboardRequest, CutGuard, PasteGuard, WriteAck,
    WriteTicket,
};

mod html;
#[cfg(feature = "native-clipboard")]
mod native;
mod plain;
#[cfg(feature = "native-clipboard")]
pub use native::NativeClipboard;

/// Portable private clipboard type. Its bytes are untrusted canonical JSON, never executable.
pub const RICH_MIME: &str = "application/vnd.craft-editor.slice+json";

/// Content-free transport/format diagnostics; hosts supply localized messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ClipboardError {
    /// No acknowledged native clipboard transport is installed.
    #[error("clipboard_unavailable")]
    Unavailable,
    /// The native clipboard could not be read.
    #[error("clipboard_read_failed")]
    Read,
    /// The native clipboard did not acknowledge a complete write; Cut must not delete text.
    #[error("clipboard_write_failed")]
    Write,
    /// Input exceeded the host's explicit clipboard budget.
    #[error("clipboard_resource_limit")]
    ResourceLimit,
    /// The request belongs to another session/revision/selection or was explicitly cancelled.
    #[error("clipboard_stale_request")]
    StaleRequest,
    /// A write acknowledgement does not match the captured content or Cut request.
    #[error("clipboard_invalid_acknowledgement")]
    InvalidAcknowledgement,
    /// A local Copy/Cut has not acknowledged its write; retry Paste after publication completes.
    #[error("clipboard_write_pending")]
    WritePending,
    /// The bounded host-request queue needs to be drained before another request is captured.
    #[error("clipboard_busy")]
    Busy,
    /// A private rich slice has an invalid envelope or open edge.
    #[error("clipboard_invalid_slice")]
    InvalidSlice,
}
/// Clipboard transport result, distinct from model input validation.
pub type ClipboardResult<T> = std::result::Result<T, ClipboardError>;

/// Concurrently advertised clipboard representations of one exact selection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClipboardBundle {
    /// Lossless versioned canonical slice under [`RICH_MIME`].
    pub rich: Option<Vec<u8>>,
    /// Sanitized interoperable HTML; it does not carry opaque internal metadata.
    pub html: Option<String>,
    /// Meaningful text, including source-form atoms rather than synthesized U+FFFC sentinels.
    pub plain: String,
    /// Host-owned opaque named formats, such as a portable project-media capsule.
    /// Native reads are limited to explicitly registered formats; these bytes grant no access.
    pub formats: BTreeMap<String, Vec<u8>>,
}
impl ClipboardBundle {
    /// Inspect canonical paste content before mutation, for host asset/project authorization.
    ///
    /// Precedence matches paste: private canonical bytes, complete source fence/formula rules,
    /// then sanitized HTML. Ordinary plain text returns None. No resource is opened or fetched.
    pub fn canonical_slice(&self, max_bytes: usize) -> Result<Option<RichSlice>> {
        self.check_budget(max_bytes)?;
        if let Some(bytes) = &self.rich {
            return RichSlice::decode(bytes, max_bytes).map(Some);
        }
        if let Some(slice) = plain::special_slice(&self.plain)? {
            return Ok(Some(slice));
        }
        if let Some(html) = self.html.as_deref().filter(|html| !html.is_empty()) {
            return html::parse(html).map(Some);
        }
        Ok(None)
    }
    /// Refuse oversized external input before decoding; the host owns the selected budget.
    pub fn check_budget(&self, max_bytes: usize) -> ClipboardResult<()> {
        let mut count = self
            .rich
            .as_ref()
            .map_or(0, Vec::len)
            .checked_add(self.html.as_ref().map_or(0, String::len))
            .and_then(|sum| sum.checked_add(self.plain.len()))
            .ok_or(ClipboardError::ResourceLimit)?;
        for (format, bytes) in &self.formats {
            count = count
                .checked_add(format.len())
                .and_then(|sum| sum.checked_add(bytes.len()))
                .ok_or(ClipboardError::ResourceLimit)?;
        }
        if count > max_bytes {
            return Err(ClipboardError::ResourceLimit);
        }
        Ok(())
    }
}
/// Host-injected native clipboard ownership. A write acknowledges all advertised formats.
///
/// Keep this service alive while the application owns an X11/Wayland selection. Image bytes,
/// filesystem paths, remote resources and cross-project asset authorization belong to the host.
pub trait ClipboardProvider: Send + Sync {
    /// Read the current clipboard representations with the provider's explicit input budget.
    fn read(&self) -> ClipboardResult<ClipboardBundle>;
    /// Publish the supplied representations together or return an actionable failure.
    fn write(&self, bundle: &ClipboardBundle) -> ClipboardResult<()>;
    /// Maximum total bytes the host permits before interpreting external clipboard content.
    fn max_bytes(&self) -> usize;
}

/// An explicitly registered opaque host format with its platform type mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardFormat {
    /// Portable application MIME identifier used in [`ClipboardBundle::formats`].
    pub mime: String,
    /// Valid reverse-DNS UTI used for this format on macOS pasteboards.
    pub macos_uti: String,
}

/// An untrusted canonical image reference; the host owns project authorization and acquisition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetReference {
    /// Optional local/project asset identity from the selected image.
    pub asset_id: Option<String>,
    /// Canonical source reference. This module never fetches it.
    pub source: String,
}

/// A canonical potentially open fragment; fields preserve exact original Node serialization.
#[derive(Debug, Clone, PartialEq)]
pub struct RichSlice {
    /// Selected canonical content, with only selected text inside boundary wrappers.
    pub content: Vec<Node>,
    /// Number of open contextual wrappers at the first edge.
    pub open_start: usize,
    /// Number of open contextual wrappers at the final edge.
    pub open_end: usize,
}
impl RichSlice {
    /// Encode without rewriting attributes, unknown fields or null/omitted presence.
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate_edges()?;
        Node::decode_open_fragment(
            serde_json::to_value(&self.content).map_err(craft_editor_model::Error::from)?,
        )?;
        Ok(serde_json::to_vec(&json!({"version":1,"content":self.content,"openStart":self.open_start,"openEnd":self.open_end})).map_err(craft_editor_model::Error::from)?)
    }
    /// Decode untrusted bytes after an explicit host budget and canonical fragment validation.
    pub fn decode(bytes: &[u8], max_bytes: usize) -> Result<Self> {
        if bytes.len() > max_bytes {
            return Err(ClipboardError::ResourceLimit.into());
        }
        let value: Value =
            serde_json::from_slice(bytes).map_err(craft_editor_model::Error::from)?;
        let object = value.as_object().ok_or(ClipboardError::InvalidSlice)?;
        if object.get("version").and_then(Value::as_u64) != Some(1)
            || object.keys().any(|key| {
                !matches!(
                    key.as_str(),
                    "version" | "content" | "openStart" | "openEnd"
                )
            })
        {
            return Err(ClipboardError::InvalidSlice.into());
        }
        let edge = |name| -> Result<usize> {
            usize::try_from(
                object
                    .get(name)
                    .and_then(Value::as_u64)
                    .ok_or(ClipboardError::InvalidSlice)?,
            )
            .map_err(|_| ClipboardError::InvalidSlice.into())
        };
        let slice = Self {
            content: Node::decode_open_fragment(
                object
                    .get("content")
                    .cloned()
                    .ok_or(ClipboardError::InvalidSlice)?,
            )?,
            open_start: edge("openStart")?,
            open_end: edge("openEnd")?,
        };
        slice.validate_edges()?;
        Ok(slice)
    }
    fn validate_edges(&self) -> Result<()> {
        fn depth(nodes: &[Node], end: bool) -> usize {
            let mut nodes = nodes;
            let mut depth = 0;
            while let Some(node) = if end { nodes.last() } else { nodes.first() } {
                if matches!(
                    node.node_type.as_str(),
                    "text"
                        | "hardBreak"
                        | "inlineMath"
                        | "imageBlock"
                        | "mermaidBlock"
                        | "mathBlock"
                        | "horizontalRule"
                        | "sceneBreak"
                ) {
                    break;
                }
                depth += 1;
                nodes = node.children();
            }
            depth
        }
        if self.open_start > 64
            || self.open_end > 64
            || self.open_start > depth(&self.content, false)
            || self.open_end > depth(&self.content, true)
        {
            return Err(ClipboardError::InvalidSlice.into());
        }
        Ok(())
    }
    /// Enumerate image references for host project/asset policy, without opening or fetching them.
    pub fn asset_references(&self) -> Vec<AssetReference> {
        fn visit(node: &Node, output: &mut Vec<AssetReference>) {
            if node.node_type == "imageBlock" {
                output.push(AssetReference {
                    asset_id: node
                        .attrs
                        .as_ref()
                        .and_then(|attrs| attrs.get("assetId"))
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    source: node
                        .attrs
                        .as_ref()
                        .and_then(|attrs| attrs.get("src"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                });
            }
            for child in node.children() {
                visit(child, output);
            }
        }
        let mut output = Vec::new();
        for node in &self.content {
            visit(node, &mut output);
        }
        output
    }
    /// Build all representations, preserving canonical metadata only in the private rich type.
    pub fn bundle(&self) -> Result<ClipboardBundle> {
        Ok(ClipboardBundle {
            rich: Some(self.encode()?),
            html: Some(html::export(&self.content)),
            plain: plain::export(&self.content),
            formats: BTreeMap::new(),
        })
    }
}

impl EditorSession {
    /// Install a shared host-owned acknowledged clipboard service without changing the document.
    pub fn set_clipboard_provider(&mut self, provider: Option<Arc<dyn ClipboardProvider>>) {
        self.clipboard_provider = provider;
    }
    /// Copy the exact selection. A caret without selected content leaves the clipboard untouched.
    pub fn copy_selection(&self) -> Result<Option<ClipboardBundle>> {
        if self.is_composing() {
            return Err(Error::CompositionActive);
        }
        if let Some(reference) = self.node_selection() {
            if self.document().node(&reference.path)? != reference.expected.as_ref() {
                return Err(craft_editor_model::Error::NodeReferenceConflict.into());
            }
            return RichSlice {
                content: vec![reference.expected.as_ref().clone()],
                open_start: 0,
                open_end: 0,
            }
            .bundle()
            .map(Some);
        }
        let Some(selection) = self.selection() else {
            return Ok(None);
        };
        if selection.anchor == selection.head {
            return Ok(None);
        }
        let projection = Projection::new(self.document())?;
        let a = projection.scalar(&selection.anchor)?;
        let b = projection.scalar(&selection.head)?;
        let from = a.min(b);
        let to = a.max(b);
        let mut selected = std::collections::HashMap::new();
        for span in &projection.spans {
            if span.atom {
                if span.start >= from && span.end <= to {
                    selected.insert(span.path.clone(), span.node.clone());
                }
            } else if span.start <= to && span.end >= from {
                let start = from.max(span.start) - span.start;
                let end = to.min(span.end) - span.start;
                if start < end
                    || (from < to
                        && (span.end == from
                            || span.start == to
                            || (span.start >= from && span.end <= to)))
                {
                    let mut node = span.node.clone();
                    if start != 0 || end != span.end - span.start {
                        node.content = Some(cut_inline(node.children(), start, end)?);
                    }
                    selected.insert(span.path.clone(), node);
                }
            }
        }
        fn extract(
            node: &Node,
            path: &NodePath,
            selected: &std::collections::HashMap<Vec<usize>, Node>,
        ) -> Option<Node> {
            if let Some(node) = selected.get(&path.0) {
                return Some(node.clone());
            }
            let children = node
                .children()
                .iter()
                .enumerate()
                .filter_map(|(index, node)| extract(node, &path.child(index), selected))
                .collect::<Vec<_>>();
            if children.is_empty() {
                return None;
            }
            let mut node = node.clone();
            node.content = Some(children);
            Some(node)
        }
        let content = self
            .document()
            .blocks()
            .iter()
            .enumerate()
            .filter_map(|(index, node)| extract(node, &NodePath(vec![index]), &selected))
            .collect::<Vec<_>>();
        let first = if a <= b {
            &selection.anchor
        } else {
            &selection.head
        };
        let last = if a <= b {
            &selection.head
        } else {
            &selection.anchor
        };
        let path_depth = |id: &str| -> Result<usize> {
            projection
                .spans
                .iter()
                .find(|span| !span.atom && span.id == id)
                .map(|span| span.path.len())
                .ok_or(Error::InvalidSelection)
        };
        RichSlice {
            content,
            open_start: path_depth(&first.block_id)?,
            open_end: path_depth(&last.block_id)?,
        }
        .bundle()
        .map(Some)
    }
    /// Publish before deleting the selected content. Clipboard failure never changes the document.
    pub fn cut_to_clipboard(&mut self) -> Result<Option<Change>> {
        let Some(bundle) = self.copy_selection()? else {
            return Ok(None);
        };
        self.clipboard_provider
            .as_ref()
            .ok_or(ClipboardError::Unavailable)?
            .write(&bundle)?;
        if self.node_selection().is_some() {
            self.delete_selected_node().map(Some)
        } else {
            self.insert_plain_text("").map(Some)
        }
    }
    /// Paste native representations without fetching resources or silently flattening rich input.
    pub fn paste_from_clipboard(&mut self) -> Result<Change> {
        if self.clipboard_intent_gate().has_pending_write() {
            return Err(ClipboardError::WritePending.into());
        }
        let provider = self
            .clipboard_provider
            .clone()
            .ok_or(ClipboardError::Unavailable)?;
        let bundle = provider.read()?;
        self.paste_bundle(&bundle, provider.max_bytes())
    }
    /// Parse and apply a bounded bundle through one canonical transaction and rich history.
    pub fn paste_bundle(&mut self, bundle: &ClipboardBundle, max_bytes: usize) -> Result<Change> {
        if let Some(slice) = bundle.canonical_slice(max_bytes)? {
            return self.paste_rich_slice(slice);
        }
        if bundle.plain.is_empty() {
            // Original ProseMirror parseFromClipboard returns no slice for an empty clipboard.
            // An unavailable text representation must never mean "delete the selected writing".
            return self.apply_commands(vec![]);
        }
        self.insert_plain_text(&bundle.plain.replace("\r\n", "\n"))
    }
    fn paste_rich_slice(&mut self, mut slice: RichSlice) -> Result<Change> {
        slice.validate_edges()?;
        // A copy creates new block identities. Source references and every other field stay exact.
        let mut reserved = insertion::block_id_reservations(self.document());
        fn reserve(nodes: &[Node], reserved: &mut HashSet<String>) {
            for node in nodes {
                if let Some(id) = node.block_id() {
                    reserved.insert(id.into());
                }
                reserve(node.children(), reserved);
            }
        }
        reserve(&slice.content, &mut reserved);
        let mut next = self.next_id;
        fn renew(
            nodes: &mut [Node],
            prefix: u64,
            next: &mut u64,
            reserved: &mut HashSet<String>,
        ) -> Result<()> {
            for node in nodes {
                if node.block_id().is_some() {
                    let id = insertion::allocate_block_id(prefix, next, reserved)?;
                    node.attrs
                        .get_or_insert_default()
                        .insert("blockId".into(), Value::String(id));
                }
                if let Some(children) = &mut node.content {
                    renew(children, prefix, next, reserved)?;
                }
            }
            Ok(())
        }
        renew(
            &mut slice.content,
            self.revision().0,
            &mut next,
            &mut reserved,
        )?;
        self.insert_open_slice(slice)
    }
}

fn cut_inline(nodes: &[Node], from: usize, to: usize) -> Result<Vec<Node>> {
    let mut result = Vec::new();
    let mut offset = 0;
    for node in nodes {
        let length = node.inline_text().chars().count();
        let end = offset + length;
        if from < end && to > offset {
            let mut fragment = node.clone();
            if node.node_type == "text" {
                fragment.text = Some(
                    node.text
                        .as_deref()
                        .ok_or(Error::InvalidSelection)?
                        .chars()
                        .skip(from.saturating_sub(offset))
                        .take(to.min(end) - from.max(offset))
                        .collect(),
                );
            }
            result.push(fragment);
        }
        offset = end;
    }
    if to > offset || from > to {
        return Err(Error::InvalidSelection);
    }
    Ok(result)
}
