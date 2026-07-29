# ADR 0005: Antialiasing strategy, and why not MSAA

- Status: accepted and **fully implemented**. All five decisions are in.

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

  **Resolved by the first of the two options.** The jitter phase became a frame *parameter* —
  `DeferredFrame::jitter` — alongside scene time, for exactly the reason scene time is one. The harness's
  `render_converged` then renders a full eight-phase cycle and captures the last frame, and two calls
  produce the same bytes. It needed one thing the design did not originally have: a way to say *this frame
  does not continue the last one*. Without `DeferredRenderer::reset_history` the second sequence started
  from what the first left behind and the two disagreed — which is also the real-game case of a jump cut,
  so the API was missing rather than the test being wrong.
- The composite's contrast-adaptive sharpen is *not* antialiasing and does not become part of this. It
  restores mid-contrast detail lost to texture filtering, and its own comment says as much; a sharpen
  applied after a resolution downsample or a TAA resolve may well want retuning, which is a tuning task
  and not a change of intent.

## What implementing decisions 2 and 3 established

Recorded here rather than only in the code, because the first of these changed a pass this ADR said it was
not touching, and the second is the reason a reader should not trust a single number about aliasing.

**The sharpen did want retuning, and more than retuning.** Predicted above as a possibility; it turned out
to be a fault. The sharpen's amplitude deliberately backs off at hard edges and rises on soft ones, to
avoid ringing on silhouettes — and a supersampled silhouette is soft *by construction*, being the
sub-pixel coverage the downsample just produced. So the pass most amplified exactly what the resolution
scale had just fixed. Measured over the two-pixel band along the sky boundary of the shadowing-terrain
fixture, a 1.5x render carried **34% more** pixel-to-pixel step energy than a native one with the sharpen
active, against 4% more with it off. Halving the strength was tried first and left most of the excess,
because the amplitude term is not a scalar a scale can compensate for — it encodes a *rule* that a soft
edge wants sharpening, and that rule stops holding the moment the softness is the antialiasing. The
sharpen is therefore off above a scale of one and unchanged at or below it, which is what keeps every
committed reference byte-identical. This is still a tuning change and not a change of intent: the pass is
a correction for magnification, and it now applies only where there is magnification.

**No single statistic distinguishes aliasing from detail, and two attempts failed before one worked.** The
obvious measure — the mean absolute Laplacian of luminance, which is near zero on any smooth ramp and
large on a one-pixel step — reports that supersampling makes the frame *worse*. It is not wrong about the
number: a higher sampling rate genuinely puts more real high-frequency content into the image, and nothing
local can tell content that belongs there from a staircase that does not. Restricting it to the silhouette
removed that objection and left a weak instrument, moving 6% across the three settings while shading
variation covers more than that, because a box-averaged hard edge is still a two-pixel transition — merely
a more accurately *placed* one. What works is asking **where** a setting acts rather than how much:
supersampling moves the silhouette band by 0.0217 and the rest of the frame by 0.0006, a ratio of 36, and
an upscale is separated from it by the interior instead — 0.0024 there against 0.0006, because averaging
four samples of a smooth surface returns what was already there while magnifying from half as many does
not. The resolve pass, by contrast, *is* measured well by the Laplacian, and the reason is order: it is
the last thing to touch the image, so nothing re-hardens what it softened.

The general form of this is already in the milestone's design notes and this is another instance of it: a
statistical assertion cannot replace a reference image. The committed captures are what verify these two
settings; the numbers are the tripwire.

## What implementing decision 4 established

Recorded here for the same reason the section above exists: two of these changed things this ADR said it
was not touching, and one of them was a defect the whole reference set had been rendered through.

**The resolve passes were sampling half a pixel away from the fragment they were shading, and a temporal
accumulator is what made it visible.** `@builtin(position)` in a fragment stage is already the pixel
*centre* — the top-left pixel is at (0.5, 0.5) — and the composite, the post pass and the first draft of
the temporal resolve all added a further half pixel before converting it to a texture coordinate. Three
passes, one habit, and nothing caught it: every committed reference had been rendered through the same
offset, so the images agreed with each other.

What exposed it was a property no capture comparison can express. Accumulating a *static* frame must reach
a fixed point — the same image blended with a history that is already that image is that image. It did not:
successive frames differed by 48, 33, 19, 9 and 6 in the worst channel. That is a convergent sequence, and
it would have satisfied any tolerance stated as "settles"; only demanding a genuine fixed point
distinguishes an accumulation that is correct from one that is merely stable. The cause was each pass
reading its own history from half a pixel away and re-filtering it every frame.

The cost of the offset in the two older passes was worth measuring rather than assuming. In the composite it
was a half-pixel translation of every frame plus, at a resolution scale of one, a bilinear average of two
texels where the downsample is meant to return one exact texel — **1.5% of pixels differing by more than
two, with a peak channel difference of 154**, and the difference image is edges and nothing else. In the
post pass it made every tap a two-texel average, so the luma gate saw a gradient nearly everywhere and
fired on about 3% of the frame; with exact taps it fires on about 1% and still halves the silhouette edge
energy. The pass got more selective and no less effective, which is what a gate is for.

Ten of the twenty-two committed references changed. A textual test now pins the convention, because this is
the one class of error a reference comparison structurally cannot catch: it was applied to the reference and
the result alike.

**The motion target is written unconditionally, and that is a decision rather than an oversight.** The
alternative is a second G-buffer pipeline per geometry kind, differing from the first in one attachment, and
four pipelines to keep in step rather than two. Two channels of half float is 4.6 MB at 1920x1200 and one
more write per fragment, and nothing but the temporal resolve reads it — so with that resolve off the frame
is byte-identical to what it was before the target existed.

**Sway made the motion vectors exact rather than approximate, for free.** Reprojecting depth against the
previous view-projection is the cheap way to get motion and it is correct only for static geometry; scenery
sway landed in the same change, so there is dynamic geometry to be wrong about. It turned out to cost
nothing: the displacement is a pure function of scene time, so evaluating the same vertex function at the
*previous* time returns exactly where the vertex was. No per-vertex history buffer, and no way for the
motion vector to disagree with what the geometry did.

**The neighbourhood clamp is in YCoCg, and the reason is the shape of the box.** In RGB the three axes are
strongly correlated on real images, so an axis-aligned box fits a neighbourhood's actual distribution badly:
loose along the diagonal, where luminance error lives, and tight across it, where clipping a small hue
difference buys nothing. YCoCg decorrelates luminance from the two chroma axes and costs a handful of adds.

**Measured cost, at 1920x1200 on an RTX 4080 SUPER**, over a 257x257 heightfield with no models or water:

| Resolve | Frame | The resolve pass itself |
|---|---|---|
| None | 0.499 ms | — |
| Post pass | 0.527 ms | 0.027 ms |
| Temporal | 0.558 ms | 0.053 ms |

The temporal resolve is twice the post pass and a nineteenth of the frame, which puts it well inside the
"quality tier" this ADR called it. The number that matters more is the one that did not move: the G-buffer
is 0.078 ms with the motion target, against 0.077 ms measured for the post-pass configuration that never
reads it — the fourth attachment is inside the noise.

**The sharpen was left alone, and this time that is the finding.** The consequences section above predicted
a TAA resolve might want the sharpen retuned. It does not, because the order saves it: the sharpen runs in
the composite and the temporal resolve runs after, so nothing re-hardens what the accumulation softened.
The interaction that mattered was the opposite one — the sharpen was amplifying a half-pixel blur that
should not have been there at all, and removing the blur is what fixed it.
