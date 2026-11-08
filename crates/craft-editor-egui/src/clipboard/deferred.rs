//! Opt-in host jobs with one-shot session guards and real acknowledged writes.
use super::{
    ClipboardBundle, ClipboardError, ClipboardProvider, ClipboardResult, PasteContext,
    PreparedPaste,
};
use crate::{EditorSession, Error, Result, Selection};
use craft_editor_model::{Change, NodeRef, Revision};
use std::collections::VecDeque;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

/// Whether focused native canvas commands execute immediately or become host jobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClipboardMode {
    /// Existing synchronous provider behavior for embedders and headless callers.
    #[default]
    Synchronous,
    /// Capture an immutable request; the host performs heavy media work off the UI thread.
    Deferred,
}
/// Shared application-local Copy/Cut intent state. It is independent of mutable OS contents.
///
/// Share one Arc across all application sessions. A pending local write causes Paste to refuse
/// before reading an older OS buffer. This gate never treats matching plain text as identity.
#[derive(Debug, Default)]
pub struct ClipboardIntentGate {
    pending: AtomicUsize,
}
impl ClipboardIntentGate {
    /// Whether a captured local Copy/Cut still owns an unacknowledged write ticket.
    pub fn has_pending_write(&self) -> bool {
        self.pending.load(Ordering::Acquire) != 0
    }
    /// Create a Copy-only request for explicitly recovered captured content. This cannot create
    /// a Cut guard or acknowledge a write, and does not inspect or retarget an editor selection.
    pub fn copy_request(
        self: &Arc<Self>,
        bundle: ClipboardBundle,
    ) -> ClipboardResult<ClipboardRequest> {
        let bundle = Arc::new(bundle);
        Ok(ClipboardRequest::Copy {
            ticket: WriteTicket {
                _pending: self.register()?,
                source: bundle.clone(),
                identity: Arc::new(()),
            },
            bundle,
        })
    }
    fn register(self: &Arc<Self>) -> ClipboardResult<PendingWrite> {
        self.pending
            .try_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .map_err(|_| ClipboardError::ResourceLimit)?;
        Ok(PendingWrite { gate: self.clone() })
    }
}
struct PendingWrite {
    gate: Arc<ClipboardIntentGate>,
}
impl Drop for PendingWrite {
    fn drop(&mut self) {
        // Each increment creates exactly one private, non-cloneable registration.
        self.gate.pending.fetch_sub(1, Ordering::AcqRel);
    }
}
pub(crate) struct DeferredState {
    identity: Arc<()>,
    intent_gate: Arc<ClipboardIntentGate>,
    generation: u64,
    mode: ClipboardMode,
    requests: VecDeque<ClipboardRequest>,
}
impl DeferredState {
    pub(crate) fn session_identity(&self) -> Arc<()> {
        self.identity.clone()
    }
}
impl Default for DeferredState {
    fn default() -> Self {
        Self {
            identity: Arc::new(()),
            intent_gate: Arc::new(ClipboardIntentGate::default()),
            generation: 0,
            mode: ClipboardMode::Synchronous,
            requests: VecDeque::new(),
        }
    }
}
/// A one-shot instruction for the host's bounded clipboard worker.
///
/// Requests and guards deliberately implement neither Clone nor Deserialize. Copy and Cut carry
/// only selected content; Paste carries one captured raw OS bundle, never a future reread request.
pub enum ClipboardRequest {
    /// Prepare optional host attachments and publish the exact selected canonical content.
    Copy {
        /// Immutable selected canonical/HTML/plain representations.
        bundle: Arc<ClipboardBundle>,
        /// Calls the real provider and seals its successful acknowledgement.
        ticket: WriteTicket,
    },
    /// Publish first, then conditionally delete from the captured still-current session.
    Cut {
        /// Immutable selected content; no unrelated document content is included.
        bundle: Arc<ClipboardBundle>,
        /// Verifiable correlation with the write that must succeed before deletion.
        ticket: WriteTicket,
        /// One-shot exact session/revision/selection guard.
        guard: CutGuard,
    },
    /// Prepare project resources from an already captured clipboard representation.
    Paste {
        /// Immutable raw multi-format snapshot captured once at the native command boundary.
        bundle: Arc<ClipboardBundle>,
        /// Small immutable preparation context correlated with this exact captured payload.
        context: PasteContext,
        /// One-shot exact destination guard; worker completion cannot retarget another session.
        guard: PasteGuard,
    },
}
struct Guard {
    session: Arc<()>,
    generation: u64,
    revision: Revision,
    selection: Option<Selection>,
    node_selection: Option<NodeRef>,
    table_selection: Option<crate::TableSelection>,
    stored_marks: Option<Vec<craft_editor_model::Mark>>,
}
/// Captured Cut destination; only a correlated successful native write can complete it.
pub struct CutGuard {
    guard: Guard,
    write: Arc<()>,
}
/// Captured Paste destination; consumes a host-prepared immutable bundle only once.
pub struct PasteGuard {
    guard: Guard,
    context: PasteContext,
}
/// A one-shot capability to acknowledge an actual write of captured canonical source content.
pub struct WriteTicket {
    _pending: PendingWrite,
    source: Arc<ClipboardBundle>,
    identity: Arc<()>,
}
/// A sealed successful provider acknowledgement, correlated with its original Cut request.
///
/// ```compile_fail
/// let forged: craft_editor_egui::clipboard::WriteAck = serde_json::from_str("{}").unwrap();
/// ```
pub struct WriteAck {
    identity: Arc<()>,
}
impl WriteTicket {
    /// Write through the real provider and seal its acknowledgement.
    ///
    /// The host may add opaque formats or improve an HTML media representation, but may not
    /// replace the captured private canonical bytes or plain content and then delete the source.
    pub fn write(
        self,
        provider: &dyn ClipboardProvider,
        outbound: &ClipboardBundle,
    ) -> ClipboardResult<WriteAck> {
        if self.source.rich != outbound.rich || self.source.plain != outbound.plain {
            return Err(ClipboardError::InvalidAcknowledgement);
        }
        outbound.check_budget(provider.max_bytes())?;
        provider.write(outbound)?;
        Ok(WriteAck {
            identity: self.identity,
        })
    }
}
impl Guard {
    fn capture(session: &EditorSession) -> Result<Self> {
        if session.is_composing() {
            return Err(Error::CompositionActive);
        }
        Ok(Self {
            session: session.clipboard_state.identity.clone(),
            generation: session.clipboard_state.generation,
            revision: session.revision(),
            selection: session.selection.clone(),
            node_selection: session.node_selection.clone(),
            table_selection: session.table_selection.clone(),
            stored_marks: session.stored_marks.clone(),
        })
    }
    fn check(&self, session: &EditorSession) -> Result<()> {
        if session.is_composing() {
            return Err(Error::CompositionActive);
        }
        if !Arc::ptr_eq(&self.session, &session.clipboard_state.identity)
            || self.generation != session.clipboard_state.generation
            || self.revision != session.revision()
            || self.selection != session.selection
            || self.node_selection != session.node_selection
            || self.table_selection != session.table_selection
            || self.stored_marks != session.stored_marks
        {
            return Err(ClipboardError::StaleRequest.into());
        }
        Ok(())
    }
}
impl EditorSession {
    /// Install one application-lifetime intent gate before dispatching clipboard jobs.
    /// A session with its own outstanding tickets cannot abandon that pending intent state.
    pub fn set_clipboard_intent_gate(&mut self, gate: Arc<ClipboardIntentGate>) -> Result<()> {
        if self.clipboard_state.intent_gate.has_pending_write() {
            return Err(ClipboardError::WritePending.into());
        }
        self.clipboard_state.intent_gate = gate;
        Ok(())
    }
    /// Shared local intent state; hosts may consult it before a Paste into another native widget.
    pub fn clipboard_intent_gate(&self) -> Arc<ClipboardIntentGate> {
        self.clipboard_state.intent_gate.clone()
    }
    /// Select the clipboard execution mode. Changing it invalidates old one-shot job guards.
    pub fn set_clipboard_mode(&mut self, mode: ClipboardMode) -> Result<()> {
        if self.clipboard_state.mode != mode {
            self.cancel_clipboard_requests()?;
            self.clipboard_state.mode = mode;
        }
        Ok(())
    }
    /// Current focused-canvas clipboard execution mode.
    pub fn clipboard_mode(&self) -> ClipboardMode {
        self.clipboard_state.mode
    }
    /// Invalidate outstanding job guards and discard queued work without changing the document.
    pub fn cancel_clipboard_requests(&mut self) -> Result<()> {
        let generation = self
            .clipboard_state
            .generation
            .checked_add(1)
            .ok_or(craft_editor_model::Error::ResourceLimit)?;
        self.clipboard_state.generation = generation;
        self.clipboard_state.requests.clear();
        Ok(())
    }
    /// Drain captured requests to the host's bounded worker; ordinary TextEdit widgets are unaffected.
    pub fn take_clipboard_requests(&mut self) -> Vec<ClipboardRequest> {
        self.clipboard_state.requests.drain(..).collect()
    }
    /// Capture an immutable Copy request without writing to an OS clipboard.
    pub fn begin_copy(&self) -> Result<Option<ClipboardRequest>> {
        let Some(bundle) = self.copy_selection()? else {
            return Ok(None);
        };
        let bundle = Arc::new(bundle);
        Ok(Some(ClipboardRequest::Copy {
            ticket: WriteTicket {
                _pending: self.clipboard_state.intent_gate.register()?,
                source: bundle.clone(),
                identity: Arc::new(()),
            },
            bundle,
        }))
    }
    /// Capture selected content plus an exact guarded Cut destination, without deleting anything.
    pub fn begin_cut(&self) -> Result<Option<ClipboardRequest>> {
        let guard = Guard::capture(self)?;
        let Some(bundle) = self.copy_selection()? else {
            return Ok(None);
        };
        let bundle = Arc::new(bundle);
        let identity = Arc::new(());
        Ok(Some(ClipboardRequest::Cut {
            ticket: WriteTicket {
                _pending: self.clipboard_state.intent_gate.register()?,
                source: bundle.clone(),
                identity: identity.clone(),
            },
            bundle,
            guard: CutGuard {
                guard,
                write: identity,
            },
        }))
    }
    /// Bind one already captured raw clipboard bundle to the exact current destination.
    pub fn begin_paste(&self, bundle: ClipboardBundle) -> Result<ClipboardRequest> {
        if self.clipboard_state.intent_gate.has_pending_write() {
            return Err(ClipboardError::WritePending.into());
        }
        let source = Arc::new(bundle);
        let (context, guard) = self.capture_paste_context(source.clone())?;
        Ok(ClipboardRequest::Paste {
            bundle: source,
            context,
            guard,
        })
    }
    pub(crate) fn capture_paste_context(
        &self,
        source: Arc<ClipboardBundle>,
    ) -> Result<(PasteContext, PasteGuard)> {
        let guard = Guard::capture(self)?;
        let context = PasteContext::capture(self, source)?;
        Ok((context.clone(), PasteGuard { guard, context }))
    }
    /// Complete Cut only after a matching real write and an unchanged exact session/selection.
    pub fn finish_cut(&mut self, guard: CutGuard, ack: WriteAck) -> Result<Change> {
        guard.guard.check(self)?;
        if !Arc::ptr_eq(&guard.write, &ack.identity) {
            return Err(ClipboardError::InvalidAcknowledgement.into());
        }
        if self.node_selection.is_some() {
            self.delete_selected_node()
        } else {
            self.insert_plain_text("")
        }
    }
    /// Complete prepared input only when it belongs to this exact captured context and the
    /// original live session/revision/selection/typing state is still current.
    pub fn finish_prepared_paste(
        &mut self,
        guard: PasteGuard,
        prepared: PreparedPaste,
    ) -> Result<Change> {
        guard.guard.check(self)?;
        if !prepared.matches(&guard.context) {
            return Err(ClipboardError::InvalidPasteContext.into());
        }
        self.apply_prepared_paste(prepared)
    }
    /// Compatibility completion for an unchanged captured bundle. Media/normalized payloads use
    /// prepare on the request's PasteContext, bind_images, then finish_prepared_paste instead.
    pub fn finish_paste(
        &mut self,
        guard: PasteGuard,
        bundle: &ClipboardBundle,
        max_bytes: usize,
    ) -> Result<Change> {
        guard.guard.check(self)?;
        if guard.context.source() != bundle {
            return Err(ClipboardError::InvalidPasteContext.into());
        }
        let prepared = guard.context.prepare(max_bytes)?;
        self.finish_prepared_paste(guard, prepared)
    }
    #[cfg(feature = "wordcraft-canvas")]
    pub(crate) fn queue_clipboard_copy(&mut self, cut: bool) -> Result<()> {
        self.check_clipboard_queue()?;
        let request = if cut {
            self.begin_cut()?
        } else {
            self.begin_copy()?
        };
        if let Some(request) = request {
            self.clipboard_state.requests.push_back(request);
        }
        Ok(())
    }
    #[cfg(feature = "wordcraft-canvas")]
    pub(crate) fn queue_clipboard_paste(&mut self) -> Result<()> {
        self.check_clipboard_queue()?;
        if self.is_composing() {
            return Err(Error::CompositionActive);
        }
        if self.clipboard_state.intent_gate.has_pending_write() {
            return Err(ClipboardError::WritePending.into());
        }
        // Capture native representations ONCE, before the host queues media work. Do not use the
        // egui text event or an equal-plaintext cache as a surrogate identity for rich clipboard.
        let provider = self
            .clipboard_provider
            .clone()
            .ok_or(ClipboardError::Unavailable)?;
        let bundle = provider.read()?;
        bundle.check_budget(provider.max_bytes())?;
        let request = self.begin_paste(bundle)?;
        self.clipboard_state.requests.push_back(request);
        Ok(())
    }
    #[cfg(feature = "wordcraft-canvas")]
    fn check_clipboard_queue(&self) -> Result<()> {
        // This bounds queued memory, not document capability. Hosts drain every frame and own
        // their worker's concurrency/cancellation; in-flight jobs retain their one-shot guards.
        if self.clipboard_state.requests.len() >= 4 {
            return Err(ClipboardError::Busy.into());
        }
        Ok(())
    }
}
