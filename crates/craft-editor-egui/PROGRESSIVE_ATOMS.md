# Progressive native atoms

Status: source implementation and focused fixtures, hosted verification pending. This is not full native editor, accessibility, operating-system input, or upstream syntax acceptance.

A host may prepare media asynchronously while the single editor canvas stays mounted. `AtomResolver::placeholder` provides an explicit `Pending` or `Failed` status, localized label, bounded source preview and logical dimensions. Existing resolvers keep their fail-closed behavior by returning no placeholder.

The projection reserves one opaque native object, including exactly one canonical scalar for inline mathematics. Transparent transport pixels are never a final diagram preview: the canvas paints a native status box at the measured object rectangle. Authored captions remain separate native caption paragraphs. Canonical source, node paths, marks and history never contain status text or placeholder pixels. Real raster output replaces only the derived geometry when the resolver generation advances; semantic selection remains attached to its original canonical blocks and offsets.

`AtomResolver::prioritize` supplies currently visible canonical atom paths. This hint has no document authority and must not block or perform I/O. Hosts can lazily prepare viewport atoms, evict derived images, and render them again when needed instead of rejecting a valid document because its total media exceeds a cache budget. The renderer package exposes `Renderer::prepare_atom` and `PreparedCatalog::encoded_bytes` for progressive acknowledgements and host cache accounting.

The host remains responsible for exact source/theme/scale identity, authorized asset snapshots, cancellation, stale-result rejection, bounds, localized labels, and retry policy. The canvas does not read URLs, files, projects or application databases. Pending/failure presentation does not waive the requirement for actual diagrams, formulas and images once available.

Focused fixtures in `tests/progressive_atoms.rs` cover editable prose during pending/failure, exact opaque-node paths, Unicode and inline scalar mapping, selection/history stability after acknowledgement, and visible-atom hints. Renderer fixtures separately exercise one-atom preparation and source-preserving failure output. Their presence is not a claim that hosted checks have run.
