# Roadmap

Progress is measured by compatibility gates, not elapsed time.

Each milestone's full charter — scope, exclusions, inputs, outputs, owning crates, acceptance
tests, determinism constraints, documentation, progress, and completion evidence — lives in its
own file under `docs/milestones/`. This file is a status index only; progress prose belongs in
the milestone file, and user-visible changes belong in `CHANGELOG.md`.

| Milestone | Status | Charter |
| --- | --- | --- |
| R0: Repository and resource-probe foundation | Complete | [r0-foundation.md](docs/milestones/r0-foundation.md) |
| R1: BIG and CSF resource probe | In progress (`BIG4` retail verification open) | [r1-big-csf.md](docs/milestones/r1-big-csf.md) |
| R2: W3D inspection and viewer | Complete | [r2-w3d-viewer.md](docs/milestones/r2-w3d-viewer.md) |
| R3: Complete MAP ingestion and terrain-scene presentation | Complete (2026-07-23) | [r3-map-scene.md](docs/milestones/r3-map-scene.md) |
| R4: WND user interface and navigable shell | Active | [r4-wnd-shell.md](docs/milestones/r4-wnd-shell.md) |
| R5: Deterministic simulation kernel | Planned | [r5-simulation.md](docs/milestones/r5-simulation.md) |
| R6: Navigation analysis and gameplay slice | Planned | [r6-gameplay-slice.md](docs/milestones/r6-gameplay-slice.md) |

- **R0** established the GPL/provenance policy, Rust workspace, bounded reader, normalized VFS
  paths, lazy providers, declarative mount profiles, deterministic manifest CLI, tests, and CI.
- **R1** covers evidence-backed BIG archive mounting and complete CSF decoding with resource
  provenance; only `BIG4` retail verification remains open.
- **R2** delivered bounded W3D decoding through an animated `wgpu` viewer and glTF export.
- **R3** delivered complete bounded MAP ingestion and the non-simulating terrain, water, road,
  bridge, and static-scenery scene with deterministic overview capture.
- **R4** boundedly decodes the WND grammar and UI resources and presents a navigable
  main-menu/skirmish shell demo without simulation.
- **R5** introduces the deterministic simulation kernel that consumes R3's immutable scenario
  description.
- **R6** derives navigation analysis and completes one build-harvest-combat gameplay slice.
