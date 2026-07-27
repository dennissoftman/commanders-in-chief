# Commanders in Chief

An open-source real-time strategy engine, written in Rust, using its own asset formats.

A player commands a faction on a heightfield map: gather resources, construct a base, produce units,
and fight other players or the AI in real time. Classic base-building RTS, built from scratch.

This is not a reimplementation of, or a compatibility layer for, any existing game. Nothing here reads
another game's data or derives from another game's code.

## Status

Early. The foundation, resource layer, and asset formats are complete; the renderer is in progress.
There is no playable game yet. See [ROADMAP.md](ROADMAP.md) for the milestone ladder and
[CURRENT.md](CURRENT.md) for what is being worked on now.

## Building

Requires the pinned toolchain in `rust-toolchain.toml`, which `rustup` installs automatically.

```bash
cargo test --workspace
```

The full gate, which is what CI runs:

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

## Crates

| Crate | Purpose |
|---|---|
| `cic-core` | Bounded binary reading with structured errors. No dependencies. |
| `cic-vfs` | Virtual paths, ordered mounts, overlay resolution, zip and tar containers. |
| `cic-assets` | glTF model import, the terrain container, JSON scenarios, map packages. |
| `cic-camera` | The RTS camera model, free of window, input, and GPU dependencies. |
| `cic-render` | WGSL shader set, terrain page residency, texture resources. |

## Asset formats

| Data | Format |
|---|---|
| Models, props, units | glTF 2.0 (`.glb`) |
| Terrain heightfield and layers | Custom chunked binary (`.cict`) |
| Scenario | JSON (`map.json`) |
| A whole map | zip (`.cicmap`) |

The reasoning behind each choice is in [docs/milestones/m2-assets.md](docs/milestones/m2-assets.md).
The short version: authored data a human edits and reviews should be text, bulk numeric data should be
tight binary, and geometry should use a standard that content tools already export.

## Engineering standards

Two documents that later work is measured against, both of them load-bearing rather than aspirational:

- [Binary parsing invariants](docs/invariants/binary-parsing.md) — every decoder treats its input as
  hostile, takes explicit limits, refuses before allocating, and cannot panic.
- [Determinism invariants](docs/invariants/determinism.md) — lockstep multiplayer, replays, and desync
  diagnosis all reduce to the same requirement, and it cannot be retrofitted.

`unsafe_code` is forbidden at workspace scope. Strict lints are errors in CI.

## Licence

Not yet chosen. See [LICENSING.md](LICENSING.md) — this is a deliberate open decision, not an
oversight, and it needs settling before the first public release.
