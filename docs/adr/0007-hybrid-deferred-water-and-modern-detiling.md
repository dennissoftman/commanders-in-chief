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
- `cic-render` stages lake polygons as stable triangle fans and paired river points as stable
  strips. Degenerate areas are retained by the format value but safely produce no GPU geometry.
- The interactive terrain path is hybrid deferred: opaque terrain, custom edges, and streamed
  detail write albedo, normal/roughness, world position, and depth; a fullscreen directional-light
  resolve writes linear `RGBA16F` scene color; a final surface pass tone maps that scene.
- Water is a later forward, depth-tested, no-depth-write pass. Its project-authored WGSL reads the
  resolved opaque scene and terrain world-position buffer to derive thickness-dependent
  Beer-Lambert absorption, refractive screen offset, Fresnel sky reflection, directional specular,
  shallow-water haze and crest effects, and depth-attenuated world-projected caustics sampled from
  a caller-supplied texture array. Presentation time is explicit viewer input and never enters
  simulation or deterministic headless capture.
- `cic-formats` boundedly decodes only the global `WaterTransparency` opacity and opaque-depth
  fields. `cic-render::WaterAppearance` owns validated, VFS-independent opacity values and an
  optional consistent luminance-frame sequence. `cic-tools` resolves user-owned INI and image
  resources and uploads complete mip chains; synthetic and engine callers may provide the same
  public renderer input directly or omit the optional sequence.
- The water shader and render graph are original project work. The pinned GPL source is used only
  for input field order, water/river classification, and provenance; its Direct3D 8 state machine,
  bump frames, framebuffer reflection path, and fixed-function equations are not translated.
- `Modern` terrain policy applies subtle world-anchored, integer-interpolated macro color
  variation after authored base/primary/extra composition. It never rotates or mirrors tile
  content. `ZeroHourLegacy` output is unchanged. Global cell coordinates make independently baked
  background and near-field regions agree.

## Acceptance and determinism

- Synthetic version-2/version-3 water payloads cover exact closure, all truncated prefixes,
  limits, degenerate markers, and stable lake/river triangulation.
- Modern macro variation must reproduce byte-for-byte across repeated staging and between a full
  32-pixel bake and the equivalent streamed detail region.
- An installed user-owned map with one nine-point lake must remain live through GPU upload and the
  water draw; no retail bytes or capture are retained.
- Full formatting, strict Clippy, and workspace tests remain required completion gates.

## Consequences

Water can evolve toward screen-space reflection, planar probes, clustered lights, and additional
source appearance overrides without coupling format parsing to GPU resources or recreating the
original engine. The current reflection is a bounded sky approximation; source `WaterSet`
colors/textures, time-of-day and map-specific overrides, map-authored lighting, shadows, SSR, and
planar reflection probes are explicit future additions. Headless `map-render` remains the
deterministic terrain-only completion artifact for now.
