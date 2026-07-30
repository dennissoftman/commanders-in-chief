//! The binary glTF container: splitting one into its two chunks, and putting one back together.
//!
//! A `.glb` is a twelve-byte header and then length-tagged chunks, each padded to four bytes: a JSON chunk
//! holding the document, and a binary chunk holding buffers and embedded images.
//!
//! # Why this is here rather than in the tool that writes them
//!
//! Both halves of the engine need it, for opposite reasons. The offline tool writes a container — that is
//! obvious. The *runtime* needs it because the `gltf` crate eagerly decodes every image and knows only PNG
//! and JPEG, so a container carrying an embedded DDS is refused outright with "unsupported image
//! encoding". Reading one therefore means lifting the DDS out and handing the crate a document whose
//! images it can cope with, which is a container rewrite in the middle of an import.
//!
//! One definition of the format, then, rather than one in the importer and another in the converter.
//!
//! # The JSON is untyped on purpose
//!
//! [`Glb::document`] is a `serde_json::Value` rather than a set of structures. Everything neither the
//! importer nor the converter touches has to survive a round trip byte for byte — including an extension
//! neither has heard of — and a typed representation drops what it has no field for. An unknown *chunk* is
//! refused instead of skipped: skipping is right for a reader and wrong for something that writes the file
//! back out, where it would be a silent loss.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use serde_json::Value;

/// The magic every container starts with.
///
/// Spelled from its bytes rather than written as hex, the way [`crate::terrain`] spells its own. The hex is
/// not reviewable: this was first written as `0x4674_6C67`, one nibble out, and matched by both the reader
/// and its own fixture — so it round-tripped happily and only the `gltf` crate refusing the fixture found
/// it.
pub const MAGIC: u32 = u32::from_le_bytes(*b"glTF");
/// The only container version this reads. A bump can change what fields mean.
pub const VERSION: u32 = 2;
/// The JSON chunk's type tag.
const CHUNK_JSON: u32 = u32::from_le_bytes(*b"JSON");
/// The binary chunk's type tag.
const CHUNK_BIN: u32 = u32::from_le_bytes(*b"BIN\0");

/// The mime type the `MSFT_texture_dds` extension gives an embedded DDS image.
pub const DDS_MIME_TYPE: &str = "image/vnd-ms.dds";

/// A container split into its document and its binary payload.
#[derive(Debug, Clone)]
pub struct Glb {
    /// The JSON chunk, parsed but untyped. See the module note on why.
    pub document: Value,
    /// The binary chunk, or empty when the container has none.
    pub binary: Vec<u8>,
}

impl Glb {
    /// Splits a container into its document and its binary chunk.
    ///
    /// # Errors
    ///
    /// Returns a structured [`GlbError`] when the magic or version is wrong, a chunk declares more bytes
    /// than it has, the JSON will not parse, or the container holds a chunk this cannot round-trip.
    pub fn split(bytes: &[u8]) -> Result<Self, GlbError> {
        let word = |offset: usize| -> Result<u32, GlbError> {
            bytes
                .get(offset..offset + 4)
                .and_then(|slice| slice.try_into().ok())
                .map(u32::from_le_bytes)
                .ok_or(GlbError::Truncated { offset })
        };
        if word(0)? != MAGIC {
            return Err(GlbError::NotGlb);
        }
        let version = word(4)?;
        if version != VERSION {
            return Err(GlbError::UnsupportedVersion(version));
        }

        let mut document = None;
        let mut binary = Vec::new();
        let mut offset = 12usize;
        while offset + 8 <= bytes.len() {
            let length = as_usize(word(offset)?);
            let kind = word(offset + 4)?;
            let start = offset + 8;
            let payload = bytes
                .get(start..start + length)
                .ok_or(GlbError::ChunkOverruns { offset, length })?;
            match kind {
                CHUNK_JSON => {
                    document = Some(
                        serde_json::from_slice::<Value>(payload)
                            .map_err(|error| GlbError::Json(error.to_string()))?,
                    );
                }
                CHUNK_BIN => binary = payload.to_vec(),
                other => return Err(GlbError::UnknownChunk(other)),
            }
            // The padding to the next four-byte boundary is part of the container.
            offset = start + length + ((4 - length % 4) % 4);
        }
        document
            .ok_or(GlbError::NoDocument)
            .map(|document| Self { document, binary })
    }

    /// Writes the container back out, padding both chunks as the format requires.
    ///
    /// The buffer's declared length is set from the binary chunk rather than trusted from the document,
    /// because the two disagreeing is exactly what a rewrite gets wrong.
    ///
    /// # Errors
    ///
    /// Returns [`GlbError::TooLarge`] when the result does not fit the format's 32-bit lengths, or
    /// [`GlbError::Json`] when the document will not serialize.
    pub fn assemble(&self) -> Result<Vec<u8>, GlbError> {
        let mut document = self.document.clone();
        if let Some(object) = document.as_object_mut() {
            let mut buffer = serde_json::Map::new();
            buffer.insert(
                "byteLength".to_owned(),
                Value::Number(self.binary.len().into()),
            );
            object.insert(
                "buffers".to_owned(),
                Value::Array(vec![Value::Object(buffer)]),
            );
        }

        let mut text =
            serde_json::to_vec(&document).map_err(|error| GlbError::Json(error.to_string()))?;
        // The JSON chunk pads with spaces and the binary chunk with zeroes, which the format specifies
        // rather than leaving to taste.
        while !text.len().is_multiple_of(4) {
            text.push(b' ');
        }
        let mut binary = self.binary.clone();
        while !binary.len().is_multiple_of(4) {
            binary.push(0);
        }

        let total = 12 + 8 + text.len() + 8 + binary.len();
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(&MAGIC.to_le_bytes());
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(
            &u32::try_from(total)
                .map_err(|_| GlbError::TooLarge)?
                .to_le_bytes(),
        );
        for (payload, kind) in [(&text, CHUNK_JSON), (&binary, CHUNK_BIN)] {
            out.extend_from_slice(
                &u32::try_from(payload.len())
                    .map_err(|_| GlbError::TooLarge)?
                    .to_le_bytes(),
            );
            out.extend_from_slice(&kind.to_le_bytes());
            out.extend_from_slice(payload);
        }
        Ok(out)
    }

    /// Returns an array member of the document, or an empty slice when it is absent.
    #[must_use]
    pub fn array(&self, key: &str) -> &[Value] {
        self.document
            .get(key)
            .and_then(Value::as_array)
            .map_or(&[], |values| values)
    }

    /// Returns the bytes one buffer view covers.
    ///
    /// `None` when the view is absent or reaches past the binary chunk, so a malformed document produces a
    /// missing texture rather than a panic.
    #[must_use]
    pub fn view(&self, index: usize) -> Option<&[u8]> {
        let view = self.array("bufferViews").get(index)?;
        let offset = as_usize(
            u32::try_from(view.get("byteOffset").and_then(Value::as_u64).unwrap_or(0)).ok()?,
        );
        let length = as_usize(u32::try_from(view.get("byteLength")?.as_u64()?).ok()?);
        self.binary.get(offset..offset.checked_add(length)?)
    }

    /// Appends bytes to the binary chunk as a new buffer view, returning its index.
    ///
    /// Aligned to four bytes, which is the strictest alignment any accessor component needs.
    ///
    /// # Errors
    ///
    /// Returns [`GlbError::MalformedDocument`] when the document is not an object, or its `bufferViews` not
    /// an array. Fallible rather than assuming: the JSON chunk is parsed from an untrusted file, so `[1, 2]`
    /// is a document this could otherwise be asked to push into.
    pub fn push_view(&mut self, payload: &[u8]) -> Result<usize, GlbError> {
        while !self.binary.len().is_multiple_of(4) {
            self.binary.push(0);
        }
        let offset = self.binary.len();
        self.binary.extend_from_slice(payload);

        let mut view = serde_json::Map::new();
        view.insert("buffer".to_owned(), Value::Number(0.into()));
        view.insert("byteOffset".to_owned(), Value::Number(offset.into()));
        view.insert("byteLength".to_owned(), Value::Number(payload.len().into()));
        let views = self
            .document
            .as_object_mut()
            .and_then(|object| {
                object
                    .entry("bufferViews")
                    .or_insert_with(|| Value::Array(Vec::new()))
                    .as_array_mut()
            })
            .ok_or(GlbError::MalformedDocument("bufferViews"))?;
        views.push(Value::Object(view));
        Ok(views.len() - 1)
    }

    /// Records an extension in `extensionsUsed`, which a conforming document must declare.
    pub fn declare_extension(&mut self, name: &str) {
        let Some(object) = self.document.as_object_mut() else {
            return;
        };
        let used = object
            .entry("extensionsUsed")
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Some(list) = used.as_array_mut()
            && !list.iter().any(|entry| entry.as_str() == Some(name))
        {
            list.push(Value::String(name.to_owned()));
        }
    }
}

/// A `u32` as a `usize`, saturating. Every call is a length or an offset already inside the file.
fn as_usize(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

/// A structured failure while reading or writing a container.
#[derive(Debug)]
pub enum GlbError {
    /// The first four bytes were not `glTF`.
    NotGlb,
    /// The container declares a version this does not read.
    UnsupportedVersion(u32),
    /// The file ended inside the header.
    Truncated {
        /// Where the read stopped.
        offset: usize,
    },
    /// A chunk declared more bytes than the file holds.
    ChunkOverruns {
        /// Where the chunk starts.
        offset: usize,
        /// Bytes it declared.
        length: usize,
    },
    /// The container holds a chunk type this cannot round-trip, so rewriting it would drop data.
    UnknownChunk(u32),
    /// The container has no JSON chunk.
    NoDocument,
    /// The document will not parse or serialize.
    Json(String),
    /// The result does not fit the format's 32-bit lengths.
    TooLarge,
    /// The document's shape is not what the format requires.
    MalformedDocument(&'static str),
}

impl Display for GlbError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotGlb => formatter.write_str("not a binary glTF: the magic is not `glTF`"),
            Self::UnsupportedVersion(version) => write!(
                formatter,
                "binary glTF version {version} is not {VERSION}, and a version bump can change what \
                 fields mean"
            ),
            Self::Truncated { offset } => {
                write!(
                    formatter,
                    "the container ends inside its header at byte {offset}"
                )
            }
            Self::ChunkOverruns { offset, length } => write!(
                formatter,
                "a chunk at byte {offset} declares {length} bytes it does not have"
            ),
            Self::UnknownChunk(kind) => write!(
                formatter,
                "the container holds a chunk of type {kind:#010x} a rewrite would drop"
            ),
            Self::NoDocument => formatter.write_str("the container has no JSON chunk"),
            Self::Json(message) => write!(formatter, "the document will not parse: {message}"),
            Self::TooLarge => {
                formatter.write_str("the container does not fit the format's 32-bit lengths")
            }
            Self::MalformedDocument(what) => {
                write!(
                    formatter,
                    "the document's `{what}` is not the shape glTF requires"
                )
            }
        }
    }
}

impl Error for GlbError {}

#[cfg(test)]
mod tests {
    use super::{Glb, GlbError, MAGIC, VERSION};
    use serde_json::json;

    fn container(document: serde_json::Value, binary: &[u8]) -> Vec<u8> {
        Glb {
            document,
            binary: binary.to_vec(),
        }
        .assemble()
        .expect("assemble")
    }

    #[test]
    fn a_container_round_trips_through_both_halves() {
        // Everything the document says survives, including a key nothing here understands -- which is the
        // property the untyped tree exists for.
        let document = json!({
            "asset": {"version": "2.0"},
            "extensions": {"SOME_vendor_thing": {"kept": [1, 2, 3]}},
            "images": [{"bufferView": 0, "mimeType": "image/png"}]
        });
        let bytes = container(document.clone(), &[1, 2, 3, 4, 5]);
        let split = Glb::split(&bytes).expect("split");
        assert_eq!(
            split.document.get("extensions"),
            document.get("extensions"),
            "an unknown extension must survive"
        );
        assert_eq!(&split.binary[..5], &[1, 2, 3, 4, 5]);
    }

    #[test]
    fn a_view_reads_exactly_its_own_bytes() {
        let mut glb = Glb {
            document: json!({"asset": {"version": "2.0"}}),
            binary: Vec::new(),
        };
        let first = glb.push_view(&[9, 8, 7]).expect("push");
        let second = glb.push_view(&[1, 2, 3, 4, 5]).expect("push");
        assert_eq!(glb.view(first), Some(&[9, 8, 7][..]));
        assert_eq!(glb.view(second), Some(&[1, 2, 3, 4, 5][..]));
        // Four-byte aligned, which is what an accessor's component alignment needs.
        let offset = glb.array("bufferViews")[second]["byteOffset"]
            .as_u64()
            .expect("an offset");
        assert!(offset.is_multiple_of(4), "a view landed at {offset}");
        assert_eq!(
            glb.view(99),
            None,
            "a view that does not exist is not a panic"
        );
    }

    #[test]
    fn the_buffer_length_is_taken_from_the_bytes_rather_than_the_document() {
        // The two disagreeing is exactly what a rewrite gets wrong, so the writer does not trust the
        // document's own figure.
        let bytes = container(
            json!({"asset": {"version": "2.0"}, "buffers": [{"byteLength": 9999}]}),
            &[0; 12],
        );
        let split = Glb::split(&bytes).expect("split");
        assert_eq!(split.array("buffers")[0]["byteLength"].as_u64(), Some(12));
    }

    #[test]
    fn an_extension_is_declared_once_however_often_it_is_recorded() {
        let mut glb = Glb {
            document: json!({"asset": {"version": "2.0"}}),
            binary: Vec::new(),
        };
        glb.declare_extension("MSFT_texture_dds");
        glb.declare_extension("MSFT_texture_dds");
        assert_eq!(glb.array("extensionsUsed").len(), 1);
    }

    #[test]
    fn what_it_cannot_round_trip_is_refused_rather_than_dropped() {
        assert!(matches!(
            Glb::split(b"not a container"),
            Err(GlbError::NotGlb)
        ));

        let mut wrong_version = container(json!({"asset": {"version": "2.0"}}), &[]);
        wrong_version[4..8].copy_from_slice(&3u32.to_le_bytes());
        assert!(matches!(
            Glb::split(&wrong_version),
            Err(GlbError::UnsupportedVersion(3))
        ));

        // An unknown chunk. The specification says a *reader* should skip one, and this refuses -- because
        // skipping and then writing the file back out is a silent loss.
        let mut extra = container(json!({"asset": {"version": "2.0"}}), &[]);
        extra.extend_from_slice(&4u32.to_le_bytes());
        extra.extend_from_slice(&u32::from_le_bytes(*b"ZOOM").to_le_bytes());
        extra.extend_from_slice(&[0u8; 4]);
        assert!(matches!(Glb::split(&extra), Err(GlbError::UnknownChunk(_))));

        // The binary chunk's *length* field, which sits eight bytes before its four bytes of payload --
        // not the type tag four bytes after it, which is what a first attempt patched and which reports as
        // an unknown chunk instead.
        let mut truncated = container(json!({"asset": {"version": "2.0"}}), &[1, 2, 3, 4]);
        let length = truncated.len();
        truncated[length - 12..length - 8].copy_from_slice(&9999u32.to_le_bytes());
        assert!(matches!(
            Glb::split(&truncated),
            Err(GlbError::ChunkOverruns { .. })
        ));

        assert_eq!(MAGIC.to_le_bytes(), *b"glTF");
        assert_eq!(VERSION, 2);
    }
}
