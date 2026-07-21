# ADR 0002: First-pass Wavefront material preview

- Status: superseded by ADR 0003
- Date: 2026-07-21

## Context

W3D meshes can use several fixed-function passes and texture stages, while Wavefront OBJ
allows one material assignment per face. Geometry and referenced images also live in
different retail BIG archives.

## Decision

The semantic decoder preserves all currently supported shader, texture, assignment, and
UV values without filesystem dependencies. The preview converter maps only material pass
zero and texture stage zero to OBJ+MTL, emits `1-V`, and records the selected W3D shader
selectors as MTL comments rather than claiming full shader equivalence.

Texture lookup occurs in the tools layer through the existing ordered VFS. It accepts
multiple BIG or directory mounts, resolves encoded names under `art/textures`, and allows
a W3D `.tga` name to select the installed `.dds` replacement. Only referenced images are
copied beside the output, using their actual format and extension.

## Consequences

- Blender can perform a texture-and-UV sanity check without a rendering dependency.
- User-owned retail images remain external inputs and never become repository fixtures.
- Multi-pass blending, secondary stages, mappers, and animated textures require a richer
  renderer or interchange representation before they can be previewed faithfully.
- The Wavefront command and implementation were removed before release when ADR 0003
  selected glTF as the sole interchange format.
