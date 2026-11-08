use craft_editor_model::{Node, table_cells_from_slice};
use serde_json::{Value, json};
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
            for child in object.values_mut() {
                normalize(child);
            }
        }
        _ => {}
    }
}
#[test]
fn native_table_normalization_matches_actual_source_rows_and_open_context() -> TestResult {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/source_table_paste.json"))?;
    let cases = fixture["cases"].as_array().ok_or("missing source cases")?;
    assert_eq!(cases.len(), 12);
    for case in cases {
        let nodes: Vec<Node> = serde_json::from_value(case["slice"].clone())?;
        let before = serde_json::to_value(&nodes)?;
        let mut next = 0;
        let actual = table_cells_from_slice(
            &nodes,
            serde_json::from_value(case["openStart"].clone())?,
            serde_json::from_value(case["openEnd"].clone())?,
            || {
                next += 1;
                Ok(format!("fill-{next}"))
            },
        )
        .map_err(|error| format!("{}: {error}", case["name"]))?;
        if case["pastedCells"].is_null() {
            assert!(actual.is_none(), "{}", case["name"]);
        } else {
            let actual = actual.ok_or("source expected a table rectangle")?;
            let rows = actual
                .rows
                .iter()
                .map(|row| row.children().to_vec())
                .collect::<Vec<_>>();
            let mut output = json!({"width":actual.width,"height":actual.height,"rows":rows});
            let mut expected = case["pastedCells"].clone();
            normalize(&mut output);
            normalize(&mut expected);
            assert_eq!(output, expected, "{}", case["name"]);
        }
        assert_eq!(serde_json::to_value(&nodes)?, before);
    }
    Ok(())
}
#[test]
fn rectangular_normalization_preserves_unknown_cell_metadata_and_rejects_bad_ids() -> TestResult {
    let nodes: Vec<Node> = serde_json::from_value(
        json!([{"type":"tableRow","vendorRow":null,"content":[{"type":"tableCell","attrs":{"colspan":2,"vendorCell":{"keep":true}},"content":[{"type":"paragraph","attrs":{"blockId":"source","sourceRefs":["voice"]},"content":[{"type":"text","text":"A😀","vendorLeaf":null,"marks":[{"type":"bold","vendorMark":null}]}]}]}]},{"type":"tableRow","content":[{"type":"tableCell","content":[{"type":"paragraph","attrs":{"blockId":"other"},"content":[{"type":"text","text":"B"}]}]}]}]),
    )?;
    let original = serde_json::to_value(&nodes)?;
    let result =
        table_cells_from_slice(&nodes, 1, 1, || Ok("new-filler".into()))?.ok_or("missing table")?;
    assert_eq!(result.rows[0], nodes[0]);
    assert_eq!(result.rows[1].children()[0], nodes[1].children()[0]);
    assert!(table_cells_from_slice(&nodes, 1, 1, || Ok("source".into())).is_err());
    assert!(table_cells_from_slice(&nodes, 65, 0, || Ok("unused".into())).is_err());
    assert_eq!(serde_json::to_value(&nodes)?, original);
    Ok(())
}

fn cell_for_id<'a>(
    table: &'a Node,
    grid: &'a craft_editor_model::TableGrid,
    id: &str,
) -> Option<&'a craft_editor_model::CellRect> {
    fn contains(node: &Node, id: &str) -> bool {
        node.block_id() == Some(id) || node.children().iter().any(|child| contains(child, id))
    }
    grid.cells
        .iter()
        .find(|rect| contains(&table.children()[rect.row].children()[rect.cell_index], id))
}
#[test]
fn table_destination_matches_source_tiling_growth_and_crossing_span_isolation() -> TestResult {
    use craft_editor_model::{
        CellSelection, Document, Editor, NodePath, PastedCells, Revision, TableGrid,
        TablePasteOptions, Transaction, paste_table_cells,
    };
    let fixture: Value = serde_json::from_str(include_str!("fixtures/source_table_paste.json"))?;
    let mut checked = 0;
    for case in fixture["cases"].as_array().ok_or("missing source cases")? {
        if case["pastedCells"].is_null() && case["cellSelection"] != true {
            continue;
        }
        let original = Document::from_json(
            json!({"type":"doc","attrs":{"schemaVersion":4},"content":case["target"]}),
        )?;
        let target = original.node_ref(&NodePath(vec![0]))?;
        let grid = TableGrid::new(&target.expected)?;
        let first = cell_for_id(
            &target.expected,
            &grid,
            case["from"][0].as_str().ok_or("missing source start")?,
        )
        .ok_or("missing start cell")?;
        let last = cell_for_id(
            &target.expected,
            &grid,
            case.get("to").unwrap_or(&case["from"])[0]
                .as_str()
                .ok_or("missing source end")?,
        )
        .ok_or("missing end cell")?;
        let selection = CellSelection {
            top: first.row.min(last.row),
            left: first.column.min(last.column),
            bottom: first.bottom().max(last.bottom()),
            right: first.right().max(last.right()),
        };
        let content: Vec<Node> = serde_json::from_value(case["slice"].clone())?;
        let open_start = serde_json::from_value(case["openStart"].clone())?;
        let open_end = serde_json::from_value(case["openEnd"].clone())?;
        let mut next = 0;
        let mut allocate = || {
            next += 1;
            Ok(format!("paste-{next}"))
        };
        let cells = match table_cells_from_slice(&content, open_start, open_end, &mut allocate)? {
            Some(cells) => cells,
            None => PastedCells::from_writing(&content, open_start, open_end, &mut allocate)?,
        };
        let (command, after_selection) = paste_table_cells(
            &original,
            &target,
            selection,
            &cells,
            TablePasteOptions {
                repeat: case["cellSelection"] == true,
                max_expanded_bytes: 16 * 1024 * 1024,
            },
            &mut allocate,
        )
        .map_err(|error| format!("{} destination: {error}", case["name"]))?;
        let mut editor = Editor::new(original.clone(), Revision(0));
        let change = editor.apply(Transaction {
            base_revision: Revision(0),
            commands: vec![command],
        })?;
        let after = editor.document().clone();
        let mut actual = after.to_json()?;
        let mut expected = case["output"].clone();
        normalize(&mut actual);
        normalize(&mut expected);
        assert_eq!(actual, expected, "{}", case["name"]);
        assert_eq!(change.replay_proof().verify(&original)?.0, after);
        let after_grid = TableGrid::new(&after.blocks()[0])?;
        assert!(
            after_grid
                .cell_at(after_selection.top, after_selection.left)
                .is_some()
        );
        assert!(
            after_grid
                .cell_at(after_selection.bottom - 1, after_selection.right - 1)
                .is_some()
        );
        editor.undo(editor.revision())?;
        assert_eq!(editor.document(), &original);
        editor.redo(editor.revision())?;
        assert_eq!(editor.document(), &after);
        checked += 1;
    }
    assert_eq!(checked, 11);
    Ok(())
}

#[test]
fn table_paste_preserves_opaque_context_untouched_notes_and_generated_identity_guards() -> TestResult
{
    use craft_editor_model::{
        CellSelection, Document, Editor, NodePath, Revision, ScalarOffset, TablePasteOptions,
        Transaction, paste_table_cells,
    };
    let original = Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":4},"content":[{"type":"table","attrs":{"blockId":"table","ownerTable":null},"content":[{"type":"tableRow","ownerRow":null,"content":[{"type":"tableCell","content":[{"type":"paragraph","attrs":{"blockId":"replace"},"content":[{"type":"text","text":"Old"}]}]},{"type":"tableCell","attrs":{"ownerCell":null},"content":[{"type":"paragraph","attrs":{"blockId":"keep","sourceRefs":["old-voice"]},"content":[{"type":"text","text":"Keep 😀 note","ownerLeaf":null}]}]}]}]},{"type":"paragraph","attrs":{"blockId":"outside"},"content":[{"type":"text","text":"Keep outside"}]}]}),
    )?;
    let input: Vec<Node> = serde_json::from_value(
        json!([{"type":"table","attrs":{"blockId":"source-table","incomingTable":{"retain":true},"sourceRefs":["voice-table"]},"content":[{"type":"tableRow","incomingRow":null,"marks":null,"content":[{"type":"tableCell","attrs":{"incomingCell":null},"content":[{"type":"paragraph","attrs":{"blockId":"source","sourceRefs":["voice-cell"]},"content":[{"type":"text","text":"New 😀","marks":[{"type":"bold","vendor":null}],"vendorLeaf":null}]}]}]}]}]),
    )?;
    let source =
        table_cells_from_slice(&input, 0, 0, || Ok("unused".into()))?.ok_or("missing source")?;
    let target = original.node_ref(&NodePath(vec![0]))?;
    let selection = CellSelection {
        top: 0,
        left: 0,
        bottom: 1,
        right: 1,
    };
    let options = TablePasteOptions {
        repeat: true,
        max_expanded_bytes: 1_000_000,
    };
    let mut next = 0;
    let (command, _) = paste_table_cells(&original, &target, selection, &source, options, || {
        next += 1;
        Ok(format!("fresh-{next}"))
    })?;
    let mut editor = Editor::new(original.clone(), Revision(0));
    let change = editor.apply(Transaction {
        base_revision: Revision(0),
        commands: vec![command],
    })?;
    let after = editor.document();
    assert_eq!(after.block("keep")?, original.block("keep")?);
    assert_eq!(after.block("outside")?, original.block("outside")?);
    for id in ["keep", "outside"] {
        let mapped = change
            .mappings
            .iter()
            .try_fold((id.to_owned(), ScalarOffset(5)), |(id, offset), map| {
                map.map_existing_caret(&id, offset, true)
            });
        assert_eq!(mapped, Some((id.into(), ScalarOffset(5))));
    }
    assert!(
        change
            .mappings
            .iter()
            .try_fold(
                ("replace".to_owned(), ScalarOffset(1)),
                |(id, offset), map| map.map_existing_caret(&id, offset, true)
            )
            .is_none()
    );
    let wire = after.to_json()?;
    assert_eq!(wire["content"][0]["attrs"]["ownerTable"], Value::Null);
    assert_eq!(
        wire["content"][0]["attrs"]["incomingTable"],
        json!({"retain":true})
    );
    assert_eq!(wire["content"][0]["content"][0]["incomingRow"], Value::Null);
    assert_eq!(
        wire["content"][0]["content"][0].get("marks"),
        Some(&Value::Null)
    );
    let inserted = &after.blocks()[0].children()[0].children()[0].children()[0];
    assert_eq!(inserted.inline_text(), "New 😀");
    assert_eq!(
        inserted
            .attrs
            .as_ref()
            .and_then(|attrs| attrs.get("sourceRefs")),
        Some(&json!(["voice-cell", "voice-table"]))
    );
    assert_eq!(
        inserted.children()[0].extra.get("vendorLeaf"),
        Some(&Value::Null)
    );
    assert_eq!(
        inserted.children()[0]
            .marks
            .as_ref()
            .and_then(|marks| marks.first())
            .and_then(|mark| mark.extra.get("vendor")),
        Some(&Value::Null)
    );
    assert!(
        paste_table_cells(&original, &target, selection, &source, options, || Ok(
            "keep".into()
        ))
        .is_err()
    );
    assert!(
        paste_table_cells(
            &original,
            &target,
            selection,
            &source,
            TablePasteOptions {
                max_expanded_bytes: 1,
                ..options
            },
            || Ok("fresh".into())
        )
        .is_err()
    );
    let mut stale = target;
    stale
        .expected
        .extra
        .insert("wrongReference".into(), json!(true));
    let mut calls = 0;
    assert!(
        paste_table_cells(&original, &stale, selection, &source, options, || {
            calls += 1;
            Ok("new".into())
        })
        .is_err()
    );
    assert_eq!(calls, 0);
    editor.undo(editor.revision())?;
    assert_eq!(editor.document(), &original);
    Ok(())
}

#[test]
fn generic_paste_keeps_wide_spans_and_resolves_wholly_covered_last_row_without_source_crash()
-> TestResult {
    use craft_editor_model::{
        CellSelection, Document, Editor, NodePath, Revision, TableGrid, TablePasteOptions,
        Transaction, paste_table_cells,
    };
    for (columns, rowspan) in [(64, 1), (1, 2)] {
        let original = Document::from_json(
            json!({"type":"doc","attrs":{"schemaVersion":4},"content":[{"type":"table","attrs":{"blockId":"table"},"content":[{"type":"tableRow","content":[{"type":"tableCell","content":[{"type":"paragraph","attrs":{"blockId":"old"},"content":[{"type":"text","text":"Old"}]}]}]}]}]}),
        )?;
        let input: Vec<Node> = serde_json::from_value(
            json!([{"type":"tableRow","content":[{"type":"tableCell","attrs":{"colspan":columns,"rowspan":rowspan},"content":[{"type":"paragraph","attrs":{"blockId":"source"},"content":[{"type":"text","text":"Wide or tall"}]}]}]}]),
        )?;
        let source =
            table_cells_from_slice(&input, 1, 1, || Ok("fill".into()))?.ok_or("missing cells")?;
        let target = original.node_ref(&NodePath(vec![0]))?;
        let mut next = 0;
        let (command, selection) = paste_table_cells(
            &original,
            &target,
            CellSelection {
                top: 0,
                left: 0,
                bottom: 1,
                right: 1,
            },
            &source,
            TablePasteOptions {
                repeat: false,
                max_expanded_bytes: 1_000_000,
            },
            || {
                next += 1;
                Ok(format!("new-{next}"))
            },
        )?;
        let mut editor = Editor::new(original.clone(), Revision(0));
        editor.apply(Transaction {
            base_revision: Revision(0),
            commands: vec![command],
        })?;
        let grid = TableGrid::new(&editor.document().blocks()[0])?;
        assert_eq!((grid.columns, grid.rows), (columns, rowspan));
        assert!(
            grid.cell_at(selection.bottom - 1, selection.right - 1)
                .is_some()
        );
        editor.undo(editor.revision())?;
        assert_eq!(editor.document(), &original);
    }
    Ok(())
}
