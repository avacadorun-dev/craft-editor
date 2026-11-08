# Canonical content insertion

Status: approved implementation contract; native qualification is still required.

## Public operation

`ContentSlice::Inline(Vec<Node>)` inserts canonical inline nodes. `ContentSlice::Blocks(Vec<Node>)` inserts canonical blocks. `EditorSession::insert_content(slice) -> Result<Change>` uses the existing document selection and semantic history. It never appends to the application root merely because resolving the current caret is inconvenient.

The complete operation is prepared against the exact current document revision. Schema validation, identities, content constraints and post-edit selection are checked before canonical state changes. A failure preserves the original document, active selection, acknowledged revision, history and pending changes.

Inline input retains supplied marks, formula source, atom attributes and opaque extension fields. It does not parse Markdown. Block input splits the actual selected text block at the insertion boundary, retains its rich trailing fragment and places the incoming blocks under the appropriate canonical parent. In a table cell, list item, blockquote or callout, the operation respects that parent's grammar. Empty paragraph replacement and heading boundaries must follow the original editor fixtures.

Incoming block identities are preserved and must be unique. Conflicts are rejected before mutation. Only genuinely new split fragments and newly inserted identified nodes with missing/null identities receive new identities; invalid explicit IDs are rejected. The canonical/reserved ID pool is scanned once per operation and counter overflow is explicit. Required schema promotion uses the model's bounded `Node::required_schema_version` calculation and retains the root's other attributes, explicit null/omitted state and extension envelope.

The post-edit caret or opaque-node selection belongs to the same rich transaction. Selection follows the original insertion fixtures: an open right-tail fragment receives its start caret; end/empty-paragraph opaque insertion receives the inserted `NodeRef`; a final text block receives its exact Unicode-scalar end caret. Undo restores the complete pre-insertion tree and selection. Redo restores the inserted tree and its recorded post-selection. No selection is guessed from repeated text or a quote.

## Range replacement and metadata

Insertion replaces the exact current text or opaque-node selection. Range preparation must account for fully covered atoms, partially covered rich text, table-grid boundaries and nested containers. Table cells are not joined by a text deletion. Structural wrappers are retained or removed according to the actual selected range and original editor behavior. No generic algorithm may silently discard conflicting unknown metadata in order to force a join.

Source references, null presence, unknown fields and unselected rich runs remain lossless. Typed commands and their immutable engine provenance form the persistence seam; callers must not trust editable public position maps or reconstruct a transaction from a plain-text diff.

## Required fixtures

- Insertion at the beginning, middle and end of a nonempty paragraph, and inside an empty paragraph.
- A split tail containing Cyrillic, emoji, combining sequences, marks, source references and opaque fields.
- Heading, code, quote and callout boundaries, including conversion rules taken from original behavior.
- Inline formula and hard-break insertion without flattening them into literal text.
- Block insertion inside a simple/merged table cell and a nested list item.
- Replacement of a selected image, formula, diagram and anonymous separator.
- Replacement across paragraphs and a fully covered atom, across nested containers, and across cell boundaries.
- Conflicting IDs, invalid node kinds, stale exact references, invalid post-selection and illegal schema placement reject atomically.
- Undo and Redo restore exact JSON, semantic revision ordering, text/node selection and command provenance.
- Actual native typing, clipboard and assistant/MCP insertion share this operation after integration.

A fixture or all-node gallery alone does not satisfy full editor acceptance. Cross-atom range deletion, selected-atom composition, list Enter/Tab behavior, rich clipboard, accessibility and native platform input remain required gates until implemented and observed.
