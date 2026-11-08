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
