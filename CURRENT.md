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
and renderer-neutral frames. Tests over one original synthetic layout pass, and
`cic-inspect ui-layout` verifies it against real data: all 80 Zero Hour and 78 Generals layouts
instantiate at 800x600, 1920x1080, and 21:9 2560x1080 under both policies with no failures and zero
diagnostics.

One measurement from that pass changes a later gate: the whole Zero Hour corpus declares only nine
`TABSTOP` controls, so keyboard traversal of a retail menu cannot come from the layouts and the shell
gate will need project-owned tab order.

Gate 6's custom `wgpu` presentation is complete. The text stack is
settled: `cosmic-text` 0.19 and `glyphon` 0.12, which declares `wgpu ^30.0.0` and unifies with the
workspace `wgpu` 30 rather than pulling a second copy; both licences are permissive and compatible
with GPL-3.0-only. `cic-inspect ui-render` writes a deterministic PNG plus hash from explicit inputs
only, and renders the retail main menu, options menu, and skirmish options with correct geometry,
batching, clipping, colour, and localized text, byte-identical across runs.

Per-family draw-data composition finished that gate. Push buttons came first, from
`GadgetPushButton.h` and `W3DGadgetPushButtonImageDraw`, along with the centred button text
`drawButtonText` produces; the rest followed — radio buttons, check boxes, text entry, both slider
orientations,
progress bars, tab controls, and the stretched single-image path list boxes, combo boxes, and static
text share. Every established family now composes from its own `Gadget*.h` index map and
`W3DGadget*ImageDraw` geometry, so no control stages a stand-in for an unimplemented family. Five
source behaviours that reading produced are recorded in
[docs/milestones/r4-wnd-shell.md](docs/milestones/r4-wnd-shell.md); the one that changed existing
output is that a check box centres its label only vertically and indents it by the control's own
height, which this project had been centring.

The image path is now selected the way the source selects it — the `IMAGE` bit picks a default
procedure at creation and a resolvable `DRAWCALLBACK` replaces it — and a family that finds nothing
at its own indices draws nothing and reports it, matching the source's early return, instead of
painting a placeholder over a control retail never shows.

Those families are now verified against a real installation, not only synthetically: every layout in
both editions renders at 1280x720 and 1920x1080, 79 of 80 in Zero Hour and 77 of 78 in Generals,
with the one exception being a layout whose single root declares `HIDDEN`. That pass found and fixed
two presentation bugs — the whole-control border, which was gated on a status bit the source never
reads and drawn on the image path where the source draws none, and the missing
`W3DGadgetPushButtonImageDrawOne` path, without which retail's eight skirmish start-position markers
were invisible. Every remaining diagnostic traces to retail's own data: an image no shipped INI
defines, or a control the source itself draws nothing for. Full numbers are in
[docs/milestones/r4-wnd-shell.md](docs/milestones/r4-wnd-shell.md).

**Gadget child creation is the next verified step**, and it is what that verification turned up. A
combo box's draw procedure paints only a background and a title; its edit box, drop-down button, and
list box are separate child windows `GadgetComboBoxCreate` builds at creation, and a list box's
scroll bar is the same. `cic-ui` builds none of them, so `OptionsMenu.wnd`'s Resolution and Detail
combos render as bare black rectangles and roughly 100 controls per edition report uncomposed art.
The layouts already carry the children's art in the `COMBOBOXEDITBOX*`, `COMBOBOXDROPDOWNBUTTON*`,
`COMBOBOXLISTBOX*`, and `LISTBOX*UPBUTTON`/`DOWNBUTTON`/`SLIDER` records, so this is retained-runtime
work in `cic-ui` rather than anything the presentation layer can fix.

Two smaller pieces remain queued behind it: the rest of Gate 4 — bounded `WindowTransition`,
`MouseCursor`, and `ShellMenuScheme` subsets, which live in the same INI family and reuse the shared
lexer — and Gate 7's shell stack, which is what hides the subpanels a retail menu overlays today:
rendering `MainMenu.wnd` currently shows every subpanel at once, because retail hides them from menu
code rather than through `STATUS`.

Separately, [docs/formats/csf.md](docs/formats/csf.md) records the language-selection mechanism
against the pinned source, for the planned goal of shipping languages the original game never had.
The `cic-tools` side of that is now done: `--language` selects `<Language>.big`, `<Language>ZH.big`,
and `Data/<Language>/`, so a `Russian.big` supplying `Generals.csf`, `HeaderTemplate.ini`, and
`Language.ini` under `Data/Russian/` fits the established mechanism without further tool changes.
