# ADR 4001: Captured skies — Radiance `.hdr`, an equirectangular lookup, and the light derived from it

- Status: accepted, implemented

## Context

Until this record the sky was two `vec3` constants in a shader and a vertical gradient across the frame.
That is a defensible sky for a strategy map seen from a fixed high angle, and it is what every committed
reference capture was rendered through.

Three things it cannot do, and each of them is something a scene wants rather than something a renderer
wants:

- **It does not respond to the camera.** The gradient runs from the top of the *frame* to the bottom, so
  pitching up brings no more zenith into view. Nothing in the shot says where the horizon is.
- **It has no content.** A reflection of it is a smooth ramp, which is why the water surface has always
  read as tinted glass rather than as water: there is nothing in the sky for it to be a mirror of.
- **It cannot be authored.** A designer asking for "the light at that place, at that hour" is asking for a
  measurement, and the only knobs available are an hour and five weather scalars.

An HDRI answers all three and brings one problem with it, which is most of what this record is about.
[ADR 0006](0006-atmosphere.md) makes a hard claim: the sun, the sky, the fog and the ambient are one
thing seen four ways, and they are *derived* from two authored numbers precisely because five numbers
holding one idea drift apart. Putting a photograph behind the scene while `SKY_NEUTRAL` still drives the
ambient and `SKY_HORIZON` still drives the fog reintroduces exactly the failure that record exists to
make inexpressible — an orange sunset overhead and blue-grey shade on the ground beneath it.

## Decision

1. **Radiance `.hdr`, read by a small decoder of this project's own.** Not OpenEXR, not a sixth
   `BlockFormat`, not a new container. See [`docs/formats/sky.md`](../formats/sky.md).
2. **Equirectangular, sampled by world direction, not a cube map.** Longitude into `u`, polar angle into
   `v`.
3. **Uploaded as `Rgba16Float` with a CPU-built mip chain that wraps in longitude.**
4. **The sky is a bind-group provider — group 3, its own WGSL chunk** — bound by the two passes that ask
   a direction what colour it is, and by nothing else. The analytic sky is the same chunk's other branch,
   not a fallback bolted beside it.
5. **The image replaces the ambient and the fog colour, and only those.** Both are re-derived from it on
   the CPU at load, through [`SkyLighting`].
6. **The image does not replace the sun.** The directional light stays derived from the hour. What closes
   the gap is [`Sky::aim_at`], which turns the *image* until its own sun sits at the light's azimuth.
7. **The sun is clamped out of the derived light.** A texel more than eight times the sky's own mean is
   treated as the sun and scaled down before the integral.
8. **An oversized image is reduced while it is read, not refused.** The size a sky is read at is a
   target; the refusal bound sits far above it.
9. **Everything is opt-in and the default is unchanged.** No image bound means the analytic branch, byte
   for byte.

## Rationale

**Radiance rather than OpenEXR, and rather than BC6H.** Every HDRI a content author will actually find is
distributed as `.hdr`. The format is a text header, a resolution line, and run-length-encoded RGBE
scanlines — two hundred lines to read, with no dependency, and the tests build their fixtures
arithmetically so nothing in the tree carries someone else's photograph. OpenEXR is the better format and
needs a library, a tile reader, and three compression codecs before it decodes a pixel.

BC6H is the block format that would have fit, and it is the interesting rejection because
[ADR 2001](2001-block-compressed-textures.md) argues hard for block compression everywhere else. The
argument does not carry here. Block compression pays because a map has *hundreds* of surface textures, all
sampled at once, all competing for the same cache. A scene has **one** sky. Writing a BC6H encoder — mode
selection over fourteen modes with signed and unsigned endpoint variants — is a project, and it would buy
sixteen megabytes on a budget that has hundreds.

**RGBE is a good fit rather than merely a convenient one.** Four bytes per texel with one shared exponent
across three channels is roughly the precision the eye can use from a sky, and a quarter of what three
half floats would cost to read.

**Eight bits per channel is not a smaller version of this; it is a different thing.** A surface albedo is a
reflectance and is bounded in `0..=1` by physics, so an integer format loses nothing an albedo had. A sky
is radiance, and the ratio between a sun disc and a shadowed cloud is four or five orders of magnitude.
Everything a captured environment contributes — the colour of the bounce, the brightness of a reflection,
the shape of a highlight — lives in the part of the range an 8-bit encoding discards. A tone-mapped sky
reflected in water is a grey card.

**Equirectangular rather than a cube map**, because that is what the content is and because the lookup is
two inverse trigonometric calls instead of a face selection. The cost is the pole distortion the
projection has, and it lands where it matters least: the zenith is a slowly varying gradient, and the
horizon — where an equirectangular image's texels are densest — is what a ground-level camera looks at.

**`Rgba16Float` rather than `Rgba32Float` or `Rgb9e5Ufloat`.** The requirement is a *filterable* format
carrying values well above one, because a sky is magnified enormously across a background. `Rgba32Float`
needs the optional `FLOAT32_FILTERABLE` feature to be sampled smoothly at all, at twice the size of
something that works. `Rgb9e5Ufloat` is RGBE's own layout at four bytes a texel and is the closest fit on
paper; it is declined for backend mileage rather than for any property, since this renderer already ships
`Rgba16Float` targets on every adapter it is tested against. Eleven bits of mantissa against RGBE's eight
means the conversion loses nothing the file carried.

**The mip chain wraps in longitude, which no hardware generator does.** The left and right columns of an
equirectangular image are adjacent *directions*. A clamping reduction averages the last column against
itself and leaves a seam of wrong texels down the meridian, wider at every level.

**A provider group rather than two more bindings on the scene.** Group 0 is the G-buffer and the camera,
and five programs bind it whether they read every entry or not. An environment texture there would hand
the composite and both antialias resolves a sky none of them samples — which is precisely the mistake the
shadow cascades already made, and were moved out of group 0 to fix. The sky follows the shape that fix
established, and it validated a second prediction with it: `reflection.wgsl` was written as the seam a
non-analytic sky would be substituted at, and wiring an HDRI took editing that one function. The water
pass reflects an environment without knowing one exists.

**The mip level is chosen from an angle, not from a roughness, and that was forced by a capture.** The
first implementation passed the surface's roughness and mapped it to a level. It rendered a lake as
**coloured speckle** — neighbouring pixels taking single texels ten degrees apart, one off the orange
horizon and the next off the blue zenith. The cause is that roughness is not what widens a water
reflection; the *wave slope* is, and it is far larger. A lake's material roughness is small — that is why
it mirrors — while its surface tilts by several degrees and a mirror doubles any angle it reflects. So
the caller states the half-angle its pixel actually reflects into and the sky converts that to a level,
which is a question the caller can answer and the sky cannot.

**Decision 8 breaks this crate's own rule, and the content is why.** Every other decoder here refuses
what crosses a bound, which is right for a mesh or a texture — nothing sensible can be done with half of
one. A sky is the case where it is wrong: HDRIs ship at 8K by convention, and 8K is more resolution than
a sky can *use*. One texel covers half a pixel at the horizon and costs 358 MiB of video memory for a
picture that is out of focus anyway. A bound that refused the ordinary size of the ordinary asset would
be a bound nobody could keep, and the workaround — "convert it yourself first" — is a step every content
author would have to be told about individually.

Reducing while decoding costs almost nothing to do properly, because the reader already walks one
scanline at a time: the oversized buffer is never allocated, so the file that motivated this reads in
280 ms and peaks at 34 MiB rather than 536. The refusal bound stays, at 16384, because a hostile file
declaring 200000 texels wide is a different thing from a large one.

**Deriving the ambient and the fog from the image is not image-based lighting, and is not meant to be.**
There is no specular prefilter and no irradiance in any direction but up. It is the three numbers
[ADR 0006](0006-atmosphere.md)'s model already consumed, taken from the new source of truth instead of
from two hand-tuned constants that no longer describe what is on screen.

**Not taking the sun from the image is the least obvious decision here and the one most likely to be
re-proposed.** Fitting a directional light to an environment is a real estimator — locate the brightest
solid angle, integrate over it — and it fails on the ordinary case. An overcast HDRI has no sun, so it
yields a direction picked out of noise, and a scene whose shadows point somewhere arbitrary for reasons
nobody can trace. The half-sine over the hour is wrong in a way a designer can predict and correct; a
fitted sun is wrong in a way nobody can. Rotating the image to agree with the light closes the same gap
from the end where failure is visible: if the rotation is wrong, the bright patch of sky and the shadows
disagree in one frame, and a person can see it.

**The sun clamp is a statement about division of labour rather than about the image.** A calibrated
HDRI's sun disc covers about a five-thousandth of the hemisphere at four or five orders of magnitude the
surrounding radiance, which works out to most of the irradiance. That is physically correct and it is the
wrong number to hand a renderer that *already has a directional light for that sun*. Adding it counts the
sun twice, and the visible result is not a brighter scene — it is a scene with no shadow contrast at all,
because the ambient becomes as strong as the beam. The ceiling is relative to the image's own mean so it
means the same thing for a file in physical units and one in arbitrary ones; eight sits clear of a bright
cumulus edge at two to four times the mean and clear of a sun at thousands.

**A captured sky is not dimmed again by the hour.** The elevation falloff in `Environment::sun_light`
models a sky that dims as its sun drops. An image already *is* the sky at whatever hour it was taken, so
applying the falloff over it darkens a photographed dusk twice — once for being a dusk, once for being at
18:00. Overcast and lightning still apply on top, because both describe something in front of the sky
rather than a property of it.

## Consequences

- **`DeferredRenderer::render` takes one more argument**, and a [`Sky`] is owned by the caller rather than
  by the renderer — the same shape a `WaterBody` has, for the same reason. A resize rebuilds the renderer
  along with every bind group holding a view of a resized target, and an environment is neither, so owning
  it there would silently drop it when the window changed size.
- **Every committed reference capture is byte-identical**, which is the only evidence that the new
  plumbing did not alter the lighting passing through it. Two references were added rather than changed.
- **`Environment` grows an `Option<SkyLighting>`**, and it is `None` by default for the reason above.
- **The analytic background stays a screen-space gradient**, deliberately. Changing it to a direction
  lookup would alter every committed reference at once, which would have destroyed the evidence in the
  bullet above at exactly the moment it was needed. So the renderer has two backgrounds: a gradient that
  does not respond to camera pitch, and an environment that does. That is a real seam and the record is
  the place to say so.
- **The reflection's blur is a plausible width, not a correct shape.** The chain was built by halving the
  image rather than by convolving it with a reflection lobe, and an equirectangular halving spans a much
  wider solid angle near the poles than at the horizon. A prefiltered environment would replace this
  rather than tune it.
- **A metal still reflects `albedo * ambient` rather than the image.** With the ambient now measured off
  the sky the two agree in colour and disagree in detail, which is a defensible state and not the end
  state. `reflection.wgsl` records what the second edit would be.
- **A captured sky made the water pass's normal aliasing visible**, and this is the unwelcome consequence
  worth naming. At a grazing view a six-degree change in a wave's slope moves the Fresnel share from 0.19
  to 0.72, so neighbouring pixels alternate between the reflected sky and the dark water body and the
  surface sparkles with dark pixels. The aliasing is pre-existing; the analytic sky hid it by being nearly
  as dark as the body, so there was nothing to alternate *with*. The fix is to damp the wave normal by the
  pixel's footprint, which changes how water shades and moves every committed water reference — so it is
  its own change rather than a rider on this one. The committed `water-sky-captured.png` shows it.
- **A sky costs about 21 MiB** at 2048x1024 with its chain, uploaded once when a scene loads, and nothing
  per frame but a sixteen-byte uniform when it is rotated. That figure is fixed rather than a property of
  the file, which is what decision 8 buys: an 8K source and a 2K one both arrive as the same texture.
- **`SkyLimits` has a bound that does not mean what the others mean**, and a reader who assumes the
  crate's uniform rule will get it wrong. `target_dimension` reduces; `maximum_dimension` refuses. The
  type's own documentation says so, because the field name alone cannot.
- **No package integration.** A scenario cannot yet name its sky; a host loads the file and hands the
  renderer a [`Sky`]. That is a scenario-format decision — a field in `map.json`, a directory convention,
  and the documentation for both — and it is deliberately not folded in here.

## What implementing it established

**Decision 3's mip chain and decision 4's provider group were both settled by looking at captures, and
each first attempt was plausible.** The chain was reduced with the general-purpose clamping reduction
before the meridian seam appeared in a rotated frame; the reflection was selected by roughness before the
speckle appeared in a lake. Neither had a failing assertion, and the second one took a deliberate
experiment to diagnose — forcing the reflection to the smallest mip level and finding the speckle *still
there* is what proved it was the Fresnel term rather than the sky lookup.

**A camera that is right for shadows is useless for a sky, and that also took a capture.** The deferred
test fixture's camera is pitched about twenty-six degrees down and the projection's half angle is a little
under that, so its topmost row sits within a degree of horizontal — every background pixel it captures is
*below* the horizon. The first run of the sky tests rendered the environment's lower hemisphere across the
whole background and read as a flat grey lid, which looks exactly like a texture that failed to bind. The
fixtures now carry a second pose whose only job is to have a horizon in it.
