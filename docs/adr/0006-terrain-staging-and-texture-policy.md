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
- Large maps retain the deterministic 8-pixel-per-cell background. The staged value also owns a
  bounded clone of the immutable decoded height/blend values and content-deduplicated texture
  resources. `map-view` intersects the camera frustum with the bounded terrain height slab but does
  not assign one density to its complete horizon-reaching rectangle. Projected cell size defines a
  16-texel mid-field depth cap and a nested source-density 32-texel foreground cap. Each tier owns
  an independent quantized request, predictive margins, cancellation generation, worker, GPU
  texture, and fade uniform. Existing texture limits remain authoritative: margins are trimmed
  before a tier's visible bounds. A resident or requested patch suppresses work whenever it already
  covers those bounds at equal density. New generations immediately cancel obsolete
  row/tile/composition work; the worker discards stale completions, and only the latest complete
  linear-light, alpha-aware mip chain reaches GPU upload. The previous GPU patch remains resident
  during a short caller-time-driven overlap, while spatial edge coverage joins each equal-depth,
  no-depth-write tier to its coarser parent. Trilinear sampling requests up to 16x anisotropy with
  backend fallback.
- Viewer slope lighting derives a face normal from world-position derivatives and applies one
  fixed directional light. This is an explicit project-authored presentation preview; it does not
  enter headless captures and is not represented as MAP-authored lighting before `LightingData`
  has its own bounded semantic gate.
- Terrain staging guarantees counter-clockwise winding when viewed from above for both diagonal
  choices. Headless and interactive terrain, custom-edge, and detail pipelines therefore cull back
  faces. Water and general model materials keep their own explicit culling policies.
- The classic monolithic MegaTexture model is not the canonical terrain representation. MAP tile,
  blend, cliff, and edge semantics remain immutable inputs needed by compatibility, tools, and
  future terrain mutation. If profiling justifies it, the renderer may replace rectangular baked
  patches with a software virtual-texture cache: fixed-size atlas pages, a page table, border
  texels, a guaranteed coarse mip, and deterministic keys over profile/region/mip/content. Page
  composition must continue to use the shared staged terrain path, and requested pages must be
  derived conservatively from the viewport rather than GPU feedback entering simulation state.

## Consequences

Synthetic and user-owned maps use the same VFS/resource path, and terrain-class resolution is no
longer guessed from class names. Layer construction can be tested independently from GPU capture,
and the renderer boundary is shared by headless and interactive presentation. The baked base and
edge atlases are intentionally bounded to 4,096 pixels per axis and 64 MiB each. Exact legacy
fixed-function custom-edge blending remains a documented preview difference rather than a hidden
claim of pixel equivalence. The near-field window restores foreground texel density without a
full-map high-resolution allocation. CPU rebakes no longer block presentation, obsolete work is
cancelled before it can become visible, and bounded GPU replacements overlap their previous
residents instead of exposing a blurry-to-sharp snap. A software virtual-texture cache is therefore
a compatible future optimization, not a replacement for renderer-neutral terrain semantics or an
immediate requirement.
