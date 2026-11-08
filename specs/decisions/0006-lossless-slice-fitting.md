# Source-compatible open-slice fitting with canonical envelopes

Status: implementation in progress; no full editor or native acceptance claim. WriterCraft issue7/epic4 remain open for owner acceptance.

The source uses ProseMirror replaceSelection/replaceRange and a separate prosemirror-tables handlePaste path. Read-only synthetic probes against the actual original editorExtensions schema cover unequal list edges, nested list/quote/callout contexts, cross-wrapper selections, Unicode, closed tables/images, rectangular cell paste and colspan expansion. The curated model fixture records twenty-two generic cases; three table-plugin cases remain a separate required seam. These are source fixtures, not owner document data or mounted UI results.

Reuse pinned MIT prosemirror0.5.2 native Transform::replace_range as a structural planner. Keep defining/isolating/code/linebreakReplacement behavior from the actual source schema. Direct native serialization is prohibited: upstream text cuts rebuild text nodes without outer attributes and would lose unknown canonical metadata.

Use private transient envelope tokens during planning. Nontext nodes carry a private planning attribute; text and inline leaves carry a nonexcluding private planning mark so text cuts retain their metadata identity. Real marks receive private envelope tokens as well. All tokens are removed during canonical reconstruction; they never reach persisted JSON, interoperability formats or MCP. Reconstruct from original canonical envelopes plus fitted text/children/surviving mark sets, preserving null/omitted/opaque fields. Fresh IDs are explicit inputs to typed replayable commands; split copies cannot keep duplicate IDs.

The model computes before/after structural indexes and native step maps internally. Block-local Unicode scalars and ProseMirror UTF-16 units stay explicit; compact surrogate indexes avoid storing another whole manuscript in each map. Caret/path mapping and note replay use those computed maps. Caller-supplied annotation maps or a root ReplaceNode map that detaches every note are insufficient.

Validate canonical input depth/node limits and open edges before invoking the upstream planner, whose Slice::size assumes valid depths. Reject a silent no-step result, missing authored atoms/text, or an unsafe opaque-envelope/context conflict without document/history mutation. Contextual source references receive an explicit merge policy rather than disappearing when a wrapper is consumed. Complete candidate validation occurs before publication.

Required qualification includes curated actual-source results, reversed/unequal ranges, exact rich/opaque data, fresh identity, UTF-16 boundaries, persistent note maps before/inside/after edits, one-step Undo/Redo, immediate save before paint, stale/unsupported/resource no-mutation paths and hosted strict checks. Rectangular table selection/paste, mounted native interactions and OS ownership remain separate required gates. No feature is removed because the planner lacks it.

## Native schema scope boundary

The pinned [DynamicSchema::with_types implementation](https://github.com/fiduswriter/prosemirror-rs/blob/v0.5.2/src/dynamic/schema.rs#L448) retains an already active thread-local type store rather than switching to a nested schema. The private fitting schema therefore produces only plain JSON and numeric step maps inside that scope. Canonical reconstruction and validation run after the scope exits, under their own schema. Running canonical validation inside the planner's scope is incorrect and was caught by opaque-attribute/structure fixtures; validation is never weakened to accommodate the private transport fields.

Identical container and mark envelopes share planning tokens so private transport does not change ordinary same-markup decisions. Distinct text envelopes retain separate leaf tokens through native cuts, then exactly compatible canonical text is coalesced after reconstruction. Structural path mappings adjust to that coalescing and detach changed nodes conservatively.
