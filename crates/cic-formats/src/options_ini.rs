// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Bounded decoding for the retail per-user `Options.ini` preferences file.
//!
//! Field names and value semantics — including `AntiAliasing`'s clamp-to-`[0,8]`-then-highest-bit
//! MSAA sample resolution and the uniform case-insensitive `"yes"`-or-else-`false` boolean
//! convention used by every flag in this file — are derived from `OptionPreferences.cpp` and
//! `OptionPreferences.h` in `GeneralsGameCode` revision `9f7abb866f5afd446db14149979e744c7216baaf`,
//! licensed under GPL-3.0-or-later with Electronic Arts Section 7 terms. Full notices are recorded
//! in `docs/provenance/options.md`.
//!
//! Unlike the retail `atoi`/`atof`-based reader, which never fails a parse and silently
//! substitutes defaults for garbage input, this decoder validates numeric fields explicitly and
//! reports resource-limit and value failures, matching this repository's other bounded INI
//! decoders (see `water_ini.rs`, `terrain_ini.rs`). The recognized field set was cross-checked
//! against real retail `Options.ini` samples from both Generals and Zero Hour installations; no
//! retail data is embedded in this repository.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// Explicit resource bounds for Options INI decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OptionsIniLimits {
    pub max_file_bytes: usize,
    pub max_lines: usize,
    pub max_line_bytes: usize,
    pub max_definitions: usize,
    pub max_string_bytes: usize,
}

impl Default for OptionsIniLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: 256 * 1_024,
            max_lines: 10_000,
            max_line_bytes: 4_096,
            max_definitions: 1_024,
            max_string_bytes: 1_024,
        }
    }
}

/// Last-field-wins values decoded from one flat, header-less `Options.ini` stream.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OptionsIni {
    resolution: Option<(u32, u32)>,
    antialiasing: Option<i32>,
    gamma: Option<f32>,
    music_volume: Option<f32>,
    sfx_volume: Option<f32>,
    sfx3d_volume: Option<f32>,
    voice_volume: Option<f32>,
    scroll_factor: Option<i32>,
    max_particle_count: Option<i32>,
    texture_reduction: Option<i32>,
    campaign_difficulty: Option<i32>,
    firewall_behavior: Option<i32>,
    firewall_port_override: Option<i32>,
    ip_address: Option<Vec<u8>>,
    gamespy_ip_address: Option<Vec<u8>>,
    ideal_static_game_lod: Option<Vec<u8>>,
    static_game_lod: Option<Vec<u8>>,
    language_filter: Option<bool>,
    send_delay: Option<bool>,
    use_alternate_mouse: Option<bool>,
    draw_scroll_anchor: Option<bool>,
    move_scroll_anchor: Option<bool>,
    building_occlusion: Option<bool>,
    dynamic_lod: Option<bool>,
    extra_animations: Option<bool>,
    heat_effects: Option<bool>,
    retaliation: Option<bool>,
    show_soft_water_edge: Option<bool>,
    show_trees: Option<bool>,
    use_cloud_map: Option<bool>,
    use_double_click_attack_move: Option<bool>,
    use_light_map: Option<bool>,
    use_shadow_decals: Option<bool>,
    use_shadow_volumes: Option<bool>,
    diagnostics: Vec<OptionsIniDiagnostic>,
}

impl OptionsIni {
    /// Returns the `Resolution` field as `(width, height)`.
    #[must_use]
    pub const fn resolution(&self) -> Option<(u32, u32)> {
        self.resolution
    }

    #[must_use]
    pub fn resolution_width(&self) -> Option<u32> {
        self.resolution.map(|(width, _)| width)
    }

    #[must_use]
    pub fn resolution_height(&self) -> Option<u32> {
        self.resolution.map(|(_, height)| height)
    }

    /// Returns the raw `AntiAliasing` field exactly as stored, before the source's
    /// clamp-and-highest-bit resolution.
    #[must_use]
    pub const fn antialiasing_raw(&self) -> Option<i32> {
        self.antialiasing
    }

    /// Resolves `AntiAliasing` to an active MSAA sample count using
    /// `OptionPreferences::getAntiAliasing`'s exact rule: clamp the raw value to
    /// `MULTISAMPLE_MODE_NONE..=MULTISAMPLE_MODE_8X` (`0..=8`), then retain only the highest set
    /// bit, yielding one of `0`, `2`, `4`, or `8`.
    #[must_use]
    pub fn antialiasing_msaa_samples(&self) -> Option<u32> {
        self.antialiasing.map(|raw| {
            let clamped = u32::try_from(raw.clamp(0, 8)).unwrap_or(0);
            highest_bit(clamped)
        })
    }

    #[must_use]
    pub const fn gamma(&self) -> Option<f32> {
        self.gamma
    }

    #[must_use]
    pub const fn music_volume(&self) -> Option<f32> {
        self.music_volume
    }

    #[must_use]
    pub const fn sfx_volume(&self) -> Option<f32> {
        self.sfx_volume
    }

    #[must_use]
    pub const fn sfx3d_volume(&self) -> Option<f32> {
        self.sfx3d_volume
    }

    #[must_use]
    pub const fn voice_volume(&self) -> Option<f32> {
        self.voice_volume
    }

    #[must_use]
    pub const fn scroll_factor(&self) -> Option<i32> {
        self.scroll_factor
    }

    #[must_use]
    pub const fn max_particle_count(&self) -> Option<i32> {
        self.max_particle_count
    }

    #[must_use]
    pub const fn texture_reduction(&self) -> Option<i32> {
        self.texture_reduction
    }

    #[must_use]
    pub const fn campaign_difficulty(&self) -> Option<i32> {
        self.campaign_difficulty
    }

    #[must_use]
    pub const fn firewall_behavior(&self) -> Option<i32> {
        self.firewall_behavior
    }

    #[must_use]
    pub const fn firewall_port_override(&self) -> Option<i32> {
        self.firewall_port_override
    }

    #[must_use]
    pub fn ip_address_bytes(&self) -> Option<&[u8]> {
        self.ip_address.as_deref()
    }

    #[must_use]
    pub fn gamespy_ip_address_bytes(&self) -> Option<&[u8]> {
        self.gamespy_ip_address.as_deref()
    }

    #[must_use]
    pub fn ideal_static_game_lod_bytes(&self) -> Option<&[u8]> {
        self.ideal_static_game_lod.as_deref()
    }

    #[must_use]
    pub fn static_game_lod_bytes(&self) -> Option<&[u8]> {
        self.static_game_lod.as_deref()
    }

    #[must_use]
    pub const fn language_filter(&self) -> Option<bool> {
        self.language_filter
    }

    #[must_use]
    pub const fn send_delay(&self) -> Option<bool> {
        self.send_delay
    }

    #[must_use]
    pub const fn use_alternate_mouse(&self) -> Option<bool> {
        self.use_alternate_mouse
    }

    #[must_use]
    pub const fn draw_scroll_anchor(&self) -> Option<bool> {
        self.draw_scroll_anchor
    }

    #[must_use]
    pub const fn move_scroll_anchor(&self) -> Option<bool> {
        self.move_scroll_anchor
    }

    #[must_use]
    pub const fn building_occlusion(&self) -> Option<bool> {
        self.building_occlusion
    }

    #[must_use]
    pub const fn dynamic_lod(&self) -> Option<bool> {
        self.dynamic_lod
    }

    #[must_use]
    pub const fn extra_animations(&self) -> Option<bool> {
        self.extra_animations
    }

    #[must_use]
    pub const fn heat_effects(&self) -> Option<bool> {
        self.heat_effects
    }

    #[must_use]
    pub const fn retaliation(&self) -> Option<bool> {
        self.retaliation
    }

    #[must_use]
    pub const fn show_soft_water_edge(&self) -> Option<bool> {
        self.show_soft_water_edge
    }

    #[must_use]
    pub const fn show_trees(&self) -> Option<bool> {
        self.show_trees
    }

    #[must_use]
    pub const fn use_cloud_map(&self) -> Option<bool> {
        self.use_cloud_map
    }

    #[must_use]
    pub const fn use_double_click_attack_move(&self) -> Option<bool> {
        self.use_double_click_attack_move
    }

    #[must_use]
    pub const fn use_light_map(&self) -> Option<bool> {
        self.use_light_map
    }

    #[must_use]
    pub const fn use_shadow_decals(&self) -> Option<bool> {
        self.use_shadow_decals
    }

    #[must_use]
    pub const fn use_shadow_volumes(&self) -> Option<bool> {
        self.use_shadow_volumes
    }

    /// Returns every field name in the file that this decoder does not recognize, in source
    /// order. These never fail parsing; they exist so an unsupported or missing field stays
    /// discoverable instead of disappearing silently.
    #[must_use]
    pub fn diagnostics(&self) -> &[OptionsIniDiagnostic] {
        &self.diagnostics
    }
}

/// One field name in `Options.ini` that this decoder does not specifically recognize.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionsIniDiagnostic {
    line: usize,
    field: Vec<u8>,
}

impl OptionsIniDiagnostic {
    /// Returns the one-based source line the field appeared on.
    #[must_use]
    pub const fn line(&self) -> usize {
        self.line
    }

    /// Returns the unrecognized field name exactly as spelled in the source.
    #[must_use]
    pub fn field_bytes(&self) -> &[u8] {
        &self.field
    }
}

/// A structured failure from bounded Options INI decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptionsIniError {
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
        line: usize,
        limit: usize,
    },
    ValueTooLong {
        line: usize,
        size: usize,
        limit: usize,
    },
    MissingValue {
        line: usize,
    },
    InvalidInteger {
        line: usize,
    },
    InvalidNumber {
        line: usize,
    },
    InvalidResolution {
        line: usize,
    },
}

impl Display for OptionsIniError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::FileTooLarge { size, limit } => {
                write!(formatter, "Options INI is {size} bytes; limit is {limit}")
            }
            Self::TooManyLines { limit } => write!(formatter, "Options INI exceeds {limit} lines"),
            Self::LineTooLong { line, size, limit } => write!(
                formatter,
                "Options INI line {line} is {size} bytes; limit is {limit}"
            ),
            Self::TooManyDefinitions { line, limit } => write!(
                formatter,
                "Options INI exceeds {limit} fields at line {line}"
            ),
            Self::ValueTooLong { line, size, limit } => write!(
                formatter,
                "Options INI value on line {line} is {size} bytes; limit is {limit}"
            ),
            Self::MissingValue { line } => {
                write!(formatter, "Options INI field has no value on line {line}")
            }
            Self::InvalidInteger { line } => {
                write!(formatter, "Options INI integer is invalid on line {line}")
            }
            Self::InvalidNumber { line } => {
                write!(formatter, "Options INI value is not finite on line {line}")
            }
            Self::InvalidResolution { line } => write!(
                formatter,
                "Options INI Resolution is not a \"width height\" pair on line {line}"
            ),
        }
    }
}

impl Error for OptionsIniError {}

/// Decodes every field this narrow decoder recognizes from a flat, header-less `Options.ini`
/// stream. Repeated fields use stable file-order last-field-wins semantics.
///
/// # Errors
///
/// Returns a structured error for resource-limit excess, malformed `Resolution` pairs, or
/// non-finite/non-integer numeric fields.
pub fn parse_options_ini(
    bytes: &[u8],
    limits: OptionsIniLimits,
) -> Result<OptionsIni, OptionsIniError> {
    if bytes.len() > limits.max_file_bytes {
        return Err(OptionsIniError::FileTooLarge {
            size: bytes.len(),
            limit: limits.max_file_bytes,
        });
    }
    let mut result = OptionsIni::default();
    let mut definitions = 0_usize;
    let mut diagnostics = Vec::new();
    for (zero_based_line, raw_line) in bytes.split(|byte| *byte == b'\n').enumerate() {
        let line_number = zero_based_line
            .checked_add(1)
            .ok_or(OptionsIniError::TooManyLines {
                limit: limits.max_lines,
            })?;
        if line_number > limits.max_lines {
            return Err(OptionsIniError::TooManyLines {
                limit: limits.max_lines,
            });
        }
        if raw_line.len() > limits.max_line_bytes {
            return Err(OptionsIniError::LineTooLong {
                line: line_number,
                size: raw_line.len(),
                limit: limits.max_line_bytes,
            });
        }
        let line = trim_ascii(strip_comment(raw_line));
        if line.is_empty() {
            continue;
        }
        let Some((field, value)) = split_assignment(line) else {
            continue;
        };
        definitions = definitions
            .checked_add(1)
            .ok_or(OptionsIniError::TooManyDefinitions {
                line: line_number,
                limit: limits.max_definitions,
            })?;
        if definitions > limits.max_definitions {
            return Err(OptionsIniError::TooManyDefinitions {
                line: line_number,
                limit: limits.max_definitions,
            });
        }
        apply_field(
            &mut result,
            field,
            value,
            line_number,
            limits,
            &mut diagnostics,
        )?;
    }
    result.diagnostics = diagnostics;
    Ok(result)
}

#[allow(clippy::too_many_lines)]
fn apply_field(
    result: &mut OptionsIni,
    field: &[u8],
    value: &[u8],
    line: usize,
    limits: OptionsIniLimits,
    diagnostics: &mut Vec<OptionsIniDiagnostic>,
) -> Result<(), OptionsIniError> {
    if field.eq_ignore_ascii_case(b"Resolution") {
        result.resolution = Some(parse_resolution(value, line)?);
    } else if field.eq_ignore_ascii_case(b"AntiAliasing") {
        result.antialiasing = Some(parse_i32(value, line)?);
    } else if field.eq_ignore_ascii_case(b"Gamma") {
        result.gamma = Some(parse_number(value, line)?);
    } else if field.eq_ignore_ascii_case(b"MusicVolume") {
        result.music_volume = Some(parse_number(value, line)?);
    } else if field.eq_ignore_ascii_case(b"SFXVolume") {
        result.sfx_volume = Some(parse_number(value, line)?);
    } else if field.eq_ignore_ascii_case(b"SFX3DVolume") {
        result.sfx3d_volume = Some(parse_number(value, line)?);
    } else if field.eq_ignore_ascii_case(b"VoiceVolume") {
        result.voice_volume = Some(parse_number(value, line)?);
    } else if field.eq_ignore_ascii_case(b"ScrollFactor") {
        result.scroll_factor = Some(parse_i32(value, line)?);
    } else if field.eq_ignore_ascii_case(b"MaxParticleCount") {
        result.max_particle_count = Some(parse_i32(value, line)?);
    } else if field.eq_ignore_ascii_case(b"TextureReduction") {
        result.texture_reduction = Some(parse_i32(value, line)?);
    } else if field.eq_ignore_ascii_case(b"CampaignDifficulty") {
        result.campaign_difficulty = Some(parse_i32(value, line)?);
    } else if field.eq_ignore_ascii_case(b"FirewallBehavior") {
        result.firewall_behavior = Some(parse_i32(value, line)?);
    } else if field.eq_ignore_ascii_case(b"FirewallPortOverride") {
        result.firewall_port_override = Some(parse_i32(value, line)?);
    } else if field.eq_ignore_ascii_case(b"IPAddress") {
        result.ip_address = Some(parse_optional_string(value, line, limits)?);
    } else if field.eq_ignore_ascii_case(b"GameSpyIPAddress") {
        result.gamespy_ip_address = Some(parse_optional_string(value, line, limits)?);
    } else if field.eq_ignore_ascii_case(b"IdealStaticGameLOD") {
        result.ideal_static_game_lod = Some(parse_optional_string(value, line, limits)?);
    } else if field.eq_ignore_ascii_case(b"StaticGameLOD") {
        result.static_game_lod = Some(parse_optional_string(value, line, limits)?);
    } else if field.eq_ignore_ascii_case(b"LanguageFilter") {
        result.language_filter = Some(parse_yes_flag(value));
    } else if field.eq_ignore_ascii_case(b"SendDelay") {
        result.send_delay = Some(parse_yes_flag(value));
    } else if field.eq_ignore_ascii_case(b"UseAlternateMouse") {
        result.use_alternate_mouse = Some(parse_yes_flag(value));
    } else if field.eq_ignore_ascii_case(b"DrawScrollAnchor") {
        result.draw_scroll_anchor = Some(parse_yes_flag(value));
    } else if field.eq_ignore_ascii_case(b"MoveScrollAnchor") {
        result.move_scroll_anchor = Some(parse_yes_flag(value));
    } else if field.eq_ignore_ascii_case(b"BuildingOcclusion") {
        result.building_occlusion = Some(parse_yes_flag(value));
    } else if field.eq_ignore_ascii_case(b"DynamicLOD") {
        result.dynamic_lod = Some(parse_yes_flag(value));
    } else if field.eq_ignore_ascii_case(b"ExtraAnimations") {
        result.extra_animations = Some(parse_yes_flag(value));
    } else if field.eq_ignore_ascii_case(b"HeatEffects") {
        result.heat_effects = Some(parse_yes_flag(value));
    } else if field.eq_ignore_ascii_case(b"Retaliation") {
        result.retaliation = Some(parse_yes_flag(value));
    } else if field.eq_ignore_ascii_case(b"ShowSoftWaterEdge") {
        result.show_soft_water_edge = Some(parse_yes_flag(value));
    } else if field.eq_ignore_ascii_case(b"ShowTrees") {
        result.show_trees = Some(parse_yes_flag(value));
    } else if field.eq_ignore_ascii_case(b"UseCloudMap") {
        result.use_cloud_map = Some(parse_yes_flag(value));
    } else if field.eq_ignore_ascii_case(b"UseDoubleClickAttackMove") {
        result.use_double_click_attack_move = Some(parse_yes_flag(value));
    } else if field.eq_ignore_ascii_case(b"UseLightMap") {
        result.use_light_map = Some(parse_yes_flag(value));
    } else if field.eq_ignore_ascii_case(b"UseShadowDecals") {
        result.use_shadow_decals = Some(parse_yes_flag(value));
    } else if field.eq_ignore_ascii_case(b"UseShadowVolumes") {
        result.use_shadow_volumes = Some(parse_yes_flag(value));
    } else {
        diagnostics.push(OptionsIniDiagnostic {
            line,
            field: field.to_vec(),
        });
    }
    Ok(())
}

/// Retains only the highest set bit, so `5` (`0b101`) becomes `4` and `0` stays `0`. This mirrors
/// `OptionPreferences::getAntiAliasing`'s `highestBit` step.
const fn highest_bit(value: u32) -> u32 {
    if value == 0 {
        0
    } else {
        1_u32 << (31 - value.leading_zeros())
    }
}

/// The source treats every boolean preference identically: only an exact case-insensitive `"yes"`
/// is true; anything else — `"no"`, garbage, or a field left blank as `DrawScrollAnchor` and
/// `MoveScrollAnchor` are in real retail files — is false. This never fails.
fn parse_yes_flag(value: &[u8]) -> bool {
    trim_ascii(value).eq_ignore_ascii_case(b"yes")
}

fn parse_i32(value: &[u8], line: usize) -> Result<i32, OptionsIniError> {
    let trimmed = trim_ascii(value);
    if trimmed.is_empty() {
        return Err(OptionsIniError::MissingValue { line });
    }
    std::str::from_utf8(trimmed)
        .ok()
        .and_then(|text| text.parse::<i32>().ok())
        .ok_or(OptionsIniError::InvalidInteger { line })
}

fn parse_number(value: &[u8], line: usize) -> Result<f32, OptionsIniError> {
    let trimmed = trim_ascii(value);
    if trimmed.is_empty() {
        return Err(OptionsIniError::MissingValue { line });
    }
    std::str::from_utf8(trimmed)
        .ok()
        .and_then(|text| text.parse::<f32>().ok())
        .filter(|value| value.is_finite())
        .ok_or(OptionsIniError::InvalidNumber { line })
}

fn parse_resolution(value: &[u8], line: usize) -> Result<(u32, u32), OptionsIniError> {
    let trimmed = trim_ascii(value);
    if trimmed.is_empty() {
        return Err(OptionsIniError::MissingValue { line });
    }
    let text =
        std::str::from_utf8(trimmed).map_err(|_| OptionsIniError::InvalidResolution { line })?;
    let mut fields = text.split_ascii_whitespace();
    let width = fields
        .next()
        .and_then(|field| field.parse::<u32>().ok())
        .ok_or(OptionsIniError::InvalidResolution { line })?;
    let height = fields
        .next()
        .and_then(|field| field.parse::<u32>().ok())
        .ok_or(OptionsIniError::InvalidResolution { line })?;
    if fields.next().is_some() {
        return Err(OptionsIniError::InvalidResolution { line });
    }
    Ok((width, height))
}

fn parse_optional_string(
    value: &[u8],
    line: usize,
    limits: OptionsIniLimits,
) -> Result<Vec<u8>, OptionsIniError> {
    let mut value = trim_ascii(value);
    if value.len() >= 2
        && ((value[0] == b'"' && value[value.len() - 1] == b'"')
            || (value[0] == b'\'' && value[value.len() - 1] == b'\''))
    {
        value = &value[1..value.len() - 1];
    }
    if value.len() > limits.max_string_bytes {
        return Err(OptionsIniError::ValueTooLong {
            line,
            size: value.len(),
            limit: limits.max_string_bytes,
        });
    }
    Ok(value.to_vec())
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
    use super::{OptionsIniError, OptionsIniLimits, parse_options_ini};

    // A synthetic fixture in the same flat key=value vocabulary observed in real retail
    // Generals/Zero Hour Options.ini files, authored for this test rather than copied from any
    // installation.
    const SAMPLE: &[u8] = b"AntiAliasing = 5\n\
        BuildingOcclusion = yes\n\
        CampaignDifficulty = 1\n\
        DrawScrollAnchor = \n\
        DynamicLOD = yes\n\
        ExtraAnimations = yes\n\
        FirewallBehavior = 0\n\
        FirewallPortOverride = 0\n\
        GameSpyIPAddress = 0.0.0.0\n\
        Gamma = 50\n\
        HeatEffects = yes\n\
        IPAddress = 0.0.0.0\n\
        IdealStaticGameLOD = Low\n\
        LanguageFilter = false\n\
        MaxParticleCount = 2500\n\
        MoveScrollAnchor = \n\
        MusicVolume = 16\n\
        Resolution = 3840 2160\n\
        Retaliation = yes\n\
        SFX3DVolume = 18\n\
        SFXVolume = 16\n\
        ScrollFactor = 76\n\
        SendDelay = yes\n\
        ShowSoftWaterEdge = yes\n\
        ShowTrees = yes\n\
        StaticGameLOD = High\n\
        TextureReduction = 2\n\
        UseAlternateMouse = yes\n\
        UseCloudMap = yes\n\
        UseDoubleClickAttackMove = yes\n\
        UseLightMap = no\n\
        UseShadowDecals = yes\n\
        UseShadowVolumes = no\n\
        VoiceVolume = 17\n";

    #[test]
    fn decodes_every_recognized_field_from_a_realistic_flat_file() {
        let parsed = parse_options_ini(SAMPLE, OptionsIniLimits::default()).expect("options INI");
        assert_eq!(parsed.resolution(), Some((3840, 2160)));
        assert_eq!(parsed.resolution_width(), Some(3840));
        assert_eq!(parsed.resolution_height(), Some(2160));
        assert_eq!(parsed.antialiasing_raw(), Some(5));
        assert_eq!(parsed.antialiasing_msaa_samples(), Some(4));
        assert_eq!(parsed.gamma(), Some(50.0));
        assert_eq!(parsed.music_volume(), Some(16.0));
        assert_eq!(parsed.sfx_volume(), Some(16.0));
        assert_eq!(parsed.sfx3d_volume(), Some(18.0));
        assert_eq!(parsed.voice_volume(), Some(17.0));
        assert_eq!(parsed.scroll_factor(), Some(76));
        assert_eq!(parsed.max_particle_count(), Some(2500));
        assert_eq!(parsed.texture_reduction(), Some(2));
        assert_eq!(parsed.campaign_difficulty(), Some(1));
        assert_eq!(parsed.firewall_behavior(), Some(0));
        assert_eq!(parsed.firewall_port_override(), Some(0));
        assert_eq!(parsed.ip_address_bytes(), Some(b"0.0.0.0".as_slice()));
        assert_eq!(
            parsed.gamespy_ip_address_bytes(),
            Some(b"0.0.0.0".as_slice())
        );
        assert_eq!(
            parsed.ideal_static_game_lod_bytes(),
            Some(b"Low".as_slice())
        );
        assert_eq!(parsed.static_game_lod_bytes(), Some(b"High".as_slice()));
        assert_eq!(parsed.language_filter(), Some(false));
        assert_eq!(parsed.send_delay(), Some(true));
        assert_eq!(parsed.use_alternate_mouse(), Some(true));
        // Blank in the real retail file; only an exact "yes" is ever true.
        assert_eq!(parsed.draw_scroll_anchor(), Some(false));
        assert_eq!(parsed.move_scroll_anchor(), Some(false));
        assert_eq!(parsed.building_occlusion(), Some(true));
        assert_eq!(parsed.dynamic_lod(), Some(true));
        assert_eq!(parsed.extra_animations(), Some(true));
        assert_eq!(parsed.heat_effects(), Some(true));
        assert_eq!(parsed.retaliation(), Some(true));
        assert_eq!(parsed.show_soft_water_edge(), Some(true));
        assert_eq!(parsed.show_trees(), Some(true));
        assert_eq!(parsed.use_cloud_map(), Some(true));
        assert_eq!(parsed.use_double_click_attack_move(), Some(true));
        assert_eq!(parsed.use_light_map(), Some(false));
        assert_eq!(parsed.use_shadow_decals(), Some(true));
        assert_eq!(parsed.use_shadow_volumes(), Some(false));
        assert!(parsed.diagnostics().is_empty());
    }

    #[test]
    fn missing_fields_stay_none_instead_of_defaulting() {
        let parsed = parse_options_ini(b"Resolution = 1024 768\n", OptionsIniLimits::default())
            .expect("options INI");
        assert_eq!(parsed.resolution(), Some((1024, 768)));
        assert_eq!(parsed.antialiasing_raw(), None);
        assert_eq!(parsed.antialiasing_msaa_samples(), None);
        assert_eq!(parsed.gamma(), None);
        assert_eq!(parsed.send_delay(), None);
        assert_eq!(parsed.ip_address_bytes(), None);
    }

    #[test]
    fn diagnoses_unrecognized_fields_without_dropping_recognized_data() {
        let parsed = parse_options_ini(
            b"AntiAliasing = 4\nSuperSampling = 2\nFutureFeature = maybe\n",
            OptionsIniLimits::default(),
        )
        .expect("options INI");
        assert_eq!(parsed.antialiasing_raw(), Some(4));
        assert_eq!(parsed.diagnostics().len(), 2);
        assert_eq!(parsed.diagnostics()[0].line(), 2);
        assert_eq!(parsed.diagnostics()[0].field_bytes(), b"SuperSampling");
        assert_eq!(parsed.diagnostics()[1].line(), 3);
        assert_eq!(parsed.diagnostics()[1].field_bytes(), b"FutureFeature");
    }

    #[test]
    fn rejects_malformed_numeric_and_resolution_values() {
        assert_eq!(
            parse_options_ini(b"AntiAliasing = nope\n", OptionsIniLimits::default()),
            Err(OptionsIniError::InvalidInteger { line: 1 })
        );
        assert_eq!(
            parse_options_ini(b"Gamma = \n", OptionsIniLimits::default()),
            Err(OptionsIniError::MissingValue { line: 1 })
        );
        assert_eq!(
            parse_options_ini(b"MusicVolume = loud\n", OptionsIniLimits::default()),
            Err(OptionsIniError::InvalidNumber { line: 1 })
        );
        assert_eq!(
            parse_options_ini(b"Resolution = 1920\n", OptionsIniLimits::default()),
            Err(OptionsIniError::InvalidResolution { line: 1 })
        );
        assert_eq!(
            parse_options_ini(b"Resolution = 1920 1080 60\n", OptionsIniLimits::default()),
            Err(OptionsIniError::InvalidResolution { line: 1 })
        );
        assert_eq!(
            parse_options_ini(b"Resolution = wide tall\n", OptionsIniLimits::default()),
            Err(OptionsIniError::InvalidResolution { line: 1 })
        );
    }

    #[test]
    fn resolves_msaa_samples_by_clamping_then_retaining_the_highest_bit() {
        for (raw, samples) in [
            (0, 0),
            (1, 1),
            (3, 2),
            (5, 4),
            (6, 4),
            (8, 8),
            (100, 8),
            (-3, 0),
        ] {
            let text = format!("AntiAliasing = {raw}\n");
            let parsed = parse_options_ini(text.as_bytes(), OptionsIniLimits::default())
                .expect("options INI");
            assert_eq!(
                parsed.antialiasing_msaa_samples(),
                Some(samples),
                "raw {raw} should resolve to {samples} MSAA samples"
            );
        }
    }

    #[test]
    fn rejects_oversized_input() {
        let limits = OptionsIniLimits {
            max_file_bytes: 3,
            ..OptionsIniLimits::default()
        };
        assert_eq!(
            parse_options_ini(b"four", limits),
            Err(OptionsIniError::FileTooLarge { size: 4, limit: 3 })
        );
        let limits = OptionsIniLimits {
            max_definitions: 1,
            ..OptionsIniLimits::default()
        };
        assert_eq!(
            parse_options_ini(b"Gamma = 50\nMusicVolume = 10\n", limits),
            Err(OptionsIniError::TooManyDefinitions { line: 2, limit: 1 })
        );
    }
}
