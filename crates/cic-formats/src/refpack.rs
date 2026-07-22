//! Bounded `RefPack` decoding for source-established `EAR\0` resource wrappers.
//!
//! The command forms are derived from `refdecode.cpp` and `CompressionManager.cpp` in
//! `GeneralsGameCode` revision `9f7abb866f5afd446db14149979e744c7216baaf`, licensed under
//! GPL-3.0-or-later with Electronic Arts Section 7 terms. Full notices and permanent links are in
//! `docs/provenance/map.md`.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use cic_core::{BinaryError, BinaryReader};

const WRAPPER_BYTES: usize = 8;

/// A structured `EAR\0` `RefPack` decompression failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefPackError {
    /// A bounded input read or output-size limit failed.
    Binary(BinaryError),
    /// The wrapper signature was not `EAR\0`.
    InvalidWrapper,
    /// A signed wrapper length was negative.
    NegativeOutputLength(i32),
    /// The inner `RefPack` type word was unknown.
    InvalidType(u16),
    /// The wrapper and inner stream declared different output sizes.
    OutputLengthMismatch { wrapper: usize, stream: usize },
    /// A copy command referred before the start of decoded output.
    InvalidBackReference { offset: usize, decoded_bytes: usize },
    /// A command would emit more bytes than the declared output length.
    OutputOverflow { attempted: usize, declared: usize },
    /// The end marker arrived before the declared output length.
    TruncatedOutput { actual: usize, declared: usize },
    /// Compressed bytes remained after the end marker.
    TrailingInput(usize),
}

impl Display for RefPackError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binary(error) => Display::fmt(error, formatter),
            Self::InvalidWrapper => formatter.write_str("invalid EAR RefPack wrapper"),
            Self::NegativeOutputLength(value) => {
                write!(formatter, "EAR RefPack output length is negative: {value}")
            }
            Self::InvalidType(value) => write!(formatter, "invalid RefPack type 0x{value:04X}"),
            Self::OutputLengthMismatch { wrapper, stream } => write!(
                formatter,
                "EAR wrapper declares {wrapper} output bytes but RefPack stream declares {stream}"
            ),
            Self::InvalidBackReference {
                offset,
                decoded_bytes,
            } => write!(
                formatter,
                "RefPack back-reference offset {offset} exceeds {decoded_bytes} decoded bytes"
            ),
            Self::OutputOverflow {
                attempted,
                declared,
            } => write!(
                formatter,
                "RefPack command would emit {attempted} bytes past declared output length {declared}"
            ),
            Self::TruncatedOutput { actual, declared } => write!(
                formatter,
                "RefPack ended after {actual} bytes; expected {declared}"
            ),
            Self::TrailingInput(count) => {
                write!(formatter, "RefPack stream has {count} trailing bytes")
            }
        }
    }
}

impl Error for RefPackError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Binary(error) => Some(error),
            _ => None,
        }
    }
}

impl From<BinaryError> for RefPackError {
    fn from(error: BinaryError) -> Self {
        Self::Binary(error)
    }
}

pub(crate) fn decompress_ear(
    bytes: &[u8],
    source: &str,
    maximum_output_bytes: usize,
) -> Result<Vec<u8>, RefPackError> {
    let mut wrapper = BinaryReader::new(bytes, source);
    if wrapper.read_exact(4)? != b"EAR\0" {
        return Err(RefPackError::InvalidWrapper);
    }
    let signed_length = i32::from_le_bytes(wrapper.read_u32_le()?.to_le_bytes());
    let declared = usize::try_from(signed_length)
        .map_err(|_| RefPackError::NegativeOutputLength(signed_length))?;
    enforce_limit(declared, maximum_output_bytes)?;

    let payload = wrapper.read_exact(wrapper.remaining())?;
    let mut reader = BinaryReader::new(payload, format!("{source}@{WRAPPER_BYTES}"));
    let stream_type = reader.read_u16_be()?;
    if !matches!(stream_type, 0x10FB | 0x11FB | 0x90FB | 0x91FB) {
        return Err(RefPackError::InvalidType(stream_type));
    }
    let size_bytes = if stream_type & 0x8000 == 0 { 3 } else { 4 };
    if stream_type & 0x0100 != 0 {
        reader.skip(size_bytes)?;
    }
    let stream_length = read_be_size(&mut reader, size_bytes)?;
    if stream_length != declared {
        return Err(RefPackError::OutputLengthMismatch {
            wrapper: declared,
            stream: stream_length,
        });
    }

    let mut output = Vec::with_capacity(declared);
    loop {
        let first = reader.read_u8()?;
        if first & 0x80 == 0 {
            let second = reader.read_u8()?;
            copy_literals(&mut reader, &mut output, usize::from(first & 3), declared)?;
            let offset = (usize::from(first & 0x60) << 3) + usize::from(second) + 1;
            let length = usize::from((first & 0x1C) >> 2) + 3;
            copy_reference(&mut output, offset, length, declared)?;
        } else if first & 0x40 == 0 {
            let second = reader.read_u8()?;
            let third = reader.read_u8()?;
            copy_literals(&mut reader, &mut output, usize::from(second >> 6), declared)?;
            let offset = (usize::from(second & 0x3F) << 8) | usize::from(third);
            copy_reference(
                &mut output,
                offset + 1,
                usize::from(first & 0x3F) + 4,
                declared,
            )?;
        } else if first & 0x20 == 0 {
            let second = reader.read_u8()?;
            let third = reader.read_u8()?;
            let fourth = reader.read_u8()?;
            copy_literals(&mut reader, &mut output, usize::from(first & 3), declared)?;
            let offset = (usize::from((first & 0x10) >> 4) << 16)
                | (usize::from(second) << 8)
                | usize::from(third);
            let length = (usize::from((first & 0x0C) >> 2) << 8) + usize::from(fourth) + 5;
            copy_reference(&mut output, offset + 1, length, declared)?;
        } else {
            let literal_length = usize::from(first & 0x1F) * 4 + 4;
            if literal_length <= 112 {
                copy_literals(&mut reader, &mut output, literal_length, declared)?;
                continue;
            }
            copy_literals(&mut reader, &mut output, usize::from(first & 3), declared)?;
            break;
        }
    }

    if output.len() != declared {
        return Err(RefPackError::TruncatedOutput {
            actual: output.len(),
            declared,
        });
    }
    if reader.remaining() != 0 {
        return Err(RefPackError::TrailingInput(reader.remaining()));
    }
    Ok(output)
}

fn read_be_size(reader: &mut BinaryReader<'_>, size_bytes: usize) -> Result<usize, RefPackError> {
    let bytes = reader.read_exact(size_bytes)?;
    let mut value = 0_usize;
    for byte in bytes {
        value = value
            .checked_shl(8)
            .and_then(|shifted| shifted.checked_add(usize::from(*byte)))
            .ok_or(BinaryError::LimitExceeded {
                what: "RefPack size field",
                actual: usize::MAX,
                maximum: usize::MAX,
            })?;
    }
    Ok(value)
}

fn copy_literals(
    reader: &mut BinaryReader<'_>,
    output: &mut Vec<u8>,
    length: usize,
    declared: usize,
) -> Result<(), RefPackError> {
    ensure_output_length(output.len(), length, declared)?;
    output.extend_from_slice(reader.read_exact(length)?);
    Ok(())
}

fn copy_reference(
    output: &mut Vec<u8>,
    offset: usize,
    length: usize,
    declared: usize,
) -> Result<(), RefPackError> {
    if offset > output.len() {
        return Err(RefPackError::InvalidBackReference {
            offset,
            decoded_bytes: output.len(),
        });
    }
    ensure_output_length(output.len(), length, declared)?;
    for _ in 0..length {
        let source_index = output.len() - offset;
        let byte = output[source_index];
        output.push(byte);
    }
    Ok(())
}

fn ensure_output_length(
    current: usize,
    addition: usize,
    declared: usize,
) -> Result<(), RefPackError> {
    let attempted = current
        .checked_add(addition)
        .ok_or(RefPackError::OutputOverflow {
            attempted: usize::MAX,
            declared,
        })?;
    if attempted > declared {
        return Err(RefPackError::OutputOverflow {
            attempted,
            declared,
        });
    }
    Ok(())
}

fn enforce_limit(actual: usize, maximum: usize) -> Result<(), RefPackError> {
    if actual > maximum {
        Err(RefPackError::Binary(BinaryError::LimitExceeded {
            what: "MAP decompressed size",
            actual,
            maximum,
        }))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{RefPackError, decompress_ear};

    fn wrapper(output_length: i32, payload: &[u8]) -> Vec<u8> {
        let mut bytes = b"EAR\0".to_vec();
        bytes.extend_from_slice(&output_length.to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    #[test]
    fn decodes_literals_and_overlapping_back_references() {
        let bytes = wrapper(
            6,
            &[0x10, 0xFB, 0, 0, 6, 0x03, 0x02, b'a', b'b', b'c', 0xFC],
        );
        assert_eq!(
            decompress_ear(&bytes, "repeat.map", 64),
            Ok(b"abcabc".to_vec())
        );
    }

    #[test]
    fn preserves_high_distance_bits_in_short_commands() {
        let mut payload = vec![0x10, 0xFB, 0, 1, 7, 0xFB];
        let mut first_literals = vec![0_u8; 112];
        first_literals[..3].copy_from_slice(b"abc");
        payload.extend_from_slice(&first_literals);
        payload.push(0xFB);
        payload.extend_from_slice(&[0_u8; 112]);
        payload.push(0xE8);
        payload.extend_from_slice(&[0_u8; 36]);
        payload.extend_from_slice(&[0x20, 0x03, 0xFC]);
        let bytes = wrapper(263, &payload);

        let output = decompress_ear(&bytes, "distance.map", 512).expect("valid RefPack");
        assert_eq!(&output[260..], b"abc");
    }

    #[test]
    fn rejects_invalid_references_and_output_limits() {
        let invalid = wrapper(3, &[0x10, 0xFB, 0, 0, 3, 0, 0, 0xFC]);
        assert!(matches!(
            decompress_ear(&invalid, "invalid.map", 64),
            Err(RefPackError::InvalidBackReference { .. })
        ));
        let limited = wrapper(65, &[0x10, 0xFB, 0, 0, 65, 0xFC]);
        assert!(matches!(
            decompress_ear(&limited, "limited.map", 64),
            Err(RefPackError::Binary(_))
        ));
    }
}
