# ADR 0003: glTF previews and installed-resource profiles

- Status: accepted
- Date: 2026-07-21

## Context

Wavefront cannot represent W3D hierarchy transforms, skinning, animation, or shader state
without lossy side channels. Retail models and textures also span multiple BIG archives,
and infantry commonly split HLOD/skin, hierarchy, and animation clips into sibling W3Ds.
Zero Hour resources are a delta over Generals rather than a standalone resource set.

## Decision

`cic-inspect w3d-export` is the sole model interchange command. The formats crate decodes
immutable hierarchy, highest-detail HLOD, mesh influence, and classic raw-animation values;
the tools crate composes sibling resources and writes a self-contained glTF 2.0 binary
(`.glb`) by default. `--gltf` selects JSON, an external binary buffer, and PNG images for
inspection. The output basename defaults to the W3D resource basename, while an explicit
output path remains available. A root transform converts W3D Z-up coordinates to glTF Y-up.
Preview materials select pass zero and stage zero while the decoder retains all supported
records. Converted base-color images preserve decoded straight-alpha RGBA texels and declare
sRGB in their PNG metadata; no additional gamma transform is applied.

Raw attachment animations may use extreme helper-bone translations as a visibility convention.
For glTF preview only, a translation farther than both 100 W3D units and 32 bind-pose model
diagonals is represented at a safe nearby translation with a step-interpolated 0.0001 node scale.
The nonzero scale avoids singular joint matrices while keeping hidden geometry imperceptible and
animated bounds useful. Decoded W3D channel values remain lossless and renderer-neutral in the
formats crate.

Skinned mesh nodes are emitted as scene roots instead of children of the axis-conversion node.
The skeleton remains beneath that conversion node, allowing joint matrices to perform the axis
conversion without relying on non-portable parent transforms for skinned meshes. Alpha cutoff is
present only when the material alpha mode is `MASK`, as required by glTF 2.0.
W3D skin vertices are bone-local and are transformed directly by their referenced hierarchy bone,
so the glTF skin uses the format's default identity inverse-bind matrices.

Resource edition is an explicit tools-layer policy, separate from future simulation
compatibility policies. Generals is the default. `--zh` mounts Generals resources first and
Zero Hour resources second. Resolution precedence is a selected-edition `--game-dir`,
edition-specific environment variable, saved configuration, then validated Steam discovery.
Explicit CLI mounts bypass automatic profiles and retain normal left-to-right VFS ordering.

## Consequences

- Blender and ordinary glTF viewers can inspect complete rigid or skinned models and raw
  animation clips without a project renderer.
- User-owned TGA or DDS inputs are embedded in default GLB output or converted beside an
  external glTF output and never enter the repo.
- Missing texture images are visible magenta placeholders with warnings; malformed model
  structure remains a hard structured error.
- Legacy offscreen attachment hiding no longer makes ordinary glTF viewers frame remote geometry
  or encounter singular hidden-joint transforms; this preview heuristic does not claim exact
  fixed-function visibility equivalence.
- Compressed animation, secondary material passes/stages, mapper behavior, and exact legacy
  fixed-function blending remain future compatibility work.
