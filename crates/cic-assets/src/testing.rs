//! Asset fixture builders for unit tests.
//!
//! Models are built as real `.glb` binaries rather than checked-in blobs. GLB is used specifically
//! because its buffers live in a binary chunk, so a fixture needs no base64 encoder and no external
//! `.bin` sidecar — which also means the fixtures exercise the same single-file path production
//! assets take.

// Fixture sizes are small, known constants, so width casts cannot truncate. `TriangleOptions` is
// deliberately a bag of independent flags rather than an enum: the cases it selects are orthogonal
// and combinable, which is exactly what `struct_excessive_bools` exists to discourage in production
// types and exactly what a fixture builder wants.
#![allow(
    clippy::cast_possible_truncation,
    clippy::struct_excessive_bools,
    clippy::float_cmp
)]

use serde_json::{Value, json};

const GLB_MAGIC: u32 = 0x4654_6C67;
const CHUNK_JSON: u32 = 0x4E4F_534A;
const CHUNK_BIN: u32 = 0x004E_4942;

/// Wraps a glTF JSON document and its binary buffer into a `.glb` container.
pub(crate) fn glb(document: &Value, binary: &[u8]) -> Vec<u8> {
    let mut json = serde_json::to_vec(document).expect("serialize glTF fixture");
    // The spec pads the JSON chunk with spaces and the binary chunk with zeros.
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut bin = binary.to_vec();
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }

    let total = 12 + 8 + json.len() + if bin.is_empty() { 0 } else { 8 + bin.len() };
    let mut output = Vec::with_capacity(total);
    output.extend_from_slice(&GLB_MAGIC.to_le_bytes());
    output.extend_from_slice(&2u32.to_le_bytes());
    output.extend_from_slice(&(total as u32).to_le_bytes());

    output.extend_from_slice(&(json.len() as u32).to_le_bytes());
    output.extend_from_slice(&CHUNK_JSON.to_le_bytes());
    output.extend_from_slice(&json);

    if !bin.is_empty() {
        output.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        output.extend_from_slice(&CHUNK_BIN.to_le_bytes());
        output.extend_from_slice(&bin);
    }
    output
}

/// How a triangle fixture should differ from the straightforward case.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TriangleOptions {
    /// Omit the `NORMAL` attribute, so the importer's flat-normal fallback runs.
    pub(crate) without_normals: bool,
    /// Omit the `TEXCOORD_0` attribute.
    pub(crate) without_uvs: bool,
    /// Omit the index accessor, making the primitive an unindexed triangle sequence.
    pub(crate) without_indices: bool,
    /// Declare a topology other than triangles.
    pub(crate) line_topology: bool,
    /// Write an index that points past the vertex array.
    pub(crate) out_of_range_index: bool,
    /// Wrap the mesh node in a parent that translates by `(10, 20, 30)` and scales by 2.
    pub(crate) nested_transform: bool,
}

/// Builds a one-triangle `.glb`.
///
/// The triangle is the unit right triangle in the XY plane with `+Z` normals, so a transform applied
/// by the importer is easy to assert against by inspection.
pub(crate) fn triangle_glb(options: TriangleOptions) -> Vec<u8> {
    let positions: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let normals: [[f32; 3]; 3] = [[0.0, 0.0, 1.0]; 3];
    let uvs: [[f32; 2]; 3] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
    let indices: [u32; 3] = if options.out_of_range_index {
        [0, 1, 99]
    } else {
        [0, 1, 2]
    };

    let mut binary = Vec::new();
    let positions_offset = binary.len();
    for position in positions {
        for value in position {
            binary.extend_from_slice(&value.to_le_bytes());
        }
    }
    let normals_offset = binary.len();
    for normal in normals {
        for value in normal {
            binary.extend_from_slice(&value.to_le_bytes());
        }
    }
    let uvs_offset = binary.len();
    for uv in uvs {
        for value in uv {
            binary.extend_from_slice(&value.to_le_bytes());
        }
    }
    let indices_offset = binary.len();
    for index in indices {
        binary.extend_from_slice(&index.to_le_bytes());
    }

    let mut views = vec![
        json!({ "buffer": 0, "byteOffset": positions_offset, "byteLength": 36 }),
        json!({ "buffer": 0, "byteOffset": normals_offset, "byteLength": 36 }),
        json!({ "buffer": 0, "byteOffset": uvs_offset, "byteLength": 24 }),
        json!({ "buffer": 0, "byteOffset": indices_offset, "byteLength": 12 }),
    ];
    views.truncate(4);

    // FLOAT is 5126 and UNSIGNED_INT is 5125. POSITION requires min/max per the spec.
    let accessors = json!([
        {
            "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
            "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0]
        },
        { "bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3" },
        { "bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC2" },
        { "bufferView": 3, "componentType": 5125, "count": 3, "type": "SCALAR" },
    ]);

    let mut attributes = json!({ "POSITION": 0 });
    if !options.without_normals {
        attributes["NORMAL"] = json!(1);
    }
    if !options.without_uvs {
        attributes["TEXCOORD_0"] = json!(2);
    }

    let mut primitive = json!({ "attributes": attributes, "material": 0 });
    if !options.without_indices {
        primitive["indices"] = json!(3);
    }
    if options.line_topology {
        // Mode 1 is LINES.
        primitive["mode"] = json!(1);
    }

    let (nodes, scene_nodes) = if options.nested_transform {
        (
            json!([
                { "mesh": 0, "name": "leaf" },
                {
                    "name": "parent",
                    "children": [0],
                    // Column-major: scale 2 with translation (10, 20, 30).
                    "matrix": [
                        2.0, 0.0, 0.0, 0.0,
                        0.0, 2.0, 0.0, 0.0,
                        0.0, 0.0, 2.0, 0.0,
                        10.0, 20.0, 30.0, 1.0
                    ]
                }
            ]),
            json!([1]),
        )
    } else {
        (json!([{ "mesh": 0, "name": "leaf" }]), json!([0]))
    };

    let document = json!({
        "asset": { "version": "2.0" },
        "scene": 0,
        "scenes": [ { "name": "fixture", "nodes": scene_nodes } ],
        "nodes": nodes,
        "meshes": [ { "primitives": [ primitive ] } ],
        "materials": [ {
            "name": "stone",
            "pbrMetallicRoughness": {
                "baseColorFactor": [0.8, 0.7, 0.6, 1.0],
                "metallicFactor": 0.0,
                "roughnessFactor": 0.9
            }
        } ],
        "accessors": accessors,
        "bufferViews": views,
        "buffers": [ { "byteLength": binary.len() } ],
    });

    glb(&document, &binary)
}
