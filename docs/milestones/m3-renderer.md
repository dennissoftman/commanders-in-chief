# M3: Renderer

Draw a map: terrain, water, models, lighting, and shadows, in a window and headlessly.

**Status:** In progress.

## Charter

- Terrain rendering from a heightfield, with level of detail that holds up at both a tactical zoom and
  a strategic one.
- Static and instanced model rendering from imported geometry.
- Directional lighting with cascaded shadows, and ambient occlusion.
- Water surfaces.
- Windowed presentation driven by the reusable camera, plus a headless capture path.
- Visual regression tests: a capture compared against a committed reference, so a rendering change
  either is intended or fails CI.

## Landed so far

- The WGSL shader set, validated at test time by the same front end the GPU backend uses. A copy error
  or a syntax regression fails the build rather than the first frame.
- Virtual-page residency bookkeeping: which terrain pages to stage and evict for a given view, decided
  without touching a GPU so it is testable in isolation.
- Texture resources: decoded images deduplicated by content hash under explicit byte budgets.
- The camera, as a standalone model with no window, input, or GPU dependency.

## Remaining

- Terrain meshing and the deferred pass, rebuilt against the M2 terrain container.
- Model pipeline against imported glTF geometry and PBR materials.
- Shadow cascades and ambient occlusion wired to the terrain and model passes.
- Water surfaces.
- Surface creation, the frame loop, and headless capture.
- Reference captures and the comparison harness.

## Exit condition

A map package loads and renders, in a window and headlessly, at a stable frame rate; the visual
regression harness runs in CI with committed references.

## Design notes

Because a clean build and a green test suite can coexist with a visibly broken frame, the visual
regression harness is treated as part of this milestone rather than as follow-up work. Rendering bugs
that tests cannot see are the expensive kind.

The camera stays free of window, input, and GPU dependencies so the same model drives the game, the
editor, and any debug viewer. Callers translate their own bindings into semantic intents.

## Explicitly not done

- No post-processing chain beyond what the shader set already covers.
- No particle system; it belongs with the gameplay that spawns effects.
- No level-of-detail generation for models. The terrain has it because a heightfield's regularity makes
  it cheap; model LOD wants measurement first.
