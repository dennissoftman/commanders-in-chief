# ADR 0006: Terrain Staging and Texture Policy

- Status: Accepted
- Date: 2026-07-22

## Context

MAP texture-class names are semantic Terrain definitions, not image paths. The legacy terrain path
loads those definitions from ordered default and edition INIs, inherits new definitions from the
current `DefaultTerrain`, then loads the selected image beneath `Art/Terrain`. Each 64-by-64 source
tile contains four independently indexed 32-by-32 cell quadrants. Terrain cells can add primary
and extra procedural-alpha layers, select a triangle diagonal, and reference separate custom-edge
or cliff-UV behavior.

Format values must remain immutable and renderer-neutral, while rendering must not acquire VFS,
filesystem, simulation, or wall-clock ownership.

## Decision

- `cic-formats` narrowly and boundedly decodes ordered `Terrain` declarations and optional
  `Texture` fields. It does not resolve resources or apply renderer policy.
- `cic-tools` applies current-value and `DefaultTerrain` inheritance in the source load order,
  resolves selected sheets beneath `Art/Terrain` through the VFS, and decodes bounded RGBA images.
- Installed terrain profiles explicitly mount MAP, Terrain, texture, INI, and patch archives in
  stable base-then-expansion order.
- `cic-render` consumes only decoded MAP values and caller-supplied texture resources. It stages
  source world scale, stable row-major cells, validated triangle indices, and the base, primary,
  then extra layer order.
- The first capture path bakes one deterministic sRGB terrain image at an explicit power-of-two
  pixels-per-cell setting. It selects packed quadrants, reproduces the source's repeated 2-by-2
  mip rounding and procedural mask equations, and composites layers in source order.
- Source blend and cliff records select triangle diagonals. Stored cliff mappings retile a cell
  only when the mapped tile remains in the selected terrain class.
- `ZeroHourLegacy` is the default explicit compatibility policy and applies the source's bounded
  steep-slope UV adjustment. `Modern` keeps stored cliff mappings but skips that implicit retile.
  Both policies use height-selected triangle diagonals when a cliff mapping stretches the cell.
- Custom edge classes produce a separate stable index stream and edge texture. Quarter-atlas
  selection follows edge direction, inversion, long-diagonal state, and row/column parity. The
  deterministic preview maps white edge texels to the selected material at half coverage, black
  texels to gaps, and colored texels to the decorative overlay; it does not claim bit-identical
  fixed-function multipass output.
- Headless capture uses an explicit deterministic isometric projection, dimensions, and bake
  resolution. No renderer clock or host ordering enters staging or output diagnostics.
- The interactive viewer consumes the identical staged values and GPU edge pass. Its perspective
  camera is presentation-only: caller-supplied frame deltas drive WASD/vertical movement, boost,
  mouse look, wheel dolly, and reset without changing staged terrain or deterministic captures.
- Large maps retain the deterministic 8-pixel-per-cell background. `map-view` additionally packs
  source terrain sheets into a compact 64-pixel tile atlas, deduplicates source-defined custom-edge
  results into a 32-pixel atlas, and uploads immutable per-cell material/UV/mask metadata once.
  Camera-frustum intersection produces camera-space-depth-capped 16/32-texel page demand rather
  than a horizon-sized atlas request.
- Interactive detail uses a project-authored software virtual texture. One hundred twenty-eight physical
  264-by-264 layers contain a 256-pixel interior plus a four-pixel filter border. Two stable page
  tables map 8-cell/32-texel and 16-cell/16-texel virtual pages into that shared cache. Deterministic
  LRU replacement retains revisitable pages; projected page bounds follow the actual camera angle,
  and coarse visible coverage is ranked before fine upgrades. A missing mapping samples the stable background.
  Compute shaders compose authored base/primary/extra layers, procedural masks, cliff UVs, custom
  edges, and Modern macro variation. Separate compute passes generate complete linear-light,
  alpha-aware mip chains. Camera movement uploads only bounded job/page-table metadata and performs
  no CPU terrain texture composition. Trilinear sampling requests up to 16x anisotropy with backend
  fallback.
- Viewer slope lighting derives a face normal from world-position derivatives and applies one
  fixed directional light. This is an explicit project-authored presentation preview; it does not
  enter headless captures and is not represented as MAP-authored lighting before `LightingData`
  has its own bounded semantic gate.
- Terrain staging guarantees counter-clockwise winding when viewed from above for both diagonal
  choices. Headless and interactive terrain, custom-edge, and detail pipelines therefore cull back
  faces. Water and general model materials keep their own explicit culling policies.
- The classic monolithic MegaTexture model is not the canonical terrain representation. MAP tile,
  blend, cliff, and edge semantics remain immutable inputs needed by compatibility, tools, future
  terrain mutation, and deterministic page regeneration. Residency demand is derived
  conservatively from the viewport; GPU feedback never enters simulation or authoritative state.

## Consequences

Synthetic and user-owned maps use the same VFS/resource path, and terrain-class resolution is no
longer guessed from class names. Layer construction can be tested independently from GPU capture,
and the renderer boundary is shared by headless and interactive presentation. The baked base and
edge atlases are intentionally bounded to 4,096 pixels per axis and 64 MiB each. Exact legacy
fixed-function custom-edge blending remains a documented preview difference rather than a hidden
claim of pixel equivalence. The page cache restores foreground texel density without a full-map
high-resolution allocation, retains bounded memory, and reuses revisited regions. The CPU baker
remains the deterministic headless/tooling reference; it is not in the interactive camera-update
path. Renderer-neutral terrain semantics remain authoritative over either representation.
