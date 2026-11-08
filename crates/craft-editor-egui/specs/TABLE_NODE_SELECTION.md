# Whole-table node selection

The original Studio TableKit explicitly enables `allowTableNodeSelection`. Installed ProseMirror tableEditing preserves a table NodeSelection when this option is true; selected rows/cells are normalized to a separate CellSelection. The native session must support that whole-node form without weakening revision/session guards or the sole approved63-column capacity exception.

## Selection and input

An explicit contextual Select table entry selects the innermost current table through its exact canonical NodeRef. A whole-table outline is derived separately from opaque-atom pointer hit geometry, so ordinary text clicks, rectangular cell gestures and resize handles remain unchanged. Wide tables use their bounded component frame; nested tables retain ancestor clips. Menu/dialog focus must not retarget the captured semantic selection.

Escape may restore the exact prior text/cell occurrence through a sealed same-session/revision bookmark. A stale bookmark fails without mutation or guessed retargeting. Horizontal/block arrow movement uses the source's before/after block search. For a sole table there is no outside target: keep the table visibly selected rather than creating a paragraph to exit.

## Authoring and clipboard

Copy emits a closed structured table slice, retaining all attributes, identities/provenance, spans, widths and nested content. Cut publishes acknowledged formats before removal; failed/stale publication leaves the document and selection unchanged. Delete/typing/Paste use one guarded canonical replacement and retain a valid parent. Undo/Redo restore the complete source tree and corresponding whole-node selection. A structurally necessary empty paragraph after removing the sole child is allowed; view navigation never creates one.

Marks and text-block formatting cover eligible descendants of the selected table and retain its whole-node selection. Structural formatting uses the same source-backed container commands and must retain the controlled selected occurrence; ambiguous remapping refuses before mutation. Contextual row/column operations remain scoped to this selected table, never a nearby table inferred from transient pixels.

## Source oracle and acceptance

`tests/fixtures/source-table-node.json` is generated from the actual installed original schema, tableEditing with allow=true, native selection content, enabled Tiptap clipboardTextSerializer helpers over selection ranges, deleteSelection and the exact Selection.findFrom non-text-block boundary used by prosemirror-view. Only synthetic writing and relative source hashes are included. It does not establish browser UI, native OS clipboard/IME/accessibility or full editor acceptance.

Qualify ordinary, nested, sole and wide tables; native entry/outline/Escape/arrow paths; Copy/Cut/Paste/Delete/typing/formatting and reversible history; stale/foreign references and failed clipboard publication; preserved siblings/unknown metadata; narrow/common widths, Light/Dark and localized labels. Compiled consumer captures and independent UX/interaction review remain required after hosted generic checks.

## Actual command-manager formatting oracle

`source-table-node-formatting.json` adds20 synthetic ordinary/nested/quoted/list scenarios from an unmounted original Tiptap Editor command manager. This runs the real source commands/schema but not mounted plugin normalization/history. Whole-table Bold/Heading/Quote and existing-list lift/type-switch retain the table node. Attempting a new list returns false yet source `clearNodes` clears retained paragraph IDs; native deliberately returns a data-preserving no-op instead of copying that identity-loss defect. Native Undo/Redo is independently asserted against exact pre/post trees, not inferred from the unmounted source's unavailable history.

The first hosted whole-table run (`e3316ff`, run42509939529) exposed oracle mismatches, not a reason to change canonical identity or the enabled clipboard serializer. The corrected plain-copy oracle invokes the installed Tiptap helpers rather than ProseMirror's different default `textBetween`. Empty JSON child arrays are compared as omitted empty content. Original setHeading drops paragraph identities, or assigns the same quote identity to both headings inside a quoted table; native keeps each existing paragraph identity. Tests isolate that source defect in structural comparison and independently require the native `a`/`b` identities, with exact native Undo/Redo trees.

The second hosted run (`0d6598f`, run42510711374) passed native whole-node geometry/input and four of five new source tests; the remaining ordered-list comparison differed only by omitted schema defaults (`start:1`, `type:null`). The oracle normalizes those exact defaults, preserving any authored non-default start/type.

## Whole-node Tab safety

An actual `goToNextCell` source probe returns false without canonical mutation for standalone, sole and leading table NodeSelections. One nested case moves into the outer sibling cell because source `isInTable`/`cellNear` infers an ancestor cell; native whole-node selection does not imply a cell in either table. Native `advance_table_cell` therefore refuses the whole-table form without mutation. This must not append a row from an invented last-cell target. Explicit text/cell selections retain their normal Tab/Shift-Tab traversal.

## Native clipboard transport qualification

The hosted macOS pasteboard and isolated X11 probes must also publish a fresh synthetic selected table, read back exact private/HTML/plain formats, acknowledge Cut only after ownership, and Undo to the exact canonical tree and table NodeRef. The fixture includes merged header widths, nested content, Unicode and retained unknown/provenance attributes. This establishes actual OS format ownership and session history; it does not establish native window gestures, IME or accessibility acceptance. Ambient clipboard contents are never read or printed before a fresh synthetic fixture is installed.

## Reversible exit after controlled whole-table formatting

Independent source UX review identified that a strictly old-revision internal return bookmark traps Escape after an ordinary owned Bold/Heading/Quote/resize, especially for a sole table. External host bookmarks remain strictly session/revision sealed. Only the session's private whole-table return may be reissued through its proven owned post-selection transaction: original text identities and exact inline text must remain; a cell return requires the same table topology plus unchanged originating endpoint identities/text; an opaque-node return must resolve one exact retained node. Ambiguous or changed occurrences invalidate the return without guessed replacement.

Selection history stores the private pre/post return snapshots. Undo/Redo may reseal a validated snapshot only after the engine restores its corresponding exact canonical/selected-node history state. Test own formatting, resize, wrapper moves, reverse cells and sole-table Escape, alongside unrelated/stale/foreign and changed/deleted return refusals. A failed edit or clipboard publication never advances or reissues a bookmark.
