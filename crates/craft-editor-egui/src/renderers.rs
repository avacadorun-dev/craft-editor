//! Host-supplied, measured native media for canonical document atoms.
//!
//! Image bytes are disposable render output. Mermaid/TeX/source and media identity remain in
//! the canonical node. A resolver must not perform hidden network or filesystem access while
//! the editor measures a frame; hosts should prepare bounded, cached results off the UI thread.

use std::sync::Arc;

use craft_editor_model::Node;

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

    /// Monotonic identity of the currently acknowledged renderer/media catalog.
    fn generation(&self) -> u64;
}
