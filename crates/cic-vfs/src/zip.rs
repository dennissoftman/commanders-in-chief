//! Bounded zip container indexing.
//!
//! Indexing walks the **central directory**, never the local file headers. The two disagree in
//! practice — streamed writers leave the local header's sizes zero and defer them to a data
//! descriptor — and the central directory is the authority. Reading local headers instead is the
//! single most common way a hand-written zip reader mis-indexes real archives.
//!
//! Two zip features are deliberately unsupported, and both fail loudly rather than silently:
//! encryption (there is no use for it in game content, and a wrong guess would hand the caller
//! ciphertext as if it were data) and Zip64 (its 64-bit offsets need a different end-of-directory
//! record; a Zip64 archive is refused rather than truncated to 32 bits).

use cic_core::BinaryReader;

use crate::archive::{ArchiveError, ArchiveIndex, ArchiveLimits, BoundedEntries, Compression};

const ARCHIVE: &str = "zip";

const END_OF_CENTRAL_DIRECTORY: u32 = 0x0605_4b50;
const CENTRAL_FILE_HEADER: u32 = 0x0201_4b50;
const LOCAL_FILE_HEADER: u32 = 0x0403_4b50;

/// Fixed byte length of an end-of-central-directory record with no comment.
const END_RECORD_LENGTH: usize = 22;
/// A zip comment length is a `u16`, so the record can start at most this far from the end.
const MAXIMUM_COMMENT: usize = u16::MAX as usize;
/// Fixed byte length of a central directory file header, before its variable-length fields.
const CENTRAL_HEADER_LENGTH: usize = 46;
/// Fixed byte length of a local file header, before its variable-length fields.
const LOCAL_HEADER_LENGTH: usize = 30;

/// The `u16` compression methods this reader implements.
const METHOD_STORED: u16 = 0;
const METHOD_DEFLATE: u16 = 8;

/// Bit 0 of the general-purpose flags marks an encrypted member.
const FLAG_ENCRYPTED: u16 = 1 << 0;

/// A `u32` size or offset of exactly this value means "see the Zip64 extra field".
const ZIP64_SENTINEL: u32 = u32::MAX;

/// Indexes a zip archive's central directory.
///
/// The returned offsets point at member *payloads*, with each local file header already skipped, so
/// a caller can read a member without parsing anything further.
///
/// # Errors
///
/// Returns a structured [`ArchiveError`] when the end-of-central-directory record is absent, a
/// header signature is wrong, a member is encrypted or Zip64, a compression method is unsupported,
/// a payload range leaves the archive, or an [`ArchiveLimits`] bound is exceeded.
pub fn parse_zip_archive(
    bytes: &[u8],
    limits: ArchiveLimits,
) -> Result<ArchiveIndex, ArchiveError> {
    let archive_size = bytes.len();
    let end_offset = locate_end_record(bytes)?;

    let mut reader = BinaryReader::new(bytes, "zip end of central directory");
    reader.seek(end_offset).map_err(binary)?;
    // Signature, then disk numbers this reader does not split across.
    let _signature = reader.read_u32_le().map_err(binary)?;
    let _disk_number = reader.read_u16_le().map_err(binary)?;
    let _directory_disk = reader.read_u16_le().map_err(binary)?;
    let _entries_this_disk = reader.read_u16_le().map_err(binary)?;
    let declared_entries = reader.read_u16_le().map_err(binary)? as usize;
    let directory_size = reader.read_u32_le().map_err(binary)? as usize;
    let directory_offset = reader.read_u32_le().map_err(binary)? as usize;

    if directory_size == ZIP64_SENTINEL as usize || directory_offset == ZIP64_SENTINEL as usize {
        return Err(ArchiveError::Signature {
            archive: ARCHIVE,
            expected: "a non-Zip64 central directory (Zip64 is unsupported)",
        });
    }

    let directory_end =
        directory_offset
            .checked_add(directory_size)
            .ok_or(ArchiveError::EntryRange {
                archive: ARCHIVE,
                entry: 0,
                offset: directory_offset,
                size: directory_size,
            })?;
    if directory_end > archive_size {
        return Err(ArchiveError::EntryOutsideArchive {
            archive: ARCHIVE,
            entry: 0,
            end: directory_end,
            archive_size,
        });
    }
    if declared_entries > limits.maximum_entries {
        return Err(ArchiveError::LimitExceeded {
            archive: ARCHIVE,
            what: "declared entry count",
            actual: declared_entries,
            maximum: limits.maximum_entries,
        });
    }

    let mut directory = BinaryReader::new(
        bytes
            .get(directory_offset..directory_end)
            .ok_or(ArchiveError::EntryOutsideArchive {
                archive: ARCHIVE,
                entry: 0,
                end: directory_end,
                archive_size,
            })?,
        "zip central directory",
    );

    let mut entries = BoundedEntries::new(limits, ARCHIVE);
    for index in 0..declared_entries {
        let record = read_central_record(&mut directory, index)?;

        // A trailing `/` is how zip stores a directory marker. It carries no payload and would
        // normalize to a path that shadows real members, so it is skipped rather than refused.
        if record.name.last() == Some(&b'/') {
            continue;
        }

        let compression = match record.method {
            METHOD_STORED => Compression::Stored,
            METHOD_DEFLATE => Compression::Deflate,
            _ => {
                return Err(ArchiveError::UnsupportedCompression {
                    archive: ARCHIVE,
                    entry: index,
                    method: record.method,
                });
            }
        };

        let payload_offset = payload_offset(
            bytes,
            record.local_header_offset as usize,
            index,
            archive_size,
        )?;
        entries.push(
            record.name,
            payload_offset,
            record.compressed_size as usize,
            record.uncompressed_size as usize,
            compression,
            archive_size,
        )?;
    }

    Ok(entries.finish())
}

/// The central-directory fields this reader acts on.
///
/// The omitted fields -- version, timestamps, CRC, disk number, attributes -- are read and dropped
/// so the cursor advances correctly, but nothing here depends on them.
struct CentralRecord<'a> {
    name: &'a [u8],
    compressed_size: u32,
    uncompressed_size: u32,
    local_header_offset: u32,
    method: u16,
}

/// Reads one central-directory file header, refusing encrypted and Zip64 members.
fn read_central_record<'a>(
    directory: &mut BinaryReader<'a>,
    index: usize,
) -> Result<CentralRecord<'a>, ArchiveError> {
    // A directory that ends early is a malformed archive, not an empty tail: the record count came
    // from the end record and must be honoured or refused.
    if directory.remaining() < CENTRAL_HEADER_LENGTH {
        return Err(ArchiveError::Signature {
            archive: ARCHIVE,
            expected: "a central directory file header for every declared entry",
        });
    }
    let signature = directory.read_u32_le().map_err(binary)?;
    if signature != CENTRAL_FILE_HEADER {
        return Err(ArchiveError::Signature {
            archive: ARCHIVE,
            expected: "a central directory file header signature",
        });
    }
    let _version_made_by = directory.read_u16_le().map_err(binary)?;
    let _version_needed = directory.read_u16_le().map_err(binary)?;
    let flags = directory.read_u16_le().map_err(binary)?;
    let method = directory.read_u16_le().map_err(binary)?;
    let _modified_time = directory.read_u16_le().map_err(binary)?;
    let _modified_date = directory.read_u16_le().map_err(binary)?;
    let _crc32 = directory.read_u32_le().map_err(binary)?;
    let compressed_size = directory.read_u32_le().map_err(binary)?;
    let uncompressed_size = directory.read_u32_le().map_err(binary)?;
    let name_length = directory.read_u16_le().map_err(binary)? as usize;
    let extra_length = directory.read_u16_le().map_err(binary)? as usize;
    let comment_length = directory.read_u16_le().map_err(binary)? as usize;
    let _disk_start = directory.read_u16_le().map_err(binary)?;
    let _internal_attributes = directory.read_u16_le().map_err(binary)?;
    let _external_attributes = directory.read_u32_le().map_err(binary)?;
    let local_header_offset = directory.read_u32_le().map_err(binary)?;

    let name = directory.read_exact(name_length).map_err(binary)?;
    directory.skip(extra_length).map_err(binary)?;
    directory.skip(comment_length).map_err(binary)?;

    if flags & FLAG_ENCRYPTED != 0 {
        return Err(ArchiveError::UnsupportedCompression {
            archive: ARCHIVE,
            entry: index,
            method,
        });
    }
    if compressed_size == ZIP64_SENTINEL
        || uncompressed_size == ZIP64_SENTINEL
        || local_header_offset == ZIP64_SENTINEL
    {
        return Err(ArchiveError::Signature {
            archive: ARCHIVE,
            expected: "non-Zip64 entry sizes and offsets (Zip64 is unsupported)",
        });
    }

    Ok(CentralRecord {
        name,
        compressed_size,
        uncompressed_size,
        local_header_offset,
        method,
    })
}

/// Resolves a member's payload offset by stepping over its local file header.
///
/// The local header's *sizes* are untrustworthy on a streamed archive, but its name and extra
/// lengths are what physically precede the payload, so they are the only fields read here.
fn payload_offset(
    bytes: &[u8],
    local_header_offset: usize,
    index: usize,
    archive_size: usize,
) -> Result<usize, ArchiveError> {
    let mut local = BinaryReader::new(bytes, "zip local file header");
    local.seek(local_header_offset).map_err(binary)?;
    if local.remaining() < LOCAL_HEADER_LENGTH {
        return Err(ArchiveError::EntryOutsideArchive {
            archive: ARCHIVE,
            entry: index,
            end: local_header_offset + LOCAL_HEADER_LENGTH,
            archive_size,
        });
    }
    let signature = local.read_u32_le().map_err(binary)?;
    if signature != LOCAL_FILE_HEADER {
        return Err(ArchiveError::Signature {
            archive: ARCHIVE,
            expected: "a local file header signature",
        });
    }
    local.skip(22).map_err(binary)?;
    let name_length = local.read_u16_le().map_err(binary)? as usize;
    let extra_length = local.read_u16_le().map_err(binary)? as usize;
    local.skip(name_length).map_err(binary)?;
    local.skip(extra_length).map_err(binary)?;
    Ok(local.position())
}

/// Scans backward for the end-of-central-directory signature.
///
/// The record is last but variable in position, because a trailing comment of up to 64 KiB may
/// follow it. Scanning backward finds the *last* candidate, which is the correct one when an
/// archive's own comment happens to contain the signature bytes.
fn locate_end_record(bytes: &[u8]) -> Result<usize, ArchiveError> {
    if bytes.len() < END_RECORD_LENGTH {
        return Err(ArchiveError::Signature {
            archive: ARCHIVE,
            expected: "an end of central directory record",
        });
    }
    let highest_start = bytes.len() - END_RECORD_LENGTH;
    let lowest_start = highest_start.saturating_sub(MAXIMUM_COMMENT);
    for start in (lowest_start..=highest_start).rev() {
        let candidate = u32::from_le_bytes([
            bytes[start],
            bytes[start + 1],
            bytes[start + 2],
            bytes[start + 3],
        ]);
        if candidate == END_OF_CENTRAL_DIRECTORY {
            // The comment length must account for exactly the bytes that follow the record, or
            // this is signature bytes inside some other field rather than the real record.
            let comment_length =
                u16::from_le_bytes([bytes[start + 20], bytes[start + 21]]) as usize;
            if start + END_RECORD_LENGTH + comment_length == bytes.len() {
                return Ok(start);
            }
        }
    }
    Err(ArchiveError::Signature {
        archive: ARCHIVE,
        expected: "an end of central directory record",
    })
}

fn binary(error: cic_core::BinaryError) -> ArchiveError {
    ArchiveError::Binary {
        archive: ARCHIVE,
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::{ArchiveLimits, parse_zip_archive};
    use crate::archive::Compression;
    use crate::testing::{ZipBuilder, deflate};

    #[test]
    fn indexes_a_stored_member_and_points_at_its_payload() {
        let payload = b"heightfield bytes".to_vec();
        let archive = ZipBuilder::new()
            .stored("terrain/heightfield.bin", &payload)
            .finish();
        let index = parse_zip_archive(&archive, ArchiveLimits::default()).expect("index");
        assert_eq!(index.len(), 1);
        let entry = &index.entries()[0];
        assert_eq!(entry.path().as_str(), "terrain/heightfield.bin");
        assert_eq!(entry.compression(), Compression::Stored);
        assert_eq!(entry.uncompressed_size(), payload.len());
        assert_eq!(
            &archive[entry.offset()..entry.end()],
            payload.as_slice(),
            "offset must point at the payload, not the local header"
        );
    }

    #[test]
    fn indexes_a_deflated_member() {
        let payload = b"a".repeat(4_096);
        let compressed = deflate(&payload);
        assert!(compressed.len() < payload.len(), "fixture must compress");
        let archive = ZipBuilder::new()
            .deflated("map.json", &compressed, payload.len())
            .finish();
        let index = parse_zip_archive(&archive, ArchiveLimits::default()).expect("index");
        let entry = &index.entries()[0];
        assert_eq!(entry.compression(), Compression::Deflate);
        assert_eq!(entry.compressed_size(), compressed.len());
        assert_eq!(entry.uncompressed_size(), payload.len());
    }

    #[test]
    fn normalizes_paths_and_folds_case() {
        let archive = ZipBuilder::new()
            .stored("Terrain\\Alpine\\Height.BIN", b"x")
            .finish();
        let index = parse_zip_archive(&archive, ArchiveLimits::default()).expect("index");
        assert_eq!(
            index.entries()[0].path().as_str(),
            "terrain/alpine/height.bin"
        );
    }

    #[test]
    fn skips_directory_markers() {
        let archive = ZipBuilder::new()
            .stored("terrain/", b"")
            .stored("terrain/height.bin", b"x")
            .finish();
        let index = parse_zip_archive(&archive, ArchiveLimits::default()).expect("index");
        assert_eq!(index.len(), 1, "the directory marker carries no payload");
        assert_eq!(index.entries()[0].path().as_str(), "terrain/height.bin");
    }

    #[test]
    fn rejects_parent_traversal_in_a_member_name() {
        let archive = ZipBuilder::new().stored("../../etc/passwd", b"x").finish();
        let error = parse_zip_archive(&archive, ArchiveLimits::default())
            .expect_err("traversal must be refused");
        assert!(
            matches!(error, super::ArchiveError::Path { .. }),
            "got {error:?}"
        );
    }

    #[test]
    fn rejects_an_archive_with_no_end_record() {
        let error = parse_zip_archive(b"not a zip at all", ArchiveLimits::default())
            .expect_err("must refuse");
        assert!(
            matches!(error, super::ArchiveError::Signature { .. }),
            "got {error:?}"
        );
    }

    #[test]
    fn refuses_a_declared_uncompressed_size_past_the_entry_limit() {
        // The zip-bomb case: a tiny payload claiming an enormous expansion. Refusing at index time
        // means no caller can be handed the allocation.
        let archive = ZipBuilder::new()
            .deflated("bomb", &[0x03, 0x00], 900_000_000)
            .finish();
        let limits = ArchiveLimits {
            maximum_entry_bytes: 1_024,
            ..ArchiveLimits::default()
        };
        let error = parse_zip_archive(&archive, limits).expect_err("must refuse");
        assert!(
            matches!(
                error,
                super::ArchiveError::LimitExceeded {
                    what: "entry uncompressed size",
                    ..
                }
            ),
            "got {error:?}"
        );
    }

    #[test]
    fn refuses_a_summed_uncompressed_size_past_the_total_limit() {
        let archive = ZipBuilder::new()
            .stored("a", &[0u8; 600])
            .stored("b", &[0u8; 600])
            .finish();
        let limits = ArchiveLimits {
            maximum_total_bytes: 1_000,
            ..ArchiveLimits::default()
        };
        let error = parse_zip_archive(&archive, limits).expect_err("must refuse");
        assert!(
            matches!(
                error,
                super::ArchiveError::LimitExceeded {
                    what: "total uncompressed size",
                    ..
                }
            ),
            "got {error:?}"
        );
    }

    #[test]
    fn refuses_an_unsupported_compression_method() {
        let archive = ZipBuilder::new().with_method("x", b"y", 99, 1).finish();
        let error = parse_zip_archive(&archive, ArchiveLimits::default()).expect_err("must refuse");
        assert!(
            matches!(
                error,
                super::ArchiveError::UnsupportedCompression { method: 99, .. }
            ),
            "got {error:?}"
        );
    }

    #[test]
    fn refuses_an_encrypted_member() {
        let archive = ZipBuilder::new().encrypted("secret", b"y").finish();
        let error = parse_zip_archive(&archive, ArchiveLimits::default()).expect_err("must refuse");
        assert!(
            matches!(error, super::ArchiveError::UnsupportedCompression { .. }),
            "got {error:?}"
        );
    }

    #[test]
    fn finds_the_end_record_behind_a_trailing_comment() {
        let archive = ZipBuilder::new()
            .stored("a.bin", b"payload")
            .comment(b"a trailing archive comment")
            .finish();
        let index = parse_zip_archive(&archive, ArchiveLimits::default()).expect("index");
        assert_eq!(index.len(), 1);
    }

    #[test]
    fn indexes_many_members_in_directory_order() {
        let mut builder = ZipBuilder::new();
        for index in 0u8..64 {
            builder = builder.stored(&format!("chunk/{index:03}.bin"), &[index; 8]);
        }
        let archive = builder.finish();
        let index = parse_zip_archive(&archive, ArchiveLimits::default()).expect("index");
        assert_eq!(index.len(), 64);
        for (position, entry) in index.entries().iter().enumerate() {
            assert_eq!(entry.path().as_str(), format!("chunk/{position:03}.bin"));
        }
    }
}
