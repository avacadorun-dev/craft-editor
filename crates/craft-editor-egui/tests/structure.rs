use craft_editor_egui::{DocPoint, EditorSession, ListKind, MarkState, Selection};
use craft_editor_model::{Document, NodePath, Revision, ScalarOffset};
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn std::error::Error>>;
fn point(id: &str, offset: usize) -> DocPoint {
    DocPoint {
        block_id: id.into(),
        offset: ScalarOffset(offset),
    }
}
fn caret(id: &str, offset: usize) -> Selection {
    let point = point(id, offset);
    Selection {
        anchor: point.clone(),
        head: point,
    }
}
fn p(id: &str, text: &str) -> Value {
    json!({"type":"paragraph","attrs":{"blockId":id,"sourceRefs":["voice"]},"content":if text.is_empty(){vec![]}else{vec![json!({"type":"text","text":text,"marks":[{"type":"bold"}],"vendorLeaf":null})]}})
}
fn item(id: &str, text: &str) -> Value {
    json!({"type":"listItem","content":[p(id,text)]})
}
fn document(blocks: Vec<Value>) -> Result<Document, craft_editor_model::Error> {
    Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":4,"vendorRoot":null},"content":blocks}),
    )
}

#[test]
fn toolbar_list_toggle_preserves_rich_blocks_and_directed_selection_through_history() -> TestResult
{
    let source = document(vec![p("a", "А😀"), p("b", "B")])?;
    let mut session = EditorSession::new(source.clone(), Revision(4));
    let selection = Selection {
        anchor: point("b", 1),
        head: point("a", 1),
    };
    session.set_selection(selection.clone())?;
    let change = session.toggle_list(ListKind::Bullet)?;
    assert_eq!(session.list_state(ListKind::Bullet)?, MarkState::On);
    assert_eq!(session.list_state(ListKind::Ordered)?, MarkState::Off);
    assert_eq!(
        session.document().blocks()[0].children()[0].children()[0],
        source.blocks()[0]
    );
    assert_eq!(
        session.document().blocks()[0].children()[1].children()[0],
        source.blocks()[1]
    );
    assert_eq!(session.selection(), Some(&selection));
    assert_eq!(
        change.replay_proof().verify(&source)?.0,
        session.document().clone()
    );
    session.undo()?;
    assert_eq!(session.document(), &source);
    session.redo()?;
    assert_eq!(session.selection(), Some(&selection));
    session.toggle_list(ListKind::Bullet)?;
    assert_eq!(session.document().blocks(), source.blocks());
    Ok(())
}

#[test]
fn list_toggle_includes_the_final_block_at_offset_zero_like_the_original_toolbar() -> TestResult {
    let source = document(vec![p("a", "A"), p("b", "B")])?;
    let mut session = EditorSession::new(source.clone(), Revision(0));
    session.set_selection(Selection {
        anchor: point("a", 0),
        head: point("b", 0),
    })?;
    session.toggle_list(ListKind::Bullet)?;
    assert_eq!(session.document().blocks().len(), 1);
    assert_eq!(session.document().blocks()[0].children().len(), 2);
    assert_eq!(
        session.document().blocks()[0].children()[1].children()[0],
        source.blocks()[1]
    );
    Ok(())
}

#[test]
fn list_state_is_mixed_for_a_range_spanning_a_list_and_plain_paragraph() -> TestResult {
    let source = document(vec![
        json!({"type":"bulletList","content":[item("a","A")]}),
        p("b", "B"),
    ])?;
    let mut session = EditorSession::new(source, Revision(0));
    session.set_selection(Selection {
        anchor: point("a", 0),
        head: point("b", 1),
    })?;
    assert_eq!(session.list_state(ListKind::Bullet)?, MarkState::Mixed);
    Ok(())
}

#[test]
fn switching_list_kind_changes_the_whole_active_list_without_rewriting_item_ids() -> TestResult {
    let source = document(vec![
        json!({"type":"orderedList","attrs":{"blockId":"list","start":5,"vendor":null},"content":[item("a","A"),item("b","B"),item("c","C")]}),
    ])?;
    let mut session = EditorSession::new(source.clone(), Revision(0));
    session.set_selection(caret("b", 1))?;
    session.toggle_list(ListKind::Bullet)?;
    let blocks = session.document().blocks();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].block_id(), Some("list"));
    assert_eq!(blocks[0].node_type, "bulletList");
    assert_eq!(blocks[0].children(), source.blocks()[0].children());
    assert_eq!(blocks[0].attrs, source.blocks()[0].attrs);
    assert_eq!(session.selection(), Some(&caret("b", 1)));
    session.undo()?;
    assert_eq!(session.document(), &source);
    Ok(())
}

#[test]
fn lifting_a_partial_identified_list_allocates_only_additional_retained_wrapper_ids() -> TestResult
{
    let source = document(vec![
        json!({"type":"orderedList","attrs":{"blockId":"list","start":5,"vendor":null},"content":[item("a","A"),item("b","B"),item("c","C")]}),
    ])?;
    let mut session = EditorSession::new(source.clone(), Revision(0));
    session.set_selection(caret("b", 1))?;
    session.toggle_list(ListKind::Ordered)?;
    let blocks = session.document().blocks();
    assert_eq!(blocks.len(), 3);
    assert_eq!(blocks[0].block_id(), Some("list"));
    assert_eq!(blocks[1].block_id(), Some("b"));
    assert_ne!(blocks[0].block_id(), blocks[2].block_id());
    assert_eq!(
        blocks[2]
            .attrs
            .as_ref()
            .and_then(|attrs| attrs.get("start")),
        Some(&json!(7))
    );
    assert_eq!(
        blocks[2]
            .attrs
            .as_ref()
            .and_then(|attrs| attrs.get("vendor")),
        Some(&Value::Null)
    );
    session.undo()?;
    assert_eq!(session.document(), &source);
    Ok(())
}

#[test]
fn heading_and_code_list_toggle_converts_to_paragraph_without_an_empty_leading_item() -> TestResult
{
    for kind in ["heading", "codeBlock"] {
        let source = document(vec![
            json!({"type":kind,"attrs":{"blockId":"text-block","level":2,"language":"rust","sourceRefs":["voice"],"vendor":true},"content":[{"type":"text","text":"А😀text"}]}),
        ])?;
        let mut session = EditorSession::new(source.clone(), Revision(0));
        session.set_selection(caret("text-block", 2))?;
        session.toggle_list(ListKind::Bullet)?;
        let item = &session.document().blocks()[0].children()[0];
        assert_eq!(item.children().len(), 1);
        assert_eq!(item.children()[0].node_type, "paragraph");
        assert_eq!(item.children()[0].inline_text(), "А😀text");
        assert_eq!(item.children()[0].attrs, source.blocks()[0].attrs);
        assert_eq!(session.selection(), Some(&caret("text-block", 2)));
        session.undo()?;
        assert_eq!(session.document(), &source);
    }
    Ok(())
}

#[test]
fn indent_and_outdent_keep_item_identity_content_and_caret() -> TestResult {
    let source = document(vec![
        json!({"type":"bulletList","content":[item("a","A"),item("b","B"),item("c","C")]}),
    ])?;
    let mut session = EditorSession::new(source.clone(), Revision(0));
    session.set_selection(caret("b", 1))?;
    session.indent_list()?;
    assert_eq!(
        session
            .document()
            .node(&NodePath(vec![0, 0, 1, 0]))?
            .children()[0],
        source.node(&NodePath(vec![0, 1, 0]))?.clone()
    );
    assert_eq!(session.selection(), Some(&caret("b", 1)));
    session.outdent_list()?;
    assert_eq!(session.document(), &source);
    assert_eq!(session.selection(), Some(&caret("b", 1)));
    session.undo()?;
    assert!(session.document().node(&NodePath(vec![0, 0, 1]))?.node_type == "bulletList");
    Ok(())
}

#[test]
fn first_list_item_indent_rejects_without_mutation_or_history() -> TestResult {
    let source = document(vec![json!({"type":"bulletList","content":[item("a","A")]})])?;
    let mut session = EditorSession::new(source.clone(), Revision(0));
    session.set_selection(caret("a", 0))?;
    assert!(session.indent_list().is_err());
    assert_eq!(session.document(), &source);
    assert!(!session.can_undo());
    assert!(session.take_pending_changes().is_empty());
    Ok(())
}

#[test]
fn quote_toggle_and_lift_preserve_provenance_and_exact_selection() -> TestResult {
    let source = document(vec![p("a", "A"), p("b", "B")])?;
    let mut session = EditorSession::new(source.clone(), Revision(0));
    let selection = Selection {
        anchor: point("a", 0),
        head: point("b", 1),
    };
    session.set_selection(selection.clone())?;
    session.toggle_blockquote()?;
    assert_eq!(session.blockquote_state()?, MarkState::On);
    assert!(session.document().blocks()[0].block_id().is_some());
    assert_eq!(session.document().blocks()[0].children(), source.blocks());
    session.toggle_blockquote()?;
    assert_eq!(session.document(), &source);
    assert_eq!(session.selection(), Some(&selection));
    session.undo()?;
    assert_eq!(session.blockquote_state()?, MarkState::On);
    Ok(())
}

#[test]
fn lifting_middle_quote_block_keeps_wrapper_metadata_and_allocates_unique_portions() -> TestResult {
    let source = document(vec![
        json!({"type":"blockquote","attrs":{"blockId":"quote","sourceRefs":["wrapper-voice"],"vendor":true},"vendorEnvelope":null,"content":[p("a","A"),p("b","B"),p("c","C")]}),
    ])?;
    let mut session = EditorSession::new(source.clone(), Revision(0));
    session.set_selection(caret("b", 1))?;
    session.toggle_blockquote()?;
    let blocks = session.document().blocks();
    assert_eq!(blocks.len(), 3);
    assert_eq!(blocks[0].block_id(), Some("quote"));
    assert_ne!(blocks[0].block_id(), blocks[2].block_id());
    assert_eq!(blocks[2].extra, source.blocks()[0].extra);
    assert_eq!(
        blocks[1]
            .attrs
            .as_ref()
            .and_then(|attrs| attrs.get("sourceRefs")),
        Some(&json!(["voice", "wrapper-voice"]))
    );
    session.undo()?;
    assert_eq!(session.document(), &source);
    Ok(())
}

#[test]
fn full_quote_lift_keeps_wrapper_provenance_without_silently_losing_opaque_fields() -> TestResult {
    let source = document(vec![
        json!({"type":"blockquote","attrs":{"blockId":"quote","sourceRefs":["wrapper"]},"content":[p("a","A")]}),
    ])?;
    let mut session = EditorSession::new(source.clone(), Revision(0));
    session.set_selection(caret("a", 1))?;
    session.toggle_blockquote()?;
    assert_eq!(
        session.document().blocks()[0]
            .attrs
            .as_ref()
            .and_then(|attrs| attrs.get("sourceRefs")),
        Some(&json!(["voice", "wrapper"]))
    );
    session.undo()?;
    assert_eq!(session.document(), &source);
    let opaque = document(vec![
        json!({"type":"blockquote","attrs":{"blockId":"quote","vendor":true},"content":[p("a","A")]}),
    ])?;
    let mut session = EditorSession::new(opaque.clone(), Revision(0));
    session.set_selection(caret("a", 0))?;
    assert!(session.toggle_blockquote().is_err());
    assert_eq!(session.document(), &opaque);
    assert!(!session.can_undo());
    Ok(())
}

#[test]
fn callout_kind_change_preserves_unknown_attrs_and_rejects_invalid_kind() -> TestResult {
    let source = document(vec![
        json!({"type":"callout","attrs":{"blockId":"callout","kind":"info","vendor":null},"content":[p("a","A")]}),
    ])?;
    let mut session = EditorSession::new(source.clone(), Revision(0));
    session.set_selection(caret("a", 1))?;
    session.set_callout_kind("warning")?;
    let node = &session.document().blocks()[0];
    assert_eq!(
        node.attrs.as_ref().and_then(|attrs| attrs.get("kind")),
        Some(&json!("warning"))
    );
    assert_eq!(
        node.attrs.as_ref().and_then(|attrs| attrs.get("vendor")),
        Some(&Value::Null)
    );
    assert_eq!(node.children(), source.blocks()[0].children());
    let after = session.document().clone();
    assert!(session.set_callout_kind("unknown").is_err());
    assert_eq!(session.document(), &after);
    session.undo()?;
    assert_eq!(session.document(), &source);
    Ok(())
}

#[test]
fn enter_splits_a_list_item_with_rich_tail_and_one_history_entry() -> TestResult {
    let source = document(vec![
        json!({"type":"bulletList","content":[item("a","А😀tail")]}),
    ])?;
    let mut session = EditorSession::new(source.clone(), Revision(0));
    session.set_selection(caret("a", 2))?;
    let change = session.insert_paragraph()?;
    let list = &session.document().blocks()[0];
    assert_eq!(list.children().len(), 2);
    assert_eq!(list.children()[0].children()[0].inline_text(), "А😀");
    assert_eq!(list.children()[1].children()[0].inline_text(), "tail");
    let tail = &list.children()[1].children()[0];
    assert_eq!(
        tail.attrs
            .as_ref()
            .and_then(|attrs| attrs.get("sourceRefs")),
        Some(&json!(["voice"]))
    );
    assert_eq!(
        tail.children()[0].marks,
        source.blocks()[0].children()[0].children()[0].children()[0].marks
    );
    let after_caret = caret(tail.block_id().ok_or("missing new paragraph identity")?, 0);
    assert_eq!(session.selection(), Some(&after_caret));
    assert_eq!(change.after_revision, Revision(1));
    session.undo()?;
    assert_eq!(session.document(), &source);
    assert_eq!(session.selection(), Some(&caret("a", 2)));
    session.redo()?;
    assert_eq!(session.selection(), Some(&after_caret));
    Ok(())
}

#[test]
fn enter_on_an_empty_list_item_lifts_it_without_losing_neighboring_items() -> TestResult {
    let source = document(vec![
        json!({"type":"bulletList","content":[item("a","A"),item("empty",""),item("b","B")]}),
    ])?;
    let mut session = EditorSession::new(source.clone(), Revision(0));
    session.set_selection(caret("empty", 0))?;
    session.insert_paragraph()?;
    let blocks = session.document().blocks();
    assert_eq!(blocks.len(), 3);
    assert_eq!(blocks[0].children()[0], source.blocks()[0].children()[0]);
    assert_eq!(blocks[1].block_id(), Some("empty"));
    assert_eq!(blocks[2].children()[0], source.blocks()[0].children()[2]);
    assert_eq!(session.selection(), Some(&caret("empty", 0)));
    session.undo()?;
    assert_eq!(session.document(), &source);
    Ok(())
}

#[test]
fn hard_break_remains_inline_while_code_enter_retains_literal_newline() -> TestResult {
    let source = document(vec![p("a", "AB")])?;
    let mut session = EditorSession::new(source.clone(), Revision(0));
    session.set_selection(caret("a", 1))?;
    session.insert_hard_break()?;
    assert_eq!(session.document().blocks().len(), 1);
    assert_eq!(
        session.document().blocks()[0].children()[1].node_type,
        "hardBreak"
    );
    assert_eq!(session.selection(), Some(&caret("a", 2)));
    session.undo()?;
    assert_eq!(session.document(), &source);
    let source = document(vec![
        json!({"type":"codeBlock","attrs":{"blockId":"code","language":"rust"},"content":[{"type":"text","text":"AB"}]}),
    ])?;
    let mut session = EditorSession::new(source.clone(), Revision(0));
    session.set_selection(caret("code", 1))?;
    session.insert_paragraph()?;
    assert_eq!(session.document().blocks().len(), 1);
    assert_eq!(session.document().block_text("code")?, "A\nB");
    assert!(
        session.document().blocks()[0]
            .children()
            .iter()
            .all(|node| node.node_type == "text")
    );
    session.undo()?;
    assert_eq!(session.document(), &source);
    Ok(())
}

#[test]
fn enter_after_a_selected_image_keeps_the_image_and_enters_a_new_paragraph() -> TestResult {
    let source = document(vec![
        json!({"type":"imageBlock","attrs":{"blockId":"image","src":"host:asset","caption":"keep caption"}}),
    ])?;
    let mut session = EditorSession::new(source.clone(), Revision(0));
    let selected = session.document().node_ref(&NodePath(vec![0]))?;
    session.select_node(selected.clone())?;
    session.insert_paragraph()?;
    assert_eq!(session.document().blocks().len(), 2);
    assert_eq!(session.document().blocks()[0], source.blocks()[0]);
    assert_eq!(session.document().blocks()[1].node_type, "paragraph");
    assert!(session.node_selection().is_none());
    let caret = session
        .selection()
        .ok_or("missing paragraph caret")?
        .clone();
    session.undo()?;
    assert_eq!(session.document(), &source);
    assert_eq!(session.node_selection(), Some(&selected));
    session.redo()?;
    assert_eq!(session.selection(), Some(&caret));
    Ok(())
}
