# Current objective

## Where the project is

M0 through M2 are complete: the workspace and its invariants, the resource layer, and the native asset
formats. **M3's charter and its exit condition are both met** — the renderer draws a lit, shadowed,
occluded, textured scene with water and weather, both headlessly and in a window, and a visual regression
harness compares captures against committed references **on the CI runner**, so a rendering regression now
fails a build rather than only a developer's machine.

The last open line was **terrain level of detail**, and it is closed by amendment rather than by
implementation: frustum culling delivered what it was for, and the density half is deliberately not built.
The measurement is the reason and the decision is recorded with a date and an owner — see
[M3's charter](docs/milestones/m3-renderer.md#charter). What is *not* acceptable, and was true of this
document twice, is a line reading as though something were done.

**M4 is complete too.** A `cic-ui` crate holds the [layout format](docs/formats/ui-layout.md), a two-pass
solver, a string table, the closed action set, widget behaviour with retained state and input-method
composition, the screen stack with transactional settings, animated screen changes, and a paint layer;
`cic-render` draws it, with a typeface authored in this tree. **The shell navigates and it is on screen** —
five authored screens, covered by eight committed reference captures and driven by hand in a window. Its last
open charter line was `tabs`, which selected and switched nothing, and **tabs now switch pages**.

Two things have since landed past the charter, both because a settings screen needed them. A **`combo`** — a
real dropdown, which is the control a resolution list, a quality preset and a level-of-detail choice all want
— and it is the first widget here that breaks the assumption the flat solved sequence rests on, since an open
list is drawn over siblings authored after it. And the **keep-or-revert question is now a dialog** raised by
applying, rather than two buttons sitting inert on the screen: the settings screen has Back and Apply, and the
revert window running out closes the dialog itself, which is the case the whole mechanism exists for.

Adding the dropdown turned up something wider: **a hit test was comparing against where the layout placed a
node rather than where it is drawn**, so every control inside a scrolled container was clickable in the wrong
place. That is why a `list` could only be driven with the arrow keys. One field —
`SolvedNode::scroll_offset` — and the rule that a hit test uses `visual_rect()` while a scroll limit uses
`rect` closes it, and a list row is now chosen by pointing at it however far the list has scrolled.

**M9 and M10 have landed, and both were chartered late for capabilities that sit early in the dependency
order.**
Audio was simply missing — not deferred, not recorded anywhere as absent. `cic-audio`
([M9](docs/milestones/m9-audio.md)) is a mixer behind a replaceable backend, with positional audio, DSP,
layered music, and a [sound bank format](docs/formats/sound-bank.md), depending on no audio library of any
kind. The one thing it does not do is reach a speaker. The roadmap gained a *depends on* column, because
appending them at the end contradicted its own claim to be ordered by dependency.

`cic-script` ([M10](docs/milestones/m10-scripting.md)) puts scenario behaviour in data: a small language
compiled to bytecode, with a closed host surface resolved at compile time, no heap, and fuel-metered
execution. **Its arithmetic is [ADR 0007](docs/adr/0007-simulation-arithmetic.md)'s, unchanged** — an
earlier draft gave it a fixed-point arithmetic of its own, which was a mistake and is written up as one
in [ADR 7001](docs/adr/7001-scripting-language.md). The game verbs a scenario would call are blocked on a
simulation kernel that does not exist yet.

Landing M3's last five renderer items turned up a defect every committed reference had been rendered
through, so **ten of the twenty-two references changed** — see the antialiasing entry below.

What works:

- **Audio, from a cue name to mixed frames** ([`cic-audio`](crates/cic-audio/src/lib.rs)), with the
  replaceable half behind a trait and the default implementation written from scratch. **The boundary is
  the decision** — see [ADR 6001](docs/adr/6001-audio-backend-boundary.md). It is a *command* interface and
  not a sample sink, because FMOD and OpenAL are complete mixers rather than devices, and reducing either
  to a sink discards everything anyone would adopt it for. Engine policy — which cue, which recording,
  which bus, what priority, whether the budget allows it — is above the line and does not move when the
  implementation does.
  - **The in-tree mixer makes every assertion headless.** 125 tests, no device, at whatever sample rate the
    test picks — because a mixer is a pure function from voices and a listener to frames. The renderer
    needed a capture harness and per-adapter references to approximate that.
  - **The limiter needed a hold stage and a lookahead delay, and both were found by an assertion failing.**
    The textbook arrangement let a sine at eight times full scale out at **1.12**. Two causes: a rectified
    sine passes through zero twice a cycle, so the envelope sags 2.6 dB between peaks at 220 Hz, and the
    attack ramp is applied to a peak that has already left. It is now checked at five frequencies from
    55 Hz to 4 kHz, because the failure is worst at low ones and 220 Hz alone would have passed with the
    bug present.
  - **The listener is not the camera, and in this genre that is the whole problem.** A camera sixty metres
    up hears a firefight forty metres away as seventy-two metres away, and the pan collapses because
    everything is nearly straight down. `Listener::for_overhead_camera` lowers the ear toward the focus
    point while keeping the camera's orientation.
  - **The reverb's comb delays are prime**, which is the water-wave lattice failure in another domain: two
    combs at related lengths reinforce at every common multiple and the tail rings at one pitch.
  - **Gain ramps across a block while spatialisation is computed once per block.** Applying the new gain
    from the block's first sample is a step discontinuity at the block rate — at 512 frames, a 94 Hz buzz
    that follows the camera.
  - **Audio draws from its own random stream**, never a simulation one. Variant selection and pitch spread
    need randomness on every gunshot, and a stream consumed by presentation is a stream the other machine
    did not consume. [ADR 0007](docs/adr/0007-simulation-arithmetic.md) decision 9 leaves presentation
    otherwise unrestricted, so the mixer uses floating point freely.

- A `cic-assets` terrain uploads to the GPU and renders through a seven-pass deferred chain: four shadow
  cascades, a G-buffer, a half-resolution ambient-occlusion estimate with a bilateral upsample, deferred
  lighting that reconstructs world position from depth, a blended water pass, and a composite that tone maps
  and resolves the render resolution to the caller's. An eighth pass antialiases, when the display settings
  ask for it. The whole chain sums to **0.68 ms** at 1920x1200 on an RTX 4080 SUPER.
- Heights and layer weights live in *writable* textures with displacement and normals computed in the
  vertex shader, so terrain deformation and route grading are texture writes rather than a remesh.
- Instanced models share the terrain G-buffer and every shadow cascade, one draw call per model
  whatever its material count, with a per-instance transform and colour tint.
- Both surfaces are textured through one mechanism: a colour texture array per drawing unit, indexed by
  a slice number the material carries. Terrain layers tile their albedo in *world* space at a per-layer
  detail scale; model materials take their base colour from the images their glTF carried. Mip chains
  are generated on the CPU in linear light.
- **Water**, as a bounded plane with five summed directional waves, blended inside the HDR target so its
  glitter tone maps with the scene. Its shoreline is not authored: the pass discards wherever the bed
  rises through the displaced surface, so a rectangle plus a heightfield give an irregular shore that
  moves with the swell.
- **Shaders compose from named chunks** ([`shader`](crates/cic-render/src/shader.rs)). WGSL has no include
  mechanism, and without composition every pass needing the cascade selection had to share one file with
  it — which is how a single shader reached 620 lines. Fifteen programs are assembled from twenty-two chunks;
  a test fails if any chunk is named by no program, and three programs are marked `staged` so work held for
  a later milestone is distinguishable from dead code — the terrain, road and boundary viewer passes, held
  for M8's map editor. The mechanism has now done its job twice in the intended direction: `ui` was staged
  for M4 and `terrain_virtual` for the page cache, and each went live when the work that needed it landed,
  with nothing to clean up because neither had rotted.
- **An atmosphere** ([`environment`](crates/cic-render/src/environment.rs)) derived from two authored
  numbers, an hour and a weather state: the sun's direction and colour, ambient, sky, fog, and cloud
  coverage all follow from them. Weather is blendable scalars rather than an enum of presets.
  - **Cloud shadows** — procedural gradient noise in world space, domain-warped into wisps, attenuating
    the sun's direct term only and with a depth that varies with cloud thickness.
  - **Height and distance fog** — marched along the view ray in six taps, so a valley pools while a ridge
    stands out of it.
  - **A weather surface response** — wetness darkening albedo and dropping roughness, and snow settling by
    *slope* so it lies on flats and not on cliffs. Applied to the G-buffer in the lighting pass, so terrain
    and models both get it from one implementation.
- **Antialiasing, in its two cheaper tiers** ([`display`](crates/cic-render/src/display.rs)), with MSAA
  declined outright per [ADR 0005](docs/adr/0005-antialiasing-strategy.md) rather than left open. A
  **resolution scale** from a half to two multiplies every screen-space target and the composite's filtered
  read of the HDR target is the downsample, so it costs no extra pass — and it is the only control that
  raises the actual sampling rate, so the only one that touches geometric, texture, specular *and*
  occlusion aliasing at once. Beneath it a **post pass** written from scratch: a luma gate, a Sobel
  orientation test, and a blend weight from the second difference across the edge, so smooth ramps are
  untouched, a hard step is halved, and an isolated sub-pixel highlight is hit hardest. Both are one
  `DisplaySettings` value, because a settings screen presents them as one choice.
  - Two findings came out of it. The composite's sharpen was *fighting* the scale — it amplifies soft edges
    hardest, and a supersampled silhouette is soft by construction — so it is now off above a scale of one.
    And no single statistic separates aliasing from detail: the obvious one reports supersampling as a
    regression, and what works is measuring *where* a setting acts rather than how much. Both are written up
    in the ADR.
- **A virtual-texture page cache** ([`terrain_page`](crates/cic-render/src/terrain_page.rs)), which is the
  consumer the residency bookkeeping never had: physical pages, a page table per level, and a compute pass
  that bakes the layer blend once per page instead of once per fragment per frame. The blend depends only on
  the terrain data, so recomputing it every frame for unchanged ground was the waste; and a page composed at
  a density chosen for how close it is, is what lets detail scale past one map-wide texture.
  - The compose shader was **rewritten, not wired up**: the staged one read a tile atlas with blend masks and
    edge tiles, and this terrain is a heightfield plus layer weights, so every input it declared was a
    resource the engine does not build.
  - A page carries a four-texel border of the neighbouring ground, because a filtered tap at a page edge
    reads across it — and a clamped border would put a seam on every boundary, crawling as the camera moves.
    Verified by reading a page back: a page straddling a colour boundary shows border red 144 against
    interior red 89, where a clamp would read 89.
  - The G-buffer samples pages once a cache is attached, and the two paths agree to a **mean of 0.004** and
    a worst case of **5** eight-bit steps. The direct blend stays as the fallback: a cache may run out of
    slots, and a frame must not depend on it having won — a one-slot cache draws 99.9% of the frame from the
    fallback.
  - **Page mip chains**, which took the path from correct to *better*. A page held one density, so ground
    under heavy minification aliased where the fallback — which samples an albedo array that has a chain —
    did not, making the cache worse than not using it on exactly the ground it exists for. Each update now
    reduces the pages it composed, one compute pass per level, and the G-buffer derives a level from
    screen-space derivatives because a residency branch is not uniform control flow.
    - **The border is the chain's budget**, which is why it went from four texels to eight: every reduction
      halves it, a filtered tap needs a whole texel of it, so `2^n` buys `n + 1` levels and four levels cover
      a minification of eight. Get it wrong and the seam the border prevents returns at every level *below*
      the base, on ground the base looks perfect on — so it is read back rather than rendered: border red
      184, 180, 170, 149 down the chain against an interior of 89, where a clamp would read 89.
    - **The reduction averages in linear light**, matching a prediction to **0.54** of an eight-bit step,
      where a byte-space average would be out by **26**. At a grazing angle the paged frame is now *smoother*
      than the blend it replaces — **238 against 386** — where sampling the base level only reads 2.00 times
      the blend.
  - Three fixtures in a row could not show what they measured before one did, which is the standing warning
    below earning its place a third time: `surface()` divides by the summed weight, so a single ramped layer
    normalizes to a constant. A fourth mistake of the same shape followed — the agreement test first drew
    through the *forward* pass, which has no page lookup, and reported the two frames as identical. The mip
    chain made it five and six: a flat-coloured page cannot tell a linear-light average from a byte-space
    one, and an aliasing metric measured across the axis the fixture's stripes run along reads 1.49 where the
    other axis reads 158.
- **A physically-based map set for models**: normal, roughness and metallic maps beside the base colour,
  with the tangent frame the first of them needs — read from glTF's `TANGENT` where a model supplies one and
  derived from the texture coordinates where it does not, which is the ordinary case rather than the
  exception. Three texture arrays per model rather than one, because base colour is sRGB-encoded and the
  other two are linear measurements and one array has one format: decoding a normal map as a colour turns a
  flat 128 into 0.216 instead of 0.502, which tilts the whole surface and reads as a lighting bug.
  - **Metallic cost no G-buffer bandwidth.** The albedo target's alpha was writing a constant 1.0 and nothing
    read it. Every expression the lighting pass gained reduces to its predecessor at zero metalness by
    construction, which is what kept the references byte-identical when the channel arrived.
  - Read from glTF and deliberately *not* applied: the occlusion map. Occlusion is an ambient-only
    multiplier and there is no channel left for one — folding it into albedo would darken the direct term,
    which is precisely what it must not do.
- **Alpha-tested materials**, which is how foliage is authored. A material that cuts its own silhouette gets
  its own index range and its own pipelines, so opaque geometry keeps its early depth rejection and its
  fragment-free shadow pass. The cut reaches **every shadow cascade**: a leaf card casting the rectangle its
  geometry occupies is worse than one casting nothing, because a hard quadrilateral on the ground reads as a
  solid object.
- **Scenery sway** ([`scenery`](crates/cic-render/src/scenery.rs)), written from scratch — three parts on
  three time scales, four profiles rather than a longer table, and every constant derived in the file from a
  stated physical argument. **This closes the last outstanding provenance case.**
  - The phase is derived from each instance's world position with integer arithmetic, so a stand of plants is
    never in unison and a capture is still reproducible. It also carries a term along the wind direction, so
    a gust visibly crosses the map — one dot product, and the single largest contributor to the effect
    reading as weather rather than as animation.
  - The flutter is at 5.37 times the sway rather than 5, because this renderer has already been caught by
    near-harmonic ratios once: five water waves at related wavelengths interfered into a visible lattice.
- **Temporal antialiasing** — the last tier of [ADR 0005](docs/adr/0005-antialiasing-strategy.md), and the
  last item on its list: a jittered projection on an eight-phase Halton sequence, a motion-vector target, a
  ping-ponged float history, and a neighbourhood clamp in YCoCg. **0.053 ms** at 1920x1200, twice the post
  pass and a nineteenth of the frame.
  - The jitter phase is a frame parameter like scene time, which is what makes a temporal capture
    reproducible: the harness renders a full cycle and compares the last frame. It also needed an API the
    design lacked — `reset_history`, for a frame that does not continue the last one, which is the real-game
    case of a jump cut.
  - Motion vectors are **exact for swaying geometry** rather than approximate, and it cost nothing: the
    displacement is a pure function of scene time, so the same vertex function at the previous time returns
    exactly where the vertex was.
  - **It found a defect the whole reference set had been rendered through.** Three resolve passes were adding
    half a pixel to a framebuffer coordinate that already carries it, so each sampled half a pixel away from
    the fragment it was shading — a translation of every frame, plus a two-texel average where the downsample
    should return one exact texel. Nothing caught it because every reference had been rendered through the
    same offset. What caught it was that accumulating a *static* frame never reached a fixed point: successive
    frames differed by 48, 33, 19, 9, 6 — convergent, and so passing any tolerance stated as "settles".
    Measured on the deferred fixture at 1.5% of pixels differing by more than two, peak 154. A textual test
    now pins the convention, because this is the one class of error a reference comparison structurally
    cannot catch.
- **Time of day drives the light by default.** `DeferredFrame::new` derives its sun from the environment,
  and `in_environment` re-derives it, so changing the hour moves the sun rather than leaving a light that
  silently disagrees with it. The derivation is calibrated against the hand-tuned preset it replaced and a
  test pins it there.
- **Scene time is a frame parameter** — `DeferredFrame::time` — and nothing in the renderer reads a
  clock. That is what makes a capture of moving water or drifting cloud reproducible.
- **Per-pass GPU timing** ([`timing`](crates/cic-render/src/timing.rs)), because every performance question
  here is workload-dependent: a total says something is slow, a breakdown says which pass. Each pass owns a
  fixed pair of timestamp queries, a pass with nothing to attribute — skipped, or recorded over no geometry
  — reads back as absent rather than as zero, and the tick-to-duration arithmetic is a pure function with
  its own tests. Optional, since `TIMESTAMP_QUERY` is — a device without it renders identically and reports
  nothing.
  - It refuted its own premise immediately. The terrain's two million unculled vertices a frame were the
    reason to build it, and at 1920x1200 the four cascades were 7% of the frame while ambient occlusion was
    58%. Measured at 720x480 the same code said the cascades were 36%, because the cascades cost the same at
    every window size and a small target leaves nothing to compare them against.
  - **A pass that draws nothing is not timed**, because the two backends disagreed about what to say when
    one did. Vulkan records timestamp commands at the pass boundaries, which run whatever the pass contains;
    Metal declares them as *stage* boundaries, and a pass that rasterises nothing never reaches the end of
    the fragment stage, so its end timestamp is never written and the pair reads as "did not run". The near
    cascade is that pass routinely — it covers the first 5.5% of the shadow distance, about 88 units, so any
    camera an appreciable height above the ground has it sitting in empty air and catching no chunk. The two
    answers for the same empty cascade were 8.7ms on llvmpipe (a real 2048² depth clear, on a CPU) and
    "absent" on an M1 Pro. Declining the pair up front makes it absent on both, at the cost of the clear
    going unattributed — beneath the noise floor on hardware, and not actionable without deleting the
    cascade.
- **Terrain frustum culling** ([`culling`](crates/cic-render/src/culling.rs)), over a decomposition into
  32-cell chunks. The camera and each fitted cascade cull against their own frustum and the survivors draw
  as instanced runs — the instance index *is* the chunk index, so this needed no new binding, buffer or
  upload. Cascades use their own frusta rather than the camera's, because a cascade reaches behind the
  camera toward the light and a caster off screen still casts into view.
  - Verified by being invisible: every committed reference still matches byte for byte. The one case they
    could not cover is a chunk the terrain does not fill — 192 cells and 128 both divide evenly by 32 — so
    that has its own test, confirmed by breaking the shader on purpose and watching it fail at the
    predicted figure.
  - The win scales with the map and is nil at fixture size: 0.008 ms on a 257x257 terrain. On a
    **1025x1025** one the cascades go from **0.809 ms to 0.131 ms** and the G-buffer from **0.239 ms to
    0.071 ms**, taking the frame from **1.534 ms to 0.692 ms**. The figure that matters is that 0.692 ms is
    what the *small* terrain costs too: terrain no longer scales with map size.
- **A half-resolution occlusion estimate**, which is what that measurement led to. One estimate per 2x2
  block of render pixels, resolved back to full resolution by the bilateral pass that already existed — now
  the upsample as well as the blur, weighting each tap by the world distance to the render pixel its
  estimate was actually *taken* at. What halves is the number of estimates, not the resolution of anything
  they read.
  - At 1920x1200 the estimate went from **0.668 ms to 0.303 ms** and its resolve from **0.161 ms to
    0.075 ms**: the summed frame is **1.160 ms down to 0.677 ms, 42% off**. Occlusion is still the largest
    single cost, at 56% of the frame rather than 72%.
  - Its blur radius came *down* rather than up, against expectation — a wider kernel over coarser noise was
    the guess, and the captures said 3x3 half-resolution taps show no more noise than the old 5x5 while
    landing closer to the frame they replace.
- **A navigable shell** ([`cic-ui`](crates/cic-ui), drawn by [`ui`](crates/cic-render/src/ui.rs)): a main
  menu, a settings screen and its keep-or-revert dialog, skirmish setup, and a quit modal, authored as
  [layout files](content/ui) and drawn through one pipeline and one draw call per screen. Buttons,
  labels, checkboxes, sliders, text entry with a caret and an input-method composition, lists, a
  dropdown, tabs that switch their pages, and a scrollable container all behave and all draw. Settings are **applied then confirmed**, with a 15-second
  window after which the change takes itself back — because a display change can leave the person who made
  it unable to see the screen well enough to undo it. Screens **fade and slide** as they change, over a
  duration a host chooses and which defaults to none: the departing screen stays alive until the change
  ends because it is still being drawn, and input reaches the arriving one immediately and the departing
  one never.
- **A scripting language** ([`cic-script`](crates/cic-script/src/lib.rs)) whose arithmetic is not its own
  decision. A script runs inside the simulation, so it inherits ADR 0007 exactly: `f64`, only the
  operations IEEE-754 requires to be correctly rounded, no platform transcendental, angles as turns.
  - **What makes it a language of this project's own is not determinism.** Lua, Rhai and WebAssembly all
    have correctly-rounded `f64` arithmetic; what they do not have is *load-time* closure. All three
    resolve calls at run time, so a mod naming a verb the engine lacks fails when a player triggers it.
    Here it fails to compile, naming the file and the line and listing what was available.
  - **The restriction is enforced twice, and the structural one is stronger than the kernel's own.** ADR
    0007 decision 8's textual scan is here too — and **caught a real violation on its first run**, a
    `powi` in the interpreter's bounds check, which is forbidden not for being inexact but for having an
    unspecified lowering. Above that, a script cannot reach a forbidden operation whatever anybody
    writes, because the bytecode has no instruction for one.
  - **Angles are turns, and the reduction is exact.** `sys.sin(1000000.125)` returns the *identical bit
    pattern* as `sys.sin(0.125)`, because reducing a turn count is a subtraction that cannot round. The
    series agrees with the platform's `sin` to the last bit on the same first-quadrant argument across
    257 points; the few units in the last place against a whole-angle reference are the *reference's*
    range reduction, which is the problem turns remove.
  - **Fuel bounds time and the absent heap bounds space**, which between them are what "safe to run
    untrusted content inside a tick" has to mean. `while true {}` is an error naming the line.
- **One arithmetic in one crate** ([`cic-math`](crates/cic-math/src/lib.rs)), extracted from the script
  VM before the kernel exists to want it: ADR 0007's permitted operations and the turn-based
  transcendentals, depending on nothing, with the exact-bit pins and the platform-oracle comparison moved
  alongside. `cic-math` and each consumer carry decision 8's textual scan separately, so the guard
  travels with the code it guards.
- **The simulation kernel's mechanics** ([`cic-sim`](crates/cic-sim/src/lib.rs)) — every M5 charter
  line, proven against a deliberately trivial subsystem before any gameplay exists to obscure a bug.
  Fixed ticks advance only through `Kernel::advance`; identifiers count deterministically and are never
  reused; random streams are named, versioned, seeded, and hashed — including their draw counts, which
  is what catches a subsystem consuming one number too many *on the tick it happens*. Commands are
  tick-stamped opaque bytes in an append-only log that refuses reordering, and the exit-condition test
  replays 120 ticks of recorded input to byte-identical per-tick hashes.
  - **`first_divergence` names the entry and the tick**, which is the point of hashing per subsystem
    rather than per run: the replay suite plants an unaccounted stream draw on tick 50 and the report
    says `kernel.streams`, tick 50 — not "the runs differ", which is all one hash per run could say.
  - **The hasher is FNV-1a and the generator is SplitMix64**, both published, public-domain algorithms
    small enough to verify by eye, both pinned to their reference vectors — because a hash whose value
    changes between Rust releases (which `DefaultHasher` reserves the right to do) would invalidate
    every recorded replay.
  - **Floats hash by bit pattern**, so `0.0` and `-0.0` are diverged, and they should be: they divide
    differently. The one `f64` the kernel itself owns is the tick length — one division, fixed at
    construction.
  - **A scenario activates into it** ([`activation`](crates/cic-sim/src/activation.rs)): players take
    seats in authored order, every placement becomes an object with an authored-order identifier, and
    the pose crosses into simulation units exactly once — `f32` widening exactly, degrees becoming an
    integer binary fraction of a revolution, per ADR 0007's "angles are integers in simulation state".
    Activation is inside the determinism claim: one moved placement diverges on tick zero, attributed
    to `forces`.
  - **The first verbs run inside it** ([`units`](crates/cic-sim/src/units.rs)): spawn, move, stop, as
    command payloads this layer decodes while the kernel keeps them opaque. Movement is a straight
    line in the permitted operation set — a `sqrt` and a division, no trigonometry, and units store no
    heading: presentation derives one from the motion it sees. An order for a unit you do not own and
    a payload that does not parse are both ignored *and counted, and the count is hashed* — every
    machine ignores identically, and one that did not diverges on the tick it happened. Six verbs
    scripted over ninety ticks replay to identical hashes.
- Windowed presentation, driven by the reusable camera:

```bash
cargo run -p cic-render --example terrain_viewer --release
```

Pass a `.cicmap` path to view a real map; with no argument it generates terrain, a water table derived
from the heightfield's own low point, their surfaces — and **a scenario, activated through the kernel
and running live**: two players' depots tinted by seat, neutral pines between them, and six scouts
that spawn by command on tick zero and patrol a square around the map's middle on standing orders,
crossing each other's paths. The kernel advances at its fixed 30 Hz from the accumulator whatever the
frame rate does, the orders are host-side inputs of exactly the shape a network session would feed,
and presentation derives each scout's facing from its motion — the simulation stores no heading. A
loaded package still shows the building scatter, because packages do not carry a template set yet —
the demo path is exactly what they will reuse when they do. `T` toggles antialiasing and the bracket keys step the resolution scale, because what an
edge does *as the camera moves* is the whole subject and no still capture reports it; `P` prints the
per-pass breakdown once a second, which is where the figures above came from; and **`V` toggles the
virtual-texture cache**, for the same reason as `T` — a crawling page seam, a step between mip levels, and a
page arriving a frame late are all motion artefacts, and until this key existed the cache was reachable only
from a test. Verified by running it: 256 pages compose on the frame the key is pressed and the window is
indistinguishable from the direct blend, which is what it should be at a camera height where no page is
minified.

**Textures load block-compressed.** A `.dds` in `textures/` named after a glTF image now overrides that
image's pixels ([ADR 2001](docs/adr/2001-block-compressed-textures.md),
[the format](docs/formats/texture.md)): base colour as BC7 sRGB, normals as BC5, packed
occlusion/roughness/metallic as BC7 linear. Blocks reach the texture unit as they are, mips already in the
file — which is the successor ADR 0004 named for its own CPU mip pass. `cic-texconv` converts a PNG per
slot, and the slot is the only knob, because the two ways to get a colour space wrong are both quiet.

Two things are worth carrying forward from building it. **The decoder was written from the published
specifications and the exercise paid for itself twice**: BC1's and BC4's interior colours are truncating
integer division rather than rounded, and a BC7 mode with no alpha bits has alpha *overridden* to 1.0
rather than derived — which otherwise yields 247, invisible on an opaque pass and wrong the moment such a
material is blended. **And the encoder's own first version was 13 dB worse than it should have been**,
because it fitted each block's line to the colour bounding box; on an anti-correlated block that diagonal
runs across the data instead of along it, and the least-squares refinement then extrapolates rather than
recovering. The principal axis fixed it, and the number is pinned by a test — a threshold set by intuition
had passed the broken version.

Verified where it counts: a model whose base colour comes through a BC7 sidecar renders **byte-identical**
to the same model through the RGBA8 path, on an M1 Pro with the hardware decoder doing the work. The bug
that check found was a copy extent given in logical rather than block-aligned texels, which `wgpu`
validation caught before any pixel did.

**A model's own textures convert in one step, and its baked occlusion now lights.**
`cic-texconv --from-glb` reads a `.glb`, works out from the material references which slot every image is
read through — no filename heuristics — converts them all, merges a separate occlusion map into the ORM
image and repoints both slots at it, and rewrites the model with 1x1 placeholders. Verified on a real
container: four images in, three sidecars out, `occlusionStrength` intact, geometry byte-identical through a
compacted binary chunk, and the base colour reaching the GPU as `BC7_UNORM_SRGB`.

The occlusion the merge makes readable is now applied, to the *ambient* term only — where glTF scopes it.
That needed a G-buffer channel and every one was claimed, so `COVERAGE_FORMAT` widened to two channels at
about 8 MiB per frame. A test asserts that with the ambient light zeroed the occluded and unoccluded frames
are identical to the byte, which is the mistake — folding occlusion into albedo — that the cost buys off.

**Terrain layers use the same path**, and needed nothing new to do it: a layer's name was already the key,
so `textures/grass.dds` is the resolution the renderer has always done, reaching the package. This is where
the format pays most — a detail texture is sampled by up to eight layers in one fragment across the whole
visible map — and it is the easiest fit, because detail textures are authored to one size and tiled. A
terrain rendered through a compressed layer array matches the same terrain through the RGBA8 one to within a
bit, and **the viewer has been run against a real `.cicmap` carrying three converted layer textures** —
sand, grass and rock, authored as PNGs, converted by `cic-texconv`, resolved by layer name out of the
package, and reported by the viewer as `3 slices at (256, 256), 9 mip levels, BC7_UNORM_SRGB`. That was the
one check this work could not make when it landed.

## Next verified step

**[M5, the simulation](docs/milestones/m5-simulation.md), is complete: the kernel mechanics and now
scenario activation, so a map's declared players and placements construct into hashed, replayable
kernel state.** The next milestone on the path to something playable is
[M6, gameplay](docs/milestones/m6-gameplay.md) — its dependencies (M5, M9, M10) are all standing, and
its first slice, **the template set, has landed**: what a `template:` id resolves to, with activation
resolving every placement and faction against it — **the activated scenario is drawn**, headlessly
in a capture test and live in the viewer — **and the first verbs work**: spawn, move, and stop, in
`cic_sim::units`, replay-identical over ninety ticks — **and the kernel ticks live in the viewer**,
scouts patrolling on standing orders. Next M6 lines to choose from: pathfinding over the heightfield,
or combat's first pass; and packages gaining a `templates.json` member so a real `.cicmap` runs the
same way the generated demo does. Those verbs are also what
[ADR 7002](docs/adr/7002-script-events.md)'s host surface and first real events hang off, so M6's
opening work and the script-event implementation meet in the same place once that record is accepted.

The prerequisite decision earned its keep: [ADR 0007](docs/adr/0007-simulation-arithmetic.md) was
settled before the kernel was written, so the kernel was written *inside* it — almost entirely integer,
one `f64` division at construction, and decision 8's scan installed from the first commit. The
transcendentals sit in `cic-math` below everything, per the extraction recorded above.

**Priorities were set by Denys on 2026-07-29: playable first.** The audio device layer (M9's one open
item) and the view-driven detail request (M3's recorded leftover, described below) are both deliberately
deferred — recorded here so neither reads as forgotten. The script-event binding model is proposed as
ADR 7002 and awaits his review before any of it is implemented.

One item is still outstanding from M3 and is described below: **a view-driven detail request**, which is
what decides *which* ground gets a page. It is not a charter line, and the page mip chain is what showed it
matters — a page's chain is four levels deep and past that a page saturates, so ground the residency map
should never have staged is the only ground where the cache still aliases.

**The shell runs in a window**, which this project treats as a separate obligation from a green capture
suite — the one bug the headless suite structurally could not catch appeared the first time a window
opened, and none of hover following a pointer, focus moving under Tab, a caret advancing as somebody
types, or a countdown actually counting is reachable from a capture:

```bash
cargo run -p cic-render --example shell --release
```

Change the resolution scale, press Apply, and do nothing: fifteen seconds later the setting comes back on
its own. Screens fade and slide as they change, which is the other thing no still image can judge. The
window also exercised what a capture at scale 1.0 could not — it opened at **1.5**, which is what prompted
the density reference.

**Drawing landed in three parts, split where the mistakes are.** A **paint layer with no GPU in it**
decides how the interface looks — which colour a focused button takes, where a checkbox's indicator sits,
how far along its track a slider's knob is — so all of that is testable by asserting on a list rather than
by capturing an image. A layout names a **role** and never a colour, the same argument the string table
makes about text. Colours are authored as sRGB bytes and leave as **linear** floats, because a shader
writing to an sRGB target must emit linear values and passing the bytes through is what makes every
surface too bright — invisible in a test that compares numbers to themselves.

**The typeface is authored in this tree**, and the licence is the reason. A font file is a binary asset
with its own obligations and this repository exists to have one set; a *system* font makes the rendered
result depend on which machine drew it, which a byte-comparison harness cannot tolerate. So
`cic-render/src/text.rs` holds ninety-five glyphs as lines and elliptical arcs on one integer grid, given
width by measuring each pixel's distance to the nearest stroke — a stroke has no inside, so there is no
scanline pass and no winding rule, and coverage falling to zero across the last pixel of the half-width
*is* the antialiasing. It reads as drafting lettering, which suits the subject. It covers Latin only: a
character with no glyph draws as a hollow box, which is visible rather than silent, and a loaded-font path
can go behind the same type later. See [LICENSING.md](LICENSING.md) for why that seam exists.

Text metrics also close a loop the solver left open. An `Auto`-sized label is now as wide as its own text,
and `ime_cursor_area` narrows from **the whole field to the caret** — on a wide field those are a long way
apart, and the candidate window appearing beside the box rather than beside the character is what the
milestone flagged.

Two findings came out of drawing it. The **specimen sheet** caught two letterforms that were wrong — an
`e` with its aperture at the top, because an arc authored against the mathematical convention comes out
mirrored on a Y-down grid — and no assertion over coverage bytes would have shown either. And running four
capture tests in parallel **crashed the driver**: four devices on one adapter, created and destroyed
concurrently, gave an access violation rather than a failed test, so the run reported nothing at all about
the images. The existing capture targets already shared one device through a `OnceLock`; this one now does
too.

**The screen stack and transactional settings landed before it.** A settings apply is undone by a machine
rather than by a user: a change goes in force, a 15-second window opens, and the *absence* of a
confirmation is what brings the previous settings back. That inversion is the whole point — a display
change can leave the person who made it unable to see the screen well enough to click undo, so an undo
that depends on them clicking is not an undo. Three consequences, all of them about leaving: applying
must not move the stack, since the revert window is only useful while the confirm button is reachable;
closing the settings screen with a change unconfirmed reverts it, since nobody will confirm on a screen
that is not open; and a second apply inside the window keeps the *first* restore point, because what is
worth returning to is the last state somebody confirmed and not the previous attempt at replacing it.

Each open screen keeps its own retained state, which is what a stack buys over one current screen, and a
screen appears at most once — navigation is by destination, so asking for one already open unwinds to it.
That also removes a bound that would otherwise have to be invented: input can push screens, and with no
duplicates the depth cannot exceed the number of screens the engine defines.

**Widget behaviour and input routing landed before it**, including input-method composition — because a
single character per keystroke is the Latin case, and assuming it is the only one is how an engine ends up
unable to accept CJK text without being rebuilt. Retained state keys off node ids, which is why the format
*requires* one on any widget holding state or taking focus rather than treating it as optional.

**Tabs switch pages**, which they did not before: `Widget::Tabs` tracked a number and nothing acted on it,
while the format's own comment claimed it switched between sibling pages. A strip's children are its headers
and a `pages` field names the stacked container holding the bodies, checked against each other at load —
three headers over two pages is a screen whose third tab shows nothing, and neither node is wrong on its own.
Visibility is decided in the *solver*, which is the one place state flows into layout: hit testing, keyboard
navigation and drawing all read the same solved sequence, so one of the three forgetting to filter is a
control the user cannot see taking a click. The consequence is that a tab change is a relayout, exactly as a
resize is.

The settings screen was the first candidate for them, and its multi-option display setting landed as a
dropdown instead — the right control for one-of-a-set. No authored screen uses a tab strip yet; pages wait
for a screen with enough categories to need them.

**The remaining renderer item is a view-driven detail request**, and the mip chain is what showed it
matters. A page's chain is four levels, covering a minification of eight; past that a page saturates while
the direct blend's albedo chain keeps going, which is why the grazing-angle capture still reads 1.93 in its
topmost thirty rows of ground against 0.62 everywhere nearer. Ground that far should have no page resident
at all, and *which ground has a page* is the residency decision nothing derives from a camera yet. The
residency map already ranks by projected size, so this is a small function over the frustum rather than a
design.

**Scripting needs host verbs, and they are blocked rather than deferred.** Spawn, order, count, query a
zone, set an objective: every one is a call into a simulation kernel, so M10's remaining half waits on M5.
The seam is ready — a kernel declares them on an `Interface` and implements one trait. The transcendentals
trap ahead of it is already closed: the implementation moved to **`cic-math`**, one crate below both the
VM and the kernel to come, so decision 4's `sin` has exactly one home and two implementations that could
disagree can no longer exist.

**Audio needs a device, and that is the one thing about it a green suite cannot establish.** The mixer
produces correct frames and nothing hands them to hardware. This is the same lesson the standing constraint
below records for presentation: the one bug the headless renderer suite structurally could not catch
appeared the first time a window opened. `Capabilities::renders_to_buffer` exists so a host knows whether
it needs a device at all, which is what keeps an FMOD backend — which owns its own output thread — from
having to pretend.

## Gate status

Formatting, strict lints (`clippy::all` and `clippy::pedantic` as errors, plus `-D warnings` as CI runs
it), and the full test suite all passes on the pinned toolchain: **847 tests across ten crates**, up from
782 across eight. The ninth crate is `cic-math` — ADR 0007's arithmetic, extracted from `cic-script` so the
kernel can share it — and its five new tests are its own copy of the decision-8 restriction scan plus a
documentation example; the ten series tests moved with the code rather than being duplicated. The tenth is
`cic-sim` at 44: the kernel mechanics' unit tests, another copy of the scan, the replay suite whose
headline test is the milestone's exit condition run twice, and eleven activation tests spanning the
one-moved-placement divergence and the template resolutions; the template format's own six live in
`cic-assets`. The CI
runner runs the same suite against Mesa's lavapipe.

**No reference moved for the page mip chain, which was not the expectation.** Every committed NVIDIA
reference still matches within tolerance, including `terrain-from-pages.png` — the paged frame changed by a
mean of 0.004 and a worst case of 5, which is inside what the comparison allows in a small region while still
failing on four steps across most of a frame. No authored screen uses a tab strip either, so none of the
interface references moved when tabs learned to switch pages. Two of them *did* move when the settings screen
changed — a dropdown replacing its antialiasing checkbox, two buttons instead of four, and the demonstration
text entry nobody read removed — and two more were added, for an open dropdown and for the dialog. The grazing-angle scene the chain is verified
on deliberately has **no committed reference**: its claim is a statistic about adjacent-pixel energy rather
than an image, and a new reference scene would force a lavapipe capture from the runner for nothing the
numbers do not already say. **Lavapipe agreed**, which could only be established on the runner: the branch
passed CI without a reference being regenerated, so the page path's change is inside tolerance on a software
rasteriser as well as on the NVIDIA set.

The rendering tests take about eleven seconds there, which is what makes this affordable on every pull
request. Captures go to `target/tmp/` and
upload as an artifact on every outcome, so a harness failure's capture and amplified difference image can
be looked at rather than being stranded on the runner.

**A new interface reference has to be generated on the runner before CI can pass.** They can only be
rendered where lavapipe is, so a branch adding a scene fails CI once with the captures uploaded as an
artifact for review — the flow the harness is built around for a deliberate rendering change, and the same
one the half-pixel fix and the five model captures went through. A missing reference is deliberately a
failure rather than a skip, because a silent pass would remove the coverage it was providing.

Seventeen references cover the scene: terrain layers, terrain drawn from composed pages, instanced
models, the deferred chain, water, water under a glancing sun, cloud shadows, fog, wet ground, snow, an
antialiased frame, a supersampled one, a temporally accumulated one, a normal-mapped model, a metallic
one, alpha-tested foliage, and a swaying canopy. **Eight more cover the interface**: the main menu, the
settings screen with every widget kind it has, that same screen at one and a half times the pixel
density, a modal over the screen it covers, a scrolled container clipped to itself, a screen change
partway through with both screens drawn, an open dropdown over the rows it covers, and the keep-or-revert
dialog over the screen that applied it. Each was generated on its own machine and looked at before being
committed — one set for an NVIDIA RTX 4080 SUPER and one for lavapipe, **both complete at twenty-five**.

The render tests still skip rather than fail when no adapter is available, so a developer machine with no
GPU reports honestly instead of red. **CI sets `CIC_REQUIRE_ADAPTER`, which makes the same situation a
failure there**, because a skipped rendering test and a passing one are the same colour and a runner that
silently lost its adapter would otherwise leave the harness protecting nothing. The regression comparison
itself is a pure function over bytes with its own unit tests, so that half is verified even with no GPU
present.

## Standing constraints

- Nothing in this tree derives from another game's code or reads another game's data. See
  [LICENSING.md](LICENSING.md). Water was written from scratch and the removed shader was not consulted;
  **scenery sway has since been written the same way, which closes the last outstanding case.** The rule
  that nothing may be copied backward across `5e824cf` still stands, and still has nothing left that wants
  to break it.
- Every decoder is bounded and total — see [binary parsing](docs/invariants/binary-parsing.md). The WAV
  reader is the newest and follows the same rules: a `data` chunk claiming four gigabytes is refused before
  the buffer is sized, an unknown chunk is skipped and an unknown *format tag* is not.
- **Presentation must not draw from a simulation random stream.** The less obvious half of the determinism
  rule, and audio is where it bites — variant selection and pitch spread want randomness on every gunshot.
  `cic-audio` carries its own stream, seeded separately, so it never has to.
- Anything that will reach simulation state follows [determinism](docs/invariants/determinism.md) from
  the start, because it cannot be retrofitted.
- **Only [ADR 0007](docs/adr/0007-simulation-arithmetic.md)'s operation set may touch simulation state**,
  and a subsystem that runs inside the simulation does not get an arithmetic of its own. `cic-script`
  learned this the expensive way: it was built on fixed point, on an argument ADR 0007 shows is false,
  and two arithmetics inside one lockstep simulation is a script and a kernel able to disagree about a
  comparison. Decision 8's textual guard is not bookkeeping — its first run on that crate found a `powi`
  nothing in the build would have objected to.
- **A rendering change is not verified by a green test suite. Look at the capture.** Every rendering bug
  so far passed its own assertions and was caught by opening the PNG: reversed layer ramps, two separate
  tone-mapping mistakes, a shadow camera on the wrong side of the scene, an occlusion blur whose
  tolerance rejected every neighbour at distance, twice a test fixture measuring itself rather than the
  renderer, a quad UV mapping that walked the unit square in the wrong order, a wave sum that interfered
  into a diamond lattice, a specular exponent so tight the highlight reached no pixel at all, water
  painted as a slab past the edge of the map, a cloud hash correlated along one axis, and a derived sun
  that matched its preset's colour exactly while sitting 27 degrees away in azimuth.
  - **And one the harness could not have caught, which is worth its own line.** Three resolve passes offset
    their texture coordinate by half a pixel, and every reference was rendered through the offset — so the
    images agreed with each other and with the code. A reference comparison cannot catch an error applied
    uniformly to both sides of it. What caught it was a *property*: a temporal accumulation of a static
    frame must reach a fixed point, and it did not. The lesson is not that the harness is weak but that it
    has a blind spot with a shape, and the things that see into it are invariants rather than images. The regression
  harness now catches this class automatically, and in CI rather than only locally — but only for the
  eleven scenes it has references for, and only once someone has looked at those references and confirmed
  they are right.
- **A fixture can be the bug.** Repeatedly now a correct implementation has been measured against a fixture
  that could not show what was being measured: a shadow fixture whose ridge was wider than its own shadow, a
  fog fixture so flat and so distant that an integral along the ray smoothed away everything the density did
  — four rounds of tuning went into that one before the fixture was suspected — a page fixture whose single
  ramped layer normalized to a constant, a flat-coloured page that could not distinguish a linear-light
  average from a byte-space one, and an aliasing metric taken across the one axis its fixture's stripes did
  not vary along. **The pattern is worth naming: a fixture that cannot fail is indistinguishable from a
  fixture that passes.** The cheap defence is to make the *wrong* implementation's prediction part of the
  assertion, which is what the linear-light test does.
- **Presentation needs running, not just testing.** The one bug the headless suite structurally could not
  catch — surface capabilities queried through an adapter from the wrong instance — appeared the first
  time the window opened.
