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
cargo run -p cic-render --example headless_capture -- target/synthetic-capture.ppm
```

Complete W3D models can be exported to glTF 2.0 for Blender or a browser-based model
viewer. On Windows, installed Steam locations are detected automatically; Generals is the
default resource profile and `--zh` layers Zero Hour over its required Generals base:

```powershell
cargo run -p cic-tools -- config show
cargo run -p cic-tools -- w3d-export art/w3d/model.w3d
cargo run -p cic-tools -- w3d-view art/w3d/model.w3d
cargo run -p cic-tools -- w3d-render art/w3d/model.w3d model-capture.ppm
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

The renderer boundary can produce a window-free synthetic PPM and RGBA SHA-256 with an explicit
pose. It consumes validated `cic-formats` values and owns no parser, filesystem, or simulation
resources. `cic-inspect w3d-view` opens a 960x720 depth-tested viewer, frames the model from a
45-degree elevated camera, rotates it around W3D's Z-up axis, and plays the selected animation.
Framing is computed once per selected clip, so animation frames do not recenter or rescale the
model. Pass-zero/stage-zero textures, UVs, source alpha, alpha testing, and common alpha/additive
blend modes are rendered directly. A bounded resource manager deduplicates decoded images by RGBA
content and reuses effective GPU materials across meshes. Left/Right switch clips and Escape closes
the window; the title shows the active clip.
`cic-inspect w3d-render` connects that boundary to the existing installed-resource profiles or
explicit BIG mounts and produces a depth-tested bind-pose geometry diagnostic. Textures and exact
fixed-function material passes are not yet applied by the headless command.

On Windows, Rust's MSVC target also requires Visual Studio Build Tools with the Desktop
development with C++ workload. The same checks run on Linux in GitHub Actions.

Directories and BIG archives are mounted from left to right. Later mounts override
earlier mounts. Archive backslashes and host separators are normalized; manifests always
emit portable `/` virtual paths.
No retail game assets are included in this repository.

See [CURRENT.md](CURRENT.md) for the active objective and [ROADMAP.md](ROADMAP.md) for
completion gates.
