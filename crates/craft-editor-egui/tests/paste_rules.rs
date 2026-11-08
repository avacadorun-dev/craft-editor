use craft_editor_egui::clipboard::{RichSlice, rules};
use craft_editor_model::{Document, Node};
use serde_json::{Value, json};
type TestResult = Result<(), Box<dyn std::error::Error>>;
const LIMIT: usize = 4 * 1024 * 1024;
#[test]
fn source_marker_and_autolink_corpus_matches_installed_original_without_inventing_markdown_links()
-> TestResult {
    let corpus: Value =
        serde_json::from_str(include_str!("fixtures/source-mark-paste-rules.json"))?;
    let cases = corpus["cases"]
        .as_array()
        .ok_or("missing source mark cases")?;
    assert_eq!(cases.len(), 20);
    for case in cases {
        let text = case["text"].as_str().ok_or("invalid fixture text")?;
        let slice = rules::plain_slice(text, &Node::text(""), LIMIT)?;
        assert_eq!(
            serde_json::to_value(slice.content[0].children())?,
            case["content"],
            "{text}"
        );
    }
    Ok(())
}
#[test]
fn raw_url_finder_corpus_preserves_source_boundaries_and_protocol_policy() -> TestResult {
    let corpus: Value = serde_json::from_str(include_str!("fixtures/source-url-rules.json"))?;
    let cases = corpus["cases"]
        .as_array()
        .ok_or("missing source URL cases")?;
    assert_eq!(cases.len(), 46);
    for case in cases {
        let text = case["text"].as_str().ok_or("invalid fixture text")?;
        let links=rules::link_spans(text)?.into_iter().map(|span|json!({"value":&text[span.bytes.clone()],"href":span.href,"from":text[..span.bytes.start].chars().count(),"to":text[..span.bytes.end].chars().count()})).collect::<Vec<_>>();
        assert_eq!(json!(links), case["links"], "{text}");
    }
    Ok(())
}
#[test]
fn adjacent_marker_spans_are_fixed_and_existing_code_is_never_reinterpreted() -> TestResult {
    let slice = rules::plain_slice("**one** **two**", &Node::text(""), LIMIT)?;
    assert_eq!(slice.content[0].inline_text(), "one two");
    let children = slice.content[0].children();
    for text in ["one", "two"] {
        assert!(children.iter().any(|node| {
            node.text.as_deref() == Some(text)
                && node
                    .marks
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .any(|mark| mark.mark_type == "bold")
        }));
    }
    let document = Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":4},"content":[
        {"type":"paragraph","attrs":{"blockId":"p"},"content":[{"type":"text","text":"**bold** https://example.com","marks":[{"type":"code","vendor":null}]}]},
        {"type":"codeBlock","attrs":{"blockId":"code","language":"text","vendor":null},"content":[{"type":"text","text":"**bold** https://example.com"}]}]}),
    )?;
    let mut slice = RichSlice {
        content: document.blocks().to_vec(),
        open_start: 0,
        open_end: 0,
    };
    let original = slice.clone();
    rules::transform_external(&mut slice, LIMIT)?;
    assert_eq!(slice, original);
    Ok(())
}
#[test]
fn fragmenting_marker_input_preserves_all_surviving_opaque_node_and_mark_envelopes() -> TestResult {
    let doc = Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":4},"content":[{"type":"blockquote","attrs":{"blockId":"quote","sourceRefs":["voice"],"vendor":null},"opaqueWrapper":null,"content":[{"type":"paragraph","attrs":{"blockId":"p","vendor":true},"opaqueBlock":{"keep":1},"content":[{"type":"text","text":"before **bold** after","attrs":null,"marks":[{"type":"underline","attrs":{"opaque":null},"vendorMark":null}],"vendorLeaf":{"keep":2}}]}]}]}),
    )?;
    let original = doc.blocks()[0].clone();
    let mut slice = RichSlice {
        content: doc.blocks().to_vec(),
        open_start: 2,
        open_end: 2,
    };
    rules::transform_external(&mut slice, LIMIT)?;
    let quote = &slice.content[0];
    assert_eq!(quote.attrs, original.attrs);
    assert_eq!(quote.extra, original.extra);
    let paragraph = &quote.children()[0];
    assert_eq!(paragraph.attrs, original.children()[0].attrs);
    assert_eq!(paragraph.extra, original.children()[0].extra);
    assert_eq!(paragraph.inline_text(), "before bold after");
    let old = &original.children()[0].children()[0];
    for node in paragraph.children() {
        assert_eq!(node.attrs, old.attrs);
        assert_eq!(node.extra, old.extra);
        assert_eq!(
            &node.marks.as_ref().ok_or("missing surviving marks")?[0],
            &old.marks.as_ref().ok_or("missing original marks")?[0]
        );
        assert!(
            serde_json::to_value(node)?
                .get("attrs")
                .is_some_and(Value::is_null)
        );
    }
    Ok(())
}
#[test]
fn opaque_metadata_expansion_is_bounded_before_fragment_cloning_and_errors_are_atomic() -> TestResult
{
    let document = Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":4},"content":[{"type":"paragraph","attrs":{"blockId":"p"},"content":[{"type":"text","text":"**a** x **b** x **c** x **d**","opaque":"x".repeat(1024)}]}]}),
    )?;
    let mut slice = RichSlice {
        content: document.blocks().to_vec(),
        open_start: 1,
        open_end: 1,
    };
    let original = slice.clone();
    assert!(rules::transform_external(&mut slice, 2000).is_err());
    assert_eq!(slice, original);
    assert!(rules::transform_external(&mut slice, 16).is_err());
    assert_eq!(slice, original);
    Ok(())
}
