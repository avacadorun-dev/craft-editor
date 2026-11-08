use craft_editor_egui::{ContentSlice, DocPoint, EditorSession, Selection};
use craft_editor_model::{Document, Node, NodePath, Revision, ScalarOffset};
use serde_json::{Map, json};

type TestResult = Result<(), Box<dyn std::error::Error>>;
fn caret(id: &str, offset: usize) -> Selection {
    let point = DocPoint {
        block_id: id.into(),
        offset: ScalarOffset(offset),
    };
    Selection {
        anchor: point.clone(),
        head: point,
    }
}
fn image(id: &str) -> Node {
    Node::block(
        "imageBlock",
        Map::from_iter([
            ("blockId".into(), json!(id)),
            ("src".into(), json!("host:asset")),
        ]),
        vec![],
    )
}

#[test]
fn empty_paragraph_is_replaced_and_end_image_is_selected() -> TestResult {
    let original = Document::plain("p", "")?;
    let mut session = EditorSession::new(original.clone(), Revision(0));
    session.set_selection(caret("p", 0))?;
    session.insert_content(ContentSlice::Blocks(vec![image("new-image")]))?;
    assert_eq!(session.document().blocks().len(), 1);
    assert_eq!(
        session.node_selection().map(|node| node.path.clone()),
        Some(NodePath(vec![0]))
    );
    assert_eq!(
        session
            .document()
            .root()
            .attrs
            .as_ref()
            .and_then(|attrs| attrs.get("schemaVersion")),
        Some(&json!(2))
    );
    session.undo()?;
    assert_eq!(session.document(), &original);
    assert_eq!(session.selection(), Some(&caret("p", 0)));
    Ok(())
}
#[test]
fn start_and_middle_block_insertions_follow_the_original_right_tail_caret() -> TestResult {
    let mut start = EditorSession::new(Document::plain("p", "abc")?, Revision(0));
    start.set_selection(caret("p", 0))?;
    start.insert_content(ContentSlice::Blocks(vec![image("start-image")]))?;
    assert_eq!(start.document().blocks()[0].node_type, "imageBlock");
    assert_eq!(start.selection(), Some(&caret("p", 0)));
    let original = Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":1,"vendorRoot":null},"content":[{"type":"paragraph","attrs":{"blockId":"p","sourceRefs":["source"],"vendor":{"keep":true}},"extension":{"keep":9},"content":[{"type":"text","text":"Я😀"},{"type":"text","text":"tail","marks":[{"type":"italic"}]}]}]}),
    )?;
    let mut middle = EditorSession::new(original.clone(), Revision(0));
    middle.set_selection(caret("p", 2))?;
    let change = middle.insert_content(ContentSlice::Blocks(vec![image("middle-image")]))?;
    let blocks = middle.document().blocks();
    assert_eq!(blocks.len(), 3);
    assert_eq!(blocks[0].inline_text(), "Я😀");
    assert_eq!(blocks[1].node_type, "imageBlock");
    assert_eq!(blocks[2].inline_text(), "tail");
    assert_ne!(blocks[2].block_id(), Some("p"));
    assert_eq!(
        blocks[2]
            .attrs
            .as_ref()
            .and_then(|attrs| attrs.get("vendor")),
        Some(&json!({"keep":true}))
    );
    assert_eq!(blocks[2].extra, original.blocks()[0].extra);
    let tail_id = blocks[2].block_id().ok_or("missing tail ID")?.to_owned();
    assert_eq!(middle.selection(), Some(&caret(&tail_id, 0)));
    assert_eq!(
        &change.replay_proof().verify(&original)?.0,
        middle.document()
    );
    middle.undo()?;
    assert_eq!(middle.document(), &original);
    middle.redo()?;
    assert_eq!(middle.selection(), Some(&caret(&tail_id, 0)));
    Ok(())
}
#[test]
fn paragraph_end_insertion_does_not_create_a_trailing_empty_paragraph() -> TestResult {
    let mut session = EditorSession::new(Document::plain("p", "abc")?, Revision(0));
    session.set_selection(caret("p", 3))?;
    session.insert_content(ContentSlice::Blocks(vec![image("new-image")]))?;
    assert_eq!(session.document().blocks().len(), 2);
    assert_eq!(
        session.node_selection().map(|node| node.path.clone()),
        Some(NodePath(vec![1]))
    );
    Ok(())
}
#[test]
fn heading_tail_retains_kind_level_and_source_metadata() -> TestResult {
    let source = Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":1},"content":[{"type":"heading","attrs":{"blockId":"h","level":2,"sourceRefs":["s"]},"content":[{"type":"text","text":"abcdef"}]}]}),
    )?;
    let mut session = EditorSession::new(source, Revision(0));
    session.set_selection(caret("h", 3))?;
    session.insert_content(ContentSlice::Blocks(vec![image("new-image")]))?;
    let blocks = session.document().blocks();
    assert_eq!(
        blocks
            .iter()
            .map(|node| node.node_type.as_str())
            .collect::<Vec<_>>(),
        vec!["heading", "imageBlock", "heading"]
    );
    assert_eq!(
        blocks[2]
            .attrs
            .as_ref()
            .and_then(|attrs| attrs.get("level")),
        Some(&json!(2))
    );
    assert_eq!(
        blocks[2]
            .attrs
            .as_ref()
            .and_then(|attrs| attrs.get("sourceRefs")),
        Some(&json!(["s"]))
    );
    Ok(())
}
#[test]
fn nested_quote_and_list_item_keep_insertion_inside_the_actual_parent() -> TestResult {
    let quote = Node::block(
        "blockquote",
        Map::new(),
        vec![Node::paragraph("p", "abcdef")],
    );
    let source = Document::from_root(Node::block(
        "doc",
        Map::from_iter([("schemaVersion".into(), json!(1))]),
        vec![quote],
    ))?;
    let mut session = EditorSession::new(source, Revision(0));
    session.set_selection(caret("p", 3))?;
    session.insert_content(ContentSlice::Blocks(vec![image("quote-image")]))?;
    assert_eq!(session.document().blocks().len(), 1);
    assert_eq!(
        session.document().blocks()[0]
            .children()
            .iter()
            .map(|node| node.node_type.as_str())
            .collect::<Vec<_>>(),
        vec!["paragraph", "imageBlock", "paragraph"]
    );
    let source = Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":1},"content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","attrs":{"blockId":"p","sourceRefs":[]},"content":[]}]}]}]}),
    )?;
    let mut session = EditorSession::new(source, Revision(0));
    session.set_selection(caret("p", 0))?;
    session.insert_content(ContentSlice::Blocks(vec![image("list-image")]))?;
    let children = session.document().node(&NodePath(vec![0, 0]))?.children();
    assert_eq!(children.len(), 2);
    assert_eq!(children[0].node_type, "paragraph");
    assert_eq!(children[1].node_type, "imageBlock");
    assert_eq!(
        session.node_selection().map(|node| node.path.clone()),
        Some(NodePath(vec![0, 0, 1]))
    );
    Ok(())
}
#[test]
fn inline_formula_and_hard_break_remain_canonical_atoms_and_promote_schema() -> TestResult {
    let mut session = EditorSession::new(Document::plain("p", "AB")?, Revision(0));
    session.set_selection(caret("p", 1))?;
    let formula = Node::block(
        "inlineMath",
        Map::from_iter([("latex".into(), json!("x^2")), ("vendor".into(), json!(9))]),
        vec![],
    );
    let hard_break = Node::block("hardBreak", Map::new(), vec![]);
    session.insert_content(ContentSlice::Inline(vec![formula.clone(), hard_break]))?;
    assert_eq!(session.document().block_text("p")?, "A\u{fffc}\nB");
    assert_eq!(session.document().block("p")?.children()[1], formula);
    assert_eq!(session.selection(), Some(&caret("p", 3)));
    assert_eq!(
        session
            .document()
            .root()
            .attrs
            .as_ref()
            .and_then(|attrs| attrs.get("schemaVersion")),
        Some(&json!(4))
    );
    Ok(())
}
#[test]
fn cross_atom_range_replacement_and_undo_are_one_exact_transaction() -> TestResult {
    let source = Document::from_root(Node::block(
        "doc",
        Map::from_iter([("schemaVersion".into(), json!(2))]),
        vec![
            Node::paragraph("left", "AB"),
            image("old-image"),
            Node::paragraph("right", "CD"),
        ],
    ))?;
    let selection = Selection {
        anchor: caret("right", 1).head,
        head: caret("left", 1).head,
    };
    let mut session = EditorSession::new(source.clone(), Revision(0));
    session.set_selection(selection.clone())?;
    session.insert_plain_text("X")?;
    assert_eq!(session.document().blocks().len(), 1);
    assert_eq!(session.document().block_text("left")?, "AXD");
    assert_eq!(session.revision(), Revision(1));
    session.undo()?;
    assert_eq!(session.document(), &source);
    assert_eq!(session.selection(), Some(&selection));
    Ok(())
}
#[test]
fn insertion_inside_a_cell_keeps_table_grid_and_tail_in_that_cell() -> TestResult {
    let source = Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":2},"content":[{"type":"table","attrs":{"blockId":"grid"},"content":[{"type":"tableRow","content":[{"type":"tableCell","attrs":{"colspan":1,"rowspan":1,"vendor":9},"content":[{"type":"paragraph","attrs":{"blockId":"cell"},"content":[{"type":"text","text":"AB"}]}]},{"type":"tableCell","content":[{"type":"paragraph","attrs":{"blockId":"other"},"content":[]}]}]}]}]}),
    )?;
    let mut session = EditorSession::new(source.clone(), Revision(0));
    session.set_selection(caret("cell", 1))?;
    session.insert_content(ContentSlice::Blocks(vec![image("new-image")]))?;
    let cell = session.document().node(&NodePath(vec![0, 0, 0]))?;
    assert_eq!(cell.children().len(), 3);
    assert_eq!(cell.children()[0].inline_text(), "A");
    assert_eq!(cell.children()[2].inline_text(), "B");
    assert_eq!(cell.attrs, source.node(&NodePath(vec![0, 0, 0]))?.attrs);
    assert_eq!(
        session
            .document()
            .node(&NodePath(vec![0, 0]))?
            .children()
            .len(),
        2
    );
    Ok(())
}
#[test]
fn duplicate_ids_and_invalid_inline_placement_reject_without_mutation() -> TestResult {
    let original = Document::plain("p", "AB")?;
    let mut session = EditorSession::new(original.clone(), Revision(0));
    session.set_selection(caret("p", 1))?;
    assert!(
        session
            .insert_content(ContentSlice::Blocks(vec![Node::paragraph(
                "p",
                "collision"
            )]))
            .is_err()
    );
    assert!(
        session
            .insert_content(ContentSlice::Inline(vec![image("new-image")]))
            .is_err()
    );
    assert_eq!(session.document(), &original);
    assert_eq!(session.revision(), Revision(0));
    assert_eq!(session.selection(), Some(&caret("p", 1)));
    assert!(!session.can_undo());
    assert!(session.take_pending_changes().is_empty());
    Ok(())
}
#[test]
fn enter_allocations_skip_existing_ids_before_and_after_reopen() -> TestResult {
    let original = Document::from_root(Node::block(
        "doc",
        Map::from_iter([("schemaVersion".into(), json!(1))]),
        vec![
            Node::paragraph("p", "AB"),
            Node::paragraph("craft-view-0-1", "retained"),
        ],
    ))?;
    let mut session = EditorSession::new(original, Revision(0));
    session.set_selection(caret("p", 1))?;
    session.insert_plain_text("\n")?;
    assert_eq!(
        session.document().blocks()[1].block_id(),
        Some("craft-view-0-2")
    );
    let persisted = session.document().to_json()?;
    let mut reopened = EditorSession::new(Document::from_json(persisted)?, Revision(0));
    reopened.set_selection(caret("p", 1))?;
    reopened.insert_plain_text("\n")?;
    assert_eq!(
        reopened.document().blocks()[1].block_id(),
        Some("craft-view-0-3")
    );
    assert_eq!(
        reopened.document().block_text("craft-view-0-1")?,
        "retained"
    );
    assert_eq!(reopened.document().block_text("craft-view-0-2")?, "B");
    Ok(())
}

#[test]
fn cross_cell_selection_removes_covered_atom_without_changing_the_grid() -> TestResult {
    let source = Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":4},"content":[{"type":"table","attrs":{"blockId":"grid"},"content":[{"type":"tableRow","content":[
            {"type":"tableCell","content":[{"type":"paragraph","attrs":{"blockId":"left"},"content":[{"type":"text","text":"AB"}]}]},
            {"type":"tableCell","attrs":{"vendor":9},"content":[{"type":"mathBlock","attrs":{"blockId":"math","latex":"x^2"}}]},
            {"type":"tableCell","content":[{"type":"paragraph","attrs":{"blockId":"right"},"content":[{"type":"text","text":"CD"}]}]}
        ]}]}]}),
    )?;
    let mut session = EditorSession::new(source.clone(), Revision(0));
    session.set_selection(Selection {
        anchor: caret("left", 1).head,
        head: caret("right", 1).head,
    })?;
    session.insert_plain_text("X")?;
    assert_eq!(session.document().block_text("left")?, "AX");
    assert_eq!(session.document().block_text("right")?, "D");
    let middle = session.document().node(&NodePath(vec![0, 0, 1]))?;
    assert_eq!(middle.children().len(), 1);
    assert_eq!(middle.children()[0].node_type, "paragraph");
    assert_eq!(middle.attrs, source.node(&NodePath(vec![0, 0, 1]))?.attrs);
    assert_eq!(
        session
            .document()
            .node(&NodePath(vec![0, 0]))?
            .children()
            .len(),
        3
    );
    session.undo()?;
    assert_eq!(session.document(), &source);
    Ok(())
}
#[test]
fn selected_block_replacement_keeps_exact_node_history() -> TestResult {
    let original = Document::from_root(Node::block(
        "doc",
        Map::from_iter([("schemaVersion".into(), json!(2))]),
        vec![image("old-image")],
    ))?;
    let mut session = EditorSession::new(original.clone(), Revision(0));
    let original_selection = session.document().node_ref(&NodePath(vec![0]))?;
    session.select_node(original_selection.clone())?;
    let formula = Node::block(
        "mathBlock",
        Map::from_iter([
            ("blockId".into(), json!("new-math")),
            ("latex".into(), json!("x^2")),
        ]),
        vec![],
    );
    session.insert_content(ContentSlice::Blocks(vec![formula.clone()]))?;
    assert_eq!(session.document().blocks()[0], formula);
    assert_eq!(
        session
            .node_selection()
            .map(|reference| reference.expected.as_ref()),
        Some(&formula)
    );
    session.undo()?;
    assert_eq!(session.document(), &original);
    assert_eq!(session.node_selection(), Some(&original_selection));
    session.redo()?;
    assert_eq!(
        session
            .node_selection()
            .map(|reference| reference.expected.as_ref()),
        Some(&formula)
    );
    Ok(())
}

#[test]
fn missing_ids_are_allocated_only_for_new_identified_nodes() -> TestResult {
    let original = Document::plain("existing", "AB")?;
    let mut session = EditorSession::new(original, Revision(0));
    session.set_selection(caret("existing", 1))?;
    let child = Node::block("paragraph", Map::new(), vec![Node::text("new child")]);
    let callout = Node::block(
        "callout",
        Map::from_iter([("kind".into(), json!("info")), ("vendor".into(), json!(9))]),
        vec![child],
    );
    session.insert_content(ContentSlice::Blocks(vec![callout]))?;
    let blocks = session.document().blocks();
    assert_eq!(blocks[0].block_id(), Some("existing"));
    assert!(blocks[1].block_id().is_some());
    assert!(blocks[1].children()[0].block_id().is_some());
    assert_ne!(blocks[1].block_id(), blocks[1].children()[0].block_id());
    assert_eq!(
        blocks[1]
            .attrs
            .as_ref()
            .and_then(|attrs| attrs.get("vendor")),
        Some(&json!(9))
    );
    assert_eq!(blocks[2].inline_text(), "B");
    Ok(())
}

#[test]
fn replacing_a_marked_range_keeps_the_original_start_marks() -> TestResult {
    let source = Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":1},"content":[{"type":"paragraph","attrs":{"blockId":"p"},"content":[{"type":"text","text":"AB","marks":[{"type":"bold"}]},{"type":"text","text":"CD","marks":[{"type":"italic"}]}]}]}),
    )?;
    let mut session = EditorSession::new(source, Revision(0));
    session.set_selection(Selection {
        anchor: caret("p", 0).head,
        head: caret("p", 2).head,
    })?;
    session.insert_plain_text("X")?;
    assert_eq!(session.document().block_text("p")?, "XCD");
    let first = &session.document().block("p")?.children()[0];
    assert!(
        first
            .marks
            .as_deref()
            .unwrap_or_default()
            .iter()
            .any(|mark| mark.mark_type == "bold")
    );
    assert!(
        !first
            .marks
            .as_deref()
            .unwrap_or_default()
            .iter()
            .any(|mark| mark.mark_type == "italic")
    );
    Ok(())
}

#[test]
fn invalid_explicit_identity_is_rejected_rather_than_rewritten() -> TestResult {
    let original = Document::plain("p", "AB")?;
    let mut session = EditorSession::new(original.clone(), Revision(0));
    session.set_selection(caret("p", 1))?;
    let invalid = Node::block(
        "imageBlock",
        Map::from_iter([
            ("blockId".into(), json!(true)),
            ("src".into(), json!("host:asset")),
        ]),
        vec![],
    );
    assert!(
        session
            .insert_content(ContentSlice::Blocks(vec![invalid]))
            .is_err()
    );
    assert_eq!(session.document(), &original);
    assert_eq!(session.selection(), Some(&caret("p", 1)));
    assert!(!session.can_undo());
    Ok(())
}
#[test]
fn multiline_plain_paste_uses_a_bounded_transaction_for_many_paragraphs() -> TestResult {
    let original = Document::plain("p", "AB")?;
    let mut session = EditorSession::new(original.clone(), Revision(0));
    session.set_selection(caret("p", 1))?;
    session.insert_plain_text(&"line\n".repeat(1200))?;
    assert_eq!(session.document().blocks().len(), 1201);
    assert_eq!(session.document().blocks()[0].inline_text(), "Aline");
    assert_eq!(
        session
            .document()
            .blocks()
            .last()
            .ok_or("missing tail")?
            .inline_text(),
        "B"
    );
    assert_eq!(session.revision(), Revision(1));
    session.undo()?;
    assert_eq!(session.document(), &original);
    Ok(())
}

#[test]
fn table_insertion_records_first_cell_selection_without_losing_the_rich_tail() -> TestResult {
    let original = Document::plain("p", "left😀right")?;
    let mut session = EditorSession::new(original.clone(), Revision(7));
    session.set_selection(caret("p", 5))?;
    let table = Node::block(
        "table",
        Map::from_iter([("blockId".into(), json!("inserted-table"))]),
        vec![Node::block(
            "tableRow",
            Map::new(),
            vec![Node::block(
                "tableCell",
                Map::new(),
                vec![Node::paragraph("first-cell", "")],
            )],
        )],
    );
    let change = session
        .insert_content_with_selection(ContentSlice::Blocks(vec![table]), caret("first-cell", 0))?;
    let inserted = session.document().clone();
    assert_eq!(inserted.blocks()[0].inline_text(), "left😀");
    assert_eq!(inserted.blocks()[2].inline_text(), "right");
    assert_eq!(session.selection(), Some(&caret("first-cell", 0)));
    assert_eq!(change.replay_proof().verify(&original)?.0, inserted);
    session.undo()?;
    assert_eq!(session.document(), &original);
    assert_eq!(session.selection(), Some(&caret("p", 5)));
    session.redo()?;
    assert_eq!(session.document(), &inserted);
    assert_eq!(session.selection(), Some(&caret("first-cell", 0)));
    Ok(())
}

#[test]
fn explicit_invalid_insertion_selection_preserves_every_live_session_state() -> TestResult {
    let original = Document::plain("p", "AB")?;
    let mut session = EditorSession::new(original.clone(), Revision(3));
    session.set_selection(caret("p", 1))?;
    assert!(
        session
            .insert_content_with_selection(
                ContentSlice::Blocks(vec![image("new-image")]),
                caret("missing", 0),
            )
            .is_err()
    );
    assert_eq!(session.document(), &original);
    assert_eq!(session.revision(), Revision(3));
    assert_eq!(session.selection(), Some(&caret("p", 1)));
    assert!(!session.can_undo());
    assert!(session.take_pending_changes().is_empty());
    Ok(())
}

#[test]
fn explicit_atom_selection_resolves_the_inserted_path_and_survives_redo() -> TestResult {
    let original = Document::plain("p", "AB")?;
    let mut session = EditorSession::new(original.clone(), Revision(0));
    session.set_selection(caret("p", 1))?;
    session.insert_content_selecting_node(
        ContentSlice::Blocks(vec![image("new-image")]),
        "new-image",
    )?;
    let reference = session
        .node_selection()
        .ok_or("missing atom selection")?
        .clone();
    assert_eq!(reference.path, NodePath(vec![1]));
    session.undo()?;
    assert_eq!(session.document(), &original);
    assert_eq!(session.selection(), Some(&caret("p", 1)));
    session.redo()?;
    assert_eq!(session.node_selection(), Some(&reference));
    Ok(())
}

#[test]
fn selecting_missing_or_nonopaque_inserted_nodes_rejects_atomically() -> TestResult {
    for id in ["missing", "p"] {
        let original = Document::plain("p", "AB")?;
        let mut session = EditorSession::new(original.clone(), Revision(0));
        session.set_selection(caret("p", 1))?;
        assert!(
            session
                .insert_content_selecting_node(ContentSlice::Blocks(vec![image("new-image")]), id,)
                .is_err()
        );
        assert_eq!(session.document(), &original);
        assert_eq!(session.selection(), Some(&caret("p", 1)));
        assert!(!session.can_undo());
        assert!(session.take_pending_changes().is_empty());
    }
    Ok(())
}
