use craft_editor_egui::{
    CellPoint, DocPoint, EditorSession, MarkState, Selection, SelectionState, TableSelection,
    TextBlockKind,
    clipboard::{
        ClipboardBundle, ClipboardError, ClipboardProvider, ClipboardRequest, ClipboardResult,
        RichSlice,
    },
};
use craft_editor_model::{Document, ListKind, Mark, Node, NodePath, Revision, ScalarOffset};
use serde_json::Value;
use std::{collections::HashSet, sync::Mutex};
type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
const LIMIT: usize = 1_000_000;
fn source() -> TestResultValue {
    Ok(serde_json::from_str(include_str!(
        "fixtures/source-table-node.json"
    ))?)
}
type TestResultValue = std::result::Result<Value, Box<dyn std::error::Error>>;
fn table_path(document: &Document) -> std::result::Result<NodePath, Box<dyn std::error::Error>> {
    fn find(node: &Node, path: NodePath) -> Option<NodePath> {
        if node.block_id() == Some("table") {
            return Some(path);
        }
        node.children()
            .iter()
            .enumerate()
            .find_map(|(index, node)| find(node, path.child(index)))
    }
    find(document.root(), NodePath::root()).ok_or_else(|| "missing synthetic selected table".into())
}
fn selected(input: &Value) -> std::result::Result<EditorSession, Box<dyn std::error::Error>> {
    let mut session = EditorSession::new(Document::from_json(input.clone())?, Revision(0));
    let reference = session
        .document()
        .node_ref(&table_path(session.document())?)?;
    session.select_node(reference)?;
    Ok(session)
}
fn input_ids(value: &Value) -> HashSet<String> {
    let mut ids = HashSet::new();
    fn walk(value: &Value, ids: &mut HashSet<String>) {
        if let Some(id) = value["attrs"]["blockId"].as_str() {
            ids.insert(id.into());
        }
        for child in value["content"].as_array().into_iter().flatten() {
            walk(child, ids);
        }
    }
    walk(value, &mut ids);
    ids
}
// Normalize schema defaults and newly allocated wrapper/paragraph identities only. Existing text
// and table identities remain compared exactly; source list-failure identity loss is not copied.
fn comparable(mut value: Value, retained: &HashSet<String>) -> Value {
    fn walk(value: &mut Value, retained: &HashSet<String>) {
        let kind = value["type"].as_str().unwrap_or_default().to_owned();
        if let Some(attrs) = value.get_mut("attrs").and_then(Value::as_object_mut) {
            let keep_id = matches!(
                kind.as_str(),
                "paragraph" | "heading" | "codeBlock" | "table"
            ) && attrs
                .get("blockId")
                .and_then(Value::as_str)
                .is_some_and(|id| retained.contains(id));
            if !keep_id {
                attrs.remove("blockId");
            }
            if attrs
                .get("sourceRefs")
                .is_some_and(|refs| refs.as_array().is_some_and(Vec::is_empty))
            {
                attrs.remove("sourceRefs");
            }
            if kind == "orderedList" {
                if attrs.get("start") == Some(&serde_json::json!(1)) {
                    attrs.remove("start");
                }
                if attrs.get("type").is_some_and(Value::is_null) {
                    attrs.remove("type");
                }
            }
            if attrs.is_empty()
                && let Some(object) = value.as_object_mut()
            {
                object.remove("attrs");
            }
        }
        if value
            .get("content")
            .is_some_and(|content| content.as_array().is_some_and(Vec::is_empty))
            && let Some(object) = value.as_object_mut()
        {
            object.remove("content");
        }
        for child in value
            .get_mut("content")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            walk(child, retained);
        }
    }
    walk(&mut value, retained);
    value
}
// Original setHeading drops identities, and a quoted table gives both headings the quote's
// identity. Compare the source's structure while separately requiring native a/b preservation.
fn heading_shape(mut value: Value) -> Value {
    fn walk(value: &mut Value) {
        if value["type"] == "heading"
            && let Some(attrs) = value.get_mut("attrs").and_then(Value::as_object_mut)
        {
            attrs.remove("blockId");
        }
        for child in value
            .get_mut("content")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            walk(child);
        }
    }
    walk(&mut value);
    value
}
fn assert_source_head(session: &EditorSession, expected: &Value) -> TestResult {
    let selected = session
        .selection()
        .ok_or("missing source-compatible text selection")?;
    if let Some(id) = expected["head"]["blockId"].as_str() {
        assert_eq!(selected.head.block_id, id);
    }
    assert_eq!(
        selected.head.offset.0,
        expected["head"]["offset"].as_u64().ok_or("source offset")? as usize
    );
    Ok(())
}
#[test]
fn source_table_node_copy_delete_and_arrow_boundaries_keep_one_selection_form() -> TestResult {
    let fixture = source()?;
    for case in fixture["cases"].as_array().ok_or("source cases")? {
        assert_eq!(case["selected"], true);
        let mut session = selected(&case["input"])?;
        let original = session.document().clone();
        let node = session.node_selection().cloned().ok_or("selected table")?;
        assert!(session.selection().is_none() && session.cell_selection().is_none());
        let copied = session.copy_selection()?.ok_or("missing table copy")?;
        assert_eq!(
            copied.plain,
            case["copy"]["plain"].as_str().ok_or("source plain copy")?
        );
        let slice = RichSlice::decode(copied.rich.as_deref().ok_or("rich table copy")?, LIMIT)?;
        assert_eq!((slice.open_start, slice.open_end), (0, 0));
        assert_eq!(
            serde_json::to_value(&slice.content)?,
            case["copy"]["slice"]["content"]
        );
        assert!(
            copied
                .html
                .as_deref()
                .is_some_and(|html| html.contains("<table"))
        );
        assert_eq!(session.document(), &original);
        assert!(!session.can_undo());
        for (name, backward) in [("before", true), ("after", false)] {
            let mut moving = selected(&case["input"])?;
            assert!(moving.navigate_selected_table(backward)?);
            if case["arrows"][name].is_null() {
                assert_eq!(moving.node_selection(), Some(&node));
            } else {
                assert_source_head(&moving, &case["arrows"][name])?;
                assert!(moving.node_selection().is_none());
            }
            assert_eq!(moving.document(), &original);
            assert!(!moving.can_undo());
        }
        for (name, backward) in [("forward", false), ("backward", true)] {
            assert_eq!(case["tabs"][name]["canonicalUnchanged"], true);
            let mut tabbing = selected(&case["input"])?;
            assert!(!tabbing.advance_table_cell(backward)?);
            assert_eq!(tabbing.node_selection(), Some(&node));
            assert_eq!(tabbing.document(), &original);
            assert!(!tabbing.can_undo());
        }
        session.delete_selected_node()?;
        let ids = input_ids(&case["input"]);
        assert_eq!(
            comparable(session.document().to_json()?, &ids),
            comparable(case["deletion"]["output"].clone(), &ids),
            "{}",
            case["name"]
        );
        assert_source_head(&session, &case["deletion"]["selection"])?;
        let deleted = session.document().clone();
        session.undo()?;
        assert_eq!(session.document(), &original);
        assert_eq!(session.node_selection(), Some(&node));
        session.redo()?;
        assert_eq!(session.document(), &deleted);
    }
    Ok(())
}
#[test]
fn table_node_enter_typing_and_closed_paste_follow_real_source_slots() -> TestResult {
    let fixture = source()?;
    for case in fixture["cases"].as_array().ok_or("source cases")? {
        let ids = input_ids(&case["input"]);
        for action in ["enter", "typing", "paste"] {
            let mut session = selected(&case["input"])?;
            let original = session.document().clone();
            let selected_node = session.node_selection().cloned().ok_or("selected table")?;
            match action {
                "enter" => {
                    session.insert_paragraph()?;
                }
                "typing" => {
                    session.insert_plain_text("Replacement")?;
                }
                _ => {
                    let copied = session.copy_selection()?.ok_or("copy")?;
                    session.paste_bundle(&copied, LIMIT)?;
                }
            }
            let actual = session.document().to_json()?;
            // Copy assigns fresh identities. Compare source body/attributes after removing only
            // incoming table/text identities; the exact retained siblings are asserted separately.
            let compare_ids = if action == "paste" {
                let mut keep = ids.clone();
                fn remove(node: &Node, keep: &mut HashSet<String>) {
                    if let Some(id) = node.block_id() {
                        keep.remove(id);
                    }
                    for child in node.children() {
                        remove(child, keep);
                    }
                }
                remove(&selected_node.expected, &mut keep);
                keep
            } else {
                ids.clone()
            };
            assert_eq!(
                comparable(actual, &compare_ids),
                comparable(case[action]["output"].clone(), &compare_ids),
                "{} {action}",
                case["name"]
            );
            if action != "paste"
                || case[action]["selection"]["head"]["blockId"]
                    .as_str()
                    .is_some_and(|id| compare_ids.contains(id))
            {
                assert_source_head(&session, &case[action]["selection"])?;
            } else {
                let point = &session.selection().ok_or("pasted table caret")?.head;
                assert_eq!(session.document().block_text(&point.block_id)?, "Bravo");
                assert_eq!(point.offset, ScalarOffset(5));
            }
            session.undo()?;
            assert_eq!(session.document(), &original);
            assert_eq!(session.node_selection(), Some(&selected_node));
        }
    }
    Ok(())
}
#[test]
fn whole_table_formatting_matches_twenty_actual_original_command_manager_cases() -> TestResult {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/source-table-node-formatting.json"))?;
    for case in fixture["cases"].as_array().ok_or("source formatting")? {
        let mut session = selected(&case["before"])?;
        let original = session.document().clone();
        let node = session.node_selection().cloned().ok_or("node")?;
        match case["action"].as_str().ok_or("action")? {
            "toggleBold" => session.toggle_mark(Mark::new("bold"))?,
            "setHeading" => {
                session.set_text_block_kind(TextBlockKind::Heading { level: 2 })?;
            }
            "toggleBlockquote" => {
                session.toggle_blockquote()?;
            }
            "toggleBulletList" => {
                session.toggle_list(ListKind::Bullet)?;
            }
            _ => {
                session.toggle_list(ListKind::Ordered)?;
            }
        }
        let after = session
            .node_selection()
            .ok_or("formatting lost whole-table node selection")?;
        assert_eq!(after.expected.block_id(), Some("table"));
        assert!(session.selection().is_none() && session.cell_selection().is_none());
        if case["handled"] == false {
            assert_eq!(
                session.document(),
                &original,
                "failed source command must not clear retained identities"
            );
            assert!(!session.can_undo());
        } else {
            let ids = input_ids(&case["before"]);
            let mut actual = comparable(session.document().to_json()?, &ids);
            let mut expected = comparable(case["after"].clone(), &ids);
            if case["action"] == "setHeading" {
                for id in ["a", "b"] {
                    let heading = session.document().block(id)?;
                    assert_eq!(heading.node_type, "heading");
                    assert_eq!(heading.block_id(), Some(id));
                }
                actual = heading_shape(actual);
                expected = heading_shape(expected);
            }
            assert_eq!(actual, expected, "{} {}", case["name"], case["action"]);
            if case["action"] == "toggleBold" {
                assert_eq!(session.mark_state("bold")?, MarkState::On);
            }
            let changed = session.document().clone();
            session.undo()?;
            assert_eq!(session.document(), &original);
            assert_eq!(session.node_selection(), Some(&node));
            session.redo()?;
            assert_eq!(session.document(), &changed);
            assert!(session.node_selection().is_some());
        }
    }
    Ok(())
}
#[derive(Default)]
struct Provider {
    fail: bool,
    bundle: Mutex<ClipboardBundle>,
}
impl ClipboardProvider for Provider {
    fn read(&self) -> ClipboardResult<ClipboardBundle> {
        Ok(self
            .bundle
            .lock()
            .map_err(|_| ClipboardError::Read)?
            .clone())
    }
    fn write(&self, bundle: &ClipboardBundle) -> ClipboardResult<()> {
        if self.fail {
            return Err(ClipboardError::Write);
        }
        *self.bundle.lock().map_err(|_| ClipboardError::Write)? = bundle.clone();
        Ok(())
    }
    fn max_bytes(&self) -> usize {
        LIMIT
    }
}
#[test]
fn whole_table_cut_requires_acknowledgement_and_rejects_stale_or_foreign_targets() -> TestResult {
    let case = &source()?["cases"][0];
    for mode in ["success", "write-failure", "moved", "foreign"] {
        let mut session = selected(&case["input"])?;
        let original = session.document().clone();
        let node = session.node_selection().cloned().ok_or("node")?;
        let Some(ClipboardRequest::Cut {
            bundle,
            ticket,
            guard,
        }) = session.begin_cut()?
        else {
            return Err("missing cut intent".into());
        };
        assert_eq!(session.document(), &original);
        assert!(!session.can_undo());
        let provider = Provider {
            fail: mode == "write-failure",
            ..Default::default()
        };
        let ack = ticket.write(&provider, &bundle);
        if mode == "write-failure" {
            assert!(ack.is_err());
            assert_eq!(session.document(), &original);
            assert_eq!(session.node_selection(), Some(&node));
            continue;
        }
        let ack = ack?;
        assert_eq!(session.document(), &original);
        if mode == "moved" {
            session.navigate_selected_table(false)?;
            let live = session.selection_state()?;
            assert!(session.finish_cut(guard, ack).is_err());
            assert_eq!(session.selection_state()?, live);
            assert_eq!(session.document(), &original);
        } else if mode == "foreign" {
            let mut foreign = selected(&case["input"])?;
            assert!(foreign.finish_cut(guard, ack).is_err());
            assert_eq!(foreign.document(), &original);
        } else {
            session.finish_cut(guard, ack)?;
            assert_ne!(session.document(), &original);
            session.undo()?;
            assert_eq!(session.document(), &original);
            assert_eq!(session.node_selection(), Some(&node));
        }
    }
    Ok(())
}
#[test]
fn table_selection_return_is_an_exact_revision_bound_text_or_reverse_cell_occurrence() -> TestResult
{
    let case = &source()?["cases"][0];
    let mut session = selected(&case["input"])?;
    let path = table_path(session.document())?;
    for previous in [
        SelectionState::Text(Selection {
            anchor: DocPoint {
                block_id: "a".into(),
                offset: ScalarOffset(1),
            },
            head: DocPoint {
                block_id: "a".into(),
                offset: ScalarOffset(3),
            },
        }),
        SelectionState::Cells(TableSelection {
            table_path: path,
            anchor: CellPoint { row: 0, column: 1 },
            head: CellPoint { row: 0, column: 0 },
        }),
    ] {
        session.restore_selection_state(Some(previous.clone()))?;
        let before = session.document().clone();
        session.select_current_table()?;
        assert!(session.leave_table_node_selection()?);
        assert_eq!(session.selection_state()?, Some(previous));
        assert_eq!(session.document(), &before);
        assert!(!session.can_undo());
    }
    Ok(())
}

#[test]
fn owned_formatting_and_history_keep_an_exact_exit_without_resealing_external_bookmarks()
-> TestResult {
    let fixture = source()?;
    for case in fixture["cases"].as_array().ok_or("source cases")? {
        for cells in [false, true] {
            for action in ["bold", "heading", "quote"] {
                let mut session = selected(&case["input"])?;
                let previous = if cells {
                    SelectionState::Cells(TableSelection {
                        table_path: table_path(session.document())?,
                        anchor: CellPoint { row: 0, column: 1 },
                        head: CellPoint { row: 0, column: 0 },
                    })
                } else {
                    SelectionState::Text(Selection {
                        anchor: DocPoint {
                            block_id: "a".into(),
                            offset: ScalarOffset(1),
                        },
                        head: DocPoint {
                            block_id: "a".into(),
                            offset: ScalarOffset(3),
                        },
                    })
                };
                session.restore_selection_state(Some(previous.clone()))?;
                let external = session.capture_selection_bookmark()?;
                let original = session.document().clone();
                session.select_current_table()?;
                let selected = session.node_selection().cloned().ok_or("table")?;
                match action {
                    "bold" => session.toggle_mark(Mark::new("bold"))?,
                    "heading" => {
                        session.set_text_block_kind(TextBlockKind::Heading { level: 2 })?;
                    }
                    _ => {
                        session.toggle_blockquote()?;
                    }
                }
                let changed = session.document().clone();
                let mut expected = previous.clone();
                if let SelectionState::Cells(cells) = &mut expected {
                    cells.table_path = table_path(session.document())?;
                }
                assert!(session.restore_selection_bookmark(&external).is_err());
                assert!(session.node_selection().is_some());
                assert!(
                    session.leave_table_node_selection()?,
                    "{} {action} cells={cells}",
                    case["name"]
                );
                assert_eq!(session.selection_state()?, Some(expected.clone()));
                assert_eq!(session.document(), &changed);
                session.undo()?;
                assert_eq!(session.document(), &original);
                assert_eq!(session.node_selection(), Some(&selected));
                assert!(session.leave_table_node_selection()?);
                assert_eq!(session.selection_state()?, Some(previous));
                session.redo()?;
                assert_eq!(session.document(), &changed);
                assert!(session.node_selection().is_some());
                assert!(session.leave_table_node_selection()?);
                assert_eq!(session.selection_state()?, Some(expected));
            }
        }
    }
    Ok(())
}

#[test]
fn unrelated_edits_and_changed_return_writing_cannot_guess_a_new_exit() -> TestResult {
    use craft_editor_model::{Command, Editor, Transaction};
    let input = &source()?["cases"][0]["input"];
    for controlled in [false, true] {
        let mut session = selected(input)?;
        let previous = SelectionState::Text(Selection {
            anchor: DocPoint {
                block_id: "a".into(),
                offset: ScalarOffset(1),
            },
            head: DocPoint {
                block_id: "a".into(),
                offset: ScalarOffset(3),
            },
        });
        session.restore_selection_state(Some(previous.clone()))?;
        session.select_current_table()?;
        let table = session.node_selection().cloned().ok_or("table")?;
        let path = if controlled {
            NodePath(vec![1, 0, 0, 0])
        } else {
            NodePath(vec![0])
        };
        let target = session.document().node_ref(&path)?;
        let mut replacement = target.expected.as_ref().clone();
        if controlled {
            // A new identity with repeated writing is not the old selected occurrence.
            replacement
                .attrs
                .get_or_insert_default()
                .insert("blockId".into(), serde_json::json!("replacement-a"));
        } else {
            replacement
                .attrs
                .get_or_insert_default()
                .insert("future".into(), serde_json::json!(true));
        }
        let commands = vec![Command::ReplaceNode {
            target,
            replacement: Box::new(replacement),
        }];
        if controlled {
            let mut candidate = Editor::new(session.document().clone(), session.revision());
            candidate.apply(Transaction {
                base_revision: candidate.revision(),
                commands: commands.clone(),
            })?;
            let after = candidate.document().node_ref(&table.path)?;
            session.apply_commands_with_node_selection(commands, after)?;
        } else {
            session.apply_commands(commands)?;
        }
        let changed = session.document().clone();
        let state = session.selection_state()?;
        assert!(session.leave_table_node_selection().is_err());
        assert_eq!(session.document(), &changed);
        assert_eq!(session.selection_state()?, state);
        session.undo()?;
        assert_eq!(session.document().to_json()?, *input);
        assert_eq!(session.node_selection(), Some(&table));
        assert!(session.leave_table_node_selection()?);
        assert_eq!(session.selection_state()?, Some(previous));
    }
    Ok(())
}
