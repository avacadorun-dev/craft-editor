# Pinned native flow geometry qualification

This is an optional hosted experiment, not an accepted editor or renderer. The default editor checkpoint remains independent of WordCraft. Full WriterCraft alpha.2 editor parity is still required.

## Exact dependency boundary

The `wordcraft-geometry` feature pins `wordcraft-doc`, `wordcraft-layout`, and `wordcraft-geom` to WordCraft **`da914ce674f18ebc4e08cca425cc8f8acca3372f`**, upstream package version **0.15.1**. No WordCraft application/UI/Office-format crates are imported and no source is copied. The upstream root declares **MIT OR Apache-2.0**; the Git dependency keeps upstream license files and font notices. Repackaging or extracting fonts must retain their separate notices.

Primary pinned sources:

- [Workspace dependencies and license](https://github.com/storytold/wordcraft/blob/da914ce674f18ebc4e08cca425cc8f8acca3372f/Cargo.toml)
- [Layout dependency manifest](https://github.com/storytold/wordcraft/blob/da914ce674f18ebc4e08cca425cc8f8acca3372f/crates/layout/Cargo.toml)
- [Document dependency manifest](https://github.com/storytold/wordcraft/blob/da914ce674f18ebc4e08cca425cc8f8acca3372f/crates/doc/Cargo.toml)
- [Font dependency manifest](https://github.com/storytold/wordcraft/blob/da914ce674f18ebc4e08cca425cc8f8acca3372f/crates/fonts/Cargo.toml)
- [Optional font-checkout build script](https://github.com/storytold/wordcraft/blob/da914ce674f18ebc4e08cca425cc8f8acca3372f/crates/fonts/build.rs)
- [Geometry manifest](https://github.com/storytold/wordcraft/blob/da914ce674f18ebc4e08cca425cc8f8acca3372f/crates/geom/Cargo.toml)

The layout graph also includes local `wordcraft-fonts` and `wordcraft-metafile`, and the external `craft-proof` package from `storytold/craft-libs` pinned by upstream to **`01315d9c92b5b0e69082d35ca56099461a1efd45`**, package version `=0.1.0`. Fonts use `harfrust 0.13` and `skrifa 0.47`; the existing egui graph uses different versions, so deduplication must not be assumed. Layout includes the image crate with PNG/JPEG/WebP/GIF/BMP/TIFF features. `wordcraft-render` and its `vello_cpu 0.2` dependency are included only by the separate default-off `wordcraft-canvas` feature, pinned to the same upstream revision. The geometry-only feature remains independent of rasterization. This adds a CPU viewport renderer, not WordCraft application/UI code.

The selected layout crates do not need WordCraft's patched egui 0.37/winit application graph. A recursive upstream tree inspection at this revision found no Git submodule entries. The font crate embeds base font assets through repository-relative `include_bytes!`; extracting only source directories would break compilation unless the assets and notices were included. Optional `CRAFT_FONTS_DIR` and `CRAFT_I18N_DIR` inputs must not be marked required for this probe. Proofing is disabled.

The five local layout/model/font/geometry/metafile crates plus the entire upstream assets directory total about **8.3 MB of files** at this revision, including tests. This is not a Cargo download, clone, cache or build-size measurement. Git dependencies fetch a repository checkout, not just these five crates. Native build size has not been measured; hosted qualification is mandatory because the local workspace has little free disk space.

## Why this boundary is plausible

[LayoutOptions and ViewMode](https://github.com/storytold/wordcraft/blob/da914ce674f18ebc4e08cca425cc8f8acca3372f/crates/layout/src/lib.rs) provide `ViewMode::Web`, documented as one window-width page with no page breaks, and `layout(&Document, &mut LayoutCache, &LayoutOptions)` produces a `DocLayout`. This can support a continuous manuscript display without adopting a paginated paper UI.

[Placed geometry](https://github.com/storytold/wordcraft/blob/da914ce674f18ebc4e08cca425cc8f8acca3372f/crates/layout/src/placed.rs) includes actual cell rectangles, horizontal spans, covered vertical-merge continuations and inline/floating object geometry. [Hit APIs](https://github.com/storytold/wordcraft/blob/da914ce674f18ebc4e08cca425cc8f8acca3372f/crates/layout/src/hit.rs) provide hit testing, caret rectangles, vertical navigation, visual stepping, table-cell hits and cross-block selection rectangles. [The display-list API](https://github.com/storytold/wordcraft/blob/da914ce674f18ebc4e08cca425cc8f8acca3372f/crates/layout/src/display/page.rs) produces glyph/image/shape primitives independently of the full application.

WordCraft positions use **UTF-8 byte offsets**, plus body/story and a hierarchical path. Craft Editor selection uses a stable canonical block identity and **Unicode-scalar offsets**. `layout.rs` explicitly converts and validates these boundaries in both directions. Synthetic vertical-merge continuation cells map back to their original canonical cell path. The disposable WordCraft document is never serialized back into the canonical ProseMirror-compatible tree; unknown metadata, source references, atom source and IDs remain authoritative in the original model.

## Explicit native editing capacity

Canonical `colspan` values up to **100** remain valid and losslessly preserved. Upstream [Cell::span](https://github.com/storytold/wordcraft/blob/da914ce674f18ebc4e08cca425cc8f8acca3372f/crates/doc/src/table.rs#L31) clamps effective spans to **63**, and the same file defines `MAX_COLS = 63`. The owner explicitly accepted a **63 logical-column native editing capacity**. A fork solely to raise that limit is not currently required.

The adapter returns `BackendColumnLimit` before rendering or mutation instead of clamping canonical content. The qualification report records `exceeds_backend_capacity` and `requires_read_only` for the valid span-100 fixture. Hosts must preserve wider canonical documents, explain the capacity limit and prevent native/MCP table edits beyond the capacity before mutation. This canvas checkpoint currently returns the diagnostic; rendering a faithful wider read-only view is still pending. This exception does not waive any other alpha.2 editing requirement.

Other unsupported probe inputs fail explicitly: non-paragraph blocks, links, nested tables, unmeasured inline atoms and literal U+FFFC text. They remain requirements for the full editor, not accepted product limitations. Host-provided inline atom sizes reserve rectangles only; this does not render formulas or validate their TeX semantics.

## Hosted commands and evidence

Do not run the heavy feature locally in this disk-constrained workspace. Resolve the lockfile separately, then execute on a hosted runner:

```sh
cargo test -p craft-editor-egui --features wordcraft-geometry --locked
cargo clippy -p craft-editor-egui --all-targets --features wordcraft-geometry --locked -- -D warnings
cargo run -p craft-editor-egui --example wordcraft_geometry --features wordcraft-geometry --locked
```

The example must emit evidence from actual layout: one continuous page, actual grid cell boxes and a covered vertical merge, correct span widths/heights, Cyrillic/emoji caret-hit roundtrips, cross-table selection rectangles, an inline atom retaining its measured dimensions and unchanged canonical JSON. The span-100 incompatibility must remain visible. A failing invariant is a failed qualification, not a reason to delete the assertion.

Hosted geometry qualification [run 42411597588](https://github.com/avacadorun-dev/craft-editor/actions/runs/42411597588) passed against the pinned source: one continuous page, six real cell boxes, one covered vertical-merge box, seven cross-table selection rectangles, four Unicode caret/hit roundtrips and one correctly sized inline atom. This proves that geometry fixture, not full native editor acceptance. Subsequent canvas source changes require their own hosted compilation and native review.

## Subsequent full-view work

If geometry qualifies, retain `EditorSession` and its atomic canonical command/history boundary. Replace the styled flat TextEdit projection with one document canvas using the native display tree and its hit/caret/selection geometry. Port only focused interaction behavior from upstream, with pinned provenance and retained notices; do not copy WordApp, ribbon, backstage, updater or Office import/export. Native composition/caret placement, rich clipboard, accessibility, all source blocks/marks, table resize/merge/selection, image/TeX/Mermaid renderers, annotation mapping and native behavior acceptance remain distinct implementation gates. A display-list or geometry probe alone cannot satisfy them.

## Native canvas checkpoint

With `wordcraft-canvas`, `EditorSession::show_canvas(ui, &CanvasStyle)` paints real wrapped grid cells using bounded 256-point raster tiles, then overlays selection/caret geometry from the same measured document. The host supplies foreground, background, selection, caret, grid/header colors and typography. There is no inner white page frame. Tile images use the renderer's documented premultiplied RGBA8 format and the viewport pixel ratio; selection remains in logical document coordinates.

A single input controller translates pointer and keyboard events into the existing canonical transactions/history. Formatting retains a directed cross-cell selection when the toolbar receives focus. Native preedit lives in a transient shadow tree; commands refuse to interrupt it and the final commit creates one canonical transaction. These synthetic paths still require actual macOS IME, selection, clipboard and Retina review. Cross-cell text-range deletion preserves the table grid; structural cell selection/merge/resize is a separate interaction contract.

Hosted verification:

```sh
cargo test -p craft-editor-egui --features wordcraft-canvas --locked
cargo clippy -p craft-editor-egui --all-targets --features wordcraft-canvas --locked -- -D warnings
cargo run -p craft-editor-egui --example wordcraft_canvas --features wordcraft-canvas --locked
```

The source fixture is `examples/canvas_fixture.json`. Focused canvas tests cover real merged/wrapped geometry, dark host colors and raster bounds, pointer-to-Unicode insertion, reverse cross-cell formatting/Undo, actual text events, provisional IME, grid-preserving deletion and read-only input. They must be executed on the hosted runner before being reported as passed. Full original block/mark coverage, object renderers, rich clipboard, accessibility, annotation decoration, table controls and native acceptance remain pending.

The first full canvas run at `4864790` compiled and passed 24 of 25 view tests; the remaining code-line test exposed a projection bug. WordCraft documents `para::LINE_BREAK` as newline, not vertical tab. The adapter now preserves code newlines and maps canonical hard breaks through that constant. The assertion is retained and a separate Cyrillic/emoji hard-break caret/hit test was added. A subsequent passing run is required before calling this fixed in a qualified build.

Hosted run [42451918260](https://github.com/avacadorun-dev/craft-editor/actions/runs/42451918260) at `c01ddeb` passed all 27 view tests and all 87 model tests, including the retained multiline-code assertion and new hard-break roundtrip. Strict Clippy then requested the Rust array-chunk API in two raster-test assertions. The follow-up uses `as_chunks::<4>()` and adds coverage for valid legacy integral JSON spans such as `2.0`; final strict qualification still requires its own passing run.
