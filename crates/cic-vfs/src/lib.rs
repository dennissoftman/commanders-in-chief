//! Deterministic resource paths, mounts, and overlays.
//!
//! Resolution is last-mounted-wins, and mount order is the caller's explicit decision. That single
//! rule is what makes mod loading predictable: mounting base content, then an expansion, then a
//! user's mod produces exactly the override order the list was written in, with no dependency on
//! directory traversal order or filesystem case behaviour.
//!
//! Every path is normalized through [`VirtualPath`], which folds case and refuses parent traversal.
//! An archive member named `../../etc/passwd` therefore cannot be mounted at all, rather than being
//! caught later by whoever happens to write it out.
//!
//! Archive containers are pluggable. [`parse_zip_archive`] and [`parse_tar_archive`] both produce an
//! [`ArchiveIndex`], and the mount methods here treat them identically -- so adding a container
//! means writing one reader, not touching this module.

mod archive;
mod tar;
#[cfg(test)]
mod testing;
mod zip;

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use archive::{ArchiveEntry, ArchiveError, ArchiveIndex, ArchiveLimits, Compression};
pub use tar::{parse_tar_archive, parse_tar_gz_archive};
pub use zip::parse_zip_archive;

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

    /// Returns the lowercase extension without its dot, if the final component has one.
    #[must_use]
    pub fn extension(&self) -> Option<&str> {
        let name = self.0.rsplit('/').next()?;
        let (stem, extension) = name.rsplit_once('.')?;
        if stem.is_empty() {
            return None;
        }
        Some(extension)
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
    /// A member of a zip container.
    Zip,
    /// A member of a tar container.
    Tar,
}

impl Display for ProviderKind {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::LooseDirectory => "directory",
            Self::Memory => "memory",
            Self::Zip => "zip",
            Self::Tar => "tar",
        })
    }
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

    /// Returns the resource length after decompression, without reading its payload.
    #[must_use]
    pub fn len(&self) -> usize {
        self.source.len()
    }

    /// Returns whether the indexed resource is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns how this entry's payload is stored.
    #[must_use]
    pub const fn compression(&self) -> Compression {
        self.source.compression()
    }

    /// Reads exactly one resource payload under a caller-selected allocation bound.
    ///
    /// Disk-backed entries are opened only when this method is called, and compressed members are
    /// inflated only here — so indexing a large archive stays cheap and a caller that never reads a
    /// member never pays for it.
    ///
    /// The bound is checked against the *decompressed* size before any inflation runs, so a member
    /// that lied about its size at index time cannot expand past what the caller allowed.
    ///
    /// # Errors
    ///
    /// Returns a structured error if the indexed size exceeds `maximum_bytes`, the backing file
    /// changed after indexing, an exact bounded read fails, or a compressed payload does not inflate
    /// to its declared length.
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
            ResourceSource::MemoryArchive {
                archive,
                offset,
                end,
                compression,
                uncompressed_size,
            } => {
                let stored = archive
                    .get(*offset..*end)
                    .ok_or(ResourceReadError::ArchiveRange {
                        offset: *offset,
                        end: *end,
                        archive_size: archive.len(),
                    })?;
                inflate(stored, *compression, *uncompressed_size)
            }
            ResourceSource::LooseFile {
                path,
                indexed_file_size,
                size,
            } => {
                let mut file = open_unchanged(path, *indexed_file_size)?;
                read_exact_at(&mut file, path, None, *size)
            }
            ResourceSource::ArchiveFile {
                path,
                indexed_file_size,
                offset,
                compressed_size,
                compression,
                uncompressed_size,
            } => {
                let mut file = open_unchanged(path, *indexed_file_size)?;
                let stored = read_exact_at(&mut file, path, Some(*offset), *compressed_size)?;
                inflate(&stored, *compression, *uncompressed_size)
            }
        }
    }
}

#[derive(Debug, Clone)]
enum ResourceSource {
    Memory(Arc<[u8]>),
    /// A member of an archive held in memory.
    MemoryArchive {
        archive: Arc<[u8]>,
        offset: usize,
        end: usize,
        compression: Compression,
        uncompressed_size: usize,
    },
    LooseFile {
        path: PathBuf,
        indexed_file_size: u64,
        size: usize,
    },
    /// A member of an archive on disk, opened only when read.
    ArchiveFile {
        path: Arc<Path>,
        indexed_file_size: u64,
        offset: usize,
        compressed_size: usize,
        compression: Compression,
        uncompressed_size: usize,
    },
}

impl ResourceSource {
    fn len(&self) -> usize {
        match self {
            Self::Memory(bytes) => bytes.len(),
            Self::MemoryArchive {
                uncompressed_size, ..
            }
            | Self::ArchiveFile {
                uncompressed_size, ..
            } => *uncompressed_size,
            Self::LooseFile { size, .. } => *size,
        }
    }

    const fn compression(&self) -> Compression {
        match self {
            Self::Memory(_) | Self::LooseFile { .. } => Compression::Stored,
            Self::MemoryArchive { compression, .. } | Self::ArchiveFile { compression, .. } => {
                *compression
            }
        }
    }
}

/// Expands a stored payload, verifying it reaches exactly its declared length.
///
/// A short or long inflation means the index and the payload disagree, which is a corrupt archive
/// rather than something to paper over with a partial buffer.
fn inflate(
    stored: &[u8],
    compression: Compression,
    uncompressed_size: usize,
) -> Result<Vec<u8>, ResourceReadError> {
    match compression {
        Compression::Stored => Ok(stored.to_vec()),
        Compression::Deflate => {
            let mut decoded = Vec::with_capacity(uncompressed_size);
            // Bounded to one byte past the declaration, so an over-long stream is detected rather
            // than allowed to allocate freely.
            flate2::read::DeflateDecoder::new(stored)
                .take(uncompressed_size as u64 + 1)
                .read_to_end(&mut decoded)
                .map_err(ResourceReadError::Inflate)?;
            if decoded.len() != uncompressed_size {
                return Err(ResourceReadError::InflatedSizeMismatch {
                    actual: decoded.len(),
                    declared: uncompressed_size,
                });
            }
            Ok(decoded)
        }
    }
}

fn open_unchanged(path: &Path, indexed_file_size: u64) -> Result<File, ResourceReadError> {
    let file = File::open(path).map_err(|error| ResourceReadError::Io {
        path: path.to_path_buf(),
        error,
    })?;
    let metadata = file.metadata().map_err(|error| ResourceReadError::Io {
        path: path.to_path_buf(),
        error,
    })?;
    if metadata.len() != indexed_file_size {
        return Err(ResourceReadError::FileChanged {
            path: path.to_path_buf(),
            indexed: indexed_file_size,
            actual: metadata.len(),
        });
    }
    Ok(file)
}

fn read_exact_at(
    file: &mut File,
    path: &Path,
    offset: Option<usize>,
    length: usize,
) -> Result<Vec<u8>, ResourceReadError> {
    if let Some(offset) = offset {
        let offset = u64::try_from(offset).map_err(|_| ResourceReadError::ArchiveRange {
            offset,
            end: offset,
            archive_size: 0,
        })?;
        file.seek(SeekFrom::Start(offset))
            .map_err(|error| ResourceReadError::Io {
                path: path.to_path_buf(),
                error,
            })?;
    }
    let mut bytes = vec![0u8; length];
    file.read_exact(&mut bytes)
        .map_err(|error| ResourceReadError::Io {
            path: path.to_path_buf(),
            error,
        })?;
    Ok(bytes)
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

    /// Mounts members of an in-memory zip archive.
    ///
    /// # Errors
    ///
    /// Returns [`MountError::Archive`] when the container is invalid or exceeds limits, or
    /// [`MountError::MountIdExhausted`] when no stable mount identifier remains.
    pub fn mount_zip_bytes(
        &mut self,
        name: impl Into<String>,
        bytes: &[u8],
        limits: ArchiveLimits,
    ) -> Result<MountId, MountError> {
        let index = parse_zip_archive(bytes, limits).map_err(MountError::Archive)?;
        self.mount_archive_bytes(name.into(), ProviderKind::Zip, bytes, &index)
    }

    /// Mounts members of an in-memory tar archive.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::mount_zip_bytes`].
    pub fn mount_tar_bytes(
        &mut self,
        name: impl Into<String>,
        bytes: &[u8],
        limits: ArchiveLimits,
    ) -> Result<MountId, MountError> {
        let index = parse_tar_archive(bytes, limits).map_err(MountError::Archive)?;
        self.mount_archive_bytes(name.into(), ProviderKind::Tar, bytes, &index)
    }

    /// Decompresses a gzip-framed tar and mounts its members.
    ///
    /// A gzip stream is not seekable, so the whole archive is decompressed and retained in memory.
    /// That is the cost of the format, and it is why plain `.tar` or `.zip` is preferable for large
    /// content that should stay on disk.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::mount_zip_bytes`].
    pub fn mount_tar_gz_bytes(
        &mut self,
        name: impl Into<String>,
        bytes: &[u8],
        limits: ArchiveLimits,
    ) -> Result<MountId, MountError> {
        let (decoded, index) = parse_tar_gz_archive(bytes, limits).map_err(MountError::Archive)?;
        self.mount_archive_bytes(name.into(), ProviderKind::Tar, &decoded, &index)
    }

    /// Indexes a zip archive on disk, leaving member payloads unread until required.
    ///
    /// # Errors
    ///
    /// Returns a structured [`MountError`] for I/O, an invalid container, or exhausted identifiers.
    pub fn mount_zip_file(
        &mut self,
        name: impl Into<String>,
        path: impl AsRef<Path>,
        limits: ArchiveLimits,
    ) -> Result<MountId, MountError> {
        self.mount_archive_file(name.into(), ProviderKind::Zip, path.as_ref(), limits)
    }

    /// Indexes a tar archive on disk, leaving member payloads unread until required.
    ///
    /// # Errors
    ///
    /// Returns a structured [`MountError`] for I/O, an invalid container, or exhausted identifiers.
    pub fn mount_tar_file(
        &mut self,
        name: impl Into<String>,
        path: impl AsRef<Path>,
        limits: ArchiveLimits,
    ) -> Result<MountId, MountError> {
        self.mount_archive_file(name.into(), ProviderKind::Tar, path.as_ref(), limits)
    }

    /// Resolves the winning entry for a normalized path.
    #[must_use]
    pub fn resolve(&self, path: &VirtualPath) -> Option<&ResourceEntry> {
        self.entries.get(path).and_then(|history| history.last())
    }

    /// Returns every provider version from earliest to latest mount.
    ///
    /// Consumers use this for cumulative definition formats whose later providers partially
    /// extend or override earlier files. Opaque replacement resources should use [`Self::resolve`].
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

    /// Returns the number of distinct virtual paths with at least one entry.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether nothing is mounted.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn mount_archive_bytes(
        &mut self,
        name: String,
        kind: ProviderKind,
        bytes: &[u8],
        index: &ArchiveIndex,
    ) -> Result<MountId, MountError> {
        let archive: Arc<[u8]> = Arc::from(bytes);
        let entries = index
            .entries()
            .iter()
            .map(|entry| {
                (
                    entry.path().clone(),
                    ResourceSource::MemoryArchive {
                        archive: archive.clone(),
                        offset: entry.offset(),
                        end: entry.end(),
                        compression: entry.compression(),
                        uncompressed_size: entry.uncompressed_size(),
                    },
                )
            })
            .collect::<Vec<_>>();
        // Duplicates are preserved rather than refused: a container may legitimately store one name
        // twice, and last-wins matches how the overlay behaves between mounts.
        self.mount_entries(name, kind, entries, DuplicatePolicy::Preserve)
    }

    fn mount_archive_file(
        &mut self,
        name: String,
        kind: ProviderKind,
        path: &Path,
        limits: ArchiveLimits,
    ) -> Result<MountId, MountError> {
        // The container is read once to index it, then dropped; member payloads are re-read from
        // disk on demand. That keeps a large archive out of memory at the cost of one full read at
        // mount time, which is unavoidable for zip because its directory lives at the end.
        let bytes = fs::read(path).map_err(|error| MountError::Io {
            path: path.to_path_buf(),
            error,
        })?;
        let indexed_file_size =
            u64::try_from(bytes.len()).map_err(|_| MountError::FileTooLarge {
                path: path.to_path_buf(),
                size: u64::MAX,
            })?;
        let index = match kind {
            ProviderKind::Zip => parse_zip_archive(&bytes, limits),
            ProviderKind::Tar => parse_tar_archive(&bytes, limits),
            ProviderKind::LooseDirectory | ProviderKind::Memory => {
                return Err(MountError::Archive(ArchiveError::Signature {
                    archive: "archive",
                    expected: "a container kind that can be indexed from a file",
                }));
            }
        }
        .map_err(MountError::Archive)?;

        let shared: Arc<Path> = Arc::from(path);
        let entries = index
            .entries()
            .iter()
            .map(|entry| {
                (
                    entry.path().clone(),
                    ResourceSource::ArchiveFile {
                        path: shared.clone(),
                        indexed_file_size,
                        offset: entry.offset(),
                        compressed_size: entry.compressed_size(),
                        compression: entry.compression(),
                        uncompressed_size: entry.uncompressed_size(),
                    },
                )
            })
            .collect::<Vec<_>>();
        self.mount_entries(name, kind, entries, DuplicatePolicy::Preserve)
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

/// A failure while mounting a provider.
#[derive(Debug)]
pub enum MountError {
    /// A filesystem operation failed.
    Io {
        /// Path being read when the failure occurred.
        path: PathBuf,
        /// The underlying I/O failure.
        error: io::Error,
    },
    /// A stored name could not become a safe virtual path.
    Path(PathError),
    /// Two entries in one mount normalized to the same virtual path.
    DuplicatePath(VirtualPath),
    /// A symbolic link was found beneath a mounted directory.
    SymbolicLink(PathBuf),
    /// A discovered path was not beneath the mount root.
    OutsideRoot {
        /// The mount root.
        root: PathBuf,
        /// The offending path.
        path: PathBuf,
    },
    /// A file's length exceeded the addressable range.
    FileTooLarge {
        /// The offending path.
        path: PathBuf,
        /// Length reported by the filesystem.
        size: u64,
    },
    /// An explicit [`DirectoryLimits`] bound was exceeded.
    DirectoryLimitExceeded {
        /// Which bound was crossed.
        what: &'static str,
        /// Observed value.
        actual: usize,
        /// Configured maximum.
        maximum: usize,
    },
    /// An archive container was malformed or exceeded an explicit limit.
    Archive(ArchiveError),
    /// No stable mount identifier remained.
    MountIdExhausted,
}

impl Display for MountError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, error } => write!(formatter, "{}: {error}", path.display()),
            Self::Path(error) => Display::fmt(error, formatter),
            Self::DuplicatePath(path) => {
                write!(formatter, "duplicate virtual path in one mount: {path}")
            }
            Self::SymbolicLink(path) => {
                write!(
                    formatter,
                    "symbolic link is not mountable: {}",
                    path.display()
                )
            }
            Self::OutsideRoot { root, path } => write!(
                formatter,
                "{} is not beneath mount root {}",
                path.display(),
                root.display()
            ),
            Self::FileTooLarge { path, size } => {
                write!(formatter, "{} is too large: {size} bytes", path.display())
            }
            Self::DirectoryLimitExceeded {
                what,
                actual,
                maximum,
            } => write!(formatter, "{what} {actual} exceeds maximum {maximum}"),
            Self::Archive(error) => Display::fmt(error, formatter),
            Self::MountIdExhausted => formatter.write_str("mount identifiers are exhausted"),
        }
    }
}

impl Error for MountError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { error, .. } => Some(error),
            Self::Path(error) => Some(error),
            Self::Archive(error) => Some(error),
            _ => None,
        }
    }
}

/// A failure while reading one resource payload.
#[derive(Debug)]
pub enum ResourceReadError {
    /// The indexed size exceeded the caller's allocation bound.
    LimitExceeded {
        /// Indexed decompressed size.
        actual: usize,
        /// Bound the caller supplied.
        maximum: usize,
    },
    /// A filesystem operation failed.
    Io {
        /// Path being read.
        path: PathBuf,
        /// The underlying I/O failure.
        error: io::Error,
    },
    /// A backing file's length changed between indexing and reading.
    FileChanged {
        /// The offending path.
        path: PathBuf,
        /// Length observed at index time.
        indexed: u64,
        /// Length observed at read time.
        actual: u64,
    },
    /// An in-memory archive member's range was outside the archive.
    ArchiveRange {
        /// Member payload offset.
        offset: usize,
        /// Member payload exclusive end.
        end: usize,
        /// Total archive length.
        archive_size: usize,
    },
    /// A compressed payload failed to inflate.
    Inflate(io::Error),
    /// A compressed payload inflated to a length other than the one indexed.
    InflatedSizeMismatch {
        /// Bytes actually produced.
        actual: usize,
        /// Length the container declared.
        declared: usize,
    },
}

impl Display for ResourceReadError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::LimitExceeded { actual, maximum } => write!(
                formatter,
                "resource is {actual} bytes, exceeding maximum {maximum}"
            ),
            Self::Io { path, error } => write!(formatter, "{}: {error}", path.display()),
            Self::FileChanged {
                path,
                indexed,
                actual,
            } => write!(
                formatter,
                "{} changed after indexing: was {indexed} bytes, now {actual}",
                path.display()
            ),
            Self::ArchiveRange {
                offset,
                end,
                archive_size,
            } => write!(
                formatter,
                "archive member range {offset}..{end} is outside a {archive_size}-byte archive"
            ),
            Self::Inflate(error) => write!(formatter, "payload failed to inflate: {error}"),
            Self::InflatedSizeMismatch { actual, declared } => write!(
                formatter,
                "payload inflated to {actual} bytes, but the container declared {declared}"
            ),
        }
    }
}

impl Error for ResourceReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { error, .. } | Self::Inflate(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ArchiveLimits, Compression, ProviderKind, Vfs, VirtualPath};
    use crate::testing::{TarBuilder, ZipBuilder, deflate, gzip};

    fn path(text: &str) -> VirtualPath {
        VirtualPath::new(text).expect("valid virtual path")
    }

    #[test]
    fn normalizes_separators_case_and_dot_components() {
        assert_eq!(
            path("Art\\Textures/./Sand.DDS").as_str(),
            "art/textures/sand.dds"
        );
        assert_eq!(path("//a///b//").as_str(), "a/b");
    }

    #[test]
    fn refuses_parent_traversal_and_empty_paths() {
        assert!(VirtualPath::new("../secret").is_err());
        assert!(VirtualPath::new("a/../b").is_err());
        assert!(VirtualPath::new("///").is_err());
    }

    #[test]
    fn reports_the_extension_of_the_final_component() {
        assert_eq!(path("maps/alpine.zip").extension(), Some("zip"));
        assert_eq!(path("models/tank.GLB").extension(), Some("glb"));
        assert_eq!(path("readme").extension(), None);
        assert_eq!(path("archive.tar/inner").extension(), None);
    }

    #[test]
    fn later_mounts_win_and_history_preserves_every_version() {
        let mut vfs = Vfs::new();
        vfs.mount_memory("base", [(path("rules.json"), b"base".to_vec())])
            .expect("mount base");
        vfs.mount_memory("expansion", [(path("rules.json"), b"expansion".to_vec())])
            .expect("mount expansion");
        vfs.mount_memory("mod", [(path("rules.json"), b"mod".to_vec())])
            .expect("mount mod");

        let winner = vfs.resolve(&path("rules.json")).expect("resolved");
        assert_eq!(winner.read(64).expect("read"), b"mod");
        assert_eq!(winner.provider().name(), "mod");

        let history = vfs.history(&path("rules.json")).expect("history");
        assert_eq!(history.len(), 3);
        let names: Vec<&str> = history.iter().map(|e| e.provider().name()).collect();
        assert_eq!(names, ["base", "expansion", "mod"], "earliest to latest");
    }

    #[test]
    fn mount_ids_increase_in_explicit_mount_order() {
        let mut vfs = Vfs::new();
        let first = vfs.mount_memory("a", []).expect("mount");
        let second = vfs.mount_memory("b", []).expect("mount");
        assert_eq!(first.get(), 0);
        assert_eq!(second.get(), 1);
    }

    #[test]
    fn refuses_two_entries_normalizing_to_one_path_in_a_single_mount() {
        let mut vfs = Vfs::new();
        let error = vfs
            .mount_memory(
                "clash",
                [
                    (path("Art/Sand.dds"), b"a".to_vec()),
                    (path("art/SAND.DDS"), b"b".to_vec()),
                ],
            )
            .expect_err("case folding makes these one path");
        assert!(matches!(error, super::MountError::DuplicatePath(_)));
    }

    #[test]
    fn mounts_a_zip_and_reads_a_stored_member() {
        let archive = ZipBuilder::new()
            .stored("terrain/height.bin", b"elevation")
            .finish();
        let mut vfs = Vfs::new();
        let mount = vfs
            .mount_zip_bytes("alpine.zip", &archive, ArchiveLimits::default())
            .expect("mount zip");
        let entry = vfs.resolve(&path("terrain/height.bin")).expect("resolved");
        assert_eq!(entry.provider().mount_id(), mount);
        assert_eq!(entry.provider().kind(), ProviderKind::Zip);
        assert_eq!(entry.compression(), Compression::Stored);
        assert_eq!(entry.read(64).expect("read"), b"elevation");
    }

    #[test]
    fn mounts_a_zip_and_inflates_a_deflated_member() {
        let payload = b"{\"players\":4}".repeat(200);
        let archive = ZipBuilder::new()
            .deflated("map.json", &deflate(&payload), payload.len())
            .finish();
        let mut vfs = Vfs::new();
        vfs.mount_zip_bytes("alpine.zip", &archive, ArchiveLimits::default())
            .expect("mount zip");
        let entry = vfs.resolve(&path("map.json")).expect("resolved");
        assert_eq!(entry.compression(), Compression::Deflate);
        assert_eq!(entry.len(), payload.len(), "len reports decompressed size");
        assert_eq!(entry.read(payload.len()).expect("read"), payload);
    }

    #[test]
    fn a_read_bound_is_checked_against_the_decompressed_size_before_inflating() {
        let payload = vec![0u8; 100_000];
        let compressed = deflate(&payload);
        assert!(compressed.len() < 1_000, "fixture must compress hard");
        let archive = ZipBuilder::new()
            .deflated("bomb", &compressed, payload.len())
            .finish();
        let mut vfs = Vfs::new();
        vfs.mount_zip_bytes("bomb.zip", &archive, ArchiveLimits::default())
            .expect("mount zip");
        let entry = vfs.resolve(&path("bomb")).expect("resolved");
        let error = entry.read(1_024).expect_err("must refuse before inflating");
        assert!(matches!(
            error,
            super::ResourceReadError::LimitExceeded {
                actual: 100_000,
                maximum: 1_024
            }
        ));
    }

    #[test]
    fn mounts_a_tar_and_reads_its_members() {
        let archive = TarBuilder::new()
            .file("map.json", b"{}")
            .file("terrain/height.bin", &[9u8; 600])
            .finish();
        let mut vfs = Vfs::new();
        vfs.mount_tar_bytes("alpine.tar", &archive, ArchiveLimits::default())
            .expect("mount tar");
        assert_eq!(vfs.len(), 2);
        let entry = vfs.resolve(&path("terrain/height.bin")).expect("resolved");
        assert_eq!(entry.provider().kind(), ProviderKind::Tar);
        assert_eq!(entry.read(1_024).expect("read"), vec![9u8; 600]);
    }

    #[test]
    fn mounts_a_gzip_framed_tar() {
        let archive = TarBuilder::new().file("map.json", b"{\"v\":1}").finish();
        let mut vfs = Vfs::new();
        vfs.mount_tar_gz_bytes("alpine.tar.gz", &gzip(&archive), ArchiveLimits::default())
            .expect("mount tar.gz");
        let entry = vfs.resolve(&path("map.json")).expect("resolved");
        assert_eq!(entry.read(64).expect("read"), b"{\"v\":1}");
    }

    #[test]
    fn a_zip_mount_overrides_a_directory_mount_beneath_it() {
        // The mod-loading case: loose base content, then a packaged override.
        let directory = tempdir("vfs-overlay");
        std::fs::create_dir_all(directory.join("terrain")).expect("create dir");
        std::fs::write(directory.join("terrain/height.bin"), b"base").expect("write");
        std::fs::write(directory.join("readme.txt"), b"untouched").expect("write");

        let archive = ZipBuilder::new()
            .stored("terrain/height.bin", b"override")
            .finish();

        let mut vfs = Vfs::new();
        vfs.mount_directory("base", &directory).expect("mount dir");
        vfs.mount_zip_bytes("patch.zip", &archive, ArchiveLimits::default())
            .expect("mount zip");

        assert_eq!(
            vfs.resolve(&path("terrain/height.bin"))
                .expect("resolved")
                .read(64)
                .expect("read"),
            b"override"
        );
        assert_eq!(
            vfs.resolve(&path("readme.txt"))
                .expect("resolved")
                .read(64)
                .expect("read"),
            b"untouched",
            "an unrelated base file must survive the overlay"
        );
        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn indexes_an_archive_on_disk_and_reads_members_lazily() {
        let directory = tempdir("vfs-lazy");
        std::fs::create_dir_all(&directory).expect("create dir");
        let archive_path = directory.join("alpine.zip");
        let payload = b"a".repeat(5_000);
        let archive = ZipBuilder::new()
            .stored("terrain/height.bin", &payload)
            .deflated("map.json", &deflate(b"{\"v\":1}"), 7)
            .finish();
        std::fs::write(&archive_path, &archive).expect("write archive");

        let mut vfs = Vfs::new();
        vfs.mount_zip_file("alpine.zip", &archive_path, ArchiveLimits::default())
            .expect("mount zip file");
        assert_eq!(
            vfs.resolve(&path("terrain/height.bin"))
                .expect("resolved")
                .read(8_192)
                .expect("read"),
            payload
        );
        assert_eq!(
            vfs.resolve(&path("map.json"))
                .expect("resolved")
                .read(64)
                .expect("read"),
            b"{\"v\":1}"
        );
        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn detects_a_backing_archive_that_changed_after_indexing() {
        let directory = tempdir("vfs-changed");
        std::fs::create_dir_all(&directory).expect("create dir");
        let archive_path = directory.join("alpine.zip");
        let archive = ZipBuilder::new().stored("a.bin", b"original").finish();
        std::fs::write(&archive_path, &archive).expect("write archive");

        let mut vfs = Vfs::new();
        vfs.mount_zip_file("alpine.zip", &archive_path, ArchiveLimits::default())
            .expect("mount");
        // Truncating changes the length, which is what indexing recorded.
        std::fs::write(&archive_path, b"gone").expect("truncate");
        let error = vfs
            .resolve(&path("a.bin"))
            .expect("resolved")
            .read(64)
            .expect_err("must detect the change");
        assert!(matches!(
            error,
            super::ResourceReadError::FileChanged { .. }
        ));
        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn iterates_resolved_entries_in_path_order() {
        let mut vfs = Vfs::new();
        vfs.mount_memory(
            "content",
            [
                (path("z.bin"), b"z".to_vec()),
                (path("a.bin"), b"a".to_vec()),
                (path("m/n.bin"), b"n".to_vec()),
            ],
        )
        .expect("mount");
        let order: Vec<&str> = vfs.iter_resolved().map(|(path, _)| path.as_str()).collect();
        assert_eq!(order, ["a.bin", "m/n.bin", "z.bin"]);
    }

    /// Returns a unique scratch directory path derived from the test name and process id.
    ///
    /// Deliberately not `std::env::temp_dir()` alone: two tests running concurrently in the same
    /// process must not collide, and the suffix keeps them apart without a dependency.
    fn tempdir(label: &str) -> std::path::PathBuf {
        let unique = std::process::id();
        let directory = std::env::temp_dir().join(format!("cic-{label}-{unique}"));
        std::fs::remove_dir_all(&directory).ok();
        std::fs::create_dir_all(&directory).expect("create scratch directory");
        directory
    }
}
