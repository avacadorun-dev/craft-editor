use craft_editor_egui::clipboard::{
    AssetReference, ClipboardBundle, ClipboardRequest, ImageBinding, PasteContext, PasteGuard,
    PreparedPaste, RichSlice,
};
use craft_editor_egui::{DocPoint, EditorSession, MarkState, Selection};
use craft_editor_model::{Document, Mark, MarkMode, NodePath, Revision, ScalarOffset};
use serde_json::{Value, json};
type TestResult = Result<(), Box<dyn std::error::Error>>;
const LIMIT: usize = 4 * 1024 * 1024;
fn point(id: &str, offset: usize) -> DocPoint {
    DocPoint {
        block_id: id.into(),
        offset: ScalarOffset(offset),
    }
}
fn range(id: &str, from: usize, to: usize) -> Selection {
    Selection {
        anchor: point(id, from),
        head: point(id, to),
    }
}
fn editor() -> Result<EditorSession, craft_editor_egui::Error> {
    let mut editor = EditorSession::new(Document::plain("p", "leftright")?, Revision(0));
    editor.set_selection(range("p", 4, 4))?;
    Ok(editor)
}
fn capture(
    editor: &EditorSession,
    bundle: ClipboardBundle,
) -> Result<(PasteContext, PasteGuard), Box<dyn std::error::Error>> {
    let ClipboardRequest::Paste { context, guard, .. } = editor.begin_paste(bundle)? else {
        return Err("missing paste request".into());
    };
    Ok((context, guard))
}
fn plain(text: &str) -> ClipboardBundle {
    ClipboardBundle {
        plain: text.into(),
        ..Default::default()
    }
}
fn mark_at(editor: &EditorSession, text: &str, kind: &str) -> bool {
    editor
        .document()
        .blocks()
        .iter()
        .flat_map(|node| node.children())
        .any(|node| {
            node.text.as_deref() == Some(text)
                && node
                    .marks
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .any(|mark| mark.mark_type == kind)
        })
}
#[test]
fn preparation_is_worker_sendable_and_one_guard_cannot_accept_equal_bytes_from_another_capture()
-> TestResult {
    fn assert_send<T: Send>() {}
    assert_send::<PasteContext>();
    assert_send::<PreparedPaste>();
    let mut editor = editor()?;
    let original = editor.document().clone();
    let (context_a, guard_a) = capture(&editor, plain("**new**"))?;
    let (context_b, guard_b) = capture(&editor, plain("**new**"))?;
    let prepared_b = std::thread::spawn(move || context_b.prepare(LIMIT))
        .join()
        .map_err(|_| "worker panicked")??;
    assert!(editor.finish_prepared_paste(guard_a, prepared_b).is_err());
    assert_eq!(editor.document(), &original);
    assert!(!editor.can_undo());
    // A context clone retains its original identity; it cannot acquire another request's guard.
    assert!(
        editor
            .finish_prepared_paste(guard_b, context_a.prepare(LIMIT)?)
            .is_err()
    );
    assert_eq!(editor.document(), &original);
    Ok(())
}
#[test]
fn prepared_transform_commits_one_history_entry_and_rejects_changed_pending_typing_style()
-> TestResult {
    let mut editor = editor()?;
    let original = editor.document().clone();
    let (context, guard) = capture(&editor, plain("**new**"))?;
    editor.finish_prepared_paste(guard, context.prepare(LIMIT)?)?;
    assert_eq!(editor.document().block_text("p")?, "leftnewright");
    assert!(mark_at(&editor, "new", "bold"));
    editor.undo()?;
    assert_eq!(editor.document(), &original);
    assert_eq!(editor.selection(), Some(&range("p", 4, 4)));
    assert!(!editor.can_undo());
    let (context, guard) = capture(&editor, plain("new"))?;
    editor.format_selection(Mark::new("italic"), MarkMode::Add)?;
    assert_eq!(editor.revision(), Revision(2));
    assert!(
        editor
            .finish_prepared_paste(guard, context.prepare(LIMIT)?)
            .is_err()
    );
    assert_eq!(editor.document(), &original);
    Ok(())
}
#[test]
fn original_whole_url_intent_is_checked_before_marker_conversion() -> TestResult {
    let mut editor = editor()?;
    editor.set_selection(range("p", 0, 4))?;
    let original = editor.document().clone();
    editor.paste_bundle(&plain("**https://example.com**"), LIMIT)?;
    assert_eq!(
        editor.document().block_text("p")?,
        "https://example.comright"
    );
    assert!(mark_at(&editor, "https://example.com", "bold"));
    assert!(mark_at(&editor, "https://example.com", "link"));
    editor.undo()?;
    assert_eq!(editor.document(), &original);
    editor.paste_bundle(&plain("https://example.com"), LIMIT)?;
    assert_eq!(editor.document().block_text("p")?, "leftright");
    assert!(mark_at(&editor, "left", "link"));
    assert_eq!(editor.selection(), Some(&range("p", 0, 4)));
    editor.undo()?;
    assert_eq!(editor.document(), &original);
    assert_eq!(editor.selection(), Some(&range("p", 0, 4)));
    Ok(())
}
#[test]
fn source_whole_url_link_on_paste_covers_inline_atoms_and_skips_code_blocks() -> TestResult {
    let source = Document::from_json(json!({"type":"doc","attrs":{"schemaVersion":4},"content":[
        {"type":"codeBlock","attrs":{"blockId":"code","language":"rust"},"content":[{"type":"text","text":"AB"}]},
        {"type":"paragraph","attrs":{"blockId":"p","sourceRefs":["voice"]},"content":[
            {"type":"text","text":"C","marks":[{"type":"bold","vendor":null}]},
            {"type":"inlineMath","attrs":{"latex":"x","description":"keep"},"opaque":true},
            {"type":"hardBreak","vendor":null},{"type":"text","text":"D"}]}]}))?;
    let mut editor = EditorSession::new(source.clone(), Revision(0));
    editor.set_selection(Selection {
        anchor: point("code", 1),
        head: point("p", 4),
    })?;
    editor.paste_bundle(&plain("https://example.com"), LIMIT)?;
    assert_eq!(editor.document().block("code")?, source.block("code")?);
    for node in editor.document().block("p")?.children() {
        assert!(
            node.marks
                .as_deref()
                .unwrap_or_default()
                .iter()
                .any(|mark| mark.mark_type == "link")
        );
    }
    editor.undo()?;
    assert_eq!(editor.document(), &source);
    let reference = editor.document().node_ref(&NodePath(vec![1, 1]))?;
    editor.select_node(reference.clone())?;
    editor.paste_bundle(&plain("example.com"), LIMIT)?;
    assert_eq!(editor.mark_state("link")?, MarkState::On);
    assert_eq!(
        editor.node_selection().map(|node| &node.path),
        Some(&reference.path)
    );
    editor.undo()?;
    assert_eq!(editor.node_selection(), Some(&reference));
    assert_eq!(editor.document(), &source);
    Ok(())
}
#[test]
fn marker_paste_replaces_an_inline_node_selection_in_place_and_undo_restores_node_selection()
-> TestResult {
    let source = Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":4},"content":[{"type":"paragraph","attrs":{"blockId":"p"},"content":[{"type":"text","text":"A"},{"type":"inlineMath","attrs":{"latex":"x"}},{"type":"text","text":"B"}]}]}),
    )?;
    let mut editor = EditorSession::new(source.clone(), Revision(0));
    let reference = source.node_ref(&NodePath(vec![0, 1]))?;
    editor.select_node(reference.clone())?;
    editor.paste_bundle(&plain("**new**"), LIMIT)?;
    assert_eq!(editor.document().blocks().len(), 1);
    assert_eq!(editor.document().block_text("p")?, "AnewB");
    assert!(mark_at(&editor, "new", "bold"));
    editor.undo()?;
    assert_eq!(editor.document(), &source);
    assert_eq!(editor.node_selection(), Some(&reference));
    Ok(())
}
#[test]
fn private_and_internal_html_bypass_rules_while_external_html_converts_once() -> TestResult {
    let document = Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":4},"content":[{"type":"paragraph","attrs":{"blockId":"from","sourceRefs":["voice"],"vendor":null},"opaque":true,"content":[{"type":"text","text":"**bold** https://example.com","vendorLeaf":null}]}]}),
    )?;
    let private = RichSlice {
        content: document.blocks().to_vec(),
        open_start: 1,
        open_end: 1,
    }
    .bundle()?;
    for bundle in [
        private,
        ClipboardBundle {
            html: Some(
                "<div data-pm-slice=\"1 1 []\"><p>**bold** https://example.com</p></div>".into(),
            ),
            plain: "**bold** https://example.com".into(),
            ..Default::default()
        },
    ] {
        let mut editor = editor()?;
        editor.paste_bundle(&bundle, LIMIT)?;
        assert_eq!(
            editor.document().block_text("p")?,
            "left**bold** https://example.comright"
        );
        assert!(!mark_at(&editor, "bold", "bold"));
    }
    let mut editor = editor()?;
    editor.paste_bundle(
        &ClipboardBundle {
            html: Some("<p>**bold** https://example.com</p>".into()),
            plain: "**bold** https://example.com".into(),
            ..Default::default()
        },
        LIMIT,
    )?;
    assert!(mark_at(&editor, "bold", "bold"));
    assert!(mark_at(&editor, "https://example.com", "link"));
    Ok(())
}
#[test]
fn image_binding_changes_only_authorized_refs_after_external_rules_without_reparsing() -> TestResult
{
    let mut editor = editor()?;
    let bundle = ClipboardBundle { html:Some("<p>**bold**</p><figure data-image-block=\"\"><img src=\"https://example.com/a.png\" alt=\"Alt\"><figcaption>Caption</figcaption></figure>".into()), plain:"**bold**".into(), ..Default::default() };
    let (context, guard) = capture(&editor, bundle.clone())?;
    let mut prepared = context.prepare(LIMIT)?;
    assert_eq!(prepared.source(), &bundle);
    let before = prepared.rich_slice().ok_or("missing rich")?.clone();
    let original = prepared
        .asset_references()
        .first()
        .ok_or("missing image")?
        .clone();
    let binding = ImageBinding {
        original,
        destination: AssetReference {
            asset_id: Some("local-id".into()),
            source: "asset://local-id".into(),
        },
    };
    prepared.bind_images(std::slice::from_ref(&binding))?;
    let after = prepared.rich_slice().ok_or("missing transformed slice")?;
    assert_eq!(before.content[0], after.content[0]);
    let mut expected = serde_json::to_value(&before.content)?;
    fn patch(value: &mut Value) {
        if let Some(nodes) = value.as_array_mut() {
            for node in nodes {
                if node["type"] == "imageBlock" {
                    node["attrs"]["src"] = json!("asset://local-id");
                    node["attrs"]["assetId"] = json!("local-id");
                }
                if let Some(content) = node.get_mut("content") {
                    patch(content);
                }
            }
        }
    }
    patch(&mut expected);
    assert_eq!(serde_json::to_value(&after.content)?, expected);
    editor.finish_prepared_paste(guard, prepared)?;
    assert!(
        editor
            .document()
            .blocks()
            .iter()
            .any(|node| node.node_type == "imageBlock"
                && node.string_attr("src") == Some("asset://local-id"))
    );
    assert!(
        editor
            .document()
            .blocks()
            .iter()
            .flat_map(|node| node.children())
            .any(|node| node.text.as_deref() == Some("bold"))
    );
    Ok(())
}
#[test]
fn invalid_image_bindings_and_budget_expansion_leave_prepared_data_unchanged() -> TestResult {
    let editor = editor()?;
    let bundle = ClipboardBundle {
        html: Some("<img src=\"https://example.com/a.png\">".into()),
        plain: String::new(),
        ..Default::default()
    };
    let (context, _) = capture(&editor, bundle)?;
    let mut prepared = context.prepare(2048)?;
    let before = prepared.rich_slice().ok_or("missing rich")?.clone();
    let original = prepared.asset_references()[0].clone();
    let binding = ImageBinding {
        original,
        destination: AssetReference {
            asset_id: Some("id".into()),
            source: "asset://id".into(),
        },
    };
    assert!(
        prepared
            .bind_images(&[binding.clone(), binding.clone()])
            .is_err()
    );
    assert_eq!(prepared.rich_slice(), Some(&before));
    let mut unused = binding.clone();
    unused.original.source = "missing".into();
    assert!(prepared.bind_images(&[unused]).is_err());
    assert_eq!(prepared.rich_slice(), Some(&before));
    let mut oversized_source = binding.clone();
    oversized_source.original.source = "x".repeat(2049);
    assert!(prepared.bind_images(&[oversized_source]).is_err());
    assert_eq!(prepared.rich_slice(), Some(&before));
    let mut oversized = binding;
    oversized.destination.source = "x".repeat(2049);
    assert!(prepared.bind_images(&[oversized]).is_err());
    assert_eq!(prepared.rich_slice(), Some(&before));
    Ok(())
}
#[test]
fn opaque_payload_with_url_text_cannot_turn_into_link_and_drop_the_image() -> TestResult {
    let mut editor = editor()?;
    editor.set_selection(range("p", 0, 4))?;
    let document = Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":4},"content":[{"type":"paragraph","attrs":{"blockId":"from"},"content":[{"type":"text","text":"https://example.com"}]},{"type":"imageBlock","attrs":{"blockId":"image","src":"asset://synthetic","assetId":"synthetic","alt":"Alt","caption":"Keep","vendor":null},"opaque":true}]}),
    )?;
    let bundle = RichSlice {
        content: document.blocks().to_vec(),
        open_start: 0,
        open_end: 0,
    }
    .bundle()?;
    editor.paste_bundle(&bundle, LIMIT)?;
    assert!(editor.document().blocks().iter().any(
        |node| node.node_type == "imageBlock" && node.extra.get("opaque") == Some(&json!(true))
    ));
    Ok(())
}
#[test]
fn code_context_uses_literal_text_and_never_reinterprets_private_atoms_as_empty_text() -> TestResult
{
    let source = Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":4},"content":[{"type":"codeBlock","attrs":{"blockId":"code","language":"text"},"content":[{"type":"text","text":"AB"}]}]}),
    )?;
    let mut editor = EditorSession::new(source.clone(), Revision(0));
    editor.set_selection(range("code", 1, 1))?;
    editor.paste_bundle(&plain("**bold** https://example.com"), LIMIT)?;
    assert_eq!(
        editor.document().block_text("code")?,
        "A**bold** https://example.comB"
    );
    assert!(
        editor
            .document()
            .block("code")?
            .children()
            .iter()
            .all(|node| node.marks.as_deref().unwrap_or_default().is_empty())
    );
    editor.undo()?;
    let atom = Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":4},"content":[{"type":"imageBlock","attrs":{"blockId":"image","src":"asset://synthetic","alt":"Alt"}}]}),
    )?;
    let bundle = RichSlice {
        content: atom.blocks().to_vec(),
        open_start: 0,
        open_end: 0,
    }
    .bundle()?;
    editor.paste_bundle(&bundle, LIMIT)?;
    assert!(
        editor
            .document()
            .blocks()
            .iter()
            .any(|node| node.node_type == "imageBlock")
    );
    editor.undo()?;
    assert_eq!(editor.document(), &source);
    Ok(())
}
