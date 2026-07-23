// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Immutable static-scenery batches grouped for deterministic GPU instancing.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use crate::AnimatedModel;

const MAX_STATIC_MODELS: usize = 16_384;
const MAX_STATIC_INSTANCES: usize = 1_000_000;
const MAX_STATIC_NAME_BYTES: usize = 1_024;

/// Explicit renderer-only ambient sway parameters for one tree instance.
///
/// The legacy defaults and ten sway families are derived from `ScriptEngine.cpp` and
/// `W3DTreeBuffer.cpp`/`.h` in `GeneralsGameCode` revision
/// `9f7abb866f5afd446db14149979e744c7216baaf`, licensed under GPL-3.0-or-later with Electronic Arts
/// Section 7 terms. The placement-ID family assignment is project-authored so preview output is
/// stable without consuming simulation RNG.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TreeSwayPresentation {
    direction: [f32; 2],
    lean: f32,
    intensity: f32,
    period_seconds: f32,
    speed_factor: f32,
    amount_factor: f32,
}

impl TreeSwayPresentation {
    /// Returns the Zero Hour constructor-default breeze with deterministic preview variation.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn zero_hour_legacy_default(placement_id: u32) -> Self {
        const SWAY_FAMILY_COUNT: u32 = 10;
        const RANDOMNESS_DELTA: f32 = 0.1;
        let direction = std::f32::consts::PI / 3.0;
        let family = placement_id % SWAY_FAMILY_COUNT;
        let speed_unit = stable_unit_interval(family ^ 0x6d2b_79f5);
        let amount_unit = stable_unit_interval(family ^ 0x1b87_3593);
        Self {
            direction: [direction.sin(), direction.cos()],
            lean: 0.07 * std::f32::consts::PI / 4.0,
            intensity: 0.07 * std::f32::consts::PI / 4.0,
            period_seconds: 5.0,
            speed_factor: 1.0 + (speed_unit * 2.0 - 1.0) * RANDOMNESS_DELTA,
            amount_factor: 1.0 + (amount_unit * 2.0 - 1.0) * RANDOMNESS_DELTA,
        }
    }

    fn gpu_rows(self) -> [[f32; 4]; 2] {
        [
            [
                self.direction[0],
                self.direction[1],
                self.lean,
                self.intensity,
            ],
            [
                self.period_seconds,
                self.speed_factor,
                self.amount_factor,
                1.0,
            ],
        ]
    }

    /// Samples the renderer-only bend of a vertical point at explicit presentation time.
    #[must_use]
    pub fn offset_at(self, seconds: f32, local_height: f32) -> [f32; 3] {
        let phase = std::f32::consts::TAU * seconds / self.period_seconds * self.speed_factor;
        let angle = self.lean + self.intensity * phase.cos();
        let (sine, cosine) = angle.sin_cos();
        [
            self.direction[0] * sine * self.amount_factor * local_height,
            self.direction[1] * sine * self.amount_factor * local_height,
            (cosine - 1.0) * self.amount_factor * local_height,
        ]
    }
}

#[allow(clippy::cast_precision_loss)]
fn stable_unit_interval(mut value: u32) -> f32 {
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846c_a68b);
    value ^= value >> 16;
    (value & 0x00ff_ffff) as f32 / 0x00ff_ffff_u32 as f32
}

/// One source placement transformed in W3D's Z-up world space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StaticSceneryInstance {
    placement_id: u32,
    position: [f32; 3],
    angle: f32,
    scale: f32,
    tree_sway: Option<TreeSwayPresentation>,
}

impl StaticSceneryInstance {
    /// Creates one finite, positive-scale scenery instance.
    ///
    /// # Errors
    ///
    /// Returns [`StaticSceneryError::InvalidTransform`] for a non-finite component or scale at or
    /// below zero.
    pub fn new(
        placement_id: u32,
        position: [f32; 3],
        angle: f32,
        scale: f32,
    ) -> Result<Self, StaticSceneryError> {
        if position.into_iter().any(|value| !value.is_finite())
            || !angle.is_finite()
            || !scale.is_finite()
            || scale <= 0.0
        {
            return Err(StaticSceneryError::InvalidTransform(placement_id));
        }
        Ok(Self {
            placement_id,
            position,
            angle,
            scale,
            tree_sway: None,
        })
    }

    /// Adds explicit renderer-only ambient tree sway to this immutable placement.
    #[must_use]
    pub const fn with_tree_sway(mut self, tree_sway: TreeSwayPresentation) -> Self {
        self.tree_sway = Some(tree_sway);
        self
    }

    #[must_use]
    pub const fn placement_id(self) -> u32 {
        self.placement_id
    }

    #[must_use]
    pub const fn position(self) -> [f32; 3] {
        self.position
    }

    #[must_use]
    pub const fn angle(self) -> f32 {
        self.angle
    }

    #[must_use]
    pub const fn scale(self) -> f32 {
        self.scale
    }

    #[must_use]
    pub const fn tree_sway(self) -> Option<TreeSwayPresentation> {
        self.tree_sway
    }

    fn transform_rows(self) -> [[f32; 4]; 3] {
        let (sine, cosine) = self.angle.sin_cos();
        [
            [
                cosine * self.scale,
                -sine * self.scale,
                0.0,
                self.position[0],
            ],
            [
                sine * self.scale,
                cosine * self.scale,
                0.0,
                self.position[1],
            ],
            [0.0, 0.0, self.scale, self.position[2]],
        ]
    }
}

/// One unique bind-pose model and all source-ordered placements that share it.
#[derive(Debug, Clone, PartialEq)]
pub struct StagedStaticSceneryModel {
    name: Vec<u8>,
    model: AnimatedModel,
    instances: Vec<StaticSceneryInstance>,
}

impl StagedStaticSceneryModel {
    /// Creates a non-empty model instance batch.
    ///
    /// # Errors
    ///
    /// Returns a structured error for an empty/oversized name or instance list.
    pub fn new(
        name: Vec<u8>,
        model: AnimatedModel,
        instances: Vec<StaticSceneryInstance>,
    ) -> Result<Self, StaticSceneryError> {
        if name.is_empty() || name.len() > MAX_STATIC_NAME_BYTES {
            return Err(StaticSceneryError::InvalidModelName(name.len()));
        }
        if instances.is_empty() || instances.len() > MAX_STATIC_INSTANCES {
            return Err(StaticSceneryError::TooManyInstances(instances.len()));
        }
        Ok(Self {
            name,
            model,
            instances,
        })
    }

    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }

    #[must_use]
    pub fn model(&self) -> &AnimatedModel {
        &self.model
    }

    #[must_use]
    pub fn instances(&self) -> &[StaticSceneryInstance] {
        &self.instances
    }

    pub(crate) fn instance_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.instances.len().saturating_mul(80));
        for instance in &self.instances {
            for value in instance.transform_rows().into_iter().flatten() {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            let sway = instance
                .tree_sway
                .map_or([[0.0; 4]; 2], TreeSwayPresentation::gpu_rows);
            for value in sway.into_iter().flatten() {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        bytes
    }
}

/// Why one source placement could not enter a static model batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaticSceneryDiagnosticKind {
    MissingDefinition,
    MissingDefaultModel,
    MissingModelResource,
    InvalidModel,
}

/// Stable, non-fatal static-scenery resolution diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticSceneryDiagnostic {
    placement_id: u32,
    name: Vec<u8>,
    kind: StaticSceneryDiagnosticKind,
}

impl StaticSceneryDiagnostic {
    #[must_use]
    pub fn new(placement_id: u32, name: Vec<u8>, kind: StaticSceneryDiagnosticKind) -> Self {
        Self {
            placement_id,
            name,
            kind,
        }
    }

    #[must_use]
    pub const fn placement_id(&self) -> u32 {
        self.placement_id
    }

    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }

    #[must_use]
    pub const fn kind(&self) -> StaticSceneryDiagnosticKind {
        self.kind
    }
}

/// Complete immutable static-scenery presentation for one MAP.
#[derive(Debug, Clone, PartialEq)]
pub struct StagedStaticScenery {
    models: Vec<StagedStaticSceneryModel>,
    diagnostics: Vec<StaticSceneryDiagnostic>,
    instance_count: usize,
}

impl StagedStaticScenery {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            models: Vec::new(),
            diagnostics: Vec::new(),
            instance_count: 0,
        }
    }

    /// Retains deterministic model and diagnostic order after validating total limits.
    ///
    /// # Errors
    ///
    /// Returns a structured error when model or total-instance limits are exceeded.
    pub fn new(
        models: Vec<StagedStaticSceneryModel>,
        diagnostics: Vec<StaticSceneryDiagnostic>,
    ) -> Result<Self, StaticSceneryError> {
        if models.len() > MAX_STATIC_MODELS {
            return Err(StaticSceneryError::TooManyModels(models.len()));
        }
        let instance_count = models.iter().try_fold(0_usize, |total, model| {
            total.checked_add(model.instances.len())
        });
        let Some(instance_count) = instance_count else {
            return Err(StaticSceneryError::TooManyInstances(usize::MAX));
        };
        if instance_count > MAX_STATIC_INSTANCES {
            return Err(StaticSceneryError::TooManyInstances(instance_count));
        }
        Ok(Self {
            models,
            diagnostics,
            instance_count,
        })
    }

    #[must_use]
    pub fn models(&self) -> &[StagedStaticSceneryModel] {
        &self.models
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[StaticSceneryDiagnostic] {
        &self.diagnostics
    }

    #[must_use]
    pub const fn instance_count(&self) -> usize {
        self.instance_count
    }
}

/// A structured static-scenery staging failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaticSceneryError {
    InvalidTransform(u32),
    InvalidModelName(usize),
    TooManyModels(usize),
    TooManyInstances(usize),
}

impl Display for StaticSceneryError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTransform(id) => {
                write!(formatter, "static placement {id} has an invalid transform")
            }
            Self::InvalidModelName(size) => {
                write!(formatter, "static model name has invalid size {size}")
            }
            Self::TooManyModels(count) => write!(
                formatter,
                "static scenery has {count} unique models; limit is {MAX_STATIC_MODELS}"
            ),
            Self::TooManyInstances(count) => write!(
                formatter,
                "static scenery has {count} instances; limit is {MAX_STATIC_INSTANCES}"
            ),
        }
    }
}

impl Error for StaticSceneryError {}

#[cfg(test)]
mod tests {
    use super::{StaticSceneryInstance, TreeSwayPresentation};

    #[test]
    fn instance_transform_is_z_up_and_stably_packed() {
        let instance =
            StaticSceneryInstance::new(7, [10.0, 20.0, 30.0], std::f32::consts::FRAC_PI_2, 2.0)
                .expect("instance");
        let rows = instance.transform_rows();
        assert!(rows[0][0].abs() < 0.000_1);
        assert_eq!(rows[0][1].to_bits(), (-2.0_f32).to_bits());
        assert_eq!(rows[1][0].to_bits(), 2.0_f32.to_bits());
        assert_eq!(
            rows[2].map(f32::to_bits),
            [0.0, 0.0, 2.0, 30.0].map(f32::to_bits)
        );
    }

    #[test]
    fn legacy_tree_sway_is_explicit_stable_and_gpu_packed() {
        let sway = TreeSwayPresentation::zero_hour_legacy_default(17);
        assert_eq!(sway, TreeSwayPresentation::zero_hour_legacy_default(17));
        assert_ne!(sway, TreeSwayPresentation::zero_hour_legacy_default(18));
        assert_eq!(sway.period_seconds.to_bits(), 5.0_f32.to_bits());
        assert!((0.9..=1.1).contains(&sway.speed_factor));
        assert!((0.9..=1.1).contains(&sway.amount_factor));
        assert_eq!(
            sway.offset_at(2.0, 20.0).map(f32::to_bits),
            sway.offset_at(2.0, 20.0).map(f32::to_bits)
        );
        assert_ne!(
            sway.offset_at(0.0, 20.0).map(f32::to_bits),
            sway.offset_at(2.0, 20.0).map(f32::to_bits)
        );

        let instance = StaticSceneryInstance::new(17, [0.0; 3], 0.0, 1.0)
            .expect("instance")
            .with_tree_sway(sway);
        assert_eq!(instance.tree_sway(), Some(sway));
    }
}
