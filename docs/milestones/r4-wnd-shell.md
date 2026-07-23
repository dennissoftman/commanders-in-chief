# R4: WND user interface and navigable shell

**Status:** Active. R3 produced the complete non-simulating MAP scene and scenario description;
the first vertical slice is bounded WND inventory/layout decoding plus a synthetic headless menu.

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

1. **WND inventory and bounded syntax.** Specify file versions, `STARTLAYOUTBLOCK`, layout
   init/update/shutdown names, nested `WINDOW`/`CHILD` blocks, creation resolution/rectangles,
   defaults, fields, `DATA`, and exact `END` closure. Preserve callback names and unknown tokens as
   data; never resolve a WND string to a native function pointer in the parser.
2. **Immutable control definitions.** Decode all established status/style names, fonts, text and
   tooltip labels, state colors/borders, image offsets, draw-data arrays, header templates, and
   gadget-specific records. Apply explicit limits to every nesting and variable-length surface.
   Stable reports must be sufficient to compare a modded WND without rendering it.
3. **Bounded WND patch overlays.** Define a versioned project-owned patch format targeting one WND
   virtual path and exact decorated window names. Support explicit preconditions, known-field
   replacement, hide/show/enable defaults, reparent/reorder where safe, and insertion of complete
   project-owned window subtrees. Apply patches in VFS/profile then file-operation order to produce
   a new immutable definition with per-field provenance; preserve the source document unchanged.
   Missing required targets, duplicate inserted IDs, cycles, limit excess, and invalid gadget data
   are structured errors. Version 1 has no wildcards, arbitrary callbacks, or imperative code.
4. **UI resource resolution.** Add bounded mapped-image, font/language, transition/scheme, cursor,
   and required menu-definition subsets. Resolve CSF labels through the existing localization
   decoder and images/fonts through the VFS. Missing resources use visible placeholders and stable
   diagnostics; system-font fallback is opt-in and never used by deterministic captures.
5. **Retained UI runtime.** Instantiate immutable definitions into an isolated menu state tree with
   show/hide/enable, parent-relative layout, classic/modern resolution policies, clipping, z-order,
   hit testing, capture, focus, tab order, hover, press, selection, text editing, scrolling, and
   control-specific invariants. UI state is presentation state, not simulation state.
6. **Custom `wgpu` presentation.** Render ordered colored/image quads, borders, state overlays,
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
