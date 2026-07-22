use std::collections::BTreeMap;

use cic_formats::{
    W3dAnimation, W3dAnimationChannel, W3dAnimationChannelKind, W3dMaterialPass, W3dModel,
    W3dStaticMesh,
};

use crate::{RenderError, TextureId, TextureResourceManager};

const MAX_GEOMETRY_BUFFER_BYTES: usize = 512 * 1_024 * 1_024;
const MAX_ABS_RENDER_COORDINATE: f32 = 1_000_000_000.0;
const HIDDEN_ATTACHMENT_MIN_DISTANCE: f32 = 100.0;
const HIDDEN_ATTACHMENT_MODEL_MULTIPLIER: f32 = 32.0;
const HIDDEN_ATTACHMENT_SCALE: f32 = 0.000_1;

#[derive(Debug, Clone, Copy, PartialEq)]
struct ModelVertex {
    position: [f32; 3],
    normal: [f32; 3],
    color: [f32; 4],
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct AnimatedVertex {
    position: [f32; 3],
    normal: [f32; 3],
    color: [f32; 4],
    texcoord: [f32; 2],
    mapper: StagedMapper,
    pivot: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct StagedMapper {
    mode: u8,
    values: [f32; 8],
}

impl Default for StagedMapper {
    fn default() -> Self {
        Self {
            mode: 0,
            values: [0.0; 8],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ModelFraming {
    center: [f32; 3],
    scale: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum BlendMode {
    Opaque,
    Alpha,
    Additive,
    Multiply,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct StagedMaterial {
    pub(crate) texture: TextureId,
    pub(crate) clamp_u: bool,
    pub(crate) clamp_v: bool,
    pub(crate) blend: BlendMode,
    pub(crate) alpha_test: bool,
    pub(crate) depth_write: bool,
    pub(crate) two_sided: bool,
}

// Provenance: `W3D_MESH_FLAG_TWO_SIDED` is defined as 0x00002000 in `w3d_file.h` at
// GeneralsGameCode revision `9f7abb866f5afd446db14149979e744c7216baaf`; see
// `docs/provenance/w3d.md` for license and notice details.
const W3D_MESH_FLAG_TWO_SIDED: u32 = 0x0000_2000;

const fn mesh_is_two_sided(attributes: u32) -> bool {
    attributes & W3D_MESH_FLAG_TWO_SIDED != 0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DrawRange {
    pub(crate) material: usize,
    pub(crate) first_index: u32,
    pub(crate) index_count: u32,
    pub(crate) pass: usize,
    pub(crate) stage: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct StagedPivot {
    parent: Option<u32>,
    translation: [f32; 3],
    rotation: [f32; 4],
}

#[derive(Debug, Clone, Copy)]
struct Transform {
    translation: [f32; 3],
    rotation: [f32; 4],
    scale: f32,
}

impl Transform {
    const IDENTITY: Self = Self {
        translation: [0.0; 3],
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: 1.0,
    };

    fn compose(self, local: Self) -> Self {
        let translated = rotate(self.rotation, scale(local.translation, self.scale));
        Self {
            translation: add(self.translation, translated),
            rotation: normalize_quaternion(multiply(self.rotation, local.rotation)),
            scale: self.scale * local.scale,
        }
    }

    fn point(self, value: [f32; 3]) -> [f32; 3] {
        add(
            self.translation,
            rotate(self.rotation, scale(value, self.scale)),
        )
    }

    fn vector(self, value: [f32; 3]) -> [f32; 3] {
        rotate(self.rotation, value)
    }
}

/// Immutable bind-pose geometry staged from a validated composed W3D model.
#[derive(Debug, Clone, PartialEq)]
pub struct StagedModel {
    vertices: Vec<ModelVertex>,
    indices: Vec<u32>,
    minimum: [f32; 3],
    maximum: [f32; 3],
}

/// Immutable local-space geometry, hierarchy, and clips staged for explicit-frame animation.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimatedModel {
    vertices: Vec<AnimatedVertex>,
    indices: Vec<u32>,
    pivots: Vec<StagedPivot>,
    animations: Vec<W3dAnimation>,
    hidden_attachment_distance: f32,
    texture_resources: TextureResourceManager,
    materials: Vec<StagedMaterial>,
    draws: Vec<DrawRange>,
}

impl AnimatedModel {
    /// Copies a validated composed W3D model without retaining VFS or parser state.
    ///
    /// # Errors
    ///
    /// Returns a structured error when geometry or hierarchy references exceed renderer limits.
    #[allow(clippy::too_many_lines)]
    pub fn from_w3d(model: &W3dModel) -> Result<Self, RenderError> {
        Self::from_w3d_with_textures(model, TextureResourceManager::default())
    }

    /// Copies a validated composed W3D model and its caller-resolved texture resources.
    ///
    /// # Errors
    ///
    /// Returns a structured error when geometry or hierarchy references exceed renderer limits.
    #[allow(clippy::too_many_lines)]
    pub fn from_w3d_with_textures(
        model: &W3dModel,
        texture_resources: TextureResourceManager,
    ) -> Result<Self, RenderError> {
        // Exercise the same bounds and hierarchy validation as deterministic bind-pose capture.
        let staged = StagedModel::from_w3d(model)?;
        let initial_vertex_count = staged.index_count();
        if initial_vertex_count
            .checked_mul(36)
            .is_none_or(|bytes| bytes > MAX_GEOMETRY_BUFFER_BYTES)
        {
            return Err(RenderError::GeometryTooLarge);
        }
        let mut vertices = Vec::with_capacity(initial_vertex_count);
        let mut indices = Vec::with_capacity(staged.index_count());
        let mut materials = Vec::new();
        let mut material_indices = BTreeMap::new();
        let mut draws: Vec<DrawRange> = Vec::new();
        for model_mesh in model.meshes() {
            let mesh = model_mesh.mesh();
            let mapper_table = mesh
                .materials()
                .vertex_materials()
                .iter()
                .map(|material| {
                    [
                        material
                            .mapper(0)
                            .map_or_else(StagedMapper::default, stage_mapper),
                        material
                            .mapper(1)
                            .map_or_else(StagedMapper::default, stage_mapper),
                    ]
                })
                .collect::<Vec<_>>();
            let pass_count = mesh.materials().passes().len().max(1);
            for pass_index in 0..pass_count {
                let pass = mesh.materials().passes().get(pass_index);
                let stage_count = pass.map_or(1, |pass| pass.texture_stages().len().max(1));
                for stage_index in 0..stage_count {
                    let stage = pass.and_then(|pass| pass.texture_stages().get(stage_index));
                    for (triangle_index, triangle) in mesh.triangles().iter().enumerate() {
                        let material = staged_material(
                            mesh,
                            pass_index,
                            stage_index,
                            triangle_index,
                            &texture_resources,
                        );
                        let material_index =
                            *material_indices.entry(material).or_insert_with(|| {
                                let index = materials.len();
                                materials.push(material);
                                index
                            });
                        let first_index = u32::try_from(indices.len())
                            .map_err(|_| RenderError::GeometryTooLarge)?;
                        if let Some(draw) = draws.last_mut()
                            && draw.material == material_index
                            && draw.pass == pass_index
                            && draw.stage == stage_index
                            && draw
                                .first_index
                                .checked_add(draw.index_count)
                                .is_some_and(|end| end == first_index)
                        {
                            draw.index_count = draw
                                .index_count
                                .checked_add(3)
                                .ok_or(RenderError::GeometryTooLarge)?;
                        } else {
                            draws.push(DrawRange {
                                material: material_index,
                                first_index,
                                index_count: 3,
                                pass: pass_index,
                                stage: stage_index,
                            });
                        }
                        let source_indices = triangle.vertex_indices();
                        let uv_indices = stage.and_then(|stage| {
                            stage.coordinate_indices(triangle_index, source_indices)
                        });
                        for corner in 0..3 {
                            let vertex_index = usize::try_from(source_indices[corner])
                                .map_err(|_| RenderError::GeometryTooLarge)?;
                            let position = mesh.vertices()[vertex_index];
                            let normal = mesh.normals()[vertex_index];
                            let pivot = mesh
                                .vertex_bones()
                                .map_or(model_mesh.pivot(), |bones| u32::from(bones[vertex_index]));
                            let pivot = if usize::try_from(pivot)
                                .ok()
                                .is_none_or(|pivot| pivot >= model.hierarchy().pivots().len())
                            {
                                model_mesh.pivot()
                            } else {
                                pivot
                            };
                            let color = preview_color(mesh, pass, vertex_index);
                            let texcoord = stage
                                .zip(uv_indices)
                                .and_then(|(stage, indices)| {
                                    usize::try_from(indices[corner])
                                        .ok()
                                        .and_then(|index| stage.texture_coordinates().get(index))
                                })
                                .map_or([0.0, 0.0], |uv| finite_texcoord(uv.u(), uv.v()));
                            let mapper =
                                staged_mapper(pass, stage_index, vertex_index, &mapper_table);
                            let index = u32::try_from(vertices.len())
                                .map_err(|_| RenderError::GeometryTooLarge)?;
                            let next_bytes = vertices
                                .len()
                                .checked_add(1)
                                .and_then(|count| count.checked_mul(36))
                                .ok_or(RenderError::GeometryTooLarge)?;
                            if next_bytes > MAX_GEOMETRY_BUFFER_BYTES {
                                return Err(RenderError::GeometryTooLarge);
                            }
                            vertices.push(AnimatedVertex {
                                position: [position.x(), position.y(), position.z()],
                                normal: [normal.x(), normal.y(), normal.z()],
                                color,
                                texcoord,
                                mapper,
                                pivot,
                            });
                            indices.push(index);
                        }
                    }
                }
            }
        }
        let pivots = model
            .hierarchy()
            .pivots()
            .iter()
            .map(|pivot| StagedPivot {
                parent: pivot.parent(),
                translation: pivot.translation(),
                rotation: normalize_quaternion(pivot.rotation().components()),
            })
            .collect();
        Ok(Self {
            vertices,
            indices,
            pivots,
            animations: model.animations().to_vec(),
            hidden_attachment_distance: hidden_attachment_distance(&staged),
            texture_resources,
            materials,
            draws,
        })
    }

    /// Stitches and deforms one intact `TerrainBridge` between source endpoints.
    ///
    /// Provenance: section naming, span-count rounding, X-offsets, and the bridge basis follow
    /// `W3DBridgeBuffer.cpp` from `GeneralsGameCode` revision
    /// `9f7abb866f5afd446db14149979e744c7216baaf`; notices are recorded in
    /// `docs/provenance/map.md`. Texture/material staging remains project-authored.
    ///
    /// # Errors
    ///
    /// Returns a structured error when the bridge lacks a usable `BRIDGE_LEFT` section, its
    /// endpoints or natural dimensions are degenerate, or expanded geometry exceeds limits.
    #[allow(clippy::too_many_lines)]
    pub fn from_bridge_w3d_with_textures(
        model: &W3dModel,
        texture_resources: TextureResourceManager,
        start: [f32; 3],
        end: [f32; 3],
        bridge_scale: f32,
    ) -> Result<Self, RenderError> {
        if start
            .into_iter()
            .chain(end)
            .chain([bridge_scale])
            .any(|value| !value.is_finite())
            || bridge_scale <= 0.0
        {
            return Err(RenderError::NonFinitePose);
        }
        let worlds = bind_pose_transforms(model)?;
        let left = model
            .meshes()
            .iter()
            .find(|mesh| mesh_name_eq(mesh.mesh(), b"BRIDGE_LEFT"))
            .ok_or(RenderError::EmptyModel)?;
        let span = model
            .meshes()
            .iter()
            .find(|mesh| mesh_name_eq(mesh.mesh(), b"BRIDGE_SPAN"));
        let right = model
            .meshes()
            .iter()
            .find(|mesh| mesh_name_eq(mesh.mesh(), b"BRIDGE_RIGHT"));
        let left_bounds = mesh_bind_x_bounds(left, &worlds)?;
        let mut natural_length;
        let mut occurrences = Vec::new();
        if let (Some(span), Some(right)) = (span, right) {
            let right_bounds = mesh_bind_x_bounds(right, &worlds)?;
            let span_length = right_bounds[0] - left_bounds[1];
            natural_length = right_bounds[1] - left_bounds[0];
            if !span_length.is_finite()
                || span_length <= f32::EPSILON
                || !natural_length.is_finite()
                || natural_length <= f32::EPSILON
            {
                return Err(RenderError::GeometryOutsideLimits);
            }
            let desired = dot(subtract(end, start), subtract(end, start)).sqrt();
            let spannable = desired - (natural_length - span_length);
            let (span_count, repeated_length, generated_length) =
                bridge_span_layout(spannable, span_length, natural_length)?;
            if generated_length <= f32::EPSILON || !generated_length.is_finite() {
                return Err(RenderError::GeometryOutsideLimits);
            }
            occurrences.push((left, -left_bounds[0]));
            let mut span_offset = 0.0_f32;
            for _ in 0..span_count {
                occurrences.push((span, -left_bounds[0] + span_offset));
                span_offset += span_length;
            }
            occurrences.push((right, -left_bounds[0] + repeated_length - span_length));
            natural_length = generated_length;
        } else {
            natural_length = left_bounds[1] - left_bounds[0];
            if !natural_length.is_finite() || natural_length <= f32::EPSILON {
                return Err(RenderError::GeometryOutsideLimits);
            }
            occurrences.push((left, -left_bounds[0]));
        }

        let delta = subtract(end, start);
        let horizontal = delta[0].hypot(delta[1]);
        if horizontal <= f32::EPSILON {
            return Err(RenderError::GeometryOutsideLimits);
        }
        let axis = scale(delta, natural_length.recip());
        let lateral = [
            -delta[1] / horizontal * bridge_scale,
            delta[0] / horizontal * bridge_scale,
            0.0,
        ];
        let direction = normalize_vector(delta);
        let lateral_direction = normalize_vector(lateral);
        let up = scale(cross(direction, lateral_direction), bridge_scale);

        let estimated_triangles = occurrences.iter().try_fold(0_usize, |total, (mesh, _)| {
            let pass_stages = mesh
                .mesh()
                .materials()
                .passes()
                .iter()
                .map(|pass| pass.texture_stages().len().max(1))
                .sum::<usize>()
                .max(1);
            total
                .checked_add(
                    mesh.mesh()
                        .triangles()
                        .len()
                        .checked_mul(pass_stages)
                        .ok_or(RenderError::GeometryTooLarge)?,
                )
                .ok_or(RenderError::GeometryTooLarge)
        })?;
        let vertex_capacity = estimated_triangles
            .checked_mul(3)
            .ok_or(RenderError::GeometryTooLarge)?;
        if vertex_capacity
            .checked_mul(36)
            .is_none_or(|bytes| bytes > MAX_GEOMETRY_BUFFER_BYTES)
        {
            return Err(RenderError::GeometryTooLarge);
        }
        let mut vertices = Vec::with_capacity(vertex_capacity);
        let mut indices = Vec::with_capacity(vertex_capacity);
        let mut materials = Vec::new();
        let mut material_indices = BTreeMap::new();
        let mut draws: Vec<DrawRange> = Vec::new();
        for (model_mesh, x_offset) in occurrences {
            let mesh = model_mesh.mesh();
            let rigid = worlds
                .get(
                    usize::try_from(model_mesh.pivot())
                        .map_err(|_| RenderError::InvalidHierarchy)?,
                )
                .copied()
                .ok_or(RenderError::InvalidHierarchy)?;
            let mapper_table = mesh
                .materials()
                .vertex_materials()
                .iter()
                .map(|material| {
                    [
                        material
                            .mapper(0)
                            .map_or_else(StagedMapper::default, stage_mapper),
                        material
                            .mapper(1)
                            .map_or_else(StagedMapper::default, stage_mapper),
                    ]
                })
                .collect::<Vec<_>>();
            let pass_count = mesh.materials().passes().len().max(1);
            for pass_index in 0..pass_count {
                let pass = mesh.materials().passes().get(pass_index);
                let stage_count = pass.map_or(1, |pass| pass.texture_stages().len().max(1));
                for stage_index in 0..stage_count {
                    let stage = pass.and_then(|pass| pass.texture_stages().get(stage_index));
                    for (triangle_index, triangle) in mesh.triangles().iter().enumerate() {
                        let material = staged_material(
                            mesh,
                            pass_index,
                            stage_index,
                            triangle_index,
                            &texture_resources,
                        );
                        let material_index =
                            *material_indices.entry(material).or_insert_with(|| {
                                let index = materials.len();
                                materials.push(material);
                                index
                            });
                        let first_index = u32::try_from(indices.len())
                            .map_err(|_| RenderError::GeometryTooLarge)?;
                        if let Some(draw) = draws.last_mut()
                            && draw.material == material_index
                            && draw.pass == pass_index
                            && draw.stage == stage_index
                            && draw.first_index.checked_add(draw.index_count) == Some(first_index)
                        {
                            draw.index_count = draw
                                .index_count
                                .checked_add(3)
                                .ok_or(RenderError::GeometryTooLarge)?;
                        } else {
                            draws.push(DrawRange {
                                material: material_index,
                                first_index,
                                index_count: 3,
                                pass: pass_index,
                                stage: stage_index,
                            });
                        }
                        let source_indices = triangle.vertex_indices();
                        let uv_indices = stage.and_then(|stage| {
                            stage.coordinate_indices(triangle_index, source_indices)
                        });
                        for corner in 0..3 {
                            let vertex_index = usize::try_from(source_indices[corner])
                                .map_err(|_| RenderError::GeometryTooLarge)?;
                            let source_position = mesh.vertices()[vertex_index];
                            let source_normal = mesh.normals()[vertex_index];
                            let transform = mesh
                                .vertex_bones()
                                .and_then(|bones| {
                                    worlds.get(usize::from(bones[vertex_index])).copied()
                                })
                                .unwrap_or(rigid);
                            let bound_position = transform.point([
                                source_position.x(),
                                source_position.y(),
                                source_position.z(),
                            ]);
                            let bound_normal = transform.vector([
                                source_normal.x(),
                                source_normal.y(),
                                source_normal.z(),
                            ]);
                            let position = add(
                                start,
                                add(
                                    scale(axis, bound_position[0] + x_offset),
                                    add(
                                        scale(lateral, bound_position[1]),
                                        scale(up, bound_position[2]),
                                    ),
                                ),
                            );
                            let normal = normalize_vector(add(
                                scale(axis, bound_normal[0]),
                                add(scale(lateral, bound_normal[1]), scale(up, bound_normal[2])),
                            ));
                            if position.into_iter().chain(normal).any(|value| {
                                !value.is_finite() || value.abs() > MAX_ABS_RENDER_COORDINATE
                            }) {
                                return Err(RenderError::GeometryOutsideLimits);
                            }
                            let color = preview_color(mesh, pass, vertex_index);
                            let texcoord = stage
                                .zip(uv_indices)
                                .and_then(|(stage, indices)| {
                                    usize::try_from(indices[corner])
                                        .ok()
                                        .and_then(|index| stage.texture_coordinates().get(index))
                                })
                                .map_or([0.0, 0.0], |uv| finite_texcoord(uv.u(), uv.v()));
                            let mapper =
                                staged_mapper(pass, stage_index, vertex_index, &mapper_table);
                            let index = u32::try_from(vertices.len())
                                .map_err(|_| RenderError::GeometryTooLarge)?;
                            vertices.push(AnimatedVertex {
                                position,
                                normal,
                                color,
                                texcoord,
                                mapper,
                                pivot: 0,
                            });
                            indices.push(index);
                        }
                    }
                }
            }
        }
        if vertices.is_empty() {
            return Err(RenderError::EmptyModel);
        }
        Ok(Self {
            vertices,
            indices,
            pivots: vec![StagedPivot {
                parent: None,
                translation: [0.0; 3],
                rotation: [0.0, 0.0, 0.0, 1.0],
            }],
            animations: Vec::new(),
            hidden_attachment_distance: HIDDEN_ATTACHMENT_MIN_DISTANCE,
            texture_resources,
            materials,
            draws,
        })
    }

    #[must_use]
    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    #[must_use]
    pub fn index_count(&self) -> usize {
        self.indices.len()
    }

    #[must_use]
    pub fn animation_count(&self) -> usize {
        self.animations.len()
    }

    #[must_use]
    pub fn animation_name(&self, index: usize) -> Option<String> {
        self.animations
            .get(index)
            .map(|animation| String::from_utf8_lossy(animation.name_bytes()).into_owned())
    }

    #[must_use]
    pub fn animation_frame_count(&self, index: usize) -> Option<u32> {
        self.animations.get(index).map(W3dAnimation::frame_count)
    }

    #[must_use]
    pub fn animation_frame_rate(&self, index: usize) -> Option<u32> {
        self.animations.get(index).map(W3dAnimation::frame_rate)
    }

    #[must_use]
    pub fn unique_texture_count(&self) -> usize {
        self.texture_resources.unique_image_count()
    }

    #[must_use]
    pub fn texture_alias_count(&self) -> usize {
        self.texture_resources.alias_count()
    }

    #[must_use]
    pub fn material_count(&self) -> usize {
        self.materials.len()
    }

    #[must_use]
    pub fn draw_count(&self) -> usize {
        self.draws.len()
    }

    pub(crate) const fn texture_resources(&self) -> &TextureResourceManager {
        &self.texture_resources
    }

    pub(crate) fn materials(&self) -> &[StagedMaterial] {
        &self.materials
    }

    pub(crate) fn draws(&self) -> &[DrawRange] {
        &self.draws
    }

    pub(crate) fn index_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.indices.len() * 4);
        for index in &self.indices {
            bytes.extend_from_slice(&index.to_le_bytes());
        }
        bytes
    }

    /// Stages bind-pose world-space vertices for instanced MAP scenery.
    pub(crate) fn bind_pose_vertex_bytes(&self) -> Result<Vec<u8>, RenderError> {
        let worlds = self.pose_transforms(None, 0)?;
        let mut bytes = Vec::with_capacity(self.vertices.len().saturating_mul(48));
        for vertex in &self.vertices {
            let world = worlds
                .get(usize::try_from(vertex.pivot).map_err(|_| RenderError::InvalidHierarchy)?)
                .copied()
                .ok_or(RenderError::InvalidHierarchy)?;
            let position = world.point(vertex.position);
            let normal = world.vector(vertex.normal);
            let texcoord = mapped_texcoord(vertex.texcoord, vertex.mapper, 0.0);
            if position
                .into_iter()
                .chain(normal)
                .chain(texcoord)
                .any(|value| !value.is_finite() || value.abs() > MAX_ABS_RENDER_COORDINATE)
            {
                return Err(RenderError::GeometryOutsideLimits);
            }
            for value in position
                .into_iter()
                .chain(normal)
                .chain(vertex.color)
                .chain([texcoord[0], 1.0 - texcoord[1]])
            {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        Ok(bytes)
    }

    /// Computes one fixed center and scale when a clip is selected.
    pub(crate) fn framing(&self, animation: Option<usize>) -> Result<ModelFraming, RenderError> {
        let worlds = self.pose_transforms(animation, 0)?;
        let mut minimum = [f32::INFINITY; 3];
        let mut maximum = [f32::NEG_INFINITY; 3];
        let mut positions = Vec::with_capacity(self.vertices.len());
        for vertex in &self.vertices {
            let world = worlds
                .get(usize::try_from(vertex.pivot).map_err(|_| RenderError::InvalidHierarchy)?)
                .copied()
                .ok_or(RenderError::InvalidHierarchy)?;
            let position = world.point(vertex.position);
            for axis in 0..3 {
                minimum[axis] = minimum[axis].min(position[axis]);
                maximum[axis] = maximum[axis].max(position[axis]);
            }
            positions.push(position);
        }
        let center = scale(add(minimum, maximum), 0.5);
        let radius = positions
            .into_iter()
            .map(|position| dot(subtract(position, center), subtract(position, center)).sqrt())
            .fold(0.0_f32, f32::max)
            .max(f32::EPSILON);
        Ok(ModelFraming {
            center,
            scale: 0.9 / radius,
        })
    }

    /// Stages one explicit integer animation frame using a fixed clip framing and model rotation.
    pub(crate) fn frame_vertex_bytes(
        &self,
        animation: Option<usize>,
        frame: u32,
        mapper_time_seconds: f32,
        rotation: f32,
        aspect: f32,
        framing: ModelFraming,
    ) -> Result<Vec<u8>, RenderError> {
        if !mapper_time_seconds.is_finite()
            || mapper_time_seconds < 0.0
            || !rotation.is_finite()
            || !aspect.is_finite()
            || aspect <= 0.0
            || !framing.scale.is_finite()
            || framing.scale <= 0.0
        {
            return Err(RenderError::NonFinitePose);
        }
        let worlds = self.pose_transforms(animation, frame)?;
        let (rotation_sine, rotation_cosine) = rotation.sin_cos();
        let rotate_up = |value: [f32; 3]| {
            [
                rotation_cosine * value[0] - rotation_sine * value[1],
                rotation_sine * value[0] + rotation_cosine * value[1],
                value[2],
            ]
        };
        let mut bytes = Vec::with_capacity(self.vertices.len() * 36);
        for vertex in &self.vertices {
            let world = worlds
                .get(usize::try_from(vertex.pivot).map_err(|_| RenderError::InvalidHierarchy)?)
                .copied()
                .ok_or(RenderError::InvalidHierarchy)?;
            let position = rotate_up(subtract(world.point(vertex.position), framing.center));
            let normal = rotate_up(world.vector(vertex.normal));
            if position
                .into_iter()
                .chain(normal)
                .any(|value| !value.is_finite() || value.abs() > MAX_ABS_RENDER_COORDINATE)
            {
                return Err(RenderError::GeometryOutsideLimits);
            }
            let (position, color) =
                project_fixed_vertex(position, normal, vertex.color, framing.scale, aspect);
            let texcoord = mapped_texcoord(vertex.texcoord, vertex.mapper, mapper_time_seconds);
            if texcoord
                .into_iter()
                .any(|value| !value.is_finite() || value.abs() > MAX_ABS_RENDER_COORDINATE)
            {
                return Err(RenderError::GeometryOutsideLimits);
            }
            let texcoord = [texcoord[0], 1.0 - texcoord[1]];
            for value in position.into_iter().chain(color).chain(texcoord) {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        Ok(bytes)
    }

    fn pose_transforms(
        &self,
        animation_index: Option<usize>,
        frame: u32,
    ) -> Result<Vec<Transform>, RenderError> {
        let animation = animation_index
            .map(|index| {
                self.animations
                    .get(index)
                    .ok_or(RenderError::InvalidAnimation)
            })
            .transpose()?;
        let frame = animation.map_or(0, |animation| {
            if animation.frame_count() == 0 {
                0
            } else {
                frame % animation.frame_count()
            }
        });
        let mut worlds = Vec::with_capacity(self.pivots.len());
        for (index, pivot) in self.pivots.iter().enumerate() {
            if index == 0 {
                worlds.push(Transform::IDENTITY);
                continue;
            }
            let channels = animation.map_or(&[][..], W3dAnimation::channels);
            let delta = [
                sample_scalar(channels, index, W3dAnimationChannelKind::X, frame),
                sample_scalar(channels, index, W3dAnimationChannelKind::Y, frame),
                sample_scalar(channels, index, W3dAnimationChannelKind::Z, frame),
            ];
            let hidden = dot(delta, delta)
                > self.hidden_attachment_distance * self.hidden_attachment_distance;
            let translation = if hidden {
                pivot.translation
            } else {
                add(pivot.translation, rotate(pivot.rotation, delta))
            };
            let animated_rotation = sample_rotation(channels, index, frame);
            let local = Transform {
                translation,
                rotation: normalize_quaternion(multiply(pivot.rotation, animated_rotation)),
                scale: if hidden { HIDDEN_ATTACHMENT_SCALE } else { 1.0 },
            };
            let world = if let Some(parent) = pivot.parent {
                worlds
                    .get(usize::try_from(parent).map_err(|_| RenderError::InvalidHierarchy)?)
                    .copied()
                    .ok_or(RenderError::InvalidHierarchy)?
                    .compose(local)
            } else {
                local
            };
            worlds.push(world);
        }
        Ok(worlds)
    }
}

impl StagedModel {
    /// Copies the selected HLOD meshes into stable model/triangle order and applies the hierarchy's
    /// bind pose. One-bone skin vertices use their decoded bone transforms; rigid meshes use their
    /// HLOD pivot.
    ///
    /// # Errors
    ///
    /// Returns a structured error if the model is empty or the combined vertex/index arrays exceed
    /// renderer address limits.
    #[allow(clippy::too_many_lines)]
    pub fn from_w3d(model: &W3dModel) -> Result<Self, RenderError> {
        let worlds = bind_pose_transforms(model)?;
        let total_vertices = model.meshes().iter().try_fold(0_usize, |total, mesh| {
            total
                .checked_add(mesh.mesh().vertices().len())
                .ok_or(RenderError::GeometryTooLarge)
        })?;
        if total_vertices == 0 || total_vertices > u32::MAX as usize {
            return Err(if total_vertices == 0 {
                RenderError::EmptyModel
            } else {
                RenderError::GeometryTooLarge
            });
        }
        let total_indices = model.meshes().iter().try_fold(0_usize, |total, mesh| {
            let mesh_indices = mesh
                .mesh()
                .triangles()
                .len()
                .checked_mul(3)
                .ok_or(RenderError::GeometryTooLarge)?;
            total
                .checked_add(mesh_indices)
                .ok_or(RenderError::GeometryTooLarge)
        })?;
        if total_indices == 0 {
            return Err(RenderError::EmptyModel);
        }
        if total_vertices
            .checked_mul(28)
            .is_none_or(|bytes| bytes > MAX_GEOMETRY_BUFFER_BYTES)
            || total_indices
                .checked_mul(4)
                .is_none_or(|bytes| bytes > MAX_GEOMETRY_BUFFER_BYTES)
        {
            return Err(RenderError::GeometryTooLarge);
        }
        let mut vertices = Vec::with_capacity(total_vertices);
        let mut indices = Vec::with_capacity(total_indices);
        let mut minimum = [f32::INFINITY; 3];
        let mut maximum = [f32::NEG_INFINITY; 3];
        for model_mesh in model.meshes() {
            let mesh = model_mesh.mesh();
            let base = u32::try_from(vertices.len()).map_err(|_| RenderError::GeometryTooLarge)?;
            let colors = mesh.preview_vertex_colors();
            let rigid_transform = usize::try_from(model_mesh.pivot())
                .ok()
                .and_then(|pivot| worlds.get(pivot))
                .copied()
                .ok_or(RenderError::InvalidHierarchy)?;
            for (vertex_index, (position, normal)) in
                mesh.vertices().iter().zip(mesh.normals()).enumerate()
            {
                let transform = if let Some(bones) = mesh.vertex_bones() {
                    worlds
                        .get(usize::from(bones[vertex_index]))
                        .copied()
                        .unwrap_or(rigid_transform)
                } else {
                    rigid_transform
                };
                let position = transform.point([position.x(), position.y(), position.z()]);
                let normal = transform.vector([normal.x(), normal.y(), normal.z()]);
                if position
                    .into_iter()
                    .chain(normal)
                    .any(|value| !value.is_finite() || value.abs() > MAX_ABS_RENDER_COORDINATE)
                {
                    return Err(RenderError::GeometryOutsideLimits);
                }
                for axis in 0..3 {
                    minimum[axis] = minimum[axis].min(position[axis]);
                    maximum[axis] = maximum[axis].max(position[axis]);
                }
                let color = colors.as_ref().map_or([0.72, 0.78, 0.86, 1.0], |colors| {
                    let color = colors[vertex_index];
                    [
                        channel(color.red()),
                        channel(color.green()),
                        channel(color.blue()),
                        channel(color.alpha()),
                    ]
                });
                vertices.push(ModelVertex {
                    position,
                    normal,
                    color,
                });
            }
            for triangle in mesh.triangles() {
                for index in triangle.vertex_indices() {
                    indices.push(
                        base.checked_add(index)
                            .ok_or(RenderError::GeometryTooLarge)?,
                    );
                }
            }
        }
        Ok(Self {
            vertices,
            indices,
            minimum,
            maximum,
        })
    }

    #[must_use]
    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    #[must_use]
    pub fn index_count(&self) -> usize {
        self.indices.len()
    }

    #[must_use]
    pub const fn minimum(&self) -> [f32; 3] {
        self.minimum
    }

    #[must_use]
    pub const fn maximum(&self) -> [f32; 3] {
        self.maximum
    }

    pub(crate) fn gpu_bytes(&self) -> (Vec<u8>, Vec<u8>) {
        let projected = self.projected_vertices();
        let mut vertex_bytes = Vec::with_capacity(projected.len() * 28);
        for (position, color) in projected {
            for value in position.into_iter().chain(color) {
                vertex_bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        let mut index_bytes = Vec::with_capacity(self.indices.len() * 4);
        for index in &self.indices {
            index_bytes.extend_from_slice(&index.to_le_bytes());
        }
        (vertex_bytes, index_bytes)
    }

    fn projected_vertices(&self) -> Vec<([f32; 3], [f32; 4])> {
        const RIGHT: [f32; 3] = [0.707_106_77, -0.707_106_77, 0.0];
        const UP: [f32; 3] = [0.408_248_3, 0.408_248_3, 0.816_496_6];
        const FORWARD: [f32; 3] = [0.577_350_26, 0.577_350_26, -0.577_350_26];
        const LIGHT: [f32; 3] = [-0.371_390_67, -0.557_086, 0.742_781_34];
        let center = scale(add(self.minimum, self.maximum), 0.5);
        let camera = self
            .vertices
            .iter()
            .map(|vertex| {
                let relative = subtract(vertex.position, center);
                [
                    dot(relative, RIGHT),
                    dot(relative, UP),
                    dot(relative, FORWARD),
                ]
            })
            .collect::<Vec<_>>();
        let mut minimum = [f32::INFINITY; 3];
        let mut maximum = [f32::NEG_INFINITY; 3];
        for position in &camera {
            for axis in 0..3 {
                minimum[axis] = minimum[axis].min(position[axis]);
                maximum[axis] = maximum[axis].max(position[axis]);
            }
        }
        let horizontal = (maximum[0] - minimum[0]).max(f32::EPSILON);
        let vertical = (maximum[1] - minimum[1]).max(f32::EPSILON);
        let fit = 1.8 / horizontal.max(vertical);
        let depth = (maximum[2] - minimum[2]).max(f32::EPSILON);
        camera
            .into_iter()
            .zip(&self.vertices)
            .map(|(position, vertex)| {
                let normal = normalize_vector(vertex.normal);
                let illumination = 0.3 + 0.7 * dot(normal, LIGHT).abs();
                let color = [
                    vertex.color[0] * illumination,
                    vertex.color[1] * illumination,
                    vertex.color[2] * illumination,
                    1.0,
                ];
                (
                    [
                        position[0] * fit,
                        position[1] * fit,
                        0.1 + 0.8 * ((position[2] - minimum[2]) / depth),
                    ],
                    color,
                )
            })
            .collect()
    }
}

fn bind_pose_transforms(model: &W3dModel) -> Result<Vec<Transform>, RenderError> {
    let pivots = model.hierarchy().pivots();
    let mut worlds = Vec::with_capacity(pivots.len());
    for (index, pivot) in pivots.iter().enumerate() {
        if index == 0 {
            worlds.push(Transform::IDENTITY);
            continue;
        }
        let local = Transform {
            translation: pivot.translation(),
            rotation: normalize_quaternion(pivot.rotation().components()),
            scale: 1.0,
        };
        let world = if let Some(parent) = pivot.parent() {
            worlds
                .get(usize::try_from(parent).map_err(|_| RenderError::InvalidHierarchy)?)
                .copied()
                .ok_or(RenderError::InvalidHierarchy)?
                .compose(local)
        } else {
            local
        };
        worlds.push(world);
    }
    Ok(worlds)
}

fn mesh_name_eq(mesh: &W3dStaticMesh, expected: &[u8]) -> bool {
    let header = mesh.header();
    let name = header.mesh_name_bytes();
    let end = name
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(name.len());
    name[..end].eq_ignore_ascii_case(expected)
}

fn mesh_bind_x_bounds(
    model_mesh: &cic_formats::W3dModelMesh,
    worlds: &[Transform],
) -> Result<[f32; 2], RenderError> {
    let mesh = model_mesh.mesh();
    let rigid = worlds
        .get(usize::try_from(model_mesh.pivot()).map_err(|_| RenderError::InvalidHierarchy)?)
        .copied()
        .ok_or(RenderError::InvalidHierarchy)?;
    let mut minimum = f32::INFINITY;
    let mut maximum = f32::NEG_INFINITY;
    for (index, position) in mesh.vertices().iter().enumerate() {
        let transform = mesh
            .vertex_bones()
            .and_then(|bones| worlds.get(usize::from(bones[index])).copied())
            .unwrap_or(rigid);
        let x = transform.point([position.x(), position.y(), position.z()])[0];
        if !x.is_finite() {
            return Err(RenderError::GeometryOutsideLimits);
        }
        minimum = minimum.min(x);
        maximum = maximum.max(x);
    }
    if minimum > maximum {
        return Err(RenderError::EmptyModel);
    }
    Ok([minimum, maximum])
}

fn bridge_span_layout(
    spannable: f32,
    span_length: f32,
    natural_length: f32,
) -> Result<(usize, f32, f32), RenderError> {
    let rounded_spannable = (spannable + span_length * 0.5).max(0.0);
    let mut span_count = 0_usize;
    let mut repeated_length = 0.0_f32;
    while repeated_length + span_length <= rounded_spannable && span_count < 65_536 {
        repeated_length += span_length;
        span_count += 1;
    }
    if repeated_length + span_length <= rounded_spannable {
        return Err(RenderError::GeometryTooLarge);
    }
    let generated_length = natural_length + repeated_length - span_length;
    Ok((span_count, repeated_length, generated_length))
}

fn hidden_attachment_distance(model: &StagedModel) -> f32 {
    let extent = subtract(model.maximum(), model.minimum());
    let diagonal = dot(extent, extent).sqrt();
    HIDDEN_ATTACHMENT_MIN_DISTANCE.max(diagonal * HIDDEN_ATTACHMENT_MODEL_MULTIPLIER)
}

fn preview_color(mesh: &W3dStaticMesh, pass: Option<&W3dMaterialPass>, vertex: usize) -> [f32; 4] {
    if let Some(color) = pass
        .and_then(W3dMaterialPass::diffuse_colors)
        .and_then(|colors| colors.get(vertex))
    {
        return [
            channel(color.red()),
            channel(color.green()),
            channel(color.blue()),
            channel(color.alpha()),
        ];
    }
    if let Some(material) = pass
        .and_then(W3dMaterialPass::vertex_material_ids)
        .and_then(|ids| ids.for_vertex(vertex))
        .and_then(|id| usize::try_from(id).ok())
        .and_then(|id| mesh.materials().vertex_materials().get(id))
    {
        let color = material.diffuse();
        return [
            channel(color.red()),
            channel(color.green()),
            channel(color.blue()),
            material.opacity().clamp(0.0, 1.0),
        ];
    }
    [0.72, 0.78, 0.86, 1.0]
}

fn finite_texcoord(u: f32, v: f32) -> [f32; 2] {
    if u.is_finite() && v.is_finite() {
        [u, v]
    } else {
        [0.0, 0.0]
    }
}

fn staged_mapper(
    pass: Option<&W3dMaterialPass>,
    stage: usize,
    vertex: usize,
    table: &[[StagedMapper; 2]],
) -> StagedMapper {
    pass.and_then(W3dMaterialPass::vertex_material_ids)
        .and_then(|ids| ids.for_vertex(vertex))
        .and_then(|id| usize::try_from(id).ok())
        .and_then(|id| table.get(id))
        .and_then(|mappers| mappers.get(stage))
        .copied()
        .unwrap_or_default()
}

fn stage_mapper(mapper: &cic_formats::W3dMapper) -> StagedMapper {
    // Provenance: formulas and argument defaults follow `MAPPERS.TXT` and `mapper.cpp` from
    // GeneralsGameCode revision `9f7abb866f5afd446db14149979e744c7216baaf`; see
    // `docs/provenance/w3d.md`. Evaluation is project-authored from an explicit caller time rather
    // than legacy global sync time so diagnostic rendering remains deterministic.
    let mode = mapper.mode().code();
    let values = match mode {
        4 | 18 => [
            mapper_value(mapper, b"UScale", 1.0),
            mapper_value(mapper, b"VScale", 1.0),
            mapper_value(mapper, b"UPerSec", 0.0),
            mapper_value(mapper, b"VPerSec", 0.0),
            mapper_value(mapper, b"UOffset", 0.0),
            mapper_value(mapper, b"VOffset", 0.0),
            0.0,
            0.0,
        ],
        6 => [
            mapper_value(mapper, b"UScale", 1.0),
            mapper_value(mapper, b"VScale", 1.0),
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
        ],
        7 | 14 | 15 | 19 | 20 => [
            mapper_value(mapper, b"FPS", 1.0),
            mapper_value(mapper, b"Log2Width", 1.0).clamp(0.0, 15.0),
            mapper_value(mapper, b"Last", 0.0).max(0.0),
            mapper_value(mapper, b"Offset", 0.0).max(0.0),
            0.0,
            0.0,
            0.0,
            0.0,
        ],
        8 => [
            mapper_value(mapper, b"Speed", 0.1),
            mapper_value(mapper, b"UCenter", 0.0),
            mapper_value(mapper, b"VCenter", 0.0),
            mapper_value(mapper, b"UScale", 1.0),
            mapper_value(mapper, b"VScale", 1.0),
            0.0,
            0.0,
            0.0,
        ],
        9 => [
            mapper_value(mapper, b"UAmp", 1.0),
            mapper_value(mapper, b"UFreq", 1.0),
            mapper_value(mapper, b"UPhase", 0.0),
            mapper_value(mapper, b"VAmp", 1.0),
            mapper_value(mapper, b"VFreq", 1.0),
            mapper_value(mapper, b"VPhase", 0.0),
            mapper_value(mapper, b"UScale", 1.0),
            mapper_value(mapper, b"VScale", 1.0),
        ],
        10 => [
            mapper_value(mapper, b"UStep", 0.0),
            mapper_value(mapper, b"VStep", 0.0),
            mapper_value(mapper, b"SPS", 0.0),
            mapper_value(mapper, b"UScale", 1.0),
            mapper_value(mapper, b"VScale", 1.0),
            0.0,
            0.0,
            0.0,
        ],
        11 => [
            mapper_value(mapper, b"UPerSec", 0.0),
            mapper_value(mapper, b"VPerSec", 0.0),
            mapper_value(mapper, b"Period", 0.0).abs(),
            mapper_value(mapper, b"UScale", 1.0),
            mapper_value(mapper, b"VScale", 1.0),
            0.0,
            0.0,
            0.0,
        ],
        16 => [
            mapper_value(mapper, b"FPS", 0.0),
            mapper_value(mapper, b"UPerSec", 0.0),
            mapper_value(mapper, b"VPerSec", 0.0),
            mapper_value(mapper, b"UScale", 1.0),
            mapper_value(mapper, b"VScale", 1.0),
            0.0,
            0.0,
            0.0,
        ],
        17 => [
            mapper_value(mapper, b"VPerSec", 0.0),
            mapper_value(mapper, b"VStart", 0.0),
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
        ],
        _ => [0.0; 8],
    };
    StagedMapper { mode, values }
}

fn mapper_value(mapper: &cic_formats::W3dMapper, key: &[u8], default: f32) -> f32 {
    let Some(arguments) = mapper.argument_bytes() else {
        return default;
    };
    for assignment in arguments.split(|byte| matches!(byte, b';' | b'\n' | b'\r')) {
        let Some(equals) = assignment.iter().position(|byte| *byte == b'=') else {
            continue;
        };
        if !trim_ascii(&assignment[..equals]).eq_ignore_ascii_case(key) {
            continue;
        }
        let raw = trim_ascii(&assignment[equals + 1..]);
        let raw = raw
            .strip_suffix(b"f")
            .or_else(|| raw.strip_suffix(b"F"))
            .unwrap_or(raw);
        if let Ok(text) = std::str::from_utf8(raw)
            && let Ok(value) = text.parse::<f32>()
            && value.is_finite()
        {
            return value;
        }
    }
    default
}

fn trim_ascii(mut value: &[u8]) -> &[u8] {
    while value.first().is_some_and(u8::is_ascii_whitespace) {
        value = &value[1..];
    }
    while value.last().is_some_and(u8::is_ascii_whitespace) {
        value = &value[..value.len() - 1];
    }
    value
}

fn staged_material(
    mesh: &W3dStaticMesh,
    pass_index: usize,
    stage_index: usize,
    triangle: usize,
    resources: &TextureResourceManager,
) -> StagedMaterial {
    // Provenance: shader selectors, texture assignments, UV indices, and texture clamp bits come
    // from `w3d_file.h` at GeneralsGameCode revision
    // `9f7abb866f5afd446db14149979e744c7216baaf`; see `docs/provenance/w3d.md`. This is a
    // project-authored fixed-function preview policy, not a translation of the legacy renderer.
    let pass = mesh.materials().passes().get(pass_index);
    let stage = pass.and_then(|pass| pass.texture_stages().get(stage_index));
    let texture_entry = stage
        .and_then(|stage| stage.texture_ids())
        .and_then(|ids| ids.for_triangle(triangle))
        .filter(|id| *id != u32::MAX)
        .and_then(|id| usize::try_from(id).ok())
        .and_then(|id| mesh.materials().textures().get(id));
    let attributes = texture_entry
        .and_then(cic_formats::W3dTexture::info)
        .map_or(0, cic_formats::W3dTextureInfo::attributes);
    let shader = pass
        .and_then(|pass| pass.shader_ids())
        .and_then(|ids| ids.for_triangle(triangle))
        .and_then(|id| usize::try_from(id).ok())
        .and_then(|id| mesh.materials().shaders().get(id))
        .copied();
    let texture = if shader.is_some_and(|shader| shader.texturing() == 0) {
        resources.fallback_white()
    } else {
        texture_entry
            .and_then(|texture| resources.texture(texture.name_bytes()))
            .unwrap_or_else(|| resources.fallback_white())
    };
    let blend = if stage_index > 0 {
        BlendMode::Multiply
    } else {
        shader.map_or(BlendMode::Opaque, |shader| {
            preview_blend(shader.source_blend(), shader.destination_blend())
        })
    };
    StagedMaterial {
        texture,
        clamp_u: attributes & 0x8 != 0,
        clamp_v: attributes & 0x10 != 0,
        blend,
        alpha_test: shader.is_some_and(|shader| shader.alpha_test() != 0),
        depth_write: pass_index == 0 && stage_index == 0 && blend == BlendMode::Opaque,
        two_sided: mesh_is_two_sided(mesh.header().attributes()),
    }
}

#[allow(clippy::too_many_lines)]
fn mapped_texcoord(texcoord: [f32; 2], mapper: StagedMapper, time: f32) -> [f32; 2] {
    let value = mapper.values;
    match mapper.mode {
        4 | 18 => [
            texcoord[0] * value[0] + (value[4] - value[2] * time).rem_euclid(1.0),
            texcoord[1] * value[1] + (value[5] - value[3] * time).rem_euclid(1.0),
        ],
        6 => [texcoord[0] * value[0], texcoord[1] * value[1]],
        7 | 14 | 15 | 19 | 20 => {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let log2_width = value[1] as u32;
            let width = 1_u32 << log2_width;
            let default_last = width.saturating_mul(width).max(1);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let last = if value[2] < 1.0 {
                default_last
            } else {
                (value[2] as u32).clamp(1, default_last)
            };
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let offset = (value[3] as u32) % last;
            #[allow(clippy::cast_possible_truncation)]
            let elapsed = (time * value[0].abs()).floor() as i64;
            let direction = if value[0] < 0.0 { -1_i64 } else { 1_i64 };
            let frame = (i64::from(offset) + direction * elapsed).rem_euclid(i64::from(last));
            let frame = u32::try_from(frame).expect("grid frame is nonnegative");
            #[allow(clippy::cast_precision_loss)]
            let reciprocal = 1.0 / width as f32;
            #[allow(clippy::cast_precision_loss)]
            let column = (frame % width) as f32;
            #[allow(clippy::cast_precision_loss)]
            let row = (frame / width) as f32;
            [
                texcoord[0] + column * reciprocal,
                texcoord[1] + row * reciprocal,
            ]
        }
        8 => {
            let angle = std::f32::consts::TAU * value[0] * time;
            let (sine, cosine) = angle.sin_cos();
            let u = texcoord[0] - value[1];
            let v = texcoord[1] - value[2];
            [
                value[3] * (cosine * u - sine * v + value[1]),
                value[4] * (sine * u + cosine * v + value[2]),
            ]
        }
        9 => [
            texcoord[0] * value[6]
                + value[0]
                    * (std::f32::consts::TAU * value[1] * time + std::f32::consts::PI * value[2])
                        .sin(),
            texcoord[1] * value[7]
                + value[3]
                    * (std::f32::consts::TAU * value[4] * time + std::f32::consts::PI * value[5])
                        .sin(),
        ],
        10 => {
            let steps = (value[2] * time).trunc();
            [
                texcoord[0] * value[3] + (value[0] * steps).rem_euclid(1.0),
                texcoord[1] * value[4] + (value[1] * steps).rem_euclid(1.0),
            ]
        }
        11 => {
            let offset_time = if value[2] > 0.0 {
                let within = time.rem_euclid(value[2]);
                within.min(value[2] - within)
            } else {
                0.0
            };
            [
                texcoord[0] * value[3] + value[0] * offset_time,
                texcoord[1] * value[4] + value[1] * offset_time,
            ]
        }
        16 => {
            let rate = value[0].abs();
            let frame = (time * rate).floor();
            let seed = frame.to_bits();
            let angle = stable_random_unit(seed) * std::f32::consts::TAU;
            let center_u = stable_random_unit(seed ^ 0x9E37_79B9);
            let center_v = stable_random_unit(seed ^ 0x85EB_CA6B);
            let remainder = if rate > 0.0 {
                time.rem_euclid(rate.recip())
            } else {
                time
            };
            let (sine, cosine) = angle.sin_cos();
            [
                cosine * value[3] * texcoord[0] - sine * value[4] * texcoord[1]
                    + (center_u + remainder * value[1]).rem_euclid(1.0),
                sine * value[3] * texcoord[0]
                    + cosine * value[4] * texcoord[1]
                    + (center_v + remainder * value[2]).rem_euclid(1.0),
            ]
        }
        17 => [
            texcoord[0],
            texcoord[1] + (value[1] + value[0] * time).rem_euclid(1.0),
        ],
        _ => texcoord,
    }
}

fn stable_random_unit(mut state: u32) -> f32 {
    state = state.wrapping_add(0xA511_E9B3);
    state ^= state << 13;
    state ^= state >> 17;
    state ^= state << 5;
    let mantissa = state >> 8;
    #[allow(clippy::cast_precision_loss)]
    let mantissa = mantissa as f32;
    mantissa / 16_777_216.0
}

const fn preview_blend(source: u8, destination: u8) -> BlendMode {
    if source == 1 && destination == 1 {
        BlendMode::Additive
    } else if destination != 0 {
        BlendMode::Alpha
    } else {
        BlendMode::Opaque
    }
}

fn sample_scalar(
    channels: &[W3dAnimationChannel],
    pivot: usize,
    kind: W3dAnimationChannelKind,
    frame: u32,
) -> f32 {
    let Some(channel) = channels
        .iter()
        .find(|channel| usize::from(channel.pivot()) == pivot && channel.kind() == kind)
    else {
        return 0.0;
    };
    if frame < channel.first_frame() || frame > channel.last_frame() {
        return 0.0;
    }
    let index = usize::try_from(frame - channel.first_frame()).expect("bounded frame index");
    channel.values()[index]
}

fn sample_rotation(channels: &[W3dAnimationChannel], pivot: usize, frame: u32) -> [f32; 4] {
    if let Some(channel) = channels.iter().find(|channel| {
        usize::from(channel.pivot()) == pivot
            && channel.kind() == W3dAnimationChannelKind::Quaternion
    }) && frame >= channel.first_frame()
        && frame <= channel.last_frame()
    {
        let offset =
            usize::try_from(frame - channel.first_frame()).expect("bounded frame index") * 4;
        return normalize_quaternion(
            channel.values()[offset..offset + 4]
                .try_into()
                .expect("validated quaternion channel"),
        );
    }
    let x = sample_scalar(channels, pivot, W3dAnimationChannelKind::XRotation, frame);
    let y = sample_scalar(channels, pivot, W3dAnimationChannelKind::YRotation, frame);
    let z = sample_scalar(channels, pivot, W3dAnimationChannelKind::ZRotation, frame);
    multiply(
        multiply(
            axis_angle([1.0, 0.0, 0.0], x),
            axis_angle([0.0, 1.0, 0.0], y),
        ),
        axis_angle([0.0, 0.0, 1.0], z),
    )
}

fn axis_angle(axis: [f32; 3], angle: f32) -> [f32; 4] {
    let (sine, cosine) = (angle * 0.5).sin_cos();
    [axis[0] * sine, axis[1] * sine, axis[2] * sine, cosine]
}

fn project_fixed_vertex(
    position: [f32; 3],
    normal: [f32; 3],
    color: [f32; 4],
    scale: f32,
    aspect: f32,
) -> ([f32; 3], [f32; 4]) {
    const SQRT_HALF: f32 = 0.707_106_77;
    const RIGHT: [f32; 3] = [-SQRT_HALF, SQRT_HALF, 0.0];
    const UP: [f32; 3] = [-0.5, -0.5, SQRT_HALF];
    const TO_CAMERA: [f32; 3] = [0.5, 0.5, SQRT_HALF];
    const LIGHT: [f32; 3] = [-0.371_390_67, -0.557_086, 0.742_781_34];
    let normal = normalize_vector(normal);
    let illumination = 0.3 + 0.7 * dot(normal, LIGHT).abs();
    (
        [
            dot(position, RIGHT) * scale / aspect,
            dot(position, UP) * scale,
            0.5 - dot(position, TO_CAMERA) * scale * 0.5,
        ],
        [
            color[0] * illumination,
            color[1] * illumination,
            color[2] * illumination,
            color[3],
        ],
    )
}

fn channel(value: u8) -> f32 {
    f32::from(value) / 255.0
}

fn add(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] + right[0], left[1] + right[1], left[2] + right[2]]
}

fn subtract(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn scale(value: [f32; 3], factor: f32) -> [f32; 3] {
    [value[0] * factor, value[1] * factor, value[2] * factor]
}

fn dot(left: [f32; 3], right: [f32; 3]) -> f32 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn cross(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn normalize_vector(value: [f32; 3]) -> [f32; 3] {
    let length = dot(value, value).sqrt();
    if length > 0.0 {
        scale(value, length.recip())
    } else {
        [0.0, 0.0, 1.0]
    }
}

fn normalize_quaternion(mut value: [f32; 4]) -> [f32; 4] {
    let length = value
        .iter()
        .map(|component| component * component)
        .sum::<f32>()
        .sqrt();
    if length > 0.0 {
        for component in &mut value {
            *component /= length;
        }
        value
    } else {
        [0.0, 0.0, 0.0, 1.0]
    }
}

fn multiply(left: [f32; 4], right: [f32; 4]) -> [f32; 4] {
    [
        left[3] * right[0] + left[0] * right[3] + left[1] * right[2] - left[2] * right[1],
        left[3] * right[1] - left[0] * right[2] + left[1] * right[3] + left[2] * right[0],
        left[3] * right[2] + left[0] * right[1] - left[1] * right[0] + left[2] * right[3],
        left[3] * right[3] - left[0] * right[0] - left[1] * right[1] - left[2] * right[2],
    ]
}

fn rotate(quaternion: [f32; 4], vector: [f32; 3]) -> [f32; 3] {
    let quaternion = normalize_quaternion(quaternion);
    let axis = [quaternion[0], quaternion[1], quaternion[2]];
    let scalar = quaternion[3];
    let projection = dot(axis, vector);
    let axis_norm = dot(axis, axis);
    let cross = [
        axis[1] * vector[2] - axis[2] * vector[1],
        axis[2] * vector[0] - axis[0] * vector[2],
        axis[0] * vector[1] - axis[1] * vector[0],
    ];
    [
        2.0 * projection * axis[0]
            + (scalar * scalar - axis_norm) * vector[0]
            + 2.0 * scalar * cross[0],
        2.0 * projection * axis[1]
            + (scalar * scalar - axis_norm) * vector[1]
            + 2.0 * scalar * cross[1],
        2.0 * projection * axis[2]
            + (scalar * scalar - axis_norm) * vector[2]
            + 2.0 * scalar * cross[2],
    ]
}

#[cfg(test)]
mod tests {
    use super::{
        BlendMode, StagedMapper, Transform, bridge_span_layout, mapped_texcoord, mesh_is_two_sided,
        preview_blend, project_fixed_vertex,
    };

    #[test]
    fn mesh_two_sided_flag_controls_culling_policy() {
        assert!(!mesh_is_two_sided(0));
        assert!(mesh_is_two_sided(0x0000_2000));
        assert!(mesh_is_two_sided(0x0000_2002));
    }

    #[test]
    fn preview_blend_distinguishes_opaque_alpha_and_additive() {
        assert_eq!(preview_blend(0, 0), BlendMode::Opaque);
        assert_eq!(preview_blend(2, 3), BlendMode::Alpha);
        assert_eq!(preview_blend(1, 1), BlendMode::Additive);
    }

    #[test]
    fn bridge_span_rounding_matches_sectional_source_layout() {
        let (count, repeated, generated) =
            bridge_span_layout(5.0, 2.0, 10.0).expect("bounded span layout");
        assert_eq!(count, 3);
        assert!((repeated - 6.0).abs() < f32::EPSILON);
        assert!((generated - 14.0).abs() < f32::EPSILON);
    }

    #[test]
    fn fixed_projection_preserves_pose_translation() {
        let (origin, _) =
            project_fixed_vertex([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0; 4], 0.25, 4.0 / 3.0);
        let (translated, _) =
            project_fixed_vertex([4.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0; 4], 0.25, 4.0 / 3.0);
        assert!(origin[0].abs() < f32::EPSILON);
        assert!(origin[1].abs() < f32::EPSILON);
        assert!((translated[0] - origin[0]).abs() > 0.1);
        assert!((translated[1] - origin[1]).abs() > 0.1);
    }

    #[test]
    fn hidden_attachment_scale_propagates_to_children() {
        let parent = Transform {
            translation: [10.0, 0.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: 0.001,
        };
        let child = Transform {
            translation: [4.0, 0.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: 1.0,
        };
        let world = parent.compose(child);
        assert!((world.translation[0] - 10.004).abs() < 0.000_01);
        assert!((world.point([1.0, 0.0, 0.0])[0] - 10.005).abs() < 0.000_01);
    }

    #[test]
    fn linear_mapper_uses_explicit_time_and_legacy_negative_rate() {
        let mapper_config = StagedMapper {
            mode: 4,
            values: [1.0, 1.0, 0.5, -0.25, 0.0, 0.0, 0.0, 0.0],
        };
        let coordinates = mapped_texcoord([0.25, 0.25], mapper_config, 2.0);
        assert!((coordinates[0] - 0.25).abs() < f32::EPSILON);
        assert!((coordinates[1] - 0.75).abs() < f32::EPSILON);
    }

    #[test]
    fn rotate_mapper_is_sampled_from_explicit_seconds() {
        let mapper_config = StagedMapper {
            mode: 8,
            values: [0.25, 0.5, 0.5, 1.0, 1.0, 0.0, 0.0, 0.0],
        };
        let coordinates = mapped_texcoord([1.0, 0.5], mapper_config, 1.0);
        assert!((coordinates[0] - 0.5).abs() < 0.000_01);
        assert!((coordinates[1] - 1.0).abs() < 0.000_01);
    }

    #[test]
    fn grid_mapper_advances_in_stable_row_major_order() {
        let mapper_config = StagedMapper {
            mode: 7,
            values: [2.0, 1.0, 4.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        };
        let coordinates = mapped_texcoord([0.0, 0.0], mapper_config, 0.5);
        assert!((coordinates[0] - 0.5).abs() < f32::EPSILON);
        assert!(coordinates[1].abs() < f32::EPSILON);
    }

    #[test]
    fn random_mapper_is_stable_for_an_explicit_frame() {
        let mapper_config = StagedMapper {
            mode: 16,
            values: [2.0, 0.1, -0.2, 1.0, 1.0, 0.0, 0.0, 0.0],
        };
        let first = mapped_texcoord([0.25, 0.75], mapper_config, 0.5).map(f32::to_bits);
        let repeated = mapped_texcoord([0.25, 0.75], mapper_config, 0.5).map(f32::to_bits);
        let next = mapped_texcoord([0.25, 0.75], mapper_config, 1.0).map(f32::to_bits);
        assert_eq!(first, repeated);
        assert_ne!(first, next);
    }
}
