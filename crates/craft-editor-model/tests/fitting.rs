use craft_editor_model::{
    Document, Editor, Revision, ScalarOffset, TextPoint, Transaction, plan_slice_replacement,
};
use serde_json::{Value, json};
type TestResult = Result<(), Box<dyn std::error::Error>>;
fn point(id: &str, offset: usize) -> TextPoint {
    TextPoint {
        block_id: id.into(),
        offset: ScalarOffset(offset),
    }
}
fn normalize(value: &mut Value) {
    if let Some(object) = value.as_object_mut() {
        if let Some(attrs) = object.get_mut("attrs").and_then(Value::as_object_mut) {
            attrs.remove("blockId");
        }
        if object
            .get("attrs")
            .and_then(Value::as_object)
            .is_some_and(serde_json::Map::is_empty)
        {
            object.remove("attrs");
        }
        if object
            .get("content")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
        {
            object.remove("content");
        }
        if let Some(children) = object.get_mut("content").and_then(Value::as_array_mut) {
            for child in children {
                normalize(child);
            }
        }
    }
}
#[test]
fn generic_fitter_matches_actual_original_replace_selection_cases_and_native_maps() -> TestResult {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/source_slice_fitting.json"))?;
    let cases = fixture["cases"].as_array().ok_or("missing source corpus")?;
    assert_eq!(cases.len(), 22);
    for case in cases {
        let document = Document::from_json(
            json!({"type":"doc","attrs":{"schemaVersion":4},"content":case["target"]}),
        )?;
        let read = |value: &Value| -> Result<TextPoint, Box<dyn std::error::Error>> {
            Ok(point(
                value[0].as_str().ok_or("invalid block")?,
                usize::try_from(value[1].as_u64().ok_or("invalid offset")?)?,
            ))
        };
        let from = read(&case["from"])?;
        let to = read(case.get("to").unwrap_or(&case["from"]))?;
        for reversed in [false, true] {
            let mut next = 0;
            let (command, map) = plan_slice_replacement(
                &document,
                if reversed { to.clone() } else { from.clone() },
                if reversed { from.clone() } else { to.clone() },
                serde_json::from_value(case["slice"].clone())?,
                usize::try_from(case["openStart"].as_u64().ok_or("missing depth")?)?,
                usize::try_from(case["openEnd"].as_u64().ok_or("missing depth")?)?,
                || {
                    next += 1;
                    Ok(format!("fit-{next}"))
                },
            )
            .map_err(|error| format!("{} reversed={reversed} plan: {error}", case["name"]))?;
            let mut editor = Editor::new(document.clone(), Revision(0));
            let change = editor.apply(Transaction {
                base_revision: Revision(0),
                commands: vec![command],
            })?;
            let mut actual = editor.document().to_json()?;
            let mut expected = case["output"].clone();
            normalize(&mut actual);
            normalize(&mut expected);
            assert_eq!(actual, expected, "{} reversed={reversed}", case["name"]);
            assert_eq!(
                serde_json::to_value(&map)?["steps"],
                case["maps"],
                "{}",
                case["name"]
            );
            assert_eq!(
                change.replay_proof().verify(&document)?.0,
                *editor.document()
            );
            let after = editor.document().clone();
            editor.undo(editor.revision())?;
            assert_eq!(editor.document(), &document);
            editor.redo(editor.revision())?;
            assert_eq!(editor.document(), &after);
            if let Some(caret) = map.inserted_caret() {
                let encoded = serde_json::to_value(&map)?;
                let block = encoded["after"]
                    .as_array()
                    .ok_or("missing after coordinates")?
                    .iter()
                    .find(|block| block["id"] == caret.block_id)
                    .ok_or("missing mapped caret block")?;
                let start = block["start"].as_u64().ok_or("missing mapped start")?;
                let units = editor
                    .document()
                    .block(&caret.block_id)?
                    .inline_text()
                    .chars()
                    .take(caret.offset.0)
                    .map(char::len_utf16)
                    .sum::<usize>();
                assert_eq!(
                    start + u64::try_from(units)?,
                    case["selection"]["anchor"]
                        .as_u64()
                        .ok_or("missing expected caret")?,
                    "{} reversed={reversed} caret",
                    case["name"]
                );
            } else {
                assert!(map.inserted_node().is_some());
            }
        }
    }
    Ok(())
}
#[test]
fn unicode_tail_carets_map_to_actual_split_block_without_detaching_unrelated_notes() -> TestResult {
    let document = Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":4},"opaqueRoot":null,"content":[{"type":"paragraph","attrs":{"blockId":"p","vendor":null,"sourceRefs":["voice"]},"content":[{"type":"text","text":"A😀tail","vendorLeaf":null,"marks":[{"type":"bold","vendorMark":null}]}]},{"type":"paragraph","attrs":{"blockId":"other"},"content":[{"type":"text","text":"same tail"}]}]}),
    )?;
    let incoming = serde_json::from_value(
        json!([{ "type":"paragraph","attrs":{"blockId":"first"},"content":[{"type":"text","text":"X"}]},{"type":"paragraph","attrs":{"blockId":"last","sourceRefs":["incoming"]},"content":[{"type":"text","text":"😀"}]}]),
    )?;
    let mut next = 0;
    let (command, map) = plan_slice_replacement(
        &document,
        point("p", 2),
        point("p", 2),
        incoming,
        1,
        1,
        || {
            next += 1;
            Ok(format!("fresh-{next}"))
        },
    )?;
    let mut editor = Editor::new(document.clone(), Revision(0));
    let change = editor.apply(Transaction {
        base_revision: Revision(0),
        commands: vec![command],
    })?;
    let mapped = map
        .map_existing_point(&point("p", 3), true)
        .ok_or("tail note detached")?;
    let block = editor.document().block(&mapped.block_id)?;
    assert!(
        block
            .attrs
            .as_ref()
            .and_then(|attrs| attrs.get("sourceRefs"))
            .and_then(Value::as_array)
            .is_some_and(|refs| refs.contains(&json!("voice")))
    );
    assert_eq!(
        block
            .inline_text()
            .chars()
            .skip(mapped.offset.0)
            .take(3)
            .collect::<String>(),
        "ail"
    );
    assert_eq!(
        map.map_existing_point(&point("other", 5), false),
        Some(point("other", 5))
    );
    assert_eq!(
        change.mappings[0].map_existing_caret("p", ScalarOffset(3), true),
        Some((mapped.block_id, mapped.offset))
    );
    assert_eq!(editor.document().root().extra, document.root().extra);
    assert!(
        editor
            .document()
            .root()
            .children()
            .iter()
            .flat_map(|node| node.children())
            .filter(|node| node
                .text
                .as_deref()
                .is_some_and(|text| text.contains("tail")))
            .all(|node| node.extra.get("vendorLeaf") == Some(&Value::Null)
                || node.text.as_deref() == Some("same tail"))
    );
    Ok(())
}
#[test]
fn copied_opaque_context_conflict_and_invalid_open_depth_preserve_document_and_history()
-> TestResult {
    let document = Document::plain("p", "unchanged")?;
    for (slice, start, end) in [
        (
            json!([{ "type":"blockquote","attrs":{"blockId":"quote","vendor":null},"opaque":true,"content":[{"type":"paragraph","attrs":{"blockId":"incoming"},"content":[{"type":"text","text":"X"}]}]}]),
            2,
            2,
        ),
        (
            json!([{ "type":"imageBlock","attrs":{"blockId":"image","src":"https://example.test/a.png"}}]),
            1,
            1,
        ),
    ] {
        let mut editor = Editor::new(document.clone(), Revision(0));
        let mut next = 0;
        assert!(
            plan_slice_replacement(
                &document,
                point("p", 1),
                point("p", 1),
                serde_json::from_value(slice)?,
                start,
                end,
                || {
                    next += 1;
                    Ok(format!("fresh-{next}"))
                }
            )
            .is_err()
        );
        assert_eq!(editor.document(), &document);
        assert!(!editor.can_undo());
        assert!(editor.undo(editor.revision()).is_err());
    }
    Ok(())
}
