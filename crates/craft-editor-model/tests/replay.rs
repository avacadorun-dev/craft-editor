#![expect(
    clippy::unwrap_used,
    reason = "Synthetic replay fixtures must fail immediately"
)]

use craft_editor_model::{
    Command, Document, Editor, ReplayAction, Revision, ScalarOffset, TextRange, Transaction,
};

fn replace(editor: &mut Editor, from: &str, to: &str) -> craft_editor_model::Change {
    editor
        .apply(Transaction {
            base_revision: editor.revision(),
            commands: vec![Command::ReplaceText {
                range: TextRange {
                    block_id: "p".into(),
                    start: ScalarOffset(0),
                    end: ScalarOffset(from.chars().count()),
                },
                expected: from.into(),
                replacement: to.into(),
            }],
        })
        .unwrap()
}

#[test]
fn replay_recomputes_commands_and_ignores_mutable_public_change_fields() {
    let original = Document::plain("p", "Я😀").unwrap();
    let mut editor = Editor::new(original.clone(), Revision(8));
    let mut change = replace(&mut editor, "Я😀", "Автор😀");
    let proof = change.replay_proof();
    let expected_maps = change.mappings.clone();
    change.mappings.clear();
    change.after = Document::plain("p", "Forged").unwrap();
    change.before = Document::plain("p", "Forged base").unwrap();
    let (actual, maps) = proof.verify(&original).unwrap();
    assert_eq!(actual, *editor.document());
    assert_eq!(maps, expected_maps);
    assert!(proof.verify(&change.before).is_err());
}

#[test]
fn undo_and_redo_target_the_exact_receipt_without_inverse_map_guessing() {
    let mut editor = Editor::new(Document::plain("p", "A").unwrap(), Revision(0));
    let forward = replace(&mut editor, "A", "B").replay_proof();
    let undo = editor.undo(editor.revision()).unwrap().replay_proof();
    assert_eq!(undo.action(), ReplayAction::Undo);
    assert_eq!(undo.before(), forward.after());
    assert_eq!(undo.after(), forward.before());
    assert!(
        undo.verify(forward.after().document())
            .unwrap()
            .1
            .is_empty()
    );
    let redo = editor.redo(editor.revision()).unwrap().replay_proof();
    assert_eq!(redo.action(), ReplayAction::Redo);
    assert_eq!(redo.before(), forward.before());
    assert_eq!(redo.after(), forward.after());
}

#[test]
fn identical_json_and_independent_sessions_never_alias_history_receipts() {
    let initial = Document::plain("p", "A").unwrap();
    let mut first = Editor::new(initial.clone(), Revision(0));
    let a_to_b = replace(&mut first, "A", "B").replay_proof();
    let b_to_a = replace(&mut first, "B", "A").replay_proof();
    assert_eq!(a_to_b.before().document(), b_to_a.after().document());
    assert_ne!(a_to_b.before(), b_to_a.after());
    let mut second = Editor::new(initial, Revision(0));
    let independent = replace(&mut second, "A", "B").replay_proof();
    assert_ne!(a_to_b.before(), independent.before());
    assert_eq!(a_to_b.before(), &a_to_b.clone().before().clone());
}

#[test]
fn no_op_keeps_the_same_receipt_and_proofs_outlive_trimmed_history() {
    let mut editor =
        Editor::new(Document::plain("p", "A").unwrap(), Revision(0)).with_history_limit(0);
    let no_op = editor
        .apply(Transaction {
            base_revision: Revision(0),
            commands: vec![],
        })
        .unwrap()
        .replay_proof();
    assert_eq!(no_op.before(), no_op.after());
    let proof = replace(&mut editor, "A", "B").replay_proof();
    assert!(!editor.can_undo());
    drop(editor);
    assert_eq!(
        proof.verify(no_op.after().document()).unwrap().0,
        *proof.after().document()
    );
}
