# ADR 0005: Antialiasing strategy, and why not MSAA

- Status: accepted

## Context

Terrain silhouettes against the sky show stair-stepping, and the water pass added a second, worse case:
sun glitter on wave normals is a field of sub-pixel highlights that sparkles as the camera moves. M3 has
carried "no multisampling" as a known gap since the deferred chain landed, and the gap was recorded
without ever stating which technique would close it.

The obvious answer is MSAA, because it is the one every graphics API offers as a switch. That answer is
wrong here, and the reason is structural rather than a matter of cost.

**MSAA is hostile to a deferred renderer.** The chain resolves lighting from a G-buffer, so multisampling
it means multisampling every G-buffer target — albedo, world normal with roughness, coverage, and depth.
That is four times the memory and bandwidth on all of them. Worse, the lighting pass cannot simply
resolve those targets first: averaging two normals across a silhouette produces a direction describing no
surface that exists, and averaging two depths produces a position between the two objects. The correct
construction is per-sample lighting, restricted to edge pixels found by a stencil pass — a substantial
complication to the one pass in the renderer that is already the most intricate.

And having paid all that, MSAA fixes only *geometric* edges. It does nothing for the terrain detail
texture at grazing angles, nothing for specular aliasing, and nothing for the noise in the occlusion
pass. The glitter case — now the most visible aliasing in the renderer — is exactly the case MSAA cannot
touch, because the variation is in the shading rather than in the coverage.

## Decision

1. **MSAA is declined.** Not deferred, not "later": the deferred G-buffer makes it the wrong tool, and
   the record should say so rather than leaving a permanent open item implying it is coming.
2. **A resolution scale is the primary control**, exposed to the player as a factor on the render
   resolution with the composite as the downsample. The chain already renders every pass into offscreen
   targets, so this is a size multiplier at allocation plus a filtered blit.
3. **FXAA is the cheap floor** — one pass over the tone-mapped image, no architectural cost.
4. **TAA is the quality tier**, with jittered projection, a motion-vector target, a history buffer, and
   neighbourhood clamping.
5. **Scene time is already a frame parameter, not a clock reading**, which is a precondition for any
   temporal technique and for the regression harness at the same time.

## Rationale

**Resolution scale is the most value per line of code in the whole list.** It addresses every class of
aliasing at once — geometric, texture, specular, and occlusion noise — because it is the only one that
raises the actual sampling rate. Players understand the control without a glossary, and an RTS at
strategic zoom routinely has GPU headroom to spend on it. Its cost is quadratic, which is an honest
trade-off to put in front of someone rather than a hidden one.

**TAA suits this genre unusually well.** Its failure mode is fast unpredictable motion, and this camera
pans, zooms, and rotates about a ground plane with small on-screen objects and no first-person view. It
also subsidises work already being paid for: accumulating over frames allows a smaller occlusion blur
radius, trading a smeared ambient term for sharper contact shadows. The glitter case is the one that most
wants it, since temporal accumulation is the standard answer to sub-pixel specular highlights.

**SMAA was considered and set aside.** It is better than FXAA for the same cost class, but its reference
implementation ships two precomputed lookup textures. That is a third-party dependency and a pair of data
blobs, and given how carefully `LICENSING.md` tracks provenance in this tree, vendoring them belongs in a
deliberate change with its own `NOTICE` entry rather than arriving as a detail of an antialiasing pass.
CMAA2 reaches similar quality with no lookup tables and is the better candidate if FXAA proves
insufficient before TAA lands.

## Consequences

- The `Remaining` list in M3 loses "multisampling" and the `Explicitly not done` list gains a *decision*
  rather than an omission. The two lists previously contradicted each other on this point.
- Antialiasing becomes a display setting with more than one option, which is M4's concern to present —
  and the first real content for the settings screen its charter already requires to apply
  transactionally.
- **TAA and the visual regression harness constrain each other.** A temporal accumulator makes a single
  captured frame depend on the frames before it, so a reference image stops being reproducible from one
  render. Whichever lands second has to account for the first: either the harness renders a fixed number
  of frames with a pinned jitter sequence to convergence, or captures are taken with the temporal path
  disabled. The reason this is a note rather than a problem is that scene time is already an explicit
  frame input, so nothing in the renderer reads a clock.
- The composite's contrast-adaptive sharpen is *not* antialiasing and does not become part of this. It
  restores mid-contrast detail lost to texture filtering, and its own comment says as much; a sharpen
  applied after a resolution downsample or a TAA resolve may well want retuning, which is a tuning task
  and not a change of intent.
