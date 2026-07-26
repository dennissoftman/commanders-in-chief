# WND and UI Provenance

## GeneralsGameCode evidence

- Repository: <https://github.com/TheSuperHackers/GeneralsGameCode>
- Revision: `9f7abb866f5afd446db14149979e744c7216baaf`
- WND parser and immutable-field evidence:
  - `Generals/Code/GameEngine/Source/GameClient/GUI/GameWindowManagerScript.cpp`
  - `Core/GameEngine/Include/GameClient/GameWindow.h`
  - `Core/GameEngine/Include/GameClient/GameWindowGlobal.h`
  - `Core/GameEngine/Include/GameClient/WindowLayout.h`
  - `Core/GameEngine/Source/GameClient/GUI/WindowLayout.cpp`
- Per-family draw-data composition, one header and one device file each:
  - `Core/GameEngineDevice/Source/W3DDevice/GameClient/GUI/Gadget/W3DRadioButton.cpp`
  - `Core/GameEngineDevice/Source/W3DDevice/GameClient/GUI/Gadget/W3DCheckBox.cpp`
  - `Core/GameEngineDevice/Source/W3DDevice/GameClient/GUI/Gadget/W3DTextEntry.cpp`
  - `Core/GameEngineDevice/Source/W3DDevice/GameClient/GUI/Gadget/W3DVerticalSlider.cpp`
  - `Core/GameEngineDevice/Source/W3DDevice/GameClient/GUI/Gadget/W3DHorizontalSlider.cpp`
  - `Core/GameEngineDevice/Source/W3DDevice/GameClient/GUI/Gadget/W3DProgressBar.cpp`
  - `Core/GameEngineDevice/Source/W3DDevice/GameClient/GUI/Gadget/W3DTabControl.cpp`
  - `Core/GameEngineDevice/Source/W3DDevice/GameClient/GUI/Gadget/W3DListBox.cpp`
  - `Core/GameEngineDevice/Source/W3DDevice/GameClient/GUI/Gadget/W3DComboBox.cpp`
  - `Core/GameEngineDevice/Source/W3DDevice/GameClient/GUI/Gadget/W3DStaticText.cpp`
  - `Core/GameEngine/Source/GameClient/GUI/Gadget/GadgetTabControl.cpp`
  - `GeneralsMD/Code/GameEngineDevice/Source/W3DDevice/GameClient/GUI/W3DGameWindow.cpp`
- Gadget declarations and behavior:
  - `Core/GameEngine/Include/GameClient/Gadget.h`
  - `Core/GameEngine/Include/GameClient/GadgetPushButton.h`
  - `Core/GameEngine/Include/GameClient/GadgetRadioButton.h`
  - `Core/GameEngine/Include/GameClient/GadgetCheckBox.h`
  - `Core/GameEngine/Include/GameClient/GadgetSlider.h`
  - `Core/GameEngine/Include/GameClient/GadgetListBox.h`
  - `Core/GameEngine/Include/GameClient/GadgetComboBox.h`
  - `Core/GameEngine/Include/GameClient/GadgetTextEntry.h`
  - `Core/GameEngine/Include/GameClient/GadgetStaticText.h`
  - `Core/GameEngine/Include/GameClient/GadgetProgressBar.h`
  - `Core/GameEngine/Include/GameClient/GadgetTabControl.h`
  - `Core/GameEngine/Source/GameClient/GUI/Gadget/`
- Retained runtime evidence:
  - `Core/GameEngine/Source/GameClient/GUI/GameWindow.cpp`
  - `GeneralsMD/Code/GameEngine/Source/GameClient/GUI/GameWindowManager.cpp`
  - `GeneralsMD/Code/GameEngine/Source/GameClient/GUI/GameWindowManagerScript.cpp`
  - `GeneralsMD/Code/GameEngine/Include/GameClient/GameWindowManager.h`
- Legacy rendering evidence:
  - `Core/GameEngineDevice/Source/W3DDevice/GameClient/GUI/Gadget/W3DPushButton.cpp`
  - `Core/GameEngine/Include/GameClient/GadgetPushButton.h`
  - `Generals/Code/GameEngineDevice/Include/W3DDevice/GameClient/W3DGameWindowManager.h`
  - `Generals/Code/GameEngineDevice/Source/W3DDevice/GameClient/GUI/W3DGameWindowManager.cpp`
  - `Core/GameEngineDevice/Include/W3DDevice/GameClient/W3DGadget.h`
  - `Core/GameEngineDevice/Source/W3DDevice/GameClient/GUI/Gadget/`
- Mapped image, font, localization, and language evidence:
  - `Core/GameEngine/Source/Common/INI/INIMappedImage.cpp`
  - `Core/GameEngine/Source/GameClient/System/Image.cpp`
  - `Core/GameEngine/Include/GameClient/Image.h`
  - `Core/GameEngine/Source/Common/INI/INI.cpp`
  - `Core/GameEngine/Include/Common/INI.h`
  - `Core/GameEngine/Source/GameClient/GUI/HeaderTemplate.cpp`
  - `Core/GameEngine/Include/GameClient/HeaderTemplate.h`
  - `Core/GameEngine/Include/GameClient/GameFont.h`
  - `Core/GameEngine/Source/GameClient/GUI/GameFont.cpp`
  - `Generals/Code/GameEngine/Include/GameClient/FontDesc.h`
  - `Core/GameEngine/Include/GameClient/GlobalLanguage.h`
  - `Core/GameEngine/Source/GameClient/GlobalLanguage.cpp`
  - `GeneralsMD/Code/GameEngine/Source/GameClient/GameClient.cpp` (`TheMappedImageCollection->load( 512 )`)
- Callback-name and shell/navigation evidence. The lexicon and shell are compiled per edition; the
  Zero Hour copies are the ones read, and the two lexicons differ only in that Zero Hour registers
  six further names:
  - `GeneralsMD/Code/GameEngine/Include/Common/FunctionLexicon.h` (`TableIndex`, the default table of
    each typed accessor)
  - `GeneralsMD/Code/GameEngine/Source/Common/System/FunctionLexicon.cpp` (all seven base tables,
    `loadTable`, `keyToFunc`, `findFunction`, `init`)
  - `Generals/Code/GameEngine/Source/Common/System/FunctionLexicon.cpp` (the base-game tables, for
    the edition difference)
  - `GeneralsMD/Code/GameEngineDevice/Source/W3DDevice/Common/System/W3DFunctionLexicon.cpp` (the
    device draw and device layout-init tables, `W3DFunctionLexicon::init`)
  - `GeneralsMD/Code/GameEngine/Source/Common/NameKeyGenerator.cpp` (`nameToKey`, which compares with
    `strcmp`, so a callback lookup is case-sensitive)
  - `GeneralsMD/Code/GameEngine/Include/GameClient/Shell.h` (the documented push and pop sequences,
    `MAX_SHELL_STACK`)
  - `GeneralsMD/Code/GameEngine/Source/GameClient/GUI/Shell/Shell.cpp` (`push`, `pop`, `popImmediate`,
    `doPush`, `doPop`, `shutdownComplete`, `showShell`, `hideShell`, `hide`, `update`, `top`,
    `linkScreen`, `unlinkScreen`, `findScreenByFilename`, `getScreenLayout`)
  - `Core/GameEngine/Include/GameClient/WindowLayout.h`
  - `Core/GameEngine/Source/GameClient/GUI/WindowLayout.cpp` (`hide`, `bringForward`, `load`,
    `addWindow`, `removeWindow`)
  - `Core/GameEngine/Source/GameClient/GUI/GameWindow.cpp` (`winBringToTop`, `winHide`,
    `winPointInChild`)
  - `GeneralsMD/Code/GameEngine/Source/GameClient/GUI/GameWindowManager.cpp` (`linkWindow`,
    `addWindowToParent`, `addWindowToParentAtEnd`, `windowHiding`, `winRepaint`,
    `getWindowUnderCursor`)
  - `GeneralsMD/Code/GameEngine/Source/GameClient/GUI/GameWindowManagerScript.cpp`
    (`parseSystemCallback`, `parseInputCallback`, `parseTooltipCallback`, `parseDrawCallback`,
    `parseInit`, `parseUpdate`, `parseShutdown`, `parseLayoutBlock`, whose differing quote handling
    is why a layout callback keeps its quotes and a window callback does not)
  - `Core/GameEngine/Include/GameClient/GameWindowTransitions.h` (`TransitionStyleNames`, every
    `*TRANSITION_END` constant, `TransitionWindow`, `TransitionGroup`)
  - `Core/GameEngine/Source/GameClient/GUI/GameWindowTransitions.cpp`
    (`INI::parseWindowTransitions`, the group parse table, `parseWindow`, `getNewGroup`, `findGroup`,
    `TransitionWindow::getTotalFrames`)
  - `GeneralsMD/Code/GameEngine/Source/GameClient/GUI/GameWindowTransitionsStyles.cpp` (each
    transition's constructor and `init`, which set `m_frameLength`)
- Main-menu and R3 map-selection integration evidence:
  - `Generals/Code/GameEngine/Source/GameClient/GUI/GUICallbacks/Menus/MainMenu.cpp`
  - `Generals/Code/GameEngine/Source/GameClient/GUI/GUICallbacks/Menus/SkirmishGameOptionsMenu.cpp`
  - `Generals/Code/GameEngine/Source/GameClient/GUI/GUICallbacks/Menus/SkirmishMapSelectMenu.cpp`
  - `Generals/Code/GameEngine/Source/GameClient/GUI/GUICallbacks/Menus/MapSelectMenu.cpp`
  - `Core/GameEngine/Source/GameClient/MapUtil.cpp`
- Language selection and archive discovery:
  - `GeneralsMD/Code/Main/WinMain.cpp` (`g_csfFile = "data\%s\Generals.csf"`)
  - `Core/GameEngine/Source/GameClient/GameText.cpp`
  - `Core/GameEngineDevice/Source/Win32Device/Common/Win32BIGFileSystem.cpp`
- Established Options/display boundary:
  - `Generals/Code/GameEngine/Source/GameClient/GUI/GUICallbacks/Menus/OptionsMenu.cpp`
  - `Core/GameEngine/Include/GameClient/Display.h`
  - `Core/GameEngine/Include/Common/OptionPreferences.h`
  - `Core/GameEngine/Source/Common/OptionPreferences.cpp`

## Permanent links

- <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Generals/Code/GameEngine/Source/GameClient/GUI/GameWindowManagerScript.cpp>
- <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/GeneralsMD/Code/GameEngine/Source/GameClient/GUI/GameWindowManagerScript.cpp>
- <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngine/Include/GameClient/GameWindow.h>
- <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngine/Include/GameClient/WindowLayout.h>
- <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngine/Source/GameClient/GUI/WindowLayout.cpp>
- <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngine/Include/GameClient/Gadget.h>
- <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngine/Source/Common/INI/INIMappedImage.cpp>
- <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngine/Include/GameClient/GlobalLanguage.h>
- <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Generals/Code/GameEngine/Source/GameClient/GUI/Shell/Shell.cpp>
- <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Generals/Code/GameEngine/Source/GameClient/GUI/GUICallbacks/Menus/MainMenu.cpp>
- <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Generals/Code/GameEngine/Source/GameClient/GUI/GUICallbacks/Menus/SkirmishGameOptionsMenu.cpp>
- <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Generals/Code/GameEngine/Source/GameClient/GUI/GUICallbacks/Menus/SkirmishMapSelectMenu.cpp>
- <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Generals/Code/GameEngine/Source/GameClient/GUI/GUICallbacks/Menus/OptionsMenu.cpp>
- <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngine/Include/GameClient/Display.h>
- <https://github.com/TheSuperHackers/GeneralsGameCode/blob/9f7abb866f5afd446db14149979e744c7216baaf/Core/GameEngine/Source/Common/OptionPreferences.cpp>

- Upstream notice: Command & Conquer Generals Zero Hour; Copyright 2025 Electronic Arts Inc.;
  historical notices identify Electronic Arts Inc. and the named original authors.
- License: GNU GPL version 3 or later with the Electronic Arts Section 7 additional terms in the
  upstream repository's `LICENSE.md`.

## Established facts used by the R4 design

The pinned WND reader establishes the leading numeric file version; version-2 layout block with
named init/update/shutdown functions; nested `WINDOW`, `CHILD`, `ENDALLCHILDREN`, and `END` structure;
required `WINDOWTYPE` and `SCREENRECT`/creation-resolution records; source-order child creation;
status/style vocabularies; colors, fonts, text/tooltip labels, draw records, image offsets, and
gadget-specific data; and named system/input/tooltip/draw callbacks. It also demonstrates that
unknown fields were skipped by the legacy reader, which the project replaces with bounded
preservation/diagnostics.

The window/gadget sources establish retained parent/child controls, visibility/enablement, focus,
input, text, images and classic gadget families. Shell sources establish a layout stack with
push/pop/hide/bring-forward behavior and named transition groups. Main-menu sources establish
`Menus/MainMenu.wnd` and navigation to `Menus/SkirmishGameOptionsMenu.wnd`; skirmish sources establish
map selection, map preview windows, player/AI/color/faction/team controls, and map-start-position
buttons. `MapUtil.cpp` establishes one-based `Player_n_Start` waypoint discovery used by R3.

`OptionsMenu.cpp` establishes `OptionsMenu.wnd:ComboBoxResolution`, population from the display-mode
catalog, display apply/confirmation behavior, and persistence of a width/height resolution string.
`Display.h` establishes mode enumeration/description as width, height, and bit depth plus a
windowed/fullscreen flag. No refresh-rate field is present in that established interface. ADR 0010's
monitor, borderless/exclusive distinction, refresh-millihertz catalog, UI scale, transactional host
adapter, and declarative WND patch format are original project design rather than source behavior.

These are format and interaction facts only. The project does not copy the legacy native callback
registry, Direct3D UI renderer, global singleton ownership, menu implementation, or gameplay launch
logic. The custom retained model, safe typed action router, `wgpu` primitives, text integration,
deterministic tests, placeholders, and demo-only bindings are original project design.

## Evaluated Rust UI/text projects

- egui: <https://github.com/emilk/egui> — documents an immediate-mode GUI and `wgpu` backend.
- iced: <https://github.com/iced-rs/iced> — documents an Elm-inspired state/view/update model and
  `wgpu` renderer.
- glyphon: <https://github.com/grovesNL/glyphon> — documents 2D text shaping/atlas/rendering for
  `wgpu`, using `cosmic-text`.
- cosmic-text: <https://github.com/pop-os/cosmic-text> — documents Unicode shaping, layout,
  fallback, editing, and rasterization in Rust.

These projects informed ADR 0010's architecture choice. No dependency or source is incorporated by
the design-only change. Implementation must review the selected versions, licenses, notices, and
`wgpu` compatibility before updating Cargo manifests.

## Implementation record

The `CHILD` keyword's real role was established by reading `parseChildWindows` in
`GameWindowManagerScript.cpp` at the pinned revision: its loop compares against
`ENDALLCHILDREN`, `END`, the five default-color keywords, and `WINDOW`, with no `CHILD` case and no
fallback branch, so `CHILD` is an inert marker and a bare `WINDOW` opens the next sibling. The
Zero Hour status vocabulary was established from `WindowStatusNames` on the `GeneralsMD` source
path, which appends `ON_MOUSE_DOWN` after `ALWAYS_COLOR`; the `Generals` path's 25-name list is a
prefix of it. Both were checked against a structural census of the 80 retail `.wnd` layouts
reachable through the `Wnd` resource profile in a local installation. That census recorded
aggregate counts only — keyword and field-name frequencies, hierarchy extremes, and vocabulary
coverage. No retail file content, geometry, string, or asset was retained in the repository.

The window field vocabulary was established from `parseWindow`'s field chain at the pinned
revision, which compares against 46 keywords, and from the record helpers it dispatches to
(`parseFont`, `parseTextColor`, `parseTooltipDelay`, `parseHeaderTemplate`, `parseText`,
`parseTooltipText`, and the four callback parsers) for each record's expected sub-label and value
sequence. That list was cross-checked against the census: the three keywords with no retail
occurrences (`TABCONTROLDATA`, `IMAGEOFFSET`, `TOOLTIP`) plus the 43 observed field names account
for all 46 exactly, and applying the derived shapes to every retail layout types every occurrence
with no malformed-field diagnostics.

The remaining record grammars — `parseDrawData`, `parseListboxData`, `parseComboBoxData`,
`parseSliderData`, `parseRadioButtonData`, `parseTextEntryData`, `parseStaticTextData`,
`parseTabControlData`, and `parseImageOffset` — were read verbatim from the same file, retrieved
through the GitHub contents API at the pinned revision rather than through any summarizing
intermediary. That direct reading corrected an error an earlier summarized reading had introduced:
the claim that `parseWindow` returns immediately after `parseChildWindows` without consuming its
own `END`. There is no such early return, which is why the hierarchy model holds for the 31 retail
layouts containing a child-bearing child followed by a later sibling.

`MAX_DRAW_DATA` is pinned at nine through `Gadget.h`'s `NUM_TAB_PANES = 8, //(MAX_DRAW_DATA - 1)`,
matching the nine entries in all 7,875 retail draw-data records. `TOOLTIP` is left untyped because
`parseTooltip` ignores its buffer and stores a placeholder marked `@todo`, so no grammar exists to
derive. `TABCONTROLDATA` and `IMAGEOFFSET` are typed from source alone and recorded as such, since
neither occurs in retail data to cross-check.

The upstream repository contains no `.wnd` files — it is a source release without game assets — so
record shapes can be established from the parser sources and validated against a user-owned
installation, but there are no upstream reference layouts to compare against.

`crates/cic-formats/src/wnd.rs` implements Gate 1 (bounded WND inventory/hierarchy decode):
`FILE_VERSION`, the `STARTLAYOUTBLOCK`/`ENDLAYOUTBLOCK` layout block, the `WINDOW`/`CHILD`/`END`/
`ENDALLCHILDREN` hierarchy with `WINDOWTYPE`/`SCREENRECT` typed, and every other field retained
generically. Its module doc comment cites `winCreateFromScript` and `parseLayoutBlock` in
`GameWindowManagerScript.cpp` at the pinned revision above. The exact lexical grammar facts
(no comments, `;` as a hard terminator, case-sensitive structural keywords versus case-insensitive
status/style names, double-quote strings with no escapes, decimal-only numbers, the layout block's
`FILE_VERSION >= 2` gating and `"[None]"` version-1 default, and the independent top-level
color/font keywords) were confirmed by directly fetching that file at revision
`9f7abb866f5afd446db14149979e744c7216baaf` during this implementation pass, not merely inferred
from the design-only facts recorded above. `crates/cic-render/src/wnd_scene.rs` stages the decoded
hierarchy as renderer-only colored quads; this staging and its capture path are original project
presentation, not derived from the legacy renderer. No C++ source was copied, translated line by
line, or imported; names and API boundaries are native to this repository.

The UI definition decoders (`crates/cic-formats/src/ui_ini.rs`, `mapped_image_ini.rs`,
`header_template_ini.rs`, `language_ini.rs`) were written against the source files listed above,
retrieved verbatim through `raw.githubusercontent.com` at the pinned revision rather than through any
summarizing intermediary. The lexical rules come from `INI::readLine`, `INI::initFromINIMulti`, the
`getSeps`/`getSepsColon`/`getSepsQuote`/`getEndToken` accessors, `INI::getNextToken`,
`INI::getNextSubToken`, `INI::getNextQuotedAsciiString`, `INI::getNextAsciiString`, `INI::scanInt`,
`INI::scanReal`, `INI::scanBool`, `INI::scanIndexList`, and `INI::parseBitString32`. The field sets
come from `Image::m_imageFieldParseTable`, `HeaderTemplateManager::m_headerFieldParseTable`, and
`TheGlobalLanguageDataFieldParseTable`; the derived values from `Image::parseImageCoords` and
`Image::parseImageStatus`; the defaults from `Image::Image`, `HeaderTemplate::HeaderTemplate`,
`GlobalLanguage::GlobalLanguage`, and `FontDesc::FontDesc`.

Reading that source corrected a conclusion this project had recorded from data alone. `docs/formats/wnd.md`
previously stated that `Data/INI/MappedImages/**` is a plain recursive merge because the
`HandCreated/` and `TextureSize_512/` name sets measured disjoint, and noted that the loader had not
been located. `ImageCollection::load` in `Image.cpp` is that loader: it loads the user-data
directory, then one `TextureSize_<N>` directory selected by its caller, then `HandCreated` last, with
`INI::loadDirectory` sorting each directory's own files before its subdirectories' files. Re-measuring
with the source order against a real installation found the name sets are not disjoint at all — 23
overrides in Generals and 43 in Zero Hour — so the earlier claim was wrong and the ordered load is
required for correctness. `GameClient::init` supplies the literal `512`.

Composition, catalog and report shapes, diagnostics, limits, the language parameterization, and the
`ui-resources` report are project design, not source behavior.

`crates/cic-ui` implements Gate 5 against source read at the same pinned revision.
`parseScreenRect` in `GameWindowManagerScript.cpp` establishes the classic scaling policy exactly:
per-axis ratios, a truncating `(Int)` cast, size derived from the scaled corners, and a child
position made relative to the parent's already-scaled screen position. `GameWindow.cpp` establishes
that a child rectangle is parent-relative (`winGetScreenPosition` sums ancestors), that the point
test is inclusive on both edges, and that `winPointInChild` walks children in source order, skips a
hidden or disabled child and continues with its siblings, and returns the parent when no child
matches. `GameWindowManager.cpp` establishes the three-pass `ABOVE`/unlayered/`BELOW` search, the
`NO_INPUT` discard, the mouse-captor confinement, `winSetFocus`'s `NOFOCUS` refusal and
parent-walking acceptance, and the wraparound tab cycle. `GameWindow.h` supplies the `WIN_STATUS_*`
bit values, which `UiStatus` mirrors one to one. `GadgetRadioButton.cpp` establishes group
exclusivity.

Reading that source produced one finding worth recording: `GameWindow::winNextTab` and
`winPrevTab` are entirely commented out at this revision and return success without moving focus, so
per-window tab traversal is not source behavior at all. The live mechanism is the window manager's
tab list. `cic-ui` reproduces the manager's wraparound cycle and derives the list from the declared
`TABSTOP` status bit, which is project design filling a documented gap rather than a port.

The arena representation, typed `UiEvent` values, the `Modern` scale policy, the clip policy, the
frame vocabulary, the character-wise text-editing model, diagnostics, and every limit are project
design.

`crates/cic-render/src/ui.rs`, `ui.wgsl`, and `ui_text.rs` implement Gate 6 as original project
presentation: the batching scheme, vertex format, shader, clip intersection and clamping, placeholder
policy, and capture path derive no algorithm from the legacy Direct3D UI renderer. Two behaviours do
follow the source: a border is drawn only for a control declaring `WIN_STATUS_BORDER`, and the
enabled/disabled/hilite slot selection follows the three-state draw data every gadget declares.

Text uses `cosmic-text` 0.19 (MIT or Apache-2.0) and `glyphon` 0.12 (MIT, Apache-2.0, or Zlib), the
pair ADR 0010 selected, at the current releases satisfying it. `glyphon` 0.12 declares
`wgpu ^30.0.0`, which unifies with the workspace `wgpu` 30 — verified with `cargo tree -i wgpu`,
which reports one `wgpu v30.0.0` shared by both. Both licences are permissive and compatible with
this project's GPL-3.0-only licence; neither library defines UI semantics.

Push-button draw-data composition is derived from two files at the pinned revision.
`GadgetPushButton.h`'s inline accessors fix the entry indices: `GadgetButtonGetLeftEnabledImage` reads
entry 0, `...MiddleEnabledImage` entry 5, `...RightEnabledImage` entry 6, and the selected variants
read 1, 3, and 4. `W3DPushButton.cpp` supplies the geometry:
`W3DGadgetPushButtonImageDraw` takes the three-piece path only when the middle image is present,
`W3DGadgetPushButtonImageDrawThree` repeats the centre in whole pieces from the left end's right
edge, draws a final partial piece under a clip region, and draws the two ends last so they sit over
the centre, with a `centerWidth <= 0` branch giving each end half the control. `drawButtonText` in the
same file centres a button's text on both axes unless the control declares `SHORTCUT_BUTTON`.

Both sides of that branch are implemented. The test is
`GadgetButtonGetMiddleEnabledImage` — the *enabled* slot's entry 5, whatever the control's own state
— and its false side is `W3DGadgetPushButtonImageDrawOne`, one image stretched from the image offset
across the control's own size. Retail depends on it: `SkirmishGameOptionsMenu.wnd`'s eight
`ButtonMapStartPosition` markers declare entry 0 alone. Two details of that procedure are reproduced:

- Its state chain reads the *hilite* slot's entry 1 for a selected button even when the button is
  enabled and unhilited, so `GadgetButtonGetEnabledSelectedImage` (enabled entry 1) is declared in
  the header and drawn by no push-button procedure.
- Because the branch tests a resolved `Image *`, a middle image whose name does not resolve reads
  exactly like no middle image and sends the button down the one-image path. An unresolved name is
  therefore only reported when the branch that draws it is the one taken.

The source has a third path that also forces the one-image procedure, and it is deliberately not
implemented: it tests the *state* mask against `WIN_STATUS_USE_OVERLAY_STATES` (`0x00200000`), a
status constant far above every `WIN_STATE_` bit, so the condition cannot be true. Its own
`DEBUG_CRASH` text describes it as a mistake being corrected at runtime.

This project's implementation trims the partial piece's texture coordinates where the source sets a
clip region — the same pixels reach the target without a state change — and that substitution is the
only intentional divergence. It also derives one rule from what the same file does *not* do:
`winDrawImage` takes no colour argument, so an image draw is untinted and a slot's `COLOR` belongs to
the colour-only draw path.

The remaining families' composition is now implemented, each from its own `Gadget*.h` index map and
`W3DGadget*` geometry read at the pinned revision. The index maps are:

| Family | Slot entries the source reads |
| --- | --- |
| `RADIOBUTTON` | left 0, unchecked box 1, checked box 2 per slot; a selected button reads hilite 3, 4, 5 |
| `CHECKBOX` | unchecked box 1, checked box 2; entry 0 is a background the source leaves commented out |
| `ENTRYFIELD` | left 0, right 1, centre 2, small centre 3 |
| `VERTSLIDER` | top 0, bottom 1, centre 2, small centre 3 |
| `HORZSLIDER` | fill and blank from disabled 0 and 1, highlight from hilite 0 |
| `PROGRESSBAR` | background left 0, right 1, centre 2; bar right 5, centre 6 |
| `TABCONTROL` | background `GTC_BACKGROUND` 0, tabs `GTC_TAB_0` through `GTC_TAB_7` at 1 through 8 |
| everything else | entry 0, stretched across the control |

Five geometry facts came out of that reading and are reproduced rather than smoothed over:

- `W3DGadgetRadioButtonImageDraw` tests `WIN_STATE_SELECTED` *before* the enabled bit, so a selected
  radio button draws from the hilite slot even while enabled and never shows disabled art.
- `W3DGadgetHorizontalSliderImageDraw` ignores the control's own state when choosing art: fill and
  blank always come from the disabled slot and the highlight row from the hilite slot. It also scales
  its tick square by `TheDisplay->getWidth() / DEFAULT_DISPLAY_WIDTH` (800), so the squares track the
  display rather than the control.
- `W3DGadgetTextEntryImageDraw` and `W3DGadgetVerticalSliderImageDraw` draw one small-centre piece
  more than fits (`pieces = gap / width + 1`), deliberately overrunning into where the end piece
  draws over it.
- `W3DGadgetProgressBarImageDraw` insets the bar ten pixels horizontally and five vertically inside
  its background, and fills the unreached remainder of the track with the bar's *right* piece rather
  than leaving it empty.
- `drawCheckBoxText` does not centre a check box's label the way `drawButtonText` and
  `drawRadioButtonText` centre theirs: it centres vertically but starts the label one control-height
  in from the left, clearing the box image.

`GadgetTabControlComputeTabRegion` supplies the tab strip's origin from `TABEDGE`, `TABORIENTATION`,
and `PANEBORDER`, with `TP_CENTER = 0`, `TP_TOPLEFT = 1`, `TP_BOTTOMRIGHT = 2`, `TP_TOP_SIDE = 3`,
`TP_RIGHT_SIDE = 4`, `TP_LEFT_SIDE = 5`, and `TP_BOTTOM_SIDE = 6` from `Gadget.h`. One fidelity note
belongs with it: `parseTabControlData` reads `TABWIDTH` and `TABHEIGHT` straight from the file and
nothing scales them, so a tab strip does not follow the creation-resolution scaling its own control
does. That is reproduced as source behaviour. No retail layout declares a tab control, so none of
this is cross-checked against real data.

The image path itself is chosen the way the source chooses it, in two steps: creating a gadget
assigns a default procedure from the `WIN_STATUS_IMAGE` bit (`getPushButtonImageDrawFunc` against
`getPushButtonDrawFunc` in `GameWindowManager.cpp`), and a `DRAWCALLBACK` the function lexicon
resolves then replaces it in `winCreateFromScript`. So a name reading as a bound draw procedure
decides, and anything else — including the overwhelmingly common `"[None]"` — leaves the status bit
deciding.

The colour path — the branch every `...Draw` procedure takes where its `...ImageDraw` twin exists —
has one shape wherever it appears. `W3DGameWinDefaultDraw`, `W3DGadgetPushButtonDraw`,
`W3DGadgetCheckBoxDraw`, and `W3DGadgetComboBoxDraw` all open a one-pixel rectangle at the control's
full bounds with the slot's border colour, then fill from one pixel inside it. Three facts fix when
that happens, and all three were established against a real installation rather than from reading
alone:

- Each colour is tested against `GAME_COLOR_UNDEFINED`, which `Color.h` defines as `0x00FFFFFF` and
  `GameMakeColor` packs `ARGB` — so the sentinel is exactly `255 255 255 0`, the value retail writes
  into every unused draw-data entry. The test is that value, not the alpha channel.
- The two colours are tested independently, so an undefined border still leaves the fill and an
  undefined fill still leaves the outline. The fill is inset by the outline's pixel either way.
- `WIN_STATUS_BORDER` (`0x00001000`) does not gate this outline. **No draw procedure reads that bit
  at all**, so nothing in the colour path is conditioned on it. Correspondingly, no `...ImageDraw`
  procedure outlines anything: art on the image path carries its own edges.

`WIN_STATUS_BORDER` does have a reader, but it is a different border and a different layer, and
this project does not implement it yet. `GameWindowManager::drawWindow` calls the window's own
`winDrawBorder()` when the bit is set and `SEE_THRU` is not — after its children for most styles,
*before* them for `GWS_SCROLL_LISTBOX`. `W3DGameWindow::winDrawBorder` draws an ornamental frame out
of mapped images, not out of any draw-data colour: `blitBorderRect` tiles `BorderTop`, `BorderLeft`,
`BorderRight`, `BorderBottom`, their `...Short` halves, and the four `BorderCorner__` pieces, all
looked up by hardcoded name. It selects geometry by scanning the style mask from bit 0 upward and
stopping at the first match, so the lowest set style bit decides, and two of those cases draw
nothing at all — `GWS_CHECK_BOX` matches and returns, and both slider cases have their blit
commented out. That is why removing this project's incorrect colour-path border from check boxes
left them correct rather than under-drawn.

This corrects a rule recorded earlier in this project, which gated the border on `WIN_STATUS_BORDER`
and drew it on both paths. Rendering the retail Options menu showed both halves of the error at
once: check boxes wore a salmon `255 128 128` outline the original never draws, while the panel
frames that divide Display, Audio, Control, and Network — colour-path windows that declare no
`BORDER` bit — were missing entirely.

Where a family's own indices declare nothing, each source procedure returns early and draws nothing.
This project reproduces that and records a diagnostic naming the family, rather than staging a
placeholder: a placeholder there would invent a control retail never shows. Placeholders stay for a
genuinely unresolved resource, which is a different failure.

## Gadget child creation

Three of the original's gadget-creation functions build child windows that no layout declares, and
those children are what most of a layout's draw-data records exist to dress. All three are
reproduced in `cic-ui` from the pinned revision.

- `gogoGadgetSlider` gives every slider a draggable thumb push button. A horizontal thumb is
  `HORIZONTAL_SLIDER_THUMB_WIDTH` by `_HEIGHT` — 13 by 16 from `Gadget.h` — at
  `HORIZONTAL_SLIDER_THUMB_POSITION`, which `GadgetSlider.h` defines as the height times two thirds
  under integer division, so 10. A vertical thumb is the slider's own width by that width plus one.
- `GadgetListboxCreateScrollbar`, called when `LISTBOXDATA` asks for `SCROLLBAR`, adds an up button,
  a down button, and a vertical slider, all 21 pixels wide two pixels in from the right edge, the
  buttons 22 tall and the slider filling `bottom - 2 * buttonHeight - 6` between them.
- `gogoGadgetComboBox` adds a drop-down button 21 pixels wide at the box's full height, an edit
  field filling the remaining width and carrying the literal text `"Entry"`, and a drop-down list
  hanging directly below the closed box at its own height. The list is created with `scrollBar = 1`,
  so it builds a scroll bar of its own, and `winHide( TRUE )` hides it immediately.

Five facts about that construction are reproduced rather than tidied:

- Every size above is a literal applied to an already-scaled parent rectangle, so gadget parts stay
  the same pixel size at every resolution while the gadget around them grows.
- Each part's status is the gadget's with bits masked out and others forced on — `BORDER` and
  `HIDDEN` always cleared, the scroll bar additionally clearing `NO_INPUT` and forcing `IMAGE`, and
  a non-editable combo box's field gaining `NO_INPUT`. Its `NO_FOCUS` companion is commented out in
  the source, so focus still reaches that field.
- `gogoGadgetSlider` sets `WIN_STATUS_TAB_STOP` on the slider *and* the thumb inherits it, so every
  slider in a layout contributes two tab stops that the file never declared. That is why the Zero
  Hour corpus's nine declared `TABSTOP` controls understate the real tab list.
- `winCreateFromScript` copies each part's arrays into it *after* creation, through file-scope
  statics holding the records of the window being created. A scroll bar's thumb therefore reads
  `SLIDERTHUMB*` from the list box two levels above it, and a combo box's internal scroll bar reads
  `LISTBOX*` and `SLIDERTHUMB*` from the combo box, not from the slider or list that owns it.
  `cic-ui` carries those records down the tree at creation instead of using a global.
- `gogoGadgetComboBox` computes a font height and `top`/`bottom` for a list title and then uses none
  of them, and declares a `buttonHeight` of 22 that its drop-down button ignores in favour of the
  box's full height. `GadgetListboxCreateScrollbar`'s title inset is real, but no retail layout
  gives a scroll list box any text and a combo box's internal list is created with none, so it is
  the one part of this that would need font metrics and it is unreachable; a layout that reached it
  would be reported rather than mislaid.

Gate 4's transition, cursor, and menu-scheme subsets and everything else recorded above remain
design-only.
