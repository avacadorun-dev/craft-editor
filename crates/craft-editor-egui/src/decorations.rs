//! Ephemeral, revision-resolved text decorations supplied by the host.
//!
//! Decorations never become canonical marks or manuscript text. Business metadata,
//! persistence and sealed note-attachment verification stay in the consuming application.
use craft_editor_model::{Document, ReplayProof, Revision};
use egui::Color32;
#[cfg(feature = "wordcraft-canvas")]
use std::collections::HashSet;

#[cfg(feature = "wordcraft-canvas")]
use crate::{Error, projection::Projection};
use crate::{Result, Selection};

/// One host-owned text range and its native presentation.
#[derive(Clone, Debug, PartialEq)]
pub struct TextDecoration {
    /// Opaque stable host identity returned on activation; no domain meaning is inferred.
    pub id: String,
    /// Exact directed Unicode-scalar selection in the supplied current document.
    pub range: Selection,
    /// Host semantic tint; the canvas limits opacity to keep glyphs readable.
    pub color: Color32,
    /// Host-localized accessible/hover explanation, separate from canonical content.
    pub label: String,
}

/// Passive host-owned spelling/proofing range. It has no activation identity,
/// never takes pointer focus, and never replaces the author's native selection.
#[derive(Clone, Debug, PartialEq)]
pub struct TextUnderline {
    /// Exact Unicode-scalar range in the current document.
    pub range: Selection,
    /// Theme-aware underline color.
    pub color: Color32,
    /// Host-localized hover explanation, separate from reader text.
    pub label: String,
}

/// Resolve authoritative ephemeral ranges after this frame's native input.
///
/// The host can combine its already drained proofs with the current undrained
/// journal and verify exact business anchors before returning ranges. No raw
/// caller position map is treated as canonical authority by the editor.
pub trait DecorationSource {
    /// Resolve ranges against the exact document/revision that will be painted.
    ///
    /// # Errors
    /// Return an explicit error if source metadata cannot be verified. The canvas
    /// keeps ordinary prose usable and does not paint potentially stale ranges.
    fn decorations(
        &mut self,
        document: &Document,
        revision: Revision,
        pending: &[ReplayProof],
    ) -> Result<Vec<TextDecoration>>;

    /// Resolve passive wavy underlines independently of interactive passage marks.
    /// Existing sources need not provide any. The same exact revision and current
    /// replay journal are supplied; callers must discard stale analysis themselves.
    ///
    /// # Errors
    /// Invalid or unavailable analysis may fail without blocking manuscript input.
    fn underlines(
        &mut self,
        _document: &Document,
        _revision: Revision,
        _pending: &[ReplayProof],
    ) -> Result<Vec<TextUnderline>> {
        Ok(Vec::new())
    }
}

#[cfg(feature = "wordcraft-canvas")]
pub(crate) fn validate(document: &Document, decorations: &[TextDecoration]) -> Result<()> {
    let projection = Projection::new(document)?;
    let mut ids = HashSet::new();
    for decoration in decorations {
        if decoration.id.is_empty() || !ids.insert(&decoration.id) {
            return Err(Error::InvalidDecoration);
        }
        let a = projection.scalar(&decoration.range.anchor)?;
        let b = projection.scalar(&decoration.range.head)?;
        if a == b {
            return Err(Error::InvalidDecoration);
        }
    }
    Ok(())
}

#[cfg(feature = "wordcraft-canvas")]
pub(crate) fn validate_underlines(document: &Document, underlines: &[TextUnderline]) -> Result<()> {
    let projection = Projection::new(document)?;
    for underline in underlines {
        let a = projection.scalar(&underline.range.anchor)?;
        let b = projection.scalar(&underline.range.head)?;
        if a == b {
            return Err(Error::InvalidDecoration);
        }
    }
    Ok(())
}
