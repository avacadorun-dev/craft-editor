use craft_editor_egui::clipboard::RichSlice;
use craft_editor_egui::{DocPoint, EditorSession, Selection};
use craft_editor_model::{Document, PositionMap, Revision, ScalarOffset};
use serde_json::json;
type TestResult = Result<(), Box<dyn std::error::Error>>;
fn caret(offset: usize) -> Selection {
    Selection {
        anchor: DocPoint {
            block_id: "target".into(),
            offset: ScalarOffset(offset),
        },
        head: DocPoint {
            block_id: "target".into(),
            offset: ScalarOffset(offset),
        },
    }
}
#[test]
fn nested_open_list_paste_uses_one_native_fit_and_restores_original_selection_on_undo() -> TestResult
{
    let mut editor = EditorSession::new(Document::plain("target", "leftright")?, Revision(0));
    editor.set_selection(caret(4))?;
    let original = editor.document().clone();
    let nodes = serde_json::from_value(
        json!([{ "type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","attrs":{"blockId":"a","sourceRefs":["incoming"]},"content":[{"type":"text","text":"A","vendorLeaf":null,"marks":[{"type":"bold","vendorMark":null}]}]}]},{"type":"listItem","content":[{"type":"paragraph","attrs":{"blockId":"b"},"content":[{"type":"text","text":"B"}]}]}]}]),
    )?;
    let bundle = RichSlice {
        content: nodes,
        open_start: 3,
        open_end: 3,
    }
    .bundle()?;
    let change = editor.paste_bundle(&bundle, 1_000_000)?;
    assert!(
        change
            .mappings
            .iter()
            .any(|map| matches!(map, PositionMap::Fitted(_)))
    );
    assert_eq!(editor.document().blocks()[0].inline_text(), "leftA");
    assert_eq!(editor.document().blocks()[1].node_type, "bulletList");
    let after = editor.document().clone();
    let after_selection = editor.selection().cloned();
    assert_eq!(change.replay_proof().verify(&original)?.0, after);
    editor.undo()?;
    assert_eq!(editor.document(), &original);
    assert_eq!(editor.selection(), Some(&caret(4)));
    assert!(!editor.can_undo());
    editor.redo()?;
    assert_eq!(editor.document(), &after);
    assert_eq!(editor.selection(), after_selection.as_ref());
    Ok(())
}
#[test]
fn unequal_open_edges_paste_preserves_source_and_existing_tail_metadata() -> TestResult {
    let original = Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":4},"content":[{"type":"paragraph","attrs":{"blockId":"target","sourceRefs":["voice"],"vendor":null},"content":[{"type":"text","text":"A😀tail","vendorLeaf":true}]}]}),
    )?;
    let mut editor = EditorSession::new(original.clone(), Revision(0));
    editor.set_selection(caret(2))?;
    let nodes = serde_json::from_value(
        json!([{ "type":"paragraph","attrs":{"blockId":"a"},"content":[{"type":"text","text":"X"}]},{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","attrs":{"blockId":"b"},"content":[{"type":"text","text":"Y"}]}]}]}]),
    )?;
    let change = editor.paste_bundle(
        &RichSlice {
            content: nodes,
            open_start: 1,
            open_end: 3,
        }
        .bundle()?,
        1_000_000,
    )?;
    let mapped = change
        .mappings
        .iter()
        .try_fold(
            ("target".to_owned(), ScalarOffset(3)),
            |(id, offset), map| map.map_existing_caret(&id, offset, true),
        )
        .ok_or("tail note detached")?;
    let block = editor.document().block(&mapped.0)?;
    assert_eq!(
        block
            .inline_text()
            .chars()
            .skip(mapped.1.0)
            .take(3)
            .collect::<String>(),
        "ail"
    );
    assert!(block.children().iter().any(|node| {
        node.text
            .as_deref()
            .is_some_and(|text| text.contains("tail"))
            && node.extra.get("vendorLeaf") == Some(&json!(true))
    }));
    assert!(
        block
            .attrs
            .as_ref()
            .and_then(|attrs| attrs.get("sourceRefs"))
            .and_then(serde_json::Value::as_array)
            .is_some_and(|refs| refs.contains(&json!("voice")))
    );
    editor.undo()?;
    assert_eq!(editor.document(), &original);
    assert_eq!(editor.selection(), Some(&caret(2)));
    Ok(())
}
#[test]
fn open_heading_and_code_keep_source_block_style_in_an_empty_document() -> TestResult {
    for (kind, attrs, text) in [
        (
            "heading",
            json!({"blockId":"source","level":5,"sourceRefs":[]}),
            "Title",
        ),
        (
            "codeBlock",
            json!({"blockId":"source","language":"rust"}),
            "one\ntwo",
        ),
    ] {
        let mut editor = EditorSession::new(Document::plain("target", "")?, Revision(0));
        editor.set_selection(caret(0))?;
        let original = editor.document().clone();
        let nodes = serde_json::from_value(
            json!([{ "type":kind,"attrs":attrs,"content":[{"type":"text","text":text}]}]),
        )?;
        editor.paste_bundle(
            &RichSlice {
                content: nodes,
                open_start: 1,
                open_end: 1,
            }
            .bundle()?,
            1_000_000,
        )?;
        assert_eq!(editor.document().blocks().len(), 1);
        assert_eq!(editor.document().blocks()[0].node_type, kind);
        assert_eq!(editor.document().blocks()[0].inline_text(), text);
        let id = editor.document().blocks()[0]
            .block_id()
            .ok_or("missing inserted identity")?;
        assert_eq!(
            editor
                .selection()
                .map(|selection| (&selection.anchor.block_id, selection.anchor.offset.0)),
            Some((&id.to_owned(), text.chars().count()))
        );
        editor.undo()?;
        assert_eq!(editor.document(), &original);
        assert_eq!(editor.selection(), Some(&caret(0)));
    }
    Ok(())
}
#[test]
fn consumed_paragraph_context_merges_nonconflicting_opaque_fields_without_flattening_marks()
-> TestResult {
    let mut editor = EditorSession::new(Document::plain("target", "leftright")?, Revision(0));
    editor.set_selection(caret(4))?;
    let original = editor.document().clone();
    let nodes = serde_json::from_value(
        json!([{ "type":"paragraph","attrs":{"blockId":"source","sourceRefs":["voice"],"vendor":null},"vendorBlock":{"keep":1},"content":[{"type":"text","text":"X","vendorLeaf":null,"marks":[{"type":"bold","vendorMark":true}]}]}]),
    )?;
    editor.paste_bundle(
        &RichSlice {
            content: nodes,
            open_start: 1,
            open_end: 1,
        }
        .bundle()?,
        1_000_000,
    )?;
    let target = editor.document().block("target")?;
    assert_eq!(target.extra.get("vendorBlock"), Some(&json!({"keep":1})));
    assert_eq!(
        target.attrs.as_ref().and_then(|attrs| attrs.get("vendor")),
        Some(&json!(null))
    );
    let inserted = target
        .children()
        .iter()
        .find(|node| node.text.as_deref() == Some("X"))
        .ok_or("missing rich inserted leaf")?;
    assert_eq!(inserted.extra.get("vendorLeaf"), Some(&json!(null)));
    assert_eq!(
        inserted.marks.as_ref().ok_or("missing bold")?[0]
            .extra
            .get("vendorMark"),
        Some(&json!(true))
    );
    editor.undo()?;
    assert_eq!(editor.document(), &original);
    Ok(())
}

#[test]
fn paste_replaces_selected_atoms_with_fitted_context_and_one_exact_selection_history() -> TestResult
{
    use craft_editor_model::NodePath;
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../craft-editor-model/tests/fixtures/source_node_slice_fitting.json"
    ))?;
    for case in fixture["cases"].as_array().ok_or("missing node corpus")? {
        if case["name"] == "selected-table-to-closed-image" {
            continue;
        }
        let original = Document::from_json(
            json!({"type":"doc","attrs":{"schemaVersion":4},"content":case["target"]}),
        )?;
        let mut editor = EditorSession::new(original.clone(), Revision(0));
        let selected =
            original.node_ref(&NodePath(serde_json::from_value(case["nodePath"].clone())?))?;
        editor.select_node(selected.clone())?;
        let slice = RichSlice {
            content: serde_json::from_value(case["slice"].clone())?,
            open_start: serde_json::from_value(case["openStart"].clone())?,
            open_end: serde_json::from_value(case["openEnd"].clone())?,
        };
        let change = editor
            .paste_bundle(&slice.bundle()?, 1_000_000)
            .map_err(|error| format!("{}: {error}", case["name"]))?;
        let after = editor.document().clone();
        let after_selection = editor.selection().cloned();
        assert!(after_selection.is_some());
        assert!(editor.node_selection().is_none());
        assert!(
            change
                .mappings
                .iter()
                .any(|map| matches!(map, PositionMap::Fitted(_)))
        );
        assert_eq!(change.replay_proof().verify(&original)?.0, after);
        editor.undo()?;
        assert_eq!(editor.document(), &original);
        assert_eq!(editor.node_selection(), Some(&selected));
        assert!(!editor.can_undo());
        editor.redo()?;
        assert_eq!(editor.document(), &after);
        assert_eq!(editor.selection(), after_selection.as_ref());
    }
    Ok(())
}
