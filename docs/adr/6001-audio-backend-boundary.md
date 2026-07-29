# ADR 6001: Where the audio backend boundary goes, and why the default is our own mixer

- Status: accepted and **implemented**. All five decisions are in.

## Context

The engine needs audio, and the requirement as posed was that the implementation be switchable — FMOD,
OpenAL, or another established library — rather than welded in. That is the right requirement, and
satisfying it turns entirely on one question that is easy to answer wrongly: **what, exactly, is on each
side of the boundary?**

There is an obvious answer. Make the backend a *device*: this engine mixes, spatialises, and filters,
and the backend receives finished stereo frames and hands them to hardware. It is simple, it makes every
backend produce byte-identical output, and it makes the whole audio system testable without a device.

It is also wrong, and the reason is not a matter of degree. **FMOD and OpenAL are not devices.** Each is
a complete mixer with its own spatialisation model, its own DSP graph, its own voice management, and in
FMOD's case an authoring tool that a sound designer works in and a project's whole sound content lives
in. Reducing either to a sample sink discards all of it. What is left is a "switchable backend" that
switches the last hundred microseconds of the signal path and nothing anyone would adopt a library for.

Three further constraints bear on the choice, and none of them is negotiable here:

**`unsafe_code` is forbidden at workspace scope.** FMOD and OpenAL are C libraries, so any first-party
binding is FFI and therefore `unsafe`. A backend crate for either cannot live under the workspace lint
set as it stands.

**The licence has to be defensible.** `LICENSING.md` records a provenance audit that made a permissive
licence possible in the first place. **FMOD is proprietary**, licensed per title, free only below a
revenue threshold, and requires attribution in the shipped product. **OpenAL Soft is LGPL**, which is
comfortable dynamically linked and a real constraint statically. Neither can be the thing this engine
*requires* in order to make a sound, or the licence claim in the README stops being true for anyone who
builds it.

**Audio is presentation, but it is adjacent to the simulation.** It reads game state constantly and must
never write to it — and, less obviously, must never draw from a simulation random stream, because
drawing from one is part of that simulation's state transition.

## Decision

1. **The boundary is a command interface, not a sample sink.** [`Backend`](../../crates/cic-audio/src/backend.rs)
   takes clips, voice requests, listener state, and bus gains. It does not take or return samples.
2. **Engine policy lives above the boundary**, in [`AudioEngine`](../../crates/cic-audio/src/engine.rs):
   which cue fires, which of its recordings, at what pitch, on which bus, at what priority, whether the
   polyphony budget and cooldown allow it, what the bus gains currently are, and what the score is
   doing. None of it changes when the implementation does.
3. **Mixing, spatialisation, DSP, and the device live below it.** That is the half FMOD or OpenAL would
   replace, and it is the half each of them is good at.
4. **The default implementation is a software mixer written from scratch**
   ([`SoftwareMixer`](../../crates/cic-audio/src/mixer.rs)), depending on nothing. It is what makes the
   licence claim hold and what makes the whole system testable headlessly.
5. **A backend that produces frames for a caller-owned device implements a second trait**,
   `RenderToBuffer`, rather than a method on `Backend` that every FFI implementation would stub out
   with an error.

## Rationale

**The command boundary is the only one where both answers are real.** With a sample sink, the software
mixer is the implementation and FMOD is a downgrade of it. With a command interface, both are complete
implementations of the same contract, and the choice between them is a genuine one — the software mixer
for a build that must stay permissive and dependency-free, FMOD for a project that has bought it and
wants its authoring tool.

**Decision 5 is what stops the trait lying.** A backend that owns its own output thread — the FMOD and
OpenAL case — has no meaningful `render` to give. A trait method every FFI implementation returns
`Unsupported` from is a design that has already decided which implementation is real.
`Capabilities::renders_to_buffer` tells a host which kind it has, so it knows whether it needs a device
at all.

**A software mixer written from scratch was cheaper than it sounds and buys more than audio.** It is
about 700 lines: a linear resampler, constant-power panning, a distance filter, per-bus effect chains,
and a limiter. What it buys beyond making a sound is that **every assertion in the crate runs headless**,
at whatever sample rate the test picks, with no device present — 125 of them. The renderer needed a
whole capture harness and per-adapter reference images to reach a comparable place. Audio gets there for
free because a mixer is a pure function from voices and a listener to frames.

**Two backends will not produce identical samples, and that is stated rather than hidden.** It is the
direct cost of putting the boundary where the value is. So the properties asserted of *any* backend live
in [`backend::conformance`] and are behavioural — a stopped voice stops, a stale handle does not control
a recycled voice, stopping one bus leaves the others alone — while every numeric test names
`SoftwareMixer` explicitly.

## Consequences

- **An FMOD or OpenAL backend is an out-of-workspace crate**, or an in-workspace one that declares its
  own lint posture, because it is FFI. That is a deliberate consequence rather than an oversight: it
  keeps `unsafe_code = "forbid"` true of everything currently in the tree, and it puts the licence
  question for either library at the boundary of the project rather than inside it.
- **`NOTICES.md` is unchanged**, because the audio system added no dependency. Adding either library
  later is a change with its own notice entry and its own line in `LICENSING.md`.
- **A compressed audio format is a later dependency, not a later decoder.** The engine reads WAV itself,
  because a chunked PCM container can be made to satisfy [the binary parsing
  invariants](../invariants/binary-parsing.md) in an afternoon and a subband codec cannot. Vorbis or
  Opus arrives as a dependency decoding to the same `Clip`.
- **Audio carries its own random stream** ([`AudioRandom`]), seeded independently. This is a
  determinism consequence rather than an audio one: variant selection and pitch spread need randomness
  on every gunshot, and [the determinism invariants](../invariants/determinism.md) forbid presentation
  from consuming a simulation stream.
- The conformance suite is what a second backend is measured against. It exists before there is a second
  backend, which is the point — those properties are exactly the ones a second implementation quietly
  fails to have.

## What implementing it established

Recorded here rather than only in the code, because two of these were defects that the obvious
implementation had and the tests caught, and one is a measurement that changed a constant.

**A limiter without a hold stage and a lookahead delay is not brick-wall, and the failure is arithmetic
rather than tuning.** The textbook arrangement — smooth the rectified level, compute a gain from it —
let a sine at eight times full scale out at **1.12**. Two independent causes. First, a rectified sine
passes through zero twice a cycle, so an envelope that releases during the trough has sagged by the time
the next peak arrives: at 220 Hz with a 120 ms release that is 2.6 dB, which is exactly the overshoot
measured. Second, the attack ramp is applied to the peak that triggered it, which is a contradiction —
by the time the envelope has reacted, the sample that caused it has been multiplied by the old gain and
left. The fixes are a **hold** (a new peak arms a timer; the envelope may not release until it expires)
and a **lookahead delay** of four attack time constants, so the reduction is in place before the peak it
was computed for emerges. The test now checks five frequencies from 55 Hz to 4 kHz, because the failure
is worst at low ones and testing 220 Hz alone would have passed with the bug present at 55.

**The master limiter's lookahead is a latency the whole mix pays, and it has to be reported.** 192 frames
at 48 kHz, four milliseconds. Two tests were written wrong before this was pinned in a test of its own —
both read output frames that had not emerged yet and concluded the mixer was silent.

**Gain has to be ramped across a block even though spatialisation is computed once per block.**
Recomputing a square root and six dot products per frame for a quantity that changes over milliseconds
is waste; *applying* the new gain from the first sample of the block is a step discontinuity every block
boundary, which at a 512-frame buffer is a 94 Hz buzz that follows the camera. Spatialisation is
therefore per block and the gain it produces interpolates across it. The filter cutoff still steps,
which is inaudible for a stated reason: a filter's output is continuous in its coefficients and a gain's
is not.

**The reverb's comb delays are prime, and this project has already been caught by the absence of that
once.** Two combs at 1200 and 1800 samples both reinforce at every multiple of 3600, so their echo
trains land on top of each other forever and the tail rings at one pitch. It is the same near-harmonic
failure as the five water waves that interfered into a visible diamond lattice, in a different domain.
Prime lengths share no multiple below their product.

**The listener is not the camera, and in this genre that is the whole problem.** An RTS camera sits sixty
metres above the battlefield looking down. Put the ear there and every sound is distant and centred: the
pan collapses because everything is nearly straight down, and a firefight forty metres away reads as
seventy-two metres away because the height dominates the horizontal distance.
`Listener::for_overhead_camera` places the ear along the line from camera to focus at a caller-chosen
fraction, while the *orientation* stays the camera's — so the mix widens and screen-left is still
audibly left.

**One bug was not audio at all.** Bus gains were ramping from unity on the very first block instead of
starting at the level they were configured at, so a bus muted before playback began still put a
full-level transient into the master limiter — which then spent its whole 120 ms release recovering from
a sound that was never meant to be audible.
