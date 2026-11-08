#![cfg(feature = "wordcraft-canvas")]

use craft_editor_egui::{
    DocPoint, EditorSession, Selection,
    canvas::CanvasStyle,
    layout::{AtomSize, FlowGeometry, GeometryStyle},
    renderers::{AtomPlaceholder, AtomPlaceholderState, AtomResolver, ResolvedAtom},
};
use craft_editor_model::{Document, Node, NodePath, Revision, ScalarOffset};
use egui::{Event, Pos2, Rect, Vec2};
use serde_json::json;
use std::sync::{Arc, Mutex};

type TestResult = Result<(), Box<dyn std::error::Error>>;
struct ProgressiveResolver {
    state: AtomPlaceholderState,
    ready: bool,
    generation: u64,
    visible: Mutex<Vec<NodePath>>,
}
impl ProgressiveResolver {
    fn new(state: AtomPlaceholderState) -> Self {
        Self {
            state,
            ready: false,
            generation: 1,
            visible: Mutex::new(vec![]),
        }
    }
}
impl AtomResolver for ProgressiveResolver {
    fn resolve(&self, _: &Node, _: f32) -> craft_editor_egui::Result<ResolvedAtom> {
        if !self.ready {
            return Err(craft_editor_egui::Error::AtomRendering(
                "not_prepared".into(),
            ));
        }
        Ok(ResolvedAtom {
            size: AtomSize {
                width: 48.0,
                height: 24.0,
            },
            baseline: Some(18.0),
            encoded: Arc::new(include_bytes!("../examples/atom_contract_pixel.png").to_vec()),
        })
    }
    fn placeholder(&self, node: &Node, width: f32) -> Option<AtomPlaceholder> {
        (!self.ready).then(|| AtomPlaceholder {
            size: AtomSize {
                width: width.min(240.0),
                height: if node.node_type == "inlineMath" {
                    28.0
                } else {
                    96.0
                },
            },
            state: self.state,
            label: "Localized status".into(),
            detail: "Preserved authored source".into(),
        })
    }
    fn prioritize(&self, visible: &[NodePath]) {
        *self
            .visible
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = visible.to_vec();
    }
    fn generation(&self) -> u64 {
        self.generation
    }
}
fn fixture() -> Result<Document, craft_editor_model::Error> {
    Document::from_json(json!({"type":"doc","attrs":{"schemaVersion":4},"content":[
        {"type":"paragraph","attrs":{"blockId":"p"},"content":[{"type":"text","text":"A"},{"type":"inlineMath","attrs":{"latex":"x^2","vendor":null}},{"type":"text","text":"😀Z"}]},
        {"type":"mermaidBlock","attrs":{"blockId":"diagram","code":"flowchart LR\nA-->B","caption":"Authored caption"}},
        {"type":"paragraph","attrs":{"blockId":"last"},"content":[{"type":"text","text":"The end"}]}
    ]}))
}
fn caret(id: &str, offset: usize) -> Selection {
    let point = DocPoint {
        block_id: id.into(),
        offset: ScalarOffset(offset),
    };
    Selection {
        anchor: point.clone(),
        head: point,
    }
}
fn frame(
    session: &mut EditorSession,
    context: &egui::Context,
    events: Vec<Event>,
    resolver: &dyn AtomResolver,
) -> craft_editor_egui::ViewResponse {
    let mut response = None;
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(700.0, 500.0))),
            focused: true,
            events,
            ..Default::default()
        },
        |root| {
            egui::CentralPanel::default().show(root, |ui| {
                response =
                    Some(session.show_canvas_with_resolver(ui, &CanvasStyle::default(), resolver));
            });
        },
    );
    output.textures_delta.clear();
    response.unwrap_or_else(|| panic!("native canvas frame did not run"))
}
#[test]
fn pending_and_failed_atoms_keep_native_prose_geometry_and_exact_opaque_paths() -> TestResult {
    for state in [AtomPlaceholderState::Pending, AtomPlaceholderState::Failed] {
        let document = fixture()?;
        let before = document.to_json()?;
        let resolver = ProgressiveResolver::new(state);
        let geometry =
            FlowGeometry::with_resolver(&document, 600.0, &GeometryStyle::default(), &resolver)?;
        let placeholders = geometry.placeholders();
        assert_eq!(placeholders.len(), 2);
        assert!(
            placeholders
                .iter()
                .all(|atom| atom.placeholder.state == state)
        );
        assert!(geometry.caret(&caret("last", 3).head)?.is_some());
        let diagram = placeholders
            .iter()
            .find(|atom| atom.canonical_path == NodePath(vec![1]))
            .ok_or("missing diagram placeholder")?;
        assert_eq!(
            geometry.hit_node(0, diagram.rect.x + 2.0, diagram.rect.y + 2.0),
            Some(NodePath(vec![1]))
        );
        assert_eq!(document.to_json()?, before);
    }
    Ok(())
}
#[test]
fn pending_inline_math_retains_one_canonical_scalar_and_unicode_caret_mapping() -> TestResult {
    let document = fixture()?;
    let resolver = ProgressiveResolver::new(AtomPlaceholderState::Pending);
    let geometry =
        FlowGeometry::with_resolver(&document, 600.0, &GeometryStyle::default(), &resolver)?;
    assert_eq!(document.block_text("p")?, "A\u{fffc}😀Z");
    for offset in [0, 1, 2, 3, 4] {
        let point = caret("p", offset).head;
        let rect = geometry.caret(&point)?.ok_or("missing caret")?;
        assert_eq!(
            geometry.hit(0, rect.x + 0.01, rect.y + rect.height / 2.0)?,
            Some(point)
        );
    }
    Ok(())
}
#[test]
fn real_canvas_input_remains_available_with_pending_or_failed_media() -> TestResult {
    for state in [AtomPlaceholderState::Pending, AtomPlaceholderState::Failed] {
        let document = fixture()?;
        let atom = document.root().children()[1].clone();
        let mut session = EditorSession::new(document, Revision(0));
        session.set_selection(caret("last", 3))?;
        let context = egui::Context::default();
        let resolver = ProgressiveResolver::new(state);
        let initial = frame(&mut session, &context, vec![], &resolver);
        assert!(initial.errors.is_empty(), "{:?}", initial.errors);
        context.memory_mut(|memory| memory.request_focus(session.options.id));
        let typed = frame(
            &mut session,
            &context,
            vec![Event::Text("Я".into())],
            &resolver,
        );
        assert!(typed.errors.is_empty(), "{:?}", typed.errors);
        assert_eq!(session.document().block_text("last")?, "TheЯ end");
        assert_eq!(session.document().root().children()[1], atom);
        assert!(!typed.changes.is_empty());
    }
    Ok(())
}
#[test]
fn acknowledging_real_media_changes_geometry_without_resetting_selection_or_history() -> TestResult
{
    let mut session = EditorSession::new(fixture()?, Revision(0));
    let selection = Selection {
        anchor: caret("p", 3).head,
        head: caret("last", 2).head,
    };
    session.set_selection(selection.clone())?;
    let context = egui::Context::default();
    let mut resolver = ProgressiveResolver::new(AtomPlaceholderState::Pending);
    let before = session.document().to_json()?;
    assert!(
        frame(&mut session, &context, vec![], &resolver)
            .errors
            .is_empty()
    );
    resolver.ready = true;
    resolver.generation = 2;
    assert!(
        frame(&mut session, &context, vec![], &resolver)
            .errors
            .is_empty()
    );
    assert_eq!(session.selection(), Some(&selection));
    assert_eq!(session.document().to_json()?, before);
    assert_eq!(session.revision(), Revision(0));
    assert!(!session.can_undo());
    Ok(())
}
#[test]
fn canvas_supplies_nonblocking_visible_atom_hints_for_lazy_host_caches() -> TestResult {
    let mut session = EditorSession::new(fixture()?, Revision(0));
    let context = egui::Context::default();
    let resolver = ProgressiveResolver::new(AtomPlaceholderState::Pending);
    assert!(
        frame(&mut session, &context, vec![], &resolver)
            .errors
            .is_empty()
    );
    let visible = resolver
        .visible
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    assert!(visible.contains(&NodePath(vec![0, 1])));
    assert!(visible.contains(&NodePath(vec![1])));
    Ok(())
}
