# ADR 0006: Atmosphere — one environment, analytic fog, procedural cloud shadows

- Status: accepted

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
3. **Fog is analytic height-and-distance fog**, with the density *integrated along the view ray*, applied
   inside the lighting and water shaders. Volumetric light shafts are declined for now.
4. **Cloud shadows are procedural gradient noise sampled in world space**, attenuating the sun's direct
   term only.
5. **The environment's sun is opt-in.** `DeferredFrame::light` stays authoritative.
6. **Airborne precipitation is out of scope.** It is a particle system, which M3 defers.

## Rationale

**Fog is integrated along the ray, not sampled at the fragment.** Sampling at the surface gets the common
case visibly wrong: a valley floor seen from a hilltop would be as clear as the hilltop, because nothing
accounts for the dense air the ray crossed on the way down. The closed form is available precisely
*because* the falloff is exponential, so the correct version costs about what the wrong one does.

**Fog is applied in the shaders, not as a depth-based post pass.** Water writes no depth — deliberately —
so a post pass would fog it at the depth of the terrain behind it, putting water in front of its own fog.

**The fog colour is derived from the sky's horizon colour rather than authored.** Fog fades distance
toward whatever is behind it, and behind it is the sky. A fog colour that disagrees puts a band along the
horizon exactly where the terrain silhouette meets it. A test pins the shared constant across the
Rust/WGSL boundary, because nothing else would catch them drifting.

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

**Lightning lifts ambient, never the beam.** A discharge across the whole sky has no position, so adding
it to the directional term would cast a hard shadow from a source that does not exist.

**Overcast raises ambient while lowering the beam.** A cloud deck is a diffuser: it moves light out of the
beam and into the sky rather than removing it. Dimming both is what makes an overcast noon read as dusk.

## Consequences

- Every default is the state that changes nothing — clear, fogless, cloudless. That is what let all five
  committed reference captures stay **byte-identical** through this change, which is the only evidence
  that the new plumbing did not quietly alter the lighting passing through it.
- `SceneCamera` grows four vectors, from 304 to 368 bytes. One uniform and one bind group still serve
  every pass.
- `Weather::wetness` and `Weather::snow` are carried and clamped but **not yet read by any shader**. The
  surface response is the next piece of work, not a claim this change already makes.
- Time of day derives a sun that nothing uses by default. That is deliberate, but it does mean a caller
  wanting a day cycle must assign `Environment::sun_light()` to the frame themselves.
- No latitude, date, or north reference: the sun is a half-sine over a fixed civil day, because no map
  format carries the inputs a real solar position model would need.
- Cloud shadows cost two gradient-noise samples per lit pixel in both the lighting and water passes, and
  are skipped entirely at zero coverage.
