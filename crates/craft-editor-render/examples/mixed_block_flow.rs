//! Real native mixed-block flow evidence for tall-object reservation; no browser runtime.
#[path = "../tests/support/mixed_block_fixture.rs"]
mod fixture;
use craft_editor_egui::layout::GeometryRect;
use craft_editor_render::{RenderOptions, Renderer};
use serde_json::json;
use std::path::PathBuf;
fn main() -> fixture::EvidenceResult<()> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("missing output directory")?;
    std::fs::create_dir_all(&output)?;
    let mut reports = vec![];
    for dark in [false, true] {
        let (document, assets) = fixture::fixture()?;
        let original = document.to_json()?;
        let renderer = Renderer::new(RenderOptions {
            dark,
            pixel_scale: 1.0,
            ..Default::default()
        })?;
        let catalog = renderer.prepare_document(&document, &assets, 1)?;
        if !catalog.failures().is_empty() {
            return Err("mixed-block fixture contains an unrendered atom".into());
        }
        for width in [520.0, 820.0] {
            let (geometry, mut report) =
                fixture::verify(&document, width, &fixture::style(dark), &catalog)?;
            let extent = geometry.extent().ok_or("missing native extent")?;
            let background = if dark { [29, 34, 42] } else { [250, 250, 248] };
            let raster = geometry.rasterize(
                GeometryRect {
                    page: 0,
                    x: 0.0,
                    y: 0.0,
                    width,
                    height: extent.height,
                },
                1.0,
                background,
            )?;
            let name = format!(
                "mixed-{}-{}.png",
                if dark { "dark" } else { "light" },
                width as u32
            );
            std::fs::write(output.join(&name), raster.to_png())?;
            report["file"] = json!(name);
            report["raster_height"] = json!(raster.height);
            reports.push(report);
        }
        if original != document.to_json()? {
            return Err("native evidence modified canonical source".into());
        }
    }
    std::fs::write(
        output.join("mixed-block-report.json"),
        serde_json::to_vec_pretty(
            &json!({"native_geometry_and_raster":true,"browser_runtime":false,"owner_acceptance":false,"renders":reports}),
        )?,
    )?;
    Ok(())
}
