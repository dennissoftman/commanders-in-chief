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

Static W3D meshes can be exported to Wavefront OBJ for a quick geometry preview. First use
the `w3d` report to find a top-level mesh index, then export that mesh:

```powershell
cargo run -p cic-tools -- w3d art/w3d/model.w3d path\to\W3D.big
cargo run -p cic-tools -- w3d-obj art/w3d/model.w3d 2 preview.obj path\to\W3D.big
```

The OBJ preserves object-space coordinates, vertex normals, triangle order, and winding.
When a first-pass diffuse material or DCG color array is present, normalized vertex colors
are appended to each `v` record. Texture coordinates and texture images remain deferred.

On Windows, Rust's MSVC target also requires Visual Studio Build Tools with the Desktop
development with C++ workload. The same checks run on Linux in GitHub Actions.

Directories and BIG archives are mounted from left to right. Later mounts override
earlier mounts. Archive backslashes and host separators are normalized; manifests always
emit portable `/` virtual paths.
No retail game assets are included in this repository.

See [CURRENT.md](CURRENT.md) for the active objective and [ROADMAP.md](ROADMAP.md) for
completion gates.
