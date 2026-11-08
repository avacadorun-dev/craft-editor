# 0008 — Bounded native table viewports and standard nested margins

Date: 2026-11-09. Status: implementation/hosted qualification in progress; no consumer or native UX acceptance is implied.

## Decision

Keep wide tables' canonical widths and the manuscript's bounded prose measure. Use one immutable native layout with a shared path-aware viewport projection for raster, hit testing, text/cell/atom bounds, decorations, caret/IME, resize handles and visible-text scheduling. Typed `Draw::Clip`/horizontal `Draw::Turned` wrappers compose nested clips and update only translation values while scrolling. A session-local offset is presentation, never canonical or transaction authority. See `TABLE_VIEWPORTS.md`.

The native component reserves its scrollbar gutter through standard disposable `TableProps.space_after`. No canonical spacer blocks, font shrinkage, clipped-away cells or app-specific geometry patches are introduced.

## Verified backend defect and minimal maintained fork

Original backend: [storytold/wordcraft](https://github.com/storytold/wordcraft), exact base `da914ce674f18ebc4e08cca425cc8f8acca3372f`, MIT OR Apache-2.0. `table_flow.rs` honors bounded finite positive table margins for body flow. `boxes.rs::layout_table_block` omits both margins for a table inside a cell/free-standing box. As a result a nested component's declared following extent is absent and subsequent content can overlap its controls.

The actual initial nested viewport regression fails against that exact base in [qualification 42502116014](https://github.com/avacadorun-dev/craft-editor/actions/runs/42502116014). The same run passes the ordinary 63-column view, exact inverse pointer/selection and changed native-pixel cases. This is executed consumer reproduction, not a claim that the newly added direct upstream regression already ran on the unmodified base.

Maintain the narrow repair in [avacadorun-dev/wordcraft](https://github.com/avacadorun-dev/wordcraft), branch `fix/nested-table-margins`, [commit 82ace77](https://github.com/avacadorun-dev/wordcraft/commit/82ace77a17d71f8fce770227624319203e74cc7b), tracked in [fork issue 1](https://github.com/avacadorun-dev/wordcraft/issues/1). Its parent is the exact original base; all history/licenses and existing crate boundaries remain intact. Only standard nested nonfloating margin semantics, shared bound visibility and direct regressions change. It is not an application fork integration or a claim to Word feature parity. No upstream issue, comment or pull request has been sent.

The five existing egui backend dependencies and atom renderer's existing font dependency use the same exact fork revision. Qualification rejects mixed/duplicate WordCraft package sources so global font registration cannot split between layouts and atom rendering. `tools/qualify-wordcraft-backend.py` verifies source identity and runs three direct margin regressions on the hosted runner before all current CraftEditor semantic/native geometry/render/clipboard contracts. Qualification remains pending until actual checks pass; consumers retain their prior qualified pin meanwhile. No local Cargo build is used.

## Maintenance boundary

Do not absorb unrelated upstream app/UI changes. Future upstream adoption must preserve or qualify the same margin/viewport cases and retain license provenance. The small patch is suitable for later upstream review, but sending it requires separate communication authorization. A successful compile/layout fixture does not replace real native scrollbar/keyboard/IME/accessibility and independent UX acceptance.

## Qualified overflow checkpoint

Exact CraftEditor source `fd96a30cd650b47ef2f889120aaf0da537b201ae` passed [run42506252736](https://github.com/avacadorun-dev/craft-editor/actions/runs/42506252736): focused backend3margin regressions, all current semantic/native geometry/render tests, strict Clippy, example evidence, and actual native clipboard probes on hosted macOS and isolated X11. Raw egui cases cover final-column pointer/IME shadow/commit, reverse-cell resize/Undo, wheel/keyboard presentation-only scrolling, hidden input clipping and large host field/text styles. Artifact13627508520 SHA-256: `f8b704661bca2014f6bee07fff99586b9e62ae8c0d81a44c9b15886b1f053ba6`.

This qualifies the generic checkpoint for consumer integration, not full table/editor parity or OS IME/AX acceptance. Whole-table NodeSelection remains a distinct required follow-up described in `TABLE_NODE_SELECTION.md`; it is not implied by rectangular cell selection or the overflow checks.
