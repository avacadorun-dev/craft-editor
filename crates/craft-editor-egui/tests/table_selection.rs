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
