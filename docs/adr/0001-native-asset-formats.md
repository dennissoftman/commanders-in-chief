# ADR 0001: Native asset formats

- Status: accepted
- Supersedes: nothing

## Context

The engine needs formats for models, terrain, scenarios, and whole maps. The obvious approaches are to
pick one encoding for everything (all JSON, all custom binary, or everything expressed in glTF) or to
choose per kind of data.

## Decision

Choose per kind of data, along one line: **who reads and edits it.**

| Data | Format |
|---|---|
| Models, props, units | glTF 2.0 (`.glb`) |
| Terrain heightfield and layers | Custom chunked binary (`.cict`) |
| Scenario | JSON (`map.json`) |
| A whole map | zip (`.cicmap`) |

## Rationale

**Models → glTF.** A published, versioned standard that every content tool exports, whose data model
already matches what a renderer needs. A custom mesh format would require writing a Blender exporter
before anyone could author a single asset. `.glb` specifically, because its buffers are inline: assets
arrive through the resource layer, where a relative `uri` pointing at the host filesystem has no meaning
and must not be followed.

**Terrain → custom binary.** No standard describes a heightfield well. JSON would store `"1024"` as five
bytes where two suffice and round-trip floats lossily. glTF describes meshes, and expressing a
heightfield as one discards the regularity that makes terrain cheap. `u16` elevations because a 16-bit
integer texture (`R16Uint`) is baseline-supported, so the payload uploads with no conversion pass.

**Scenario → JSON.** The bulk numerics live in the terrain container, so a scenario is kilobytes and the
package compresses it anyway — which erases most of a binary encoding's size advantage. What JSON buys
is diffability: reviewable changes, `git blame` on a balance tweak, hand-merged edits, and repair in a
text editor when a tool misbehaves.

**Map → zip.** A map is three kinds of data at once. Zip already provides a directory, per-member
compression, and universal tooling; a bespoke container would reinvent all three.

## Consequences

- Four decoders instead of one, each with its own tests. Accepted: each is simple because it is not
  compromising for the others.
- Cross-format validation has nowhere natural to live, so the package layer owns it — it is the only
  layer that sees both a scenario and its terrain.
- Scenario JSON rejects unknown fields, so adding a field is a format change requiring a version bump
  rather than something older builds silently ignore. Accepted: a typo in a hand-edited map failing
  loudly is worth more than lenient parsing.
