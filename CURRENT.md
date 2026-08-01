# Current objective

## Where the project is

M0 through M2 are complete: the workspace and its invariants, the resource layer, and the native asset
formats — M2 on its read path, with the model and package writers waiting on M8's editor. **M3's charter and its exit condition are both met** — the renderer draws a lit, shadowed,
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
in [ADR 7001](docs/adr/7001-scripting-language.md).

**A scenario's scripts now run.** The event dispatcher landed in the kernel as `cic_sim::scripts`,
implementing [ADR 7002](docs/adr/7002-script-events.md): `map.json` carries an ordered `scripts` array,
the package reads what it names, every entry is compiled at load against a closed set of verbs and
events, and dispatch order is authored order. A handler *is* the subscription — there is no registration
call and no binding table, so a misspelled event is a compile error rather than a handler that silently
never fires. Mission memory — flags, counters, timers — lives on the kernel's side of the host boundary,
which is what puts scripted behaviour inside the determinism claim instead of beside it: a script that
behaves differently on two machines diverges on the tick it happened, attributed to `scripts`.

Two things the implementation settled that the record had wrong. The initial event set is **three, not
five**: `zone_entered` and `zone_exited` are designed but not declared, because an event declared before
anything raises it compiles and then never fires, which is the exact silent no-op the closed set exists
to prevent. And a `str` handler argument **cannot be synthesized** — a string is an index into the
program's constant table and there is no heap — so a timer's name resolves against each receiving
script's own constants, and a script hears about a timer only if it names it.

What is left is the verbs that reach *another* subsystem — spawn, order, count. The immutable half of
that boundary now exists: a subsystem can read its peers during a tick. The mutable half still needs an
attributable route, because the answer must not be "a script forges a player's command" or "a subsystem
takes a mutable reference to its peer". Proposed [ADR 3008](docs/adr/3008-deterministic-task-execution.md)
puts typed effects through a stable phase commit, which is also the semantic contract a later parallel
task executor must preserve.

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
    command payloads this layer decodes while the kernel keeps them opaque. Movement is in the
    permitted operation set — a `sqrt` and a division, no trigonometry, and units store no
    heading: presentation derives one from the motion it sees. An order for a unit you do not own and
    a payload that does not parse are both ignored *and counted, and the count is hashed* — every
    machine ignores identically, and one that did not diverges on the tick it happened. Six verbs
    scripted over ninety ticks replay to identical hashes.
  - **And they path around ground they cannot cross** ([`ground`](crates/cic-sim/src/ground.rs)):
    [ADR 3001](docs/adr/3001-pathfinding.md)'s grid and search. Passability is *derived* — slope
    between adjacent samples against a grade, and a cell wholly under the water line — on the
    heightfield's own grid, so there is no second resolution to keep registered and no authored layer
    to go stale when the ground is edited. The search is **A\* with no floating point in it at all**:
    `10` an orthogonal step and `14` a diagonal by default, ties broken on the lower cell index, no
    diagonal cutting a corner past an impassable cell, a binary heap over flat arrays and not one
    hashed container. ADR 0007 would have *permitted* `f64` costs; integers make the accumulation
    question unaskable instead of manageable, which is the same shape as choosing integer turns for
    angles.
    - **An unreachable target does not fail.** The route ends at the nearest cell the search closed,
      which is what a player asking for the far side of a wall means and costs nothing, because the
      search already did the work.
    - **A corner costs a unit no time.** Routes are string-pulled to straight runs and the stepper
      spends one tick's travel across as many legs as it reaches. Without that, every turn would
      strand a fraction of a step and the pathfinder would have made units worse at going places —
      the test for it is pinned to an exact arrival tick, and it was rewritten once after the first
      version turned out to pass with the carry-over deliberately removed.
    - **And a corner is rounded rather than turned on the spot.** A string-pulled route still bends
      at cell centres, which on an eight-metre grid is a 45° pivot every few strides and reads as
      clockwork. Each interior corner is cut back and interpolated across a short quadratic Bézier —
      and **every segment of the arc is checked against the grid, with the corner left sharp if any
      of it would clip**, because a smoothing pass that ignored the grid would reintroduce exactly
      the cut corners the search took care to avoid, at exactly the turns that exist to get around
      something.
    - **Every coefficient is a setting, and every setting is in the hash.** The grade that makes a
      cliff, the water line, the cost class the derivation assigns, what a step costs, and how far
      back a corner starts turning all live on `GroundRules` rather than in constants. They are
      folded into the subsystem hash because they decide what the grid *means*: two machines playing
      one match under different grades are playing two different games, and that should surface as a
      divergence on tick zero rather than as somebody diffing configuration files. The heuristic
      prices itself against the **cheapest class the grid actually holds** rather than assuming that
      class is `1`, which is what keeps a renumbered ladder from quietly making A\* inadmissible —
      ADR 3001's amendment A is now a test rather than a hazard.
    - **And objects stamp it.** [ADR 3001](docs/adr/3001-pathfinding.md) decision 4: templates carry
      a `footprint` — the ground an object denies — and a `passage` — the ground it grants, at a
      cost class — both rectangles of whole cells, both optional, and neither allowed on a unit,
      whose own occupancy is decision 10's business. Precedence is derivation, then passage, then
      occlusion, so a bridge crosses a river and a depot raised at the bridgehead denies it. Nothing
      constructs anything yet and none was needed: scenario activation already places structures, so
      `Ground` reads `Forces` as an earlier peer and reconciles once a tick — construction,
      demolition and movement are one comparison rather than four call sites.
      - **The grid is layered rather than overwritten, and that is the whole difficulty.** The
        terrain's own derivation is kept whole underneath, and the classes the search reads are
        computed from it plus the stamps, because *an object's death has to restore what was under
        it*. A grid that wrote over its classes could only guess on removal, and the guess is wrong
        exactly where it matters: a depot raised on a shoreline, destroyed, leaving walkable water.
        The test puts one footprint over lake bed and open ground at once and requires each cell to
        come back as itself — the wrong implementation gets two of three right.
      - **Routes replan on the tick the grid changes** (decision 7), in identifier order, and a
        repath is counted and hashed the same way an ignored order is. The intersection test is an
        over-approximation — a leg is checked by the bounding box of the cells its ends fall in — so
        it can cost a repath that returns the same route and it cannot miss one, which is why the
        record's "a unit whose next step is blocked" fallback turned out to have nothing to catch.
      - **And they keep out of each other** (decision 10). A unit is a circle of `radius` metres —
      the field arriving with its consumer exactly as `speed` and the stamps did — and after
      everybody has stepped, each overlapping pair gives up half the overlap along the line between
      their centres. Every push is measured before any is applied, so identifier order decides
      nothing but which way two units standing in the *very same spot* step; and every push is
      checked against the grid, because a shove is not a licence to enter a building. A push the
      ground refuses is retried one axis at a time, which is the record's "slide along" and is why a
      unit shoved into a wall travels down it rather than stopping against it.
    - **Stamping found a defect in the slice before it.** String-pulling asked only whether a
        shortcut was walkable — the right question until two adjacent cells cost different amounts,
        and `passage` is the first thing in the engine that makes them. A route that went four rows
        out of its way to reach a road was pulled straight back off it, so A\* made the decision the
        cost ladder exists for and the next pass silently undid it. A shortcut now has to cost no
        more than the chain it replaces, priced on the walk that was already checking passability;
        on ground of one class the two are always equal, because a Bresenham line takes the
        octile-optimal mix of steps and that is what A\* found, so no existing route moved.
  - **A subsystem can now read its peers** ([`subsystem`](crates/cic-sim/src/subsystem.rs)), which is
    what movement asking the ground where a unit may walk actually requires. Immutably, and only
    peers: the kernel splits its own list around the subsystem it is running, so the running one holds
    `&mut` to itself and `&` to everything else and the mutation the rule forbids cannot be spelled.
    What a read *sees* is pinned by the order that already existed — a peer registered earlier has
    advanced this tick, one registered later has not — so it is asserted both ways round rather than
    left to whichever order a host happened to pick. This is the query half of M10's cross-subsystem
    verbs; writes still need the typed effect route proposed in ADR 3008.
- Windowed presentation, driven by the reusable camera:

```bash
cargo run -p cic-render --example terrain_viewer --release
```

Pass a `.cicmap` path to view a real map; with no argument it generates terrain, a water table derived
from the heightfield's own low point, their surfaces — and **a scenario, activated through the kernel
and running live**: two players' depots tinted by seat, neutral pines between them, and six scouts
that spawn by command on tick zero and patrol a square around the map's middle on standing orders,
crossing each other's paths and going round the lake rather than through it. **The depots now deny
the ground they stand on** — five cells square against the eight-metre grid — and the viewer prints
the grid twice, before and after the placements stamp it, because the difference between the two
numbers is the only part of this a still image cannot show. **And the scouts patrol as two groups
rather than as six units**: each side's three are sent to the next corner by one `move_group` order,
so they arrive in the shape they set out in instead of in a pile, and a **plate is drawn on the
ground at every unit's slot** — which is the formation made visible, before anybody reaches it and
gone once they have. The demo also **drags a sixty-metre line across the way each side is
marching**, standing in for the mouse the viewer does not have, so both patrols arrive as a line
abreast rather than as whatever huddle they were in — which is the part of this that no still image
reports. The kernel advances at
its fixed 30 Hz from the accumulator whatever the frame rate does, and the orders are host-side
inputs of exactly the shape a network session would feed.

**Presentation interpolates between the last two ticks**, which it did not, and that omission was
the whole of a complaint that movement looked clunky: a unit whose position is read straight from
the snapshot moves thirty times a second in front of somebody watching a hundred and forty frames of
it. `TickAccumulator::alpha` had existed since M5, documented as "what a renderer interpolates by",
with nothing calling it. Between two computed ticks rather than extrapolated past the latest one —
presentation shows a moment the simulation has already been through, one tick behind, and never a
position it did not compute. Facing is derived from that motion and **turn-rate limited**, which is
[ADR 3001](docs/adr/3001-pathfinding.md) decision 9's own answer and the other half of the
clockwork: without it a unit reaching a waypoint pivots between one frame and the next. A
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

**A model's compressed textures live inside its own `.glb`.** `MSFT_texture_dds` puts the DDS in the
container beside the model that uses it, so there is no naming convention to keep and nothing to lose track
of; the extension's fallback image means a reader that has never heard of it still sees a complete glTF.
Sidecars stay for terrain, which has no container, and for a package sharing one texture between models.

Reading it needed the GLB container reader moved into `cic-assets`, because the `gltf` crate decodes every
image eagerly and knows only PNG and JPEG — so it refuses a container over a DDS image no material would
have sampled. Every import now lifts those out of its way, `import_model` included, since a function that
refuses a valid model is a trap whether or not its caller wanted the textures.

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

**The sky can now be a photograph.** Until this the sky was two `vec3` constants and a vertical gradient
across the frame — a sky that does not respond to camera pitch, has nothing in it for water to mirror, and
cannot be authored at all. `cic-assets::sky` reads Radiance `.hdr` (both scanline encodings, the header
exposure divided back out, bounded and total like every other decoder here), and `cic_render::Sky` uploads
it as `Rgba16Float` with a mip chain that **wraps in longitude**, which no hardware generator does and
without which a seam runs down the meridian. It is group 3 and its own WGSL chunk, bound by the two passes
that ask a direction what colour it is — so the analytic gradient and a captured environment are two
branches of one file, and every committed reference stayed byte-identical. See
[ADR 4001](docs/adr/4001-hdri-sky.md) and [the format](docs/formats/sky.md).

**An 8K file is reduced rather than refused**, which breaks this crate's uniform "refuse what crosses a
bound" rule deliberately. HDRIs ship at 8K by convention and 8K is more resolution than a sky can use —
one texel covers half a pixel at the horizon, for 358 MiB of video memory. The reader already walks one
scanline at a time, so box filtering by a power of two as it goes costs almost nothing and never
allocates the oversized buffer: the 128 MiB 8192x4096 file this was tested against reads in 280 ms and
peaks around 34 MiB. `maximum_dimension` still refuses, at 16384, because a declaration of 200000 texels
is a different thing from a large picture.

The part that was not a texture lookup is the light. [ADR 0006](docs/adr/0006-atmosphere.md) derives the
fog colour and the ambient from the *sky*, so binding a photograph behind a scene lit by two hand-tuned
constants would have reintroduced exactly the disagreement that record exists to make inexpressible — an
orange sunset overhead and blue-grey shade beneath it. Both are now measured off the image at load, **with
the sun clamped out of the integral**: an HDRI's sun disc carries most of the irradiance, and this renderer
already has a directional light for it, so counting it twice does not brighten the scene — it removes
shadow contrast entirely, because the ambient becomes as strong as the beam. The sun itself is *not* taken
from the image, because fitting a light to an overcast environment picks a direction out of noise;
`Sky::aim_at` turns the image until its own sun sits at the light's azimuth instead, and the viewer
re-aims it every frame so scrubbing the hour moves the sky and the shadows together.

Two things the implementation settled, both by looking at captures rather than by an assertion. **A
reflection's blur is selected by an angle, not by a roughness**, and the first version got a lake as
coloured speckle before that was clear: water's material roughness is small — that is why it mirrors —
while its wave *slope* is not, and it is the slope that decides how much sky one pixel reflects. And **the
deferred fixture's camera has no horizon in it**: it is pitched twenty-six degrees down against a
projection half-angle a little under that, so its every background pixel is below the horizon, and the
first sky capture rendered the environment's lower hemisphere as a flat grey lid that looked exactly like
a texture that had failed to bind.

One unwelcome consequence, recorded rather than smoothed over: a captured sky makes the **water pass's
existing normal aliasing visible**. At a grazing view a six-degree change in a wave's slope moves the
Fresnel share from 0.19 to 0.72, so neighbouring pixels alternate between the bright reflected sky and the
dark water body and the surface sparkles. The analytic sky hid it by being nearly as dark as the body.
Fixing it means damping the wave normal by the pixel's footprint, which changes how water shades and moves
every committed water reference, so it is its own change. `water-sky-captured.png` shows it.

## Next verified step

**[M5, the simulation](docs/milestones/m5-simulation.md), is complete: the kernel mechanics and now
scenario activation, so a map's declared players and placements construct into hashed, replayable
kernel state.** The next milestone on the path to something playable is
[M6, gameplay](docs/milestones/m6-gameplay.md) — its dependencies (M5 and M10) are both standing, and
its first slice, **the template set, has landed**: what a `template:` id resolves to, with activation
resolving every placement and faction against it — **the activated scenario is drawn**, headlessly
in a capture test and live in the viewer — **and the first verbs work**: spawn, move, and stop, in
`cic_sim::units`, replay-identical over ninety ticks — **and the kernel ticks live in the viewer**,
scouts patrolling on standing orders.

**Pathfinding has landed on top of that, all but its last decision**: `cic_sim::ground` derives the
passability grid, stamps it, and searches it, and a move order is answered with a route rather than
a straight line. On the viewer's generated terrain the derivation is 8,661 impassable cells out of
65,536 — water and cliffs the scouts walk around instead of through — and the four placed depots
stamp it to **8,718**, denying 57 cells that were not already denied. That difference is the
mechanic and the layering at once: four five-cell-square footprints cover a hundred cells, and
forty-three of them were water or cliff the terrain had already refused, which the grid knows
because it kept the derivation whole underneath rather than writing over it.

So decision 4 is built — `footprint` denies, `passage` grants at a cost class, precedence runs
derivation, then passage, then occlusion — and so is decision 7, routes replanning on the tick the
grid changes. Neither needed a construction mechanic in the end: scenario activation already places
structures, and `Ground` reads them from `Forces` as an earlier peer.

**And decision 10 with them, so [ADR 3001](docs/adr/3001-pathfinding.md) is now implemented in
full.** Local avoidance is the modest thing the record reserved ground for: a unit is a circle,
overlapping circles push apart, and a push the ground refuses slides along whichever axis is free.
Sixteen units ordered to one cell on the rough test map end with **17 of 120 pairs** standing in
each other, against **120 of 120** with the coefficient at zero — and on the tightest muster point
that map has, nobody is ever pushed onto ground the grid refuses. What the record left open is
written into the record rather than into the code: `radius` is required for a unit and refused for
everything else, because a standing object's occupancy is its footprint and a mover's is a circle;
every push is measured before any is applied, so being spawned first buys nothing; and the one
coefficient there is, is in the hash.

One limitation is recorded rather than smoothed over: a head-on pair stalls, because a push
straight backwards has no sideways component to slide on, and choosing a side is the negotiation
the record declined. The quadratic pair loop was measured rather than assumed at **0.013 ms per
tick at 100 units, 0.14 ms at 500 and 0.49 ms at 1000**, against a 33 ms tick, so the spatial bin
that would remove it is recorded and not built.

**The other limitation it recorded is now closed.** A crowd converging on one point kept jostling —
every unit still walking at a point every other unit was standing on — and the record named the fix
without building it: sixteen units sent to one place should be given sixteen places. That is
**formation movement**, [ADR 3003](docs/adr/3003-formation-movement.md), and a `move_group` order
now does exactly that. The formation is **the one the group is already in**: each member's slot is
its own offset from the group's centre, carried to the destination, so a line arrives as a line and
a wedge as a wedge. There is no box, no template and no shape table anywhere in it — which is the
*free* half — and the assignment is the identity, member `i` to slot `i`, which is the *not random*
half and also means that on open ground every displacement is the same vector and nobody crosses
anybody.

Two things the translation cannot do alone. A group that set out in a heap has no shape to carry, so
the slots are **opened out** by the same radius-aware push avoidance uses — taking the *average* of
what each slot's neighbours ask for rather than the sum, because summing overshoots and the group
oscillates outward instead of settling. And a slot the ground refuses is **re-placed widest member
first**: the object that has the hardest time fitting anywhere gets first refusal on the roomy
ground, and the narrow ones fill in around it. That ordering is the whole of "wide units placed
efficiently", and it is a one-line sort rather than a packing algorithm.

The measurement is the point: sixteen units sent to one cell **as a group** end with **zero** of a
hundred and twenty pairs overlapping, against **twelve** for the same crowd sent by sixteen separate
orders.

**Three amendments to that record landed with it.** **The line the player drags is the line the
squad stands on** — the *Red Alert 3* gesture, where the press point is where the rank begins and
the release point is where it ends. Its direction is which way the rank runs and **its length is the
width**: drag long and the group strings out along it, drag short and it folds into ranks stacked
behind the first, drag almost nothing and it goes single file. Members keep the order they were
standing in, so a squad forming up does not cross itself, and a wide unit takes more of the line
than a narrow one.

**A plain click still carries the shape exactly.** The only thing that ever replaces a formation is
the player drawing a new one — which is what makes the drag an addition rather than the game
deciding to rearrange things. And a group now **marches at the pace of its slowest member**, so a
column ordered together does not string out; the pace is held on the *base* speed, so a road still
speeds the whole column rather than pulling it apart again.

**And the method is measured rather than argued about**, which is the answer to "how do we know this
is the right one". `cic-sim/tests/formation.rs` sends an eight-unit squad to every eleventh passable
cell of the rough map and counts what happens to the shape: **2612 of 2728 slots — 95.7% — come
through as a pure translation**, and the slots that move are *exactly* the ones the terrain refused,
none others. A compact box laid over the same suite needs **72 repairs against the carried shape's
116**, so keeping the player's arrangement costs about **1.6 times** the repair of the tightest
packing there is. That is the trade in two numbers, and the suite is the harness a different method
would be scored on. What it cannot settle is whether it *looks* like an army moving; the viewer
draws the slots so there is something to watch.

**The economy is decided, and so is the cost ladder.** Denys accepted
[ADR 3002](docs/adr/3002-corridor-economy.md) on 2026-08-01, together with the three amendments it
raised against [ADR 3001](docs/adr/3001-pathfinding.md). Two of those are now built:

- **A**, the ladder — metalled `1`, graded `2`, plain `3`, mud `4`, rubble `5`. It cost one default
  value, because the heuristic had already been made to price itself against the cheapest class the
  grid holds rather than a hardcoded `1`. A test walks one route under three ladders and requires the
  same answer, which is what makes the renumbering provably free rather than merely apparently so.
- **B**, class reaching the movement rate — a unit crosses a metalled cell three times as fast as a
  field, so grading is an income increase and not just a routing preference. The amendment did not
  say what "three times" is measured *against*, and something has to: `reference_class` declares
  which rung a template's authored `speed` is the speed for, kept separate from `plain_class` because
  what the terrain derives to and what a speed means are different questions.
- **C**, wrecks stamping a cost class rather than a footprint, still has nothing to implement —
  there is no combat, so there are no wrecks. What has changed is that the mechanism is no longer
  waiting either: a wreck's class is a `passage` with a dear one, laid and lifted through the same
  reconcile a road is. It is accepted so the first one is not made a wall by whoever writes it.

Accepting 3002 fixes the design and does not schedule it. Nothing of the economy is built, the record
names decision 1 — gates, yards, carriage — as the minimum viable version, and its own build order is
shared carriage first and faction divergence second.

**The next sequence is explicit now.** First, packages gain a bounded `templates.json` member so a real
`.cicmap` activates and draws through the same path as the generated demo. Second, proposed
[ADR 3008](docs/adr/3008-deterministic-task-execution.md)'s phase and typed-effect contract is settled and
lands with a serial executor, preserving every existing replay hash. Third, combat's first pass adds
health, one weapon, attack, death and the rubble-class wreck the accepted economy record already names.
The first scripting verbs and combat events then have mechanics and an attributable effect path to reach.

Parallel execution follows a measured workload rather than blocking those mechanics. The schedule owns
dependencies, visibility and stable commit order; an executor owns only which ready job runs on which
worker. Buffering occurs at dependency barriers rather than once for the whole tick, so a structure that
changes the ground and a unit that routes over it do not acquire an accidental tick of latency. A serial
backend remains the oracle, and a custom work-stealing backend is considered only if a general scoped pool
cannot meet a measured latency or isolation requirement.

[Faction colour](https://github.com/dennissoftman/commanders-in-chief/issues/61) is independent
presentation work and can land alongside the sequence above.

The prerequisite decision earned its keep: [ADR 0007](docs/adr/0007-simulation-arithmetic.md) was
settled before the kernel was written, so the kernel was written *inside* it — almost entirely integer,
one `f64` division at construction, and decision 8's scan installed from the first commit. The
transcendentals sit in `cic-math` below everything, per the extraction recorded above.

**Priorities were set by Denys on 2026-07-29: playable first.** The audio device layer (M9's one open
item) and the view-driven detail request (M3's recorded leftover, described below) are both deliberately
deferred — recorded here so neither reads as forgotten. The script-event binding model is accepted and
implemented as ADR 7002; its cross-subsystem verbs wait on mechanics and the typed effect path above.

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

**Scripting needs host verbs, and the read half of their boundary has since been built.** Spawn, order,
count, query a zone, set an objective: every one reaches simulation state, and M5 is complete. A kernel
declares them on an `Interface`, and **a subsystem can read its peers during a tick**, which is how a
query such as count can observe its owner. Spawn and order are writes, though, and an immutable peer read
cannot honestly implement them. Proposed ADR 3008 gives those writes typed effects committed by the state
owner, distinct from player commands and stable under parallel scheduling. The transcendentals trap ahead
of all of it was closed earlier: the implementation moved to **`cic-math`**, one crate below both the VM
and the kernel, so decision 4's `sin` has exactly one home and two implementations that could disagree can
no longer exist.

**Audio needs a device, and that is the one thing about it a green suite cannot establish.** The mixer
produces correct frames and nothing hands them to hardware. This is the same lesson the standing constraint
below records for presentation: the one bug the headless renderer suite structurally could not catch
appeared the first time a window opened. `Capabilities::renders_to_buffer` exists so a host knows whether
it needs a device at all, which is what keeps an FMOD backend — which owns its own output thread — from
having to pretend.

## Gate status

Formatting, strict lints (`clippy::all` and `clippy::pedantic` as errors, plus `-D warnings` as CI runs
it), and the full test suite all pass on the pinned toolchain. The CI runner runs the same suite against
Mesa's lavapipe.

The two newest crates are the ones worth describing. **`cic-math`** holds ADR 0007's arithmetic, extracted
from `cic-script` so the kernel can share it; it carries its own copy of the decision-8 restriction scan,
and the series tests moved with the code rather than being duplicated. **`cic-sim`** carries the kernel
mechanics' unit tests, another copy of the scan, the replay suite whose headline test is M5's exit
condition run twice, and the activation tests — spanning the one-moved-placement divergence and the
template resolutions. The template format's own tests live in `cic-assets`, beside the format.

> **No test tally here, on purpose.** This paragraph used to open with a total and a per-crate
> breakdown, and it had drifted by about ninety-five tests before anyone noticed — which is what a
> hand-maintained inventory does. The rule this settles on: **a number in the tree's prose should be a
> measurement that argues something, not an inventory that rots.** The frame times, the mip-chain border
> values and the aliasing figures below all argue a point and stay true; a test count argues only that
> tests exist, and is wrong a week later. Counts belong in a pull request body, which is a record of one
> moment and never goes stale, or behind a generator that CI diffs — as the
> [design documents' derived counts](docs/design/mechanics.md#10-what-this-document-obliges-the-engine-to-gain)
> now are.

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
