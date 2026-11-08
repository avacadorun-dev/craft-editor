//! Hosted qualification output; no GUI or browser runtime is used.
use craft_editor_model::Document;
use craft_editor_render::{AssetMap, RenderOptions, Renderer};
use serde_json::json;
use std::{path::PathBuf, sync::Arc};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("artifacts/native-atoms"));
    std::fs::create_dir_all(&output)?;
    let mut assets = AssetMap::default();
    assets.insert("demo-illustration", Arc::new(br##"<svg xmlns="http://www.w3.org/2000/svg" width="640" height="260" viewBox="0 0 640 260"><rect width="640" height="260" rx="12" fill="#e8edf5"/><path d="M0 190 Q100 140 220 182 T440 178 T640 170 V260 H0Z" fill="#567fad"/><path d="M0 220 Q130 165 270 210 T540 205 T640 210 V260 H0Z" fill="#345e89"/><path d="M250 199 L285 70 H335 L370 199Z" fill="#fbf4e4"/><path d="M275 72 V49 H345 V72Z" fill="#3c4b5e"/><path d="M268 48 L310 21 L352 48Z" fill="#577eae"/><rect x="300" y="125" width="20" height="48" rx="9" fill="#53687f"/><path d="M278 61 H342" stroke="#f2c96d" stroke-width="7"/><circle cx="510" cy="64" r="25" fill="#f5d997"/></svg>"##.to_vec()));
    let document = Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":4},"content":[
            {"type":"mermaidBlock","attrs":{"blockId":"flowchart","code":"flowchart LR\n Idea[An idea] --> Draft[A draft]\n Draft --> Review{Ready?}\n Review -->|Yes| Publish[Publication]\n Review -->|Again| Draft","caption":"A real native flowchart"}},
            {"type":"mermaidBlock","attrs":{"blockId":"sequence","code":"sequenceDiagram\n participant A as Author\n participant E as Editor\n A->>E: Send a draft\n E-->>A: Return notes\n A->>E: Revise","caption":"A real native sequence diagram"}},
            {"type":"mermaidBlock","attrs":{"blockId":"state","code":"stateDiagram-v2\n [*] --> Writing\n Writing --> Review\n Review --> Writing: Revise\n Review --> Published\n Published --> [*]","caption":"A real native state diagram"}},
            {"type":"mathBlock","attrs":{"blockId":"quadratic","latex":"\\frac{-b \\pm \\sqrt{b^2-4ac}}{2a}","description":"The quadratic formula"}},
            {"type":"mathBlock","attrs":{"blockId":"summation","latex":"\\sum_{i=1}^{n} i = \\frac{n(n+1)}{2}","description":"The sum of the first n integers"}},
            {"type":"paragraph","attrs":{"blockId":"inline"},"content":[{"type":"text","text":"Inline: "},{"type":"inlineMath","attrs":{"latex":"E=mc^2","description":"Mass energy equivalence"}},{"type":"text","text":" remains inline."}]},
            {"type":"imageBlock","attrs":{"blockId":"illustration","assetId":"demo-illustration","src":"host-asset:demo-illustration","alt":"A lighthouse by a calm sea","caption":"A synthetic vector illustration rendered natively"}},
            {"type":"mermaidBlock","attrs":{"blockId":"explicit-failure","code":"not Mermaid <source> & metadata","caption":"A clearly labelled source fallback"}}
        ]}),
    )?;
    let before = document.to_json()?;
    let renderer = Renderer::new(RenderOptions::default())?;
    let catalog = renderer.prepare_document(&document, &assets, 1)?;
    let mut report = vec![];
    fn visit(
        node: &craft_editor_model::Node,
        renderer: &Renderer,
        assets: &AssetMap,
        output: &std::path::Path,
        report: &mut Vec<serde_json::Value>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if matches!(
            node.node_type.as_str(),
            "mermaidBlock" | "imageBlock" | "inlineMath" | "mathBlock"
        ) && node.block_id() != Some("explicit-failure")
        {
            let image = renderer.render_atom(node, assets)?;
            let name = node.block_id().unwrap_or("inline-formula");
            std::fs::write(output.join(format!("{name}.png")), image.encoded.as_slice())?;
            report.push(json!({"node_type":node.node_type,"identity":name,"width":image.size.width,"height":image.size.height,"baseline":image.baseline,"png_bytes":image.encoded.len()}));
        }
        for child in node.children() {
            visit(child, renderer, assets, output, report)?;
        }
        Ok(())
    }
    visit(document.root(), &renderer, &assets, &output, &mut report)?;
    let broken = document
        .blocks()
        .last()
        .ok_or("Missing synthetic broken atom")?;
    use craft_editor_egui::renderers::AtomResolver;
    let fallback = catalog.resolve(broken, 640.0)?;
    std::fs::write(
        output.join("explicit-failure.png"),
        fallback.encoded.as_slice(),
    )?;
    let evidence = json!({"checkpoint":"native-atom-renderers","browser_runtime":false,"source_json_preserved":before==document.to_json()?,"atoms":report,"catalog_entries":catalog.len(),"explicit_failures":catalog.failures().iter().map(|failure|json!({"kind":failure.node_type,"block_id":failure.block_id,"code":failure.error.to_string()})).collect::<Vec<_>>(),"editor_acceptance":false,"upstream_syntax_parity_acceptance":false});
    let encoded = serde_json::to_string_pretty(&evidence)?;
    std::fs::write(output.join("render-report.json"), &encoded)?;
    println!("{encoded}");
    Ok(())
}
