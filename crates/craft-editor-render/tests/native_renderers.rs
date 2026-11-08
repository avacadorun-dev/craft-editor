use craft_editor_egui::renderers::AtomResolver;
use craft_editor_model::{Document, Node};
use craft_editor_render::{AssetMap, Error, RenderOptions, Renderer};
use serde_json::{Value, json};
use std::sync::Arc;

fn atom(value: Value) -> Node {
    serde_json::from_value(value).expect("synthetic atom")
}
fn renderer() -> Renderer {
    Renderer::new(RenderOptions::default()).expect("valid renderer options")
}
fn ink_count(bytes: &[u8]) -> usize {
    image::load_from_memory(bytes)
        .expect("real encoded PNG")
        .to_rgba8()
        .pixels()
        .filter(|pixel| pixel.0[3] > 0)
        .count()
}

#[test]
fn native_mermaid_flowchart_contains_real_raster_ink() {
    let image = renderer().render_atom(&atom(json!({"type":"mermaidBlock","attrs":{"code":"flowchart LR\n A[Start] --> B[Done]"}})), &AssetMap::default()).expect("native flowchart");
    assert!(image.size.width > 100.0 && image.size.height > 20.0);
    assert!(ink_count(&image.encoded) > 500);
}

#[test]
fn native_mermaid_sequence_and_state_families_render_without_a_browser() {
    for source in [
        "sequenceDiagram\n participant A as Author\n participant E as Editor\n A->>E: Draft\n E-->>A: Saved",
        "stateDiagram-v2\n [*] --> Writing\n Writing --> Review\n Review --> [*]",
    ] {
        let image = renderer()
            .render_atom(
                &atom(json!({"type":"mermaidBlock","attrs":{"code":source}})),
                &AssetMap::default(),
            )
            .expect("real native diagram family");
        assert!(ink_count(&image.encoded) > 500);
    }
}

#[test]
fn native_formula_fraction_root_sum_and_inline_baseline_are_measured_from_layout() {
    let renderer = renderer();
    for source in [
        r"\frac{-b \pm \sqrt{b^2-4ac}}{2a}",
        r"\sum_{i=1}^{n} i = \frac{n(n+1)}{2}",
    ] {
        let image = renderer
            .render_atom(
                &atom(json!({"type":"inlineMath","attrs":{"latex":source}})),
                &AssetMap::default(),
            )
            .expect("real math layout");
        let baseline = image.baseline.expect("measured formula baseline");
        assert!(baseline > 0.0 && baseline < image.size.height);
        assert!(ink_count(&image.encoded) > 20);
        let fitted = image.fit(image.size.width / 2.0).expect("fitted baseline");
        assert!((fitted.baseline.expect("scaled baseline") - baseline / 2.0).abs() < 0.01);
    }
}

#[test]
fn source_image_bytes_are_authorized_by_asset_id_and_preserve_requested_size() {
    let mut assets = AssetMap::default();
    assets.insert("illustration", Arc::new(br##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><rect width="80" height="40" fill="#3f6cb1"/><circle cx="40" cy="20" r="12" fill="#f2e4b8"/></svg>"##.to_vec()));
    let image = renderer().render_atom(&atom(json!({"type":"imageBlock","attrs":{"assetId":"illustration","src":"must-not-open-this-path","width":160,"alt":"A synthetic illustration"}})), &assets).expect("host supplied SVG rendered natively");
    assert_eq!(image.size.width, 160.0);
    assert_eq!(image.size.height, 80.0);
    assert!(ink_count(&image.encoded) > 100);
}

#[test]
fn invalid_formula_reports_failure_without_modifying_authored_source() {
    let node = atom(
        json!({"type":"inlineMath","attrs":{"latex":"\\frac{","description":"Keep this description"},"vendor":{"retain":true}}),
    );
    let before = serde_json::to_value(&node).expect("source snapshot");
    assert_eq!(
        renderer()
            .render_atom(&node, &AssetMap::default())
            .expect_err("invalid math"),
        Error::FormulaRender
    );
    assert_eq!(
        serde_json::to_value(node).expect("source after error"),
        before
    );
}

#[test]
fn one_invalid_atom_has_an_explicit_source_fallback_without_hiding_valid_neighbors() {
    let document = Document::from_json(json!({"type":"doc","attrs":{"schemaVersion":4},"content":[
        {"type":"mermaidBlock","attrs":{"blockId":"broken","code":"not Mermaid <source> & metadata","caption":"Keep caption"}},
        {"type":"mathBlock","attrs":{"blockId":"valid","latex":"x^2+y^2=z^2","description":"Pythagoras"}}
    ]})).expect("valid canonical atom document");
    let before = document.to_json().expect("snapshot");
    let catalog = renderer()
        .prepare_document(&document, &AssetMap::default(), 7)
        .expect("source fallback catalog");
    assert_eq!(catalog.len(), 2);
    assert_eq!(catalog.failures().len(), 1);
    assert_eq!(catalog.failures()[0].block_id.as_deref(), Some("broken"));
    let output = catalog
        .resolve(&document.blocks()[0], 300.0)
        .expect("labelled source fallback");
    assert!(ink_count(&output.encoded) > 100);
    assert_eq!(catalog.generation(), 7);
    assert_eq!(
        document.to_json().expect("unchanged canonical source"),
        before
    );
}

#[test]
fn arbitrary_svg_file_or_network_references_are_refused_before_rendering() {
    for href in [
        "file:///private/source.png",
        "https://example.invalid/tracking.png",
        "../outside.png",
    ] {
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><image href="{href}" width="20" height="20"/></svg>"#
        );
        let mut assets = AssetMap::default();
        assets.insert("image", Arc::new(svg.into_bytes()));
        let result = renderer().render_atom(
            &atom(json!({"type":"imageBlock","attrs":{"assetId":"image"}})),
            &assets,
        );
        assert_eq!(
            result.expect_err("external resource"),
            Error::ExternalResource
        );
    }
}

#[test]
fn raster_and_catalog_limits_fail_explicitly_before_unbounded_output() {
    let mut options = RenderOptions::default();
    options.limits.dimension = 32;
    let bounded = Renderer::new(options).expect("bounded options");
    let mut assets = AssetMap::default();
    assets.insert("image", Arc::new(br#"<svg xmlns="http://www.w3.org/2000/svg" width="1000000" height="1000000"><rect width="1000000" height="1000000"/></svg>"#.to_vec()));
    assert_eq!(
        bounded
            .render_atom(
                &atom(json!({"type":"imageBlock","attrs":{"assetId":"image"}})),
                &assets
            )
            .expect_err("oversized raster"),
        Error::ResourceLimit
    );
    let mut options = RenderOptions::default();
    options.limits.atoms = 1;
    let bounded = Renderer::new(options).expect("catalog bound");
    let document = Document::from_json(json!({"type":"doc","attrs":{"schemaVersion":4},"content":[
        {"type":"mathBlock","attrs":{"blockId":"a","latex":"a"}},
        {"type":"mathBlock","attrs":{"blockId":"b","latex":"b"}}
    ]}))
    .expect("two-atom document");
    assert_eq!(
        bounded
            .prepare_document(&document, &AssetMap::default(), 1)
            .expect_err("atom limit"),
        Error::ResourceLimit
    );
}
