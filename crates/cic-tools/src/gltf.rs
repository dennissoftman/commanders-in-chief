//! Deterministic glTF 2.0 export for composed W3D models.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use cic_formats::{
    W3dAnimationChannel, W3dAnimationChannelKind, W3dAnimationEncoding, W3dFaceIds, W3dMaterialIds,
    W3dModel, W3dStaticMesh,
};
use serde_json::{Map, Value, json};

// Provenance: project-authored preview policy based on user-owned asset verification and the raw
// channel semantics in GeneralsGameCode revision `9f7abb866f5afd446db14149979e744c7216baaf`;
// see `docs/provenance/w3d.md`. W3D assets can hide carried or equipped geometry by translating
// its attachment bone hundreds or thousands of model widths away. The legacy renderer tolerates
// that convention, but glTF viewers include the remote geometry in animated bounds and reduce the
// actual model to a speck. Keep this policy local to interchange export: translations farther than
// both this absolute floor and the model-relative multiplier become near-zero-scale attachment
// states without introducing singular joint transforms.
const HIDDEN_ATTACHMENT_MIN_DISTANCE: f32 = 100.0;
const HIDDEN_ATTACHMENT_MODEL_MULTIPLIER: f32 = 32.0;
const HIDDEN_ATTACHMENT_SCALE: f32 = 0.000_1;

/// One source image that the caller must resolve and convert to the named PNG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GltfTextureRequest {
    source_name: Vec<u8>,
    output_name: String,
    additive_preview: bool,
}

impl GltfTextureRequest {
    #[must_use]
    pub fn source_name_bytes(&self) -> &[u8] {
        &self.source_name
    }
    #[must_use]
    pub fn output_name(&self) -> &str {
        &self.output_name
    }

    /// Returns whether this image is a derived core-glTF approximation of W3D `ONE + ONE`
    /// additive blending rather than the unmodified decoded source image.
    #[must_use]
    pub const fn is_additive_preview(&self) -> bool {
        self.additive_preview
    }
}

/// A `.gltf` JSON document, its external binary buffer, and image requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct W3dGltfBundle {
    pub json: String,
    pub binary: Vec<u8>,
    pub textures: Vec<GltfTextureRequest>,
}

/// A failure while packing a generated glTF bundle into the GLB container.
#[derive(Debug)]
pub enum W3dGlbError {
    Json(serde_json::Error),
    TextureCount { expected: usize, actual: usize },
    GeneratedDocument(&'static str),
    OutputTooLarge,
}

impl Display for W3dGlbError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => Display::fmt(error, formatter),
            Self::TextureCount { expected, actual } => write!(
                formatter,
                "glTF bundle requested {expected} textures, but {actual} PNG images were supplied"
            ),
            Self::GeneratedDocument(what) => {
                write!(formatter, "generated glTF document has invalid {what}")
            }
            Self::OutputTooLarge => formatter.write_str("GLB output exceeds its 32-bit size limit"),
        }
    }
}

impl Error for W3dGlbError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

impl From<serde_json::Error> for W3dGlbError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

/// Packs a generated glTF bundle and its resolved PNG images into one GLB 2.0 file.
///
/// # Errors
///
/// Returns an error if the generated document is inconsistent, the PNG count differs from
/// its image requests, JSON serialization fails, or the output exceeds GLB's 32-bit size limit.
pub fn pack_w3d_glb(
    mut bundle: W3dGltfBundle,
    png_images: &[Vec<u8>],
) -> Result<Vec<u8>, W3dGlbError> {
    if bundle.textures.len() != png_images.len() {
        return Err(W3dGlbError::TextureCount {
            expected: bundle.textures.len(),
            actual: png_images.len(),
        });
    }
    let mut document: Value = serde_json::from_str(&bundle.json)?;
    let root = document
        .as_object_mut()
        .ok_or(W3dGlbError::GeneratedDocument("root object"))?;

    let mut image_views = Vec::with_capacity(png_images.len());
    {
        let views = root
            .get_mut("bufferViews")
            .and_then(Value::as_array_mut)
            .ok_or(W3dGlbError::GeneratedDocument("bufferViews array"))?;
        for png in png_images {
            align_bytes(&mut bundle.binary, 0);
            let offset = bundle.binary.len();
            let end = offset
                .checked_add(png.len())
                .ok_or(W3dGlbError::OutputTooLarge)?;
            if end > u32::MAX as usize {
                return Err(W3dGlbError::OutputTooLarge);
            }
            bundle.binary.extend_from_slice(png);
            image_views.push(views.len());
            views.push(json!({
                "buffer": 0,
                "byteOffset": offset,
                "byteLength": png.len()
            }));
        }
    }
    if !image_views.is_empty() {
        let images = root
            .get_mut("images")
            .and_then(Value::as_array_mut)
            .ok_or(W3dGlbError::GeneratedDocument("images array"))?;
        if images.len() != image_views.len() {
            return Err(W3dGlbError::GeneratedDocument("images count"));
        }
        for (image, view) in images.iter_mut().zip(image_views) {
            let object = image
                .as_object_mut()
                .ok_or(W3dGlbError::GeneratedDocument("image object"))?;
            object.remove("uri");
            object.insert("mimeType".into(), json!("image/png"));
            object.insert("bufferView".into(), json!(view));
        }
    }
    let buffer = root
        .get_mut("buffers")
        .and_then(Value::as_array_mut)
        .and_then(|buffers| buffers.first_mut())
        .and_then(Value::as_object_mut)
        .ok_or(W3dGlbError::GeneratedDocument("primary buffer"))?;
    buffer.remove("uri");
    buffer.insert("byteLength".into(), json!(bundle.binary.len()));

    let mut json_bytes = serde_json::to_vec(&document)?;
    align_bytes(&mut json_bytes, b' ');
    let binary_length = bundle.binary.len();
    align_bytes(&mut bundle.binary, 0);
    let total_length = 12_usize
        .checked_add(8)
        .and_then(|length| length.checked_add(json_bytes.len()))
        .and_then(|length| length.checked_add(8))
        .and_then(|length| length.checked_add(bundle.binary.len()))
        .ok_or(W3dGlbError::OutputTooLarge)?;
    let total_length = u32::try_from(total_length).map_err(|_| W3dGlbError::OutputTooLarge)?;
    let json_length = u32::try_from(json_bytes.len()).map_err(|_| W3dGlbError::OutputTooLarge)?;
    let binary_chunk_length =
        u32::try_from(bundle.binary.len()).map_err(|_| W3dGlbError::OutputTooLarge)?;
    let mut glb = Vec::with_capacity(total_length as usize);
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2_u32.to_le_bytes());
    glb.extend_from_slice(&total_length.to_le_bytes());
    glb.extend_from_slice(&json_length.to_le_bytes());
    glb.extend_from_slice(&0x4E4F_534A_u32.to_le_bytes());
    glb.extend_from_slice(&json_bytes);
    glb.extend_from_slice(&binary_chunk_length.to_le_bytes());
    glb.extend_from_slice(&0x004E_4942_u32.to_le_bytes());
    glb.extend_from_slice(&bundle.binary);
    debug_assert!(binary_chunk_length as usize - binary_length < 4);
    Ok(glb)
}

fn align_bytes(bytes: &mut Vec<u8>, padding: u8) {
    while !bytes.len().is_multiple_of(4) {
        bytes.push(padding);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct MaterialKey {
    texture: Option<u32>,
    shader: Option<u32>,
    vertex_material: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TextureVariant {
    Source,
    AdditivePreview,
}

#[derive(Default)]
struct BufferBuilder {
    bytes: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl BufferBuilder {
    fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        while !self.bytes.len().is_multiple_of(4) {
            self.bytes.push(0);
        }
        let offset = self.bytes.len();
        self.bytes.extend_from_slice(bytes);
        let mut view = Map::new();
        view.insert("buffer".into(), json!(0));
        view.insert("byteOffset".into(), json!(offset));
        view.insert("byteLength".into(), json!(bytes.len()));
        if let Some(target) = target {
            view.insert("target".into(), json!(target));
        }
        self.views.push(Value::Object(view));
        self.views.len() - 1
    }

    #[allow(clippy::too_many_arguments)]
    fn accessor(
        &mut self,
        view: usize,
        component: u32,
        count: usize,
        kind: &str,
        min: Option<Value>,
        max: Option<Value>,
        normalized: bool,
    ) -> usize {
        let mut accessor = Map::new();
        accessor.insert("bufferView".into(), json!(view));
        accessor.insert("componentType".into(), json!(component));
        accessor.insert("count".into(), json!(count));
        accessor.insert("type".into(), json!(kind));
        if let Some(min) = min {
            accessor.insert("min".into(), min);
        }
        if let Some(max) = max {
            accessor.insert("max".into(), max);
        }
        if normalized {
            accessor.insert("normalized".into(), json!(true));
        }
        self.accessors.push(Value::Object(accessor));
        self.accessors.len() - 1
    }

    fn f32_accessor(
        &mut self,
        values: &[f32],
        components: usize,
        kind: &str,
        target: Option<u32>,
        bounds: bool,
    ) -> usize {
        let mut bytes = Vec::with_capacity(values.len() * 4);
        for value in values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let view = self.view(&bytes, target);
        let (min, max) = if bounds {
            let mut minimum = vec![f32::INFINITY; components];
            let mut maximum = vec![f32::NEG_INFINITY; components];
            for row in values.chunks_exact(components) {
                for (index, value) in row.iter().copied().enumerate() {
                    minimum[index] = minimum[index].min(value);
                    maximum[index] = maximum[index].max(value);
                }
            }
            (Some(json!(minimum)), Some(json!(maximum)))
        } else {
            (None, None)
        };
        self.accessor(view, 5126, values.len() / components, kind, min, max, false)
    }
}

/// Builds a complete, Z-up-to-Y-up converted glTF 2.0 scene.
///
/// # Panics
///
/// Panics only if a `W3dModel` violates invariants enforced by `decode_w3d_model`, such as a
/// validated vertex, UV, pivot, material, or texture index being out of range.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn render_w3d_gltf(
    model: &W3dModel,
    binary_uri: &str,
    texture_directory: &str,
) -> W3dGltfBundle {
    let mut buffer = BufferBuilder::default();
    let mut meshes = Vec::new();
    let mut materials = Vec::new();
    let mut images = Vec::new();
    let mut textures_json = Vec::new();
    let mut samplers = Vec::new();
    let mut requests = Vec::new();
    let mut source_texture_map = BTreeMap::new();
    let mut additive_texture_map = BTreeMap::new();

    for (mesh_index, model_mesh) in model.meshes().iter().enumerate() {
        let mesh = model_mesh.mesh();
        let expanded = expand_mesh(mesh);
        let position = buffer.f32_accessor(&expanded.positions, 3, "VEC3", Some(34962), true);
        let normal = buffer.f32_accessor(&expanded.normals, 3, "VEC3", Some(34962), false);
        let texcoord = (!expanded.texcoords.is_empty())
            .then(|| buffer.f32_accessor(&expanded.texcoords, 2, "VEC2", Some(34962), false));
        let color = (!expanded.colors.is_empty()).then(|| {
            let view = buffer.view(&expanded.colors, Some(34962));
            buffer.accessor(
                view,
                5121,
                expanded.colors.len() / 4,
                "VEC4",
                None,
                None,
                true,
            )
        });
        let joints = (!expanded.joints.is_empty()).then(|| {
            let mut bytes = Vec::with_capacity(expanded.joints.len() * 2);
            for value in &expanded.joints {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            let view = buffer.view(&bytes, Some(34962));
            buffer.accessor(
                view,
                5123,
                expanded.joints.len() / 4,
                "VEC4",
                None,
                None,
                false,
            )
        });
        let weights = (!expanded.joints.is_empty()).then(|| {
            let mut values = Vec::with_capacity(expanded.vertex_count * 4);
            for _ in 0..expanded.vertex_count {
                values.extend_from_slice(&[1.0, 0.0, 0.0, 0.0]);
            }
            buffer.f32_accessor(&values, 4, "VEC4", Some(34962), false)
        });

        let mut groups: BTreeMap<MaterialKey, Vec<u32>> = BTreeMap::new();
        for triangle in 0..mesh.triangles().len() {
            let key = material_key(mesh, triangle);
            let base = u32::try_from(triangle).expect("bounded triangle index") * 3;
            groups
                .entry(key)
                .or_default()
                .extend_from_slice(&[base, base + 1, base + 2]);
        }
        let mut primitives = Vec::new();
        for (key, indices) in groups {
            let mut bytes = Vec::with_capacity(indices.len() * 4);
            for index in &indices {
                bytes.extend_from_slice(&index.to_le_bytes());
            }
            let view = buffer.view(&bytes, Some(34963));
            let index_accessor =
                buffer.accessor(view, 5125, indices.len(), "SCALAR", None, None, false);
            let material = build_material(
                mesh,
                mesh_index,
                key,
                !expanded.colors.is_empty(),
                texture_directory,
                &mut source_texture_map,
                &mut additive_texture_map,
                &mut materials,
                &mut images,
                &mut textures_json,
                &mut samplers,
                &mut requests,
            );
            let mut attributes = Map::new();
            attributes.insert("POSITION".into(), json!(position));
            attributes.insert("NORMAL".into(), json!(normal));
            if let Some(value) = texcoord {
                attributes.insert("TEXCOORD_0".into(), json!(value));
            }
            if let Some(value) = color {
                attributes.insert("COLOR_0".into(), json!(value));
            }
            if let Some(value) = joints {
                attributes.insert("JOINTS_0".into(), json!(value));
            }
            if let Some(value) = weights {
                attributes.insert("WEIGHTS_0".into(), json!(value));
            }
            primitives.push(json!({"attributes": attributes, "indices": index_accessor, "material": material, "mode": 4}));
        }
        for texture_id in 0..mesh.materials().textures().len() {
            ensure_gltf_texture(
                mesh,
                mesh_index,
                u32::try_from(texture_id).expect("bounded texture index"),
                texture_directory,
                TextureVariant::Source,
                &mut source_texture_map,
                &mut images,
                &mut textures_json,
                &mut samplers,
                &mut requests,
            );
        }
        meshes.push(json!({
            "name": mesh_name(mesh),
            "primitives": primitives,
            "extras": build_w3d_material_extras(mesh, mesh_index, &source_texture_map)
        }));
    }

    let (nodes, mesh_nodes, pivot_nodes) = build_nodes(model);
    let mut nodes = nodes;
    for (model_mesh, (node_index, mesh_index)) in
        model.meshes().iter().zip(mesh_nodes.iter().copied())
    {
        let mut node = Map::new();
        node.insert(
            "name".into(),
            json!(format!("{} instance", mesh_name(model_mesh.mesh()))),
        );
        node.insert("mesh".into(), json!(mesh_index));
        if model_mesh.mesh().vertex_bones().is_some() {
            node.insert("skin".into(), json!(0));
        }
        nodes[node_index] = Value::Object(node);
    }
    let mut scene_nodes = vec![0];
    scene_nodes.extend(
        model
            .meshes()
            .iter()
            .zip(&mesh_nodes)
            .filter(|(model_mesh, _)| model_mesh.mesh().vertex_bones().is_some())
            .map(|(_, (node_index, _))| *node_index),
    );

    let has_skin = model
        .meshes()
        .iter()
        .any(|mesh| mesh.mesh().vertex_bones().is_some());
    let skins = if has_skin {
        vec![json!({"name": "W3D hierarchy", "skeleton": pivot_nodes[0], "joints": pivot_nodes})]
    } else {
        Vec::new()
    };

    let animations = build_animations(model, &pivot_nodes, &mut buffer);
    let mut root = Map::new();
    root.insert(
        "asset".into(),
        json!({"version":"2.0", "generator":"Commanders in Chief"}),
    );
    root.insert("scene".into(), json!(0));
    root.insert(
        "scenes".into(),
        json!([{"name":"W3D model", "nodes":scene_nodes}]),
    );
    root.insert("nodes".into(), Value::Array(nodes));
    root.insert("meshes".into(), Value::Array(meshes));
    root.insert("materials".into(), Value::Array(materials));
    if !images.is_empty() {
        root.insert("images".into(), Value::Array(images));
    }
    if !textures_json.is_empty() {
        root.insert("textures".into(), Value::Array(textures_json));
    }
    if !samplers.is_empty() {
        root.insert("samplers".into(), Value::Array(samplers));
    }
    if !skins.is_empty() {
        root.insert("skins".into(), Value::Array(skins));
    }
    if !animations.is_empty() {
        root.insert("animations".into(), Value::Array(animations));
    }
    root.insert(
        "buffers".into(),
        json!([{"uri":binary_uri, "byteLength":buffer.bytes.len()}]),
    );
    root.insert("bufferViews".into(), Value::Array(buffer.views));
    root.insert("accessors".into(), Value::Array(buffer.accessors));
    let json = serde_json::to_string_pretty(&Value::Object(root))
        .expect("glTF values are serializable")
        + "\n";
    W3dGltfBundle {
        json,
        binary: buffer.bytes,
        textures: requests,
    }
}

struct ExpandedMesh {
    positions: Vec<f32>,
    normals: Vec<f32>,
    texcoords: Vec<f32>,
    colors: Vec<u8>,
    joints: Vec<u16>,
    vertex_count: usize,
}

fn expand_mesh(mesh: &W3dStaticMesh) -> ExpandedMesh {
    let stage = mesh
        .materials()
        .passes()
        .first()
        .and_then(|pass| pass.texture_stages().first());
    let colors = mesh.preview_vertex_colors();
    let bones = mesh.vertex_bones();
    let mut result = ExpandedMesh {
        positions: Vec::new(),
        normals: Vec::new(),
        texcoords: Vec::new(),
        colors: Vec::new(),
        joints: Vec::new(),
        vertex_count: mesh.triangles().len() * 3,
    };
    for (triangle_index, triangle) in mesh.triangles().iter().enumerate() {
        let vertices = triangle.vertex_indices();
        let uv_indices = stage.and_then(|stage| stage.coordinate_indices(triangle_index, vertices));
        for corner in 0..3 {
            let vertex_index = usize::try_from(vertices[corner]).expect("decoded vertex index");
            let vertex = mesh.vertices()[vertex_index];
            let normal = mesh.normals()[vertex_index];
            result
                .positions
                .extend_from_slice(&[vertex.x(), vertex.y(), vertex.z()]);
            result
                .normals
                .extend_from_slice(&[normal.x(), normal.y(), normal.z()]);
            if let (Some(stage), Some(uv_indices)) = (stage, uv_indices) {
                let uv = stage.texture_coordinates()
                    [usize::try_from(uv_indices[corner]).expect("decoded UV index")];
                result.texcoords.extend_from_slice(&[uv.u(), 1.0 - uv.v()]);
            }
            if let Some(color) = colors.as_ref().and_then(|colors| colors.get(vertex_index)) {
                result.colors.extend_from_slice(&[
                    color.red(),
                    color.green(),
                    color.blue(),
                    color.alpha(),
                ]);
            }
            if let Some(bones) = bones {
                result
                    .joints
                    .extend_from_slice(&[bones[vertex_index], 0, 0, 0]);
            }
        }
    }
    result
}

fn material_key(mesh: &W3dStaticMesh, triangle: usize) -> MaterialKey {
    let pass = mesh.materials().passes().first();
    MaterialKey {
        texture: pass
            .and_then(|pass| pass.texture_stages().first())
            .and_then(|stage| stage.texture_ids())
            .and_then(|ids| ids.for_triangle(triangle))
            .filter(|id| *id != u32::MAX),
        shader: pass
            .and_then(|pass| pass.shader_ids())
            .and_then(|ids| ids.for_triangle(triangle)),
        vertex_material: pass.and_then(|pass| match pass.vertex_material_ids() {
            Some(W3dMaterialIds::Single(id)) => Some(*id),
            _ => None,
        }),
    }
}

#[allow(clippy::too_many_arguments)]
fn build_material(
    mesh: &W3dStaticMesh,
    mesh_index: usize,
    key: MaterialKey,
    has_colors: bool,
    texture_directory: &str,
    source_texture_map: &mut BTreeMap<(usize, u32), usize>,
    additive_texture_map: &mut BTreeMap<(usize, u32), usize>,
    materials: &mut Vec<Value>,
    images: &mut Vec<Value>,
    textures_json: &mut Vec<Value>,
    samplers: &mut Vec<Value>,
    requests: &mut Vec<GltfTextureRequest>,
) -> usize {
    let shader = key
        .shader
        .and_then(|id| usize::try_from(id).ok())
        .and_then(|id| mesh.materials().shaders().get(id))
        .copied();
    let source_texture = key.texture.map(|texture_id| {
        ensure_gltf_texture(
            mesh,
            mesh_index,
            texture_id,
            texture_directory,
            TextureVariant::Source,
            source_texture_map,
            images,
            textures_json,
            samplers,
            requests,
        )
    });
    // Provenance: W3DSHADER_SRCBLEND_ONE and W3DSHADER_DESTBLEND_ONE selector values come
    // from `w3d_file.h` in GeneralsGameCode revision
    // `9f7abb866f5afd446db14149979e744c7216baaf`; see `docs/provenance/w3d.md`. Core glTF has
    // no additive blend equation, so use a separate derived image for its visible preview while
    // retaining the unmodified decoded source image above for fixed-function metadata consumers.
    let additive_preview =
        shader.is_some_and(|shader| shader.source_blend() == 1 && shader.destination_blend() == 1);
    let texture = if additive_preview {
        key.texture.map(|texture_id| {
            ensure_gltf_texture(
                mesh,
                mesh_index,
                texture_id,
                texture_directory,
                TextureVariant::AdditivePreview,
                additive_texture_map,
                images,
                textures_json,
                samplers,
                requests,
            )
        })
    } else {
        source_texture
    };
    let vertex_material = key
        .vertex_material
        .and_then(|id| usize::try_from(id).ok())
        .and_then(|id| mesh.materials().vertex_materials().get(id));
    let factor = if has_colors {
        [1.0, 1.0, 1.0, 1.0]
    } else if let Some(material) = vertex_material {
        let color = material.diffuse();
        [
            channel(color.red()),
            channel(color.green()),
            channel(color.blue()),
            material.opacity(),
        ]
    } else {
        [1.0, 1.0, 1.0, 1.0]
    };
    let mut pbr = Map::new();
    pbr.insert("baseColorFactor".into(), json!(factor));
    pbr.insert("metallicFactor".into(), json!(0.0));
    pbr.insert("roughnessFactor".into(), json!(1.0));
    if let Some(texture) = texture {
        pbr.insert("baseColorTexture".into(), json!({"index":texture}));
    }
    let alpha_mode = shader.map_or("OPAQUE", |shader| {
        if shader.alpha_test() != 0 {
            "MASK"
        } else if shader.destination_blend() != 0 {
            "BLEND"
        } else {
            "OPAQUE"
        }
    });
    let material_index = materials.len();
    let mut material = Map::new();
    material.insert(
        "name".into(),
        json!(format!("mesh {mesh_index} material {material_index}")),
    );
    material.insert("pbrMetallicRoughness".into(), Value::Object(pbr));
    material.insert("alphaMode".into(), json!(alpha_mode));
    if alpha_mode == "MASK" {
        material.insert("alphaCutoff".into(), json!(0.5));
    }
    material.insert("doubleSided".into(), json!(false));
    material.insert(
        "extras".into(),
        json!({
            "w3dShader": key.shader,
            "w3dPreviewBlend": if additive_preview {
                "additive-alpha-coverage-v1"
            } else {
                "source-rgba"
            }
        }),
    );
    materials.push(Value::Object(material));
    material_index
}

#[allow(clippy::too_many_arguments)]
fn ensure_gltf_texture(
    mesh: &W3dStaticMesh,
    mesh_index: usize,
    texture_id: u32,
    texture_directory: &str,
    variant: TextureVariant,
    texture_map: &mut BTreeMap<(usize, u32), usize>,
    images: &mut Vec<Value>,
    textures_json: &mut Vec<Value>,
    samplers: &mut Vec<Value>,
    requests: &mut Vec<GltfTextureRequest>,
) -> usize {
    *texture_map
        .entry((mesh_index, texture_id))
        .or_insert_with(|| {
            let source = &mesh.materials().textures()
                [usize::try_from(texture_id).expect("decoded texture ID")];
            let suffix = match variant {
                TextureVariant::Source => "",
                TextureVariant::AdditivePreview => "_additive-preview",
            };
            let output_name = format!(
                "m{mesh_index:03}_t{texture_id:04}_{}{suffix}.png",
                safe_stem(source.name_bytes())
            );
            let image_index = images.len();
            images.push(json!({
                "name": String::from_utf8_lossy(source.name_bytes()),
                "uri": format!("{texture_directory}/{output_name}")
            }));
            let attributes = source
                .info()
                .map_or(0, cic_formats::W3dTextureInfo::attributes);
            let sampler_index = samplers.len();
            samplers.push(json!({
                "wrapS": if attributes & 0x8 != 0 { 33071 } else { 10497 },
                "wrapT": if attributes & 0x10 != 0 { 33071 } else { 10497 }
            }));
            let texture_index = textures_json.len();
            textures_json.push(json!({"source":image_index, "sampler":sampler_index}));
            requests.push(GltfTextureRequest {
                source_name: source.name_bytes().to_vec(),
                output_name,
                additive_preview: variant == TextureVariant::AdditivePreview,
            });
            texture_index
        })
}

#[allow(clippy::too_many_lines)]
fn build_w3d_material_extras(
    mesh: &W3dStaticMesh,
    mesh_index: usize,
    texture_map: &BTreeMap<(usize, u32), usize>,
) -> Value {
    // Project-authored interchange policy. Field meanings come from the GPL-3.0-or-later
    // GeneralsGameCode revision named in `docs/provenance/w3d.md`; no upstream code is copied.
    let materials = mesh.materials();
    let vertex_materials = materials
        .vertex_materials()
        .iter()
        .map(|material| {
            let mappers = (0..2)
                .map(|stage| {
                    let mapper = material.mapper(stage).expect("fixed mapper stage");
                    json!({
                        "stage": stage,
                        "mode": mapper.mode().code(),
                        "modeName": mapper.mode().name(),
                        "argumentBytes": mapper.argument_bytes()
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "nameBytes": material.name_bytes(),
                "attributes": material.attributes(),
                "ambient": rgb_json(material.ambient()),
                "diffuse": rgb_json(material.diffuse()),
                "specular": rgb_json(material.specular()),
                "emissive": rgb_json(material.emissive()),
                "shininessBits": material.shininess().to_bits(),
                "opacityBits": material.opacity().to_bits(),
                "translucencyBits": material.translucency().to_bits(),
                "mappers": mappers
            })
        })
        .collect::<Vec<_>>();
    let shaders = materials
        .shaders()
        .iter()
        .map(|shader| json!(shader.bytes()))
        .collect::<Vec<_>>();
    let textures = materials
        .textures()
        .iter()
        .enumerate()
        .map(|(texture_id, texture)| {
            let info = texture.info().map(|info| {
                json!({
                    "attributes": info.attributes(),
                    "animationType": info.animation_type(),
                    "frameCount": info.frame_count(),
                    "frameRateBits": info.frame_rate().to_bits()
                })
            });
            let texture_id = u32::try_from(texture_id).expect("bounded texture index");
            json!({
                "nameBytes": texture.name_bytes(),
                "info": info,
                "gltfTexture": texture_map.get(&(mesh_index, texture_id))
            })
        })
        .collect::<Vec<_>>();
    let passes = materials
        .passes()
        .iter()
        .enumerate()
        .map(|(pass_index, pass)| {
            let stages = pass
                .texture_stages()
                .iter()
                .enumerate()
                .map(|(stage_index, stage)| {
                    let coordinate_bits = stage
                        .texture_coordinates()
                        .iter()
                        .map(|coordinate| [coordinate.u().to_bits(), coordinate.v().to_bits()])
                        .collect::<Vec<_>>();
                    json!({
                        "stage": stage_index,
                        "textureIds": face_ids_json(stage.texture_ids()),
                        "textureCoordinateBits": coordinate_bits,
                        "perFaceTextureCoordinateIds": stage.per_face_coordinate_ids()
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "pass": pass_index,
                "vertexMaterialIds": material_ids_json(pass.vertex_material_ids()),
                "shaderIds": face_ids_json(pass.shader_ids()),
                "diffuseColors": pass.diffuse_colors().map(|colors| colors.iter().map(|color| [color.red(), color.green(), color.blue(), color.alpha()]).collect::<Vec<_>>()),
                "diffuseIllumination": pass.diffuse_illumination().map(|colors| colors.iter().map(|color| [color.red(), color.green(), color.blue()]).collect::<Vec<_>>()),
                "specularColors": pass.specular_colors().map(|colors| colors.iter().map(|color| [color.red(), color.green(), color.blue()]).collect::<Vec<_>>()),
                "textureStages": stages
            })
        })
        .collect::<Vec<_>>();
    json!({
        "w3dMaterialPolicy": "fixed-function-metadata-v1",
        "visiblePreview": {"pass": 0, "stage": 0, "approximation": "metallic-roughness"},
        "materialInfo": materials.info().map(|info| json!({
            "passes": info.pass_count(),
            "vertexMaterials": info.vertex_material_count(),
            "shaders": info.shader_count(),
            "textures": info.texture_count()
        })),
        "vertexMaterials": vertex_materials,
        "shaders": shaders,
        "textures": textures,
        "passes": passes
    })
}

fn rgb_json(color: cic_formats::W3dRgb8) -> [u8; 3] {
    [color.red(), color.green(), color.blue()]
}

fn material_ids_json(ids: Option<&W3dMaterialIds>) -> Value {
    match ids {
        None => Value::Null,
        Some(W3dMaterialIds::Single(id)) => json!({"single": id}),
        Some(W3dMaterialIds::PerVertex(ids)) => json!({"perVertex": ids}),
    }
}

fn face_ids_json(ids: Option<&W3dFaceIds>) -> Value {
    match ids {
        None => Value::Null,
        Some(W3dFaceIds::Single(id)) => json!({"single": id}),
        Some(W3dFaceIds::PerTriangle(ids)) => json!({"perTriangle": ids}),
    }
}

fn build_nodes(model: &W3dModel) -> (Vec<Value>, Vec<(usize, usize)>, Vec<usize>) {
    let pivot_count = model.hierarchy().pivots().len();
    let mut children = vec![Vec::new(); pivot_count];
    for (index, pivot) in model.hierarchy().pivots().iter().enumerate().skip(1) {
        if let Some(parent) = pivot.parent() {
            children[usize::try_from(parent).expect("decoded parent")].push(index + 1);
        }
    }
    let mut mesh_nodes = Vec::new();
    let first_mesh_node = 1 + pivot_count;
    for (mesh_index, model_mesh) in model.meshes().iter().enumerate() {
        let node_index = first_mesh_node + mesh_index;
        if model_mesh.mesh().vertex_bones().is_none() {
            let parent = usize::try_from(model_mesh.pivot()).expect("decoded pivot");
            children[parent].push(node_index);
        }
        mesh_nodes.push((node_index, mesh_index));
    }
    let mut nodes = Vec::with_capacity(first_mesh_node + model.meshes().len());
    nodes.push(json!({"name":"W3D Z-up to glTF Y-up", "rotation":[-0.707_106_77,0.0,0.0,0.707_106_77], "children":[1]}));
    for (index, pivot) in model.hierarchy().pivots().iter().enumerate() {
        let mut node = Map::new();
        node.insert(
            "name".into(),
            json!(String::from_utf8_lossy(pivot.name_bytes())),
        );
        if index != 0 {
            node.insert("translation".into(), json!(pivot.translation()));
            node.insert(
                "rotation".into(),
                json!(normalize(pivot.rotation().components())),
            );
        }
        if !children[index].is_empty() {
            node.insert("children".into(), json!(children[index]));
        }
        nodes.push(Value::Object(node));
    }
    nodes.resize(first_mesh_node + model.meshes().len(), Value::Null);
    let pivot_nodes = (1..=pivot_count).collect();
    (nodes, mesh_nodes, pivot_nodes)
}

#[allow(clippy::cast_precision_loss, clippy::too_many_lines)]
fn build_animations(
    model: &W3dModel,
    pivot_nodes: &[usize],
    buffer: &mut BufferBuilder,
) -> Vec<Value> {
    let mut result = Vec::new();
    let hidden_attachment_distance = hidden_attachment_distance(model);
    for animation in model.animations() {
        let times = (0..animation.frame_count())
            .map(|frame| frame as f32 / animation.frame_rate() as f32)
            .collect::<Vec<_>>();
        let time_accessor = buffer.f32_accessor(&times, 1, "SCALAR", None, true);
        let mut samplers = Vec::new();
        let mut channels_json = Vec::new();
        let animated_pivots = animation
            .channels()
            .iter()
            .map(cic_formats::W3dAnimationChannel::pivot)
            .filter(|pivot| *pivot != 0)
            .collect::<BTreeSet<_>>();
        for pivot in animated_pivots {
            let base = &model.hierarchy().pivots()[usize::from(pivot)];
            let pivot_channels = animation
                .channels()
                .iter()
                .filter(|channel| channel.pivot() == pivot)
                .collect::<Vec<_>>();
            if pivot_channels.iter().any(|channel| {
                matches!(
                    channel.kind(),
                    W3dAnimationChannelKind::X
                        | W3dAnimationChannelKind::Y
                        | W3dAnimationChannelKind::Z
                )
            }) {
                let (values, scales) = build_translation_samples(
                    &pivot_channels,
                    animation.frame_count(),
                    base.translation(),
                    base.rotation().components(),
                    hidden_attachment_distance,
                );
                add_animation_channel(
                    buffer,
                    &mut samplers,
                    &mut channels_json,
                    time_accessor,
                    &values,
                    3,
                    "VEC3",
                    pivot_nodes[usize::from(pivot)],
                    "translation",
                    "LINEAR",
                );
                if let Some(scales) = scales {
                    add_animation_channel(
                        buffer,
                        &mut samplers,
                        &mut channels_json,
                        time_accessor,
                        &scales,
                        3,
                        "VEC3",
                        pivot_nodes[usize::from(pivot)],
                        "scale",
                        "STEP",
                    );
                }
            }
            if pivot_channels
                .iter()
                .any(|channel| channel.kind() == W3dAnimationChannelKind::Quaternion)
            {
                let mut values = Vec::with_capacity(times.len() * 4);
                let mut previous = None;
                for frame in 0..animation.frame_count() {
                    let animated = sample_quaternion(&pivot_channels, frame);
                    let mut value = normalize(multiply(base.rotation().components(), animated));
                    if previous.is_some_and(|previous: [f32; 4]| dot(previous, value) < 0.0) {
                        for component in &mut value {
                            *component = -*component;
                        }
                    }
                    previous = Some(value);
                    values.extend_from_slice(&value);
                }
                add_animation_channel(
                    buffer,
                    &mut samplers,
                    &mut channels_json,
                    time_accessor,
                    &values,
                    4,
                    "VEC4",
                    pivot_nodes[usize::from(pivot)],
                    "rotation",
                    "LINEAR",
                );
            }
        }
        if !channels_json.is_empty() {
            let encoding = match animation.encoding() {
                W3dAnimationEncoding::Raw => "raw",
                W3dAnimationEncoding::TimeCoded => "time-coded",
                W3dAnimationEncoding::AdaptiveDelta => "adaptive-delta",
            };
            result.push(json!({
                "name": String::from_utf8_lossy(animation.name_bytes()),
                "samplers": samplers,
                "channels": channels_json,
                "extras": {"w3dEncoding": encoding}
            }));
        }
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn add_animation_channel(
    buffer: &mut BufferBuilder,
    samplers: &mut Vec<Value>,
    channels: &mut Vec<Value>,
    input: usize,
    values: &[f32],
    components: usize,
    kind: &str,
    node: usize,
    path: &str,
    interpolation: &str,
) {
    let output = buffer.f32_accessor(values, components, kind, None, false);
    let sampler = samplers.len();
    samplers.push(json!({"input":input, "output":output, "interpolation":interpolation}));
    channels.push(json!({"sampler":sampler, "target":{"node":node, "path":path}}));
}

fn build_translation_samples(
    channels: &[&W3dAnimationChannel],
    frame_count: u32,
    base_translation: [f32; 3],
    base_rotation: [f32; 4],
    hidden_attachment_distance: f32,
) -> (Vec<f32>, Option<Vec<f32>>) {
    let mut translations = Vec::with_capacity(frame_count as usize);
    for frame in 0..frame_count {
        let delta = [
            sample_scalar(channels, W3dAnimationChannelKind::X, frame),
            sample_scalar(channels, W3dAnimationChannelKind::Y, frame),
            sample_scalar(channels, W3dAnimationChannelKind::Z, frame),
        ];
        let rotated = rotate(base_rotation, delta);
        translations.push([
            base_translation[0] + rotated[0],
            base_translation[1] + rotated[1],
            base_translation[2] + rotated[2],
        ]);
    }
    let scales = mask_hidden_attachment_translations(
        &mut translations,
        base_translation,
        hidden_attachment_distance,
    );
    (
        translations.into_iter().flatten().collect::<Vec<_>>(),
        scales,
    )
}

fn hidden_attachment_distance(model: &W3dModel) -> f32 {
    let mut minimum = [f32::INFINITY; 3];
    let mut maximum = [f32::NEG_INFINITY; 3];
    for vertex in model
        .meshes()
        .iter()
        .flat_map(|model_mesh| model_mesh.mesh().vertices())
    {
        for (axis, value) in [vertex.x(), vertex.y(), vertex.z()].into_iter().enumerate() {
            minimum[axis] = minimum[axis].min(value);
            maximum[axis] = maximum[axis].max(value);
        }
    }
    if !minimum[0].is_finite() {
        return HIDDEN_ATTACHMENT_MIN_DISTANCE;
    }
    let extent = [
        maximum[0] - minimum[0],
        maximum[1] - minimum[1],
        maximum[2] - minimum[2],
    ];
    let diagonal = (extent[0] * extent[0] + extent[1] * extent[1] + extent[2] * extent[2]).sqrt();
    HIDDEN_ATTACHMENT_MIN_DISTANCE.max(diagonal * HIDDEN_ATTACHMENT_MODEL_MULTIPLIER)
}

fn mask_hidden_attachment_translations(
    translations: &mut [[f32; 3]],
    base: [f32; 3],
    distance: f32,
) -> Option<Vec<f32>> {
    let distance_squared = distance * distance;
    let hidden = translations
        .iter()
        .map(|translation| {
            let delta = [
                translation[0] - base[0],
                translation[1] - base[1],
                translation[2] - base[2],
            ];
            delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2] > distance_squared
        })
        .collect::<Vec<_>>();
    if !hidden.iter().any(|hidden| *hidden) {
        return None;
    }

    let first_visible = hidden
        .iter()
        .position(|hidden| !hidden)
        .map_or(base, |index| translations[index]);
    let mut last_visible = first_visible;
    let mut scales = Vec::with_capacity(translations.len() * 3);
    for (translation, hidden) in translations.iter_mut().zip(hidden) {
        if hidden {
            *translation = last_visible;
            scales.extend_from_slice(&[HIDDEN_ATTACHMENT_SCALE; 3]);
        } else {
            last_visible = *translation;
            scales.extend_from_slice(&[1.0, 1.0, 1.0]);
        }
    }
    Some(scales)
}

fn sample_scalar(
    channels: &[&W3dAnimationChannel],
    kind: W3dAnimationChannelKind,
    frame: u32,
) -> f32 {
    let Some(channel) = channels.iter().find(|channel| channel.kind() == kind) else {
        return 0.0;
    };
    if frame < channel.first_frame() || frame > channel.last_frame() {
        return 0.0;
    }
    channel.values()[usize::try_from(frame - channel.first_frame()).expect("frame index")]
}

fn sample_quaternion(channels: &[&W3dAnimationChannel], frame: u32) -> [f32; 4] {
    let Some(channel) = channels
        .iter()
        .find(|channel| channel.kind() == W3dAnimationChannelKind::Quaternion)
    else {
        return [0.0, 0.0, 0.0, 1.0];
    };
    if frame < channel.first_frame() || frame > channel.last_frame() {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let offset = usize::try_from(frame - channel.first_frame()).expect("frame index") * 4;
    normalize(
        channel.values()[offset..offset + 4]
            .try_into()
            .expect("decoded quaternion vector"),
    )
}

fn multiply(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}
fn rotate(quaternion: [f32; 4], vector: [f32; 3]) -> [f32; 3] {
    let quaternion = normalize(quaternion);
    let axis = [quaternion[0], quaternion[1], quaternion[2]];
    let scalar = quaternion[3];
    let projection = axis[0] * vector[0] + axis[1] * vector[1] + axis[2] * vector[2];
    let axis_norm = axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2];
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
fn normalize(mut q: [f32; 4]) -> [f32; 4] {
    let length = dot(q, q).sqrt();
    if length > 0.0 {
        for value in &mut q {
            *value /= length;
        }
    } else {
        q = [0.0, 0.0, 0.0, 1.0];
    }
    q
}
fn dot(a: [f32; 4], b: [f32; 4]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]
}
fn channel(value: u8) -> f32 {
    f32::from(value) / 255.0
}
fn mesh_name(mesh: &W3dStaticMesh) -> String {
    let h = mesh.header();
    format!(
        "{}.{}",
        safe_name(h.container_name_bytes()),
        safe_name(h.mesh_name_bytes())
    )
}
fn safe_name(bytes: &[u8]) -> String {
    let bytes = &bytes[..bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len())];
    let base = bytes
        .rsplit(|byte| matches!(byte, b'/' | b'\\'))
        .next()
        .unwrap_or(bytes);
    let mut result = String::new();
    for byte in base {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.') {
            result.push(char::from(*byte));
        } else {
            result.push('_');
        }
    }
    if result.is_empty() {
        "asset".to_owned()
    } else {
        result
    }
}

fn safe_stem(bytes: &[u8]) -> String {
    let name = safe_name(bytes);
    name.rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .map_or(name.clone(), |(stem, _)| stem.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{HIDDEN_ATTACHMENT_SCALE, mask_hidden_attachment_translations};

    #[test]
    fn ordinary_animation_translation_is_unchanged() {
        let mut translations = [[1.0, 2.0, 3.0], [2.0, 2.0, 3.0]];
        assert_eq!(
            mask_hidden_attachment_translations(&mut translations, [1.0, 2.0, 3.0], 100.0),
            None
        );
        assert_eq!(translations, [[1.0, 2.0, 3.0], [2.0, 2.0, 3.0]]);
    }

    #[test]
    fn remote_attachment_translations_become_safe_nonsingular_states() {
        let mut translations = [
            [0.0, 0.0, -1_000.0],
            [4.0, 5.0, 6.0],
            [0.0, 0.0, -2_000.0],
            [7.0, 8.0, 9.0],
        ];
        let scales = mask_hidden_attachment_translations(&mut translations, [1.0, 2.0, 3.0], 100.0)
            .expect("remote samples require a visibility scale channel");

        assert_eq!(
            translations,
            [
                [4.0, 5.0, 6.0],
                [4.0, 5.0, 6.0],
                [4.0, 5.0, 6.0],
                [7.0, 8.0, 9.0],
            ]
        );
        assert_eq!(
            scales,
            [
                HIDDEN_ATTACHMENT_SCALE,
                HIDDEN_ATTACHMENT_SCALE,
                HIDDEN_ATTACHMENT_SCALE,
                1.0,
                1.0,
                1.0,
                HIDDEN_ATTACHMENT_SCALE,
                HIDDEN_ATTACHMENT_SCALE,
                HIDDEN_ATTACHMENT_SCALE,
                1.0,
                1.0,
                1.0
            ]
        );
    }
}
