# Repository Agent Rules

These rules apply to every contribution, including LLM-generated work.

## Product boundary

- Build a compatible engine and tools; do not distribute retail game data.
- Keep `ZeroHourLegacy` and `Modern` behavior behind explicit policies, never scattered
  ad-hoc conditionals.
- Tools and engine code may read user-owned installations but must work with synthetic
  fixtures in CI.

## Licensing and provenance

- New project code is GPL-3.0-only.
- GPL source inspection and source-derived implementation are permitted, but every
  derived file or algorithm must name its source, revision, and applicable notices in a
  provenance comment or adjacent document.
- Preserve all copyright notices and GNU GPL Section 7 terms from incorporated source.
- Do not import EA assets, strings, logos, maps, audio, or other game content.
- Never describe this project as official or endorsed by Electronic Arts.

## Architecture

- Dependency direction is tools/render/net/AI -> formats/VFS/sim -> core. Core depends
  on no project crate.
- Parse untrusted data into immutable, renderer-neutral values. Parsers do not mutate
  live simulation or rendering state.
- Simulation never owns renderer, audio, filesystem, or network resources.
- Prefer a small number of crates until a dependency boundary is demonstrated.

## Required invariants

- All input reads are bounded and return structured errors; parsers must not panic on
  malformed data.
- Counts, allocation sizes, offsets, recursion, and string lengths have explicit limits.
- Authoritative iteration is stable. Do not use randomized map iteration in simulation,
  manifests, hashes, commands, or parser output.
- Simulation uses fixed ticks, stable IDs, explicit integer/fixed-point math, seeded RNG
  streams, and versioned state hashes.
- The same inputs, mount order, profile, and seed must produce the same output.
- Dynamic work may execute in parallel only when results are committed in stable order.

## Change protocol

- Read `CURRENT.md` and the active milestone charter in `docs/milestones/` before starting
  work; update them when the verified next step moves.
- Every fact has exactly one documentation home. Milestone charters, progress, and
  completion evidence live in `docs/milestones/<milestone>.md`. `ROADMAP.md` is a status
  index and carries no progress prose. `CURRENT.md` holds only the active objective,
  current status, and the next verified step. User-visible completed work goes to
  `CHANGELOG.md` under the active milestone heading; permanent design choices go to
  `docs/adr/`. Link between these files instead of duplicating content.
- Every milestone charter defines scope, exclusions, inputs, outputs, owning crate,
  acceptance tests, determinism constraints, documentation, and a completion artifact.
- On milestone completion: set the status and date in the milestone file and the
  `ROADMAP.md` index, retitle the changelog's active section to that milestone, open a new
  active section, and point `CURRENT.md` at the next milestone.
- Add or update synthetic fixtures and negative tests with every parser.
- Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  and `cargo test --workspace` before declaring work complete.

## Forbidden shortcuts

- No unchecked indexing or integer arithmetic over untrusted offsets.
- No `unsafe` code without an accepted ADR changing the workspace policy.
- No retail assets in tests, screenshots, examples, or benchmarks.
- No wall-clock time, host filesystem ordering, locale, or platform hash seeds in
  deterministic output.
- No gameplay cheats hidden behind AI difficulty.

