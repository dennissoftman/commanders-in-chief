# Architecture

## Dependency direction

```text
retail files / mods
        |
        v
  cic-vfs <--- archive and directory providers
        |
        v
bounded parsing / immutable format IR
        |                         |
        v                         v
definition database          asset database
        |             |           |
        |             v           |
        |     immutable MAP scene |
        |       description       |
        |             |           |
        +-------------+-----------+
                      |
          +-----------+-------------------+
          |                               |
          v                               v
 presentation resolver          R4 retained UI shell
          |                    (WND, menus, controls,
          |                     map/spawn previews)
          |                               |
          +------------+------------------+
                       v
              renderer / viewers
                       ^
                       |
              immutable render snapshot
                       |
              deterministic simulation
              (R5 runtime activation,
               scripts, commands)
```

The current workspace has five deliberately narrow crates:

- `cic-core`: dependency-free invariants and bounded binary input.
- `cic-formats`: bounded decoders and immutable, renderer-neutral format values.
- `cic-vfs`: normalized paths, providers, overlay order, and asset provenance.
- `cic-render`: stable model staging, bounded texture resources, deterministic
  diagnostic capture, and interactive `wgpu` presentation.
- `cic-tools`: diagnostic applications that compose the public VFS, format, and
  renderer APIs.

R4 will add a narrow `cic-ui` crate for retained UI state, input, and safe navigation while keeping
WND parsing in `cic-formats` and GPU presentation in `cic-render`. Simulation, AI, networking, and
script execution remain excluded until R5. R3 does include bounded
MAP script decoding because scripts are part of the persisted map format; the resulting immutable
tree has no evaluator, callbacks, timers, or access to live engine state.

## Boundaries

- VFS providers expose bytes plus provenance; parsers do not inspect physical paths.
- Disk VFS providers index paths, lengths, and archive ranges only. Callers lazily read one winning
  resource under an explicit allocation bound; the VFS retains no implicit payload cache.
- Built-in retail mount profiles are compatibility presets. Declarative custom profiles and ordered
  mod providers use the same VFS and do not require retail archive names.
- Parsers return immutable semantic values or structured errors.
- Rendering owns GPU/window resources but never parsing, VFS, or simulation state.
- Texture images are bounded and content-addressed; aliases and effective materials reuse
  existing resources without changing stable draw order.
- Tools may format diagnostics but must not contain parsing rules.
- Deterministic behavior is an API property and must be tested at each boundary.

## R3 MAP scene boundary

R3 separates persisted scenario facts from both presentation resolution and runtime behavior:

| Layer | Owns | Must not own |
|---|---|---|
| MAP/INI format layer | Versioned chunks, typed dictionaries, placements, road endpoints, waypoints, sides/teams, build lists, polygon areas, lighting, water inputs, and the nested script tree | VFS lookup, GPU values, live objects, script dispatch, compatibility guesses |
| Presentation resolver | Stable definition lookup, initial draw-state selection, W3D/texture references, terrain-relative transforms, missing-resource diagnostics, and immutable renderer inputs | Authoritative IDs, team activation, gameplay modules, script execution |
| Renderer | GPU terrain/water/road/object resources, culling, batching, shadows/reflections, and explicit-time ambient visual animation | MAP or INI parsing, VFS ownership, simulation mutation, wall-clock diagnostic state |
| R4 UI shell | Retained menu/control state, focus/input, safe callback routing, transitions, layout stack, map/spawn preview bindings, and render-neutral UI frames | MAP-script execution, gameplay objects, networking, arbitrary native callbacks |
| R5 simulation | Runtime IDs, players/teams, spawn assignment, live objects, fixed ticks, script evaluation/actions, commands, RNG, and state hashes | Parser mutation, renderer/window/filesystem ownership |

`ObjectsList` is therefore decoded before any object exists. A placed-object value retains source
order, transform, flags, template name, waypoint fields, and typed property data. Road and bridge
records are a specialized presentation view over those same immutable placements plus bounded
TerrainRoad/TerrainBridge definitions; they are not terrain blend tiles and do not imply collision,
damage, or repair state in R3.

The presentation resolver may select an object's initial drawable state and source-authored ambient
visual animation, including vegetation sway, W3D clips, texture mappers, and animated textures.
Time is an explicit input. This is presentation only: no update module is constructed and no
authoritative state advances when a tree waves or a texture scrolls.

Waypoints named by the established one-based `Player_n_Start` convention become spawn candidates in
the immutable scenario description. `SidesList` side/team/build-list records and player-script
lists are decoded and cross-referenced without assigning controllers or spawning anything. R4 may
display these values in skirmish setup and map selection. R5 may consume them to activate players,
teams, initial objects, and scripts under fixed-tick rules.

## MAP script safety boundary

The script decoder preserves versioned nesting, source order, flags, delays, integer opcode values,
typed parameters, comments, and unknown values. It applies explicit limits to nesting depth,
records, parameters, and strings, and returns structured errors on malformed input. It may emit
stable unresolved-reference diagnostics, but it does not normalize by consulting runtime opcode
tables and does not apply source-era compatibility rewrites implicitly.

Only R5 may map decoded opcodes to executable conditions or actions. That mapping is versioned,
deterministically scheduled, and tested through state hashes and replay. Unsupported opcodes remain
reported or deterministically inert rather than invoking guessed behavior.

## R4 WND and UI boundary

WND is a persisted retained hierarchy, not application code. `cic-formats` decodes file/layout
versions, rectangles and creation resolution, child order, status/style flags, draw/text data,
gadget-specific records, and callback names into immutable values. It does not resolve callback
strings to function pointers, load images/fonts, inspect physical paths, or create live controls.

Modern layout augmentation is also data. A bounded patch is applied as a pure transformation from
one immutable WND definition to another after parse and before UI instantiation. Patches target exact
decorated names, carry preconditions and provenance, and layer in explicit profile/VFS/file order.
They may replace known fields or insert bounded subtrees, but never edit source bytes, execute code,
or make the renderer/menu router search for hardcoded retail window names.

The planned `cic-ui` layer instantiates those definitions into non-authoritative presentation state:
visibility, enablement, focus, text editing, list/slider/combo selection, menu push/pop, transitions,
and typed demo actions. Callback names are resolved only through an explicit allowlist supplied by
the application. Unknown names remain inert diagnostics. Main-menu and skirmish UI state can bind
R3's map catalog, preview, boundaries, and spawn candidates, but pressing Start produces at most a
validated launch description until R5 exists.

Display enumeration and mutation stay outside `cic-ui`. The platform adapter supplies an immutable
monitor/video-mode catalog and accepts a typed requested display preference; the UI owns only the
pending selection and confirmation presentation. The `winit` host and `cic-render` surface backend
apply or roll back window/surface changes and return a result. Deterministic tests inject catalog,
result, and time values instead of enumerating host monitors or reading a clock. Confirmed
window-mode, resolution, refresh-millihertz, and UI-scale preferences are presentation configuration,
not simulation state.

The UI renderer is custom `wgpu` presentation over stable colored/image quads, scissors, borders,
cursors, and Unicode text. This preserves WND's exact retained layout and state-image semantics and
composes naturally over R3 scenes. A full Rust GUI toolkit is not used as the compatibility model.
Focused text technology is encouraged: `cosmic-text` for shaping/layout and `glyphon` for a `wgpu`
atlas after dependency compatibility is verified. Host font discovery is excluded from
deterministic captures; fonts come from explicit synthetic or user-owned VFS resources.

UI input and menu transitions are deterministic presentation events. Tests supply viewport, scale
policy, locale, font set, time, pointer/focus state, and ordered input events explicitly. The UI may
later emit typed R5 commands, but neither the UI model nor renderer may own simulation state.
