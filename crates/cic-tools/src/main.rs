use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use cic_formats::{
    CsfLimits, MapLimits, MapWaterError, TerrainIniLimits, W3dFile, W3dLimits, W3dMeshLimits,
    W3dSceneLimits, WaterIniLimits, decode_map_blend, decode_map_height, decode_map_water,
    decode_static_mesh, decode_w3d_model_set, parse_csf, parse_map, parse_terrain_ini, parse_w3d,
    parse_water_transparency_ini, w3d_model_hierarchy_name,
};
use cic_render::{
    AnimatedModel, HeadlessRenderer, ModelFrame, StagedTerrain, StagedWater,
    TerrainCompatibilityPolicy, TerrainStagingOptions, TextureId, TextureResourceManager,
    WaterAppearance, WaterCausticSequence, run_model_viewer, run_terrain_viewer,
};
use cic_tools::resource::{
    GameEdition, MountProfile, MountProfileLimits, ResourceKind, StoredLocations, config_path,
    discover_steam_locations, resolve_archives, validate_installation,
};
use cic_tools::{
    GltfTextureRequest, encode_capture_png, encode_map_height_png, pack_w3d_glb, render_csf,
    render_manifest, render_map, render_map_blend, render_map_height, render_map_water, render_w3d,
    render_w3d_gltf, render_w3d_mesh,
};
use cic_vfs::{BigLimits, Vfs, VirtualPath};

const USAGE: &str = "Usage:\n\
  cic-inspect [--zh] [--game-dir <path>] [--profile <profile>] [--mod <mount>]... <command> ...\n\
  cic-inspect config show\n\
  cic-inspect config set <generals-dir|zero-hour-dir> <path>\n\
  cic-inspect manifest <mount> [<mount> ...]\n\
  cic-inspect csf <virtual-path> <mount> [<mount> ...]\n\
  cic-inspect map <virtual-path> <mount> [<mount> ...]\n\
  cic-inspect map-height [--report | --png <output.png>] <virtual-path> <mount> [<mount> ...]\n\
  cic-inspect map-blend <virtual-path> <mount> [<mount> ...]\n\
  cic-inspect map-water <virtual-path> <mount> [<mount> ...]\n\
  cic-inspect map-render [--size <pixels>] [--pixels-per-cell <pixels>] [--terrain-policy <legacy|modern>] <virtual-path> [<output.png>] [<mount> ...]\n\
  cic-inspect map-view [--pixels-per-cell <pixels>] [--terrain-policy <legacy|modern>] <virtual-path> [<mount> ...]\n\
  cic-inspect w3d <virtual-path> <mount> [<mount> ...]\n\
  cic-inspect w3d-mesh <virtual-path> <top-level-index> <mount> [<mount> ...]\n\
  cic-inspect w3d-view <virtual-path> [<mount> ...]\n\
  cic-inspect w3d-render [--animation <index>] [--frame <frame>] [--time <seconds>] [--rotation <radians>] <virtual-path> [<output.ppm>] [<mount> ...]\n\
  cic-inspect w3d-export [--gltf] <virtual-path> [<output.glb|output.gltf>] [<mount> ...]\n\
Each mount is a directory or BIG archive. Mounts are applied from left to right; later mounts override earlier mounts.";

const MAX_ENCODED_IMAGE_BYTES: usize = 256 * 1_024 * 1_024;

#[derive(Debug)]
struct CliOptions {
    edition: GameEdition,
    edition_explicit: bool,
    game_dir: Option<PathBuf>,
    profile: Option<PathBuf>,
    mods: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExportFormat {
    Glb,
    Gltf,
}

fn main() -> ExitCode {
    match run(env::args().skip(1)) {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

fn run(arguments: impl IntoIterator<Item = String>) -> Result<String, Box<dyn Error>> {
    let mut arguments = arguments.into_iter().peekable();
    let options = parse_cli_options(&mut arguments)?;
    let command = arguments.next().ok_or("missing command")?;
    match command.as_str() {
        "config" => configure(arguments),
        "manifest" => {
            let mounts = arguments.collect::<Vec<_>>();
            let vfs = mount_all("manifest", &mounts, &options, ResourceKind::Manifest)?;
            Ok(render_manifest(&vfs))
        }
        "csf" => {
            let resource_name = arguments.next().ok_or("csf requires a virtual path")?;
            let mounts = arguments.collect::<Vec<_>>();
            let vfs = mount_all("csf", &mounts, &options, ResourceKind::Localization)?;
            let resource_path = VirtualPath::new(&resource_name)?;
            let entry = vfs
                .resolve(&resource_path)
                .ok_or_else(|| format!("resource not found: {resource_path}"))?;
            let limits = CsfLimits::default();
            let bytes = entry.read(limits.maximum_file_bytes)?;
            let csf = parse_csf(&bytes, resource_path.as_str(), limits)?;
            Ok(render_csf(&csf))
        }
        "map" => {
            let resource_name = arguments
                .next()
                .ok_or_else(|| format!("{command} requires a virtual path"))?;
            let mounts = arguments.collect::<Vec<_>>();
            let vfs = mount_all(&command, &mounts, &options, ResourceKind::Map)?;
            let resource_path = VirtualPath::new(&resource_name)?;
            let entry = vfs
                .resolve(&resource_path)
                .ok_or_else(|| format!("resource not found: {resource_path}"))?;
            let limits = MapLimits::default();
            let bytes = entry.read(limits.maximum_file_bytes)?;
            let map = parse_map(&bytes, resource_path.as_str(), limits)?;
            Ok(render_map(&map))
        }
        "map-height" => report_map_height(&mut arguments, &options),
        "map-blend" => report_map_blend(arguments, &options),
        "map-water" => report_map_water(arguments, &options),
        "map-render" => render_terrain_capture(&mut arguments, &options),
        "map-view" => view_terrain(&mut arguments, &options),
        "w3d" => {
            let resource_name = arguments.next().ok_or("w3d requires a virtual path")?;
            let mounts = arguments.collect::<Vec<_>>();
            let vfs = mount_all("w3d", &mounts, &options, ResourceKind::W3d)?;
            let resource_path = VirtualPath::new(&resource_name)?;
            let entry = vfs
                .resolve(&resource_path)
                .ok_or_else(|| format!("resource not found: {resource_path}"))?;
            let limits = W3dLimits::default();
            let bytes = entry.read(limits.maximum_file_bytes)?;
            let w3d = parse_w3d(&bytes, resource_path.as_str(), limits)?;
            Ok(render_w3d(&w3d))
        }
        "w3d-mesh" => {
            let resource_name = arguments.next().ok_or("w3d-mesh requires a virtual path")?;
            let chunk_index = arguments
                .next()
                .ok_or("w3d-mesh requires a top-level chunk index")?
                .parse::<usize>()?;
            let mounts = arguments.collect::<Vec<_>>();
            let vfs = mount_all("w3d-mesh", &mounts, &options, ResourceKind::W3d)?;
            let resource_path = VirtualPath::new(&resource_name)?;
            let entry = vfs
                .resolve(&resource_path)
                .ok_or_else(|| format!("resource not found: {resource_path}"))?;
            let limits = W3dLimits::default();
            let bytes = entry.read(limits.maximum_file_bytes)?;
            let w3d = parse_w3d(&bytes, resource_path.as_str(), limits)?;
            let chunk = w3d.chunks().get(chunk_index).ok_or_else(|| {
                format!(
                    "top-level chunk index {chunk_index} is out of range for {} chunks",
                    w3d.chunks().len()
                )
            })?;
            let mesh = decode_static_mesh(chunk, W3dMeshLimits::default())?;
            Ok(render_w3d_mesh(&mesh))
        }
        "w3d-render" => render_model_capture(&mut arguments, &options),
        "w3d-view" => view_model(&mut arguments, &options),
        "w3d-export" => export_model(&mut arguments, &options),
        _ => Err(format!("unknown command {command:?}").into()),
    }
}

fn parse_cli_options<I>(
    arguments: &mut std::iter::Peekable<I>,
) -> Result<CliOptions, Box<dyn Error>>
where
    I: Iterator<Item = String>,
{
    let mut options = CliOptions {
        edition: GameEdition::Generals,
        edition_explicit: false,
        game_dir: None,
        profile: None,
        mods: Vec::new(),
    };
    while let Some(argument) = arguments.peek() {
        match argument.as_str() {
            "--zh" => {
                options.edition = GameEdition::ZeroHour;
                options.edition_explicit = true;
                arguments.next();
            }
            "--game-dir" => {
                arguments.next();
                options.game_dir = Some(PathBuf::from(
                    arguments.next().ok_or("--game-dir requires a path")?,
                ));
            }
            "--profile" => {
                arguments.next();
                if options.profile.is_some() {
                    return Err("--profile may be supplied only once".into());
                }
                options.profile = Some(PathBuf::from(
                    arguments.next().ok_or("--profile requires a path")?,
                ));
            }
            "--mod" => {
                arguments.next();
                options.mods.push(PathBuf::from(
                    arguments.next().ok_or("--mod requires a path")?,
                ));
            }
            _ => break,
        }
    }
    if options.profile.is_some() && (options.game_dir.is_some() || options.edition_explicit) {
        return Err("--profile cannot be combined with --game-dir or --zh".into());
    }
    Ok(options)
}

fn report_map_height<I>(
    arguments: &mut std::iter::Peekable<I>,
    options: &CliOptions,
) -> Result<String, Box<dyn Error>>
where
    I: Iterator<Item = String>,
{
    let mut report = false;
    let mut png_path = None;
    while arguments
        .peek()
        .is_some_and(|argument| argument.starts_with("--"))
    {
        match arguments.next().expect("peeked map-height option").as_str() {
            "--report" => report = true,
            "--png" => {
                png_path = Some(PathBuf::from(
                    arguments.next().ok_or("--png requires an output path")?,
                ));
            }
            option => return Err(format!("unknown map-height option {option:?}").into()),
        }
    }
    if report && png_path.is_some() {
        return Err("map-height --report and --png are mutually exclusive".into());
    }
    let resource_name = arguments
        .next()
        .ok_or("map-height requires a virtual path")?;
    let mounts = arguments.collect::<Vec<_>>();
    let vfs = mount_all("map-height", &mounts, options, ResourceKind::Map)?;
    let resource_path = VirtualPath::new(&resource_name)?;
    let entry = vfs
        .resolve(&resource_path)
        .ok_or_else(|| format!("resource not found: {resource_path}"))?;
    let limits = MapLimits::default();
    let bytes = entry.read(limits.maximum_file_bytes)?;
    let map = parse_map(&bytes, resource_path.as_str(), limits)?;
    let height = decode_map_height(&map, limits)?;
    if report {
        Ok(render_map_height(&height))
    } else {
        let path = png_path.unwrap_or(default_map_output_path(&resource_path, "png")?);
        let png = encode_map_height_png(&height)?;
        fs::write(&path, &png)?;
        Ok(format!(
            "height-png\t{}\t{}\t{}\t{}\n",
            path.display(),
            height.width(),
            height.height(),
            png.len()
        ))
    }
}

fn default_map_output_path(
    resource_path: &VirtualPath,
    extension: &str,
) -> Result<PathBuf, Box<dyn Error>> {
    let stem = Path::new(resource_path.as_str())
        .file_stem()
        .ok_or("MAP resource path has no file name")?;
    Ok(PathBuf::from(stem).with_extension(extension))
}

fn report_map_blend<I>(arguments: I, options: &CliOptions) -> Result<String, Box<dyn Error>>
where
    I: Iterator<Item = String>,
{
    let mut arguments = arguments;
    let resource_name = arguments
        .next()
        .ok_or("map-blend requires a virtual path")?;
    let mounts = arguments.collect::<Vec<_>>();
    let vfs = mount_all("map-blend", &mounts, options, ResourceKind::Map)?;
    let resource_path = VirtualPath::new(&resource_name)?;
    let entry = vfs
        .resolve(&resource_path)
        .ok_or_else(|| format!("resource not found: {resource_path}"))?;
    let limits = MapLimits::default();
    let bytes = entry.read(limits.maximum_file_bytes)?;
    let map = parse_map(&bytes, resource_path.as_str(), limits)?;
    let height = decode_map_height(&map, limits)?;
    let blend = decode_map_blend(&map, &height, limits)?;
    Ok(render_map_blend(&blend))
}

fn report_map_water<I>(arguments: I, options: &CliOptions) -> Result<String, Box<dyn Error>>
where
    I: Iterator<Item = String>,
{
    let mut arguments = arguments;
    let resource_name = arguments
        .next()
        .ok_or("map-water requires a virtual path")?;
    let mounts = arguments.collect::<Vec<_>>();
    let vfs = mount_all("map-water", &mounts, options, ResourceKind::Map)?;
    let resource_path = VirtualPath::new(&resource_name)?;
    let entry = vfs
        .resolve(&resource_path)
        .ok_or_else(|| format!("resource not found: {resource_path}"))?;
    let limits = MapLimits::default();
    let bytes = entry.read(limits.maximum_file_bytes)?;
    let map = parse_map(&bytes, resource_path.as_str(), limits)?;
    let water = decode_map_water(&map, limits)?;
    Ok(render_map_water(&water))
}

fn render_terrain_capture<I>(
    arguments: &mut std::iter::Peekable<I>,
    options: &CliOptions,
) -> Result<String, Box<dyn Error>>
where
    I: Iterator<Item = String>,
{
    let mut size = 768_u32;
    let mut pixels_per_cell = TerrainStagingOptions::SOURCE_BACKGROUND.pixels_per_cell();
    let mut compatibility = TerrainCompatibilityPolicy::ZeroHourLegacy;
    while arguments
        .peek()
        .is_some_and(|argument| argument.starts_with("--"))
    {
        let option = arguments.next().expect("peeked map-render option");
        let value = arguments
            .next()
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option.as_str() {
            "--size" => size = value.parse::<u32>()?,
            "--pixels-per-cell" => pixels_per_cell = value.parse::<u32>()?,
            "--terrain-policy" => compatibility = parse_terrain_policy(&value)?,
            _ => return Err(format!("unknown map-render option {option:?}").into()),
        }
    }
    let resource_name = arguments
        .next()
        .ok_or("map-render requires a virtual path")?;
    let resource_path = VirtualPath::new(&resource_name)?;
    let remaining = arguments.collect::<Vec<_>>();
    let (output_path, mounts) = if remaining.first().is_some_and(|candidate| {
        Path::new(candidate)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
    }) {
        (PathBuf::from(&remaining[0]), remaining[1..].to_vec())
    } else {
        (default_terrain_render_path(&resource_path)?, remaining)
    };
    let vfs = mount_all("map-render", &mounts, options, ResourceKind::Terrain)?;
    let terrain = load_staged_terrain(&vfs, &resource_path, pixels_per_cell, compatibility)?;
    let renderer = pollster::block_on(HeadlessRenderer::new())?;
    let capture = renderer.capture_terrain(size, size, &terrain)?;
    let png = encode_capture_png(&capture)?;
    fs::write(&output_path, png)?;
    let primary_layers = terrain
        .cells()
        .iter()
        .filter(|cell| cell.primary().is_some())
        .count();
    let extra_layers = terrain
        .cells()
        .iter()
        .filter(|cell| cell.extra().is_some())
        .count();
    Ok(format!(
        "adapter\t{}\nterrain_policy\t{}\ngrid\t{}\t{}\ncells\t{}\nvertices\t{}\nindices\t{}\nedge_indices\t{}\nprimary_layers\t{}\nextra_layers\t{}\ncustom_edge_cells\t{}\nbaked_texture\t{}\t{}\nrgba_sha256\t{}\nwrote\t{}\n",
        renderer.adapter_info().name,
        terrain_policy_name(compatibility),
        terrain.width(),
        terrain.height(),
        terrain.cells().len(),
        terrain.vertices().len(),
        terrain.indices().len(),
        terrain.edge_indices().len(),
        primary_layers,
        extra_layers,
        terrain.custom_edge_cell_count(),
        terrain.texture_width(),
        terrain.texture_height(),
        capture.sha256(),
        output_path.display()
    ))
}

fn view_terrain<I>(
    arguments: &mut std::iter::Peekable<I>,
    options: &CliOptions,
) -> Result<String, Box<dyn Error>>
where
    I: Iterator<Item = String>,
{
    let mut pixels_per_cell = TerrainStagingOptions::SOURCE_BACKGROUND.pixels_per_cell();
    let mut compatibility = TerrainCompatibilityPolicy::ZeroHourLegacy;
    while arguments
        .peek()
        .is_some_and(|argument| argument.starts_with("--"))
    {
        let option = arguments.next().expect("peeked map-view option");
        let value = arguments
            .next()
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option.as_str() {
            "--pixels-per-cell" => pixels_per_cell = value.parse::<u32>()?,
            "--terrain-policy" => compatibility = parse_terrain_policy(&value)?,
            _ => return Err(format!("unknown map-view option {option:?}").into()),
        }
    }
    let resource_name = arguments.next().ok_or("map-view requires a virtual path")?;
    let resource_path = VirtualPath::new(&resource_name)?;
    let mounts = arguments.collect::<Vec<_>>();
    let vfs = mount_all("map-view", &mounts, options, ResourceKind::Terrain)?;
    let (terrain, water, water_appearance) =
        load_staged_terrain_and_water(&vfs, &resource_path, pixels_per_cell, compatibility)?;
    let cells = terrain.cells().len();
    let vertices = terrain.vertices().len();
    run_terrain_viewer(
        terrain,
        water,
        water_appearance,
        format!("Commanders in Chief - terrain - {resource_path}"),
    )?;
    Ok(format!(
        "closed terrain viewer for {resource_path} ({cells} cells, {vertices} vertices)\n"
    ))
}

fn load_staged_terrain_and_water(
    vfs: &Vfs,
    resource_path: &VirtualPath,
    pixels_per_cell: u32,
    compatibility: TerrainCompatibilityPolicy,
) -> Result<(StagedTerrain, StagedWater, WaterAppearance), Box<dyn Error>> {
    let entry = vfs
        .resolve(resource_path)
        .ok_or_else(|| format!("resource not found: {resource_path}"))?;
    let limits = MapLimits::default();
    let bytes = entry.read(limits.maximum_file_bytes)?;
    let map = parse_map(&bytes, resource_path.as_str(), limits)?;
    let height = decode_map_height(&map, limits)?;
    let blend = decode_map_blend(&map, &height, limits)?;
    let water = match decode_map_water(&map, limits) {
        Ok(water) => StagedWater::from_map(&water)?,
        Err(MapWaterError::MissingPolygonTriggers | MapWaterError::UnsupportedVersion(1)) => {
            StagedWater::empty()
        }
        Err(error) => return Err(error.into()),
    };
    let textures = load_terrain_textures(vfs, &blend)?;
    let staging = TerrainStagingOptions::new(pixels_per_cell)?.with_compatibility(compatibility);
    let terrain = StagedTerrain::from_map(&height, &blend, &textures, staging)?;
    let water_appearance = load_water_appearance(vfs)?;
    Ok((terrain, water, water_appearance))
}

fn load_water_appearance(vfs: &Vfs) -> Result<WaterAppearance, Box<dyn Error>> {
    let first_path = VirtualPath::new("art/textures/caust00.tga")?;
    let mut appearance = WaterAppearance::without_caustics();
    if vfs.resolve(&first_path).is_some() {
        let mut frames = Vec::with_capacity(32);
        let mut dimensions = None;
        for index in 0..32 {
            let path = VirtualPath::new(&format!("art/textures/caust{index:02}.tga"))?;
            let entry = vfs
                .resolve(&path)
                .ok_or_else(|| format!("incomplete caustic sequence: missing {path}"))?;
            let bytes = entry.read(MAX_ENCODED_IMAGE_BYTES)?;
            let image = decode_viewer_texture(&bytes, image::ImageFormat::Tga)?;
            let current = (image.width(), image.height());
            if dimensions.is_some_and(|expected| expected != current) {
                return Err(format!("caustic frame dimensions disagree at {path}").into());
            }
            dimensions = Some(current);
            frames.push(image.pixels().map(|pixel| pixel[0]).collect());
        }
        let (width, height) = dimensions.ok_or("caustic sequence is empty")?;
        appearance =
            WaterAppearance::with_caustics(WaterCausticSequence::new(width, height, 16, frames)?);
    }
    let mut deep_opacity = appearance.deep_opacity();
    let mut opaque_depth = appearance.opaque_depth();
    for raw_path in ["data/ini/default/water.ini", "data/ini/water.ini"] {
        let path = VirtualPath::new(raw_path)?;
        let Some(entry) = vfs.resolve(&path) else {
            continue;
        };
        let limits = WaterIniLimits::default();
        let bytes = entry.read(limits.max_file_bytes)?;
        let parsed = parse_water_transparency_ini(&bytes, limits)?;
        deep_opacity = parsed.minimum_opacity().unwrap_or(deep_opacity);
        opaque_depth = parsed.opaque_depth().unwrap_or(opaque_depth);
    }
    Ok(appearance.with_transparency(deep_opacity, opaque_depth)?)
}

fn load_staged_terrain(
    vfs: &Vfs,
    resource_path: &VirtualPath,
    pixels_per_cell: u32,
    compatibility: TerrainCompatibilityPolicy,
) -> Result<StagedTerrain, Box<dyn Error>> {
    let entry = vfs
        .resolve(resource_path)
        .ok_or_else(|| format!("resource not found: {resource_path}"))?;
    let limits = MapLimits::default();
    let bytes = entry.read(limits.maximum_file_bytes)?;
    let map = parse_map(&bytes, resource_path.as_str(), limits)?;
    let height = decode_map_height(&map, limits)?;
    let blend = decode_map_blend(&map, &height, limits)?;
    let textures = load_terrain_textures(vfs, &blend)?;
    let staging = TerrainStagingOptions::new(pixels_per_cell)?.with_compatibility(compatibility);
    Ok(StagedTerrain::from_map(
        &height, &blend, &textures, staging,
    )?)
}

fn parse_terrain_policy(value: &str) -> Result<TerrainCompatibilityPolicy, Box<dyn Error>> {
    match value {
        "legacy" => Ok(TerrainCompatibilityPolicy::ZeroHourLegacy),
        "modern" => Ok(TerrainCompatibilityPolicy::Modern),
        _ => Err(format!("unknown terrain policy {value:?}; expected legacy or modern").into()),
    }
}

const fn terrain_policy_name(policy: TerrainCompatibilityPolicy) -> &'static str {
    match policy {
        TerrainCompatibilityPolicy::ZeroHourLegacy => "legacy",
        TerrainCompatibilityPolicy::Modern => "modern",
    }
}

fn default_terrain_render_path(resource_path: &VirtualPath) -> Result<PathBuf, Box<dyn Error>> {
    let stem = Path::new(resource_path.as_str())
        .file_stem()
        .ok_or("MAP resource path has no file name")?;
    Ok(PathBuf::from(format!("{}-terrain", stem.to_string_lossy())).with_extension("png"))
}

fn load_terrain_textures(
    vfs: &Vfs,
    blend: &cic_formats::MapBlendData,
) -> Result<TextureResourceManager, Box<dyn Error>> {
    let mut textures = TextureResourceManager::default();
    let catalog = load_terrain_texture_catalog(vfs)?;
    for class in blend
        .texture_classes()
        .iter()
        .chain(blend.edge_texture_classes())
    {
        if textures.contains_alias(class.name_bytes()) {
            continue;
        }
        let (path, bytes) = resolve_terrain_texture(vfs, class.name_bytes(), &catalog)?;
        let image = decode_viewer_texture(&bytes, image_format(&path)?)?;
        textures.insert(
            class.name_bytes(),
            image.width(),
            image.height(),
            image.into_raw(),
        )?;
    }
    Ok(textures)
}

fn load_terrain_texture_catalog(vfs: &Vfs) -> Result<BTreeMap<Vec<u8>, Vec<u8>>, Box<dyn Error>> {
    let mut catalog = BTreeMap::new();
    for raw_path in ["data/ini/default/terrain.ini", "data/ini/terrain.ini"] {
        let path = VirtualPath::new(raw_path)?;
        let Some(entry) = vfs.resolve(&path) else {
            continue;
        };
        let limits = TerrainIniLimits::default();
        let bytes = entry.read(limits.max_file_bytes)?;
        let ini = parse_terrain_ini(&bytes, limits)?;
        for definition in ini.definitions() {
            let key = ascii_fold(definition.name_bytes());
            let inherited = catalog
                .get(&key)
                .cloned()
                .or_else(|| catalog.get(b"defaultterrain".as_slice()).cloned());
            if let Some(texture) = definition.texture_bytes().map(Vec::from).or(inherited) {
                catalog.insert(key, texture);
            }
        }
    }
    Ok(catalog)
}

fn resolve_terrain_texture(
    vfs: &Vfs,
    class_name: &[u8],
    catalog: &BTreeMap<Vec<u8>, Vec<u8>>,
) -> Result<(VirtualPath, Vec<u8>), Box<dyn Error>> {
    if let Some(texture_name) = catalog.get(&ascii_fold(class_name)) {
        return resolve_image(vfs, texture_name, "art/terrain").map_err(|error| {
            format!(
                "terrain class {} maps to {} but its image could not be loaded: {error}",
                String::from_utf8_lossy(class_name),
                String::from_utf8_lossy(texture_name)
            )
            .into()
        });
    }
    resolve_texture(vfs, class_name)
}

fn ascii_fold(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().map(u8::to_ascii_lowercase).collect()
}

fn view_model<I>(
    arguments: &mut std::iter::Peekable<I>,
    options: &CliOptions,
) -> Result<String, Box<dyn Error>>
where
    I: Iterator<Item = String>,
{
    let resource_name = arguments.next().ok_or("w3d-view requires a virtual path")?;
    let resource_path = VirtualPath::new(&resource_name)?;
    let mounts = arguments.collect::<Vec<_>>();
    let vfs = mount_all("w3d-view", &mounts, options, ResourceKind::W3dWithTextures)?;
    let model = load_composed_model(&vfs, &resource_path)?;
    let textures = load_renderer_textures(&vfs, &model)?;
    let staged = AnimatedModel::from_w3d_with_textures(&model, textures)?;
    let animation_count = staged.animation_count();
    let material_count = staged.material_count();
    let texture_count = staged.unique_texture_count();
    let texture_alias_count = staged.texture_alias_count();
    eprintln!(
        "viewer resources: {material_count} materials, {texture_count} unique textures from {texture_alias_count} names"
    );
    run_model_viewer(staged, format!("Commanders in Chief — {resource_path}"))?;
    Ok(format!(
        "closed viewer for {resource_path} ({animation_count} animations, {material_count} materials, {texture_count} unique textures from {texture_alias_count} names)\n"
    ))
}

fn load_renderer_textures(
    vfs: &Vfs,
    model: &cic_formats::W3dModel,
) -> Result<TextureResourceManager, Box<dyn Error>> {
    let mut resources = TextureResourceManager::default();
    let mut resolved_images: BTreeMap<String, TextureId> = BTreeMap::new();
    for model_mesh in model.meshes() {
        let mesh = model_mesh.mesh();
        for pass in mesh.materials().passes() {
            for stage in pass.texture_stages() {
                for triangle in 0..mesh.triangles().len() {
                    let texturing_disabled = pass
                        .shader_ids()
                        .and_then(|ids| ids.for_triangle(triangle))
                        .and_then(|id| usize::try_from(id).ok())
                        .and_then(|id| mesh.materials().shaders().get(id))
                        .is_some_and(|shader| shader.texturing() == 0);
                    if texturing_disabled {
                        continue;
                    }
                    let Some(texture) = stage
                        .texture_ids()
                        .and_then(|ids| ids.for_triangle(triangle))
                        .filter(|id| *id != u32::MAX)
                        .and_then(|id| usize::try_from(id).ok())
                        .and_then(|id| mesh.materials().textures().get(id))
                    else {
                        continue;
                    };
                    if resources.contains_alias(texture.name_bytes()) {
                        continue;
                    }
                    match resolve_texture(vfs, texture.name_bytes()) {
                        Ok((path, bytes)) => {
                            if let Some(existing) = resolved_images.get(path.as_str()) {
                                resources.insert_alias(texture.name_bytes(), *existing)?;
                                continue;
                            }
                            let format = image_format(&path)?;
                            let image = decode_viewer_texture(&bytes, format)?;
                            let id = resources.insert(
                                texture.name_bytes(),
                                image.width(),
                                image.height(),
                                image.into_raw(),
                            )?;
                            resolved_images.insert(path.to_string(), id);
                        }
                        Err(error) => {
                            eprintln!(
                                "warning: {error}; using a magenta viewer placeholder for {}",
                                String::from_utf8_lossy(texture.name_bytes())
                            );
                            resources.insert(texture.name_bytes(), 1, 1, vec![255, 0, 255, 255])?;
                        }
                    }
                }
            }
        }
    }
    Ok(resources)
}

fn decode_viewer_texture(
    bytes: &[u8],
    format: image::ImageFormat,
) -> Result<image::RgbaImage, image::ImageError> {
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8_192);
    limits.max_image_height = Some(8_192);
    limits.max_alloc = Some(256 * 1_024 * 1_024);
    reader.limits(limits);
    Ok(reader.decode()?.to_rgba8())
}

fn export_model<I>(
    arguments: &mut std::iter::Peekable<I>,
    options: &CliOptions,
) -> Result<String, Box<dyn Error>>
where
    I: Iterator<Item = String>,
{
    let format = if arguments
        .peek()
        .is_some_and(|argument| argument == "--gltf")
    {
        arguments.next();
        ExportFormat::Gltf
    } else {
        ExportFormat::Glb
    };
    let resource_name = arguments
        .next()
        .ok_or("w3d-export requires a virtual path")?;
    let resource_path = VirtualPath::new(&resource_name)?;
    let remaining = arguments.collect::<Vec<_>>();
    let (output_path, mounts) = if remaining
        .first()
        .is_some_and(|candidate| has_export_extension(Path::new(candidate)))
    {
        (PathBuf::from(&remaining[0]), remaining[1..].to_vec())
    } else {
        (default_export_path(&resource_path, format)?, remaining)
    };
    validate_export_extension(format, &output_path)?;
    let vfs = mount_all(
        "w3d-export",
        &mounts,
        options,
        ResourceKind::W3dWithTextures,
    )?;
    let model = load_composed_model(&vfs, &resource_path)?;
    write_model_export(&vfs, &model, &output_path, format)?;
    Ok(format!("wrote {}\n", output_path.display()))
}

fn render_model_capture<I>(
    arguments: &mut std::iter::Peekable<I>,
    options: &CliOptions,
) -> Result<String, Box<dyn Error>>
where
    I: Iterator<Item = String>,
{
    let mut animation = None;
    let mut frame = 0_u32;
    let mut mapper_time_seconds = 0.0_f32;
    let mut rotation = 0.0_f32;
    while arguments
        .peek()
        .is_some_and(|argument| argument.starts_with("--"))
    {
        let option = arguments.next().expect("peeked renderer option");
        let value = arguments
            .next()
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option.as_str() {
            "--animation" => animation = Some(value.parse::<usize>()?),
            "--frame" => frame = value.parse::<u32>()?,
            "--time" => mapper_time_seconds = value.parse::<f32>()?,
            "--rotation" => rotation = value.parse::<f32>()?,
            _ => return Err(format!("unknown w3d-render option {option:?}").into()),
        }
    }
    let resource_name = arguments
        .next()
        .ok_or("w3d-render requires a virtual path")?;
    let resource_path = VirtualPath::new(&resource_name)?;
    let remaining = arguments.collect::<Vec<_>>();
    let (output_path, mounts) = if remaining.first().is_some_and(|candidate| {
        Path::new(candidate)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("ppm"))
    }) {
        (PathBuf::from(&remaining[0]), remaining[1..].to_vec())
    } else {
        (default_render_path(&resource_path)?, remaining)
    };
    if !output_path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ppm"))
    {
        return Err("W3D render capture requires a .ppm output path".into());
    }
    let vfs = mount_all(
        "w3d-render",
        &mounts,
        options,
        ResourceKind::W3dWithTextures,
    )?;
    let model = load_composed_model(&vfs, &resource_path)?;
    let textures = load_renderer_textures(&vfs, &model)?;
    let staged = AnimatedModel::from_w3d_with_textures(&model, textures)?;
    let explicit_frame = ModelFrame::new(animation, frame, mapper_time_seconds, rotation)?;
    let renderer = pollster::block_on(HeadlessRenderer::new())?;
    let capture = renderer.capture_animated_model(512, 512, &staged, explicit_frame)?;
    fs::write(&output_path, capture.ppm())?;
    Ok(format!(
        "adapter\t{}\nanimation\t{}\nframe\t{}\nmapper_time_seconds\t{}\nvertices\t{}\nindices\t{}\ndraws\t{}\nmaterials\t{}\ntextures\t{}\nrgba_sha256\t{}\nwrote\t{}\n",
        renderer.adapter_info().name,
        animation.map_or_else(|| "bind".to_owned(), |index| index.to_string()),
        frame,
        mapper_time_seconds,
        staged.vertex_count(),
        staged.index_count(),
        staged.draw_count(),
        staged.material_count(),
        staged.unique_texture_count(),
        capture.sha256(),
        output_path.display()
    ))
}

fn default_render_path(resource_path: &VirtualPath) -> Result<PathBuf, Box<dyn Error>> {
    let stem = Path::new(resource_path.as_str())
        .file_stem()
        .ok_or("W3D resource path has no file name")?;
    Ok(PathBuf::from(stem).with_extension("ppm"))
}

fn load_composed_model(
    vfs: &Vfs,
    resource_path: &VirtualPath,
) -> Result<cic_formats::W3dModel, Box<dyn Error>> {
    let entry = vfs
        .resolve(resource_path)
        .ok_or_else(|| format!("resource not found: {resource_path}"))?;
    let limits = W3dLimits::default();
    let bytes = entry.read(limits.maximum_file_bytes)?;
    let w3d = parse_w3d(&bytes, resource_path.as_str(), limits)?;
    let files = collect_model_files(vfs, resource_path, w3d)?;
    let file_refs = files.iter().collect::<Vec<_>>();
    Ok(decode_w3d_model_set(
        &file_refs,
        W3dMeshLimits::default(),
        W3dSceneLimits::default(),
    )?)
}

fn has_export_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("glb") || extension.eq_ignore_ascii_case("gltf")
        })
}

fn default_export_path(
    resource_path: &VirtualPath,
    format: ExportFormat,
) -> Result<PathBuf, Box<dyn Error>> {
    let stem = Path::new(resource_path.as_str())
        .file_stem()
        .ok_or("W3D resource path has no file name")?;
    let extension = match format {
        ExportFormat::Glb => "glb",
        ExportFormat::Gltf => "gltf",
    };
    Ok(PathBuf::from(stem).with_extension(extension))
}

fn validate_export_extension(format: ExportFormat, path: &Path) -> Result<(), Box<dyn Error>> {
    let expected = match format {
        ExportFormat::Glb => "glb",
        ExportFormat::Gltf => "gltf",
    };
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(expected))
    {
        Ok(())
    } else {
        Err(format!(
            "{} export requires a .{expected} output path",
            expected.to_uppercase()
        )
        .into())
    }
}

fn collect_model_files(
    vfs: &Vfs,
    resource_path: &VirtualPath,
    primary: W3dFile,
) -> Result<Vec<W3dFile>, Box<dyn Error>> {
    let hierarchy_name = w3d_model_hierarchy_name(&primary)?;
    let mut files = vec![primary];
    let Some(hierarchy_name) = hierarchy_name else {
        return Ok(files);
    };
    let hierarchy_name = std::str::from_utf8(&hierarchy_name)
        .map_err(|_| "W3D hierarchy resource name is not UTF-8")?
        .to_ascii_lowercase();
    let directory = resource_path
        .as_str()
        .rsplit_once('/')
        .map_or("", |(directory, _)| directory);
    let companion_name = if directory.is_empty() {
        format!("{hierarchy_name}.w3d")
    } else {
        format!("{directory}/{hierarchy_name}.w3d")
    };
    let companion_path = VirtualPath::new(&companion_name)?;
    if !files[0].chunks().iter().any(|chunk| chunk.id() == 0x100) {
        let entry = vfs
            .resolve(&companion_path)
            .ok_or_else(|| format!("referenced W3D hierarchy not found: {companion_path}"))?;
        let limits = W3dLimits::default();
        let bytes = entry.read(limits.maximum_file_bytes)?;
        files.push(parse_w3d(&bytes, companion_path.as_str(), limits)?);
    }

    let animation_prefix = hierarchy_name
        .strip_suffix("_skl")
        .unwrap_or(&hierarchy_name);
    let file_prefix = if directory.is_empty() {
        format!("{animation_prefix}_")
    } else {
        format!("{directory}/{animation_prefix}_")
    };
    for (path, entry) in vfs.iter_resolved() {
        let name = path.as_str();
        if name == resource_path.as_str()
            || name == companion_path.as_str()
            || !name.starts_with(&file_prefix)
            || !Path::new(name)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("w3d"))
        {
            continue;
        }
        let limits = W3dLimits::default();
        let bytes = entry.read(limits.maximum_file_bytes)?;
        let candidate = parse_w3d(&bytes, name, limits)?;
        if !candidate.chunks().is_empty()
            && candidate
                .chunks()
                .iter()
                .all(|chunk| matches!(chunk.id(), 0x200 | 0x280))
        {
            files.push(candidate);
        }
    }
    Ok(files)
}

fn configure(mut arguments: impl Iterator<Item = String>) -> Result<String, Box<dyn Error>> {
    let action = arguments.next().ok_or("config requires show or set")?;
    let path = config_path()?;
    match action.as_str() {
        "show" => {
            if arguments.next().is_some() {
                return Err("config show takes no arguments".into());
            }
            let stored = StoredLocations::load(&path)?;
            let discovered = discover_steam_locations();
            Ok(format!(
                "config\t{}\nstored-generals\t{}\nstored-zero-hour\t{}\ndetected-generals\t{}\ndetected-zero-hour\t{}\n",
                path.display(),
                display_path(stored.generals.as_deref()),
                display_path(stored.zero_hour.as_deref()),
                display_path(discovered.generals.as_deref()),
                display_path(discovered.zero_hour.as_deref())
            ))
        }
        "set" => {
            let key = arguments.next().ok_or("config set requires a key")?;
            let value = PathBuf::from(arguments.next().ok_or("config set requires a path")?);
            if arguments.next().is_some() {
                return Err("config set received extra arguments".into());
            }
            let mut stored = StoredLocations::load(&path)?;
            match key.as_str() {
                "generals-dir" => {
                    validate_installation(GameEdition::Generals, &value)?;
                    stored.generals = Some(value);
                }
                "zero-hour-dir" => {
                    validate_installation(GameEdition::ZeroHour, &value)?;
                    stored.zero_hour = Some(value);
                }
                _ => return Err(format!("unknown config key {key:?}").into()),
            }
            stored.save(&path)?;
            Ok(format!("wrote {}\n", path.display()))
        }
        _ => Err(format!("unknown config action {action:?}").into()),
    }
}

fn display_path(path: Option<&Path>) -> String {
    path.map_or_else(String::new, |path| path.display().to_string())
}

fn write_model_export(
    vfs: &Vfs,
    model: &cic_formats::W3dModel,
    output_path: &Path,
    format: ExportFormat,
) -> Result<(), Box<dyn Error>> {
    match format {
        ExportFormat::Glb => write_glb(vfs, model, output_path),
        ExportFormat::Gltf => write_gltf(vfs, model, output_path),
    }
}

fn write_glb(
    vfs: &Vfs,
    model: &cic_formats::W3dModel,
    output_path: &Path,
) -> Result<(), Box<dyn Error>> {
    let bundle = render_w3d_gltf(model, "embedded.bin", "embedded_textures");
    let images = encode_png_textures(vfs, &bundle.textures)?;
    let png_images = images
        .into_iter()
        .map(|image| {
            println!("texture {} -> embedded PNG", image.source_name);
            image.bytes
        })
        .collect::<Vec<_>>();
    fs::write(output_path, pack_w3d_glb(bundle, &png_images)?)?;
    Ok(())
}

fn write_gltf(
    vfs: &Vfs,
    model: &cic_formats::W3dModel,
    output_path: &Path,
) -> Result<(), Box<dyn Error>> {
    let parent = output_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = output_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or("glTF output path requires a UTF-8 file stem")?;
    let binary_name = format!("{stem}.bin");
    let texture_directory_name = format!("{stem}_textures");
    let bundle = render_w3d_gltf(model, &binary_name, &texture_directory_name);
    let images = encode_png_textures(vfs, &bundle.textures)?;
    fs::write(output_path, bundle.json)?;
    fs::write(parent.join(binary_name), bundle.binary)?;
    if !bundle.textures.is_empty() {
        let texture_directory = parent.join(&texture_directory_name);
        fs::create_dir_all(&texture_directory)?;
        for (texture, image) in bundle.textures.into_iter().zip(images) {
            fs::write(texture_directory.join(texture.output_name()), image.bytes)?;
            println!(
                "texture {} -> {texture_directory_name}/{}",
                image.source_name,
                texture.output_name()
            );
        }
    }
    Ok(())
}

struct EncodedTexture {
    source_name: String,
    bytes: Vec<u8>,
}

fn encode_png_textures(
    vfs: &Vfs,
    requests: &[GltfTextureRequest],
) -> Result<Vec<EncodedTexture>, Box<dyn Error>> {
    requests
        .iter()
        .map(|texture| encode_png_texture(vfs, texture))
        .collect()
}

fn encode_png_texture(
    vfs: &Vfs,
    texture: &GltfTextureRequest,
) -> Result<EncodedTexture, Box<dyn Error>> {
    let resolved = resolve_texture(vfs, texture.source_name_bytes());
    let (source_name, image) = match resolved {
        Ok((source_path, bytes)) => {
            let format = image_format(&source_path)?;
            let image = image::load_from_memory_with_format(&bytes, format)?;
            (source_path.to_string(), image)
        }
        Err(error) => {
            eprintln!(
                "warning: {error}; writing a magenta placeholder for {}",
                String::from_utf8_lossy(texture.source_name_bytes())
            );
            let image =
                image::RgbaImage::from_pixel(1, 1, image::Rgba([u8::MAX, 0, u8::MAX, u8::MAX]));
            (
                "missing texture".to_owned(),
                image::DynamicImage::ImageRgba8(image),
            )
        }
    };
    let mut rgba = image.to_rgba8();
    if texture.is_additive_preview() {
        apply_additive_preview_alpha(&mut rgba);
    }
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, rgba.width(), rgba.height());
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(rgba.as_raw())?;
    }
    Ok(EncodedTexture { source_name, bytes })
}

/// Converts a black-backed additive source image into a deterministic straight-alpha preview.
///
/// Core glTF only defines source-over alpha blending. W3D `ONE + ONE` materials instead add the
/// source RGB directly and ignore its alpha for color. Treat the largest color channel as coverage
/// and unassociate the other channels from that coverage. This keeps black pixels invisible and
/// retains the source color ratios without changing the separately packaged source image.
fn apply_additive_preview_alpha(image: &mut image::RgbaImage) {
    for pixel in image.pixels_mut() {
        let strength = pixel[0].max(pixel[1]).max(pixel[2]);
        if strength == 0 {
            pixel[3] = 0;
            continue;
        }
        let strength_u16 = u16::from(strength);
        for channel in &mut pixel.0[..3] {
            let numerator = u16::from(*channel) * 255 + strength_u16 / 2;
            *channel = u8::try_from(numerator / strength_u16)
                .expect("normalized additive channel fits u8");
        }
        pixel[3] = strength;
    }
}

fn image_format(path: &VirtualPath) -> Result<image::ImageFormat, Box<dyn Error>> {
    match Path::new(path.as_str())
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("dds") => Ok(image::ImageFormat::Dds),
        Some("tga") => Ok(image::ImageFormat::Tga),
        Some("png") => Ok(image::ImageFormat::Png),
        extension => Err(format!("unsupported texture image format: {extension:?}").into()),
    }
}

fn resolve_texture(vfs: &Vfs, raw_name: &[u8]) -> Result<(VirtualPath, Vec<u8>), Box<dyn Error>> {
    resolve_image(vfs, raw_name, "art/textures")
}

fn resolve_image(
    vfs: &Vfs,
    raw_name: &[u8],
    resource_directory: &str,
) -> Result<(VirtualPath, Vec<u8>), Box<dyn Error>> {
    let name = std::str::from_utf8(raw_name)
        .map_err(|_| "texture name is not UTF-8 and cannot be mapped to the VFS")?;
    let normalized = name.replace('\\', "/");
    let basename = normalized
        .rsplit('/')
        .next()
        .ok_or("texture name is empty")?;
    let mut candidates = vec![
        normalized.clone(),
        format!("{resource_directory}/{normalized}"),
    ];
    if basename != normalized {
        candidates.push(format!("{resource_directory}/{basename}"));
    }
    let original_candidates = candidates.clone();
    for candidate in &original_candidates {
        if candidate
            .get(candidate.len().saturating_sub(4)..)
            .is_some_and(|extension| extension.eq_ignore_ascii_case(".tga"))
        {
            candidates.push(format!("{}.dds", &candidate[..candidate.len() - 4]));
        }
    }
    if Path::new(basename).extension().is_none() {
        for extension in ["tga", "dds", "png"] {
            candidates.push(format!("{normalized}.{extension}"));
            candidates.push(format!("{resource_directory}/{normalized}.{extension}"));
            if basename != normalized {
                candidates.push(format!("{resource_directory}/{basename}.{extension}"));
            }
        }
    }
    let mut checked = Vec::new();
    for candidate in candidates {
        if checked.contains(&candidate) {
            continue;
        }
        checked.push(candidate.clone());
        let path = VirtualPath::new(&candidate)?;
        if let Some(entry) = vfs.resolve(&path) {
            return Ok((path, entry.read(MAX_ENCODED_IMAGE_BYTES)?));
        }
    }
    Err(format!("referenced texture not found in mounted resources: {name}").into())
}

fn mount_all(
    command: &str,
    mounts: &[String],
    options: &CliOptions,
    kind: ResourceKind,
) -> Result<Vfs, Box<dyn Error>> {
    let mut planned = if let Some(profile_path) = options.profile.as_deref() {
        let profile = MountProfile::load(profile_path, MountProfileLimits::default())?;
        let mut paths = Vec::with_capacity(profile.mounts().len());
        for mount in profile.mounts() {
            if mount.optional() && !mount.path().try_exists()? {
                continue;
            }
            paths.push(mount.path().to_path_buf());
        }
        paths
    } else if mounts.is_empty() {
        resolve_archives(options.edition, kind, options.game_dir.as_deref())?
    } else {
        mounts.iter().map(PathBuf::from).collect()
    };
    if options.profile.is_some() {
        planned.extend(mounts.iter().map(PathBuf::from));
    }
    planned.extend(options.mods.iter().cloned());
    if planned.is_empty() {
        return Err(format!("{command} resolved no resource archives").into());
    }
    let mut vfs = Vfs::new();
    for (index, mount) in planned.iter().enumerate() {
        let metadata = fs::metadata(mount)?;
        let provider_name = format!("mount-{index}");
        if metadata.is_dir() {
            vfs.mount_directory(provider_name, mount)?;
        } else if metadata.is_file() {
            vfs.mount_big_file(provider_name, mount, BigLimits::default())?;
        } else {
            return Err(format!(
                "mount is neither a directory nor a regular file: {}",
                mount.display()
            )
            .into());
        }
    }
    Ok(vfs)
}

#[cfg(test)]
mod tests {
    use super::apply_additive_preview_alpha;

    #[test]
    fn additive_preview_makes_black_transparent_and_unassociates_color() {
        let mut image =
            image::RgbaImage::from_raw(3, 1, vec![0, 0, 0, 255, 64, 32, 0, 0, 255, 128, 64, 17])
                .expect("fixture dimensions");

        apply_additive_preview_alpha(&mut image);

        assert_eq!(
            image.as_raw(),
            &[0, 0, 0, 0, 255, 128, 0, 64, 255, 128, 64, 255]
        );
    }
}
