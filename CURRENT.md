# Current Objective

## Objective

Finish resource-probe gate R1 robustness work, then begin the W3D chunk-inventory gate.

## Implemented foundation

- R0 completion suite passed in GitHub CI run `29840005186`.
- Rust workspace and CI policy.
- Bounded, cursor-based binary reads with structured errors.
- Normalized, ASCII case-insensitive virtual paths.
- Deterministic last-mounted-wins overlays with full provider history.
- Loose-directory manifest CLI and synthetic tests.
- Evidence-backed `BIGF`/`BIG4` indexing with explicit limits and synthetic fixture.
- BIG duplicate-name history with deterministic last-entry-wins resolution.
- Mixed directory/BIG manifests through `cic-inspect`.
- Evidence-backed CSF version 3 decoding with raw names, complemented UTF-16, optional
  wave names, zero-string labels, and all variants preserved.
- Deterministic `cic-inspect csf` reports through loose-directory or BIG mounts.
- Original CSF fixture and synthetic BIG-to-CSF CLI completion artifact.
- Local formatting, strict Clippy, and all 25 runtime tests pass.
- All 18 installed Steam Generals BIG archives have matching declared sizes and bounded
  verified directory trailers; `INI.big` resolves 92 deterministic manifest entries.
- The installed Steam Generals CSF parses exactly to its 282,246-byte member boundary and
  reports version 3, 2,806 labels, and 2,805 strings.

## Known blockers

- `BIG4` remains implemented from corroborating source but unverified against retail data.
- The checked-in CSF fuzz target compile-checks with `libfuzzer-sys`; a bounded runtime
  smoke test remains pending because the `cargo-fuzz` runner is not installed.

## Next verified step

Install/run the CSF fuzz-target smoke gate, then specify a bounded W3D chunk inventory that
preserves unknown chunks before decoding geometry.
