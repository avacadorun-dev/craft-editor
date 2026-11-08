# Selection-aware structure commands

Tracking: [WriterCraft authoring issue #7](https://github.com/avacadorun-dev/writer-craft/issues/7), part of the [complete editor gate #4](https://github.com/avacadorun-dev/writer-craft/issues/4).

## Native session API

- `toggle_list(ListKind::{Bullet, Ordered})` lifts the selected item portion when toggling the active kind, changes the whole current list when switching kind, or wraps the selected sibling blocks. Headings/code are normalized to paragraphs without inserting an empty leading item; stable identities, text, provenance and opaque attributes are retained. `list_state(kind)` returns `MarkState::{Off, On, Mixed}`.
- `indent_list()` and `outdent_list()` move the selected items by one level through the model's guarded list operations.
- `toggle_blockquote()` wraps sibling blocks or lifts the selected enclosing quotation portion. `blockquote_state()` reports the selected text's container state.
- `set_callout_kind(kind)` changes an enclosing callout to the source-defined `info`, `idea`, `warning` or `todo` kind. Creating a callout uses `ContentSlice::Blocks`, preserving the original toolbar's insertion semantics.
- `insert_paragraph()` splits list items, exits genuinely empty items, retains literal code newlines and creates a paragraph after a selected opaque block. `insert_hard_break()` inserts a real inline node outside code.

These operations reject active composition and invalid selections before mutation. They keep the exact directed text selection, existing canonical block identities, rich runs and source references. Structural selection is validated against the prepared candidate and recorded in the same Undo/Redo entry. New identified wrapper portions receive collision-safe IDs. A text range ending at another block's start does not format that final block.

Quote lifting intentionally retires the removed quotation's identity. Known wrapper source references are merged into retained provenance-bearing descendants; Undo restores the complete prior wrapper and fields. Additional retained quotation portions keep all opaque metadata, with fresh identities where necessary. When a full lift would discard opaque wrapper metadata or nonempty provenance has no supported destination, the command refuses atomically rather than silently losing data.

The canvas routes Enter, Shift+Enter and list Tab/Shift+Tab through these same APIs. Table-cell Tab traversal retains priority. Hosts must still exercise the actual platform interaction and provide localized diagnostics; synthetic session tests do not qualify desktop UX.

## Remaining full-editor qualification

This tranche does not close issue #7 or the complete editor gate. Cross-wrapper range joins, open rich-clipboard slices, table Tab behavior at the final cell, source-matched code-block exit/input rules, generalized formatting of partial multi-container/table selections, precise annotation remapping for list split operations, list wrappers with opaque metadata that cannot be lifted, and actual native IME/accessibility remain required until separately implemented and verified. The headless command outcomes and conservative preservation guards are not permission to reduce the approved original editor scope.

## Original command probe

On 2026-11-08, the installed original Tiptap command implementation was executed headlessly with synthetic JSON and equivalent identity attributes (the original BlockIdentity appendTransaction was not invoked). It confirmed that heading/code list toggles normalize to paragraphs, switching kind changes the whole active list, same-kind toggling lifts only selected items, and a paragraph inside a quote becomes a list within that quote. Generic model operations remain more explicit and need not discard metadata as upstream clearNodes/setNodeMarkup can. This probe is behavioral research, not full original-app or native UX acceptance.

## Existing-link editing

`set_mark_attributes(Mark)` applies the model's `MarkMode::Patch`: merge supplied attributes and opaque fields of that mark type, retain every other mark, and validate the complete result atomically. This changes an existing href without losing a title, unknown link attributes, bold/italic marks or source metadata. At a collapsed caret, it changes validated pending typing marks without inventing a content revision. Invalid patches preserve the document, typing marks, history and pending changes.

`extend_mark_range("link")` expands a contained selection to contiguous runs carrying the same link attributes. It follows the installed source helper's boundary rule: prefer the right marked run, otherwise the left; a range crossing different links is not expanded. Expansion is transient selection, and the host can combine it with its explicit link edit dialog. It does not automatically open a URL or mutate manuscript content.
