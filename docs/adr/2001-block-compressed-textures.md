# ADR 2001: Block-compressed textures in DDS, and a converter to make them

- Status: accepted
- Extends: [ADR 0001](0001-native-asset-formats.md) (adds a row to the format table)
- Takes up: [ADR 0004](0004-texture-arrays-and-world-space-tiling.md) (its own note that precomputed mips
  in the asset are the answer if CPU mip generation ever costs too much)

## Context

Textures arrived embedded in `.glb` files as PNG, were decoded to RGBA8 by the glTF importer, and were
uploaded by the renderer, which built each slice's mip chain on the CPU as it went. That works, and it
costs three things that all scale with the texture budget rather than with the number of textures:

1. **Video memory.** A 2048-pixel base colour is 16 MiB as RGBA8, 21 MiB with its mip chain. A model
   carries three such maps. Nothing about this gets better as content grows.
2. **Sample bandwidth.** An RGBA8 texel is four bytes in cache. A compressed one is a quarter of that, and
   the decompression is fixed-function hardware inside the texture unit.
3. **Load time.** Building a mip chain is a full-image pass per level per slice, on the CPU, at load. ADR
   0004 recorded this as an accepted cost and named its successor: *precomputed mips in the asset, not a
   GPU blit chain.*

The question was which compressed format, in which container, with which tool — and how a texture gets
from an author's PNG to the GPU without any stage guessing what the bytes mean.

## Decision

1. **BC1, BC5 and BC7, in a DDS container**, added to ADR 0001's format table as a fifth row.
2. **The slot decides the format, and there is no other knob.** Base colour is BC7 sRGB; a tangent-space
   normal map is BC5; a packed occlusion/roughness/metallic map is BC7 linear; BC1 is available for flat
   colour and masks at half the bytes.
3. **A texture is a sidecar keyed by a name the asset already carries.** A model's image named
   `hull_basecolor` is overridden by `textures/hull_basecolor.dds`; a terrain layer named `grass` is
   surfaced by `textures/grass.dds`. One `resolve_named_textures` answers both. An absent sidecar is not an
   error; a present-but-broken one is.
4. **The decision to use the compressed upload path is made per array, and it is all-or-nothing.** For a
   model that means per material slot, since its three slots are three arrays; for terrain it means the one
   layer array. Either every texture the array needs is compressed and they agree on format and size, or
   that array stays on the RGBA8 path.
5. **A software decoder for all three formats lives in `cic-assets`**, exact to the published
   specifications, and is used both as the fallback for devices without `TEXTURE_COMPRESSION_BC` and as
   the oracle the converter's encoders are measured against.
6. **`cic-texconv` converts PNG to DDS**, with hand-written encoders and mip chains generated in the
   slot's colour space by the same code the renderer uses.

## Rationale

**BC7 for base colour, and for ORM.** BC7 is 8 bpp with a per-block choice of endpoint and partition
modes, and it is the only BC format that carries alpha without visibly banding it — which foliage cutouts
need. For a packed ORM map it is also the right answer, and this is worth stating because the intuitive
choice is wrong: **BC5 cannot hold ORM.** BC5 is two BC4 blocks, red and green, and there is no third
channel. BC1 would fit three channels at half the size and couples them through one shared RGB endpoint
pair, so a roughness gradient drags the metallic channel along with it. BC7 costs the same 8 bpp as BC5
and carries all three.

The ORM channel order is glTF's own — occlusion in red, roughness in green, metallic in blue — so nothing
in the shader changes.

**BC5 for normals.** Two independent 8-bit channels with no shared endpoints, which is what a normal map
needs: `x` and `y` vary independently over a surface, and a single line through colour space cannot follow
both. Measured on a fixture whose red varies along one axis and green along the other, BC5 reaches 42.9 dB
where BC7 at the same bitrate manages 27.8. `z` was never needed — the shader already rebuilds it from
`xy`, because averaging a normal map down a mip chain does not preserve unit length and the *stored* `z`
lies at every level but the first.

**DDS rather than KTX2.** KTX2 is the better-specified container and the one glTF's own extension uses.
DDS wins on one practical point: every texture tool a content author already has writes it, and its header
is 128 bytes of fixed-offset fields needing no dependency to parse. KTX2 also permits a supercompressed
payload (Zstd, Basis), which would put a decompressor in the runtime before the texture unit ever sees a
block — a real feature, and not one needed here, because the package is already a zip.

**The slot as the only knob.** Three things must be right about a converted texture: its block format, its
colour space, and which channel means what. All three follow from what the texture *is*, and both ways of
getting the colour space wrong are quiet. A normal map converted as sRGB tilts every surface by the same
amount and reads as a lighting bug. A base colour converted as linear pales as the camera pulls back,
because its mip chain was averaged in the wrong space. Separate `--format` and `--space` flags would make
those combinations expressible; naming the slot does not.

**A sidecar rather than bytes in the `.glb`.** The runtime cannot follow a glTF `uri` — that would read the
host filesystem from an untrusted asset, and `gltf::import_slice` correctly refuses it. So the container
keeps declaring its images, and the sidecar overrides their pixels. That leaves an author two working
arrangements, and the override supports both: keep the authored PNG embedded and let the sidecar win, or
replace it with a 1x1 placeholder once the sidecar exists and pay nothing for it.

Keying on the glTF *image name* rather than on a path means the link is authored in the DCC tool, and two
models naming the same image get the same texture — which is the sharing glTF already intends.

**All-or-nothing per slot.** A compressed array cannot mix formats or sizes, and it has no resample
available: resampling blocks means decoding, resampling and re-encoding, which is the expensive half of an
offline tool run at load time to produce a worse image than converting at the right size would have. So a
slot whose sidecars are incomplete or inconsistent waits, on a path that still works. Per *slot* rather
than per model is what lets a model whose base colour is BC7 and whose normal is BC5 use both.

**A software decoder, and why it is not optional.** The headless test suite runs on a software rasteriser
that does not advertise `TEXTURE_COMPRESSION_BC`, and `wgpu` correctly refuses a format the adapter lacks.
A renderer that could only draw compressed textures on real hardware would be one whose textures CI cannot
check — the same argument that put headless capture before any window. The decoder is also the only
instrument that can answer "is this block encoded correctly": a reference capture answers it through six
other passes.

Writing it against the published specifications rather than from memory caught two real defects during
implementation, both of which would have shipped:

- The interior colours of BC1 and BC4 are **truncating** integer division in
  `EXT_texture_compression_s3tc` and `ARB_texture_compression_rgtc`. Adding the obvious rounding term
  moves every interior index by up to one least-significant bit away from what the hardware produces.
- BC7 modes with no alpha bits have alpha **overridden** to 1.0. Deriving it instead — widening an
  all-ones stand-in through the parity path — yields 247 in a four-bit mode whose parity bit is zero,
  which is invisible on an opaque pass and wrong the moment such a material is blended.

The BC7 partition and anchor tables are 2176 arbitrary digits with nothing to derive and nothing to check
by inspection, so they are generated from the specification text rather than typed, and a test asserts
their internal consistency: every two-subset partition uses both subsets, every three-subset partition all
three, and every anchor lands on a texel of the subset it anchors.

**BC7 mode 6 only, for now, in the encoder.** Mode 6 is the best single-subset mode — four-bit indices and
eight-bit endpoints. The modes that would beat it are the partitioned ones, which need a 64-partition
search, and that search is where a compressor's bugs live: a wrong anchor, an unswapped endpoint pair, an
empty subset. Mode 6 alone is complete, exact and verifiable, and it is a real improvement on BC1 at the
same bitrate as BC3. The cost is measured and pinned by a test rather than assumed: 49.6 dB on a collinear
block against 31.0 dB on one straddling a hard edge, which is what a partitioned mode would recover.

## Consequences

- Five formats in `cic-assets` instead of four, and the first one with an encoder anywhere in the tree
  (in the tool, not the runtime — a runtime has no business carrying a compressor).
- Two upload paths in `cic-render` that must produce the same picture. What keeps them honest is that both
  average through `cic_assets::image`: a converter that averaged encoded sRGB while the renderer averaged
  linear light would bake a visibly different mip chain into the asset, and the difference would appear
  only as the camera pulls back, on precisely the textures that had been converted — reading as "block
  compression darkens things" rather than as an arithmetic mistake.
- The colour-space vocabulary moved from `cic-render` to `cic-assets`, which now owns the averaging rules
  both the runtime and the tool select between. `ColourSpace::format()` became a free function in
  `cic-render`, because `cic-assets` has no business naming a `wgpu` type.
- A flat colour is **not** exactly representable in BC7 mode 6 unless its channels agree on their low bit,
  because the parity bit is per endpoint rather than per channel. The identity values the renderer fills
  unused array slices with are chosen from the set that is exact.
- Terrain layer textures use the same path, and needed nothing new to do it. A layer's *name* was already
  the key — the `.cict` container has never held layer pixels, only names and weights, and the renderer has
  always resolved a name against a material set it was handed — so `textures/<layer>.dds` is that same
  resolution reaching the package. This is where the format pays most: a detail texture is sampled by up to
  eight layers in one fragment across the whole visible map, and detail textures are authored to one size
  and tiled, so the uniform-size requirement of a compressed array costs nothing.
- The all-or-nothing rule applies per *array* for terrain and per *material slot* for a model, which is the
  same rule in both cases — one array holds one format at one size. An untextured terrain layer is not an
  obstacle to it: it takes a flat white slice in the array's own format, so a partly-textured map still
  takes the fast path.
- Both `resolve_model_textures` and `resolve_terrain_textures` are wrappers over one
  `resolve_named_textures`. The convention is a single definition rather than two that must agree, and
  `TextureResolveError` is named for the operation rather than for whichever caller came first.
- Content that has not been converted keeps working unchanged, and converted content keeps working on a
  device that cannot sample blocks. Neither is a fallback in the sense of being worse than nothing: the
  first is the old path and the second is the same picture built the slow way.
