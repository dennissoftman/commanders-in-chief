// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Renderer-owned copies of validated, immutable MAP lighting inputs.

use cic_formats::MapLightingData;

/// One directional light copied across the parser/renderer boundary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerrainDirectionalLight {
    ambient: [f32; 3],
    diffuse: [f32; 3],
    source_direction: [f32; 3],
}

impl TerrainDirectionalLight {
    const DISABLED: Self = Self {
        ambient: [0.0; 3],
        diffuse: [0.0; 3],
        source_direction: [0.0, 0.0, -1.0],
    };

    #[must_use]
    pub const fn ambient(self) -> [f32; 3] {
        self.ambient
    }

    #[must_use]
    pub const fn diffuse(self) -> [f32; 3] {
        self.diffuse
    }

    /// Returns the source light-position vector; shading uses its negation toward the light.
    #[must_use]
    pub const fn source_direction(self) -> [f32; 3] {
        self.source_direction
    }
}

/// Three stable terrain lights for one viewer presentation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerrainLighting {
    lights: [TerrainDirectionalLight; 3],
}

impl Default for TerrainLighting {
    fn default() -> Self {
        Self::preview()
    }
}

impl TerrainLighting {
    /// Original fallback used only when a MAP has no established lighting chunk.
    #[must_use]
    pub const fn preview() -> Self {
        Self {
            lights: [
                TerrainDirectionalLight {
                    ambient: [0.38; 3],
                    diffuse: [0.62; 3],
                    source_direction: [0.45, 0.35, -0.82],
                },
                TerrainDirectionalLight::DISABLED,
                TerrainDirectionalLight::DISABLED,
            ],
        }
    }

    /// Copies the selected MAP terrain sun and accents in source order.
    #[must_use]
    pub fn from_map(lighting: &MapLightingData) -> Self {
        let mut lights = [TerrainDirectionalLight::DISABLED; 3];
        for (target, source) in lights
            .iter_mut()
            .zip(lighting.selected_period().terrain_lights())
        {
            *target = TerrainDirectionalLight {
                ambient: source.ambient(),
                diffuse: source.diffuse(),
                source_direction: source.direction(),
            };
        }
        Self { lights }
    }

    #[must_use]
    pub const fn lights(self) -> [TerrainDirectionalLight; 3] {
        self.lights
    }
}

#[cfg(test)]
mod tests {
    use cic_formats::{MapLimits, decode_map_lighting, parse_map};

    use super::TerrainLighting;

    #[test]
    fn copies_selected_terrain_lights_and_disables_missing_accents() {
        let mut payload = 2_i32.to_le_bytes().to_vec();
        for period in 0..4 {
            for light in 0..2 {
                for component in 0..9 {
                    let scalar = u16::try_from(period * 100 + light * 10 + component)
                        .expect("test scalar fits u16");
                    let value = f32::from(scalar);
                    payload.extend_from_slice(&value.to_le_bytes());
                }
            }
        }
        let mut bytes = b"CkMp".to_vec();
        bytes.extend_from_slice(&1_i32.to_le_bytes());
        bytes.push(14);
        bytes.extend_from_slice(b"GlobalLighting");
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(
            &i32::try_from(payload.len())
                .expect("test payload fits i32")
                .to_le_bytes(),
        );
        bytes.extend_from_slice(&payload);
        let map = parse_map(&bytes, "light.map", MapLimits::default()).expect("MAP");
        let decoded = decode_map_lighting(&map).expect("lighting");
        let staged = TerrainLighting::from_map(&decoded).lights();
        assert_eq!(
            staged[0].ambient().map(f32::to_bits),
            [100.0_f32, 101.0, 102.0].map(f32::to_bits)
        );
        assert_eq!(
            staged[1].diffuse().map(f32::to_bits),
            [0.0_f32.to_bits(); 3]
        );
    }
}
