# ADR 0006: Atmosphere — one environment, marched fog, procedural cloud shadows

- Status: accepted, implemented. **Decision 6 was reversed by the implementation** — see What implementing it established.

## Context

Cloud shadows, fog, and weather were wanted together, and each has an obvious independent implementation.
Taken independently they produce a renderer where an overcast sky is set in five places — a light preset,
two sky constants, a fog density, and a cloud coverage — that nothing keeps in agreement.

They are not independent. An overcast sky is dimmer in the beam *and* brighter in the ambient *and*
greyer *and* foggier, because all of those are one cloud deck seen from different angles. A designer
asking for "storm" is asking for one thing.

## Decision

1. **One [`Environment`] derives all of it**, from two authored numbers: an hour of the day and a
   [`Weather`] state. Sun direction, sun colour, ambient, sky colours, fog colour and density, and cloud
   coverage all follow.
2. **Weather is a set of blendable scalars, not an enum of presets** — `overcast`, `wetness`, `snow`,
   `flash`, `wind`. Presets are constructors over those scalars.
3. **Fog is marched along the view ray in six taps**, with an exponential height falloff per tap,
   applied inside the lighting and water shaders. Volumetric light shafts - which need the shadow map
   sampled per step - are declined.
4. **Cloud shadows are procedural gradient noise sampled in world space**, attenuating the sun's direct
   term only.
5. **Weather's surface response is applied to the G-buffer in the lighting pass**, not in the terrain
   and model shaders that wrote it. Wetness darkens albedo and drops roughness; snow blends toward a
   snow surface by *slope*.
6. **The environment's sun is opt-in.** `DeferredFrame::light` stays authoritative. — **Reversed by the
   implementation; see [below](#what-implementing-it-established).** The sun is now derived by default
   and `light` is the override.
7. **Airborne precipitation is out of scope.** It is a particle system, which M3 defers.

## Rationale

**Fog is integrated along the ray, not sampled at the fragment.** Sampling at the surface gets the
common case visibly wrong: a valley floor seen from a hilltop would be as clear as the hilltop, because
nothing accounts for the dense air the ray crossed on the way down.

This began as a closed form, which is exact while the density varies with height alone. Adding
patchiness made it vary laterally as well, and no closed form survives that, so it became a six-tap
march. The first patchiness attempt kept the closed form and merely scaled it by one noise sample at the
ray's midpoint. That cannot work for a reason independent of tuning: multiplying a smooth field by a
mildly varying one leaves it smooth. Three rounds of raising patchiness, shrinking the scale, and
lowering the density to escape the exponential's saturation all produced the same uniform wash before
the cause was clear.

**Patch scale must be large, which is the reverse of what the single-tap version wanted.** Because the
density is integrated, a small scale averages several banks per ray and neighbouring pixels agree.

**Fog is applied in the shaders, not as a depth-based post pass.** Water writes no depth — deliberately —
so a post pass would fog it at the depth of the terrain behind it, putting water in front of its own fog.

**The fog colour is derived from the sky's horizon colour rather than authored.** Fog fades distance
toward whatever is behind it, and behind it is the sky. A fog colour that disagrees puts a band along the
horizon exactly where the terrain silhouette meets it. A test pins the shared constant across the
Rust/WGSL boundary, because nothing else would catch them drifting.

This is the decision [ADR 4001](4001-hdri-sky.md) had to satisfy rather than sidestep. A captured sky
makes both of the constants this model derives from — the horizon colour here and `SKY_NEUTRAL` for the
ambient — stop describing what is on screen, so it re-derives them from the image instead of binding a
photograph behind a scene lit by something else.

**Cloud shadows attenuate the direct term only.** A cloud occludes the sun's disc, not the sky. Taking
ambient down with it makes cloud shade read as a hole in the world rather than as an overcast patch.

**Coverage moves a threshold rather than scaling a density.** Scaling darkens every pixel by the same
factor, which is a brightness slider wearing a cloud's name — and it would satisfy any assertion that
merely asked whether the frame got darker. Thresholding grows patches outward from where they already are.
The test asserts the *variance* of the per-pixel drop for this reason.

**Gradient noise, not value noise, and it took three attempts to see why.** Value noise stores a scalar
per lattice point, so inside each cell the field is a bilinear patch — and coverage thresholds that field,
turning the patches' contours into visibly straight edges meeting at cell corners. Rotating the octaves
removed the axis-aligned steps but left angular facets; a quintic interpolant fixed the creases *between*
cells without changing the field inside them. Gradient noise is zero at every lattice point and carries a
random direction instead, so there is no stored value to leak into the contours. Each attempt looked
plausible in code and was rejected by looking at the capture.

**The surface response acts on the G-buffer, which is why it costs one implementation instead of two.**
Terrain and models write albedo, normal and roughness into the same targets, and those three are exactly
what wetness and snow modify. Applying it at the source would mean the same logic in two shaders reading
two different uniform blocks, with nothing keeping them in step.

**Wet ground is darker and smoother, not bluer.** Water fills the pores, so less light scatters back out
and what does reflect leaves more coherently. Darkening is the larger half of the cue: a wetness that
only dropped roughness reads as a polished floor, which is why the test asserts the *mean* drop.

**Snow settles by slope, not by altitude.** `normal.z` is the cosine of the surface against vertical,
which is the physical criterion. An altitude threshold puts snow on a sheer cliff face high up and none
on the valley floor beside it — precisely backwards. The test measures the *spread* of the per-pixel
change rather than its mean, because snow covering everything uniformly would brighten the frame just as
much and satisfy any assertion about the average.

**Emission takes its hue from the material, not the weathered surface.** A lamp under snow is still a
lamp; snow on its housing should not recolour the light coming out of it.

**Lightning lifts ambient, never the beam.** A discharge across the whole sky has no position, so adding
it to the directional term would cast a hard shadow from a source that does not exist.

**Overcast raises ambient while lowering the beam.** A cloud deck is a diffuser: it moves light out of the
beam and into the sky rather than removing it. Dimming both is what makes an overcast noon read as dusk.

## Consequences

- Every default is the state that changes nothing — clear, fogless, cloudless. That is what let all five
  committed reference captures stay **byte-identical** through this change, which is the only evidence
  that the new plumbing did not quietly alter the lighting passing through it.
- `SceneCamera` grows five vectors, from 304 to 384 bytes — fog colour and density, fog falloff and
  patchiness, cloud parameters, cloud drift, and the surface weather. One uniform and one bind group still
  serve every pass, and a test pins the size because a mismatch does not fail validation: it silently
  misaligns every field past the drift.
- `Weather::wetness` and `Weather::snow` are read by the lighting pass and verified by capture: snow is
  visibly held off the spire's flanks and the ridge's steep face while covering the plain, and wet ground
  darkens across the frame.
- ~~Time of day derives a sun that nothing uses by default. That is deliberate, but it does mean a caller
  wanting a day cycle must assign `Environment::sun_light()` to the frame themselves.~~ **No longer true**,
  and it is the consequence the reversal of decision 6 was aimed at: the frame derives its own sun and a
  caller wanting a day cycle does nothing at all. Struck rather than deleted, because the burden this
  bullet describes is exactly the argument for the reversal.
- No latitude, date, or north reference: the sun is a half-sine over a fixed civil day, because no map
  format carries the inputs a real solar position model would need.
- Cloud shadows cost seven gradient-noise evaluations per lit pixel — five octaves plus two for the warp —
  in both the lighting and water passes, and are skipped entirely at zero coverage. Fog costs two per tap
  over six taps, using a cheaper two-octave field for exactly that reason: marching the cloud field would
  have cost forty-two.
- **The cloud lattice was a hash fault, and two earlier fixes treated symptoms.** `fract(sin(dot(p, k)) * c)`
  is a function of a linear combination of the coordinates, so every cell on a line perpendicular to `k`
  gets a correlated value — the field is streaked before any interpolation happens. Rotating the octaves and
  moving to gradient noise both improved the interpolation and could only ever soften it. Integer bit mixing
  has no preferred direction and removed it outright. Worth recording because the wrong diagnosis was
  plausible twice.
- **Height fog is verified; banks are still marginal.** The first fixture was a broad, gentle,
  near-planar basin seen from a distant camera, and it could not show height fog however it was tuned:
  every ray had much the same length and crossed much the same air, and an integral along the ray smooths
  away whatever the density does. Four rounds of tuning failed against a fixture fault. Moved onto the
  spire-and-ridge terrain it works immediately — the plain pools while the spire and the ridge stand out
  of it. A second arithmetic trap sat behind the first: with the camera 614 units up and a 52-unit fog
  layer, `exp(-(614-30)/52)` is about 1e-5, so the rays passed through almost no fog at all. The layer has
  to be thick and dense *relative to the camera height*, not to the terrain. Patchiness contributes
  visible variation but not crisp banks; crisp banks want a lower camera or a denser march.

## What implementing it established

**Decision 6 was reversed, and it was the right reversal.** The record said the environment's sun is
opt-in and `DeferredFrame::light` stays authoritative. The implementation does the opposite:
`DeferredFrame::new` derives its light from the environment's hour, `in_environment` re-derives it, and
an explicit `light` assigned *afterwards* is what now reads as the deliberate override.

The argument that turned it round is one the original decision did not consider. Leaving the two
independent does not produce flexibility, it produces a default that is quietly wrong — an environment
carrying a 6 a.m. hour beside a light still pointing where it did at noon is not a configuration anyone
wants, and every caller changing the time of day has to remember to update both or ship a frame whose
sky and shadows disagree. Opt-in defaults are correct when the derived value is *a* reasonable answer;
here it is the *only* consistent one, so the burden belongs on the caller who wants them to disagree.

The derivation is calibrated against `daylight_with_occlusion`, the hand-tuned preset it replaced as the
default, and a test pins it there — so the reversal did not quietly change what a frame looks like.

**And the calibration caught a derived sun that looked right and was not.** It matched its preset's
colour exactly while sitting 27 degrees away in azimuth, which no assertion over the two colours would
have found. That is the standing rule about looking at the capture, arriving in this record's subject
area rather than in a shader.
