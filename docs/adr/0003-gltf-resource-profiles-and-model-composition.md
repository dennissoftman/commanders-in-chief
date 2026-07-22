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
immutable hierarchy, highest-detail HLOD, mesh influence, classic raw-animation values, and
time-coded or adaptive-delta compressed animation values;
the tools crate composes sibling resources and writes a self-contained glTF 2.0 binary
(`.glb`) by default. `--gltf` selects JSON, an external binary buffer, and PNG images for
inspection. The output basename defaults to the W3D resource basename, while an explicit
output path remains available. A root transform converts W3D Z-up coordinates to glTF Y-up.
Preview materials select pass zero and stage zero while the decoder retains all supported
records. Packaged source images preserve decoded straight-alpha RGBA texels and declare sRGB in
their PNG metadata; no additional gamma transform is applied. A `ONE + ONE` additive material
also receives a separate deterministic preview image whose maximum RGB channel becomes alpha
coverage and whose RGB is unassociated from that coverage. The material uses the derived image;
`fixed-function-metadata-v1` continues to link the untouched source image.

Core glTF cannot exactly express ordered W3D fixed-function passes, arbitrary stage combiners,
animated mappers, or their framebuffer blend operations. The visible metallic-roughness preview
therefore remains pass zero/stage zero. A versioned `fixed-function-metadata-v1` mesh extra retains
every decoded pass, stage, assignment, shader byte, color array, mapper selector/argument string,
animated-texture descriptor, and exact float bits. Every table texture is packaged even when it is
not selected by the visible preview. This is an inspection/interchange contract, not a claim of
fixed-function visual equivalence.

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

Zero Hour is a Generals delta at both archive and definition levels. The following ordering is a
repository invariant for every built-in `--zh` consumer:

1. enumerate and mount the required Generals providers in their stable profile order;
2. enumerate and mount Zero Hour providers in their stable profile order;
3. append explicit mod providers in command-line order.

Consumers must then classify each input by its source semantics. An opaque/replacement resource
uses the last-mounted winning entry. A cumulative definition resource whose later files may contain
partial additions or overrides must parse `Vfs::history` from earliest to latest and apply the
format's own merge rules. It is incorrect to resolve only the winning edition INI merely because
the physical path is shadowed: doing so erases Generals definitions that Zero Hour intentionally
inherits. Each new cumulative consumer requires a synthetic test where the base supplies a needed
definition and the overlay shadows the file while omitting that definition.

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
- Time-coded and adaptive-delta animation share the raw-animation export path after bounded
  decompression; their source encoding is named in animation extras.
- Black-backed additive light sprites remain transparent in ordinary glTF viewers without
  replacing or mutating the decoded source-RGBA interchange image.
- Exact legacy fixed-function visual equivalence remains renderer work, while the interchange
  artifact retains the complete decoded input needed for that renderer gate.
- Installed Zero Hour consumers cannot accidentally become expansion-only: archive discovery,
  mount order, and cumulative-definition parsing preserve the Generals base before edition and mod
  overlays.
