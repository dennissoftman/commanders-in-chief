//! The project's own terrain heightfield container.
//!
//! # Why this is not JSON or glTF
//!
//! A heightfield is a large, regular grid of numbers, and both obvious alternatives are wrong for
//! it. JSON would store `"1024"` as four bytes plus a delimiter where two suffice, cost a parse of
//! every sample, and — for floats — round-trip lossily unless every value is written to 17
//! significant digits. glTF describes *meshes*: expressing a heightfield as one would discard the
//! regularity that makes terrain cheap, turning an implicit grid into explicit vertices and indices
//! that cost an order of magnitude more space and forbid GPU-side level of detail.
//!
//! # Layout
//!
//! A tagged chunk container, deliberately similar in shape to GLB: a small header, then
//! `[tag][length][payload]` records. Unknown chunks are preserved and skipped rather than refused,
//! so a newer editor's extra data does not make a map unreadable by an older build.
//!
//! ```text
//! "CICT"  u32 version  u32 chunk_count
//! then, repeated:  [u32 tag][u32 byte_length][payload, padded to 4 bytes]
//! ```
//!
//! Elevations are `u16` rather than `f32`. That halves the heightfield, and — the actual reason —
//! 16-bit integer is a baseline GPU texture format (`R16Uint`), so the payload uploads as a height
//! texture byte-for-byte with no conversion pass. 65,536 quantization levels across a sane
//! vertical range is far finer than terrain needs.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use cic_core::{BinaryError, BinaryReader};
use cic_vfs::Vfs;

use crate::texture::{TextureAsset, TextureLimits, TextureResolveError, resolve_named_textures};

/// Container magic.
pub const MAGIC: &[u8; 4] = b"CICT";

/// The format version this build writes and reads.
pub const VERSION: u32 = 1;

const CHUNK_HEADER: u32 = u32::from_le_bytes(*b"HEAD");
const CHUNK_HEIGHTS: u32 = u32::from_le_bytes(*b"HGHT");
const CHUNK_LAYER_WEIGHTS: u32 = u32::from_le_bytes(*b"LYRW");
const CHUNK_LAYER_NAMES: u32 = u32::from_le_bytes(*b"LYRN");

/// Byte length of the `HEAD` chunk payload.
const HEADER_PAYLOAD: usize = 24;

/// Explicit bounds applied while decoding an untrusted terrain container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerrainLimits {
    /// Maximum samples along either axis.
    pub maximum_dimension: u32,
    /// Maximum total samples, bounding `width * height` before any allocation.
    pub maximum_samples: usize,
    /// Maximum number of texture layers.
    pub maximum_layers: usize,
    /// Maximum number of chunks, including unknown ones.
    pub maximum_chunks: usize,
}

impl Default for TerrainLimits {
    fn default() -> Self {
        Self {
            maximum_dimension: 8_192,
            maximum_samples: 16 * 1_024 * 1_024,
            maximum_layers: 32,
            maximum_chunks: 256,
        }
    }
}

/// One named texture layer and its per-sample weights.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerrainLayer {
    /// Layer name, resolved against the material set by the renderer.
    pub name: String,
    /// One weight per sample, in row-major order, `0` absent and `255` fully covering.
    pub weights: Vec<u8>,
}

/// Looks up a block-compressed sidecar for each of a terrain's layers.
///
/// # The convention
///
/// A layer named `grass` is textured by `textures/grass.dds`. The layer name is the key, which is what
/// [`TerrainLayer::name`] has always been for — the container has never held layer *pixels*, only the name
/// and the weights, and the renderer has always resolved that name against a material set it was handed.
/// This makes the name resolve against the package as well.
///
/// Terrain is where block compression pays most. A detail texture is sampled by up to eight layers in one
/// fragment across the whole visible map, so it is both the largest texture budget here and the most
/// bandwidth-sensitive; and detail textures are authored to one size and tiled, so the uniform-size
/// requirement of a compressed array costs nothing.
///
/// Returns one entry per layer, in layer order, so the result indexes alongside
/// [`Terrain::layers`] and the material set the renderer takes.
///
/// # Errors
///
/// As [`resolve_named_textures`]: an absent sidecar is not an error — that is a layer that renders as its
/// flat palette colour, which is what an unconverted or deliberately untextured layer has always done —
/// and one that exists but will not read is.
pub fn resolve_terrain_textures(
    terrain: &Terrain,
    vfs: &Vfs,
    limits: TextureLimits,
) -> Result<Vec<Option<TextureAsset>>, TextureResolveError> {
    resolve_named_textures(
        terrain.layers().iter().map(|layer| layer.name.as_str()),
        vfs,
        limits,
    )
}

/// A decoded terrain heightfield with its texture layers.
#[derive(Debug, Clone, PartialEq)]
pub struct Terrain {
    width: u32,
    height: u32,
    horizontal_scale: f32,
    vertical_scale: f32,
    elevations: Vec<u16>,
    layers: Vec<TerrainLayer>,
}

impl Terrain {
    /// Builds a terrain from raw parts.
    ///
    /// # Errors
    ///
    /// Returns [`TerrainError::SampleCountMismatch`] when the elevation or any layer's weight count
    /// does not equal `width * height`, or [`TerrainError::EmptyDimension`] for a zero axis.
    pub fn new(
        width: u32,
        height: u32,
        horizontal_scale: f32,
        vertical_scale: f32,
        elevations: Vec<u16>,
        layers: Vec<TerrainLayer>,
    ) -> Result<Self, TerrainError> {
        if width == 0 || height == 0 {
            return Err(TerrainError::EmptyDimension { width, height });
        }
        let expected = samples(width, height)?;
        if elevations.len() != expected {
            return Err(TerrainError::SampleCountMismatch {
                what: "elevations",
                actual: elevations.len(),
                expected,
            });
        }
        for layer in &layers {
            if layer.weights.len() != expected {
                return Err(TerrainError::SampleCountMismatch {
                    what: "layer weights",
                    actual: layer.weights.len(),
                    expected,
                });
            }
        }
        Ok(Self {
            width,
            height,
            horizontal_scale,
            vertical_scale,
            elevations,
            layers,
        })
    }

    /// Returns the sample count along X.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the sample count along Y.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Returns world units between adjacent samples.
    #[must_use]
    pub const fn horizontal_scale(&self) -> f32 {
        self.horizontal_scale
    }

    /// Returns world units per quantization step of elevation.
    #[must_use]
    pub const fn vertical_scale(&self) -> f32 {
        self.vertical_scale
    }

    /// Returns the quantized elevations in row-major order.
    #[must_use]
    pub fn elevations(&self) -> &[u16] {
        &self.elevations
    }

    /// Returns the texture layers.
    #[must_use]
    pub fn layers(&self) -> &[TerrainLayer] {
        &self.layers
    }

    /// Returns the world-space elevation at a sample, or `None` when out of range.
    #[must_use]
    pub fn elevation_at(&self, x: u32, y: u32) -> Option<f32> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let index = y as usize * self.width as usize + x as usize;
        Some(f32::from(self.elevations[index]) * self.vertical_scale)
    }

    /// Returns the interpolated world-space elevation at a world XY position.
    ///
    /// Returns `None` outside the terrain, which callers should treat as "no ground known here"
    /// rather than as zero — a camera that reads absent ground as sea level dives through the map at
    /// its edges.
    ///
    /// Bilinear rather than nearest. The camera holds a height above the ground beneath it, and
    /// nearest sampling makes that height jump by a whole quantisation step every time the focus
    /// crosses a sample boundary, which reads as the camera ticking rather than gliding.
    #[must_use]
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    pub fn elevation_at_world(&self, x: f32, y: f32) -> Option<f32> {
        if !x.is_finite() || !y.is_finite() || self.horizontal_scale <= 0.0 {
            return None;
        }
        let [extent_x, extent_y] = self.world_extent();
        if x < 0.0 || y < 0.0 || x > extent_x || y > extent_y {
            return None;
        }
        // Sample-space coordinates. Both are inside the grid by the bounds check above, and the
        // dimension limit keeps them far inside exact f32 integer range.
        let sample_x = x / self.horizontal_scale;
        let sample_y = y / self.horizontal_scale;
        let x0 = (sample_x.floor() as u32).min(self.width - 1);
        let y0 = (sample_y.floor() as u32).min(self.height - 1);
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);
        let fx = sample_x - x0 as f32;
        let fy = sample_y - y0 as f32;

        let at = |x: u32, y: u32| {
            let index = y as usize * self.width as usize + x as usize;
            f32::from(self.elevations[index])
        };
        let top = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
        let bottom = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
        Some((top * (1.0 - fy) + bottom * fy) * self.vertical_scale)
    }

    /// Returns the world-space extent as `[x, y]`.
    ///
    /// One fewer than the sample count per axis: `n` samples span `n - 1` intervals.
    // `TerrainLimits::maximum_dimension` caps a dimension at 8,192, far inside the 2^24 range
    // where `u32 -> f32` is exact, so no precision is lost here in any accepted terrain.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn world_extent(&self) -> [f32; 2] {
        [
            (self.width - 1) as f32 * self.horizontal_scale,
            (self.height - 1) as f32 * self.horizontal_scale,
        ]
    }

    /// Encodes the terrain into its container form.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut chunks: Vec<(u32, Vec<u8>)> = Vec::new();

        let mut header = Vec::with_capacity(HEADER_PAYLOAD);
        header.extend_from_slice(&self.width.to_le_bytes());
        header.extend_from_slice(&self.height.to_le_bytes());
        header.extend_from_slice(&self.horizontal_scale.to_le_bytes());
        header.extend_from_slice(&self.vertical_scale.to_le_bytes());
        header.extend_from_slice(
            &u32::try_from(self.layers.len())
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        header.extend_from_slice(&0u32.to_le_bytes()); // reserved flags
        chunks.push((CHUNK_HEADER, header));

        let mut heights = Vec::with_capacity(self.elevations.len() * 2);
        for elevation in &self.elevations {
            heights.extend_from_slice(&elevation.to_le_bytes());
        }
        chunks.push((CHUNK_HEIGHTS, heights));

        if !self.layers.is_empty() {
            // Names are NUL-terminated and concatenated, so the chunk is one allocation and the
            // count is already known from the header.
            let mut names = Vec::new();
            for layer in &self.layers {
                names.extend_from_slice(layer.name.as_bytes());
                names.push(0);
            }
            chunks.push((CHUNK_LAYER_NAMES, names));

            let mut weights = Vec::with_capacity(self.layers.len() * self.elevations.len());
            for layer in &self.layers {
                weights.extend_from_slice(&layer.weights);
            }
            chunks.push((CHUNK_LAYER_WEIGHTS, weights));
        }

        let mut output = Vec::new();
        output.extend_from_slice(MAGIC);
        output.extend_from_slice(&VERSION.to_le_bytes());
        output.extend_from_slice(
            &u32::try_from(chunks.len())
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        for (tag, payload) in chunks {
            output.extend_from_slice(&tag.to_le_bytes());
            output.extend_from_slice(
                &u32::try_from(payload.len())
                    .unwrap_or(u32::MAX)
                    .to_le_bytes(),
            );
            output.extend_from_slice(&payload);
            let padding = payload.len().next_multiple_of(4) - payload.len();
            output.extend_from_slice(&vec![0u8; padding]);
        }
        output
    }
}

/// Decodes a terrain container.
///
/// # Errors
///
/// Returns a structured [`TerrainError`] for wrong magic, an unsupported version, a missing required
/// chunk, a payload whose length disagrees with the declared dimensions, or an exceeded limit.
pub fn decode_terrain(bytes: &[u8], limits: TerrainLimits) -> Result<Terrain, TerrainError> {
    let mut reader = BinaryReader::new(bytes, "terrain");
    let magic = reader.read_exact(4)?;
    if magic != MAGIC {
        return Err(TerrainError::Magic);
    }
    let version = reader.read_u32_le()?;
    if version != VERSION {
        return Err(TerrainError::UnsupportedVersion(version));
    }
    let chunk_count = reader.read_u32_le()? as usize;
    if chunk_count > limits.maximum_chunks {
        return Err(TerrainError::LimitExceeded {
            what: "chunk count",
            actual: chunk_count,
            maximum: limits.maximum_chunks,
        });
    }

    let mut dimensions: Option<Header> = None;
    let mut heights: Option<&[u8]> = None;
    let mut layer_weights: Option<&[u8]> = None;
    let mut layer_names: Option<&[u8]> = None;

    for _ in 0..chunk_count {
        let tag = reader.read_u32_le()?;
        let length = reader.read_u32_le()? as usize;
        let payload = reader.read_exact(length)?;
        // Padding is part of the container, not the payload, so it is skipped before the next tag.
        let padding = length.next_multiple_of(4) - length;
        reader.skip(padding)?;

        match tag {
            CHUNK_HEADER => dimensions = Some(decode_header(payload, length, limits)?),
            CHUNK_HEIGHTS => heights = Some(payload),
            CHUNK_LAYER_WEIGHTS => layer_weights = Some(payload),
            CHUNK_LAYER_NAMES => layer_names = Some(payload),
            // An unknown chunk from a newer writer is skipped, not refused. Its bytes were already
            // stepped over above, so nothing further is needed here.
            _ => {}
        }
    }

    let (width, height, horizontal_scale, vertical_scale, layer_count) =
        dimensions.ok_or(TerrainError::MissingChunk("HEAD"))?;
    let heights = heights.ok_or(TerrainError::MissingChunk("HGHT"))?;
    let count = samples(width, height)?;

    if heights.len() != count * 2 {
        return Err(TerrainError::ChunkLength {
            what: "HGHT",
            actual: heights.len(),
            expected: count * 2,
        });
    }
    let elevations = heights
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect::<Vec<_>>();

    let mut layers = Vec::with_capacity(layer_count);
    if layer_count > 0 {
        let names = layer_names.ok_or(TerrainError::MissingChunk("LYRN"))?;
        let weights = layer_weights.ok_or(TerrainError::MissingChunk("LYRW"))?;
        let expected_weights = count
            .checked_mul(layer_count)
            .ok_or(TerrainError::SampleOverflow { width, height })?;
        if weights.len() != expected_weights {
            return Err(TerrainError::ChunkLength {
                what: "LYRW",
                actual: weights.len(),
                expected: expected_weights,
            });
        }

        let mut decoded_names = Vec::with_capacity(layer_count);
        for raw in names.split(|byte| *byte == 0) {
            if decoded_names.len() == layer_count {
                break;
            }
            // The concatenation ends with a terminator, so `split` yields one trailing empty slice.
            if raw.is_empty() && decoded_names.len() + 1 > layer_count {
                continue;
            }
            let text = str::from_utf8(raw)
                .map_err(|_| TerrainError::NonUtf8LayerName(decoded_names.len()))?;
            decoded_names.push(text.to_owned());
        }
        if decoded_names.len() != layer_count {
            return Err(TerrainError::LayerNameCount {
                actual: decoded_names.len(),
                expected: layer_count,
            });
        }

        for (index, name) in decoded_names.into_iter().enumerate() {
            let start = index * count;
            layers.push(TerrainLayer {
                name,
                weights: weights[start..start + count].to_vec(),
            });
        }
    }

    Terrain::new(
        width,
        height,
        horizontal_scale,
        vertical_scale,
        elevations,
        layers,
    )
}

/// Dimensions and scales a validated `HEAD` chunk declares.
type Header = (u32, u32, f32, f32, usize);

/// Decodes and bounds the `HEAD` chunk.
///
/// Every limit is applied here, before any payload-sized allocation is considered, so a header
/// claiming an enormous terrain is refused while the only thing read is 24 bytes.
fn decode_header(
    payload: &[u8],
    length: usize,
    limits: TerrainLimits,
) -> Result<Header, TerrainError> {
    if length != HEADER_PAYLOAD {
        return Err(TerrainError::ChunkLength {
            what: "HEAD",
            actual: length,
            expected: HEADER_PAYLOAD,
        });
    }
    let mut head = BinaryReader::new(payload, "terrain HEAD");
    let width = head.read_u32_le()?;
    let height = head.read_u32_le()?;
    let horizontal_scale = f32::from_bits(head.read_u32_le()?);
    let vertical_scale = f32::from_bits(head.read_u32_le()?);
    let layer_count = head.read_u32_le()? as usize;
    let _reserved = head.read_u32_le()?;

    if width == 0 || height == 0 {
        return Err(TerrainError::EmptyDimension { width, height });
    }
    if width > limits.maximum_dimension || height > limits.maximum_dimension {
        return Err(TerrainError::LimitExceeded {
            what: "terrain dimension",
            actual: width.max(height) as usize,
            maximum: limits.maximum_dimension as usize,
        });
    }
    if layer_count > limits.maximum_layers {
        return Err(TerrainError::LimitExceeded {
            what: "layer count",
            actual: layer_count,
            maximum: limits.maximum_layers,
        });
    }
    if !horizontal_scale.is_finite()
        || !vertical_scale.is_finite()
        || horizontal_scale <= 0.0
        || vertical_scale <= 0.0
    {
        return Err(TerrainError::Scale {
            horizontal: horizontal_scale,
            vertical: vertical_scale,
        });
    }
    let count = samples(width, height)?;
    if count > limits.maximum_samples {
        return Err(TerrainError::LimitExceeded {
            what: "sample count",
            actual: count,
            maximum: limits.maximum_samples,
        });
    }
    Ok((width, height, horizontal_scale, vertical_scale, layer_count))
}

fn samples(width: u32, height: u32) -> Result<usize, TerrainError> {
    (width as usize)
        .checked_mul(height as usize)
        .ok_or(TerrainError::SampleOverflow { width, height })
}

/// A structured failure while decoding or building terrain.
#[derive(Debug)]
pub enum TerrainError {
    /// The container magic was not `CICT`.
    Magic,
    /// The container declared a version this build does not implement.
    UnsupportedVersion(u32),
    /// A bounded read left the container.
    Binary(BinaryError),
    /// A required chunk was absent.
    MissingChunk(&'static str),
    /// A chunk's payload length disagreed with the declared dimensions.
    ChunkLength {
        /// Chunk tag.
        what: &'static str,
        /// Observed payload length.
        actual: usize,
        /// Length the header implies.
        expected: usize,
    },
    /// A width or height was zero.
    EmptyDimension {
        /// Declared width.
        width: u32,
        /// Declared height.
        height: u32,
    },
    /// `width * height` overflowed the address space.
    SampleOverflow {
        /// Declared width.
        width: u32,
        /// Declared height.
        height: u32,
    },
    /// A sample count did not match the declared dimensions.
    SampleCountMismatch {
        /// Which array disagreed.
        what: &'static str,
        /// Observed count.
        actual: usize,
        /// Count the dimensions imply.
        expected: usize,
    },
    /// A scale was zero, negative, or not finite.
    Scale {
        /// Declared horizontal scale.
        horizontal: f32,
        /// Declared vertical scale.
        vertical: f32,
    },
    /// A layer name was not valid UTF-8.
    NonUtf8LayerName(usize),
    /// The layer-name chunk held a different count than the header declared.
    LayerNameCount {
        /// Names decoded.
        actual: usize,
        /// Count the header declared.
        expected: usize,
    },
    /// An explicit [`TerrainLimits`] bound was exceeded.
    LimitExceeded {
        /// Which bound was crossed.
        what: &'static str,
        /// Observed value.
        actual: usize,
        /// Configured maximum.
        maximum: usize,
    },
}

impl From<BinaryError> for TerrainError {
    fn from(error: BinaryError) -> Self {
        Self::Binary(error)
    }
}

impl Display for TerrainError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Magic => formatter.write_str("terrain magic is not CICT"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported terrain version {version}")
            }
            Self::Binary(error) => Display::fmt(error, formatter),
            Self::MissingChunk(tag) => write!(formatter, "terrain is missing its {tag} chunk"),
            Self::ChunkLength {
                what,
                actual,
                expected,
            } => write!(
                formatter,
                "{what} chunk is {actual} bytes, expected {expected}"
            ),
            Self::EmptyDimension { width, height } => {
                write!(formatter, "terrain dimension {width}x{height} is empty")
            }
            Self::SampleOverflow { width, height } => {
                write!(
                    formatter,
                    "terrain {width}x{height} overflows the address space"
                )
            }
            Self::SampleCountMismatch {
                what,
                actual,
                expected,
            } => write!(formatter, "{what} count {actual} does not match {expected}"),
            Self::Scale {
                horizontal,
                vertical,
            } => write!(
                formatter,
                "terrain scales must be finite and positive, got {horizontal} and {vertical}"
            ),
            Self::NonUtf8LayerName(index) => {
                write!(formatter, "layer {index} name is not UTF-8")
            }
            Self::LayerNameCount { actual, expected } => write!(
                formatter,
                "decoded {actual} layer names but the header declared {expected}"
            ),
            Self::LimitExceeded {
                what,
                actual,
                maximum,
            } => write!(formatter, "{what} {actual} exceeds maximum {maximum}"),
        }
    }
}

impl Error for TerrainError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Binary(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    // Every float compared here is an exactly-representable constant the fixtures set directly
    // (0.0, 1.0, 10.0, 0.9, ...), so exact comparison is the correct assertion -- an epsilon would
    // weaken these tests rather than make them robust.
    #![allow(clippy::float_cmp)]
    // Fixture sizes are small, known constants, so the width casts below cannot truncate, and sample
    // indices are bounded by the dimension limit at 8,192, far inside exact f32 range.
    #![allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    use super::{
        MAGIC, Terrain, TerrainError, TerrainLayer, TerrainLimits, VERSION, decode_terrain,
    };

    /// Byte offset of the `HEAD` payload: 4 magic, 4 version, 4 chunk count, then the first
    /// chunk's 4-byte tag and 4-byte length. Named so a layout change breaks one constant rather
    /// than several hand-counted literals.
    const HEAD_PAYLOAD_START: usize = 20;
    const HEAD_WIDTH: usize = HEAD_PAYLOAD_START;
    const HEAD_HORIZONTAL_SCALE: usize = HEAD_PAYLOAD_START + 8;

    fn sample_terrain() -> Terrain {
        let elevations = (0..(4u16 * 3)).collect::<Vec<_>>();
        Terrain::new(
            4,
            3,
            10.0,
            0.5,
            elevations,
            vec![
                TerrainLayer {
                    name: "grass".to_owned(),
                    weights: vec![255; 12],
                },
                TerrainLayer {
                    name: "rock".to_owned(),
                    weights: (0..12).collect(),
                },
            ],
        )
        .expect("valid terrain")
    }

    #[test]
    fn round_trips_through_the_container() {
        let terrain = sample_terrain();
        let encoded = terrain.encode();
        assert_eq!(&encoded[0..4], MAGIC);
        let decoded = decode_terrain(&encoded, TerrainLimits::default()).expect("decode");
        assert_eq!(decoded, terrain);
    }

    #[test]
    fn round_trips_with_no_layers() {
        let terrain = Terrain::new(2, 2, 1.0, 1.0, vec![0, 1, 2, 3], Vec::new()).expect("valid");
        let decoded = decode_terrain(&terrain.encode(), TerrainLimits::default()).expect("decode");
        assert_eq!(decoded, terrain);
        assert!(decoded.layers().is_empty());
    }

    #[test]
    fn resolves_world_elevation_and_extent() {
        let terrain = sample_terrain();
        // Sample (1, 2) is row-major index 2*4+1 = 9, elevation 9, vertical scale 0.5.
        assert_eq!(terrain.elevation_at(1, 2), Some(4.5));
        assert_eq!(terrain.elevation_at(4, 0), None, "x is out of range");
        assert_eq!(terrain.elevation_at(0, 3), None, "y is out of range");
        // 4 samples span 3 intervals of 10, 3 samples span 2.
        assert_eq!(terrain.world_extent(), [30.0, 20.0]);
    }

    #[test]
    fn interpolates_world_elevation_between_samples() {
        // Spacing 10, vertical 0.5, elevations 0..11 row-major over a 4x3 grid.
        let terrain = sample_terrain();
        // Exactly on sample (1, 0): elevation 1, world 0.5.
        assert!((terrain.elevation_at_world(10.0, 0.0).expect("in range") - 0.5).abs() < 1.0e-4);
        // Halfway between samples (0, 0) and (1, 0): elevations 0 and 1, so 0.5 steps -> 0.25 world.
        assert!((terrain.elevation_at_world(5.0, 0.0).expect("in range") - 0.25).abs() < 1.0e-4);
        // Halfway along Y between rows 0 and 1 at x = 0: elevations 0 and 4 -> 2 steps -> 1.0 world.
        assert!((terrain.elevation_at_world(0.0, 5.0).expect("in range") - 1.0).abs() < 1.0e-4);
    }

    #[test]
    fn world_elevation_is_absent_outside_the_terrain() {
        // Absent rather than zero: a camera that reads missing ground as sea level dives through the
        // map at its edges.
        let terrain = sample_terrain();
        let [extent_x, extent_y] = terrain.world_extent();
        assert!(terrain.elevation_at_world(-0.1, 0.0).is_none());
        assert!(terrain.elevation_at_world(0.0, -0.1).is_none());
        assert!(terrain.elevation_at_world(extent_x + 0.1, 0.0).is_none());
        assert!(terrain.elevation_at_world(0.0, extent_y + 0.1).is_none());
        assert!(terrain.elevation_at_world(f32::NAN, 0.0).is_none());
        // The far corner is inclusive.
        assert!(terrain.elevation_at_world(extent_x, extent_y).is_some());
    }

    #[test]
    fn world_elevation_agrees_with_sample_elevation_on_the_grid() {
        let terrain = sample_terrain();
        for y in 0..terrain.height() {
            for x in 0..terrain.width() {
                let by_sample = terrain.elevation_at(x, y).expect("in range");
                let world_x = x as f32 * terrain.horizontal_scale();
                let world_y = y as f32 * terrain.horizontal_scale();
                let by_world = terrain
                    .elevation_at_world(world_x, world_y)
                    .expect("in range");
                assert!(
                    (by_sample - by_world).abs() < 1.0e-3,
                    "sample ({x}, {y}): {by_sample} against {by_world}"
                );
            }
        }
    }

    #[test]
    fn preserves_an_unknown_chunk_from_a_newer_writer() {
        // Forward compatibility: an unrecognized tag must be stepped over, not refused.
        let terrain = sample_terrain();
        let mut encoded = terrain.encode();
        // Bump the chunk count and append one unknown chunk with a 6-byte payload, which also
        // exercises the 4-byte padding on an unaligned length.
        let count = u32::from_le_bytes([encoded[8], encoded[9], encoded[10], encoded[11]]);
        encoded[8..12].copy_from_slice(&(count + 1).to_le_bytes());
        encoded.extend_from_slice(&u32::from_le_bytes(*b"XTRA").to_le_bytes());
        encoded.extend_from_slice(&6u32.to_le_bytes());
        encoded.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        encoded.extend_from_slice(&[0, 0]); // padding to 4
        let decoded = decode_terrain(&encoded, TerrainLimits::default()).expect("decode");
        assert_eq!(decoded, terrain);
    }

    #[test]
    fn rejects_wrong_magic_and_version() {
        let mut encoded = sample_terrain().encode();
        encoded[0] = b'X';
        assert!(matches!(
            decode_terrain(&encoded, TerrainLimits::default()),
            Err(TerrainError::Magic)
        ));

        let mut encoded = sample_terrain().encode();
        encoded[4..8].copy_from_slice(&(VERSION + 99).to_le_bytes());
        assert!(matches!(
            decode_terrain(&encoded, TerrainLimits::default()),
            Err(TerrainError::UnsupportedVersion(_))
        ));
    }

    #[test]
    fn rejects_a_height_chunk_that_disagrees_with_the_declared_dimensions() {
        let terrain = sample_terrain();
        let mut encoded = terrain.encode();
        // Claim 8x3 while the height payload still holds 4x3 samples. The HEAD payload starts at
        // byte 20: 4 magic, 4 version, 4 chunk count, then the chunk's own 4-byte tag and length.
        encoded[HEAD_WIDTH..HEAD_WIDTH + 4].copy_from_slice(&8u32.to_le_bytes());
        let error = decode_terrain(&encoded, TerrainLimits::default()).expect_err("must refuse");
        assert!(
            matches!(error, TerrainError::ChunkLength { what: "HGHT", .. }),
            "got {error:?}"
        );
    }

    #[test]
    fn refuses_a_dimension_past_the_limit_before_allocating() {
        let terrain = Terrain::new(4, 3, 1.0, 1.0, vec![0; 12], Vec::new()).expect("valid");
        let limits = TerrainLimits {
            maximum_dimension: 2,
            ..TerrainLimits::default()
        };
        let error = decode_terrain(&terrain.encode(), limits).expect_err("must refuse");
        assert!(
            matches!(
                error,
                TerrainError::LimitExceeded {
                    what: "terrain dimension",
                    ..
                }
            ),
            "got {error:?}"
        );
    }

    #[test]
    fn refuses_a_non_finite_or_non_positive_scale() {
        let terrain = sample_terrain();
        for bad in [0.0f32, -1.0, f32::NAN, f32::INFINITY] {
            let mut encoded = terrain.encode();
            encoded[HEAD_HORIZONTAL_SCALE..HEAD_HORIZONTAL_SCALE + 4]
                .copy_from_slice(&bad.to_bits().to_le_bytes());
            let error =
                decode_terrain(&encoded, TerrainLimits::default()).expect_err("must refuse {bad}");
            assert!(matches!(error, TerrainError::Scale { .. }), "got {error:?}");
        }
    }

    #[test]
    fn refuses_construction_when_a_layer_has_the_wrong_weight_count() {
        let error = Terrain::new(
            2,
            2,
            1.0,
            1.0,
            vec![0; 4],
            vec![TerrainLayer {
                name: "grass".to_owned(),
                weights: vec![0; 3],
            }],
        )
        .expect_err("must refuse");
        assert!(
            matches!(
                error,
                TerrainError::SampleCountMismatch {
                    what: "layer weights",
                    ..
                }
            ),
            "got {error:?}"
        );
    }

    #[test]
    fn refuses_a_truncated_container() {
        let encoded = sample_terrain().encode();
        for cut in [4, 8, 12, 20, encoded.len() - 4] {
            let error = decode_terrain(&encoded[..cut], TerrainLimits::default())
                .expect_err("truncation at {cut} must refuse");
            assert!(
                matches!(
                    error,
                    TerrainError::Binary(_) | TerrainError::ChunkLength { .. }
                ),
                "cut {cut} gave {error:?}"
            );
        }
    }

    #[test]
    fn encodes_a_realistic_heightfield_at_two_bytes_per_sample() {
        // The size argument for u16 over f32 and over JSON, asserted rather than claimed.
        let width = 512;
        let height = 512;
        let terrain = Terrain::new(
            width,
            height,
            10.0,
            0.25,
            vec![1_234; (width * height) as usize],
            Vec::new(),
        )
        .expect("valid");
        let encoded = terrain.encode();
        let samples = (width * height) as usize;
        let overhead = encoded.len() - samples * 2;
        assert!(
            overhead < 64,
            "container overhead should be a small fixed header, was {overhead}"
        );
    }
}
