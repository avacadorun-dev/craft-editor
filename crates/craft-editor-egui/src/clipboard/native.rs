//! Safe optional OS transport; actual native interoperability is separately qualified.
use super::{ClipboardBundle, ClipboardError, ClipboardProvider, ClipboardResult, RICH_MIME};

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
}
impl NativeClipboard {
    /// Open the native clipboard. Do not instantiate per copy on selection-owned desktops.
    pub fn new(max_bytes: usize) -> ClipboardResult<Self> {
        Ok(Self {
            context: Mutex::new(ClipboardContext::new().map_err(|_| ClipboardError::Unavailable)?),
            max_bytes,
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
        let bundle = ClipboardBundle { rich, html, plain };
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
        Ok(())
    }
    fn max_bytes(&self) -> usize {
        self.max_bytes
    }
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
