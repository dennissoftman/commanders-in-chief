# ADR 0010: Retained WND UI Model and Custom wgpu Renderer

- Status: Accepted
- Date: 2026-07-22

## Context

The next milestone after complete MAP presentation needs to exercise installed resources through a
real user flow before simulation exists. A main menu and skirmish/map-selection shell will expose
VFS, CSF, image/font, MAP metadata, preview, boundary, and spawn-position compatibility in one
interactive artifact.

The source UI is persisted as WND files. Pinned GeneralsGameCode revision
`9f7abb866f5afd446db14149979e744c7216baaf` establishes a retained hierarchy with file/layout
versions, exact creation rectangles, parent/child blocks, status/style flags, gadget-specific data,
state draw records, text/font data, and named init/update/shutdown/system/input/draw/tooltip
callbacks. The shell maintains a layout stack and transition groups. Callback strings were resolved
to native function pointers in the source runtime, which is not safe or appropriate for untrusted
input in this project.

A general Rust GUI toolkit could render new application UI, but it would become a second semantics
layer between WND and pixels. egui is explicitly immediate-mode. iced uses its own Elm-style state,
view, update, widget, and responsive-layout model. Translating WND into either would obscure exact
rectangles, hierarchy, focus, state images, callback names, and shell transitions. The workspace
already owns a cross-platform `wgpu`/`winit` presentation boundary.

## Decision

- R4 is a complete WND/UI ingestion and navigable-shell milestone between R3 MAP presentation and
  R5 simulation. The prior simulation and gameplay milestones move to R5 and R6.
- `cic-formats` will parse established WND syntax and narrowly scoped UI definition formats into
  immutable values under explicit byte, line, token, string, window, child-depth, draw-data, list,
  and allocation limits. Unknown tokens, flags, styles, callback names, and unresolved resources
  remain inspectable.
- Modern or profile-specific layout additions use a bounded versioned WND patch document applied
  after parsing. Exact decorated-name selectors, explicit preconditions, known-field replacement,
  safe reparent/reorder, and complete inserted WND fragments produce a second immutable definition
  with per-field provenance. Source WND bytes remain unchanged. Version 1 has no wildcards,
  destructive deletion, imperative code, or implicit renderer/menu special cases.
- A planned `cic-ui` crate will own retained control instances, layout scaling, clipping, z-order,
  hit testing, focus/capture, input editing, gadget state, transitions, and shell push/pop. It will
  depend only on immutable definitions and supplied resources/data, not on VFS, GPU, or simulation
  ownership.
- Callback fields are data. Applications provide an allowlisted registry mapping recognized names
  to typed UI events. No parser resolves arbitrary strings to addresses, and unknown callbacks are
  inert diagnostics. Demo actions may change menus and UI presentation state but cannot execute MAP
  scripts or construct gameplay objects.
- `cic-render` will add a custom `wgpu` UI backend consuming stable render-neutral UI frames. It
  renders colored/image quads, borders, overlays, scissors, cursors, and Unicode text in WND order
  and may composite over R3 scene color.
- Text shaping/rasterization is delegated to focused Rust libraries instead of being reimplemented.
  Prefer `cosmic-text` for Unicode shaping/layout and `glyphon` for `wgpu` glyph-atlas rendering
  after verifying their versions against the workspace `wgpu`. If direct `glyphon` integration is
  incompatible, retain `cosmic-text` and implement only the small atlas/upload bridge; do not replace
  WND semantics with a full toolkit.
- Mapped images, fonts, CSF strings, cursors, transitions, and menu resources are resolved through
  explicit VFS composition outside the parser. Deterministic captures use only explicit synthetic
  or user-owned font resources and never platform font discovery.
- Classic source resolution scaling and a Modern aspect-preserving policy are explicit selectable
  presentation policies. Neither changes stored WND rectangles. Viewport, DPI/scale, locale,
  transition time, pointer/focus state, and ordered input events are explicit diagnostic inputs.
- The Settings demo reuses the established `OptionsMenu.wnd:ComboBoxResolution` and applies a
  project-owned WND patch to add absent monitor, window-mode, refresh-rate, and UI-scale controls.
  A platform adapter supplies a stable-sorted immutable mode catalog. Exclusive fullscreen chooses
  advertised width/height/refresh-millihertz tuples; borderless follows the monitor desktop mode;
  windowed mode chooses client size and reports desktop refresh. Mode changes use explicit-time
  confirmation and rollback, and only confirmed choices enter versioned project preferences.
- The R4 completion artifact loads the user-owned main menu and navigates to skirmish options and
  map selection. It binds R3 map names, previews, playable bounds, and ordered spawn candidates;
  controls edit demo slot values. A configured shell MAP may use its R3 presentation scene as the
  non-simulating background. Start validates and reports a launch description but remains
  non-executing until R5.

## Ownership and dependency boundary

```text
WND / UI INI / CSF / images / fonts / R3 map catalog / WND patches
                         |
                         v
          bounded immutable format/resource values
                         |
                         v
        ordered pure patch transform + provenance
                         |
                         v
             cic-ui retained presentation state
                |                         |
                v                         v
       render-neutral UiFrame       typed demo events
                |                         |
                v                         v
       cic-render wgpu backend      shell/menu router
                                          |
                                          v
                              future typed R5 commands
```

`cic-ui` is a demonstrated dependency boundary because retained controls and input/navigation state
are neither file formats nor GPU resources. R4 may introduce the crate when implementation begins;
this ADR does not create code or dependencies by itself.

## Acceptance and determinism

Original fixtures must exercise every established gadget family, nested children, state flags,
draw/text records, callback names, mapped images, Unicode labels and editing, focus/tab order,
clipping, menu push/pop, and transitions. Negative tests cover every token boundary, malformed
blocks, recursion and count limits, invalid rectangles/numbers/colors, duplicate IDs, unresolved
resources, and unsupported callbacks. Stable reports preserve source order and raw names.

Headless captures specify all presentation inputs and produce checked hashes. Interactive local
verification loads user-owned resources and completes Main Menu -> Options -> display apply/confirm
or rollback -> Main Menu -> Skirmish Options -> Map Select -> Skirmish Options -> Main Menu without
starting gameplay or retaining retail captures. Patch tests prove source immutability, precondition
handling, deterministic layering, and provenance. Display tests inject monitor/mode catalogs and
explicit confirmation time, including failed apply and timeout rollback. The map list visibly
reports unsupported MAP versions or missing resources instead of silently hiding them.

## Consequences

- R4 becomes an integration harness for R0-R3 resource, localization, rendering, and MAP support.
- Source WND layouts and modded menus retain their hierarchy and state model rather than being
  approximated through another GUI framework.
- Modern settings can augment edition/mod layouts through auditable profile data rather than
  hardcoded window-name checks or modified retail files.
- The project must implement classic gadget behavior and safe menu routing, but can reuse mature
  text shaping/rasterization and the existing GPU/window stack.
- The UI can later submit typed commands to R5 and display immutable simulation snapshots without
  becoming authoritative.
- Gameplay HUD layouts may be parsed and rendered in R4, but bindings that require live objects,
  command dispatch, or simulation values remain inert until R5/R6.

## Rejected alternatives

- **Use egui as the WND runtime:** rejected because immediate-mode layout/state is not the persisted
  retained WND model and would make exact compatibility harder to test.
- **Translate WND widgets into iced:** rejected because iced's application/widget/layout model would
  duplicate menu state and alter source layout semantics.
- **Copy the legacy callback/function-lexicon model:** rejected because untrusted strings must never
  select native function pointers.
- **Hardcode refresh-rate widgets in the Options callback or renderer:** rejected because it would
  couple one installed layout to Rust code, bypass mod layouts, and hide provenance. A bounded patch
  expresses the same augmentation as inspectable profile data.
- **Wait for simulation before building UI:** rejected because the shell is valuable as an R3 map
  compatibility and resource-integration harness on its own.

## Evaluated library references

- egui: <https://github.com/emilk/egui>
- iced: <https://github.com/iced-rs/iced>
- glyphon: <https://github.com/grovesNL/glyphon>
- cosmic-text: <https://github.com/pop-os/cosmic-text>

These references informed the architectural choice only. Dependency versions and license notices
must be reviewed and recorded when implementation adds a crate.
