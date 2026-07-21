//! CSF localization decoding.
//!
//! Format facts and clean-room implementation provenance are recorded in
//! `docs/formats/csf.md` and `docs/provenance/csf.md`.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use cic_core::{BinaryError, BinaryReader};

const FILE_TAG: [u8; 4] = *b" FSC";
const LABEL_TAG: [u8; 4] = *b" LBL";
const STRING_TAG: [u8; 4] = *b" RTS";
const STRING_WITH_WAVE_TAG: [u8; 4] = *b"WRTS";

/// Explicit resource limits for one CSF input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CsfLimits {
    /// Maximum complete input length.
    pub maximum_file_bytes: usize,
    /// Maximum declared label count.
    pub maximum_labels: usize,
    /// Maximum declared total string count.
    pub maximum_strings: usize,
    /// Maximum string variants attached to one label.
    pub maximum_variants_per_label: usize,
    /// Maximum label-name length in bytes.
    pub maximum_label_bytes: usize,
    /// Maximum text length in UTF-16 code units.
    pub maximum_text_units: usize,
    /// Maximum wave-name length in bytes.
    pub maximum_wave_bytes: usize,
}

impl Default for CsfLimits {
    fn default() -> Self {
        Self {
            maximum_file_bytes: 64 * 1024 * 1024,
            maximum_labels: 100_000,
            maximum_strings: 1_000_000,
            maximum_variants_per_label: 65_536,
            maximum_label_bytes: 4_096,
            maximum_text_units: 1_048_576,
            maximum_wave_bytes: 4_096,
        }
    }
}

/// The six fixed-width CSF header fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CsfHeader {
    version: u32,
    label_count: u32,
    string_count: u32,
    reserved: u32,
    language_id: u32,
}

impl CsfHeader {
    /// Returns the file format version.
    #[must_use]
    pub const fn version(self) -> u32 {
        self.version
    }

    /// Returns the declared label-record count.
    #[must_use]
    pub const fn label_count(self) -> u32 {
        self.label_count
    }

    /// Returns the declared total string-record count.
    #[must_use]
    pub const fn string_count(self) -> u32 {
        self.string_count
    }

    /// Returns the reserved header field without interpreting it.
    #[must_use]
    pub const fn reserved(self) -> u32 {
        self.reserved
    }

    /// Returns the raw language identifier.
    ///
    /// The original loader uses this field for version 2 and later and defaults older
    /// versions to US English.
    #[must_use]
    pub const fn language_id(self) -> u32 {
        self.language_id
    }
}

/// One decoded CSF file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsfFile {
    header: CsfHeader,
    labels: Vec<CsfLabel>,
}

impl CsfFile {
    /// Returns the parsed header.
    #[must_use]
    pub const fn header(&self) -> CsfHeader {
        self.header
    }

    /// Returns labels in original file order.
    #[must_use]
    pub fn labels(&self) -> &[CsfLabel] {
        &self.labels
    }
}

/// One label and all of its string variants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsfLabel {
    name: Vec<u8>,
    strings: Vec<CsfString>,
}

impl CsfLabel {
    /// Returns the label's uninterpreted name bytes.
    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }

    /// Returns every string variant in original file order.
    #[must_use]
    pub fn strings(&self) -> &[CsfString] {
        &self.strings
    }
}

/// One decoded text variant with an optional raw wave name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsfString {
    text: String,
    wave_name: Option<Vec<u8>>,
}

impl CsfString {
    /// Returns decoded Unicode text with file whitespace preserved.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the optional uninterpreted wave-name bytes.
    #[must_use]
    pub fn wave_name_bytes(&self) -> Option<&[u8]> {
        self.wave_name.as_deref()
    }
}

/// A structured CSF decoding failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CsfError {
    /// A bounded binary read or resource limit failed.
    Binary(BinaryError),
    /// The fixed file tag was not ` FSC`.
    InvalidFileTag([u8; 4]),
    /// A label or string record had the wrong tag.
    InvalidRecordTag {
        /// Byte offset of the tag.
        offset: usize,
        /// Expected record kind.
        expected: &'static str,
        /// Actual four bytes.
        actual: [u8; 4],
    },
    /// Decoded code units were not well-formed UTF-16.
    InvalidUtf16 {
        /// Zero-based label index.
        label: usize,
        /// Zero-based string index within the label.
        string: usize,
    },
    /// Parsed record totals disagreed with the header.
    StringCountMismatch {
        /// Header declaration.
        declared: usize,
        /// Number represented by parsed label records.
        actual: usize,
    },
    /// Bytes remained after all declared labels were parsed.
    TrailingData {
        /// First unconsumed byte offset.
        offset: usize,
        /// Number of unconsumed bytes.
        length: usize,
    },
}

impl Display for CsfError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binary(error) => Display::fmt(error, formatter),
            Self::InvalidFileTag(actual) => {
                write!(formatter, "invalid CSF file tag {actual:02X?}")
            }
            Self::InvalidRecordTag {
                offset,
                expected,
                actual,
            } => write!(
                formatter,
                "invalid CSF record tag {actual:02X?} at offset {offset}; expected {expected}"
            ),
            Self::InvalidUtf16 { label, string } => write!(
                formatter,
                "invalid UTF-16 in CSF label {label}, string variant {string}"
            ),
            Self::StringCountMismatch { declared, actual } => write!(
                formatter,
                "CSF header declares {declared} strings but label records contain {actual}"
            ),
            Self::TrailingData { offset, length } => write!(
                formatter,
                "CSF contains {length} trailing bytes beginning at offset {offset}"
            ),
        }
    }
}

impl Error for CsfError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Binary(error) => Some(error),
            Self::InvalidFileTag(_)
            | Self::InvalidRecordTag { .. }
            | Self::InvalidUtf16 { .. }
            | Self::StringCountMismatch { .. }
            | Self::TrailingData { .. } => None,
        }
    }
}

impl From<BinaryError> for CsfError {
    fn from(error: BinaryError) -> Self {
        Self::Binary(error)
    }
}

/// Decodes one complete CSF byte region.
///
/// Labels, duplicate names, zero-string labels, and all variants remain in file order.
///
/// # Errors
///
/// Returns [`CsfError`] for truncation, invalid tags, malformed UTF-16, inconsistent
/// counts, trailing data, or any configured resource-limit excess.
pub fn parse_csf(
    bytes: &[u8],
    source: impl Into<String>,
    limits: CsfLimits,
) -> Result<CsfFile, CsfError> {
    enforce_limit("CSF file size", bytes.len(), limits.maximum_file_bytes)?;
    let mut reader = BinaryReader::new(bytes, source);

    let file_tag = read_tag(&mut reader)?;
    if file_tag != FILE_TAG {
        return Err(CsfError::InvalidFileTag(file_tag));
    }

    let header = CsfHeader {
        version: reader.read_u32_le()?,
        label_count: reader.read_u32_le()?,
        string_count: reader.read_u32_le()?,
        reserved: reader.read_u32_le()?,
        language_id: reader.read_u32_le()?,
    };
    let label_count = limited_u32(header.label_count, "CSF label count", limits.maximum_labels)?;
    let declared_string_count = limited_u32(
        header.string_count,
        "CSF string count",
        limits.maximum_strings,
    )?;

    let mut labels = Vec::with_capacity(label_count);
    let mut actual_string_count = 0_usize;
    for label_index in 0..label_count {
        let (label, following_string_count) = parse_label(
            &mut reader,
            label_index,
            actual_string_count,
            declared_string_count,
            limits,
        )?;
        actual_string_count = following_string_count;
        labels.push(label);
    }

    if actual_string_count != declared_string_count {
        return Err(CsfError::StringCountMismatch {
            declared: declared_string_count,
            actual: actual_string_count,
        });
    }
    if reader.remaining() != 0 {
        return Err(CsfError::TrailingData {
            offset: reader.position(),
            length: reader.remaining(),
        });
    }

    Ok(CsfFile { header, labels })
}

fn parse_label(
    reader: &mut BinaryReader<'_>,
    label_index: usize,
    preceding_string_count: usize,
    declared_string_count: usize,
    limits: CsfLimits,
) -> Result<(CsfLabel, usize), CsfError> {
    expect_tag(reader, LABEL_TAG, "label (` LBL`)")?;
    let variant_count = limited_u32(
        reader.read_u32_le()?,
        "CSF variants per label",
        limits.maximum_variants_per_label,
    )?;
    let following_string_count =
        preceding_string_count
            .checked_add(variant_count)
            .ok_or(BinaryError::LimitExceeded {
                what: "CSF total parsed string count",
                actual: usize::MAX,
                maximum: limits.maximum_strings,
            })?;
    enforce_limit(
        "CSF total parsed string count",
        following_string_count,
        limits.maximum_strings,
    )?;
    if following_string_count > declared_string_count {
        return Err(CsfError::StringCountMismatch {
            declared: declared_string_count,
            actual: following_string_count,
        });
    }
    let name_length = limited_u32(
        reader.read_u32_le()?,
        "CSF label byte length",
        limits.maximum_label_bytes,
    )?;
    let name = reader.read_exact(name_length)?.to_vec();
    let mut strings = Vec::with_capacity(variant_count);
    for string_index in 0..variant_count {
        strings.push(parse_string(reader, label_index, string_index, limits)?);
    }
    Ok((CsfLabel { name, strings }, following_string_count))
}

fn parse_string(
    reader: &mut BinaryReader<'_>,
    label_index: usize,
    string_index: usize,
    limits: CsfLimits,
) -> Result<CsfString, CsfError> {
    let tag_offset = reader.position();
    let tag = read_tag(reader)?;
    let has_wave = match tag {
        STRING_TAG => false,
        STRING_WITH_WAVE_TAG => true,
        _ => {
            return Err(CsfError::InvalidRecordTag {
                offset: tag_offset,
                expected: "string (` RTS`) or string-with-wave (`WRTS`)",
                actual: tag,
            });
        }
    };

    let unit_count = limited_u32(
        reader.read_u32_le()?,
        "CSF text UTF-16 unit count",
        limits.maximum_text_units,
    )?;
    let mut units = Vec::with_capacity(unit_count);
    for _ in 0..unit_count {
        units.push(reader.read_u16_le()? ^ u16::MAX);
    }
    let text = String::from_utf16(&units).map_err(|_| CsfError::InvalidUtf16 {
        label: label_index,
        string: string_index,
    })?;

    let wave_name = if has_wave {
        let wave_length = limited_u32(
            reader.read_u32_le()?,
            "CSF wave-name byte length",
            limits.maximum_wave_bytes,
        )?;
        Some(reader.read_exact(wave_length)?.to_vec())
    } else {
        None
    };
    Ok(CsfString { text, wave_name })
}

fn read_tag(reader: &mut BinaryReader<'_>) -> Result<[u8; 4], BinaryError> {
    let bytes = reader.read_exact(4)?;
    let mut tag = [0_u8; 4];
    tag.copy_from_slice(bytes);
    Ok(tag)
}

fn expect_tag(
    reader: &mut BinaryReader<'_>,
    expected_tag: [u8; 4],
    expected_name: &'static str,
) -> Result<(), CsfError> {
    let offset = reader.position();
    let actual = read_tag(reader)?;
    if actual == expected_tag {
        Ok(())
    } else {
        Err(CsfError::InvalidRecordTag {
            offset,
            expected: expected_name,
            actual,
        })
    }
}

fn limited_u32(value: u32, what: &'static str, maximum: usize) -> Result<usize, BinaryError> {
    let value = usize::try_from(value).map_err(|_| BinaryError::LimitExceeded {
        what,
        actual: usize::MAX,
        maximum,
    })?;
    enforce_limit(what, value, maximum)?;
    Ok(value)
}

fn enforce_limit(what: &'static str, actual: usize, maximum: usize) -> Result<(), BinaryError> {
    if actual > maximum {
        Err(BinaryError::LimitExceeded {
            what,
            actual,
            maximum,
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{CsfError, CsfLimits, parse_csf};

    fn fixture() -> Vec<u8> {
        let hex = include_str!("../tests/fixtures/minimal.csf.hex");
        let digits = hex
            .bytes()
            .filter(u8::is_ascii_hexdigit)
            .collect::<Vec<_>>();
        digits
            .chunks_exact(2)
            .map(|pair| {
                let pair = std::str::from_utf8(pair).expect("ASCII fixture");
                u8::from_str_radix(pair, 16).expect("valid hex fixture")
            })
            .collect()
    }

    #[test]
    fn decodes_plain_wave_and_zero_string_labels() {
        let parsed =
            parse_csf(&fixture(), "minimal.csf", CsfLimits::default()).expect("valid fixture");

        assert_eq!(parsed.header().version(), 3);
        assert_eq!(parsed.header().label_count(), 3);
        assert_eq!(parsed.header().string_count(), 2);
        assert_eq!(parsed.labels()[0].name_bytes(), b"GUI:HELLO");
        assert_eq!(parsed.labels()[0].strings()[0].text(), "Hello");
        assert_eq!(parsed.labels()[0].strings()[0].wave_name_bytes(), None);
        assert_eq!(parsed.labels()[1].name_bytes(), b"SPEECH:READY");
        assert_eq!(parsed.labels()[1].strings()[0].text(), "Ready");
        assert_eq!(
            parsed.labels()[1].strings()[0].wave_name_bytes(),
            Some(b"ready.wav".as_slice())
        );
        assert_eq!(parsed.labels()[2].name_bytes(), b"TOOLTIP:EMPTY");
        assert!(parsed.labels()[2].strings().is_empty());
    }

    #[test]
    fn every_truncated_prefix_returns_an_error() {
        let bytes = fixture();
        for length in 0..bytes.len() {
            assert!(
                parse_csf(&bytes[..length], "truncated.csf", CsfLimits::default()).is_err(),
                "prefix of {length} bytes unexpectedly parsed"
            );
        }
    }

    #[test]
    fn rejects_invalid_tags_and_unpaired_surrogates() {
        let mut bad_file_tag = fixture();
        bad_file_tag[0] = b'X';
        assert!(matches!(
            parse_csf(&bad_file_tag, "file-tag.csf", CsfLimits::default()),
            Err(CsfError::InvalidFileTag(_))
        ));

        let mut bad_label_tag = fixture();
        bad_label_tag[24] = b'X';
        assert!(matches!(
            parse_csf(&bad_label_tag, "tag.csf", CsfLimits::default()),
            Err(CsfError::InvalidRecordTag { offset: 24, .. })
        ));

        let mut bad_string_tag = fixture();
        bad_string_tag[45] = b'X';
        assert!(matches!(
            parse_csf(&bad_string_tag, "string-tag.csf", CsfLimits::default()),
            Err(CsfError::InvalidRecordTag { offset: 45, .. })
        ));

        let mut bad_utf16 = fixture();
        bad_utf16[53] = 0xFF;
        bad_utf16[54] = 0x27;
        assert!(matches!(
            parse_csf(&bad_utf16, "utf16.csf", CsfLimits::default()),
            Err(CsfError::InvalidUtf16 {
                label: 0,
                string: 0
            })
        ));
    }

    #[test]
    fn enforces_counts_lengths_and_complete_consumption() {
        let bytes = fixture();
        let limits = CsfLimits {
            maximum_labels: 2,
            ..CsfLimits::default()
        };
        assert!(matches!(
            parse_csf(&bytes, "limits.csf", limits),
            Err(CsfError::Binary(cic_core::BinaryError::LimitExceeded {
                what: "CSF label count",
                ..
            }))
        ));

        let mut wrong_string_count = bytes.clone();
        wrong_string_count[12..16].copy_from_slice(&1_u32.to_le_bytes());
        assert!(matches!(
            parse_csf(&wrong_string_count, "count.csf", CsfLimits::default()),
            Err(CsfError::StringCountMismatch {
                declared: 1,
                actual: 2
            })
        ));

        let mut trailing = bytes;
        trailing.push(0);
        assert!(matches!(
            parse_csf(&trailing, "trailing.csf", CsfLimits::default()),
            Err(CsfError::TrailingData { length: 1, .. })
        ));
    }

    #[test]
    fn enforces_each_variable_length_limit_before_reading_payload() {
        let bytes = fixture();
        let cases = [
            (
                CsfLimits {
                    maximum_file_bytes: bytes.len() - 1,
                    ..CsfLimits::default()
                },
                "CSF file size",
            ),
            (
                CsfLimits {
                    maximum_variants_per_label: 0,
                    ..CsfLimits::default()
                },
                "CSF variants per label",
            ),
            (
                CsfLimits {
                    maximum_label_bytes: 8,
                    ..CsfLimits::default()
                },
                "CSF label byte length",
            ),
            (
                CsfLimits {
                    maximum_text_units: 4,
                    ..CsfLimits::default()
                },
                "CSF text UTF-16 unit count",
            ),
            (
                CsfLimits {
                    maximum_wave_bytes: 8,
                    ..CsfLimits::default()
                },
                "CSF wave-name byte length",
            ),
        ];

        for (limits, expected) in cases {
            assert!(matches!(
                parse_csf(&bytes, "limits.csf", limits),
                Err(CsfError::Binary(cic_core::BinaryError::LimitExceeded {
                    what,
                    ..
                })) if what == expected
            ));
        }
    }

    #[test]
    fn duplicate_labels_are_preserved_in_file_order() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b" FSC");
        bytes.extend_from_slice(&3_u32.to_le_bytes());
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        for _ in 0..2 {
            bytes.extend_from_slice(b" LBL");
            bytes.extend_from_slice(&0_u32.to_le_bytes());
            bytes.extend_from_slice(&3_u32.to_le_bytes());
            bytes.extend_from_slice(b"DUP");
        }

        let parsed = parse_csf(&bytes, "duplicates.csf", CsfLimits::default())
            .expect("duplicates are representable");
        assert_eq!(parsed.labels().len(), 2);
        assert_eq!(parsed.labels()[0].name_bytes(), b"DUP");
        assert_eq!(parsed.labels()[1].name_bytes(), b"DUP");
    }
}
