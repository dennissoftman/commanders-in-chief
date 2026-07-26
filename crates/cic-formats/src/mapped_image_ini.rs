// Commanders in Chief
// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only
//
// Provenance: the `MappedImage` block shape, its five fields, the UV/size derivation, and the
// rotated-image size swap are derived from Electronic Arts' GPL-3.0 source release,
// GeneralsGameCode revision 9f7abb866f5afd446db14149979e744c7216baaf, specifically
// `Core/GameEngine/Source/Common/INI/INIMappedImage.cpp` (`INI::parseMappedImageDefinition`),
// `Core/GameEngine/Source/GameClient/System/Image.cpp` (`Image::m_imageFieldParseTable`,
// `Image::parseImageCoords`, `Image::parseImageStatus`, `ImageCollection::load`), and
// `Core/GameEngine/Include/GameClient/Image.h` (`ImageStatus`, `imageStatusNames`).
// This decoder is a bounded, project-authored implementation and contains no retail data.

use crate::ui_ini::{
    LineReader, Separators, Tokens, UiIniDiagnostic, UiIniDiagnosticKind, UiIniError, UiIniFormat,
    UiIniLimits, diagnostic_text, is_end_token, scan_bit_string, scan_int,
};

/// The established `Status` vocabulary, in bit order.
///
/// `imageStatusNames` lists exactly these two names, so bit 0 is a 90-degree clockwise rotation
/// and bit 1 marks a definition whose texture data is supplied in memory rather than by file.
pub const MAPPED_IMAGE_STATUS_NAMES: [&str; 2] = ["ROTATED_90_CLOCKWISE", "RAW_TEXTURE"];

/// Bit 0: the packed region is stored rotated 90 degrees clockwise on its texture page.
pub const MAPPED_IMAGE_STATUS_ROTATED_90_CLOCKWISE: u32 = 0x0000_0001;
/// Bit 1: the definition carries raw texture data rather than naming a texture file.
pub const MAPPED_IMAGE_STATUS_RAW_TEXTURE: u32 = 0x0000_0002;

/// The texture-page region a `MappedImage` names, in page pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MappedImageCoords {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl MappedImageCoords {
    /// Returns the left page pixel column.
    #[must_use]
    pub const fn left(self) -> i32 {
        self.left
    }

    /// Returns the top page pixel row.
    #[must_use]
    pub const fn top(self) -> i32 {
        self.top
    }

    /// Returns the exclusive right page pixel column.
    #[must_use]
    pub const fn right(self) -> i32 {
        self.right
    }

    /// Returns the exclusive bottom page pixel row.
    #[must_use]
    pub const fn bottom(self) -> i32 {
        self.bottom
    }
}

/// One immutable named texture region.
///
/// Field order matters and is preserved: the source applies `Coords` and `Status` in the order the
/// file writes them, so a `Status` line naming `ROTATED_90_CLOCKWISE` before its `Coords` line
/// swaps a still-empty size and leaves the later `Coords` size unswapped. That quirk is reproduced
/// rather than corrected, because a modded definition relying on it must render the same way here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedImage {
    name: Vec<u8>,
    line: usize,
    texture: Vec<u8>,
    texture_width: i32,
    texture_height: i32,
    coords: Option<MappedImageCoords>,
    status: u32,
    image_width: i32,
    image_height: i32,
}

impl MappedImage {
    /// Returns the definition name exactly as spelled. Lookup is case-insensitive because the
    /// source keys its collection through a lowercased name key.
    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }

    /// Returns the one-based line the definition opened on.
    #[must_use]
    pub const fn line(&self) -> usize {
        self.line
    }

    /// Returns the referenced texture file name, empty when the definition names none.
    #[must_use]
    pub fn texture_bytes(&self) -> &[u8] {
        &self.texture
    }

    /// Returns the declared texture page width in pixels, zero when undeclared.
    #[must_use]
    pub const fn texture_width(&self) -> i32 {
        self.texture_width
    }

    /// Returns the declared texture page height in pixels, zero when undeclared.
    #[must_use]
    pub const fn texture_height(&self) -> i32 {
        self.texture_height
    }

    /// Returns the declared page region, absent when the definition declares no `Coords`.
    #[must_use]
    pub const fn coords(&self) -> Option<MappedImageCoords> {
        self.coords
    }

    /// Returns the raw `Status` bits.
    #[must_use]
    pub const fn status(&self) -> u32 {
        self.status
    }

    /// Returns whether the region is stored rotated 90 degrees clockwise.
    #[must_use]
    pub const fn is_rotated_90_clockwise(&self) -> bool {
        self.status & MAPPED_IMAGE_STATUS_ROTATED_90_CLOCKWISE != 0
    }

    /// Returns whether the definition declares raw in-memory texture data.
    #[must_use]
    pub const fn is_raw_texture(&self) -> bool {
        self.status & MAPPED_IMAGE_STATUS_RAW_TEXTURE != 0
    }

    /// Returns the presentation size in pixels, with the rotation swap already applied exactly
    /// where the source applied it.
    #[must_use]
    pub const fn image_size(&self) -> (i32, i32) {
        (self.image_width, self.image_height)
    }

    /// Returns the normalized `[left, top, right, bottom]` texture coordinates.
    ///
    /// The source divides each page pixel coordinate by the declared page dimension and leaves the
    /// coordinate unscaled when that dimension is zero, which this reproduces. Without `Coords` the
    /// source's constructor default of the full `0,0..1,1` page is returned.
    #[must_use]
    pub fn uv(&self) -> [f32; 4] {
        let Some(coords) = self.coords else {
            return [0.0, 0.0, 1.0, 1.0];
        };
        let scale = |value: i32, extent: i32| {
            #[expect(
                clippy::cast_precision_loss,
                reason = "page coordinates are small texture pixel counts; the source stores the \
                          same division in a 32-bit float"
            )]
            let value = value as f32;
            if extent == 0 {
                value
            } else {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "page dimensions are small texture pixel counts"
                )]
                let extent = extent as f32;
                value / extent
            }
        };
        [
            scale(coords.left, self.texture_width),
            scale(coords.top, self.texture_height),
            scale(coords.right, self.texture_width),
            scale(coords.bottom, self.texture_height),
        ]
    }
}

/// A bounded, immutable set of mapped-image definitions from one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedImageIni {
    images: Vec<MappedImage>,
    diagnostics: Vec<UiIniDiagnostic>,
}

impl MappedImageIni {
    /// Returns every definition in source order.
    #[must_use]
    pub fn images(&self) -> &[MappedImage] {
        &self.images
    }

    /// Returns every non-fatal observation in encounter order.
    #[must_use]
    pub fn diagnostics(&self) -> &[UiIniDiagnostic] {
        &self.diagnostics
    }
}

const FORMAT: UiIniFormat = UiIniFormat::MappedImage;

/// Decodes one `Data/INI/MappedImages/**` file into immutable named texture regions.
///
/// Unknown top-level blocks and unknown or malformed fields are retained as diagnostics rather
/// than dropped or promoted to failures, so an unsupported field stays discoverable. A duplicate
/// name overwrites the earlier definition's fields, matching the legacy loader, and records a
/// diagnostic.
///
/// # Errors
///
/// Returns a structured error when the input or any count, name, or value exceeds its explicit
/// limit, when a named block opens without a name, or when a block is never closed by `End`.
pub fn parse_mapped_image_ini(
    bytes: &[u8],
    limits: UiIniLimits,
) -> Result<MappedImageIni, UiIniError> {
    let mut reader = LineReader::new(bytes, FORMAT, limits)?;
    let mut images: Vec<MappedImage> = Vec::new();
    let mut diagnostics = Vec::new();

    while let Some((line, text)) = reader.next_line()? {
        let mut tokens = Tokens::new(text);
        let Some(keyword) = tokens.next(Separators::Default) else {
            continue;
        };
        if keyword != b"MappedImage" {
            diagnostics.push(UiIniDiagnostic::new(
                line,
                UiIniDiagnosticKind::UnknownBlock {
                    keyword: diagnostic_text(keyword),
                },
            ));
            skip_block(&mut reader, line)?;
            continue;
        }
        let Some(name) = tokens.next(Separators::Default) else {
            return Err(UiIniError::MissingBlockName {
                format: FORMAT,
                line,
            });
        };
        if name.len() > limits.max_name_bytes {
            return Err(UiIniError::NameTooLong {
                format: FORMAT,
                line,
                size: name.len(),
                limit: limits.max_name_bytes,
            });
        }
        let existing = images
            .iter()
            .position(|image| image.name.eq_ignore_ascii_case(name));
        if let Some(index) = existing {
            diagnostics.push(UiIniDiagnostic::new(
                line,
                UiIniDiagnosticKind::DuplicateDefinition {
                    name: diagnostic_text(name),
                    first_line: images[index].line,
                },
            ));
        } else if images.len() >= limits.max_definitions {
            return Err(UiIniError::TooManyDefinitions {
                format: FORMAT,
                line,
                limit: limits.max_definitions,
            });
        }
        let index = if let Some(index) = existing {
            index
        } else {
            images.push(MappedImage {
                name: name.to_vec(),
                line,
                texture: Vec::new(),
                texture_width: 0,
                texture_height: 0,
                coords: None,
                status: 0,
                image_width: 0,
                image_height: 0,
            });
            images.len() - 1
        };
        decode_block(
            &mut reader,
            &mut images[index],
            &mut diagnostics,
            line,
            limits,
        )?;
    }

    Ok(MappedImageIni {
        images,
        diagnostics,
    })
}

fn decode_block(
    reader: &mut LineReader<'_>,
    image: &mut MappedImage,
    diagnostics: &mut Vec<UiIniDiagnostic>,
    opened: usize,
    limits: UiIniLimits,
) -> Result<(), UiIniError> {
    loop {
        let Some((line, text)) = reader.next_line()? else {
            return Err(UiIniError::UnterminatedBlock {
                format: FORMAT,
                line: opened,
            });
        };
        let mut tokens = Tokens::new(text);
        let Some(field) = tokens.next(Separators::Default) else {
            continue;
        };
        if is_end_token(field) {
            return Ok(());
        }
        // Field names are matched case-sensitively, as `findFieldParse` uses `strcmp`.
        match field {
            b"Texture" => {
                let Some(value) = tokens.next_ascii_string() else {
                    push_malformed(diagnostics, line, field, "expected a texture file name");
                    continue;
                };
                if value.len() > limits.max_value_bytes {
                    return Err(UiIniError::ValueTooLong {
                        format: FORMAT,
                        line,
                        field: diagnostic_text(field),
                        size: value.len(),
                        limit: limits.max_value_bytes,
                    });
                }
                image.texture = value;
            }
            b"TextureWidth" => match tokens.next(Separators::Default).and_then(scan_int) {
                Some(value) => image.texture_width = value,
                None => push_malformed(diagnostics, line, field, "expected an integer"),
            },
            b"TextureHeight" => match tokens.next(Separators::Default).and_then(scan_int) {
                Some(value) => image.texture_height = value,
                None => push_malformed(diagnostics, line, field, "expected an integer"),
            },
            b"Coords" => match decode_coords(&mut tokens) {
                Some(coords) => {
                    image.coords = Some(coords);
                    // The source recomputes the presentation size from the region here, without
                    // consulting the rotation flag; `Status` performs that swap when it is read.
                    image.image_width = coords.right.wrapping_sub(coords.left);
                    image.image_height = coords.bottom.wrapping_sub(coords.top);
                }
                None => push_malformed(
                    diagnostics,
                    line,
                    field,
                    "expected Left:<int> Top:<int> Right:<int> Bottom:<int>",
                ),
            },
            b"Status" => {
                match scan_bit_string(&mut tokens, &MAPPED_IMAGE_STATUS_NAMES, image.status) {
                    Ok(bits) => {
                        image.status = bits;
                        if bits & MAPPED_IMAGE_STATUS_ROTATED_90_CLOCKWISE != 0 {
                            // The packed region rect describes the rotated image, so the source
                            // swaps the stored size at the point `Status` is parsed.
                            std::mem::swap(&mut image.image_width, &mut image.image_height);
                        }
                    }
                    Err(error) => diagnostics.push(UiIniDiagnostic::new(
                        line,
                        UiIniDiagnosticKind::MalformedField {
                            field: diagnostic_text(field),
                            reason: error.reason(),
                        },
                    )),
                }
            }
            _ => diagnostics.push(UiIniDiagnostic::new(
                line,
                UiIniDiagnosticKind::UnknownField {
                    field: diagnostic_text(field),
                },
            )),
        }
    }
}

fn decode_coords(tokens: &mut Tokens<'_>) -> Option<MappedImageCoords> {
    let left = scan_int(tokens.next_sub_token("Left")?)?;
    let top = scan_int(tokens.next_sub_token("Top")?)?;
    let right = scan_int(tokens.next_sub_token("Right")?)?;
    let bottom = scan_int(tokens.next_sub_token("Bottom")?)?;
    Some(MappedImageCoords {
        left,
        top,
        right,
        bottom,
    })
}

fn push_malformed(diagnostics: &mut Vec<UiIniDiagnostic>, line: usize, field: &[u8], reason: &str) {
    diagnostics.push(UiIniDiagnostic::new(
        line,
        UiIniDiagnosticKind::MalformedField {
            field: diagnostic_text(field),
            reason: reason.to_owned().into_boxed_str(),
        },
    ));
}

/// Consumes an unowned block up to its `End`, so one unrelated block cannot desynchronize the
/// remaining definitions.
fn skip_block(reader: &mut LineReader<'_>, opened: usize) -> Result<(), UiIniError> {
    loop {
        let Some((_, text)) = reader.next_line()? else {
            return Err(UiIniError::UnterminatedBlock {
                format: FORMAT,
                line: opened,
            });
        };
        let mut tokens = Tokens::new(text);
        if tokens.next(Separators::Default).is_some_and(is_end_token) {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MAPPED_IMAGE_STATUS_RAW_TEXTURE, MAPPED_IMAGE_STATUS_ROTATED_90_CLOCKWISE,
        parse_mapped_image_ini,
    };
    use crate::ui_ini::{UiIniDiagnosticKind, UiIniError, UiIniFormat, UiIniLimits};

    #[track_caller]
    fn assert_uv(actual: [f32; 4], expected: [f32; 4]) {
        for (actual, expected) in actual.into_iter().zip(expected) {
            assert!(
                (actual - expected).abs() < f32::EPSILON,
                "expected {expected}, got {actual}"
            );
        }
    }

    #[test]
    fn decodes_regions_and_derives_uv_and_size() {
        let ini = parse_mapped_image_ini(
            b"; synthetic fixture\r\n\
              MappedImage SynthButton\r\n\
                Texture = SynthPage.tga\r\n\
                TextureWidth = 512\r\n\
                TextureHeight = 256\r\n\
                Coords = Left:0 Top:0 Right:128 Bottom:64\r\n\
                Status = NONE\r\n\
              End\r\n",
            UiIniLimits::default(),
        )
        .expect("decode mapped images");
        assert_eq!(ini.images().len(), 1);
        let image = &ini.images()[0];
        assert_eq!(image.name_bytes(), b"SynthButton");
        assert_eq!(image.texture_bytes(), b"SynthPage.tga");
        assert_eq!(image.texture_width(), 512);
        assert_eq!(image.texture_height(), 256);
        assert_eq!(image.image_size(), (128, 64));
        assert_uv(image.uv(), [0.0, 0.0, 0.25, 0.25]);
        assert_eq!(image.status(), 0);
        assert!(ini.diagnostics().is_empty());
    }

    #[test]
    fn rotation_status_swaps_the_size_where_the_source_swaps_it() {
        let ini = parse_mapped_image_ini(
            b"MappedImage Sideways\n\
                TextureWidth = 512\n\
                TextureHeight = 512\n\
                Coords = Left:0 Top:0 Right:40 Bottom:10\n\
                Status = ROTATED_90_CLOCKWISE RAW_TEXTURE\n\
              End\n\
              MappedImage StatusFirst\n\
                Status = ROTATED_90_CLOCKWISE\n\
                Coords = Left:0 Top:0 Right:40 Bottom:10\n\
              End\n",
            UiIniLimits::default(),
        )
        .expect("decode rotated images");
        let sideways = &ini.images()[0];
        assert_eq!(sideways.image_size(), (10, 40));
        assert!(sideways.is_rotated_90_clockwise());
        assert!(sideways.is_raw_texture());
        assert_eq!(
            sideways.status(),
            MAPPED_IMAGE_STATUS_ROTATED_90_CLOCKWISE | MAPPED_IMAGE_STATUS_RAW_TEXTURE
        );

        // Declared before `Coords`, the flag swaps an empty size and the later region wins
        // unswapped. This is the source's order dependence, kept deliberately.
        let status_first = &ini.images()[1];
        assert!(status_first.is_rotated_90_clockwise());
        assert_eq!(status_first.image_size(), (40, 10));
        // Without a declared page size the source leaves the page pixel coordinates unscaled.
        assert_uv(status_first.uv(), [0.0, 0.0, 40.0, 10.0]);
    }

    #[test]
    fn unknown_blocks_and_fields_stay_discoverable() {
        let ini = parse_mapped_image_ini(
            b"Terrain SomethingElse\n  Texture = other.tga\nEnd\n\
              MappedImage Synth\n  Sparkle = Yes\n  Coords = Left:oops\nEnd\n",
            UiIniLimits::default(),
        )
        .expect("decode with diagnostics");
        assert_eq!(ini.images().len(), 1);
        assert_eq!(ini.diagnostics().len(), 3);
        assert_eq!(
            ini.diagnostics()[0].kind(),
            &UiIniDiagnosticKind::UnknownBlock {
                keyword: "Terrain".to_owned().into_boxed_str(),
            }
        );
        assert_eq!(
            ini.diagnostics()[1].kind(),
            &UiIniDiagnosticKind::UnknownField {
                field: "Sparkle".to_owned().into_boxed_str(),
            }
        );
        assert!(matches!(
            ini.diagnostics()[2].kind(),
            UiIniDiagnosticKind::MalformedField { field, .. } if &**field == "Coords"
        ));
        assert_eq!(ini.images()[0].coords(), None);
    }

    #[test]
    fn a_duplicate_name_overwrites_and_reports() {
        let ini = parse_mapped_image_ini(
            b"MappedImage Synth\n  Texture = first.tga\n  TextureWidth = 8\nEnd\n\
              MappedImage synth\n  Texture = second.tga\nEnd\n",
            UiIniLimits::default(),
        )
        .expect("decode duplicates");
        assert_eq!(ini.images().len(), 1);
        assert_eq!(ini.images()[0].texture_bytes(), b"second.tga");
        // Fields the second definition omits keep the first definition's value, as the source
        // parses into the existing image rather than replacing it.
        assert_eq!(ini.images()[0].texture_width(), 8);
        assert_eq!(
            ini.diagnostics()[0].kind(),
            &UiIniDiagnosticKind::DuplicateDefinition {
                name: "synth".to_owned().into_boxed_str(),
                first_line: 1,
            }
        );
    }

    #[test]
    fn rejects_structural_failures_and_limit_excess() {
        assert_eq!(
            parse_mapped_image_ini(b"MappedImage Synth\n", UiIniLimits::default()),
            Err(UiIniError::UnterminatedBlock {
                format: UiIniFormat::MappedImage,
                line: 1,
            })
        );
        assert_eq!(
            parse_mapped_image_ini(
                b"MappedImage\n  Texture = a.tga\nEnd\n",
                UiIniLimits::default()
            ),
            Err(UiIniError::MissingBlockName {
                format: UiIniFormat::MappedImage,
                line: 1,
            })
        );
        let limits = UiIniLimits {
            max_definitions: 1,
            ..UiIniLimits::default()
        };
        assert_eq!(
            parse_mapped_image_ini(b"MappedImage A\nEnd\nMappedImage B\nEnd\n", limits),
            Err(UiIniError::TooManyDefinitions {
                format: UiIniFormat::MappedImage,
                line: 3,
                limit: 1,
            })
        );
        let limits = UiIniLimits {
            max_name_bytes: 2,
            ..UiIniLimits::default()
        };
        assert_eq!(
            parse_mapped_image_ini(b"MappedImage ABC\nEnd\n", limits),
            Err(UiIniError::NameTooLong {
                format: UiIniFormat::MappedImage,
                line: 1,
                size: 3,
                limit: 2,
            })
        );
        let limits = UiIniLimits {
            max_value_bytes: 3,
            ..UiIniLimits::default()
        };
        assert!(matches!(
            parse_mapped_image_ini(b"MappedImage A\n  Texture = long.tga\nEnd\n", limits),
            Err(UiIniError::ValueTooLong {
                size: 8,
                limit: 3,
                ..
            })
        ));
    }

    #[test]
    fn truncating_at_every_prefix_never_panics() {
        let complete = b"MappedImage Synth\n  Texture = a.tga\n  TextureWidth = 4\n  \
                         Coords = Left:0 Top:0 Right:2 Bottom:2\n  Status = RAW_TEXTURE\nEnd\n";
        for length in 0..=complete.len() {
            let _ = parse_mapped_image_ini(&complete[..length], UiIniLimits::default());
        }
    }
}
