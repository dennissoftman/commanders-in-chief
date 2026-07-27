# M2: Asset formats

Decide what a model, a terrain, a scenario, and a map physically are, and be able to read all four.

**Status:** Complete on the read path. Writing is complete for terrain and scenarios, absent for
models and packages.

## Charter

- Choose a format per kind of data, on the merits of that data rather than for uniformity.
- Decode each one under explicit limits, refusing rather than allocating.
- Cross-check between formats where neither can validate alone.

## The decisions

| Data | Format | Why |
|---|---|---|
| Models, props, units | glTF 2.0 (`.glb`) | A published standard every content tool already exports. A custom mesh format would mean writing an exporter before anyone could author an asset. |
| Terrain heightfield and layers | Custom chunked binary (`.cict`) | A regular numeric grid, which no standard describes well. `u16` elevations upload directly as a baseline `R16Uint` GPU texture. |
| Scenario: placements, players, waypoints | JSON (`map.json`) | Diffable, reviewable, hand-fixable. The bulk numerics live in the terrain container, so the size argument for a binary encoding does not apply. |
| A whole map | zip (`.cicmap`) | Already has a directory, per-member compression, and universal tooling. |

The split is the point. Authored data that a human edits and reviews wants to be text; bulk numeric
data that only a machine reads wants to be tight binary; geometry wants a standard. One format for all
three would be wrong for two of them.

## Exit condition

Met for reading. Terrain round-trips through its container including a forward-compatibility case for
unknown chunks; scenarios round-trip through JSON and reject unknown fields; models import from real
`.glb` fixtures including transform flattening and the spec's flat-normal fallback; packages resolve
both halves and cross-check that authored positions lie inside the terrain's extent.

## Design notes

`u16` elevations rather than `f32`: this halves the heightfield, and — the load-bearing reason — a
16-bit integer texture (`R16Uint`) is baseline-supported, so the payload uploads as a height texture
byte-for-byte with no conversion pass. Note the integer rather than the normalized form: `R16Unorm`
needs an optional device feature, and heights are only ever loaded, never filtered.

Scenario JSON **rejects** unknown fields rather than ignoring them. A typo in a hand-edited map is
then a loud error at load rather than a silently-defaulted value that surfaces later as a gameplay
bug.

The package layer performs the check neither format can make alone: the scenario knows where things
are, the terrain knows how large the world is, and only the package sees both. A unit authored outside
the map becomes a load-time error instead of a unit spawning in the void.

## Explicitly not done

- **Skinned and animated model import.** The importer detects that a skin or animation exists and
  reports it, then imports bind-pose geometry. Reporting rather than silently dropping the rig is the
  deliberate part; consuming it belongs with the renderer that can play it.
- **Texture decoding.** Model materials record which image index they reference; turning that into
  pixels is the renderer's concern.
- **Model and package writers.** Needed by the editor in M8, not by the engine.
- **Object and faction templates.** The scenario references templates by identifier; defining the
  template format itself waits until M6 knows what a unit needs.
