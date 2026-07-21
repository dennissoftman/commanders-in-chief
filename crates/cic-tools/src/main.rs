use std::env;
use std::error::Error;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use cic_formats::{
    CsfLimits, W3dFile, W3dLimits, W3dMeshLimits, W3dSceneLimits, decode_static_mesh,
    decode_w3d_model_set, parse_csf, parse_w3d, w3d_model_hierarchy_name,
};
use cic_tools::resource::{
    GameEdition, ResourceKind, StoredLocations, config_path, discover_steam_locations,
    resolve_archives, validate_installation,
};
use cic_tools::{render_csf, render_manifest, render_w3d, render_w3d_gltf, render_w3d_mesh};
use cic_vfs::{BigLimits, Vfs, VirtualPath};

const USAGE: &str = "Usage:\n\
  cic-inspect [--zh] [--game-dir <path>] <command> ...\n\
  cic-inspect config show\n\
  cic-inspect config set <generals-dir|zero-hour-dir> <path>\n\
  cic-inspect manifest <mount> [<mount> ...]\n\
  cic-inspect csf <virtual-path> <mount> [<mount> ...]\n\
  cic-inspect w3d <virtual-path> <mount> [<mount> ...]\n\
  cic-inspect w3d-mesh <virtual-path> <top-level-index> <mount> [<mount> ...]\n\
  cic-inspect w3d-gltf <virtual-path> <output.gltf> [<mount> ...]\n\
Each mount is a directory or BIG archive. Mounts are applied from left to right; later mounts override earlier mounts.";

#[derive(Debug)]
struct CliOptions {
    edition: GameEdition,
    game_dir: Option<PathBuf>,
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
        "w3d-gltf" => {
            let resource_name = arguments.next().ok_or("w3d-gltf requires a virtual path")?;
            let output_path = arguments.next().ok_or("w3d-gltf requires an output path")?;
            let mounts = arguments.collect::<Vec<_>>();
            let vfs = mount_all("w3d-gltf", &mounts, &options, ResourceKind::W3dWithTextures)?;
            let resource_path = VirtualPath::new(&resource_name)?;
            let entry = vfs
                .resolve(&resource_path)
                .ok_or_else(|| format!("resource not found: {resource_path}"))?;
            let w3d = parse_w3d(entry.bytes(), resource_path.as_str(), W3dLimits::default())?;
            let files = collect_model_files(&vfs, &resource_path, w3d)?;
            let file_refs = files.iter().collect::<Vec<_>>();
            let model = decode_w3d_model_set(
                &file_refs,
                W3dMeshLimits::default(),
                W3dSceneLimits::default(),
            )?;
            write_gltf_bundle(&vfs, &model, Path::new(&output_path))?;
            Ok(format!("wrote {output_path}\n"))
        }
        _ => Err(format!("unknown command {command:?}").into()),
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
            && candidate.chunks().iter().all(|chunk| chunk.id() == 0x200)
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

fn write_gltf_bundle(
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
    fs::write(output_path, bundle.json)?;
    fs::write(parent.join(binary_name), bundle.binary)?;
    if !bundle.textures.is_empty() {
        let texture_directory = parent.join(&texture_directory_name);
        fs::create_dir_all(&texture_directory)?;
        for texture in bundle.textures {
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
                    let image = image::RgbaImage::from_pixel(
                        1,
                        1,
                        image::Rgba([u8::MAX, 0, u8::MAX, u8::MAX]),
                    );
                    (
                        "missing texture".to_owned(),
                        image::DynamicImage::ImageRgba8(image),
                    )
                }
            };
            let mut png = Cursor::new(Vec::new());
            image.write_to(&mut png, image::ImageFormat::Png)?;
            fs::write(
                texture_directory.join(texture.output_name()),
                png.into_inner(),
            )?;
            println!(
                "texture {source_name} -> {texture_directory_name}/{}",
                texture.output_name()
            );
        }
    }
    Ok(())
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
