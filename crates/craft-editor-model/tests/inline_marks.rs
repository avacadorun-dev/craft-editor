use craft_editor_model::{
    Command, Document, Editor, Mark, MarkMode, Revision, ScalarOffset, TextRange, Transaction,
};
use serde_json::json;
type TestResult = Result<(), Box<dyn std::error::Error>>;
fn document() -> craft_editor_model::Result<Document> {
    Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":4},"content":[{"type":"paragraph","attrs":{"blockId":"p","sourceRefs":["source"]},"content":[{"type":"text","text":"A","opaque":null},{"type":"inlineMath","attrs":{"latex":"x^2","description":"keep","vendor":null},"opaqueAtom":true},{"type":"text","text":"B"},{"type":"hardBreak","vendorBreak":null},{"type":"text","text":"😀"}]}]}),
    )
}
#[test]
fn mark_range_through_math_and_hard_break_retains_all_envelopes_and_exact_undo_redo() -> TestResult
{
    let original = document()?;
    let before = original.to_json()?;
    let mut editor = Editor::new(original, Revision(0));
    let change = editor.apply(Transaction {
        base_revision: Revision(0),
        commands: vec![Command::ApplyMarks {
            range: TextRange {
                block_id: "p".into(),
                start: ScalarOffset(0),
                end: ScalarOffset(5),
            },
            marks: vec![Mark::new("bold")],
            mode: MarkMode::Add,
        }],
    })?;
    assert_eq!(change.after_revision, Revision(1));
    let expected = editor.document().to_json()?;
    for child in editor.document().block("p")?.children() {
        assert!(
            child
                .marks
                .as_deref()
                .unwrap_or_default()
                .iter()
                .any(|mark| mark.mark_type == "bold")
        );
    }
    let mut stripped = expected.clone();
    for child in stripped["content"][0]["content"]
        .as_array_mut()
        .ok_or("missing runs")?
    {
        child.as_object_mut().ok_or("invalid node")?.remove("marks");
    }
    assert_eq!(stripped, before);
    editor.undo(editor.revision())?;
    assert_eq!(editor.document().to_json()?, before);
    editor.redo(editor.revision())?;
    assert_eq!(editor.document().to_json()?, expected);
    Ok(())
}
#[test]
fn stale_inline_atom_marking_is_atomic_before_resolving_any_range() -> TestResult {
    let original = document()?;
    let mut editor = Editor::new(original.clone(), Revision(2));
    let result = editor.apply(Transaction {
        base_revision: Revision(1),
        commands: vec![Command::ApplyMarks {
            range: TextRange {
                block_id: "p".into(),
                start: ScalarOffset(1),
                end: ScalarOffset(2),
            },
            marks: vec![Mark::new("underline")],
            mode: MarkMode::Add,
        }],
    });
    assert!(result.is_err());
    assert_eq!(editor.document(), &original);
    assert!(!editor.can_undo());
    Ok(())
}
