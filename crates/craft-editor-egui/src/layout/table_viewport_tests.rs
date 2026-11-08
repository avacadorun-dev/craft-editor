use super::*;
fn writing(id: &str, text: &str) -> Value {
    json!({"type":"paragraph","attrs":{"blockId":id},"content":[{"type":"text","text":text}]})
}
fn wide_source() -> Result<Document> {
    let cells:Vec<_>=(0..63).map(|column|json!({"type":"tableCell","attrs":{"colwidth":[80]},"content":[writing(&format!("cell-{column}"),&format!("Cell {column} 😀"))]})).collect();
    Ok(Document::from_json(
        json!({"type":"doc","attrs":{"schemaVersion":4},"content":[
            writing("before","Ordinary prose stays at its bounded measure."),
            {"type":"table","attrs":{"blockId":"wide"},"content":[{"type":"tableRow","content":cells}]},
            writing("after","Following prose stays in its own original position.")
        ]}),
    )?)
}
#[test]
fn wide_table_viewport_reaches_the_last_column_with_the_same_hit_selection_and_native_pixels()
-> Result<()> {
    let source = wide_source()?;
    let canonical = source.to_json()?;
    let mut geometry = FlowGeometry::new(&source, 600.0)?;
    let before = geometry.caret(&point("before", 0))?;
    let after = geometry.caret(&point("after", 0))?;
    assert_eq!(geometry.table_viewports().len(), 1);
    let viewport = geometry.table_viewports().remove(0);
    assert!(viewport.content_width > 5000.0);
    assert!(viewport.rect.width <= 564.1);
    assert!(geometry.caret(&point("cell-62", 0))?.is_none());
    #[cfg(feature = "wordcraft-canvas")]
    let pixels = geometry
        .rasterize(viewport.rect, 1.0, [250, 250, 248])?
        .pixels;
    assert!(geometry.scroll_table(&viewport.table_path, viewport.maximum)?);
    let caret = geometry
        .caret(&point("cell-62", 0))?
        .ok_or(Error::UnmappedPosition)?;
    assert!(caret.x >= viewport.rect.x && caret.x < viewport.rect.x + viewport.rect.width);
    let hit = geometry
        .hit(0, caret.x + 0.1, caret.y + caret.height * 0.5)?
        .ok_or(Error::UnmappedPosition)?;
    assert_eq!(hit.block_id, "cell-62");
    assert_eq!(hit.offset, ScalarOffset(0));
    let selected = geometry.selection_rects(&Selection {
        anchor: point("cell-62", 0),
        head: point("cell-62", 4),
    })?;
    assert!(!selected.is_empty());
    assert!(selected.iter().all(|rect| rect.x >= viewport.rect.x
        && rect.x + rect.width <= viewport.rect.x + viewport.rect.width + 0.1));
    assert_eq!(geometry.caret(&point("before", 0))?, before);
    assert_eq!(geometry.caret(&point("after", 0))?, after);
    #[cfg(feature = "wordcraft-canvas")]
    assert_ne!(
        geometry
            .rasterize(viewport.rect, 1.0, [250, 250, 248])?
            .pixels,
        pixels
    );
    assert_eq!(source.to_json()?, canonical);
    Ok(())
}
#[test]
fn caret_reveal_and_typography_reflow_keep_presentation_offsets_separate_from_source() -> Result<()>
{
    let source = wide_source()?;
    let canonical = source.to_json()?;
    let mut geometry = FlowGeometry::new(&source, 600.0)?;
    assert!(geometry.reveal_caret(&point("cell-62", 4))?);
    assert!(geometry.caret(&point("cell-62", 4))?.is_some());
    assert!(geometry.reveal_caret(&point("cell-0", 0))?);
    assert!(geometry.caret(&point("cell-0", 0))?.is_some());
    let path = NodePath(vec![1]);
    assert!(geometry.scroll_table(&path, 100_000.0)?);
    let offsets = geometry.table_offsets();
    let mut narrower = FlowGeometry::new(&source, 440.0)?;
    narrower.restore_table_offsets(&offsets);
    let narrow = narrower.table_viewports().remove(0);
    assert!(narrow.offset <= narrow.maximum);
    assert!(narrow.rect.width <= 404.1);
    assert!(narrower.scroll_table(&path, f32::NAN).is_err());
    assert_eq!(source.to_json()?, canonical);
    Ok(())
}
#[test]
fn nested_table_projection_uses_both_ancestor_clips_and_exact_inverse_positions() -> Result<()> {
    let nested_cells:Vec<_>=(0..3).map(|column|json!({"type":"tableCell","attrs":{"colwidth":[160]},"content":[writing(&format!("inner-{column}"),&format!("Inner {column}"))]})).collect();
    let source = Document::from_json(json!({"type":"doc","attrs":{"schemaVersion":4},"content":[
        {"type":"table","attrs":{"blockId":"outer"},"content":[{"type":"tableRow","content":[
            {"type":"tableCell","attrs":{"colwidth":[300]},"content":[{"type":"table","attrs":{"blockId":"inner"},"content":[{"type":"tableRow","content":nested_cells}]}]},
            {"type":"tableCell","attrs":{"colwidth":[300]},"content":[writing("sibling","A different outer cell")]}
        ]}]}
    ]}))?;
    let original = source.to_json()?;
    let mut geometry = FlowGeometry::new(&source, 440.0)?;
    assert_eq!(geometry.table_viewports().len(), 2);
    assert!(geometry.reveal_caret(&point("inner-2", 3))?);
    let caret = geometry
        .caret(&point("inner-2", 3))?
        .ok_or(Error::UnmappedPosition)?;
    let hit = geometry
        .hit(0, caret.x + 0.1, caret.y + caret.height * 0.5)?
        .ok_or(Error::UnmappedPosition)?;
    assert_eq!(hit.block_id, "inner-2");
    assert_eq!(hit.offset, ScalarOffset(3));
    assert_eq!(source.to_json()?, original);
    assert!(geometry.reveal_caret(&point("sibling", 0))?);
    assert!(geometry.caret(&point("sibling", 0))?.is_some());
    Ok(())
}
