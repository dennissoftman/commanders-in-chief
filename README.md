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
```

Complete W3D models can be exported to glTF 2.0 for Blender or a browser-based model
viewer. On Windows, installed Steam locations are detected automatically; Generals is the
default resource profile and `--zh` layers Zero Hour over its required Generals base:

```powershell
cargo run -p cic-tools -- config show
cargo run -p cic-tools -- w3d-gltf art/w3d/model.w3d preview.gltf
cargo run -p cic-tools -- --zh w3d-gltf art/w3d/model_skn.w3d preview.gltf
```

The command writes `preview.gltf`, `preview.bin`, and PNG images beneath
`preview_textures`. It composes HLOD meshes, hierarchy transforms, skins, and classic raw
animation clips, including retail layouts that split `_SKN`, `_SKL`, and animation W3Ds.
First-pass colors, shaders, textures, and UVs are preserved for preview; W3D `.tga`
references may resolve to installed `.dds` replacements. A missing retail image produces
a visible magenta placeholder and warning instead of preventing geometry inspection.

Use `--game-dir <path>` for a one-off installation or persist roots explicitly:

```powershell
cargo run -p cic-tools -- config set generals-dir "D:\Games\Generals"
cargo run -p cic-tools -- config set zero-hour-dir "D:\Games\Zero Hour"
```

Explicit directory or BIG mounts remain supported after the command arguments for
synthetic fixtures and custom overlays.

On Windows, Rust's MSVC target also requires Visual Studio Build Tools with the Desktop
development with C++ workload. The same checks run on Linux in GitHub Actions.

Directories and BIG archives are mounted from left to right. Later mounts override
earlier mounts. Archive backslashes and host separators are normalized; manifests always
emit portable `/` virtual paths.
No retail game assets are included in this repository.

See [CURRENT.md](CURRENT.md) for the active objective and [ROADMAP.md](ROADMAP.md) for
completion gates.
