//! Installed-game discovery and explicit resource-edition mount policies.

use std::collections::BTreeSet;
use std::env;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// User-selectable retail resource edition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameEdition {
    /// Command & Conquer: Generals resources.
    Generals,
    /// Zero Hour resources layered over their required Generals base resources.
    ZeroHour,
}

/// Command-specific archive subset used by an automatic resource mount.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    /// Every BIG archive in explicit policy order.
    Manifest,
    /// Localization archives.
    Localization,
    /// W3D model archives without texture images.
    W3d,
    /// W3D model and texture archives.
    W3dWithTextures,
}

/// Persisted installation roots. Missing values remain eligible for auto-detection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoredLocations {
    pub generals: Option<PathBuf>,
    pub zero_hour: Option<PathBuf>,
}

impl StoredLocations {
    /// Reads the versioned line-oriented configuration, if it exists.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or contains an unknown or malformed record.
    pub fn load(path: &Path) -> Result<Self, ResourceError> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => {
                return Err(ResourceError::Io {
                    path: path.to_path_buf(),
                    error,
                });
            }
        };
        let mut result = Self::default();
        for (line_number, line) in text.lines().enumerate() {
            let Some((key, value)) = line.split_once('=') else {
                return Err(ResourceError::InvalidConfig {
                    line: line_number + 1,
                });
            };
            match key {
                "version" if value == "1" => {}
                "generals_dir" => result.generals = nonempty_path(value),
                "zero_hour_dir" => result.zero_hour = nonempty_path(value),
                _ => {
                    return Err(ResourceError::InvalidConfig {
                        line: line_number + 1,
                    });
                }
            }
        }
        Ok(result)
    }

    /// Atomically saves the configuration beneath its platform config directory.
    ///
    /// # Errors
    ///
    /// Returns an error if a path cannot be encoded or the configuration cannot be written.
    pub fn save(&self, path: &Path) -> Result<(), ResourceError> {
        let parent = path
            .parent()
            .ok_or_else(|| ResourceError::InvalidConfigPath(path.to_path_buf()))?;
        fs::create_dir_all(parent).map_err(|error| ResourceError::Io {
            path: parent.to_path_buf(),
            error,
        })?;
        let generals = config_value(self.generals.as_deref())?;
        let zero_hour = config_value(self.zero_hour.as_deref())?;
        let text = format!("version=1\ngenerals_dir={generals}\nzero_hour_dir={zero_hour}\n");
        let temporary = path.with_extension("tmp");
        fs::write(&temporary, text).map_err(|error| ResourceError::Io {
            path: temporary.clone(),
            error,
        })?;
        if !path.exists() {
            return fs::rename(&temporary, path).map_err(|error| ResourceError::Io {
                path: path.to_path_buf(),
                error,
            });
        }
        let backup = path.with_extension("bak");
        if backup.exists() {
            fs::remove_file(&backup).map_err(|error| ResourceError::Io {
                path: backup.clone(),
                error,
            })?;
        }
        fs::rename(path, &backup).map_err(|error| ResourceError::Io {
            path: path.to_path_buf(),
            error,
        })?;
        match fs::rename(&temporary, path) {
            Ok(()) => fs::remove_file(&backup).map_err(|error| ResourceError::Io {
                path: backup,
                error,
            }),
            Err(error) => {
                let _ = fs::rename(&backup, path);
                Err(ResourceError::Io {
                    path: path.to_path_buf(),
                    error,
                })
            }
        }
    }
}

/// Returns the per-user configuration path, overridable for automation and tests.
///
/// # Errors
///
/// Returns an error when no supported per-user configuration directory is available.
pub fn config_path() -> Result<PathBuf, ResourceError> {
    if let Some(path) = env::var_os("CIC_CONFIG_PATH") {
        return Ok(PathBuf::from(path));
    }
    #[cfg(windows)]
    if let Some(root) = env::var_os("APPDATA") {
        return Ok(PathBuf::from(root).join("CommandersInChief").join("config"));
    }
    if let Some(root) = env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(root)
            .join("commanders-in-chief")
            .join("config"));
    }
    if let Some(root) = env::var_os("HOME") {
        return Ok(PathBuf::from(root)
            .join(".config")
            .join("commanders-in-chief")
            .join("config"));
    }
    Err(ResourceError::ConfigDirectoryUnavailable)
}

/// Resolves configured or discovered roots and returns a deterministic archive list.
///
/// # Errors
///
/// Returns an error when configuration cannot be read, an installation cannot be located or
/// validated, or its archive directory cannot be enumerated.
pub fn resolve_archives(
    edition: GameEdition,
    kind: ResourceKind,
    explicit_game_dir: Option<&Path>,
) -> Result<Vec<PathBuf>, ResourceError> {
    let stored = StoredLocations::load(&config_path()?)?;
    let discovered = discover_steam_locations();
    let generals = if edition == GameEdition::Generals {
        explicit_game_dir.map(Path::to_path_buf)
    } else {
        None
    }
    .or_else(|| env::var_os("CIC_GENERALS_DIR").map(PathBuf::from))
    .or(stored.generals)
    .or(discovered.generals)
    .ok_or(ResourceError::InstallationNotFound(GameEdition::Generals))?;
    validate_root(GameEdition::Generals, &generals)?;

    let zero_hour = if edition == GameEdition::ZeroHour {
        let root = explicit_game_dir
            .map(Path::to_path_buf)
            .or_else(|| env::var_os("CIC_ZERO_HOUR_DIR").map(PathBuf::from))
            .or(stored.zero_hour)
            .or(discovered.zero_hour)
            .ok_or(ResourceError::InstallationNotFound(GameEdition::ZeroHour))?;
        validate_root(GameEdition::ZeroHour, &root)?;
        Some(root)
    } else {
        None
    };

    let mut archives = edition_archives(GameEdition::Generals, kind, &generals)?;
    if let Some(root) = zero_hour {
        archives.extend(edition_archives(GameEdition::ZeroHour, kind, &root)?);
    }
    Ok(archives)
}

/// Finds Steam installations by validated archive sentinels.
#[must_use]
pub fn discover_steam_locations() -> StoredLocations {
    let mut libraries = steam_roots();
    let mut expanded = libraries.clone();
    for root in &libraries {
        let file = root.join("steamapps").join("libraryfolders.vdf");
        if let Ok(text) = fs::read_to_string(file) {
            for line in text.lines() {
                let fields = quoted_fields(line);
                if fields.first().is_some_and(|field| field == "path")
                    && let Some(value) = fields.get(1)
                {
                    expanded.push(PathBuf::from(value.replace("\\\\", "\\")));
                }
            }
        }
    }
    libraries.append(&mut expanded);
    libraries.sort();
    libraries.dedup();

    let mut result = StoredLocations::default();
    for library in libraries {
        let steamapps = library.join("steamapps");
        let mut manifests = match fs::read_dir(&steamapps) {
            Ok(entries) => entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| {
                            name.starts_with("appmanifest_")
                                && Path::new(name)
                                    .extension()
                                    .is_some_and(|extension| extension.eq_ignore_ascii_case("acf"))
                        })
                })
                .collect::<Vec<_>>(),
            Err(_) => Vec::new(),
        };
        manifests.sort();
        for manifest in manifests {
            let Ok(text) = fs::read_to_string(manifest) else {
                continue;
            };
            let Some(install_dir) = vdf_value(&text, "installdir") else {
                continue;
            };
            let root = steamapps.join("common").join(install_dir);
            if result.generals.is_none() && validate_root(GameEdition::Generals, &root).is_ok() {
                result.generals = Some(root.clone());
            }
            if result.zero_hour.is_none() && validate_root(GameEdition::ZeroHour, &root).is_ok() {
                result.zero_hour = Some(root);
            }
        }
    }
    result
}

fn edition_archives(
    edition: GameEdition,
    kind: ResourceKind,
    root: &Path,
) -> Result<Vec<PathBuf>, ResourceError> {
    let names: Vec<&str> = match (edition, kind) {
        (_, ResourceKind::Manifest) => return all_big_files(root),
        (GameEdition::Generals, ResourceKind::Localization) => vec!["English.big", "Patch.big"],
        (GameEdition::ZeroHour, ResourceKind::Localization) => vec!["EnglishZH.big", "PatchZH.big"],
        (GameEdition::Generals, ResourceKind::W3d) => vec!["W3D.big", "Patch.big"],
        (GameEdition::ZeroHour, ResourceKind::W3d) => {
            vec!["W3DZH.big", "W3DEnglishZH.big", "PatchZH.big"]
        }
        (GameEdition::Generals, ResourceKind::W3dWithTextures) => {
            vec!["W3D.big", "Textures.big", "Patch.big"]
        }
        (GameEdition::ZeroHour, ResourceKind::W3dWithTextures) => vec![
            "W3DZH.big",
            "W3DEnglishZH.big",
            "TexturesZH.big",
            "PatchZH.big",
        ],
    };
    Ok(names
        .into_iter()
        .map(|name| root.join(name))
        .filter(|path| path.is_file())
        .collect())
}

fn all_big_files(root: &Path) -> Result<Vec<PathBuf>, ResourceError> {
    let entries = fs::read_dir(root).map_err(|error| ResourceError::Io {
        path: root.to_path_buf(),
        error,
    })?;
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("big"))
        })
        .collect::<Vec<_>>();
    paths.sort_by_key(|path| path.file_name().map(std::ffi::OsStr::to_ascii_lowercase));
    Ok(paths)
}

fn validate_root(edition: GameEdition, root: &Path) -> Result<(), ResourceError> {
    let sentinels = match edition {
        GameEdition::Generals => ["W3D.big", "Textures.big"],
        GameEdition::ZeroHour => ["W3DZH.big", "TexturesZH.big"],
    };
    if sentinels.iter().all(|name| root.join(name).is_file()) {
        Ok(())
    } else {
        Err(ResourceError::InvalidInstallation {
            edition,
            path: root.to_path_buf(),
        })
    }
}

/// Validates the edition-specific archive sentinels beneath an installation root.
///
/// # Errors
///
/// Returns an error when the selected installation does not contain its required archives.
pub fn validate_installation(edition: GameEdition, root: &Path) -> Result<(), ResourceError> {
    validate_root(edition, root)
}

fn steam_roots() -> Vec<PathBuf> {
    let mut roots = BTreeSet::new();
    if let Some(path) = env::var_os("CIC_STEAM_ROOT") {
        roots.insert(PathBuf::from(path));
    }
    #[cfg(windows)]
    {
        if let Ok(output) = Command::new("reg.exe")
            .args(["query", r"HKCU\Software\Valve\Steam", "/v", "SteamPath"])
            .output()
            && output.status.success()
        {
            let text = String::from_utf8_lossy(&output.stdout);
            if let Some(line) = text.lines().find(|line| line.contains("SteamPath"))
                && let Some(path) = line.split("REG_SZ").nth(1)
            {
                roots.insert(PathBuf::from(path.trim()));
            }
        }
        for variable in ["PROGRAMFILES(X86)", "PROGRAMFILES"] {
            if let Some(path) = env::var_os(variable) {
                roots.insert(PathBuf::from(path).join("Steam"));
            }
        }
    }
    roots.into_iter().collect()
}

fn quoted_fields(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut remaining = line;
    while let Some(start) = remaining.find('"') {
        remaining = &remaining[start + 1..];
        let Some(end) = remaining.find('"') else {
            break;
        };
        fields.push(remaining[..end].to_owned());
        remaining = &remaining[end + 1..];
    }
    fields
}

fn vdf_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let fields = quoted_fields(line);
        (fields
            .first()
            .is_some_and(|field| field.eq_ignore_ascii_case(key)))
        .then(|| fields.get(1).cloned())
        .flatten()
    })
}

fn nonempty_path(value: &str) -> Option<PathBuf> {
    (!value.is_empty()).then(|| PathBuf::from(value))
}

fn config_value(path: Option<&Path>) -> Result<String, ResourceError> {
    let Some(path) = path else {
        return Ok(String::new());
    };
    let value = path
        .to_str()
        .ok_or_else(|| ResourceError::NonUtf8Path(path.to_path_buf()))?;
    if value.contains(['\n', '\r']) {
        return Err(ResourceError::InvalidConfigPath(path.to_path_buf()));
    }
    Ok(value.to_owned())
}

/// Installed-resource configuration or discovery failure.
#[derive(Debug)]
pub enum ResourceError {
    ConfigDirectoryUnavailable,
    InvalidConfig { line: usize },
    InvalidConfigPath(PathBuf),
    NonUtf8Path(PathBuf),
    InstallationNotFound(GameEdition),
    InvalidInstallation { edition: GameEdition, path: PathBuf },
    Io { path: PathBuf, error: io::Error },
}

impl Display for ResourceError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConfigDirectoryUnavailable => {
                formatter.write_str("user configuration directory is unavailable")
            }
            Self::InvalidConfig { line } => write!(
                formatter,
                "resource configuration is invalid at line {line}"
            ),
            Self::InvalidConfigPath(path) => write!(
                formatter,
                "invalid resource configuration path: {}",
                path.display()
            ),
            Self::NonUtf8Path(path) => write!(formatter, "path is not UTF-8: {}", path.display()),
            Self::InstallationNotFound(edition) => write!(
                formatter,
                "{edition} installation not found; use --game-dir or cic-inspect config set"
            ),
            Self::InvalidInstallation { edition, path } => write!(
                formatter,
                "{} installation is missing required archives: {}",
                edition,
                path.display()
            ),
            Self::Io { path, error } => write!(formatter, "{}: {error}", path.display()),
        }
    }
}

impl Error for ResourceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { error, .. } => Some(error),
            _ => None,
        }
    }
}

impl Display for GameEdition {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Generals => "Generals",
            Self::ZeroHour => "Zero Hour",
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{GameEdition, ResourceKind, StoredLocations, edition_archives};

    fn test_root(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/resource-tests")
            .join(name)
    }

    #[test]
    fn stored_locations_round_trip_and_replace_existing_config() {
        let root = test_root("config");
        if root.exists() {
            fs::remove_dir_all(&root).expect("remove stale config test");
        }
        let path = root.join("config");
        let first = StoredLocations {
            generals: Some(PathBuf::from("generals")),
            zero_hour: None,
        };
        first.save(&path).expect("save initial config");
        assert_eq!(StoredLocations::load(&path).expect("load config"), first);
        let second = StoredLocations {
            generals: Some(PathBuf::from("new-generals")),
            zero_hour: Some(PathBuf::from("zero-hour")),
        };
        second.save(&path).expect("replace config");
        assert_eq!(StoredLocations::load(&path).expect("reload config"), second);
        fs::remove_dir_all(root).expect("remove config test");
    }

    #[test]
    fn archive_profiles_use_explicit_stable_order() {
        let root = test_root("profiles");
        if root.exists() {
            fs::remove_dir_all(&root).expect("remove stale profile test");
        }
        fs::create_dir_all(&root).expect("create profile test");
        for name in [
            "Patch.big",
            "Textures.big",
            "W3D.big",
            "PatchZH.big",
            "TexturesZH.big",
            "W3DEnglishZH.big",
            "W3DZH.big",
        ] {
            fs::write(root.join(name), []).expect("create archive sentinel");
        }
        let names = |edition| {
            edition_archives(edition, ResourceKind::W3dWithTextures, &root)
                .expect("resolve archive profile")
                .into_iter()
                .map(|path| path.file_name().expect("file name").to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(GameEdition::Generals),
            ["W3D.big", "Textures.big", "Patch.big"]
        );
        assert_eq!(
            names(GameEdition::ZeroHour),
            [
                "W3DZH.big",
                "W3DEnglishZH.big",
                "TexturesZH.big",
                "PatchZH.big"
            ]
        );
        fs::remove_dir_all(root).expect("remove profile test");
    }
}
