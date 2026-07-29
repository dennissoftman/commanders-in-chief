# Roadmap

Milestones are named for the **engine capability** they deliver, not for a file format or a
compatibility target. Each one is defined by what becomes possible when it lands, and each states its
own exit condition so "done" is a fact rather than an opinion.

Milestones are ordered by dependency, not by priority. M5 cannot be built on a renderer that does not
exist, but nothing here forbids work starting early on a later milestone's design.

| Milestone | Capability | Status |
|---|---|---|
| [M0](docs/milestones/m0-foundation.md) | Foundation: workspace, invariants, gate | Complete |
| [M1](docs/milestones/m1-resources.md) | Resource layer: mounts, overlays, containers | Complete |
| [M2](docs/milestones/m2-assets.md) | Asset formats: models, terrain, scenarios | Complete (read path) |
| [M3](docs/milestones/m3-renderer.md) | Renderer: terrain, models, water, lighting, presentation | Exit condition met — terrain level of detail outstanding |
| [M4](docs/milestones/m4-interface.md) | Interface: layout, widgets, shell | **Complete** |
| [M5](docs/milestones/m5-simulation.md) | Simulation: deterministic fixed-tick kernel | Planned |
| [M6](docs/milestones/m6-gameplay.md) | Gameplay: units, orders, combat, economy | Planned |
| [M7](docs/milestones/m7-network.md) | Network: lockstep, replay, desync diagnosis | Planned |
| [M8](docs/milestones/m8-tooling.md) | Tooling: map editor, asset pipeline | Planned |

## Why the milestones read as mechanics

A milestone describes an engine capability — a heightfield that renders, a kernel that ticks
deterministically, a lockstep session that replays — because that is what can be finished and tested. The
game those capabilities add up to is described in [the README](README.md#the-game) and specified in the
[faction bible](docs/design/faction-bible.md), which is upstream of several entries here rather than
downstream of them: M3's writable terrain textures exist because a faction paves the map.

Everything is implemented from scratch in the project's own formats. This is not a reimplementation of or
a compatibility layer for any existing game, and nothing here reads another game's data or derives from
another game's code.

## What "complete" means for a milestone

Three things, all of them checkable:

1. Every capability the milestone claims is exercised by a test in CI.
2. The full gate passes: formatting, strict lints, and the whole test suite.
3. The milestone document records what was *not* done and why, so a later reader does not mistake a
   deliberate omission for an oversight.
