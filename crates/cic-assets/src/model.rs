//! glTF 2.0 model import.
//!
//! # Why glTF rather than another custom format
//!
//! Terrain got a custom container because no standard describes a heightfield well. Models are the
//! opposite case: glTF is a published, versioned standard for exactly this, every DCC tool exports
//! it, and its data model — nodes, meshes, PBR materials, skins, animation samplers — already
//! matches what a renderer needs. Inventing a mesh format here would mean writing an exporter for
//! Blender before anyone could make a single asset.
//!
//! `.glb` is the expected form. It is a single seekable file with its buffers inline, so it needs no
//! sidecar resolution — which matters because assets arrive through [`cic_vfs`], where a relative
//! `uri` pointing at the host filesystem has no meaning and must not be followed.
//!
//! # Scope
//!
//! This imports the default scene's static geometry, flattening the node hierarchy and baking each
//! node's world transform into its vertices. Skins and animations are read as far as detecting that
//! they exist — a skinned model imports its bind-pose geometry and reports `has_skin`, rather than
//! silently dropping the rig without telling the caller.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// Explicit bounds applied while importing an untrusted model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelLimits {
    /// Maximum vertices summed across every imported primitive.
    pub maximum_vertices: usize,
    /// Maximum indices summed across every imported primitive.
    pub maximum_indices: usize,
    /// Maximum primitives retained from one model.
    pub maximum_primitives: usize,
    /// Maximum materials retained from one model.
    pub maximum_materials: usize,
    /// Maximum node-hierarchy depth walked while accumulating transforms.
    pub maximum_depth: usize,
}

impl Default for ModelLimits {
    fn default() -> Self {
        Self {
            maximum_vertices: 4 * 1_024 * 1_024,
            maximum_indices: 16 * 1_024 * 1_024,
            maximum_primitives: 4_096,
            maximum_materials: 1_024,
            maximum_depth: 128,
        }
    }
}

/// One imported vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelVertex {
    /// Position in the model's own space, with the node transform already applied.
    pub position: [f32; 3],
    /// Unit normal, likewise transformed.
    pub normal: [f32; 3],
    /// First texture coordinate set, or zero when the primitive has none.
    pub uv: [f32; 2],
}

/// One imported triangle list.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelPrimitive {
    /// Vertices, in accessor order.
    pub vertices: Vec<ModelVertex>,
    /// Triangle indices into `vertices`.
    pub indices: Vec<u32>,
    /// Index into [`Model::materials`], or `None` for the glTF default material.
    pub material: Option<usize>,
}

/// A physically-based material, in the subset a first renderer needs.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelMaterial {
    /// Material name, or an empty string when unnamed.
    pub name: String,
    /// Linear base colour and alpha.
    pub base_color: [f32; 4],
    /// Metallic factor in `0..=1`.
    pub metallic: f32,
    /// Roughness factor in `0..=1`.
    pub roughness: f32,
    /// Index of the base-colour texture within the glTF's image list, if any.
    pub base_color_texture: Option<usize>,
    /// Whether the material asks for alpha blending rather than opaque or masked rendering.
    pub blended: bool,
}

/// An imported model.
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    /// The default scene's name, or an empty string.
    pub name: String,
    /// Flattened triangle lists.
    pub primitives: Vec<ModelPrimitive>,
    /// Materials referenced by `primitives`.
    pub materials: Vec<ModelMaterial>,
    /// Whether the source declared any skin, so a caller knows bind-pose geometry was imported
    /// rather than assuming the model is genuinely static.
    pub has_skin: bool,
    /// Whether the source declared any animation.
    pub has_animation: bool,
}

impl Model {
    /// Returns the total vertex count across every primitive.
    #[must_use]
    pub fn vertex_count(&self) -> usize {
        self.primitives.iter().map(|p| p.vertices.len()).sum()
    }

    /// Returns the total triangle count across every primitive.
    #[must_use]
    pub fn triangle_count(&self) -> usize {
        self.primitives.iter().map(|p| p.indices.len() / 3).sum()
    }

    /// Returns the axis-aligned bounds as `(minimum, maximum)`, or `None` when there is no geometry.
    #[must_use]
    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        let mut minimum = [f32::INFINITY; 3];
        let mut maximum = [f32::NEG_INFINITY; 3];
        let mut seen = false;
        for primitive in &self.primitives {
            for vertex in &primitive.vertices {
                seen = true;
                for axis in 0..3 {
                    minimum[axis] = minimum[axis].min(vertex.position[axis]);
                    maximum[axis] = maximum[axis].max(vertex.position[axis]);
                }
            }
        }
        seen.then_some((minimum, maximum))
    }
}

/// Imports a `.glb` or self-contained `.gltf` model from bytes.
///
/// # Errors
///
/// Returns a structured [`ModelError`] when the container is malformed, references external files,
/// omits required attributes, uses a non-triangle topology, or exceeds a [`ModelLimits`] bound.
pub fn import_model(bytes: &[u8], limits: ModelLimits) -> Result<Model, ModelError> {
    let (document, buffers, _images) =
        gltf::import_slice(bytes).map_err(|error| ModelError::Gltf(Box::new(error)))?;

    if document.materials().len() > limits.maximum_materials {
        return Err(ModelError::LimitExceeded {
            what: "material count",
            actual: document.materials().len(),
            maximum: limits.maximum_materials,
        });
    }

    let materials = document
        .materials()
        .map(|material| {
            let pbr = material.pbr_metallic_roughness();
            ModelMaterial {
                name: material.name().unwrap_or_default().to_owned(),
                base_color: pbr.base_color_factor(),
                metallic: pbr.metallic_factor(),
                roughness: pbr.roughness_factor(),
                base_color_texture: pbr
                    .base_color_texture()
                    .map(|info| info.texture().source().index()),
                blended: matches!(material.alpha_mode(), gltf::material::AlphaMode::Blend),
            }
        })
        .collect::<Vec<_>>();

    let scene = document
        .default_scene()
        .or_else(|| document.scenes().next())
        .ok_or(ModelError::NoScene)?;

    let mut primitives = Vec::new();
    let mut totals = Totals::default();
    for node in scene.nodes() {
        walk(
            &node,
            IDENTITY,
            0,
            &buffers,
            limits,
            &mut primitives,
            &mut totals,
        )?;
    }

    Ok(Model {
        name: scene.name().unwrap_or_default().to_owned(),
        primitives,
        materials,
        has_skin: document.skins().len() > 0,
        has_animation: document.animations().len() > 0,
    })
}

#[derive(Debug, Default)]
struct Totals {
    vertices: usize,
    indices: usize,
}

const IDENTITY: [[f32; 4]; 4] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

fn walk(
    node: &gltf::Node<'_>,
    parent: [[f32; 4]; 4],
    depth: usize,
    buffers: &[gltf::buffer::Data],
    limits: ModelLimits,
    output: &mut Vec<ModelPrimitive>,
    totals: &mut Totals,
) -> Result<(), ModelError> {
    if depth > limits.maximum_depth {
        return Err(ModelError::LimitExceeded {
            what: "node depth",
            actual: depth,
            maximum: limits.maximum_depth,
        });
    }
    let world = multiply(parent, node.transform().matrix());

    if let Some(mesh) = node.mesh() {
        for primitive in mesh.primitives() {
            if output.len() >= limits.maximum_primitives {
                return Err(ModelError::LimitExceeded {
                    what: "primitive count",
                    actual: output.len() + 1,
                    maximum: limits.maximum_primitives,
                });
            }
            output.push(import_primitive(
                &primitive, world, buffers, limits, totals,
            )?);
        }
    }

    for child in node.children() {
        walk(
            &child,
            world,
            depth.saturating_add(1),
            buffers,
            limits,
            output,
            totals,
        )?;
    }
    Ok(())
}

/// Reads one triangle-list primitive, baking `world` into its vertices.
fn import_primitive(
    primitive: &gltf::Primitive<'_>,
    world: [[f32; 4]; 4],
    buffers: &[gltf::buffer::Data],
    limits: ModelLimits,
    totals: &mut Totals,
) -> Result<ModelPrimitive, ModelError> {
    if primitive.mode() != gltf::mesh::Mode::Triangles {
        return Err(ModelError::UnsupportedTopology {
            mode: format!("{:?}", primitive.mode()),
        });
    }

    let reader = primitive.reader(|buffer| buffers.get(buffer.index()).map(|data| &data.0[..]));
    let positions = reader
        .read_positions()
        .ok_or(ModelError::MissingAttribute("POSITION"))?
        .collect::<Vec<_>>();

    totals.vertices += positions.len();
    if totals.vertices > limits.maximum_vertices {
        return Err(ModelError::LimitExceeded {
            what: "vertex count",
            actual: totals.vertices,
            maximum: limits.maximum_vertices,
        });
    }

    // A missing normal set is legal glTF -- the spec says to compute flat normals. Rather than
    // silently shipping zero normals to the GPU, they are generated below.
    let normals: Option<Vec<[f32; 3]>> = reader.read_normals().map(Iterator::collect);
    let uvs = reader
        .read_tex_coords(0)
        .map(|iter| iter.into_f32().collect::<Vec<_>>());

    let indices = match reader.read_indices() {
        Some(indices) => indices.into_u32().collect::<Vec<_>>(),
        // An unindexed primitive is a plain triangle sequence.
        None => (0..u32::try_from(positions.len()).map_err(|_| ModelError::LimitExceeded {
            what: "vertex count",
            actual: positions.len(),
            maximum: u32::MAX as usize,
        })?)
            .collect(),
    };
    if !indices.len().is_multiple_of(3) {
        return Err(ModelError::IndexCountNotTriangles(indices.len()));
    }
    totals.indices += indices.len();
    if totals.indices > limits.maximum_indices {
        return Err(ModelError::LimitExceeded {
            what: "index count",
            actual: totals.indices,
            maximum: limits.maximum_indices,
        });
    }
    // Checked before any vertex is built, so a hostile index cannot reach an indexing operation.
    for index in &indices {
        if *index as usize >= positions.len() {
            return Err(ModelError::IndexOutOfRange {
                index: *index,
                vertices: positions.len(),
            });
        }
    }

    let normal_matrix = normal_basis(world);
    let mut vertices = Vec::with_capacity(positions.len());
    for (position_index, position) in positions.iter().enumerate() {
        let normal = normals
            .as_ref()
            .and_then(|set| set.get(position_index).copied())
            .unwrap_or([0.0, 0.0, 0.0]);
        vertices.push(ModelVertex {
            position: transform_point(world, *position),
            normal: normalize(transform_direction(normal_matrix, normal)),
            uv: uvs
                .as_ref()
                .and_then(|set| set.get(position_index).copied())
                .unwrap_or([0.0, 0.0]),
        });
    }
    if normals.is_none() {
        generate_flat_normals(&mut vertices, &indices);
    }

    Ok(ModelPrimitive {
        vertices,
        indices,
        material: primitive.material().index(),
    })
}

/// Accumulates area-weighted face normals, which is what the glTF spec's flat-normal fallback
/// amounts to for a shared-vertex triangle list.
fn generate_flat_normals(vertices: &mut [ModelVertex], indices: &[u32]) {
    for vertex in vertices.iter_mut() {
        vertex.normal = [0.0; 3];
    }
    for triangle in indices.chunks_exact(3) {
        let [a, b, c] = [
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        ];
        let edge1 = subtract(vertices[b].position, vertices[a].position);
        let edge2 = subtract(vertices[c].position, vertices[a].position);
        let face = cross(edge1, edge2);
        for index in [a, b, c] {
            for (component, contribution) in vertices[index].normal.iter_mut().zip(face) {
                *component += contribution;
            }
        }
    }
    for vertex in vertices.iter_mut() {
        vertex.normal = normalize(vertex.normal);
    }
}

fn multiply(left: [[f32; 4]; 4], right: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    // glTF matrices are column-major: `matrix[column][row]`.
    let mut result = [[0.0f32; 4]; 4];
    for column in 0..4 {
        for row in 0..4 {
            let mut sum = 0.0;
            for step in 0..4 {
                sum += left[step][row] * right[column][step];
            }
            result[column][row] = sum;
        }
    }
    result
}

fn transform_point(matrix: [[f32; 4]; 4], point: [f32; 3]) -> [f32; 3] {
    let mut result = [0.0f32; 3];
    for row in 0..3 {
        result[row] = matrix[0][row] * point[0]
            + matrix[1][row] * point[1]
            + matrix[2][row] * point[2]
            + matrix[3][row];
    }
    result
}

fn transform_direction(matrix: [[f32; 3]; 3], direction: [f32; 3]) -> [f32; 3] {
    let mut result = [0.0f32; 3];
    for row in 0..3 {
        result[row] = matrix[0][row] * direction[0]
            + matrix[1][row] * direction[1]
            + matrix[2][row] * direction[2];
    }
    result
}

/// Returns the upper-left 3x3 of a transform, which is the correct basis for a normal as long as the
/// transform carries no non-uniform scale or shear.
///
/// A full inverse-transpose is the general answer. It is deliberately not used here: node transforms
/// in practice are rotate-translate-uniform-scale, where this agrees exactly, and the normals are
/// renormalized afterward — so the general case would cost an inverse per node to change nothing.
const fn normal_basis(matrix: [[f32; 4]; 4]) -> [[f32; 3]; 3] {
    [
        [matrix[0][0], matrix[0][1], matrix[0][2]],
        [matrix[1][0], matrix[1][1], matrix[1][2]],
        [matrix[2][0], matrix[2][1], matrix[2][2]],
    ]
}

fn subtract(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn cross(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn normalize(vector: [f32; 3]) -> [f32; 3] {
    let length = (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt();
    if length > 0.0 && length.is_finite() {
        [vector[0] / length, vector[1] / length, vector[2] / length]
    } else {
        [0.0, 0.0, 1.0]
    }
}

/// A structured failure while importing a model.
#[derive(Debug)]
pub enum ModelError {
    /// The glTF container itself was malformed, or referenced an external resource.
    Gltf(Box<gltf::Error>),
    /// The document declared no scene.
    NoScene,
    /// A required vertex attribute was absent.
    MissingAttribute(&'static str),
    /// A primitive used a topology other than triangles.
    UnsupportedTopology {
        /// The declared mode.
        mode: String,
    },
    /// A primitive's index count was not a multiple of three.
    IndexCountNotTriangles(usize),
    /// An index pointed past the primitive's vertex array.
    IndexOutOfRange {
        /// The offending index.
        index: u32,
        /// Vertices actually present.
        vertices: usize,
    },
    /// An explicit [`ModelLimits`] bound was exceeded.
    LimitExceeded {
        /// Which bound was crossed.
        what: &'static str,
        /// Observed value.
        actual: usize,
        /// Configured maximum.
        maximum: usize,
    },
}

impl Display for ModelError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gltf(error) => write!(formatter, "glTF import failed: {error}"),
            Self::NoScene => formatter.write_str("glTF declares no scene"),
            Self::MissingAttribute(name) => {
                write!(formatter, "primitive is missing its {name} attribute")
            }
            Self::UnsupportedTopology { mode } => {
                write!(formatter, "unsupported primitive topology {mode}")
            }
            Self::IndexCountNotTriangles(count) => {
                write!(formatter, "index count {count} is not a multiple of three")
            }
            Self::IndexOutOfRange { index, vertices } => write!(
                formatter,
                "index {index} is past the {vertices} vertices present"
            ),
            Self::LimitExceeded {
                what,
                actual,
                maximum,
            } => write!(formatter, "{what} {actual} exceeds maximum {maximum}"),
        }
    }
}

impl Error for ModelError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Gltf(error) => Some(error.as_ref()),
            _ => None,
        }
    }
}
