#![expect(
    clippy::expect_used,
    reason = "Synthetic canonical fixtures must construct successfully"
)]
use craft_editor_model::{Document, Editor, Node, NodePath, Revision, ScalarOffset, Transaction};
use serde_json::{Map, Value, json};

fn apply(editor: &mut Editor, command: craft_editor_model::Command) -> craft_editor_model::Change {
    editor
        .apply(Transaction {
            base_revision: editor.revision(),
            commands: vec![command],
        })
        .expect("command")
}
#[test]
fn guarded_root_replacement_promotes_schema_without_losing_unknown_fields() {
    let mut document = Document::plain("p", "Текст 😀").expect("plain");
    let mut root = document.root().clone();
    root.extra.insert("vendor".into(), json!({"keep":null}));
    document = Document::from_root(root).expect("metadata");
    let target = document.node_ref(&NodePath::root()).expect("root");
    let mut replacement = target.expected.as_ref().clone();
    replacement.content.as_mut().expect("blocks").push(serde_json::from_value(json!({"type":"mathBlock","attrs":{"blockId":"math","latex":"x^2","description":"square"}})).expect("formula"));
    let command = document
        .replace_node(&target, replacement)
        .expect("validated builder");
    let mut editor = Editor::new(document.clone(), Revision(1));
    let change = apply(&mut editor, command);
    let json = editor.document().to_json().expect("json");
    assert_eq!(json["attrs"]["schemaVersion"], json!(4));
    assert_eq!(json["vendor"], json!({"keep":null}));
    assert_eq!(
        change.mappings[0].map_existing_caret("p", ScalarOffset(2), true),
        Some(("p".into(), ScalarOffset(2)))
    );
    editor.undo(editor.revision()).expect("undo");
    assert_eq!(editor.document(), &document);
}
#[test]
fn patching_anonymous_inline_atom_attributes_preserves_source_and_extension_fields() {
    let input = json!({"type":"doc","attrs":{"schemaVersion":4},"content":[{"type":"paragraph","attrs":{"blockId":"p"},"content":[{"type":"inlineMath","attrs":{"latex":"x","description":"old","vendor":null},"extra":{"keep":true}}]}]});
    let document = Document::from_json(input.clone()).expect("fixture");
    let target = document.node_ref(&NodePath(vec![0, 0])).expect("atom");
    let command = document
        .patch_node_attrs(
            &target,
            Map::from_iter([("description".into(), json!("new"))]),
        )
        .expect("patch");
    let mut editor = Editor::new(document, Revision(1));
    apply(&mut editor, command);
    let after = editor.document().to_json().expect("json");
    assert_eq!(
        after["content"][0]["content"][0]["attrs"]["latex"],
        json!("x")
    );
    assert_eq!(
        after["content"][0]["content"][0]["attrs"]["vendor"],
        Value::Null
    );
    assert_eq!(
        after["content"][0]["content"][0]["extra"],
        json!({"keep":true})
    );
}
#[test]
fn rich_inline_slice_preserves_unselected_runs_and_inserts_real_math_and_breaks() {
    let input = json!({"type":"doc","content":[{"type":"paragraph","attrs":{"blockId":"p","sourceRefs":["source"]},"content":[{"type":"text","text":"А😀Б","marks":[{"type":"bold"}],"vendor":null}]}]});
    let document = Document::from_json(input.clone()).expect("fixture");
    let target = document.node_ref(&NodePath(vec![0])).expect("paragraph");
    let children = vec![
        Node::text("е́"),
        serde_json::from_value(json!({"type":"hardBreak"})).expect("break"),
        serde_json::from_value(
            json!({"type":"inlineMath","attrs":{"latex":"x^2","description":"square"}}),
        )
        .expect("math"),
    ];
    let command = document
        .replace_inline(&target, ScalarOffset(1), ScalarOffset(2), children)
        .expect("slice");
    let mut editor = Editor::new(document.clone(), Revision(1));
    let change = apply(&mut editor, command);
    assert_eq!(
        editor.document().block_text("p").expect("text"),
        "Ае́\n\u{fffc}Б"
    );
    let json = editor.document().to_json().expect("json");
    assert_eq!(json["attrs"]["schemaVersion"], json!(4));
    assert_eq!(json["content"][0]["attrs"], input["content"][0]["attrs"]);
    assert_eq!(
        json["content"][0]["content"][0]["marks"],
        json!([{"type":"bold"}])
    );
    assert_eq!(
        json["content"][0]["content"][4]["marks"],
        json!([{"type":"bold"}])
    );
    assert_eq!(
        change.mappings[0].map_existing_caret("p", ScalarOffset(3), true),
        Some(("p".into(), ScalarOffset(6)))
    );
    editor.undo(editor.revision()).expect("undo");
    assert_eq!(editor.document(), &document);
}
#[test]
fn stale_reference_or_invalid_replacement_never_changes_current_state() {
    let document = Document::plain("p", "A").expect("fixture");
    let target = document.node_ref(&NodePath(vec![0])).expect("paragraph");
    let mut invalid = target.expected.as_ref().clone();
    invalid.node_type = "futureBlock".into();
    assert!(document.replace_node(&target, invalid).is_err());
    let command = document
        .patch_node_attrs(&target, Map::from_iter([("changed".into(), json!(true))]))
        .expect("patch");
    let mut editor = Editor::new(document, Revision(1));
    apply(&mut editor, command);
    assert!(
        editor
            .document()
            .patch_node_attrs(&target, Map::new())
            .is_err()
    );
    let fresh = editor
        .document()
        .node_ref(&NodePath(vec![0]))
        .expect("fresh");
    assert!(
        editor
            .document()
            .replace_inline(&fresh, ScalarOffset(0), ScalarOffset(99), vec![])
            .is_err()
    );
    assert_eq!(editor.document().block_text("p").expect("text"), "A");
}
#[test]
fn legacy_integral_number_spelling_null_provenance_and_empty_math_source_roundtrip() {
    let input = json!({"type":"doc","attrs":{"schemaVersion":4.0},"content":[{"type":"heading","attrs":{"blockId":"h","level":6.0,"sourceRefs":null},"content":[{"type":"text","text":"Title"}]},{"type":"mathBlock","attrs":{"blockId":"math","latex":"","description":"invalid-source fallback stays editable"}}]});
    assert_eq!(
        Document::from_json(input.clone())
            .expect("legacy nullable/numeric forms")
            .to_json()
            .expect("json"),
        input
    );
}

#[test]
fn serialized_reference_still_matches_after_patching_an_explicit_null_attribute_field() {
    let document = Document::from_json(json!({"type":"doc","attrs":null,"content":[{"type":"paragraph","attrs":{"blockId":"p"},"content":[{"type":"text","text":"Keep"}]}]})).expect("nullable root");
    let target = document.node_ref(&NodePath::root()).expect("root");
    let command = document
        .patch_node_attrs(&target, Map::from_iter([("vendor".into(), json!(true))]))
        .expect("patch null attrs");
    let mut editor = Editor::new(document, Revision(1));
    apply(&mut editor, command);
    let target = editor
        .document()
        .node_ref(&NodePath::root())
        .expect("updated root");
    let wire_target =
        serde_json::from_value(serde_json::to_value(&target).expect("reference JSON"))
            .expect("reference roundtrip");
    let command = editor
        .document()
        .patch_node_attrs(
            &wire_target,
            Map::from_iter([("second".into(), json!(true))]),
        )
        .expect("wire-equivalent guard");
    apply(&mut editor, command);
    let roundtrip = Document::from_json(editor.document().to_json().expect("JSON"))
        .expect("document roundtrip");
    assert_eq!(editor.document(), &roundtrip);
}

#[test]
fn replacing_nullable_mark_attributes_uses_effective_wire_equality() {
    let mut mark: craft_editor_model::Mark =
        serde_json::from_value(json!({"type":"bold","attrs":null})).expect("nullable mark");
    mark.attrs = Some(Map::from_iter([("vendor".into(), json!("value"))]));
    let roundtrip = serde_json::from_value::<craft_editor_model::Mark>(
        serde_json::to_value(&mark).expect("mark JSON"),
    )
    .expect("mark roundtrip");
    assert_eq!(mark, roundtrip);
}

#[test]
fn joining_paragraphs_refuses_different_null_envelopes_without_mutation() {
    let document = Document::from_json(json!({"type":"doc","content":[{"type":"paragraph","attrs":{"blockId":"a"},"text":null,"content":[{"type":"text","text":"A"}]},{"type":"paragraph","attrs":{"blockId":"b"},"content":[{"type":"text","text":"B"}]}]})).expect("nullable metadata");
    let mut editor = Editor::new(document.clone(), Revision(1));
    assert!(
        editor
            .apply(Transaction {
                base_revision: Revision(1),
                commands: vec![craft_editor_model::Command::JoinBlocks {
                    left_id: "a".into(),
                    right_id: "b".into()
                }]
            })
            .is_err()
    );
    assert_eq!(editor.document(), &document);
}
