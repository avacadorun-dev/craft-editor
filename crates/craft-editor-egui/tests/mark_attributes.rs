use craft_editor_egui::{DocPoint, EditorSession, Selection};
use craft_editor_model::{Document, Mark, Revision, ScalarOffset};
use serde_json::{Map, json};

type TestResult = Result<(), Box<dyn std::error::Error>>;
fn point(offset: usize) -> DocPoint {
    DocPoint {
        block_id: "p".into(),
        offset: ScalarOffset(offset),
    }
}
fn selection(from: usize, to: usize) -> Selection {
    Selection {
        anchor: point(from),
        head: point(to),
    }
}
fn link(href: &str) -> Mark {
    let mut mark = Mark::new("link");
    mark.attrs = Some(Map::from_iter([("href".into(), json!(href))]));
    mark
}
fn source() -> craft_editor_model::Result<Document> {
    Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":4},"content":[{"type":"paragraph","attrs":{"blockId":"p","sourceRefs":["voice"]},"content":[
            {"type":"text","text":"А😀","marks":[{"type":"bold","vendorBold":null},{"type":"link","attrs":{"href":"https://old.example","title":"First","vendor":true},"vendorMark":null}]},
            {"type":"text","text":"tail","marks":[{"type":"italic"},{"type":"link","attrs":{"href":"https://old.example","title":"Second","vendor":false},"vendorMark":{"keep":1}}]}
        ]}]}),
    )
}

#[test]
fn link_attribute_patch_preserves_each_runs_other_marks_and_opaque_fields_with_one_undo()
-> TestResult {
    let source = source()?;
    let mut session = EditorSession::new(source.clone(), Revision(0));
    session.set_selection(selection(0, 6))?;
    let change = session.set_mark_attributes(link("https://new.example"))?;
    assert_eq!(session.revision(), Revision(1));
    for (before, after) in source.blocks()[0]
        .children()
        .iter()
        .zip(session.document().blocks()[0].children())
    {
        assert_eq!(before.text, after.text);
        let old = before.marks.as_ref().ok_or("missing old marks")?;
        let new = after.marks.as_ref().ok_or("missing new marks")?;
        assert_eq!(old[0], new[0]);
        assert_eq!(old[1].extra, new[1].extra);
        assert_eq!(
            old[1].attrs.as_ref().and_then(|attrs| attrs.get("title")),
            new[1].attrs.as_ref().and_then(|attrs| attrs.get("title"))
        );
        assert_eq!(
            old[1].attrs.as_ref().and_then(|attrs| attrs.get("vendor")),
            new[1].attrs.as_ref().and_then(|attrs| attrs.get("vendor"))
        );
        assert_eq!(
            new[1].attrs.as_ref().and_then(|attrs| attrs.get("href")),
            Some(&json!("https://new.example"))
        );
    }
    let updated = session.document().clone();
    assert_eq!(change.replay_proof().verify(&source)?.0, updated);
    session.undo()?;
    assert_eq!(session.document(), &source);
    assert_eq!(session.selection(), Some(&selection(0, 6)));
    session.redo()?;
    assert_eq!(session.document(), &updated);
    Ok(())
}

#[test]
fn collapsed_link_patch_changes_typing_marks_without_a_revision_then_types_with_new_href()
-> TestResult {
    let source = source()?;
    let mut session = EditorSession::new(source.clone(), Revision(5));
    session.set_selection(selection(1, 1))?;
    let change = session.set_mark_attributes(link("https://new.example"))?;
    assert_eq!(change.before_revision, change.after_revision);
    assert_eq!(session.document(), &source);
    assert!(!session.can_undo());
    session.insert_plain_text("X")?;
    let inserted = session.document().blocks()[0]
        .children()
        .iter()
        .find(|node| node.text.as_deref() == Some("X"))
        .ok_or("missing inserted run")?;
    let marks = inserted.marks.as_ref().ok_or("missing typing marks")?;
    assert_eq!(
        marks[0],
        source.blocks()[0].children()[0]
            .marks
            .as_ref()
            .ok_or("missing old marks")?[0]
    );
    assert_eq!(
        marks[1].attrs.as_ref().and_then(|attrs| attrs.get("href")),
        Some(&json!("https://new.example"))
    );
    assert_eq!(
        marks[1].attrs.as_ref().and_then(|attrs| attrs.get("title")),
        Some(&json!("First"))
    );
    session.undo()?;
    assert_eq!(session.document(), &source);
    Ok(())
}

#[test]
fn partial_attribute_patch_keeps_required_href_and_invalid_patch_rejects_everything() -> TestResult
{
    let source = source()?;
    let mut session = EditorSession::new(source.clone(), Revision(0));
    session.set_selection(selection(0, 2))?;
    let mut title = Mark::new("link");
    title.attrs = Some(Map::from_iter([("title".into(), json!("New title"))]));
    session.set_mark_attributes(title)?;
    let mark = &session.document().blocks()[0].children()[0]
        .marks
        .as_ref()
        .ok_or("missing marks")?[1];
    assert_eq!(
        mark.attrs.as_ref().and_then(|attrs| attrs.get("href")),
        Some(&json!("https://old.example"))
    );
    assert_eq!(
        mark.attrs.as_ref().and_then(|attrs| attrs.get("title")),
        Some(&json!("New title"))
    );
    let before = session.document().clone();
    let revision = session.revision();
    let pending = session.take_pending_changes().len();
    assert_eq!(pending, 1);
    assert!(session.set_mark_attributes(link("javascript:bad")).is_err());
    assert_eq!(session.document(), &before);
    assert_eq!(session.revision(), revision);
    assert_eq!(session.selection(), Some(&selection(0, 2)));
    assert!(session.take_pending_changes().is_empty());
    session.undo()?;
    assert_eq!(session.document(), &source);
    Ok(())
}

#[test]
fn range_extension_crosses_style_changes_but_stops_at_different_link_attributes() -> TestResult {
    let source = Document::from_json(
        json!({"type":"doc","content":[{"type":"paragraph","attrs":{"blockId":"p"},"content":[
            {"type":"text","text":"A","marks":[{"type":"link","attrs":{"href":"https://a.example"}}]},
            {"type":"text","text":"😀B","marks":[{"type":"bold"},{"type":"link","attrs":{"href":"https://a.example"}}]},
            {"type":"text","text":"C","marks":[{"type":"link","attrs":{"href":"https://b.example"}}]},
            {"type":"text","text":"plain"}
        ]}]}),
    )?;
    let mut session = EditorSession::new(source.clone(), Revision(0));
    session.set_selection(selection(1, 1))?;
    assert!(session.extend_mark_range("link")?);
    assert_eq!(session.selection(), Some(&selection(0, 3)));
    assert_eq!(session.document(), &source);
    assert!(!session.can_undo());
    session.set_selection(selection(3, 3))?;
    session.extend_mark_range("link")?;
    assert_eq!(session.selection(), Some(&selection(3, 4)));
    session.set_selection(selection(4, 4))?;
    session.extend_mark_range("link")?;
    assert_eq!(session.selection(), Some(&selection(3, 4)));
    session.set_selection(selection(5, 5))?;
    assert!(!session.extend_mark_range("link")?);
    assert_eq!(session.selection(), Some(&selection(5, 5)));
    session.set_selection(selection(0, 4))?;
    assert!(!session.extend_mark_range("link")?);
    assert_eq!(session.selection(), Some(&selection(0, 4)));
    Ok(())
}

#[test]
fn invalid_collapsed_patch_keeps_previous_pending_mark_attributes() -> TestResult {
    let source = source()?;
    let mut session = EditorSession::new(source.clone(), Revision(0));
    session.set_selection(selection(1, 1))?;
    session.set_mark_attributes(link("https://pending.example"))?;
    assert!(session.set_mark_attributes(link("javascript:bad")).is_err());
    session.insert_plain_text("X")?;
    let inserted = session.document().blocks()[0]
        .children()
        .iter()
        .find(|node| node.text.as_deref() == Some("X"))
        .ok_or("missing inserted run")?;
    let mark = inserted
        .marks
        .as_ref()
        .ok_or("missing marks")?
        .iter()
        .find(|mark| mark.mark_type == "link")
        .ok_or("missing link")?;
    assert_eq!(
        mark.attrs.as_ref().and_then(|attrs| attrs.get("href")),
        Some(&json!("https://pending.example"))
    );
    Ok(())
}

#[test]
fn atomic_mark_range_formatting_restores_the_original_collapsed_caret_on_undo() -> TestResult {
    let source = source()?;
    let mut session = EditorSession::new(source.clone(), Revision(0));
    session.set_selection(selection(1, 1))?;
    session.format_mark_range(
        link("https://new.example"),
        craft_editor_model::MarkMode::Patch,
    )?;
    assert_eq!(session.selection(), Some(&selection(0, 2)));
    session.undo()?;
    assert_eq!(session.document(), &source);
    assert_eq!(session.selection(), Some(&selection(1, 1)));
    session.redo()?;
    assert_eq!(session.selection(), Some(&selection(0, 2)));
    Ok(())
}
