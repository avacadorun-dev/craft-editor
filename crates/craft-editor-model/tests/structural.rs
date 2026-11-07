#![expect(
    clippy::expect_used,
    reason = "Synthetic fixtures must be valid before their behavior is exercised"
)]

use craft_editor_model::{
    Command, Document, Editor, Error, Node, NodePath, Revision, ScalarOffset, TextRange,
    Transaction,
};
use serde_json::{Value, json};

fn document() -> Document {
    Document::from_json(json!({"type":"doc","attrs":{"schemaVersion":4,"vendor":{"root":null}},"content":[
        {"type":"paragraph","attrs":{"blockId":"p","sourceRefs":["source"]},"content":[{"type":"text","text":"A"}]},
        {"type":"paragraph","attrs":{"blockId":"middle"},"content":[{"type":"text","text":"B"}]},
        {"type":"blockquote","attrs":{"blockId":"quote","sourceRefs":[]},"vendor":"preserve","content":[
            {"type":"paragraph","attrs":{"blockId":"q1"},"content":[{"type":"text","text":"C"}]},
            {"type":"paragraph","attrs":{"blockId":"q2","sourceRefs":["source2"]},"content":[{"type":"text","text":"D"}]}
        ]}
    ]})).expect("fixture")
}
fn apply(
    editor: &mut Editor,
    commands: Vec<Command>,
) -> craft_editor_model::Result<craft_editor_model::Change> {
    editor.apply(Transaction {
        base_revision: editor.revision(),
        commands,
    })
}
fn node(value: Value) -> Node {
    serde_json::from_value(value).expect("fixture node")
}

#[test]
fn insert_inline_atom_uses_exact_nested_parent_and_roundtrips_wire_command() {
    let mut editor = Editor::new(document(), Revision(1));
    let parent = editor
        .document()
        .node_ref(&NodePath(vec![2, 0]))
        .expect("nested paragraph");
    let atom = node(
        json!({"type":"inlineMath","attrs":{"latex":"x^2","description":"square","vendor":null},"vendorTop":true}),
    );
    let command = Command::InsertChildren {
        parent,
        index: 1,
        children: vec![atom.clone()],
    };
    let encoded = serde_json::to_value(&command).expect("encode");
    let decoded = serde_json::from_value(encoded).expect("decode");
    let change = apply(&mut editor, vec![decoded]).expect("insert");
    assert_eq!(
        editor
            .document()
            .node(&NodePath(vec![2, 0, 1]))
            .expect("atom"),
        &atom
    );
    assert_eq!(
        change.mappings[0].map_path(&NodePath(vec![2, 0, 0])),
        Some(NodePath(vec![2, 0, 0]))
    );
    editor.undo(editor.revision()).expect("undo");
    assert_eq!(editor.document(), &document());
}
#[test]
fn atom_replacement_preserves_unknown_fields_and_detaches_old_node_path() {
    let mut editor = Editor::new(document(), Revision(1));
    let parent = editor.document().node_ref(&NodePath::root()).expect("root");
    let atom = node(
        json!({"type":"mathBlock","attrs":{"blockId":"math","latex":"\\sqrt{x}","description":"root","unknown":{"keep":true}},"extension":null}),
    );
    let change = apply(
        &mut editor,
        vec![Command::ReplaceChildren {
            parent,
            from: 1,
            to: 2,
            children: vec![atom.clone()],
        }],
    )
    .expect("replace");
    assert_eq!(
        editor.document().node(&NodePath(vec![1])).expect("math"),
        &atom
    );
    assert_eq!(change.mappings[0].map_path(&NodePath(vec![1])), None);
    assert!(
        change.mappings[0]
            .map_existing_caret("middle", ScalarOffset(0), true)
            .is_none()
    );
    assert_eq!(
        change.mappings[0].map_path(&NodePath(vec![2, 1, 0])),
        Some(NodePath(vec![2, 1, 0]))
    );
}
#[test]
fn deleting_nested_children_preserves_parent_metadata_and_maps_following_paths() {
    let mut editor = Editor::new(document(), Revision(1));
    let parent = editor
        .document()
        .node_ref(&NodePath(vec![2]))
        .expect("quote");
    let change = apply(
        &mut editor,
        vec![Command::DeleteChildren {
            parent,
            from: 0,
            to: 1,
        }],
    )
    .expect("delete");
    assert_eq!(
        editor
            .document()
            .node(&NodePath(vec![2]))
            .expect("quote")
            .extra
            .get("vendor"),
        Some(&json!("preserve"))
    );
    assert_eq!(change.mappings[0].map_path(&NodePath(vec![2, 0, 0])), None);
    assert_eq!(
        change.mappings[0].map_path(&NodePath(vec![2, 1, 0])),
        Some(NodePath(vec![2, 0, 0]))
    );
    assert!(
        change.mappings[0]
            .map_existing_caret("q1", ScalarOffset(0), true)
            .is_none()
    );
}
#[test]
fn same_parent_move_uses_pre_removal_ordinal_and_retains_moved_paths() {
    let mut editor = Editor::new(document(), Revision(1));
    let source = editor.document().node_ref(&NodePath::root()).expect("root");
    let change = apply(
        &mut editor,
        vec![Command::MoveChildren {
            source: source.clone(),
            from: 0,
            to: 1,
            destination: source,
            index: 3,
        }],
    )
    .expect("move first to end");
    assert_eq!(editor.document().blocks()[2].block_id(), Some("p"));
    assert_eq!(
        change.mappings[0].map_path(&NodePath(vec![0, 0])),
        Some(NodePath(vec![2, 0]))
    );
    assert_eq!(
        change.mappings[0].map_path(&NodePath(vec![1, 0])),
        Some(NodePath(vec![0, 0]))
    );
    assert_eq!(
        change.mappings[0].map_path(&NodePath(vec![2, 1, 0])),
        Some(NodePath(vec![1, 1, 0]))
    );
    assert_eq!(
        change.mappings[0].map_existing_caret("p", ScalarOffset(1), true),
        Some(("p".into(), ScalarOffset(1)))
    );
}
#[test]
fn move_into_later_nested_parent_tracks_destination_path_shift_and_full_metadata() {
    let mut editor = Editor::new(document(), Revision(1));
    let original = editor.document().blocks()[0].clone();
    let source = editor.document().node_ref(&NodePath::root()).expect("root");
    let destination = editor
        .document()
        .node_ref(&NodePath(vec![2]))
        .expect("quote");
    let change = apply(
        &mut editor,
        vec![Command::MoveChildren {
            source,
            from: 0,
            to: 1,
            destination,
            index: 1,
        }],
    )
    .expect("move into quote");
    assert_eq!(
        editor
            .document()
            .node(&NodePath(vec![1, 1]))
            .expect("moved"),
        &original
    );
    assert_eq!(
        change.mappings[0].map_path(&NodePath(vec![0, 0])),
        Some(NodePath(vec![1, 1, 0]))
    );
    assert_eq!(
        change.mappings[0].map_path(&NodePath(vec![2, 1, 0])),
        Some(NodePath(vec![1, 2, 0]))
    );
    assert_eq!(
        change.mappings[0].map_path(&NodePath(vec![1, 0])),
        Some(NodePath(vec![0, 0]))
    );
}
#[test]
fn move_from_nested_parent_to_ancestor_preserves_paths_in_both_subtrees() {
    let mut editor = Editor::new(document(), Revision(1));
    let source = editor
        .document()
        .node_ref(&NodePath(vec![2]))
        .expect("quote");
    let destination = editor.document().node_ref(&NodePath::root()).expect("root");
    let change = apply(
        &mut editor,
        vec![Command::MoveChildren {
            source,
            from: 1,
            to: 2,
            destination,
            index: 0,
        }],
    )
    .expect("lift to root");
    assert_eq!(editor.document().blocks()[0].block_id(), Some("q2"));
    assert_eq!(
        change.mappings[0].map_path(&NodePath(vec![2, 1, 0])),
        Some(NodePath(vec![0, 0]))
    );
    assert_eq!(
        change.mappings[0].map_path(&NodePath(vec![2, 0, 0])),
        Some(NodePath(vec![3, 0, 0]))
    );
}
#[test]
fn moving_range_inside_itself_is_no_op_and_descendant_cycles_are_rejected() {
    let mut editor = Editor::new(document(), Revision(1));
    let root = editor.document().node_ref(&NodePath::root()).expect("root");
    let no_op = apply(
        &mut editor,
        vec![Command::MoveChildren {
            source: root.clone(),
            from: 0,
            to: 2,
            destination: root.clone(),
            index: 1,
        }],
    )
    .expect("same range");
    assert_eq!(no_op.before_revision, no_op.after_revision);
    assert!(!editor.can_undo());
    let descendant = editor
        .document()
        .node_ref(&NodePath(vec![2]))
        .expect("quote");
    assert!(
        apply(
            &mut editor,
            vec![Command::MoveChildren {
                source: root,
                from: 2,
                to: 3,
                destination: descendant,
                index: 0
            }]
        )
        .is_err()
    );
    assert_eq!(editor.document(), &document());
}
#[test]
fn stale_or_wrong_exact_node_guard_cannot_retarget_same_type_node() {
    let mut editor = Editor::new(document(), Revision(1));
    let mut reference = editor
        .document()
        .node_ref(&NodePath(vec![2, 0]))
        .expect("first paragraph");
    reference.path = NodePath(vec![2, 1]);
    let error = apply(
        &mut editor,
        vec![Command::InsertChildren {
            parent: reference,
            index: 0,
            children: vec![Node::text("bad")],
        }],
    )
    .expect_err("wrong same-type node");
    assert!(matches!(error, Error::NodeReferenceConflict));
    assert_eq!(editor.document(), &document());
    assert!(!editor.can_undo());
}
#[test]
fn reference_guard_is_checked_against_sequential_intermediate_state_atomically() {
    let mut editor = Editor::new(document(), Revision(1));
    let captured = editor.document().node_ref(&NodePath::root()).expect("root");
    let first = Command::ReplaceText {
        range: TextRange {
            block_id: "p".into(),
            start: ScalarOffset(0),
            end: ScalarOffset(1),
        },
        expected: "A".into(),
        replacement: "changed".into(),
    };
    let second = Command::DeleteChildren {
        parent: captured,
        from: 1,
        to: 2,
    };
    assert!(matches!(
        apply(&mut editor, vec![first, second]),
        Err(Error::NodeReferenceConflict)
    ));
    assert_eq!(editor.document(), &document());
    assert_eq!(editor.revision(), Revision(1));
}
#[test]
fn sequential_current_references_commit_one_revision_and_undo_the_entire_batch() {
    let original = document();
    let first_parent = original.node_ref(&NodePath::root()).expect("root");
    let first = Command::InsertChildren {
        parent: first_parent,
        index: 1,
        children: vec![Node::paragraph("new", "new")],
    };
    let mut intermediate = Editor::new(original.clone(), Revision(1));
    apply(&mut intermediate, vec![first.clone()]).expect("intermediate");
    let next_parent = intermediate
        .document()
        .node_ref(&NodePath::root())
        .expect("intermediate root");
    let second = Command::DeleteChildren {
        parent: next_parent,
        from: 2,
        to: 3,
    };
    let mut editor = Editor::new(original.clone(), Revision(1));
    let change = apply(&mut editor, vec![first, second]).expect("batch");
    assert_eq!(change.after_revision, Revision(2));
    assert_eq!(change.mappings.len(), 2);
    editor.undo(Revision(2)).expect("undo");
    assert_eq!(editor.document(), &original);
    editor.redo(Revision(3)).expect("redo");
    assert_eq!(editor.document(), &change.after);
}
#[test]
fn invalid_bounds_duplicate_ids_and_schema_failure_never_mutate_document_or_history() {
    let mut editor = Editor::new(document(), Revision(1));
    for (from, to) in [(0, 99), (usize::MAX, 0)] {
        let parent = editor.document().node_ref(&NodePath::root()).expect("root");
        assert!(
            apply(
                &mut editor,
                vec![Command::DeleteChildren { parent, from, to }]
            )
            .is_err()
        );
        assert_eq!(editor.document(), &document());
    }
    let parent = editor.document().node_ref(&NodePath::root()).expect("root");
    assert!(
        apply(
            &mut editor,
            vec![Command::InsertChildren {
                parent,
                index: 0,
                children: vec![Node::paragraph("p", "duplicate")]
            }]
        )
        .is_err()
    );
    let parent = editor.document().node_ref(&NodePath::root()).expect("root");
    assert!(
        apply(
            &mut editor,
            vec![Command::DeleteChildren {
                parent,
                from: 0,
                to: 3
            }]
        )
        .is_err()
    );
    assert_eq!(editor.document(), &document());
    assert!(!editor.can_undo());
}
#[test]
fn anonymous_table_and_list_nodes_can_be_guarded_without_inventing_ids() {
    let input = json!({"type":"doc","attrs":{"schemaVersion":2},"content":[
        {"type":"table","attrs":{"blockId":"t"},"content":[{"type":"tableRow","content":[{"type":"tableCell","attrs":{"colspan":1,"rowspan":1,"vendor":null},"content":[{"type":"paragraph","attrs":{"blockId":"cell-p"},"content":[]}]}]}]},
        {"type":"bulletList","content":[{"type":"listItem","vendor":{"keep":true},"content":[{"type":"paragraph","attrs":{"blockId":"item-p"},"content":[]}]}]}
    ]});
    let mut editor = Editor::new(
        Document::from_json(input.clone()).expect("fixture"),
        Revision(1),
    );
    let parent = editor
        .document()
        .node_ref(&NodePath(vec![0, 0]))
        .expect("anonymous row");
    let mut cell = parent.expected.children()[0].clone();
    cell.content = Some(vec![Node::paragraph("new-cell-p", "second")]);
    apply(
        &mut editor,
        vec![Command::InsertChildren {
            parent,
            index: 1,
            children: vec![cell],
        }],
    )
    .expect("row children");
    let list = editor
        .document()
        .node_ref(&NodePath(vec![1]))
        .expect("list");
    let mut item = list.expected.children()[0].clone();
    item.content = Some(vec![Node::paragraph("new-item-p", "second item")]);
    apply(
        &mut editor,
        vec![Command::InsertChildren {
            parent: list,
            index: 1,
            children: vec![item],
        }],
    )
    .expect("list children");
    let after = editor.document().to_json().expect("json");
    assert_eq!(
        after["content"][0]["content"][0]["content"][0],
        input["content"][0]["content"][0]["content"][0]
    );
    assert_eq!(
        after["content"][1]["content"][1]["vendor"],
        json!({"keep":true})
    );
}
#[test]
fn structural_transactions_reject_stale_revisions_before_resolving_paths() {
    let mut editor = Editor::new(document(), Revision(2));
    let parent = editor.document().node_ref(&NodePath::root()).expect("root");
    assert!(matches!(
        editor.apply(Transaction {
            base_revision: Revision(1),
            commands: vec![Command::DeleteChildren {
                parent,
                from: 0,
                to: 1
            }]
        }),
        Err(Error::StaleRevision)
    ));
    assert_eq!(editor.document(), &document());
}
