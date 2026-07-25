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
and renderer-neutral frames. Twenty-one tests over one original synthetic layout pass, and
`cic-inspect ui-layout` verifies it against real data: all 80 Zero Hour and 78 Generals layouts
instantiate at 800x600, 1920x1080, and 21:9 2560x1080 under both policies with no failures and zero
diagnostics.

One measurement from that pass changes a later gate: the whole Zero Hour corpus declares only nine
`TABSTOP` controls, so keyboard traversal of a retail menu cannot come from the layouts and the shell
gate will need project-owned tab order.

Gate 6's custom `wgpu` presentation is implemented and verified against real data. The text stack is
settled: `cosmic-text` 0.19 and `glyphon` 0.12, which declares `wgpu ^30.0.0` and unifies with the
workspace `wgpu` 30 rather than pulling a second copy; both licences are permissive and compatible
with GPL-3.0-only. `cic-inspect ui-render` writes a deterministic PNG plus hash from explicit inputs
only, and renders the retail main menu, options menu, and skirmish options with correct geometry,
batching, clipping, colour, and localized text, byte-identical across runs.

Push-button draw-data composition is implemented from `GadgetPushButton.h` and
`W3DGadgetPushButtonImageDraw`, along with the centred button text `drawButtonText` produces. The
retail main menu now renders as a real menu: background art, logo, gold-framed buttons, centred
localized labels.

The next verified step is **the remaining families' draw-data composition** — sliders, list boxes,
combo boxes, check boxes, text entry, progress bars, and tab controls — each needing its `Gadget*.h`
index map and `W3DGadget*` geometry read at the pinned revision. Until then those controls stage a
visible placeholder plus an `UncomposedFamily` diagnostic rather than a misleading fill. One finding
shapes that work: the draw procedure is selected by the control's retained draw-callback name (an
`...ImageDraw` variant against a plain `...Draw`), not by the `IMAGE` status bit, so that name is the
correct discriminator.

Two smaller pieces remain queued: the rest of Gate 4 — bounded `WindowTransition`, `MouseCursor`, and
`ShellMenuScheme` subsets, which live in the same INI family and reuse the shared lexer — and Gate 7's
shell stack, which is what hides the subpanels a retail menu overlays today: rendering `MainMenu.wnd`
currently shows every subpanel at once, because retail hides them from menu code rather than through
`STATUS`.

Separately, [docs/formats/csf.md](docs/formats/csf.md) records the language-selection mechanism
against the pinned source, for the planned goal of shipping languages the original game never had.
The `cic-tools` side of that is now done: `--language` selects `<Language>.big`, `<Language>ZH.big`,
and `Data/<Language>/`, so a `Russian.big` supplying `Generals.csf`, `HeaderTemplate.ini`, and
`Language.ini` under `Data/Russian/` fits the established mechanism without further tool changes.
