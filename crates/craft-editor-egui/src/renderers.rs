//! Host-supplied, measured native media for canonical document atoms.
//!
//! Image bytes are disposable render output. Mermaid/TeX/source and media identity remain in
//! the canonical node. A resolver must not perform hidden network or filesystem access while
//! the editor measures a frame; hosts should prepare bounded, cached results off the UI thread.

use std::sync::{Arc, OnceLock};

use craft_editor_model::{Node, NodePath};

use crate::{Result, layout::AtomSize};

/// Actual raster output and its corresponding logical document bounds.
#[derive(Debug, Clone)]
pub struct ResolvedAtom {
    /// Measured bounds in logical points, from the actual image/diagram/formula renderer.
    pub size: AtomSize,
    /// Logical distance from the raster top to its mathematical baseline.
    /// `None` aligns an ordinary image to the bottom; values must lie within the height.
    pub baseline: Option<f32>,
    /// Encoded PNG (preferred) or another image format supported by the pinned native backend.
    /// SVG must be rasterized before this boundary; source SVG is never executed by a browser.
    pub encoded: Arc<Vec<u8>>,
}

/// Explicit nonfinal presentation of one canonical atom, never editable document text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtomPlaceholderState {
    /// The host is preparing this exact source/presentation identity off the UI thread.
    Pending,
    /// Preparation failed; the unchanged canonical source remains available to the host editor.
    Failed,
}

/// Host-localized, measured status for an unavailable atom.
///
/// The canvas reserves the original opaque-node boundary and paints this native status box.
/// `detail` is a bounded source preview supplied by the host, not replacement document content.
#[derive(Debug, Clone)]
pub struct AtomPlaceholder {
    /// Reserved logical bounds; inline hosts should choose a compact single-line size.
    pub size: AtomSize,
    /// Whether preparation is pending or explicitly failed.
    pub state: AtomPlaceholderState,
    /// Localized status label.
    pub label: String,
    /// Optional bounded source/alternative-text preview, with an explicit ellipsis if shortened.
    pub detail: String,
}

impl AtomPlaceholder {
    pub(crate) fn reserved_image(&self) -> ResolvedAtom {
        // Transparent transport pixels reserve one native inline object. They are never presented
        // as a rendered diagram: the canvas paints the explicit native status box over its bounds.
        static PIXEL: OnceLock<Arc<Vec<u8>>> = OnceLock::new();
        const PNG: &[u8] = &[
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1,
            8, 6, 0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 11, 73, 68, 65, 84, 120, 156, 99, 96, 0, 2,
            0, 0, 5, 0, 1, 122, 94, 171, 63, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
        ];
        ResolvedAtom {
            size: self.size,
            baseline: None,
            encoded: Arc::clone(PIXEL.get_or_init(|| Arc::new(PNG.to_vec()))),
        }
    }
}

/// Immutable, generation-aware source of native atom render results.
///
/// Application policy controls media bytes and resource access. Implementations should cache
/// complete source/theme/scale identities, bound rendering work, and report explicit failures
/// while preserving the original canonical node. Changing source-independent output (such as
/// theme or an acknowledged media update) requires a new generation to invalidate view tiles.
pub trait AtomResolver: Send + Sync {
    /// Resolve one exact canonical atom at the available logical width.
    ///
    /// # Errors
    /// Returns a content-free renderer diagnostic when real output is unavailable. Do not
    /// substitute a fake diagram or silently remove canonical content.
    fn resolve(&self, node: &Node, available_width: f32) -> Result<ResolvedAtom>;

    /// Explicit pending/failed presentation when `resolve` is unavailable.
    ///
    /// Returning `None` retains the existing fail-closed geometry error. Hosts that return a
    /// placeholder must preserve canonical source and advance `generation` when its state changes.
    fn placeholder(&self, _node: &Node, _available_width: f32) -> Option<AtomPlaceholder> {
        None
    }

    /// Optional viewport hint for asynchronous, bounded derived-image caches.
    ///
    /// Paths identify currently visible canonical opaque atoms. A host may prioritize or lazily
    /// prepare these results; the callback must not block, mutate source, or perform I/O.
    fn prioritize(&self, _visible: &[NodePath]) {}

    /// Monotonic identity of the currently acknowledged renderer/media catalog.
    fn generation(&self) -> u64;
}
