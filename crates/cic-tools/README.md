# cic-tools

Deterministic diagnostic applications over VFS resources and immutable format values.

- `cic-inspect manifest <mount>...` reports resolved resource paths and providers.
- `cic-inspect csf <virtual-path> <mount>...` reports decoded localization records.
- `cic-inspect w3d <virtual-path> <mount>...` reports the complete nested chunk inventory.
- `cic-inspect w3d-mesh <virtual-path> <top-level-index> <mount>...` reports decoded geometry.
- `cic-inspect w3d-export [--gltf] <virtual-path> ...` exports a portable animated preview.
- `cic-inspect w3d-render [--animation <index>] [--frame <frame>] [--time <seconds>]
  [--rotation <radians>] <virtual-path> ...` writes a deterministic textured diagnostic capture.
- `cic-inspect w3d-view <virtual-path> ...` opens the textured, rotating animated viewer.

Owns user-facing diagnostic programs. Tools compose public VFS and format APIs and may
format reports, resolve user-owned resources, and launch renderer APIs, but must not
duplicate parsing rules or engine behavior.
