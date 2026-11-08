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
        .fold(
            Some(("target".to_owned(), ScalarOffset(3))),
            |point, map| point.and_then(|(id, offset)| map.map_existing_caret(&id, offset, true)),
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
