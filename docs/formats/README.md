# Native asset formats

Every format this engine reads is either its own or a published standard. Nothing here documents another
game's format, because nothing here reads one.

| Format | Purpose | Specification |
|---|---|---|
| `.cict` | Terrain heightfield and layer weights | [terrain.md](terrain.md) |
| `map.json` | Scenario: players, placements, waypoints | [scenario.md](scenario.md) |
| `templates.json` | Template set: what a unit, structure, prop, or faction is | [templates.md](templates.md) |
| `.cicmap` | Map package | [package.md](package.md) |
| `*.ciclayout.json` | Interface layout: one screen's structure | [ui-layout.md](ui-layout.md) |
| `strings.<language>.json` | Display text, keyed, so no layout holds a literal | [ui-layout.md](ui-layout.md) |
| `*.cicbank.json` | Sound bank: what a sound *event* is | [sound-bank.md](sound-bank.md) |
| `*.cics` | Script: behaviour in data — scenario logic and objectives | [script.md](script.md) |
| `.glb` | Models, props, units | [glTF 2.0](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html) |
| `.wav` | Audio clips | [RIFF/WAVE](https://learn.microsoft.com/en-us/windows/win32/xaudio2/resource-interchange-file-format--riff-) |
| `.zip`, `.tar` | Content containers | Their own published specifications |

## The choice behind the choices

The formats look inconsistent until you notice where the line is drawn.

**Authored data is text.** A scenario is edited by hand, reviewed in diffs, merged between people, and
occasionally repaired when a tool misbehaves. Every one of those needs the file to be readable.

**Bulk numeric data is tight binary.** A heightfield is generated, read only by machines, and needs to
reach the GPU without a conversion pass — its `u16` samples are copied straight into an `R16Uint`
texture. Its regularity is what makes terrain cheap, and a
general-purpose encoding throws that away.

**Audio samples use a standard too, and the smallest one that could be read correctly.** WAV is a chunked
PCM container, so a decoder that treats its input as hostile and cannot panic is an afternoon's work; a
subband codec held to the same bar is a research project. A compressed format arrives later as a dependency
with its own notice entry, decoding to the same in-memory clip. See [M9](../milestones/m9-audio.md).

**Geometry uses a standard.** glTF is published, versioned, and exported by every content tool that
matters. A custom mesh format would mean writing an exporter before anyone could author an asset —
paying a large cost to solve an already-solved problem.

## What every decoder guarantees

- Explicit limits, supplied by the caller rather than hardcoded.
- A limit is checked before the allocation it bounds, never after.
- Structured errors naming what was found, what was expected, and where.
- No panics on hostile input.
- Unknown chunks skipped for forward compatibility; unknown *versions* refused.

The full standard is in [binary parsing invariants](../invariants/binary-parsing.md).
