# R4: WND user interface and navigable shell

**Status:** Active. R3 produced the complete non-simulating MAP scene and scenario description;
the first vertical slice is bounded WND inventory/layout decoding plus a synthetic headless menu.

**Progress:** Gate 1 (WND inventory and bounded syntax) is implemented. `crates/cic-formats/src/wnd.rs`
bounds `FILE_VERSION`, the `STARTLAYOUTBLOCK`/`ENDLAYOUTBLOCK` layout block (with the source-confirmed
`FILE_VERSION >= 2` gating and `"[None]"` version-1 default), and the complete `WINDOW`/`CHILD`/`END`/
`ENDALLCHILDREN` hierarchy with `WINDOWTYPE`/`SCREENRECT` typed; every other field is retained
generically rather than dropped, and unrecognized top-level keywords or out-of-vocabulary
`WINDOWTYPE` values are surfaced as non-fatal diagnostics. Original synthetic positive,
exhaustive-truncation, per-limit, and unknown-field-preservation tests pass. `cic-inspect wnd`
produces a stable source-order report, and `cic-inspect wnd-render` stages every window rectangle
as a flat colored quad and writes a surface-free deterministic capture through the existing
`HeadlessRenderer` boundary, proving the immutable decoded value can drive a renderer capture. Gate
2's typed per-gadget fields (fonts, state colors/borders, draw-data arrays, header templates,
gadget-specific `DATA`), resource resolution (mapped images/fonts/CSF), the retained `cic-ui`
runtime, and main-menu navigation remain unimplemented and are the next slice.

Gate 1 is now verified against real data: every one of the 80 retail `.wnd` layouts reachable
through the `Wnd` resource profile in both editions decodes under default limits, producing one
non-fatal diagnostic across the whole corpus. That verification corrected two grammar facts —
`CHILD` is an inert marker rather than a required prefix, and the status vocabulary is
edition-dependent — and confirmed that no configured limit needs raising (deepest nesting 6,
largest layout 113 windows, widest child list 80, longest record 833 bytes). A field census over
the same corpus scopes Gate 2: 43 distinct window field names, 13 of them present on every window,
with every gadget-specific record cleanly confined to its own window types.

Gate 2 (immutable typed control definitions) is implemented. `parseWindow`'s 46-keyword field chain
is enumerated from source and reconciled exactly with the census, and all 46 are accounted for:
45 are typed — the common window records, the 21 draw-data arrays, the seven gadget `DATA` records,
and `IMAGEOFFSET` — while `TOOLTIP` is deliberately untyped because its source parser ignores the
record and stores a placeholder marked `@todo`. Every field name occurring anywhere in either
retail edition decodes, corpus-wide, with no malformed-field diagnostics. Reports expose the typed
values so a modded layout can be compared record by record without rendering it, satisfying this
gate's stable-report requirement. Two records (`TABCONTROLDATA`, `IMAGEOFFSET`) rest on source
evidence alone, having no retail occurrence to cross-check, and are marked as such.

Gate 3 (bounded WND patch overlays) is implemented. The versioned patch document, its bounded
decoder, and the apply engine cover preconditions, field replacement and addition, rectangle
replacement, and the structural `reorder`, `reparent`, and `insert-window` operations — with
per-field provenance, an unmodified source document, and structured errors for every failure mode
including reparent cycles, duplicate inserted names, and out-of-range indices. An inserted subtree
is parsed by the ordinary bounded WND decoder, so it obeys the same grammar, limits, and typed-field
rules as authored source. `cic-inspect wnd-patch` reports operations, provenance, and the patched
hierarchy.

Gate 4 (UI resource resolution) is implemented for definition resources. Three narrow decoders over
one shared bounded INI lexer cover `MappedImage` blocks, `HeaderTemplate.ini`, and `Language.ini`,
each derived field by field from the pinned source parse tables, including the source's own quirks:
the `Coords`/`Status` order dependence that swaps a rotated region's presentation size, the
two-token quoted-string rejoin that drops a one-character continuation, the reversed
`LocalFontFile` list order, and every constructor default. `cic-tools`' resolution layer composes the
VFS, those decoders, and the existing CSF decoder into an immutable result where each demanded name
either binds to a definition with its defining file recorded or is explicitly unresolved.

Locating `ImageCollection::load` corrected a previously recorded conclusion: the mapped-image load is
an ordered three-stage load with an explicit texture-size selection, not a recursive merge, and the
directories' name sets are *not* disjoint in retail data — 23 overrides in Generals and 43 in Zero
Hour. Implementing the source order is therefore required for correctness, not a fidelity nicety.

Verified against a real installation across every layout in both editions: mapped images resolve
1,849/1,978 in Zero Hour and 1,789/1,932 in Generals, header templates resolve completely (209 and
196, zero unresolved), and Zero Hour's 17 unresolved labels reproduce the evidence pass's independent
count exactly. What does not resolve is retail's own gap — 50 and 48 distinct image names no shipped
INI defines, and the three font families (`Generals`, `Abadi MT Bold`, `Placard MT Condensed`) retail
names but never ships — which is why placeholders and diagnostics are the ordinary path. The
localization mount is now language-parameterized (`--language`, `<Language>.big`/`<Language>ZH.big`,
`Data/<Language>/`) rather than hardcoded to English, which is what shipping a language the original
never had requires. `cic-inspect ui-resources` reports all of it. Transitions, cursors, menu schemes,
and the runtime-side visible placeholders remain the rest of Gate 4.

Gate 5 (retained UI runtime) is implemented as the new `cic-ui` crate, which depends only on
`cic-formats`: it consumes immutable definitions and produces renderer-neutral frames, so it links to
no rendering API and holds no simulation state. Layout reproduces `parseScreenRect` exactly —
per-axis ratios, a truncating cast, size derived from the scaled corners, and child positions made
relative to the parent's already-scaled origin — alongside a project-designed uniform-scale
`Modern` policy. Hit testing reproduces the three-pass `ABOVE`/unlayered/`BELOW` search, the
source-order child descent that skips a hidden or disabled child and falls through to the parent, the
inclusive edge test, the `NO_INPUT` discard, and mouse-captor confinement. Focus reproduces the
`NOFOCUS` refusal and the parent-walking acceptance. Control invariants cover radio-group
exclusivity, check toggling, slider clamping with an inverted-bounds diagnostic, list and combo
selection that refuses an out-of-range index rather than clamping it, list scroll clamping,
character-wise Unicode text entry against the declared `MAXLEN`, and progress clamping. Hiding or
disabling a control clears hover, press, focus, and capture through its whole subtree. Twenty-one
tests over one original synthetic layout covering every control family pass, including a determinism
check that two instantiations of the same inputs produce identical controls and frames.

Gate 5 is verified against real data through `cic-inspect ui-layout`, which instantiates a layout for
an explicit viewport and scale policy and reports the tree, tab order, frame submission order, and
diagnostics. Every one of the 80 Zero Hour layouts and 78 Generals layouts instantiates at 800x600,
1920x1080, and 21:9 2560x1080 under both policies — 480 instantiations — with no failures and, after
mapping the complete `WindowStatusNames` vocabulary, **zero diagnostics in either edition**. The 80
Zero Hour layouts yield 1,667 retained controls, matching the WND census's window count exactly, and
their family distribution is 539 static text, 424 push buttons, 411 windows with no gadget state, 115
combo boxes, 45 check boxes, 39 list boxes, 34 progress bars, 32 entry fields, 19 radio buttons, and
nine sliders.

That pass also measured something the runtime gates need to know: the whole Zero Hour corpus declares
only **nine** `TABSTOP` controls. Keyboard traversal of a retail menu therefore cannot come from the
layouts, which is consistent with the original populating the manager's tab list from menu code rather
than from the WND. A usable demo will need project-owned tab order, and that belongs to the shell gate
rather than to the parser.

Reading the runtime source produced one finding worth recording: `GameWindow::winNextTab` and
`winPrevTab` are entirely commented out at the pinned revision and return success without moving
focus, so per-window tab traversal is not source behavior. The live mechanism is the window manager's
tab list, whose wraparound cycle `cic-ui` reproduces over the declared `TABSTOP` bits.

Gate 6 (custom `wgpu` presentation) is implemented. `cic-render` stages a retained frame into batched
geometry, breaking a batch only when the bound texture page or scissor rectangle changes, and executes
it through the existing surface-free capture boundary. Nested clips intersect and are clamped into the
attachment; alpha is straight to match the source's stored channel bytes; pages upload in the capture
target's colour space, because declaring a page sRGB against a linear target linearizes on read
without re-encoding on write and darkens every image. A border draws only for a control declaring
`BORDER` — honouring a border colour alone outlines the entire menu, since most retail controls carry
one.

The text stack is settled and pinned: `cosmic-text` 0.19 for shaping and `glyphon` 0.12 for `wgpu`
glyph rendering, the pair ADR 0010 selected. `glyphon` 0.12 declares `wgpu ^30.0.0` and unifies with
the workspace `wgpu` 30 rather than pulling a second copy, verified with `cargo tree -i wgpu`; both
licences are permissive and compatible with GPL-3.0-only. Fonts are always supplied as bytes, never
enumerated from the host, and a capture with no font supplied stages a visible placeholder bar plus a
diagnostic per run.

Verified against a real installation at 1280x720 with a user-owned font: `MainMenu.wnd` stages 37
quads in 12 batches over three texture pages with 29 shaped runs, `OptionsMenu.wnd` 41 quads and 25
runs, `SkirmishGameOptionsMenu.wnd` 52 quads and 21 runs, with byte-identical hashes across repeated
runs. Localized labels resolve through the CSF decoder before staging, so captures show real menu text
in the right places.

Push-button draw-data composition followed, taking the largest interactive family first: 424 of the
1,667 retail controls. `GadgetPushButton.h` fixes the entry indices and
`W3DGadgetPushButtonImageDrawThree` the geometry, both reproduced including the branch where the ends
alone do not fit. Button text is centred on both axes as `drawButtonText` does. With that in place the
retail main menu renders as a menu — background art, logo, gold-framed buttons, centred localized
labels — and staged quads rise from 37 to 682.

Rendering real data corrected two colour assumptions. An image draw is **untinted**: `winDrawImage`
takes no colour argument, and a slot's `COLOR` belongs to the colour-only fill path, so multiplying an
image by it painted every textured control in whatever that unused field held — frequently red in
retail data. And a control declaring `IMAGE` whose slot has no entry-0 image keeps its art at indices
only its own family reads, so filling with the slot colour there painted that same red; those controls
staged a visible placeholder plus a diagnostic naming the family instead, until the family itself
composed.

The remaining families' composition followed and completes Gate 6's per-family work: radio buttons,
check boxes, text entry, both slider orientations, progress bars, tab controls, and the stretched
single-image path list boxes, combo boxes, and static text share. Each family's index map comes from
its `Gadget*.h` accessors and its geometry from the matching `W3DGadget*ImageDraw`, both at the
pinned revision, and the full table is in [docs/provenance/wnd.md](../provenance/wnd.md). The frame
now carries all three draw-data slots and the live state a composition branches on, because a draw
procedure does not always read the slot the control's own state selected.

Reading those files corrected or established five behaviours worth recording:

- A selected radio button reads the hilite slot's second image triple even while enabled, because
  the source tests `WIN_STATE_SELECTED` before the enabled bit. It therefore never shows disabled
  art while selected.
- A horizontal slider ignores the control's state entirely when choosing art — fill and blank always
  come from the disabled slot, the highlight row from the hilite slot — and scales its tick squares
  against a fixed 800-pixel display reference rather than against the control.
- A check box draws no background at all: the source leaves that draw commented out and renders only
  the box, three pixels down and six shorter than the control. Its label is not centred the way a
  button's is either; `drawCheckBoxText` centres vertically but indents by the control's own height.
  That fixes a placement this project had previously centred.
- A text entry and a vertical slider each draw one small-centre piece more than fits, deliberately
  overrunning under the end piece that covers it.
- A progress bar fills the *unreached* part of its track with the bar's right piece rather than
  leaving it empty, with the whole bar inset ten pixels horizontally and five vertically.

The image path is now chosen the way the source chooses it: the `IMAGE` status bit picks a default
procedure at creation and a resolvable `DRAWCALLBACK` name replaces it, so a name reading as a draw
procedure decides and `"[None]"` leaves the bit deciding. With every established family composed,
the `UncomposedFamily` placeholder is retired. A control whose family finds nothing at its own
indices now draws nothing, which is the source's early return, and records an `UncomposedArt`
diagnostic naming the family; a visible placeholder there would have invented a control retail never
shows. Placeholders remain for a genuinely unresolved mapped image.

Two things about tab controls are reproduced as source behaviour and cannot be cross-checked, since
no retail layout declares one: `TABWIDTH` and `TABHEIGHT` are read raw and never scaled, so a tab
strip does not follow the creation-resolution scaling its own control does, and the strip's origin
comes from `GadgetTabControlComputeTabRegion`'s edge and orientation arithmetic.

Verification is synthetic: an original layout declaring every composed family
(`crates/cic-render/tests/fixtures/synthetic-gadgets.wnd`) drives per-family geometry assertions and
a surface-free capture that is byte-identical across runs, and the capture was rendered and looked
at rather than only asserted. Retail verification of these families is still open — this pass had no
installation available — so the earlier corpus-wide numbers still describe push buttons and
backgrounds only.

One compatibility fact belongs to Gates 7 and 8 rather than to presentation: rendering
`Menus/MainMenu.wnd` shows every subpanel at once, with labels overlapping. Retail hides those
subpanels from menu code, not through `STATUS`, so a correct main menu needs the shell stack's
show/hide semantics — the layout alone does not describe which subpanel is visible.

The Gate 3 patch work was verified end to end against the retail `OptionsMenu.wnd`: the stock
`ComboBoxResolution` is
reused and repositioned while a project-owned refresh-rate combo is inserted beside it. That is
precisely the Gate 9 composition ADR 0010 requires be expressible as auditable data rather than
hardcoded window names, demonstrated before any of the Options UI exists. Profile-driven patch
selection is the remaining integration step.

**Scope:** Boundedly decode the complete source-established WND grammar and the UI definition
resources required by it, then present those values through a retained, non-gameplay UI runtime.
Cover nested layouts, exact creation rectangles, resolution scaling, status/style flags, draw and
text states, named callbacks, tooltips, focus/tab order, shell layout stacking, transition groups,
bounded post-parse WND patches, mapped images, fonts, CSF localization, cursors, and the classic
gadget vocabulary: push/radio/check buttons, vertical/horizontal sliders, scroll list boxes, entry
fields, static text, progress bars,
user windows, mouse-tracking/animated windows, tab controls/panes, and combo boxes. The interactive
artifact must render a working main menu and navigate in demo mode through the skirmish setup and
map-selection screens.

**Exclusions:** Gameplay simulation, MAP-script execution, match launch, AI, networking, account or
online services, save/replay behavior, operating-system dialogs, and arbitrary execution of callback
names from untrusted WND data. R4 does not distribute retail WND files, images, fonts, sounds,
logos, or strings. Unsupported menu actions remain disabled or produce explicit demo diagnostics.

**Inputs:** Original synthetic WND layouts and UI definitions; original synthetic images/fonts/CSF
labels; user-owned installed or modded WND, mapped-image, font, texture, CSF, transition, and menu
resources through the VFS; project-owned or mod-supplied bounded WND patches; an explicit platform
display-mode catalog; and R3 map metadata, preview images, playable boundaries, and ordered spawn
candidates for the skirmish/map-selection demo.

**Outputs:** Stable WND/UI inventories and semantic reports; immutable UI definitions; a retained
menu/gadget state tree; deterministic render-neutral UI frames and headless capture hashes; and an
interactive `wgpu` shell demo. The demo renders the user-owned main-menu composition, supports
mouse/keyboard focus and established buttons/text controls, switches layouts through a bounded menu
stack, opens skirmish options/map selection, enumerates supported maps, displays map preview and
spawn markers, edits demo player slots, and returns safely without starting simulation. Profiles
that select a 3D shell map may display its completed R3 presentation scene behind the WND overlay,
without running that map's scripts or objects as gameplay. The Options path presents modern
monitor/window-mode, resolution, refresh-rate, and UI-scale controls, applies display changes with a
bounded confirmation/rollback transaction, and persists only accepted settings.

**Owner:** `cic-formats` owns bounded WND and narrowly scoped UI INI decoding. A planned `cic-ui`
crate owns retained layout instances, control state, focus/input, safe action routing, transitions,
menu stack, and render-neutral UI frames. `cic-render` owns the `wgpu` UI backend and text/image GPU
resources. `cic-tools` composes the VFS, CSF/map data, callback registry, diagnostics, headless
captures, and interactive demo launch. No R4 layer may depend on the future simulation crate.

**Acceptance tests:** Every supported WND field and gadget receives original positive fixtures,
every-token truncation/unterminated-record tests, explicit byte/line/token/string/window/depth/list
limits, duplicate/stable-ID policy, unknown token and callback preservation, exact hierarchy
closure, and deterministic reports. UI behavior tests cover hit testing, clipping, z/order, focus,
tab traversal, hover/press/disabled/selected states, radio/check invariants, slider/list/combo bounds,
Unicode text entry, menu push/pop, transition sampling, localization fallback, resolution scaling,
and missing resources. Patch tests cover target/precondition failures, inserted/modified controls,
stable overlay order, provenance, and source immutability. Display-setting tests inject synthetic
monitor/video-mode catalogs and cover stable filtering, deduplication, dependent resolution/refresh
choices, windowed/borderless/exclusive behavior, apply/confirm, timeout rollback, and persistence.
Renderer tests use explicit viewport/DPI/time/input sequences and checked synthetic hashes.
Installed smoke tests retain no retail output.

**Determinism:** WND file and child order control hierarchy, hit testing, focus order, and draw
submission. Stable IDs derive from decorated source names plus deterministic duplicate diagnostics,
never host hashes. VFS mount order controls definitions and assets. Captures specify viewport,
scale policy, locale, font set, transition time, cursor position, focus, input events, selected map,
demo slot values, and a complete display-mode catalog. Mode lists sort deterministically by monitor
key, width, height, refresh millihertz, bit depth, and stable source index. Host DPI, monitor
enumeration order, filesystem order, locale, wall clock, and platform font discovery cannot silently
affect diagnostic output.

**Documentation:** `docs/formats/wnd.md`, `docs/provenance/wnd.md`, ADR 0010, architecture and
compatibility updates, synthetic UI authoring instructions, and user-owned capture guidance. Every
implemented UI family records source revision/notices, exact limits, unsupported fields, resource
fallbacks, input behavior, and completion evidence.

**Completion artifact:** An original synthetic multi-layout WND suite using every established
gadget family, mapped images, Unicode text, callbacks-as-data, focus navigation, and transitions;
checked inventory/semantic reports and headless hashes; plus local user-owned verification that the
main menu renders and can navigate Main Menu -> Options -> display-mode apply/confirm or rollback ->
Main Menu -> Skirmish Options -> Map Select -> Skirmish Options -> Main Menu with map preview/spawn
markers and no simulation launch.

### R4 architecture decision

R4 uses a project-owned retained WND model and a custom UI renderer on the existing `wgpu`/`winit`
stack. Full GUI toolkits are not the compatibility boundary: egui is immediate-mode, while iced
introduces a separate widget/layout/application model. Either would require a lossy translation of
WND rectangles, hierarchy, state images, focus, callbacks, and shell transitions. Focused libraries
remain appropriate below the compatibility layer: prefer `cosmic-text` for Unicode shaping/layout
and `glyphon` for `wgpu` glyph-atlas rendering after verifying compatibility with the workspace's
selected `wgpu`; fall back to a small project-owned glyph upload backend over `cosmic-text` rather
than changing WND semantics. Modern controls absent from a retail or modded layout are introduced by
a versioned declarative WND patch applied after parsing and before retained-state instantiation; no
source WND bytes are edited and no renderer path searches for special window names.

### R4 implementation gates

1. **WND inventory and bounded syntax (implemented).** Specify file versions, `STARTLAYOUTBLOCK`,
   layout init/update/shutdown names, nested `WINDOW`/`CHILD` blocks, creation resolution/rectangles,
   defaults, fields, `DATA`, and exact `END` closure. Preserve callback names and unknown tokens as
   data; never resolve a WND string to a native function pointer in the parser. `DATA` and
   per-gadget field typing are still generic (Gate 2), but nothing is dropped.
2. **Immutable control definitions (implemented).** Decode all established status/style names,
   fonts, text and tooltip labels, state colors/borders, image offsets, draw-data arrays, header
   templates, and gadget-specific records. Apply explicit limits to every nesting and
   variable-length surface. Stable reports must be sufficient to compare a modded WND without
   rendering it.
3. **Bounded WND patch overlays (implemented).** Define a versioned project-owned patch format targeting one WND
   virtual path and exact decorated window names. Support explicit preconditions, known-field
   replacement, hide/show/enable defaults, reparent/reorder where safe, and insertion of complete
   project-owned window subtrees. Apply patches in VFS/profile then file-operation order to produce
   a new immutable definition with per-field provenance; preserve the source document unchanged.
   Missing required targets, duplicate inserted IDs, cycles, limit excess, and invalid gadget data
   are structured errors. Version 1 has no wildcards, arbitrary callbacks, or imperative code.
4. **UI resource resolution (definitions implemented; transitions, cursors, and schemes pending).**
   Add bounded mapped-image, font/language, transition/scheme, cursor,
   and required menu-definition subsets. Resolve CSF labels through the existing localization
   decoder and images/fonts through the VFS. Missing resources use visible placeholders and stable
   diagnostics; system-font fallback is opt-in and never used by deterministic captures.
5. **Retained UI runtime (implemented).** Instantiate immutable definitions into an isolated menu state tree with
   show/hide/enable, parent-relative layout, classic/modern resolution policies, clipping, z-order,
   hit testing, capture, focus, tab order, hover, press, selection, text editing, scrolling, and
   control-specific invariants. UI state is presentation state, not simulation state.
6. **Custom `wgpu` presentation (implemented).**
   Render ordered colored/image quads, borders, state overlays,
   scissor rectangles, cursors, and shaped Unicode text over either a 2D background or an R3 scene.
   Support source alpha and explicit color-space handling, bounded atlases, batched stable draws,
   explicit transition time, and surface-free deterministic capture.
7. **Safe callbacks, shell stack, and transitions.** Retain source system/input/draw/tooltip and
   layout callback names, then route only allowlisted demo actions through typed events. Implement
   push/pop/bring-forward/hide semantics and established transition groups without invoking MAP
   scripts or arbitrary symbols. Unknown callbacks remain reportable and inert.
8. **Working main-menu artifact.** Load the user-owned `Menus/MainMenu.wnd`, mapped images, fonts,
   and CSF labels; render its original controls and text; support hover/focus/click, established
   subpanels, Back, Options, Skirmish navigation, and safe Exit. When configured,
   compose the R3-rendered shell MAP as a non-simulating 3D background beneath the UI. No retail
   capture is checked in.
9. **Modern Options/display settings.** Load `Menus/OptionsMenu.wnd`, reuse its established
   `ComboBoxResolution`, and apply a bounded project patch that adds missing monitor, window-mode,
   refresh-rate, and UI-scale labels/controls without changing user-owned bytes. Enumerate platform
   modes into a stable catalog. Windowed and borderless use explicit desktop/presentation refresh
   semantics; exclusive fullscreen selects an advertised resolution/refresh pair. Apply through
   `winit`/surface reconfiguration, show a timed confirmation dialog, roll back on timeout/failure,
   and persist only confirmed project-owned preferences. Deterministic tests inject the catalog and
   explicit confirmation time rather than reading host monitors or a clock.
10. **Skirmish and map-selection compatibility harness.** Load the user-owned skirmish and map-select
   WND layouts. Bind R3's deterministic map catalog, display name, preview/minimap, playable bounds,
   and `Player_n_Start` candidates. Support demo player-name entry, open/closed/AI slot choices,
   color/faction/team combos, start-position selection, map switching, Back, and a non-executing
   Start validation result. This UI must expose unsupported MAP versions/resources visibly instead
   of hiding incompatible maps.
11. **R4 closure.** Inventory every user-owned WND in the selected profile under parser limits,
   exercise all control families and patch operations synthetically, verify the complete main-menu,
   settings, and skirmish navigation loop at multiple aspect ratios/refresh catalogs, and document
   fields/callbacks that remain retained-but-inert until R5 or later.
