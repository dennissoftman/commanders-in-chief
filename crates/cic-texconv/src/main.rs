//! `cic-texconv` — converts authored PNG textures to block-compressed DDS.
//!
//! ```text
//! cic-texconv --slot base hull_basecolor.png
//! cic-texconv --slot normal hull_normal.png -o textures/hull_normal.dds
//! ```
//!
//! # Why the slot is the only knob
//!
//! The three things that have to be right about a converted texture are its block format, its colour
//! space, and which channel means what — and all three follow from what the texture *is*. A base colour is
//! BC7 in sRGB; a normal map is BC5 in linear with `z` dropped; a packed occlusion/roughness/metallic map
//! is BC7 in linear with the channels in glTF's order.
//!
//! Exposing format and colour space as separate flags would let those be combined wrongly, and the two
//! wrong combinations are both quiet. A normal map converted as sRGB tilts every surface by the same
//! amount and reads as a lighting bug. A base colour converted as linear pales as the camera pulls back,
//! because its mip chain was averaged in the wrong space. Naming the slot instead makes the combination
//! unrepresentable, which is the same reason `ColourSpace` is a parameter rather than a convention in the
//! renderer.
//!
//! # What it writes
//!
//! One `.dds` with a DX10 header, a full mip chain down to 1x1, and the levels averaged in the slot's own
//! colour space by [`cic_assets::image`] — the same code the renderer uses for the textures it still mips
//! itself, so a converted texture and an unconverted one recede identically.
//!
//! Name the output after the glTF image it belongs to and put it in the package's `textures/` directory;
//! that name is the key the runtime looks it up by. See `cic_assets::resolve_model_textures`.

mod encode;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use cic_assets::image::mip_chain;
use cic_assets::texture::{BlockFormat, TextureAsset, TextureLimits};

/// What a texture is for, and therefore how it must be encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    /// Base colour or albedo: BC7, sRGB, alpha preserved.
    Base,
    /// Tangent-space normal map: BC5, linear, `z` dropped and rebuilt in the shader.
    Normal,
    /// Packed occlusion, roughness and metallic in R, G and B: BC7, linear.
    Orm,
    /// Flat or low-detail colour at half the bytes: BC1, sRGB, punch-through alpha.
    Bc1Colour,
    /// A single-channel or masked linear map at half the bytes: BC1, linear.
    Bc1Mask,
}

impl Slot {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "base" | "base-colour" | "base-color" | "albedo" => Some(Self::Base),
            "normal" => Some(Self::Normal),
            "orm" | "occlusion-roughness-metallic" | "metallic-roughness" => Some(Self::Orm),
            "bc1-colour" | "bc1-color" => Some(Self::Bc1Colour),
            "bc1-mask" | "mask" => Some(Self::Bc1Mask),
            _ => None,
        }
    }

    const fn format(self) -> BlockFormat {
        match self {
            Self::Base => BlockFormat::Bc7UnormSrgb,
            Self::Normal => BlockFormat::Bc5Unorm,
            Self::Orm => BlockFormat::Bc7Unorm,
            Self::Bc1Colour => BlockFormat::Bc1RgbaUnormSrgb,
            Self::Bc1Mask => BlockFormat::Bc1RgbaUnorm,
        }
    }

    /// What this slot is called in the usage text, and what the summary line prints.
    const fn name(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Normal => "normal",
            Self::Orm => "orm",
            Self::Bc1Colour => "bc1-colour",
            Self::Bc1Mask => "bc1-mask",
        }
    }
}

const USAGE: &str = "\
cic-texconv — convert an authored PNG texture to block-compressed DDS

usage:
    cic-texconv --slot <slot> <input.png> [-o <output.dds>]

slots:
    base         base colour or albedo      BC7, sRGB, alpha kept
    normal       tangent-space normal map   BC5, linear, z dropped
    orm          occlusion/roughness/metal  BC7, linear, glTF channel order
    bc1-colour   flat or low-detail colour  BC1, sRGB, punch-through alpha
    bc1-mask     linear mask or single map  BC1, linear

The output defaults to the input path with a .dds extension. Name it after the glTF
image it belongs to: that name is the key the runtime looks it up by.
";

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(summary) => {
            println!("{summary}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("cic-texconv: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Parses the arguments, converts, writes, and returns the line to print.
///
/// Separated from `main` so the whole tool is testable: everything but the process exit and the two
/// filesystem calls is in here.
fn run(arguments: Vec<String>) -> Result<String, String> {
    let mut slot = None;
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut rest = arguments.into_iter();
    while let Some(argument) = rest.next() {
        match argument.as_str() {
            "--help" | "-h" => return Ok(USAGE.to_owned()),
            "--slot" | "-s" => {
                let name = rest.next().ok_or("--slot needs a value")?;
                slot = Some(
                    Slot::parse(&name)
                        .ok_or_else(|| format!("unknown slot `{name}`\n\n{USAGE}"))?,
                );
            }
            "-o" | "--output" => {
                output = Some(PathBuf::from(rest.next().ok_or("-o needs a path")?));
            }
            flag if flag.starts_with('-') => {
                return Err(format!("unknown option `{flag}`\n\n{USAGE}"));
            }
            path if input.is_none() => input = Some(PathBuf::from(path)),
            extra => return Err(format!("unexpected argument `{extra}`\n\n{USAGE}")),
        }
    }

    let slot = slot.ok_or_else(|| format!("--slot is required\n\n{USAGE}"))?;
    let input = input.ok_or_else(|| format!("an input path is required\n\n{USAGE}"))?;
    let output = output.unwrap_or_else(|| input.with_extension("dds"));

    let bytes = std::fs::read(&input).map_err(|error| format!("{}: {error}", input.display()))?;
    let (width, height, rgba) =
        decode_png(&bytes).map_err(|error| format!("{}: {error}", input.display()))?;
    let texture = convert(&rgba, width, height, slot)
        .map_err(|error| format!("{}: {error}", input.display()))?;
    let encoded = texture.encode();
    let summary = summarize(&input, &output, slot, &texture, rgba.len(), encoded.len());
    std::fs::write(&output, &encoded).map_err(|error| format!("{}: {error}", output.display()))?;
    Ok(summary)
}

/// Compresses one decoded image into a mipped block-compressed texture.
///
/// # Errors
///
/// Returns a message when the image is empty or exceeds [`TextureLimits`].
fn convert(rgba: &[u8], width: u32, height: u32, slot: Slot) -> Result<TextureAsset, String> {
    let format = slot.format();
    // The chain is built in the slot's colour space, which is what makes a converted texture recede the
    // same way an unconverted one does. See `cic_assets::image`.
    let space = format.colour_space();
    let levels: Vec<Vec<u8>> = mip_chain(rgba, width, height, space)
        .into_iter()
        .map(|(level_width, level_height, pixels)| {
            encode::encode_level(&pixels, level_width, level_height, format)
        })
        .collect();
    if levels.is_empty() {
        return Err(format!("a {width}x{height} image has nothing to convert"));
    }
    TextureAsset::new(width, height, format, levels, TextureLimits::default())
        .map_err(|error| error.to_string())
}

/// The one line the tool prints on success: what it did, and what it saved.
fn summarize(
    input: &Path,
    output: &Path,
    slot: Slot,
    texture: &TextureAsset,
    source_bytes: usize,
    written: usize,
) -> String {
    // Against the *uncompressed mipped* size, because that is what the renderer would otherwise have
    // uploaded -- comparing against the base level alone would overstate the saving by a third.
    let uncompressed = source_bytes + source_bytes / 3;
    #[allow(clippy::cast_precision_loss)]
    let ratio = if written == 0 {
        0.0
    } else {
        uncompressed as f64 / written as f64
    };
    format!(
        "{} -> {} [{}] {}x{}, {} levels, {} as {}, {:.1}x smaller than RGBA8 with mips",
        input.display(),
        output.display(),
        slot.name(),
        texture.width(),
        texture.height(),
        texture.level_count(),
        human(written),
        texture.format().name(),
        ratio,
    )
}

/// A byte count as something a person reads.
fn human(bytes: usize) -> String {
    #[allow(clippy::cast_precision_loss)]
    let value = bytes as f64;
    if bytes >= 1_048_576 {
        format!("{:.1} MiB", value / 1_048_576.0)
    } else if bytes >= 1_024 {
        format!("{:.1} KiB", value / 1_024.0)
    } else {
        format!("{bytes} B")
    }
}

/// Decodes a PNG to straight-alpha RGBA8, whatever layout it was authored in.
///
/// Palette expansion and 16-bit narrowing are left to the decoder's own transformations; the channel
/// widening is done here, because the decoder will still hand back greyscale as one channel and this tool
/// needs four. A 16-bit source keeps its high byte, which is the same loss the glTF importer takes and for
/// the same reason: the destination is eight bits per channel either way.
fn decode_png(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    // A cursor rather than the slice itself: the decoder needs `Seek`, which `&[u8]` does not provide.
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
    let colour = reader.info().color_type;
    let mut buffer = vec![0u8; reader.output_buffer_size().unwrap_or(0)];
    let frame = reader
        .next_frame(&mut buffer)
        .map_err(|error| error.to_string())?;
    buffer.truncate(frame.buffer_size());

    let channels = match colour {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => {
            return Err("a palette should have been expanded by the decoder".to_owned());
        }
    };
    let mut rgba = Vec::with_capacity(buffer.len() / channels * 4);
    for pixel in buffer.chunks_exact(channels) {
        let first = pixel[0];
        rgba.push(first);
        // One and two channels mean greyscale, with the second channel as alpha where it exists.
        rgba.push(if channels >= 3 { pixel[1] } else { first });
        rgba.push(if channels >= 3 { pixel[2] } else { first });
        rgba.push(match channels {
            2 => pixel[1],
            4 => pixel[3],
            _ => u8::MAX,
        });
    }
    Ok((frame.width, frame.height, rgba))
}

#[cfg(test)]
mod tests {
    use super::{Slot, convert, decode_png, run};
    use cic_assets::image::ColourSpace;
    use cic_assets::texture::{BlockFormat, TextureLimits, decode_dds};

    /// Encodes a PNG the way an authoring tool would, so the decode path is exercised for real rather
    /// than handed bytes it never has to interpret.
    fn png(width: u32, height: u32, colour: png::ColorType, pixels: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, width, height);
            encoder.set_color(colour);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("header");
            writer.write_image_data(pixels).expect("pixels");
            writer.finish().expect("finish");
        }
        out
    }

    #[test]
    fn every_slot_pairs_one_format_with_one_colour_space() {
        // The whole reason the slot is the only knob. Both wrong combinations are quiet: an sRGB normal
        // map tilts every surface, and a linear base colour pales as the camera pulls back.
        assert_eq!(Slot::Base.format(), BlockFormat::Bc7UnormSrgb);
        assert_eq!(Slot::Base.format().colour_space(), ColourSpace::Srgb);
        assert_eq!(Slot::Normal.format(), BlockFormat::Bc5Unorm);
        assert_eq!(Slot::Normal.format().colour_space(), ColourSpace::Linear);
        assert_eq!(Slot::Orm.format(), BlockFormat::Bc7Unorm);
        assert_eq!(
            Slot::Orm.format().colour_space(),
            ColourSpace::Linear,
            "ORM is three measurements, not a colour"
        );
        assert_eq!(Slot::Bc1Colour.format().colour_space(), ColourSpace::Srgb);
        assert_eq!(Slot::Bc1Mask.format().colour_space(), ColourSpace::Linear);
    }

    #[test]
    fn the_names_an_author_would_type_all_resolve() {
        for (name, expected) in [
            ("base", Slot::Base),
            ("base-color", Slot::Base),
            ("albedo", Slot::Base),
            ("normal", Slot::Normal),
            ("orm", Slot::Orm),
            ("metallic-roughness", Slot::Orm),
            ("mask", Slot::Bc1Mask),
        ] {
            assert_eq!(Slot::parse(name), Some(expected), "{name}");
        }
        assert_eq!(Slot::parse("bc6h"), None);
    }

    #[test]
    fn a_converted_texture_carries_a_full_mip_chain_and_reads_back() {
        // End to end through the container: what the tool writes is what the runtime reads, including the
        // colour space, which is the one property nothing downstream could recover from the pixels.
        let rgba: Vec<u8> = (0..16 * 16)
            .flat_map(|index| {
                let value = u8::try_from(index % 256).unwrap_or(0);
                [value, 255 - value, 128, 255]
            })
            .collect();
        let texture = convert(&rgba, 16, 16, Slot::Base).expect("convert");
        assert_eq!(texture.level_count(), 5, "16x16 reaches 1x1 in five levels");
        let read = decode_dds(&texture.encode(), TextureLimits::default()).expect("read back");
        assert_eq!(read, texture);
        assert_eq!(read.format(), BlockFormat::Bc7UnormSrgb);
        // 8 bpp over the chain: 16 blocks, then 4, then 1, 1, 1.
        assert_eq!(read.byte_count(), (16 + 4 + 1 + 1 + 1) * 16);
    }

    #[test]
    fn a_normal_map_converts_to_two_channels_and_keeps_them_independent() {
        // The slot's whole purpose. Red varying one way and green the other is what a single shared
        // endpoint pair cannot follow, and BC5's two independent halves can.
        let rgba: Vec<u8> = (0..8 * 8)
            .flat_map(|index| {
                let x = u8::try_from((index % 8) * 32).unwrap_or(0);
                let y = u8::try_from((index / 8) * 32).unwrap_or(0);
                [x, y, 255, 255]
            })
            .collect();
        let texture = convert(&rgba, 8, 8, Slot::Normal).expect("convert");
        assert_eq!(texture.format(), BlockFormat::Bc5Unorm);
        let decoded = texture.decode();
        for (index, texel) in decoded.chunks_exact(4).enumerate() {
            let expected_x = u8::try_from((index % 8) * 32).unwrap_or(0);
            let expected_y = u8::try_from((index / 8) * 32).unwrap_or(0);
            assert!(
                texel[0].abs_diff(expected_x) <= 8 && texel[1].abs_diff(expected_y) <= 8,
                "texel {index} came back {texel:?}, wanted about ({expected_x}, {expected_y})"
            );
        }
    }

    #[test]
    fn a_greyscale_or_rgb_source_is_widened_to_four_channels() {
        // An occlusion map is routinely authored as a single channel and a base colour as RGB with no
        // alpha. Both must convert without the author having to widen them first.
        let (width, height, rgba) =
            decode_png(&png(2, 1, png::ColorType::Grayscale, &[40, 200])).expect("grey");
        assert_eq!((width, height), (2, 1));
        assert_eq!(rgba, [40, 40, 40, 255, 200, 200, 200, 255]);

        let (_, _, rgba) = decode_png(&png(1, 1, png::ColorType::Rgb, &[10, 20, 30])).expect("rgb");
        assert_eq!(rgba, [10, 20, 30, 255]);

        let (_, _, rgba) =
            decode_png(&png(1, 1, png::ColorType::GrayscaleAlpha, &[90, 12])).expect("grey+alpha");
        assert_eq!(rgba, [90, 90, 90, 12], "the second channel is alpha");
    }

    #[test]
    fn the_command_line_reports_what_it_cannot_do_rather_than_guessing() {
        assert!(run(vec![]).is_err(), "no arguments at all");
        assert!(
            run(vec!["--slot".to_owned()]).is_err(),
            "a flag with no value"
        );
        let error = run(vec![
            "--slot".to_owned(),
            "bc6h".to_owned(),
            "a.png".to_owned(),
        ])
        .expect_err("an unknown slot must be refused rather than defaulted");
        assert!(error.contains("unknown slot"), "got {error}");
        let error = run(vec!["-x".to_owned()]).expect_err("an unknown option");
        assert!(error.contains("unknown option"), "got {error}");
        assert!(
            run(vec!["--help".to_owned()])
                .expect("help is not a failure")
                .contains("usage:")
        );
        // A missing input file reports the path, so a mistyped name is obvious.
        let error = run(vec![
            "--slot".to_owned(),
            "base".to_owned(),
            "definitely/not/here.png".to_owned(),
        ])
        .expect_err("a missing input");
        assert!(error.contains("definitely/not/here.png"), "got {error}");
    }
}
