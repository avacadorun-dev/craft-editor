# Native syntax highlighting contract

Tracking: [WriterCraft #8](https://github.com/avacadorun-dev/writer-craft/issues/8), with full editor acceptance in #4. This is a reusable native token/layout foundation, not completed syntax-language parity or native UX acceptance.

## Source audit

The read-only original `src/features/editor/model/code-languages.ts` contains **77 options: 76 named formats and intentional plain text**. SHA-256: `120a3705405d86fcf70da88917aa869bdd193cdc83dc478f7a2b478ae74b9163`. Preserve those option values and imported unknown values exactly in canonical JSON and Markdown fences. Lookup normalization is separate and never serialized. Original `code-block.tsx` uses Refractor decorations over unchanged source, keeps multiline code, provides language selection/copy, and preserves unknown imported language choices. That behavior remains the consumer contract.

`syntax::REQUIRED_LANGUAGES` records the full option set, including structured text. `coverage_report()` queries actual pinned grammars. `MissingRequiredGrammar` differs from intentional `PlainText` and `UnknownLanguage`; no known format may silently disappear from the release checklist. Native foundation availability is not complete editor authoring/copy UI or persistence acceptance.

## Dependency decision

Use exact `syntect = 5.3.0` and `two-face = 0.5.2+bat-0.26.1`, with only parser/dump and native Oniguruma features. The two-face bundle is curated from bat rather than a generated collection of approximate keyword regexes. Its pure-Rust fancy-regex bundle explicitly excludes source-required PowerShell and Sass (and other grammars). The selected maintained grammars therefore use native Oniguruma; the UI, document model, commands and token adapter remain Rust with no JavaScript, browser or WebView runtime. This is a native parser dependency, like the application's existing native SQLite dependency, not a vendored word-processor application.

Primary references verified on 2026-11-08:

- [syntect 5.3.0 API](https://docs.rs/syntect/5.3.0/syntect/), [native/pure Rust regex differences](https://github.com/trishume/syntect#pure-rust-fancy-regex-mode-without-onig).
- [two-face 0.5.2+bat-0.26.1 API and grammar exclusions](https://docs.rs/two-face/0.5.2%2Bbat-0.26.1/two_face/).
- [Exact bundle manifest](https://docs.rs/crate/two-face/0.5.2%2Bbat-0.26.1/source/Cargo.toml.orig).
- [Full upstream notices](https://codeberg.org/CosmicHarper/two-face/src/tag/v0.5.2+bat-0.26.1/generated/acknowledgements_full.md), retained verbatim in `third_party/licenses/two-face-NOTICE.md`. Hosts also have `acknowledgements_markdown()` for packaging.

The selected bundle does not advertise all original long-tail grammars. Supplementary licensed native grammars are still required for formats such as BASIC/VB.NET, COBOL, JSON5, PromQL, Mermaid and PlantUML; the actual hosted coverage report, including exact grammar names, is authoritative. JSX, Scheme, Gradle, Java properties, HTTP and alias mappings also require explicit audit rather than an unrelated-language fallback. Do not mark #8 complete from the broad bundle alone.

## Public token and color contract

`highlight(source, language)` returns exact half-open **UTF-8 byte** `TokenSpan` ranges plus semantic color roles and actual `Coverage`. Source bytes include whitespace, tabs, CRLF/LF, Unicode scalar/grapheme sequences and trailing newlines unchanged. Ranges form one ordered, non-overlapping complete source partition. `Highlighted::validate` and `TokenSpan::text` reject invalid ranges before any backend operation. Parser state carries across original lines, including multiline comments and strings.

No highlighting markup is saved into canonical text, marks or language attributes. A future source-dialog layouter can reuse the same tokens with egui while retaining its own fonts and cursor model. Unknown imported formats remain readable/editable; a missing required grammar remains an explicit acceptance gap.

`Palette::for_surface(foreground, background)` derives Light/Dark/tinted-surface colors with semantic roles and 4.5:1 foreground contrast where the host foreground permits it. Callers can supply individual role colors through `GeometryStyle.syntax_palette`; `None` leaves code plain. `CanvasStyle` derives its palette from existing host colors without adding a new caller field. Syntax colors never change font size, family, weight, italic, spacing or canonical marks.

The WordCraft projection linearly splits disposable formatting runs at token boundaries. It preserves inherited font/mark properties and source bytes, avoiding repeated quadratic range-format calls. Exact canonical byte/scalar and native caret mappings remain authoritative. Each code block reports its exact stored language and coverage through `FlowGeometry::syntax_reports()`.

## Qualification and remaining work

Hosted tests cover Unicode byte partitions, multiline state, Rust semantic classes, YAML/JSON structured text, aliases, explicit unknown/plain distinction, source option inventory, Light/Dark/tinted contrast, native caret geometry and canonical metadata preservation. The hosted `native_syntax` example retains actual native Light/Dark raster PNGs, the 77-format coverage report and complete notices. Raster/example success is not native editor UX acceptance.

Required before issue/release acceptance:

- Supply and qualify every missing required grammar and source-specific aliases; no generic catch-all token simulation.
- Exercise all supported source samples, invalid/incomplete edit states, whitespace and Unicode combinations.
- Integrate visible language controls, original code copy behavior and source-dialog highlighting in the consumer.
- Qualify parsing/caching/worker scheduling for long code documents so editing remains responsive; a synchronous native layout checkpoint alone is not performance acceptance.
- Observe actual compiled native Light/Dark/palette behavior, caret/selection, Undo/Redo and restart durability.
