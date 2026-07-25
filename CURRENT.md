# Current Objective

## Objective

R4 is active. Its first vertical slice — a bounded, unknown-preserving WND inventory and
immutable layout/control decoder, plus a surface-free `wgpu` capture of one original synthetic
menu — is complete (see [docs/milestones/r4-wnd-shell.md](docs/milestones/r4-wnd-shell.md) Gate 1).
The next slice adds user-owned mapped images/fonts/CSF labels, the retained `cic-ui` runtime,
the main-menu stack, modern display settings, and the skirmish/map-selection harness. R4 remains
presentation-only: callbacks are allowlisted typed events, MAP scripts stay inert until R5, and
project-owned post-parse patches augment rather than modify user-owned WND bytes.

R3 is complete; its charter, progress, and completion evidence are recorded in
[docs/milestones/r3-map-scene.md](docs/milestones/r3-map-scene.md). R4 adds
bounded WND/UI ingestion and a navigable `wgpu` main-menu/skirmish demo so map compatibility can be
inspected through the intended shell before simulation exists. Its Options path will use bounded
post-parse WND patches—not hardcoded window-name rendering—to add modern window mode, resolution,
refresh-rate, and UI-scale controls with transactional confirmation/rollback.

## Status

- Local formatting, strict Clippy, and the complete workspace test suite pass.
- R1 remains in progress: `BIG4` retail verification is open (see
  [docs/milestones/r1-big-csf.md](docs/milestones/r1-big-csf.md)).

## Next verified step

Gates 1 and 2 are complete: the WND grammar and every established field are decoded into immutable
typed values, verified against all 80 retail layouts in both editions with no malformed-field
diagnostics (see [docs/formats/wnd.md](docs/formats/wnd.md)). Gate 3's patch overlays are implemented, value-level
and structural, with per-field provenance, an unmodified source document, and a
`cic-inspect wnd-patch` report; the composition Gate 9 needs is verified against the retail
`OptionsMenu.wnd`.

Gate 4 (UI resource resolution) has its evidence pass done and recorded in
[docs/formats/wnd.md](docs/formats/wnd.md): the demand side is measured (217 mapped images, 7 font
families, 15 header templates, 366 label references), every definition source is located, and label
coverage is checked against a real installation. The next verified step is the first decoder —
bounded `MappedImage` INI decoding over a recursive `Data/INI/MappedImages/**` load — followed by
`HeaderTemplate.ini` and `Language.ini`, then binding CSF labels through the existing decoder. Three
facts from the evidence pass shape that work:

- The header-template and font definitions live in the localization archive under
  `Data/<Language>/`, not `INI.big`, so resolution needs a localization mount alongside the `Wnd`
  profile.
- Retail ships no font files, so a project-supplied font is the default path for deterministic
  captures rather than an opt-in fallback.
- The two mapped-image directories merge rather than select: their name sets are measured disjoint
  across both editions, so no variant-selection policy is needed.

Separately, [docs/formats/csf.md](docs/formats/csf.md) now records the language-selection mechanism
against the pinned source, for the planned goal of shipping languages the original game never had.
The language is a path component (`Data/<Language>/`), and the original client mounts every `*.big`,
so a `Russian.big` supplying `Generals.csf`, `HeaderTemplate.ini`, and `Language.ini` under
`Data/Russian/` fits the established mechanism. Acting on that requires parameterizing this
project's localization archive candidates and path prefix, which currently hardcode `English.big`
and `EnglishZH.big` per edition; that is a `cic-tools` resource-profile change, best sequenced with
Gate 4 since it resolves the same files.

The retained `cic-ui` runtime and main-menu navigation follow.
