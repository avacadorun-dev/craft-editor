# Craft Editor

Reusable native Rust rich-document editing components for WriterCraft and other desktop applications.

**Development status:** architecture and implementation work in progress. No full native-editor parity or production readiness is claimed. WriterCraft alpha.2 requires the complete original editor workflow; internal model/view checkpoints do not satisfy that release gate.

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
