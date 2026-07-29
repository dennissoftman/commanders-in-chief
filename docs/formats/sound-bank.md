# Sound bank (`*.cicbank.json`)

What a sound *event* is, as distinct from what a file is. JSON, schema version 1.

Gameplay code names a cue — `bank.play("unit.rifle.fire", at)` — and everything that makes game audio
not sound like a slideshow lives in this file: which recordings the cue chooses between, how far the
pitch is allowed to wander, how many may sound at once, how soon another may follow, and how it falls
off with distance. All of it is a property of the *event*, so a sound designer changes it without
touching code.

## Example

```json
{
  "format_version": 1,
  "cues": {
    "unit.rifle.fire": {
      "bus": "effects",
      "variants": [
        { "clip": "audio/rifle_a.wav", "weight": 2 },
        { "clip": "audio/rifle_b.wav" },
        { "clip": "audio/rifle_c.wav", "gain_db": -1.5 }
      ],
      "gain_db": -3.0,
      "pitch_range": [0.94, 1.06],
      "attenuation": { "kind": "inverse", "reference": 8.0, "far": 400.0, "rolloff": 1.0 },
      "polyphony": 4,
      "cooldown_ms": 45.0,
      "priority": 140
    },
    "vehicle.tank.engine": {
      "bus": "effects",
      "variants": [{ "clip": "audio/tank_loop.wav" }],
      "looping": true,
      "fade_in_seconds": 0.25,
      "doppler": 0.6,
      "cone": { "inner_degrees": 90.0, "outer_degrees": 240.0, "outer_gain": 0.55 },
      "attenuation": { "kind": "exponential", "near": 6.0, "far": 220.0, "rolloff": 1.6 }
    },
    "ui.button.press": {
      "bus": "interface",
      "variants": [{ "clip": "audio/click.wav" }]
    }
  }
}
```

`ui.button.press` declares no `attenuation`, which is what makes it *unpositioned*: it plays at full
level in both ears whatever the camera is doing. Triggering it at a world position does not change that
— a cue with no distance curve has no position, and that is a property of the cue rather than of the
call site.

## Cue fields

| Field | Required | Default | Notes |
|---|---|---|---|
| `variants` | **yes** | — | At least one. See below. |
| `bus` | no | `"master"` | One of `master`, `music`, `effects`, `speech`, `interface`, `ambience`. |
| `gain_db` | no | `0.0` | Applied to every variant. |
| `pitch_range` | no | `[1.0, 1.0]` | Inclusive, ascending, both positive. A playback rate is drawn from it per instance. |
| `attenuation` | no | absent | The distance curve. Absent means the cue has no position. |
| `cone` | no | absent | `{ "inner_degrees", "outer_degrees", "outer_gain" }`. |
| `spread` | no | `0.0` | `0.0` is a point source, `1.0` removes the pan entirely. |
| `doppler` | no | `0.0` | `0.0` is off, `1.0` is physical. Above one exaggerates. |
| `polyphony` | no | `8` | How many instances may sound at once. At least one. |
| `cooldown_ms` | no | `0.0` | How long after one instance starts before another may. |
| `priority` | no | `128` | What an instance is worth when the voice budget is full. |
| `looping` | no | `false` | Whether an instance repeats until stopped. |
| `fade_in_seconds` | no | `0.0` | A looping sound started at full gain clicks, because a clip's first sample is rarely zero. |

## Variant fields

| Field | Required | Default | Notes |
|---|---|---|---|
| `clip` | **yes** | — | Virtual path, resolved through the resource layer. Never a filesystem path. |
| `weight` | no | `1` | Relative to its siblings. Zero disables a variant without deleting it. |
| `gain_db` | no | `0.0` | For levelling one recording against another. |

## Attenuation curves

Three shapes, and they are not interchangeable.

```json
{ "kind": "linear",      "near": 5.0, "far": 200.0 }
{ "kind": "inverse",     "reference": 8.0, "far": 400.0, "rolloff": 1.0 }
{ "kind": "exponential", "near": 6.0, "far": 220.0, "rolloff": 1.6 }
{ "kind": "none" }
```

- **`inverse`** is the physical one: -6 dB per doubling of distance at a `rolloff` of one. It **never
  reaches silence**, so a sound using it is audible at some level across the whole map and cannot be
  distance-culled. Right for gunfire and explosions.
- **`linear`** reaches exactly zero at `far`, which is what a designer wants for a sound that must be
  *inaudible* outside a radius.
- **`exponential`** sits between them and holds a sound up longer before dropping it. The usual choice
  for ambience.
- **`none`** applies no falloff, which is not the same as omitting `attenuation` entirely: `none` still
  makes the cue positional, so it pans with the listener while staying at full level.

## Why variants, pitch spread, polyphony and cooldown are all here

Because each one is the difference between a sound set and a loop, and none of them belongs in the code
that fires the rifle.

**Variants.** One clip played twenty times is not twenty rifle shots, it is a machine gun with a very
obvious loop. **A variant is not repeated immediately** where the cue has an alternative — with three
variants chosen uniformly the same one comes up twice running about a third of the time, and a repeat is
the most audible thing in a sound set because it is the one case where a listener has an exact reference
to compare against. A cue with a single variant still plays; the suppression only applies where
something else could be chosen.

**Pitch spread.** Even across variants, exact repetition is audible. A few percent either way is
inaudible individually and removes the mechanical quality entirely.

**Polyphony and cooldown.** Forty units firing on one tick is forty voices starting on the same sample.
That sums to something forty times louder than one *and* correlated with itself, so it reads as a single
loud crack rather than as a volley — and it is what actually drives the mixer into its limiter. A cue
admitting four instances and refusing another within 45 milliseconds sounds like *more* units, not
fewer.

## Buses

A closed set, for the reason [the interface layout's action set](ui-layout.md) is one: a bank is data,
and data must not name a destination the engine did not define. `"bus": "secret"` fails to load rather
than routing nowhere.

It is also the shape the player needs. A volume settings screen is a fixed list of sliders, and every
one of them has to exist whether or not any content currently routes to it.

| Bus | For |
|---|---|
| `master` | Everything ends here. Carries the limiter. |
| `music` | Score and ambient beds. |
| `effects` | The world: weapons, engines, impacts, construction. |
| `speech` | Unit responses, briefings, the advisor. |
| `interface` | Buttons and notifications — the shell rather than the world. |
| `ambience` | Wind, rain, and the rest of the weather bed. |

## What the loader refuses

Unknown fields, an unknown version, an unknown bus, and any cue that could never make a sound: no
variants, a variant naming no clip, every variant at weight zero, a polyphony of zero, or a pitch range
that is not a positive ascending interval.

The refusal happens **at load, naming the cue**. A cue that could never sound is not a cue that plays
quietly — it is a silence somebody will eventually notice and have no way to diagnose.

Clip paths are *not* checked at load, because a bank and the clips it names are separately mounted and a
mod may supply either. `AudioEngine::missing_clips` reports what has not been bound, which is what a
content tool checks and what a host loads.
