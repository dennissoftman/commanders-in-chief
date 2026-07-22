# ADR 0005: MAP Inventory, Height, and Blend Semantic Boundary

- Status: Accepted
- Date: 2026-07-21

## Context

The MAP `DataChunk` format has a symbol table and versioned, length-delimited chunks but no bit or
type field that generically distinguishes opaque data from nested chunks. The legacy loader knows
which payloads to recurse into through registered label-specific callbacks. Treating every payload
as nested would reject valid field data; guessing based on byte patterns could misclassify unknown
content and lose compatibility evidence.

Version-1 height data also has a legacy downsampling transform in some source loading paths, while
the stored record itself remains a complete byte grid.

## Decision

- The generic MAP inventory parses the global symbol table and top-level chunk framing only.
- Every top-level payload is preserved as opaque bytes, including known labels.
- Source-established `EAR\0` RefPack wrappers are decompressed through a separately bounded format
  layer before `CkMp` parsing; other wrappers are not guessed.
- Identifier lookup follows the source reader's deterministic last-table-entry-wins behavior while
  retaining all symbol records.
- Semantic decoders open only explicit known labels and established versions.
- The first semantic gate accepts `HeightMapData` versions 1 through 4, validates exact layout and
  sample cardinality, and returns the stored grid without applying version-1 resampling.
- The next gate accepts `BlendTileData` versions 6 and 7, retains signed per-cell indices, and
  validates all table ranges and exact closure. Version 6 derives cliff flags from neighboring
  height samples using the source's greater-than-9.8-world-unit rule; because stored height units
  scale by 0.625, this is the exact integer threshold of 16. Version 7 normalizes the source-defined
  short cliff-bitmap stride into a conventional row stride with unavailable right-edge bits clear.
- Texture names remain opaque bytes, and cliff UV values remain exact finite IEEE-754 values;
  renderer interpretation is outside the parser.
- Any future compatibility transform must be selected by an explicit `ZeroHourLegacy` or `Modern`
  policy at a consumer boundary, not hidden in the lossless parser.

## Consequences

Unknown chunks remain round-trippable evidence and cannot trigger speculative recursion. Nested
object inventories require separate label-aware gates. Renderer ingestion can consume validated
height and blend values without owning parsing or compatibility policy. Version-1 maps remain
inspectable, while visible compatibility behavior stays deferred until user-owned observations
support a policy. ADR 0009 defines the ordered label-aware gates for the remaining complete MAP
scene, including objects, roads, scenario metadata, and scripts, without changing this inventory
boundary.
