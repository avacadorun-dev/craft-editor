//! Hosted-only native clipboard ownership probe with fresh synthetic content.
use craft_editor_egui::clipboard::{
    ClipboardBundle, ClipboardFormat, ClipboardProvider, NativeClipboard, RichSlice,
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
    let formats = vec![ClipboardFormat {
        mime: "application/vnd.craft-editor.clipboard-probe+json".into(),
        macos_uti: "app.craft-editor.clipboard-probe-json".into(),
    }];
    let writer = Arc::new(NativeClipboard::with_formats(
        32 * 1024 * 1024,
        formats.clone(),
    )?);
    let reader = NativeClipboard::with_formats(32 * 1024 * 1024, formats)?;
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
                json!({"native_clipboard":true,"backend":if cfg!(target_os="macos"){"macOS pasteboard"}else{"isolated X11"},"fresh_nonce_verified":true,"private_json":true,"html":true,"unicode_plain":true,"canonical_roundtrip":true,"registered_opaque_format":true,"synthetic_fixture_restored":true,"empty_plain_rich_cut_undo":true,"native_ui_acceptance":false})
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
    let mut bundle = session
        .copy_selection()?
        .ok_or("native_clipboard_missing_selection")?;
    bundle.formats.insert(
        "application/vnd.craft-editor.clipboard-probe+json".into(),
        serde_json::to_vec(&json!({"nonce":nonce,"opaque":true}))?,
    );
    writer.write(&bundle)?;
    let actual = reader.read()?;
    if actual.plain != bundle.plain || !actual.plain.starts_with(nonce) {
        return Err("native_clipboard_plain_nonce_mismatch".into());
    }
    if actual.formats != bundle.formats {
        return Err("native_clipboard_registered_format_mismatch".into());
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
    let atom = Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":4},"content":[{"type":"mermaidBlock","attrs":{"blockId":"native-atom","code":format!("graph TD; A[\"{nonce}\"]-->B"),"caption":nonce}}]}),
    )?;
    let mut session = EditorSession::new(atom.clone(), Revision(0));
    let selected = session
        .document()
        .node_ref(&craft_editor_model::NodePath(vec![0]))?;
    session.select_node(selected.clone())?;
    let Some(craft_editor_egui::clipboard::ClipboardRequest::Cut {
        bundle,
        ticket,
        guard,
    }) = session.begin_cut()?
    else {
        return Err("native_empty_plain_missing_cut".into());
    };
    if !bundle.plain.is_empty() || bundle.rich.is_none() || bundle.html.is_none() {
        return Err("native_empty_plain_wrong_source_contract".into());
    }
    let ack = ticket.write(writer.as_ref(), &bundle)?;
    let actual = reader.read()?;
    if actual != *bundle {
        return Err("native_empty_plain_rich_roundtrip_mismatch".into());
    }
    session.finish_cut(guard, ack)?;
    if session.document() == &atom {
        return Err("native_empty_plain_cut_not_applied".into());
    }
    session.undo()?;
    if session.document() != &atom || session.node_selection() != Some(&selected) {
        return Err("native_empty_plain_cut_undo_mismatch".into());
    }
    Ok(())
}
