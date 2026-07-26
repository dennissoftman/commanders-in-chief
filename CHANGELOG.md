# Changelog

All notable user-visible changes are recorded here, grouped by milestone period. New entries
land under the active milestone heading.

## R4: WND user interface and navigable shell (active)

### Added

- Added a bounded WND text-format decoder (`crates/cic-formats/src/wnd.rs`) covering `FILE_VERSION`,
  the `STARTLAYOUTBLOCK`/`ENDLAYOUTBLOCK` layout block, and the complete `WINDOW`/`CHILD`/`END`/
  `ENDALLCHILDREN` hierarchy with `WINDOWTYPE`/`SCREENRECT` typed. Every other field is retained
  generically rather than dropped, and unrecognized top-level keywords or out-of-vocabulary
  `WINDOWTYPE` values are surfaced as non-fatal diagnostics instead of silently disappearing.
  Original synthetic positive, exhaustive-truncation, per-limit, and unknown-field-preservation
  tests pass.
- Added `cic-inspect wnd`, a stable source-order inventory report over the decoded hierarchy,
  generic fields, and diagnostics.
- Added `cic-inspect wnd-render`, a surface-free proof-of-pipeline capture that stages every window
  rectangle as a flat colored quad (`crates/cic-render/src/wnd_scene.rs`) and renders it through the
  existing `HeadlessRenderer` boundary to a deterministic PNG plus RGBA SHA-256 hash. It has no
  images, text, gadget visuals, or scaling policy yet; it proves the immutable decoded value can
  drive a renderer capture ahead of the retained UI runtime.
- Added an original synthetic `BIG4` fixture and truncation-at-every-prefix tests alongside the
  existing `BIGF` coverage, and a bounded `big` libFuzzer target, closing an R1 acceptance-test gap
  where BIG4 had no automated coverage and BIG archives had no fuzz target.

- Added the project-owned WND patch overlay layer (`crates/cic-formats/src/wnd_patch.rs`), so
  modern controls and profile-specific adjustments are auditable data rather than hardcoded window
  names in the parser, renderer, or a menu callback. A versioned line-oriented patch targets one WND
  virtual path and exact decorated control names, and supports `require-window`/`require-field`
  preconditions plus `set-field`, `add-field`, and `set-rect`. Applying returns a new document —
  the parsed source is never mutated, so one parse can be patched differently per profile — and
  every write records provenance. A patched field is re-typed, so an overlay that rewrites `STATUS`
  is visible through the typed accessor. The structural operations `reorder`, `reparent`, and
  `insert-window` complete the set: an inserted subtree is parsed by the ordinary bounded WND
  decoder, so it obeys the same grammar, limits, and typed-field rules as authored source, and its
  window ids are renumbered so they cannot collide. `reparent` refuses to move a window beneath its
  own descendant, and a failure after detaching restores the tree rather than dropping the subtree.
  Verified end to end against the retail `OptionsMenu.wnd`, reusing and repositioning the stock
  resolution combo while inserting a project-owned refresh-rate control beside it.
- Added `cic-inspect wnd-patch`, reporting each patch's declared operations, the provenance of every
  field written, and the resulting hierarchy, without ever rewriting the source WND.
- Completed WND field decoding: the 21 draw-data arrays (nine `IMAGE`/`COLOR`/`BORDERCOLOR` entries
  each, with `NoImage` decoded as an absent image) and all seven gadget `DATA` records
  (`LISTBOXDATA`, `COMBOBOXDATA`, `SLIDERDATA`, `RADIOBUTTONDATA`, `TEXTENTRYDATA`,
  `STATICTEXTDATA`, `TABCONTROLDATA`) plus `IMAGEOFFSET`, reported as `window_draw_entry` and
  `window_gadget_data` rows. Every field name occurring in either retail edition is now typed, and
  the whole corpus decodes with no malformed-field diagnostics: 7,875 of 7,875 draw-data arrays and
  753 of 753 gadget records. `LISTBOXDATA`'s `SCROLLIFATEND` is decoded as genuinely optional, which
  five retail records depend on; an omitted sub-record stays distinguishable from an explicit false.
  `TABCONTROLDATA`'s pane count is bounded at the source's own array width, which the source reads
  past without checking. `TOOLTIP` is deliberately left untyped: its parser ignores the record and
  stores a placeholder marked `@todo`, so no grammar exists to decode.
- Added typed decoding for the common WND window records — `STATUS`/`STYLE` flag lists, the four
  callback names, `FONT`, `HEADERTEMPLATE`, `TOOLTIPDELAY`, `TEXT`, `TOOLTIPTEXT`, and `TEXTCOLOR`'s
  six state colors — exposed as accessors on the immutable window value and as
  `window_flag`/`window_callback`/`window_property`/`window_font`/`window_text_color` rows in
  `cic-inspect wnd`, so a modded layout can be compared record by record without rendering it.
  Typed values are views: every record also remains in the generic field list. Across both retail
  editions this types every occurrence (1,667 of 1,667 `FONT` and `TEXTCOLOR` records, 6,668 of
  6,668 callbacks) with no malformed-field diagnostics. A record that does not match its established
  shape produces a `MalformedField` diagnostic and an absent typed view rather than failing the
  document; required structural values (`FILE_VERSION`, `WINDOWTYPE`, `SCREENRECT`) remain hard
  errors.
- Added quoting- and punctuation-preserving WND record retention. Each field now carries an ordered
  token sequence alongside its verbatim value, so `FONT = NAME: "Times New Roman", SIZE: 14;` is no
  longer indistinguishable from the same characters written unquoted — previously the value
  flattened to `NAME: Times New Roman , SIZE: 14`, leaving no way to delimit a font name containing
  spaces. `,`, `:`, and `+` are tokenized outside quotes; quoted tokens are never split.
- Added typed decorated window names (`WndWindow::name`/`control_name`), reported by
  `cic-inspect wnd` in a new column, plus a `DuplicateWindowName` diagnostic for two windows sharing
  a non-empty control name. Windows declaring only a layout prefix are treated as unnamed.
- Added `STATUS` and `STYLE` flag validation over the `+`-separated name lists, against the union of
  both editions' vocabularies. Applied to every retail layout in both editions, this produces no
  false positives.
- Added a bounded `wnd` libFuzzer target and a `maximum_record_tokens` limit (default 4,096) so one
  record cannot allocate a token vector sized only by the much larger record byte limit.

- Added the UI definition resources a WND layout names, so a decoded layout's images, fonts, header
  templates, and localized text can be resolved instead of left as strings. Three bounded decoders
  share one lexer derived from the original INI reader — `MappedImage` blocks over
  `Data/INI/MappedImages`, `Data/<Language>/HeaderTemplate.ini`, and `Data/<Language>/Language.ini`
  with all 25 fields including its 17 font roles. Field names and block keywords are case-sensitive
  and `End` is not, matching the source's own `strcmp`/`stricmp` split. Source quirks are reproduced
  rather than corrected, because a definition authored against the original reader must resolve the
  same way: `Status = ROTATED_90_CLOCKWISE` swaps a region's presentation size at the point it is
  read, so placing it before `Coords` behaves differently; a quoted string is rejoined from at most
  two tokens and loses a one-character continuation; an unquoted multi-word value keeps only its
  first token; and repeated `LocalFontFile` names apply in reverse file order. Unknown fields and
  blocks become diagnostics instead of disappearing, and a duplicate definition overwrites field by
  field like the original loader rather than replacing the whole definition.
- Added `cic-inspect ui-resources`, which loads those catalogs through the VFS and reports every
  demanded resource with its binding, every definition file that contributed, every name a later file
  overrode, and per-kind resolved/unresolved counts. Verified against a real installation across every
  layout in both editions: header templates resolve completely (209 of 209 in Zero Hour, 196 of 196 in
  Generals) and mapped images resolve 1,849 of 1,978 and 1,789 of 1,932. What is left unresolved is
  retail's own gap, now visible rather than silent — around 50 distinct image names no shipped INI
  defines, three font families named but never shipped, and the 17 Zero Hour labels the string table
  omits, which reproduces an earlier independent measurement exactly.
- Added language selection to localization mounts. The localization archive and definition paths were
  hardcoded to `English.big`/`EnglishZH.big`; a `--language <name>` option now selects
  `<Language>.big`, `<Language>ZH.big`, and `Data/<Language>/`, which is what shipping a language the
  original game never had requires. A new `Ui` mount profile adds the INI, texture, and selected
  localization archives to the window archives, because header templates, fonts, and labels live in
  the localization archive rather than the window archive.

- Added `cic-ui`, the retained user-interface runtime, so an immutable WND definition becomes a live
  control tree with presentation state. It depends only on `cic-formats`: it consumes definitions and
  produces renderer-neutral frames, so it links to no rendering API and holds no simulation state.
  Layout reproduces the original's scaling exactly — each stored corner scaled per axis by
  viewport-over-creation-resolution and truncated, size derived from the scaled corners, and child
  positions made relative to the parent's already-scaled origin — so an 800x600 layout on 1600x900
  stretches the way the original does. A project-designed `Modern` policy applies one uniform ratio
  and centres the result instead, for callers that would rather letterbox than stretch.
- Added hit testing that reproduces the original's layered search: `ABOVE` windows first, then
  unlayered, then `BELOW`, descending through children in source order and returning the first
  visible, enabled one, with a hidden or disabled child skipped so the click falls through to the
  parent instead of being swallowed. Edge tests are inclusive on both ends, which decides which of two
  adjacent controls a boundary click reaches, and a control declaring `NO_INPUT` discards the result.
- Added focus, tab traversal, and control invariants: `NOFOCUS` refusal with the original's
  parent-walking acceptance; a wraparound tab cycle over declared `TABSTOP` controls that skips
  disabled and hidden stops; radio-group exclusivity; slider clamping that orders an inverted
  `MINVALUE`/`MAXVALUE` pair with a diagnostic; list and combo selection that refuses an out-of-range
  index rather than clamping it to a different row; list scroll clamping that keeps the last page
  full; and text entry that counts characters rather than bytes against its declared `MAXLEN`, so a
  Unicode field holds what its definition promises. Hiding or disabling a control clears hover, press,
  focus, and capture through its whole subtree.
- Added renderer-neutral UI frames: an ordered list of quads carrying the mapped-image name and
  colours of the draw-data slot the control's current state selects, text runs carrying the label,
  font, state colour, and a mask flag for secret entries, and optional clip push/pop. Submission
  order inverts the hit-test layering and emits each subtree parent before children, so a child draws
  over its parent. Clipping is an explicit policy because the original does not clip and retail
  layouts rely on that.

- Added `cic-inspect ui-layout`, which instantiates a layout for an explicit viewport and scale
  policy and reports the retained tree, tab order, frame submission order, and diagnostics without
  reading the host display. Every one of the 80 Zero Hour and 78 Generals layouts instantiates at
  800x600, 1920x1080, and 21:9 2560x1080 under both policies — 480 instantiations — with no failures
  and zero diagnostics. The Zero Hour corpus yields 1,667 retained controls, matching the WND
  census's window count. That pass also measured that the whole corpus declares only nine `TABSTOP`
  controls, so keyboard traversal of a retail menu will need project-owned tab order.

- Added custom `wgpu` presentation for retained UI, so a decoded layout renders as a real menu rather
  than as flat rectangles. `cic-render` stages a frame into batched geometry - breaking a batch only
  when the bound texture page or scissor rectangle changes - and executes it through the existing
  surface-free capture boundary. Nested clips intersect and are clamped into the attachment, alpha is
  straight rather than premultiplied to match the source's stored channel bytes, and texture pages
  upload in the capture target's own colour space so a sampled byte reaches the attachment unchanged.
  A border draws only for a control declaring `BORDER`; honouring a border colour alone outlines the
  entire menu, because most retail controls carry one.
- Added Unicode text shaping through `cosmic-text` 0.19 and `glyphon` 0.12, the pair ADR 0010
  selected. `glyphon` 0.12 declares `wgpu ^30.0.0` and unifies with the workspace `wgpu` 30 rather
  than pulling a second copy, and both licences are permissive. Fonts are always supplied as bytes by
  the caller - nothing enumerates host fonts, because a capture that silently picked up a platform
  face would hash differently on another machine. With no font supplied, a visible placeholder bar and
  a diagnostic stand in for each run instead of the text silently disappearing, and a secret entry
  field renders one mask glyph per character rather than its contents.
- Added `cic-inspect ui-render`, which writes a deterministic PNG plus an RGBA SHA-256 hash from
  explicit inputs only: viewport, scale policy, clip policy, language, texture-size selection, and
  font files. Verified against a real installation at 1280x720 - `MainMenu.wnd` stages 37 quads in 12
  batches over three texture pages with 29 shaped runs, `OptionsMenu.wnd` 41 quads and 25 runs,
  `SkirmishGameOptionsMenu.wnd` 52 quads and 21 runs - with byte-identical hashes across repeated
  runs and localized labels resolved through the CSF decoder before staging.

- Added push-button draw-data composition and centred button text, so a retail menu renders as a menu
  rather than as stretched single-piece art. `GadgetPushButton.h` fixes the indices (unselected left 0,
  middle 5, right 6; pushed 1, 3, 4) and `W3DGadgetPushButtonImageDraw` takes the three-piece path only
  when the middle image is present. The centre repeats in whole pieces, a final partial piece covers
  the remainder, and the ends draw last over it, including the source's branch for ends that do not
  fit. Button text is centred on both axes, as `drawButtonText` does.

- Added draw-data composition for every remaining gadget family, completing Gate 6: radio buttons,
  check boxes, text entry, both slider orientations, progress bars, tab controls, and the stretched
  single-image path list boxes, combo boxes, and static text share. Each family's entry indices come
  from its `Gadget*.h` accessors and its geometry from the matching `W3DGadget*ImageDraw`, so a
  layout's art now reaches the screen the way the family that authored it intended rather than as one
  stretched background. Several source behaviours are reproduced rather than smoothed over: a
  selected radio button reads the hilite slot even while enabled, a horizontal slider takes its tick
  art from fixed slots whatever its own state and sizes those ticks against an 800-pixel display
  reference, a text entry and a vertical slider each draw one seam piece more than fits so the end
  piece covers it, and a progress bar fills the unreached part of its track with the bar's right
  piece. A check box draws only its box — the source leaves its background draw commented out — and
  its label is now indented past that box rather than centred, which is what `drawCheckBoxText` does.
  A new `crates/cic-render/tests/ui_capture.rs` renders an original all-families synthetic layout
  through the surface-free capture boundary, byte-identically across runs.

- Added the retained draw-callback name to the UI runtime, and with it the source's own two-step
  choice of draw procedure: the `IMAGE` status bit picks a default when a gadget is created, and a
  `DRAWCALLBACK` the function lexicon would resolve then replaces it. A layout naming
  `GadgetStaticTextDraw` now draws colour-only even while declaring `IMAGE`, and the ubiquitous
  `"[None]"` correctly leaves the status bit deciding.

### Fixed

- Fixed the whole-control border being drawn from the wrong rule, which was both adding outlines the
  original never draws and omitting ones it does. This project had gated the border on the
  `WIN_STATUS_BORDER` status bit and drawn it whichever draw path a control took. At the pinned
  revision no draw procedure reads that bit at all, and the border belongs to the colour path alone:
  `W3DGameWinDefaultDraw` and each gadget's colour draw open a one-pixel rectangle at the control's
  bounds and then fill one pixel inside it, while every matching `...ImageDraw` outlines nothing and
  leaves edges to the art. Each colour is compared against `GAME_COLOR_UNDEFINED` — `0x00FFFFFF`,
  which is exactly the `255 255 255 0` retail writes into unused draw-data entries — and the fill and
  outline are tested independently, so one being undefined still leaves the other. Rendering the
  retail Options menu showed both halves of the old error at once: every check box wore a salmon
  outline, and the panel frames dividing Display, Audio, Control, and Network were missing because
  those windows take the colour path without declaring `BORDER`.
- Fixed push buttons that declare a single image drawing nothing.
  `W3DGadgetPushButtonImageDraw` chooses between two procedures on whether the *enabled* slot
  declares a middle image, and only the three-piece side was implemented; the other side,
  `W3DGadgetPushButtonImageDrawOne`, stretches one image across the control from its image offset.
  Retail depends on it — `SkirmishGameOptionsMenu.wnd`'s eight `ButtonMapStartPosition` markers
  declare entry 0 alone and were invisible, and now draw. Because the original branches on a
  resolved image pointer, a middle image whose name does not resolve reads as no middle and takes
  the same path, so an unresolved name is reported only when the branch that draws it is taken.
- Fixed images being tinted by their slot's `COLOR`. `winDrawImage` takes no colour - that field
  belongs to the colour-only fill path - and retail frequently leaves an unused red there beside a
  valid image, so every textured control rendered red. A control declaring `IMAGE` whose slot has no
  entry-0 image first staged a visible placeholder instead of painting that same unused colour;
  now that every family composes, such a control draws nothing — the source's own early return —
  and records an `UncomposedArt` diagnostic naming the family, since a placeholder there would
  invent a control retail never shows. Placeholders remain for a genuinely unresolved mapped image.
- Corrected the recorded mapped-image load policy. This project had documented
  `Data/INI/MappedImages/**` as a plain recursive merge, on the measured basis that the
  `HandCreated/` and `TextureSize_512/` name sets were disjoint, and noted that the source loader had
  not been located. It has been: `ImageCollection::load` loads the user-data directory, then one
  `TextureSize_<N>` directory selected by its caller, then `HandCreated` last, sorting each
  directory's own files before its subdirectories'. Re-measured with that order against a real
  installation, the name sets are not disjoint — 23 definitions are overridden in Generals and 43 in
  Zero Hour, and both editions ship a `HandCreatedMappedImages.ini` in both directories — so a
  merge-everything loader resolves some names to the wrong texture region. The implemented loader
  follows the source order.

- Fixed the WND decoder rejecting Zero Hour's `Menus/MainMenu.wnd`, the layout the R4 main-menu
  artifact is built around. The decoder required a `CHILD` marker before every child window, but
  the source's child-list loop has no `CHILD` case at all — the marker is inert and a bare `WINDOW`
  opens the next sibling. A census of all 80 retail layouts in both editions found exactly one
  sibling written without its marker, and it is in that file. The decoder now accepts either
  spelling once the child list is open and reports the unmarked form as a non-fatal
  `MissingChildKeyword` diagnostic; a bare `WINDOW` before any `CHILD` is still a field name, and
  `ENDALLCHILDREN` still closes the list. All 80 retail layouts in both editions now decode, with
  one diagnostic across the whole corpus.
- Corrected the documented WND status vocabulary, which listed only the `Generals` source path's 25
  names. Zero Hour adds `ON_MOUSE_DOWN`, used 67 times in its retail layouts, so validating against
  the Generals list alone would report 67 false unknowns against a stock Zero Hour install.
- Repaired the `map` libFuzzer target, which no longer compiled after `MapLimits` gained polygon
  and water-trigger fields during R3; fuzzing was not part of the workspace test suite so the
  regression was silent.
- Corrected the R2 milestone doc's stale W3D chunk-identifier count (73 to 77).

### Changed

- Changed every renderer capture from an uncompressed netpbm PPM to PNG, which also preserves the
  alpha channel that PPM discarded and tags perceptual sRGB. `Capture::ppm` is replaced by
  `Capture::png` at the renderer boundary, so `cic-inspect wnd-render`, `w3d-render`, and the
  `headless_capture` example all emit `.png` and default their output name accordingly. Reported
  hashes are taken over the capture's raw RGBA bytes before encoding, so determinism is unchanged;
  ADR 0004 carries an amendment note recording this.
- Added `*.png`/`*.ppm` to `.gitignore` (excluding `docs/`). The `*-render` commands write into the
  working directory when given no output path, so a capture of a user-owned layout or map could
  otherwise be committed as retail-derived output.

- Changed `terrain_ini.rs`, `water_ini.rs`, `road_ini.rs`, and `object_ini.rs` (owned by R3) so an
  unrecognized field name inside a block they otherwise decode is never silently dropped. Each
  narrow INI decoder now retains every such field as a non-fatal diagnostic
  (`TerrainIniDiagnostic`, `WaterIniDiagnostic`, `RoadIniDiagnostic`, `ObjectIniDiagnostic`,
  exposed via a new `diagnostics()` accessor on `TerrainIni`/`WaterIni`/`RoadIni`/`ObjectIni`), so
  an unsupported or genuinely missing field stays discoverable instead of disappearing silently.
  Already-recognized fields keep their exact prior behavior; entirely unrelated INI blocks and
  `object_ini`'s intentionally out-of-scope gameplay modules (`Behavior`, `Body`, and similar)
  remain excluded as before, since that boundary is architectural, not a dropped field.

- Closed R3 and advanced the active objective to R4's bounded WND inventory/layout decoder and
  synthetic headless menu vertical slice. Version-1 height presentation now explicitly retains its
  native stored grid; source-editor preview/auxiliary chunks remain opaque and R4 previews are
  generated from `map-render`.
- Inserted an R4 WND/UI compatibility milestone before simulation. The design selects a custom
  retained WND model and `wgpu` renderer, bounded UI resource loading, safe menu callback routing,
  a versioned post-parse WND patch layer, modern resolution/refresh-rate settings with confirmed
  apply/rollback, and a navigable main-menu/skirmish/map-selection demo using R3 map previews and
  spawn candidates.

## R3: Complete MAP ingestion and terrain-scene presentation (completed 2026-07-23)

### Added

- Added bounded renderer-only MAP diagnostics to `map-view` and `map-render`: larger per-player
  start beacons, ordinary waypoint beacons, named waypoint-path ribbons, and source-ordered
  translucent polygon perimeter walls. Named paths receive deterministic distinct colors and
  connect members in stored waypoint-ID order while following the staged terrain; shared waypoints
  may participate in multiple paths. Marker and zone bases sample the staged terrain, water areas
  use a distinct blue treatment, and no overlay creates simulation state or executes callbacks.
- Added complete bounded `PolygonTriggers` version 2 through 4 retention and the stable
  `map-polygons` report. The existing water report remains a filtered compatibility projection;
  per-area and total retained-point ceilings have independent negative tests.
- Added bounded `W3DTreeDraw` resource parsing and explicit-time source-default `BreezeInfo` tree
  sway. Stable placement IDs select deterministic sway families and randomness without executing
  the decoded `SET_TREE_SWAY` script action.
- Added a shared 2048-square primary directional shadow map for terrain, alpha-tested scenery, and
  forward water, plus edge-aware post-process anti-aliasing.
- Expanded `map-render --time` into a deterministic fixed-isometric full-scene overview containing
  terrain, source-ordered roads and water, scenery markers, and animated tree markers, with scene
  counts and an RGBA SHA-256 diagnostic.
- Added a pinned-source MAP scene compatibility matrix and exhaustive synthetic tests for every
  currently modeled constructor/default, parser input branch, format structure and limit, blend
  version/stride boundary, water trigger version/filter, road diagnostic/topology/atlas output,
  road mip shape, viewer input transition, and wireframe/depth-bias diagnostic value.
- Added source-backed Zero Hour MAP compatibility for `BlendTileData` version 8's corrected cliff
  bitmap stride and `PolygonTriggers` version 4's bounded WorldBuilder layer name. Synthetic tests
  cover the stride delta, truncation, limits, and unsupported neighboring versions.
- Added bounded immutable `WorldInfo`, `ObjectsList`, `SidesList`, build-list, team, and complete
  nested player-script decoders. Stable `map-objects` and `map-sides` reports expose exact scalar
  bits, typed dictionaries, endpoint flags, spawn candidates, and raw script opcodes/parameters
  without validation repair, live object construction, or script execution.
- Added source-order scene staging for road/bridge endpoints, visible scenery placements, hidden
  records, waypoints, and one-based `Player_n_Start` candidates. Definition resolution and actual
  road, bridge, building, vegetation, and prop rendering remain separate presentation gates.
- Added WaterSet sky/environment texture resolution, sibling-map overrides, Modern bounded
  screen-space/environment reflection inputs, and a frozen explicit presentation-time mode for
  `map-view`.
- Added bounded `Road` INI decoding and deterministic regular-road rendering in `map-view`.
  Consecutive Point1/Point2 records resolve source textures and widths, tessellate at terrain-cell
  intervals, follow maximum underlying height, and alpha-overlay in stable MAP order. Missing
  definitions/textures remain explicit diagnostics. Connected endpoints now receive bounded
  edge-derived corner/junction polygons instead of oversized circular fillers.
- Added bounded `Bridge` model/scale, four body-state model/texture pairs, and four tower-template
  references. Consecutive endpoints deform pristine `BRIDGE_LEFT`/`BRIDGE_SPAN`/`BRIDGE_RIGHT`
  sections onto the terrain-sampled sloped axis, and optional towers resolve through the existing
  object/W3D path at source corners and facing. Towers are renderer-only scenery; damage selection,
  transition effects, repair, collision, and targetable tower behavior remain deferred.
- Added an on-demand full-scene wireframe diagnostic to `map-view` on M when the selected GPU
  exposes polygon-line rasterization. Unsupported adapters continue with the normal renderer.
- Added bounded initial Object draw-definition decoding, reskin inheritance, default W3D model and
  scale selection, standalone static-mesh composition, and GPU-instanced static scenery in
  `map-view`. Placements sample the exact rendered terrain triangle, including MAP border and
  diagonal selection, then add the authored relative Z offset verbatim, including negative
  offsets and with no clamp or renderer epsilon.
- Added a renderer-only translucent playable-boundary fence. Its base follows perimeter terrain and
  its global top clears the map's highest terrain sample without changing pathing or simulation.
- Bounded `GlobalLighting` versions 1 through 3 with separate ordered terrain/object lights for
  morning through night, optional packed shadow color, exact-bit `map-lighting` reports, and
  selected-time viewer shading. The complete source-established `WaterSet` and
  `WaterTransparency` field tables are retained under explicit limits; selected diffuse color,
  standing-water texture/blend policy, opacity, and scroll inputs now feed the forward-water
  presentation.
- Bounded water-only MAP decoding/reporting, stable lake/river staging, a modern hybrid-deferred
  terrain viewer with thickness-aware forward water, and deterministic Modern-profile de-tiling.
- Horizon-safe terrain detail streaming with a persistent 128-page GPU-composed virtual-texture
  cache over the stable 8-pixel background. Bordered 16/32-pixel pages preserve authored layers,
  cliff UVs, custom edges, and Modern macro variation; stable page tables, LRU reuse,
  GPU-generated linear mipmaps, and anisotropic sampling remove runtime CPU terrain rebakes.
  Water now uses bounded
  source-resolved caustic animation, source transparency depth,
  a more opaque body, and restored shallow shoreline haze and crest effects.
- Added bounded bare and `EAR\0` RefPack-wrapped `CkMp` MAP symbol-table and top-level chunk
  inventories with opaque unknown payload preservation, deterministic last-symbol-wins name
  resolution, and stable VFS-backed reports.
- Added `HeightMapData` versions 1 through 4 with explicit dimension, border, boundary, payload,
  allocation, and sample-cardinality checks plus stable row-major `cic-inspect map-height` output.
- Added deterministic 8-bit grayscale PNG export to `cic-inspect map-height --png` with exact
  stored sample order and no color-space transform.
- Added bounded immutable `BlendTileData` version-6/7/8 tile planes, version-6 source-equivalent
  height-derived cliff flags, version-7 legacy cliff-bitmap normalization, version-8 corrected
  cliff rows, terrain and edge texture classes, blend records, and cliff UV records, plus a stable
  VFS-backed `cic-inspect map-blend` report.
- Added an original versioned MAP fixture, negative parser tests, a synthetic BIG-backed completion
  artifact, and a bounded MAP fuzz target.
- `cic-inspect map-height` now writes a basename-derived grayscale PNG by default; `--report`
  selects the stable text report and `--png` supplies an explicit output path.
- Added a bounded Terrain INI declaration decoder and deterministic `DefaultTerrain` inheritance so
  semantic MAP texture classes resolve through mounted `Terrain.big`/`INI.big` resources.
- Added source-scaled terrain geometry and deterministic base/primary/extra texture staging with
  packed tile quadrants, source-rounded mip reduction, procedural blend masks, and source-selected
  triangle diagonals.
- Added `cic-inspect map-render`, which produces a depth-tested isometric sRGB PNG and stable
  geometry/layer/hash diagnostics through the headless GPU renderer. An original layered-terrain
  fixture carries a checked RGBA SHA-256 completion hash.
- Added `cic-inspect map-view`, a perspective terrain flyover sharing the map-render resource and
  staging path, with WASD/vertical flight, speed boost, right-mouse look, wheel dolly, and camera
  reset controls.
- Added explicit `legacy` and `modern` terrain policies. Both apply same-class stored cliff UVs;
  the default legacy policy also reproduces bounded steep-slope UV retile and height-selected
  triangle diagonals.
- Added separately indexed custom-edge geometry and deterministic quarter-atlas texturing for
  white material coverage, black gaps, and colored decorative edge pixels in both headless and
  interactive terrain rendering.
- Added bounded renderer detail streaming: quantized, depth-capped screen-space footprints rebake
  authored terrain as independent 16- and 32-pixel tiers over the unchanged deterministic 8-pixel
  background. Generation checks immediately cancel obsolete work and suppress stale uploads;
  explicit-time overlap transitions retain the previous resident patch during replacement.
- Added a bounded `WaterTransparency` INI decoder and renderer-neutral `WaterAppearance` input.
  Installed profiles may resolve complete `caust00`-`caust31` image sequences into a mipmapped GPU
  texture array; synthetic mounts remain valid without retail resources.
- Added terrain-surface directional shading to `map-view`. This explicit presentation preview
  improves slope readability without changing staged values or deterministic headless hashes;
  source-authored MAP lighting remains a later semantic decoder.
- Enabled back-face culling for terrain, custom edges, and streamed detail after verifying the
  stable height-field winding; deterministic terrain capture hashes remain unchanged.

### Changed

- Restored the source road texture's three-level mip budget and handed curve traversal, and added a
  renderer-only road depth bias on top of the legacy terrain lift. This avoids whole-atlas distant
  mip collapse and reduces road/terrain Z-fighting without mutating staged road coordinates.
- Documented the repository-wide Zero Hour layering invariant: enumerate and mount Generals first,
  apply Zero Hour second and mods last; replacement resources use the winner while cumulative
  definition formats parse the complete provider history in order.
- Expanded the R3 design from terrain-only presentation to complete bounded MAP ingestion and a
  non-simulating terrain scene: source lighting and water, object/world records, roads and
  bridges, static scenery and ambient animation, waypoints/player starts, sides/teams/build lists,
  polygon areas, and lossless map scripts. ADR 0009 keeps all runtime activation and script
  execution behind the future deterministic R5 simulation boundary.

### Fixed

- Fixed three `wgpu` validation failures in the new shadow/AA path: shadow resources now belong to
  the deferred-lighting layout rather than the terrain virtual-texture layout, the HDR scene input
  is declared filterable for the AA sampler, and the terrain shadow pass uses its own group-zero
  camera layout. A GPU regression test now constructs both deferred pipelines directly.
- Road and railroad intersections no longer stretch each approach texture across a generic shared
  fan. A deterministic topology pass now trims connected strips and uses legacy curve/miter and
  tee/Y/slanted/four-way atlas geometry. Different materials stay isolated unless an open endpoint
  explicitly requests the legacy alpha-join cap.
- Initial map objects whose W3D draw fields are aligned with their `Draw` declaration now render.
  The bounded parser follows `End`-delimited modules and recognizes the source-equivalent first
  `ConditionState = NONE`, restoring supply docks/stashes, command centers, and similarly authored
  campaign structures without constructing gameplay objects.

- `map-view` now uses an explicit legacy-preview W3D recovery policy for damaged shipped assets:
  missing optional HLOD meshes are skipped, invalid one-past-end HLOD/skin references fall back to
  a rigid root/pivot, and non-finite UVs become zero only at presentation/export boundaries while
  their immutable exact bits remain preserved. Strict W3D composition remains unchanged.
- Intact bridges no longer treat the complete W3D as a midpoint-scaled static prop. Their endpoint
  marker height, repeated sections, lateral scale, slope, and orientation now follow the dedicated
  bridge presentation path.

- Static W3D meshes now honor the Header3 two-sided flag: ordinary meshes cull back faces while
  explicitly two-sided foliage and planar props retain both sides, removing coplanar backface
  flicker caused by the previous global no-cull policy.

- Version-4 height maps now preserve signed playable-boundary coordinates instead of rejecting
  negative values accepted by the source reader. River staging now honors the stored seam index
  and walks the two perimeter banks in opposite directions, eliminating crossing or bank-only
  ribbons on long rivers.
- Terrain detail now uses a 256-page cache, a slightly farther screen-density threshold, and
  distance cross-fades between 32-, 16-, and 8-texel tiers. Large inward-facing frusta no longer
  consume the complete cache with coarse pages and expose direction-dependent blurry boundaries.
  Keyboard and wheel flight also use frame-rate-independent acceleration and deceleration.
- Generals standing water now starts from the source constructor defaults before ordered base,
  expansion, mod, and map-local INI overrides. This restores its default standing texture instead
  of falling back to a flat diagnostic surface and honors companion `Map.ini` water settings.
- Terrain and water definitions now accumulate every shadowed VFS provider in stable mount order.
  Zero Hour therefore retains inherited Generals terrain classes such as those used by CHI01.
- Source-compatible zero-entry cliff-info tables are accepted as empty instead of rejected,
  allowing affected version-7 maps such as USA07 to load.
- Zero Hour `WaterTransparency` standing and radar colors now accept the source byte-RGB syntax
  and normalize it at the immutable format boundary, allowing maps using the installed profile to
  pass water configuration loading.
- Default legacy water no longer replaces the scene with an opaque procedural gray surface. It
  resolves the source standing-water texture, selected diffuse tint/alpha, additive policy, and
  depth opacity, then alpha-composites with terrain-depth shoreline feathering. The existing
  refractive presentation remains available only under the explicit Modern policy.
- Streamed custom-edge transparency now remains authored coverage instead of being mistaken for a
  missing virtual page. The edge pass composites only albedo and no longer overwrites deferred
  normals or world positions; smooth height-field vertex normals also remove exaggerated
  per-triangle terrain faceting in the interactive viewer.
- Water INI integer RGBA fields now accept the source-established optional alpha channel and
  default omitted alpha to 255, allowing installed vertex-color definitions to load correctly.
- Headless terrain and map-render capture tests now skip when the host exposes no graphics adapter,
  matching the existing synthetic capture policy while preserving real renderer and hash failures.
- Linux and macOS builds no longer retain the Windows-only Steam registry command import.
- Angled terrain views now select virtual-texture detail in camera-space depth and rank projected
  page bounds instead of filling a world-axis square around the viewport footprint. Coarse visible
  coverage is retained before fine upgrades, removing the misplaced rectangular LOD island.

## R2: W3D inspection and viewer

### Added

- Bounded, unknown-preserving W3D chunk inventories with stable nested paths and known
  identifier names.
- `cic-inspect w3d` reports W3D chunk trees through mounted directories and BIG archives.
- Immutable W3D Header3 static geometry decoding with bounded vertex/triangle counts,
  exact record-size checks, static-channel validation, and range-checked triangle indices.
- `cic-inspect w3d-mesh` exact-bit geometry reports through mounted directories and BIG
  archives.
- Bounded W3D material inventories, vertex-material colors, first-pass material IDs, and
  explicit per-vertex diffuse color arrays.
- Bounded W3D fixed-function shader records, texture names/info, per-triangle shader and
  texture assignments, and texture-coordinate arrays.
- Bounded W3D hierarchy, highest-detail HLOD, rigid/skinned mesh composition, and classic
  raw-animation channel decoding, including split skeleton/skin/animation resources.
- `cic-inspect w3d-export` glTF 2.0 export with hierarchy transforms, skins, animation
  clips, first-pass PBR preview materials, UV conversion, and TGA/DDS-to-PNG image
  conversion. It emits one self-contained GLB by default and external glTF with `--gltf`,
  inferring the output name from the W3D resource unless an override is supplied.
- Base-color PNG output preserves decoded RGBA texels and declares the sRGB transfer
  function without applying an additional gamma transform or premultiplying alpha.
- Generals and Zero Hour resource profiles with `--zh`, one-off `--game-dir`, persisted
  installation roots, Steam library discovery, and deterministic base-then-expansion VFS
  layering.
- Missing referenced retail textures produce warned magenta placeholders so geometry and
  animation remain inspectable.
- glTF animation preview maps legacy offscreen attachment-bone hiding to bounded nonsingular
  near-zero-scale states, preventing carried props from expanding animated scene bounds by orders
  of magnitude or producing invalid joint rotations in glTF viewers.
- glTF skinned meshes are scene roots, and alpha cutoff is limited to masked materials, eliminating
  the corresponding Khronos validator findings.
- W3D bone-local skin vertices now use identity glTF inverse binds, fixing separated body parts and
  exploded animated infantry poses.
- Time-coded and adaptive-delta W3D animations now decode under explicit expansion limits and
  export through the same glTF animation path as classic raw clips.
- Vertex-material mapper modes and bounded argument strings, per-pass diffuse illumination and
  specular colors, and validated animated-texture metadata are retained as immutable values.
- GLB/glTF mesh extras preserve every fixed-function pass, texture stage, assignment, shader byte,
  mapper, animated-texture descriptor, and exact UV/scalar bits. All referenced base textures are
  embedded, while the visible metallic-roughness preview remains explicitly pass 0/stage 0.
- W3D `ONE + ONE` additive materials use separate alpha-coverage PNGs in the core-glTF preview,
  eliminating black sprite rectangles while retaining untouched decoded source RGBA images for
  fixed-function metadata consumers.
- Added the `cic-render` boundary with stable W3D geometry staging, a `wgpu` 30 Vulkan/Metal/DX12
  backend, explicit pose inputs, and bounded surface-free RGBA8 capture/readback.
- Added a synthetic translated-triangle capture example and checked-in SHA-256 completion hash.
- Added `cic-inspect w3d-render` for installed profiles or explicit BIG overlays. It composes the
  selected HLOD and hierarchy, stages rigid/one-bone bind geometry, and writes a depth-tested PPM
  plus adapter, geometry-count, and RGBA-hash diagnostics.
- Added `cic-inspect w3d-view` with a 960x720 presentation surface, automatic orthographic fit,
  45-degree elevated camera, continuous Z-up rotation, explicit-frame hierarchy/one-bone animation
  sampling, and Left/Right clip selection. The viewer applies the established bounded hidden-helper
  policy so legacy offscreen attachment sentinels cannot collapse animated model framing.
- Viewer framing is now computed once when a clip is selected; individual animation ticks preserve
  that fixed center and scale, removing per-frame alignment bobbing while Z-up rotation remains
  continuous.
- Added pass-zero/stage-zero W3D material rendering with expanded per-face UVs, source-alpha
  sampling, alpha testing, and opaque, source-alpha, or `ONE + ONE` additive GPU pipelines.
- Added a bounded texture resource manager with stable aliases, SHA-256 RGBA-content deduplication,
  resolved-VFS decode reuse, and effective GPU-material reuse across mesh draw ranges.
- Added stable rendering for every decoded W3D pass and texture stage. Later stages use an explicit
  multiplicative preview while each pass retains its decoded opaque, alpha, or additive blend.
- Added explicit-time CPU sampling for temporal UV mapper arguments, including scrolling, atlas,
  rotation, sine, step, zigzag, deterministic-random, edge, and bump-linear inputs.
- Extended `cic-inspect w3d-render` to resolve deduplicated textures and capture a selected animation
  frame, mapper time, and rotation without reading a clock. The synthetic two-pass/two-stage
  textured animation capture has a checked RGBA SHA-256 completion hash.

## R1: BIG and CSF resource probe

### Added

- Bounded `BIGF`/`BIG4` archive indexing and mounting with stable entry provenance.
- Directory and BIG overlays in `cic-inspect manifest`.
- Bounded CSF localization decoding with complemented UTF-16, optional wave names,
  zero-string labels, and lossless raw names.
- `cic-inspect csf` deterministic localization reports through mounted directories and
  BIG archives.

## R0: Repository and resource-probe foundation

### Added

- Bounded declarative mount profiles and repeatable ordered `--mod` layers for custom bases and
  total conversions, plus lazy directory/BIG providers that index on mount and read only requested
  resources under caller-selected limits.
- Initial GPL-3.0-only repository charter and provenance policy.
- Rust workspace with bounded binary input and deterministic virtual filesystem crates.
- `cic-inspect manifest` command for deterministic loose-directory inventories.
- Synthetic unit and integration tests plus CI quality gates.
