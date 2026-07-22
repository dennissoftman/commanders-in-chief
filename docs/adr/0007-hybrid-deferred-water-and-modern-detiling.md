# ADR 0007: Hybrid-Deferred Water and Modern De-tiling

- Status: Accepted
- Date: 2026-07-22

## Context

MAP water footprints are water-flagged records inside `PolygonTriggers`, not object or script
records and not a terrain texture layer. Opaque terrain benefits from deferred material and
lighting separation, while transmissive water must read already-lit scene color and terrain depth
or position. Putting transparent water into the opaque G-buffer would lose ordered transmission
and still require a later composition pass.

Repeated terrain tiles also need a Modern-profile treatment that does not rotate roads, cliffs, or
other directional authored content and does not make camera-streamed detail disagree with the
background bake.

## Decision

- `cic-formats` decodes only water-flagged records from established `PolygonTriggers` versions 2
  and 3. It bounds every trigger, name, point array, and retained point total, skips allocation for
  non-water points, preserves degenerate water markers, and rejects truncation and trailing bytes.
- General trigger semantics, script execution, and object loading remain outside this boundary.
- `cic-render` stages lake polygons as stable triangle fans. River records retain perimeter order;
  staging starts around `river_start`, advances one bank and retreats the other with bounded
  wraparound, then emits stable paired strips. Degenerate areas and invalid seams are retained by
  the format value but safely produce no GPU geometry.
- The interactive terrain path is hybrid deferred: opaque terrain and streamed detail write
  albedo, normal/roughness, world position, and depth. Coplanar custom-edge overlays alpha-composite
  only albedo, retaining the base geometry targets. Virtual-page residency is represented
  independently from sampled alpha so authored transparent edge coverage never triggers a
  lower-resolution fallback. A fullscreen directional-light resolve writes linear `RGBA16F` scene
  color; a final surface pass tone maps that scene.
- Water is a later forward, depth-tested, no-depth-write pass under an explicit presentation
  policy. `ZeroHourLegacy` resolves the source standing-water texture, selected diffuse tint/alpha,
  additive choice, texture scale, minimum opacity, and opaque depth; it uses terrain depth to
  feather shoreline coverage and alpha-composites over the resolved scene. `Modern` retains the
  project-authored Beer-Lambert absorption, refraction, Fresnel sky response, directional specular,
  shallow-water effects, and world-projected caustics. Presentation time is explicit viewer input
  and never enters simulation or deterministic headless capture.
- `cic-formats` boundedly decodes the complete source `WaterSet` and `WaterTransparency` tables.
  `cic-render::WaterAppearance` owns validated, VFS-independent opacity, texture, tint, blend,
  policy, and optional caustic-frame values. `cic-tools` resolves user-owned INI and image resources
  and uploads complete mip chains; synthetic and engine callers may provide the same public inputs
  directly or omit optional resources.
- Installed-profile water resolution starts from the source `WaterTransparencySetting` constructor
  values, applies every global INI provider in stable earliest-to-latest mount order, and finally
  applies the MAP's sibling `Map.ini` when present. Degenerate water markers remain data but produce
  no geometry. Mission scripts can describe dynamic water state; R3 preserves that data and never
  executes it before the deterministic R5 simulation boundary.
- Legacy standing-water resource selection, polygon dispatch, 150-world-unit texture scale,
  diffuse alpha, source-over/additive choice, and depth-driven soft-edge intent are derived from
  pinned GPL `W3DWater.cpp`. The project does not copy its Direct3D 8 state machine or shaders.
  Modern shading and the shared `wgpu` render graph remain original project work.
- `Modern` terrain policy applies subtle world-anchored, integer-interpolated macro color
  variation after authored base/primary/extra composition. It never rotates or mirrors tile
  content. `ZeroHourLegacy` output is unchanged. Global cell coordinates make independently baked
  background and near-field regions agree.

## Acceptance and determinism

- Synthetic version-2/version-3 water payloads cover exact closure, all truncated prefixes,
  limits, degenerate markers, nonzero seam wraparound, and stable lake/river triangulation.
- Modern macro variation must reproduce byte-for-byte across repeated staging and between a full
  32-pixel bake and the equivalent streamed detail region.
- An installed user-owned map with one nine-point lake must remain live through GPU upload and the
  water draw; no retail bytes or capture are retained.
- Full formatting, strict Clippy, and workspace tests remain required completion gates.

## Consequences

Water can evolve toward source sky/environment textures, screen-space reflection, planar probes,
clustered lights, and map-specific overrides without coupling format parsing to GPU resources or
recreating the original engine. The legacy standing-water path is now source-driven but remains
WIP pending repeatable visual comparisons; shadows, sky/environment resolution, SSR, and planar
reflection probes remain R3 gates. Headless `map-render` remains the deterministic terrain-only
completion artifact for now. ADR 0009 expands R3 to complete MAP ingestion and scene presentation
while preserving this ADR's narrow water/render-graph boundary.
