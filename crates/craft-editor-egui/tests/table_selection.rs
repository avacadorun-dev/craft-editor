use craft_editor_egui::clipboard::{
    ClipboardBundle, ClipboardProvider, ClipboardResult, RichSlice,
};
use craft_editor_egui::{CellPoint, DocPoint, EditorSession, Error, Selection, TableSelection};
use craft_editor_model::{Document, Node, NodePath, Revision, ScalarOffset, TableGrid};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
type TestResult = Result<(), Box<dyn std::error::Error>>;
fn normalize(value: &mut Value) {
    match value {
        Value::Array(items) => {
            for item in items {
                normalize(item);
            }
        }
        Value::Object(object) => {
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
            for value in object.values_mut() {
                normalize(value);
            }
        }
        _ => {}
    }
}
fn selected_case(document: &Document, case: &Value) -> TestResultValue<TableSelection> {
    let table = &document.blocks()[0];
    let grid = TableGrid::new(table)?;
    fn has(node: &Node, id: &str) -> bool {
        node.block_id() == Some(id) || node.children().iter().any(|node| has(node, id))
    }
    let point = |value: &Value| -> TestResultValue<CellPoint> {
        let id = value[0].as_str().ok_or("missing id")?;
        let cell = grid
            .cells
            .iter()
            .find(|rect| has(&table.children()[rect.row].children()[rect.cell_index], id))
            .ok_or("missing cell")?;
        Ok(CellPoint {
            row: cell.row,
            column: cell.column,
        })
    };
    Ok(TableSelection {
        table_path: NodePath(vec![0]),
        anchor: point(&case["from"])?,
        head: point(case.get("to").unwrap_or(&case["from"]))?,
    })
}
type TestResultValue<T> = Result<T, Box<dyn std::error::Error>>;
fn fixture() -> TestResultValue<Document> {
    Document::from_json(json!({"type":"doc","attrs":{"schemaVersion":4},"content":[{"type":"table","attrs":{"blockId":"table"},"content":[{"type":"tableRow","content":[{"type":"tableHeader","content":[{"type":"paragraph","attrs":{"blockId":"a"},"content":[{"type":"text","text":"Alpha"}]}]},{"type":"tableHeader","content":[{"type":"paragraph","attrs":{"blockId":"b"},"content":[{"type":"text","text":"Bravo"}]}]}]},{"type":"tableRow","content":[{"type":"tableCell","content":[{"type":"paragraph","attrs":{"blockId":"c"},"content":[{"type":"text","text":"Charlie"}]}]},{"type":"tableCell","content":[{"type":"paragraph","attrs":{"blockId":"d"},"content":[{"type":"text","text":"Delta"}]}]}]}]}]})).map_err(Into::into)
}
fn rectangle() -> TableSelection {
    TableSelection {
        table_path: NodePath(vec![0]),
        anchor: CellPoint { row: 1, column: 1 },
        head: CellPoint { row: 0, column: 0 },
    }
}
#[test]
fn table_clipboard_session_matches_source_grid_paste_and_restores_original_selection() -> TestResult
{
    let corpus: Value = serde_json::from_str(include_str!(
        "../../craft-editor-model/tests/fixtures/source_table_paste.json"
    ))?;
    for case in corpus["cases"].as_array().ok_or("missing corpus")? {
        let original = Document::from_json(
            json!({"type":"doc","attrs":{"schemaVersion":4},"content":case["target"]}),
        )?;
        let mut session = EditorSession::new(original.clone(), Revision(0));
        let cells = selected_case(&original, case)?;
        if case["cellSelection"] == true {
            session.set_cell_selection(cells.clone())?;
        } else {
            let point = DocPoint {
                block_id: case["from"][0].as_str().ok_or("missing block")?.into(),
                offset: ScalarOffset(serde_json::from_value(case["from"][1].clone())?),
            };
            session.set_selection(Selection {
                anchor: point.clone(),
                head: point,
            })?;
        }
        let original_text = session.selection().cloned();
        let original_cells = session.cell_selection().cloned();
        let slice = RichSlice {
            content: serde_json::from_value(case["slice"].clone())?,
            open_start: serde_json::from_value(case["openStart"].clone())?,
            open_end: serde_json::from_value(case["openEnd"].clone())?,
        };
        let change = session
            .paste_bundle(&slice.bundle()?, 16 * 1024 * 1024)
            .map_err(|error| format!("{}: {error}", case["name"]))?;
        let after = session.document().clone();
        let after_text = session.selection().cloned();
        let after_cells = session.cell_selection().cloned();
        let mut actual = after.to_json()?;
        let mut expected = case["output"].clone();
        normalize(&mut actual);
        normalize(&mut expected);
        assert_eq!(actual, expected, "{}", case["name"]);
        assert_eq!(change.replay_proof().verify(&original)?.0, after);
        session.undo()?;
        assert_eq!(session.document(), &original);
        assert_eq!(session.selection(), original_text.as_ref());
        assert_eq!(session.cell_selection(), original_cells.as_ref());
        assert!(!session.can_undo());
        session.redo()?;
        assert_eq!(session.document(), &after);
        assert_eq!(session.selection(), after_text.as_ref());
        assert_eq!(session.cell_selection(), after_cells.as_ref());
    }
    Ok(())
}
#[test]
fn reversed_rectangular_copy_is_canonical_and_plain_cells_remain_in_document_order() -> TestResult {
    let original = fixture()?;
    let mut session = EditorSession::new(original.clone(), Revision(0));
    session.set_cell_selection(rectangle())?;
    let bundle = session.copy_selection()?.ok_or("missing cells")?;
    assert_eq!(bundle.plain, "Alpha\n\nBravo\n\nCharlie\n\nDelta");
    assert_eq!(session.selected_text()?, bundle.plain);
    assert!(
        bundle
            .html
            .as_ref()
            .is_some_and(|html| html.contains("<table"))
    );
    let slice = RichSlice::decode(bundle.rich.as_ref().ok_or("missing rich")?, 1_000_000)?;
    assert_eq!(slice.content[0], original.blocks()[0]);
    assert_eq!((slice.open_start, slice.open_end), (1, 1));
    assert_eq!(session.document(), &original);
    assert_eq!(session.cell_selection(), Some(&rectangle()));
    assert!(!session.can_undo());
    Ok(())
}
#[derive(Default)]
struct Provider(Mutex<ClipboardBundle>);
impl ClipboardProvider for Provider {
    fn read(&self) -> ClipboardResult<ClipboardBundle> {
        Ok(self
            .0
            .lock()
            .map_err(|_| craft_editor_egui::clipboard::ClipboardError::Read)?
            .clone())
    }
    fn write(&self, bundle: &ClipboardBundle) -> ClipboardResult<()> {
        *self
            .0
            .lock()
            .map_err(|_| craft_editor_egui::clipboard::ClipboardError::Write)? = bundle.clone();
        Ok(())
    }
    fn max_bytes(&self) -> usize {
        1_000_000
    }
}
#[test]
fn rectangular_typing_cut_and_clear_have_distinct_source_selection_semantics_and_single_undo()
-> TestResult {
    let original = fixture()?;
    for operation in [0, 1, 2] {
        let mut session = EditorSession::new(original.clone(), Revision(0));
        session.set_cell_selection(rectangle())?;
        let provider = Arc::new(Provider::default());
        session.set_clipboard_provider(Some(provider.clone()));
        let change = match operation {
            0 => session.insert_plain_text("New 😀")?,
            1 => session.clear_selected_cells()?,
            _ => session.cut_to_clipboard()?.ok_or("missing cut")?,
        };
        let after = session.document().clone();
        let written = after.blocks()[0]
            .children()
            .iter()
            .flat_map(Node::children)
            .flat_map(Node::children)
            .map(Node::inline_text)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>();
        assert_eq!(
            written,
            if operation == 0 {
                vec!["New 😀".to_owned()]
            } else {
                vec![]
            },
            "all other selected cell writing must be cleared"
        );
        let cells = session.cell_selection().cloned();
        let text = session.selection().cloned();
        assert_eq!(session.revision(), Revision(1));
        assert_eq!(
            after.blocks()[0].children()[0].children()[0].node_type,
            "tableHeader"
        );
        if operation == 1 {
            assert_eq!(cells, Some(rectangle()));
        } else {
            assert!(cells.is_none());
            assert!(text.is_some());
        }
        if operation == 0 {
            let head = session.selection().ok_or("missing caret")?;
            assert_eq!(after.block_text(&head.head.block_id)?, "New 😀");
            assert_eq!(head.head.offset, ScalarOffset(5));
        }
        assert_eq!(change.replay_proof().verify(&original)?.0, after);
        session.undo()?;
        assert_eq!(session.document(), &original);
        assert_eq!(session.cell_selection(), Some(&rectangle()));
        assert!(!session.can_undo());
        session.redo()?;
        assert_eq!(session.document(), &after);
        assert_eq!(session.cell_selection(), cells.as_ref());
        assert_eq!(session.selection(), text.as_ref());
    }
    Ok(())
}
#[test]
fn rectangle_changes_stale_deferred_paste_and_native_capacity_refuses_before_mutation() -> TestResult
{
    let original = fixture()?;
    let mut session = EditorSession::new(original.clone(), Revision(0));
    session.set_cell_selection(rectangle())?;
    let request = session.begin_paste(ClipboardBundle {
        plain: "New".into(),
        ..Default::default()
    })?;
    let craft_editor_egui::clipboard::ClipboardRequest::Paste { context, guard, .. } = request
    else {
        return Err("wrong request".into());
    };
    let prepared = context.prepare(1_000_000)?;
    let mut changed = rectangle();
    changed.head = CellPoint { row: 0, column: 1 };
    session.set_cell_selection(changed)?;
    assert!(session.finish_prepared_paste(guard, prepared).is_err());
    assert_eq!(session.document(), &original);
    session.set_table_column_limit(Some(63))?;
    session.set_selection(Selection {
        anchor: DocPoint {
            block_id: "a".into(),
            offset: ScalarOffset(0),
        },
        head: DocPoint {
            block_id: "a".into(),
            offset: ScalarOffset(0),
        },
    })?;
    let slice = RichSlice {
        content: serde_json::from_value(
            json!([{"type":"tableRow","content":[{"type":"tableCell","attrs":{"colspan":64},"content":[{"type":"paragraph","attrs":{"blockId":"source"},"content":[{"type":"text","text":"Wide"}]}]}]}]),
        )?,
        open_start: 1,
        open_end: 1,
    };
    assert!(matches!(
        session.paste_bundle(&slice.bundle()?, 1_000_000),
        Err(Error::TableColumnLimit {
            requested: 64,
            maximum: 63
        })
    ));
    let mut outside = EditorSession::new(Document::plain("outside", "")?, Revision(0));
    outside.set_table_column_limit(Some(63))?;
    let point = DocPoint {
        block_id: "outside".into(),
        offset: ScalarOffset(0),
    };
    outside.set_selection(Selection {
        anchor: point.clone(),
        head: point,
    })?;
    let wide = RichSlice {
        content: vec![Node::block(
            "table",
            serde_json::Map::from_iter([("blockId".into(), json!("wide"))]),
            slice.content.clone(),
        )],
        open_start: 0,
        open_end: 0,
    };
    assert!(matches!(
        outside.paste_bundle(&wide.bundle()?, 1_000_000),
        Err(Error::TableColumnLimit {
            requested: 64,
            maximum: 63
        })
    ));
    assert_eq!(outside.document().block_text("outside")?, "");
    assert_eq!(outside.revision(), Revision(0));
    assert_eq!(session.document(), &original);
    assert_eq!(session.revision(), Revision(0));
    assert!(!session.can_undo());
    Ok(())
}

#[test]
fn source_cell_enter_tab_and_delete_commands_preserve_writing_and_one_undo() -> TestResult {
    let cells = fixture()?;
    let mut input = cells.to_json()?;
    let rows = input["content"].as_array_mut().ok_or("missing blocks")?;
    rows.insert(
        0,
        serde_json::to_value(Node::paragraph("before", "Before"))?,
    );
    rows.push(serde_json::to_value(Node::paragraph("after", "After"))?);
    // Source fixture uses body cells; header behavior is checked separately.
    for cell in input["content"][1]["content"][0]["content"]
        .as_array_mut()
        .ok_or("missing cells")?
    {
        cell["type"] = "tableCell".into();
    }
    fn source_defaults(node: &mut Value) {
        if node["type"] == "paragraph" {
            node["attrs"]["sourceRefs"] = json!([]);
        }
        if node["type"] == "tableCell" || node["type"] == "tableHeader" {
            node["attrs"] = json!({"colspan":1,"rowspan":1,"colwidth":null,"align":null});
        }
        if let Some(children) = node.get_mut("content").and_then(Value::as_array_mut) {
            for child in children {
                source_defaults(child);
            }
        }
    }
    source_defaults(&mut input);
    let original = Document::from_json(input)?;
    let corpus: Value = serde_json::from_str(include_str!("fixtures/source-cell-keys.json"))?;
    for case in corpus["cases"].as_array().ok_or("missing cases")? {
        let key = case["key"].as_str().ok_or("missing key")?;
        if !matches!(key, "Enter" | "Tab" | "Shift-Tab" | "Backspace") {
            continue;
        }
        let name = case["name"].as_str().ok_or("missing name")?;
        let (anchor, head) = match name {
            "all" => (
                CellPoint { row: 0, column: 0 },
                CellPoint { row: 1, column: 1 },
            ),
            "partial" => (
                CellPoint { row: 0, column: 0 },
                CellPoint { row: 0, column: 1 },
            ),
            "reversed" => (
                CellPoint { row: 1, column: 1 },
                CellPoint { row: 0, column: 0 },
            ),
            _ => (
                CellPoint { row: 1, column: 1 },
                CellPoint { row: 1, column: 1 },
            ),
        };
        let selected = TableSelection {
            table_path: NodePath(vec![1]),
            anchor,
            head,
        };
        let mut session = EditorSession::new(original.clone(), Revision(0));
        session.set_cell_selection(selected.clone())?;
        match key {
            "Enter" => {
                session.insert_paragraph()?;
            }
            "Backspace" => {
                session.delete_cell_selection()?;
            }
            _ => {
                assert!(session.advance_table_cell(key == "Shift-Tab")?);
            }
        }
        if let Some(expected) = case.get("doc") {
            let mut expected = expected.clone();
            let mut actual = session.document().to_json()?;
            normalize(&mut actual);
            normalize(&mut expected);
            assert_eq!(actual, expected, "{name}/{key}");
        }
        if case["selection"]["type"] == "text" {
            let selected = session.selection().ok_or("missing caret")?;
            for (point, expected) in [
                (&selected.anchor, &case["from"]),
                (&selected.head, &case["to"]),
            ] {
                if let Some(id) = expected["blockId"].as_str() {
                    assert_eq!(point.block_id, id, "{name}/{key}");
                }
                assert_eq!(
                    point.offset.0,
                    expected["offset"].as_u64().ok_or("missing offset")? as usize,
                    "{name}/{key}"
                );
            }
        }
        if session.revision() != Revision(0) {
            session.undo()?;
            assert_eq!(session.document(), &original, "{name}/{key}");
            assert_eq!(session.cell_selection(), Some(&selected));
            assert!(!session.can_undo());
        }
    }
    Ok(())
}
#[test]
fn tab_after_a_header_only_table_creates_body_cells_without_cloning_source_metadata() -> TestResult
{
    let mut input = fixture()?.to_json()?;
    input["content"][0]["content"]
        .as_array_mut()
        .ok_or("missing rows")?
        .truncate(1);
    let original = Document::from_json(input)?;
    let mut session = EditorSession::new(original.clone(), Revision(0));
    session.set_cell_selection(TableSelection {
        table_path: NodePath(vec![0]),
        anchor: CellPoint { row: 0, column: 1 },
        head: CellPoint { row: 0, column: 1 },
    })?;
    assert!(session.advance_table_cell(false)?);
    assert!(
        session.document().blocks()[0].children()[1]
            .children()
            .iter()
            .all(|cell| cell.node_type == "tableCell")
    );
    session.undo()?;
    assert_eq!(session.document(), &original);
    Ok(())
}
#[test]
fn rectangular_mark_formatting_is_one_transaction_and_preserves_headers_and_direction() -> TestResult
{
    let original = fixture()?;
    let mut session = EditorSession::new(original.clone(), Revision(0));
    session.set_cell_selection(rectangle())?;
    session.toggle_mark(craft_editor_model::Mark::new("bold"))?;
    assert_eq!(
        session.mark_state("bold")?,
        craft_editor_egui::MarkState::On
    );
    assert_eq!(session.cell_selection(), Some(&rectangle()));
    assert_eq!(session.revision(), Revision(1));
    for cell in session.document().blocks()[0]
        .children()
        .iter()
        .flat_map(Node::children)
    {
        assert!(
            cell.children()[0].children()[0]
                .marks
                .as_ref()
                .is_some_and(|marks| marks.iter().any(|mark| mark.mark_type == "bold"))
        );
    }
    session.undo()?;
    assert_eq!(session.document(), &original);
    assert_eq!(session.cell_selection(), Some(&rectangle()));
    Ok(())
}

#[test]
fn selection_forms_stay_exclusive_through_setters_history_guard_cancel_and_reload() -> TestResult {
    let mut input = fixture()?.to_json()?;
    input["content"]
        .as_array_mut()
        .ok_or("missing blocks")?
        .push(json!({"type":"horizontalRule","attrs":{"blockId":"rule"}}));
    let original = Document::from_json(input)?;
    let mut session = EditorSession::new(original.clone(), Revision(0));
    let invariant = |session: &EditorSession| {
        usize::from(session.selection().is_some())
            + usize::from(session.node_selection().is_some())
            + usize::from(session.cell_selection().is_some())
    };
    assert_eq!(invariant(&session), 0);
    let text = Selection {
        anchor: DocPoint {
            block_id: "a".into(),
            offset: ScalarOffset(0),
        },
        head: DocPoint {
            block_id: "a".into(),
            offset: ScalarOffset(2),
        },
    };
    session.set_selection(text.clone())?;
    assert_eq!(invariant(&session), 1);
    session.select_node(session.document().node_ref(&NodePath(vec![1]))?)?;
    assert_eq!(invariant(&session), 1);
    session.set_cell_selection(rectangle())?;
    assert_eq!(invariant(&session), 1);
    let pending = session.begin_paste(ClipboardBundle {
        plain: "wrong".into(),
        ..Default::default()
    })?;
    session.cancel_clipboard_requests()?;
    assert_eq!(session.cell_selection(), Some(&rectangle()));
    assert_eq!(invariant(&session), 1);
    let craft_editor_egui::clipboard::ClipboardRequest::Paste { context, guard, .. } = pending
    else {
        return Err("wrong request".into());
    };
    assert!(
        session
            .finish_prepared_paste(guard, context.prepare(1_000_000)?)
            .is_err()
    );
    assert_eq!(session.document(), &original);
    assert_eq!(invariant(&session), 1);
    session.insert_paragraph()?;
    assert!(session.selection().is_some());
    assert_eq!(invariant(&session), 1);
    session.undo()?;
    assert_eq!(session.cell_selection(), Some(&rectangle()));
    assert_eq!(invariant(&session), 1);
    session.redo()?;
    assert!(session.selection().is_some());
    assert_eq!(invariant(&session), 1);
    session.set_cell_selection(rectangle())?;
    let reference = session.document().node_ref(&NodePath(vec![1]))?;
    session.apply_commands_with_node_selection(vec![], reference)?;
    assert!(session.node_selection().is_some());
    assert_eq!(invariant(&session), 1);
    session.set_cell_selection(rectangle())?;
    session.apply_commands_with_selection(vec![], text)?;
    assert!(session.selection().is_some());
    assert_eq!(invariant(&session), 1);
    let reloaded = EditorSession::new(
        Document::from_json(session.document().to_json()?)?,
        session.revision(),
    );
    assert_eq!(invariant(&reloaded), 0);
    assert_eq!(reloaded.document(), session.document());
    Ok(())
}

#[test]
fn source_cell_block_list_quote_and_insertion_policies_preserve_exact_rectangle_history()
-> TestResult {
    use craft_editor_egui::{ContentSlice, ListKind, TextBlockKind};
    let corpus: Value = serde_json::from_str(include_str!("fixtures/source-cell-authoring.json"))?;
    for case in corpus["cases"].as_array().ok_or("missing source cases")? {
        let original = Document::from_json(
            corpus[if case["codeInput"] == true {
                "codeInput"
            } else {
                "input"
            }]
            .clone(),
        )?;
        let name = case["name"].as_str().ok_or("missing name")?;
        let action = case["action"].as_str().ok_or("missing action")?;
        let (anchor, head) = match name {
            "all" => (
                CellPoint { row: 0, column: 0 },
                CellPoint { row: 1, column: 1 },
            ),
            "partial" => (
                CellPoint { row: 0, column: 0 },
                CellPoint { row: 0, column: 1 },
            ),
            _ => (
                CellPoint { row: 1, column: 1 },
                CellPoint { row: 0, column: 0 },
            ),
        };
        let selected = TableSelection {
            table_path: NodePath(vec![1]),
            anchor,
            head,
        };
        let mut session = EditorSession::new(original.clone(), Revision(0));
        session.set_cell_selection(selected.clone())?;
        let change = match action {
            "paragraph" => session.set_text_block_kind(TextBlockKind::Paragraph)?,
            "heading" => session.set_text_block_kind(TextBlockKind::Heading { level: 2 })?,
            "bullet" => session.toggle_list(ListKind::Bullet)?,
            "ordered" => session.toggle_list(ListKind::Ordered)?,
            "quote" => session.toggle_blockquote()?,
            _ => {
                let table = original.node(&selected.table_path)?;
                let grid = TableGrid::new(table)?;
                let cell = grid
                    .cell_at(head.row, head.column)
                    .ok_or("missing source head cell")?;
                let path = selected.table_path.child(cell.row).child(cell.cell_index);
                let mut source = &case["output"];
                for index in &path.0 {
                    source = &source["content"][*index];
                }
                // Raw source null identities are intentional: native insertion must allocate fresh
                // IDs before validating its full canonical candidate. The source identity plugin
                // is not mounted by the independent headless command probe.
                let nodes: Vec<Node> = serde_json::from_value(source["content"].clone())?;
                let slice = if action == "formula" || action == "writing" {
                    ContentSlice::Inline(nodes[0].children().to_vec())
                } else {
                    ContentSlice::Blocks(nodes)
                };
                session.insert_content(slice)?
            }
        };
        let mut actual = session.document().to_json()?;
        let mut expected = case["output"].clone();
        normalize(&mut actual);
        normalize(&mut expected);
        assert_eq!(actual, expected, "{name}/{action}");
        assert_eq!(
            change.replay_proof().verify(&original)?.0,
            *session.document()
        );
        if case["selection"]["type"] == "cell" {
            assert_eq!(session.cell_selection(), Some(&selected), "{name}/{action}");
        } else if case["selection"]["type"] == "node" {
            assert!(session.node_selection().is_some());
            assert!(session.cell_selection().is_none());
        } else {
            assert!(session.selection().is_some());
            assert!(session.cell_selection().is_none());
        }
        // Source appendTransaction may replace reset/default-null identities. Native must retain
        // every untouched input identity, including all other selected cells for head-only commands.
        if !matches!(action, "paragraph" | "heading") {
            for id in ["a", "b", "c", "d"] {
                let head_id = match (head.row, head.column) {
                    (0, 0) => "a",
                    (0, 1) => "b",
                    (1, 0) => "c",
                    _ => "d",
                };
                if id != head_id {
                    assert_eq!(
                        session.document().block(id)?,
                        original.block(id)?,
                        "{name}/{action}/{id}"
                    );
                }
            }
        }
        if session.revision() != Revision(0) {
            let after = session.document().clone();
            let text = session.selection().cloned();
            let cells = session.cell_selection().cloned();
            let node = session.node_selection().cloned();
            session.undo()?;
            assert_eq!(session.document(), &original);
            assert_eq!(session.cell_selection(), Some(&selected));
            assert!(!session.can_undo());
            session.redo()?;
            assert_eq!(session.document(), &after);
            assert_eq!(session.selection(), text.as_ref());
            assert_eq!(session.cell_selection(), cells.as_ref());
            assert_eq!(session.node_selection(), node.as_ref());
        }
    }
    Ok(())
}
#[test]
fn rectangular_repeat_list_toggle_repairs_source_identity_reset_bug_without_losing_selected_writing()
-> TestResult {
    let original = fixture()?;
    let mut session = EditorSession::new(original.clone(), Revision(0));
    session.set_cell_selection(rectangle())?;
    session.toggle_list(craft_editor_egui::ListKind::Bullet)?;
    assert_eq!(
        session.list_state(craft_editor_egui::ListKind::Bullet)?,
        craft_editor_egui::MarkState::On
    );
    session.toggle_list(craft_editor_egui::ListKind::Bullet)?;
    assert_eq!(session.document(), &original);
    assert_eq!(session.cell_selection(), Some(&rectangle()));
    for id in ["a", "b", "c", "d"] {
        assert_eq!(session.document().block(id)?, original.block(id)?);
    }
    session.undo()?;
    assert_eq!(
        session.list_state(craft_editor_egui::ListKind::Bullet)?,
        craft_editor_egui::MarkState::On
    );
    session.undo()?;
    assert_eq!(session.document(), &original);
    assert_eq!(session.cell_selection(), Some(&rectangle()));
    Ok(())
}

#[test]
fn explicit_table_deletion_preserves_original_text_selection_and_one_reversible_history()
-> TestResult {
    let original = fixture()?;
    let mut session = EditorSession::new(original.clone(), Revision(0));
    let selection = Selection {
        anchor: DocPoint {
            block_id: "d".into(),
            offset: ScalarOffset(2),
        },
        head: DocPoint {
            block_id: "d".into(),
            offset: ScalarOffset(2),
        },
    };
    session.set_selection(selection.clone())?;
    session.delete_current_table()?;
    assert_eq!(session.document().blocks().len(), 1);
    assert_eq!(session.document().blocks()[0].node_type, "paragraph");
    let after = session.document().to_json()?;
    let target = session.selection().cloned();
    assert!(target.is_some());
    session.undo()?;
    assert_eq!(session.document(), &original);
    assert_eq!(session.selection(), Some(&selection));
    assert!(!session.can_undo());
    session.redo()?;
    assert_eq!(session.document().to_json()?, after);
    assert_eq!(session.selection(), target.as_ref());
    Ok(())
}

#[test]
fn format_selector_reports_uniform_and_mixed_table_blocks_without_changing_direction() -> TestResult
{
    use craft_editor_egui::{TextBlockKind, TextBlockState};
    let original = fixture()?;
    let mut session = EditorSession::new(original.clone(), Revision(0));
    let cells = rectangle();
    session.set_cell_selection(cells.clone())?;
    assert_eq!(session.text_block_state()?, TextBlockState::Paragraph);
    session.set_text_block_kind(TextBlockKind::Heading { level: 4 })?;
    assert_eq!(
        session.text_block_state()?,
        TextBlockState::Heading { level: 4 }
    );
    let point = DocPoint {
        block_id: "a".into(),
        offset: ScalarOffset(0),
    };
    session.set_selection(Selection {
        anchor: point.clone(),
        head: point,
    })?;
    session.set_text_block_kind(TextBlockKind::Paragraph)?;
    session.set_cell_selection(cells.clone())?;
    let before = session.document().to_json()?;
    let revision = session.revision();
    assert_eq!(session.text_block_state()?, TextBlockState::Mixed);
    assert_eq!(session.cell_selection(), Some(&cells));
    assert_eq!(session.document().to_json()?, before);
    assert_eq!(session.revision(), revision);
    session.set_selection(Selection {
        anchor: DocPoint {
            block_id: "d".into(),
            offset: ScalarOffset(0),
        },
        head: DocPoint {
            block_id: "a".into(),
            offset: ScalarOffset(0),
        },
    })?;
    assert_eq!(session.text_block_state()?, TextBlockState::Mixed);
    Ok(())
}

#[test]
fn format_selector_preserves_legacy_integral_heading_attributes() -> TestResult {
    let canonical = json!({"type":"doc","attrs":{"schemaVersion":4},"content":[
        {"type":"heading","attrs":{"blockId":"h","level":4.0},"content":[{"type":"text","text":"Legacy"}]}
    ]});
    let mut session = EditorSession::new(Document::from_json(canonical.clone())?, Revision(0));
    let point = DocPoint {
        block_id: "h".into(),
        offset: ScalarOffset(0),
    };
    session.set_selection(Selection {
        anchor: point.clone(),
        head: point,
    })?;
    assert_eq!(
        session.text_block_state()?,
        craft_editor_egui::TextBlockState::Heading { level: 4 }
    );
    assert_eq!(session.document().to_json()?, canonical);
    assert_eq!(session.revision(), Revision(0));
    Ok(())
}
