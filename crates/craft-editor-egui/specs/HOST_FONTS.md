# Host-owned native font registration

The host's egui font catalog and atom renderer fonts are separate from the WordCraft body layout/raster database. Loading a CJK font into only those first two systems leaves native body text with missing ideograph glyphs. Register the host's acknowledged bytes in `craft_editor_egui::fonts` before first body layout.

## API

- `register_font(Vec<u8>) -> Result<Registration>` validates every OpenType/TTC face/name/cmap before global registration. It supports the pinned backend's font-container policy and refuses its 256-face capacity before mutation instead of registering a prefix. Returned data includes actual `faces_added`, intrinsic `families` and native `generation`.
- `coverage(family, style, text) -> Result<GlyphCoverage>` checks an explicitly loaded face rather than accepting a different fallback family. It returns requested/resolved styles, `exact_style` and distinct uncovered scalars. Control characters do not need visible glyphs. Cmap coverage is not by itself a shaping/IME/layout proof.
- `generation()` exposes the shared native database generation for geometry/raster cache invalidation. The canvas measurement key includes it; a later registration cannot reuse an earlier missing-glyph layout or raster tile.

Registration uses the pinned backend's `add_document_fonts` API, which atomically acknowledges faces, clears fallback caches and increments generation. Its low-level `add_font` does not bump generation and is unsuitable for an already-used view cache. An empty document alias preserves intrinsic family names. Registration neither changes canonical document data nor creates a font folder, persists bytes, installs a system font or replaces an existing family/style. The host owns font bytes and redistribution licenses; no large CJK font is copied into CraftEditor.

The new direct optional `wordcraft-fonts` dependency uses the same exact WordCraft revision as document/layout/render; `skrifa = 0.47.0` is the existing native font parser version from that graph. The module is enabled by `wordcraft-geometry`.

## Consumer acceptance

WriterCraft already bundles an OFL Noto Sans CJK JP font. Register it once at application startup, check actual Chinese/Japanese cmap coverage, then verify native body glyphs, selection/caret geometry and restart behavior with real compiled evidence. Retain the source font's full OFL notice.

A Regular CJK face does not automatically qualify Bold, Italic, region-specific forms, all Unicode/emoji, baseline alignment or composition. The returned exact-style and missing-scalar diagnostics must remain explicit. Native syntax raster examples made before host font registration show this distinction: their source/positions are preserved but CJK body glyphs are absent. Do not call those examples CJK UX acceptance.
