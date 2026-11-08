# ProseMirror table clipboard semantics

The generic native table clipboard implementation follows the enabled original editor's ProseMirror tables 1.8.5 behavior, with preserved canonical envelopes, explicit IDs, revision guards and no browser runtime. This directory preserves the upstream MIT copyright/license notice for the source-backed normalization, tiling, growth and span-isolation algorithms.

Primary source: [ProseMirror/prosemirror-tables v1.8.5, copypaste.ts](https://github.com/ProseMirror/prosemirror-tables/blob/v1.8.5/src/copypaste.ts). The actual installed 1.8.5 plugin supplies read-only synthetic expected results in the curated fixtures. Original application code and owner data are not included.
