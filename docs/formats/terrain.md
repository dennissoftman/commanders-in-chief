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

## Why `u16` elevations

Two reasons, the second being the one that decided it:

1. Half the size of `f32`, which matters at 8,192 × 8,192.
2. `u16` normalized is a native GPU texture format (`R16Unorm`), so the payload uploads as a height
   texture with no conversion pass.

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
