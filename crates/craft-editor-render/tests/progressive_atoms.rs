use craft_editor_egui::renderers::AtomResolver;
use craft_editor_model::Document;
use craft_editor_render::{AssetMap, RenderOptions, Renderer};
use serde_json::json;

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn one_atom_catalog_prepares_without_rendering_unrelated_document_atoms() -> TestResult {
    let document = Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":4},"content":[
            {"type":"mathBlock","attrs":{"blockId":"formula","latex":"x^2","vendor":{"retain":true}}},
            {"type":"mermaidBlock","attrs":{"blockId":"unrelated","code":"not a valid diagram"}}
        ]}),
    )?;
    let before = document.to_json()?;
    let atom = &document.root().children()[0];
    let renderer = Renderer::new(RenderOptions::default())?;
    let catalog = renderer.prepare_atom(atom, &AssetMap::default(), 4)?;
    assert_eq!(catalog.len(), 1);
    assert!(catalog.failures().is_empty());
    assert!(catalog.encoded_bytes() > 0);
    assert_eq!(catalog.generation(), 4);
    assert!(catalog.resolve(atom, 500.0).is_ok());
    assert_eq!(document.to_json()?, before);
    Ok(())
}

#[test]
fn a_failed_atom_keeps_explicit_source_fallback_in_its_independent_catalog() -> TestResult {
    let document = Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":4},"content":[
            {"type":"mathBlock","attrs":{"blockId":"formula","latex":"\\href{https://invalid.test}{source}","description":"Preserved source"}}
        ]}),
    )?;
    let before = document.to_json()?;
    let atom = &document.root().children()[0];
    let catalog =
        Renderer::new(RenderOptions::default())?.prepare_atom(atom, &AssetMap::default(), 5)?;
    assert_eq!(catalog.failures().len(), 1);
    assert!(catalog.resolve(atom, 500.0).is_ok());
    assert_eq!(document.to_json()?, before);
    Ok(())
}
