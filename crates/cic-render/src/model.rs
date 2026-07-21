use std::collections::BTreeMap;

use cic_formats::{
    W3dAnimation, W3dAnimationChannel, W3dAnimationChannelKind, W3dModel, W3dStaticMesh,
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
    pivot: u32,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct StagedMaterial {
    pub(crate) texture: TextureId,
    pub(crate) clamp_u: bool,
    pub(crate) clamp_v: bool,
    pub(crate) blend: BlendMode,
    pub(crate) alpha_test: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DrawRange {
    pub(crate) material: usize,
    pub(crate) first_index: u32,
    pub(crate) index_count: u32,
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
        let expanded_vertex_count = staged.index_count();
        if expanded_vertex_count
            .checked_mul(36)
            .is_none_or(|bytes| bytes > MAX_GEOMETRY_BUFFER_BYTES)
        {
            return Err(RenderError::GeometryTooLarge);
        }
        let mut vertices = Vec::with_capacity(expanded_vertex_count);
        let mut indices = Vec::with_capacity(staged.index_count());
        let mut materials = Vec::new();
        let mut material_indices = BTreeMap::new();
        let mut draws: Vec<DrawRange> = Vec::new();
        for model_mesh in model.meshes() {
            let mesh = model_mesh.mesh();
            let colors = mesh.preview_vertex_colors();
            let stage = mesh
                .materials()
                .passes()
                .first()
                .and_then(|pass| pass.texture_stages().first());
            for (triangle_index, triangle) in mesh.triangles().iter().enumerate() {
                let material = staged_material(mesh, triangle_index, &texture_resources);
                let material_index = *material_indices.entry(material).or_insert_with(|| {
                    let index = materials.len();
                    materials.push(material);
                    index
                });
                let first_index =
                    u32::try_from(indices.len()).map_err(|_| RenderError::GeometryTooLarge)?;
                if let Some(draw) = draws.last_mut()
                    && draw.material == material_index
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
                    });
                }
                let source_indices = triangle.vertex_indices();
                let uv_indices = stage
                    .and_then(|stage| stage.coordinate_indices(triangle_index, source_indices));
                for corner in 0..3 {
                    let vertex_index = usize::try_from(source_indices[corner])
                        .map_err(|_| RenderError::GeometryTooLarge)?;
                    let position = mesh.vertices()[vertex_index];
                    let normal = mesh.normals()[vertex_index];
                    let pivot = mesh
                        .vertex_bones()
                        .map_or(model_mesh.pivot(), |bones| u32::from(bones[vertex_index]));
                    if usize::try_from(pivot)
                        .ok()
                        .is_none_or(|pivot| pivot >= model.hierarchy().pivots().len())
                    {
                        return Err(RenderError::InvalidHierarchy);
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
                    let texcoord = stage
                        .zip(uv_indices)
                        .and_then(|(stage, indices)| {
                            usize::try_from(indices[corner])
                                .ok()
                                .and_then(|index| stage.texture_coordinates().get(index))
                        })
                        .map_or([0.0, 0.0], |uv| [uv.u(), 1.0 - uv.v()]);
                    let index =
                        u32::try_from(vertices.len()).map_err(|_| RenderError::GeometryTooLarge)?;
                    vertices.push(AnimatedVertex {
                        position: [position.x(), position.y(), position.z()],
                        normal: [normal.x(), normal.y(), normal.z()],
                        color,
                        texcoord,
                        pivot,
                    });
                    indices.push(index);
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
        rotation: f32,
        aspect: f32,
        framing: ModelFraming,
    ) -> Result<Vec<u8>, RenderError> {
        if !rotation.is_finite()
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
            for value in position.into_iter().chain(color).chain(vertex.texcoord) {
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
                        .ok_or(RenderError::InvalidHierarchy)?
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

fn hidden_attachment_distance(model: &StagedModel) -> f32 {
    let extent = subtract(model.maximum(), model.minimum());
    let diagonal = dot(extent, extent).sqrt();
    HIDDEN_ATTACHMENT_MIN_DISTANCE.max(diagonal * HIDDEN_ATTACHMENT_MODEL_MULTIPLIER)
}

fn staged_material(
    mesh: &W3dStaticMesh,
    triangle: usize,
    resources: &TextureResourceManager,
) -> StagedMaterial {
    // Provenance: shader selectors, texture assignments, UV indices, and texture clamp bits come
    // from `w3d_file.h` at GeneralsGameCode revision
    // `9f7abb866f5afd446db14149979e744c7216baaf`; see `docs/provenance/w3d.md`. This is a
    // project-authored pass-zero preview policy, not a translation of the legacy renderer.
    let pass = mesh.materials().passes().first();
    let stage = pass.and_then(|pass| pass.texture_stages().first());
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
    let blend = shader.map_or(BlendMode::Opaque, |shader| {
        preview_blend(shader.source_blend(), shader.destination_blend())
    });
    StagedMaterial {
        texture,
        clamp_u: attributes & 0x8 != 0,
        clamp_v: attributes & 0x10 != 0,
        blend,
        alpha_test: shader.is_some_and(|shader| shader.alpha_test() != 0),
    }
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
    use super::{BlendMode, Transform, preview_blend, project_fixed_vertex};

    #[test]
    fn preview_blend_distinguishes_opaque_alpha_and_additive() {
        assert_eq!(preview_blend(0, 0), BlendMode::Opaque);
        assert_eq!(preview_blend(2, 3), BlendMode::Alpha);
        assert_eq!(preview_blend(1, 1), BlendMode::Additive);
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
}
