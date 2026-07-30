# M11: Persistence

What survives the process: a running match saved and restored, and the player's settings.

**Status:** Planned.

## Charter

- Save and load of a running match: the kernel's state written to a versioned file, and restored into
  a kernel that continues as if it had never stopped.
- The save is the hash's view of the world. Whatever the per-subsystem hashes cover is exactly what
  the save contains, produced by the same explicit write path — one serialization, not two that can
  drift, so a field that can desync is by construction a field that saves.
- Verification at load: a loaded save must reproduce the per-subsystem hashes recorded at save time
  *before* the first tick advances. A corrupt or truncated save is refused with the subsystem named,
  not played until it misbehaves.
- Versioning: a save names the state-layout version that wrote it, and an unknown version is refused
  with both versions in the error. A save file is hostile input like any other — the
  [binary parsing invariants](../invariants/binary-parsing.md) apply in full.
- Settings persistence: video, audio, and input bindings in a text format that *tolerates* unknown
  keys. Settings are preferences, not simulation state, which is why lenience is safe here and
  forbidden one bullet up.

## Exit condition

A skirmish saved mid-match, the process ended, the save loaded, and the match run to completion
produces per-tick hashes identical to the same match run without interruption — verified in CI.

## Design notes

This is chartered as its own milestone rather than folded into M5 because
[M5 explicitly excluded it](m5-simulation.md#explicitly-not-done): a save format is a contract about
the state layout, and stabilising a layout whose only occupant was a placeholder subsystem would have
frozen the wrong thing. [M6](m6-gameplay.md) populates the kernel with its real subsystems; this
milestone is where their on-disk shape becomes a promise.

Two designs were considered. A replay-based save — initial state plus the command log — already
exists for free out of M5 and costs almost nothing to write, but its load time grows with match
length and it cannot, even in principle, outlive a state-layout change. A state snapshot loads in
constant time and is what this charter specifies. The command log stays what it is: the replay and
desync-diagnosis tool, not the save format.

A lockstep save is the same state on every client by construction, so saving a networked match costs
nothing extra at the format level. *Resuming* one is a session-negotiation problem that belongs to
[M7](m7-network.md); the format's only obligation is not to preclude it.

## Explicitly not done

- No save migration across state-layout versions. Migration is real machinery and wants a stable
  layout to migrate *from*; until one exists, a version mismatch is a clean refusal rather than a
  best-effort read.
- No autosave policy. Cadence and rotation are shell concerns to layer on once saving itself is
  proven.
- No campaign progress. It follows a campaign existing.
- No cloud sync, no per-machine profiles.
