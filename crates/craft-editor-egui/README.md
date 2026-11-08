# Craft Editor egui checkpoint

This component is an internal native editing checkpoint, not full WriterCraft alpha.2 editor acceptance.

## Public seam

`EditorSession::new(Document, Revision)` owns one canonical rich model, one directed block/scalar selection and one semantic Undo/Redo history. `show(&mut Ui)` returns `ViewResponse { response, changes, errors }`; the host persists acknowledged rich `Change` snapshots and maps diagnostics to its language. The host owns fonts, colors, application permissions and storage.

One continuous egui input surface maps insert/delete/split/join/format actions into model commands. It never rebuilds a document from plain text. Plain clipboard and native input events use egui; built-in string-only undo is disabled, and semantic Undo/Redo restores rich snapshots and selections. IME preedit is kept in a transient shadow document; only its final commit changes the canonical model. Commands cannot interrupt an active composition.

The current projection includes paragraph/heading/code text and a linear traversal through table cells. The canonical table grid and metadata are retained. This is **not actual table-grid layout** and does not satisfy full rich-editor parity. Structural cross-cell deletion is rejected atomically. Blocks with no qualified renderer (images, block math, Mermaid, separators) fail explicitly rather than silently vanishing. Inline math currently projects to the model's single U+FFFC atom; native formula rendering is pending.

## Executed evidence

Ten focused tests cover scalar mappings with Cyrillic/emoji, cross-paragraph/table-cell selection, actual egui text input into a canonical table cell, shifted keyboard selection across a cell boundary, display-only IME preedit followed by one atomic commit, rich formatting and Undo/Redo, actual SplitBlock newline behavior, rejected structural deletion and unsupported-renderer diagnostics. These synthetic headless tests do not qualify real macOS/Windows/Linux input-method behavior, clipboard interoperability, visual quality or full editor parity.

No WordCraft application code was copied. This checkpoint uses the selected egui library and the shared model's canonical command boundary.

## Full native view path

The next layout boundary must replace the linear projection with a document display tree: paragraph layouts, nested container geometry, real table tracks/spans/cell rectangles, image/object boxes and block renderers. A single semantic selection/caret remains independent of that display tree. Pointer hit tests map each display item back to an exact block/scalar position; selection geometry can cross block and table boundaries without separate TextEdit widgets.

WordCraft's focused layout/hit-test, canvas selection, table and object interaction, IME and accessibility modules are the strongest reuse references. Extract those as a narrow engine component with pinned provenance and license notices, **not** WordApp/ribbon/backstage/application services. Its WordDoc cannot silently replace the canonical ProseMirror tree: a transient layout adapter and explicit path/byte/scalar maps are required, or every editing command must be adapted to the canonical model.

Keep the session/history/command API while introducing that display tree and one input controller. Add a host boundary for structured/plain/HTML clipboard, assets, composition geometry, accessible document nodes and atom renderer services. Rich clipboard paste, lists/quotes/callouts, true table selection/resize/merge, images, formulas, Mermaid, provenance and annotation geometry must each pass original behavior fixtures plus observed native interaction. A styled flat buffer or a set of per-block fields is not the completion boundary.

## Optional geometry qualification

The default-off `wordcraft-geometry` feature explores WordCraft's pinned standalone document/layout/font boundary, with real continuous-flow table geometry and exact canonical scalar/byte mapping. Its hosted probe is separate from `EditorSession::show` and does not change the current editing surface. See [WORDCRAFT_GEOMETRY.md](WORDCRAFT_GEOMETRY.md) for exact provenance, API/dependency facts, commands and blockers. The owner accepted native editing up to 63 logical columns. Wider valid canonical tables remain preserved and must be handled read-only without silent clamping; this does not waive the remaining editor requirements.

## Optional real-grid canvas

`wordcraft-canvas` adds `EditorSession::show_canvas(&mut Ui, &canvas::CanvasStyle)`. This is a real continuous document raster surface with wrapped/merged table tracks, mapped pointer hits, one semantic caret/selection, B/I/U, plain-text input, provisional IME and the same canonical Undo/Redo. The host provides all panel/text/grid/header/selection colors, font family and spacing, and encloses the widget in its normal scroll area. The existing `show` checkpoint remains available independently.

The geometry fixture passed hosted run [42411597588](https://github.com/avacadorun-dev/craft-editor/actions/runs/42411597588). The new raster/input canvas tests and `wordcraft_canvas` example require a separate hosted run; source implementation is not native acceptance. Unsupported blocks return explicit diagnostics with canonical content unchanged. Full source-editor blocks/marks, rich clipboard, object renderers, annotation decoration, structured accessibility and observed native UX remain acceptance gates.


Hosted canvas qualification [42452125336](https://github.com/avacadorun-dev/craft-editor/actions/runs/42452125336) at `5f11b46` passed all 28 view and 87 model tests, strict Clippy and actual geometry/raster/input examples. This verifies the synthetic checkpoint, not native UX or full schema acceptance. The host can query `MarkState::{Off, On, Mixed}`, toggle stored insertion marks, and use the same selection/history for text and structure. Structural integrations should use `apply_commands_with_selection` to validate and record the exact new/surviving-cell caret atomically; `request_scroll_to_caret` reveals it on the next canvas frame.
