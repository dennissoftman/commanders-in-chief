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
| Emissive | `BC7_UNORM_SRGB` | sRGB | RGB | 8 |
| Flat or low-detail colour | `BC1_UNORM_SRGB` | sRGB | RGB + punch-through alpha | 4 |
| Linear mask or single channel | `BC1_UNORM` | linear | RGB + punch-through alpha | 4 |

**ORM does not fit in BC5.** BC5 is two BC4 blocks — red and green — and has no third channel. A packed
occlusion/roughness/metallic map goes in BC7, at the same 8 bpp, with all three channels intact. BC1 would
fit three channels in half the space and shares one RGB endpoint pair between them, so a roughness gradient
drags the metallic channel with it.

The ORM channel order is glTF's own: **R occlusion, G roughness, B metallic**, and all three are read —
occlusion included, which the renderer applies to the ambient term only, honouring the material's
`occlusionStrength`. That is where glTF scopes it: a baked crevice is dark because less skylight reaches it,
not because the sun stopped shining on it.

Occlusion is read from the red of the **metallic-roughness image**, so the two must be one image. A material
whose occlusion is a separate image reports no occlusion and renders unoccluded;
[`--from-glb`](#converting-a-models-own-textures) is what merges such a model into the readable arrangement.

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

## Where a model's textures live

Two arrangements, and a model uses one or the other:

**Inside the container** (`MSFT_texture_dds`, the default). One file holds the model, its materials and its
compressed textures. A texture keeps its ordinary `source` — now a 1×1 placeholder — and adds an extension
naming a second image whose payload is the DDS:

```json
"textures": [{ "source": 0, "extensions": { "MSFT_texture_dds": { "source": 1 } } }],
"images": [
  { "bufferView": 1, "mimeType": "image/png" },
  { "bufferView": 2, "mimeType": "image/vnd-ms.dds" }
]
```

Nothing to name, nothing to lose track of, and the fallback means a reader that has never heard of the
extension still sees a complete glTF.

**Beside it**, as `textures/<image name>.dds` — what `--sidecars` writes, and what a package sharing one
texture between several models wants. Terrain layers always use this, having no container to live in.

Read with `import_model_with_textures`, which returns the model and its textures together. The textures are
keyed by the image index a *material* names — the fallback's — so an embedded texture and a sidecar are the
same thing by the time the renderer sees one.

### One wrinkle worth knowing

The extension is designed so a reader ignorant of it falls back to the PNG, and that holds for a reader
which decodes only the images it uses. The `gltf` crate decodes **every** entry in `images` eagerly and
knows PNG and JPEG only — so a container with an embedded DDS is refused outright over an image no material
would ever have sampled.

So every import here first lifts the DDS images out of the crate's way, `import_model` included, even
though it discards them. Otherwise the simpler function is a trap for whoever reaches for it next. A DDS
image that *no* texture names is refused with a message saying so, because the extension is the only thing
that makes a DDS image legal glTF in the first place.

## Converting a model's own textures

A `.glb` carries its textures inside it, and `--from-glb` converts all of them at once:

```bash
cic-texconv --from-glb models/hull.glb              # textures inside the model
cic-texconv --from-glb models/hull.glb --sidecars textures   # textures beside it
```

```text
hull.glb -> hull.glb
  hull_orm.dds                  128x128       21.5 KiB  BC7_UNORM (merged for occlusion/roughness/metallic)
  hull_basecolor.dds            128x128       21.5 KiB  BC7_UNORM_SRGB
  hull_normal.dds               128x128       21.5 KiB  BC5_UNORM
  2 images are no longer referenced by any material, left as a placeholder
  glb 2.7 KiB -> 66.6 KiB, 4 images replaced by a 1x1 placeholder and the DDS embedded beside it
```

**No `--slot` here, and no filename heuristics.** A glTF material states which slot every image is read
through, so the format and the colour space are *derived*. An image read as a base colour by one material
and as a normal map by another has no single answer and is reported rather than resolved by guessing.

JPEG sources work as well as PNG, because glTF permits both and the importer decodes both.

**It merges a separate occlusion map into the ORM image.** glTF permits occlusion and metallic-roughness to
be two different images; this engine reads all three channels from one. So red comes from the occlusion map,
green and blue from the metallic-roughness map, and both material slots are repointed at the result — which
is why this rewrites the model rather than only writing textures. A channel with no source is **255**, not
whatever the other image happened to carry there: glTF leaves red unused in a metallic-roughness image, and
reading it as occlusion would darken the surface for no reason.

**It slims the model's own images.** Every one becomes a 1×1 placeholder and the binary chunk is compacted —
so with `--sidecars` the container shrinks outright, and when embedding it grows by exactly the textures it
now carries. The
images are not *removed*, because a material's slot references are how the runtime knows which sidecar
belongs to which slot — a named image entry is the link. An image whose only reader was a merged-away slot
stays as a placeholder too, and is reported so it can be pruned at the source; removing it would renumber
every later image and every texture that references one.

The rewrite preserves everything it does not deliberately change, including a texture reference's
`strength`, its `texCoord` and any extension on it — and refuses a container holding a chunk it does not
understand rather than dropping it.

## How a terrain layer finds its texture

The same convention, keyed on the layer's name:

```text
alpine.cicmap  (a zip)
  map.json
  terrain/alpine.cict          declares layers named `grass`, `rock`, `sand`
  textures/grass.dds           BC7 sRGB, tiled in world space
  textures/rock.dds
```

Nothing new was needed for this. `TerrainLayer::name` has always been the key — the `.cict` container
carries names and weights, never pixels, and the renderer has always resolved a name against a material set
it was handed. `resolve_terrain_textures` makes that name resolve against the package too, returning one
entry per layer in layer order.

A layer with no file renders as its flat palette colour, exactly as an untextured layer always has.

Terrain is where the format pays most: a detail texture is sampled by up to eight layers in one fragment
across the whole visible map, so it is both the largest texture budget here and the most
bandwidth-sensitive. It is also the easiest fit, because detail textures are authored to one size and tiled,
so a compressed array's uniform-size requirement costs nothing.

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

For **terrain** the same rule applies per *array*, since a terrain layer has one surface rather than three
slots: the compressed path is taken when every layer that has a texture at all has a compressed one, and
those agree. A layer with **no** texture abstains rather than blocking it — it takes a flat white slice in
the array's own format, so a partly-textured map still takes the fast path.

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
