# ADR 4002: Three kinds of water — a spread wave train, kind presets, and normal level-of-detail

- Status: accepted, implemented

## Context

The water pass shipped with one surface. Every body on every map was five sinusoids at one steepness,
spread by the golden angle over the whole circle, over one green-blue tint and one depth ramp. A lake, a
river and a stretch of sea were the same water at different sizes.

Two faults, and they arrived from opposite directions.

**The train read as a pattern.** Five components summed have a short beat, and the committed capture of a
still lake showed unmistakable diagonal banding across the whole body. This is the third time the wave
sum's *structure* has been the visible fault rather than its parameters: the first version used
wavelengths of 1, ½, ⅓ and ⅙ and produced a diamond grid, the second stepped them by a flat 0.61 and left
the pair 1 and 0.372 near enough to 8:3 to beat across a map. Each fix was found by looking at a capture.

**The surface sparkled, and that one was latent for months.** The shading normal is the analytic
derivative of the wave sum, taken at full strength however little of the train one pixel covers. The
Fresnel term is violently sensitive to it at a grazing view: near the horizon a six-degree change in a
wave's slope moves the reflected share from about 0.19 to 0.72, so neighbouring pixels alternate between
the reflected sky and the dark water body and the surface reads as scattered dark speckle. Nothing about
this was new. It was invisible while the sky was the analytic two-colour gradient, which is nearly as dark
as the water body — there was nothing to alternate *with*. [ADR 4001](4001-hdri-sky.md) added captured
skies, the reflected term became several times brighter, and the fault appeared in the reference capture
of a scene whose shader had not changed.

The two are one change because they are the same question asked at two scales: what should be in the wave
train, and how much of it should survive to a pixel.

## Decision

1. **Nine components rather than five, in irrational powers of the golden ratio.** Wavelength shares are
   `golden^(-index/2)`, so the ratio between *any* two is irrational and no two ever line up again after
   the origin. The band spans about 7:1.
2. **Amplitudes are derived from the wavelengths rather than tabled beside them.** Each component's
   amplitude is `steepness * wavelength` for one shared steepness, which is the reciprocal of the shares
   summed. Constant steepness across the train was already the intent; it is now structural rather than a
   property two tables have to keep agreeing about.
3. **Directions lean off a heading by a low-discrepancy fraction of a spread half-angle**, rather than
   stepping around the whole circle. Phases are offset by a second, different irrational.
4. **The spread is what a kind mostly is.** A river is nearly collimated, an ocean swell fans out about
   the wind, and a lake is isotropic.
5. **Each component is a second-order Stokes wave, not a sine**, with the harmonic coefficient exposed as
   a peaking in `0..=1` — one being the coefficient at which the trough goes exactly flat.
6. **The whole surface is advected by a current**, separately from the speed its waves travel at.
7. **Shore foam**, keyed on depth and gated on the wave train's own height.
8. **`WaterKind` names the three bodies and resolves to a whole `WaterMaterial`.** It is a selector, not
   stored state, and nothing downstream branches on it.
9. **Each wave component's contribution to the shading normal is damped by how much of its own wavelength
   one pixel covers**, from a quarter to a half of it. The footprint comes from `dpdx`/`dpdy` of the
   interpolated world position. Height is not damped: geometry is not a shading question.

## Rationale

**Nine and not more.** The cost is nine sines and nine cosines per vertex and per shaded pixel, against
two texture loads and a shadow lookup the same fragment already pays for. The count was raised until the
beat stopped being visible in a capture and not further.

**Golden-ratio powers rather than another arithmetic step.** The property wanted is that no pair of
components has a rational wavelength ratio, and taking powers of one irrational gives it for every pair at
once rather than for neighbours only. This is the same family of argument as the golden-angle direction
step it replaces, applied to the axis that actually beat. Low-discrepancy sequences of this kind are
standard — Roberts' generalisation of the Kronecker sequence is the usual reference — and they are used
here for the same reason they are used for sampling: they fill an interval about as evenly as anything can
without ever repeating.

**A second-order Stokes wave is the cheapest honest way to peak a crest.** Adding the first harmonic at
`-q·cos(2φ)` sharpens the crest and flattens the trough, which is what a steep gravity wave does; it is
the classical second-order solution (Stokes, 1847) and not an ad-hoc curve. Two properties made it the
right shape rather than merely a plausible one: it shifts neither the mean surface nor the crest-to-trough
height, so `wave_height` still means what it says, and its derivative is one more term rather than a
special case. At `q = 0.25` the trough curvature is exactly zero; past that a trough grows a dimple in the
middle, which no water does, so the exposed parameter is scaled to reach that limit at one.

The alternative was a Gerstner wave, which is the more usual choice and gives genuinely cusped crests. It
was declined because it displaces horizontally as well as vertically, so the analytic gradient stops being
the derivative of the height and needs the inverse of a Jacobian — a real cost in the one function both
stages call, for a difference that shows at the water's edge and nowhere else.

**A current is separate from the wave speed because they are different physics.** Chop travels *over* water
that is itself moving. Folding them into one figure gives a river whose waves run downstream over water
that stands still, which reads as a texture scrolling across a lake — and a river is the one kind where a
player will notice, because a river's whole identity is that it goes somewhere. Implementing it is a
translation of the sample position, which leaves the gradient alone and therefore costs nothing.

**Foam earns its place by how much of a kind it carries.** An ocean breaking on a beach is mostly read from
its surf, and the term is a few lines because `depth` is already in hand and the wave sum is already a
noise field. Gating on the crest rather than on depth alone is what makes the band pulse along a shore
instead of ringing it evenly.

**`WaterKind` resolves to a material rather than being carried alongside one.** A kind stored next to the
numbers is a second source of truth the numbers can drift away from: a `River` whose spread has been
widened to a lake's is not a river, and no assertion can say which of the two fields was meant. Naming a
kind is how a map author or a scenario format will ask for water; the material is what the renderer has.
Every field stays public, because the presets are a starting point and not a policy.

**Normal level-of-detail per component, not per gradient.** Damping the whole gradient by the shortest
component is simpler and wrong in a way that shows: the shortest wave and the dominant one differ by seven
times in wavelength, so they stop being resolvable seven times apart, and one scalar keyed to the short
chop flattens a swell that is still perfectly sampled. Per component the two fade where each of them
actually should. This is normal-map filtering applied to an analytic surface — the argument is Toksvig's
(2005) and LEAN mapping's (Olano and Baker, 2010): a normal that a pixel cannot resolve must not be
shaded with, because the BRDF turns unresolved geometry into noise rather than into an average.

**The fade runs from a quarter of a wavelength to a half, which is more aggressive than Nyquist.** Nyquist
puts the limit at half a wavelength per pixel and a box filter still passes about 0.64 of a component's
amplitude there. Damping to zero by that point is deliberate: the shading is *nonlinear* in the normal —
a fifth power in the Fresnel term and an exponent in the hundreds in the specular — so the shaded result
carries far higher frequencies than the surface does, and a component sampled at the geometric limit
aliases badly after the BRDF. The band was widened toward Nyquist during implementation and the speckle
came back.

**`dpdx` after `discard` is sound, and was the open question.** The pass discards three times before it
shades — a depth test it performs itself, a coverage test, and the shoreline clip. A `discard` in WGSL
demotes the invocation to a helper rather than branching around what follows it, so the derivative is
still in uniform control flow and every lane of the quad still carries a world position. This is the same
reason an alpha-tested pass may sample a mipped texture after its own cutout test. naga accepts it, which
settled the question the fallback — reconstructing the footprint from view distance and the projection —
existed for.

**The wider of the two screen derivatives, not their average.** At a grazing view one axis is many times
the other, and averaging leaves the long axis aliasing — which is the axis the sparkle was on.

## Consequences

- **Every committed water reference moved**, on every adapter, and three new scenes were added. The wave
  train, the tints and the shading all changed; that is what the record is about.
- **The uniform block grew from five `vec4`s to seven.** Both the shader struct and the packing assert the
  size, because a mismatch does not fail validation — it silently misaligns every field past the drift.
- **`WaterMaterial::default()` is now a lake** and its figures are not the old shared ones. Callers that
  took the default and overrode a field or two keep compiling and render differently, which is intended:
  the old figures were a compromise across three kinds and gave every pond an ocean's swell.
- **The reflection cone is now doing double duty and is right for it.** It is built from the material's
  whole slope RMS and never consults the shading normal, so the variance the level-of-detail removes from
  the geometry is exactly the variance that term already spends widening the lobe. A distant surface comes
  out flat-but-rough, which is what water at that distance is.
- **What the damping cannot recover is the Fresnel term's own average.** The reflected share is convex in
  the incidence angle, so a rough surface's mean Fresnel is higher than the flat-plane value the damped
  normal converges to, and distant water is therefore fractionally under-reflective. It is a bias of a few
  percent against an artefact that was the most visible thing in the frame, and correcting it needs a
  roughness-aware Fresnel this pass does not have.
- **A river is one body per reach.** The flow is a vector rather than a heading and a speed precisely so a
  spline-authored channel resolves to a chain of them, each carrying its own tangent. Nothing authors that
  chain yet.
- **No package integration.** A map cannot yet name a kind, for the same reason a scenario cannot yet name
  a sky.

## What implementing it established

**Two of the three faults this record fixes were invisible until something else changed.** The sparkle
needed a bright sky to alternate with; the banding needed a reference capture anyone looked at. Neither
had a failing assertion, and the sparkle survived being investigated *as a sky bug* — forcing the
reflection to a mip level where the environment is a flat wash and finding the speckle still there is what
proved it was the Fresnel term.

**A camera can hide a water change completely, and this took three captures to accept.** At ten degrees
above a surface the reflected share is over two thirds everywhere, so all three kinds render as the same
sky and their tints contribute almost nothing — the first side-by-side capture had a river and an ocean
1.9 apart in 255 while looking nothing alike. The same pose flattens the wave train for an unrelated
reason: the footprint along the view runs as the reciprocal of the sine of the pitch, so a shallow camera
puts three world units in a pixel and the level-of-detail correctly removes six of the nine components.
The fixtures now carry two poses whose pitch is chosen rather than inherited, and the second one is
documented with the arithmetic.

**A mean colour is the wrong statistic for "these are different waters".** An ocean runs from turquoise
over a bank to near-black in the middle and averages to very nearly the flat mid-blue of a lake. What
separates the kinds is *where* the colour is, so the assertion compares captures pixel by pixel and asks
separately whether the ocean's own range is wider than the lake's.

**A river needed its own fixture.** A body laid over a bowl clips to a disc however its material is set,
and a disc of moving water is a lake with a grain in it. What makes a river read as a river is that it is
narrow and continuous, and that is a property of the bed rather than of the water — so the test cuts a
meandering channel and lays a map-sized body over it, which is exactly what the shoreline clip is for.
