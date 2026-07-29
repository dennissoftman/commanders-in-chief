# M9: Audio

Sound: a mixer behind a replaceable backend, positional effects, music, DSP, and the authored layer
between "the rifle fired" and "play a file".

**Status:** Charter met, with the device layer and a compressed format explicitly outstanding. The
engine mixes, spatialises, filters and limits; what it does not yet do is hand the result to a sound
card.

## Charter

- A **replaceable backend**, so FMOD, OpenAL, or another library can be substituted without engine
  policy changing. **Done** — see [ADR 6001](../adr/6001-audio-backend-boundary.md) for where the
  boundary went and why the obvious placement was wrong.
- A **mixer** with named buses, per-bus effects, and a master limiter. **Done.**
- **Positional audio**: a listener, distance curves, panning, Doppler, cones, occlusion. **Done.**
- **DSP**: filters, reverb, and dynamics, described in data and instantiated at a sample rate. **Done.**
- **Music** that does not have to guess how long a scene will last. **Done**, as layered stems.
- An **authored format** for sound events, so gameplay code names a cue rather than a file. **Done** —
  see [the sound bank specification](../formats/sound-bank.md).
- **A device**, so the mix reaches a speaker. **Not done.** See [Remaining](#remaining).

## Landed

- **`cic-audio`**, depending on `cic-core` for the bounded WAV reader and `serde` for the bank format,
  and on **no audio library of any kind**. The engine can make a sound with nothing installed, which is
  what keeps the README's licence claim true for anyone who builds it.
- **The backend boundary is a command interface**, not a sample sink. FMOD and OpenAL are complete
  mixers rather than devices, and a sink would discard everything anyone adopts either of them for. The
  full argument is in the ADR; the consequence is that engine policy — which cue, which variant, which
  bus, what priority, whether the budget allows it — is written once and does not move when the
  implementation does.
  - **A conformance suite exists before the second backend does**, which is the point: the properties a
    second implementation quietly fails to have are exactly the ones nobody writes down. A stopped voice
    stops. A stale handle does not control a recycled voice. Stopping one bus leaves the others alone.
    Setting a voice that already ended is ignored rather than an error.
  - **Two backends will not produce identical samples**, and that is the direct cost of putting the
    boundary where the value is. Behavioural properties are asserted of any backend; every numeric
    assertion names `SoftwareMixer`.
- **A software mixer written from scratch**: a linear resampler, constant-power panning, a per-voice
  distance filter, per-bus effect chains, and a limiter. About 700 lines, and it makes **every
  assertion in the crate run headless** at whatever sample rate a test picks — which is the thing the
  renderer needed a whole capture harness and per-adapter reference images to approximate.
  - **Gain ramps across a block; spatialisation is computed once per block.** Doing the second per frame
    is waste, and applying its result from the first sample of the block is a step discontinuity at the
    block rate — at 512 frames, a 94 Hz buzz that follows the camera.
  - **Stopping a voice fades it over five milliseconds.** Cutting a waveform mid-cycle is a click, and a
    game stops sounds constantly. The handle dies immediately, so a caller observes what it expects
    while the slot lingers.
  - **Voice stealing compares priority first and loudness second.** With fifty rifles at one priority
    the one to drop is the furthest away, not whichever sits in the highest slot index. A lower-priority
    sound cannot displace a higher-priority one at all, or a looping ambience restarting every second
    would evict speech.
- **A listener placed for an overhead camera**, which is a genre problem rather than a detail. A camera
  sixty metres up hears a firefight forty metres away as seventy-two metres away, and the pan collapses
  because everything is nearly straight down. `Listener::for_overhead_camera` lowers the ear along the
  line to the focus point while keeping the camera's *orientation*, so the image widens and screen-left
  is still audibly left.
- **Distance curves as three shapes rather than one**, because they are not interchangeable: the inverse
  law never reaches silence, so a sound using it is audible map-wide and cannot be distance-culled;
  linear reaches exactly zero, which is what a designer wants for a sound that must be inaudible outside
  a radius. `silence_distance` returns `None` for the curves that never get there, so a caller cannot
  invent a cull radius for one.
- **Occlusion is two effects, not one.** Blocking geometry removes high frequencies far faster than it
  removes energy, so a sound that is merely quieter reads as distant while one that is quieter *and*
  duller reads as being behind something. Whether a building is in the way is a question about the
  world, so the value arrives from the caller.
- **A DSP set with its constants derived in the file.** Filters, a reverberator, and one dynamics
  processor serving as both compressor and limiter.
  - **The reverb's comb delays are prime**, and this project has been caught by the absence of that
    once: two combs at related lengths reinforce at every common multiple, so the tail rings at one
    pitch. It is the same near-harmonic failure as the five water waves that interfered into a visible
    diamond lattice.
  - **The limiter needed a hold stage and a lookahead delay**, and both were added because the textbook
    arrangement failed its own assertion at 1.12 rather than because a book recommended them. Written up
    in the ADR.
- **Buses as a closed enum**, so a bank cannot route to a destination the engine does not define and a
  settings screen's list of volume sliders does not depend on what content happens to be installed.
  - **Snapshots**, so ducking is not a special case. A briefing engages a named set of offsets that
    blend in and out; several can be active at once and they *sum*, so a pause during a briefing is the
    two together rather than a race to restore the gain.
- **A sound bank format** ([specification](../formats/sound-bank.md)) carrying what makes game audio not
  sound like a slideshow: variants, weights, pitch spread, polyphony, cooldown, priority. All of it is a
  property of the *event*, so it belongs in a file a sound designer edits rather than in the code that
  fires the rifle.
  - **A variant is not repeated immediately** where the cue has an alternative. With three variants
    chosen uniformly the same one comes up twice running about a third of the time, and a repeat is the
    most audible thing in a sound set — it is the one case where a listener has an exact reference.
  - **Cooldown and polyphony are what make a volley sound like a volley.** Forty units firing on one
    tick is forty voices starting on the same sample, which sums correlated with itself and reads as one
    loud crack.
- **Layered music**, because a strategy game does not know how long its own scenes are. Stems play
  together and the mix between them follows an intensity that *ramps* — music tracking the game
  instantly reads as a meter rather than as a score.
  - **Layers are started together and never individually restarted.** They stay in phase only because
    they began on the same frame, so an inaudible layer plays at silence rather than stopping. Restarting
    one to bring it back in produces not quiet music but wrong music.
  - **Crossfades are equal power**, for the reason panning is: a linear crossfade dips 3 dB in the
    middle, and it does it at a moment the listener is already attending to.
- **A bounded WAV decoder** built on the same `BinaryReader` every other decoder uses. Unknown chunks are
  skipped and unknown *format tags* are not — they look like the same kind of unknown and are not, since
  a format tag names how every byte of the payload is to be read.
- **Audio carries its own random stream.** A determinism consequence rather than an audio one: variant
  selection and pitch spread need randomness on every gunshot, and a stream consumed by presentation is
  a stream the simulation did not consume on the other machine.

## Remaining

- **A device.** The mixer produces frames and nothing hands them to hardware. This is a host concern —
  `Capabilities::renders_to_buffer` exists so a host knows whether it needs one — and it is the piece
  that cannot be verified headlessly, which is the same lesson the renderer recorded as *presentation
  needs running, not just testing*.
- **A compressed format.** WAV only. Vorbis or Opus arrives as a dependency decoding to the same `Clip`,
  with its own `NOTICES.md` entry. Uncompressed short effects are what a game wants in memory anyway;
  it is music that will need this.
- **An FMOD or OpenAL backend**, which is now a matter of implementing a trait rather than a matter of
  design. Either is FFI, so it is an out-of-workspace crate or one declaring its own lint posture — see
  the ADR's consequences.

## Exit condition

A scenario can name a cue and have it play at a world position, at the right level, on the right bus,
with the player's volume settings applied — verified by tests that run with no audio device present.

**Met.** 125 tests across the decoder, the spatial model, the DSP, the mixer, the bank, the score and
the frontend, all headless.

The exit condition is deliberately silent about a speaker. What can be *tested* is that the right
samples are produced; that they reach a sound card is a host integration, and writing an exit condition
that a green suite cannot establish would be writing one that lies.

## Explicitly not done

- **No HRTF or binaural rendering.** Stereo constant-power panning with a distance filter. An overhead
  strategy view is the case where head-related transfer functions buy least, and they are a large amount
  of measured data with its own provenance question.
- **No streaming.** Clips are decoded whole. Streaming matters for music, and music is also what wants
  the compressed format, so the two arrive together or not at all.
- **No convolution reverb.** The algorithmic reverberator is a few hundred lines and no data; a
  convolution reverb is an FFT and a library of impulse responses somebody recorded, which is a content
  and provenance question rather than a code one.
- **No audio thread.** Deliberate while there is no device: `render` is a pure function a caller invokes,
  which is what makes it testable. The threading question belongs with the device.
