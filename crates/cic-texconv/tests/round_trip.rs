//! The converter end to end on a picture, with the result written out to be looked at.
//!
//! The unit tests in `encode` measure the encoder on fixtures chosen to isolate one property each — a
//! collinear ramp, two independent channels, a hard edge. This is the other half of the same job: an image
//! with all of those at once, converted through the real container, read back through the real reader, and
//! written to `target/tmp` as a PNG.
//!
//! That last part is deliberate and is this project's standing rule: a green assertion is not verification
//! for anything that produces an image. Every rendering bug here so far passed its own tests and was caught
//! by opening the capture. A compressor is the same kind of thing — a blocky artefact, a shifted channel or
//! a mip level in the wrong place all satisfy a decibel threshold that was set generously.

use std::path::PathBuf;

use cic_assets::texture::{BlockFormat, TextureLimits, decode_dds};

/// A 256-pixel test image holding, at once, everything a block encoder finds easy and hard.
///
/// Quadrants: a smooth correlated gradient, two independently varying channels, hard-edged blocks of flat
/// colour, and fine one-texel detail. A single number over this image says more than four numbers over
/// four fixtures, because a real texture is all of them at different places.
fn picture() -> (u32, Vec<u8>) {
    const SIZE: u32 = 256;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (left, top) = (x < SIZE / 2, y < SIZE / 2);
            let texel: [u8; 4] = match (left, top) {
                // Smooth and correlated: what one line through colour space fits well.
                (true, true) => {
                    let ramp = u8::try_from(u32::midpoint(x, y)).unwrap_or(255);
                    [ramp, ramp / 2 + 40, 255 - ramp, 255]
                }
                // Independent channels: what one line cannot fit at all.
                (false, true) => [
                    u8::try_from(x % 256).unwrap_or(0),
                    u8::try_from((y * 2) % 256).unwrap_or(0),
                    90,
                    255,
                ],
                // Hard-edged flat blocks: what a partitioned mode would win on.
                (true, false) => {
                    if (x / 16 + y / 16) % 2 == 0 {
                        [200, 40, 60, 255]
                    } else {
                        [30, 90, 180, 255]
                    }
                }
                // One-texel detail, which no 4x4 block can hold and every encoder must degrade gracefully.
                (false, false) => {
                    if (x + y) % 2 == 0 {
                        [240, 240, 230, 255]
                    } else {
                        [25, 20, 30, 255]
                    }
                }
            };
            rgba.extend_from_slice(&texel);
        }
    }
    (SIZE, rgba)
}

fn write_png(name: &str, width: u32, height: u32, rgba: &[u8]) {
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let mut file = std::fs::File::create(&path).expect("create png");
    let mut encoder = png::Encoder::new(&mut file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("header");
    writer.write_image_data(rgba).expect("pixels");
    writer.finish().expect("finish");
    eprintln!("wrote {}", path.display());
}

fn psnr(original: &[u8], decoded: &[u8]) -> f64 {
    let mut total = 0.0f64;
    let mut count = 0usize;
    for (a, b) in original.iter().zip(decoded) {
        let difference = f64::from(*a) - f64::from(*b);
        total += difference * difference;
        count += 1;
    }
    #[allow(clippy::cast_precision_loss)]
    let mean = total / count.max(1) as f64;
    if mean == 0.0 {
        return f64::INFINITY;
    }
    10.0 * (255.0f64 * 255.0 / mean).log10()
}

#[test]
fn a_picture_survives_the_whole_pipeline_and_every_mip_level_is_where_it_belongs() {
    let (size, original) = picture();
    write_png("texconv-source.png", size, size, &original);

    for (slot, format) in [
        ("base", BlockFormat::Bc7UnormSrgb),
        ("bc1", BlockFormat::Bc1RgbaUnormSrgb),
    ] {
        // Through the tool's own conversion, then the container, then the reader -- so this covers the mip
        // chain, the DX10 header, the level offsets and the decode, not only the block encoder.
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_cic-texconv"));
        let source = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("texconv-source.png");
        let destination =
            PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("texconv-{slot}.dds"));
        let converted = command
            .arg("--slot")
            .arg(if slot == "bc1" { "bc1-colour" } else { slot })
            .arg(&source)
            .arg("-o")
            .arg(&destination)
            .output()
            .expect("run the converter");
        assert!(
            converted.status.success(),
            "{slot}: {}",
            String::from_utf8_lossy(&converted.stderr)
        );

        let bytes = std::fs::read(&destination).expect("read the converted texture");
        let texture = decode_dds(&bytes, TextureLimits::default()).expect("read back");
        assert_eq!(texture.format(), format, "{slot} chose the wrong format");
        assert_eq!(
            texture.level_count(),
            9,
            "{slot}: a 256-pixel texture reaches 1x1 in nine levels"
        );

        // Every level, not only the base one. A mip chain written at the wrong offsets decodes to
        // plausible-looking noise at level 1 and onward, and nothing about the base level would show it.
        for level in 0..texture.level_count() {
            let (level_width, level_height) = texture.level_size(level);
            let decoded = texture
                .decode_level(level)
                .unwrap_or_else(|| panic!("{slot} level {level} is missing"));
            assert_eq!(
                decoded.len(),
                (level_width as usize) * (level_height as usize) * 4,
                "{slot} level {level} is the wrong size"
            );
            if level == 0 {
                write_png(&format!("texconv-{slot}-decoded.png"), size, size, &decoded);
                let quality = psnr(&original, &decoded);
                let floor = if format == BlockFormat::Bc1RgbaUnormSrgb {
                    // Measured 22.7 dB. Low because a quarter of this picture is one-texel checkerboard,
                    // which 4 bpp cannot hold -- that quadrant is what the number is dominated by.
                    21.0
                } else {
                    // Measured 27.5 dB, on the same adversarial mix.
                    26.0
                };
                assert!(quality > floor, "{slot} managed only {quality:.1} dB");
            }

            // A flat tail: the last level is one texel, and its value must be a plausible average of the
            // picture rather than black or white. A chain built from the wrong source at each step
            // converges to something, and that something is usually an extreme.
            if level + 1 == texture.level_count() {
                assert_eq!(decoded.len(), 4, "{slot}: the tail is one texel");
                assert!(
                    (40..=210).contains(&decoded[0]) && (40..=210).contains(&decoded[1]),
                    "{slot}: the 1x1 tail is {decoded:?}, which is not an average of the picture"
                );
            }
        }
    }
}
