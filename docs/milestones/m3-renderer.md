# M3: Renderer

Draw a map: terrain, water, models, lighting, and shadows, in a window and headlessly.

**Status:** In progress. Terrain renders.

## Charter

- Terrain rendering from a heightfield, with level of detail that holds up at both a tactical zoom and
  a strategic one.
- Static and instanced model rendering from imported geometry.
- Directional lighting with cascaded shadows, and ambient occlusion.
- Water surfaces.
- Windowed presentation driven by the reusable camera, plus a headless capture path.
- Visual regression tests: a capture compared against a committed reference, so a rendering change
  either is intended or fails CI.

## Landed

- **Headless device and capture.** A device with a software-adapter fallback, bounded capture sizing,
  padded readback, and PNG encoding. Headless before windowed, because a capture is the only rendering
  verification that runs in CI.
- **A forward terrain pass.** A `cic-assets` terrain renders with directional lighting and blended
  texture layers. Heights go into an `R16Uint` texture and layer weights into an `R8Unorm` array, both
  writable; the grid is procedural in the vertex shader and normals come from central differences on
  the height texture. Terrain deformation and route grading are therefore texture writes, not a remesh
  — verified by tests that draw, write a region, and draw again.
- **View and projection**, with a test that pins the near and far planes to depth 0 and 1. That
  assertion exists because an OpenGL-convention matrix here would clip half the scene and look like a
  culling bug.
- **The WGSL shader set**, validated at test time by the same front end the GPU backend uses.
- **Virtual-page residency bookkeeping**, decided without touching a GPU so it is testable in isolation.
- **Texture resources**, deduplicated by content hash under explicit byte budgets.
- **The camera**, as a standalone model with no window, input, or GPU dependency.

## Remaining

- The deferred chain: G-buffer, cascaded shadows, ambient occlusion. Shaders exist and validate; they
  need pipeline scaffolding and matching bind group layouts. This is also what the forward pass most
  visibly lacks — terrain currently reads as correctly shaped but flatly lit, because nothing occludes
  anything.
- Model pipeline against imported glTF geometry and PBR materials.
- Water surfaces, including re-authoring the shader whose constants were left behind.
- Albedo textures per terrain layer, replacing the current flat palette colours.
- Wiring the residency bookkeeping to a real virtual-texture cache, so terrain detail scales past what
  one texture can hold.
- Surface creation, the frame loop, and windowed presentation.
- Committed reference captures and the comparison harness.

## Exit condition

A map package loads and renders, in a window and headlessly, at a stable frame rate; the visual
regression harness runs in CI with committed references.

## Design notes

**Heights and weights are textures, not a baked mesh.** This was decided against known future
requirements rather than discovered: a faction whose map presence is literally paved needs to grade
roads across terrain at runtime, and levelling a structure needs terrain to deform. Both are texture
writes in this design and a remesh-plus-upload in the obvious alternative. The cost of designing for it
now was nil; the cost of retrofitting it would not have been.

**The forward pass does not tone map.** Albedo and the light terms are both near unity, so a Reinhard
curve compressed contrast that was never out of range — flattening exactly the slope shading the pass
exists to show. Tone mapping belongs with the deferred path, where accumulated lights genuinely can
exceed one.

**Ambient is deliberately low.** A large constant ambient lifts shadowed slopes almost to lit ones,
which flattens a heightfield just as effectively as a vertical light does. Skylight belongs in an
ambient-occlusion pass, not a constant.

**Captures are asserted on luminance spread, not colour count.** A distinct-colour count mostly reports
how varied the fixture's palette was; luminance range and deviation report whether light actually
differentiates slopes from flats.

**One GPU device per test binary, not per test.** Creating and destroying several devices concurrently
on one adapter crashed the driver outright — an access violation rather than a test failure.

## Explicitly not done

- No post-processing chain beyond what the shader set already covers.
- No particle system; it belongs with the gameplay that spawns effects.
- No level-of-detail generation for models. Terrain has it because a heightfield's regularity makes it
  cheap; model LOD wants measurement first.
- No multisampling. Terrain silhouettes against the sky currently show stair-stepping, which is the
  expected cost and a known, deliberate gap.
