# Current Objective

## Objective

Complete resource-probe foundation gate R0 by executing its test suite locally or in CI,
then begin evidence-backed BIG archive specification work.

## Implemented foundation

- Rust workspace and CI policy.
- Bounded, cursor-based binary reads with structured errors.
- Normalized, ASCII case-insensitive virtual paths.
- Deterministic last-mounted-wins overlays with full provider history.
- Loose-directory manifest CLI and synthetic tests.
- Formatting and strict Clippy checks pass for all targets.

## Known blockers

- This Windows host has Rust but lacks the MSVC linker/runtime libraries. All test
  targets compile through Clippy, but `cargo test --workspace` cannot link until Visual
  Studio Build Tools with the C++ workload is installed or CI runs the suite.
- BIG format facts and variants need an evidence-backed format specification before
  decoder implementation.

## Next verified step

Run `cargo test --workspace` in GitHub Actions or after installing the Windows C++ build
tools. Once green, write `docs/formats/big.md` from an approved GPL source revision and
create synthetic valid and malformed archives.
