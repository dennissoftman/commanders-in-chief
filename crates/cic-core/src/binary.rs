use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// A structured failure while reading an untrusted byte region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinaryError {
    /// A read extended beyond the reader's bounded region.
    UnexpectedEof {
        /// Diagnostic name of the input.
        source: String,
        /// Offset at which the read began.
        offset: usize,
        /// Number of requested bytes.
        requested: usize,
        /// Number of bytes that remained.
        remaining: usize,
    },
    /// A requested cursor position was outside the bounded region.
    InvalidSeek {
        /// Diagnostic name of the input.
        source: String,
        /// Requested position.
        offset: usize,
        /// Length of the bounded region.
        length: usize,
    },
    /// Input exceeded an explicit parser resource limit.
    LimitExceeded {
        /// Name of the limited quantity.
        what: &'static str,
        /// Value supplied by the input.
        actual: usize,
        /// Maximum accepted value.
        maximum: usize,
    },
}

impl Display for BinaryError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof {
                source,
                offset,
                requested,
                remaining,
            } => write!(
                formatter,
                "{source}: read of {requested} bytes at offset {offset} exceeds the bounded input ({remaining} bytes remain)"
            ),
            Self::InvalidSeek {
                source,
                offset,
                length,
            } => write!(
                formatter,
                "{source}: seek to offset {offset} exceeds the bounded input length {length}"
            ),
            Self::LimitExceeded {
                what,
                actual,
                maximum,
            } => write!(
                formatter,
                "{what} value {actual} exceeds the configured limit {maximum}"
            ),
        }
    }
}

impl Error for BinaryError {}

/// Cursor-based reads restricted to a borrowed byte region.
#[derive(Debug, Clone)]
pub struct BinaryReader<'a> {
    source: String,
    bytes: &'a [u8],
    position: usize,
}

impl<'a> BinaryReader<'a> {
    /// Creates a reader over the complete supplied byte slice.
    pub fn new(bytes: &'a [u8], source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            bytes,
            position: 0,
        }
    }

    /// Returns the diagnostic input name.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Returns the bounded region length.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Returns whether the bounded region is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Returns the current cursor position.
    #[must_use]
    pub const fn position(&self) -> usize {
        self.position
    }

    /// Returns the number of unread bytes.
    #[must_use]
    pub const fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    /// Moves the cursor to an absolute position inside the bounded region.
    ///
    /// # Errors
    ///
    /// Returns [`BinaryError::InvalidSeek`] when `position` exceeds the region length.
    pub fn seek(&mut self, position: usize) -> Result<(), BinaryError> {
        if position > self.bytes.len() {
            return Err(BinaryError::InvalidSeek {
                source: self.source.clone(),
                offset: position,
                length: self.bytes.len(),
            });
        }
        self.position = position;
        Ok(())
    }

    /// Advances by `length` bytes.
    ///
    /// # Errors
    ///
    /// Returns [`BinaryError::UnexpectedEof`] when the region is too short.
    pub fn skip(&mut self, length: usize) -> Result<(), BinaryError> {
        self.read_exact(length).map(|_| ())
    }

    /// Borrows exactly `length` bytes and advances the cursor.
    ///
    /// # Errors
    ///
    /// Returns [`BinaryError::UnexpectedEof`] when the region is too short.
    pub fn read_exact(&mut self, length: usize) -> Result<&'a [u8], BinaryError> {
        let remaining = self.remaining();
        if length > remaining {
            return Err(BinaryError::UnexpectedEof {
                source: self.source.clone(),
                offset: self.position,
                requested: length,
                remaining,
            });
        }

        let start = self.position;
        self.position += length;
        Ok(&self.bytes[start..self.position])
    }

    /// Creates a reader bounded to the next `length` bytes and advances the parent.
    ///
    /// # Errors
    ///
    /// Returns [`BinaryError::UnexpectedEof`] when the parent region is too short.
    pub fn read_region(&mut self, length: usize) -> Result<Self, BinaryError> {
        let start = self.position;
        let bytes = self.read_exact(length)?;
        Ok(Self::new(bytes, format!("{}@{start}", self.source)))
    }

    /// Reads one byte.
    ///
    /// # Errors
    ///
    /// Returns [`BinaryError::UnexpectedEof`] at the end of the region.
    pub fn read_u8(&mut self) -> Result<u8, BinaryError> {
        Ok(self.read_array::<1>()?[0])
    }

    /// Reads a little-endian 16-bit unsigned integer.
    ///
    /// # Errors
    ///
    /// Returns [`BinaryError::UnexpectedEof`] when fewer than two bytes remain.
    pub fn read_u16_le(&mut self) -> Result<u16, BinaryError> {
        Ok(u16::from_le_bytes(self.read_array()?))
    }

    /// Reads a big-endian 16-bit unsigned integer.
    ///
    /// # Errors
    ///
    /// Returns [`BinaryError::UnexpectedEof`] when fewer than two bytes remain.
    pub fn read_u16_be(&mut self) -> Result<u16, BinaryError> {
        Ok(u16::from_be_bytes(self.read_array()?))
    }

    /// Reads a little-endian 32-bit unsigned integer.
    ///
    /// # Errors
    ///
    /// Returns [`BinaryError::UnexpectedEof`] when fewer than four bytes remain.
    pub fn read_u32_le(&mut self) -> Result<u32, BinaryError> {
        Ok(u32::from_le_bytes(self.read_array()?))
    }

    /// Reads a big-endian 32-bit unsigned integer.
    ///
    /// # Errors
    ///
    /// Returns [`BinaryError::UnexpectedEof`] when fewer than four bytes remain.
    pub fn read_u32_be(&mut self) -> Result<u32, BinaryError> {
        Ok(u32::from_be_bytes(self.read_array()?))
    }

    fn read_array<const LENGTH: usize>(&mut self) -> Result<[u8; LENGTH], BinaryError> {
        let bytes = self.read_exact(LENGTH)?;
        let mut result = [0; LENGTH];
        result.copy_from_slice(bytes);
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::{BinaryError, BinaryReader};

    #[test]
    fn reads_endian_values_and_tracks_position() {
        let bytes = [0x34, 0x12, 0x12, 0x34, 0x78, 0x56, 0x34, 0x12];
        let mut reader = BinaryReader::new(&bytes, "fixture");

        assert_eq!(reader.read_u16_le(), Ok(0x1234));
        assert_eq!(reader.read_u16_be(), Ok(0x1234));
        assert_eq!(reader.read_u32_le(), Ok(0x1234_5678));
        assert_eq!(reader.position(), bytes.len());
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn reports_truncation_without_advancing() {
        let mut reader = BinaryReader::new(&[1, 2, 3], "short.bin");

        assert_eq!(
            reader.read_u32_le(),
            Err(BinaryError::UnexpectedEof {
                source: "short.bin".to_owned(),
                offset: 0,
                requested: 4,
                remaining: 3,
            })
        );
        assert_eq!(reader.position(), 0);
    }

    #[test]
    fn sub_reader_cannot_escape_parent_region() {
        let bytes = [1, 2, 3, 4];
        let mut reader = BinaryReader::new(&bytes, "regions.bin");
        let mut region = reader.read_region(2).expect("valid region");

        assert_eq!(region.read_u16_le(), Ok(0x0201));
        assert!(region.read_u8().is_err());
        assert_eq!(reader.read_u16_le(), Ok(0x0403));
    }

    #[test]
    fn seek_accepts_end_and_rejects_past_end() {
        let mut reader = BinaryReader::new(&[1, 2], "seek.bin");

        assert_eq!(reader.seek(2), Ok(()));
        assert!(matches!(
            reader.seek(3),
            Err(BinaryError::InvalidSeek {
                offset: 3,
                length: 2,
                ..
            })
        ));
        assert_eq!(reader.position(), 2);
    }
}
