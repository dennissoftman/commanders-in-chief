//! A bounded RIFF/WAVE decoder.
//!
//! # Why WAV, when the engine will want a compressed format too
//!
//! Because it is the format that costs nothing to be *correct* about, and every compressed format costs
//! a great deal. A Vorbis or Opus decoder is thousands of lines of entropy coding and transform maths,
//! and this project's rule is that a decoder treats its input as hostile and cannot panic — a bar that
//! is a weekend for a chunked PCM container and a research project for a subband codec.
//!
//! So this is the format the engine reads itself, and a compressed one arrives later as a dependency
//! with its own `NOTICE` entry, decoding to the same [`Clip`] this produces. Uncompressed short effects
//! are also what a game actually wants in memory: decoding a gunshot on every trigger is worse than
//! storing it.
//!
//! # What this refuses
//!
//! Every rule in [the binary parsing invariants](../../../docs/invariants/binary-parsing.md) applies.
//! Two are worth naming because they are the ones a WAV reader usually gets wrong:
//!
//! **The declared data size is checked before the sample buffer is allocated.** A 44-byte file whose
//! `data` header claims four gigabytes is refused while only the header has been read. Reading first and
//! checking after would mean the refusal costs the allocation it exists to prevent.
//!
//! **An unknown chunk is skipped, and an unknown format tag is not.** They look like the same kind of
//! unknown and they are not: a `LIST` chunk carrying an authoring tool's name has no bearing on the
//! samples, while a format tag names how every byte in `data` is to be interpreted. Guessing at the
//! first costs nothing and guessing at the second produces noise.

use cic_core::{BinaryError, BinaryReader};

use crate::sample::{Clip, ClipError, ClipLimits, MAX_CHANNELS};

use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// Largest number of chunks the reader will walk before giving up.
///
/// A file can legitimately carry a dozen; a file carrying thousands of empty chunks is a decoder
/// denial-of-service rather than content, and the walk is what it targets.
const MAX_CHUNKS: usize = 256;

/// Uncompressed integer samples.
const FORMAT_PCM: u16 = 1;
/// IEEE 754 single-precision samples.
const FORMAT_FLOAT: u16 = 3;
/// The extensible header, whose real format tag is the first two bytes of its sub-format GUID.
const FORMAT_EXTENSIBLE: u16 = 0xFFFE;

/// A failure decoding a WAVE file.
#[derive(Debug, Clone, PartialEq)]
pub enum WavError {
    /// A bounded read failed.
    Read(BinaryError),
    /// The file did not begin `RIFF`, or its form was not `WAVE`.
    NotWave {
        /// The four bytes found where the marker was expected.
        found: [u8; 4],
    },
    /// A required chunk was absent.
    MissingChunk {
        /// The four-character identifier that was not found.
        id: &'static str,
    },
    /// More chunks were encountered than the reader will walk before giving up.
    TooManyChunks,
    /// The `fmt ` chunk was shorter than the fields it must contain.
    ShortFormatChunk {
        /// Length the chunk declared.
        length: u32,
    },
    /// The format tag names a compression this decoder does not implement.
    UnsupportedFormat {
        /// The tag found.
        tag: u16,
    },
    /// The sample width is not one this decoder implements for its format tag.
    UnsupportedBitDepth {
        /// Bits per sample found.
        bits: u16,
        /// Format tag they were found under.
        tag: u16,
    },
    /// The `data` chunk holds a partial frame.
    RaggedData {
        /// Bytes the chunk holds.
        bytes: usize,
        /// Bytes one frame occupies.
        frame_bytes: usize,
    },
    /// A limit was crossed. Reported before the allocation it bounds.
    Limit {
        /// Name of the limited quantity.
        what: &'static str,
        /// Value the file declared.
        actual: usize,
        /// Largest value accepted.
        maximum: usize,
    },
    /// The decoded samples did not form a valid clip.
    Clip(ClipError),
}

impl Display for WavError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(error) => write!(formatter, "{error}"),
            Self::NotWave { found } => {
                write!(formatter, "not a WAVE file: found {}", Marker(*found))
            }
            Self::MissingChunk { id } => write!(formatter, "required chunk `{id}` is absent"),
            Self::TooManyChunks => {
                write!(formatter, "more than {MAX_CHUNKS} chunks before `data`")
            }
            Self::ShortFormatChunk { length } => write!(
                formatter,
                "`fmt ` chunk of {length} bytes is shorter than the 16 bytes it must contain"
            ),
            Self::UnsupportedFormat { tag } => {
                write!(formatter, "unsupported WAVE format tag {tag}")
            }
            Self::UnsupportedBitDepth { bits, tag } => write!(
                formatter,
                "unsupported sample width of {bits} bits under format tag {tag}"
            ),
            Self::RaggedData { bytes, frame_bytes } => write!(
                formatter,
                "`data` chunk of {bytes} bytes does not divide into whole frames of {frame_bytes} bytes"
            ),
            Self::Limit {
                what,
                actual,
                maximum,
            } => write!(
                formatter,
                "{what} value {actual} exceeds the configured limit {maximum}"
            ),
            Self::Clip(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for WavError {}

impl From<BinaryError> for WavError {
    fn from(error: BinaryError) -> Self {
        Self::Read(error)
    }
}

impl From<ClipError> for WavError {
    fn from(error: ClipError) -> Self {
        Self::Clip(error)
    }
}

/// Renders a four-byte chunk identifier readably, whatever it holds.
struct Marker([u8; 4]);

impl Display for Marker {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            if byte.is_ascii_graphic() || byte == b' ' {
                write!(formatter, "{}", char::from(byte))?;
            } else {
                write!(formatter, "\\x{byte:02x}")?;
            }
        }
        Ok(())
    }
}

/// What the `fmt ` chunk declared.
#[derive(Debug, Clone, Copy)]
struct Format {
    tag: u16,
    channels: u16,
    sample_rate: u32,
    bits: u16,
}

impl Format {
    /// Bytes one frame of this format occupies.
    fn frame_bytes(self) -> usize {
        usize::from(self.channels) * (usize::from(self.bits) / 8)
    }
}

/// Decodes a WAVE file into a [`Clip`].
///
/// `source` names the input in any error raised, so a failure says which file was bad rather than only
/// that one was.
///
/// # Errors
///
/// Returns [`WavError`] for a wrong marker, a missing or short `fmt ` chunk, a format or sample width
/// this decoder does not implement, a `data` chunk holding a partial frame, or any limit in `limits`
/// being crossed. Limits are reported before the allocation they bound.
pub fn decode(bytes: &[u8], source: &str, limits: ClipLimits) -> Result<Clip, WavError> {
    let mut reader = BinaryReader::new(bytes, source);

    let riff = read_marker(&mut reader)?;
    if &riff != b"RIFF" {
        return Err(WavError::NotWave { found: riff });
    }
    // The declared RIFF length is deliberately not trusted as a bound. It is frequently wrong in real
    // files -- a recorder that crashes mid-write leaves the header claiming the length it intended --
    // and the slice the caller supplied is the real bound in every case.
    let _declared_length = reader.read_u32_le()?;
    let wave = read_marker(&mut reader)?;
    if &wave != b"WAVE" {
        return Err(WavError::NotWave { found: wave });
    }

    let mut format: Option<Format> = None;
    let mut chunks = 0usize;

    loop {
        if reader.remaining() < 8 {
            // A well-formed file ends exactly at a chunk boundary; a few trailing bytes are the
            // ordinary result of a padded write and not worth refusing a file over.
            break;
        }
        chunks += 1;
        if chunks > MAX_CHUNKS {
            return Err(WavError::TooManyChunks);
        }

        let id = read_marker(&mut reader)?;
        let declared = reader.read_u32_le()?;
        let length = usize::try_from(declared).unwrap_or(usize::MAX);

        match &id {
            b"fmt " => {
                let mut chunk = reader.read_region(length.min(reader.remaining()))?;
                format = Some(read_format(&mut chunk, declared)?);
            }
            b"data" => {
                let format = format.ok_or(WavError::MissingChunk { id: "fmt " })?;
                // The bound is the smaller of what the chunk declares and what is actually present,
                // and it is applied before any buffer is sized. A header claiming four gigabytes over
                // a 44-byte file is refused here, having allocated nothing.
                let available = length.min(reader.remaining());
                return decode_data(&mut reader, format, available, limits);
            }
            _ => {
                // Forward compatibility, exactly as the chunked container invariant requires: a chunk
                // this build does not know about is skipped rather than refused, so a file carrying a
                // newer annotation still plays.
                let skip = length.min(reader.remaining());
                reader.skip(skip)?;
            }
        }

        // RIFF pads every odd-length chunk to an even boundary, and the pad byte is not counted in the
        // declared length. Missing this reads every subsequent chunk identifier one byte late.
        if length % 2 == 1 && reader.remaining() > 0 {
            reader.skip(1)?;
        }
    }

    Err(WavError::MissingChunk { id: "data" })
}

/// Reads a four-byte chunk identifier.
fn read_marker(reader: &mut BinaryReader<'_>) -> Result<[u8; 4], BinaryError> {
    let bytes = reader.read_exact(4)?;
    let mut marker = [0u8; 4];
    marker.copy_from_slice(bytes);
    Ok(marker)
}

/// Reads the `fmt ` chunk's fields, resolving an extensible header to its real format tag.
fn read_format(chunk: &mut BinaryReader<'_>, declared: u32) -> Result<Format, WavError> {
    if chunk.len() < 16 {
        return Err(WavError::ShortFormatChunk { length: declared });
    }

    let mut tag = chunk.read_u16_le()?;
    let channels = chunk.read_u16_le()?;
    let sample_rate = chunk.read_u32_le()?;
    let _byte_rate = chunk.read_u32_le()?;
    let _block_align = chunk.read_u16_le()?;
    let bits = chunk.read_u16_le()?;

    if tag == FORMAT_EXTENSIBLE {
        // WAVE_FORMAT_EXTENSIBLE moves the real tag into the first two bytes of a sub-format GUID,
        // after a `cbSize` and 22 bytes of channel-mask and valid-bits fields. Files written by
        // modern tools at more than 16 bits are routinely extensible, so refusing it would refuse
        // most 24-bit content.
        let _cb_size = chunk.read_u16_le()?;
        let _valid_bits = chunk.read_u16_le()?;
        let _channel_mask = chunk.read_u32_le()?;
        tag = chunk.read_u16_le()?;
    }

    if tag != FORMAT_PCM && tag != FORMAT_FLOAT {
        return Err(WavError::UnsupportedFormat { tag });
    }
    let supported = match tag {
        FORMAT_FLOAT => bits == 32,
        _ => matches!(bits, 8 | 16 | 24 | 32),
    };
    if !supported {
        return Err(WavError::UnsupportedBitDepth { bits, tag });
    }
    if channels == 0 || channels > MAX_CHANNELS {
        return Err(WavError::Limit {
            what: "channel count",
            actual: usize::from(channels),
            maximum: usize::from(MAX_CHANNELS),
        });
    }

    Ok(Format {
        tag,
        channels,
        sample_rate,
        bits,
    })
}

/// Converts `available` bytes of `data` into a clip, having checked every limit first.
fn decode_data(
    reader: &mut BinaryReader<'_>,
    format: Format,
    available: usize,
    limits: ClipLimits,
) -> Result<Clip, WavError> {
    let frame_bytes = format.frame_bytes();
    if frame_bytes == 0 {
        return Err(WavError::UnsupportedBitDepth {
            bits: format.bits,
            tag: format.tag,
        });
    }
    if !available.is_multiple_of(frame_bytes) {
        return Err(WavError::RaggedData {
            bytes: available,
            frame_bytes,
        });
    }

    // Every limit is checked here, before `Vec::with_capacity` below sees a count derived from the
    // file. This ordering is the invariant, not a style preference.
    let frames = available / frame_bytes;
    if frames > limits.max_frames {
        return Err(WavError::Limit {
            what: "frame count",
            actual: frames,
            maximum: limits.max_frames,
        });
    }
    let maximum_channels = limits.max_channels.min(MAX_CHANNELS);
    if format.channels > maximum_channels {
        return Err(WavError::Limit {
            what: "channel count",
            actual: usize::from(format.channels),
            maximum: usize::from(maximum_channels),
        });
    }
    if format.sample_rate == 0 || format.sample_rate > limits.max_sample_rate {
        return Err(WavError::Limit {
            what: "sample rate",
            actual: format.sample_rate as usize,
            maximum: limits.max_sample_rate as usize,
        });
    }

    let payload = reader.read_exact(available)?;
    let count = frames * usize::from(format.channels);
    let mut samples = Vec::with_capacity(count);

    match (format.tag, format.bits) {
        (FORMAT_FLOAT, _) => {
            for chunk in payload.chunks_exact(4) {
                let bits = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                let value = f32::from_bits(bits);
                // A NaN or an infinity in the payload would propagate through every sum in the mixer
                // and silence the whole master bus, so it is clamped at the boundary where it enters.
                samples.push(if value.is_finite() {
                    value.clamp(-1.0, 1.0)
                } else {
                    0.0
                });
            }
        }
        (_, 8) => {
            // Eight-bit WAV is *unsigned*, biased by 128. Reading it as signed inverts the waveform
            // and shifts it by full scale, which is audible as a loud click rather than as a subtlety.
            for &byte in payload {
                samples.push((f32::from(byte) - 128.0) / 128.0);
            }
        }
        (_, 16) => {
            for chunk in payload.chunks_exact(2) {
                let value = i16::from_le_bytes([chunk[0], chunk[1]]);
                samples.push(f32::from(value) / 32_768.0);
            }
        }
        (_, 24) => {
            for chunk in payload.chunks_exact(3) {
                // Sign-extend by placing the three bytes in the *high* 24 bits of an `i32` and
                // shifting back down arithmetically, which avoids a branch on the sign bit.
                let value = i32::from_le_bytes([0, chunk[0], chunk[1], chunk[2]]) >> 8;
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "24 bits fit an `f32` mantissa exactly, so this conversion is lossless"
                )]
                samples.push(value as f32 / 8_388_608.0);
            }
        }
        (_, _) => {
            for chunk in payload.chunks_exact(4) {
                let value = i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "32-bit integer audio exceeds an `f32` mantissa by 8 bits, which is 48 dB \
                              below the noise floor of any recording that would be stored this way"
                )]
                samples.push(value as f32 / 2_147_483_648.0);
            }
        }
    }

    Ok(Clip::new(
        format.sample_rate,
        format.channels,
        samples,
        limits,
    )?)
}

#[cfg(test)]
pub(crate) mod testing {
    //! Builders for real WAVE files.
    //!
    //! Fixtures are constructed rather than committed, which is this project's testing posture: a test
    //! that states "a `data` chunk claiming four gigabytes" in code is reviewable, and a blob is not.

    /// Assembles a WAVE file from a format tag, a sample width, and raw payload bytes.
    pub fn wave(tag: u16, channels: u16, sample_rate: u32, bits: u16, payload: &[u8]) -> Vec<u8> {
        let mut format = Vec::new();
        format.extend_from_slice(&tag.to_le_bytes());
        format.extend_from_slice(&channels.to_le_bytes());
        format.extend_from_slice(&sample_rate.to_le_bytes());
        let block_align = u32::from(channels) * u32::from(bits) / 8;
        format.extend_from_slice(&(sample_rate * block_align).to_le_bytes());
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a fixture's block alignment is small by construction"
        )]
        format.extend_from_slice(&(block_align as u16).to_le_bytes());
        format.extend_from_slice(&bits.to_le_bytes());

        let mut chunks = Vec::new();
        chunks.extend_from_slice(b"fmt ");
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a fixture's format chunk is 16 bytes"
        )]
        chunks.extend_from_slice(&(format.len() as u32).to_le_bytes());
        chunks.extend_from_slice(&format);
        chunks.extend_from_slice(b"data");
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a fixture's payload is small by construction"
        )]
        chunks.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        chunks.extend_from_slice(payload);

        riff(&chunks)
    }

    /// Wraps assembled chunks in a `RIFF`/`WAVE` header.
    pub fn riff(chunks: &[u8]) -> Vec<u8> {
        let mut file = Vec::new();
        file.extend_from_slice(b"RIFF");
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a fixture is small by construction"
        )]
        file.extend_from_slice(&((chunks.len() + 4) as u32).to_le_bytes());
        file.extend_from_slice(b"WAVE");
        file.extend_from_slice(chunks);
        file
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::testing::{riff, wave};
    use super::{FORMAT_FLOAT, FORMAT_PCM, WavError, decode};
    use crate::sample::ClipLimits;

    #[test]
    fn a_sixteen_bit_stereo_file_decodes_to_the_samples_it_holds() {
        let mut payload = Vec::new();
        for (left, right) in [(0i16, 0i16), (i16::MAX, i16::MIN)] {
            payload.extend_from_slice(&left.to_le_bytes());
            payload.extend_from_slice(&right.to_le_bytes());
        }
        let file = wave(FORMAT_PCM, 2, 48_000, 16, &payload);

        let clip = decode(&file, "fixture.wav", ClipLimits::DEFAULT).expect("decode");
        assert_eq!(clip.sample_rate(), 48_000);
        assert_eq!(clip.channels(), 2);
        assert_eq!(clip.frames(), 2);
        assert_eq!(clip.stereo_frame(0), [0.0, 0.0]);
        assert!((clip.stereo_frame(1)[0] - 0.999_97).abs() < 1e-4);
        assert!((clip.stereo_frame(1)[1] + 1.0).abs() < 1e-6);
    }

    #[test]
    fn eight_bit_samples_are_read_as_unsigned() {
        // Reading these as signed inverts the waveform and offsets it by full scale. The midpoint of
        // an 8-bit WAV is 128, and 128 must decode to silence.
        let file = wave(FORMAT_PCM, 1, 22_050, 8, &[128, 255, 0]);
        let clip = decode(&file, "fixture.wav", ClipLimits::DEFAULT).expect("decode");
        assert_eq!(clip.stereo_frame(0)[0], 0.0);
        assert!(clip.stereo_frame(1)[0] > 0.99);
        assert_eq!(clip.stereo_frame(2)[0], -1.0);
    }

    #[test]
    fn twenty_four_bit_samples_are_sign_extended() {
        // 0xFFFFFF is -1 in 24-bit two's complement. Read without sign extension it is the largest
        // positive value instead, which is the loudest possible way to be wrong.
        let file = wave(
            FORMAT_PCM,
            1,
            48_000,
            24,
            &[0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x40],
        );
        let clip = decode(&file, "fixture.wav", ClipLimits::DEFAULT).expect("decode");
        assert!((clip.stereo_frame(0)[0] + 1.0 / 8_388_608.0).abs() < 1e-9);
        assert!((clip.stereo_frame(1)[0] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_non_finite_float_sample_is_replaced_rather_than_propagated() {
        // One NaN summed into the master bus silences every voice on it. Clamping at the boundary is
        // what keeps a malformed file from taking out the mix.
        let mut payload = Vec::new();
        payload.extend_from_slice(&f32::NAN.to_bits().to_le_bytes());
        payload.extend_from_slice(&f32::INFINITY.to_bits().to_le_bytes());
        payload.extend_from_slice(&7.5f32.to_bits().to_le_bytes());
        let file = wave(FORMAT_FLOAT, 1, 48_000, 32, &payload);

        let clip = decode(&file, "fixture.wav", ClipLimits::DEFAULT).expect("decode");
        assert_eq!(clip.stereo_frame(0)[0], 0.0);
        assert_eq!(clip.stereo_frame(1)[0], 0.0);
        assert_eq!(clip.stereo_frame(2)[0], 1.0, "out of range is clamped");
    }

    #[test]
    fn an_unknown_chunk_is_skipped_and_an_odd_one_is_padded() {
        // Forward compatibility. An authoring tool's annotation must not stop the file playing, and
        // the pad byte after an odd-length chunk is what a decoder that ignores alignment gets wrong:
        // every identifier after it reads one byte late.
        let mut chunks = Vec::new();
        chunks.extend_from_slice(b"LIST");
        chunks.extend_from_slice(&3u32.to_le_bytes());
        chunks.extend_from_slice(b"abc");
        chunks.push(0); // the pad byte

        let inner = wave(FORMAT_PCM, 1, 48_000, 16, &[0x00, 0x40]);
        chunks.extend_from_slice(&inner[12..]);

        let clip = decode(&riff(&chunks), "fixture.wav", ClipLimits::DEFAULT).expect("decode");
        assert_eq!(clip.frames(), 1);
        assert!((clip.stereo_frame(0)[0] - 0.5).abs() < 1e-4);
    }

    #[test]
    fn an_extensible_header_resolves_to_its_sub_format_tag() {
        let mut format = Vec::new();
        format.extend_from_slice(&super::FORMAT_EXTENSIBLE.to_le_bytes());
        format.extend_from_slice(&1u16.to_le_bytes());
        format.extend_from_slice(&48_000u32.to_le_bytes());
        format.extend_from_slice(&96_000u32.to_le_bytes());
        format.extend_from_slice(&2u16.to_le_bytes());
        format.extend_from_slice(&16u16.to_le_bytes());
        format.extend_from_slice(&22u16.to_le_bytes());
        format.extend_from_slice(&16u16.to_le_bytes());
        format.extend_from_slice(&4u32.to_le_bytes());
        format.extend_from_slice(&FORMAT_PCM.to_le_bytes());
        format.extend_from_slice(&[0u8; 14]);

        let mut chunks = Vec::new();
        chunks.extend_from_slice(b"fmt ");
        #[expect(clippy::cast_possible_truncation, reason = "fixture")]
        chunks.extend_from_slice(&(format.len() as u32).to_le_bytes());
        chunks.extend_from_slice(&format);
        chunks.extend_from_slice(b"data");
        chunks.extend_from_slice(&2u32.to_le_bytes());
        chunks.extend_from_slice(&[0x00, 0x40]);

        let clip = decode(&riff(&chunks), "fixture.wav", ClipLimits::DEFAULT).expect("decode");
        assert_eq!(clip.frames(), 1);
    }

    #[test]
    fn a_data_chunk_claiming_four_gigabytes_is_refused_before_allocating() {
        // The invariant this file exists to demonstrate. The whole file is 46 bytes; the header says
        // the payload is four gigabytes. It must be refused, and refused without having tried.
        let mut chunks = Vec::new();
        chunks.extend_from_slice(b"fmt ");
        chunks.extend_from_slice(&16u32.to_le_bytes());
        chunks.extend_from_slice(&FORMAT_PCM.to_le_bytes());
        chunks.extend_from_slice(&1u16.to_le_bytes());
        chunks.extend_from_slice(&48_000u32.to_le_bytes());
        chunks.extend_from_slice(&96_000u32.to_le_bytes());
        chunks.extend_from_slice(&2u16.to_le_bytes());
        chunks.extend_from_slice(&16u16.to_le_bytes());
        chunks.extend_from_slice(b"data");
        chunks.extend_from_slice(&u32::MAX.to_le_bytes());
        chunks.extend_from_slice(&[0x00, 0x40]);

        // The declared size is clamped to what is present, so this decodes the one frame that is
        // really there rather than trusting the header.
        let clip = decode(&riff(&chunks), "liar.wav", ClipLimits::DEFAULT).expect("decode");
        assert_eq!(clip.frames(), 1);

        // And with a limit below what is present, the refusal names the limit.
        let strict = ClipLimits {
            max_frames: 0,
            ..ClipLimits::DEFAULT
        };
        assert!(matches!(
            decode(&riff(&chunks), "liar.wav", strict),
            Err(WavError::Limit {
                what: "frame count",
                ..
            })
        ));
    }

    #[test]
    fn a_wrong_marker_is_refused() {
        assert!(matches!(
            decode(b"RIFX\0\0\0\0WAVE", "bad.wav", ClipLimits::DEFAULT),
            Err(WavError::NotWave { .. })
        ));
        assert!(matches!(
            decode(b"RIFF\0\0\0\0AVI ", "bad.wav", ClipLimits::DEFAULT),
            Err(WavError::NotWave { .. })
        ));
    }

    #[test]
    fn an_unsupported_format_or_width_is_refused_rather_than_guessed_at() {
        // Unlike an unknown chunk, a format tag names how every byte of `data` is to be read, so
        // skipping past the question would produce noise at full volume.
        let adpcm = wave(17, 1, 48_000, 4, &[0, 0]);
        assert!(matches!(
            decode(&adpcm, "adpcm.wav", ClipLimits::DEFAULT),
            Err(WavError::UnsupportedFormat { tag: 17 })
        ));

        let float16 = wave(FORMAT_FLOAT, 1, 48_000, 16, &[0, 0]);
        assert!(matches!(
            decode(&float16, "f16.wav", ClipLimits::DEFAULT),
            Err(WavError::UnsupportedBitDepth {
                bits: 16,
                tag: FORMAT_FLOAT
            })
        ));
    }

    #[test]
    fn truncation_at_several_offsets_is_an_error_and_never_a_panic() {
        let file = wave(FORMAT_PCM, 2, 48_000, 16, &[0x00, 0x40, 0x00, 0x40]);
        for length in 0..file.len() {
            // The contract is that every prefix produces a structured error rather than a panic or a
            // wrong clip. Some prefixes legitimately decode -- a truncated payload is still a whole
            // number of frames -- and what must never happen is the third outcome.
            let outcome = decode(&file[..length], "truncated.wav", ClipLimits::DEFAULT);
            if let Ok(clip) = outcome {
                assert!(clip.frames() <= 1, "a prefix cannot hold more than it has");
            }
        }
    }

    #[test]
    fn a_partial_frame_in_the_data_chunk_is_refused() {
        let file = wave(FORMAT_PCM, 2, 48_000, 16, &[0x00, 0x40, 0x00]);
        assert!(matches!(
            decode(&file, "ragged.wav", ClipLimits::DEFAULT),
            Err(WavError::RaggedData {
                bytes: 3,
                frame_bytes: 4
            })
        ));
    }

    #[test]
    fn a_file_without_a_data_chunk_names_what_is_missing() {
        let mut chunks = Vec::new();
        chunks.extend_from_slice(b"fmt ");
        chunks.extend_from_slice(&16u32.to_le_bytes());
        chunks.extend_from_slice(&[0u8; 16]);
        assert!(matches!(
            decode(&riff(&chunks), "nodata.wav", ClipLimits::DEFAULT),
            Err(WavError::MissingChunk { id: "data" } | WavError::UnsupportedFormat { .. })
        ));
    }

    #[test]
    fn a_file_of_thousands_of_empty_chunks_is_refused_rather_than_walked() {
        let mut chunks = Vec::new();
        for _ in 0..=super::MAX_CHUNKS {
            chunks.extend_from_slice(b"junk");
            chunks.extend_from_slice(&0u32.to_le_bytes());
        }
        assert!(matches!(
            decode(&riff(&chunks), "dos.wav", ClipLimits::DEFAULT),
            Err(WavError::TooManyChunks)
        ));
    }
}
