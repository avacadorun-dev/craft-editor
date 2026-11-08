# Selection-aware structure commands

Tracking: [WriterCraft authoring issue #7](https://github.com/avacadorun-dev/writer-craft/issues/7), part of the [complete editor gate #4](https://github.com/avacadorun-dev/writer-craft/issues/4).

## Native session API

- `toggle_list(ListKind::{Bullet, Ordered})` toggles the current list portion or wraps the selected sibling blocks. `list_state(kind)` returns `MarkState::{Off, On, Mixed}`.
- `indent_list()` and `outdent_list()` move the selected items by one level through the model's guarded list operations.
- `toggle_blockquote()` wraps sibling blocks or lifts the selected enclosing quotation portion. `blockquote_state()` reports the selected text's container state.
- `set_callout_kind(kind)` changes an enclosing callout to the source-defined `info`, `idea`, `warning` or `todo` kind. Creating a callout uses `ContentSlice::Blocks`, preserving the original toolbar's insertion semantics.
- `insert_paragraph()` splits list items, exits genuinely empty items, retains literal code newlines and creates a paragraph after a selected opaque block. `insert_hard_break()` inserts a real inline node outside code.

These operations reject active composition and invalid selections before mutation. They keep the exact directed text selection, existing canonical block identities, rich runs and source references. Structural selection is validated against the prepared candidate and recorded in the same Undo/Redo entry. New identified wrapper portions receive collision-safe IDs. A text range ending at another block's start does not format that final block.

Quote lifting intentionally retires the removed quotation's identity. Known wrapper source references are merged into retained provenance-bearing descendants; Undo restores the complete prior wrapper and fields. Additional retained quotation portions keep all opaque metadata, with fresh identities where necessary. When a full lift would discard opaque wrapper metadata or nonempty provenance has no supported destination, the command refuses atomically rather than silently losing data.

The canvas routes Enter, Shift+Enter and list Tab/Shift+Tab through these same APIs. Table-cell Tab traversal retains priority. Hosts must still exercise the actual platform interaction and provide localized diagnostics; synthetic session tests do not qualify desktop UX.

## Remaining full-editor qualification

This tranche does not close issue #7 or the complete editor gate. Cross-wrapper range joins, open rich-clipboard slices, table Tab behavior at the final cell, source-matched code-block exit/input rules, generalized formatting of partial multi-container/table selections, precise annotation remapping for list split operations, list wrappers with opaque metadata that cannot be lifted, and actual native IME/accessibility remain required until separately implemented and verified. The headless command outcomes and conservative preservation guards are not permission to reduce the approved original editor scope.
