# cic-tools

Deterministic diagnostic applications over VFS resources and immutable format values.

- `cic-inspect manifest <mount>...` reports resolved resource paths and providers.
- `cic-inspect csf <virtual-path> <mount>...` reports decoded localization records.
- `cic-inspect map <virtual-path> <mount>...` inventories the MAP symbol table and opaque chunks.
- `cic-inspect map-height [--report | --png <output.png>] <virtual-path> <mount>...` writes exact
  height samples to a basename-derived 8-bit grayscale PNG by default, reports them with
  `--report`, or accepts an explicit PNG path.
- `cic-inspect map-blend <virtual-path> <mount>...` reports bounded version-6/7 tile, blend,
  edge-texture-class, and cliff values in stable source order.
- `cic-inspect map-render [--size <pixels>] [--pixels-per-cell <pixels>]
  [--terrain-policy <legacy|modern>] <virtual-path>
  [<output.png>] [<mount>...]` resolves Terrain INI classes, stages layered terrain, and writes a
  deterministic headless sRGB capture.
- `cic-inspect map-view [--pixels-per-cell <pixels>] [--terrain-policy <legacy|modern>]
  <virtual-path> [<mount>...]` opens the same terrain and custom-edge path in a perspective
  flyover. WASD/Space/Ctrl move, Shift boosts, right-drag looks, the wheel moves forward/back,
  R resets the camera, and Escape closes. Depth-capped 16/32-texel screen-space tiers stream
  asynchronously over the deterministic 8-texel background, so an oblique horizon cannot dilute
  foreground density. Superseded CPU bakes cancel immediately and resident replacements overlap
  briefly. It uses edge blending, mipmaps, and anisotropic
  filtering while keeping directional shading viewer-only. Installed terrain profiles also resolve
  texture archives, bounded caustic animation, and water-transparency inputs before calling the
  renderer-facing viewer API.
- `cic-inspect w3d <virtual-path> <mount>...` reports the complete nested chunk inventory.
- `cic-inspect w3d-mesh <virtual-path> <top-level-index> <mount>...` reports decoded geometry.
- `cic-inspect w3d-export [--gltf] <virtual-path> ...` exports a portable animated preview.
- `cic-inspect w3d-render [--animation <index>] [--frame <frame>] [--time <seconds>]
  [--rotation <radians>] <virtual-path> ...` writes a deterministic textured diagnostic capture.
- `cic-inspect w3d-view <virtual-path> ...` opens the textured, rotating animated viewer.

Owns user-facing diagnostic programs. Tools compose public VFS and format APIs and may
format reports, resolve user-owned resources, and launch renderer APIs, but must not
duplicate parsing rules or engine behavior.
