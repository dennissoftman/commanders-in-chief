# Commanders in Chief

Commanders in Chief is a GPL-licensed, cross-platform compatibility engine project for
classic SAGE-era real-time strategy game data. It is an independent community project
and is not affiliated with or endorsed by Electronic Arts.

The first executable milestone is a deterministic resource inspector. The current code
provides bounded binary input, normalized virtual paths, deterministic overlay handling,
BIG archive mounting, and a CLI that inventories mounted resources.

```powershell
cargo test --workspace
cargo run -p cic-tools -- manifest path\to\base path\to\archive.big path\to\override
cargo run -p cic-tools -- map maps\synthetic\synthetic.map path\to\maps.big
cargo run -p cic-tools -- map-height maps\synthetic\synthetic.map path\to\maps.big
cargo run -p cic-tools -- map-height --report maps\synthetic\synthetic.map path\to\maps.big
cargo run -p cic-tools -- map-blend maps\synthetic\blend.map path\to\maps.big
cargo run -p cic-tools -- map-render --size 768 maps\synthetic\blend.map path\to\maps.big path\to\terrain-resources
cargo run -p cic-render --example headless_capture -- target/synthetic-capture.ppm
```

Complete W3D models can be exported to glTF 2.0 for Blender or a browser-based model
viewer. On Windows, installed Steam locations are detected automatically; Generals is the
default resource profile and `--zh` layers Zero Hour over its required Generals base:

```powershell
cargo run -p cic-tools -- config show
cargo run -p cic-tools -- w3d-export art/w3d/model.w3d
cargo run -p cic-tools -- w3d-view art/w3d/model.w3d
cargo run -p cic-tools -- w3d-render --animation 0 --frame 10 --time 0.5 art/w3d/model.w3d model-capture.ppm
cargo run -p cic-tools -- --zh w3d-export art/w3d/model_skn.w3d custom-name.glb
cargo run -p cic-tools -- w3d-export --gltf art/w3d/model.w3d preview.gltf
```

With no output argument, the resource basename determines the result: `model.w3d` becomes
`model.glb`, or `model.gltf` with `--gltf`. An explicit output path overrides that name.
GLB is one self-contained file; `--gltf` instead writes JSON, an external `.bin`, and PNG
images beneath a sibling `_textures` directory. The exporter composes HLOD
meshes, hierarchy transforms, skins, and raw or compressed animation clips, including retail
layouts that split `_SKN`, `_SKL`, and animation W3Ds. Pass-zero/stage-zero colors, shaders,
textures, and UVs drive the visible core-glTF preview; versioned mesh extras preserve every W3D
pass, stage, mapper, shader, and animated-texture descriptor for inspection and later renderer
ingestion. W3D `.tga` references may resolve to installed `.dds` replacements. Source images
preserve decoded RGBA texels and are explicitly tagged sRGB in PNG output. Additive `ONE + ONE`
materials use a separate derived alpha-coverage image in the core-glTF preview so black sprite
backgrounds remain invisible without changing the packaged source image. A missing retail image
produces a visible magenta placeholder
and warning instead of preventing geometry inspection.

Use `--game-dir <path>` for a one-off installation or persist roots explicitly:

```powershell
cargo run -p cic-tools -- config set generals-dir "D:\Games\Generals"
cargo run -p cic-tools -- config set zero-hour-dir "D:\Games\Zero Hour"
```

Explicit directory or BIG mounts remain supported after the command arguments for
synthetic fixtures and custom overlays.

Custom bases and total conversions can declare arbitrary ordered providers in a bounded profile:

```text
version=1
mount=base.assets
optional=loose-overrides
```

Paths are relative to the profile unless absolute. `mount` is required at launch; a missing
`optional` provider is skipped. Repeatable mod layers are appended in command-line order:

```text
cargo run -p cic-tools -- --profile custom.cic-profile --mod mods/first --mod mods/second manifest
```

Built-in Generals/Zero Hour archive lists are convenience presets only; custom profiles do not
require retail filenames or sentinels. Disk mounts retain directory/BIG indices rather than
payloads. A resource is read only when selected, under the consuming parser's explicit size limit.

The renderer boundary can produce a window-free synthetic PPM and RGBA SHA-256 with an explicit
pose. It consumes validated `cic-formats` values and owns no parser, filesystem, or simulation
resources. `cic-inspect w3d-view` opens a 960x720 depth-tested viewer, frames the model from a
45-degree elevated camera, rotates it around W3D's Z-up axis, and plays the selected animation.
Framing is computed once per selected clip, so animation frames do not recenter or rescale the
model. All decoded passes/stages are submitted in stable order: each pass uses its decoded preview
blend and later texture stages multiply the accumulated color. Temporal UV mappers use explicit
elapsed seconds. A bounded resource manager deduplicates decoded images by RGBA content and reuses
effective GPU materials across meshes. Left/Right switch clips and Escape closes the window; the
title shows the active clip.
`cic-inspect w3d-render` connects that boundary to the existing installed-resource profiles or
explicit BIG mounts and produces the same textured material preview without a window. Animation
index/frame, mapper seconds, and rotation are explicit command arguments, so its RGBA hash is a
deterministic diagnostic rather than a wall-clock snapshot.

On Windows, Rust's MSVC target also requires Visual Studio Build Tools with the Desktop
development with C++ workload. The same checks run on Linux in GitHub Actions.

Directories and BIG archives are mounted from left to right. Later mounts override
earlier mounts. Archive backslashes and host separators are normalized; manifests always
emit portable `/` virtual paths.
No retail game assets are included in this repository.

The current R3 terrain gate inventories MAP chunks, decodes immutable height and version-6/7 blend
values, resolves semantic terrain classes through mounted Terrain INI definitions, and stages
source-scaled layered terrain for a deterministic headless capture:

```powershell
cargo run -p cic-tools -- map-height "maps/synthetic/synthetic.map"
cargo run -p cic-tools -- map-height --report "maps/synthetic/synthetic.map"
cargo run -p cic-tools -- map-render --size 768 "maps/synthetic/synthetic.map"
cargo run -p cic-tools -- map-view "maps/synthetic/synthetic.map"
cargo run -p cic-tools -- map-water "maps/synthetic/synthetic.map"
```

The first command derives `synthetic.png`; the terrain render derives `synthetic-terrain.png`.
Explicit output paths and directory/BIG mounts remain supported. `map-view` opens a perspective
flyover: WASD moves, Space/Ctrl changes altitude, Shift boosts, right-drag looks, the wheel moves
forward/back, R resets, and Escape closes. Terrain rendering defaults to the source-compatible
`--terrain-policy legacy`; `modern` keeps stored cliff mappings, disables implicit steep-slope
retiling, and adds world-anchored macro variation. Custom edge classes render through a separately
indexed overlay pass. The viewer resolves opaque terrain through a G-buffer, then draws decoded
water polygons in a depth-aware forward pass. It keeps the deterministic 8-pixel background and
uses a persistent GPU-composed virtual-texture cache for camera-space-depth-capped 16- and
32-texel pages. Fixed-size bordered pages retain authored base/primary/extra blends, cliff UVs,
custom edges, and Modern macro variation; projected viewport ranking preserves coarse visible
coverage before fine upgrades, while an LRU page table reuses revisited regions without CPU
texture rebakes.
GPU-generated linear mip chains, trilinear filtering, and up to 16x anisotropy keep terrain stable
across movement and pitch changes.
Installed profiles resolve bounded source caustic frames and water-transparency
depth into renderer-neutral appearance inputs. The shader projects the subtle animation onto the
underwater bed and combines it with depth absorption and shallow shoreline effects.

See [CURRENT.md](CURRENT.md) for the active objective and [ROADMAP.md](ROADMAP.md) for
completion gates.
