# CraftEditor native renderers

Reusable native Mermaid, formula and image rendering for canonical structured atoms. This crate is a development checkpoint, not a claim of complete Mermaid/KaTeX compatibility or native editor acceptance.

Prepare `Renderer::prepare_document` on a host-owned worker. The resulting immutable `PreparedCatalog` implements the editor's `AtomResolver`; frame measurement only looks up previously acknowledged real PNG output. The host supplies exact asset bytes with `AssetMap`/`AssetSource` and owns worker scheduling, stale-result cancellation and catalog generations. Source JSON, LaTeX, diagram code, media identity, descriptions and captions stay canonical and unchanged.

Untrusted formulas reject HTML/resource, TeX definition and file/I/O commands before parsing. This preserves the source editor's rendering policy while retaining the complete authored source for correction.

The catalog records each failed atom and provides an explicitly labelled source fallback, so one invalid diagram does not remove the surrounding document. The host supplies localized `FallbackLabels`. Fallback previews are bounded and visibly use an ellipsis when source continues; they never truncate canonical source. `render_atom` returns a typed failure instead of a fallback when the caller needs direct success/failure control.

## Dependency decisions

- [Merman 0.9.3](https://github.com/Latias94/merman/tree/v0.9.3): Rust parsing, native diagram layout and resvg-safe SVG output. Full default family/ELK/math closure is selected; no JavaScript, browser or command-line subprocess is used. Its crate is MIT OR Apache-2.0. The ELK implementation additionally requires EPL-2.0 notices and source provenance in distributed applications.
- [RaTeX 0.1.15](https://github.com/erweixin/RaTeX): MIT parser, layout, display lists and native PNG rendering, with embedded KaTeX fonts. Inline baselines come from the actual layout height/depth. Upstream's broad syntax claims are not treated as our own parity evidence.
- [resvg/usvg 0.47.0](https://github.com/linebender/resvg/tree/v0.47.0): MIT OR Apache-2.0 native SVG processing/rasterization. No ambient system font scan or external image/file lookup is enabled; supplied font faces and bounded embedded raster images are allowed.
- [image 0.25.10](https://github.com/image-rs/image): MIT OR Apache-2.0 raster decoding. PNG, JPEG, GIF and WebP are enabled; animated assets currently produce their first frame, while original bytes remain host-owned. Animation and other codec behavior require separate migration qualification.
- WordCraft fonts at maintained fork `82ace77a17d71f8fce770227624319203e74cc7b`, with unchanged font code/assets from original `da914ce674f18ebc4e08cca425cc8f8acca3372f`: its public `bundled()` accessor supplies 13 existing Source Sans 3, Source Serif 4, Inter and JetBrains Mono OFL faces. No font files are copied into this repository. Hosts may add authorized CJK or other faces explicitly.

Retain the relevant Cargo/package license material, font OFL notices, and Merman ELK provenance when packaging this crate. The consuming application generates its complete feature-aware third-party notices.

## Verification boundary

Focused tests exercise real flowchart/sequence/state output, fractions/roots/sums, formula baseline/width fitting, host-authorized media, explicit invalid-source fallback and resource limits. Run them on the hosted qualification workflow; local Cargo is intentionally not used in the current constrained workstation. The GUI, full source syntax corpus, IME, accessibility, animation, cancellation ownership and stale worker results remain host/native acceptance work.
