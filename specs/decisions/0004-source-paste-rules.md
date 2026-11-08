# Source-compatible paste rules without a Markdown editor runtime

Status: accepted for bounded implementation; compatibility and native acceptance remain unverified until the named hosted tests pass. Tracked by WriterCraft issue7 and full editor epic4.

## Context

The source editor enables bold/italic/strike/inline-code paste rules and Link autolink/link-on-paste. Its installed Link defaults `markdownLinks:false`; the app does not override it. A general Markdown parser would invent conversions beyond the source's enabled behavior. Canonical/private internal paste must retain exact known/opaque node and mark metadata and skip source-style reinterpretation. The source also excludes code blocks and ProseMirror `data-pm-slice` HTML from ordinary mark paste rules.

Actual installed source hashes:

- `@tiptap/core/src/PasteRule.ts`: d789dabd5b98dd89c5c2fbb394b4a43910bf23e8b8ae83e1f6b7e3f69f1b284c
- `@tiptap/core/src/pasteRules/markPasteRule.ts`: 370eb7b18493224f328be038b076d538a4a0ff7adf876b5d895799804ace3980
- `@tiptap/extension-link/src/link.ts`: a86d57e74715057a26e0a0881b461e3468225d75fc66133bd8c151cec7949a89
- `linkifyjs/dist/linkify.cjs`4.3.3: c2235aeb0733ff36f12bda2aa8dc1f5efbfc94315089d5b69bc75934ccea5f9c

Synthetic source probes use only installed read-only dependencies and synthetic text. A headless result that appears anomalous is a hypothesis for mounted/source validation, not permission to encode an inferred bug as a product contract.

## Decision

Use bounded structural text-run transformations for the four enabled marker families. Apply them only to eligible incoming external text/HTML; preserve surviving text-node/mark envelopes exactly and keep all transformations in one guarded insertion/history entry. Skip code blocks, incompatible inline code and private internal representations. Keep complete code-fence/formula handling separate. Whole-URL-over-selection behavior uses the shared canonical mark planner, not a second history transaction or app-specific range implementation.

Evaluate pinned `linkify`0.11.0 for linear Unicode-aware URL/email boundary scanning. Its [official documentation](https://docs.rs/linkify/0.11.0/linkify/) documents linear scanning and explicitly does not validate TLDs. Its [packaged manifest](https://docs.rs/crate/linkify/0.11.0/source/Cargo.toml) has one runtime dependency, memchr2.0.1, and offers MIT OR Apache-2.0. Select the MIT option and preserve notices. The [pinned upstream CI](https://github.com/robinst/linkify/blob/0.11.0/.github/workflows/ci.yml) explicitly tests minimum Rust1.46.0; this is below CraftEditor's1.97 declaration. Upstream release0.11.0 fixes URL-delimiter handling, as recorded in the [official changelog](https://docs.rs/crate/linkify/0.11.0/source/CHANGELOG.md).

A frozen MIT/IANA TLD mirror from the source linkifyjs4.3.3 lives in third_party/linkifyjs with its license and source digest. Use original defaultProtocol http, permitted URI validation and shouldAutoLink policy. The Rust scanner remains a component, not an assertion of equivalence: actual-source corpus comparisons must verify punctuation, Unicode, schemes, email and bare-domain behavior; implement documented compatibility adjustments or keep gaps explicit.

No parser opens URLs, reads files, fetches image bytes or introduces another runtime. Input remains bounded by the existing host clipboard budget and canonical model resource limits. Match planning must be linear/bounded and operate in explicit UTF-8/scalar coordinates, without catastrophic regex backtracking. Unsupported wrapper/table fitting remains a separate required workstream; paste rules do not flatten such data or close the full-editor gate.

## Verification

Required evidence includes actual-source synthetic corpus, exact rich/opaque metadata preservation, code/internal exclusions, link-on-paste, atomic Undo/Redo selection and revision maps, stale/error no-mutation paths, and full hosted Rust fmt/Clippy/tests. Native OS interoperability and workspace interaction stay independent gates. No local Cargo/fetch/build or author clipboard access is used.
