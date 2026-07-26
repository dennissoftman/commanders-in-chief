# Commanders in Chief

**A cross-platform compatibility engine and inspection toolkit for classic SAGE-era real-time
strategy game data.**

Commanders in Chief decodes the archives, maps, models, text, and menu layouts of a game
installation you already own, and presents them through deterministic reports, deterministic
image captures, and interactive `wgpu` viewers.

> [!IMPORTANT]
> This is an independent community project licensed under **GPL-3.0-only**. It is not affiliated
> with or endorsed by Electronic Arts. **No retail game assets are included in this repository**,
> and none are distributed with it — every example below reads data from your own installation or
> from original synthetic fixtures.

## Contents

- [Project status](#project-status)
- [Requirements](#requirements)
- [Quick start](#quick-start)
- [Command reference](#command-reference)
- [Resources, profiles, and mods](#resources-profiles-and-mods)
- [Maps and terrain scenes](#maps-and-terrain-scenes)
- [Models](#models)
- [User interface](#user-interface)
- [Design guarantees](#design-guarantees)
- [Repository layout](#repository-layout)
- [Documentation map](#documentation-map)
- [License and provenance](#license-and-provenance)

## Project status

Progress is measured by compatibility gates, not elapsed time. The table below is a summary;
[ROADMAP.md](ROADMAP.md) is the authoritative status index and each milestone's charter,
progress, and completion evidence live in `docs/milestones/`.

| Milestone | What it delivers | Status |
| --- | --- | --- |
| R0 | Workspace, bounded reader, normalized VFS, mount profiles, manifest CLI | Complete |
| R1 | BIG archive mounting and complete CSF decoding | In progress (`BIG4` retail verification open) |
| R2 | Bounded W3D decoding, animated viewer, glTF 2.0 export | Complete |
| R3 | Complete MAP ingestion and the non-simulating terrain scene | Complete |
| R4 | WND decoding and a navigable main-menu / skirmish shell | **Active** |
| R5 | Deterministic simulation kernel | Planned |
| R6 | Navigation analysis and one gameplay slice | Planned |

There is **no playable game yet.** R4 is presentation-only: nothing activates players, constructs
gameplay objects, or executes map scripts. That begins at the R5 simulation boundary.

See [CURRENT.md](CURRENT.md) for the active objective and the next verified step, and
[CHANGELOG.md](CHANGELOG.md) for user-visible changes.

## Requirements

| Requirement | Notes |
| --- | --- |
| Rust `1.93.0` | Pinned by [rust-toolchain.toml](rust-toolchain.toml); `rustup` installs it automatically |
| A GPU with a working `wgpu` backend | Needed only by the render, view, and capture commands |
| Visual Studio Build Tools (Windows) | Rust's MSVC target needs the *Desktop development with C++* workload |
| An installed copy of the game | Optional — the test suite and CI run entirely on synthetic fixtures |

The same `fmt` / `clippy` / `test` checks run on Linux in GitHub Actions.

## Quick start

### 1. Build and verify

```powershell
cargo test --workspace
```

Contributions must also pass the checks CI enforces:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

### 2. Point the tools at an installation

On Windows, installed Steam locations are detected automatically. Verify what was found:

```powershell
cargo run -p cic-tools -- config show
```

If detection misses your install, persist the roots explicitly:

```powershell
cargo run -p cic-tools -- config set generals-dir "D:\Games\Generals"
cargo run -p cic-tools -- config set zero-hour-dir "D:\Games\Zero Hour"
```

Use `--game-dir <path>` instead for a one-off run. Generals is the default resource profile;
`--zh` layers Zero Hour over its required Generals base.

### 3. Inspect something

```powershell
cargo run -p cic-tools -- manifest
cargo run -p cic-tools -- w3d-export art/w3d/model.w3d
cargo run -p cic-tools -- map-view "maps/synthetic/synthetic.map"
cargo run -p cic-tools -- map-view --output scene.png "maps/synthetic/synthetic.map"
```

The binary installs as `cic-inspect`, so `cargo run -p cic-tools -- <command>` and
`cic-inspect <command>` are equivalent. Explicit directory or BIG mounts may follow the command
arguments at any time, which is how synthetic fixtures and custom overlays are inspected:

```powershell
cargo run -p cic-tools -- manifest path\to\base path\to\archive.big path\to\override
cargo run -p cic-tools -- map-height maps\synthetic\synthetic.map path\to\maps.big
```

Reports are stable, tab-separated text and image captures are accompanied by an RGBA SHA-256, so
both are safe to diff between runs.

## Command reference

All commands accept the global options `[--zh] [--game-dir <path>] [--profile <profile>]
[--mod <mount>]...` before the command name, and trailing `<mount>...` arguments after it.

### Installation and configuration

| Command | Purpose |
| --- | --- |
| `config show` | Report detected and stored installation roots |
| `config set <key> <path>` | Persist a root; keys are `generals-dir`, `zero-hour-dir`, `generals-options-ini`, `zero-hour-options-ini` |
| `options` | Decode the user's `Options.ini` preferences; `--options-ini <path>` overrides discovery |

### Resources and text

| Command | Purpose |
| --- | --- |
| `manifest` | Inventory every resource the mounted providers expose, with provenance |
| `csf <path>` | Decode CSF localization labels, strings, and wave names |

### Maps

| Command | Purpose |
| --- | --- |
| `map <path>` | MAP chunk inventory |
| `map-height [--report \| --png <out.png>] <path>` | Write a heightmap PNG, or print the height report with `--report` |
| `map-blend <path>` | Version 6/7/8 terrain blend layers |
| `map-lighting <path>` | Per-time-of-day lighting and shadow records |
| `map-water <path>` | Decoded water areas |
| `map-polygons <path>` | `PolygonTriggers` areas, versions 2 through 4 |
| `map-objects <path>` | `ObjectsList` placements as immutable data |
| `map-sides <path>` | Sides, teams, build lists, and the complete nested script tree |
| `map-view <path>` | Interactive real-time-strategy view of the staged scene |
| `map-view --output <out.png\|out.ppm> <path>` | The same scene rendered once offscreen, plus its RGBA hash |

`map-view` takes `[--modern]`, `[--pixels-per-cell <pixels>]`, `[--terrain-policy <legacy\|modern>]`,
`[--time <seconds>]` in either mode, plus `[--yaw <degrees>]`, `[--height <units>]`,
`[--focus <x>,<y>]`, and `[--overlays <on|off>]` to place and frame the view, `[--size <pixels\|WxH>]`
for the capture target, and `[--shadows <on|off>]` / `[--occlusion <on|off>]` to isolate a single
lighting contribution.

### Models

| Command | Purpose |
| --- | --- |
| `w3d <path>` | Recursive W3D chunk inventory with unknown-payload preservation |
| `w3d-mesh <path> <top-level-index>` | Report one top-level mesh |
| `w3d-export [--gltf] <path> [<out.glb\|out.gltf>]` | Export to glTF 2.0 |
| `w3d-view <path>` | Interactive animated model viewer |
| `w3d-render <path> [<out.ppm>]` | Deterministic textured capture plus RGBA hash |

`w3d-render` takes `[--animation <index>] [--frame <frame>] [--time <seconds>]
[--rotation <radians>]`.

### User interface

| Command | Purpose |
| --- | --- |
| `wnd <path>` | Source-order inventory of the decoded WND hierarchy, fields, and diagnostics |
| `wnd-render <path> [<out.ppm>]` | Surface-free capture of every window rectangle plus RGBA hash |

### Renderer boundary

The renderer boundary can be exercised on its own, with no parser, filesystem, or simulation
resources involved:

```powershell
cargo run -p cic-render --example headless_capture -- target/synthetic-capture.ppm
```

It produces a window-free PPM and RGBA SHA-256 from an explicit pose.

## Resources, profiles, and mods

### Mount order

Directories and BIG archives are mounted **from left to right, and later mounts override earlier
mounts**. Archive backslashes and host separators are normalized, and manifests always emit
portable `/` virtual paths.

Disk mounts retain directory and BIG *indices* rather than payloads. A resource is read only when
it is selected, under the consuming parser's explicit size limit.

### Zero Hour layering contract

Zero Hour is treated as a delta over Generals, never as a standalone resource set. The built-in
`--zh` profile enumerates and mounts the required Generals providers first, then the Zero Hour
providers, then explicit mod layers.

Consumers must distinguish two resource behaviors:

- **Replacement resources** — a single MAP, image, or W3D — resolve to the last mounted
  provider.
- **Cumulative definition resources** — such as partial INI registries — parse *every* provider
  version from earliest to latest and apply their format-specific override semantics.

Using only the winning INI would discard base definitions that Zero Hour expects to inherit. New
definition consumers must therefore use VFS provider history explicitly and include a synthetic
base-definition / partial-overlay test. The decision is permanent and recorded in
[ADR 0003](docs/adr/0003-gltf-resource-profiles-and-model-composition.md); the VFS mechanics are in
[ADR 0008](docs/adr/0008-lazy-vfs-and-mod-mount-profiles.md).

### Custom profiles

Custom bases and total conversions declare arbitrary ordered providers in a bounded profile file:

```text
version=1
mount=base.assets
optional=loose-overrides
```

Paths are relative to the profile unless absolute. `mount` is required at launch; a missing
`optional` provider is skipped. Repeatable mod layers are appended in command-line order:

```powershell
cargo run -p cic-tools -- --profile custom.cic-profile --mod mods/first --mod mods/second manifest
```

Built-in Generals / Zero Hour archive lists are convenience presets only. Custom profiles do not
require retail filenames or sentinels.

## Maps and terrain scenes

R3 inventories MAP chunks and decodes immutable height, version 6/7/8 blend, complete polygon,
object, side/team, and script data; resolves the referenced presentation resources; and stages a
deterministic, non-simulating scene.

```powershell
cargo run -p cic-tools -- map-height "maps/synthetic/synthetic.map"
cargo run -p cic-tools -- map-height --report "maps/synthetic/synthetic.map"
cargo run -p cic-tools -- map-view "maps/synthetic/synthetic.map"
cargo run -p cic-tools -- map-view --output scene.png "maps/synthetic/synthetic.map"
cargo run -p cic-tools -- map-polygons "maps/synthetic/synthetic.map"
cargo run -p cic-tools -- map-objects "maps/synthetic/synthetic.map"
cargo run -p cic-tools -- map-sides "maps/synthetic/synthetic.map"
```

### Output naming

| Command | Derived output |
| --- | --- |
| `map-height` | `synthetic.png` |

`map-view --output` has no derived name; the capture path is always explicit, and its extension
selects PNG or PPM. `map-height` writes the heightmap PNG by default and only prints the text report when `--report` is
passed; `--report` and `--png` are mutually exclusive. Explicit output paths always override the
derived name.

### What the staged scene contains

`map-view` has one renderer, whether it opens a window or writes a file. There is no separate
thumbnail path to drift out of agreement with what the window shows.

**The staged scene.** Terrain with authored blend layers and cliff mappings,
roads with legacy-radius curves, miters, and junctions, bridges resolved from paired endpoints,
instanced static scenery, water with caustics and depth absorption, source-default vegetation sway,
and renderer-only diagnostics for player-start candidates, waypoints, named waypoint paths, polygon
zones, and the playable boundary. Terrain, roads, custom edges, and static scenery rasterize into a
4x multisampled G-buffer; water and the boundary follow as depth-aware forward passes, and the
composite finishes with a bounded contrast-adaptive sharpen. Terrain detail is served from a
persistent GPU-composed virtual-texture cache with GPU-generated mip chains, trilinear filtering,
and up to 16x anisotropy over the deterministic 8-pixel-per-cell baked background.

**`--output` is that same render, once, offscreen.** It draws the shadow cascade passes, the
multisampled G-buffer, occlusion, deferred lighting, and the composite exactly as the window does,
into a texture it copies back instead of a surface it presents. So a capture is evidence about what
`map-view` shows, not an approximation of it, and a lighting or shadow change can be judged from a
file rather than from a screenshot of a window.

Every input to a capture is explicit — target size, camera placement, presentation time, and which
lighting contributions run — and nothing in the path reads a clock or an RNG, so identical inputs
reproduce an identical image and the RGBA SHA-256 moves only when a tuning change moves it.
`--shadows off` and `--occlusion off` leave their target at the neutral clear value the pass already
writes, so differencing two captures attributes a suspicious region to a specific term. Ground
placements sample the exact rendered terrain triangle and add the MAP-authored relative Z offset
verbatim, without clamping or an added epsilon.

Full detail lives in [docs/formats/map.md](docs/formats/map.md) and
[docs/milestones/r3-map-scene.md](docs/milestones/r3-map-scene.md).

### Terrain policies

`--modern` takes no value and selects every project-authored presentation policy at once — terrain
and water follow the same switch — so it is `--terrain-policy modern` under a shorter name.

| `--terrain-policy` | Behavior |
| --- | --- |
| `legacy` (default) | Source-compatible terrain rendering |
| `modern` | Keeps stored cliff mappings, disables implicit steep-slope retiling, and adds world-anchored macro variation |

Exact legacy fixed-function pixel equivalence is outside the compatibility claim.

### Flyover controls

| Input | Action |
| --- | --- |
| `W` `A` `S` `D` | Move |
| `Space` / `Ctrl` | Change altitude |
| `Shift` | Boost |
| Right-drag | Look |
| Mouse wheel | Move forward / back |
| `R` | Reset camera |
| `M` | Toggle full-scene wireframe, where the GPU supports polygon-line mode |
| `Esc` | Close |

## Models

Complete W3D models export to glTF 2.0 for Blender or a browser-based model viewer:

```powershell
cargo run -p cic-tools -- w3d-export art/w3d/model.w3d
cargo run -p cic-tools -- w3d-export --gltf art/w3d/model.w3d preview.gltf
cargo run -p cic-tools -- --zh w3d-export art/w3d/model_skn.w3d custom-name.glb
cargo run -p cic-tools -- w3d-view art/w3d/model.w3d
cargo run -p cic-tools -- w3d-render --animation 0 --frame 10 --time 0.5 art/w3d/model.w3d model-capture.ppm
```

### Export format

With no output argument the resource basename decides the result: `model.w3d` becomes `model.glb`,
or `model.gltf` with `--gltf`. An explicit output path overrides that name.

| Format | Output |
| --- | --- |
| GLB (default) | One self-contained file |
| `--gltf` | JSON, an external `.bin`, and PNG images under a sibling `_textures` directory |

### What the exporter composes

- HLOD meshes, hierarchy transforms, skins, and raw or compressed animation clips — including
  retail layouts that split `_SKN`, `_SKL`, and animation W3Ds into sibling resources.
- Pass-zero / stage-zero colors, shaders, textures, and UVs drive the visible core-glTF preview.
- Versioned mesh extras preserve **every** W3D pass, stage, mapper, shader, and animated-texture
  descriptor for inspection and later renderer ingestion.

### Texture handling

- W3D `.tga` references may resolve to installed `.dds` replacements.
- Source images preserve decoded RGBA texels and are explicitly tagged sRGB in PNG output.
- Additive `ONE + ONE` materials use a separate derived alpha-coverage image in the core-glTF
  preview, so black sprite backgrounds stay invisible without altering the packaged source image.
- A missing retail image produces a visible magenta placeholder and a warning rather than
  preventing geometry inspection.

### Viewer and capture

`w3d-view` opens a 960x720 depth-tested viewer, frames the model from a 45-degree elevated camera,
rotates it around W3D's Z-up axis, and plays the selected animation. Framing is computed once per
selected clip, so animation frames never recenter or rescale the model. All decoded passes and
stages are submitted in stable order: each pass uses its decoded preview blend and later texture
stages multiply the accumulated color. Temporal UV mappers consume explicit elapsed seconds, and a
bounded resource manager deduplicates decoded images by RGBA content and reuses effective GPU
materials across meshes. `Left` / `Right` switch clips, `Esc` closes, and the title bar shows the
active clip.

`w3d-render` connects the same boundary to installed profiles or explicit BIG mounts and produces
that textured preview without a window. Animation index, frame, mapper seconds, and rotation are
all explicit arguments.

## User interface

R4 is active and adds bounded WND/UI ingestion plus a navigable `wgpu` main-menu and skirmish demo,
so map compatibility can be inspected through the intended shell before simulation exists.

**Available now** (Gate 1):

- A bounded, unknown-preserving WND decoder for file/layout versions, the layout block, and the
  complete `WINDOW` / `CHILD` hierarchy. Unrecognized keywords surface as non-fatal diagnostics
  instead of disappearing.
- `wnd`, a stable source-order inventory report.
- `wnd-render`, a surface-free proof-of-pipeline capture staging each window rectangle as a flat
  colored quad through the existing headless renderer.

**Planned for the rest of R4:** user-owned mapped images, explicit fonts, and CSF labels; the
retained `cic-ui` runtime; main-menu navigation into skirmish setup and map selection with R3
previews and spawn markers; and a bounded declarative WND patch layer that adds modern window-mode,
resolution, refresh-rate, and UI-scale controls with apply/confirm and timeout rollback.

Patches are applied as a pure transformation from one immutable WND definition to another, after
parse and before UI instantiation — the user-owned WND bytes are never modified. Callback names
resolve only through an explicit application-supplied allowlist; unknown names stay inert
diagnostics. Pressing Start produces at most a validated launch description until R5 exists.

The boundary is specified in [ADR 0010](docs/adr/0010-wnd-ui-model-and-wgpu-renderer.md) and
[docs/formats/wnd.md](docs/formats/wnd.md).

## Design guarantees

- **Bounded input.** Every read of untrusted data has explicit limits on counts, allocation sizes,
  offsets, recursion depth, and string lengths, and returns structured errors instead of panicking.
- **Determinism.** The same inputs, mount order, profile, and seed produce the same output. No
  wall-clock time, host filesystem ordering, locale, or platform hash seed reaches deterministic
  output, and determinism is tested at each API boundary.
- **Nothing silently dropped.** Unrecognized fields and payloads are retained as values or
  surfaced as diagnostics rather than discarded.
- **Strict layering.** Parsers return immutable, renderer-neutral values; the VFS exposes bytes
  plus provenance; the renderer owns GPU and window resources but never parsing, VFS, or
  simulation state; tools format diagnostics but contain no parsing rules.
- **Documented provenance.** Source-derived implementations name their source, revision, and
  applicable notices. No retail assets appear in tests, examples, or fixtures.
- **No `unsafe`.** Forbidden workspace-wide.

[ARCHITECTURE.md](ARCHITECTURE.md) has the dependency graph and layer table;
[COMPATIBILITY.md](COMPATIBILITY.md) tracks per-capability status and evidence.

## Repository layout

| Crate | Responsibility |
| --- | --- |
| [`cic-core`](crates/cic-core) | Dependency-free invariants and bounded binary input |
| [`cic-formats`](crates/cic-formats) | Bounded decoders and immutable, renderer-neutral format values |
| [`cic-vfs`](crates/cic-vfs) | Normalized paths, providers, overlay order, and asset provenance |
| [`cic-render`](crates/cic-render) | Model/scene staging, bounded texture resources, deterministic capture, interactive `wgpu` presentation |
| [`cic-tools`](crates/cic-tools) | The `cic-inspect` diagnostic CLI, composing the public VFS, format, and renderer APIs |

R4 adds a narrow `cic-ui` crate for retained UI state, input, and safe navigation. Simulation, AI,
networking, and script *execution* remain excluded until R5.

[`fuzz/`](fuzz) holds libFuzzer targets for `big`, `csf`, `map`, and `water_ini`.

## Documentation map

| Document | Contents |
| --- | --- |
| [CURRENT.md](CURRENT.md) | Active objective, status, and next verified step |
| [ROADMAP.md](ROADMAP.md) | Milestone status index |
| [ARCHITECTURE.md](ARCHITECTURE.md) | Dependency direction, boundaries, and layer ownership |
| [COMPATIBILITY.md](COMPATIBILITY.md) | Per-capability status and verification evidence |
| [CHANGELOG.md](CHANGELOG.md) | User-visible changes, grouped by milestone |
| [AGENTS.md](AGENTS.md) | Contribution rules, required invariants, and change protocol |
| [`docs/milestones/`](docs/milestones) | Each milestone's charter, progress, and completion evidence |
| [`docs/formats/`](docs/formats) | Format specifications for BIG, CSF, W3D, MAP, and WND |
| [`docs/adr/`](docs/adr) | Permanent design decisions |
| [`docs/provenance/`](docs/provenance) | Source evidence for every source-derived format |
| [`docs/invariants/`](docs/invariants) | Binary-parsing and determinism rules |
| [`docs/testing/`](docs/testing) | Test strategy for source-derived behavior |

Every fact has exactly one documentation home; these files link to each other rather than
duplicating content.

## License and provenance

Project-authored code is licensed under the **GNU General Public License, version 3 only**. See
[LICENSE.md](LICENSE.md) and [NOTICE.md](NOTICE.md), and
[ADR 0001](docs/adr/0001-license-and-provenance.md) for the license and provenance policy.

Files derived from third-party GPL sources retain their original copyright, license, attribution,
and applicable GNU GPL Section 7 notices, and record their source and exact revision.
