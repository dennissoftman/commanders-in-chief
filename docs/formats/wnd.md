# WND Layout and R4 UI Compatibility Plan

- Status: source-established design; implementation not started
- Owning crates: `cic-formats` for syntax/immutable values, planned `cic-ui` for retained state,
  `cic-render` for GPU presentation
- Last updated: 2026-07-22

## Evidence and boundary

The WND parser, window/gadget declarations, window layout and shell sources in
TheSuperHackers/GeneralsGameCode revision `9f7abb866f5afd446db14149979e744c7216baaf`
establish the persisted hierarchy, field vocabulary, classic controls, callback names, layout stack,
and menu transitions. Mapped-image, font/language, CSF, main-menu, and skirmish sources establish the
resource and demo bindings. Exact paths, notices, and permanent links are in
`docs/provenance/wnd.md`.

R4 treats WND as untrusted declarative data. The legacy runtime's function-pointer lookup is
evidence for callback-name fields, not an implementation model. Retail WND files and UI assets are
never included in fixtures or captures.

## Source-established file structure

The first record is a numeric `FILE_VERSION`. Version 2 and later require a layout block:

```text
FILE_VERSION = <version>
STARTLAYOUTBLOCK
  LAYOUTINIT = <name>;
  LAYOUTUPDATE = <name>;
  LAYOUTSHUTDOWN = <name>;
ENDLAYOUTBLOCK

<optional default visual records>
WINDOW
  WINDOWTYPE = <style>;
  SCREENRECT = UPPERLEFT: <x> <y> BOTTOMRIGHT: <x> <y>
               CREATIONRESOLUTION: <width> <height>;
  <window fields>
  DATA = <gadget-specific values>;
  CHILD
    WINDOW
      ...
    END
  ENDALLCHILDREN
END
```

The exact lexical grammar, comments/quoting, default-record set, and version differences require an
original synthetic fixture before implementation. The parser will preserve source byte strings and
unknown records while exposing normalized known tokens. It will not reproduce the source reader's
assertion, truncation, or unknown-token skipping behavior.

Window rectangles are stored with a creation resolution. Child positions become parent-relative in
the retained hierarchy. The immutable value keeps stored coordinates and creation resolution
exactly; scaling happens only in the UI presentation policy.

## Established status and style vocabulary

Status names are retained in source order:

```text
ACTIVE TOGGLE DRAGABLE ENABLED HIDDEN ABOVE BELOW IMAGE TABSTOP NOINPUT
NOFOCUS DESTROYED BORDER SMOOTH_TEXT ONE_LINE NO_FLUSH SEE_THRU RIGHT_CLICK
WRAP_CENTERED CHECK_LIKE HOTKEY_TEXT USE_OVERLAY_STATES NOT_READY FLASHING ALWAYS_COLOR
```

Style names are:

```text
PUSHBUTTON RADIOBUTTON CHECKBOX VERTSLIDER HORZSLIDER SCROLLLISTBOX ENTRYFIELD
STATICTEXT PROGRESSBAR USER MOUSETRACK ANIMATED TABSTOP TABCONTROL TABPANE COMBOBOX
```

Known names map to explicit enums/bit sets while raw spellings and unknown names remain reportable.
Duplicate or contradictory flags are not silently repaired by the parser.

## Window fields and control data

The planned immutable window record includes:

- decorated source name and a stable source-order ID;
- window type, stored rectangle, creation resolution, hierarchy and child order;
- status/style names and unknown bits/tokens;
- system, input, tooltip, draw, layout-init, layout-update, and layout-shutdown callback names;
- font name/size/bold, header-template name, text and tooltip CSF labels, tooltip delay, image offset,
  and enabled/disabled/highlight text/border colors;
- state-specific image/color/border draw records, including composite records used by sliders,
  list boxes, combo boxes, edit boxes, drop-down buttons, scroll buttons, and thumbs; and
- gadget-specific `DATA` or named records for slider ranges, radio grouping, list columns/scrolling,
  combo composition, static-text alignment/margins, entry maximum/secret/filter policy, tab sizing,
  and other established fields.

All values remain renderer-neutral. Numeric conversions are checked, colors preserve exact channel
bytes, rectangles and ranges reject overflow, and source callback strings never become callable
addresses.

## Planned default parser limits

The first implementation should use conservative configurable defaults and document any change:

- 8 MiB per WND file;
- 262,144 lexical tokens/records and 65,536 physical lines;
- 65,536 bytes per physical or semicolon-terminated record;
- 4,096 bytes per name, callback, image, font, text-label, or tooltip-label field;
- 16,384 windows per layout and 256 parent/child depth;
- 64 draw records per state/category and 256 list/combo columns or nested component records; and
- 16,384 layouts/resources in one profile inventory.

Limits are checked before allocation. Unterminated quoted/semicolon records, unmatched child/end
blocks, missing required fields, non-finite or overflowing numeric data, invalid ranges, duplicate
decorated names, excessive nesting, trailing lexical garbage, and unsupported required versions
return structured errors. User-owned observations may raise a limit, but may not remove it.

## Planned WND patch overlays

Modern controls must not be hardcoded in the parser, renderer, or menu callback implementation.
R4 adds a project-owned declarative `.cic-wnd-patch` layer after WND parsing and before retained
control instantiation:

```text
source WND bytes -> immutable WndDocument -> ordered patches -> immutable PatchedWndDocument
```

A version-1 patch identifies one normalized target WND virtual path and uses exact decorated window
names. It may declare required target/field/value preconditions, replace a known immutable field,
change initial visibility/enablement, reparent or reorder a window when this cannot form a cycle, or
insert a complete bounded WND fragment beneath an exact parent. It cannot use selectors/wildcards,
delete source records, execute code, introduce unregistered callback behavior, or modify source
bytes. Hiding a source control remains a visible patch operation rather than destructive deletion.

Patch files are selected explicitly by the active profile and then layered in VFS mount/file order;
operations apply in file order. Every resulting field and inserted subtree retains source/patch
provenance. A missing required target, failed precondition, duplicate decorated name, invalid
fragment, cycle, unsupported patch version, or limit excess is a structured error. Optional targets
may be skipped only with a stable diagnostic. Planned defaults are 1 MiB per patch, 4,096 operations,
4,096-byte paths/names/values, and the enclosing WND's existing window/depth/allocation limits.

This mechanism supports installed editions and mods without embedding retail-specific geometry in
Rust code. A project patch can reuse `OptionsMenu.wnd:ComboBoxResolution`, reposition surrounding
controls, and insert project-owned labels/combo boxes for monitor, window mode, refresh rate, and UI
scale. Mod profiles may replace that patch or provide a compatible overlay for a redesigned
Options layout.

## Related UI resources

WND names are resolved only after parsing through explicit VFS composition:

- mapped-image definitions select named texture regions and dimensions;
- image files provide state backgrounds, overlays, borders, cursors, and icons;
- CSF labels provide localized window, button, tooltip, list, and field text;
- language/font definitions provide explicit font files and named size/bold descriptions;
- header templates and menu schemes provide shared presentation definitions;
- window-transition definitions provide named groups and explicit-time effects; and
- R3 provides map catalog entries, preview/minimap data, playable bounds, and ordered spawn
  candidates for skirmish controls.

Each related format receives a narrow bounded decoder or established existing decoder. Definition
overrides use VFS mount order. Missing names produce stable diagnostics and visible placeholders.
Deterministic captures never fall back to host fonts, locale, DPI, filesystem paths, or resource
enumeration order.

## Retained UI behavior

The parser returns definitions; the planned `cic-ui` layer creates retained instances. Required R4
behavior includes parent-relative layout, classic and Modern scaling policies, visibility,
enablement, z/order, clipping, mouse hit testing/capture, hover/press/release, keyboard focus and tab
traversal, radio/check invariants, slider bounds, list/combo selection and scrolling, Unicode text
entry/selection, password masking, tooltips, cursors, and transition sampling.

Control state changes produce typed UI events. Callback fields are looked up only in an application
allowlist. Unknown callback names are inert. Layout update names do not create a general scripting
language, and MAP scripts are never dispatched by the UI runtime.

## Rendering policy

The source-compatible UI model is rendered by a custom `wgpu` backend, not translated into egui or
iced widgets. Stable WND order produces colored/image quads, borders, state overlays, scissor
rectangles, cursors, and text runs. Image color space and alpha mode are explicit. Texture/glyph
atlases and batches are bounded and may optimize submission only if committed in stable order.

Unicode shaping/layout should use `cosmic-text`; `glyphon` is the preferred `wgpu` glyph renderer if
its selected version is compatible with the workspace `wgpu`. These libraries are implementation
components, not UI semantics. Classic presentation follows stored creation-resolution scaling;
Modern presentation preserves hierarchy and coordinates while applying an explicitly documented
aspect/safe-area policy.

## Main-menu, Settings, and skirmish demo

The completion demo loads user-owned `Menus/MainMenu.wnd` and its referenced resources, renders
established buttons/text fields, and supports focus, hover, click, subpanel transitions, Back,
Skirmish, and safe Exit. Actions requiring online services, save/replay, campaign simulation, or
external tools are visibly disabled or diagnostic in demo mode. If the selected profile names a
shell MAP, its completed R3 presentation scene may render behind the WND overlay with explicit
presentation time; no MAP script or gameplay object is activated.

The Settings path loads user-owned `Menus/OptionsMenu.wnd`. The established layout/runtime already
names `OptionsMenu.wnd:ComboBoxResolution` and populates width/height/bit-depth display modes, but
the established display API has no refresh-rate field. R4 keeps that resolution control and applies
the bounded patch above to add missing Modern display controls:

- monitor selector for the explicit platform catalog;
- window mode: windowed, borderless desktop, or exclusive fullscreen;
- resolution selector supporting modern 16:9, ultrawide, high-DPI, and other advertised modes;
- refresh selector stored exactly in millihertz for exclusive-mode pairs; and
- UI scale/policy selector independent from render resolution.

The platform adapter supplies immutable monitor/mode records containing a stable per-session key,
name, dimensions, refresh millihertz, bit depth/format where available, and source index. The UI
sorts and deduplicates them deterministically. Exclusive fullscreen exposes only advertised
resolution/refresh pairs. Borderless uses the selected monitor's desktop mode; windowed mode keeps
an explicit client size and reports desktop refresh rather than pretending to select it.

Apply is transactional: retain the previous accepted mode, request the new `winit` window/surface
configuration, present an explicit-time confirmation dialog, and commit the project-owned
preference only after confirmation. Failure, timeout, window close, or lost confirmation restores
the previous mode. Deterministic tests inject the complete mode catalog, previous preference,
confirmation event, and elapsed time. They never enumerate host monitors or read a wall clock.
The workspace's current `winit` 0.30 mode API exposes video-mode refresh in millihertz on supported
platforms; when a backend reports no selectable modes or refresh, the corresponding control is
disabled with an explicit capability diagnostic rather than fabricating values.

Skirmish navigation loads the established skirmish-options and map-select layouts. It binds a
stable R3 map catalog to map lists and displays localized name, preview/minimap, playable bounds,
and `Player_n_Start` markers. Demo slot controls cover player name, open/closed/AI state,
color/faction/team selection, and start-position assignment. Start validates required selections
and emits a stable launch description; it cannot construct teams, run scripts, or start a match
before R5.

## Planned reports and artifacts

- `cic-inspect wnd` reports file/layout metadata, hierarchy, rectangles, flags, fields, gadget data,
  callback names, and unknowns in source order.
- `cic-inspect wnd-patch` reports target/preconditions, operations, resulting hierarchy, per-field
  provenance, and stable incompatibility diagnostics without writing a patched retail WND.
- `cic-inspect ui-resources` reports resolved/missing images, fonts, labels, transitions, and
  provenance without embedding retail data.
- `cic-inspect ui-render` emits a deterministic synthetic PNG/hash for an explicit layout,
  viewport, scale policy, locale, font set, time, and input/state snapshot.
- `cic-inspect ui-demo` launches the interactive main-menu/skirmish compatibility harness.

The checked-in completion artifact is entirely original. Installed verification records aggregate
layout/control/resource counts and navigation success only; no retail screenshots or assets are
retained.

## Explicit exclusions and later bindings

- R4 does not execute MAP scripts, create gameplay objects, assign runtime players/teams, or launch
  a match. It may own versioned presentation/display preferences, but not authoritative gameplay
  state.
- Network/login/lobby services, save/replay operations, platform dialogs, web links, and external
  tools are outside the demo even when their controls render.
- Gameplay HUD WND files may parse and render, but live object/command bindings wait for R5/R6.
- Unknown WND versions, required tokens, callbacks, and gadget extensions remain visible and inert
  until separately established; nearby layouts are never guessed.
