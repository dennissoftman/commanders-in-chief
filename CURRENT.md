# Current Objective

## Objective

R4 is active and its main-menu slice now runs end to end: the user-owned main menu loads, renders, and
navigates to Options and Skirmish Options and back through the shell stack
(see [docs/milestones/r4-wnd-shell.md](docs/milestones/r4-wnd-shell.md) Gate 8). Gates 1 through 8 are
complete — bounded WND inventory and typed control decoding, patch overlays, UI resource resolution,
the retained `cic-ui` runtime, custom `wgpu` presentation, safe callbacks with the shell stack and
transitions, and the working main-menu artifact. The remaining slices add modern display settings and
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

Gadget child creation followed, completing Gate 5's retained runtime. `cic-ui` now builds the
windows the original's creation code builds and no layout declares: a thumb for every slider, an up
button, down button, and slider for a list box that asks for a scroll bar, and a drop-down button,
edit field, and hidden drop-down list for a combo box. That is 958 extra controls across the Zero
Hour corpus, and `OptionsMenu.wnd`'s combos now render as retail draws them. The subtle part is that
a part's art is not on its parent — a scroll bar's thumb reads `SLIDERTHUMB*` from the list box two
levels above it — because the original bridges that distance through file statics.

Gate 7's safe callbacks and shell stack are implemented. The allowlist did not need inventing: the
original already has one, in `FunctionLexicon`'s nine fixed name tables, where a name absent from the
searched table yields a null pointer nothing ever calls. `cic-ui` carries those tables as data and
classifies every retained name as established, the explicit `[None]` placeholder, or unknown — the
last two inert here exactly as they were inert there — while a separate project-owned
`UiActionAllowlist` decides which controls may run a typed demo action at all. `UiShell` reproduces
`Shell`'s sixteen-screen pseudo-stack, including the two-phase push and pop whose purpose is
animation, with the shutdown protocol exposed rather than hidden so a capture steps it without a
clock. Bounded `WindowTransitions.ini` decoding landed alongside, which is the transition part of
Gate 4's remainder.

That pass is verified against both installations: 6,908 retained callback names across Zero Hour's
80 layouts and 6,350 across Generals' 78 classify with six and five unknowns, every one of them a
layout-level name the shipped client's own lexicon never registers. The transition INI decodes with
zero diagnostics in both — 56 groups over 381 windows in Zero Hour, 55 over 379 in Generals — and
`cic-inspect ui-shell` walks Main Menu -> Options -> back -> Skirmish Options -> back over the real
layouts with every layout callback resolving, byte-identically across runs.

Reading the window manager to build that also found a Gate 5 bug and fixed it: hit testing walked
top-level windows and children in file order, but `winCreate` links every new window at the *head* of
its list, so the original tests the last window in the file first — which is also the front-most one,
since `winRepaint` draws from the tail backwards. Only overlapping siblings could tell the difference,
which is why the Gate 6 sweeps never surfaced it.

The transition runtime completed Gate 7. `UiTransitionHandler` reproduces the handler's scheduling —
current, pending, and the two draw groups, set/reverse/remove, fire-once, and the accumulator that
steps every whole frame it crosses so a discrete state machine cannot skip one — over each of the
fifteen styles' own per-frame machine of hidden-state changes and draw states. Time is the caller's, so
a capture advances exactly one frame with no clock involved. `cic-inspect ui-transitions --run` arms
every group, loads the layouts its window names point at, and steps it to completion: every window of
every retail group resolves, 0 unresolved of 379 named in Zero Hour and 377 in Generals, and the sweep
is byte-identical across runs.

Building it found two source conditions, both now reported where they happen. A group naming a window
no loaded layout carries never finishes, because the arm that would set the flag tests the window
first. And `TYPETEXT` cannot finish when its label is under thirty characters: it finishes only on the
state numbered by its declared length, while arming shortens that length to the character count and
the per-window frame filter refuses anything past it. That is the four unfinished groups in each
edition. `COUNTUP` shortens identically and does finish, thanks to one extra assignment `TYPETEXT`
lacks.

Gate 8's main-menu artifact is complete. `cic-inspect ui-menu` loads the user-owned
`Menus/MainMenu.wnd` with its images, fonts, and labels and drives hover, focus, click, the subpanels,
Back, Options, Skirmish, and a safe Exit through the shell stack, the transition handler, and the
action allowlist together, capturing a PNG at each named point. The complete loop runs on both
installations at 1280x720 with every routed action applied, nothing unrouted, no unresolved image, and
byte-identical repeat runs; returning to the menu by any of the three routes reproduces the default
menu's capture hash exactly. What each control does is a project-owned table in `cic-tools`'s
`shell_menu`, derived from `MainMenu.cpp`, which `cic-ui` never consults.

Two facts from that pass are worth carrying forward. The retail main menu draws nothing but the logo
until the player's first input, because `MainMenuInit` hides every panel and `MainMenuInput` is what
reveals the default one — so the earlier observation that rendering `MainMenu.wnd` shows every subpanel
at once was the layout without any of its menu behaviour, and both the hiding and the revealing are now
reproduced. And hovering used to hilite any control, where the original hilites only a control
declaring `MOUSETRACK`; that had been repainting the whole main-menu background whenever the pointer
rested on it, and is fixed.

**Gate 9's modern Options and display settings is the next verified step**: loading
`Menus/OptionsMenu.wnd`, reusing its established `ComboBoxResolution`, and applying a bounded
project-owned patch that adds monitor, window-mode, refresh-rate, and UI-scale controls without
changing user-owned bytes, then applying a mode through a confirmation/rollback transaction against an
injected catalog. The patch mechanism itself is already demonstrated end to end against this layout
(Gate 3); profile-driven patch selection is the remaining integration step.

Four smaller pieces remain queued. Transition draws are renderer-neutral records that
`cic-render` does not execute yet, so transitions run and report but do not reach a surface. Gate 8's
optional shell-MAP background — composing the R3 scene beneath the UI — is not wired up. The rest
of Gate 4 is bounded `MouseCursor` and `ShellMenuScheme` subsets, which live in the same INI family and
reuse the shared lexer. And the ornamental border is unimplemented: `WIN_STATUS_BORDER` makes the
window manager tile a frame from hardcoded `BorderTop`/`BorderCorner__`-style mapped images, which is a
different border from the colour-path outline and is drawn by no draw procedure.

Separately, [docs/formats/csf.md](docs/formats/csf.md) records the language-selection mechanism
against the pinned source, for the planned goal of shipping languages the original game never had.
The `cic-tools` side of that is now done: `--language` selects `<Language>.big`, `<Language>ZH.big`,
and `Data/<Language>/`, so a `Russian.big` supplying `Generals.csf`, `HeaderTemplate.ini`, and
`Language.ini` under `Data/Russian/` fits the established mechanism without further tool changes.
