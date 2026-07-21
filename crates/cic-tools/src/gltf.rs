//! Deterministic glTF 2.0 export for composed W3D models.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use cic_formats::{
    W3dAnimationChannel, W3dAnimationChannelKind, W3dMaterialIds, W3dModel, W3dStaticMesh,
};
use serde_json::{Map, Value, json};

/// One source image that the caller must resolve and convert to the named PNG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GltfTextureRequest {
    source_name: Vec<u8>,
    output_name: String,
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
    let mut texture_map = BTreeMap::new();

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
                &mut texture_map,
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
        meshes.push(json!({"name": mesh_name(mesh), "primitives": primitives}));
    }

    let (nodes, mesh_nodes, pivot_nodes) = build_nodes(model);
    let mut nodes = nodes;
    for (model_mesh, (node_index, mesh_index)) in model.meshes().iter().zip(mesh_nodes) {
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

    let has_skin = model
        .meshes()
        .iter()
        .any(|mesh| mesh.mesh().vertex_bones().is_some());
    let skins = if has_skin {
        let matrices = inverse_bind_matrices(model);
        let accessor = buffer.f32_accessor(&matrices, 16, "MAT4", None, false);
        vec![
            json!({"name": "W3D hierarchy", "inverseBindMatrices": accessor, "skeleton": pivot_nodes[0], "joints": pivot_nodes}),
        ]
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
    root.insert("scenes".into(), json!([{"name":"W3D model", "nodes":[0]}]));
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
    texture_map: &mut BTreeMap<(usize, u32), usize>,
    materials: &mut Vec<Value>,
    images: &mut Vec<Value>,
    textures_json: &mut Vec<Value>,
    samplers: &mut Vec<Value>,
    requests: &mut Vec<GltfTextureRequest>,
) -> usize {
    let texture = key.texture.map(|texture_id| {
        *texture_map.entry((mesh_index, texture_id)).or_insert_with(|| {
            let source = &mesh.materials().textures()[usize::try_from(texture_id).expect("decoded texture ID")];
            let output_name = format!(
                "m{mesh_index:03}_t{texture_id:04}_{}.png",
                safe_stem(source.name_bytes())
            );
            let image_index = images.len();
            images.push(json!({"name":String::from_utf8_lossy(source.name_bytes()), "uri":format!("{texture_directory}/{output_name}")}));
            let attributes = source
                .info()
                .map_or(0, cic_formats::W3dTextureInfo::attributes);
            let sampler_index = samplers.len();
            samplers.push(json!({"wrapS":if attributes & 0x8 != 0 {33071} else {10497}, "wrapT":if attributes & 0x10 != 0 {33071} else {10497}}));
            let texture_index = textures_json.len();
            textures_json.push(json!({"source":image_index, "sampler":sampler_index}));
            requests.push(GltfTextureRequest { source_name: source.name_bytes().to_vec(), output_name });
            texture_index
        })
    });
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
    let shader = key
        .shader
        .and_then(|id| usize::try_from(id).ok())
        .and_then(|id| mesh.materials().shaders().get(id))
        .copied();
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
    materials.push(json!({"name":format!("mesh {mesh_index} material {}", material_index), "pbrMetallicRoughness":pbr, "alphaMode":alpha_mode, "alphaCutoff":0.5, "doubleSided":false,
        "extras":{"w3dShader":key.shader}}));
    material_index
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
        let parent = if model_mesh.mesh().vertex_bones().is_some() {
            0
        } else {
            usize::try_from(model_mesh.pivot()).expect("decoded pivot")
        };
        if parent == 0 && model_mesh.mesh().vertex_bones().is_some() {
            // Skinned meshes are siblings of the skeleton beneath the axis-conversion root.
        } else {
            children[parent].push(node_index);
        }
        mesh_nodes.push((node_index, mesh_index));
    }
    let mut nodes = Vec::with_capacity(first_mesh_node + model.meshes().len());
    let mut root_children = vec![1];
    root_children.extend(
        model
            .meshes()
            .iter()
            .enumerate()
            .filter(|(_, mesh)| mesh.mesh().vertex_bones().is_some())
            .map(|(index, _)| first_mesh_node + index),
    );
    nodes.push(json!({"name":"W3D Z-up to glTF Y-up", "rotation":[-0.707_106_77,0.0,0.0,0.707_106_77], "children":root_children}));
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

#[allow(clippy::cast_precision_loss)]
fn build_animations(
    model: &W3dModel,
    pivot_nodes: &[usize],
    buffer: &mut BufferBuilder,
) -> Vec<Value> {
    let mut result = Vec::new();
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
                let mut values = Vec::with_capacity(times.len() * 3);
                for frame in 0..animation.frame_count() {
                    let delta = [
                        sample_scalar(&pivot_channels, W3dAnimationChannelKind::X, frame),
                        sample_scalar(&pivot_channels, W3dAnimationChannelKind::Y, frame),
                        sample_scalar(&pivot_channels, W3dAnimationChannelKind::Z, frame),
                    ];
                    let rotated = rotate(base.rotation().components(), delta);
                    let translation = base.translation();
                    values.extend_from_slice(&[
                        translation[0] + rotated[0],
                        translation[1] + rotated[1],
                        translation[2] + rotated[2],
                    ]);
                }
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
                );
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
                );
            }
        }
        if !channels_json.is_empty() {
            result.push(json!({"name":String::from_utf8_lossy(animation.name_bytes()), "samplers":samplers, "channels":channels_json}));
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
) {
    let output = buffer.f32_accessor(values, components, kind, None, false);
    let sampler = samplers.len();
    samplers.push(json!({"input":input, "output":output, "interpolation":"LINEAR"}));
    channels.push(json!({"sampler":sampler, "target":{"node":node, "path":path}}));
}

fn sample_scalar(
    channels: &[&W3dAnimationChannel],
    kind: W3dAnimationChannelKind,
    frame: u32,
) -> f32 {
    let Some(channel) = channels.iter().find(|channel| channel.kind() == kind) else {
        return 0.0;
    };
    if frame < u32::from(channel.first_frame()) || frame > u32::from(channel.last_frame()) {
        return 0.0;
    }
    channel.values()
        [usize::try_from(frame - u32::from(channel.first_frame())).expect("frame index")]
}

fn sample_quaternion(channels: &[&W3dAnimationChannel], frame: u32) -> [f32; 4] {
    let Some(channel) = channels
        .iter()
        .find(|channel| channel.kind() == W3dAnimationChannelKind::Quaternion)
    else {
        return [0.0, 0.0, 0.0, 1.0];
    };
    if frame < u32::from(channel.first_frame()) || frame > u32::from(channel.last_frame()) {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let offset =
        usize::try_from(frame - u32::from(channel.first_frame())).expect("frame index") * 4;
    normalize(
        channel.values()[offset..offset + 4]
            .try_into()
            .expect("decoded quaternion vector"),
    )
}

fn inverse_bind_matrices(model: &W3dModel) -> Vec<f32> {
    let mut globals = Vec::with_capacity(model.hierarchy().pivots().len());
    for (index, pivot) in model.hierarchy().pivots().iter().enumerate() {
        let local = if index == 0 {
            identity()
        } else {
            matrix(
                pivot.translation(),
                normalize(pivot.rotation().components()),
            )
        };
        let global = pivot.parent().map_or(local, |parent| {
            multiply_matrix(
                globals[usize::try_from(parent).expect("decoded parent")],
                local,
            )
        });
        globals.push(global);
    }
    globals.into_iter().flat_map(invert_rigid).collect()
}

fn identity() -> [f32; 16] {
    [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}
fn matrix(translation: [f32; 3], quaternion: [f32; 4]) -> [f32; 16] {
    let [qx, qy, qz, qw] = quaternion;
    [
        1.0 - 2.0 * (qy * qy + qz * qz),
        2.0 * (qx * qy + qz * qw),
        2.0 * (qx * qz - qy * qw),
        0.0,
        2.0 * (qx * qy - qz * qw),
        1.0 - 2.0 * (qx * qx + qz * qz),
        2.0 * (qy * qz + qx * qw),
        0.0,
        2.0 * (qx * qz + qy * qw),
        2.0 * (qy * qz - qx * qw),
        1.0 - 2.0 * (qx * qx + qy * qy),
        0.0,
        translation[0],
        translation[1],
        translation[2],
        1.0,
    ]
}
fn multiply_matrix(a: [f32; 16], b: [f32; 16]) -> [f32; 16] {
    let mut out = [0.0; 16];
    for column in 0..4 {
        for row in 0..4 {
            out[column * 4 + row] = (0..4).map(|k| a[k * 4 + row] * b[column * 4 + k]).sum();
        }
    }
    out
}
fn invert_rigid(m: [f32; 16]) -> [f32; 16] {
    let mut out = identity();
    for c in 0..3 {
        for r in 0..3 {
            out[c * 4 + r] = m[r * 4 + c];
        }
    }
    let t = [m[12], m[13], m[14]];
    out[12] = -(out[0] * t[0] + out[4] * t[1] + out[8] * t[2]);
    out[13] = -(out[1] * t[0] + out[5] * t[1] + out[9] * t[2]);
    out[14] = -(out[2] * t[0] + out[6] * t[1] + out[10] * t[2]);
    out
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
