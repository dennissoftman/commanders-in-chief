# ADR 0006: Atmosphere — one environment, marched fog, procedural cloud shadows

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
3. **Fog is marched along the view ray in six taps**, with an exponential height falloff per tap,
   applied inside the lighting and water shaders. Volumetric light shafts - which need the shadow map
   sampled per step - are declined.
4. **Cloud shadows are procedural gradient noise sampled in world space**, attenuating the sun's direct
   term only.
5. **The environment's sun is opt-in.** `DeferredFrame::light` stays authoritative.
6. **Airborne precipitation is out of scope.** It is a particle system, which M3 defers.

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
- **Fog banks are not visually verified.** The marched implementation is better founded than the closed form
  it replaced, and fog demonstrably fills a basin while the rim reads through it. But the basin fixture is a
  broad, gentle, near-planar surface seen from a distant camera, so every ray has much the same length and
  direction and the integral smooths out whatever the density does. Showing banks needs a scene with real
  depth structure — a ridge standing out of the fog, objects at differing distances — which is a fixture
  this milestone does not have. Treat patchiness as implemented and unproven.
