# Roadmap

Milestones are named for the **engine capability** they deliver, not for a file format or a
compatibility target. Each one is defined by what becomes possible when it lands, and each states its
own exit condition so "done" is a fact rather than an opinion.

**A number is an identifier, not a position in a queue.** It is assigned when a milestone is chartered and
never changes, because renumbering would break every inbound link and every reference in the commit
history. What orders the work is the *depends on* column, which is why it exists: M0 through M8 happened to
be chartered in dependency order, and M9 was not.

| Milestone | Capability | Depends on | Status |
|---|---|---|---|
| [M0](docs/milestones/m0-foundation.md) | Foundation: workspace, invariants, gate | — | Complete |
| [M1](docs/milestones/m1-resources.md) | Resource layer: mounts, overlays, containers | M0 | Complete |
| [M2](docs/milestones/m2-assets.md) | Asset formats: models, terrain, scenarios | M1 | Complete (read path) |
| [M3](docs/milestones/m3-renderer.md) | Renderer: terrain, models, water, lighting, presentation | M2 | Exit condition met — terrain level of detail outstanding |
| [M4](docs/milestones/m4-interface.md) | Interface: layout, widgets, shell | M3 | **Complete** |
| [M5](docs/milestones/m5-simulation.md) | Simulation: deterministic fixed-tick kernel | M2 | Planned |
| [M6](docs/milestones/m6-gameplay.md) | Gameplay: units, orders, combat, economy | M5, M9 | Planned |
| [M7](docs/milestones/m7-network.md) | Network: lockstep, replay, desync diagnosis | M5 | Planned |
| [M8](docs/milestones/m8-tooling.md) | Tooling: map editor, asset pipeline | M2, M4 | Planned |
| [M9](docs/milestones/m9-audio.md) | Audio: mixer, spatialisation, DSP, music, cues | M1 | Charter met — device layer outstanding |

M9 sits early in the dependency order and was chartered late, which is the honest reason the ladder is now
a graph rather than a line. It never needed a renderer or a kernel — audio needs bytes from the resource
layer and nothing else. It is something M6 will need in place before it starts, and it would have been
expensive to retrofit, because the backend boundary is a shape rather than a feature.

## Why the milestones read as mechanics

A milestone describes an engine capability — a heightfield that renders, a kernel that ticks
deterministically, a lockstep session that replays, a cue that plays where the unit is — because that is
what can be finished and tested. The
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
