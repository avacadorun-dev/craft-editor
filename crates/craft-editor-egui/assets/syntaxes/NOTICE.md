# Native supplemental syntax data

These are native YAML grammar definitions, not JavaScript runtime code. The exact source editor advertised 77 options. The baseline pinned two-face bundle lacked ten of them; the additional data is loaded explicitly through syntect's native YAML loader.

`manifest.json` records the exact upstream URL/revision, SHA-256, modification status and retained license for every added grammar. `source-format-samples.json` is synthetic qualification data, one sample per original option; it contains no manuscript content.

## Source-specific MIT grammar ports

BASIC, Visual Basic/.NET, Arduino, Fortran, Scheme and Mermaid port the precise language-specific regex vocabularies of Refractor **5.0.0**, commit `ed824ac801ef4bb5b25f73a4bcd6ce46b7eec8c9`. Full original MIT text is retained in `../../third_party/licenses/refractor-LICENSE.txt`. Copyright Titus Wormer and Lea Verou. Source hashes and direct file links are in the manifest.

Changes for native execution:

- Regex flags and source token roles become syntect-native scopes and numeric YAML capture keys; source bytes are never altered. Lookbehind capture prefixes remain outside the colored capture.
- Arduino's original C++ inheritance uses the maintained native C++ grammar with the source Arduino keyword/constants/built-ins injected as a prototype. The role adapter explicitly preserves opaque parent C++ comment/string scopes even when a with_prototype match is injected into them. Tests require this behavior, including inside function bodies.
- Scheme's source R7RS vocabulary/number definitions remain specific to Scheme. Native contexts carry strings, nested block comments and datum comments across lines. The source dangling-radix alternative is grouped correctly instead of treating a bare radix prefix as a number.
- Fortran string contexts preserve doubled quotes and continuation-line comments. Type/statement/operator vocabularies match the source grammar.
- Mermaid preserves the source's distinct ER/flowchart/sequence/class arrow families, labels, styles, annotations and case-sensitive/case-insensitive declarations. No rendered-diagram behavior is simulated by these source tokens.
- The advertised original `vbnet` option was not itself registered as a Refractor alias. Native lookup binds it to the source Visual Basic vocabulary rather than copying that missing-highlight bug.

## Unmodified primary-source assets

The following definitions in `../../third_party/syntaxes/` are retained unchanged, with exact revisions and checksums in the manifest:

- JSON5: Brian Schwind's `bschwind/sublime-json5`, MIT.
- PlantUML: `nsiena/sublime-plantuml`, MIT.
- PromQL: `prometheus-community/sublimelsp-promql`, Apache-2.0.
- COBOL: `adukhan99/sublime_cobol`, the included BSD 2-Clause text (Copyright Colin T. A. Gray).

All license texts are in `../../third_party/licenses/`. The earlier standalone hlfcoding Mermaid package was considered and rejected because its current license adds use restrictions; none of its grammar content is included. The native Mermaid port is derived only from the explicitly pinned MIT source-editor grammar.

Grammar availability and token samples are semantic/native raster evidence. Complete consumer syntax acceptance still requires actual source-dialog/document interactions, language/copy controls, save/reopen, themes and independent native review.
