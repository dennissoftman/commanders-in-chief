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
- Legacy rendering evidence:
  - `Generals/Code/GameEngineDevice/Include/W3DDevice/GameClient/W3DGameWindowManager.h`
  - `Generals/Code/GameEngineDevice/Source/W3DDevice/GameClient/GUI/W3DGameWindowManager.cpp`
  - `Core/GameEngineDevice/Include/W3DDevice/GameClient/W3DGadget.h`
  - `Core/GameEngineDevice/Source/W3DDevice/GameClient/GUI/Gadget/`
- Mapped image, font, localization, and language evidence:
  - `Core/GameEngine/Source/Common/INI/INIMappedImage.cpp`
  - `Core/GameEngine/Include/GameClient/GameFont.h`
  - `Core/GameEngine/Source/GameClient/GUI/GameFont.cpp`
  - `Generals/Code/GameEngine/Include/GameClient/FontDesc.h`
  - `Core/GameEngine/Include/GameClient/GlobalLanguage.h`
  - `Core/GameEngine/Source/GameClient/GlobalLanguage.cpp`
- Callback-name and shell/navigation evidence:
  - `Generals/Code/GameEngine/Include/Common/FunctionLexicon.h`
  - `Generals/Code/GameEngine/Source/Common/System/FunctionLexicon.cpp`
  - `Generals/Code/GameEngine/Include/GameClient/Shell.h`
  - `Generals/Code/GameEngine/Source/GameClient/GUI/Shell/Shell.cpp`
  - `Core/GameEngine/Include/GameClient/GameWindowTransitions.h`
  - `Core/GameEngine/Source/GameClient/GUI/GameWindowTransitions.cpp`
  - `Generals/Code/GameEngine/Source/GameClient/GUI/GameWindowTransitionsStyles.cpp`
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
line, or imported; names and API boundaries are native to this repository. Gate 2 (typed per-gadget
fields), resource resolution, the retained `cic-ui` runtime, and everything else recorded above
remain design-only.
