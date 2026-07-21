# Current Objective

## Objective

Complete resource-probe gate R1 by specifying and implementing bounded CSF localization
decoding through the verified BIG-backed VFS.

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
- Local formatting, strict Clippy, and all 16 runtime tests pass.
- All 18 installed Steam Generals BIG archives have matching declared sizes and bounded
  verified directory trailers; `INI.big` resolves 92 deterministic manifest entries.

## Known blockers

- CSF variants, encoding rules, duplicate-label behavior, and resource limits require an
  evidence-backed format specification.
- `BIG4` remains implemented from corroborating source but unverified against retail data.

## Next verified step

Inspect the approved GPL CSF loader at an exact revision, write `docs/formats/csf.md`,
construct synthetic valid and malformed fixtures, then add deterministic localization
reporting without exposing retail strings.
