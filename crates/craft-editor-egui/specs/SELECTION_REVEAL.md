# Explicit measured selection reveal

This bounded editor correction starts from the qualified consumer revision `fd96a30cd650b47ef2f889120aaf0da537b201ae`. It addresses a range whose first line remains clipped when a host asks only to reveal its moving endpoint. Implementation and synthetic widget tests are prepared; hosted qualification and consumer native acceptance are pending. No unrelated newer engine revision is adopted.

With `wordcraft-canvas`, `EditorSession::request_scroll_to_selection()` requests one presentation operation on the next canvas frame. The engine measures with the actual current host width and style, then uses the union of the exact selected ranges. When that union fits the canvas/clip intersection, the whole range is revealed; the usual 12-point breathing room is included only if it also fits. For an oversized text range, the document-order start caret is revealed with leading alignment, independent of forward/reverse selection direction. An oversized selected node reveals its leading edge. Collapsed selection, table-cell selection and active IME use measured caret geometry.

The request changes no content, selection direction, history or focus. It supersedes an earlier pending caret request; subsequent ordinary typing/navigation or `request_scroll_to_caret()` supersedes it. Each request is consumed even if measurement fails or no range can be resolved. It does not pin the viewport after the author scrolls manually. Hosts request again after an intentional width/layout change that requires revealing the selected passage.

`canvas_selection_rect(&Response)` returns the current measured union, node bounds or caret in screen coordinates without layout or canonical mutation. Use the response from the last completed canvas frame. Missing/stale document-revision geometry returns `None`; invalid measured positions return the existing diagnostic. This is a viewport observation, never authority to change a text range.

An exact host navigation bookmark wins over a new reveal. Before installing that offset, call `cancel_scroll_request()` to consume pending engine caret/range requests. This method cannot cancel animation already submitted to a host-owned egui `ScrollArea`; the host must cancel/reset that scroll-area animation itself before restoring its saved offset. Do not issue another reveal during the offset-restoration frame.

Prepared regressions cover actual settled geometry below a host toolbar for forward/reverse two-line ranges, width reflow, oversized normalized-start behavior, ordinary caret-navigation precedence, node fallback, later wheel scrolling, explicit offset cancellation, stale queries and failed-measurement request consumption. They also retain exact content, directed selection, focus and a real Undo entry. These hosted engineering checks do not replace native consumer evidence for Notes, Find, Review Apply and inspector resizing.

Hosted verification commands (not run locally):

```sh
cargo test -p craft-editor-egui --all-features --locked selection_reveal_tests
cargo test --workspace --all-features --locked --no-fail-fast
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo fmt --all -- --check
```
