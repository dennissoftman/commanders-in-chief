use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use cic_formats::{
    CsfLimits, W3dFile, W3dLimits, W3dMeshLimits, W3dSceneLimits, decode_static_mesh,
    decode_w3d_model_set, parse_csf, parse_w3d, w3d_model_hierarchy_name,
};
use cic_render::{HeadlessRenderer, StagedModel};
use cic_tools::resource::{
    GameEdition, ResourceKind, StoredLocations, config_path, discover_steam_locations,
    resolve_archives, validate_installation,
};
use cic_tools::{
    GltfTextureRequest, pack_w3d_glb, render_csf, render_manifest, render_w3d, render_w3d_gltf,
    render_w3d_mesh,
};
use cic_vfs::{BigLimits, Vfs, VirtualPath};

const USAGE: &str = "Usage:\n\
  cic-inspect [--zh] [--game-dir <path>] <command> ...\n\
  cic-inspect config show\n\
  cic-inspect config set <generals-dir|zero-hour-dir> <path>\n\
  cic-inspect manifest <mount> [<mount> ...]\n\
  cic-inspect csf <virtual-path> <mount> [<mount> ...]\n\
  cic-inspect w3d <virtual-path> <mount> [<mount> ...]\n\
  cic-inspect w3d-mesh <virtual-path> <top-level-index> <mount> [<mount> ...]\n\
  cic-inspect w3d-render <virtual-path> [<output.ppm>] [<mount> ...]\n\
  cic-inspect w3d-export [--gltf] <virtual-path> [<output.glb|output.gltf>] [<mount> ...]\n\
Each mount is a directory or BIG archive. Mounts are applied from left to right; later mounts override earlier mounts.";

#[derive(Debug)]
struct CliOptions {
    edition: GameEdition,
    game_dir: Option<PathBuf>,
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
    let mut options = CliOptions {
        edition: GameEdition::Generals,
        game_dir: None,
    };
    while let Some(argument) = arguments.peek() {
        match argument.as_str() {
            "--zh" => {
                options.edition = GameEdition::ZeroHour;
                arguments.next();
            }
            "--game-dir" => {
                arguments.next();
                options.game_dir = Some(PathBuf::from(
                    arguments.next().ok_or("--game-dir requires a path")?,
                ));
            }
            _ => break,
        }
    }
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
            let csf = parse_csf(entry.bytes(), resource_path.as_str(), CsfLimits::default())?;
            Ok(render_csf(&csf))
        }
        "w3d" => {
            let resource_name = arguments.next().ok_or("w3d requires a virtual path")?;
            let mounts = arguments.collect::<Vec<_>>();
            let vfs = mount_all("w3d", &mounts, &options, ResourceKind::W3d)?;
            let resource_path = VirtualPath::new(&resource_name)?;
            let entry = vfs
                .resolve(&resource_path)
                .ok_or_else(|| format!("resource not found: {resource_path}"))?;
            let w3d = parse_w3d(entry.bytes(), resource_path.as_str(), W3dLimits::default())?;
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
            let w3d = parse_w3d(entry.bytes(), resource_path.as_str(), W3dLimits::default())?;
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
        "w3d-export" => export_model(&mut arguments, &options),
        _ => Err(format!("unknown command {command:?}").into()),
    }
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
    let vfs = mount_all("w3d-render", &mounts, options, ResourceKind::W3d)?;
    let model = load_composed_model(&vfs, &resource_path)?;
    let staged = StagedModel::from_w3d(&model)?;
    let renderer = pollster::block_on(HeadlessRenderer::new())?;
    let capture = renderer.capture_model(512, 512, &staged)?;
    fs::write(&output_path, capture.ppm())?;
    Ok(format!(
        "adapter\t{}\nvertices\t{}\nindices\t{}\nrgba_sha256\t{}\nwrote\t{}\n",
        renderer.adapter_info().name,
        staged.vertex_count(),
        staged.index_count(),
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
    let w3d = parse_w3d(entry.bytes(), resource_path.as_str(), W3dLimits::default())?;
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
        files.push(parse_w3d(
            entry.bytes(),
            companion_path.as_str(),
            W3dLimits::default(),
        )?);
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
        let candidate = parse_w3d(entry.bytes(), name, W3dLimits::default())?;
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
            let image = image::load_from_memory_with_format(bytes, format)?;
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
        extension => Err(format!("unsupported W3D texture image format: {extension:?}").into()),
    }
}

fn resolve_texture<'a>(
    vfs: &'a Vfs,
    raw_name: &[u8],
) -> Result<(VirtualPath, &'a [u8]), Box<dyn Error>> {
    let name = std::str::from_utf8(raw_name)
        .map_err(|_| "W3D texture name is not UTF-8 and cannot be mapped to the VFS")?;
    let normalized = name.replace('\\', "/");
    let basename = normalized
        .rsplit('/')
        .next()
        .ok_or("W3D texture name is empty")?;
    let mut candidates = vec![normalized.clone(), format!("art/textures/{normalized}")];
    if basename != normalized {
        candidates.push(format!("art/textures/{basename}"));
    }
    if let Some(stem) = basename
        .strip_suffix(".tga")
        .or_else(|| basename.strip_suffix(".TGA"))
    {
        candidates.push(format!("art/textures/{stem}.dds"));
    }
    for candidate in candidates {
        let path = VirtualPath::new(&candidate)?;
        if let Some(entry) = vfs.resolve(&path) {
            return Ok((path, entry.bytes()));
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
    let mounts = if mounts.is_empty() {
        resolve_archives(options.edition, kind, options.game_dir.as_deref())?
    } else {
        mounts.iter().map(PathBuf::from).collect()
    };
    if mounts.is_empty() {
        return Err(format!("{command} resolved no resource archives").into());
    }
    let mut vfs = Vfs::new();
    for (index, mount) in mounts.iter().enumerate() {
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
