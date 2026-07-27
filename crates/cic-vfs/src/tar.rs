//! Bounded tar container indexing, with optional gzip framing.
//!
//! Tar is a flat sequence of 512-byte headers, each followed by its payload padded to the next
//! 512-byte boundary. There is no central directory, so indexing walks the whole archive — which is
//! cheap, because only headers are read and payloads are skipped by arithmetic.
//!
//! Only regular files are indexed. Directories carry no payload, and links, devices, and FIFOs have
//! no meaning as game resources — a link in particular is a path-escape vector, so they are skipped
//! rather than resolved. The `ustar` prefix field is honoured so paths longer than 100 bytes work;
//! GNU long-name extensions are not, and such a member is skipped rather than silently truncated.

use cic_core::BinaryReader;

use crate::archive::{ArchiveError, ArchiveIndex, ArchiveLimits, BoundedEntries, Compression};

const ARCHIVE: &str = "tar";

/// Every tar header and every payload is padded to this boundary.
const BLOCK: usize = 512;

// Field offsets within a 512-byte header, from the ustar layout.
const NAME: (usize, usize) = (0, 100);
const SIZE: (usize, usize) = (124, 12);
const CHECKSUM: (usize, usize) = (148, 8);
const TYPE_FLAG: usize = 156;
const MAGIC: (usize, usize) = (257, 6);
const PREFIX: (usize, usize) = (345, 155);

/// `ustar\0`, present on POSIX archives. GNU tar writes `ustar  \0` instead, so only the first five
/// bytes are compared.
const USTAR: &[u8] = b"ustar";

/// Type flags that denote a regular file. Historic archives use `\0`; POSIX uses `'0'`.
const TYPE_REGULAR: &[u8] = b"0\0";

/// Indexes a tar archive.
///
/// # Errors
///
/// Returns a structured [`ArchiveError`] when a header checksum fails, a size field is not valid
/// octal, a payload leaves the archive, or an [`ArchiveLimits`] bound is exceeded.
pub fn parse_tar_archive(
    bytes: &[u8],
    limits: ArchiveLimits,
) -> Result<ArchiveIndex, ArchiveError> {
    let archive_size = bytes.len();
    let mut entries = BoundedEntries::new(limits, ARCHIVE);
    let mut reader = BinaryReader::new(bytes, "tar");
    let mut index = 0usize;

    while reader.remaining() >= BLOCK {
        let header_offset = reader.position();
        let header = reader.read_exact(BLOCK).map_err(binary)?;

        // Two consecutive zero blocks end the archive. One is enough to stop on: nothing valid
        // follows a header with a zero name and zero checksum.
        if header.iter().all(|byte| *byte == 0) {
            break;
        }

        if !valid_checksum(header) {
            return Err(ArchiveError::Signature {
                archive: ARCHIVE,
                expected: "a header with a matching octal checksum",
            });
        }

        let size = read_octal(field(header, SIZE), index)?;
        let size = usize::try_from(size).map_err(|_| ArchiveError::LimitExceeded {
            archive: ARCHIVE,
            what: "entry size",
            actual: usize::MAX,
            maximum: limits.maximum_entry_bytes,
        })?;

        let payload_offset = header_offset + BLOCK;
        // Payloads are padded up to the next block boundary, and the padding is part of what the
        // next header's position depends on -- so this rounding is structural, not cosmetic.
        let padded = size
            .checked_next_multiple_of(BLOCK)
            .ok_or(ArchiveError::EntryRange {
                archive: ARCHIVE,
                entry: index,
                offset: payload_offset,
                size,
            })?;

        let type_flag = header[TYPE_FLAG];
        let is_regular = TYPE_REGULAR.contains(&type_flag);
        if is_regular {
            let name = assemble_name(header);
            // A ustar prefix is only meaningful on an archive that declares ustar; a historic
            // archive's bytes at that offset are not a path component.
            entries.push(
                &name,
                payload_offset,
                size,
                size,
                Compression::Stored,
                archive_size,
            )?;
        }

        reader.skip(padded).map_err(binary)?;
        index += 1;
    }

    Ok(entries.finish())
}

/// Decompresses a gzip stream, then indexes the tar inside it.
///
/// The whole stream must be decompressed to be indexed, because gzip is not seekable — so this
/// returns the decompressed bytes alongside the index, and the caller mounts against those rather
/// than against the original file.
///
/// # Errors
///
/// Returns [`ArchiveError::Signature`] when the gzip stream is malformed or expands past
/// `limits.maximum_total_bytes`, then any error [`parse_tar_archive`] can produce.
pub fn parse_tar_gz_archive(
    bytes: &[u8],
    limits: ArchiveLimits,
) -> Result<(Vec<u8>, ArchiveIndex), ArchiveError> {
    use std::io::Read;

    let mut decoded = Vec::new();
    // Bounded before the read rather than after, so a gzip bomb cannot allocate its way to the
    // limit check. `take` makes the reader itself refuse to produce more.
    let ceiling = limits.maximum_total_bytes;
    let mut stream =
        flate2::read::GzDecoder::new(bytes).take(u64::try_from(ceiling).unwrap_or(u64::MAX) + 1);
    stream
        .read_to_end(&mut decoded)
        .map_err(|_| ArchiveError::Signature {
            archive: "tar.gz",
            expected: "a well-formed gzip stream",
        })?;
    if decoded.len() > ceiling {
        return Err(ArchiveError::LimitExceeded {
            archive: "tar.gz",
            what: "decompressed stream size",
            actual: decoded.len(),
            maximum: ceiling,
        });
    }
    let index = parse_tar_archive(&decoded, limits)?;
    Ok((decoded, index))
}

fn field(header: &[u8], (offset, length): (usize, usize)) -> &[u8] {
    &header[offset..offset + length]
}

/// Joins the `ustar` prefix field to the name field, when the archive declares `ustar`.
fn assemble_name(header: &[u8]) -> Vec<u8> {
    let name = trim_nul(field(header, NAME));
    if !field(header, MAGIC).starts_with(USTAR) {
        return name.to_vec();
    }
    let prefix = trim_nul(field(header, PREFIX));
    if prefix.is_empty() {
        return name.to_vec();
    }
    let mut joined = Vec::with_capacity(prefix.len() + 1 + name.len());
    joined.extend_from_slice(prefix);
    joined.push(b'/');
    joined.extend_from_slice(name);
    joined
}

fn trim_nul(bytes: &[u8]) -> &[u8] {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    &bytes[..end]
}

/// Parses a space- or NUL-padded octal field.
fn read_octal(bytes: &[u8], entry: usize) -> Result<u64, ArchiveError> {
    let text = trim_nul(bytes);
    let text = text.strip_suffix(b" ").unwrap_or(text);
    let trimmed: Vec<u8> = text
        .iter()
        .copied()
        .skip_while(|byte| *byte == b' ')
        .collect();
    if trimmed.is_empty() {
        return Ok(0);
    }
    let mut value = 0u64;
    for byte in trimmed {
        if !(b'0'..=b'7').contains(&byte) {
            return Err(ArchiveError::Signature {
                archive: ARCHIVE,
                expected: "octal digits in a numeric header field",
            });
        }
        value = value
            .checked_mul(8)
            .and_then(|shifted| shifted.checked_add(u64::from(byte - b'0')))
            .ok_or(ArchiveError::EntryRange {
                archive: ARCHIVE,
                entry,
                offset: 0,
                size: usize::MAX,
            })?;
    }
    Ok(value)
}

/// Verifies the header checksum, which is the unsigned sum of every byte with the checksum field
/// itself read as spaces.
fn valid_checksum(header: &[u8]) -> bool {
    let Ok(declared) = read_octal(field(header, CHECKSUM), 0) else {
        return false;
    };
    let (start, length) = CHECKSUM;
    let sum: u64 = header
        .iter()
        .enumerate()
        .map(|(offset, byte)| {
            if (start..start + length).contains(&offset) {
                u64::from(b' ')
            } else {
                u64::from(*byte)
            }
        })
        .sum();
    sum == declared
}

fn binary(error: cic_core::BinaryError) -> ArchiveError {
    ArchiveError::Binary {
        archive: ARCHIVE,
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::{ArchiveLimits, parse_tar_archive, parse_tar_gz_archive};
    use crate::testing::{TarBuilder, gzip};

    #[test]
    fn indexes_a_regular_file_and_points_at_its_payload() {
        let payload = b"scenario json".to_vec();
        let archive = TarBuilder::new().file("map.json", &payload).finish();
        let index = parse_tar_archive(&archive, ArchiveLimits::default()).expect("index");
        assert_eq!(index.len(), 1);
        let entry = &index.entries()[0];
        assert_eq!(entry.path().as_str(), "map.json");
        assert_eq!(entry.uncompressed_size(), payload.len());
        assert_eq!(&archive[entry.offset()..entry.end()], payload.as_slice());
    }

    #[test]
    fn indexes_several_files_across_block_boundaries() {
        // Sizes chosen to straddle the 512-byte padding: if padding is mishandled, the second and
        // third headers land at the wrong offsets and the checksum check fires.
        let archive = TarBuilder::new()
            .file("a.bin", &[1u8; 511])
            .file("b.bin", &[2u8; 512])
            .file("c.bin", &[3u8; 513])
            .finish();
        let index = parse_tar_archive(&archive, ArchiveLimits::default()).expect("index");
        assert_eq!(index.len(), 3);
        for (entry, expected) in index.entries().iter().zip([511, 512, 513]) {
            assert_eq!(entry.uncompressed_size(), expected);
            assert_eq!(
                &archive[entry.offset()..entry.end()],
                &vec![
                    match expected {
                        511 => 1u8,
                        512 => 2u8,
                        _ => 3u8,
                    };
                    expected
                ]
            );
        }
    }

    #[test]
    fn skips_directories_and_symbolic_links() {
        let archive = TarBuilder::new()
            .directory("terrain")
            .symlink("shortcut", "map.json")
            .file("terrain/height.bin", b"x")
            .finish();
        let index = parse_tar_archive(&archive, ArchiveLimits::default()).expect("index");
        assert_eq!(index.len(), 1, "only the regular file is a resource");
        assert_eq!(index.entries()[0].path().as_str(), "terrain/height.bin");
    }

    #[test]
    fn honours_the_ustar_prefix_for_long_paths() {
        let long = "a".repeat(120);
        let path = format!("{long}/height.bin");
        let archive = TarBuilder::new().file(&path, b"x").finish();
        let index = parse_tar_archive(&archive, ArchiveLimits::default()).expect("index");
        assert_eq!(index.entries()[0].path().as_str(), path.to_lowercase());
    }

    #[test]
    fn stops_at_the_terminating_zero_block() {
        let archive = TarBuilder::new().file("a.bin", b"x").finish();
        let mut padded = archive.clone();
        padded.extend_from_slice(&[0u8; 1024]);
        let index = parse_tar_archive(&padded, ArchiveLimits::default()).expect("index");
        assert_eq!(index.len(), 1);
    }

    #[test]
    fn rejects_a_corrupted_header_checksum() {
        let mut archive = TarBuilder::new().file("a.bin", b"x").finish();
        archive[0] = b'Z';
        let error = parse_tar_archive(&archive, ArchiveLimits::default()).expect_err("must refuse");
        assert!(
            matches!(error, super::ArchiveError::Signature { .. }),
            "got {error:?}"
        );
    }

    #[test]
    fn rejects_a_payload_that_leaves_the_archive() {
        // A header declaring more payload than the archive contains.
        let archive = TarBuilder::new()
            .file_with_size("a.bin", b"x", 4096)
            .finish();
        let error = parse_tar_archive(&archive, ArchiveLimits::default()).expect_err("must refuse");
        assert!(
            matches!(
                error,
                super::ArchiveError::EntryOutsideArchive { .. }
                    | super::ArchiveError::Binary { .. }
            ),
            "got {error:?}"
        );
    }

    #[test]
    fn rejects_parent_traversal_in_a_member_name() {
        let archive = TarBuilder::new().file("../escape", b"x").finish();
        let error = parse_tar_archive(&archive, ArchiveLimits::default()).expect_err("must refuse");
        assert!(
            matches!(error, super::ArchiveError::Path { .. }),
            "got {error:?}"
        );
    }

    #[test]
    fn indexes_a_gzip_framed_tar() {
        let archive = TarBuilder::new()
            .file("map.json", b"{}")
            .file("terrain/height.bin", &[7u8; 300])
            .finish();
        let compressed = gzip(&archive);
        let (decoded, index) =
            parse_tar_gz_archive(&compressed, ArchiveLimits::default()).expect("index");
        assert_eq!(decoded, archive, "decompression must round-trip");
        assert_eq!(index.len(), 2);
        assert_eq!(index.entries()[1].uncompressed_size(), 300);
    }

    #[test]
    fn refuses_a_gzip_stream_that_expands_past_the_total_limit() {
        let archive = TarBuilder::new().file("a.bin", &[0u8; 8192]).finish();
        let compressed = gzip(&archive);
        let limits = ArchiveLimits {
            maximum_total_bytes: 1_024,
            ..ArchiveLimits::default()
        };
        let error = parse_tar_gz_archive(&compressed, limits).expect_err("must refuse");
        assert!(
            matches!(error, super::ArchiveError::LimitExceeded { .. }),
            "got {error:?}"
        );
    }
}
