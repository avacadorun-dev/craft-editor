use craft_editor_egui::{
    DocPoint,
    layout::{FlowGeometry, GeometryStyle, LineHeightBasis},
    renderers::AtomResolver,
};
use craft_editor_model::{Document, NodePath, ScalarOffset};
use craft_editor_render::AssetMap;
use serde_json::{Value, json};
use std::sync::Arc;
pub type EvidenceResult<T> = Result<T, Box<dyn std::error::Error>>;

pub fn fixture() -> EvidenceResult<(Document, AssetMap)> {
    let mut assets = AssetMap::default();
    assets.insert("tall-illustration", Arc::new(br##"<svg xmlns="http://www.w3.org/2000/svg" width="480" height="750"><rect width="480" height="750" fill="#acc9e8"/><path d="M0 580 Q160 470 320 590 T480 560 V750 H0Z" fill="#355c8a"/><path d="M130 590 L180 170 H300 L350 590Z" fill="#fff4dc"/><rect x="168" y="130" width="144" height="50" fill="#34465e"/><path d="M150 130 L240 65 L330 130Z" fill="#376a9d"/><rect x="222" y="360" width="36" height="90" fill="#52627a"/><circle cx="380" cy="100" r="38" fill="#f6d99b"/></svg>"##.to_vec()));
    let document = Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":4},"content":[
            {"type":"heading","attrs":{"blockId":"heading","level":2},"content":[{"type":"text","text":"Mixed native block flow"}]},
            {"type":"imageBlock","attrs":{"blockId":"image","assetId":"tall-illustration","src":"host-asset:tall-illustration","alt":"A tall synthetic lighthouse","caption":"Image caption stays below the entire illustration","width":480,"height":750}},
            {"type":"codeBlock","attrs":{"blockId":"code","language":"rust"},"content":[{"type":"text","text":"fn after_image() {\n    let caption_is_above = true;\n}"}]},
            {"type":"table","attrs":{"blockId":"table"},"content":[{"type":"tableRow","content":[{"type":"tableHeader","content":[{"type":"paragraph","attrs":{"blockId":"cell-a"},"content":[{"type":"text","text":"Table below code"}]}]},{"type":"tableHeader","content":[{"type":"paragraph","attrs":{"blockId":"cell-b"},"content":[{"type":"text","text":"No painted overlap"}]}]}]},{"type":"tableRow","content":[{"type":"tableCell","content":[{"type":"paragraph","attrs":{"blockId":"cell-c"},"content":[{"type":"text","text":"Native row"}]}]},{"type":"tableCell","content":[{"type":"paragraph","attrs":{"blockId":"cell-d"},"content":[{"type":"text","text":"Native geometry"}]}]}]}]},
            {"type":"mermaidBlock","attrs":{"blockId":"flow","code":"flowchart TD\n Idea[An idea] --> Draft[A draft]\n Draft --> Review{Ready?}\n Review -->|Again| Draft\n Review -->|Yes| Publish[Publication]","caption":"Flowchart caption remains outside the diagram"}},
            {"type":"mermaidBlock","attrs":{"blockId":"sequence","code":"sequenceDiagram\n participant A as Author\n participant E as Editor\n A->>E: Send manuscript\n E-->>A: Return notes\n A->>E: Revise\n E-->>A: Ready","caption":"Sequence diagram follows the complete flowchart"}},
            {"type":"mathBlock","attrs":{"blockId":"math","latex":"\\frac{-b \\pm \\sqrt{b^2-4ac}}{2a}","description":"The quadratic formula"}},
            {"type":"paragraph","attrs":{"blockId":"inline"},"content":[{"type":"text","text":"Inline "},{"type":"inlineMath","attrs":{"latex":"\\frac{\\frac{a}{b}}{\\frac{c}{d}}","description":"A tall nested fraction"}},{"type":"hardBreak"},{"type":"text","text":"The next line remains below the fraction."}]},
            {"type":"paragraph","attrs":{"blockId":"end"},"content":[{"type":"text","text":"The manuscript continues after every measured block."}]}
        ]}),
    )?;
    Ok((document, assets))
}
pub fn style(dark: bool) -> GeometryStyle {
    GeometryStyle {
        font_size: 19.0,
        line_height: 1.75,
        line_height_basis: LineHeightBasis::FontSize,
        text_color: if dark { [225, 230, 238] } else { [35, 39, 47] },
        ..Default::default()
    }
}
pub fn verify(
    document: &Document,
    width: f32,
    style: &GeometryStyle,
    resolver: &dyn AtomResolver,
) -> EvidenceResult<(FlowGeometry, Value)> {
    let geometry = FlowGeometry::with_resolver(document, width, style, resolver)?;
    let nodes = geometry.nodes();
    let rect = |index| {
        nodes
            .iter()
            .find(|node| node.canonical_path == NodePath(vec![index]))
            .map(|node| node.rect)
            .ok_or("missing opaque geometry")
    };
    let image = rect(1)?;
    let flow = rect(4)?;
    let sequence = rect(5)?;
    let math = rect(6)?;
    let caret = |id: &str, offset| {
        geometry
            .caret(&DocPoint {
                block_id: id.into(),
                offset: ScalarOffset(offset),
            })?
            .ok_or(craft_editor_egui::layout::Error::UnmappedPosition)
    };
    let code = caret("code", 0)?;
    let cells = geometry.cells()?;
    let table_top = cells
        .iter()
        .map(|cell| cell.rect.y)
        .fold(f32::INFINITY, f32::min);
    let table_bottom = cells
        .iter()
        .map(|cell| cell.rect.y + cell.rect.height)
        .fold(0.0, f32::max);
    for (label, top, previous_bottom) in [
        (
            "code after tall image and caption",
            code.y,
            image.y + image.height,
        ),
        ("table after code", table_top, {
            let last = caret("code", document.block_text("code")?.chars().count())?;
            last.y + last.height
        }),
        ("flowchart after table", flow.y, table_bottom),
        (
            "sequence after flowchart and caption",
            sequence.y,
            flow.y + flow.height,
        ),
        (
            "formula after sequence and caption",
            math.y,
            sequence.y + sequence.height,
        ),
        (
            "inline paragraph after formula",
            caret("inline", 0)?.y,
            math.y + math.height,
        ),
    ] {
        if top + 0.1 < previous_bottom {
            return Err(
                format!("painted bounds overlap: {label}: {top} < {previous_bottom}").into(),
            );
        }
    }
    let inline = nodes
        .iter()
        .find(|node| node.canonical_path == NodePath(vec![7, 1]))
        .ok_or("missing inline formula geometry")?
        .rect;
    let next_line = caret("inline", 9)?;
    if next_line.y + 0.1 < inline.y + inline.height {
        return Err("inline object overlaps the following native line".into());
    }
    let end = caret("end", 0)?;
    if end.y <= next_line.y {
        return Err("following prose did not advance below the inline paragraph".into());
    }
    let bounds=nodes.iter().map(|node|json!({"path":node.canonical_path.0,"x":node.rect.x,"y":node.rect.y,"width":node.rect.width,"height":node.rect.height})).collect::<Vec<_>>();
    Ok((
        geometry,
        json!({"width":width,"font_size":style.font_size,"line_height":style.line_height,"bounds":bounds,"actual_painted_bounds_nonoverlapping":true}),
    ))
}
