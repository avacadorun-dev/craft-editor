use craft_editor_egui::clipboard::{
    ClipboardBundle, ClipboardError, ClipboardMode, ClipboardProvider, ClipboardRequest,
    ClipboardResult,
};
use craft_editor_egui::{DocPoint, EditorSession, Selection};
use craft_editor_model::{Document, Revision, ScalarOffset};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
type TestResult = Result<(), Box<dyn std::error::Error>>;
const LIMIT: usize = 1024 * 1024;
fn selection(from: usize, to: usize) -> Selection {
    Selection {
        anchor: DocPoint {
            block_id: "p".into(),
            offset: ScalarOffset(from),
        },
        head: DocPoint {
            block_id: "p".into(),
            offset: ScalarOffset(to),
        },
    }
}
fn editor() -> Result<EditorSession, craft_editor_egui::Error> {
    let mut session = EditorSession::new(Document::plain("p", "ABCDE")?, Revision(0));
    session.set_selection(selection(1, 4))?;
    Ok(session)
}
#[derive(Default)]
struct Provider {
    bundle: Mutex<ClipboardBundle>,
    reads: AtomicUsize,
    writes: AtomicUsize,
    fail: bool,
}
impl ClipboardProvider for Provider {
    fn read(&self) -> ClipboardResult<ClipboardBundle> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        Ok(self
            .bundle
            .lock()
            .map_err(|_| ClipboardError::Read)?
            .clone())
    }
    fn write(&self, bundle: &ClipboardBundle) -> ClipboardResult<()> {
        self.writes.fetch_add(1, Ordering::Relaxed);
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
fn cut_waits_for_real_ack_and_restores_original_range_on_undo() -> TestResult {
    let mut session = editor()?;
    let original = session.document().clone();
    let provider = Provider::default();
    let Some(ClipboardRequest::Cut {
        bundle,
        ticket,
        guard,
    }) = session.begin_cut()?
    else {
        return Err("missing cut".into());
    };
    assert_eq!(session.document(), &original);
    assert!(!session.can_undo());
    let mut outbound = (*bundle).clone();
    outbound.formats.insert(
        "application/vnd.example.asset+json".into(),
        b"opaque host bytes".to_vec(),
    );
    let ack = ticket.write(&provider, &outbound)?;
    assert_eq!(session.document(), &original);
    session.finish_cut(guard, ack)?;
    assert_eq!(session.document().block_text("p")?, "AE");
    assert_eq!(provider.writes.load(Ordering::Relaxed), 1);
    session.undo()?;
    assert_eq!(session.document(), &original);
    assert_eq!(session.selection(), Some(&selection(1, 4)));
    Ok(())
}
#[test]
fn failed_write_produces_no_ack_and_cannot_delete_the_source() -> TestResult {
    let session = editor()?;
    let original = session.document().clone();
    let provider = Provider {
        fail: true,
        ..Default::default()
    };
    let Some(ClipboardRequest::Cut { bundle, ticket, .. }) = session.begin_cut()? else {
        return Err("missing cut".into());
    };
    assert!(ticket.write(&provider, &bundle).is_err());
    assert_eq!(session.document(), &original);
    assert!(!session.can_undo());
    Ok(())
}
#[test]
fn changed_canonical_source_is_rejected_before_any_provider_write() -> TestResult {
    let session = editor()?;
    let provider = Provider::default();
    let Some(ClipboardRequest::Copy { bundle, ticket }) = session.begin_copy()? else {
        return Err("missing copy".into());
    };
    let mut wrong = (*bundle).clone();
    wrong.rich = Some(b"another canonical payload".to_vec());
    assert!(ticket.write(&provider, &wrong).is_err());
    assert_eq!(provider.writes.load(Ordering::Relaxed), 0);
    Ok(())
}
#[test]
fn another_successful_write_cannot_acknowledge_the_wrong_cut_request() -> TestResult {
    let mut session = editor()?;
    let original = session.document().clone();
    let provider = Provider::default();
    let Some(ClipboardRequest::Cut { guard, .. }) = session.begin_cut()? else {
        return Err("missing cut".into());
    };
    let Some(ClipboardRequest::Copy { bundle, ticket }) = session.begin_copy()? else {
        return Err("missing copy".into());
    };
    let wrong_ack = ticket.write(&provider, &bundle)?;
    assert!(session.finish_cut(guard, wrong_ack).is_err());
    assert_eq!(session.document(), &original);
    assert!(!session.can_undo());
    Ok(())
}
#[test]
fn changed_selection_or_revision_and_cancelled_jobs_cannot_complete_cut() -> TestResult {
    for case in 0..3 {
        let mut session = editor()?;
        let provider = Provider::default();
        let Some(ClipboardRequest::Cut {
            bundle,
            ticket,
            guard,
        }) = session.begin_cut()?
        else {
            return Err("missing cut".into());
        };
        let ack = ticket.write(&provider, &bundle)?;
        match case {
            0 => session.set_selection(selection(0, 0))?,
            1 => {
                session.insert_plain_text("new")?;
            }
            _ => session.cancel_clipboard_requests()?,
        }
        let current = session.document().clone();
        let revision = session.revision();
        assert!(session.finish_cut(guard, ack).is_err());
        assert_eq!(session.document(), &current);
        assert_eq!(session.revision(), revision);
    }
    Ok(())
}
#[test]
fn identical_document_revision_and_selection_in_another_session_are_not_the_same_target()
-> TestResult {
    let first = editor()?;
    let mut second = editor()?;
    let source = second.document().clone();
    let ClipboardRequest::Paste { guard, bundle } = first.begin_paste(ClipboardBundle {
        plain: "insert".into(),
        ..Default::default()
    })?
    else {
        return Err("missing paste".into());
    };
    assert!(second.finish_paste(guard, &bundle, LIMIT).is_err());
    assert_eq!(second.document(), &source);
    assert!(!second.can_undo());
    Ok(())
}
#[test]
fn prepared_paste_restores_exact_selection_and_does_not_read_os_again() -> TestResult {
    let mut session = editor()?;
    let original = session.document().clone();
    let provider = Arc::new(Provider::default());
    session.set_clipboard_provider(Some(provider.clone()));
    let ClipboardRequest::Paste { bundle, guard } = session.begin_paste(ClipboardBundle {
        plain: "😀".into(),
        ..Default::default()
    })?
    else {
        return Err("missing paste".into());
    };
    session.finish_paste(guard, &bundle, LIMIT)?;
    assert_eq!(session.document().block_text("p")?, "A😀E");
    assert_eq!(provider.reads.load(Ordering::Relaxed), 0);
    session.undo()?;
    assert_eq!(session.document(), &original);
    assert_eq!(session.selection(), Some(&selection(1, 4)));
    Ok(())
}
#[test]
fn requests_are_worker_sendable_and_a_mode_change_invalidates_old_guards() -> TestResult {
    fn assert_send<T: Send>() {}
    assert_send::<ClipboardRequest>();
    let mut session = editor()?;
    let ClipboardRequest::Paste { bundle, guard } = session.begin_paste(ClipboardBundle {
        plain: "X".into(),
        ..Default::default()
    })?
    else {
        return Err("missing paste".into());
    };
    session.set_clipboard_mode(ClipboardMode::Deferred)?;
    assert!(session.finish_paste(guard, &bundle, LIMIT).is_err());
    Ok(())
}
#[cfg(feature = "wordcraft-canvas")]
#[test]
fn focused_canvas_captures_one_native_bundle_then_uses_that_snapshot_after_clipboard_changes()
-> TestResult {
    let mut session = editor()?;
    let provider = Arc::new(Provider::default());
    *provider.bundle.lock().map_err(|_| "mock lock")? = ClipboardBundle {
        plain: "FIRST".into(),
        ..Default::default()
    };
    session.set_clipboard_provider(Some(provider.clone()));
    session.set_clipboard_mode(ClipboardMode::Deferred)?;
    let context = egui::Context::default();
    context.memory_mut(|memory| memory.request_focus(session.options.id));
    let mut diagnostics = Vec::new();
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::Vec2::new(700.0, 650.0),
            )),
            events: vec![egui::Event::Paste("old notification text".into())],
            focused: true,
            ..Default::default()
        },
        |root| {
            egui::CentralPanel::default().show(root, |ui| {
                let response = session.show_canvas(ui, &Default::default());
                diagnostics.extend(response.errors);
            });
        },
    );
    output.textures_delta.clear();
    assert!(diagnostics.is_empty());
    assert_eq!(provider.reads.load(Ordering::Relaxed), 1);
    let request = session
        .take_clipboard_requests()
        .pop()
        .ok_or("missing deferred paste")?;
    let ClipboardRequest::Paste { bundle, guard } = request else {
        return Err("wrong request kind".into());
    };
    *provider.bundle.lock().map_err(|_| "mock lock")? = ClipboardBundle {
        plain: "LATER".into(),
        ..Default::default()
    };
    session.finish_paste(guard, &bundle, LIMIT)?;
    assert_eq!(session.document().block_text("p")?, "AFIRSTE");
    assert_eq!(provider.reads.load(Ordering::Relaxed), 1);
    Ok(())
}

#[test]
#[cfg(feature = "wordcraft-canvas")]
fn ordinary_dialog_text_edit_keeps_native_paste_without_intercepting_rich_canvas_jobs() -> TestResult
{
    let mut session = editor()?;
    let original = session.document().clone();
    let provider = Arc::new(Provider::default());
    session.set_clipboard_provider(Some(provider.clone()));
    session.set_clipboard_mode(ClipboardMode::Deferred)?;
    let context = egui::Context::default();
    let dialog_id = egui::Id::new("synthetic-dialog-input");
    context.memory_mut(|memory| memory.request_focus(dialog_id));
    let mut dialog = String::new();
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::Vec2::new(700.0, 650.0),
            )),
            events: vec![egui::Event::Paste("normal dialog paste".into())],
            focused: true,
            ..Default::default()
        },
        |root| {
            egui::CentralPanel::default().show(root, |ui| {
                ui.add(egui::TextEdit::singleline(&mut dialog).id(dialog_id));
                session.show_canvas(ui, &Default::default());
            });
        },
    );
    output.textures_delta.clear();
    assert_eq!(dialog, "normal dialog paste");
    assert_eq!(provider.reads.load(Ordering::Relaxed), 0);
    assert_eq!(provider.writes.load(Ordering::Relaxed), 0);
    assert!(session.take_clipboard_requests().is_empty());
    assert_eq!(session.document(), &original);
    Ok(())
}
