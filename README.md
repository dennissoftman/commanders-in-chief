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
| `cic-render` | Deferred chain, terrain, instanced models, texturing, windowed presentation, the WGSL shader set. |

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

Design decisions with consequences are recorded as [ADRs](docs/adr/), and what the game *is* — as
distinct from how it is built — lives in [docs/design/](docs/design/README.md). That is not decoration
either: the factions described there are why terrain heights and layer weights are writable GPU
textures rather than a baked mesh, and why a model instance carries its own colour.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Commits need a DCO `Signed-off-by` line (`git commit -s`), and
there is one rule that matters more than the rest: **do not port code, data, or constants from another
game.** The permissive licence below exists only because such a derivation was removed, and a single
ported constant table would undo it.

## Licence

Dual-licensed **MIT OR Apache-2.0**, at your option — see [LICENSE-MIT](LICENSE-MIT) and
[LICENSE-APACHE](LICENSE-APACHE).

Third-party dependency notices are in [NOTICES.md](NOTICES.md); all 281 are permissive.
[LICENSING.md](LICENSING.md) records the provenance audit that made a permissive licence possible, and
the two files still to be written from scratch because of it.
