use craft_editor_egui::clipboard::{
    ClipboardBundle, ClipboardError, ClipboardProvider, ClipboardResult, RichSlice,
};
use craft_editor_egui::{DocPoint, EditorSession, Selection};
use craft_editor_model::{Document, Node, NodePath, Revision, ScalarOffset};
use serde_json::{Map, json};
use std::sync::{Arc, Mutex};
type TestResult = Result<(), Box<dyn std::error::Error>>;
const LIMIT: usize = 16 * 1024 * 1024;
fn point(id: &str, offset: usize) -> DocPoint {
    DocPoint {
        block_id: id.into(),
        offset: ScalarOffset(offset),
    }
}
fn range(a: &str, from: usize, b: &str, to: usize) -> Selection {
    Selection {
        anchor: point(a, from),
        head: point(b, to),
    }
}
fn caret(id: &str, offset: usize) -> Selection {
    range(id, offset, id, offset)
}
fn p(id: &str, text: &str) -> Node {
    Node::paragraph(id, text)
}
fn session() -> Result<EditorSession, craft_editor_model::Error> {
    let mut session = EditorSession::new(Document::plain("target", "leftright")?, Revision(0));
    session
        .set_selection(caret("target", 4))
        .map_err(|_| craft_editor_model::Error::InvalidPosition)?;
    Ok(session)
}
fn bundle(slice: RichSlice) -> Result<ClipboardBundle, craft_editor_egui::Error> {
    slice.bundle()
}

#[test]
fn rich_partial_copy_preserves_marks_atoms_opaque_fields_and_exact_unicode() -> TestResult {
    let source = Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":4},"content":[{"type":"paragraph","attrs":{"blockId":"source","sourceRefs":["voice"],"vendor":null},"vendorBlock":{"keep":1},"content":[{"type":"text","text":"A😀Б","marks":[{"type":"bold","vendor":null},{"type":"link","attrs":{"href":"https://example.test","vendor":true}}],"vendorLeaf":null},{"type":"inlineMath","attrs":{"latex":"x^2","description":"keep"}},{"type":"text","text":"tail"}]}]}),
    )?;
    let mut editor = EditorSession::new(source.clone(), Revision(0));
    editor.set_selection(range("source", 1, "source", 4))?;
    let copied = editor.copy_selection()?.ok_or("missing copy")?;
    let slice = RichSlice::decode(copied.rich.as_deref().ok_or("missing private type")?, LIMIT)?;
    assert_eq!(slice.open_start, 1);
    assert_eq!(slice.open_end, 1);
    assert_eq!(slice.content[0].attrs, source.blocks()[0].attrs);
    assert_eq!(slice.content[0].extra, source.blocks()[0].extra);
    assert_eq!(slice.content[0].children()[0].text.as_deref(), Some("😀Б"));
    assert_eq!(
        slice.content[0].children()[0].marks,
        source.blocks()[0].children()[0].marks
    );
    assert_eq!(
        slice.content[0].children()[1],
        source.blocks()[0].children()[1]
    );
    assert_eq!(copied.plain, "😀Б$x^2$");
    assert!(!copied.plain.contains('\u{fffc}'));
    assert!(
        !copied
            .html
            .as_deref()
            .ok_or("missing html")?
            .contains("vendor")
    );
    assert_eq!(editor.document(), &source);
    Ok(())
}
#[test]
fn single_block_rich_paste_keeps_target_style_and_merges_source_provenance() -> TestResult {
    let source = Node::block(
        "paragraph",
        Map::from_iter([
            ("blockId".into(), json!("copied")),
            ("sourceRefs".into(), json!(["voice"])),
        ]),
        vec![Node::text("😀")],
    );
    let bundle = bundle(RichSlice {
        content: vec![source],
        open_start: 1,
        open_end: 1,
    })?;
    let mut editor = session()?;
    let original = editor.document().clone();
    let change = editor.paste_bundle(&bundle, LIMIT)?;
    assert_eq!(editor.document().block_text("target")?, "left😀right");
    assert_eq!(
        editor.document().blocks()[0]
            .attrs
            .as_ref()
            .and_then(|attrs| attrs.get("sourceRefs")),
        Some(&json!(["voice"]))
    );
    assert_eq!(editor.selection(), Some(&caret("target", 5)));
    assert_eq!(
        change.replay_proof().verify(&original)?.0,
        editor.document().clone()
    );
    editor.undo()?;
    assert_eq!(editor.document(), &original);
    assert_eq!(editor.selection(), Some(&caret("target", 4)));
    editor.redo()?;
    assert_eq!(editor.selection(), Some(&caret("target", 5)));
    Ok(())
}
#[test]
fn multi_paragraph_open_paste_joins_rich_edges_and_records_one_history_entry() -> TestResult {
    let copied = bundle(RichSlice {
        content: vec![p("first", "One"), p("second", "Two")],
        open_start: 1,
        open_end: 1,
    })?;
    let mut editor = session()?;
    let original = editor.document().clone();
    let change = editor.paste_bundle(&copied, LIMIT)?;
    assert_eq!(editor.document().blocks().len(), 2);
    assert_eq!(editor.document().blocks()[0].inline_text(), "leftOne");
    assert_eq!(editor.document().blocks()[1].inline_text(), "Tworight");
    let last_id = editor.document().blocks()[1]
        .block_id()
        .ok_or("missing last identity")?
        .to_owned();
    assert_ne!(last_id, "second");
    assert_eq!(editor.selection(), Some(&caret(&last_id, 3)));
    assert_eq!(change.after_revision, Revision(1));
    assert_eq!(
        change.replay_proof().verify(&original)?.0,
        editor.document().clone()
    );
    editor.undo()?;
    assert_eq!(editor.document(), &original);
    assert_eq!(editor.selection(), Some(&caret("target", 4)));
    editor.redo()?;
    assert_eq!(editor.selection(), Some(&caret(&last_id, 3)));
    Ok(())
}
#[test]
fn paragraph_break_copy_retains_both_empty_open_boundaries() -> TestResult {
    let source = Document::from_root(Node::block(
        "doc",
        Map::new(),
        vec![p("a", "A"), p("b", "B")],
    ))?;
    let mut editor = EditorSession::new(source, Revision(0));
    editor.set_selection(range("a", 1, "b", 0))?;
    let copied = editor.copy_selection()?.ok_or("missing boundary copy")?;
    assert_eq!(copied.plain, "\n");
    let slice = RichSlice::decode(copied.rich.as_deref().ok_or("missing rich")?, LIMIT)?;
    assert_eq!(slice.content.len(), 2);
    let mut target = session()?;
    target.paste_bundle(&copied, LIMIT)?;
    assert_eq!(target.document().blocks().len(), 2);
    assert_eq!(target.document().blocks()[0].inline_text(), "left");
    assert_eq!(target.document().blocks()[1].inline_text(), "right");
    Ok(())
}
#[test]
fn opaque_block_copy_has_real_plain_fallback_and_paste_renews_only_identity() -> TestResult {
    let source = Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":2},"content":[{"type":"mermaidBlock","attrs":{"blockId":"diagram","code":"flowchart TD\nA-->B","caption":"Caption","vendor":null},"vendorNode":true}]}),
    )?;
    let mut editor = EditorSession::new(source.clone(), Revision(0));
    editor.select_node(editor.document().node_ref(&NodePath(vec![0]))?)?;
    let copied = editor.copy_selection()?.ok_or("missing atom copy")?;
    assert!(
        copied
            .plain
            .contains("```mermaid\nflowchart TD\nA-->B\n```")
    );
    assert!(!copied.plain.contains('\u{fffc}'));
    let mut target = session()?;
    target.paste_bundle(&copied, LIMIT)?;
    let inserted = &target.document().blocks()[1];
    assert_ne!(inserted.block_id(), Some("diagram"));
    assert_eq!(inserted.extra, source.blocks()[0].extra);
    assert_eq!(
        inserted
            .attrs
            .as_ref()
            .and_then(|attrs| attrs.get("vendor")),
        Some(&json!(null))
    );
    assert_eq!(
        inserted.attrs.as_ref().and_then(|attrs| attrs.get("code")),
        source.blocks()[0]
            .attrs
            .as_ref()
            .and_then(|attrs| attrs.get("code"))
    );
    Ok(())
}
#[test]
fn malformed_private_type_never_falls_back_to_replacing_selection_with_plain_text() -> TestResult {
    for value in [
        json!({"version":99,"content":[]}),
        json!({"version":1,"openStart":65,"openEnd":0,"content":[]}),
        json!({"version":1,"openStart":0,"openEnd":0,"content":[{"type":"paragraph","attrs":{"blockId":"p"},"content":[{"type":"text","text":"bad","marks":[{"type":"link","attrs":{"href":"javascript:bad"}}]}]}]}),
    ] {
        let mut target = session()?;
        let original = target.document().clone();
        let bundle = ClipboardBundle {
            rich: Some(serde_json::to_vec(&value)?),
            plain: "must not replace".into(),
            html: None,
            ..Default::default()
        };
        assert!(target.paste_bundle(&bundle, LIMIT).is_err());
        assert_eq!(target.document(), &original);
        assert!(!target.can_undo());
        assert!(target.take_pending_changes().is_empty());
    }
    Ok(())
}
#[derive(Default)]
struct MockClipboard {
    content: Mutex<ClipboardBundle>,
    fail_write: bool,
}
impl ClipboardProvider for MockClipboard {
    fn read(&self) -> ClipboardResult<ClipboardBundle> {
        Ok(self
            .content
            .lock()
            .map_err(|_| ClipboardError::Read)?
            .clone())
    }
    fn write(&self, bundle: &ClipboardBundle) -> ClipboardResult<()> {
        if self.fail_write {
            return Err(ClipboardError::Write);
        }
        *self.content.lock().map_err(|_| ClipboardError::Write)? = bundle.clone();
        Ok(())
    }
    fn max_bytes(&self) -> usize {
        LIMIT
    }
}
#[test]
fn clipboard_write_failure_keeps_selected_text_revision_selection_and_history() -> TestResult {
    let mut editor = session()?;
    editor.set_selection(range("target", 1, "target", 4))?;
    editor.set_clipboard_provider(Some(Arc::new(MockClipboard {
        fail_write: true,
        ..Default::default()
    })));
    let original = editor.document().clone();
    assert!(editor.cut_to_clipboard().is_err());
    assert_eq!(editor.document(), &original);
    assert_eq!(editor.revision(), Revision(0));
    assert_eq!(editor.selection(), Some(&range("target", 1, "target", 4)));
    assert!(!editor.can_undo());
    assert!(editor.take_pending_changes().is_empty());
    Ok(())
}
#[test]
fn acknowledged_cut_and_rich_paste_use_the_same_canonical_history() -> TestResult {
    let provider = Arc::new(MockClipboard::default());
    let mut editor = session()?;
    editor.set_clipboard_provider(Some(provider.clone()));
    editor.set_selection(range("target", 1, "target", 4))?;
    editor.cut_to_clipboard()?;
    assert_eq!(editor.document().block_text("target")?, "lright");
    assert_eq!(provider.read()?.plain, "eft");
    editor.paste_from_clipboard()?;
    assert_eq!(editor.document().block_text("target")?, "leftright");
    editor.undo()?;
    assert_eq!(editor.document().block_text("target")?, "lright");
    editor.undo()?;
    assert_eq!(editor.document().block_text("target")?, "leftright");
    Ok(())
}
#[test]
fn complete_fence_and_math_paste_are_real_blocks_but_raw_mermaid_is_plain() -> TestResult {
    let cases = [
        (
            "```RuSt\r\nlet x = 1;  \r\n\tprintln!(\"x\");\r\n```\r\n",
            "codeBlock",
        ),
        ("$$ x^2 + y^2 $$", "mathBlock"),
    ];
    for (plain, kind) in cases {
        let mut editor = session()?;
        editor.paste_bundle(
            &ClipboardBundle {
                plain: plain.into(),
                ..Default::default()
            },
            LIMIT,
        )?;
        assert_eq!(editor.document().blocks()[1].node_type, kind);
        editor.undo()?;
        assert_eq!(editor.document().block_text("target")?, "leftright");
    }
    let mut editor = session()?;
    editor.paste_bundle(
        &ClipboardBundle {
            plain: "$x^2$".into(),
            ..Default::default()
        },
        LIMIT,
    )?;
    assert!(
        editor.document().blocks()[0]
            .children()
            .iter()
            .any(|node| node.node_type == "inlineMath")
    );
    let mut editor = session()?;
    editor.paste_bundle(
        &ClipboardBundle {
            plain: "flowchart TD\nA-->B".into(),
            ..Default::default()
        },
        LIMIT,
    )?;
    assert!(
        !editor
            .document()
            .blocks()
            .iter()
            .any(|node| node.node_type == "mermaidBlock")
    );
    Ok(())
}
#[test]
fn html_paste_retains_word_styles_but_discards_active_content_and_unsafe_urls() -> TestResult {
    let mut editor = session()?;
    let html = r#"<script>hidden script</script><p class="MsoNormal"><span style="font-weight:700">Bold</span><span style="font-style:italic;text-decoration:underline">Both</span><a href="javascript:bad" onclick="bad()">label</a></p><div>Next</div>"#;
    editor.paste_bundle(
        &ClipboardBundle {
            html: Some(html.into()),
            plain: String::new(),
            rich: None,
            ..Default::default()
        },
        LIMIT,
    )?;
    let encoded = editor.document().to_json()?.to_string();
    assert!(!encoded.contains("javascript"));
    assert!(!encoded.contains("hidden script"));
    assert!(!encoded.contains("onclick"));
    assert!(encoded.contains("bold"));
    assert!(encoded.contains("italic"));
    assert!(encoded.contains("underline"));
    assert!(editor.document().plain_text().contains("BoldBothlabel"));
    assert!(editor.document().plain_text().contains("Next"));
    Ok(())
}
#[test]
fn html_table_callout_code_diagram_and_math_are_canonical_without_external_fetch() -> TestResult {
    let html = r#"<aside data-callout-kind="warning"><p>Risk</p></aside><table><tbody><tr><th colspan="2">Header</th></tr><tr><td>A</td><td>B</td></tr></tbody></table><pre><code class="language-rust">let x = 1;</code></pre><figure data-mermaid-block><pre data-mermaid-source>flowchart TD
A--&gt;B</pre><figcaption>Diagram</figcaption><svg><script>hidden</script></svg></figure><figure data-math-block data-latex="x^2"><pre data-math-source>x^2</pre></figure><figure data-image-block><img src="https://example.test/image.png" alt="Map" width="640"><figcaption>Caption</figcaption></figure>"#;
    let mut editor = session()?;
    editor.paste_bundle(
        &ClipboardBundle {
            html: Some(html.into()),
            plain: String::new(),
            rich: None,
            ..Default::default()
        },
        LIMIT,
    )?;
    let kinds = editor
        .document()
        .blocks()
        .iter()
        .map(|node| node.node_type.as_str())
        .collect::<Vec<_>>();
    for kind in [
        "callout",
        "table",
        "codeBlock",
        "mermaidBlock",
        "mathBlock",
        "imageBlock",
    ] {
        assert!(kinds.contains(&kind), "missing {kind}");
    }
    assert!(!editor.document().to_json()?.to_string().contains("hidden"));
    Ok(())
}
#[test]
fn paste_budget_is_explicit_and_failure_preserves_content() -> TestResult {
    let mut editor = session()?;
    let original = editor.document().clone();
    assert!(
        editor
            .paste_bundle(
                &ClipboardBundle {
                    plain: "long input".into(),
                    ..Default::default()
                },
                3
            )
            .is_err()
    );
    assert_eq!(editor.document(), &original);
    assert!(!editor.can_undo());
    Ok(())
}
#[test]
fn full_fixture_copy_preserves_every_selected_node_and_mark_without_whole_document_leak()
-> TestResult {
    let source = Document::from_json(serde_json::from_str(include_str!(
        "../examples/all_nodes_fixture.json"
    ))?)?;
    fn points(node: &Node, output: &mut Vec<(String, usize)>) {
        if matches!(
            node.node_type.as_str(),
            "paragraph" | "heading" | "codeBlock"
        ) {
            if let Some(id) = node.block_id() {
                output.push((id.into(), node.inline_text().chars().count()));
            }
        } else {
            for child in node.children() {
                points(child, output);
            }
        }
    }
    let mut ends = Vec::new();
    points(source.root(), &mut ends);
    let first = ends.first().ok_or("missing first")?;
    let last = ends.last().ok_or("missing last")?;
    let mut editor = EditorSession::new(source.clone(), Revision(0));
    editor.set_selection(range(&first.0, 0, &last.0, last.1))?;
    let copied = editor.copy_selection()?.ok_or("missing full copy")?;
    let slice = RichSlice::decode(copied.rich.as_deref().ok_or("missing rich")?, LIMIT)?;
    assert_eq!(slice.content, source.blocks());
    assert!(!copied.plain.contains('\u{fffc}'));
    assert!(!copied.plain.contains("vendorRoot"));
    Ok(())
}
