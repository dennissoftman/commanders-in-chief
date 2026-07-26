# WND Layout and R4 UI Compatibility Plan

- Status: Gates 1-3 (bounded decode, typed control definitions, patch overlays), Gate 4's definition
  resources, Gate 5 (retained runtime), and Gate 6 (custom `wgpu` presentation) implemented, each
  verified against every retail layout in both editions. Gate 4's transition/cursor/scheme subsets,
  per-family gadget draw-data composition, and Gates 7-11 not started
- Owning crates: `cic-formats` for syntax/immutable values, `cic-ui` for retained state,
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
| Mapped images | `Data/INI/MappedImages/TextureSize_512/` then `HandCreated/`, in `INI.big`/`INIZH.big`/`Patch.big` |
| Header templates | `Data/<Language>/HeaderTemplate.ini` — in the **localization** archive, not `INI.big` |
| Fonts | `Data/<Language>/Language.ini` — likewise localized |
| CSF labels | `Data/<Language>/Generals.csf`, decodable today by `cic-formats`' CSF decoder |

Resource resolution therefore needs a localization mount alongside the `Wnd` profile. The language
is a **path component**, not an archive name; see [csf.md](csf.md) for the selection mechanism and
what adding a new language requires.

### Mapped-image load order

The loader has since been located, and it corrects an earlier conclusion recorded here. The client's
mapped-image load is an **ordered three-stage load with an explicit texture-size selection**, not a
recursive merge of `Data/INI/MappedImages/**`:

1. `<UserData>/INI/MappedImages/*.ini`, when that directory exists;
2. `Data/INI/MappedImages/TextureSize_<N>` — one directory, chosen by the caller's `N`; and
3. `Data/INI/MappedImages/HandCreated`, last, so it overrides both.

`GameClient::init` calls the loader with a literal `512`, so `TextureSize_512` is the shipped
selection and any other `TextureSize_<N>` directory in the data is deliberately not loaded. Each
stage loads a directory's own files before its subdirectories' files, each group sorted by name; the
source comments that the sort keeps machines in step in a network game, so the order is a
determinism requirement rather than an implementation detail.

Order matters in retail data, which is the part the earlier note got wrong. Definitions are **not**
disjoint across the directories: applying the source order to a real installation records 23 name
overrides in Generals (12 within `TextureSize_512`, 11 from `HandCreated`) and 43 in Zero Hour (33
within `TextureSize_512`, 10 from `HandCreated`). Both editions ship a `HandCreatedMappedImages.ini`
in *both* directories. Two definitions are even duplicated inside a single file — `FairPlay` in
`SCShellUserInterface512.ini` and `SSRadarVanScan` in `SUUserInterface512.ini` — which the source
loader parses into the existing definition, so later fields win and omitted fields are inherited.
A merge-everything loader would resolve some of those names to the wrong texture region.

### Measured resource coverage

Applying the implemented resolution to every layout in a real installation, with the project's own
decoders:

| Resource | Zero Hour (80 layouts) | Generals (78 layouts) |
| --- | --- | --- |
| Mapped images | 1,849 resolved, 129 unresolved (50 distinct) | 1,789 resolved, 143 unresolved (48 distinct) |
| Header templates | 209 resolved, 0 unresolved | 196 resolved, 0 unresolved |
| Font families | 137 resolved, 43 unresolved (3 distinct) | 133 resolved, 41 unresolved (3 distinct) |
| CSF labels | 537 resolved, 28 unresolved (17 distinct) | 505 resolved, 21 unresolved (13 distinct) |

Three findings shape the runtime gates:

- **Retail layouts name images retail never defines.** Fifty distinct names in Zero Hour and 48 in
  Generals — the whole `ProgressBarDisabled*` family, `CheckBoxUseStats*`, `MarketingScreen`, and
  others — appear in no shipped mapped-image INI in either edition. Visible placeholders and stable
  diagnostics are the ordinary path, not an edge case.
- **Zero Hour's 17 unresolved labels reproduce exactly**, matching the independent count from the
  evidence pass and cross-checking the label classifier against the CSF decoder. Generals leaves 13
  unresolved against its smaller 2,806-label string table (Zero Hour ships 6,422).
- **Only three font families go unresolved** — `Generals`, `Abadi MT Bold`, and
  `Placard MT Condensed` — none declared by `Language.ini` and none shipped as a file.

`[None]` (spelled in retail as both `[None]` and `[NONE]`) is the writer's explicit "selects
nothing" placeholder for `HEADERTEMPLATE`, `FONT`, and text records, the same spelling the layout
block uses for an absent callback. `MainMenu.wnd` alone carries 31 `[NONE]` and two `[None]` header
templates. A placeholder is an absent demand, never a missing resource.

**Retail ships no font files.** `Language.ini` names only host font families — `Arial`,
`Times New Roman`, `Courier New`, `Arial Unicode MS`, `FixedSys` — and its one `LocalFontFile` line
is commented out with the note that game fonts were never added. Since deterministic captures may
not use host fonts, a project-supplied substitute is the *default* capture path here, not an opt-in
fallback. `Language.ini` also fixes `ResolutionFontAdjustment = 0.7`, so font size grows at 70% of
the resolution increase — a presentation-policy input for the scaling gate.

### Implemented UI definition grammar

The three narrow decoders share one bounded lexer (`crates/cic-formats/src/ui_ini.rs`) derived from
the source INI reader, because all three are read by the same one:

- a line ends at `\n`; a `;` terminates the line at that byte, so comment text is never tokenized;
  bytes below 32 become spaces, which makes `\r` insignificant;
- the default separator set is `" \n\r\t="`, so `=` is only a separator and `Field = value`,
  `Field=value`, and `Field value` are one record; `:` joins the set for `Left:12` sub-tokens; a
  quoted string is delimited by `"` and `=` alone;
- block keywords and field names are matched with `strcmp`, so both are **case-sensitive**, while the
  `End` terminator is matched with `stricmp` and is not;
- an unknown field inside a block is skipped by the release client; this project retains it as a
  diagnostic. Reaching end of file with a block open is a hard error in both.

Two source quirks are reproduced rather than corrected, because a definition authored against the
original reader has to resolve to the same value here: a quoted string is rejoined from at most two
tokens and a continuation of exactly one character is dropped (`"Synth 0"` reads as `Synth`), and an
*unquoted* multi-word value keeps only its first token (`UnicodeFontName = Arial Unicode MS` reads as
`Arial`). Both are covered by tests that assert the quirk.

| Format | Block | Fields |
| --- | --- | --- |
| `mapped_image_ini.rs` | `MappedImage <Name>` | `Texture`, `TextureWidth`, `TextureHeight`, `Coords`, `Status` |
| `header_template_ini.rs` | `HeaderTemplate <Name>` | `Font`, `Point`, `Bold` |
| `language_ini.rs` | `Language` (unnamed singleton) | 25 fields: 17 font roles plus `UnicodeFontName`, `LocalFontFile`, `MilitaryCaptionSpeed`, `MilitaryCaptionDelayMS`, `UseHardWordWrap`, `ResolutionFontAdjustment`, `ResolutionFontSizeMethod` |
| `window_transitions_ini.rs` | `WindowTransition <Name>` | `FireOnce` plus repeated `Window` sub-blocks of `WinName`, `Style`, `FrameDelay` |

Mapped-image lookup is case-insensitive because the source keys its collection through a lowercased
name key; header-template lookup is case-sensitive because the source compares names directly. Both
apply a duplicate definition to the existing entry rather than replacing it, so a field the second
declaration omits keeps the first declaration's value, and the duplicate is reported.

`Coords` and `Status` are order-dependent in the source and stay so here: the region rect describes
the *rotated* image, so `Status = ROTATED_90_CLOCKWISE` swaps the stored presentation size at the
point it is read. A `Status` line placed before its `Coords` line therefore swaps an empty size and
leaves the later region unswapped. `Status` accepts the `parseBitString32` shape — `NONE`, bare
names, or `+`/`-` operators, which may not be mixed with bare names.

`Language.ini` supplies every default from the source constructor when a field is absent:
`ResolutionFontAdjustment` 0.7, `MilitaryCaptionDelayMS` 750, and per-role
`Arial Unicode MS` at 12 points, not bold. `ResolutionFontSizeMethod` decodes all four policies the
pinned revision declares (`CLASSIC`, `CLASSIC_NO_CEILING`, `STRICT`, `BALANCED`) with that
revision's `CLASSIC_NO_CEILING` default; only `CLASSIC` is original behavior. `LocalFontFile` repeats
and the source pushes each name onto the *front* of its list, so the decoded order is reversed from
file order — the order fonts are actually registered in.

Default limits are 4 MiB per file, 100,000 lines, 4,096 bytes per line, 16,384 definitions per file,
255-byte names, 1,024-byte values, and 256 entries per repeated field. Resolution adds 4,096
definition files per directory tree and 65,536 catalog definitions.

### Window transition groups

`Data/INI/WindowTransitions.ini` names groups of windows to animate together. A group is a list of
`Window` sub-blocks, each naming one window by its **decorated** `<layout>:<control>` name, one style
from a fixed fifteen-entry vocabulary, and a frame delay. `FireOnce = Yes` marks a group that clears
itself after finishing; a group without it stays current, and the handler reverses it when another
group is set, which is how a menu's forward animation plays backwards on the way out.

The style vocabulary is `TransitionStyleNames`, and each style's length is its own `*TRANSITION_END`
constant, in frames of the source's fixed thirty-per-second transition clock:

| Style | Frames | Style | Frames | Style | Frames |
| --- | --- | --- | --- | --- | --- |
| `FLASH` | 8 | `TYPETEXT` | 30 | `MAINMENUMEDIUMSCALEUP` | 3 |
| `BUTTONFLASH` | 17 | `SCREENFADE` | 30 | `MAINMENUSMALLSCALEDOWN` | 6 |
| `WINFADE` | 10 | `COUNTUP` | 30 | `CONTROLBARARROW` | 22 |
| `WINSCALEUP` | 6 | `FULLFADE` | 10 | `SCORESCALEUP` | 6 |
| `MAINMENUSCALEUP` | 5 | `TEXTONFRAME` | 1 | `REVERSESOUND` | 2 |

`TYPETEXT` and `COUNTUP` shorten that length at creation from their window's own text rather than from
the definition — one frame per character, or per one, hundred, or thousand of the integer counted to —
so the table gives the declared maximum for them. `COUNTUP` runs no frames at all when its window
starts hidden.

Three lookup rules differ from the rest of the INI family, and each is reproduced. A style name is
compared case-insensitively, because `parseLookupList` reaches `scanIndexList`. A group name is
compared case-insensitively when a caller asks for it, because `findGroup` uses `compareNoCase`. A
window name is compared **case-sensitively**, because the handler turns it into a name key through
`nameToKey`, which compares with `strcmp` — the same key a WND `NAME` record produces.

A repeated group name behaves unlike every other UI definition family: `getNewGroup` refuses to
allocate a second group with an existing name and returns nothing, so the source parses the repeated
definition's fields into no group at all. The later definition is therefore dropped whole rather than
merged, and this decoder reports it.

#### Measured transition coverage

Both installed editions decode with **zero diagnostics**: Zero Hour declares 56 groups over 381
windows, base Generals 55 groups over 379. Fourteen of the fifteen styles are exercised;
`MAINMENUSMALLSCALEDOWN` is declared by no retail group in either edition. `FLASH` (117 windows) and
`BUTTONFLASH` (77) dominate, and `COUNTUP`'s 56 windows all belong to one group — the score screen.
Four further `TYPETEXT` windows are present but commented out, which the decoder correctly ignores.

`cic-inspect ui-transitions` reports the inventory, per-style census, and diagnostics; it defaults to
`Data/INI/WindowTransitions.ini` and accepts any other virtual path for a modded file.

## Custom `wgpu` presentation

Gate 6 is implemented in `crates/cic-render/src/ui.rs` (staging), `ui.wgsl` (the quad pipeline), and
`ui_text.rs` (Unicode shaping), driven through `HeadlessRenderer::capture_ui_frame`.

Staging turns a frame's ordered instructions into vertices, indices, and batches, breaking a batch
only when the bound texture page or the scissor rectangle changes, so submission keeps the frame's
order while still batching adjacent work. Nested clips intersect, so an inner region can never draw
outside an outer one, and a scissor is clamped into the attachment rather than trusted — a layout may
legitimately clip against a region partly off screen. Colour handling is explicit: pages upload in
the capture target's own space so a sampled byte reaches the attachment unchanged, because declaring
a page sRGB against a linear target linearizes on read without re-encoding on write and darkens every
image. Alpha is straight, not premultiplied, matching the source's stored channel bytes.

A border is drawn only for a control declaring `BORDER`. A border colour alone is inert, which
matters because most retail controls carry one: honouring colour alone outlines the entire menu.

**Text** uses `cosmic-text` 0.19 for Unicode shaping and `glyphon` 0.12 for `wgpu` glyph rendering,
the pair ADR 0010 selected. `glyphon` 0.12 declares `wgpu ^30.0.0`, which unifies with the workspace
`wgpu` 30 rather than pulling a second copy; both are permissively licensed (`cosmic-text` MIT or
Apache-2.0, `glyphon` MIT, Apache-2.0, or Zlib) and so compatible with this project's GPL-3.0-only
licence. Fonts are always supplied as bytes by the caller: nothing enumerates host fonts, because a
capture that silently picked up a platform face would hash differently on another machine. With no
font supplied, staging emits a visible placeholder bar per run and a diagnostic instead of silently
dropping the text. A secret entry field renders one mask glyph per character, never its contents.

Verified against a real installation at 1280x720 with a user-owned font: `MainMenu.wnd` stages 37
quads in 12 batches over three texture pages with 29 shaped runs, `OptionsMenu.wnd` 41 quads and 25
runs, `SkirmishGameOptionsMenu.wnd` 52 quads and 21 runs. Repeated runs produce byte-identical
hashes. Localized labels resolve through the CSF decoder before staging, so the capture shows real
menu text.

### Gadget draw-data composition

A draw-data record holds nine entries, and each gadget family composes them from its own indices.
`GadgetPushButton.h` fixes the push-button map: the unselected art is left 0, middle 5, right 6, and
the pushed art is left 1, middle 3, right 4. `W3DGadgetPushButtonImageDraw` takes the three-piece
path only when the middle image is present and otherwise stretches the entry-0 background, which is
reproduced exactly: the centre repeats in whole pieces from the left end's right edge, a final partial
piece covers the remainder, and the two ends draw last so they sit over the centre. The source's
`centerWidth <= 0` branch — ends alone not fitting, so each takes half the control — is reproduced
too. The source clips that final partial piece with a clip region; trimming its texture coordinates
samples the same pixels without a state change and keeps the batch intact.

Button text is centred on both axes, as `drawButtonText` does, through the shaper's own alignment
horizontally and a measured offset vertically. Static text centres only when its own `CENTERED` flag
is set.

One colour rule came out of rendering real data: **an image draw is untinted**. `winDrawImage` takes
no colour, and a slot's `COLOR` belongs to the colour-only fill path, so multiplying an image by it
paints every textured control in whatever that unused field happens to hold — which in retail data is
frequently red.

Every other established family composes too, each from its own `Gadget*.h` index map:

| Family | Entries the source reads |
| --- | --- |
| `PUSHBUTTON` | left 0, middle 5, right 6; pushed 1, 3, 4 |
| `RADIOBUTTON` | left 0, unchecked 1, checked 2 per slot; selected reads hilite 3, 4, 5 |
| `CHECKBOX` | unchecked box 1, checked box 2 |
| `ENTRYFIELD` | left 0, right 1, centre 2, small centre 3 |
| `VERTSLIDER` | top 0, bottom 1, centre 2, small centre 3 |
| `HORZSLIDER` | fill and blank from disabled 0 and 1, highlight from hilite 0 |
| `PROGRESSBAR` | background left 0, right 1, centre 2; bar right 5, centre 6 |
| `TABCONTROL` | background 0, tabs 1 through 8 |
| everything else | entry 0, stretched across the control |

Two of those read a slot the control's own state did not select, which is why a frame carries all
three: a selected radio button reads the hilite slot even while enabled, because the source tests
`WIN_STATE_SELECTED` before the enabled bit, and a horizontal slider always takes its fill and blank
squares from the disabled slot and its highlight from the hilite slot. That slider also sizes its
squares against a fixed 800-pixel display reference rather than against the control, so they track
the display.

Three more source behaviours are reproduced rather than tidied. A check box draws no background at
all — the source leaves that draw commented out — and only its box, three pixels down and six shorter
than the control. A text entry and a vertical slider each draw one small-centre piece more than fits,
deliberately overrunning under the end piece that covers it. A progress bar fills the unreached part
of its track with the bar's *right* piece rather than leaving it empty, with the whole bar inset ten
pixels horizontally and five vertically.

Text placement follows the same files. Push-button and radio-button labels are centred on both axes,
as `drawButtonText` and `drawRadioButtonText` do, through the shaper's own alignment horizontally and
a measured offset vertically. A check box's label is *not*: `drawCheckBoxText` centres it vertically
but starts it one control-height in from the left, clearing the box. Static text centres only when
its own `CENTERED` flag is set.

The image path itself is chosen the way the source chooses it, in two steps: creating a gadget
assigns a default procedure from the `IMAGE` status bit, and a `DRAWCALLBACK` the function lexicon
resolves then replaces it. So a name that reads as a bound draw procedure decides — an `...ImageDraw`
variant against a plain `...Draw` — and anything else, including the overwhelmingly common
`"[None]"`, leaves the status bit deciding.

Where a family's own indices declare nothing, each source procedure returns early and draws nothing.
That is reproduced, with an `UncomposedArt` diagnostic naming the family: a placeholder there would
invent a control retail never shows. A placeholder still stands in for a mapped image the layout
names but the catalog cannot resolve, which is a different failure.

A tab control carries one fidelity note, untestable against retail because no retail layout declares
one: `parseTabControlData` reads `TABWIDTH` and `TABHEIGHT` straight from the file and nothing scales
them, so a tab strip does not follow the creation-resolution scaling its own control does.

## Retained UI behavior

The parser returns definitions; `cic-ui` creates retained instances. That crate is implemented for
layout, hit testing, focus, control invariants, and renderer-neutral frames; tooltips, cursors, and
transition sampling wait for the later gates that own them.

### Implemented layout and scaling

`parseScreenRect` establishes the exact classic policy, reproduced in `UiScalePolicy::Classic`: each
stored corner is scaled by `viewport / creation_resolution` **per axis** and truncated toward zero,
size is derived from the scaled corners, and a child's position is then made relative to its
parent's already-scaled screen position. Non-uniform viewports therefore stretch — an 800x600 layout
on 1600x900 scales 2.0 horizontally and 1.5 vertically, so a 100x40 button becomes 200x60 rather
than keeping its authored aspect ratio. `screen_position` sums every ancestor's parent-relative
origin, so nested rectangles compose the way the original composes them.

`UiScalePolicy::Modern` is project design, not source behavior: it applies the smaller axis ratio to
both axes and centres the result, letterboxing the authored composition instead of stretching it.

### Implemented hit testing

`getWindowUnderCursor` establishes a layered search that is reproduced exactly. Top-level windows
are searched in three passes — `ABOVE` first, then windows with none of `ABOVE`/`BELOW`/`HIDDEN`,
then `BELOW` — and the first whose region contains the point descends through `winPointInChild`,
which returns the first visible, enabled child containing the point and recurses into it. A hidden or
disabled child that contains the point is skipped and iteration continues with the next child, so a
click falls through to the parent rather than being swallowed. A control declaring `NO_INPUT` discards
the result. Every edge test is inclusive on both ends (`x >= left && x <= left + width`), which
decides which of two adjacent controls a boundary click reaches. While a control holds the mouse the
search is confined to it.

Both walks run in **reverse file order**, which is a consequence of how the window manager stores
windows rather than of the search itself. `winCreate` links every new window at the head of its list
— the top-level list through `linkWindow`, a child list through `addWindowToParent`, whose
append-at-end variant is commented out — so a layout's lists are the reverse of its file order, and
`winBringToTop` pulls from the layout tail specifically to preserve that. `getWindowUnderCursor` and
`winPointInChild` then walk from the head, so the last window in the file is tested first. That is
also the front-most window: `winRepaint` draws from the tail backwards, which puts the last window in
the file on top. Only overlapping siblings can tell the difference, which is why this went unnoticed
until the shell needed one search spanning several layouts.

When more than one layout is up, the three passes belong to the shell rather than to a layout, because
the original runs them over the window manager's single global list. `UiShell::hit_test` therefore
walks its own draw order front to back inside each pass, delegating the descent to the owning layout.

### Implemented focus and tab traversal

`winSetFocus` establishes the refusal rules: a control declaring `NOFOCUS` refuses focus outright,
and otherwise the request walks up parents until one accepts, with focus becoming absent when none
does. Reproduced.

Tab traversal needed a source finding. `GameWindow::winNextTab` and `winPrevTab` are **entirely
commented out** at the pinned revision and return success without moving focus; the live mechanism is
the window manager's own tab list, which cycles with wraparound and is inert while a modal is up.
`cic-ui` reproduces the manager's cycle and derives the list from the declared `TABSTOP` status bit
in source order, skipping stops that are disabled or hidden so focus cannot be trapped.

Measured against real data, that list is nearly empty: the whole Zero Hour corpus declares **nine**
`TABSTOP` controls across 80 layouts. Keyboard traversal of a retail menu cannot come from the
layouts, so the shell gate will need project-owned tab order.

### Implemented control invariants

Radio buttons are exclusive within their declared `GROUP` across the owning window's peers, matching
`GadgetRadioButtonSetSelection`. Sliders clamp into their declared `MINVALUE`/`MAXVALUE`, and an
inverted pair is ordered with a diagnostic rather than accepted. List and combo selection refuse an
index outside the current row set instead of clamping, so a stale index cannot silently select a
different row; single-select lists replace rather than accumulate. List scrolling clamps so the last
page stays full. Text entry counts **characters, not bytes**, against its declared `MAXLEN`, so a
Unicode field holds what its definition promises, and caret motion and deletion are character-wise.
Progress bars clamp to `0..=100`. Hiding or disabling a control clears its hover, press, focus, and
capture state through its whole subtree, because a hidden window takes no input.

### Implemented frames

A frame is an ordered list of renderer-neutral instructions: optional clip push/pop, one quad per
visible control carrying the mapped-image name, fill, and border colour of the draw-data slot its
current state selects, and one text run carrying the `TEXT` value, font, state colour, and a mask
flag for secret entries. Submission order inverts the hit-test layering — `BELOW`, then unlayered,
then `ABOVE` — with each subtree emitted parent before children in source order, so a child draws
over its parent. Disabled wins over hover when selecting the slot. A control declaring `SEE_THRU`
emits no quad but still emits its children. Clipping is an explicit policy: the default matches the
original, which does not clip and which retail layouts rely on, and `ClipToParent` is available for
callers that want a masked region.

Default limits are 4,096 controls, 64 levels of nesting, 1,024 characters in an entry field whose
definition declares no limit, and 4,096 list rows.

### Measured retained-layout coverage

Every one of the 80 Zero Hour and 78 Generals layouts instantiates at 800x600, 1920x1080, and 21:9
2560x1080 under both scale policies — 480 instantiations — with no failures and zero diagnostics. The
Zero Hour corpus yields 1,667 retained controls, matching the WND census's window count, distributed
as 539 static text, 424 push buttons, 411 without gadget state, 115 combo boxes, 45 check boxes, 39
list boxes, 34 progress bars, 32 entry fields, 19 radio buttons, and nine sliders. Mapping the
complete `WindowStatusNames` vocabulary was what closed the last diagnostic: `HOTKEY_TEXT` occurred
once corpus-wide.

Control state changes produce typed UI events. Callback fields are looked up only in an application
allowlist. Unknown callback names are inert. Layout update names do not create a general scripting
language, and MAP scripts are never dispatched by the UI runtime.

### Implemented safe callbacks

The original resolves an authored callback name exactly once, at creation, through
`FunctionLexicon`: nine fixed tables of `{name, function}` pairs, keyed by `nameToKey`, which compares
with `strcmp`. A name absent from the searched table yields a null pointer that is simply never
called. That mechanism *is* the allowlist this gate needs, so `cic-ui` carries the same nine tables as
data and classifies a retained name as `established`, the explicit `[None]` placeholder, or `unknown`
— the last two being inert here exactly as they were inert there.

Two of the seven WND callback records search every table rather than their own, because
`gameWinDrawFunc` and `winLayoutInitFunc` default to `TABLE_ANY`. That is the only reason a control's
`W3DGadgetPushButtonImageDraw` or a layout's `W3DMainMenuInit` resolves at all: both live in device
tables — `TABLE_GAME_WIN_DEVICEDRAW` and `TABLE_WIN_LAYOUT_DEVICEINIT` — that the pinned accessors
never look in. An every-table search walks tables in `TableIndex` order and takes the first match, so a
name that appears in two tables resolves to the earlier one. Every other record is pinned to one
table, which means the same spelling in the wrong record resolves to nothing.

The two editions compile separate copies of these tables and Zero Hour's are a strict superset: it
adds `ChallengeMenuInit`, `ChallengeMenuInput`, `ChallengeMenuShutdown`, `ChallengeMenuSystem`,
`ChallengeMenuUpdate`, and `PopupHostGameUpdate`. Device tables are identical. No retail Generals
layout names any of the six, so both corpora classify identically today; classification is still
edition-parameterized, because a modded layout need not be so tidy.

One asymmetry in the source's own parsing survives here. A **window** callback is read by scanning to
the first `"` and taking what is inside, so `SYSTEMCALLBACK = "MainMenuSystem"` yields
`MainMenuSystem`. A **layout** callback is read with `strtok(buffer, " =")` and never has a quote
stripped, so a quoted `LAYOUTINIT` would produce a name no table carries. Retail writes window
callbacks quoted and layout callbacks bare, so both resolve; a modded file that quotes a layout
callback is inert in the original as much as here, and the retained value keeps its quotes to say so.

Routing a callback to an action is separate and project-owned. `UiActionAllowlist` maps an authored
control name — decorated for one layout, undecorated to cover every menu's Back — to a
`UiDemoAction`: push or pop a screen, show or hide a control, set a transition group, or leave the
demo. A control absent from the allowlist routes nothing however established its callback name is,
which is what keeps a presentation-only milestone from starting a game.

#### Measured callback coverage

Across both installed corpora, **6,908 retained names in Zero Hour's 80 layouts and 6,350 in
Generals' 78** classify with **six and five unknowns respectively**, and every one of those is
retail's own gap rather than a decoding failure: `MarketingScreenInit`, `MarketingScreenUpdate`,
`MarketingScreenShutdown`, `SinglePlayerLoadScreenShutdown` (twice), and, in Zero Hour only,
`ChallengeLoadScreenShutdown`. All six are layout-level names the shipped client's lexicon never
registers, so those screens' init, update, or shutdown did nothing in the original either. The Zero
Hour corpus exercises 223 distinct record-and-name pairs, Generals 217.

`cic-inspect ui-callbacks <layout>` reports every name with its slot, binding, and resolving table.

### Implemented shell stack

`Shell` is a sixteen-screen pseudo-stack over window layouts, reproduced in `UiShell`. Its push and
pop are two-phase, and the reason is animation: pushing records a *pending push*, runs the current
top's shutdown, and links the new screen only when that shutdown reports back through
`shutdownComplete`, which lets a layout animate itself away first. Popping is the same shape with a
*pending pop*. `popImmediate` deliberately leaves the pending flag clear, tells the shutdown an
immediate pop is coming, and unlinks as soon as it returns. A push over an empty or already-hidden top
short-circuits straight to `shutdownComplete`, so it completes in one call.

Since this project never calls a resolved function, the protocol is exposed rather than hidden: a push
returns a typed shutdown event carrying the retained callback name, and the caller calls
`shutdown_complete` when it considers that shutdown finished. A deterministic capture therefore steps
the whole sequence explicitly, with no clock involved.

Three further behaviours are reproduced. `Shell::hide` walks the entire stack rather than only the
top, and each layout's own `hide` walks the file's top-level windows — the set
`winCreateFromScript` puts in the layout, in file order — with children following because a hidden
parent hides its subtree. `Shell::update` runs every screen's update whether or not it is on top or
visible, starting at the top index and counting down. And the stack is not the draw order: the stack
is navigation history, while z-order lives in the window manager's list, which `bringForward`
reorders. `UiShell` keeps a separate draw order for exactly that reason, so a screen can be on top of
the stack while another screen draws over it. `doPush` brings the pushed screen forward; `doPop` does
not, because the source's call there is commented out.

`cic-inspect ui-shell --step ...` drives an explicit script — `push:<path>`, `pop`, `pop-immediate`,
`complete`, `complete-for-push`, `update`, `hide`, `show`, `show-shell`, `hide-shell`,
`forward:<index>` — and reports each step's events plus the resulting stack, draw order, and
visibility. Against the installed Zero Hour layouts, Main Menu → Options → back → Skirmish Options →
back runs with every layout callback resolving as established, and the report is byte-identical
across runs.

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
- `cic-inspect ui-resources` (implemented) reports the selected language and its font policy, every
  definition file that loaded with its definition count, every definition file the active texture-size
  selection skipped, every name a later file overrode, all 17 font roles, every declared font file,
  per-kind resolved/unresolved counts, and one row per demanded resource with each site that named it
  plus its binding — defining file, texture file and resolved texture path, header-template font,
  point and weight, or the matched font role. Rows carry names, virtual paths, and counts only; no
  retail definition content is embedded. Transitions and cursors are not covered yet.
- `cic-inspect ui-layout` (implemented) instantiates a layout for an explicit viewport and scale
  policy and reports the retained tree with resolved and screen-space rectangles, live state, status
  bits, control family, tab order, frame submission order, and diagnostics. Nothing is read from the
  host display, so the report is reproducible on any machine. Its `role` column names a control the
  gadget-creation code built rather than the file — a slider thumb, a scroll bar's parts, or a combo
  box's field, button, and drop-down list — and those rows carry a `<part>` suffix on the owner's
  name, which is this project's addition since the original leaves them unnamed.
- `cic-inspect ui-render` (implemented) executes a retained frame on the GPU and writes a
  surface-free PNG plus an RGBA SHA-256 hash. Every presentation input is explicit — viewport, scale
  policy, clip policy, language, texture-size selection, and the font files used for shaping — so two
  runs with the same inputs produce the same bytes. It reports staged quad, batch, text-run, page, and
  font counts plus every staging diagnostic. A diagnostic names both the frame-item index and the
  control it belongs to, so it can be looked up in the `ui-layout` report.
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
