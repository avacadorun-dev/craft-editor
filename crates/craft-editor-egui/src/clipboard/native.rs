//! Safe optional OS transport; actual native interoperability is separately qualified.
use super::{
    ClipboardBundle, ClipboardError, ClipboardFormat, ClipboardProvider, ClipboardResult, RICH_MIME,
};

// NSPasteboard rejects raw MIME identifiers containing '/' or '+'. The canonical wire format
// remains portable; only the native type identifier differs on macOS.
const MACOS_RICH_UTI: &str = "app.craft-editor.document-slice-json";
fn rich_format() -> &'static str {
    if cfg!(target_os = "macos") {
        MACOS_RICH_UTI
    } else {
        RICH_MIME
    }
}
use clipboard_rs::{Clipboard, ClipboardContent, ClipboardContext, ContentFormat};
use std::sync::Mutex;

/// A long-lived native multi-format clipboard. The host chooses its explicit input budget.
pub struct NativeClipboard {
    context: Mutex<ClipboardContext>,
    max_bytes: usize,
    formats: Vec<ClipboardFormat>,
}
impl NativeClipboard {
    /// Open the native clipboard. Do not instantiate per copy on selection-owned desktops.
    pub fn new(max_bytes: usize) -> ClipboardResult<Self> {
        Self::with_formats(max_bytes, Vec::new())
    }
    /// Open with an explicit allowlist of additional opaque formats. Other OS formats are not read.
    pub fn with_formats(max_bytes: usize, formats: Vec<ClipboardFormat>) -> ClipboardResult<Self> {
        let mut mimes = std::collections::HashSet::new();
        let mut utis = std::collections::HashSet::new();
        for format in &formats {
            if !valid_mime(&format.mime)
                || !valid_uti(&format.macos_uti)
                || matches!(format.mime.as_str(), RICH_MIME | "text/html" | "text/plain")
                || matches!(
                    format.macos_uti.as_str(),
                    MACOS_RICH_UTI | "public.html" | "public.utf8-plain-text"
                )
                || !mimes.insert(format.mime.clone())
                || !utis.insert(format.macos_uti.clone())
            {
                return Err(ClipboardError::InvalidSlice);
            }
        }
        Ok(Self {
            context: Mutex::new(ClipboardContext::new().map_err(|_| ClipboardError::Unavailable)?),
            max_bytes,
            formats,
        })
    }
}
impl ClipboardProvider for NativeClipboard {
    fn read(&self) -> ClipboardResult<ClipboardBundle> {
        let ctx = self
            .context
            .lock()
            .map_err(|_| ClipboardError::Unavailable)?;
        let rich = if ctx.has(ContentFormat::Other(rich_format().into())) {
            Some(
                ctx.get_buffer(rich_format())
                    .map_err(|_| ClipboardError::Read)?,
            )
        } else {
            None
        };
        let html = if ctx.has(ContentFormat::Html) {
            Some(ctx.get_html().map_err(|_| ClipboardError::Read)?)
        } else {
            None
        };
        let plain = if ctx.has(ContentFormat::Text) {
            ctx.get_text().map_err(|_| ClipboardError::Read)?
        } else {
            String::new()
        };
        let mut formats = std::collections::BTreeMap::new();
        for format in &self.formats {
            let native = format_name(format);
            if ctx.has(ContentFormat::Other(native.into())) {
                formats.insert(
                    format.mime.clone(),
                    ctx.get_buffer(native).map_err(|_| ClipboardError::Read)?,
                );
            }
        }
        let bundle = ClipboardBundle {
            rich,
            html,
            plain,
            formats,
        };
        bundle.check_budget(self.max_bytes)?;
        Ok(bundle)
    }
    fn write(&self, bundle: &ClipboardBundle) -> ClipboardResult<()> {
        bundle.check_budget(self.max_bytes)?;
        let ctx = self
            .context
            .lock()
            .map_err(|_| ClipboardError::Unavailable)?;
        let mut contents = vec![ClipboardContent::Text(bundle.plain.clone())];
        if let Some(html) = &bundle.html {
            contents.push(ClipboardContent::Html(html.clone()));
        }
        if let Some(rich) = &bundle.rich {
            contents.push(ClipboardContent::Other(rich_format().into(), rich.clone()));
        }
        for (mime, bytes) in &bundle.formats {
            let format = self
                .formats
                .iter()
                .find(|format| &format.mime == mime)
                .ok_or(ClipboardError::InvalidSlice)?;
            contents.push(ClipboardContent::Other(
                format_name(format).into(),
                bytes.clone(),
            ));
        }
        ctx.set(contents).map_err(|_| ClipboardError::Write)?;
        // A successful backend call alone is insufficient: NSPasteboard can reject one custom
        // type while the library reports success. Refuse Cut's acknowledgement unless every
        // advertised representation is actually readable from the current native clipboard.
        if ctx.get_text().map_err(|_| ClipboardError::Write)? != bundle.plain {
            return Err(ClipboardError::Write);
        }
        if let Some(rich) = &bundle.rich
            && ctx
                .get_buffer(rich_format())
                .map_err(|_| ClipboardError::Write)?
                != *rich
        {
            return Err(ClipboardError::Write);
        }
        if let Some(html) = &bundle.html
            && ctx.get_html().map_err(|_| ClipboardError::Write)? != *html
        {
            return Err(ClipboardError::Write);
        }
        for (mime, bytes) in &bundle.formats {
            let format = self
                .formats
                .iter()
                .find(|format| &format.mime == mime)
                .ok_or(ClipboardError::InvalidSlice)?;
            if ctx
                .get_buffer(format_name(format))
                .map_err(|_| ClipboardError::Write)?
                != *bytes
            {
                return Err(ClipboardError::Write);
            }
        }
        Ok(())
    }
    fn max_bytes(&self) -> usize {
        self.max_bytes
    }
}

fn format_name(format: &ClipboardFormat) -> &str {
    if cfg!(target_os = "macos") {
        &format.macos_uti
    } else {
        &format.mime
    }
}
fn valid_uti(value: &str) -> bool {
    value.contains('.')
        && value.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == b'-')
        })
}
fn valid_mime(value: &str) -> bool {
    value.split_once('/').is_some_and(|(kind, name)| {
        !kind.is_empty()
            && !name.is_empty()
            && kind
                .bytes()
                .chain(name.bytes())
                .all(|ch| ch.is_ascii_alphanumeric() || b".+-".contains(&ch))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_private_type_uses_a_valid_macos_uti_without_changing_the_portable_mime() {
        assert!(MACOS_RICH_UTI.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == b'-')
        }));
        assert_ne!(MACOS_RICH_UTI, RICH_MIME);
        assert_eq!(
            rich_format(),
            if cfg!(target_os = "macos") {
                MACOS_RICH_UTI
            } else {
                RICH_MIME
            }
        );
    }
}
