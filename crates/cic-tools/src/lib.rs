//! Stable diagnostic report formatting.

mod gltf;
pub mod resource;
pub mod shell_menu;
pub mod ui_resources;

pub use gltf::{GltfTextureRequest, W3dGlbError, W3dGltfBundle, pack_w3d_glb, render_w3d_gltf};

use std::fmt::Write;

use crate::shell_menu::{ShellMenuActionOutcome, ShellMenuHide, ShellMenuRecord};
use crate::ui_resources::{
    LocalizationResources, MappedImageCatalog, UiResourceBinding, UiResourceKind,
    UiResourceResolution,
};
use cic_formats::{
    CsfFile, LanguageFontRole, MapBlendData, MapDictionary, MapDictionaryValue, MapFile,
    MapHeightField, MapLightingData, MapPolygonData, MapScript, MapScriptAction,
    MapScriptParameterValue, MapSidesData, MapWaterData, MapWorldObjects, OptionsIni,
    PatchedWndDocument, TransitionStyle, UiIniDiagnosticKind, W3dChunk, W3dFile, W3dStaticMesh,
    W3dVector3, WindowTransitionsIni, WndCallbackKind, WndDiagnosticKind, WndDocument,
    WndDrawDataSlot, WndGadgetData, WndPatch, WndPatchOperation, WndWindow, w3d_chunk_name,
};
use cic_render::Capture;
use cic_ui::{
    UI_CALLBACK_SLOTS, UiActionAllowlist, UiCallbackBinding, UiCallbackEdition, UiCallbackSlot,
    UiClipPolicy, UiControlKind, UiDiagnosticKind, UiFrameItem, UiLayout, UiScalePolicy,
    UiScreenId, UiShell, UiShellEvent, UiTransitionDiagnostic, UiTransitionDiagnosticKind,
    UiTransitionDraw, classify_callback_in,
};
use cic_vfs::Vfs;

/// Formats winning VFS entries as deterministic tab-separated records.
#[must_use]
pub fn render_manifest(vfs: &Vfs) -> String {
    let mut output = String::from("path\tbytes\tprovider\n");
    for (path, entry) in vfs.iter_resolved() {
        let provider = entry.provider();
        writeln!(
            output,
            "{}\t{}\t{}:{}",
            path,
            entry.len(),
            provider.kind(),
            provider.name()
        )
        .expect("writing to a String cannot fail");
    }
    output
}

/// Formats a decoded CSF as a deterministic, lossless tab-separated report.
///
/// Labels are ordered by ASCII case-insensitive name with file order as the tie-breaker.
/// Text controls and raw non-ASCII name bytes are escaped so every record occupies one
/// physical line.
#[must_use]
pub fn render_csf(csf: &CsfFile) -> String {
    let header = csf.header();
    let mut output = String::from("version\tlanguage\tlabels\tstrings\n");
    writeln!(
        output,
        "{}\t{}\t{}\t{}",
        header.version(),
        header.language_id(),
        header.label_count(),
        header.string_count()
    )
    .expect("writing to a String cannot fail");
    output.push_str("label\tvariant\ttext\twave\n");

    let mut labels = csf.labels().iter().enumerate().collect::<Vec<_>>();
    labels.sort_by(|(left_index, left), (right_index, right)| {
        ascii_fold(left.name_bytes())
            .cmp(&ascii_fold(right.name_bytes()))
            .then_with(|| left_index.cmp(right_index))
    });

    for (_, label) in labels {
        let name = escape_bytes(label.name_bytes());
        if label.strings().is_empty() {
            writeln!(output, "{name}\t-\t\t").expect("writing to a String cannot fail");
            continue;
        }
        for (variant, string) in label.strings().iter().enumerate() {
            let text = escape_text(string.text());
            let wave = string
                .wave_name_bytes()
                .map_or_else(String::new, escape_bytes);
            writeln!(output, "{name}\t{variant}\t{text}\t{wave}")
                .expect("writing to a String cannot fail");
        }
    }
    output
}

/// Formats a decoded `Options.ini` as a deterministic `field\tvalue` report, followed by any
/// unrecognized fields as diagnostics. Fields absent from the file are omitted rather than shown
/// as empty, so the report only lists what was actually read.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn render_options_ini(options: &OptionsIni) -> String {
    let mut output = String::from("field\tvalue\n");
    if let Some((width, height)) = options.resolution() {
        writeln!(output, "Resolution\t{width} {height}").expect("writing to a String cannot fail");
    }
    if let Some(raw) = options.antialiasing_raw() {
        let samples = options.antialiasing_msaa_samples().unwrap_or(0);
        writeln!(output, "AntiAliasing\t{raw} ({samples}x MSAA)")
            .expect("writing to a String cannot fail");
    }
    write_option(&mut output, "Gamma", options.gamma());
    write_option(&mut output, "MusicVolume", options.music_volume());
    write_option(&mut output, "SFXVolume", options.sfx_volume());
    write_option(&mut output, "SFX3DVolume", options.sfx3d_volume());
    write_option(&mut output, "VoiceVolume", options.voice_volume());
    write_option(&mut output, "ScrollFactor", options.scroll_factor());
    write_option(
        &mut output,
        "MaxParticleCount",
        options.max_particle_count(),
    );
    write_option(&mut output, "TextureReduction", options.texture_reduction());
    write_option(
        &mut output,
        "CampaignDifficulty",
        options.campaign_difficulty(),
    );
    write_option(&mut output, "FirewallBehavior", options.firewall_behavior());
    write_option(
        &mut output,
        "FirewallPortOverride",
        options.firewall_port_override(),
    );
    write_option_bytes(&mut output, "IPAddress", options.ip_address_bytes());
    write_option_bytes(
        &mut output,
        "GameSpyIPAddress",
        options.gamespy_ip_address_bytes(),
    );
    write_option_bytes(
        &mut output,
        "IdealStaticGameLOD",
        options.ideal_static_game_lod_bytes(),
    );
    write_option_bytes(
        &mut output,
        "StaticGameLOD",
        options.static_game_lod_bytes(),
    );
    write_option(&mut output, "LanguageFilter", options.language_filter());
    write_option(&mut output, "SendDelay", options.send_delay());
    write_option(
        &mut output,
        "UseAlternateMouse",
        options.use_alternate_mouse(),
    );
    write_option(
        &mut output,
        "DrawScrollAnchor",
        options.draw_scroll_anchor(),
    );
    write_option(
        &mut output,
        "MoveScrollAnchor",
        options.move_scroll_anchor(),
    );
    write_option(
        &mut output,
        "BuildingOcclusion",
        options.building_occlusion(),
    );
    write_option(&mut output, "DynamicLOD", options.dynamic_lod());
    write_option(&mut output, "ExtraAnimations", options.extra_animations());
    write_option(&mut output, "HeatEffects", options.heat_effects());
    write_option(&mut output, "Retaliation", options.retaliation());
    write_option(
        &mut output,
        "ShowSoftWaterEdge",
        options.show_soft_water_edge(),
    );
    write_option(&mut output, "ShowTrees", options.show_trees());
    write_option(&mut output, "UseCloudMap", options.use_cloud_map());
    write_option(
        &mut output,
        "UseDoubleClickAttackMove",
        options.use_double_click_attack_move(),
    );
    write_option(&mut output, "UseLightMap", options.use_light_map());
    write_option(&mut output, "UseShadowDecals", options.use_shadow_decals());
    write_option(
        &mut output,
        "UseShadowVolumes",
        options.use_shadow_volumes(),
    );

    output.push_str("diagnostic_line\tunrecognized_field\n");
    for diagnostic in options.diagnostics() {
        writeln!(
            output,
            "{}\t{}",
            diagnostic.line(),
            escape_bytes(diagnostic.field_bytes())
        )
        .expect("writing to a String cannot fail");
    }
    output
}

fn write_option(output: &mut String, field: &str, value: Option<impl std::fmt::Display>) {
    if let Some(value) = value {
        writeln!(output, "{field}\t{value}").expect("writing to a String cannot fail");
    }
}

fn write_option_bytes(output: &mut String, field: &str, value: Option<&[u8]>) {
    if let Some(value) = value {
        writeln!(output, "{field}\t{}", escape_bytes(value))
            .expect("writing to a String cannot fail");
    }
}

/// Formats a MAP symbol table and top-level chunk stream as a stable inventory.
#[must_use]
pub fn render_map(map: &MapFile) -> String {
    let mut output = format!("compression\t{}\n", map.compression());
    output.push_str("symbol\toffset\tid\tname\n");
    for (index, symbol) in map.symbols().iter().enumerate() {
        writeln!(
            output,
            "{}\t{}\t0x{:08X}\t{}",
            index,
            symbol.offset(),
            symbol.id(),
            escape_bytes(symbol.name_bytes())
        )
        .expect("writing to a String cannot fail");
    }
    output.push_str("chunk\toffset\tid\tversion\tpayload\tname\n");
    for (index, chunk) in map.chunks().iter().enumerate() {
        let name = map
            .symbol_name(chunk.id())
            .map_or_else(|| "unknown".to_owned(), escape_bytes);
        writeln!(
            output,
            "{}\t{}\t0x{:08X}\t{}\t{}\t{}",
            index,
            chunk.offset(),
            chunk.id(),
            chunk.version(),
            chunk.data().len(),
            name
        )
        .expect("writing to a String cannot fail");
    }
    output
}

/// Formats decoded MAP terrain heights in stable row-major order.
///
/// # Panics
///
/// Panics only if a validated MAP width cannot fit the current platform's address size.
#[must_use]
pub fn render_map_height(height: &MapHeightField) -> String {
    let mut output =
        String::from("version\twidth\theight\tborder\tcell_size\tboundaries\tsamples\n");
    writeln!(
        output,
        "{}\t{}\t{}\t{}\t{}\t{}\t{}",
        height.version(),
        height.width(),
        height.height(),
        height.border_size(),
        height.cell_size_world_units(),
        height.boundaries().len(),
        height.samples().len()
    )
    .expect("writing to a String cannot fail");
    output.push_str("boundary\tx\ty\n");
    for (index, boundary) in height.boundaries().iter().enumerate() {
        writeln!(output, "{}\t{}\t{}", index, boundary.x(), boundary.y())
            .expect("writing to a String cannot fail");
    }
    output.push_str("sample\tx\ty\tvalue\n");
    let width = usize::try_from(height.width()).expect("validated MAP width fits usize");
    for (index, sample) in height.samples().iter().enumerate() {
        writeln!(
            output,
            "{}\t{}\t{}\t{}",
            index,
            index % width,
            index / width,
            sample
        )
        .expect("writing to a String cannot fail");
    }
    output
}

/// Encodes MAP height samples as a deterministic 8-bit grayscale PNG in stored row order.
///
/// Height samples are scalar data, so the PNG carries no sRGB or gamma declaration.
///
/// # Errors
///
/// Returns a PNG encoding error if the validated dimensions or sample stream cannot be encoded.
pub fn encode_map_height_png(height: &MapHeightField) -> Result<Vec<u8>, png::EncodingError> {
    let mut output = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut output, height.width(), height.height());
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(height.samples())?;
    }
    Ok(output)
}

/// Encodes a headless renderer capture as an sRGB RGBA8 PNG.
///
/// # Errors
///
/// Returns a PNG encoding error if the validated capture cannot be encoded.
pub fn encode_capture_png(capture: &Capture) -> Result<Vec<u8>, png::EncodingError> {
    capture.png()
}

/// Formats decoded MAP blend, edge, and cliff values in stable source order.
///
/// # Panics
///
/// Panics only if validated MAP dimensions cannot fit the current platform's address size.
#[must_use]
pub fn render_map_blend(blend: &MapBlendData) -> String {
    let mut output = String::from(
        "version\twidth\theight\tcells\tbitmap_tiles\tblended_tiles\tcliff_info\ttexture_classes\tedge_tiles\tedge_texture_classes\tcliff_stride\n",
    );
    writeln!(
        output,
        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        blend.version(),
        blend.width(),
        blend.height(),
        blend.tile_indices().len(),
        blend.bitmap_tile_count(),
        blend.blended_tile_count(),
        blend.cliff_info_count(),
        blend.texture_classes().len(),
        blend.edge_tile_count(),
        blend.edge_texture_classes().len(),
        blend.cliff_flag_stride()
    )
    .expect("writing to a String cannot fail");

    render_map_blend_cells(&mut output, blend);
    render_map_texture_classes(&mut output, blend);
    render_map_blend_tiles(&mut output, blend);
    render_map_cliff_info(&mut output, blend);
    output
}

/// Formats complete polygon data in stable source and point order.
#[must_use]
pub fn render_map_polygons(polygons: &MapPolygonData) -> String {
    let point_count = polygons
        .areas()
        .iter()
        .map(|area| area.points().len())
        .sum::<usize>();
    let water_count = polygons
        .areas()
        .iter()
        .filter(|area| area.is_water())
        .count();
    let river_count = polygons
        .areas()
        .iter()
        .filter(|area| area.is_river())
        .count();
    let mut output =
        String::from("version\tpolygon_areas\tpolygon_points\twater_areas\triver_areas\n");
    writeln!(
        output,
        "{}\t{}\t{}\t{}\t{}",
        polygons.version(),
        polygons.areas().len(),
        point_count,
        water_count,
        river_count
    )
    .expect("writing to a String cannot fail");
    output.push_str("area\tsource_index\tid\twater\triver\triver_start\tpoints\tname\tlayer\n");
    for (index, area) in polygons.areas().iter().enumerate() {
        writeln!(
            output,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            index,
            area.source_index(),
            area.trigger_id(),
            u8::from(area.is_water()),
            u8::from(area.is_river()),
            area.river_start(),
            area.points().len(),
            escape_bytes(area.name_bytes()),
            escape_bytes(area.layer_name_bytes())
        )
        .expect("writing to a String cannot fail");
        for (point_index, point) in area.points().iter().enumerate() {
            let [x, y, z] = point.coordinates();
            writeln!(output, "point\t{index}\t{point_index}\t{x}\t{y}\t{z}")
                .expect("writing to a String cannot fail");
        }
    }
    output
}

/// Formats water-only polygon data in stable source order.
#[must_use]
pub fn render_map_water(water: &MapWaterData) -> String {
    let point_count = water
        .areas()
        .iter()
        .map(|area| area.points().len())
        .sum::<usize>();
    let mut output = String::from("version\tsource_triggers\twater_areas\twater_points\n");
    writeln!(
        output,
        "{}\t{}\t{}\t{}",
        water.version(),
        water.source_trigger_count(),
        water.areas().len(),
        point_count
    )
    .expect("writing to a String cannot fail");
    output.push_str("area\tsource_index\tid\triver\triver_start\tpoints\tname\n");
    for (index, area) in water.areas().iter().enumerate() {
        writeln!(
            output,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            index,
            area.source_index(),
            area.trigger_id(),
            u8::from(area.is_river()),
            area.river_start(),
            area.points().len(),
            escape_bytes(area.name_bytes())
        )
        .expect("writing to a String cannot fail");
        for (point_index, point) in area.points().iter().enumerate() {
            let [x, y, z] = point.coordinates();
            writeln!(output, "point\t{index}\t{point_index}\t{x}\t{y}\t{z}")
                .expect("writing to a String cannot fail");
        }
    }
    output
}

/// Formats separate terrain/object MAP lights in stable time and source-light order.
#[must_use]
pub fn render_map_lighting(lighting: &MapLightingData) -> String {
    const NAMES: [&str; 4] = ["morning", "afternoon", "evening", "night"];
    let shadow = lighting
        .shadow_color()
        .map_or_else(|| "none".to_owned(), |color| format!("0x{color:08X}"));
    let mut output = String::from("version\tselected_time\tshadow_color\n");
    writeln!(
        output,
        "{}\t{}\t{}",
        lighting.version(),
        lighting.selected_time().name(),
        shadow
    )
    .expect("writing to a String cannot fail");
    output.push_str(
        "period\ttime\tset\tlight\tambient_r\tambient_g\tambient_b\tdiffuse_r\tdiffuse_g\tdiffuse_b\tdirection_x\tdirection_y\tdirection_z\n",
    );
    for (period_index, (period, name)) in lighting.periods().iter().zip(NAMES).enumerate() {
        for (set_name, lights) in [
            ("terrain", period.terrain_lights()),
            ("objects", period.object_lights()),
        ] {
            for (light_index, light) in lights.iter().enumerate() {
                let ambient = light.ambient().map(float_bits);
                let diffuse = light.diffuse().map(float_bits);
                let direction = light.direction().map(float_bits);
                writeln!(
                    output,
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    period_index,
                    name,
                    set_name,
                    light_index,
                    ambient[0],
                    ambient[1],
                    ambient[2],
                    diffuse[0],
                    diffuse[1],
                    diffuse[2],
                    direction[0],
                    direction[1],
                    direction[2]
                )
                .expect("writing to a String cannot fail");
            }
        }
    }
    output
}

/// Formats immutable world metadata, placements, waypoints, starts, and endpoint flags.
#[must_use]
pub fn render_map_world_objects(world: &MapWorldObjects) -> String {
    let mut output = String::from(
        "world_version\tobjects_version\tobjects\tunknown_object_children\tplayer_starts\n",
    );
    writeln!(
        output,
        "{}\t{}\t{}\t{}\t{}",
        world.world().version(),
        world.objects_version(),
        world.objects().len(),
        world.unknown_object_children().len(),
        world.player_starts().count()
    )
    .expect("writing to a String cannot fail");
    render_dictionary(&mut output, "world", 0, world.world().properties());
    output.push_str(
        "object\tversion\tx\ty\tz\tangle\tflags\tmirror\tdont_render\twaypoint_id\twaypoint_name\tplayer_start\tname\n",
    );
    for object in world.objects() {
        let position = object.position().map(float_bits);
        let waypoint_id = object
            .waypoint_id()
            .map_or_else(String::new, |value| value.to_string());
        let waypoint_name = object
            .waypoint_name_bytes()
            .map_or_else(String::new, escape_bytes);
        let player_start = object
            .player_start_number()
            .map_or_else(String::new, |value| value.to_string());
        writeln!(
            output,
            "{}\t{}\t{}\t{}\t{}\t{}\t0x{:08X}\t{}\t{}\t{}\t{}\t{}\t{}",
            object.placement_id(),
            object.version(),
            position[0],
            position[1],
            position[2],
            float_bits(object.angle()),
            object.flags(),
            u8::from(object.flags() & cic_formats::object_flags::DRAWS_IN_MIRROR != 0),
            u8::from(object.flags() & cic_formats::object_flags::DONT_RENDER != 0),
            waypoint_id,
            waypoint_name,
            player_start,
            escape_bytes(object.name_bytes())
        )
        .expect("writing to a String cannot fail");
        render_dictionary(
            &mut output,
            "object",
            usize::try_from(object.placement_id()).unwrap_or(usize::MAX),
            object.properties(),
        );
    }
    output
}

/// Formats sides, teams, build lists, and the nested script tree strictly as data.
#[must_use]
pub fn render_map_sides(sides: &MapSidesData) -> String {
    let script_lists = sides
        .player_scripts()
        .map_or(0, |scripts| scripts.lists().len());
    let mut output = String::from("version\tsides\tteams\tplayer_script_lists\tunknown_children\n");
    writeln!(
        output,
        "{}\t{}\t{}\t{}\t{}",
        sides.version(),
        sides.sides().len(),
        sides.teams().len(),
        script_lists,
        sides.unknown_children().len()
    )
    .expect("writing to a String cannot fail");
    for (side_index, side) in sides.sides().iter().enumerate() {
        render_dictionary(&mut output, "side", side_index, side.properties());
        for (build_index, build) in side.build_list().iter().enumerate() {
            let position = build.position().map(float_bits);
            writeln!(
                output,
                "build\t{side_index}\t{build_index}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                escape_bytes(build.building_name_bytes()),
                escape_bytes(build.template_name_bytes()),
                position[0],
                position[1],
                position[2],
                float_bits(build.angle()),
                build.initially_built_raw(),
                build.rebuild_count(),
                build.script_bytes().map_or_else(String::new, escape_bytes),
                build.health().map_or_else(String::new, |value| value.to_string()),
                build.whiner_raw().map_or_else(String::new, |value| value.to_string()),
                build.unsellable_raw().map_or_else(String::new, |value| value.to_string()),
                build.repairable_raw().map_or_else(String::new, |value| value.to_string()),
            )
            .expect("writing to a String cannot fail");
        }
    }
    for (team_index, team) in sides.teams().iter().enumerate() {
        render_dictionary(&mut output, "team", team_index, team);
    }
    if let Some(player_scripts) = sides.player_scripts() {
        for (list_index, list) in player_scripts.lists().iter().enumerate() {
            writeln!(
                output,
                "script_list\t{list_index}\t{}\t{}\t{}\t{}",
                list.version(),
                list.scripts().len(),
                list.groups().len(),
                list.unknown_children().len()
            )
            .expect("writing to a String cannot fail");
            for (script_index, script) in list.scripts().iter().enumerate() {
                render_script(&mut output, list_index, None, script_index, script);
            }
            for (group_index, group) in list.groups().iter().enumerate() {
                writeln!(
                    output,
                    "script_group\t{list_index}\t{group_index}\t{}\t{}\t{}\t{}\t{}",
                    group.version(),
                    escape_bytes(group.name_bytes()),
                    group.active_raw(),
                    group
                        .subroutine_raw()
                        .map_or_else(String::new, |value| value.to_string()),
                    group.scripts().len()
                )
                .expect("writing to a String cannot fail");
                for (script_index, script) in group.scripts().iter().enumerate() {
                    render_script(
                        &mut output,
                        list_index,
                        Some(group_index),
                        script_index,
                        script,
                    );
                }
            }
        }
    }
    output
}

fn render_dictionary(output: &mut String, scope: &str, owner: usize, dictionary: &MapDictionary) {
    for (index, entry) in dictionary.entries().iter().enumerate() {
        let key = entry
            .key_name_bytes()
            .map_or_else(|| format!("unknown:0x{:06X}", entry.key_id()), escape_bytes);
        let (kind, value) = dictionary_value(entry.value());
        writeln!(
            output,
            "dict\t{scope}\t{owner}\t{index}\t0x{:06X}\t{key}\t{kind}\t{value}",
            entry.key_id()
        )
        .expect("writing to a String cannot fail");
    }
}

fn dictionary_value(value: &MapDictionaryValue) -> (&'static str, String) {
    match value {
        MapDictionaryValue::Bool(value) => ("bool", value.to_string()),
        MapDictionaryValue::Int(value) => ("int", value.to_string()),
        MapDictionaryValue::Real(value) => ("real", float_bits(*value)),
        MapDictionaryValue::Ascii(value) => ("ascii", escape_bytes(value)),
        MapDictionaryValue::Unicode(value) => (
            "unicode",
            value
                .iter()
                .map(|unit| format!("{unit:04X}"))
                .collect::<Vec<_>>()
                .join(","),
        ),
    }
}

fn render_script(
    output: &mut String,
    list: usize,
    group: Option<usize>,
    script_index: usize,
    script: &MapScript,
) {
    let group = group.map_or_else(|| "-".to_owned(), |value| value.to_string());
    writeln!(
        output,
        "script\t{list}\t{group}\t{script_index}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        script.version(),
        escape_bytes(script.name_bytes()),
        escape_bytes(script.comment_bytes()),
        escape_bytes(script.condition_comment_bytes()),
        escape_bytes(script.action_comment_bytes()),
        script.active_raw(),
        script.one_shot_raw(),
        script.easy_raw(),
        script.normal_raw(),
        script.hard_raw(),
        script.subroutine_raw(),
        script.evaluation_delay_seconds().map_or_else(String::new, |value| value.to_string()),
        script.or_conditions().len(),
        script.actions().len(),
        script.false_actions().len(),
    )
    .expect("writing to a String cannot fail");
    for (or_index, or_condition) in script.or_conditions().iter().enumerate() {
        for (condition_index, condition) in or_condition.conditions().iter().enumerate() {
            writeln!(
                output,
                "condition\t{list}\t{group}\t{script_index}\t{or_index}\t{condition_index}\t{}\t{}\t{}",
                condition.version(),
                condition.opcode(),
                condition.parameters().len()
            )
            .expect("writing to a String cannot fail");
            render_parameters(output, "condition_parameter", condition.parameters());
        }
    }
    for (branch, actions) in [
        ("true", script.actions()),
        ("false", script.false_actions()),
    ] {
        for (action_index, action) in actions.iter().enumerate() {
            render_action(
                output,
                list,
                &group,
                script_index,
                branch,
                action_index,
                action,
            );
        }
    }
}

fn render_action(
    output: &mut String,
    list: usize,
    group: &str,
    script: usize,
    branch: &str,
    index: usize,
    action: &MapScriptAction,
) {
    writeln!(
        output,
        "action\t{list}\t{group}\t{script}\t{branch}\t{index}\t{}\t{}\t{}",
        action.version(),
        action.opcode(),
        action.parameters().len()
    )
    .expect("writing to a String cannot fail");
    render_parameters(output, "action_parameter", action.parameters());
}

fn render_parameters(
    output: &mut String,
    record: &str,
    parameters: &[cic_formats::MapScriptParameter],
) {
    for (index, parameter) in parameters.iter().enumerate() {
        match parameter.value() {
            MapScriptParameterValue::Coordinate(position) => {
                let position = position.map(float_bits);
                writeln!(
                    output,
                    "{record}\t{index}\t{}\tcoord\t{}\t{}\t{}",
                    parameter.parameter_type(),
                    position[0],
                    position[1],
                    position[2]
                )
                .expect("writing to a String cannot fail");
            }
            MapScriptParameterValue::Scalar {
                integer,
                real,
                string,
            } => {
                writeln!(
                    output,
                    "{record}\t{index}\t{}\tscalar\t{}\t{}\t{}",
                    parameter.parameter_type(),
                    integer,
                    float_bits(*real),
                    escape_bytes(string)
                )
                .expect("writing to a String cannot fail");
            }
        }
    }
}

fn render_map_blend_cells(output: &mut String, blend: &MapBlendData) {
    output.push_str("cell\tx\ty\ttile\tblend\textra_blend\tcliff_info\tcliff_flag\n");
    let width = usize::try_from(blend.width()).expect("validated MAP width fits usize");
    for index in 0..blend.tile_indices().len() {
        let x = index % width;
        let y = index / width;
        let cliff = blend
            .is_cliff(
                u32::try_from(x).expect("validated X fits u32"),
                u32::try_from(y).expect("validated Y fits u32"),
            )
            .expect("row-major cell is in range");
        writeln!(
            output,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            index,
            x,
            y,
            blend.tile_indices()[index],
            blend.blend_indices()[index],
            blend.extra_blend_indices()[index],
            blend.cliff_info_indices()[index],
            u8::from(cliff)
        )
        .expect("writing to a String cannot fail");
    }
}

fn render_map_texture_classes(output: &mut String, blend: &MapBlendData) {
    output.push_str("texture\tkind\tfirst\tcount\twidth\tlegacy\tname\n");
    for (kind, classes) in [
        ("terrain", blend.texture_classes()),
        ("edge", blend.edge_texture_classes()),
    ] {
        for (index, class) in classes.iter().enumerate() {
            let legacy = class
                .legacy()
                .map_or_else(String::new, |value| value.to_string());
            writeln!(
                output,
                "{}\t{}\t{}\t{}\t{}\t{}\t{}",
                index,
                kind,
                class.first_tile(),
                class.tile_count(),
                class.width(),
                legacy,
                escape_bytes(class.name_bytes())
            )
            .expect("writing to a String cannot fail");
        }
    }
}

fn render_map_blend_tiles(output: &mut String, blend: &MapBlendData) {
    output.push_str(
        "blend\tblend_index\thorizontal\tvertical\tright_diagonal\tleft_diagonal\tinverted\tlong_diagonal\tcustom_edge_class\n",
    );
    for tile in blend.blend_tiles() {
        writeln!(
            output,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            tile.table_index(),
            tile.blend_index(),
            tile.horizontal(),
            tile.vertical(),
            tile.right_diagonal(),
            tile.left_diagonal(),
            tile.inverted(),
            tile.long_diagonal(),
            tile.custom_edge_class()
        )
        .expect("writing to a String cannot fail");
    }
}

fn render_map_cliff_info(output: &mut String, blend: &MapBlendData) {
    output.push_str("cliff\ttile\tu0\tv0\tu1\tv1\tu2\tv2\tu3\tv3\tflip\tmutant\n");
    for cliff in blend.cliff_info() {
        let uv = cliff.uv().map(float_bits);
        writeln!(
            output,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            cliff.table_index(),
            cliff.tile_index(),
            uv[0],
            uv[1],
            uv[2],
            uv[3],
            uv[4],
            uv[5],
            uv[6],
            uv[7],
            cliff.flip(),
            cliff.mutant()
        )
        .expect("writing to a String cannot fail");
    }
}

/// Formats a W3D chunk tree as a stable, depth-first tab-separated inventory.
#[must_use]
pub fn render_w3d(w3d: &W3dFile) -> String {
    let mut output = String::from("path\tdepth\toffset\tid\tkind\tpayload\tname\n");
    let mut path = Vec::new();
    render_w3d_level(&mut output, w3d.chunks(), &mut path);
    output
}

/// Formats immutable static mesh geometry with exact floating-point bit patterns.
#[must_use]
pub fn render_w3d_mesh(mesh: &W3dStaticMesh) -> String {
    let header = mesh.header();
    let mesh_name = escape_bytes(fixed_name(header.mesh_name_bytes()));
    let container_name = escape_bytes(fixed_name(header.container_name_bytes()));
    let mut output = String::from(
        "version\tattributes\tmesh\tcontainer\tvertices\ttriangles\tmaterials\tdamage_stages\tsort_level\tprelit\tvertex_channels\tface_channels\n",
    );
    writeln!(
        output,
        "0x{:08X}\t0x{:08X}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t0x{:08X}\t0x{:08X}\t0x{:08X}",
        header.version(),
        header.attributes(),
        mesh_name,
        container_name,
        header.vertex_count(),
        header.triangle_count(),
        header.material_count(),
        header.damage_stage_count(),
        header.sort_level(),
        header.prelit_version(),
        header.vertex_channels(),
        header.face_channels()
    )
    .expect("writing to a String cannot fail");

    output.push_str("bound\tx\ty\tz\tradius\n");
    render_bound(&mut output, "minimum", header.minimum(), None);
    render_bound(&mut output, "maximum", header.maximum(), None);
    render_bound(
        &mut output,
        "sphere",
        header.sphere_center(),
        Some(header.sphere_radius()),
    );

    output.push_str("vertex\tx\ty\tz\tnx\tny\tnz\n");
    for (index, (vertex, normal)) in mesh.vertices().iter().zip(mesh.normals()).enumerate() {
        writeln!(
            output,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            index,
            float_bits(vertex.x()),
            float_bits(vertex.y()),
            float_bits(vertex.z()),
            float_bits(normal.x()),
            float_bits(normal.y()),
            float_bits(normal.z())
        )
        .expect("writing to a String cannot fail");
    }

    output.push_str("triangle\tv0\tv1\tv2\tattributes\tnx\tny\tnz\tdistance\n");
    for (index, triangle) in mesh.triangles().iter().enumerate() {
        let vertices = triangle.vertex_indices();
        let normal = triangle.normal();
        writeln!(
            output,
            "{}\t{}\t{}\t{}\t0x{:08X}\t{}\t{}\t{}\t{}",
            index,
            vertices[0],
            vertices[1],
            vertices[2],
            triangle.attributes(),
            float_bits(normal.x()),
            float_bits(normal.y()),
            float_bits(normal.z()),
            float_bits(triangle.distance())
        )
        .expect("writing to a String cannot fail");
    }
    output
}

fn render_bound(output: &mut String, name: &str, value: W3dVector3, radius: Option<f32>) {
    let radius = radius.map_or_else(String::new, float_bits);
    writeln!(
        output,
        "{}\t{}\t{}\t{}\t{}",
        name,
        float_bits(value.x()),
        float_bits(value.y()),
        float_bits(value.z()),
        radius
    )
    .expect("writing to a String cannot fail");
}

fn float_bits(value: f32) -> String {
    format!("0x{:08X}", value.to_bits())
}

fn fixed_name(bytes: &[u8; 16]) -> &[u8] {
    let length = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    &bytes[..length]
}

/// Formats a decoded WND document as a stable, source-order inventory report.
///
/// Every row is tagged by kind in its first column (`top_level_field`, `window`,
/// `window_field`, `diagnostic`) so the hierarchy and every generically retained field or
/// non-fatal diagnostic are all visible from one report, in source order.
#[must_use]
pub fn render_wnd(document: &WndDocument) -> String {
    let mut output = format!("file_version\t{}\n", document.file_version());
    if let Some(layout) = document.layout() {
        writeln!(output, "layout_init\t{}", layout.init().unwrap_or("(none)"))
            .expect("writing to a String cannot fail");
        writeln!(
            output,
            "layout_update\t{}",
            layout.update().unwrap_or("(none)")
        )
        .expect("writing to a String cannot fail");
        writeln!(
            output,
            "layout_shutdown\t{}",
            layout.shutdown().unwrap_or("(none)")
        )
        .expect("writing to a String cannot fail");
    }
    output.push_str("top_level_field\tname\tvalue\tline\n");
    for field in document.top_level_fields() {
        writeln!(
            output,
            "top_level_field\t{}\t{}\t{}",
            field.name(),
            field.raw_value(),
            field.line()
        )
        .expect("writing to a String cannot fail");
    }
    output.push_str(
        "window\tpath\tdepth\tid\tname\twindow_type\tupper_left_x\tupper_left_y\tbottom_right_x\tbottom_right_y\tcreation_width\tcreation_height\n",
    );
    output.push_str("window_field\tpath\tname\tvalue\tline\n");
    output.push_str("window_flag\tpath\tfield\tname\tknown\n");
    output.push_str("window_callback\tpath\tkind\tname\n");
    output.push_str("window_property\tpath\tproperty\tvalue\n");
    output.push_str("window_font\tpath\tname\tsize\tbold\n");
    output.push_str("window_text_color\tpath\tstate\tred\tgreen\tblue\talpha\n");
    output.push_str(
        "window_draw_entry\tpath\tslot\tindex\timage\tred\tgreen\tblue\talpha\tborder_red\tborder_green\tborder_blue\tborder_alpha\n",
    );
    output.push_str("window_gadget_data\tpath\tgadget\tproperty\tvalue\n");
    let mut path = Vec::new();
    for (index, window) in document.windows().iter().enumerate() {
        path.push(index);
        render_wnd_window(&mut output, window, &mut path);
        path.pop();
    }
    output.push_str("diagnostic\tline\twindow_id\tkind\tdetail\n");
    for diagnostic in document.diagnostics() {
        let window_id = diagnostic
            .window_id()
            .map_or_else(|| "-".to_owned(), |id| id.to_string());
        let (kind, detail) = match diagnostic.kind() {
            WndDiagnosticKind::UnknownField { name } => ("unknown_field", name.to_string()),
            WndDiagnosticKind::UnrecognizedValue { field, value } => {
                ("unrecognized_value", format!("{field}={value}"))
            }
            WndDiagnosticKind::MissingChildKeyword => ("missing_child_keyword", "-".to_owned()),
            WndDiagnosticKind::MalformedField { field, reason } => {
                ("malformed_field", format!("{field}: {reason}"))
            }
            WndDiagnosticKind::DuplicateWindowName {
                name,
                first_window_id,
            } => (
                "duplicate_window_name",
                format!("{name} (first declared by window {first_window_id})"),
            ),
        };
        writeln!(
            output,
            "diagnostic\t{}\t{window_id}\t{kind}\t{detail}",
            diagnostic.line()
        )
        .expect("writing to a String cannot fail");
    }
    output
}

/// Formats a retained layout instantiated for one viewport: the control tree with resolved
/// rectangles and live state, the tab order, the frame submission order, and any diagnostics.
///
/// Rows carry names, geometry, and counts. This is the report an acceptance check compares when a
/// modded layout changes, without rendering it.
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "one straight-line report writer per row family; splitting it would separate each \
              header from the rows it describes"
)]
pub fn render_ui_layout(layout: &UiLayout) -> String {
    let presentation = layout.presentation();
    let mut output = String::from("ui_layout\twidth\theight\tscale\tcontrols\ttab_stops\n");
    writeln!(
        output,
        "ui_layout\t{}\t{}\t{}\t{}\t{}",
        presentation.viewport.width(),
        presentation.viewport.height(),
        match presentation.scale {
            UiScalePolicy::Classic => "classic",
            UiScalePolicy::Modern => "modern",
        },
        layout.controls().len(),
        layout.tab_order().len()
    )
    .expect("writing to a String cannot fail");

    output.push_str(
        "ui_control\tid\tdepth\tparent\tname\ttype\tx\ty\twidth\theight\tscreen_x\tscreen_y\t\
         hidden\tenabled\tstatus\tkind\trole\ttext\n",
    );
    for control in layout.controls() {
        let origin = layout.screen_origin(control.id());
        let rect = control.rect();
        writeln!(
            output,
            "ui_control\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:#010x}\t{}\t{}\t{}",
            control.id().index(),
            control.depth(),
            control
                .parent()
                .map_or_else(|| "-".to_owned(), |parent| parent.index().to_string()),
            control.name().unwrap_or("-"),
            control.window_type(),
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            origin.x,
            origin.y,
            control.is_hidden(),
            control.is_enabled(),
            control.status().bits(),
            ui_control_kind_name(control.kind()),
            control
                .gadget_role()
                .map_or("-", cic_ui::UiGadgetRole::name),
            control.text_label().unwrap_or("-")
        )
        .expect("writing to a String cannot fail");
    }

    output.push_str("ui_tab_stop\torder\tid\tname\n");
    for (order, id) in layout.tab_order().iter().enumerate() {
        writeln!(
            output,
            "ui_tab_stop\t{order}\t{}\t{}",
            id.index(),
            layout.control(*id).name().unwrap_or("-")
        )
        .expect("writing to a String cannot fail");
    }

    output.push_str("ui_frame_item\torder\tkind\tid\tdetail\n");
    for (order, item) in layout.frame(UiClipPolicy::None).items().iter().enumerate() {
        let (kind, id, detail) = match item {
            UiFrameItem::PushClip { rect } => (
                "push_clip",
                "-".to_owned(),
                format!("{}x{}+{}+{}", rect.width, rect.height, rect.x, rect.y),
            ),
            UiFrameItem::PopClip => ("pop_clip", "-".to_owned(), "-".to_owned()),
            UiFrameItem::Quad {
                control,
                slot,
                images,
                family,
                ..
            } => (
                "quad",
                control.index().to_string(),
                format!(
                    "{}\t{}\t{}",
                    wnd_draw_slot_name(*slot),
                    images.image(*slot, 0).unwrap_or("-"),
                    family.name()
                ),
            ),
            UiFrameItem::Text(run) => ("text", run.control.index().to_string(), run.label.clone()),
        };
        writeln!(output, "ui_frame_item\t{order}\t{kind}\t{id}\t{detail}")
            .expect("writing to a String cannot fail");
    }

    output.push_str("ui_layout_diagnostic\tid\tkind\tdetail\n");
    for diagnostic in layout.diagnostics() {
        let (kind, detail) = match diagnostic.kind() {
            UiDiagnosticKind::UnmappedStatus { name } => ("unmapped_status", name.to_string()),
            UiDiagnosticKind::InvertedSliderBounds { minimum, maximum } => {
                ("inverted_slider_bounds", format!("{minimum}..{maximum}"))
            }
            UiDiagnosticKind::ListRowsClamped { declared, applied } => {
                ("list_rows_clamped", format!("{declared} to {applied}"))
            }
            UiDiagnosticKind::TextLengthClamped { declared, applied } => {
                ("text_length_clamped", format!("{declared} to {applied}"))
            }
            UiDiagnosticKind::UntitledScrollBarAssumed => (
                "untitled_scroll_bar_assumed",
                "scroll bar laid out without a title inset".to_owned(),
            ),
        };
        writeln!(
            output,
            "ui_layout_diagnostic\t{}\t{kind}\t{detail}",
            diagnostic.control().index()
        )
        .expect("writing to a String cannot fail");
    }
    output
}

fn ui_control_kind_name(kind: &UiControlKind) -> &'static str {
    match kind {
        UiControlKind::PushButton => "push_button",
        UiControlKind::RadioButton { .. } => "radio_button",
        UiControlKind::CheckBox { .. } => "check_box",
        UiControlKind::Slider { .. } => "slider",
        UiControlKind::ListBox { .. } => "list_box",
        UiControlKind::ComboBox { .. } => "combo_box",
        UiControlKind::TextEntry { .. } => "text_entry",
        UiControlKind::StaticText { .. } => "static_text",
        UiControlKind::ProgressBar { .. } => "progress_bar",
        UiControlKind::TabControl { .. } => "tab_control",
        UiControlKind::Generic => "generic",
    }
}

/// Formats UI resource resolution for one layout: which definition files loaded, which demands
/// bound to which definitions, and which did not resolve.
///
/// No retail definition content is embedded: rows carry names, virtual paths, and counts, which is
/// what a compatibility check needs.
#[must_use]
pub fn render_ui_resources(
    catalog: &MappedImageCatalog,
    localization: &LocalizationResources,
    resolution: &UiResourceResolution,
) -> String {
    let mut output = String::new();
    render_ui_language(&mut output, localization);
    render_ui_definition_files(&mut output, catalog, localization);
    render_ui_fonts(&mut output, localization);
    render_ui_bindings(&mut output, resolution);
    render_ui_definition_diagnostics(&mut output, catalog, localization);
    output
}

fn render_ui_language(output: &mut String, localization: &LocalizationResources) {
    output.push_str("ui_language\tname\tfont_size_method\tfont_adjustment\tlabels\n");
    let text = localization.text();
    writeln!(
        output,
        "ui_language\t{}\t{}\t{}\t{}",
        localization.language(),
        text.resolution_font_size_method().name(),
        text.resolution_font_adjustment(),
        localization.labels().map_or_else(
            || "-".to_owned(),
            |(path, csf)| format!("{path}:{}", csf.labels().len())
        )
    )
    .expect("writing to a String cannot fail");
}

fn render_ui_definition_files(
    output: &mut String,
    catalog: &MappedImageCatalog,
    localization: &LocalizationResources,
) {
    output.push_str("ui_definition_file\tkind\tpath\tdefinitions\n");
    for file in catalog.files() {
        writeln!(
            output,
            "ui_definition_file\tmapped_image\t{}\t{}",
            file.path(),
            file.definitions()
        )
        .expect("writing to a String cannot fail");
    }
    for file in localization.text_files() {
        writeln!(
            output,
            "ui_definition_file\tlanguage\t{}\t{}",
            file.path(),
            file.definitions()
        )
        .expect("writing to a String cannot fail");
    }
    for (path, ini) in localization.header_template_files() {
        writeln!(
            output,
            "ui_definition_file\theader_template\t{path}\t{}",
            ini.templates().len()
        )
        .expect("writing to a String cannot fail");
    }

    output.push_str("ui_unselected_file\tpath\n");
    for path in catalog.unselected_files() {
        writeln!(output, "ui_unselected_file\t{path}").expect("writing to a String cannot fail");
    }

    output.push_str("ui_definition_override\tname\tsuperseded\twinner\n");
    for entry in catalog.overrides() {
        writeln!(
            output,
            "ui_definition_override\t{}\t{}\t{}",
            String::from_utf8_lossy(entry.name_bytes()),
            entry.previous(),
            entry.winner()
        )
        .expect("writing to a String cannot fail");
    }
}

fn render_ui_fonts(output: &mut String, localization: &LocalizationResources) {
    let text = localization.text();
    output.push_str("ui_font_role\trole\tfamily\tsize\tbold\tdeclared\n");
    for role in LanguageFontRole::ALL {
        let font = text.font(role);
        writeln!(
            output,
            "ui_font_role\t{}\t{}\t{}\t{}\t{}",
            role.field_name(),
            String::from_utf8_lossy(font.name_bytes()),
            font.size(),
            font.bold(),
            font.is_declared()
        )
        .expect("writing to a String cannot fail");
    }
    output.push_str("ui_font_file\tname\n");
    for file in text.local_font_files() {
        writeln!(output, "ui_font_file\t{}", String::from_utf8_lossy(file))
            .expect("writing to a String cannot fail");
    }
}

fn render_ui_bindings(output: &mut String, resolution: &UiResourceResolution) {
    output.push_str("ui_summary\tkind\tresolved\tmissing\n");
    for kind in [
        UiResourceKind::MappedImage,
        UiResourceKind::Font,
        UiResourceKind::HeaderTemplate,
        UiResourceKind::Label,
    ] {
        let (resolved, missing) = resolution.counts(kind);
        writeln!(
            output,
            "ui_summary\t{}\t{resolved}\t{missing}",
            kind.row_name()
        )
        .expect("writing to a String cannot fail");
    }

    output.push_str("ui_resource\tkind\tname\tstatus\tsites\tdetail\n");
    output.push_str("ui_resource_site\tkind\tname\twindow_id\twindow\trecord\n");
    for resource in resolution.resources() {
        let demand = resource.demand();
        let (status, detail) = ui_binding_row(resource.binding());
        writeln!(
            output,
            "ui_resource\t{}\t{}\t{status}\t{}\t{detail}",
            demand.kind().row_name(),
            demand.name(),
            demand.sites().len()
        )
        .expect("writing to a String cannot fail");
        for site in demand.sites() {
            writeln!(
                output,
                "ui_resource_site\t{}\t{}\t{}\t{}\t{}",
                demand.kind().row_name(),
                demand.name(),
                site.window_id(),
                site.window_name().unwrap_or("-"),
                site.detail()
            )
            .expect("writing to a String cannot fail");
        }
    }
}

fn ui_binding_row(binding: &UiResourceBinding) -> (&'static str, String) {
    match binding {
        UiResourceBinding::Image {
            definition,
            texture,
            texture_path,
            size,
        } => (
            if texture_path.is_some() {
                "resolved"
            } else {
                "texture_missing"
            },
            format!(
                "{definition}\t{texture}\t{}\t{}x{}",
                texture_path
                    .as_ref()
                    .map_or_else(|| "-".to_owned(), ToString::to_string),
                size.0,
                size.1
            ),
        ),
        UiResourceBinding::HeaderTemplate {
            definition,
            font,
            point,
            bold,
        } => ("resolved", format!("{definition}\t{font}\t{point}\t{bold}")),
        UiResourceBinding::Font {
            role,
            local_font_file,
        } => (
            "resolved",
            format!(
                "{}\t{}",
                role.unwrap_or("unicode_font_name"),
                local_font_file.as_deref().unwrap_or("-")
            ),
        ),
        UiResourceBinding::Label { definition } => ("resolved", definition.to_string()),
        UiResourceBinding::Missing => ("missing", "-".to_owned()),
    }
}

fn render_ui_definition_diagnostics(
    output: &mut String,
    catalog: &MappedImageCatalog,
    localization: &LocalizationResources,
) {
    output.push_str("ui_definition_diagnostic\tpath\tline\tkind\tdetail\n");
    let files = catalog
        .files()
        .iter()
        .chain(localization.text_files().iter());
    for file in files {
        for diagnostic in file.diagnostics() {
            let (kind, detail) = ui_ini_diagnostic_row(diagnostic.kind());
            writeln!(
                output,
                "ui_definition_diagnostic\t{}\t{}\t{kind}\t{detail}",
                file.path(),
                diagnostic.line()
            )
            .expect("writing to a String cannot fail");
        }
    }
    for (path, ini) in localization.header_template_files() {
        for diagnostic in ini.diagnostics() {
            let (kind, detail) = ui_ini_diagnostic_row(diagnostic.kind());
            writeln!(
                output,
                "ui_definition_diagnostic\t{path}\t{}\t{kind}\t{detail}",
                diagnostic.line()
            )
            .expect("writing to a String cannot fail");
        }
    }
}

fn ui_ini_diagnostic_row(kind: &UiIniDiagnosticKind) -> (&'static str, String) {
    match kind {
        UiIniDiagnosticKind::UnknownBlock { keyword } => ("unknown_block", keyword.to_string()),
        UiIniDiagnosticKind::UnknownField { field } => ("unknown_field", field.to_string()),
        UiIniDiagnosticKind::MalformedField { field, reason } => {
            ("malformed_field", format!("{field}: {reason}"))
        }
        UiIniDiagnosticKind::DuplicateDefinition { name, first_line } => (
            "duplicate_definition",
            format!("{name} first declared on line {first_line}"),
        ),
    }
}

/// Formats every retained callback name in one layout with what the original's function lexicon
/// would have resolved it to.
///
/// This is the compatibility view of R4's callback boundary: names are data here, so the interesting
/// question is which of them the original could have dispatched at all. A name reported `unknown` is
/// inert in this project and was inert in the original too, because no table carried it. `edition`
/// selects which build's lexicon answers, since Zero Hour registers six names base Generals does
/// not.
#[must_use]
pub fn render_ui_callbacks(path: &str, edition: UiCallbackEdition, layout: &UiLayout) -> String {
    let mut output = String::from("ui_callback_file\tpath\tcontrols\tnames\tunknown\n");
    let mut rows: Vec<(UiCallbackSlot, String, Option<usize>, Option<String>)> = Vec::new();
    for slot in [
        UiCallbackSlot::LayoutInit,
        UiCallbackSlot::LayoutUpdate,
        UiCallbackSlot::LayoutShutdown,
    ] {
        let name = match slot {
            UiCallbackSlot::LayoutInit => layout.layout_init_callback(),
            UiCallbackSlot::LayoutUpdate => layout.layout_update_callback(),
            _ => layout.layout_shutdown_callback(),
        };
        if let Some(name) = name {
            rows.push((slot, name.to_owned(), None, None));
        }
    }
    for control in layout.controls() {
        for slot in [
            UiCallbackSlot::System,
            UiCallbackSlot::Input,
            UiCallbackSlot::Tooltip,
            UiCallbackSlot::Draw,
        ] {
            let name = match slot {
                UiCallbackSlot::System => control.system_callback(),
                UiCallbackSlot::Input => control.input_callback(),
                UiCallbackSlot::Tooltip => control.tooltip_callback(),
                _ => control.draw_callback(),
            };
            if let Some(name) = name {
                rows.push((
                    slot,
                    name.to_owned(),
                    Some(control.id().index()),
                    control.name().map(str::to_owned),
                ));
            }
        }
    }
    let unknown = rows
        .iter()
        .filter(|(slot, name, _, _)| {
            classify_callback_in(edition, *slot, name) == UiCallbackBinding::Unknown
        })
        .count();
    writeln!(
        output,
        "ui_callback_file\t{path}\t{}\t{}\t{unknown}",
        layout.controls().len(),
        rows.len()
    )
    .expect("writing to a String cannot fail");

    output.push_str("ui_callback\tslot\trecord\tname\tbinding\ttable\tcontrol_id\tcontrol\n");
    for (slot, name, control_id, control_name) in &rows {
        let binding = classify_callback_in(edition, *slot, name);
        let table = match binding {
            UiCallbackBinding::Established { table } => table.row_name(),
            UiCallbackBinding::None | UiCallbackBinding::Unknown => "-",
        };
        writeln!(
            output,
            "ui_callback\t{}\t{}\t{name}\t{}\t{table}\t{}\t{}",
            slot.row_name(),
            slot.record_name(),
            binding.row_name(),
            control_id.map_or_else(|| "-".to_owned(), |id| id.to_string()),
            control_name.as_deref().unwrap_or("-")
        )
        .expect("writing to a String cannot fail");
    }

    output.push_str("ui_callback_summary\tslot\tnames\testablished\tnone\tunknown\n");
    for slot in UI_CALLBACK_SLOTS {
        let mut names = 0_usize;
        let mut established = 0_usize;
        let mut placeholder = 0_usize;
        let mut missing = 0_usize;
        for (candidate, name, _, _) in &rows {
            if *candidate != slot {
                continue;
            }
            names += 1;
            match classify_callback_in(edition, slot, name) {
                UiCallbackBinding::Established { .. } => established += 1,
                UiCallbackBinding::None => placeholder += 1,
                UiCallbackBinding::Unknown => missing += 1,
            }
        }
        writeln!(
            output,
            "ui_callback_summary\t{}\t{names}\t{established}\t{placeholder}\t{missing}",
            slot.row_name()
        )
        .expect("writing to a String cannot fail");
    }
    output
}

/// Formats the result of a scripted shell navigation: the events each step produced, then the
/// resulting stack, draw order, and per-screen visibility.
#[must_use]
pub fn render_ui_shell(steps: &[(String, Vec<UiShellEvent>)], shell: &UiShell) -> String {
    let mut output = String::from("ui_shell_step\tindex\tcommand\tevents\n");
    for (index, (command, events)) in steps.iter().enumerate() {
        writeln!(
            output,
            "ui_shell_step\t{index}\t{command}\t{}",
            events.len()
        )
        .expect("writing to a String cannot fail");
    }

    output.push_str("ui_shell_event\tstep\tkind\tscreen\tdetail\n");
    for (index, (_, events)) in steps.iter().enumerate() {
        for event in events {
            let (kind, screen, detail) = ui_shell_event_row(event);
            writeln!(
                output,
                "ui_shell_event\t{index}\t{kind}\t{screen}\t{detail}"
            )
            .expect("writing to a String cannot fail");
        }
    }

    output.push_str("ui_shell_screen\tindex\tpath\thidden\tcontrols\tdraw_position\n");
    let draw_order = shell.draw_order();
    for (index, screen) in shell.screens().iter().enumerate() {
        let position = draw_order
            .iter()
            .position(|candidate| candidate.index() == index);
        writeln!(
            output,
            "ui_shell_screen\t{index}\t{}\t{}\t{}\t{}",
            screen.path(),
            screen.layout().is_hidden(),
            screen.layout().controls().len(),
            position.map_or_else(|| "-".to_owned(), |position| position.to_string())
        )
        .expect("writing to a String cannot fail");
    }

    output.push_str("ui_shell_state\tscreens\ttop\thidden\tpending\n");
    writeln!(
        output,
        "ui_shell_state\t{}\t{}\t{}\t{}",
        shell.screen_count(),
        shell.top().map_or("-", cic_ui::UiScreen::path),
        shell.is_hidden(),
        shell.is_operation_pending()
    )
    .expect("writing to a String cannot fail");
    output
}

/// Formats a scripted menu session: what each step did, then the allowlist and the resulting stack.
///
/// `steps` pairs each step's spec with the range of records it produced, so a reader can attribute
/// every record to the input that caused it without the records carrying a step index.
#[must_use]
pub fn render_ui_menu(
    steps: &[(String, std::ops::Range<usize>)],
    records: &[ShellMenuRecord],
    allowlist: &UiActionAllowlist,
    shell: &UiShell,
) -> String {
    let mut output = String::from("ui_menu_step\tindex\tcommand\trecords\n");
    for (index, (command, range)) in steps.iter().enumerate() {
        writeln!(output, "ui_menu_step\t{index}\t{command}\t{}", range.len())
            .expect("writing to a String cannot fail");
    }

    // Records before the first step belong to opening the screen, which no step asked for.
    let step_of = |record: usize| {
        steps
            .iter()
            .position(|(_, range)| range.contains(&record))
            .map_or_else(|| "open".to_owned(), |index| index.to_string())
    };
    output.push_str("ui_menu_record\tstep\tkind\tsubject\tdetail\n");
    for (index, record) in records.iter().enumerate() {
        let step = step_of(index);
        let (kind, subject, detail) = ui_menu_record_row(record);
        writeln!(
            output,
            "ui_menu_record\t{step}\t{kind}\t{subject}\t{detail}"
        )
        .expect("writing to a String cannot fail");
    }

    // The initial hidden set is where the `initialHide` redundancy shows, so it is counted rather
    // than left for a reader to tally.
    let mut hidden = 0;
    let mut already = 0;
    let mut missing = 0;
    for record in records {
        if let ShellMenuRecord::InitialHide { outcome, .. } = record {
            match outcome {
                ShellMenuHide::Hidden => hidden += 1,
                ShellMenuHide::AlreadyHidden => already += 1,
                ShellMenuHide::Missing => missing += 1,
            }
        }
    }
    output.push_str("ui_menu_initial_hide\thidden\talready_hidden\tmissing\n");
    writeln!(
        output,
        "ui_menu_initial_hide\t{hidden}\t{already}\t{missing}"
    )
    .expect("writing to a String cannot fail");

    output.push_str("ui_menu_binding\tcontrol\tindex\taction\tdetail\n");
    for (control, actions) in allowlist.entries() {
        for (index, action) in actions.iter().enumerate() {
            writeln!(
                output,
                "ui_menu_binding\t{control}\t{index}\t{}\t{}",
                action.row_name(),
                blank_as_dash(&action.row_detail())
            )
            .expect("writing to a String cannot fail");
        }
    }

    let routed = records
        .iter()
        .filter(|record| matches!(record, ShellMenuRecord::Action { .. }))
        .count();
    let unrouted = records
        .iter()
        .filter(|record| matches!(record, ShellMenuRecord::Unrouted { .. }))
        .count();
    let captures = records
        .iter()
        .filter(|record| matches!(record, ShellMenuRecord::Capture { .. }))
        .count();
    output.push_str("ui_menu_summary\tscreens\ttop\trouted_actions\tunrouted\tcaptures\n");
    writeln!(
        output,
        "ui_menu_summary\t{}\t{}\t{routed}\t{unrouted}\t{captures}",
        shell.screen_count(),
        shell.top().map_or("-", cic_ui::UiScreen::path)
    )
    .expect("writing to a String cannot fail");
    output
}

fn ui_menu_record_row(record: &ShellMenuRecord) -> (&'static str, String, String) {
    match record {
        ShellMenuRecord::InitialHide { control, outcome } => (
            "initial_hide",
            control.clone(),
            outcome.row_name().to_owned(),
        ),
        ShellMenuRecord::Shell(event) => {
            let (kind, screen, detail) = ui_shell_event_row(event);
            ("shell", format!("{kind}@{screen}"), detail)
        }
        ShellMenuRecord::Input {
            screen,
            control,
            kind,
            detail,
        } => (
            "input",
            format!("{}@{screen}", control.as_deref().unwrap_or("-")),
            format!("{kind} {}", blank_as_dash(detail)),
        ),
        ShellMenuRecord::FirstInput => (
            "first_input",
            "-".to_owned(),
            "MainMenuInput reveals the default panel".to_owned(),
        ),
        ShellMenuRecord::Action {
            control,
            action,
            outcome,
        } => (
            "action",
            control.clone(),
            format!(
                "{} {} {}",
                action.row_name(),
                blank_as_dash(&action.row_detail()),
                match outcome {
                    ShellMenuActionOutcome::Refused(reason) => format!("refused={reason}"),
                    other => other.row_name().to_owned(),
                }
            ),
        ),
        ShellMenuRecord::Unrouted { control, callback } => (
            "unrouted",
            control.as_deref().unwrap_or("-").to_owned(),
            format!("callback={}", callback.as_deref().unwrap_or("-")),
        ),
        ShellMenuRecord::Transition {
            frames,
            group,
            finished,
            diagnostics,
        } => (
            "transition",
            group.as_deref().unwrap_or("-").to_owned(),
            format!("frames={frames} finished={finished} diagnostics={diagnostics}"),
        ),
        ShellMenuRecord::Capture {
            path,
            width,
            height,
            sha256,
            quads,
            batches,
            text_runs,
            diagnostics,
        } => (
            "capture",
            path.clone(),
            format!(
                "{width}x{height} {sha256} quads={quads} batches={batches} text_runs={text_runs} diagnostics={diagnostics}"
            ),
        ),
        ShellMenuRecord::CaptureDiagnostic {
            item,
            control,
            kind,
            detail,
        } => (
            "capture_diagnostic",
            format!(
                "{}@item{item}",
                control.map_or_else(|| "-".to_owned(), |control| control.to_string())
            ),
            format!("{kind} {}", blank_as_dash(detail)),
        ),
    }
}

fn blank_as_dash(value: &str) -> String {
    if value.is_empty() {
        "-".to_owned()
    } else {
        value.to_owned()
    }
}

fn ui_shell_event_row(event: &UiShellEvent) -> (&'static str, String, String) {
    let screen_text = |screen: UiScreenId| screen.index().to_string();
    match event {
        UiShellEvent::ScreenPushed { screen, path } => {
            ("screen_pushed", screen_text(*screen), path.clone())
        }
        UiShellEvent::ScreenPopped { path } => ("screen_popped", "-".to_owned(), path.clone()),
        UiShellEvent::LayoutInit {
            screen,
            callback,
            binding,
        } => (
            "layout_init",
            screen_text(*screen),
            ui_callback_detail(callback.as_deref(), *binding),
        ),
        UiShellEvent::LayoutUpdate {
            screen,
            callback,
            binding,
        } => (
            "layout_update",
            screen_text(*screen),
            ui_callback_detail(callback.as_deref(), *binding),
        ),
        UiShellEvent::LayoutShutdown {
            screen,
            callback,
            binding,
            immediate,
        } => (
            "layout_shutdown",
            screen_text(*screen),
            format!(
                "{}\timmediate={immediate}",
                ui_callback_detail(callback.as_deref(), *binding)
            ),
        ),
        UiShellEvent::BroughtForward { screen } => {
            ("brought_forward", screen_text(*screen), String::new())
        }
        UiShellEvent::VisibilityChanged { hidden } => (
            "visibility_changed",
            "-".to_owned(),
            format!("hidden={hidden}"),
        ),
    }
}

fn ui_callback_detail(callback: Option<&str>, binding: Option<UiCallbackBinding>) -> String {
    match (callback, binding) {
        (Some(name), Some(binding)) => format!("{name}\t{}", binding.row_name()),
        _ => "-\t-".to_owned(),
    }
}

/// One group's outcome from a transition sweep.
#[derive(Debug, Clone)]
pub struct TransitionRunOutcome {
    /// The group's name.
    pub group: String,
    /// How many windows it declares.
    pub windows: usize,
    /// How many of those name a window at all; the two window-less styles name none.
    pub named: usize,
    /// How many resolved to a control in a loaded layout, counting the window-less blocks.
    pub resolved: usize,
    /// The layouts its windows name, in the order first seen.
    pub layouts: Vec<String>,
    /// The frame the group declared it would finish on, after arming.
    pub declared_frames: i32,
    /// How many frames were actually stepped before it reported finished.
    pub stepped_frames: i32,
    /// Whether it reported finished within the frame budget.
    pub finished: bool,
    /// How many draw records it produced across the run.
    pub draws: usize,
    /// Every observation, deduplicated by kind and window.
    pub diagnostics: Vec<TransitionRunNote>,
}

/// One deduplicated observation from a transition sweep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionRunNote {
    /// The decorated window name it belongs to.
    pub window: String,
    /// A stable kind name.
    pub kind: &'static str,
    /// The detail, or a dash.
    pub detail: String,
    /// How many times it occurred.
    pub count: usize,
}

/// Formats the outcome of running every transition group, one row per group.
///
/// This is the compatibility view of the transition runtime: whether each retail group's windows
/// resolve in the layouts they name, whether it runs to completion, and what it draws on the way.
#[must_use]
pub fn render_transition_run(path: &str, outcomes: &[TransitionRunOutcome]) -> String {
    let mut output =
        String::from("ui_transition_run\tpath\tgroups\tunfinished\twindows\tnamed\tunresolved\n");
    let windows: usize = outcomes.iter().map(|outcome| outcome.windows).sum();
    let named: usize = outcomes.iter().map(|outcome| outcome.named).sum();
    let resolved: usize = outcomes.iter().map(|outcome| outcome.resolved).sum();
    let unfinished = outcomes.iter().filter(|outcome| !outcome.finished).count();
    writeln!(
        output,
        "ui_transition_run\t{path}\t{}\t{unfinished}\t{windows}\t{named}\t{}",
        outcomes.len(),
        windows - resolved
    )
    .expect("writing to a String cannot fail");

    output.push_str(
        "ui_transition_group_run\tgroup\twindows\tnamed\tresolved\tdeclared_frames\t\
         stepped_frames\tfinished\tdraws\tlayouts\n",
    );
    for outcome in outcomes {
        writeln!(
            output,
            "ui_transition_group_run\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            outcome.group,
            outcome.windows,
            outcome.named,
            outcome.resolved,
            outcome.declared_frames,
            outcome.stepped_frames,
            outcome.finished,
            outcome.draws,
            if outcome.layouts.is_empty() {
                "-".to_owned()
            } else {
                outcome.layouts.join(",")
            }
        )
        .expect("writing to a String cannot fail");
    }

    output.push_str("ui_transition_note\tgroup\twindow\tkind\tcount\tdetail\n");
    for outcome in outcomes {
        for note in &outcome.diagnostics {
            writeln!(
                output,
                "ui_transition_note\t{}\t{}\t{}\t{}\t{}",
                outcome.group, note.window, note.kind, note.count, note.detail
            )
            .expect("writing to a String cannot fail");
        }
    }
    output
}

/// Collapses a run's observations into stable, counted rows.
#[must_use]
pub fn summarize_transition_diagnostics(
    diagnostics: &[UiTransitionDiagnostic],
) -> Vec<TransitionRunNote> {
    let mut rows: Vec<TransitionRunNote> = Vec::new();
    for diagnostic in diagnostics {
        let (kind, detail) = match diagnostic.kind() {
            UiTransitionDiagnosticKind::WindowNotFound { name } => {
                ("window_not_found", name.to_string())
            }
            UiTransitionDiagnosticKind::CompanionNotFound { name } => {
                ("companion_not_found", name.to_string())
            }
            UiTransitionDiagnosticKind::UnsupportedDraw { style, reason } => {
                ("unsupported_draw", format!("{}: {reason}", style.name()))
            }
            UiTransitionDiagnosticKind::NeverFinishes {
                style,
                declared_length,
                armed_length,
            } => (
                "never_finishes",
                format!(
                    "{}: armed for {armed_length} frames but finishes only on state {declared_length}",
                    style.name()
                ),
            ),
            UiTransitionDiagnosticKind::AudioCue { event } => ("audio_cue", event.to_string()),
        };
        let window = diagnostic.window().to_owned();
        match rows
            .iter_mut()
            .find(|row| row.window == window && row.kind == kind && row.detail == detail)
        {
            Some(row) => row.count += 1,
            None => rows.push(TransitionRunNote {
                window,
                kind,
                detail,
                count: 1,
            }),
        }
    }
    rows
}

/// Formats one transition draw as a stable row detail.
#[must_use]
pub fn transition_draw_row(draw: &UiTransitionDraw) -> (&'static str, String) {
    match draw {
        UiTransitionDraw::Rect {
            rect,
            fill,
            outline,
            outline_width,
        } => (
            "rect",
            format!(
                "{},{} {}x{}\tfill={}\toutline={}\twidth={outline_width}",
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                fill.map_or_else(|| "-".to_owned(), render_color),
                outline.map_or_else(|| "-".to_owned(), render_color)
            ),
        ),
        UiTransitionDraw::ControlImage {
            target,
            slot,
            entry,
            rect,
            color,
        } => (
            "control_image",
            format!(
                "screen={} control={} slot={slot:?} entry={entry} {},{} {}x{}\tcolor={}",
                target.screen.index(),
                target.control.index(),
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                render_color(*color)
            ),
        ),
        UiTransitionDraw::NamedImage { image, rect, color } => (
            "named_image",
            format!(
                "{image} {},{} {}x{}\tcolor={}",
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                render_color(*color)
            ),
        ),
        UiTransitionDraw::PushButtonPieces { target, alpha } => (
            "push_button_pieces",
            format!(
                "screen={} control={} alpha={alpha}",
                target.screen.index(),
                target.control.index()
            ),
        ),
        UiTransitionDraw::TypedText { target, text } => (
            "typed_text",
            format!(
                "screen={} control={} {text:?}",
                target.screen.index(),
                target.control.index()
            ),
        ),
    }
}

fn render_color(color: cic_formats::WndColor) -> String {
    let [red, green, blue, alpha] = color.channels();
    format!("{red} {green} {blue} {alpha}")
}

/// Formats decoded transition groups as a deterministic tab-separated inventory.
///
/// Rows carry group and window names, the established style each window runs, and frame counts,
/// which is what a compatibility check of a user-owned or modded `WindowTransitions.ini` needs. The
/// style census reports how much of the vocabulary a file actually exercises.
#[must_use]
pub fn render_window_transitions(path: &str, ini: &WindowTransitionsIni) -> String {
    let mut output = String::from("ui_transition_file\tpath\tgroups\twindows\tdiagnostics\n");
    let windows: usize = ini.groups().iter().map(|group| group.windows().len()).sum();
    writeln!(
        output,
        "ui_transition_file\t{path}\t{}\t{windows}\t{}",
        ini.groups().len(),
        ini.diagnostics().len()
    )
    .expect("writing to a String cannot fail");

    output.push_str("ui_transition_group\tname\tline\tfire_once\twindows\ttotal_frames\n");
    output
        .push_str("ui_transition_window\tgroup\tindex\twindow\tstyle\tframe_delay\ttotal_frames\n");
    for group in ini.groups() {
        let name = String::from_utf8_lossy(group.name_bytes());
        writeln!(
            output,
            "ui_transition_group\t{name}\t{}\t{}\t{}\t{}",
            group.line(),
            group.fire_once(),
            group.windows().len(),
            group.total_frames()
        )
        .expect("writing to a String cannot fail");
        for (index, window) in group.windows().iter().enumerate() {
            writeln!(
                output,
                "ui_transition_window\t{name}\t{index}\t{}\t{}\t{}\t{}",
                render_optional_bytes(window.window_name_bytes()),
                window.style().name(),
                window.frame_delay(),
                window.total_frames()
            )
            .expect("writing to a String cannot fail");
        }
    }

    output.push_str("ui_transition_style\tstyle\tframe_length\twindows\tgroups\n");
    for style in TRANSITION_STYLES {
        let mut window_count = 0_usize;
        let mut group_count = 0_usize;
        for group in ini.groups() {
            let declared = group
                .windows()
                .iter()
                .filter(|window| window.style() == style)
                .count();
            window_count += declared;
            if declared > 0 {
                group_count += 1;
            }
        }
        writeln!(
            output,
            "ui_transition_style\t{}\t{}\t{window_count}\t{group_count}",
            style.name(),
            style.declared_frame_length()
        )
        .expect("writing to a String cannot fail");
    }

    output.push_str("ui_transition_diagnostic\tline\tkind\tdetail\n");
    for diagnostic in ini.diagnostics() {
        let (kind, detail) = ui_ini_diagnostic_row(diagnostic.kind());
        writeln!(
            output,
            "ui_transition_diagnostic\t{}\t{kind}\t{detail}",
            diagnostic.line()
        )
        .expect("writing to a String cannot fail");
    }
    output
}

/// Every established transition style, in `TransitionStyleNames` order.
const TRANSITION_STYLES: [TransitionStyle; 15] = [
    TransitionStyle::Flash,
    TransitionStyle::ButtonFlash,
    TransitionStyle::WinFade,
    TransitionStyle::WinScaleUp,
    TransitionStyle::MainMenuScaleUp,
    TransitionStyle::TypeText,
    TransitionStyle::ScreenFade,
    TransitionStyle::CountUp,
    TransitionStyle::FullFade,
    TransitionStyle::TextOnFrame,
    TransitionStyle::MainMenuMediumScaleUp,
    TransitionStyle::MainMenuSmallScaleDown,
    TransitionStyle::ControlBarArrow,
    TransitionStyle::ScoreScaleUp,
    TransitionStyle::ReverseSound,
];

/// Renders raw name bytes, or a dash when the definition declared none.
fn render_optional_bytes(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        "-".to_owned()
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

/// Formats the effect of applied patch overlays: what each patch declared, what it wrote,
/// and the resulting hierarchy. The source WND is never rewritten.
#[must_use]
pub fn render_wnd_patch(patches: &[WndPatch], result: &PatchedWndDocument) -> String {
    let mut output = String::from("patch\tname\ttarget\tversion\toperations\n");
    for patch in patches {
        writeln!(
            output,
            "patch\t{}\t{}\t{}\t{}",
            patch.name(),
            patch.target(),
            patch.version(),
            patch.steps().len()
        )
        .expect("writing to a String cannot fail");
    }
    output.push_str("operation\tpatch\tline\tkind\tcontrol\tdetail\n");
    for patch in patches {
        for step in patch.steps() {
            let (kind, control, detail) = match step.operation() {
                WndPatchOperation::RequireWindow { control } => {
                    ("require-window", &**control, String::new())
                }
                WndPatchOperation::RequireField {
                    control,
                    field,
                    value,
                } => ("require-field", &**control, format!("{field}={value}")),
                WndPatchOperation::SetField {
                    control,
                    field,
                    value,
                } => ("set-field", &**control, format!("{field}={value}")),
                WndPatchOperation::AddField {
                    control,
                    field,
                    value,
                } => ("add-field", &**control, format!("{field}={value}")),
                WndPatchOperation::SetRect { control, rect } => {
                    let (left, top) = rect.upper_left();
                    let (right, bottom) = rect.bottom_right();
                    let (width, height) = rect.creation_resolution();
                    (
                        "set-rect",
                        &**control,
                        format!("{left} {top} {right} {bottom} {width} {height}"),
                    )
                }
                WndPatchOperation::Reorder { control, index } => {
                    ("reorder", &**control, index.to_string())
                }
                WndPatchOperation::Reparent {
                    control,
                    parent,
                    index,
                } => ("reparent", &**control, format!("{parent} at {index}")),
                WndPatchOperation::InsertWindow { parent, index, .. } => {
                    ("insert-window", &**parent, format!("at {index}"))
                }
            };
            writeln!(
                output,
                "operation\t{}\t{}\t{kind}\t{control}\t{detail}",
                patch.name(),
                step.line()
            )
            .expect("writing to a String cannot fail");
        }
    }
    output.push_str("provenance\tcontrol\tfield\tpatch\tline\n");
    for record in result.provenance() {
        writeln!(
            output,
            "provenance\t{}\t{}\t{}\t{}",
            record.control(),
            record.field(),
            record.patch(),
            record.line()
        )
        .expect("writing to a String cannot fail");
    }
    output.push_str(&render_wnd(result.document()));
    output
}

fn render_wnd_window(output: &mut String, window: &WndWindow, path: &mut Vec<usize>) {
    let path_text = path
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join("/");
    let rect = window.rect();
    let (upper_left_x, upper_left_y) = rect.upper_left();
    let (bottom_right_x, bottom_right_y) = rect.bottom_right();
    let (creation_width, creation_height) = rect.creation_resolution();
    writeln!(
        output,
        "window\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        path_text,
        path.len() - 1,
        window.id(),
        window.name().unwrap_or("-"),
        window.window_type(),
        upper_left_x,
        upper_left_y,
        bottom_right_x,
        bottom_right_y,
        creation_width,
        creation_height
    )
    .expect("writing to a String cannot fail");
    for field in window.fields() {
        writeln!(
            output,
            "window_field\t{}\t{}\t{}\t{}",
            path_text,
            field.name(),
            field.raw_value(),
            field.line()
        )
        .expect("writing to a String cannot fail");
    }
    render_wnd_typed_fields(output, window, &path_text);
    for (index, child) in window.children().iter().enumerate() {
        path.push(index);
        render_wnd_window(output, child, path);
        path.pop();
    }
}

/// Emits the typed view of a window's fields, so a modded layout can be compared field by
/// field without rendering it. Each record also appears verbatim as a `window_field` row.
fn render_wnd_typed_fields(output: &mut String, window: &WndWindow, path_text: &str) {
    for (field, flags) in [("STATUS", window.status()), ("STYLE", window.style())] {
        for flag in flags {
            writeln!(
                output,
                "window_flag\t{path_text}\t{field}\t{}\t{}",
                flag.name(),
                if flag.is_known() { "yes" } else { "no" }
            )
            .expect("writing to a String cannot fail");
        }
    }
    for (kind, label) in [
        (WndCallbackKind::System, "system"),
        (WndCallbackKind::Input, "input"),
        (WndCallbackKind::Tooltip, "tooltip"),
        (WndCallbackKind::Draw, "draw"),
    ] {
        if let Some(name) = window.callbacks().get(kind) {
            writeln!(output, "window_callback\t{path_text}\t{label}\t{name}")
                .expect("writing to a String cannot fail");
        }
    }
    for (property, value) in [
        ("header_template", window.header_template()),
        ("text", window.text()),
        ("tooltip_text", window.tooltip_text()),
    ] {
        if let Some(value) = value {
            writeln!(output, "window_property\t{path_text}\t{property}\t{value}")
                .expect("writing to a String cannot fail");
        }
    }
    if let Some(delay) = window.tooltip_delay() {
        writeln!(
            output,
            "window_property\t{path_text}\ttooltip_delay\t{delay}"
        )
        .expect("writing to a String cannot fail");
    }
    if let Some((x, y)) = window.image_offset() {
        writeln!(
            output,
            "window_property\t{path_text}\timage_offset\t{x} {y}"
        )
        .expect("writing to a String cannot fail");
    }
    if let Some(font) = window.font() {
        writeln!(
            output,
            "window_font\t{path_text}\t{}\t{}\t{}",
            font.name(),
            font.size(),
            if font.bold() { "yes" } else { "no" }
        )
        .expect("writing to a String cannot fail");
    }
    if let Some(colors) = window.text_colors() {
        for (state, color) in [
            ("enabled", colors.enabled()),
            ("enabled_border", colors.enabled_border()),
            ("disabled", colors.disabled()),
            ("disabled_border", colors.disabled_border()),
            ("hilite", colors.hilite()),
            ("hilite_border", colors.hilite_border()),
        ] {
            let [red, green, blue, alpha] = color.channels();
            writeln!(
                output,
                "window_text_color\t{path_text}\t{state}\t{red}\t{green}\t{blue}\t{alpha}"
            )
            .expect("writing to a String cannot fail");
        }
    }
    for (slot, data) in window.draw_data() {
        for (index, entry) in data.entries().iter().enumerate() {
            let [red, green, blue, alpha] = entry.color().channels();
            let [border_red, border_green, border_blue, border_alpha] =
                entry.border_color().channels();
            writeln!(
                output,
                "window_draw_entry\t{path_text}\t{}\t{index}\t{}\t{red}\t{green}\t{blue}\t{alpha}\t{border_red}\t{border_green}\t{border_blue}\t{border_alpha}",
                wnd_draw_slot_name(*slot),
                entry.image().unwrap_or("-")
            )
            .expect("writing to a String cannot fail");
        }
    }
    if let Some(data) = window.gadget_data() {
        render_wnd_gadget_data(output, data, path_text);
    }
}

fn render_wnd_gadget_data(output: &mut String, data: &WndGadgetData, path_text: &str) {
    let mut write = |gadget: &str, property: &str, value: String| {
        writeln!(
            output,
            "window_gadget_data\t{path_text}\t{gadget}\t{property}\t{value}"
        )
        .expect("writing to a String cannot fail");
    };
    match data {
        WndGadgetData::ListBox(list) => {
            write("list_box", "length", list.length().to_string());
            write("list_box", "auto_scroll", list.auto_scroll().to_string());
            write(
                "list_box",
                "scroll_if_at_end",
                list.scroll_if_at_end()
                    .map_or_else(|| "absent".to_owned(), |value| value.to_string()),
            );
            write("list_box", "auto_purge", list.auto_purge().to_string());
            write("list_box", "scroll_bar", list.scroll_bar().to_string());
            write("list_box", "multi_select", list.multi_select().to_string());
            write("list_box", "columns", list.columns().to_string());
            for (index, width) in list.column_widths().iter().enumerate() {
                write(
                    "list_box",
                    &format!("column_width_{index}"),
                    width.to_string(),
                );
            }
            write("list_box", "force_select", list.force_select().to_string());
        }
        WndGadgetData::ComboBox(combo) => {
            write("combo_box", "is_editable", combo.is_editable().to_string());
            write(
                "combo_box",
                "maximum_characters",
                combo.maximum_characters().to_string(),
            );
            write(
                "combo_box",
                "maximum_display",
                combo.maximum_display().to_string(),
            );
            write("combo_box", "ascii_only", combo.ascii_only().to_string());
            write(
                "combo_box",
                "letters_and_numbers_only",
                combo.letters_and_numbers_only().to_string(),
            );
        }
        WndGadgetData::Slider(slider) => {
            write("slider", "minimum", slider.minimum().to_string());
            write("slider", "maximum", slider.maximum().to_string());
        }
        WndGadgetData::RadioButtonGroup(group) => {
            write("radio_button", "group", group.to_string());
        }
        WndGadgetData::TextEntry(entry) => {
            write(
                "text_entry",
                "maximum_length",
                entry.maximum_length().to_string(),
            );
            write("text_entry", "secret_text", entry.secret_text().to_string());
            write(
                "text_entry",
                "numerical_only",
                entry.numerical_only().to_string(),
            );
            write(
                "text_entry",
                "alphanumerical_only",
                entry.alphanumerical_only().to_string(),
            );
            write("text_entry", "ascii_only", entry.ascii_only().to_string());
        }
        WndGadgetData::StaticTextCentered(centered) => {
            write("static_text", "centered", centered.to_string());
        }
        WndGadgetData::TabControl(tabs) => {
            write(
                "tab_control",
                "tab_orientation",
                tabs.tab_orientation().to_string(),
            );
            write("tab_control", "tab_edge", tabs.tab_edge().to_string());
            write("tab_control", "tab_width", tabs.tab_width().to_string());
            write("tab_control", "tab_height", tabs.tab_height().to_string());
            write("tab_control", "tab_count", tabs.tab_count().to_string());
            write("tab_control", "pane_border", tabs.pane_border().to_string());
            for (index, disabled) in tabs.pane_disabled().iter().enumerate() {
                write(
                    "tab_control",
                    &format!("pane_disabled_{index}"),
                    disabled.to_string(),
                );
            }
        }
    }
}

pub(crate) fn wnd_draw_slot_name(slot: WndDrawDataSlot) -> &'static str {
    match slot {
        WndDrawDataSlot::Enabled => "enabled",
        WndDrawDataSlot::Disabled => "disabled",
        WndDrawDataSlot::Hilite => "hilite",
        WndDrawDataSlot::ListBoxEnabledUpButton => "list_box_enabled_up_button",
        WndDrawDataSlot::ListBoxDisabledUpButton => "list_box_disabled_up_button",
        WndDrawDataSlot::ListBoxHiliteUpButton => "list_box_hilite_up_button",
        WndDrawDataSlot::ListBoxEnabledDownButton => "list_box_enabled_down_button",
        WndDrawDataSlot::ListBoxDisabledDownButton => "list_box_disabled_down_button",
        WndDrawDataSlot::ListBoxHiliteDownButton => "list_box_hilite_down_button",
        WndDrawDataSlot::ListBoxEnabledSlider => "list_box_enabled_slider",
        WndDrawDataSlot::ListBoxDisabledSlider => "list_box_disabled_slider",
        WndDrawDataSlot::ListBoxHiliteSlider => "list_box_hilite_slider",
        WndDrawDataSlot::SliderThumbEnabled => "slider_thumb_enabled",
        WndDrawDataSlot::SliderThumbDisabled => "slider_thumb_disabled",
        WndDrawDataSlot::SliderThumbHilite => "slider_thumb_hilite",
        WndDrawDataSlot::ComboBoxDropDownButtonEnabled => "combo_box_drop_down_button_enabled",
        WndDrawDataSlot::ComboBoxDropDownButtonDisabled => "combo_box_drop_down_button_disabled",
        WndDrawDataSlot::ComboBoxDropDownButtonHilite => "combo_box_drop_down_button_hilite",
        WndDrawDataSlot::ComboBoxEditBoxEnabled => "combo_box_edit_box_enabled",
        WndDrawDataSlot::ComboBoxEditBoxDisabled => "combo_box_edit_box_disabled",
        WndDrawDataSlot::ComboBoxEditBoxHilite => "combo_box_edit_box_hilite",
        WndDrawDataSlot::ComboBoxListBoxEnabled => "combo_box_list_box_enabled",
        WndDrawDataSlot::ComboBoxListBoxDisabled => "combo_box_list_box_disabled",
        WndDrawDataSlot::ComboBoxListBoxHilite => "combo_box_list_box_hilite",
    }
}

fn render_w3d_level(output: &mut String, chunks: &[W3dChunk], path: &mut Vec<usize>) {
    for (index, chunk) in chunks.iter().enumerate() {
        path.push(index);
        let path_text = path
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join("/");
        let kind = if chunk.is_container() {
            "container"
        } else {
            "data"
        };
        let name = w3d_chunk_name(chunk.id()).unwrap_or("unknown");
        writeln!(
            output,
            "{}\t{}\t{}\t0x{:08X}\t{}\t{}\t{}",
            path_text,
            path.len() - 1,
            chunk.offset(),
            chunk.id(),
            kind,
            chunk.payload_length(),
            name
        )
        .expect("writing to a String cannot fail");
        if let Some(children) = chunk.children() {
            render_w3d_level(output, children, path);
        }
        path.pop();
    }
}

fn ascii_fold(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().map(u8::to_ascii_lowercase).collect()
}

fn escape_bytes(bytes: &[u8]) -> String {
    let mut escaped = String::new();
    for byte in bytes {
        match byte {
            b'\\' => escaped.push_str("\\\\"),
            b'\t' => escaped.push_str("\\t"),
            b'\n' => escaped.push_str("\\n"),
            b'\r' => escaped.push_str("\\r"),
            0x20..=0x7e => escaped.push(char::from(*byte)),
            _ => write!(escaped, "\\x{byte:02X}").expect("writing to a String cannot fail"),
        }
    }
    escaped
}

fn escape_text(text: &str) -> String {
    let mut escaped = String::new();
    for character in text.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '\t' => escaped.push_str("\\t"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            value if value.is_control() => write!(escaped, "\\u{{{:X}}}", u32::from(value))
                .expect("writing to a String cannot fail"),
            value => escaped.push(value),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use cic_formats::{
        CsfLimits, MapLimits, W3dLimits, W3dMeshLimits, decode_map_blend, decode_map_height,
        decode_map_polygons, decode_static_mesh, parse_csf, parse_map, parse_w3d,
    };
    use cic_vfs::{Vfs, VirtualPath};

    use cic_ui::{UiActionAllowlist, UiDemoAction, UiShell};

    use super::shell_menu::{ShellMenuHide, ShellMenuRecord, main_menu_bindings};
    use super::{
        encode_map_height_png, render_csf, render_manifest, render_map, render_map_blend,
        render_map_height, render_map_polygons, render_ui_menu, render_w3d, render_w3d_mesh,
    };

    fn hex_fixture(hex: &str) -> Vec<u8> {
        let digits = hex
            .bytes()
            .filter(u8::is_ascii_hexdigit)
            .collect::<Vec<_>>();
        digits
            .chunks_exact(2)
            .map(|pair| {
                let pair = std::str::from_utf8(pair).expect("ASCII fixture");
                u8::from_str_radix(pair, 16).expect("valid hex fixture")
            })
            .collect()
    }

    #[test]
    fn manifest_is_sorted_and_reports_winning_provenance() {
        let mut vfs = Vfs::new();
        vfs.mount_memory(
            "base",
            [
                (
                    VirtualPath::new("z.txt").expect("valid path"),
                    b"z".to_vec(),
                ),
                (
                    VirtualPath::new("a.txt").expect("valid path"),
                    b"old".to_vec(),
                ),
            ],
        )
        .expect("base mount");
        vfs.mount_memory(
            "override",
            [(
                VirtualPath::new("A.TXT").expect("valid path"),
                b"new!".to_vec(),
            )],
        )
        .expect("override mount");

        assert_eq!(
            render_manifest(&vfs),
            "path\tbytes\tprovider\na.txt\t4\tmemory:override\nz.txt\t1\tmemory:base\n"
        );
    }

    #[test]
    fn csf_report_is_sorted_and_includes_zero_string_labels() {
        let hex = include_str!("../../cic-formats/tests/fixtures/minimal.csf.hex");
        let digits = hex
            .bytes()
            .filter(u8::is_ascii_hexdigit)
            .collect::<Vec<_>>();
        let bytes = digits
            .chunks_exact(2)
            .map(|pair| {
                let pair = std::str::from_utf8(pair).expect("ASCII fixture");
                u8::from_str_radix(pair, 16).expect("valid hex fixture")
            })
            .collect::<Vec<_>>();
        let csf = parse_csf(&bytes, "minimal.csf", CsfLimits::default()).expect("valid CSF");

        assert_eq!(
            render_csf(&csf),
            "version\tlanguage\tlabels\tstrings\n\
             3\t0\t3\t2\n\
             label\tvariant\ttext\twave\n\
             GUI:HELLO\t0\tHello\t\n\
             SPEECH:READY\t0\tReady\tready.wav\n\
             TOOLTIP:EMPTY\t-\t\t\n"
        );
    }

    #[test]
    fn w3d_report_uses_stable_slash_separated_tree_paths() {
        let hex = include_str!("../../cic-formats/tests/fixtures/minimal.w3d.hex");
        let digits = hex
            .bytes()
            .filter(u8::is_ascii_hexdigit)
            .collect::<Vec<_>>();
        let bytes = digits
            .chunks_exact(2)
            .map(|pair| {
                let pair = std::str::from_utf8(pair).expect("ASCII fixture");
                u8::from_str_radix(pair, 16).expect("valid hex fixture")
            })
            .collect::<Vec<_>>();
        let w3d = parse_w3d(&bytes, "minimal.w3d", W3dLimits::default()).expect("valid W3D");

        assert_eq!(
            render_w3d(&w3d),
            "path\tdepth\toffset\tid\tkind\tpayload\tname\n\
             0\t0\t0\t0x00000000\tcontainer\t29\tW3D_CHUNK_MESH\n\
             0/0\t1\t8\t0x11111111\tdata\t3\tunknown\n\
             0/1\t1\t19\t0x22222222\tcontainer\t10\tunknown\n\
             0/1/0\t2\t27\t0x33333333\tdata\t2\tunknown\n\
             1\t0\t37\t0xDEADBEEF\tdata\t4\tunknown\n"
        );
    }

    #[test]
    fn map_reports_preserve_inventory_and_emit_row_major_heights() {
        let bytes = hex_fixture(include_str!(
            "../../cic-formats/tests/fixtures/minimal.map.hex"
        ));
        let map = parse_map(&bytes, "minimal.map", MapLimits::default()).expect("valid MAP");

        assert_eq!(
            render_map(&map),
            "compression\tnone\n\
             symbol\toffset\tid\tname\n\
             0\t8\t0x00000007\tHeightMapData\n\
             1\t26\t0x00000009\tMystery\n\
             chunk\toffset\tid\tversion\tpayload\tname\n\
             0\t38\t0x00000007\t4\t34\tHeightMapData\n\
             1\t82\t0x00000009\t2\t3\tMystery\n\
             2\t95\t0xFEEDBEEF\t9\t2\tunknown\n"
        );

        let height = decode_map_height(&map, MapLimits::default()).expect("valid heights");
        assert_eq!(
            render_map_height(&height),
            "version\twidth\theight\tborder\tcell_size\tboundaries\tsamples\n\
             4\t3\t2\t0\t10\t1\t6\n\
             boundary\tx\ty\n\
             0\t3\t2\n\
             sample\tx\ty\tvalue\n\
             0\t0\t0\t0\n\
             1\t1\t0\t16\n\
             2\t2\t0\t32\n\
             3\t0\t1\t48\n\
             4\t1\t1\t64\n\
             5\t2\t1\t255\n"
        );

        let png = encode_map_height_png(&height).expect("encode height PNG");
        let image = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
            .expect("decode height PNG")
            .to_luma8();
        assert_eq!(image.dimensions(), (3, 2));
        assert_eq!(image.as_raw(), &[0, 16, 32, 48, 64, 255]);
    }

    #[test]
    fn map_blend_report_is_stable_and_preserves_exact_uv_bits() {
        let bytes = hex_fixture(include_str!(
            "../../cic-formats/tests/fixtures/blend.map.hex"
        ));
        let map = parse_map(&bytes, "blend.map", MapLimits::default()).expect("valid MAP");
        let height = decode_map_height(&map, MapLimits::default()).expect("valid heights");
        let blend = decode_map_blend(&map, &height, MapLimits::default()).expect("valid blend");

        assert_eq!(
            render_map_blend(&blend),
            "version\twidth\theight\tcells\tbitmap_tiles\tblended_tiles\tcliff_info\ttexture_classes\tedge_tiles\tedge_texture_classes\tcliff_stride\n\
             7\t8\t2\t16\t4\t2\t2\t1\t2\t1\t1\n\
             cell\tx\ty\ttile\tblend\textra_blend\tcliff_info\tcliff_flag\n\
             0\t0\t0\t0\t0\t0\t1\t1\n\
             1\t1\t0\t1\t0\t0\t0\t0\n\
             2\t2\t0\t2\t0\t0\t0\t0\n\
             3\t3\t0\t3\t0\t0\t0\t0\n\
             4\t4\t0\t0\t0\t0\t0\t0\n\
             5\t5\t0\t1\t1\t0\t0\t0\n\
             6\t6\t0\t2\t0\t1\t0\t0\n\
             7\t7\t0\t3\t0\t0\t0\t0\n\
             8\t0\t1\t0\t0\t0\t0\t0\n\
             9\t1\t1\t1\t0\t0\t0\t0\n\
             10\t2\t1\t2\t0\t0\t0\t0\n\
             11\t3\t1\t3\t0\t0\t0\t0\n\
             12\t4\t1\t0\t0\t0\t0\t0\n\
             13\t5\t1\t1\t0\t0\t0\t0\n\
             14\t6\t1\t2\t0\t0\t0\t0\n\
             15\t7\t1\t3\t0\t0\t0\t1\n\
             texture\tkind\tfirst\tcount\twidth\tlegacy\tname\n\
             0\tterrain\t0\t4\t2\t0\tBase\n\
             0\tedge\t0\t2\t1\t\tShore\n\
             blend\tblend_index\thorizontal\tvertical\tright_diagonal\tleft_diagonal\tinverted\tlong_diagonal\tcustom_edge_class\n\
             1\t1\t1\t0\t1\t0\t3\t1\t0\n\
             cliff\ttile\tu0\tv0\tu1\tv1\tu2\tv2\tu3\tv3\tflip\tmutant\n\
             1\t3\t0x00000000\t0x00000000\t0x00000000\t0x3F800000\t0x3F800000\t0x3F800000\t0x3F800000\t0x00000000\t1\t0\n"
        );
    }

    #[test]
    fn complete_polygon_report_retains_nonwater_records_and_layer_names() {
        let mut payload = 1_i32.to_le_bytes().to_vec();
        payload.extend_from_slice(&4_u16.to_le_bytes());
        payload.extend_from_slice(b"Area");
        payload.extend_from_slice(&5_u16.to_le_bytes());
        payload.extend_from_slice(b"Layer");
        payload.extend_from_slice(&7_i32.to_le_bytes());
        payload.push(0);
        payload.push(0);
        payload.extend_from_slice(&0_i32.to_le_bytes());
        payload.extend_from_slice(&2_i32.to_le_bytes());
        for point in [[1_i32, 2, 3], [4, 5, 6]] {
            for value in point {
                payload.extend_from_slice(&value.to_le_bytes());
            }
        }
        let mut bytes = b"CkMp".to_vec();
        bytes.extend_from_slice(&1_i32.to_le_bytes());
        bytes.push(15);
        bytes.extend_from_slice(b"PolygonTriggers");
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&4_u16.to_le_bytes());
        bytes.extend_from_slice(
            &i32::try_from(payload.len())
                .expect("payload length")
                .to_le_bytes(),
        );
        bytes.extend_from_slice(&payload);
        let map = parse_map(&bytes, "polygon.map", MapLimits::default()).expect("inventory");
        let polygons = decode_map_polygons(&map, MapLimits::default()).expect("complete polygons");
        assert_eq!(
            render_map_polygons(&polygons),
            "version\tpolygon_areas\tpolygon_points\twater_areas\triver_areas\n\
             4\t1\t2\t0\t0\n\
             area\tsource_index\tid\twater\triver\triver_start\tpoints\tname\tlayer\n\
             0\t0\t7\t0\t0\t0\t2\tArea\tLayer\n\
             point\t0\t0\t1\t2\t3\n\
             point\t0\t1\t4\t5\t6\n"
        );
    }

    #[test]
    fn static_mesh_report_preserves_exact_geometry_bits() {
        let hex = include_str!("../../cic-formats/tests/fixtures/static-mesh.w3d.hex");
        let digits = hex
            .bytes()
            .filter(u8::is_ascii_hexdigit)
            .collect::<Vec<_>>();
        let bytes = digits
            .chunks_exact(2)
            .map(|pair| {
                let pair = std::str::from_utf8(pair).expect("ASCII fixture");
                u8::from_str_radix(pair, 16).expect("valid hex fixture")
            })
            .collect::<Vec<_>>();
        let w3d = parse_w3d(&bytes, "static-mesh.w3d", W3dLimits::default()).expect("valid W3D");
        let mesh = decode_static_mesh(&w3d.chunks()[0], W3dMeshLimits::default())
            .expect("valid static mesh");
        let report = render_w3d_mesh(&mesh);

        assert!(report.starts_with(
            "version\tattributes\tmesh\tcontainer\tvertices\ttriangles\tmaterials\tdamage_stages\tsort_level\tprelit\tvertex_channels\tface_channels\n\
             0x00040002\t0x00000000\tTri\tTest\t3\t1\t0\t0\t0\t0x00000000\t0x00000001\t0x00000001\n"
        ));
        assert!(report.contains(
            "vertex\tx\ty\tz\tnx\tny\tnz\n\
             0\t0x00000000\t0x00000000\t0x00000000\t0x00000000\t0x00000000\t0x3F800000\n"
        ));
        assert!(report.ends_with(
            "triangle\tv0\tv1\tv2\tattributes\tnx\tny\tnz\tdistance\n\
             0\t0\t1\t2\t0x00000000\t0x00000000\t0x00000000\t0x3F800000\t0x00000000\n"
        ));
    }

    /// The main-menu table is source-derived data, so what it must satisfy is structural: every
    /// action it names is one a session can run, and the panel arms are complete and in order.
    #[test]
    fn the_main_menu_bindings_route_only_complete_panel_changes() {
        let bindings = main_menu_bindings();
        assert_eq!(bindings.path, "Menus/MainMenu.wnd");

        // `MainMenuInit` hides all five drop-down panels; the loop that does it starts past the
        // unassigned `DROPDOWN_NONE` slot, so exactly five, not four.
        let panels = bindings
            .initial_hidden
            .iter()
            .filter(|name| name.contains(":MapBorder"))
            .count();
        assert_eq!(panels, 5);
        // `showSelectiveButtons(SHOW_NONE)` hides two buttons for each of the three factions.
        let saves = bindings
            .initial_hidden
            .iter()
            .filter(|name| name.contains("RecentSave") || name.contains("LoadGame"))
            .count();
        assert_eq!(saves, 6);
        assert!(
            bindings
                .initial_hidden
                .contains(&"MainMenu.wnd:MainMenuRuler")
        );

        // The first input reveals the default panel: that reveal is an explicit `winHide(FALSE)` in
        // `MainMenuInput`, not something the transition does, so it must be in the list.
        assert!(bindings.first_input.iter().any(|action| matches!(
            action,
            UiDemoAction::ShowControl { control } if control == "MainMenu.wnd:MapBorder2"
        )));
        // Re-entry must not repeat that reveal: `MainMenuDefaultMenuLogoFade`'s own `FLASH` unhides
        // the panel, and the source's call at that point is commented out.
        assert!(!bindings.re_entry.iter().any(|action| matches!(
            action,
            UiDemoAction::ShowControl { control } if control == "MainMenu.wnd:MapBorder2"
        )));

        // Every panel change reveals its panel first, then removes, reverses, and sets a group.
        for control in [
            "MainMenu.wnd:ButtonSinglePlayer",
            "MainMenu.wnd:ButtonSingleBack",
            "MainMenu.wnd:ButtonMultiplayer",
            "MainMenu.wnd:ButtonMultiBack",
            "MainMenu.wnd:ButtonLoadReplay",
            "MainMenu.wnd:ButtonLoadReplayBack",
        ] {
            let actions = bindings
                .allowlist
                .resolve(control)
                .unwrap_or_else(|| panic!("{control} must be bound"));
            assert_eq!(
                actions
                    .iter()
                    .map(UiDemoAction::row_name)
                    .collect::<Vec<_>>(),
                [
                    "show_control",
                    "remove_transition_group",
                    "reverse_transition_group",
                    "set_transition_group"
                ],
                "{control}"
            );
        }

        // Only Skirmish pushes a gameplay-adjacent screen, and Exit only ever reports.
        assert_eq!(
            bindings
                .allowlist
                .resolve("MainMenu.wnd:ButtonExit")
                .map(<[UiDemoAction]>::to_vec),
            Some(vec![UiDemoAction::Quit])
        );
        assert!(
            bindings
                .allowlist
                .resolve("MainMenu.wnd:ButtonSkirmish")
                .expect("the skirmish binding")
                .iter()
                .any(|action| matches!(
                    action,
                    UiDemoAction::PushScreen { path } if path == "Menus/SkirmishGameOptionsMenu.wnd"
                ))
        );
        // A control the table says nothing about routes nothing, however established its callback.
        assert!(
            bindings
                .allowlist
                .resolve("MainMenu.wnd:ButtonWorldBuilder")
                .is_none()
        );
    }

    #[test]
    fn a_menu_report_attributes_every_record_to_the_step_that_caused_it() {
        let records = vec![
            ShellMenuRecord::InitialHide {
                control: "MainMenu.wnd:MapBorder2".to_owned(),
                outcome: ShellMenuHide::Hidden,
            },
            ShellMenuRecord::InitialHide {
                control: "MainMenu.wnd:WinFactionUS".to_owned(),
                outcome: ShellMenuHide::AlreadyHidden,
            },
            ShellMenuRecord::FirstInput,
            ShellMenuRecord::Unrouted {
                control: Some("MainMenu.wnd:ButtonCredits".to_owned()),
                callback: Some("MainMenuSystem".to_owned()),
            },
        ];
        // The first two records precede every step, which is the screen opening.
        let steps = vec![("move:1,1".to_owned(), 2..4)];
        let report = render_ui_menu(&steps, &records, &UiActionAllowlist::new(), &UiShell::new());

        assert!(report.contains(
            "ui_menu_record\topen\tinitial_hide\tMainMenu.wnd:MapBorder2\thidden\n\
             ui_menu_record\topen\tinitial_hide\tMainMenu.wnd:WinFactionUS\talready_hidden\n"
        ));
        assert!(report.contains("ui_menu_record\t0\tfirst_input\t"));
        assert!(report.contains(
            "ui_menu_record\t0\tunrouted\tMainMenu.wnd:ButtonCredits\tcallback=MainMenuSystem\n"
        ));
        // The initial-hide tally is what makes `initialHide`'s redundancy visible at a glance.
        assert!(report.contains("ui_menu_initial_hide\t1\t1\t0\n"));
        assert!(report.contains("ui_menu_summary\t0\t-\t0\t1\t0\n"));
    }
}
