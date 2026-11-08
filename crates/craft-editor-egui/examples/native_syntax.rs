//! Hosted native highlighting/raster evidence; this is not full editor UX acceptance.
use std::path::PathBuf;

use craft_editor_egui::{
    layout::{FlowGeometry, GeometryRect, GeometryStyle},
    syntax::{self, Coverage},
};
use craft_editor_model::Document;
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("missing output directory")?;
    std::fs::create_dir_all(&output)?;
    let coverage: Vec<_> = syntax::coverage_report()
        .into_iter()
        .map(|(language, coverage)| {
            let (status, grammar) = match coverage {
                Coverage::PlainText => ("plain", None),
                Coverage::Highlighted { grammar } => ("highlighted", Some(grammar)),
                Coverage::MissingRequiredGrammar { .. } => ("missing-required-grammar", None),
                Coverage::UnknownLanguage { .. } => ("unknown", None),
            };
            json!({"language":language,"status":status,"grammar":grammar})
        })
        .collect();
    let missing: Vec<_> = coverage
        .iter()
        .filter(|row| row["status"] == "missing-required-grammar")
        .map(|row| row["language"].clone())
        .collect();
    let code = "// Unicode source remains exact: Русский 日本語 🪶\nfn describe_story(title: &str) -> String {\n    let chapter_count = 42;\n    format!(\"{title}: {chapter_count}\")\n}";
    let source = Document::from_json(json!({"type":"doc","attrs":{"schemaVersion":4},"content":[
        {"type":"heading","attrs":{"blockId":"title","level":2},"content":[{"type":"text","text":"Native syntax highlighting"}]},
        {"type":"codeBlock","attrs":{"blockId":"rust","language":"rust"},"content":[{"type":"text","text":code}]},
        {"type":"codeBlock","attrs":{"blockId":"yaml","language":"yaml"},"content":[{"type":"text","text":"story:\n  title: \"The lighthouse\"\n  chapter_count: 42\n  draft: true\n"}]},
        {"type":"codeBlock","attrs":{"blockId":"json","language":"json"},"content":[{"type":"text","text":"{\"title\": \"The lighthouse\", \"draft\": true, \"chapters\": 42}"}]}
    ]}))?;
    let before = source.to_json()?;
    let mut renders = vec![];
    for (name, foreground, background) in [
        ("light", [35, 39, 47], [250, 250, 248]),
        ("dark", [225, 230, 238], [29, 34, 42]),
    ] {
        let style = GeometryStyle {
            text_color: foreground,
            font_size: 18.0,
            syntax_palette: Some(syntax::Palette::for_surface(foreground, background)),
            ..Default::default()
        };
        let geometry = FlowGeometry::with_style(&source, 860.0, &|_| None, Some(&style))?;
        let extent = geometry.extent().ok_or("missing native extent")?;
        let raster = geometry.rasterize(
            GeometryRect {
                page: 0,
                x: 0.0,
                y: 0.0,
                width: 860.0,
                height: extent.height.min(900.0),
            },
            1.5,
            background,
        )?;
        let png = raster.to_png();
        if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Err("invalid syntax evidence PNG".into());
        }
        let file = format!("syntax-{name}.png");
        std::fs::write(output.join(&file), png)?;
        renders.push(json!({"theme":name,"file":file,"width":raster.width,"height":raster.height,"actual_native_code_blocks":geometry.syntax_reports().len()}));
    }
    if source.to_json()? != before {
        return Err("highlighting changed canonical content".into());
    }
    std::fs::write(
        output.join("two-face-NOTICE.md"),
        syntax::acknowledgements_markdown(),
    )?;
    let report = json!({"source_option_count":coverage.len(),"coverage":coverage,"missing_required_grammars":missing,"real_native_rasters":renders,"canonical_json_unchanged":true,"full_syntax_parity":missing.is_empty(),"native_editor_ux_acceptance":false});
    std::fs::write(
        output.join("syntax-report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
