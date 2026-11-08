#![cfg(feature = "native-layout-evidence")]
#[path = "support/mixed_block_fixture.rs"]
mod fixture;
use craft_editor_render::Renderer;
#[test]
fn font_size_spacing_reserves_actual_image_diagram_formula_and_caption_bounds()
-> fixture::EvidenceResult<()> {
    for dark in [false, true] {
        let (document, assets) = fixture::fixture()?;
        let original = document.to_json()?;
        let renderer = Renderer::new(fixture::options(dark))?;
        let catalog = renderer.prepare_document(&document, &assets, 1)?;
        assert!(catalog.failures().is_empty(), "{:?}", catalog.failures());
        for width in [520.0, 820.0] {
            fixture::verify(&document, width, &fixture::style(dark), &catalog)?;
        }
        assert_eq!(document.to_json()?, original);
    }
    Ok(())
}
