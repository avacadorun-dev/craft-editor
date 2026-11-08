# Whole-table node selection

The original Studio TableKit explicitly enables `allowTableNodeSelection`. Installed ProseMirror tableEditing preserves a table NodeSelection when this option is true; selected rows/cells are normalized to a separate CellSelection. The native session must support that whole-node form without weakening revision/session guards or the sole approved63-column capacity exception.

## Selection and input

An explicit contextual Select table entry selects the innermost current table through its exact canonical NodeRef. A whole-table outline is derived separately from opaque-atom pointer hit geometry, so ordinary text clicks, rectangular cell gestures and resize handles remain unchanged. Wide tables use their bounded component frame; nested tables retain ancestor clips. Menu/dialog focus must not retarget the captured semantic selection.

Escape may restore the exact prior text/cell occurrence through a sealed same-session/revision bookmark. A stale bookmark fails without mutation or guessed retargeting. Horizontal/block arrow movement uses the source's before/after block search. For a sole table there is no outside target: keep the table visibly selected rather than creating a paragraph to exit.

## Authoring and clipboard

Copy emits a closed structured table slice, retaining all attributes, identities/provenance, spans, widths and nested content. Cut publishes acknowledged formats before removal; failed/stale publication leaves the document and selection unchanged. Delete/typing/Paste use one guarded canonical replacement and retain a valid parent. Undo/Redo restore the complete source tree and corresponding whole-node selection. A structurally necessary empty paragraph after removing the sole child is allowed; view navigation never creates one.

Marks and text-block formatting cover eligible descendants of the selected table and retain its whole-node selection. Structural formatting uses the same source-backed container commands and must retain the controlled selected occurrence; ambiguous remapping refuses before mutation. Contextual row/column operations remain scoped to this selected table, never a nearby table inferred from transient pixels.

## Source oracle and acceptance

`tests/fixtures/source-table-node.json` is generated from the actual installed original schema, tableEditing with allow=true, native selection content, deleteSelection and the exact Selection.findFrom non-text-block boundary used by prosemirror-view. Only synthetic writing and relative source hashes are included. It does not establish browser UI, native OS clipboard/IME/accessibility or full editor acceptance.

Qualify ordinary, nested, sole and wide tables; native entry/outline/Escape/arrow paths; Copy/Cut/Paste/Delete/typing/formatting and reversible history; stale/foreign references and failed clipboard publication; preserved siblings/unknown metadata; narrow/common widths, Light/Dark and localized labels. Compiled consumer captures and independent UX/interaction review remain required after hosted generic checks.

## Actual command-manager formatting oracle

`source-table-node-formatting.json` adds20 synthetic ordinary/nested/quoted/list scenarios from an unmounted original Tiptap Editor command manager. This runs the real source commands/schema but not mounted plugin normalization/history. Whole-table Bold/Heading/Quote and existing-list lift/type-switch retain the table node. Attempting a new list returns false yet source `clearNodes` clears retained paragraph IDs; native deliberately returns a data-preserving no-op instead of copying that identity-loss defect. Native Undo/Redo is independently asserted against exact pre/post trees, not inferred from the unmounted source's unavailable history.
