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
7. **`--from-glb` converts a model's own textures and rewrites it.** The slot of every image is *derived*
   from the material references rather than guessed from a filename; a separate occlusion map is merged into
   the ORM image and both slots repointed at it; and every image becomes a 1x1 placeholder so the container
   slims while keeping the named-image link a sidecar is found by.
8. **A material's baked occlusion is applied**, to the ambient term only, from the red of the ORM image.
9. **A model's textures go inside its container by default**, as `MSFT_texture_dds`. Sidecars remain for
   terrain, which has no container, and for a package deliberately sharing one texture between models.

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

**DDS rather than KTX2.** KTX2 is the better-specified container and the one glTF's ratified extension
(`KHR_texture_basisu`) uses. DDS wins on one practical point: every texture tool a content author already
has writes it, and its header is 128 bytes of fixed-offset fields needing no dependency to parse.

The argument against KTX2 is a runtime one and not a size one, and it is worth being exact because the
size point runs the *other* way. A KTX2 payload can be supercompressed — Basis Universal as ETC1S or
UASTC, optionally Zstd on top — and that is genuinely smaller on disk than raw BC7 even after the package
zips it, because BC7 is fixed-rate and compresses poorly while UASTC is designed to compress. So KTX2 would
win on disk.

What it costs is a **transcoder in the runtime**: a UASTC payload is not blocks the texture unit can read
until something converts it, per texture, at load. The reference implementation is C++, its Rust binding is
FFI, and this workspace forbids `unsafe` — so adopting it is an ADR about that policy rather than a format
choice. A hand-written UASTC transcoder is a great deal more than the BC7 decoder this record already
justifies.

**The case that would reverse this is portability, not size.** BC is a desktop feature: `wgpu` reports it on
desktops, on WebGPU, and on only some Apple mobile parts. A BC-only pipeline therefore assumes a desktop
target, and a second one — ETC2 or ASTC — would mean converting every texture twice. UASTC transcodes to
whichever the device has, from one asset. If this engine ever targets mobile, revisit this record; nothing
else about the decision changes.

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

**Deriving the slot rather than guessing it.** A glTF material states which slot each image is read
through, so the block format and the colour space follow with certainty. A filename convention would be a
second, weaker source of the same fact — and the two would disagree on the first asset whose author named
files unhelpfully. An image referenced through slots that disagree is reported, because there is no single
answer and picking one silently is how a normal map ends up in sRGB.

**Merging occlusion, and why that forces the model to be rewritten.** This engine reads occlusion from the
red of the metallic-roughness image, because a fourth array slot and a fourth slice index in a material
record with one float left are not free. glTF permits the two to be separate images, so the merge is what
makes such a model readable — and once two images become one, both material slots have to point at it,
which is a change to the glTF and not to a texture. That is why conversion and rewriting are one operation.

A channel with no source is **255** rather than whatever the other image carried. glTF leaves red unused in
a metallic-roughness image, so carrying its contents into the occlusion channel would darken the surface by
an arbitrary amount — the kind of wrong that looks like a lighting bug.

**Placeholders rather than deleted images.** The container cannot carry the DDS itself, and the images
cannot simply go: a material's slot reference is how the runtime knows which sidecar belongs to which slot,
and a named image entry is that link. A 1x1 placeholder keeps it for a hundred bytes. An image left
unreferenced by a merge stays as a placeholder too and is *reported*, because removing it would renumber
every later image and every texture that names one — a rewrite with more ways to go wrong than the bytes are
worth.

**Applying occlusion in the lighting pass.** glTF scopes occlusion to indirect light: a baked crevice is
dark because less skylight reaches it, not because the sun stopped shining on it. So it has to reach the
pass that computes the ambient term, which needed a G-buffer channel — and every one was claimed, albedo's
alpha having taken the last at no cost. `COVERAGE_FORMAT` therefore widens from `R16Float` to `Rg16Float`:
two bytes per pixel, about 8 MiB per frame at 2560x1600, measured under a tenth of a millisecond on an
M1 Pro. The alternative, folding occlusion into albedo in the G-buffer, is cheaper and wrong — it would
darken the direct term too, which a test now pins shut by asserting that with the ambient light zeroed an
occluded and an unoccluded frame are identical to the byte.

The screen-space and baked terms combine by `min` rather than by multiplying, because they describe the
*same* occlusion by different means and multiplying darkens a crevice twice.

**Embedding, having first chosen sidecars.** The sidecar was the right call when the alternative was
hand-parsing an extension for a format nothing here could yet read. Once a DDS reader and a GLB writer
existed, the balance changed: `MSFT_texture_dds` costs about a hundred lines and removes a convention that
must not be broken — a renamed image silently loses its texture, and nothing fails loudly when it does.

The extension's own design is what makes it safe to ship. A texture keeps its ordinary `source`, so a reader
that has never heard of the extension sees an ordinary textured glTF rather than an untextured one. It is a
vendor extension rather than a Khronos one, which matters less than it appears: the *fallback* is what other
tools see, and it is a conformant glTF.

What it does not remove is the sidecar path. Terrain has no container to live in, and a package sharing one
texture between several models should hold one copy rather than one per model.

**And a limitation of the arrangement, which is a property of the crate rather than the extension.** The
extension assumes a reader decodes only the images it uses; the `gltf` crate decodes all of them eagerly and
knows PNG and JPEG. So a container with an embedded DDS is refused over an image no material would have
sampled, and reading one means rewriting the document first — which is why the GLB container module sits in
`cic-assets` and not only in the tool. Every import does it, `import_model` included, because a function that
refuses a valid model is a trap regardless of whether its caller wanted the textures.

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
- Green in the coverage target is 1.0 wherever nothing baked occlusion, so a scene with no occlusion maps
  renders exactly as it did before the channel existed — which is what kept every committed reference
  capture valid through the widening, and they all still pass.
- The rewriter is a GLB *writer*, which the `gltf` crate does not provide, so this tree now has one. It
  parses the JSON chunk into an untyped tree specifically so that anything it does not touch — including an
  extension it has never heard of — survives; and it refuses a container holding an unknown *chunk* rather
  than dropping it, because skipping one is right for a reader and wrong for something writing the file back
  out.
- **A hardware BC decoder is not required to be bit-exact, and they are not.** Measured on the same flat
  mode-6 block: Apple's Metal decoder reconstructs it exactly, and Mesa's `llvmpipe` — which CI runs on, and
  which does advertise `TEXTURE_COMPRESSION_BC` — rounds one least-significant bit differently across a
  quarter of the frame. So a test comparing the compressed and uncompressed upload paths bounds the
  per-channel difference at one bit rather than demanding equality. This is consistent with a decision
  already made here: reference captures are named per adapter for the same reason.
- Content that has not been converted keeps working unchanged, and converted content keeps working on a
  device that cannot sample blocks. Neither is a fallback in the sense of being worse than nothing: the
  first is the old path and the second is the same picture built the slow way.
