# Current Objective

## Objective

R4 is active. Its first vertical slice — a bounded, unknown-preserving WND inventory and
immutable layout/control decoder, plus a surface-free `wgpu` capture of one original synthetic
menu — is complete (see [docs/milestones/r4-wnd-shell.md](docs/milestones/r4-wnd-shell.md) Gate 1).
Gates 2 through 4's definition side follow it: typed per-gadget fields, bounded patch overlays, and
resolution of the mapped images, fonts, header templates, and CSF labels a layout names. The
remaining slices add the retained `cic-ui` runtime, the main-menu stack, modern display settings, and
the skirmish/map-selection harness. R4 remains presentation-only: callbacks are allowlisted typed
events, MAP scripts stay inert until R5, and project-owned post-parse patches augment rather than
modify user-owned WND bytes.

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

Gates 1 through 3 are complete: the WND grammar and every established field decode into immutable
typed values against all 80 retail layouts in both editions with no malformed-field diagnostics, and
patch overlays apply value-level and structural edits with per-field provenance over an unmodified
source document (see [docs/formats/wnd.md](docs/formats/wnd.md)).

Gate 4's definition resources are implemented and verified against a real installation. Bounded
decoders cover `MappedImage`, `HeaderTemplate.ini`, and `Language.ini` over one shared lexer derived
from the original INI reader; `cic-tools` composes them with the existing CSF decoder into an
immutable resolution result, reported by `cic-inspect ui-resources`. Header templates resolve
completely in both editions and mapped images resolve 1,849/1,978 in Zero Hour and 1,789/1,932 in
Generals; what remains unresolved is retail's own gap, now visible. Localization mounts are
language-parameterized rather than hardcoded to English.

Three facts from that pass shape the runtime work:

- Retail names roughly 50 distinct mapped images no shipped INI defines, and three font families it
  never ships as files, so visible placeholders and stable diagnostics are the ordinary path rather
  than an edge case.
- `[None]`, spelled in retail as both `[None]` and `[NONE]`, is the writer's explicit "selects
  nothing" placeholder for `HEADERTEMPLATE`, `FONT`, and text records, not a missing resource.
- `Language.ini` fixes `ResolutionFontAdjustment = 0.7` and a font-scaling policy, which is the
  presentation-policy input the scaling gate needs.

Gate 5's retained runtime is implemented as the new `cic-ui` crate: layout reproducing
`parseScreenRect` exactly plus a project-designed uniform-scale `Modern` policy, the original's
three-pass layered hit testing with source-order child descent, focus with its `NOFOCUS` refusal and
parent walk, a wraparound tab cycle over declared `TABSTOP` controls, every control-family invariant,
and renderer-neutral frames. Twenty-one tests over one original synthetic layout pass.

The next verified step is Gate 6, custom `wgpu` presentation: executing a `UiFrame` as ordered
image and colour quads with borders, scissor rectangles, and shaped Unicode text over either a 2D
background or an R3 scene, with bounded atlases, batched stable draws, and surface-free deterministic
capture. That gate needs the text stack decided first — ADR 0010 prefers `cosmic-text` with `glyphon`
subject to verifying compatibility with the workspace `wgpu`, and licences and notices must be
reviewed before either enters the manifests.

Two smaller pieces remain queued behind it: the rest of Gate 4 — bounded `WindowTransition`,
`MouseCursor`, and `ShellMenuScheme` subsets, which live in the same INI family and reuse the shared
lexer — and the runtime-side visible placeholders for the resources retail names but never defines.

Separately, [docs/formats/csf.md](docs/formats/csf.md) records the language-selection mechanism
against the pinned source, for the planned goal of shipping languages the original game never had.
The `cic-tools` side of that is now done: `--language` selects `<Language>.big`, `<Language>ZH.big`,
and `Data/<Language>/`, so a `Russian.big` supplying `Generals.csf`, `HeaderTemplate.ini`, and
`Language.ini` under `Data/Russian/` fits the established mechanism without further tool changes.
