# ADR 0003: glTF previews and installed-resource profiles

- Status: accepted
- Date: 2026-07-21

## Context

Wavefront cannot represent W3D hierarchy transforms, skinning, animation, or shader state
without lossy side channels. Retail models and textures also span multiple BIG archives,
and infantry commonly split HLOD/skin, hierarchy, and animation clips into sibling W3Ds.
Zero Hour resources are a delta over Generals rather than a standalone resource set.

## Decision

`cic-inspect w3d-gltf` is the sole model interchange command. The formats crate decodes
immutable hierarchy, highest-detail HLOD, mesh influence, and classic raw-animation values;
the tools crate composes sibling resources and writes glTF 2.0 JSON plus an external binary
buffer and PNG images. A root transform converts W3D Z-up coordinates to glTF Y-up. Preview
materials select pass zero and stage zero while the decoder retains all supported records.

Resource edition is an explicit tools-layer policy, separate from future simulation
compatibility policies. Generals is the default. `--zh` mounts Generals resources first and
Zero Hour resources second. Resolution precedence is a selected-edition `--game-dir`,
edition-specific environment variable, saved configuration, then validated Steam discovery.
Explicit CLI mounts bypass automatic profiles and retain normal left-to-right VFS ordering.

## Consequences

- Blender and ordinary glTF viewers can inspect complete rigid or skinned models and raw
  animation clips without a project renderer.
- User-owned TGA or DDS inputs are converted beside the output and never enter the repo.
- Missing texture images are visible magenta placeholders with warnings; malformed model
  structure remains a hard structured error.
- Compressed animation, secondary material passes/stages, mapper behavior, and exact legacy
  fixed-function blending remain future compatibility work.
