//! Hosted-only native clipboard ownership probe with fresh synthetic content.
use craft_editor_egui::clipboard::{
    ClipboardBundle, ClipboardProvider, NativeClipboard, RichSlice,
};
use craft_editor_egui::{DocPoint, EditorSession, Selection};
use craft_editor_model::{Document, Revision, ScalarOffset};
use serde_json::json;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

type ProbeResult<T> = Result<T, Box<dyn std::error::Error>>;
fn main() -> ProbeResult<()> {
    let nonce = format!(
        "craft-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    );
    let writer = Arc::new(NativeClipboard::new(32 * 1024 * 1024)?);
    let reader = NativeClipboard::new(32 * 1024 * 1024)?;
    // Never read or print ambient clipboard data. Establish our own fresh fixture first and
    // restore that synthetic fixture even if the actual rich-content probe rejects.
    let fixture = ClipboardBundle {
        plain: format!("{nonce}-fixture"),
        ..Default::default()
    };
    writer.write(&fixture)?;
    let result = exercise(&nonce, &writer, &reader);
    let cleanup = writer.write(&fixture);
    match (result, cleanup) {
        (Err(error), Err(_)) => {
            eprintln!("native_clipboard_fixture_cleanup_failed");
            Err(error)
        }
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(error.into()),
        (Ok(()), Ok(())) => {
            println!(
                "{}",
                json!({"native_clipboard":true,"backend":if cfg!(target_os="macos"){"macOS pasteboard"}else{"isolated X11"},"fresh_nonce_verified":true,"private_json":true,"html":true,"unicode_plain":true,"canonical_roundtrip":true,"synthetic_fixture_restored":true,"native_ui_acceptance":false})
            );
            Ok(())
        }
    }
}
fn exercise(
    nonce: &str,
    writer: &Arc<NativeClipboard>,
    reader: &NativeClipboard,
) -> ProbeResult<()> {
    let fixture = reader.read()?;
    if fixture.plain != format!("{nonce}-fixture") {
        return Err("native_clipboard_fixture_nonce_mismatch".into());
    }
    let source = Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":4},"content":[{"type":"paragraph","attrs":{"blockId":"source","sourceRefs":["synthetic-voice"],"vendor":null},"vendorBlock":true,"content":[{"type":"text","text":format!("{nonce} Автор 😀 "),"marks":[{"type":"bold"},{"type":"link","attrs":{"href":"https://example.test","vendor":null}}]},{"type":"inlineMath","attrs":{"latex":"x^2","description":"square"}}]}]}),
    )?;
    let mut session = EditorSession::new(source.clone(), Revision(0));
    session.set_selection(Selection {
        anchor: DocPoint {
            block_id: "source".into(),
            offset: ScalarOffset(0),
        },
        head: DocPoint {
            block_id: "source".into(),
            offset: ScalarOffset(source.block_text("source")?.chars().count()),
        },
    })?;
    let bundle = session
        .copy_selection()?
        .ok_or("native_clipboard_missing_selection")?;
    writer.write(&bundle)?;
    let actual = reader.read()?;
    if actual.plain != bundle.plain || !actual.plain.starts_with(nonce) {
        return Err("native_clipboard_plain_nonce_mismatch".into());
    }
    if actual.rich != bundle.rich {
        return Err("native_clipboard_private_type_mismatch".into());
    }
    let html = actual
        .html
        .as_deref()
        .ok_or("native_clipboard_missing_html")?;
    if !html.contains(nonce) || !html.contains("data-inline-math") {
        return Err("native_clipboard_html_semantics_mismatch".into());
    }
    let slice = RichSlice::decode(
        actual
            .rich
            .as_deref()
            .ok_or("native_clipboard_missing_private")?,
        32 * 1024 * 1024,
    )?;
    if slice.content != source.blocks() {
        return Err("native_clipboard_canonical_roundtrip_mismatch".into());
    }
    Ok(())
}
