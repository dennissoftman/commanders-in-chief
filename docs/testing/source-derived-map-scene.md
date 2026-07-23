# Source-Derived MAP Scene Test Matrix

This matrix records the executable compatibility contract derived from
TheSuperHackers/GeneralsGameCode revision
`9f7abb866f5afd446db14149979e744c7216baaf`. The upstream source is
GPL-3.0-or-later with the Electronic Arts Section 7 additional terms; full notices and permanent
links are in [`../provenance/map.md`](../provenance/map.md).

“Exhaustive” here means every input branch, version boundary, constructor/default value, structure
field, limit, and output primitive represented by the current bounded project model. Runtime-only
legacy state that the immutable parser or renderer does not model is explicitly excluded rather
than copied into unused fields.

| Project boundary | Pinned upstream evidence | Exhaustive executable contract |
| --- | --- | --- |
| Road construction defaults | `W3DRoadBuffer.cpp` constructor and constants | `source_constants_and_empty_constructor_are_exact` checks the empty staged constructor, both radii, 30-degree step, tee adjustment, miter limit, atlas V center, and terrain lift. |
| Road endpoint input | `W3DRoadBuffer::loadRoads` | `every_endpoint_input_failure_has_a_stable_diagnostic` covers non-road records, unexpected/missing/wrong second endpoints, absent/invalid definitions, absent texture fields/resources, and zero length. |
| Road topology output | `insertCurve`, `insert3Way`, `insert4Way`, `doCrossTypeJoins`, `adjustStacking` | `topology_dispatches_curve_miter_tight_tee_y_h_and_four_way`, `topology_noop_and_open_join_inputs_are_stable`, and `material_stacking_matches_every_source_branch` cover regular/tight curves, angled and too-short miters, tee, Y, both slanted-tee orientations, four-way, isolated/straight/overfull no-ops, open alpha caps, and all material stacking relations. |
| Road atlas geometry | `loadCurve`, `loadTee`, `loadY`, `loadH`, `loadAlphaJoin` | `source_atlas_functions_emit_exact_geometry_and_uv_inputs` checks primitive kind, handedness, first top/bottom positions, height lift, and atlas UV values for every emitted primitive. Terrain-fit tests cover tessellation, slope sampling, ordering, stable vertices/indices, degenerate input, and the bounded geometry ceiling. |
| Road GPU policy and controls | `W3DRoadBuffer::loadTexture` plus project-authored diagnostics | `road_texture_inputs_keep_at_most_three_total_levels` covers every mip-chain shape through the three-level cap and malformed image input. `viewer_diagnostic_defaults_and_title_inputs_are_exact` fixes depth-bias values and every wireframe/title availability combination. `terrain_input_constructor_and_every_key_transition_are_exact` covers the zero constructor, every accepted press/release alias, simultaneous opposites, and ignored keys. |
| Initial object presentation | `W3DModelDrawModuleData` constructor and `parseConditionState`; generic INI `End` handling | `constructor_defaults_and_implicit_draw_scale_are_exact` fixes all modeled parser limits and the source implicit scale. `condition_state_inputs_select_only_source_default_presentation` covers both default spellings, first-state rules, aliases/transitions, `NONE`, and non-W3D modules. Scale rejection and every structure limit have dedicated table tests. |
| `BlendTileData` structure | `WorldHeightMap.cpp`, `WorldHeightMap.h`, `TileData.h`, WorldBuilder writers | Offset/record-size assertions fix every persisted table layout. Full decode tests check every accessor. Every-field prefix truncation, missing/duplicate/trailing chunks, signed/zero counts, range checks, flags, non-finite UVs, and every table/name limit are rejected structurally. |
| Blend versions 6–8 | Source version dispatch and cliff readers | `version_six_cliff_threshold_is_exact_at_sixteen_height_units` covers both sides of the derived threshold. `v7_and_v8_cliff_strides_cover_every_byte_boundary` covers widths 1 through 32, all consumed bytes, legacy zero-fill, and corrected rows. Unsupported neighboring versions are rejected. |
| `PolygonTriggers` versions 2–4 | GeneralsMD `PolygonTrigger.cpp` reader | `version_defaults_and_v4_structure_match_the_source_reader` fixes every field and v2/v3 defaults. `nonwater_records_are_skipped_without_renumbering_water_sources` fixes filtering and stable source indices. Every byte prefix, neighboring version, missing/duplicate/trailing structure, signed count, and count/string/retention limit is covered. |

The upstream `W3DModelDrawModuleData` constructor also initializes gameplay/render-runtime fields
such as recoil, particle attachment, power requirements, minimum LOD, dynamic lights, and condition
state slots. They are outside the current initial-model extraction value and are intentionally not
represented or tested here. Likewise, Direct3D buffer ownership and mutable road allocation fields
are replaced by bounded Rust vectors; only their observable empty/default and geometry behavior is
part of this compatibility contract.
