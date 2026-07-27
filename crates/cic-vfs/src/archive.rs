//! Container-agnostic archive index shared by every archive reader.
//!
//! A reader's only job is to turn untrusted bytes into an [`ArchiveIndex`]: a bounded, ordered list
//! of members, each recording where its payload lives and how it is compressed. Nothing here reads
//! a payload. That keeps indexing cheap and total, and leaves the decision of *when* to spend
//! memory on a decompressed member to [`crate::ResourceEntry::read`].
//!
//! Every reader shares one [`ArchiveLimits`] and one [`ArchiveError`] so a caller can mount a zip
//! and a tar through the same code path and handle their failures identically.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use crate::{PathError, VirtualPath};

/// How one member's payload is stored inside its container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    /// Stored verbatim. The compressed and uncompressed sizes are equal.
    Stored,
    /// Raw DEFLATE, as zip method 8. No zlib or gzip wrapper.
    Deflate,
}

impl Display for Compression {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Stored => "stored",
            Self::Deflate => "deflate",
        })
    }
}

/// Explicit bounds applied while indexing one archive.
///
/// The defaults are deliberately generous for a game's own content and still far below what an
/// adversarial archive would need to exhaust memory. `maximum_total_bytes` is the one that stops a
/// zip bomb: a few kilobytes of central directory can claim terabytes of output, and refusing at
/// index time means no caller ever gets the chance to allocate it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveLimits {
    /// Maximum number of members retained from one archive.
    pub maximum_entries: usize,
    /// Maximum declared uncompressed size of any single member.
    pub maximum_entry_bytes: usize,
    /// Maximum summed declared uncompressed size across every member.
    pub maximum_total_bytes: usize,
    /// Maximum stored path length in bytes, before normalization.
    pub maximum_path_bytes: usize,
}

impl Default for ArchiveLimits {
    fn default() -> Self {
        Self {
            maximum_entries: 65_536,
            maximum_entry_bytes: 512 * 1_024 * 1_024,
            maximum_total_bytes: 4 * 1_024 * 1_024 * 1_024,
            maximum_path_bytes: 4_096,
        }
    }
}

/// One indexed archive member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveEntry {
    path: VirtualPath,
    offset: usize,
    compressed_size: usize,
    uncompressed_size: usize,
    compression: Compression,
}

impl ArchiveEntry {
    pub(crate) const fn new(
        path: VirtualPath,
        offset: usize,
        compressed_size: usize,
        uncompressed_size: usize,
        compression: Compression,
    ) -> Self {
        Self {
            path,
            offset,
            compressed_size,
            uncompressed_size,
            compression,
        }
    }

    /// Returns the member's normalized virtual path.
    #[must_use]
    pub const fn path(&self) -> &VirtualPath {
        &self.path
    }

    /// Returns the byte offset of the member's payload within the archive.
    #[must_use]
    pub const fn offset(&self) -> usize {
        self.offset
    }

    /// Returns the payload length as stored, before decompression.
    #[must_use]
    pub const fn compressed_size(&self) -> usize {
        self.compressed_size
    }

    /// Returns the payload length after decompression, as the container declares it.
    #[must_use]
    pub const fn uncompressed_size(&self) -> usize {
        self.uncompressed_size
    }

    /// Returns how the payload is stored.
    #[must_use]
    pub const fn compression(&self) -> Compression {
        self.compression
    }

    /// Returns the exclusive end offset of the stored payload.
    #[must_use]
    pub const fn end(&self) -> usize {
        // Checked during indexing, which rejects any entry whose range overflows or leaves the
        // archive, so this cannot wrap on an index that exists.
        self.offset + self.compressed_size
    }
}

/// A bounded, ordered index of one archive's members.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArchiveIndex {
    entries: Vec<ArchiveEntry>,
}

impl ArchiveIndex {
    pub(crate) const fn from_entries(entries: Vec<ArchiveEntry>) -> Self {
        Self { entries }
    }

    /// Returns every member in the container's own order.
    ///
    /// Order is preserved rather than sorted because a container may legitimately store the same
    /// name twice, and which one wins is a mount-policy decision rather than an indexing one.
    #[must_use]
    pub fn entries(&self) -> &[ArchiveEntry] {
        &self.entries
    }

    /// Returns the number of indexed members.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether the archive indexed no members.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Accumulates members while enforcing [`ArchiveLimits`] as each one is added.
///
/// Checking during accumulation rather than afterward is what keeps a hostile archive from ever
/// causing a large allocation: the entry that crosses a limit is refused before it is stored.
pub(crate) struct BoundedEntries {
    limits: ArchiveLimits,
    entries: Vec<ArchiveEntry>,
    total_uncompressed: usize,
    archive: &'static str,
}

impl BoundedEntries {
    pub(crate) const fn new(limits: ArchiveLimits, archive: &'static str) -> Self {
        Self {
            limits,
            entries: Vec::new(),
            total_uncompressed: 0,
            archive,
        }
    }

    /// Normalizes and bounds one member, then retains it.
    pub(crate) fn push(
        &mut self,
        raw_path: &[u8],
        offset: usize,
        compressed_size: usize,
        uncompressed_size: usize,
        compression: Compression,
        archive_size: usize,
    ) -> Result<(), ArchiveError> {
        if self.entries.len() >= self.limits.maximum_entries {
            return Err(ArchiveError::LimitExceeded {
                archive: self.archive,
                what: "entry count",
                actual: self.entries.len() + 1,
                maximum: self.limits.maximum_entries,
            });
        }
        if raw_path.len() > self.limits.maximum_path_bytes {
            return Err(ArchiveError::LimitExceeded {
                archive: self.archive,
                what: "entry path length",
                actual: raw_path.len(),
                maximum: self.limits.maximum_path_bytes,
            });
        }
        if uncompressed_size > self.limits.maximum_entry_bytes {
            return Err(ArchiveError::LimitExceeded {
                archive: self.archive,
                what: "entry uncompressed size",
                actual: uncompressed_size,
                maximum: self.limits.maximum_entry_bytes,
            });
        }
        let total = self
            .total_uncompressed
            .checked_add(uncompressed_size)
            .ok_or(ArchiveError::LimitExceeded {
                archive: self.archive,
                what: "total uncompressed size",
                actual: usize::MAX,
                maximum: self.limits.maximum_total_bytes,
            })?;
        if total > self.limits.maximum_total_bytes {
            return Err(ArchiveError::LimitExceeded {
                archive: self.archive,
                what: "total uncompressed size",
                actual: total,
                maximum: self.limits.maximum_total_bytes,
            });
        }

        let end = offset
            .checked_add(compressed_size)
            .ok_or(ArchiveError::EntryRange {
                archive: self.archive,
                entry: self.entries.len(),
                offset,
                size: compressed_size,
            })?;
        if end > archive_size {
            return Err(ArchiveError::EntryOutsideArchive {
                archive: self.archive,
                entry: self.entries.len(),
                end,
                archive_size,
            });
        }
        if compression == Compression::Stored && compressed_size != uncompressed_size {
            return Err(ArchiveError::StoredSizeMismatch {
                archive: self.archive,
                entry: self.entries.len(),
                compressed_size,
                uncompressed_size,
            });
        }

        let text = str::from_utf8(raw_path).map_err(|_| ArchiveError::NonUtf8Path {
            archive: self.archive,
            entry: self.entries.len(),
        })?;
        let path = VirtualPath::new(text).map_err(|error| ArchiveError::Path {
            archive: self.archive,
            entry: self.entries.len(),
            error,
        })?;

        self.total_uncompressed = total;
        self.entries.push(ArchiveEntry::new(
            path,
            offset,
            compressed_size,
            uncompressed_size,
            compression,
        ));
        Ok(())
    }

    pub(crate) fn finish(self) -> ArchiveIndex {
        ArchiveIndex::from_entries(self.entries)
    }
}

/// A structured failure while indexing an untrusted archive.
///
/// Every variant names the container kind, so one match arm can report a zip and a tar failure
/// without the caller tracking which reader it called.
#[derive(Debug)]
pub enum ArchiveError {
    /// A bounded read left the archive's byte region.
    Binary {
        /// Container kind that was being indexed.
        archive: &'static str,
        /// The underlying bounded-read failure.
        error: cic_core::BinaryError,
    },
    /// The container's magic or structural signature was absent.
    Signature {
        /// Container kind that was being indexed.
        archive: &'static str,
        /// What the reader was looking for.
        expected: &'static str,
    },
    /// An explicit [`ArchiveLimits`] bound was exceeded.
    LimitExceeded {
        /// Container kind that was being indexed.
        archive: &'static str,
        /// Which bound was crossed.
        what: &'static str,
        /// Observed value.
        actual: usize,
        /// Configured maximum.
        maximum: usize,
    },
    /// One member's offset and size overflowed when added.
    EntryRange {
        /// Container kind that was being indexed.
        archive: &'static str,
        /// Zero-based member index.
        entry: usize,
        /// Declared payload offset.
        offset: usize,
        /// Declared payload size.
        size: usize,
    },
    /// One member's payload extended beyond the archive.
    EntryOutsideArchive {
        /// Container kind that was being indexed.
        archive: &'static str,
        /// Zero-based member index.
        entry: usize,
        /// Computed exclusive end offset.
        end: usize,
        /// Total archive length.
        archive_size: usize,
    },
    /// A stored member declared different compressed and uncompressed sizes.
    StoredSizeMismatch {
        /// Container kind that was being indexed.
        archive: &'static str,
        /// Zero-based member index.
        entry: usize,
        /// Declared stored size.
        compressed_size: usize,
        /// Declared uncompressed size.
        uncompressed_size: usize,
    },
    /// A member used a compression method this reader does not implement.
    UnsupportedCompression {
        /// Container kind that was being indexed.
        archive: &'static str,
        /// Zero-based member index.
        entry: usize,
        /// The container's own method identifier.
        method: u16,
    },
    /// A member's stored path was not valid UTF-8.
    NonUtf8Path {
        /// Container kind that was being indexed.
        archive: &'static str,
        /// Zero-based member index.
        entry: usize,
    },
    /// A member's stored path could not become a safe virtual path.
    Path {
        /// Container kind that was being indexed.
        archive: &'static str,
        /// Zero-based member index.
        entry: usize,
        /// The underlying normalization failure.
        error: PathError,
    },
}

impl Display for ArchiveError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binary { archive, error } => write!(formatter, "{archive}: {error}"),
            Self::Signature { archive, expected } => {
                write!(formatter, "{archive}: missing {expected}")
            }
            Self::LimitExceeded {
                archive,
                what,
                actual,
                maximum,
            } => write!(
                formatter,
                "{archive}: {what} {actual} exceeds maximum {maximum}"
            ),
            Self::EntryRange {
                archive,
                entry,
                offset,
                size,
            } => write!(
                formatter,
                "{archive}: entry {entry} range {offset}+{size} overflows"
            ),
            Self::EntryOutsideArchive {
                archive,
                entry,
                end,
                archive_size,
            } => write!(
                formatter,
                "{archive}: entry {entry} ends at {end}, past archive size {archive_size}"
            ),
            Self::StoredSizeMismatch {
                archive,
                entry,
                compressed_size,
                uncompressed_size,
            } => write!(
                formatter,
                "{archive}: stored entry {entry} declares {compressed_size} stored \
                 against {uncompressed_size} uncompressed"
            ),
            Self::UnsupportedCompression {
                archive,
                entry,
                method,
            } => write!(
                formatter,
                "{archive}: entry {entry} uses unsupported compression method {method}"
            ),
            Self::NonUtf8Path { archive, entry } => {
                write!(formatter, "{archive}: entry {entry} path is not UTF-8")
            }
            Self::Path {
                archive,
                entry,
                error,
            } => write!(formatter, "{archive}: entry {entry} path: {error}"),
        }
    }
}

impl Error for ArchiveError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Binary { error, .. } => Some(error),
            Self::Path { error, .. } => Some(error),
            _ => None,
        }
    }
}
