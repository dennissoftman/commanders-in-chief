//! Installed-game discovery and explicit resource-edition mount policies.

use std::collections::BTreeSet;
use std::env;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
#[cfg(windows)]
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
    /// Terrain MAP archives.
    Map,
    /// Terrain MAP, terrain texture-sheet, and Terrain INI archives.
    Terrain,
    /// W3D model archives without texture images.
    W3d,
    /// W3D model and texture archives.
    W3dWithTextures,
}

/// Explicit bounds for a declarative mount profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MountProfileLimits {
    pub maximum_file_bytes: usize,
    pub maximum_mounts: usize,
    pub maximum_path_bytes: usize,
}

impl Default for MountProfileLimits {
    fn default() -> Self {
        Self {
            maximum_file_bytes: 1024 * 1024,
            maximum_mounts: 4096,
            maximum_path_bytes: 4096,
        }
    }
}

/// One ordered directory or archive provider declared by a custom profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileMount {
    path: PathBuf,
    optional: bool,
}

impl ProfileMount {
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub const fn optional(&self) -> bool {
        self.optional
    }
}

/// A bounded, ordered custom base or total-conversion mount profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountProfile {
    mounts: Vec<ProfileMount>,
}

impl MountProfile {
    /// Loads `version=1` followed by ordered `mount=<path>` or `optional=<path>` records.
    /// Relative paths are resolved against the profile's parent directory.
    ///
    /// # Errors
    ///
    /// Returns a structured error for I/O, invalid UTF-8 or records, unsupported versions, empty
    /// profiles, and all configured byte/count/path limits.
    pub fn load(path: &Path, limits: MountProfileLimits) -> Result<Self, MountProfileError> {
        let mut file = fs::File::open(path).map_err(|error| MountProfileError::Io {
            path: path.to_path_buf(),
            error,
        })?;
        let metadata = file.metadata().map_err(|error| MountProfileError::Io {
            path: path.to_path_buf(),
            error,
        })?;
        let actual = usize::try_from(metadata.len()).unwrap_or(usize::MAX);
        if actual > limits.maximum_file_bytes {
            return Err(MountProfileError::FileTooLarge {
                actual,
                maximum: limits.maximum_file_bytes,
            });
        }
        let read_limit = u64::try_from(limits.maximum_file_bytes)
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        let mut bytes = Vec::new();
        file.by_ref()
            .take(read_limit)
            .read_to_end(&mut bytes)
            .map_err(|error| MountProfileError::Io {
                path: path.to_path_buf(),
                error,
            })?;
        if bytes.len() > limits.maximum_file_bytes {
            return Err(MountProfileError::FileTooLarge {
                actual: bytes.len(),
                maximum: limits.maximum_file_bytes,
            });
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| MountProfileError::InvalidUtf8)?;
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let mut version = None;
        let mut mounts = Vec::new();
        for (line_index, raw_line) in text.lines().enumerate() {
            let line_number = line_index + 1;
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .ok_or(MountProfileError::InvalidRecord { line: line_number })?;
            let key = key.trim();
            let value = value.trim();
            if value.is_empty() {
                return Err(MountProfileError::InvalidRecord { line: line_number });
            }
            if key == "version" {
                if version.replace(value.to_owned()).is_some() {
                    return Err(MountProfileError::DuplicateVersion { line: line_number });
                }
                if value != "1" {
                    return Err(MountProfileError::UnsupportedVersion(value.to_owned()));
                }
                continue;
            }
            let optional = match key {
                "mount" => false,
                "optional" => true,
                _ => return Err(MountProfileError::UnknownKey { line: line_number }),
            };
            if version.as_deref() != Some("1") {
                return Err(MountProfileError::VersionMustComeFirst { line: line_number });
            }
            if value.len() > limits.maximum_path_bytes {
                return Err(MountProfileError::PathTooLong {
                    line: line_number,
                    actual: value.len(),
                    maximum: limits.maximum_path_bytes,
                });
            }
            if mounts.len() >= limits.maximum_mounts {
                return Err(MountProfileError::TooManyMounts {
                    actual: mounts.len() + 1,
                    maximum: limits.maximum_mounts,
                });
            }
            let declared = PathBuf::from(value);
            let resolved = if declared.is_absolute() {
                declared
            } else {
                parent.join(declared)
            };
            mounts.push(ProfileMount {
                path: resolved,
                optional,
            });
        }
        if version.is_none() {
            return Err(MountProfileError::MissingVersion);
        }
        if mounts.is_empty() {
            return Err(MountProfileError::Empty);
        }
        Ok(Self { mounts })
    }

    #[must_use]
    pub fn mounts(&self) -> &[ProfileMount] {
        &self.mounts
    }
}

/// Failure while reading a bounded custom mount profile.
#[derive(Debug)]
pub enum MountProfileError {
    Io {
        path: PathBuf,
        error: io::Error,
    },
    FileTooLarge {
        actual: usize,
        maximum: usize,
    },
    InvalidUtf8,
    InvalidRecord {
        line: usize,
    },
    DuplicateVersion {
        line: usize,
    },
    UnknownKey {
        line: usize,
    },
    VersionMustComeFirst {
        line: usize,
    },
    PathTooLong {
        line: usize,
        actual: usize,
        maximum: usize,
    },
    TooManyMounts {
        actual: usize,
        maximum: usize,
    },
    MissingVersion,
    UnsupportedVersion(String),
    Empty,
}

impl Display for MountProfileError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, error } => write!(formatter, "{}: {error}", path.display()),
            Self::FileTooLarge { actual, maximum } => write!(
                formatter,
                "mount profile size {actual} exceeds limit {maximum}"
            ),
            Self::InvalidUtf8 => formatter.write_str("mount profile is not valid UTF-8"),
            Self::InvalidRecord { line } => {
                write!(formatter, "invalid mount profile record at line {line}")
            }
            Self::DuplicateVersion { line } => {
                write!(formatter, "duplicate mount profile version at line {line}")
            }
            Self::UnknownKey { line } => {
                write!(formatter, "unknown mount profile key at line {line}")
            }
            Self::VersionMustComeFirst { line } => write!(
                formatter,
                "mount profile version must precede mounts at line {line}"
            ),
            Self::PathTooLong {
                line,
                actual,
                maximum,
            } => write!(
                formatter,
                "mount profile path at line {line} has {actual} bytes; limit is {maximum}"
            ),
            Self::TooManyMounts { actual, maximum } => write!(
                formatter,
                "mount profile contains {actual} mounts; limit is {maximum}"
            ),
            Self::MissingVersion => formatter.write_str("mount profile has no version record"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported mount profile version {version:?}")
            }
            Self::Empty => formatter.write_str("mount profile contains no providers"),
        }
    }
}

impl Error for MountProfileError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { error, .. } => Some(error),
            Self::FileTooLarge { .. }
            | Self::InvalidUtf8
            | Self::InvalidRecord { .. }
            | Self::DuplicateVersion { .. }
            | Self::UnknownKey { .. }
            | Self::VersionMustComeFirst { .. }
            | Self::PathTooLong { .. }
            | Self::TooManyMounts { .. }
            | Self::MissingVersion
            | Self::UnsupportedVersion(_)
            | Self::Empty => None,
        }
    }
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
/// Zero Hour is a delta profile: its list always contains the required Generals archives first
/// and the Zero Hour archives second. Consumers of cumulative definition files must parse
/// [`cic_vfs::Vfs::history`] earliest-to-latest rather than reading only the winning entry.
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
        (GameEdition::Generals, ResourceKind::Map) => vec!["Maps.big", "Patch.big"],
        (GameEdition::ZeroHour, ResourceKind::Map) => vec!["MapsZH.big", "PatchZH.big"],
        (GameEdition::Generals, ResourceKind::Terrain) => {
            vec![
                "Maps.big",
                "Terrain.big",
                "Textures.big",
                "INI.big",
                "Patch.big",
            ]
        }
        (GameEdition::ZeroHour, ResourceKind::Terrain) => {
            vec![
                "MapsZH.big",
                "TerrainZH.big",
                "TexturesZH.big",
                "INIZH.big",
                "PatchZH.big",
            ]
        }
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
    let mut paths = Vec::with_capacity(names.len());
    for name in names {
        if let Some(path) = resolve_named_file(root, name)? {
            paths.push(path);
        }
    }
    Ok(paths)
}

fn all_big_files(root: &Path) -> Result<Vec<PathBuf>, ResourceError> {
    let entries = fs::read_dir(root).map_err(|error| ResourceError::Io {
        path: root.to_path_buf(),
        error,
    })?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| ResourceError::Io {
            path: root.to_path_buf(),
            error,
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| ResourceError::Io {
            path: path.clone(),
            error,
        })?;
        if file_type.is_file()
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("big"))
        {
            paths.push(path);
        }
    }
    paths.sort_by(|left, right| {
        let left_name = left.file_name().unwrap_or_default();
        let right_name = right.file_name().unwrap_or_default();
        left_name
            .to_ascii_lowercase()
            .cmp(&right_name.to_ascii_lowercase())
            .then_with(|| left_name.cmp(right_name))
    });
    Ok(paths)
}

fn validate_root(edition: GameEdition, root: &Path) -> Result<(), ResourceError> {
    let sentinels = match edition {
        GameEdition::Generals => ["W3D.big", "Textures.big"],
        GameEdition::ZeroHour => ["W3DZH.big", "TexturesZH.big"],
    };
    let mut complete = true;
    for name in sentinels {
        complete &= resolve_named_file(root, name)?.is_some();
    }
    if complete {
        Ok(())
    } else {
        Err(ResourceError::InvalidInstallation {
            edition,
            path: root.to_path_buf(),
        })
    }
}

fn resolve_named_file(root: &Path, expected: &str) -> Result<Option<PathBuf>, ResourceError> {
    let entries = fs::read_dir(root).map_err(|error| ResourceError::Io {
        path: root.to_path_buf(),
        error,
    })?;
    let mut matches = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| ResourceError::Io {
            path: root.to_path_buf(),
            error,
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| ResourceError::Io {
            path: path.clone(),
            error,
        })?;
        if file_type.is_file()
            && entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.eq_ignore_ascii_case(expected))
        {
            matches.push(path);
        }
    }
    matches.sort_by(|left, right| left.file_name().cmp(&right.file_name()));
    match matches.len() {
        0 => Ok(None),
        1 => Ok(matches.pop()),
        _ => Err(ResourceError::AmbiguousArchiveName {
            root: root.to_path_buf(),
            expected: expected.to_owned(),
            matches,
        }),
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
    InvalidConfig {
        line: usize,
    },
    InvalidConfigPath(PathBuf),
    NonUtf8Path(PathBuf),
    InstallationNotFound(GameEdition),
    InvalidInstallation {
        edition: GameEdition,
        path: PathBuf,
    },
    AmbiguousArchiveName {
        root: PathBuf,
        expected: String,
        matches: Vec<PathBuf>,
    },
    Io {
        path: PathBuf,
        error: io::Error,
    },
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
            Self::AmbiguousArchiveName {
                root,
                expected,
                matches,
            } => write!(
                formatter,
                "installation {} contains {} files matching {expected:?} by ASCII case",
                root.display(),
                matches.len()
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

    #[cfg(target_os = "linux")]
    use super::ResourceError;
    use super::{
        GameEdition, MountProfile, MountProfileError, MountProfileLimits, ResourceKind,
        StoredLocations, edition_archives,
    };

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
    fn mount_profiles_resolve_ordered_relative_and_optional_providers() {
        let root = test_root("mount-profile");
        if root.exists() {
            fs::remove_dir_all(&root).expect("remove stale mount-profile test");
        }
        fs::create_dir_all(&root).expect("create mount-profile test");
        let path = root.join("total-conversion.cic-profile");
        fs::write(
            &path,
            "# ordered custom resources\nversion=1\nmount=base.assets\noptional=loose-overrides\n",
        )
        .expect("write mount profile");

        let profile =
            MountProfile::load(&path, MountProfileLimits::default()).expect("load mount profile");
        assert_eq!(profile.mounts().len(), 2);
        assert_eq!(profile.mounts()[0].path(), root.join("base.assets"));
        assert!(!profile.mounts()[0].optional());
        assert_eq!(profile.mounts()[1].path(), root.join("loose-overrides"));
        assert!(profile.mounts()[1].optional());

        let limits = MountProfileLimits {
            maximum_mounts: 1,
            ..MountProfileLimits::default()
        };
        assert!(matches!(
            MountProfile::load(&path, limits),
            Err(MountProfileError::TooManyMounts { .. })
        ));
        fs::remove_dir_all(root).expect("remove mount-profile test");
    }

    #[test]
    fn mount_profiles_reject_unversioned_and_unknown_records() {
        let root = test_root("invalid-mount-profiles");
        if root.exists() {
            fs::remove_dir_all(&root).expect("remove stale invalid-profile test");
        }
        fs::create_dir_all(&root).expect("create invalid-profile test");
        let path = root.join("invalid.cic-profile");
        fs::write(&path, "mount=base.big\n").expect("write unversioned profile");
        assert!(matches!(
            MountProfile::load(&path, MountProfileLimits::default()),
            Err(MountProfileError::VersionMustComeFirst { line: 1 })
        ));
        fs::write(&path, "version=1\nsurprise=value\n").expect("write unknown profile");
        assert!(matches!(
            MountProfile::load(&path, MountProfileLimits::default()),
            Err(MountProfileError::UnknownKey { line: 2 })
        ));
        fs::remove_dir_all(root).expect("remove invalid-profile test");
    }

    #[test]
    fn archive_profiles_use_explicit_stable_order() {
        let root = test_root("profiles");
        if root.exists() {
            fs::remove_dir_all(&root).expect("remove stale profile test");
        }
        fs::create_dir_all(&root).expect("create profile test");
        for name in [
            "Maps.big",
            "Patch.big",
            "Textures.big",
            "Terrain.big",
            "INI.big",
            "W3D.big",
            "MapsZH.big",
            "PatchZH.big",
            "TexturesZH.big",
            "TerrainZH.big",
            "INIZH.big",
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
        assert_eq!(
            edition_archives(GameEdition::Generals, ResourceKind::Terrain, &root)
                .expect("terrain archive profile")
                .into_iter()
                .map(|path| path.file_name().expect("file name").to_owned())
                .collect::<Vec<_>>(),
            [
                "Maps.big",
                "Terrain.big",
                "Textures.big",
                "INI.big",
                "Patch.big"
            ]
        );
        assert_eq!(
            edition_archives(GameEdition::ZeroHour, ResourceKind::Terrain, &root)
                .expect("Zero Hour terrain archive profile")
                .into_iter()
                .map(|path| path.file_name().expect("file name").to_owned())
                .collect::<Vec<_>>(),
            [
                "MapsZH.big",
                "TerrainZH.big",
                "TexturesZH.big",
                "INIZH.big",
                "PatchZH.big"
            ]
        );
        fs::remove_dir_all(root).expect("remove profile test");
    }

    #[test]
    fn built_in_profiles_resolve_actual_archive_casing() {
        let root = test_root("archive-case");
        if root.exists() {
            fs::remove_dir_all(&root).expect("remove stale archive-case test");
        }
        fs::create_dir_all(&root).expect("create archive-case test");
        for name in ["w3d.BIG", "textures.BIG", "patch.BIG"] {
            fs::write(root.join(name), []).expect("create mixed-case archive");
        }
        let names = edition_archives(GameEdition::Generals, ResourceKind::W3dWithTextures, &root)
            .expect("resolve mixed-case archives")
            .into_iter()
            .map(|path| path.file_name().expect("file name").to_owned())
            .collect::<Vec<_>>();
        assert_eq!(names, ["w3d.BIG", "textures.BIG", "patch.BIG"]);
        fs::remove_dir_all(root).expect("remove archive-case test");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn built_in_profiles_reject_ambiguous_archive_casing() {
        let root = test_root("ambiguous-archive-case");
        if root.exists() {
            fs::remove_dir_all(&root).expect("remove stale ambiguous-case test");
        }
        fs::create_dir_all(&root).expect("create ambiguous-case test");
        for name in ["W3D.big", "w3d.BIG", "Textures.big"] {
            fs::write(root.join(name), []).expect("create archive candidate");
        }
        assert!(matches!(
            edition_archives(GameEdition::Generals, ResourceKind::W3dWithTextures, &root),
            Err(ResourceError::AmbiguousArchiveName { .. })
        ));
        fs::remove_dir_all(root).expect("remove ambiguous-case test");
    }
}
