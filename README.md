# Commanders in Chief

Commanders in Chief is a GPL-licensed, cross-platform compatibility engine project for
classic SAGE-era real-time strategy game data. It is an independent community project
and is not affiliated with or endorsed by Electronic Arts.

The first executable milestone is a deterministic resource inspector. The current code
provides bounded binary input, normalized virtual paths, deterministic overlay handling,
and a CLI that inventories mounted directories.

```powershell
cargo test --workspace
cargo run -p cic-tools -- manifest path\to\base path\to\override
```

On Windows, Rust's MSVC target also requires Visual Studio Build Tools with the Desktop
development with C++ workload. The same checks run on Linux in GitHub Actions.

Directories are mounted from left to right. Later mounts override earlier mounts.
No retail game assets are included in this repository.

See [CURRENT.md](CURRENT.md) for the active objective and [ROADMAP.md](ROADMAP.md) for
completion gates.
