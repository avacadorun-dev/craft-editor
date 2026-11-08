#![expect(
    clippy::expect_used,
    reason = "Synthetic table fixtures must construct successfully"
)]

use craft_editor_model::{
    CellAlignment, CellSelection, Command, Document, Editor, Node, NodePath, Revision, TableGrid,
    Transaction, delete_table_column, delete_table_row, empty_table_cell, insert_table_column,
    insert_table_row, merge_table_cells, set_table_cell_alignment, set_table_column_width,
    split_table_cell,
};
use serde_json::{Map, Value, json};

fn cell(id: &str, text: &str, header: bool) -> Node {
    let mut cell = empty_table_cell(id, header);
    cell.content = Some(vec![Node::paragraph(id, text)]);
    cell.extra.insert("vendor".into(), json!({"keep":null}));
    cell.attrs
        .get_or_insert_default()
        .insert("custom".into(), json!([1, "two"]));
    cell
}
fn row(cells: Vec<Node>) -> Node {
    Node::block("tableRow", Map::new(), cells)
}
fn table(rows: Vec<Node>) -> Node {
    let mut table = Node::block(
        "table",
        Map::from_iter([
            ("blockId".into(), json!("table")),
            ("vendor".into(), json!({"root":true})),
        ]),
        rows,
    );
    table.extra.insert("opaque".into(), json!([null, true]));
    table
}
fn doc(table: Node) -> Document {
    Document::from_root(Node::block(
        "doc",
        Map::from_iter([
            ("schemaVersion".into(), json!(2)),
            ("keep".into(), json!("root")),
        ]),
        vec![table],
    ))
    .expect("table document")
}
fn square() -> Document {
    doc(table(vec![
        row(vec![cell("a", "A", true), cell("b", "B", true)]),
        row(vec![cell("c", "C", false), cell("d", "D", false)]),
    ]))
}
fn target(document: &Document) -> craft_editor_model::NodeRef {
    document
        .node_ref(&NodePath(vec![0]))
        .expect("table reference")
}
fn apply(editor: &mut Editor, command: Command) {
    editor
        .apply(Transaction {
            base_revision: editor.revision(),
            commands: vec![command],
        })
        .expect("table edit");
}
fn attrs(node: &mut Node, values: Value) {
    node.attrs
        .get_or_insert_default()
        .extend(serde_json::from_value::<Map<String, Value>>(values).expect("fixture attrs"));
}
fn spanning() -> Document {
    let mut a = cell("a", "Span content", true);
    attrs(&mut a, json!({"rowspan":2}));
    doc(table(vec![
        row(vec![a, cell("b", "B", false)]),
        row(vec![cell("c", "C", false)]),
    ]))
}

#[test]
fn canonical_span_one_hundred_and_total_width_over_one_hundred_are_preserved() {
    let mut a = cell("a", "wide", false);
    attrs(&mut a, json!({"colspan":100}));
    let document = doc(table(vec![row(vec![a, cell("b", "outside", false)])]));
    let grid = TableGrid::new(&document.blocks()[0]).expect("wide grid");
    assert_eq!(grid.columns, 101);
    assert_eq!(grid.cells[0].colspan, 100);
    assert_eq!(
        document.to_json().expect("json")["content"][0]["content"][0]["content"][0]["attrs"]["colspan"],
        json!(100)
    );
}
#[test]
fn grid_validation_rejects_ragged_overlap_invalid_span_and_out_of_bounds_rowspan() {
    let mut a = cell("a", "A", false);
    attrs(&mut a, json!({"rowspan":2}));
    assert!(
        TableGrid::new(&table(vec![
            row(vec![a.clone(), cell("b", "B", false)]),
            row(vec![])
        ]))
        .is_err()
    );
    let mut middle = cell("m", "M", false);
    attrs(&mut middle, json!({"rowspan":2}));
    let mut overlap = cell("o", "O", false);
    attrs(&mut overlap, json!({"colspan":2}));
    assert!(
        TableGrid::new(&table(vec![
            row(vec![cell("l", "L", false), middle, cell("r", "R", false)]),
            row(vec![overlap, cell("s", "S", false)])
        ]))
        .is_err()
    );
    for invalid in [0, 101] {
        let mut bad = cell("bad", "bad", false);
        attrs(&mut bad, json!({"colspan":invalid}));
        assert!(TableGrid::new(&table(vec![row(vec![bad])])).is_err());
    }
    attrs(&mut a, json!({"rowspan":3}));
    assert!(TableGrid::new(&table(vec![row(vec![a]), row(vec![])])).is_err());
}
#[test]
fn insert_and_delete_row_preserve_unrelated_metadata_and_undo_exactly() {
    let original = square();
    let mut editor = Editor::new(original.clone(), Revision(1));
    let command = insert_table_row(
        editor.document(),
        &target(editor.document()),
        1,
        row(vec![
            empty_table_cell("new1", false),
            empty_table_cell("new2", false),
        ]),
    )
    .expect("row command");
    apply(&mut editor, command);
    assert_eq!(
        TableGrid::new(&editor.document().blocks()[0])
            .expect("grid")
            .rows,
        3
    );
    assert_eq!(
        editor.document().to_json().expect("json")["content"][0]["attrs"],
        original.to_json().expect("original")["content"][0]["attrs"]
    );
    let command =
        delete_table_row(editor.document(), &target(editor.document()), 1).expect("delete row");
    apply(&mut editor, command);
    assert_eq!(editor.document(), &original);
    editor.undo(editor.revision()).expect("undo delete");
    editor.undo(editor.revision()).expect("undo insert");
    assert_eq!(editor.document(), &original);
}
#[test]
fn insert_through_rowspan_extends_it_and_delete_origin_keeps_content() {
    let original = spanning();
    let mut editor = Editor::new(original.clone(), Revision(1));
    let command = insert_table_row(
        editor.document(),
        &target(editor.document()),
        1,
        row(vec![empty_table_cell("new", false)]),
    )
    .expect("inside rowspan");
    apply(&mut editor, command);
    let grid = TableGrid::new(&editor.document().blocks()[0]).expect("grid");
    assert_eq!((grid.rows, grid.columns, grid.cells[0].rowspan), (3, 2, 3));
    let command =
        delete_table_row(editor.document(), &target(editor.document()), 0).expect("delete origin");
    apply(&mut editor, command);
    assert_eq!(
        editor
            .document()
            .block_text("a")
            .expect("moved original content"),
        "Span content"
    );
    assert_eq!(
        TableGrid::new(&editor.document().blocks()[0])
            .expect("grid")
            .cells[0]
            .rowspan,
        2
    );
}
#[test]
fn covered_empty_rows_are_valid_and_span_growth_past_one_hundred_refuses() {
    let mut a = cell("a", "A", false);
    attrs(&mut a, json!({"rowspan":100}));
    let mut rows = vec![row(vec![a])];
    rows.extend((1..100).map(|_| row(vec![])));
    let document = doc(table(rows));
    assert_eq!(
        TableGrid::new(&document.blocks()[0]).expect("grid").rows,
        100
    );
    assert!(insert_table_row(&document, &target(&document), 50, row(vec![])).is_err());
    assert_eq!(document.block_text("a").expect("preserved"), "A");
}
#[test]
fn column_insertion_and_deletion_update_span_width_arrays_without_losing_content() {
    let mut a = cell("a", "A", false);
    attrs(&mut a, json!({"colspan":2,"colwidth":[100,200]}));
    let mut b = cell("b", "B", false);
    attrs(&mut b, json!({"colwidth":[300]}));
    let original = doc(table(vec![row(vec![a, b])]));
    let mut editor = Editor::new(original.clone(), Revision(1));
    let command = insert_table_column(editor.document(), &target(editor.document()), 1, vec![])
        .expect("widen spanning cell");
    apply(&mut editor, command);
    assert_eq!(
        editor.document().to_json().expect("json")["content"][0]["content"][0]["content"][0]["attrs"]
            ["colwidth"],
        json!([100, 0, 200])
    );
    let command =
        delete_table_column(editor.document(), &target(editor.document()), 1).expect("shrink span");
    apply(&mut editor, command);
    assert_eq!(editor.document(), &original);
    assert_eq!(editor.document().block_text("a").expect("source"), "A");
}
#[test]
fn column_boundary_insertion_consumes_exact_host_supplied_cells() {
    let document = square();
    let mut editor = Editor::new(document.clone(), Revision(1));
    assert!(insert_table_column(&document, &target(&document), 1, vec![]).is_err());
    let command = insert_table_column(
        editor.document(),
        &target(editor.document()),
        1,
        vec![empty_table_cell("n1", true), empty_table_cell("n2", false)],
    )
    .expect("new column");
    apply(&mut editor, command);
    let grid = TableGrid::new(&editor.document().blocks()[0]).expect("grid");
    assert_eq!(grid.columns, 3);
    assert_eq!(
        editor.document().blocks()[0].children()[1].children()[1].children()[0].block_id(),
        Some("n2")
    );
}
#[test]
fn width_and_alignment_edits_preserve_attributes_and_consistent_grid_widths() {
    let mut editor = Editor::new(square(), Revision(1));
    let command = set_table_column_width(editor.document(), &target(editor.document()), 0, 250)
        .expect("width");
    apply(&mut editor, command);
    let command = set_table_cell_alignment(
        editor.document(),
        &target(editor.document()),
        1,
        1,
        Some(CellAlignment::Right),
    )
    .expect("align");
    apply(&mut editor, command);
    let json = editor.document().to_json().expect("json");
    assert_eq!(
        json["content"][0]["content"][0]["content"][0]["attrs"]["colwidth"],
        json!([250])
    );
    assert_eq!(
        json["content"][0]["content"][1]["content"][0]["attrs"]["colwidth"],
        json!([250])
    );
    assert_eq!(
        json["content"][0]["content"][1]["content"][1]["attrs"]["align"],
        json!("right")
    );
    assert_eq!(
        json["content"][0]["content"][1]["content"][1]["attrs"]["custom"],
        json!([1, "two"])
    );
}
#[test]
fn merge_and_split_keep_all_existing_block_ids_content_and_opaque_metadata() {
    let original = square();
    let mut editor = Editor::new(original.clone(), Revision(1));
    let command = merge_table_cells(
        editor.document(),
        &target(editor.document()),
        CellSelection {
            top: 0,
            left: 0,
            bottom: 2,
            right: 2,
        },
    )
    .expect("merge");
    apply(&mut editor, command);
    let merged = &editor.document().blocks()[0].children()[0].children()[0];
    assert_eq!(
        merged
            .children()
            .iter()
            .filter_map(Node::block_id)
            .collect::<Vec<_>>(),
        vec!["a", "b", "c", "d"]
    );
    assert!(
        editor.document().blocks()[0].children()[1]
            .children()
            .is_empty()
    );
    let command = split_table_cell(
        editor.document(),
        &target(editor.document()),
        0,
        0,
        vec![
            empty_table_cell("s1", true),
            empty_table_cell("s2", true),
            empty_table_cell("s3", true),
        ],
    )
    .expect("split");
    apply(&mut editor, command);
    let grid = TableGrid::new(&editor.document().blocks()[0]).expect("grid");
    assert_eq!(grid.cells.len(), 4);
    for (id, text) in [("a", "A"), ("b", "B"), ("c", "C"), ("d", "D")] {
        assert_eq!(
            editor
                .document()
                .block_text(id)
                .expect("retained original block"),
            text
        );
    }
    assert_eq!(
        editor.document().blocks()[0].children()[1].children()[1]
            .extra
            .get("vendor"),
        Some(&json!({"keep":null}))
    );
    editor.undo(editor.revision()).expect("undo split");
    editor.undo(editor.revision()).expect("undo merge");
    assert_eq!(editor.document(), &original);
}
#[test]
fn merge_refuses_partial_spans_conflicting_opaque_metadata_and_discarded_cell_ids() {
    let mut wide = cell("wide", "wide", false);
    attrs(&mut wide, json!({"colspan":2}));
    let document = doc(table(vec![row(vec![wide, cell("right", "right", false)])]));
    assert!(
        merge_table_cells(
            &document,
            &target(&document),
            CellSelection {
                top: 0,
                left: 1,
                bottom: 1,
                right: 3
            }
        )
        .is_err()
    );
    let mut root = square().root().clone();
    root.content.as_mut().expect("blocks")[0]
        .content
        .as_mut()
        .expect("rows")[1]
        .content
        .as_mut()
        .expect("cells")[0]
        .extra
        .insert("different".into(), json!(true));
    let document = Document::from_root(root).expect("opaque fixture");
    assert!(
        merge_table_cells(
            &document,
            &target(&document),
            CellSelection {
                top: 0,
                left: 0,
                bottom: 2,
                right: 2
            }
        )
        .is_err()
    );
    let mut root = square().root().clone();
    attrs(
        &mut root.content.as_mut().expect("blocks")[0]
            .content
            .as_mut()
            .expect("rows")[1]
            .content
            .as_mut()
            .expect("cells")[0],
        json!({"blockId":"wrapper-cell"}),
    );
    let document = Document::from_root(root).expect("cell identity");
    assert!(
        merge_table_cells(
            &document,
            &target(&document),
            CellSelection {
                top: 0,
                left: 0,
                bottom: 2,
                right: 2
            }
        )
        .is_err()
    );
}
#[test]
fn split_of_identified_cell_requires_fresh_wrapper_ids_and_preserves_original_identity() {
    let mut a = cell("a", "A", false);
    attrs(&mut a, json!({"colspan":2,"blockId":"cell-original"}));
    let document = doc(table(vec![row(vec![a])]));
    assert!(
        split_table_cell(
            &document,
            &target(&document),
            0,
            0,
            vec![empty_table_cell("new-p", false)]
        )
        .is_err()
    );
    let mut fresh = empty_table_cell("new-p", false);
    attrs(&mut fresh, json!({"blockId":"cell-new"}));
    let command =
        split_table_cell(&document, &target(&document), 0, 0, vec![fresh]).expect("fresh identity");
    let mut editor = Editor::new(document, Revision(1));
    apply(&mut editor, command);
    assert!(editor.document().block("cell-original").is_ok());
    assert!(editor.document().block("cell-new").is_ok());
    assert_eq!(editor.document().block_text("a").expect("content"), "A");
}
#[test]
fn inconsistent_widths_and_wrong_width_lengths_are_rejected_before_mutation() {
    let mut a = cell("a", "A", false);
    attrs(&mut a, json!({"colwidth":[100]}));
    let mut b = cell("b", "B", false);
    attrs(&mut b, json!({"colwidth":[200]}));
    assert!(TableGrid::new(&table(vec![row(vec![a]), row(vec![b])])).is_err());
    let mut wrong = cell("w", "W", false);
    attrs(&mut wrong, json!({"colspan":2,"colwidth":[100]}));
    assert!(TableGrid::new(&table(vec![row(vec![wrong])])).is_err());
}
#[test]
fn last_row_column_and_stale_references_do_not_destroy_author_data() {
    let document = doc(table(vec![row(vec![cell("a", "A", false)])]));
    assert!(delete_table_row(&document, &target(&document), 0).is_err());
    assert!(delete_table_column(&document, &target(&document), 0).is_err());
    let old = target(&document);
    let mut editor = Editor::new(document, Revision(1));
    let command = set_table_column_width(editor.document(), &target(editor.document()), 0, 100)
        .expect("width");
    apply(&mut editor, command);
    assert!(
        set_table_cell_alignment(editor.document(), &old, 0, 0, Some(CellAlignment::Left)).is_err()
    );
    assert_eq!(editor.document().block_text("a").expect("data"), "A");
}

#[test]
fn merging_cells_refuses_null_and_omitted_metadata_envelopes() {
    for field in ["text", "marks"] {
        let mut input = square().to_json().expect("fixture JSON");
        input["content"][0]["content"][1]["content"][0][field] = Value::Null;
        let document = Document::from_json(input.clone()).expect("nullable cell metadata");
        assert!(
            merge_table_cells(
                &document,
                &target(&document),
                CellSelection {
                    top: 0,
                    left: 0,
                    bottom: 2,
                    right: 2
                }
            )
            .is_err()
        );
        assert_eq!(document.to_json().expect("retained input"), input);
    }
}

#[test]
fn split_inherits_source_envelopes_and_preserves_explicit_template_nulls() {
    let mut a = cell("a", "A", false);
    attrs(&mut a, json!({"colspan":2}));
    let document = doc(table(vec![row(vec![a])]));
    let mut template_json =
        serde_json::to_value(empty_table_cell("new", false)).expect("template JSON");
    template_json["text"] = Value::Null;
    template_json["marks"] = Value::Null;
    let template = serde_json::from_value(template_json).expect("nullable template");
    let command =
        split_table_cell(&document, &target(&document), 0, 0, vec![template]).expect("split");
    let mut editor = Editor::new(document, Revision(1));
    apply(&mut editor, command);
    let after = editor.document().to_json().expect("JSON");
    let added = &after["content"][0]["content"][0]["content"][1];
    assert!(added.get("text").is_some_and(Value::is_null));
    assert!(added.get("marks").is_some_and(Value::is_null));
    assert_eq!(added["vendor"], json!({"keep":null}));
}
