# Textures: `.dds` with BC1, BC5 or BC7 blocks

The format decision and its reasoning are in
[ADR 2001](../adr/2001-block-compressed-textures.md). This page is the reference: what the engine reads,
what it writes, and how a texture gets from an authored PNG to the GPU.

## What a texture is for, and therefore what it is

The **slot** decides everything. There is no combination of format and colour space to get wrong, because
the slot fixes both.

| Slot | Format | Colour space | Channels | Bits per texel |
|---|---|---|---|---|
| Base colour, albedo | `BC7_UNORM_SRGB` | sRGB | RGB + alpha | 8 |
| Tangent-space normal | `BC5_UNORM` | linear | `x`, `y` — `z` is rebuilt in the shader | 8 |
| Occlusion / roughness / metallic | `BC7_UNORM` | linear | R, G, B in glTF's order | 8 |
| Flat or low-detail colour | `BC1_UNORM_SRGB` | sRGB | RGB + punch-through alpha | 4 |
| Linear mask or single channel | `BC1_UNORM` | linear | RGB + punch-through alpha | 4 |

**ORM does not fit in BC5.** BC5 is two BC4 blocks — red and green — and has no third channel. A packed
occlusion/roughness/metallic map goes in BC7, at the same 8 bpp, with all three channels intact. BC1 would
fit three channels in half the space and shares one RGB endpoint pair between them, so a roughness gradient
drags the metallic channel with it.

The ORM channel order is glTF's own: **R occlusion, G roughness, B metallic**. Nothing in the shader
changes for a converted texture.

## Converting

```bash
cargo run -p cic-texconv -- --slot base hull_basecolor.png
```

```text
usage:
    cic-texconv --slot <slot> <input.png> [-o <output.dds>]

slots:
    base         base colour or albedo      BC7, sRGB, alpha kept
    normal       tangent-space normal map   BC5, linear, z dropped
    orm          occlusion/roughness/metal  BC7, linear, glTF channel order
    bc1-colour   flat or low-detail colour  BC1, sRGB, punch-through alpha
    bc1-mask     linear mask or single map  BC1, linear
```

The output defaults to the input path with a `.dds` extension. Greyscale, grey+alpha, RGB, RGBA, palette
and 16-bit PNGs are all accepted; 16-bit sources keep their high byte, which is the same loss the glTF
importer takes and for the same reason.

Every file gets a full mip chain down to 1x1, averaged in the slot's own colour space by the same code the
renderer uses for the textures it still mips itself — so a converted texture and an unconverted one recede
identically.

Textures from `texconv`, Compressonator or any other tool are read too, as long as they are one of the five
formats above. A file in BC2, BC3, BC6H or an uncompressed layout is refused with an error naming what it
found.

## How a model finds its textures

A texture is a **sidecar keyed by the glTF image's own name**:

```text
alpine.cicmap  (a zip)
  map.json
  models/tank.glb              declares images named `hull_basecolor`, `hull_normal`, `hull_orm`
  textures/hull_basecolor.dds  BC7 sRGB
  textures/hull_normal.dds     BC5
  textures/hull_orm.dds        BC7 linear
```

The `.glb` keeps declaring its images — the runtime cannot follow a glTF `uri`, because that would read the
host filesystem from an untrusted asset — and the sidecar overrides their pixels. Two arrangements both
work:

- **Keep the authored PNG embedded.** The sidecar wins at runtime; the container stays a complete,
  openable asset.
- **Replace it with a 1x1 placeholder** once the sidecar exists. Decoding it costs nothing.

An image with no name is never looked up. An **absent** sidecar is not an error — that is simply a texture
that has not been converted. A sidecar that **exists and will not read** is an error, because it means a
converted texture is being silently rendered from its placeholder.

## What the renderer does with them

Per array slot, and all-or-nothing:

- If every image that slot is read through has a sidecar of one format and one size, and the device has
  `TEXTURE_COMPRESSION_BC`, the blocks are copied to the GPU untouched — no decode, no resample, no mip
  pass.
- Otherwise that slot decodes to RGBA8 and takes the ordinary path, where a sidecar is still used: its
  base level is better pixels than a placeholder.

The two requirements that reject the compressed path are ordinary content states rather than corruption. A
slot **half converted** cannot mix a compressed slice with an uncompressed one in one array. Sidecars at
**different sizes** cannot be reconciled, because resampling blocks means decoding, resampling and
re-encoding — the offline tool's work, done at load time, for a worse result than converting at the right
size.

Per slot rather than per model is what lets a model whose base colour is BC7 and whose normal is BC5 use
both at once.

A device without `TEXTURE_COMPRESSION_BC` decodes every block on the CPU and uploads RGBA8. The picture is
the same; the memory and the load time are not.

**Two adapters need not agree to the last bit.** A hardware or driver BC decoder is not required to be
bit-exact, and measurably is not: on the same flat BC7 mode-6 block, Apple's Metal decoder reconstructs the
colour exactly while Mesa's `llvmpipe` rounds one least-significant bit differently. Nothing here depends on
more than that, and the tests comparing the two upload paths bound the per-channel difference at one bit
rather than demanding equality — the same reason reference captures are named per adapter.

## Container details

A DDS with a `DX10` header is always what the converter writes, because BC7 has no legacy four-character
code and the legacy codes cannot express the sRGB/linear distinction — the one property of a texture that
nothing downstream can recover by looking at the pixels. On read, the legacy `DXT1` and `ATI2`/`BC5U` codes
are also accepted, as linear, for files from tools that predate the DX10 header.

Refused, with a structured error naming what was found: cube maps, volume textures, array textures (array
slices come from separate files), unsupported DXGI formats, a payload shorter than the header's own
declarations, and anything past the explicit `TextureLimits` bounds. Level sizes are derived from the
dimensions rather than read from `dwPitchOrLinearSize`, which tools disagree about and a hostile file would
simply lie in.

A chain may stop before 1x1 — that is legal DDS, and the renderer creates a texture with exactly the levels
present. It will alias at distance, which is what a mip chain prevents, so the converter always writes a
full one.

## Quality

Measured by decoding the encoder's output with the engine's own specification-derived decoder, in
`cic-texconv`'s tests. Peak signal-to-noise ratio in decibels, higher being closer to the source:

| Fixture | BC7 | BC1 | BC5 (two channels) |
|---|---|---|---|
| Collinear ramp — every channel moving together | 49.6 | 36.1 | 42.9 |
| Independent channels — red along x, green along y | 27.8 | — | 42.9 |
| Hard diagonal edge across a gradient | 31.0 | 28.2 | 37.2 |

Three things worth reading out of that table:

- **The independent-channels row is why a normal map is BC5.** No single line through colour space follows
  two channels that vary independently, whatever the encoder does. BC5 is two independent single-channel
  compressors and has no such constraint.
- **The hard-edge row is the cost of BC7 mode 6 alone.** The encoder does not search partitioned modes,
  which are what a block holding two colour clusters needs. This is a deliberate, measured, documented
  limit — see ADR 2001 — and the gap is pinned by a test so a future partition search has a number to beat.
- **A flat colour is exact in BC5 and within one least-significant bit in BC7.** BC7 mode 6's parity bit is
  per *endpoint*, shared across its four channels, so a colour whose channels disagree on their low bit has
  no exact encoding.
