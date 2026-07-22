//! Deterministic resource paths, mounts, overlays, and provenance.

mod big;

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use big::{BigArchiveIndex, BigEntry, BigError, BigLimits, BigVersion, parse_big_archive};

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

/// Explicit bounds for indexing one loose-directory provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectoryLimits {
    /// Maximum number of regular files retained in one directory index.
    pub maximum_files: usize,
    /// Maximum number of nested directory components below the mount root.
    pub maximum_depth: usize,
    /// Maximum normalized UTF-8 byte length of one virtual path.
    pub maximum_virtual_path_bytes: usize,
}

impl Default for DirectoryLimits {
    fn default() -> Self {
        Self {
            maximum_files: 1_000_000,
            maximum_depth: 256,
            maximum_virtual_path_bytes: 4096,
        }
    }
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
    provider: Arc<Provider>,
    source: ResourceSource,
}

impl ResourceEntry {
    /// Returns the resource's provider metadata.
    #[must_use]
    pub fn provider(&self) -> &Provider {
        &self.provider
    }

    /// Returns the indexed resource length without reading its payload.
    #[must_use]
    pub fn len(&self) -> usize {
        self.source.len()
    }

    /// Returns whether the indexed resource is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Reads exactly one resource payload under a caller-selected allocation bound.
    ///
    /// Disk-backed directory and BIG entries are opened only when this method is called.
    ///
    /// # Errors
    ///
    /// Returns a structured error if the indexed size exceeds `maximum_bytes`, the backing file
    /// changed after indexing, or an exact bounded read fails.
    pub fn read(&self, maximum_bytes: usize) -> Result<Vec<u8>, ResourceReadError> {
        let size = self.len();
        if size > maximum_bytes {
            return Err(ResourceReadError::LimitExceeded {
                actual: size,
                maximum: maximum_bytes,
            });
        }
        match &self.source {
            ResourceSource::Memory(bytes) => Ok(bytes.to_vec()),
            ResourceSource::MemoryBig {
                archive,
                offset,
                end,
            } => archive
                .get(*offset..*end)
                .map(<[u8]>::to_vec)
                .ok_or(ResourceReadError::MemoryRangeInvalid),
            ResourceSource::LooseFile {
                path,
                indexed_file_size,
                ..
            } => read_file_range(path, *indexed_file_size, 0, size),
            ResourceSource::BigFile {
                path,
                indexed_file_size,
                offset,
                size,
            } => read_file_range(path, *indexed_file_size, *offset, *size),
        }
    }
}

#[derive(Debug, Clone)]
enum ResourceSource {
    Memory(Arc<[u8]>),
    MemoryBig {
        archive: Arc<[u8]>,
        offset: usize,
        end: usize,
    },
    LooseFile {
        path: PathBuf,
        indexed_file_size: u64,
        size: usize,
    },
    BigFile {
        path: Arc<Path>,
        indexed_file_size: u64,
        offset: usize,
        size: usize,
    },
}

impl ResourceSource {
    fn len(&self) -> usize {
        match self {
            Self::Memory(bytes) => bytes.len(),
            Self::MemoryBig { offset, end, .. } => end - offset,
            Self::LooseFile { size, .. } | Self::BigFile { size, .. } => *size,
        }
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
        self.mount_entries(
            name.into(),
            ProviderKind::Memory,
            entries
                .into_iter()
                .map(|(path, bytes)| (path, ResourceSource::Memory(Arc::from(bytes)))),
            DuplicatePolicy::Reject,
        )
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
        self.mount_directory_with_limits(name, root, DirectoryLimits::default())
    }

    /// Recursively indexes regular files beneath a directory with explicit metadata limits.
    ///
    /// # Errors
    ///
    /// Returns the same structured errors as [`Self::mount_directory`], including an error before
    /// exceeding the configured file-count, recursion-depth, or virtual-path-length limit.
    pub fn mount_directory_with_limits(
        &mut self,
        name: impl Into<String>,
        root: impl AsRef<Path>,
        limits: DirectoryLimits,
    ) -> Result<MountId, MountError> {
        let root = root.as_ref();
        let mut files = Vec::new();
        collect_directory(root, root, 0, limits, &mut files)?;
        self.mount_entries(
            name.into(),
            ProviderKind::LooseDirectory,
            files,
            DuplicatePolicy::Reject,
        )
    }

    /// Mounts members from an in-memory BIG archive.
    ///
    /// Duplicate normalized names are preserved in file-table order and the last entry
    /// wins, matching observed retail archive behavior.
    ///
    /// # Errors
    ///
    /// Returns [`MountError::Big`] when the archive index is invalid or exceeds limits,
    /// or [`MountError::MountIdExhausted`] when no stable mount identifier remains.
    pub fn mount_big_bytes(
        &mut self,
        name: impl Into<String>,
        bytes: &[u8],
        limits: BigLimits,
    ) -> Result<MountId, MountError> {
        let index = parse_big_archive(bytes, limits).map_err(MountError::Big)?;
        let archive: Arc<[u8]> = Arc::from(bytes);
        let mut entries = Vec::with_capacity(index.entries().len());
        for entry in index.entries() {
            let end = entry.offset().checked_add(entry.size()).ok_or_else(|| {
                MountError::Big(BigError::EntryRangeOverflow {
                    entry: entries.len(),
                    offset: entry.offset(),
                    size: entry.size(),
                })
            })?;
            entries.push((
                entry.path().clone(),
                ResourceSource::MemoryBig {
                    archive: archive.clone(),
                    offset: entry.offset(),
                    end,
                },
            ));
        }
        self.mount_entries(
            name.into(),
            ProviderKind::BigArchive,
            entries,
            DuplicatePolicy::Preserve,
        )
    }

    /// Reads, validates, and mounts a BIG archive from disk.
    ///
    /// # Errors
    ///
    /// Returns a structured [`MountError`] for metadata/read failures, archive-size limit
    /// excess, invalid BIG data, or exhausted mount identifiers.
    pub fn mount_big_file(
        &mut self,
        name: impl Into<String>,
        path: impl AsRef<Path>,
        limits: BigLimits,
    ) -> Result<MountId, MountError> {
        let path = path.as_ref();
        let index = index_big_file(path, limits)?;
        let archive_path: Arc<Path> = Arc::from(path);
        let indexed_file_size =
            u64::try_from(index.archive_size()).map_err(|_| MountError::FileTooLarge {
                path: path.to_path_buf(),
                size: u64::MAX,
            })?;
        let entries = index.entries().iter().map(|entry| {
            (
                entry.path().clone(),
                ResourceSource::BigFile {
                    path: archive_path.clone(),
                    indexed_file_size,
                    offset: entry.offset(),
                    size: entry.size(),
                },
            )
        });
        self.mount_entries(
            name.into(),
            ProviderKind::BigArchive,
            entries,
            DuplicatePolicy::Preserve,
        )
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
        duplicate_policy: DuplicatePolicy,
    ) -> Result<MountId, MountError>
    where
        I: IntoIterator<Item = (VirtualPath, ResourceSource)>,
    {
        let batch = entries.into_iter().collect::<Vec<_>>();
        if duplicate_policy == DuplicatePolicy::Reject {
            let mut seen = BTreeSet::new();
            for (path, _) in &batch {
                if !seen.insert(path.clone()) {
                    return Err(MountError::DuplicatePath(path.clone()));
                }
            }
        }

        let following = self
            .next_mount_id
            .checked_add(1)
            .ok_or(MountError::MountIdExhausted)?;
        let mount_id = MountId(self.next_mount_id);
        let provider = Arc::new(Provider {
            mount_id,
            name,
            kind,
        });

        for (path, source) in batch {
            self.entries.entry(path).or_default().push(ResourceEntry {
                provider: provider.clone(),
                source,
            });
        }
        self.next_mount_id = following;
        Ok(mount_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DuplicatePolicy {
    Reject,
    Preserve,
}

fn collect_directory(
    root: &Path,
    directory: &Path,
    depth: usize,
    limits: DirectoryLimits,
    output: &mut Vec<(VirtualPath, ResourceSource)>,
) -> Result<(), MountError> {
    if depth > limits.maximum_depth {
        return Err(MountError::DirectoryLimitExceeded {
            what: "directory recursion depth",
            actual: depth,
            maximum: limits.maximum_depth,
        });
    }
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
            let child_depth = depth
                .checked_add(1)
                .ok_or(MountError::DirectoryLimitExceeded {
                    what: "directory recursion depth",
                    actual: usize::MAX,
                    maximum: limits.maximum_depth,
                })?;
            collect_directory(root, &path, child_depth, limits, output)?;
        } else if file_type.is_file() {
            if output.len() >= limits.maximum_files {
                return Err(MountError::DirectoryLimitExceeded {
                    what: "directory file count",
                    actual: output.len() + 1,
                    maximum: limits.maximum_files,
                });
            }
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
            if virtual_text.len() > limits.maximum_virtual_path_bytes {
                return Err(MountError::DirectoryLimitExceeded {
                    what: "virtual path length",
                    actual: virtual_text.len(),
                    maximum: limits.maximum_virtual_path_bytes,
                });
            }
            let virtual_path = VirtualPath::new(&virtual_text).map_err(MountError::Path)?;
            let metadata = entry.metadata().map_err(|error| MountError::Io {
                path: path.clone(),
                error,
            })?;
            let size = usize::try_from(metadata.len()).map_err(|_| MountError::FileTooLarge {
                path: path.clone(),
                size: metadata.len(),
            })?;
            output.push((
                virtual_path,
                ResourceSource::LooseFile {
                    path,
                    indexed_file_size: metadata.len(),
                    size,
                },
            ));
        }
    }
    Ok(())
}

fn index_big_file(path: &Path, limits: BigLimits) -> Result<BigArchiveIndex, MountError> {
    let mut file = File::open(path).map_err(|error| MountError::Io {
        path: path.to_path_buf(),
        error,
    })?;
    let metadata = file.metadata().map_err(|error| MountError::Io {
        path: path.to_path_buf(),
        error,
    })?;
    let archive_size = usize::try_from(metadata.len()).map_err(|_| MountError::FileTooLarge {
        path: path.to_path_buf(),
        size: metadata.len(),
    })?;
    let mut header = Vec::with_capacity(16);
    file.by_ref()
        .take(16)
        .read_to_end(&mut header)
        .map_err(|error| MountError::Io {
            path: path.to_path_buf(),
            error,
        })?;
    let prefix_length =
        big::big_directory_prefix_length(&header, archive_size, limits).map_err(MountError::Big)?;
    let mut prefix = vec![0; prefix_length];
    file.seek(SeekFrom::Start(0))
        .and_then(|_| file.read_exact(&mut prefix))
        .map_err(|error| MountError::Io {
            path: path.to_path_buf(),
            error,
        })?;
    big::parse_big_archive_prefix(&prefix, archive_size, limits).map_err(MountError::Big)
}

fn read_file_range(
    path: &Path,
    indexed_file_size: u64,
    offset: usize,
    size: usize,
) -> Result<Vec<u8>, ResourceReadError> {
    let mut file = File::open(path).map_err(|error| ResourceReadError::Io {
        path: path.to_path_buf(),
        error,
    })?;
    let actual_size = file
        .metadata()
        .map_err(|error| ResourceReadError::Io {
            path: path.to_path_buf(),
            error,
        })?
        .len();
    if actual_size != indexed_file_size {
        return Err(ResourceReadError::BackingFileSizeChanged {
            path: path.to_path_buf(),
            indexed: indexed_file_size,
            actual: actual_size,
        });
    }
    let offset = u64::try_from(offset).map_err(|_| ResourceReadError::OffsetTooLarge { offset })?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|error| ResourceReadError::Io {
            path: path.to_path_buf(),
            error,
        })?;
    let mut bytes = vec![0; size];
    file.read_exact(&mut bytes)
        .map_err(|error| ResourceReadError::Io {
            path: path.to_path_buf(),
            error,
        })?;
    Ok(bytes)
}

/// A failure while lazily reading one indexed resource.
#[derive(Debug)]
pub enum ResourceReadError {
    /// The indexed payload exceeds the caller's explicit allocation bound.
    LimitExceeded { actual: usize, maximum: usize },
    /// A backing file could not be opened, inspected, sought, or read exactly.
    Io { path: PathBuf, error: io::Error },
    /// The physical file length changed after its provider was indexed.
    BackingFileSizeChanged {
        path: PathBuf,
        indexed: u64,
        actual: u64,
    },
    /// A host offset could not be represented by the filesystem seek API.
    OffsetTooLarge { offset: usize },
    /// An in-memory archive range violated an already validated index invariant.
    MemoryRangeInvalid,
}

impl Display for ResourceReadError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::LimitExceeded { actual, maximum } => write!(
                formatter,
                "indexed resource size {actual} exceeds read limit {maximum}"
            ),
            Self::Io { path, error } => write!(formatter, "{}: {error}", path.display()),
            Self::BackingFileSizeChanged {
                path,
                indexed,
                actual,
            } => write!(
                formatter,
                "backing file size changed after indexing: {} was {indexed} bytes and is now {actual} bytes",
                path.display()
            ),
            Self::OffsetTooLarge { offset } => {
                write!(
                    formatter,
                    "resource offset {offset} does not fit the seek API"
                )
            }
            Self::MemoryRangeInvalid => {
                formatter.write_str("in-memory BIG member range is invalid")
            }
        }
    }
}

impl Error for ResourceReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { error, .. } => Some(error),
            Self::LimitExceeded { .. }
            | Self::BackingFileSizeChanged { .. }
            | Self::OffsetTooLarge { .. }
            | Self::MemoryRangeInvalid => None,
        }
    }
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
    /// A BIG archive was malformed or exceeded an explicit limit.
    Big(BigError),
    /// A file is too large to index on this host.
    FileTooLarge { path: PathBuf, size: u64 },
    /// A loose-directory metadata index exceeded an explicit resource limit.
    DirectoryLimitExceeded {
        what: &'static str,
        actual: usize,
        maximum: usize,
    },
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
            Self::Big(error) => Display::fmt(error, formatter),
            Self::FileTooLarge { path, size } => write!(
                formatter,
                "file is too large to index on this host: {} has {size} bytes",
                path.display()
            ),
            Self::DirectoryLimitExceeded {
                what,
                actual,
                maximum,
            } => write!(formatter, "{what} value {actual} exceeds limit {maximum}"),
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
            Self::Big(error) => Some(error),
            Self::OutsideRoot { .. }
            | Self::FileTooLarge { .. }
            | Self::DirectoryLimitExceeded { .. }
            | Self::DuplicatePath(_)
            | Self::SymbolicLink(_)
            | Self::MountIdExhausted => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{MountError, Vfs, VirtualPath};

    fn path(value: &str) -> VirtualPath {
        VirtualPath::new(value).expect("valid test path")
    }

    fn test_root(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/vfs-tests")
            .join(name)
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
        assert_eq!(resource.read(8).expect("read override"), b"override");
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

    #[test]
    fn duplicate_big_names_preserve_history_and_last_entry_wins() {
        let archive = synthetic_big(&[(r"Data\A.txt", b"old"), ("data/a.TXT", b"new")]);
        let mut vfs = Vfs::new();
        vfs.mount_big_bytes("duplicate.big", &archive, super::BigLimits::default())
            .expect("BIG mount");

        let resource = vfs.resolve(&path("data/a.txt")).expect("resolved entry");
        assert_eq!(resource.read(8).expect("read winner"), b"new");
        let history = vfs.history(&path("data/a.txt")).expect("entry history");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].read(8).expect("read history"), b"old");
        assert_eq!(history[1].provider().mount_id().get(), 0);
    }

    #[test]
    fn disk_providers_index_without_retaining_payloads() {
        let root = test_root("lazy-providers");
        if root.exists() {
            fs::remove_dir_all(&root).expect("remove stale lazy-provider test");
        }
        let loose_root = root.join("loose");
        fs::create_dir_all(loose_root.join("data")).expect("create loose tree");
        let loose_path = loose_root.join("data/value.bin");
        fs::write(&loose_path, b"loose").expect("write loose fixture");
        let archive_path = root.join("custom.assets");
        fs::write(
            &archive_path,
            synthetic_big(&[("data/archive.bin", b"archive")]),
        )
        .expect("write BIG fixture");

        let mut vfs = Vfs::new();
        vfs.mount_directory("loose", &loose_root)
            .expect("index loose directory");
        vfs.mount_big_file("archive", &archive_path, super::BigLimits::default())
            .expect("index BIG file");
        assert_eq!(
            vfs.resolve(&path("data/value.bin"))
                .expect("loose entry")
                .len(),
            5
        );
        assert_eq!(
            vfs.resolve(&path("data/archive.bin"))
                .expect("archive entry")
                .len(),
            7
        );

        fs::remove_file(loose_path).expect("remove indexed loose payload");
        fs::remove_file(archive_path).expect("remove indexed archive payload");
        assert!(
            vfs.resolve(&path("data/value.bin"))
                .expect("indexed loose entry")
                .read(16)
                .is_err()
        );
        assert!(
            vfs.resolve(&path("data/archive.bin"))
                .expect("indexed archive entry")
                .read(16)
                .is_err()
        );
        fs::remove_dir_all(root).expect("remove lazy-provider test");
    }

    #[test]
    fn lazy_reads_enforce_the_callers_allocation_limit() {
        let mut vfs = Vfs::new();
        vfs.mount_memory("memory", [(path("data/value.bin"), vec![0; 17])])
            .expect("memory mount");
        let error = vfs
            .resolve(&path("data/value.bin"))
            .expect("memory entry")
            .read(16)
            .expect_err("limit must reject read");
        assert!(matches!(
            error,
            super::ResourceReadError::LimitExceeded {
                actual: 17,
                maximum: 16
            }
        ));
    }

    #[test]
    fn directory_indices_enforce_metadata_limits() {
        let root = test_root("directory-limits");
        if root.exists() {
            fs::remove_dir_all(&root).expect("remove stale directory-limit test");
        }
        fs::create_dir_all(&root).expect("create directory-limit test");
        fs::write(root.join("a.bin"), []).expect("write first indexed file");
        fs::write(root.join("b.bin"), []).expect("write second indexed file");
        let limits = super::DirectoryLimits {
            maximum_files: 1,
            ..super::DirectoryLimits::default()
        };
        let mut vfs = Vfs::new();
        assert!(matches!(
            vfs.mount_directory_with_limits("limited", &root, limits),
            Err(MountError::DirectoryLimitExceeded {
                what: "directory file count",
                actual: 2,
                maximum: 1
            })
        ));
        assert!(vfs.iter_resolved().next().is_none());
        fs::remove_dir_all(root).expect("remove directory-limit test");
    }

    fn synthetic_big(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let table_size = entries
            .iter()
            .map(|(name, _)| 8 + name.len() + 1)
            .sum::<usize>();
        let data_start = 16 + table_size;
        let archive_size = data_start + entries.iter().map(|(_, bytes)| bytes.len()).sum::<usize>();
        let mut archive = Vec::with_capacity(archive_size);
        archive.extend_from_slice(b"BIGF");
        archive.extend_from_slice(
            &u32::try_from(archive_size)
                .expect("synthetic archive size fits u32")
                .to_le_bytes(),
        );
        archive.extend_from_slice(
            &u32::try_from(entries.len())
                .expect("synthetic entry count fits u32")
                .to_be_bytes(),
        );
        archive.extend_from_slice(
            &u32::try_from(data_start)
                .expect("synthetic data offset fits u32")
                .to_be_bytes(),
        );

        let mut offset = data_start;
        for (name, bytes) in entries {
            archive.extend_from_slice(
                &u32::try_from(offset)
                    .expect("synthetic member offset fits u32")
                    .to_be_bytes(),
            );
            archive.extend_from_slice(
                &u32::try_from(bytes.len())
                    .expect("synthetic member size fits u32")
                    .to_be_bytes(),
            );
            archive.extend_from_slice(name.as_bytes());
            archive.push(0);
            offset += bytes.len();
        }
        for (_, bytes) in entries {
            archive.extend_from_slice(bytes);
        }
        archive
    }
}
