# ADR 0004: Texture arrays, world-space tiling, and CPU mip generation

- Status: accepted

## Context

Two surfaces needed base-colour textures, and both draw in a single call over many materials.

Terrain blends up to eight weighted layers *in one fragment*, so every layer's surface has to be
reachable simultaneously. Models concatenate their primitives into one vertex and index buffer
specifically so a model of twenty primitives is one draw rather than twenty — a decision ADR-0003's
boundary work and the model module already committed to, and one a texture per material would undo.

Neither can bind a texture per surface. What they need is to *index into* a bound resource.

A third question sits underneath both: what coordinate does a terrain layer sample at? The obvious
answer is the terrain's own normalized `uv`, which is already interpolated and already used for the
weight lookup.

## Decision

1. **One `D2Array` colour texture per drawing unit.** Terrain gets one array with a slice per weight
   layer; each model gets one with a slice per image it carried. Materials store a slice *index*, not a
   resource. Slices that arrive at different sizes are resampled to the largest present.
2. **Terrain layers are sampled at world position divided by a per-layer detail scale**, not at the
   terrain's normalized coordinate. The scale is authored in world units per repeat.
3. **The palette colour multiplies the sampled texel** rather than being replaced by it. A layer with no
   image multiplies an opaque-white slice.
4. **Mip chains are generated on the CPU at upload**, and both the resample and the mip reduction average
   in linear light rather than in stored sRGB.
5. **Sampling is never branched around.** A material with no texture samples slice 0 anyway and discards
   the result with a `select`.

## Rationale

**Array over atlas.** An atlas keeps every image at its authored size, which is the array's one real
cost. It was still wrong here. Terrain detail textures tile by definition, and an atlas cannot express a
wrapped coordinate — a repeat would walk into the neighbouring rect. Below the gutter width every mip
level bleeds between rects as well, so the aliasing the mip chain exists to remove comes back as
neighbouring materials smeared into each other. The array's cost is memory for upsampled slices; the
atlas's cost is correctness for half the callers.

Bindless arrays of textures would sidestep both and need a device capability this renderer does not
require.

**World space over `uv`.** A normalized coordinate fits exactly one copy of the image across the whole
map. On a two-kilometre map a 512-pixel grass texture then resolves at about four metres per texel,
which is uniform blur at every zoom a player actually uses. A world-space divisor fixes the repeat at a
real size, so the same authored value means the same thing on a small map and a large one — and it costs
no interpolator, because world position is recoverable from `uv` exactly: `uv` is the grid coordinate
over the cell count, and world position is that coordinate times the sample spacing.

**Multiply rather than replace.** Terrain authored against flat colours renders byte for byte as it did,
which is asserted by a test. It also means one greyscale detail texture can be recoloured per map
instead of shipping a second copy of the image.

**Mips on the CPU.** Not an optimisation. A strategic-zoom camera minifies a detail texture by two orders
of magnitude, and an unmipped sample of that is a field of aliasing that crawls when the camera moves.
Generating them here rather than through a GPU blit chain keeps the whole thing a pure function over
bytes, testable with no device — which is where the linear-light requirement was actually pinned down.
Averaging sRGB-encoded values is not the same as averaging the light they encode: the transfer curve is
concave, so the mean of two encoded values sits above the encoding of their mean, and a high-contrast
texture visibly pales as it recedes.

**No branch around the sample.** `textureSample` picks its mip level from screen-space derivatives, which
WGSL defines only in uniform control flow. A material index is per-vertex and a terrain layer's weight is
per-fragment; neither is uniform. Skipping the sample for the fragments that do not need it would leave
the mip level undefined for the fragments that do — the surfaces that matter, not the ones being skipped.
The `select` costs the sample and buys defined behaviour. Shader validation catches a regression here,
because `naga` runs the same uniformity analysis the backend does.

## Consequences

- A model mixing a 1024-pixel hull texture with a 256-pixel decal sheet stores the decal upsampled. Memory,
  not quality. Explicit byte budgets bound it.
- Terrain layers are capped at eight, unchanged: the array adds a slice per layer, not a new limit.
- Uploading a large array costs a CPU pass per slice to build its mip chain. Acceptable at load time; if
  it ever is not, the answer is precomputed mips in the asset, not a GPU blit chain.
- `cic-assets` now decodes glTF images and normalizes them to RGBA8, because ten pixel layouts are a
  property of the format and a renderer should not know any of them. Floating-point layouts are refused
  rather than tone mapped with a guessed exposure.
- Anisotropic filtering is deliberately not used. It is an optional device capability, and a sampler that
  fails to create on a software adapter would take the headless tests with it. Trilinear until there is a
  measured reason and a capability check.
