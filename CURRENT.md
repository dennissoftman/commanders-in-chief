# Current Objective

## Objective

R4 is now active. Its first vertical slice is a bounded, unknown-preserving WND inventory and
immutable layout/control decoder, followed immediately by a surface-free `wgpu` capture of one
original synthetic menu. That creates a visible UI result before adding retained interaction,
user-owned mapped images/fonts/CSF labels, the main-menu stack, modern display settings, and the
skirmish/map-selection harness. R4 remains presentation-only: callbacks are allowlisted typed
events, MAP scripts stay inert until R5, and project-owned post-parse patches augment rather than
modify user-owned WND bytes.

R3 is complete; its charter, progress, and completion evidence are recorded in
[docs/milestones/r3-map-scene.md](docs/milestones/r3-map-scene.md). R4 adds
bounded WND/UI ingestion and a navigable `wgpu` main-menu/skirmish demo so map compatibility can be
inspected through the intended shell before simulation exists. Its Options path will use bounded
post-parse WND patches—not hardcoded window-name rendering—to add modern window mode, resolution,
refresh-rate, and UI-scale controls with transactional confirmation/rollback.

## Status

- Local formatting, strict Clippy, and the complete workspace test suite pass.
- R1 remains in progress: `BIG4` retail verification is open (see
  [docs/milestones/r1-big-csf.md](docs/milestones/r1-big-csf.md)).

## Next verified step

Start R4 with a bounded WND container/layout inventory and immutable control tree that preserves
unknown fields and callback names without invoking them. Add original synthetic positive,
truncation, limit, and unknown-preservation fixtures, a stable `wnd` report, and a surface-free
`wgpu` capture of one synthetic menu. The next slice resolves user-owned mapped images, explicit
fonts, and CSF labels before retained interaction and main-menu navigation.
