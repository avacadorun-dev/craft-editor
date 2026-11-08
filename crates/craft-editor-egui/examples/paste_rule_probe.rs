//! Hosted candidate comparison, not a completed editor-paste acceptance report.
use craft_editor_egui::clipboard::rules::link_spans;
use serde_json::{Value, json};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source: Value =
        serde_json::from_str(include_str!("../tests/fixtures/source-url-rules.json"))?;
    let cases = source["cases"].as_array().ok_or("missing source corpus")?;
    let mut differences = Vec::new();
    for case in cases {
        let text = case["text"].as_str().ok_or("invalid source text")?;
        let actual=link_spans(text).into_iter().map(|span|json!({"value":&text[span.bytes.clone()],"href":span.href,"from":text[..span.bytes.start].chars().count(),"to":text[..span.bytes.end].chars().count()})).collect::<Vec<_>>();
        if json!(actual) != case["links"] {
            differences.push(json!({"text":text,"expected":case["links"],"actual":actual}));
        }
    }
    println!(
        "{}",
        json!({"scope":"Candidate scanner vs frozen actual source finder; mark/code/internal transforms are separate","source":source["source"],"cases":cases.len(),"mismatches":differences,"full_editor_parity":false})
    );
    Ok(())
}
