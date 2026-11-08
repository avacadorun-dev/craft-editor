#![expect(
    clippy::unwrap_used,
    reason = "Synthetic projection fixtures fail immediately"
)]
use craft_editor_model::{BlockTextProjection, Document, ScalarOffset, TextPoint};
use serde_json::json;
fn quote() -> Document {
    Document::from_json(json!({"type":"doc","content":[{"type":"blockquote","attrs":{"blockId":"q"},"content":[{"type":"paragraph","attrs":{"blockId":"a"},"content":[{"type":"text","text":"先😀 α"}]},{"type":"paragraph","attrs":{"blockId":"b"},"content":[{"type":"text","text":"次の文"}]}]}]})).unwrap()
}
#[test]
fn parent_newline_projection_round_trips_every_scalar_to_leaf_carets() {
    let document = quote();
    let before = document.to_json().unwrap();
    let projection = BlockTextProjection::new(&document, "q").unwrap();
    assert_eq!(projection.text(), "先😀 α\n次の文");
    assert_eq!(document.block_text("q").unwrap(), "先😀 α次の文");
    for offset in 0..=8 {
        for right in [false, true] {
            let point = projection.point(ScalarOffset(offset), right).unwrap();
            assert_eq!(projection.offset(&point).unwrap(), ScalarOffset(offset));
        }
    }
    assert_eq!(
        projection.point(ScalarOffset(4), false).unwrap(),
        TextPoint {
            block_id: "a".into(),
            offset: ScalarOffset(4)
        }
    );
    assert_eq!(
        projection.point(ScalarOffset(5), true).unwrap(),
        TextPoint {
            block_id: "b".into(),
            offset: ScalarOffset(0)
        }
    );
    assert_eq!(document.to_json().unwrap(), before);
}
#[test]
fn native_inline_atom_and_hard_break_coordinates_are_not_legacy_spelling() {
    let document=Document::from_json(json!({"type":"doc","content":[{"type":"paragraph","attrs":{"blockId":"p"},"content":[{"type":"text","text":"中"},{"type":"inlineMath","attrs":{"latex":"x^2"}},{"type":"hardBreak"},{"type":"text","text":"😀"}]}]})).unwrap();
    let projection = BlockTextProjection::new(&document, "p").unwrap();
    assert_eq!(projection.text(), "中\u{fffc}\n😀");
    for offset in 0..=4 {
        assert_eq!(
            projection.point(ScalarOffset(offset), true).unwrap().offset,
            ScalarOffset(offset)
        );
    }
}
#[test]
fn invalid_or_foreign_leaf_boundaries_are_rejected() {
    let document = quote();
    let projection = BlockTextProjection::new(&document, "q").unwrap();
    assert!(projection.point(ScalarOffset(9), true).is_err());
    assert!(
        projection
            .offset(&TextPoint {
                block_id: "b".into(),
                offset: ScalarOffset(4)
            })
            .is_err()
    );
    assert!(
        projection
            .offset(&TextPoint {
                block_id: "q".into(),
                offset: ScalarOffset(0)
            })
            .is_err()
    );
}
#[test]
fn nontext_block_atoms_are_never_silently_dropped_from_a_parent_passage() {
    let document=Document::from_json(json!({"type":"doc","content":[{"type":"blockquote","attrs":{"blockId":"q"},"content":[{"type":"paragraph","attrs":{"blockId":"a"},"content":[{"type":"text","text":"Before"}]},{"type":"imageBlock","attrs":{"blockId":"image","src":"image.png"}}]}]})).unwrap();
    assert!(BlockTextProjection::new(&document, "q").is_err());
}
