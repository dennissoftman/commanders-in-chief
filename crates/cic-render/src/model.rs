use cic_formats::{W3dAnimation, W3dAnimationChannel, W3dAnimationChannelKind, W3dModel};

use crate::RenderError;

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
    pivot: u32,
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
}

impl AnimatedModel {
    /// Copies a validated composed W3D model without retaining VFS or parser state.
    ///
    /// # Errors
    ///
    /// Returns a structured error when geometry or hierarchy references exceed renderer limits.
    #[allow(clippy::too_many_lines)]
    pub fn from_w3d(model: &W3dModel) -> Result<Self, RenderError> {
        // Exercise the same bounds and hierarchy validation as deterministic bind-pose capture.
        let staged = StagedModel::from_w3d(model)?;
        let mut vertices = Vec::with_capacity(staged.vertex_count());
        let mut indices = Vec::with_capacity(staged.index_count());
        for model_mesh in model.meshes() {
            let mesh = model_mesh.mesh();
            let base = u32::try_from(vertices.len()).map_err(|_| RenderError::GeometryTooLarge)?;
            let colors = mesh.preview_vertex_colors();
            for (vertex_index, (position, normal)) in
                mesh.vertices().iter().zip(mesh.normals()).enumerate()
            {
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
                vertices.push(AnimatedVertex {
                    position: [position.x(), position.y(), position.z()],
                    normal: [normal.x(), normal.y(), normal.z()],
                    color,
                    pivot,
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

    pub(crate) fn index_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.indices.len() * 4);
        for index in &self.indices {
            bytes.extend_from_slice(&index.to_le_bytes());
        }
        bytes
    }

    /// Stages one explicit integer animation frame and model rotation for the presentation GPU.
    pub(crate) fn frame_vertex_bytes(
        &self,
        animation: Option<usize>,
        frame: u32,
        rotation: f32,
        aspect: f32,
    ) -> Result<Vec<u8>, RenderError> {
        if !rotation.is_finite() || !aspect.is_finite() || aspect <= 0.0 {
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
        let mut posed = Vec::with_capacity(self.vertices.len());
        let mut minimum = [f32::INFINITY; 3];
        let mut maximum = [f32::NEG_INFINITY; 3];
        for vertex in &self.vertices {
            let world = worlds
                .get(usize::try_from(vertex.pivot).map_err(|_| RenderError::InvalidHierarchy)?)
                .copied()
                .ok_or(RenderError::InvalidHierarchy)?;
            let position = rotate_up(world.point(vertex.position));
            let normal = rotate_up(world.vector(vertex.normal));
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
            posed.push((position, normal, vertex.color));
        }
        Ok(projected_vertex_bytes(&posed, minimum, maximum, aspect))
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

fn projected_vertex_bytes(
    vertices: &[([f32; 3], [f32; 3], [f32; 4])],
    minimum: [f32; 3],
    maximum: [f32; 3],
    aspect: f32,
) -> Vec<u8> {
    const SQRT_HALF: f32 = 0.707_106_77;
    const RIGHT: [f32; 3] = [-SQRT_HALF, SQRT_HALF, 0.0];
    const UP: [f32; 3] = [-0.5, -0.5, SQRT_HALF];
    const TO_CAMERA: [f32; 3] = [0.5, 0.5, SQRT_HALF];
    const LIGHT: [f32; 3] = [-0.371_390_67, -0.557_086, 0.742_781_34];
    let center = scale(add(minimum, maximum), 0.5);
    let camera = vertices
        .iter()
        .map(|(position, _, _)| {
            let relative = subtract(*position, center);
            [
                dot(relative, RIGHT),
                dot(relative, UP),
                -dot(relative, TO_CAMERA),
            ]
        })
        .collect::<Vec<_>>();
    let mut camera_minimum = [f32::INFINITY; 3];
    let mut camera_maximum = [f32::NEG_INFINITY; 3];
    for position in &camera {
        for axis in 0..3 {
            camera_minimum[axis] = camera_minimum[axis].min(position[axis]);
            camera_maximum[axis] = camera_maximum[axis].max(position[axis]);
        }
    }
    let horizontal = (camera_maximum[0] - camera_minimum[0]).max(f32::EPSILON);
    let vertical = (camera_maximum[1] - camera_minimum[1]).max(f32::EPSILON);
    let camera_center = [
        (camera_minimum[0] + camera_maximum[0]) * 0.5,
        (camera_minimum[1] + camera_maximum[1]) * 0.5,
    ];
    let fit = (1.8 * aspect / horizontal).min(1.8 / vertical);
    let depth = (camera_maximum[2] - camera_minimum[2]).max(f32::EPSILON);
    let mut bytes = Vec::with_capacity(vertices.len() * 28);
    for (position, (_, normal, color)) in camera.into_iter().zip(vertices) {
        let normal = normalize_vector(*normal);
        let illumination = 0.3 + 0.7 * dot(normal, LIGHT).abs();
        let output = [
            (position[0] - camera_center[0]) * fit / aspect,
            (position[1] - camera_center[1]) * fit,
            0.1 + 0.8 * ((position[2] - camera_minimum[2]) / depth),
            color[0] * illumination,
            color[1] * illumination,
            color[2] * illumination,
            1.0,
        ];
        for value in output {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    bytes
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
    use super::{Transform, projected_vertex_bytes};

    #[test]
    fn projected_frame_recenters_asymmetric_geometry_inside_margin() {
        let vertices = [
            ([-9.0, -2.0, 0.0], [0.0, 0.0, 1.0], [1.0; 4]),
            ([4.0, 1.0, 3.0], [0.0, 0.0, 1.0], [1.0; 4]),
            ([12.0, 8.0, -1.0], [0.0, 0.0, 1.0], [1.0; 4]),
        ];
        let bytes =
            projected_vertex_bytes(&vertices, [-9.0, -2.0, -1.0], [12.0, 8.0, 3.0], 4.0 / 3.0);
        let positions = bytes
            .chunks_exact(28)
            .map(|vertex| {
                std::array::from_fn(|axis| {
                    f32::from_le_bytes(
                        vertex[axis * 4..axis * 4 + 4]
                            .try_into()
                            .expect("four-byte float"),
                    )
                })
            })
            .collect::<Vec<[f32; 3]>>();
        let minimum_x = positions
            .iter()
            .map(|position| position[0])
            .fold(f32::INFINITY, f32::min);
        let maximum_x = positions
            .iter()
            .map(|position| position[0])
            .fold(f32::NEG_INFINITY, f32::max);
        let minimum_y = positions
            .iter()
            .map(|position| position[1])
            .fold(f32::INFINITY, f32::min);
        let maximum_y = positions
            .iter()
            .map(|position| position[1])
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(minimum_x >= -0.901 && maximum_x <= 0.901);
        assert!(minimum_y >= -0.901 && maximum_y <= 0.901);
        assert!((minimum_x + maximum_x).abs() < 0.000_1);
        assert!((minimum_y + maximum_y).abs() < 0.000_1);
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
