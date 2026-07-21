//! Deterministic resource paths, mounts, overlays, and provenance.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A canonical, platform-independent resource path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VirtualPath(String);

impl VirtualPath {
    /// Normalizes separators, removes empty and `.` components, folds ASCII case, and
    /// rejects parent traversal.
    ///
    /// # Errors
    ///
    /// Returns [`PathError::Empty`] when no resource name remains or
    /// [`PathError::ParentTraversal`] when any component is `..`.
    pub fn new(raw: &str) -> Result<Self, PathError> {
        let mut components = Vec::new();
        for component in raw.split(['/', '\\']) {
            match component {
                "" | "." => {}
                ".." => return Err(PathError::ParentTraversal(raw.to_owned())),
                value => {
                    let mut folded = value.to_owned();
                    folded.make_ascii_lowercase();
                    components.push(folded);
                }
            }
        }

        if components.is_empty() {
            return Err(PathError::Empty);
        }
        Ok(Self(components.join("/")))
    }

    /// Returns the normalized path text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for VirtualPath {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A failure to create a safe virtual resource path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathError {
    /// No resource-name component remained after normalization.
    Empty,
    /// Parent traversal is forbidden in virtual resource paths.
    ParentTraversal(String),
}

impl Display for PathError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("virtual path is empty"),
            Self::ParentTraversal(path) => {
                write!(formatter, "virtual path contains parent traversal: {path}")
            }
        }
    }
}

impl Error for PathError {}

/// Stable identifier assigned according to explicit mount order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MountId(u64);

impl MountId {
    /// Returns the zero-based mount sequence number.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Type of storage that supplied an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    /// A file discovered beneath a mounted directory.
    LooseDirectory,
    /// Bytes supplied directly, primarily for tests and adapters.
    Memory,
    /// An entry read from a BIG archive.
    BigArchive,
}

impl Display for ProviderKind {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::LooseDirectory => "directory",
            Self::Memory => "memory",
            Self::BigArchive => "big",
        })
    }
}

/// Provenance for one concrete resource entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provider {
    mount_id: MountId,
    name: String,
    kind: ProviderKind,
}

impl Provider {
    /// Returns the provider's explicit mount sequence number.
    #[must_use]
    pub const fn mount_id(&self) -> MountId {
        self.mount_id
    }

    /// Returns the stable diagnostic provider name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the provider storage kind.
    #[must_use]
    pub const fn kind(&self) -> ProviderKind {
        self.kind
    }
}

/// One version of a virtual resource.
#[derive(Debug, Clone)]
pub struct ResourceEntry {
    provider: Provider,
    bytes: Arc<[u8]>,
}

impl ResourceEntry {
    /// Returns the resource's provider metadata.
    #[must_use]
    pub const fn provider(&self) -> &Provider {
        &self.provider
    }

    /// Returns the immutable resource bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// A deterministic, last-mounted-wins virtual filesystem.
#[derive(Debug, Default)]
pub struct Vfs {
    next_mount_id: u64,
    entries: BTreeMap<VirtualPath, Vec<ResourceEntry>>,
}

impl Vfs {
    /// Creates an empty virtual filesystem.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            next_mount_id: 0,
            entries: BTreeMap::new(),
        }
    }

    /// Mounts an in-memory set of entries as one atomic provider.
    ///
    /// # Errors
    ///
    /// Returns [`MountError::DuplicatePath`] if two entries normalize to the same path
    /// within this mount, or [`MountError::MountIdExhausted`] after `u64::MAX` mounts.
    pub fn mount_memory<I>(
        &mut self,
        name: impl Into<String>,
        entries: I,
    ) -> Result<MountId, MountError>
    where
        I: IntoIterator<Item = (VirtualPath, Vec<u8>)>,
    {
        self.mount_entries(name.into(), ProviderKind::Memory, entries)
    }

    /// Recursively mounts regular files beneath a directory.
    ///
    /// Directory traversal order does not affect the result. Symbolic links are rejected
    /// to keep the physical input boundary explicit.
    ///
    /// # Errors
    ///
    /// Returns a structured [`MountError`] for I/O, invalid virtual paths, duplicate
    /// normalized paths, symbolic links, or exhausted mount identifiers.
    pub fn mount_directory(
        &mut self,
        name: impl Into<String>,
        root: impl AsRef<Path>,
    ) -> Result<MountId, MountError> {
        let root = root.as_ref();
        let mut files = Vec::new();
        collect_directory(root, root, &mut files)?;
        self.mount_entries(name.into(), ProviderKind::LooseDirectory, files)
    }

    /// Resolves the winning entry for a normalized path.
    #[must_use]
    pub fn resolve(&self, path: &VirtualPath) -> Option<&ResourceEntry> {
        self.entries.get(path).and_then(|history| history.last())
    }

    /// Returns every provider version from earliest to latest mount.
    #[must_use]
    pub fn history(&self, path: &VirtualPath) -> Option<&[ResourceEntry]> {
        self.entries.get(path).map(Vec::as_slice)
    }

    /// Iterates winning entries in normalized path order.
    pub fn iter_resolved(&self) -> impl Iterator<Item = (&VirtualPath, &ResourceEntry)> {
        self.entries
            .iter()
            .filter_map(|(path, history)| history.last().map(|entry| (path, entry)))
    }

    fn mount_entries<I>(
        &mut self,
        name: String,
        kind: ProviderKind,
        entries: I,
    ) -> Result<MountId, MountError>
    where
        I: IntoIterator<Item = (VirtualPath, Vec<u8>)>,
    {
        let mut batch = BTreeMap::new();
        for (path, bytes) in entries {
            if batch.insert(path.clone(), bytes).is_some() {
                return Err(MountError::DuplicatePath(path));
            }
        }

        let following = self
            .next_mount_id
            .checked_add(1)
            .ok_or(MountError::MountIdExhausted)?;
        let mount_id = MountId(self.next_mount_id);
        let provider = Provider {
            mount_id,
            name,
            kind,
        };

        for (path, bytes) in batch {
            self.entries.entry(path).or_default().push(ResourceEntry {
                provider: provider.clone(),
                bytes: Arc::from(bytes),
            });
        }
        self.next_mount_id = following;
        Ok(mount_id)
    }
}

fn collect_directory(
    root: &Path,
    directory: &Path,
    output: &mut Vec<(VirtualPath, Vec<u8>)>,
) -> Result<(), MountError> {
    for entry in fs::read_dir(directory).map_err(|error| MountError::Io {
        path: directory.to_path_buf(),
        error,
    })? {
        let entry = entry.map_err(|error| MountError::Io {
            path: directory.to_path_buf(),
            error,
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| MountError::Io {
            path: path.clone(),
            error,
        })?;

        if file_type.is_symlink() {
            return Err(MountError::SymbolicLink(path));
        }
        if file_type.is_dir() {
            collect_directory(root, &path, output)?;
        } else if file_type.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| MountError::OutsideRoot {
                    root: root.to_path_buf(),
                    path: path.clone(),
                })?;
            let virtual_text = relative
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            let virtual_path = VirtualPath::new(&virtual_text).map_err(MountError::Path)?;
            let bytes = fs::read(&path).map_err(|error| MountError::Io {
                path: path.clone(),
                error,
            })?;
            output.push((virtual_path, bytes));
        }
    }
    Ok(())
}

/// A failure while constructing a VFS mount.
#[derive(Debug)]
pub enum MountError {
    /// A physical filesystem operation failed.
    Io {
        /// Path associated with the failed operation.
        path: PathBuf,
        /// Original I/O failure.
        error: io::Error,
    },
    /// A physical entry unexpectedly escaped the mount root.
    OutsideRoot {
        /// Requested mount root.
        root: PathBuf,
        /// Escaping physical path.
        path: PathBuf,
    },
    /// A virtual resource path was invalid.
    Path(PathError),
    /// Two entries in one provider normalized to the same path.
    DuplicatePath(VirtualPath),
    /// Symbolic links are outside the loose-directory mount contract.
    SymbolicLink(PathBuf),
    /// No additional stable mount identifier can be assigned.
    MountIdExhausted,
}

impl Display for MountError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, error } => write!(formatter, "{}: {error}", path.display()),
            Self::OutsideRoot { root, path } => write!(
                formatter,
                "{} is outside mount root {}",
                path.display(),
                root.display()
            ),
            Self::Path(error) => Display::fmt(error, formatter),
            Self::DuplicatePath(path) => {
                write!(formatter, "provider contains duplicate virtual path {path}")
            }
            Self::SymbolicLink(path) => write!(
                formatter,
                "symbolic links are not supported in directory mounts: {}",
                path.display()
            ),
            Self::MountIdExhausted => formatter.write_str("VFS mount identifier exhausted"),
        }
    }
}

impl Error for MountError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { error, .. } => Some(error),
            Self::Path(error) => Some(error),
            Self::OutsideRoot { .. }
            | Self::DuplicatePath(_)
            | Self::SymbolicLink(_)
            | Self::MountIdExhausted => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MountError, Vfs, VirtualPath};

    fn path(value: &str) -> VirtualPath {
        VirtualPath::new(value).expect("valid test path")
    }

    #[test]
    fn normalizes_resource_paths() {
        assert_eq!(
            VirtualPath::new(r"Data\\English//./GENERALS.CSF")
                .expect("valid path")
                .as_str(),
            "data/english/generals.csf"
        );
        assert!(VirtualPath::new("../secret").is_err());
        assert!(VirtualPath::new("./").is_err());
    }

    #[test]
    fn later_mount_wins_and_history_is_preserved() {
        let mut vfs = Vfs::new();
        vfs.mount_memory("base", [(path("Data/A.txt"), b"base".to_vec())])
            .expect("base mount");
        vfs.mount_memory("mod", [(path("data/a.TXT"), b"override".to_vec())])
            .expect("mod mount");

        let resource = vfs.resolve(&path("DATA/a.txt")).expect("resolved entry");
        assert_eq!(resource.bytes(), b"override");
        assert_eq!(resource.provider().name(), "mod");

        let history = vfs.history(&path("data/a.txt")).expect("entry history");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].provider().name(), "base");
        assert_eq!(history[1].provider().mount_id().get(), 1);
    }

    #[test]
    fn resolved_iteration_is_sorted() {
        let mut vfs = Vfs::new();
        vfs.mount_memory(
            "base",
            [
                (path("z/file"), vec![1]),
                (path("a/file"), vec![2]),
                (path("m/file"), vec![3]),
            ],
        )
        .expect("mount");

        let paths = vfs
            .iter_resolved()
            .map(|(resource_path, _)| resource_path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(paths, ["a/file", "m/file", "z/file"]);
    }

    #[test]
    fn duplicate_path_rejects_the_entire_mount() {
        let mut vfs = Vfs::new();
        let result = vfs.mount_memory(
            "broken",
            [(path("Data/A.txt"), vec![1]), (path("data/a.TXT"), vec![2])],
        );

        assert!(matches!(result, Err(MountError::DuplicatePath(_))));
        assert!(vfs.resolve(&path("data/a.txt")).is_none());
    }
}
