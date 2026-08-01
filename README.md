# Commanders in Chief

An open-source real-time strategy engine, written in Rust, using its own asset formats.

A player commands a faction on a heightfield map: gather resources, construct a base, produce units,
and fight other players or the AI in real time. Classic base-building RTS, built from scratch.

This is not a reimplementation of, or a compatibility layer for, any existing game. Nothing here reads
another game's data or derives from another game's code.

## The game

Three belligerents contest a single trade corridor. All three want the road open — the war exists because
"open" has three incompatible definitions, and none of them is fighting for territory. They are fighting
over the terms of passage.

The setting has no correct side. Each faction has a defensible internal logic, a constituency it
genuinely serves, one real thing it is wrong about, and an internal wing that thinks the current course
is a mistake. Institutions here cause harm through process rather than malice; a form filed correctly
should be able to kill people. The register is restrained and procedural, never comic and never "dark".

| | **AEC** | **Concord** | **Meridian** |
|---|---|---|---|
| Full form | Allied Expeditionary Command | Continental Concord | Meridian Council |
| Colour *(campaign)* | blue | green | yellow |
| Names itself after | a command structure | a civilisation | a line of connection |
| Claim to legitimacy | mandate, legality, coalition consent | delivered outcomes — the road exists | presence, and the consent of people on the road |
| What it cannot admit | that it is a foreign army | that order is being imposed | that it needs the war |
| Production | **Logistics.** Force arrives off-map via pads and drop zones | **Industry.** Few huge factories, batch delivery, permanent infrastructure | **Occupation.** Converts existing map buildings into production |
| Economy | expensive, precise, air-dependent | slow to spin up, then unstoppable | taxes throughput on held route nodes |
| Army | small, exact, drone-repaired | mass with attrition tolerance | salvaged enemy hulls at degraded reliability |
| Buys | **vision** — ISR sweeps on cooldown | **permanence** — graded roads, depots | **ambiguity** — tunnels, false signatures |
| Breaks against | air denial, EW and GPS denial | anything it did not plan for | being observed and pinned |

Those colours are the campaign's. Skirmish and multiplayer let a player pick their own, unique within
the game, because a match can field two seats of the same faction and ownership still has to be legible.

Every faction covers every tactical role. Divergence is in *how* a role is filled, never in whether it
exists — no faction is helpless against anything.

The full treatment is the **[faction character bible](docs/design/faction-bible.md)**: self-conception,
voice, doctrine, aesthetic, internal fault lines, and the lexicon of what each faction calls the others.
It is a specification for generated text, not flavour — briefing copy, unit barks, and UI strings will be
written by several people and tools over the project's life, and the failure mode is not bad writing but
*inconsistent* writing.

How those factions are *played* is [docs/design/mechanics.md](docs/design/mechanics.md), and the economy
under it is [ADR 3002](docs/adr/3002-corridor-economy.md): goods enter at the map's edges, accumulate at
yards, and have to be carried somewhere by units that can be killed. One currency, earned three
different ways — AEC's freight arrives on off-map sorties it does not own, Concord hauls overland on
roads it built, Meridian charges a duty on other people's carriage. The setting's premise is the
mechanic: the economy is passage.

And a link of the corridor where somebody's assets are being destroyed **stops carrying freight** until
the fighting does. So a grinding battle on the middle of the map makes both sides poorer, the efficient
economic attack is a deep raid behind the enemy's line, and Meridian — which is paid out of throughput
and paid again out of wreckage — ends up with its two halves on opposite sides of the war's intensity.

Per-faction detail is [faction-mechanics.md](docs/design/faction-mechanics.md) and the method for
setting any number is [balance.md](docs/design/balance.md).

It is also a source of engine requirements, and has already been one. A faction whose map presence is
literally paved has to grade roads across terrain at runtime, and a faction that converts existing
buildings into production needs those buildings to be destructible map objects — which is why terrain
heights and layer weights are writable GPU textures rather than a baked mesh. An army fielded from
recovered enemy hulls is one mesh under many markings, which is why a model instance carries its own
colour. Both were nearly free to design for and expensive to retrofit.

## Status

Early. The foundation, resource layer, asset formats, and interface are complete, the renderer meets its
exit condition, the audio layer mixes, scenario behaviour can be written in a script, and **the
simulation kernel runs** — a scenario activates into it, units take orders, and they path around
ground they cannot cross. There is no playable game yet: there is no combat, no economy, no
construction and no opponent, and the audio does not yet reach a speaker. See
[ROADMAP.md](ROADMAP.md) for the milestone ladder and [CURRENT.md](CURRENT.md) for what is being
worked on now.

## Building

Requires the pinned toolchain in `rust-toolchain.toml`, which `rustup` installs automatically.

```bash
cargo test --workspace
```

The full gate, which is what CI runs:

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

To see it draw something:

```bash
cargo run -p cic-render --example terrain_viewer --release
```

Pass a `.cicmap` path to view a real map; with no argument it generates terrain, buildings, and their
surfaces, so the viewer runs before any content exists. `T` cycles antialiasing through none, the post
pass, and the temporal one; the bracket keys step the resolution scale; `P` prints a per-pass GPU timing
breakdown. The viewer's own header lists every control.

And to navigate the shell:

```bash
cargo run -p cic-render --example shell --release
```

A main menu, settings, skirmish setup, and a quit modal, drawn with a typeface authored in this tree.
Change the resolution scale, press Apply, and do nothing: fifteen seconds later the setting takes itself
back, because a display change can leave the person who made it unable to see well enough to undo it.

## Crates

| Crate | Purpose |
|---|---|
| `cic-core` | Bounded binary reading with structured errors. No dependencies. |
| `cic-vfs` | Virtual paths, ordered mounts, overlay resolution, zip and tar containers. |
| `cic-assets` | glTF model import, the terrain container, JSON scenarios, map packages. |
| `cic-camera` | The RTS camera model, free of window, input, and GPU dependencies. |
| `cic-ui` | Interface layout and its format, the two-pass solver, widgets and retained state, input routing with input-method composition, the screen stack, transactional settings, screen transitions, and the paint layer. Free of window, GPU and font dependencies. |
| `cic-render` | Deferred chain, terrain, instanced models, physically-based texturing, scenery sway, antialiasing to a temporal tier, interface drawing with an authored typeface, windowed presentation, the WGSL shader set. |
| `cic-audio` | Mixer behind a replaceable backend, spatialisation, DSP, layered music, sound banks. No audio dependency at all. |
| `cic-math` | ADR 0007's simulation arithmetic: the permitted operations and the turn-based transcendentals, shared by everything that runs inside the simulation. No dependencies. |
| `cic-script` | A deterministic sandboxed language for scenario logic: `cic-math` arithmetic, no heap, fuel-metered. |
| `cic-sim` | The deterministic fixed-tick kernel: ordered subsystems that may read their peers but mutate only themselves, stable ids, named random streams, command recording, replay, per-tick state hashes, scenario activation — and gameplay above it: unit orders, and a passability grid searched with integer A\*. |

## Asset formats

| Data | Format |
|---|---|
| Models, props, units | glTF 2.0 (`.glb`) |
| Terrain heightfield and layers | Custom chunked binary (`.cict`) |
| Scenario | JSON (`map.json`) |
| A whole map | zip (`.cicmap`) |
| One interface screen | JSON (`*.ciclayout.json`) |
| Display text | JSON, keyed (`strings.<language>.json`) |
| Sound events | JSON (`*.cicbank.json`) |
| Audio clips | RIFF/WAVE (`.wav`) |
| Scenario behaviour | A small language of the project's own (`*.cics`) |

Specifications are in [docs/formats/](docs/formats/README.md), and the reasoning is in
[docs/milestones/m2-assets.md](docs/milestones/m2-assets.md). The short version: authored data a human
edits and reviews should be text, bulk numeric data should be tight binary, and geometry should use a
standard that content tools already export.

## Audio

[`cic-audio`](crates/cic-audio/) is a mixer behind a replaceable backend. The requirement was that FMOD,
OpenAL, or another library could be substituted, and the whole decision turned out to be *where the
boundary goes*: it is a command interface rather than a sample sink, because FMOD and OpenAL are complete
mixers rather than devices, and a sink discards everything anyone adopts either of them for.

The in-tree implementation of that trait is a software mixer written from scratch with no audio
dependency at all, which is what keeps the licence claim below true for anyone who builds it — FMOD is
proprietary and per-title licensed, and OpenAL Soft is LGPL. It also makes the whole system testable
headlessly, since a mixer is a pure function from voices and a listener to frames. See
[ADR 6001](docs/adr/6001-audio-backend-boundary.md).

## Scripting

[`cic-script`](crates/cic-script/) puts scenario behaviour in data: a small language compiled to
bytecode and run by an interpreter that cannot allocate, cannot hang, and cannot reach anything the
engine did not offer it.

Its arithmetic is not its own — it inherits
[ADR 0007](docs/adr/0007-simulation-arithmetic.md)'s exactly, because a script and the simulation kernel
have to reach the same answer on the same two numbers. What makes it a language of this project's own
rather than Lua or Rhai is that those resolve calls at *run* time, so a mod naming a verb the engine
lacks fails when a player triggers it; here it fails to compile, naming the file and the line. Fuel
bounds its time and the absence of a heap bounds its space, which between them are what "safe to run
untrusted content inside a tick" has to mean. See
[ADR 7001](docs/adr/7001-scripting-language.md) and [the specification](docs/formats/script.md).

## Engineering standards

Two documents that later work is measured against, both of them load-bearing rather than aspirational:

- [Binary parsing invariants](docs/invariants/binary-parsing.md) — every decoder treats its input as
  hostile, takes explicit limits, refuses before allocating, and cannot panic.
- [Determinism invariants](docs/invariants/determinism.md) — lockstep multiplayer, replays, and desync
  diagnosis all reduce to the same requirement, and it cannot be retrofitted.

`unsafe_code` is forbidden at workspace scope. Strict lints are errors in CI. Design decisions with
consequences are recorded as [ADRs](docs/adr/), and [ARCHITECTURE.md](ARCHITECTURE.md) covers the
dependency direction and layering rules.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Commits need a DCO `Signed-off-by` line (`git commit -s`), and
there is one rule that matters more than the rest: **do not port code, data, or constants from another
game.** The permissive licence below exists only because such a derivation was removed, and a single
ported constant table would undo it.

## Licence

The **engine** is licensed under **Apache-2.0** — see [LICENSE](LICENSE) and [NOTICE](NOTICE).
That covers everything you need to build and run it: the crates, the tools, the build configuration, the
format specifications, and the engineering documentation.

The **game's design and narrative content** is reserved — see [LICENSE-CONTENT](LICENSE-CONTENT). That is
`docs/design/`, including the faction bible, plus any narrative or art asset added later. Quoting it to
discuss or review the project is fine; shipping it in your own game is not. None of it is required to use
the engine, so a fork takes the software under Apache-2.0 without touching it.

Third-party dependency notices are in [NOTICES.md](NOTICES.md); all 281 are permissive.
[LICENSING.md](LICENSING.md) explains both choices, records the provenance audit that made a permissive
licence possible, and names the two files still to be written from scratch because of it.
