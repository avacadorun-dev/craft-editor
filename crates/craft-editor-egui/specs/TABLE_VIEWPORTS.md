# Bounded native table viewports

A table wider than its containing writing column must remain fully reachable through local horizontal scrolling. Fitting by scaling/truncating columns, widening ordinary prose or inserting canonical spacer blocks is not acceptable.

## Architecture

Keep the canonical document and measured WordCraft layout immutable. Derive table-local viewport rectangles from real cell boxes, logical tracks and the containing column width. The disposable native table projection reserves one scrollbar gutter with `TableProps.space_after`; this is a component extent, not authored content. Nested viewports intersect their ancestor clips.

One shared viewport map owns horizontal offsets, forward/inverse point conversion and clipping. It is used by raster drawing, text/cell/node geometry, pointer hits, selection/decorations, caret/IME placement, visible-text scheduling and resize handles. Use WordCraft's typed `Draw::Clip` and horizontal `Draw::Turned` wrappers around per-placed-item display output; update wrapper translation without reshaping/reflowing text on every scroll. Offscreen glyphs remain canonical and measured, rather than being discarded.

Store view offsets in the native session, keyed by stable table identity. Clamp/reapply them after revision, typography or width changes; anonymous tables use content/path-qualified view keys. Never use view keys or offsets as command/persistence authority. Scrolling changes no document revision, Undo, note anchor or clipboard content.

Native horizontal controls retain a proportional rectangular thumb, keyboard/accessible range semantics and a host-localized label. A horizontal wheel gesture over a table affects its innermost visible scrolling viewport. Ordinary vertical scrolling stays with the manuscript. Keyboard/caret navigation reveals the exact target, including the final logical column, without moving surrounding prose horizontally. Source dialogs and column drags continue using sealed semantic targets; a viewport change does not authorize an edit.

## Required scenarios

1. A 63-column or explicitly wide table remains within a bounded document surface; ordinary prose before/after keeps its original measure. Scroll to the final column and edit/copy it.
2. After horizontal movement, the visible text/cell/atom hit, selected rectangle, decoration, resize border and IME cursor refer to the same canonical positions and clipped bounds. Hidden content cannot intercept a visible sibling's input.
3. A nested wide table scrolls inside its cell/ancestor clips. Independent table offsets do not leak between tables/documents.
4. Tab/arrows/find/reveal reach an offscreen cell and keep the caret/selection visible. Escape dismisses the active transient before changing document focus.
5. Resize, font/measure/theme changes, pending media completion and Undo preserve canonical widths/text, anchors and history; view offsets clamp coherently.
6. Wheel/scrollbar/pointer/keyboard interactions alter only presentation. Verify exact JSON/revision/history before/after; rejected resize/paste operations still leave content unchanged.
7. Real compiled native evidence at narrow/common widths, Light/Dark and long localized labels complements raw-event/layout/renderer tests. This contract does not imply complete OS IME/accessibility acceptance.
