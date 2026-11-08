# Host typography and measure

Status: implemented in source; hosted/native qualification pending.

The canonical tree stores author content and formatting, not workspace reading preferences. Native `CanvasStyle`/`GeometryStyle` keep font family, size, line height, list-item spacing and standard ligature policy in the disposable presentation. Defaults preserve pre-existing backend metric semantics.

`LineHeightBasis::FontSize` means an exact font-size multiple for ordinary prose, rather than a multiple of the font's native ascent/descent. This distinction matters when a host migrates a browser's unitless CSS line-height. List spacing is independent: when supplied, list paragraphs have zero paragraph gap and sibling/nested list starts receive the requested item gap. Heading and code-specific semantics remain distinct from ordinary prose. No setting rewrites canonical JSON or editor history.

`fonts::zero_advance` uses the exact loaded family/style and actual zero-glyph advance. A host can therefore implement character measures without treating `ch` as an average glyph width. The current canvas reserves 18 points on each side; hosts account for that separately from their desired text measure.

Standard ligatures can be explicitly disabled while contextual/required script shaping remains enabled. Font-specific localized-form equivalence requires a host font/glyph qualification; no unsupported no-op `locl` switch is advertised by this API.

All inputs are finite, positive where required and nonnegative for gaps. Invalid typography fails layout without canonical mutation. Existing callers can opt in through the new fields or retain defaults. Actual supported viewport, language, font and IME behavior remains a consumer acceptance gate.
