// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Bounded decoding for source-established `WaterSet` and `WaterTransparency` INI blocks.
//!
//! Field names and value kinds are derived from `Water.h`, `Water.cpp`, `INIWater.cpp`, and the
//! generic color decoders in `INI.cpp` from `GeneralsGameCode` revision
//! `9f7abb866f5afd446db14149979e744c7216baaf`, licensed under GPL-3.0-or-later with Electronic Arts
//! Section 7 terms. Full notices are recorded in `docs/provenance/map.md`.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use crate::MapTimeOfDay;

const MAX_ABSOLUTE_SCALAR: f32 = 1_000_000.0;
const MAX_REPEAT_COUNT: i32 = 1_000_000;

/// Explicit resource bounds for water INI inspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaterIniLimits {
    pub max_file_bytes: usize,
    pub max_lines: usize,
    pub max_line_bytes: usize,
    pub max_definitions: usize,
    pub max_string_bytes: usize,
}

impl Default for WaterIniLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: 4 * 1_024 * 1_024,
            max_lines: 100_000,
            max_line_bytes: 4_096,
            max_definitions: 1_024,
            max_string_bytes: 1_024,
        }
    }
}

/// One exact integer RGBA value from a `WaterSet` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaterRgba8([u8; 4]);

impl WaterRgba8 {
    #[must_use]
    pub const fn channels(self) -> [u8; 4] {
        self.0
    }
}

/// Last-field-wins values for one named time-of-day `WaterSet`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WaterSetIni {
    sky_texture: Option<Vec<u8>>,
    water_texture: Option<Vec<u8>>,
    vertex_colors: [Option<WaterRgba8>; 4],
    diffuse_color: Option<WaterRgba8>,
    transparent_diffuse_color: Option<WaterRgba8>,
    u_scroll_per_ms: Option<f32>,
    v_scroll_per_ms: Option<f32>,
    sky_texels_per_unit: Option<f32>,
    water_repeat_count: Option<i32>,
}

impl WaterSetIni {
    #[must_use]
    pub fn sky_texture_bytes(&self) -> Option<&[u8]> {
        self.sky_texture.as_deref()
    }

    #[must_use]
    pub fn water_texture_bytes(&self) -> Option<&[u8]> {
        self.water_texture.as_deref()
    }

    /// Returns vertex 00, 10, 01, and 11 colors in the source field-table order.
    #[must_use]
    pub const fn vertex_colors(&self) -> &[Option<WaterRgba8>; 4] {
        &self.vertex_colors
    }

    #[must_use]
    pub const fn diffuse_color(&self) -> Option<WaterRgba8> {
        self.diffuse_color
    }

    #[must_use]
    pub const fn transparent_diffuse_color(&self) -> Option<WaterRgba8> {
        self.transparent_diffuse_color
    }

    #[must_use]
    pub const fn u_scroll_per_ms(&self) -> Option<f32> {
        self.u_scroll_per_ms
    }

    #[must_use]
    pub const fn v_scroll_per_ms(&self) -> Option<f32> {
        self.v_scroll_per_ms
    }

    #[must_use]
    pub const fn sky_texels_per_unit(&self) -> Option<f32> {
        self.sky_texels_per_unit
    }

    #[must_use]
    pub const fn water_repeat_count(&self) -> Option<i32> {
        self.water_repeat_count
    }
}

/// Last-field-wins water-transparency values retained from one INI stream.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WaterTransparencyIni {
    minimum_opacity: Option<f32>,
    opaque_depth: Option<f32>,
    standing_water_color: Option<[f32; 3]>,
    radar_water_color: Option<[f32; 3]>,
    standing_water_texture: Option<Vec<u8>>,
    additive_blending: Option<bool>,
    skybox_textures: [Option<Vec<u8>>; 5],
}

impl WaterTransparencyIni {
    #[must_use]
    pub const fn minimum_opacity(&self) -> Option<f32> {
        self.minimum_opacity
    }

    #[must_use]
    pub const fn opaque_depth(&self) -> Option<f32> {
        self.opaque_depth
    }

    #[must_use]
    pub const fn standing_water_color(&self) -> Option<[f32; 3]> {
        self.standing_water_color
    }

    #[must_use]
    pub const fn radar_water_color(&self) -> Option<[f32; 3]> {
        self.radar_water_color
    }

    #[must_use]
    pub fn standing_water_texture_bytes(&self) -> Option<&[u8]> {
        self.standing_water_texture.as_deref()
    }

    #[must_use]
    pub const fn additive_blending(&self) -> Option<bool> {
        self.additive_blending
    }

    /// Returns north, east, south, west, and top skybox texture names.
    #[must_use]
    pub const fn skybox_textures(&self) -> &[Option<Vec<u8>>; 5] {
        &self.skybox_textures
    }
}

/// Complete water appearance values decoded from one INI stream.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WaterIni {
    sets: [Option<WaterSetIni>; 4],
    transparency: WaterTransparencyIni,
}

impl WaterIni {
    #[must_use]
    pub fn water_set(&self, time: MapTimeOfDay) -> Option<&WaterSetIni> {
        self.sets[time.index()].as_ref()
    }

    #[must_use]
    pub const fn water_sets(&self) -> &[Option<WaterSetIni>; 4] {
        &self.sets
    }

    #[must_use]
    pub const fn transparency(&self) -> &WaterTransparencyIni {
        &self.transparency
    }
}

/// A structured failure from bounded water INI decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaterIniError {
    FileTooLarge {
        size: usize,
        limit: usize,
    },
    TooManyLines {
        limit: usize,
    },
    LineTooLong {
        line: usize,
        size: usize,
        limit: usize,
    },
    TooManyDefinitions {
        limit: usize,
    },
    ValueTooLong {
        line: usize,
        size: usize,
        limit: usize,
    },
    NestedWaterTransparency {
        line: usize,
    },
    NestedWaterSet {
        line: usize,
    },
    UnterminatedWaterTransparency {
        line: usize,
    },
    UnterminatedWaterSet {
        line: usize,
    },
    InvalidTimeOfDay {
        line: usize,
    },
    MissingValue {
        line: usize,
    },
    InvalidNumber {
        line: usize,
    },
    InvalidInteger {
        line: usize,
    },
    InvalidBoolean {
        line: usize,
    },
    InvalidColor {
        line: usize,
    },
    InvalidOpacity {
        line: usize,
    },
    InvalidDepth {
        line: usize,
    },
    InvalidScalarRange {
        line: usize,
    },
}

impl Display for WaterIniError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::FileTooLarge { size, limit } => {
                write!(formatter, "Water INI is {size} bytes; limit is {limit}")
            }
            Self::TooManyLines { limit } => write!(formatter, "Water INI exceeds {limit} lines"),
            Self::LineTooLong { line, size, limit } => write!(
                formatter,
                "Water INI line {line} is {size} bytes; limit is {limit}"
            ),
            Self::TooManyDefinitions { limit } => {
                write!(formatter, "Water INI exceeds {limit} water definitions")
            }
            Self::ValueTooLong { line, size, limit } => write!(
                formatter,
                "Water INI value on line {line} is {size} bytes; limit is {limit}"
            ),
            Self::NestedWaterTransparency { line } => write!(
                formatter,
                "water block begins before WaterTransparency ends at line {line}"
            ),
            Self::NestedWaterSet { line } => write!(
                formatter,
                "water block begins before WaterSet ends at line {line}"
            ),
            Self::UnterminatedWaterTransparency { line } => write!(
                formatter,
                "WaterTransparency opened on line {line} has no End"
            ),
            Self::UnterminatedWaterSet { line } => {
                write!(formatter, "WaterSet opened on line {line} has no End")
            }
            Self::InvalidTimeOfDay { line } => {
                write!(
                    formatter,
                    "WaterSet has an invalid time of day on line {line}"
                )
            }
            Self::MissingValue { line } => {
                write!(formatter, "water field has no value on line {line}")
            }
            Self::InvalidNumber { line } => {
                write!(formatter, "water value is not finite on line {line}")
            }
            Self::InvalidInteger { line } => {
                write!(formatter, "water integer is invalid on line {line}")
            }
            Self::InvalidBoolean { line } => {
                write!(formatter, "water boolean is invalid on line {line}")
            }
            Self::InvalidColor { line } => {
                write!(formatter, "water color is invalid on line {line}")
            }
            Self::InvalidOpacity { line } => {
                write!(
                    formatter,
                    "water opacity is outside zero through one on line {line}"
                )
            }
            Self::InvalidDepth { line } => {
                write!(
                    formatter,
                    "water opaque depth is outside valid bounds on line {line}"
                )
            }
            Self::InvalidScalarRange { line } => {
                write!(
                    formatter,
                    "water scalar is outside valid bounds on line {line}"
                )
            }
        }
    }
}

impl Error for WaterIniError {}

#[derive(Debug, Clone, Copy)]
enum Block {
    None,
    Other,
    Transparency { line: usize },
    Set { line: usize, index: usize },
}

#[derive(Debug, Clone, Copy)]
enum WaterHeader {
    Transparency,
    Set(usize),
}

/// Decodes all source-established `WaterSet` and `WaterTransparency` fields.
///
/// Repeated blocks and fields use stable file-order last-field-wins semantics. Unrelated blocks
/// are ignored as complete units.
///
/// # Errors
///
/// Returns a structured error for resource-limit excess, malformed block closure, invalid names,
/// non-finite/out-of-range scalars, malformed colors, integers, booleans, or strings.
pub fn parse_water_ini(bytes: &[u8], limits: WaterIniLimits) -> Result<WaterIni, WaterIniError> {
    if bytes.len() > limits.max_file_bytes {
        return Err(WaterIniError::FileTooLarge {
            size: bytes.len(),
            limit: limits.max_file_bytes,
        });
    }
    let mut result = WaterIni::default();
    let mut block = Block::None;
    let mut definitions = 0_usize;
    for (zero_based_line, raw_line) in bytes.split(|byte| *byte == b'\n').enumerate() {
        let line_number = zero_based_line
            .checked_add(1)
            .ok_or(WaterIniError::TooManyLines {
                limit: limits.max_lines,
            })?;
        if line_number > limits.max_lines {
            return Err(WaterIniError::TooManyLines {
                limit: limits.max_lines,
            });
        }
        if raw_line.len() > limits.max_line_bytes {
            return Err(WaterIniError::LineTooLong {
                line: line_number,
                size: raw_line.len(),
                limit: limits.max_line_bytes,
            });
        }
        let line = trim_ascii(strip_comment(raw_line));
        if line.is_empty() {
            continue;
        }
        if line.eq_ignore_ascii_case(b"End") {
            block = Block::None;
            continue;
        }
        if matches!(block, Block::Other) {
            continue;
        }
        if let Some(header) = water_header(line, line_number)? {
            match block {
                Block::Transparency { .. } => {
                    return Err(WaterIniError::NestedWaterTransparency { line: line_number });
                }
                Block::Set { .. } => {
                    return Err(WaterIniError::NestedWaterSet { line: line_number });
                }
                Block::Other => continue,
                Block::None => {}
            }
            definitions = definitions
                .checked_add(1)
                .ok_or(WaterIniError::TooManyDefinitions {
                    limit: limits.max_definitions,
                })?;
            if definitions > limits.max_definitions {
                return Err(WaterIniError::TooManyDefinitions {
                    limit: limits.max_definitions,
                });
            }
            block = match header {
                WaterHeader::Set(index) => {
                    result.sets[index].get_or_insert_with(WaterSetIni::default);
                    Block::Set {
                        line: line_number,
                        index,
                    }
                }
                WaterHeader::Transparency => Block::Transparency { line: line_number },
            };
            continue;
        }
        if matches!(block, Block::None) && !line.contains(&b'=') {
            block = Block::Other;
            continue;
        }
        let Some((field, value)) = split_assignment(line) else {
            continue;
        };
        match block {
            Block::Transparency { .. } => {
                parse_transparency_field(
                    &mut result.transparency,
                    field,
                    value,
                    line_number,
                    limits,
                )?;
            }
            Block::Set { index, .. } => {
                let set = result.sets[index].get_or_insert_with(WaterSetIni::default);
                parse_set_field(set, field, value, line_number, limits)?;
            }
            Block::None | Block::Other => {}
        }
    }
    match block {
        Block::Transparency { line } => Err(WaterIniError::UnterminatedWaterTransparency { line }),
        Block::Set { line, .. } => Err(WaterIniError::UnterminatedWaterSet { line }),
        Block::None | Block::Other => Ok(result),
    }
}

/// Compatibility projection that retains only the transparency block values.
///
/// # Errors
///
/// Returns the same bounded syntax and value failures as [`parse_water_ini`].
pub fn parse_water_transparency_ini(
    bytes: &[u8],
    limits: WaterIniLimits,
) -> Result<WaterTransparencyIni, WaterIniError> {
    Ok(parse_water_ini(bytes, limits)?.transparency)
}

fn water_header(line: &[u8], line_number: usize) -> Result<Option<WaterHeader>, WaterIniError> {
    if line.eq_ignore_ascii_case(b"WaterTransparency") {
        return Ok(Some(WaterHeader::Transparency));
    }
    let mut words = line.split(u8::is_ascii_whitespace);
    if !words
        .next()
        .is_some_and(|word| word.eq_ignore_ascii_case(b"WaterSet"))
    {
        return Ok(None);
    }
    let name = words
        .next()
        .filter(|word| !word.is_empty())
        .ok_or(WaterIniError::InvalidTimeOfDay { line: line_number })?;
    if words.any(|word| !word.is_empty()) {
        return Err(WaterIniError::InvalidTimeOfDay { line: line_number });
    }
    let index = if name.eq_ignore_ascii_case(b"Morning") {
        0
    } else if name.eq_ignore_ascii_case(b"Afternoon") {
        1
    } else if name.eq_ignore_ascii_case(b"Evening") {
        2
    } else if name.eq_ignore_ascii_case(b"Night") {
        3
    } else {
        return Err(WaterIniError::InvalidTimeOfDay { line: line_number });
    };
    Ok(Some(WaterHeader::Set(index)))
}

fn parse_set_field(
    set: &mut WaterSetIni,
    field: &[u8],
    value: &[u8],
    line: usize,
    limits: WaterIniLimits,
) -> Result<(), WaterIniError> {
    if field.eq_ignore_ascii_case(b"SkyTexture") {
        set.sky_texture = Some(parse_string(value, line, limits)?);
    } else if field.eq_ignore_ascii_case(b"WaterTexture") {
        set.water_texture = Some(parse_string(value, line, limits)?);
    } else if field.eq_ignore_ascii_case(b"Vertex00Color") {
        set.vertex_colors[0] = Some(parse_rgba8(value, line)?);
    } else if field.eq_ignore_ascii_case(b"Vertex10Color") {
        set.vertex_colors[1] = Some(parse_rgba8(value, line)?);
    } else if field.eq_ignore_ascii_case(b"Vertex01Color") {
        set.vertex_colors[2] = Some(parse_rgba8(value, line)?);
    } else if field.eq_ignore_ascii_case(b"Vertex11Color") {
        set.vertex_colors[3] = Some(parse_rgba8(value, line)?);
    } else if field.eq_ignore_ascii_case(b"DiffuseColor") {
        set.diffuse_color = Some(parse_rgba8(value, line)?);
    } else if field.eq_ignore_ascii_case(b"TransparentDiffuseColor") {
        set.transparent_diffuse_color = Some(parse_rgba8(value, line)?);
    } else if field.eq_ignore_ascii_case(b"UScrollPerMS") {
        set.u_scroll_per_ms = Some(parse_bounded_scalar(value, line)?);
    } else if field.eq_ignore_ascii_case(b"VScrollPerMS") {
        set.v_scroll_per_ms = Some(parse_bounded_scalar(value, line)?);
    } else if field.eq_ignore_ascii_case(b"SkyTexelsPerUnit") {
        set.sky_texels_per_unit = Some(parse_bounded_scalar(value, line)?);
    } else if field.eq_ignore_ascii_case(b"WaterRepeatCount") {
        set.water_repeat_count = Some(parse_repeat_count(value, line)?);
    }
    Ok(())
}

fn parse_transparency_field(
    transparency: &mut WaterTransparencyIni,
    field: &[u8],
    value: &[u8],
    line: usize,
    limits: WaterIniLimits,
) -> Result<(), WaterIniError> {
    if field.eq_ignore_ascii_case(b"TransparentWaterMinOpacity") {
        let value = parse_number(value, line)?;
        if !(0.0..=1.0).contains(&value) {
            return Err(WaterIniError::InvalidOpacity { line });
        }
        transparency.minimum_opacity = Some(value);
    } else if field.eq_ignore_ascii_case(b"TransparentWaterDepth") {
        let value = parse_number(value, line)?;
        if value <= 0.0 || value > 10_000.0 {
            return Err(WaterIniError::InvalidDepth { line });
        }
        transparency.opaque_depth = Some(value);
    } else if field.eq_ignore_ascii_case(b"StandingWaterColor") {
        transparency.standing_water_color = Some(parse_rgb(value, line)?);
    } else if field.eq_ignore_ascii_case(b"RadarWaterColor") {
        transparency.radar_water_color = Some(parse_rgb(value, line)?);
    } else if field.eq_ignore_ascii_case(b"StandingWaterTexture") {
        transparency.standing_water_texture = Some(parse_string(value, line, limits)?);
    } else if field.eq_ignore_ascii_case(b"AdditiveBlending") {
        transparency.additive_blending = Some(parse_bool(value, line)?);
    } else {
        for (index, name) in [
            b"SkyboxTextureN".as_slice(),
            b"SkyboxTextureE".as_slice(),
            b"SkyboxTextureS".as_slice(),
            b"SkyboxTextureW".as_slice(),
            b"SkyboxTextureT".as_slice(),
        ]
        .into_iter()
        .enumerate()
        {
            if field.eq_ignore_ascii_case(name) {
                transparency.skybox_textures[index] = Some(parse_string(value, line, limits)?);
                break;
            }
        }
    }
    Ok(())
}

fn parse_rgba8(value: &[u8], line: usize) -> Result<WaterRgba8, WaterIniError> {
    let component_count = std::str::from_utf8(trim_ascii(value))
        .map_err(|_| WaterIniError::InvalidColor { line })?
        .split_ascii_whitespace()
        .count();
    let labels = match component_count {
        3 => b"RGB".as_slice(),
        4 => b"RGBA".as_slice(),
        _ => return Err(WaterIniError::InvalidColor { line }),
    };
    let components = parse_labeled_components(value, line, labels)?;
    let mut color = [0_u8, 0, 0, 255];
    for (target, component) in color.iter_mut().take(component_count).zip(components) {
        *target = component
            .parse::<u8>()
            .map_err(|_| WaterIniError::InvalidColor { line })?;
    }
    Ok(WaterRgba8(color))
}

fn parse_rgb(value: &[u8], line: usize) -> Result<[f32; 3], WaterIniError> {
    let components = parse_labeled_components(value, line, b"RGB")?;
    let mut color = [0.0; 3];
    for (target, component) in color.iter_mut().zip(components) {
        let component = component
            .parse::<u8>()
            .map_err(|_| WaterIniError::InvalidColor { line })?;
        *target = f32::from(component) / 255.0;
    }
    Ok(color)
}

fn parse_labeled_components<'a>(
    value: &'a [u8],
    line: usize,
    labels: &[u8],
) -> Result<Vec<&'a str>, WaterIniError> {
    let text =
        std::str::from_utf8(trim_ascii(value)).map_err(|_| WaterIniError::InvalidColor { line })?;
    let words = text.split_ascii_whitespace().collect::<Vec<_>>();
    if words.len() != labels.len() {
        return Err(WaterIniError::InvalidColor { line });
    }
    let mut components = Vec::with_capacity(labels.len());
    for (word, label) in words.into_iter().zip(labels) {
        let bytes = word.as_bytes();
        if bytes.len() < 3 || !bytes[0].eq_ignore_ascii_case(label) || bytes[1] != b':' {
            return Err(WaterIniError::InvalidColor { line });
        }
        components.push(&word[2..]);
    }
    Ok(components)
}

fn parse_string(
    value: &[u8],
    line: usize,
    limits: WaterIniLimits,
) -> Result<Vec<u8>, WaterIniError> {
    let mut value = trim_ascii(value);
    if value.len() >= 2
        && ((value[0] == b'"' && value[value.len() - 1] == b'"')
            || (value[0] == b'\'' && value[value.len() - 1] == b'\''))
    {
        value = &value[1..value.len() - 1];
    }
    if value.is_empty() {
        return Err(WaterIniError::MissingValue { line });
    }
    if value.len() > limits.max_string_bytes {
        return Err(WaterIniError::ValueTooLong {
            line,
            size: value.len(),
            limit: limits.max_string_bytes,
        });
    }
    Ok(value.to_vec())
}

fn parse_bounded_scalar(value: &[u8], line: usize) -> Result<f32, WaterIniError> {
    let value = parse_number(value, line)?;
    if value.abs() > MAX_ABSOLUTE_SCALAR {
        return Err(WaterIniError::InvalidScalarRange { line });
    }
    Ok(value)
}

fn parse_repeat_count(value: &[u8], line: usize) -> Result<i32, WaterIniError> {
    let value = std::str::from_utf8(trim_ascii(value))
        .ok()
        .and_then(|value| value.parse::<i32>().ok())
        .ok_or(WaterIniError::InvalidInteger { line })?;
    if !(0..=MAX_REPEAT_COUNT).contains(&value) {
        return Err(WaterIniError::InvalidInteger { line });
    }
    Ok(value)
}

fn parse_bool(value: &[u8], line: usize) -> Result<bool, WaterIniError> {
    let value = trim_ascii(value);
    if value.eq_ignore_ascii_case(b"yes") || value.eq_ignore_ascii_case(b"true") || value == b"1" {
        Ok(true)
    } else if value.eq_ignore_ascii_case(b"no")
        || value.eq_ignore_ascii_case(b"false")
        || value == b"0"
    {
        Ok(false)
    } else {
        Err(WaterIniError::InvalidBoolean { line })
    }
}

fn parse_number(value: &[u8], line: usize) -> Result<f32, WaterIniError> {
    let value = trim_ascii(value);
    if value.is_empty() {
        return Err(WaterIniError::MissingValue { line });
    }
    std::str::from_utf8(value)
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| value.is_finite())
        .ok_or(WaterIniError::InvalidNumber { line })
}

fn strip_comment(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    line.iter()
        .position(|byte| *byte == b';')
        .map_or(line, |index| &line[..index])
}

fn trim_ascii(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[1..];
    }
    while bytes.last().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

fn split_assignment(line: &[u8]) -> Option<(&[u8], &[u8])> {
    let equals = line.iter().position(|byte| *byte == b'=')?;
    Some((trim_ascii(&line[..equals]), trim_ascii(&line[equals + 1..])))
}

#[cfg(test)]
mod tests {
    use crate::MapTimeOfDay;

    use super::{WaterIniError, WaterIniLimits, parse_water_ini, parse_water_transparency_ini};

    #[test]
    fn retains_all_water_fields_with_stable_last_field_wins_semantics() {
        let parsed = parse_water_ini(
            b"WaterSet MORNING\n\
               SkyTexture = sky.tga\n WaterTexture = water.tga\n\
               Vertex00Color = R:1 G:2 B:3\n Vertex10Color = R:5 G:6 B:7 A:8\n\
               Vertex01Color = R:9 G:10 B:11 A:12\n Vertex11Color = R:13 G:14 B:15 A:16\n\
               DiffuseColor = R:17 G:18 B:19 A:20\n\
               TransparentDiffuseColor = R:21 G:22 B:23 A:24\n\
               UScrollPerMS = 0.1\n VScrollPerMS = -0.2\n SkyTexelsPerUnit = 3.5\n\
               WaterRepeatCount = 7\nEnd\n\
               WaterSet MORNING\n UScrollPerMS = 0.25\nEnd\n\
               WaterTransparency\n TransparentWaterMinOpacity = 0.8\n\
               TransparentWaterDepth = 2.0\n StandingWaterColor = R:255 G:128 B:0\n\
               RadarWaterColor = R:140 G:140 B:255\n StandingWaterTexture = stand.tga\n\
               AdditiveBlending = yes\n SkyboxTextureN = north.tga\n SkyboxTextureE = east.tga\n\
               SkyboxTextureS = south.tga\n SkyboxTextureW = west.tga\n SkyboxTextureT = top.tga\nEnd\n",
            WaterIniLimits::default(),
        )
        .expect("water INI");
        let morning = parsed
            .water_set(MapTimeOfDay::Morning)
            .expect("morning set");
        assert_eq!(morning.sky_texture_bytes(), Some(b"sky.tga".as_slice()));
        assert_eq!(
            morning.vertex_colors()[3].expect("color").channels(),
            [13, 14, 15, 16]
        );
        assert_eq!(
            morning.vertex_colors()[0].expect("color").channels(),
            [1, 2, 3, 255]
        );
        assert_eq!(
            morning
                .transparent_diffuse_color()
                .expect("color")
                .channels(),
            [21, 22, 23, 24]
        );
        assert_eq!(
            morning.u_scroll_per_ms().map(f32::to_bits),
            Some(0.25_f32.to_bits())
        );
        assert_eq!(morning.water_repeat_count(), Some(7));
        assert_eq!(
            parsed.transparency().standing_water_color(),
            Some([1.0, 128.0 / 255.0, 0.0])
        );
        assert_eq!(
            parsed.transparency().radar_water_color(),
            Some([140.0 / 255.0, 140.0 / 255.0, 1.0])
        );
        assert_eq!(parsed.transparency().additive_blending(), Some(true));
        assert_eq!(
            parsed.transparency().skybox_textures()[4].as_deref(),
            Some(b"top.tga".as_slice())
        );
    }

    #[test]
    fn compatibility_projection_retains_transparency_and_ignores_other_blocks() {
        let parsed = parse_water_transparency_ini(
            b"Other\n Value = nope\nEnd\n\
              WaterTransparency\n TransparentWaterMinOpacity = 0.8\n TransparentWaterDepth = 2.0\nEnd\n\
              WaterTransparency\n TransparentWaterMinOpacity = 1.0 ; final\nEnd\n",
            WaterIniLimits::default(),
        )
        .expect("water transparency");
        assert_eq!(
            parsed.minimum_opacity().map(f32::to_bits),
            Some(1.0_f32.to_bits())
        );
        assert_eq!(
            parsed.opaque_depth().map(f32::to_bits),
            Some(2.0_f32.to_bits())
        );
    }

    #[test]
    fn rejects_unterminated_invalid_and_oversized_values() {
        assert_eq!(
            parse_water_ini(
                b"WaterTransparency\n TransparentWaterMinOpacity = 2\nEnd\n",
                WaterIniLimits::default(),
            ),
            Err(WaterIniError::InvalidOpacity { line: 2 })
        );
        assert_eq!(
            parse_water_ini(b"WaterSet DAWN\n", WaterIniLimits::default()),
            Err(WaterIniError::InvalidTimeOfDay { line: 1 })
        );
        assert_eq!(
            parse_water_ini(b"WaterSet NIGHT\n", WaterIniLimits::default()),
            Err(WaterIniError::UnterminatedWaterSet { line: 1 })
        );
        assert_eq!(
            parse_water_ini(
                b"WaterSet NIGHT\n DiffuseColor = R:1 G:2 B:999 A:4\nEnd\n",
                WaterIniLimits::default(),
            ),
            Err(WaterIniError::InvalidColor { line: 2 })
        );
        for invalid in [
            b"R:256 G:0 B:0".as_slice(),
            b"R:-1 G:0 B:0".as_slice(),
            b"R:1.0 G:0 B:0".as_slice(),
        ] {
            let mut input = b"WaterTransparency\n StandingWaterColor = ".to_vec();
            input.extend_from_slice(invalid);
            input.extend_from_slice(b"\nEnd\n");
            assert_eq!(
                parse_water_ini(&input, WaterIniLimits::default()),
                Err(WaterIniError::InvalidColor { line: 2 })
            );
        }
        let limits = WaterIniLimits {
            max_file_bytes: 3,
            ..WaterIniLimits::default()
        };
        assert_eq!(
            parse_water_ini(b"four", limits),
            Err(WaterIniError::FileTooLarge { size: 4, limit: 3 })
        );
    }
}
