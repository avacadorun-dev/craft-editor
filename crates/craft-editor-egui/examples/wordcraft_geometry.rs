//! Hosted-only native layout qualification. This is not a rich-editor acceptance demo.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let report = craft_editor_egui::layout::qualification_report()?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
