# Craft Editor engineering agreements

This independently versioned repository contains reusable native rich-document editing components consumed by WriterCraft. It is an implementation in progress, not a released full-parity widget.

- Rust application/library code only; no browser, WebView or JavaScript UI.
- Never use a `codex/` branch prefix. Use `feature/`, `fix/`, `chore/` or `release/`.
- Follow the [consumer Rust engineering skill](https://github.com/avacadorun-dev/writer-craft/blob/main/.agents/skills/rust-best-practices/SKILL.md) and its relevant reference chapters; contributor instructions are never a runtime dependency.
- Preserve canonical structured JSON, unknown metadata, stable block identity and source provenance. Never flatten rich data into fresh paragraphs.
- Use explicit UTF-8 byte, Unicode-scalar and UTF-16 positions. Validate exact ranges against a revision. Never retarget an ambiguous repeated quote.
- A document transaction is atomic and revision-guarded. Undo/redo preserve rich structure, metadata and corresponding selection/annotation changes.
- One document-level editing session, selection and history. Per-block fields that break cross-block selection are not native rich editing parity.
- Keep generic document/view mechanics here. WriterCraft projects, SQLite persistence, assistant permissions, dictionary semantics and publishing policy stay in the application.
- No production unwrap/expect or silently dropped errors. Preserve original data on unsupported input or failed operations.
- Do not import an entire word processor application. Reused upstream code requires pinned provenance and preserved licenses.
- Run focused semantic tests, fmt and strict Clippy. GUI appearance requires real compiled/native evidence, including IME and clipboard where applicable.
- Do not build optimized releases locally or replace/restart the owner's running WriterCraft preview. Local model tests are permitted; desktop distribution builds run on GitHub.
- Do not modify the original Writers Studio repository or user projects.
- Keep documents/README/specifications in English. Library diagnostics are machine-readable or host-localized; the host supplies UI translations.

## Completion boundary

The consumer requires full original editor parity in WriterCraft alpha.2: all current blocks, marks and editing interactions. A fixture test, offscreen layout, renderer preview or subset is an internal checkpoint only. Full native editing and integration acceptance cannot be declared from model tests. Keep gaps explicit until implemented and observed; do not remove a requirement because an upstream library lacks it.
