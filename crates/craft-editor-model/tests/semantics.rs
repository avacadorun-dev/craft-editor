#![expect(
    clippy::expect_used,
    reason = "Synthetic fixtures fail immediately when their declared valid construction is broken"
)]
use craft_editor_model::{
    ByteOffset, Command, Document, Editor, Error, Mark, MarkMode, Node, Revision, ScalarOffset,
    TextRange, Transaction, Utf16Offset, byte_to_scalar, scalar_to_byte, scalar_to_utf16,
    utf16_to_scalar,
};
use serde_json::{Value, json};

fn range(id: &str, start: usize, end: usize) -> TextRange {
    TextRange {
        block_id: id.into(),
        start: ScalarOffset(start),
        end: ScalarOffset(end),
    }
}
fn transact(
    editor: &mut Editor,
    commands: Vec<Command>,
) -> craft_editor_model::Result<craft_editor_model::Change> {
    editor.apply(Transaction {
        base_revision: editor.revision(),
        commands,
    })
}
fn editor(text: &str) -> Editor {
    Editor::new(
        Document::plain("p", text).expect("fixture document"),
        Revision(7),
    )
}
fn text_node(text: &str) -> Value {
    json!({"type":"text","text":text})
}

fn full_document() -> Value {
    json!({"type":"doc","attrs":{"schemaVersion":4,"vendorRoot":{"keep":true}},"vendorTop":{"null":null},"content":[
        {"type":"paragraph","attrs":{"blockId":"p","sourceRefs":["audio-1"],"vendor":{"keep":[1,null,"два"]}},"vendorNode":"keep","content":[
            {"type":"text","text":"А😀 е́","vendorLeaf":42,"marks":[{"type":"bold","vendorMark":{"keep":true}},{"type":"italic"},{"type":"underline"},{"type":"strike"},{"type":"link","attrs":{"href":"https://example.org","target":"_blank","rel":"noopener noreferrer","class":"custom","title":"Title","vendor":1}},{"type":"code"}]},
            {"type":"hardBreak"},{"type":"inlineMath","attrs":{"latex":"x^2","description":"squared","vendor":"math"}}
        ]},
        {"type":"heading","attrs":{"blockId":"h","sourceRefs":[],"level":6},"content":[text_node("Heading")]},
        {"type":"blockquote","attrs":{"blockId":"q","sourceRefs":["audio-2"]},"content":[{"type":"paragraph","attrs":{"blockId":"quote-p"},"content":[text_node("Quote")]}]},
        {"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","attrs":{"blockId":"bullet-p"},"content":[text_node("List")]}]}]},
        {"type":"orderedList","attrs":{"start":3,"type":"a"},"content":[{"type":"listItem","content":[{"type":"paragraph","attrs":{"blockId":"ordered-p"},"content":[text_node("Ordered")]}]}]},
        {"type":"horizontalRule"},{"type":"sceneBreak"},
        {"type":"codeBlock","attrs":{"blockId":"code","language":"rust","vendor":null},"content":[text_node("\tfn main() {}\n")]},
        {"type":"table","attrs":{"blockId":"table"},"content":[{"type":"tableRow","content":[
            {"type":"tableHeader","attrs":{"colspan":1,"rowspan":1,"colwidth":[120],"align":"center"},"content":[{"type":"paragraph","attrs":{"blockId":"header-p"},"content":[text_node("Header")]}]},
            {"type":"tableCell","attrs":{"colspan":1,"rowspan":1,"colwidth":null,"align":null},"content":[{"type":"paragraph","attrs":{"blockId":"cell-p"},"content":[text_node("Cell")]}]}
        ]}]},
        {"type":"imageBlock","attrs":{"blockId":"image","assetId":"asset-1","src":"/assets/asset-1","alt":"Alt","caption":"Caption","width":120,"height":80}},
        {"type":"mermaidBlock","attrs":{"blockId":"mermaid","code":"graph TD\nA-->B","caption":"Diagram"}},
        {"type":"callout","attrs":{"blockId":"callout","kind":"idea"},"content":[{"type":"paragraph","attrs":{"blockId":"callout-p"},"content":[text_node("Idea")]}]},
        {"type":"mathBlock","attrs":{"blockId":"math","latex":"\\frac{1}{2}","description":"one half"}}
    ]})
}

#[test]
fn all_twenty_one_nodes_six_marks_and_unknown_fields_roundtrip() {
    let original = full_document();
    let document = Document::from_json(original.clone()).expect("complete legacy fixture");
    assert_eq!(document.to_json().expect("serialize"), original);
}
#[test]
fn omitted_and_empty_fields_remain_distinct() {
    let input =
        json!({"type":"doc","content":[{"type":"paragraph","attrs":{},"marks":[],"content":[]}]});
    assert_eq!(
        Document::from_json(input.clone())
            .expect("fixture")
            .to_json()
            .expect("serialize"),
        input
    );
}
#[test]
fn future_unknown_nodes_and_invalid_structure_are_rejected_without_rewriting() {
    for invalid in [
        json!({"type":"doc","content":[{"type":"futureBlock"}]}),
        json!({"type":"doc","content":[{"type":"text","text":"bare text"}]}),
        json!({"type":"doc","attrs":{"schemaVersion":5},"content":[{"type":"paragraph"}]}),
    ] {
        assert!(Document::from_json(invalid).is_err());
    }
}
#[test]
fn schema_gates_do_not_eagerly_upgrade_plain_documents() {
    let original = json!({"type":"doc","content":[{"type":"paragraph","attrs":{"blockId":"p"},"content":[text_node("old")]}]});
    let mut editor = Editor::new(Document::from_json(original).expect("v1"), Revision(1));
    transact(
        &mut editor,
        vec![Command::ReplaceText {
            range: range("p", 0, 3),
            expected: "old".into(),
            replacement: "new".into(),
        }],
    )
    .expect("edit");
    assert!(editor.document().root().attrs.is_none());
    transact(
        &mut editor,
        vec![Command::ApplyMarks {
            range: range("p", 0, 3),
            marks: vec![Mark::new("code")],
            mode: MarkMode::Add,
        }],
    )
    .expect("code");
    assert_eq!(
        editor
            .document()
            .root()
            .attrs
            .as_ref()
            .and_then(|attrs| attrs.get("schemaVersion")),
        Some(&json!(2))
    );
}
#[test]
fn exact_second_repeated_quote_after_unicode_changes_only_requested_occurrence() {
    let mut editor = editor("А😀 повтор / повтор");
    transact(
        &mut editor,
        vec![Command::ReplaceText {
            range: range("p", 12, 18),
            expected: "повтор".into(),
            replacement: "другое".into(),
        }],
    )
    .expect("exact edit");
    assert_eq!(
        editor.document().block_text("p").expect("text"),
        "А😀 повтор / другое"
    );
}
#[test]
fn exact_edit_preserves_surrounding_marks_unknown_metadata_and_provenance() {
    let mut original = full_document();
    original["content"][0]["content"] = json!([{ "type":"text","text":"before ","marks":[{"type":"italic"}],"extra":"A"},{"type":"text","text":"target","marks":[{"type":"bold"}],"extra":"B"},{"type":"text","text":" after","extra":"C"}]);
    let mut editor = Editor::new(
        Document::from_json(original.clone()).expect("fixture"),
        Revision(1),
    );
    transact(
        &mut editor,
        vec![Command::ReplaceText {
            range: range("p", 7, 13),
            expected: "target".into(),
            replacement: "новое".into(),
        }],
    )
    .expect("edit");
    let after = editor.document().to_json().expect("serialize");
    assert_eq!(
        after["content"][0]["attrs"],
        original["content"][0]["attrs"]
    );
    assert_eq!(
        after["content"][0]["content"][0],
        original["content"][0]["content"][0]
    );
    assert_eq!(
        after["content"][0]["content"][1]["marks"],
        json!([{"type":"bold"}])
    );
    assert_eq!(
        after["content"][0]["content"][2],
        original["content"][0]["content"][2]
    );
}
#[test]
fn failed_batch_and_stale_revision_leave_state_and_history_unchanged() {
    let mut editor = editor("А😀Б");
    let original = editor.document().clone();
    let result = transact(
        &mut editor,
        vec![
            Command::ReplaceText {
                range: range("p", 0, 1),
                expected: "А".into(),
                replacement: "X".into(),
            },
            Command::ReplaceText {
                range: range("p", 99, 100),
                expected: "?".into(),
                replacement: "bad".into(),
            },
        ],
    );
    assert!(result.is_err());
    assert_eq!(editor.document(), &original);
    assert_eq!(editor.revision(), Revision(7));
    assert!(!editor.can_undo());
    assert!(matches!(
        editor.apply(Transaction {
            base_revision: Revision(6),
            commands: vec![]
        }),
        Err(Error::StaleRevision)
    ));
}
#[test]
fn top_level_insertion_uses_block_ordinals_and_rejects_duplicate_ids_atomically() {
    let mut editor = editor("first");
    transact(
        &mut editor,
        vec![Command::InsertBlocks {
            index: 1,
            blocks: vec![Node::paragraph("second", "second")],
        }],
    )
    .expect("append ordinal");
    assert_eq!(editor.document().blocks().len(), 2);
    let before = editor.document().clone();
    assert!(
        transact(
            &mut editor,
            vec![Command::InsertBlocks {
                index: 0,
                blocks: vec![Node::paragraph("p", "duplicate")]
            }]
        )
        .is_err()
    );
    assert_eq!(editor.document(), &before);
}
#[test]
fn split_join_preserves_exact_json_in_history_and_unions_source_refs() {
    let mut original = full_document();
    original["content"][0]["content"] = json!([text_node("А😀Б")]);
    let mut editor = Editor::new(
        Document::from_json(original.clone()).expect("fixture"),
        Revision(3),
    );
    transact(
        &mut editor,
        vec![Command::SplitBlock {
            block_id: "p".into(),
            at: ScalarOffset(2),
            new_block_id: "right".into(),
        }],
    )
    .expect("split");
    assert_eq!(editor.document().block_text("p").expect("left"), "А😀");
    assert_eq!(editor.document().block_text("right").expect("right"), "Б");
    assert_eq!(
        editor
            .document()
            .block("right")
            .expect("right")
            .attrs
            .as_ref()
            .and_then(|attrs| attrs.get("sourceRefs")),
        Some(&json!(["audio-1"]))
    );
    transact(
        &mut editor,
        vec![
            Command::SetBlockAttrs {
                block_id: "right".into(),
                attrs: serde_json::from_value(json!({"sourceRefs":["audio-2"]})).expect("attrs"),
            },
            Command::JoinBlocks {
                left_id: "p".into(),
                right_id: "right".into(),
            },
        ],
    )
    .expect("join");
    assert_eq!(editor.document().block_text("p").expect("joined"), "А😀Б");
    assert_eq!(
        editor
            .document()
            .block("p")
            .expect("joined")
            .attrs
            .as_ref()
            .and_then(|attrs| attrs.get("sourceRefs")),
        Some(&json!(["audio-1", "audio-2"]))
    );
    editor.undo(editor.revision()).expect("undo join");
    editor.undo(editor.revision()).expect("undo split");
    assert_eq!(editor.document().to_json().expect("serialize"), original);
}
#[test]
fn join_refuses_conflicting_unknown_metadata_instead_of_discarding_it() {
    let mut editor = editor("left");
    let mut right = Node::paragraph("right", "right");
    right
        .extra
        .insert("vendor".into(), json!({"different":true}));
    transact(
        &mut editor,
        vec![Command::InsertBlocks {
            index: 1,
            blocks: vec![right],
        }],
    )
    .expect("insert");
    let before = editor.document().clone();
    assert!(matches!(
        transact(
            &mut editor,
            vec![Command::JoinBlocks {
                left_id: "p".into(),
                right_id: "right".into()
            }]
        ),
        Err(Error::UnsupportedEdit)
    ));
    assert_eq!(editor.document(), &before);
}
#[test]
fn mark_add_remove_replace_have_explicit_distinct_semantics() {
    let mut editor = editor("one two");
    transact(
        &mut editor,
        vec![Command::ApplyMarks {
            range: range("p", 4, 7),
            marks: vec![Mark::new("bold")],
            mode: MarkMode::Add,
        }],
    )
    .expect("bold");
    transact(
        &mut editor,
        vec![Command::ApplyMarks {
            range: range("p", 4, 7),
            marks: vec![Mark::new("italic")],
            mode: MarkMode::Add,
        }],
    )
    .expect("italic");
    let marks = editor.document().block("p").expect("block").children()[1]
        .marks
        .as_ref()
        .expect("marks");
    assert_eq!(marks.len(), 2);
    transact(
        &mut editor,
        vec![Command::ApplyMarks {
            range: range("p", 4, 7),
            marks: vec![Mark::new("bold")],
            mode: MarkMode::Remove,
        }],
    )
    .expect("remove bold");
    assert_eq!(
        editor.document().block("p").expect("block").children()[1]
            .marks
            .as_ref()
            .expect("marks"),
        &vec![Mark::new("italic")]
    );
    transact(
        &mut editor,
        vec![Command::ApplyMarks {
            range: range("p", 4, 7),
            marks: vec![],
            mode: MarkMode::Replace,
        }],
    )
    .expect("clear marks");
    assert!(
        editor.document().block("p").expect("block").children()[1]
            .marks
            .as_ref()
            .expect("empty marks")
            .is_empty()
    );
}
#[test]
fn undo_redo_use_monotonic_revisions_preserve_json_and_clear_redo_on_branch() {
    let mut editor = editor("before");
    let before = editor.document().to_json().expect("before");
    transact(
        &mut editor,
        vec![Command::ReplaceText {
            range: range("p", 0, 6),
            expected: "before".into(),
            replacement: "after".into(),
        }],
    )
    .expect("edit");
    let after = editor.document().to_json().expect("after");
    editor.undo(Revision(8)).expect("undo");
    assert_eq!(editor.revision(), Revision(9));
    assert_eq!(editor.document().to_json().expect("json"), before);
    editor.redo(Revision(9)).expect("redo");
    assert_eq!(editor.revision(), Revision(10));
    assert_eq!(editor.document().to_json().expect("json"), after);
    editor.undo(Revision(10)).expect("undo");
    transact(
        &mut editor,
        vec![Command::ReplaceText {
            range: range("p", 0, 6),
            expected: "before".into(),
            replacement: "branch".into(),
        }],
    )
    .expect("branch");
    assert!(!editor.can_redo());
}
#[test]
fn every_unicode_boundary_roundtrips_between_scalars_utf16_and_bytes() {
    let text = "Ёж 😀 é 👩‍💻 中文 日本語";
    for index in 0..=text.chars().count() {
        let scalar = ScalarOffset(index);
        assert_eq!(
            utf16_to_scalar(text, scalar_to_utf16(text, scalar).expect("utf16")).expect("scalar"),
            scalar
        );
        assert_eq!(
            byte_to_scalar(text, scalar_to_byte(text, scalar).expect("bytes")).expect("scalar"),
            scalar
        );
    }
    assert!(utf16_to_scalar("А😀Б", Utf16Offset(2)).is_err());
    assert!(byte_to_scalar("А😀Б", ByteOffset(1)).is_err());
}
#[test]
fn shared_projection_distinguishes_hard_break_and_inline_math_atoms() {
    let doc = Document::from_json(full_document()).expect("fixture");
    assert_eq!(doc.block_text("p").expect("projection"), "А😀 е́\n\u{fffc}");
}
#[test]
fn nested_text_blocks_use_the_same_exact_edit_engine() {
    let mut editor = Editor::new(
        Document::from_json(full_document()).expect("fixture"),
        Revision(1),
    );
    transact(
        &mut editor,
        vec![Command::ReplaceText {
            range: range("cell-p", 0, 4),
            expected: "Cell".into(),
            replacement: "Ячейка😀".into(),
        }],
    )
    .expect("cell edit");
    assert_eq!(
        editor.document().block_text("cell-p").expect("cell"),
        "Ячейка😀"
    );
}
#[test]
fn no_op_does_not_consume_revision_or_history() {
    let mut editor = editor("same");
    let change = transact(
        &mut editor,
        vec![Command::ReplaceText {
            range: range("p", 0, 4),
            expected: "same".into(),
            replacement: "same".into(),
        }],
    )
    .expect("no op");
    assert_eq!(change.before_revision, change.after_revision);
    assert!(!editor.can_undo());
}

#[test]
fn explicit_null_known_fields_survive_without_becoming_omitted() {
    let input = json!({"type":"doc","marks":null,"text":null,"content":[
        {"type":"paragraph","attrs":null,"marks":null,"text":null,"content":null},
        {"type":"paragraph","content":[{"type":"text","text":"x","attrs":null,"content":null,"marks":[{"type":"bold","attrs":null,"vendor":null}]}]}
    ]});
    assert_eq!(
        Document::from_json(input.clone())
            .expect("nullable legacy envelope")
            .to_json()
            .expect("serialize"),
        input
    );
}

#[test]
fn reserved_node_extra_fields_cannot_override_typed_state_or_enter_transactions() {
    for key in ["type", "attrs", "text", "content", "marks"] {
        let mut node = Node::paragraph("inserted", "safe");
        node.extra
            .insert(key.into(), json!({"malicious":"collision"}));
        assert!(serde_json::to_value(&node).is_err(), "reserved key {key}");
        let mut editor = editor("original");
        let before = editor.document().clone();
        assert!(
            transact(
                &mut editor,
                vec![Command::InsertBlocks {
                    index: 1,
                    blocks: vec![node]
                }]
            )
            .is_err()
        );
        assert_eq!(editor.document(), &before);
        assert_eq!(editor.revision(), Revision(7));
        assert!(!editor.can_undo());
    }
}
#[test]
fn reserved_mark_extra_fields_cannot_override_canonical_mark_attributes() {
    for key in ["type", "attrs"] {
        let mut mark = Mark::new("bold");
        mark.extra.insert(key.into(), json!({"collision":true}));
        assert!(serde_json::to_value(&mark).is_err());
        let mut editor = editor("original");
        let before = editor.document().clone();
        assert!(
            transact(
                &mut editor,
                vec![Command::ApplyMarks {
                    range: range("p", 0, 3),
                    marks: vec![mark],
                    mode: MarkMode::Add
                }]
            )
            .is_err()
        );
        assert_eq!(editor.document(), &before);
        assert!(!editor.can_undo());
    }
}

#[test]
fn hundreds_of_typing_transactions_coalesce_identical_text_metadata() {
    let mut editor = editor("А").with_history_limit(4);
    for position in 1..=160 {
        transact(
            &mut editor,
            vec![Command::ReplaceText {
                range: range("p", position, position),
                expected: String::new(),
                replacement: "😀".into(),
            }],
        )
        .expect("type");
        assert_eq!(
            editor
                .document()
                .block("p")
                .expect("paragraph")
                .children()
                .len(),
            1
        );
    }
    assert_eq!(
        editor
            .document()
            .block_text("p")
            .expect("text")
            .chars()
            .count(),
        161
    );
}
#[test]
fn coalescing_preserves_marks_unknown_fields_and_block_provenance() {
    let original = json!({"type":"doc","content":[{"type":"paragraph","attrs":{"blockId":"p","sourceRefs":["source"]},"content":[
        {"type":"text","text":"А","attrs":{"custom":1},"marks":[{"type":"bold","vendor":"mark"}],"vendor":{"nested":[1,null]}},
        {"type":"text","text":"Б","attrs":{"custom":2},"marks":[{"type":"bold","vendor":"mark"}],"vendor":{"nested":[1,null]}}
    ]}]});
    let mut editor = Editor::new(
        Document::from_json(original.clone()).expect("fixture"),
        Revision(1),
    )
    .with_history_limit(4);
    for _ in 0..120 {
        transact(
            &mut editor,
            vec![Command::ReplaceText {
                range: range("p", 1, 1),
                expected: String::new(),
                replacement: "я".into(),
            }],
        )
        .expect("insert inside second metadata run");
    }
    let after = editor.document().to_json().expect("json");
    let children = editor.document().block("p").expect("paragraph").children();
    assert_eq!(children.len(), 2);
    assert_eq!(children[0].extra, children[1].extra);
    assert_ne!(children[0].attrs, children[1].attrs);
    assert_eq!(
        after["content"][0]["attrs"],
        original["content"][0]["attrs"]
    );
    assert_eq!(
        after["content"][0]["content"][1]["marks"],
        original["content"][0]["content"][1]["marks"]
    );
    assert_eq!(
        after["content"][0]["content"][1]["vendor"],
        original["content"][0]["content"][1]["vendor"]
    );
}
#[test]
fn coalescing_never_erases_null_empty_or_different_unknown_field_presence() {
    let input = json!({"type":"doc","content":[{"type":"paragraph","attrs":{"blockId":"p"},"content":[
        {"type":"text","text":"a","attrs":null},
        {"type":"text","text":"b"},
        {"type":"text","text":"c","marks":[]},
        {"type":"text","text":"d","vendor":null},
        {"type":"text","text":"e","vendor":{"same":false}}
    ]}]});
    let mut editor = Editor::new(
        Document::from_json(input.clone()).expect("fixture"),
        Revision(1),
    );
    transact(
        &mut editor,
        vec![Command::ReplaceText {
            range: range("p", 5, 5),
            expected: String::new(),
            replacement: "x".into(),
        }],
    )
    .expect("append");
    let after = editor.document().to_json().expect("json");
    assert_eq!(editor.document().block("p").expect("p").children().len(), 5);
    for index in 0..4 {
        assert_eq!(
            after["content"][0]["content"][index],
            input["content"][0]["content"][index]
        );
    }
    assert_eq!(
        after["content"][0]["content"][4]["vendor"],
        json!({"same":false})
    );
}

#[test]
fn source_link_uri_policy_accepts_original_protocols_and_relative_targets() {
    for value in [
        "http://example.test",
        "https://example.test",
        "ftp://example.test",
        "ftps://example.test",
        "mailto:author@example.test",
        "tel:+123",
        "callto:author",
        "sms:+123",
        "cid:attachment",
        "xmpp:author@example.test",
        "/chapter",
        "../chapter",
        "chapter",
        "#part",
        "",
        "HtTpS://example.test",
    ] {
        assert!(
            craft_editor_model::is_allowed_link_uri(value),
            "source-allowed URI rejected: {value}"
        );
    }
    for value in [
        "javascript:bad",
        "java\u{0000}script:bad",
        "data:text/html,bad",
        "vbscript:bad",
        "file:///private/file",
        "custom:bad",
    ] {
        assert!(
            !craft_editor_model::is_allowed_link_uri(value),
            "source-disallowed URI accepted: {value}"
        );
    }
}
