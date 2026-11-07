# CraftEditor

Reusable native Rust rich-document editing components for WriterCraft and other desktop applications.

**Development status:** architecture and implementation work in progress. No full native-editor parity or production readiness is claimed. WriterCraft alpha.2 requires the complete original editor workflow; internal model/view checkpoints do not satisfy that release gate.

## Verified development checkpoint

The workspace passes **45 semantic/headless tests** locally: 35 model/structure checks and 10 native-input-session checks, plus formatting and strict Clippy. The model preserves complete JSON, unknown fields/explicit nulls, typed positions, exact edits, marks, split/join, nested guarded Insert/Replace/Delete/Move and Undo/Redo. Local typing coalesces compatible text runs without losing metadata.

The initial egui session has one input/selection surface and emits semantic model commands. Headless input tests cover text, shifted selection and IME preedit/commit into a table-cell paragraph. **Tables still use a linear projection; this is not a completed WYSIWYG grid.** Complex unsupported blocks fail explicitly. True table/atom layout, all block renderers, rich clipboard, platform IME/accessibility and full host integration remain mandatory work; neither these tests nor a projection can pass full editor parity.

## Boundaries

- `craft-editor-model`: revisioned structured documents, typed positions, validated transactions and rich history.
- `craft-editor-egui`: one continuous native editing session, layout/selection/input and block rendering for egui.
- The host application owns projects, persistence, media authorization, annotations/dictionary meaning, assistant review and localization.

Canonical ProseMirror-shaped JSON, stable block identity and metadata must survive editing. Existing Rust model libraries may be reused through a lossless adapter; their presence does not supply a finished native view. Large upstream applications are not vendored wholesale.

The initial consumer is [WriterCraft](https://github.com/avacadorun-dev/writer-craft). Its source-backed editor parity matrix and independent UX acceptance define consumer requirements. Full native selection, clipboard, IME, accessibility, nested structures and supported block interactions require actual evidence.

## Development

```sh
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

MIT; upstream dependencies and any adapted source retain their own notices. See [LICENSE](LICENSE) and [AGENTS.md](AGENTS.md).
