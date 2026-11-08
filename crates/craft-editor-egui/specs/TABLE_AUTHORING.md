# Native table authoring contract

Source: the installed original TableKit configuration (`resizable`, `renderWrapper`, `allowTableNodeSelection`) and ProseMirror table commands. Generic canonical table builders already preserve spans/widths/unknown attributes. This contract adds native consumer wiring; model-only evidence is not native parity.

- Width arrays describe logical columns; zero/absent entries are automatic. Native measured tracks honor explicit widths and distribute remaining space to automatic tracks. Imported widths are not normalized in canonical JSON. Nested tables use their containing cell's available width. Cell alignment changes the disposable cell paragraph projection, never source marks/text.
- Native pointer handles target a measured logical column and are bound to session/revision. Resizing displays a guide, clamps new widths to the original 25-point minimum and existing 20000 bound, and commits one guarded width transaction on release. Escape/cancellation, stale source, composition and read-only mode commit nothing.
- Rectangular selection, text selection and atom selection remain distinct mutually exclusive forms with one Undo history. Host dialogs use sealed bookmarks, including pending typing marks.
- Add row/column acts after the complete source selected rectangle. Delete row/column removes the complete selected range atomically but refuses deleting the last remaining extent, matching the original explicit Delete table command. Delete table retains a valid parent and a nearby insertion target; Undo restores exact writing and original selection.
- Copy/Cut/Paste use existing acknowledged clipboard publication. Cut does not delete before publication success. Native capacity is a host policy; generic parsing preserves wider data.

Required tests include merged/ordinary widths, inherited nested content alignment, range mutation/Undo, pointer down/move/release, Escape, stale revision, read only and rejection without mutation. Hosted geometry/native clipboard qualification precedes consumer repinning; real native authoring/IME/AX and independent UX remain separate acceptance.

The formatting selector reports Paragraph, the real Heading1–6, Code, Mixed or Unavailable from the same complete text/cell scope used by the formatting command. A read-only state query preserves selection direction, pending typing marks, canonical content and history.
