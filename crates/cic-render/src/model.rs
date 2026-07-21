use cic_formats::W3dModel;

use crate::RenderError;

const MAX_GEOMETRY_BUFFER_BYTES: usize = 512 * 1_024 * 1_024;
const MAX_ABS_RENDER_COORDINATE: f32 = 1_000_000_000.0;

#[derive(Debug, Clone, Copy, PartialEq)]
struct ModelVertex {
    position: [f32; 3],
    normal: [f32; 3],
    color: [f32; 4],
}

#[derive(Debug, Clone, Copy)]
struct Transform {
    translation: [f32; 3],
    rotation: [f32; 4],
}

impl Transform {
    const IDENTITY: Self = Self {
        translation: [0.0; 3],
        rotation: [0.0, 0.0, 0.0, 1.0],
    };

    fn compose(self, local: Self) -> Self {
        let translated = rotate(self.rotation, local.translation);
        Self {
            translation: add(self.translation, translated),
            rotation: normalize_quaternion(multiply(self.rotation, local.rotation)),
        }
    }

    fn point(self, value: [f32; 3]) -> [f32; 3] {
        add(self.translation, rotate(self.rotation, value))
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
