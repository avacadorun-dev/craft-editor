use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// A Unicode scalar boundary, never a grapheme or UTF-16 index.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ScalarOffset(#[doc = "Scalar boundary index."] pub usize);
/// A JavaScript/ProseMirror UTF-16 code-unit boundary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Utf16Offset(#[doc = "UTF-16 code-unit boundary index."] pub usize);
/// A UTF-8 byte boundary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ByteOffset(#[doc = "UTF-8 byte boundary index."] pub usize);

/// A half-open range in the inline projection of one identified text block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextRange {
    /// Stable block identity, resolved uniquely at the transaction's revision.
    pub block_id: String,
    /// Inclusive scalar boundary.
    pub start: ScalarOffset,
    /// Exclusive scalar boundary.
    pub end: ScalarOffset,
}

/// Convert a valid scalar boundary into its UTF-8 byte boundary.
pub fn scalar_to_byte(text: &str, offset: ScalarOffset) -> Result<ByteOffset> {
    if offset.0 == text.chars().count() {
        return Ok(ByteOffset(text.len()));
    }
    text.char_indices()
        .nth(offset.0)
        .map(|(index, _)| ByteOffset(index))
        .ok_or(Error::InvalidPosition)
}
/// Convert a UTF-8 boundary, rejecting the interior of a code point.
pub fn byte_to_scalar(text: &str, offset: ByteOffset) -> Result<ScalarOffset> {
    if offset.0 > text.len() || !text.is_char_boundary(offset.0) {
        return Err(Error::InvalidPosition);
    }
    Ok(ScalarOffset(text[..offset.0].chars().count()))
}
/// Convert a valid scalar boundary into UTF-16 code units.
pub fn scalar_to_utf16(text: &str, offset: ScalarOffset) -> Result<Utf16Offset> {
    scalar_to_byte(text, offset)?;
    Ok(Utf16Offset(
        text.chars().take(offset.0).map(char::len_utf16).sum(),
    ))
}
/// Convert UTF-16 units, rejecting surrogate-pair interiors and out-of-range units.
pub fn utf16_to_scalar(text: &str, offset: Utf16Offset) -> Result<ScalarOffset> {
    let mut units = 0;
    for (scalar, character) in text.chars().enumerate() {
        if units == offset.0 {
            return Ok(ScalarOffset(scalar));
        }
        units += character.len_utf16();
        if units > offset.0 {
            return Err(Error::InvalidPosition);
        }
    }
    if units == offset.0 {
        Ok(ScalarOffset(text.chars().count()))
    } else {
        Err(Error::InvalidPosition)
    }
}
