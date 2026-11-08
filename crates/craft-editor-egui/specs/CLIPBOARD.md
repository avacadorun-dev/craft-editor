# Canonical native clipboard contract

Tracking: [WriterCraft authoring #7](https://github.com/avacadorun-dev/writer-craft/issues/7), [complete editor #4](https://github.com/avacadorun-dev/writer-craft/issues/4). This is an implementation checkpoint; it does not close either issue or qualify the native desktop workflow.

## Sources and scope

The frozen original editor's `sanitize-pasted-html.ts`, `code-block.tsx`, `math.tsx`, `extensions.ts`, installed Tiptap clipboard/mark/link commands and related tests define behavior. No generic Markdown editor/parser is enabled in the original. Complete fenced-code and whole inline/display formula paste are explicit plugins; inline mark/Markdown-link paste rules and autolinking are additional required behavior. Plain Mermaid text is prose; a fenced `mermaid` language block follows the original code-paste rule unless canonical/HTML data explicitly identifies a diagram.

The private `application/vnd.craft-editor.slice+json` representation contains only the selected canonical fragment, with open boundary depths. It preserves all 21 canonical node kinds (the document root is represented by the fragment envelope), all six marks, attrs, unknown fields, and null/omitted presence. A clipboard copy must never embed the full unselected document as a convenient reconstruction mechanism.

Private data is untrusted. Decode checks the host's explicit byte budget, envelope version/fields, wire shape, bounded tree depth/node count, identities, marks, attributes and open-depth claims. Open edge wrappers may temporarily lack a complete child grammar/grid; insertion validates the complete candidate before one revision-guarded transaction commits. Invalid rich data rejects rather than quietly falling back to replacing a selection with its plain representation.

## Session and transport API

- `copy_selection() -> Result<Option<ClipboardBundle>>`: None means a caret with no selection; leave the clipboard untouched.
- `ClipboardBundle { rich, html, plain }`: advertise canonical JSON, interoperable HTML and meaningful text concurrently. Atom text uses formula/diagram/image source forms; synthesized U+FFFC is never published as an object fallback.
- `set_clipboard_provider(Option<Arc<dyn ClipboardProvider>>)` installs a shared long-lived host service.
- `cut_to_clipboard()` acknowledges a complete native write before deleting selected content. A write failure leaves content, revision, selection, history and pending changes intact. A later deletion rejection can leave a useful copy on the clipboard, but does not lose manuscript data.
- `paste_bundle(bundle, max_bytes)` and `paste_from_clipboard()` use the same canonical selection/history and never fetch assets. Pasted identities are new; source references and selected opaque content remain canonical.
- `ClipboardBundle::canonical_slice(max_bytes)` performs source-priority parsing without mutation; `RichSlice::asset_references()` exposes image sources/IDs to host ownership and media-acquisition policy. Neither transport nor parsers authorize cross-project assets. The host's asset resolver must remain project scoped.

Open text-edge insertion joins first/last fragments into the destination with exact Split/Text/Join maps, preserves the existing rich tail and records the result caret in the same Undo/Redo entry. Target context wins for the first open wrapper, as in native rich paste; selected inner nodes/marks stay exact. Source references are merged explicitly. Closed fragments retain their wrappers and opaque fields.

The host supplies the byte budget; there is no hidden smaller clipboard default. Size is the sum of concurrently advertised formats. Production budget selection must account for multiple representations of a document, not accidentally reject a valid body merely because it has both HTML and private JSON.

## Dependencies and platform boundary

The optional `native-clipboard` feature uses [clipboard-rs 0.3.5](https://github.com/ChurchTao/clipboard-rs), MIT, with image decoding disabled. Its public multi-format write supports private custom bytes, HTML and text in one ownership operation. The portable MIME type maps to valid `app.craft-editor.document-slice-json` on macOS; raw MIME strings are not valid pasteboard UTIs. The adapter verifies native readback of every advertised representation before acknowledging a write, because the backend can report success while a custom type was rejected. macOS/Windows/X11 are explicit backend targets; `wayland-clipboard` additionally selects its optional Wayland backend. Compiling a backend is not native interoperability evidence. The provider remains alive while serving a selection; a per-copy instance or blocking sleep is not the ownership solution.

The default egui/eframe integration carries plain text only. With no host provider, Copy publishes a meaningful text alternative; Cut refuses to delete without an acknowledged transport. The actual WriterCraft integration must install the service before claiming rich Copy/Cut/Paste works.

HTML uses [Scraper 0.29.0](https://github.com/rust-scraper/scraper) (ISC), [ego-tree 0.11.0](https://github.com/rust-scraper/ego-tree) and maintained html5ever 0.40 rather than an ad-hoc tag parser or the test-only rcdom tree. Scripting, event handlers, arbitrary CSS and resource loading are never executed. Word/Docs style semantics are converted into canonical marks. Images retain only source-approved references/dimensions; actual bytes belong to host media policy. Original formula/scene/heading serialization is recognized rather than repeating the original sanitizer's accidental stripping of those node markers.

The shared `is_allowed_link_uri` model function matches the installed original Link default protocols and exact whitespace normalization. The source's external HTML sanitizer is intentionally stricter (http/https/mailto and a bounded internal project route); those are distinct source contracts, not competing manual UI validators.

## Required remaining acceptance

This checkpoint covers exact selection export, closed/ordinary open text fragments, explicit fence/formula rules, basic sanitized rich HTML, safe native ownership seam and history. It does **not** establish full clipboard parity. Complex open list/table/wrapper fitting, rectangular cell selections, matching inline mark/Markdown-link/autolink rules, selected-URL link application, image clipboard/drop acquisition, original source references across owned projects, real OS round trips and multi-app HTML interoperability remain required. Unsupported complex fitting refuses atomically with the clipboard intact. No such gap is permission to reduce alpha.2 scope.

Native macOS Copy/Cut/Paste, platform menu shortcuts, clipboard-manager behavior, external Word/Docs/browser input and X11/Wayland ownership need compiled observation before issue closure. Synthetic tests cannot supply that evidence.

## Deferred host media jobs

`ClipboardMode::Deferred` captures focused rich-canvas Copy/Cut/Paste requests. Ordinary dialog `TextEdit` widgets retain their normal clipboard behavior. Paste captures the raw provider bundle exactly once at the command boundary, then hands immutable bytes to the host's bounded worker; equal plain text is never a rich clipboard identity. The native transport does not promise a physically atomic cross-format OS generation snapshot. A host media capsule must bind the exact private rich bytes.

`WriteTicket::write` produces a sealed `WriteAck` only after the real provider acknowledges all advertised formats. The host may add registered opaque attachment formats but cannot replace captured canonical/plain source content. Cut and Paste completion consume one-shot session/revision/exact-selection guards; composition, stale state, cancellation and mismatched acknowledgements preserve canonical writing. Copy completion never mutates writing. Hosts own bounded worker queues and cancellation on project/document close.

`ClipboardBundle::formats` carries explicitly registered host formats inside the same combined input budget. `NativeClipboard::with_formats` reads no unregistered OS type and maps each portable MIME to a validated reverse-DNS macOS UTI. Private payloads remain untrusted. The hosted native probe publishes a fresh nonce in one additional opaque format as well as canonical JSON, HTML and Unicode plain text; no local author clipboard is accessed.
