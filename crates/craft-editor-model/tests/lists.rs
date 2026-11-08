#![expect(
    clippy::expect_used,
    reason = "Synthetic fixtures must be valid before testing list behavior"
)]

use craft_editor_model::{
    Command, Document, Editor, Error, ListKind, ListStyle, NodePath, Revision, ScalarOffset,
    TextRange, Transaction, exit_empty_list_item, indent_list_items, join_adjacent_lists,
    lift_list_items, lift_list_items_with_ids, outdent_list_items, outdent_list_items_with_ids,
    split_list_item, toggle_list, toggle_list_with_ids, wrap_list,
};
use serde_json::{Value, json};

fn paragraph(id: &str, text: &str) -> Value {
    json!({"type":"paragraph","attrs":{"blockId":id,"sourceRefs":["voice"]},"content": if text.is_empty() {vec![]} else {vec![json!({"type":"text","text":text,"marks":[{"type":"bold"}],"vendorLeaf":null})]}})
}
fn item(id: &str, text: &str) -> Value {
    json!({"type":"listItem","content":[paragraph(id,text)]})
}
fn document(value: Value) -> Document {
    Document::from_json(value).expect("list fixture")
}
fn simple_list() -> Document {
    document(
        json!({"type":"doc","content":[{"type":"orderedList","attrs":{"start":5,"type":"a"},"content":[item("a","A"),item("b","B"),item("c","C")]}]}),
    )
}
fn editor(document: Document) -> Editor {
    Editor::new(document, Revision(1))
}
fn apply(
    editor: &mut Editor,
    command: Command,
) -> craft_editor_model::Result<craft_editor_model::Change> {
    editor.apply(Transaction {
        base_revision: editor.revision(),
        commands: vec![command],
    })
}
fn value(editor: &Editor) -> Value {
    editor.document().to_json().expect("serialize")
}

#[test]
fn wrap_preserves_complete_paragraphs_marks_unknown_data_and_source_refs() {
    let input = json!({"type":"doc","attrs":{"vendor":null},"content":[paragraph("a","А😀"),paragraph("b","Б")]});
    let original = document(input.clone());
    let command = wrap_list(
        &original,
        original.node_ref(&NodePath::root()).expect("root"),
        0,
        2,
        ListStyle::new(ListKind::Bullet),
        &[],
    )
    .expect("wrap");
    let mut editor = editor(original.clone());
    apply(&mut editor, command).expect("apply");
    let after = value(&editor);
    assert_eq!(after["content"][0]["type"], json!("bulletList"));
    assert_eq!(
        after["content"][0]["content"][0]["content"][0],
        input["content"][0]
    );
    assert_eq!(
        after["content"][0]["content"][1]["content"][0],
        input["content"][1]
    );
    editor.undo(Revision(2)).expect("undo");
    assert_eq!(editor.document(), &original);
}
#[test]
fn nonparagraph_wrap_requires_explicit_leading_identity_and_keeps_original_block() {
    let input = json!({"type":"doc","content":[{"type":"heading","attrs":{"level":2,"blockId":"h","sourceRefs":["voice"],"vendor":true},"content":[{"type":"text","text":"Heading"}]}]});
    let original = document(input.clone());
    let parent = original.node_ref(&NodePath::root()).expect("root");
    assert!(
        wrap_list(
            &original,
            parent.clone(),
            0,
            1,
            ListStyle::new(ListKind::Bullet),
            &[]
        )
        .is_err()
    );
    let command = wrap_list(
        &original,
        parent,
        0,
        1,
        ListStyle::new(ListKind::Bullet),
        &["leading".into()],
    )
    .expect("wrap");
    let mut editor = editor(original);
    apply(&mut editor, command).expect("apply");
    let after = value(&editor);
    assert_eq!(
        after["content"][0]["content"][0]["content"][0]["attrs"]["blockId"],
        json!("leading")
    );
    assert_eq!(
        after["content"][0]["content"][0]["content"][1],
        input["content"][0]
    );
}
#[test]
fn converting_list_kind_keeps_wrapper_metadata_and_ordered_attributes() {
    let mut input = simple_list().to_json().expect("fixture");
    input["content"][0]["vendorList"] = json!({"keep":null});
    let original = document(input.clone());
    let command = toggle_list(
        &original,
        original.node_ref(&NodePath(vec![0])).expect("list"),
        0,
        3,
        ListStyle::new(ListKind::Bullet),
    )
    .expect("convert");
    let mut editor = editor(original);
    apply(&mut editor, command).expect("apply");
    let after = value(&editor);
    assert_eq!(after["content"][0]["type"], json!("bulletList"));
    assert_eq!(after["content"][0]["attrs"], input["content"][0]["attrs"]);
    assert_eq!(
        after["content"][0]["vendorList"],
        input["content"][0]["vendorList"]
    );
}
#[test]
fn partial_kind_conversion_preserves_numbering_and_all_three_portions() {
    let original = simple_list();
    let command = toggle_list(
        &original,
        original.node_ref(&NodePath(vec![0])).expect("list"),
        1,
        2,
        ListStyle::new(ListKind::Bullet),
    )
    .expect("partial conversion");
    let mut editor = editor(original);
    apply(&mut editor, command).expect("apply");
    let after = value(&editor);
    assert_eq!(after["content"].as_array().expect("blocks").len(), 3);
    assert_eq!(after["content"][0]["attrs"]["start"], json!(5));
    assert_eq!(after["content"][1]["type"], json!("bulletList"));
    assert_eq!(after["content"][1]["attrs"]["start"], json!(6));
    assert_eq!(after["content"][2]["attrs"]["start"], json!(7));
}
#[test]
fn lifting_middle_items_preserves_remaining_wrapper_metadata_and_continuation_start() {
    let mut input = simple_list().to_json().expect("fixture");
    input["content"][0]["vendorList"] = json!("retain");
    let original = document(input.clone());
    let command = lift_list_items(
        &original,
        original.node_ref(&NodePath(vec![0])).expect("list"),
        1,
        2,
    )
    .expect("lift middle");
    let mut editor = editor(original);
    apply(&mut editor, command).expect("apply");
    let after = value(&editor);
    assert_eq!(
        after["content"][1],
        input["content"][0]["content"][1]["content"][0]
    );
    assert_eq!(after["content"][0]["vendorList"], json!("retain"));
    assert_eq!(after["content"][2]["vendorList"], json!("retain"));
    assert_eq!(after["content"][2]["attrs"]["start"], json!(7));
}
#[test]
fn unwrap_refuses_unknown_wrapper_metadata_or_duplicate_split_identity() {
    for field in ["vendorList", "blockId"] {
        let mut input = simple_list().to_json().expect("fixture");
        if field == "blockId" {
            input["content"][0]["attrs"]["blockId"] = json!("list-id");
        } else {
            input["content"][0][field] = json!("must remain");
        }
        let original = document(input);
        let reference = original.node_ref(&NodePath(vec![0])).expect("list");
        let result = if field == "blockId" {
            lift_list_items(&original, reference, 1, 2)
        } else {
            lift_list_items(&original, reference, 0, 3)
        };
        assert!(matches!(result, Err(Error::UnsupportedEdit)));
    }
}
#[test]
fn unwrap_never_discards_list_item_metadata() {
    let mut input = simple_list().to_json().expect("fixture");
    input["content"][0]["content"][0]["vendorItem"] = json!({"keep":true});
    let original = document(input);
    assert!(matches!(
        lift_list_items(
            &original,
            original.node_ref(&NodePath(vec![0])).expect("list"),
            0,
            1
        ),
        Err(Error::UnsupportedEdit)
    ));
}
#[test]
fn indent_creates_nested_list_and_keeps_all_item_content_and_identity() {
    let original = simple_list();
    let before = original.to_json().expect("before");
    let command = indent_list_items(
        &original,
        original.node_ref(&NodePath(vec![0])).expect("list"),
        1,
        3,
    )
    .expect("indent");
    let mut editor = editor(original);
    apply(&mut editor, command).expect("apply");
    let after = value(&editor);
    let nested = &after["content"][0]["content"][0]["content"][1];
    assert_eq!(nested["type"], json!("orderedList"));
    assert_eq!(nested["content"][0], before["content"][0]["content"][1]);
    assert_eq!(nested["content"][1], before["content"][0]["content"][2]);
}
#[test]
fn indent_extends_existing_nested_list_without_replacing_its_metadata() {
    let mut input = simple_list().to_json().expect("fixture");
    input["content"][0]["content"][0]["content"].as_array_mut().expect("item blocks").push(json!({"type":"orderedList","attrs":{"start":9,"vendor":"nested"},"content":[item("nested","N")]}));
    let original = document(input);
    let command = indent_list_items(
        &original,
        original.node_ref(&NodePath(vec![0])).expect("list"),
        1,
        2,
    )
    .expect("indent");
    let mut editor = editor(original);
    apply(&mut editor, command).expect("apply");
    let after = value(&editor);
    let nested = &after["content"][0]["content"][0]["content"][1];
    assert_eq!(nested["attrs"], json!({"start":9,"vendor":"nested"}));
    assert_eq!(nested["content"].as_array().expect("items").len(), 2);
}
#[test]
fn first_item_indent_is_rejected_without_inventing_content() {
    let original = simple_list();
    assert!(matches!(
        indent_list_items(
            &original,
            original.node_ref(&NodePath(vec![0])).expect("list"),
            0,
            1
        ),
        Err(Error::UnsupportedEdit)
    ));
}
#[test]
fn nested_outdent_moves_trailing_inner_items_below_last_lifted_item() {
    let input = json!({"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","vendorParent":null,"content":[paragraph("parent","Parent"),{"type":"orderedList","attrs":{"start":4,"vendor":"inner"},"content":[item("a","A"),item("b","B"),item("c","C")]},paragraph("tail","Tail")]}]}]});
    let original = document(input.clone());
    let command = outdent_list_items(
        &original,
        original
            .node_ref(&NodePath(vec![0, 0, 1]))
            .expect("nested list"),
        1,
        2,
    )
    .expect("outdent");
    let mut editor = editor(original);
    apply(&mut editor, command).expect("apply");
    let after = value(&editor);
    let parent = &after["content"][0]["content"][0];
    let lifted = &after["content"][0]["content"][1];
    assert_eq!(parent["vendorParent"], Value::Null);
    assert_eq!(
        parent["content"][1]["content"][0],
        input["content"][0]["content"][0]["content"][1]["content"][0]
    );
    assert_eq!(lifted["content"][0]["attrs"]["blockId"], json!("b"));
    assert_eq!(
        lifted["content"][1]["attrs"],
        json!({"start":6,"vendor":"inner"})
    );
    assert_eq!(
        lifted["content"][2],
        input["content"][0]["content"][0]["content"][2]
    );
}
#[test]
fn split_preserves_unicode_marks_unknown_fields_provenance_and_trailing_blocks() {
    let input = json!({"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","vendorItem":{"keep":true},"content":[paragraph("p","А😀Б"),paragraph("tail","Tail")]}]}]});
    let original = document(input.clone());
    let command = split_list_item(
        &original,
        original.node_ref(&NodePath(vec![0])).expect("list"),
        0,
        0,
        ScalarOffset(2),
        "right",
        None,
    )
    .expect("split");
    let mut editor = editor(original.clone());
    apply(&mut editor, command).expect("apply");
    assert_eq!(editor.document().block_text("p").expect("left"), "А😀");
    assert_eq!(editor.document().block_text("right").expect("right"), "Б");
    let after = value(&editor);
    let right = &after["content"][0]["content"][1];
    assert_eq!(right["vendorItem"], json!({"keep":true}));
    assert_eq!(right["content"][0]["attrs"]["sourceRefs"], json!(["voice"]));
    assert_eq!(
        right["content"][0]["content"][0]["marks"],
        input["content"][0]["content"][0]["content"][0]["content"][0]["marks"]
    );
    assert_eq!(
        right["content"][1],
        input["content"][0]["content"][0]["content"][1]
    );
    editor.undo(Revision(2)).expect("undo");
    assert_eq!(editor.document(), &original);
}
#[test]
fn splitting_identified_item_requires_new_independent_item_identity() {
    let mut input = simple_list().to_json().expect("fixture");
    input["content"][0]["content"][0]["attrs"] = json!({"blockId":"item-a","vendor":"keep"});
    let original = document(input);
    let reference = original.node_ref(&NodePath(vec![0])).expect("list");
    assert!(
        split_list_item(
            &original,
            reference.clone(),
            0,
            0,
            ScalarOffset(1),
            "new-p",
            None
        )
        .is_err()
    );
    let command = split_list_item(
        &original,
        reference,
        0,
        0,
        ScalarOffset(1),
        "new-p",
        Some("new-item"),
    )
    .expect("fresh item ID");
    let mut editor = editor(original);
    apply(&mut editor, command).expect("apply");
    assert_eq!(
        editor
            .document()
            .block("new-item")
            .expect("new item")
            .string_attr("vendor"),
        Some("keep")
    );
}
#[test]
fn enter_on_sole_empty_item_lifts_existing_paragraph_without_losing_its_identity() {
    let input = json!({"type":"doc","content":[{"type":"orderedList","attrs":{"start":3,"type":"a"},"content":[item("empty","")]}]});
    let original = document(input.clone());
    let command = split_list_item(
        &original,
        original.node_ref(&NodePath(vec![0])).expect("list"),
        0,
        0,
        ScalarOffset(0),
        "unused",
        None,
    )
    .expect("exit");
    let mut editor = editor(original);
    apply(&mut editor, command).expect("apply");
    assert_eq!(
        value(&editor)["content"][0],
        input["content"][0]["content"][0]["content"][0]
    );
}
#[test]
fn empty_exit_refuses_hidden_nested_content_and_stale_reference_is_not_retargeted() {
    let mut input = simple_list().to_json().expect("fixture");
    input["content"][0]["content"][0]["content"] =
        json!([paragraph("a",""),{"type":"bulletList","content":[item("nested","keep")]}]);
    let original = document(input);
    assert!(matches!(
        exit_empty_list_item(
            &original,
            original.node_ref(&NodePath(vec![0])).expect("list"),
            0
        ),
        Err(Error::UnsupportedEdit)
    ));
    let old_ref = original.node_ref(&NodePath(vec![0])).expect("list");
    let mut editor = editor(original);
    apply(
        &mut editor,
        Command::ReplaceText {
            range: TextRange {
                block_id: "b".into(),
                start: ScalarOffset(0),
                end: ScalarOffset(1),
            },
            expected: "B".into(),
            replacement: "changed".into(),
        },
    )
    .expect("edit");
    assert!(matches!(
        indent_list_items(editor.document(), old_ref, 1, 2),
        Err(Error::NodeReferenceConflict)
    ));
}
#[test]
fn negative_ordered_start_remains_valid_and_continuation_uses_checked_arithmetic() {
    let mut input = simple_list().to_json().expect("fixture");
    input["content"][0]["attrs"]["start"] = json!(-3);
    let original = document(input);
    let command = lift_list_items(
        &original,
        original.node_ref(&NodePath(vec![0])).expect("list"),
        1,
        2,
    )
    .expect("lift");
    let mut editor = editor(original);
    apply(&mut editor, command).expect("apply");
    assert_eq!(value(&editor)["content"][2]["attrs"]["start"], json!(-1));
}

fn list(kind: &str, items: Vec<Value>) -> Value {
    json!({"type":kind,"content":items})
}

#[test]
fn wrapping_between_compatible_bullet_lists_joins_both_sides_with_exact_items() {
    let input = json!({"type":"doc","content":[list("bulletList",vec![item("a","A")]),paragraph("b","Б😀"),list("bulletList",vec![item("c","C")])]});
    let original = document(input.clone());
    let command = wrap_list(
        &original,
        original.node_ref(&NodePath::root()).expect("root"),
        1,
        2,
        ListStyle::new(ListKind::Bullet),
        &[],
    )
    .expect("wrap and join");
    let mut editor = editor(original.clone());
    apply(&mut editor, command).expect("apply");
    let after = value(&editor);
    assert_eq!(after["content"].as_array().expect("blocks").len(), 1);
    let items = after["content"][0]["content"].as_array().expect("items");
    assert_eq!(items.len(), 3);
    assert_eq!(items[0], input["content"][0]["content"][0]);
    assert_eq!(items[1]["content"][0], input["content"][1]);
    assert_eq!(items[2], input["content"][2]["content"][0]);
    editor.undo(Revision(2)).expect("undo");
    assert_eq!(editor.document(), &original);
}

#[test]
fn ordered_auto_join_preserves_all_existing_item_numbers() {
    let input = json!({"type":"doc","content":[{"type":"orderedList","attrs":{"start":5,"type":"a","vendor":null},"content":[item("a","A"),item("b","B")]},paragraph("c","C"),{"type":"orderedList","attrs":{"start":8,"type":"a","vendor":null},"content":[item("d","D")]}]});
    let original = document(input.clone());
    let style = ListStyle {
        kind: ListKind::Ordered,
        attrs: serde_json::from_value(json!({"start":7,"type":"a","vendor":null})).expect("attrs"),
    };
    let command = wrap_list(
        &original,
        original.node_ref(&NodePath::root()).expect("root"),
        1,
        2,
        style,
        &[],
    )
    .expect("wrap and join continuations");
    let mut editor = editor(original);
    apply(&mut editor, command).expect("apply");
    let after = value(&editor);
    assert_eq!(after["content"].as_array().expect("blocks").len(), 1);
    assert_eq!(after["content"][0]["attrs"], input["content"][0]["attrs"]);
    let items = after["content"][0]["content"].as_array().expect("items");
    assert_eq!(items.len(), 4);
    assert_eq!(items[0], input["content"][0]["content"][0]);
    assert_eq!(items[1], input["content"][0]["content"][1]);
    assert_eq!(items[2]["content"][0], input["content"][1]);
    assert_eq!(items[3], input["content"][2]["content"][0]);
}

#[test]
fn explicit_join_requires_numbering_continuation_and_preserves_metadata() {
    let input = json!({"type":"doc","content":[{"type":"orderedList","attrs":{"start":-3,"type":"i","vendor":{"keep":null}},"vendorEnvelope":true,"content":[item("a","A")]},{"type":"orderedList","attrs":{"start":-2,"type":"i","vendor":{"keep":null}},"vendorEnvelope":true,"content":[item("b","B")]}]});
    let original = document(input.clone());
    let command = join_adjacent_lists(
        &original,
        original.node_ref(&NodePath(vec![0])).expect("left"),
        original.node_ref(&NodePath(vec![1])).expect("right"),
    )
    .expect("join");
    let mut editor = editor(original);
    apply(&mut editor, command).expect("apply");
    let after = value(&editor);
    assert_eq!(after["content"].as_array().expect("blocks").len(), 1);
    assert_eq!(after["content"][0]["attrs"], input["content"][0]["attrs"]);
    assert_eq!(after["content"][0]["vendorEnvelope"], json!(true));
    assert_eq!(
        after["content"][0]["content"][1],
        input["content"][1]["content"][0]
    );
}

#[test]
fn incompatible_or_identified_wrappers_are_not_merged() {
    let base = json!({"type":"doc","content":[{"type":"orderedList","attrs":{"start":1,"type":"1"},"content":[item("a","A")]},{"type":"orderedList","attrs":{"start":2,"type":"1"},"content":[item("b","B")]}]});
    for variant in 0..6 {
        let mut input = base.clone();
        match variant {
            0 => input["content"][1]["attrs"]["start"] = json!(1),
            1 => input["content"][1]["attrs"]["type"] = json!("a"),
            2 => input["content"][1]["attrs"]["vendor"] = Value::Null,
            3 => input["content"][1]["vendorEnvelope"] = Value::Null,
            4 => input["content"][0]["attrs"]["blockId"] = json!("left-list"),
            _ => input["content"][1]["attrs"]["blockId"] = json!("right-list"),
        }
        let original = document(input);
        assert!(matches!(
            join_adjacent_lists(
                &original,
                original.node_ref(&NodePath(vec![0])).expect("left"),
                original.node_ref(&NodePath(vec![1])).expect("right")
            ),
            Err(Error::UnsupportedEdit)
        ));
    }
}

#[test]
fn ordered_join_overflow_is_refused_without_arithmetic_panic() {
    let original = document(
        json!({"type":"doc","content":[{"type":"orderedList","attrs":{"start":i64::MAX},"content":[item("a","A")]},{"type":"orderedList","attrs":{"start":1},"content":[item("b","B")]}]}),
    );
    assert!(matches!(
        join_adjacent_lists(
            &original,
            original.node_ref(&NodePath(vec![0])).expect("left"),
            original.node_ref(&NodePath(vec![1])).expect("right")
        ),
        Err(Error::UnsupportedEdit)
    ));
}

#[test]
fn wrap_keeps_incompatible_neighbors_instead_of_dropping_wrapper_metadata() {
    let input = json!({"type":"doc","content":[{"type":"bulletList","attrs":{"blockId":"left-list"},"content":[item("a","A")]},paragraph("b","B"),{"type":"bulletList","vendorEnvelope":null,"content":[item("c","C")]}]});
    let original = document(input.clone());
    let command = wrap_list(
        &original,
        original.node_ref(&NodePath::root()).expect("root"),
        1,
        2,
        ListStyle::new(ListKind::Bullet),
        &[],
    )
    .expect("wrap");
    let mut editor = editor(original);
    apply(&mut editor, command).expect("apply");
    let after = value(&editor);
    assert_eq!(after["content"].as_array().expect("blocks").len(), 3);
    assert_eq!(after["content"][0], input["content"][0]);
    assert_eq!(after["content"][2], input["content"][2]);
}

#[test]
fn kind_toggle_joins_compatible_neighbors_but_retains_all_exact_item_nodes() {
    let input = json!({"type":"doc","content":[list("bulletList",vec![item("a","A")]),list("orderedList",vec![item("b","B")]),list("bulletList",vec![item("c","C")])]});
    let original = document(input.clone());
    let command = toggle_list(
        &original,
        original.node_ref(&NodePath(vec![1])).expect("middle"),
        0,
        1,
        ListStyle::new(ListKind::Bullet),
    )
    .expect("toggle and join");
    let mut editor = editor(original);
    apply(&mut editor, command).expect("apply");
    let after = value(&editor);
    assert_eq!(after["content"].as_array().expect("blocks").len(), 1);
    for index in 0..3 {
        assert_eq!(
            after["content"][0]["content"][index],
            input["content"][index]["content"][0]
        );
    }
}

#[test]
fn identified_partial_toggle_allocates_fresh_wrapper_ids_in_document_order() {
    let mut input = simple_list().to_json().expect("fixture");
    input["content"][0]["attrs"]["blockId"] = json!("old-list");
    input["content"][0]["attrs"]["sourceRefs"] = json!(["voice"]);
    input["content"][0]["vendorEnvelope"] = Value::Null;
    let original = document(input.clone());
    let command = toggle_list_with_ids(
        &original,
        original.node_ref(&NodePath(vec![0])).expect("list"),
        1,
        2,
        ListStyle::new(ListKind::Bullet),
        &["new-middle".into(), "new-last".into()],
    )
    .expect("toggle with IDs");
    let mut editor = editor(original.clone());
    apply(&mut editor, command).expect("apply");
    let after = value(&editor);
    for (index, id) in ["old-list", "new-middle", "new-last"].iter().enumerate() {
        assert_eq!(after["content"][index]["attrs"]["blockId"], json!(id));
        assert_eq!(
            after["content"][index]["attrs"]["sourceRefs"],
            json!(["voice"])
        );
        assert!(
            after["content"][index]
                .as_object()
                .expect("list")
                .contains_key("vendorEnvelope")
        );
        assert_eq!(
            after["content"][index]["content"][0],
            input["content"][0]["content"][index]
        );
    }
    editor.undo(Revision(2)).expect("undo");
    assert_eq!(editor.document(), &original);
}

#[test]
fn identified_split_rejects_missing_duplicate_and_unused_wrapper_ids_atomically() {
    let mut input = simple_list().to_json().expect("fixture");
    input["content"][0]["attrs"]["blockId"] = json!("old-list");
    let original = document(input.clone());
    for ids in [
        vec![],
        vec!["only-one".into()],
        vec!["same".into(), "same".into()],
        vec!["old-list".into(), "fresh".into()],
        vec!["a".into(), "fresh".into()],
        vec!["one".into(), "two".into(), "unused".into()],
    ] {
        assert!(
            toggle_list_with_ids(
                &original,
                original.node_ref(&NodePath(vec![0])).expect("list"),
                1,
                2,
                ListStyle::new(ListKind::Bullet),
                &ids
            )
            .is_err()
        );
        assert_eq!(original.to_json().expect("unchanged"), input);
    }
}

#[test]
fn identified_partial_lift_keeps_first_wrapper_id_and_assigns_only_fresh_remainder() {
    let mut input = simple_list().to_json().expect("fixture");
    input["content"][0]["attrs"]["blockId"] = json!("old-list");
    input["content"][0]["vendorEnvelope"] = json!({"retained":null});
    let original = document(input.clone());
    let command = lift_list_items_with_ids(
        &original,
        original.node_ref(&NodePath(vec![0])).expect("list"),
        1,
        2,
        &["remainder-list".into()],
    )
    .expect("lift with ID");
    let mut editor = editor(original);
    apply(&mut editor, command).expect("apply");
    let after = value(&editor);
    assert_eq!(after["content"][0]["attrs"]["blockId"], json!("old-list"));
    assert_eq!(
        after["content"][2]["attrs"]["blockId"],
        json!("remainder-list")
    );
    assert_eq!(
        after["content"][0]["vendorEnvelope"],
        input["content"][0]["vendorEnvelope"]
    );
    assert_eq!(
        after["content"][2]["vendorEnvelope"],
        input["content"][0]["vendorEnvelope"]
    );
    assert_eq!(
        after["content"][1],
        input["content"][0]["content"][1]["content"][0]
    );
    assert_eq!(after["content"][2]["attrs"]["start"], json!(7));
}

#[test]
fn identified_nested_outdent_splits_wrappers_without_duplicating_identity() {
    let input = json!({"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","vendorParent":null,"content":[paragraph("parent","P"),{"type":"orderedList","attrs":{"blockId":"inner-list","start":4,"sourceRefs":["voice"]},"vendorEnvelope":null,"content":[item("a","A"),item("b","B"),item("c","C")]},paragraph("tail","Tail")]}]}]});
    let original = document(input.clone());
    let command = outdent_list_items_with_ids(
        &original,
        original.node_ref(&NodePath(vec![0, 0, 1])).expect("nested"),
        1,
        2,
        &["inner-tail".into()],
    )
    .expect("outdent with ID");
    let mut editor = editor(original);
    apply(&mut editor, command).expect("apply");
    let after = value(&editor);
    let retained = &after["content"][0]["content"][0]["content"][1];
    let trailing = &after["content"][0]["content"][1]["content"][1];
    assert_eq!(retained["attrs"]["blockId"], json!("inner-list"));
    assert_eq!(trailing["attrs"]["blockId"], json!("inner-tail"));
    assert_eq!(trailing["attrs"]["start"], json!(6));
    assert_eq!(retained["attrs"]["sourceRefs"], json!(["voice"]));
    assert_eq!(trailing["attrs"]["sourceRefs"], json!(["voice"]));
    assert_eq!(
        retained["content"][0],
        input["content"][0]["content"][0]["content"][1]["content"][0]
    );
    assert_eq!(
        trailing["content"][0],
        input["content"][0]["content"][0]["content"][1]["content"][2]
    );
    assert_eq!(
        after["content"][0]["content"][1]["content"][2],
        input["content"][0]["content"][0]["content"][2]
    );
}

#[test]
fn explicit_join_never_retargets_stale_expected_list_content() {
    let original = document(
        json!({"type":"doc","content":[list("bulletList",vec![item("a","A")]),list("bulletList",vec![item("b","B")])]}),
    );
    let left = original.node_ref(&NodePath(vec![0])).expect("left");
    let right = original.node_ref(&NodePath(vec![1])).expect("right");
    let mut editor = editor(original);
    apply(
        &mut editor,
        Command::ReplaceText {
            range: TextRange {
                block_id: "b".into(),
                start: ScalarOffset(0),
                end: ScalarOffset(1),
            },
            expected: "B".into(),
            replacement: "changed".into(),
        },
    )
    .expect("edit");
    assert!(matches!(
        join_adjacent_lists(editor.document(), left, right),
        Err(Error::NodeReferenceConflict)
    ));
}

#[test]
fn adjacent_lists_with_different_null_envelopes_are_never_joined() {
    for kind in ["bulletList", "orderedList"] {
        for field in ["attrs", "marks", "text"] {
            let mut input = json!({"type":"doc","content":[{"type":kind,"content":[item("a","A")]},{"type":kind,"content":[item("b","B")]}]});
            if kind == "orderedList" {
                input["content"][1]["attrs"] = json!({"start":2});
            }
            input["content"][0][field] = Value::Null;
            let original = document(input.clone());
            assert!(
                join_adjacent_lists(
                    &original,
                    original.node_ref(&NodePath(vec![0])).expect("left"),
                    original.node_ref(&NodePath(vec![1])).expect("right")
                )
                .is_err()
            );
            assert_eq!(original.to_json().expect("retained input"), input);
        }
    }
}
