//! Explicit host-owned fonts for the same native database used by layout and rasterization.
//!
//! Registration does not read paths, scan the machine, persist fonts or grant redistribution rights.
//! The host supplies acknowledged bytes and retains their license. Register before first layout.
use skrifa::{MetadataProvider, raw::FileRef, string::StringId};
use wordcraft_fonts::FontDb;

/// Content-free native font failure; manuscript text and file paths are never included.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The bytes do not contain a usable supported OpenType font/collection with named faces.
    #[error("native_font_invalid")]
    InvalidFont,
    /// The file exceeds the pinned backend's 256-face registration capacity.
    #[error("native_font_face_capacity")]
    FaceCapacity,
    /// A validated family could not be found in the native loaded-font database after registration.
    #[error("native_font_registration_failed")]
    RegistrationFailed,
    /// Coverage requires an explicitly loaded family, not an unrelated implicit fallback.
    #[error("native_font_family_not_loaded")]
    FamilyNotLoaded,
    /// A metric was requested at an invalid size or the exact face lacks its zero glyph.
    #[error("native_font_invalid_metric")]
    InvalidMetric,
}
/// Native font registration or coverage result.
pub type Result<T> = std::result::Result<T, Error>;

/// Acknowledgement of a host-owned, process-local registration.
#[derive(Debug, Clone)]
pub struct Registration {
    /// New faces actually inserted; zero means the usable family/style was already present.
    pub faces_added: usize,
    /// Intrinsic named families validated from the supplied file, sorted and deduplicated.
    pub families: Vec<String>,
    /// Native database generation after acknowledgement; view caches must include it.
    pub generation: u64,
}

/// A direct cmap check for the actual loaded family/style, independent of ambient fallback fonts.
#[derive(Debug, Clone)]
pub struct GlyphCoverage {
    /// Requested intrinsic family name.
    pub family: String,
    /// Requested style, retained exactly for diagnostics.
    pub requested_style: String,
    /// Face style the native backend actually resolved.
    pub resolved_style: String,
    /// Whether the requested style exists exactly; a Regular fallback does not prove Bold parity.
    pub exact_style: bool,
    /// Distinct uncovered Unicode scalars in source order; this is not a complete shaping proof.
    pub missing: Vec<char>,
}

/// Validate every face before adding acknowledged font bytes to the native process database.
///
/// The native backend never replaces an existing family/style. Duplicate registration is therefore
/// idempotent. Font containers are decoded through the pinned backend policy; all named faces need
/// a usable cmap. Document-font registration is used rather than the backend's low-level add_font:
/// it atomically acknowledges faces, forgets old fallback choices and bumps the font generation.
/// No font folder is configured and no bytes are persisted as a user-installed font.
pub fn register_font(bytes: Vec<u8>) -> Result<Registration> {
    let sfnt = wordcraft_fonts::decode_font_file(&bytes).map_err(|_| Error::InvalidFont)?;
    drop(bytes);
    let families = {
        let file = FileRef::new(&sfnt).map_err(|_| Error::InvalidFont)?;
        let mut families = Vec::new();
        let mut faces = 0;
        for face in file.fonts() {
            let face = face.map_err(|_| Error::InvalidFont)?;
            if face.charmap().mappings().next().is_none() {
                return Err(Error::InvalidFont);
            }
            let family = [StringId::TYPOGRAPHIC_FAMILY_NAME, StringId::FAMILY_NAME]
                .into_iter()
                .find_map(|id| {
                    face.localized_strings(id)
                        .english_or_first()
                        .map(|value| value.chars().take(128).collect::<String>())
                        .filter(|value| !value.is_empty())
                })
                .ok_or(Error::InvalidFont)?;
            // The pinned backend enumerates at most 256 collection/variable-instance faces.
            // Reject before global mutation rather than silently register a prefix.
            faces += face.named_instances().iter().take(257).count().max(1);
            if faces > 256 {
                return Err(Error::FaceCapacity);
            }
            if !families.contains(&family) {
                families.push(family);
            }
        }
        if faces == 0 {
            return Err(Error::InvalidFont);
        }
        families.sort();
        families
    };
    let database = FontDb::global();
    // An empty document alias retains each face's intrinsic name in the pinned backend.
    let faces_added = database.add_document_fonts(vec![(sfnt, String::new())]);
    if families.iter().any(|family| !database.is_loaded(family)) {
        return Err(Error::RegistrationFailed);
    }
    Ok(Registration {
        faces_added,
        families,
        generation: database.generation(),
    })
}

/// Advance of the actual loaded face's zero glyph, in logical points.
/// This is the `ch` reference metric; it is not an average-character approximation.
///
/// # Errors
/// Rejects unknown/fallback faces, missing exact styles, invalid sizes and missing glyphs.
pub fn zero_advance(family: &str, style: &str, size: f32) -> Result<f32> {
    if !size.is_finite() || size <= 0.0 {
        return Err(Error::InvalidMetric);
    }
    let database = FontDb::global();
    if !database.is_loaded(family) {
        return Err(Error::FamilyNotLoaded);
    }
    let face = database.face(family, style);
    if !face.named(family) || !face.style.eq_ignore_ascii_case(style) {
        return Err(Error::FamilyNotLoaded);
    }
    let glyph = face.glyph_for('0');
    let advance = face.advance(glyph) / face.upem * f64::from(size);
    if glyph == 0 || !advance.is_finite() || advance <= 0.0 {
        return Err(Error::InvalidMetric);
    }
    Ok(advance as f32)
}

/// Diagnose whether an explicit OpenType feature value changes language-neutral
/// native shaping for an exact loaded face and sample. This does not change any
/// font, layout setting or canonical text; a host can qualify a migration policy.
///
/// # Errors
/// Rejects a missing family/style instead of silently measuring a fallback.
pub fn feature_changes_shaping(
    family: &str,
    style: &str,
    text: &str,
    tag: [u8; 4],
    value: u32,
) -> Result<bool> {
    let database = FontDb::global();
    if !database.is_loaded(family) {
        return Err(Error::FamilyNotLoaded);
    }
    let face = database.face(family, style);
    if !face.named(family) || !face.style.eq_ignore_ascii_case(style) {
        return Err(Error::FamilyNotLoaded);
    }
    let baseline = wordcraft_fonts::shape(&face, text, &[], |scalar| scalar);
    let feature = wordcraft_fonts::feature_value(&tag, value);
    let explicit = wordcraft_fonts::shape(&face, text, &[feature], |scalar| scalar);
    Ok(baseline != explicit)
}

/// Current shared native font generation. Include this in geometry/raster cache keys.
pub fn generation() -> u64 {
    FontDb::global().generation()
}

/// Inspect a loaded face's exact cmap coverage without silently using a different family.
///
/// Control characters do not request visible glyphs. Other scalars, including combining marks,
/// are checked explicitly; a successful cmap check does not independently prove shaping, style
/// selection, baseline alignment or native IME behavior.
pub fn coverage(family: &str, style: &str, text: &str) -> Result<GlyphCoverage> {
    let database = FontDb::global();
    if !database.is_loaded(family) {
        return Err(Error::FamilyNotLoaded);
    }
    let face = database.face(family, style);
    if !face.named(family) {
        return Err(Error::FamilyNotLoaded);
    }
    let mut missing = Vec::new();
    for scalar in text.chars().filter(|scalar| !scalar.is_control()) {
        if !face.covers(scalar) && !missing.contains(&scalar) {
            missing.push(scalar);
        }
    }
    Ok(GlyphCoverage {
        family: face.family.clone(),
        requested_style: style.into(),
        resolved_style: face.style.clone(),
        exact_style: face.style.eq_ignore_ascii_case(style),
        missing,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zero_metric_uses_an_exact_face_and_scales_linearly() -> Result<()> {
        let small = zero_advance("Source Serif 4", "Regular", 17.0)?;
        let large = zero_advance("Source Serif 4", "Regular", 34.0)?;
        assert!(small > 0.0 && (large - small * 2.0).abs() < 0.001);
        assert!(matches!(
            zero_advance("Source Serif 4", "Imaginary", 17.0),
            Err(Error::FamilyNotLoaded)
        ));
        assert!(matches!(
            zero_advance("missing-family", "Regular", 17.0),
            Err(Error::FamilyNotLoaded)
        ));
        assert!(matches!(
            zero_advance("Source Serif 4", "Regular", f32::NAN),
            Err(Error::InvalidMetric)
        ));
        Ok(())
    }

    #[test]
    fn invalid_font_never_changes_native_generation() {
        let before = generation();
        assert!(matches!(
            register_font(b"not a font".to_vec()),
            Err(Error::InvalidFont)
        ));
        assert_eq!(generation(), before);
    }
    #[test]
    fn truncated_opentype_header_is_rejected_before_registration() {
        let before = generation();
        assert!(matches!(
            register_font(b"OTTO invalid".to_vec()),
            Err(Error::InvalidFont)
        ));
        assert_eq!(generation(), before);
    }
    #[test]
    fn acknowledged_bundled_font_is_idempotent_and_keeps_intrinsic_name() -> Result<()> {
        let bytes = wordcraft_fonts::bundled()
            .first()
            .ok_or(Error::InvalidFont)?;
        let first = register_font(bytes.to_vec())?;
        let second = register_font(bytes.to_vec())?;
        assert!(
            first
                .families
                .iter()
                .any(|family| family == "Source Sans 3")
        );
        assert_eq!(second.faces_added, 0);
        assert_eq!(first.generation, second.generation);
        Ok(())
    }
    #[test]
    fn coverage_does_not_claim_an_unknown_family_works_via_fallback() {
        assert!(matches!(
            coverage("Craft Missing Test Family", "Regular", "Story"),
            Err(Error::FamilyNotLoaded)
        ));
    }
    #[test]
    fn direct_latin_cyrillic_coverage_is_distinct_from_missing_ideographs() -> Result<()> {
        let report = coverage("Source Sans 3", "Regular", "Story Привет é\n中中")?;
        assert_eq!(report.missing, vec!['中']);
        assert!(report.exact_style);
        Ok(())
    }
}
