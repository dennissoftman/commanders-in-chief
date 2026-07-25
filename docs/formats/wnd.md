# WND Layout and R4 UI Compatibility Plan

- Status: Gates 1 and 2 (bounded WND inventory/hierarchy decode and immutable typed control
  definitions) and Gate 3 (bounded patch overlays) implemented; Gates 1-2 are verified against
  every retail layout in both editions. Resource resolution (Gate 4) and the retained `cic-ui`
  state (Gate 5+) not started
- Owning crates: `cic-formats` for syntax/immutable values, planned `cic-ui` for retained state,
  `cic-render` for GPU presentation
- Last updated: 2026-07-25

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
FILE_VERSION = <version>;
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
  CHILD
    WINDOW
      ...
    END
  ENDALLCHILDREN
END
```

`FILE_VERSION` is itself a semicolon-terminated record, not a bare newline-terminated one — every
retail `.wnd` file in `Window.big` opens with `FILE_VERSION = 2;`.

`CHILD` is an **optional marker, not a required prefix**. The source's child-list loop
(`parseChildWindows`) dispatches only on `ENDALLCHILDREN`, `END`, the five default-color keywords,
and `WINDOW`; it has no `CHILD` branch and no fallback branch, so a `CHILD` token inside a child
list is silently skipped and a bare `WINDOW` opens the next sibling. `ENDALLCHILDREN` closes the
list; `END` closes it too. A window with no children omits `CHILD`/`ENDALLCHILDREN` entirely.

Retail data nevertheless writes `CHILD` before nearly every child, which is why an earlier revision
of this document described the marker as mandatory. A structural census over all 80 `.wnd` files
reachable through the `Wnd` resource profile in both retail editions measured 1,667 `WINDOW`
records against 80 roots plus 1,586 `CHILD` markers — leaving exactly **one** sibling declared
without its marker, in Zero Hour's `Menus/MainMenu.wnd`. Requiring the marker therefore made the
single most important layout for the R4 main-menu artifact undecodable. The decoder now accepts
either spelling once the child list is open and reports the unmarked form as a non-fatal
`MissingChildKeyword` diagnostic; a bare `WINDOW` before any `CHILD` remains a field name, matching
the source's separate field loop. No retail file content is reproduced here or in any fixture.

The census also measured every configured parser limit against real data: maximum child depth 6,
maximum windows per file 113, maximum children under one window 80, and longest semicolon-terminated
record 833 bytes. Every default below has at least an order of magnitude of headroom, so no limit
needs raising for retail compatibility.

The lexical grammar is now confirmed directly from `winCreateFromScript` and `parseLayoutBlock` in
`GameWindowManagerScript.cpp`: there is no comment syntax; `;` is a hard statement terminator (a
dedicated "read until semicolon" scan collapses whitespace runs and trims), not a comment marker;
structural keywords (`WINDOW`, `CHILD`, `END`, `ENDALLCHILDREN`, `WINDOWTYPE`, `FILE_VERSION`,
`SCREENRECT`, `STARTLAYOUTBLOCK`, `ENDLAYOUTBLOCK`) are matched case-sensitively while `STATUS`/
`STYLE` names are matched case-insensitively; names are double-quote-delimited with no escape
handling; numbers are plain decimal; and colors are four separate decimal R/G/B/A tokens, never a
packed hex literal. `SCREENRECT` is one semicolon-terminated record whose `UPPERLEFT`/
`BOTTOMRIGHT`/`CREATIONRESOLUTION` sub-labels and numeric pairs are tokenized together. The layout
block is mandatory only for `FILE_VERSION >= 2`; version 1 documents default all three callback
names to the literal source string `"[None]"`. The `<optional default visual records>` placeholder
is not one construct: it is several independently keyworded, order-free top-level statements in the
same flat parse loop as `WINDOW` — `ENABLEDCOLOR`, `DISABLEDCOLOR`, `HILITECOLOR`, `SELECTEDCOLOR`,
`TEXTCOLOR`, `BACKGROUNDCOLOR` (a color value or the literal `TRANSPARENT`) and `FONT` (the real
source stubs this field: the value is read and discarded, marked `@todo`).

The bounded decoder (`crates/cic-formats/src/wnd.rs`) preserves the complete `WINDOW`/`CHILD`
hierarchy with `WINDOWTYPE` and `SCREENRECT` typed; every other field, at both the top level and
inside a window, is retained generically and is **never dropped**, whether or not its name is
recognized. This is a deliberate divergence from this crate's other text/INI decoders, which
silently ignore unrecognized fields — a dropped field can hide a missing or unsupported feature
with no way to notice. Unrecognized top-level keywords and out-of-vocabulary `WINDOWTYPE`,
`STATUS`, and `STYLE` values are additionally surfaced as a non-fatal `WndDiagnostic` so gaps stay
discoverable.

Each retained field carries both a verbatim `raw_value` (source text with whitespace runs collapsed
and ends trimmed) and an ordered token sequence that keeps quoting and `,`/`:`/`+` punctuation
explicit. A flattened string alone is lossy in a way later gates cannot recover:
`FONT = NAME: "Times New Roman", SIZE: 14, BOLD: 0;` and the same record written without quotes
collapse to the same characters, leaving no way to tell where a font name containing spaces ends.
Quoted tokens are never split, so a decorated name like `"MainMenu.wnd:ButtonExit"` keeps its `:`
while an unquoted sub-label separator is tokenized.

A window's `NAME` record is typed into `WndWindow::name`, with `control_name` exposing the portion
after the first `:`. Typed accessors are views over the retained field list rather than
replacements for it, so the never-dropped invariant is unaffected as later gates add types. Two
windows sharing a non-empty control name produce a `DuplicateWindowName` diagnostic rather than an
error: the legacy runtime creates both, and no retail layout in either edition contains such a pair,
so rejecting the document would only make an unusual modded layout undecodable while hiding the
collision a patch overlay needs to see. Windows whose control part is empty (`"OptionsMenu.wnd:"`,
126 of 1,667 across both editions) are unnamed rather than name-sharing, and are excluded from that
comparison — counting them would report 31 retail layouts as containing duplicates.

### Established window field vocabulary

`parseWindow`'s field chain at the pinned revision compares against exactly 46 keywords. Three of
them — `TABCONTROLDATA`, `IMAGEOFFSET`, and `TOOLTIP` — have **zero occurrences** across all 80
retail layouts in both editions; the remaining 43 account for every field name the census observed,
so the source list and the data agree exactly.

Thirteen of the 43 appear on every one of the 1,667 retail windows: `NAME`, `STATUS`, `STYLE`, the
four `*CALLBACK` records, `FONT`, `TEXTCOLOR`, `HEADERTEMPLATE`, and the three universal
`ENABLEDDRAWDATA`/`DISABLEDDRAWDATA`/`HILITEDRAWDATA` records. Everything else is gadget-scoped and
never appears on a window type it does not belong to.

These common records are typed, with their established shapes confirmed against retail data:

| Record | Shape | Notes |
| --- | --- | --- |
| `NAME` | one quoted decorated name | `"MainMenu.wnd:ButtonExit"` |
| `STATUS` / `STYLE` | `+`-separated flag names | validated against the union vocabulary |
| `*CALLBACK` | one quoted name | `"[None]"` on 1,468 of 1,667 windows for `SYSTEMCALLBACK` |
| `FONT` | `NAME: "<name>", SIZE: <int>, BOLD: <int>` | quoting is what delimits a name with spaces |
| `HEADERTEMPLATE` | one quoted name | sentinel is case-variant: `[NONE]` and `[None]` both occur |
| `TOOLTIPDELAY` | one bare integer | `-1` on 1,590 windows |
| `TEXT` / `TOOLTIPTEXT` | one quoted value | **not always a label**: retail authors both label keys (`GUI:Monkey`) and literal strings (`Static Text`), so the value is retained unclassified |
| `TEXTCOLOR` | six labeled RGBA colors | `ENABLED`, `ENABLEDBORDER`, `DISABLED`, `DISABLEDBORDER`, `HILITE`, `HILITEBORDER` |

Applying these to every retail layout in both editions produces no malformed-field diagnostics and
types every occurrence: 1,667 of 1,667 `FONT` and `TEXTCOLOR` records and 6,668 of 6,668 callbacks.

A typed decode that fails is a diagnostic, not an error. The line is deliberate: required
structural values (`FILE_VERSION`, `WINDOWTYPE`, `SCREENRECT`) are hard errors because nothing
downstream can proceed without them, while an optional presentation record is a *view* over a field
that is retained generically either way — so a malformed one stays visible without making the whole
layout undecodable.

The 21 draw-data arrays share one shape: exactly **nine** `IMAGE: <name>, COLOR: r g b a,
BORDERCOLOR: r g b a` entries. The count is fixed by the format — `parseDrawData` loops
`MAX_DRAW_DATA` times and `Gadget.h` pins that constant via `NUM_TAB_PANES = 8, //(MAX_DRAW_DATA -
1)` — and all 7,875 retail records carry exactly nine. `NoImage` is the explicit no-image sentinel,
decoded as an absent image rather than an image with that name.

The seven gadget `DATA` records are:

| Record | Shape |
| --- | --- |
| `LISTBOXDATA` | `LENGTH`, `AUTOSCROLL`, **optional** `SCROLLIFATEND`, `AUTOPURGE`, `SCROLLBAR`, `MULTISELECT`, `COLUMNS`, one `COLUMNSWIDTH` per column when `COLUMNS > 1`, `FORCESELECT` |
| `COMBOBOXDATA` | `ISEDITABLE`, `MAXCHARS`, `MAXDISPLAY`, `ASCIIONLY`, `LETTERSANDNUMBERS` |
| `SLIDERDATA` | `MINVALUE`, `MAXVALUE` |
| `RADIOBUTTONDATA` | `GROUP` |
| `TEXTENTRYDATA` | `MAXLEN`, `SECRETTEXT`, `NUMERICALONLY`, `ALPHANUMERICALONLY`, `ASCIIONLY` |
| `STATICTEXTDATA` | `CENTERED` |
| `TABCONTROLDATA` | `TABORIENTATION`, `TABEDGE`, `TABWIDTH`, `TABHEIGHT`, `TABCOUNT`, `PANEBORDER`, `PANEDISABLED` with a count and one flag per counted pane |

`LISTBOXDATA`'s `SCROLLIFATEND` really is optional — the source peeks that label, matches it
case-insensitively, and only consumes a value when it matches; five retail records omit it, and a
fixed-sequence decode misreads every one of them. An omitted sub-record is decoded as absent rather
than false, preserving the distinction the source collapses.

`IMAGEOFFSET` is two bare integers with no sub-labels; note the source splits that record on
whitespace only, unlike the label-bearing records.

**Sub-label spellings are not validated.** The source `strtok`s past every label and reads values
positionally, so a layout may spell them anything and the legacy runtime still loads it. Rejecting
those would refuse files the game itself accepts. `SCROLLIFATEND` is the single exception the source
genuinely tests.

`TOOLTIP` is **not typed and never will be from this evidence**: `parseTooltip` ignores its buffer
entirely and stores the placeholder `"Need tooltip translation"`, marked `@todo`. The record has no
established grammar, so it stays retained generically. `TOOLTIPTEXT` is the field that actually
carries tooltip content.

Two typed records rest on the source alone, with no retail occurrence to cross-check:
`TABCONTROLDATA` and `IMAGEOFFSET`. Both are marked as such in the decoder.

With these, every field name occurring anywhere in either retail edition is typed, and the whole
corpus decodes with no malformed-field diagnostics: 7,875 of 7,875 draw-data arrays (70,875
entries) and 753 of 753 gadget `DATA` records.

Window rectangles are stored with a creation resolution. Child positions become parent-relative in
the retained hierarchy. The immutable value keeps stored coordinates and creation resolution
exactly; scaling happens only in the UI presentation policy. The renderer-only
`StagedWndScene`/`capture_wnd_scene` proof-of-pipeline capture (`crates/cic-render/src/wnd_scene.rs`)
treats every rectangle as absolute (not parent-relative) for now, deferring correct parent-relative
positioning to the retained UI runtime gate.

## Established status and style vocabulary

Status names are retained in source order. **The vocabulary is edition-dependent.** The
`Generals` source path defines 25 names:

```text
ACTIVE TOGGLE DRAGABLE ENABLED HIDDEN ABOVE BELOW IMAGE TABSTOP NOINPUT
NOFOCUS DESTROYED BORDER SMOOTH_TEXT ONE_LINE NO_FLUSH SEE_THRU RIGHT_CLICK
WRAP_CENTERED CHECK_LIKE HOTKEY_TEXT USE_OVERLAY_STATES NOT_READY FLASHING ALWAYS_COLOR
```

The `GeneralsMD` (Zero Hour) source path appends a 26th name after `ALWAYS_COLOR`:

```text
ON_MOUSE_DOWN
```

`ON_MOUSE_DOWN` occurs 67 times in retail Zero Hour layouts and never in Generals layouts, so a
decoder validating status names against the Generals list alone would report 67 false unknowns
against a stock Zero Hour install. Bit positions are array indices in both editions, so the shared
prefix keeps identical values and Zero Hour only extends the high end.

Multiple status names are joined with `+` (`ENABLED+NOFOCUS+SEE_THRU`), not whitespace or commas.
Only 15 of the 26 names occur anywhere in retail data; the remainder are established by source but
unexercised, and still require synthetic fixtures.

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
- 4,096 retained tokens per record, bounding the per-record token vector independently of the
  record byte limit, which alone would permit a 65,536-token allocation from one record;
- 256 `COLUMNSWIDTH` entries per `LISTBOXDATA` and 8 panes per `TABCONTROLDATA`. Both counts are
  read from the record itself and are therefore attacker-controlled. The tab-pane bound is the
  source's own `NUM_TAB_PANES` array width, which the source reads past without checking; the
  widest retail list declares eight columns;
- draw-data arrays need no configurable limit: the entry count is fixed at nine by the format, and
  a record with any other count is a malformed-field diagnostic;
- 16,384 windows per layout and 256 parent/child depth;
- 64 draw records per state/category and 256 list/combo columns or nested component records; and
- 16,384 layouts/resources in one profile inventory.

Limits are checked before allocation. Unterminated quoted/semicolon records, unmatched child/end
blocks, missing required fields, non-finite or overflowing numeric data, invalid ranges, duplicate
decorated names, excessive nesting, trailing lexical garbage, and unsupported required versions
return structured errors. User-owned observations may raise a limit, but may not remove it.

## WND patch overlays

Implemented in `crates/cic-formats/src/wnd_patch.rs`, with `cic-inspect wnd-patch` reporting the
result.

A patch is a line-oriented text document. Arguments are whitespace-separated and may be
double-quoted; inside quotes, `\"` and `\\` are escapes. This is a deliberate divergence from the
WND grammar, which has no escapes: a patch must be able to *express* a WND value that itself
contains quotes, such as a `FONT` record, and without escapes that value would be unwritable. A `#`
beginning a bare token starts a comment.

```text
# Reposition the stock resolution combo and give it a modern label.
version 1
target Menus/OptionsMenu.wnd

require-window "OptionsMenu.wnd:ComboBoxResolution"
require-field  "OptionsMenu.wnd:ComboBoxResolution" STATUS "ENABLED"
set-field      "OptionsMenu.wnd:ComboBoxResolution" STATUS "ENABLED+IMAGE"
add-field      "OptionsMenu.wnd:ComboBoxResolution" TOOLTIPDELAY "250"
set-rect       "OptionsMenu.wnd:ComboBoxResolution" 240 300 460 330 800 600
reorder        "OptionsMenu.wnd:ComboBoxResolution" 0
reparent       "OptionsMenu.wnd:StaticTextResolution" "OptionsMenu.wnd:OptionsMenuParent" 1

insert-window  "OptionsMenu.wnd:OptionsMenuParent" 0
WINDOW
  WINDOWTYPE = COMBOBOX;
  SCREENRECT = UPPERLEFT: 240 340 BOTTOMRIGHT: 460 370 CREATIONRESOLUTION: 800 600;
  NAME = "OptionsMenu.wnd:ComboBoxRefreshRate";
  STATUS = ENABLED;
END
end-window
```

An `insert-window` body runs to a line reading `end-window` and is parsed by the ordinary bounded
WND decoder, wrapped in a minimal version-1 document. An inserted subtree is therefore held to
exactly the same grammar, limits, and typed-field rules as authored source, and its window ids are
renumbered past the highest the document already uses so they cannot collide.

`reparent` refuses to move a window beneath itself or one of its own descendants. The moved window
is detached before its destination is resolved, so a failure after detaching restores the tree
rather than dropping the subtree.

`version` and `target` must precede the first operation. Operations name one **exact decorated
control name** (`File.wnd:Control`). Windows whose control part is empty are never matched: several
windows in one layout share that spelling, so targeting one would be ambiguous.

`target` is compared against the document's virtual path case-insensitively with `\` normalized to
`/`, so a patch matches regardless of how the VFS spells the path.

Patches apply in slice order and operations within a patch apply in file order, so a later patch
observes an earlier one's result. Applying returns a **new** document; the parsed source value is
never mutated, so one parse can be patched differently per profile. Every write records provenance
(control, field, patch name, patch line).

A patched field is re-typed from its new value, so an overlay that rewrites `STATUS` is visible
through the typed accessor and not only in the raw record.

Structured errors cover a target mismatch, a missing required control, a failed precondition, a
`set-field` naming an absent field, an `add-field` naming a present one, a value that will not
tokenize as a WND record, an unknown directive, an unsupported version, and every limit excess.
Default limits are 1 MiB per patch, 16,384 lines, 4,096 operations, and 4,096 bytes per argument.

### Patch overlay boundaries

Version 1 deliberately has no selectors or wildcards, and cannot delete source records, execute
code, or introduce unregistered callback behavior. Hiding a source control is a visible `STATUS`
edit rather than destructive deletion.

Profile-driven patch selection — naming patch files in a profile and layering them in VFS mount
order — is the remaining integration step; the apply engine already applies an ordered slice.

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

### Measured Gate 4 scope

Gate 2's typed records make the demand side countable. Across the Zero Hour profile's 80 layouts,
the WND records name **217 distinct mapped images**, **7 distinct font families** in 24
name/size/bold combinations, **15 header templates**, and **366 label-shaped `TEXT`/`TOOLTIPTEXT`
values**. Fifteen further `TEXT` values are literal strings rather than label keys, which is why the
parser retains that field unclassified.

The definitions live in three places, and two of them are not where a reader might expect:

| Resource | Location |
| --- | --- |
| Mapped images | `Data/INI/MappedImages/**/*.INI` in `INI.big`/`INIZH.big`/`Patch.big`, under `HandCreated/` and `TextureSize_512/` |
| Header templates | `Data/<Language>/HeaderTemplate.ini` — in the **localization** archive, not `INI.big` |
| Fonts | `Data/<Language>/Language.ini` — likewise localized |
| CSF labels | `Data/<Language>/Generals.csf`, decodable today by `cic-formats`' CSF decoder |

Resource resolution therefore needs a localization mount alongside the `Wnd` profile. The language
is a **path component**, not an archive name; see [csf.md](csf.md) for the selection mechanism and
what adding a new language requires.

The two mapped-image directories are a **plain recursive merge, not a variant selection.** Their
names invite the opposite conclusion, so this was measured: across both editions the sets are
completely disjoint — `HandCreated/` defines 32 names in Generals and 41 in Zero Hour, and
`TextureSize_512/` defines 946 and 1,279, with **zero** appearing in both. No second size directory
ships either. Loading `Data/INI/MappedImages/**` recursively is therefore correct for retail data,
and a mod that did introduce a colliding name would resolve through ordinary VFS mount order. The
source-side loader that walks this directory has not been located, so the merge conclusion rests on
the data rather than on the code.

Coverage against a real installation, measured with the project's own CSF decoder: **349 of 366
referenced labels resolve; 17 do not.** Retail layouts genuinely reference labels the shipped CSF
does not define, so visible placeholders and stable diagnostics for missing labels are the ordinary
path, not an edge case.

**Retail ships no font files.** `Language.ini` names only host font families — `Arial`,
`Times New Roman`, `Courier New`, `Arial Unicode MS`, `FixedSys` — and its one `LocalFontFile` line
is commented out with the note that game fonts were never added. Since deterministic captures may
not use host fonts, a project-supplied substitute is the *default* capture path here, not an opt-in
fallback. `Language.ini` also fixes `ResolutionFontAdjustment = 0.7`, so font size grows at 70% of
the resolution increase — a presentation-policy input for the scaling gate.

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

## Reports and artifacts

- `cic-inspect wnd` (implemented) reports file/layout metadata, the complete `WINDOW`/`CHILD`
  hierarchy with decorated names and rectangles, every generically retained field in source order
  with its verbatim value, every typed record as `window_flag`/`window_callback`/
  `window_property`/`window_font`/`window_text_color`/`window_draw_entry`/`window_gadget_data`
  rows, and every non-fatal diagnostic.
- `cic-inspect wnd-render` (implemented, Gate 1 proof-of-pipeline only) stages every window
  rectangle as a flat colored quad in source order and writes a surface-free deterministic PNG
  capture plus an RGBA SHA-256 hash. It has no images, text, gadget visuals, or scaling policy; it
  exists to prove the immutable decoded value can drive a renderer capture, not to preview a real
  menu.
- `cic-inspect wnd-patch` (implemented) reports target/preconditions, operations, resulting hierarchy,
  per-field provenance, and stable incompatibility diagnostics without writing a patched retail WND.
- `cic-inspect ui-resources` (planned) reports resolved/missing images, fonts, labels, transitions,
  and provenance without embedding retail data.
- `cic-inspect ui-render` (planned) emits a deterministic synthetic PNG/hash for an explicit layout,
  viewport, scale policy, locale, font set, time, and input/state snapshot.
- `cic-inspect ui-demo` (planned) launches the interactive main-menu/skirmish compatibility harness.

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
