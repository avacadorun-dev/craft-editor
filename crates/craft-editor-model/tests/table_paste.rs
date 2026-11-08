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
