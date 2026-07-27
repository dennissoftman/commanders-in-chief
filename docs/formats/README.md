# Native asset formats

Every format this engine reads is either its own or a published standard. Nothing here documents another
game's format, because nothing here reads one.

| Format | Purpose | Specification |
|---|---|---|
| `.cict` | Terrain heightfield and layer weights | [terrain.md](terrain.md) |
| `map.json` | Scenario: players, placements, waypoints | [scenario.md](scenario.md) |
| `.cicmap` | Map package | [package.md](package.md) |
| `.glb` | Models, props, units | [glTF 2.0](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html) |
| `.zip`, `.tar` | Content containers | Their own published specifications |

## The choice behind the choices

The formats look inconsistent until you notice where the line is drawn.

**Authored data is text.** A scenario is edited by hand, reviewed in diffs, merged between people, and
occasionally repaired when a tool misbehaves. Every one of those needs the file to be readable.

**Bulk numeric data is tight binary.** A heightfield is generated, read only by machines, and needs to
reach the GPU without a conversion pass — its `u16` samples are copied straight into an `R16Uint`
texture. Its regularity is what makes terrain cheap, and a
general-purpose encoding throws that away.

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
