// SPDX-License-Identifier: GPL-3.0-only
//
// Format provenance (facts, not a line-for-line translation):
// - TheSuperHackers/GeneralsGameCode, revision
//   9f7abb866f5afd446db14149979e744c7216baaf,
//   Core/GameEngineDevice/Source/StdDevice/Common/StdBIGFileSystem.cpp
// - OpenSAGE/OpenSAGE, revision 588ac477367a0022adf29f20a084e8873014e6ce,
//   src/OpenSage.FileFormats.Big/BigArchive.cs
// See docs/provenance/big.md for the evidence and licensing record.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use cic_core::{BinaryError, BinaryReader};

use crate::{PathError, VirtualPath};

const HEADER_LENGTH: usize = 16;

/// Supported BIG archive header variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BigVersion {
    /// Archive begins with `BIGF`.
    BigF,
    /// Archive begins with `BIG4`.
    Big4,
}

/// Explicit resource limits applied before archive entries are allocated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BigLimits {
    /// Maximum accepted complete archive size.
    pub maximum_archive_bytes: usize,
    /// Maximum number of file-table entries.
    pub maximum_entries: usize,
    /// Maximum byte length of one zero-terminated entry name.
    pub maximum_name_bytes: usize,
    /// Maximum opaque bytes between the final entry and first payload.
    pub maximum_directory_trailer_bytes: usize,
}

impl Default for BigLimits {
    fn default() -> Self {
        Self {
            maximum_archive_bytes: 2 * 1024 * 1024 * 1024,
            maximum_entries: 1_000_000,
            maximum_name_bytes: 4096,
            maximum_directory_trailer_bytes: 64 * 1024,
        }
    }
}

/// Validated location and normalized name of one BIG member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BigEntry {
    path: VirtualPath,
    offset: usize,
    size: usize,
    end: usize,
}

impl BigEntry {
    /// Returns the normalized virtual resource path.
    #[must_use]
    pub const fn path(&self) -> &VirtualPath {
        &self.path
    }

    /// Returns the member's byte offset in the archive.
    #[must_use]
    pub const fn offset(&self) -> usize {
        self.offset
    }

    /// Returns the member's byte length.
    #[must_use]
    pub const fn size(&self) -> usize {
        self.size
    }

    pub(crate) fn bytes<'a>(&self, archive: &'a [u8]) -> Option<&'a [u8]> {
        archive.get(self.offset..self.end)
    }
}

/// Validated, allocation-bounded BIG file table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BigArchiveIndex {
    version: BigVersion,
    archive_size: usize,
    first_file_offset: usize,
    entries: Vec<BigEntry>,
    directory_trailer: Vec<u8>,
}

impl BigArchiveIndex {
    /// Returns the BIG header variant.
    #[must_use]
    pub const fn version(&self) -> BigVersion {
        self.version
    }

    /// Returns the declared complete archive size.
    #[must_use]
    pub const fn archive_size(&self) -> usize {
        self.archive_size
    }

    /// Returns the declared first payload byte.
    #[must_use]
    pub const fn first_file_offset(&self) -> usize {
        self.first_file_offset
    }

    /// Returns file-table entries in their original order.
    #[must_use]
    pub fn entries(&self) -> &[BigEntry] {
        &self.entries
    }

    /// Returns opaque bytes between the final table entry and first payload.
    #[must_use]
    pub fn directory_trailer(&self) -> &[u8] {
        &self.directory_trailer
    }
}

/// Parses and validates a BIG file table without copying member payloads.
///
/// # Errors
///
/// Returns [`BigError`] for unsupported signatures, truncation, resource-limit excess,
/// invalid header bounds, invalid names, excessive directory trailer data, or member
/// ranges outside the declared payload region.
pub fn parse_big_archive(bytes: &[u8], limits: BigLimits) -> Result<BigArchiveIndex, BigError> {
    if bytes.len() > limits.maximum_archive_bytes {
        return Err(BinaryError::LimitExceeded {
            what: "BIG archive size",
            actual: bytes.len(),
            maximum: limits.maximum_archive_bytes,
        }
        .into());
    }

    let mut reader = BinaryReader::new(bytes, "BIG archive");
    let signature = match reader.read_exact(4)? {
        [first, second, third, fourth] => [*first, *second, *third, *fourth],
        _ => {
            return Err(BinaryError::UnexpectedEof {
                source: "BIG archive".to_owned(),
                offset: 0,
                requested: 4,
                remaining: 0,
            }
            .into());
        }
    };
    let version = match &signature {
        b"BIGF" => BigVersion::BigF,
        b"BIG4" => BigVersion::Big4,
        _ => return Err(BigError::UnsupportedSignature(signature)),
    };

    let archive_size = host_index(reader.read_u32_le()?, "archive size")?;
    if archive_size != bytes.len() {
        return Err(BigError::ArchiveSizeMismatch {
            declared: archive_size,
            actual: bytes.len(),
        });
    }

    let entry_count = host_index(reader.read_u32_be()?, "entry count")?;
    if entry_count > limits.maximum_entries {
        return Err(BinaryError::LimitExceeded {
            what: "BIG entry count",
            actual: entry_count,
            maximum: limits.maximum_entries,
        }
        .into());
    }

    let first_file_offset = host_index(reader.read_u32_be()?, "first-file offset")?;
    if !(HEADER_LENGTH..=archive_size).contains(&first_file_offset) {
        return Err(BigError::InvalidFirstFileOffset {
            offset: first_file_offset,
            archive_size,
        });
    }

    let directory_length = first_file_offset - HEADER_LENGTH;
    let mut directory = reader.read_region(directory_length)?;
    let entries = parse_entries(
        &mut directory,
        entry_count,
        first_file_offset,
        archive_size,
        limits.maximum_name_bytes,
    )?;

    let trailer_length = directory.remaining();
    if trailer_length > limits.maximum_directory_trailer_bytes {
        return Err(BinaryError::LimitExceeded {
            what: "BIG directory trailer size",
            actual: trailer_length,
            maximum: limits.maximum_directory_trailer_bytes,
        }
        .into());
    }
    let directory_trailer = directory.read_exact(trailer_length)?.to_vec();

    Ok(BigArchiveIndex {
        version,
        archive_size,
        first_file_offset,
        entries,
        directory_trailer,
    })
}

fn parse_entries(
    directory: &mut BinaryReader<'_>,
    entry_count: usize,
    first_file_offset: usize,
    archive_size: usize,
    maximum_name_bytes: usize,
) -> Result<Vec<BigEntry>, BigError> {
    let mut entries = Vec::with_capacity(entry_count);
    for entry_index in 0..entry_count {
        let offset = host_index(directory.read_u32_be()?, "member offset")?;
        let size = host_index(directory.read_u32_be()?, "member size")?;
        let name_offset = HEADER_LENGTH + directory.position();
        let name_bytes = directory.read_c_string_bytes(maximum_name_bytes)?;
        let name = std::str::from_utf8(name_bytes).map_err(|_| BigError::InvalidNameEncoding {
            entry: entry_index,
            offset: name_offset,
        })?;
        let path = VirtualPath::new(name).map_err(|source| BigError::InvalidPath {
            entry: entry_index,
            source,
        })?;
        let end = offset
            .checked_add(size)
            .ok_or(BigError::EntryRangeOverflow {
                entry: entry_index,
                offset,
                size,
            })?;
        let is_zero_length_marker = offset == 0 && size == 0;
        if !is_zero_length_marker && (offset < first_file_offset || end > archive_size) {
            return Err(BigError::EntryOutsidePayload {
                entry: entry_index,
                offset,
                size,
                first_file_offset,
                archive_size,
            });
        }
        entries.push(BigEntry {
            path,
            offset,
            size,
            end,
        });
    }
    Ok(entries)
}

fn host_index(value: u32, field: &'static str) -> Result<usize, BigError> {
    usize::try_from(value).map_err(|_| BigError::HostIndexTooNarrow { field, value })
}

/// A structured failure while indexing a BIG archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BigError {
    /// A bounded binary read failed.
    Binary(BinaryError),
    /// The four-byte archive signature is unsupported.
    UnsupportedSignature([u8; 4]),
    /// A 32-bit format field does not fit the host index type.
    HostIndexTooNarrow {
        /// Name of the format field.
        field: &'static str,
        /// Value that cannot be represented.
        value: u32,
    },
    /// The header's archive size differs from the supplied byte region.
    ArchiveSizeMismatch {
        /// Header value.
        declared: usize,
        /// Supplied byte-region length.
        actual: usize,
    },
    /// The first payload offset is before the header or after the archive.
    InvalidFirstFileOffset {
        /// Header value.
        offset: usize,
        /// Declared archive length.
        archive_size: usize,
    },
    /// A member name is not valid UTF-8.
    InvalidNameEncoding {
        /// Zero-based file-table entry.
        entry: usize,
        /// Absolute byte offset of the name.
        offset: usize,
    },
    /// A member name is not a safe virtual path.
    InvalidPath {
        /// Zero-based file-table entry.
        entry: usize,
        /// Path normalization failure.
        source: PathError,
    },
    /// Member offset plus size overflowed the host index type.
    EntryRangeOverflow {
        /// Zero-based file-table entry.
        entry: usize,
        /// Member byte offset.
        offset: usize,
        /// Member byte length.
        size: usize,
    },
    /// A member range does not lie completely in the payload region.
    EntryOutsidePayload {
        /// Zero-based file-table entry.
        entry: usize,
        /// Member byte offset.
        offset: usize,
        /// Member byte length.
        size: usize,
        /// First legal payload byte.
        first_file_offset: usize,
        /// First byte beyond the archive.
        archive_size: usize,
    },
}

impl From<BinaryError> for BigError {
    fn from(error: BinaryError) -> Self {
        Self::Binary(error)
    }
}

impl Display for BigError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binary(error) => Display::fmt(error, formatter),
            Self::UnsupportedSignature(signature) => write!(
                formatter,
                "unsupported BIG signature {signature:02X?}; expected BIGF or BIG4"
            ),
            Self::HostIndexTooNarrow { field, value } => write!(
                formatter,
                "BIG {field} value {value} does not fit the host index type"
            ),
            Self::ArchiveSizeMismatch { declared, actual } => write!(
                formatter,
                "BIG header declares {declared} bytes but input contains {actual} bytes"
            ),
            Self::InvalidFirstFileOffset {
                offset,
                archive_size,
            } => write!(
                formatter,
                "BIG first-file offset {offset} is outside header-to-archive range 16..={archive_size}"
            ),
            Self::InvalidNameEncoding { entry, offset } => write!(
                formatter,
                "BIG entry {entry} name at byte {offset} is not valid UTF-8"
            ),
            Self::InvalidPath { entry, source } => {
                write!(formatter, "BIG entry {entry} has invalid path: {source}")
            }
            Self::EntryRangeOverflow {
                entry,
                offset,
                size,
            } => write!(
                formatter,
                "BIG entry {entry} range overflows: offset {offset}, size {size}"
            ),
            Self::EntryOutsidePayload {
                entry,
                offset,
                size,
                first_file_offset,
                archive_size,
            } => write!(
                formatter,
                "BIG entry {entry} range ({offset}, {size}) is outside payload {first_file_offset}..{archive_size}"
            ),
        }
    }
}

impl Error for BigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Binary(error) => Some(error),
            Self::InvalidPath { source, .. } => Some(source),
            Self::UnsupportedSignature(_)
            | Self::HostIndexTooNarrow { .. }
            | Self::ArchiveSizeMismatch { .. }
            | Self::InvalidFirstFileOffset { .. }
            | Self::InvalidNameEncoding { .. }
            | Self::EntryRangeOverflow { .. }
            | Self::EntryOutsidePayload { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BigError, BigLimits, BigVersion, parse_big_archive};

    fn fixture() -> Vec<u8> {
        let hex = include_str!("../tests/fixtures/minimal.big.hex");
        let digits = hex
            .bytes()
            .filter(|byte| !byte.is_ascii_whitespace())
            .collect::<Vec<_>>();
        assert_eq!(digits.len() % 2, 0);
        digits
            .chunks_exact(2)
            .map(|pair| {
                let pair = std::str::from_utf8(pair).expect("ASCII hex");
                u8::from_str_radix(pair, 16).expect("valid hex fixture")
            })
            .collect()
    }

    #[test]
    fn indexes_synthetic_archive() {
        let archive = fixture();
        let index = parse_big_archive(&archive, BigLimits::default()).expect("valid BIG");

        assert_eq!(index.version(), BigVersion::BigF);
        assert_eq!(index.archive_size(), 69);
        assert_eq!(index.first_file_offset(), 62);
        assert_eq!(index.directory_trailer(), b"L231\0\0\0\0");
        assert_eq!(index.entries().len(), 2);
        assert_eq!(index.entries()[0].path().as_str(), "data/a.txt");
        assert_eq!(index.entries()[0].bytes(&archive), Some(b"new!".as_slice()));
        assert_eq!(index.entries()[1].path().as_str(), "data/z.bin");
        assert_eq!(
            index.entries()[1].bytes(&archive),
            Some([0, 1, 2].as_slice())
        );
    }

    #[test]
    fn rejects_every_truncated_prefix() {
        let archive = fixture();
        for length in 0..archive.len() {
            assert!(
                parse_big_archive(&archive[..length], BigLimits::default()).is_err(),
                "prefix of {length} bytes must fail"
            );
        }
    }

    #[test]
    fn rejects_unsupported_signature_and_excessive_count() {
        let mut archive = fixture();
        archive[..4].copy_from_slice(b"NOPE");
        assert!(matches!(
            parse_big_archive(&archive, BigLimits::default()),
            Err(BigError::UnsupportedSignature(_))
        ));

        let mut archive = fixture();
        archive[8..12].copy_from_slice(&3_u32.to_be_bytes());
        let limits = BigLimits {
            maximum_entries: 2,
            ..BigLimits::default()
        };
        assert!(parse_big_archive(&archive, limits).is_err());
    }

    #[test]
    fn rejects_entry_outside_payload() {
        let mut archive = fixture();
        archive[16..20].copy_from_slice(&0_u32.to_be_bytes());
        assert!(matches!(
            parse_big_archive(&archive, BigLimits::default()),
            Err(BigError::EntryOutsidePayload { entry: 0, .. })
        ));
    }

    #[test]
    fn accepts_retail_zero_length_offset_zero_marker() {
        let mut archive = fixture();
        archive[16..20].copy_from_slice(&0_u32.to_be_bytes());
        archive[20..24].copy_from_slice(&0_u32.to_be_bytes());
        let index = parse_big_archive(&archive, BigLimits::default()).expect("valid marker");
        assert_eq!(index.entries()[0].bytes(&archive), Some([].as_slice()));
    }
}
