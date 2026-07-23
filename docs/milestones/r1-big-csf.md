# R1: BIG and CSF resource probe

**Status:** In progress.

**Scope:** Evidence-backed BIG archive mounting and complete CSF decoding with resource
provenance.

**Exclusions:** Compression not present in verified variants, localization UI, retail
fixture distribution, W3D/MAP parsing.

**Inputs:** Synthetic BIG and CSF files plus user-owned archives for local verification.

**Outputs:** Resolved VFS manifests and deterministic localization reports.

**Owner:** `cic-vfs` for BIG and new `cic-formats` for CSF.

**Acceptance tests:** Valid variants, truncation at every field, invalid counts/offsets,
duplicates, overlay conflicts, string bounds, and fuzz targets.

**Determinism:** Stable archive entry ordering, last-mounted-wins policy, stable label
ordering and diagnostics.

**Documentation:** `docs/formats/big.md`, `docs/formats/csf.md`, compatibility matrix.

**Completion artifact:** Synthetic archive containing a CSF file and a checked-in stable
manifest snapshot.

**Progress:** BIGF indexing and mounting pass the complete local suite and all 18
installed Steam Generals archives. Mixed-endian fields, slash-normalized paths, and
none/`L225`/`L231` directory trailers are verified. The bounded CSF decoder, lossless
record IR, original fixture, deterministic report, and synthetic BIG-to-CSF CLI artifact
are implemented and verified against the installed Generals CSF. A 30-second AddressSanitizer
libFuzzer smoke run completed 4,077,155 CSF inputs without a finding. BIG4 retail
verification remains open.

## Completion evidence

- Evidence-backed `BIGF`/`BIG4` indexing with explicit limits and synthetic fixture.
- BIG duplicate-name history with deterministic last-entry-wins resolution.
- Mixed directory/BIG manifests through `cic-inspect`.
- Evidence-backed CSF version 3 decoding with raw names, complemented UTF-16, optional
  wave names, zero-string labels, and all variants preserved.
- Deterministic `cic-inspect csf` reports through loose-directory or BIG mounts.
- Original CSF fixture and synthetic BIG-to-CSF CLI completion artifact.
- All 18 installed Steam Generals BIG archives have matching declared sizes and bounded
  verified directory trailers; `INI.big` resolves 92 deterministic manifest entries.
- The installed Steam Generals CSF parses exactly to its 282,246-byte member boundary and
  reports version 3, 2,806 labels, and 2,805 strings.
- The CSF AddressSanitizer/libFuzzer smoke gate completed 4,077,155 inputs in 31 seconds
  without a crash or sanitizer finding.

## Open items

- `BIG4` remains implemented from corroborating source but unverified against retail data.
