# Terrain container (`.cict`)

A tagged chunk container holding a heightfield and its texture layer weights.

Version 1. Little-endian throughout.

## Structure

```text
offset  size  field
0       4     magic, "CICT"
4       4     u32 version
8       4     u32 chunk_count

then chunk_count records:
+0      4     u32 tag
+4      4     u32 payload byte length
+8      n     payload
              padding to the next 4-byte boundary
```

Padding is part of the container, not the payload. A decoder steps over it before reading the next tag.

## Chunks

### `HEAD` — required, 24-byte payload

```text
0   u32  width in samples
4   u32  height in samples
8   f32  horizontal scale, world units between adjacent samples
12  f32  vertical scale, world units per elevation step
16  u32  layer count
20  u32  reserved, must be written as zero
```

Both scales must be finite and positive. Both dimensions must be non-zero.

### `HGHT` — required

`width * height` `u16` elevations, row-major, X varying fastest.

World elevation is `sample * vertical_scale`. The world extent of an axis is
`(samples - 1) * horizontal_scale`, because `n` samples span `n - 1` intervals.

### `LYRN` — required when `layer_count > 0`

`layer_count` NUL-terminated UTF-8 layer names, concatenated. The count comes from `HEAD`, so the
chunk carries no count of its own.

### `LYRW` — required when `layer_count > 0`

`layer_count * width * height` `u8` weights: all of layer 0's samples, then all of layer 1's, and so
on. `0` is absent, `255` is fully covering.

## What a layer name means

A name and nothing else. The container carries *where* each layer is and *how much* of it there is; it
carries no colour, no image reference, and no tiling scale. Those are the renderer's business, resolved
against the name.

That split is deliberate. A surface is a rendering concern that will change — texture, roughness, and
detail scale are all things an artist adjusts without touching a map, and several of them did not exist
when this format was first written. Putting them in the terrain container would have meant a format
version bump for each, and would have forced every editor and tool that reads a heightfield to
understand materials it has no use for.

The consequence worth stating: a `.cict` is not self-describing as an *image*. Open one without the
material set it was authored against and you get correct geometry with a placeholder surface, which is
the right failure — visibly unfinished rather than silently wrong.

See [ADR 0004](../adr/0004-texture-arrays-and-world-space-tiling.md) for how the renderer resolves it.

## Why `u16` elevations

Two reasons, the second being the one that decided it:

1. Half the size of `f32`, which matters at 8,192 × 8,192.
2. 16-bit integer is a *baseline* GPU texture format (`R16Uint`), so the payload uploads as a
   height texture byte-for-byte with no conversion pass. The normalized variants (`R16Unorm`) are
   not baseline — they need an optional device feature — which is why the integer form is used.
   Nothing is lost by it: elevations are only ever loaded at exact texel coordinates, never
   filtered.

65,536 quantization levels across any sane vertical range is far finer than terrain needs. At a
vertical scale of 0.25 world units, the range is over 16 km with 25 cm resolution.

## Why not JSON, and why not glTF

JSON would store `"1024"` as five bytes where two suffice, cost a parse of every sample, and — for
floats — round-trip lossily unless every value is written to 17 significant digits.

glTF describes meshes. Expressing a heightfield as one discards the regularity that makes terrain
cheap: an implicit grid becomes explicit vertices and indices, costing an order of magnitude more
space and forbidding GPU-side level of detail.

## Forward compatibility

Unknown chunk tags are skipped, not refused, so data written by a newer build stays readable. An
unknown *version* is refused: a version bump means existing fields may have changed meaning, which is
not something to guess at.

## Limits

A decoder takes explicit bounds for the maximum dimension, total sample count, layer count, and chunk
count. Every one is checked against the `HEAD` chunk before any payload-sized allocation is considered
— so a header claiming an enormous terrain is refused while only 24 bytes have been read.
