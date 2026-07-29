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

/// Builds a one-triangle `.glb` whose material carries an embedded base-colour texture.
///
/// The image is a real PNG in the binary chunk, referenced through a buffer view — the form an
/// exporter actually produces for a self-contained `.glb`. A fixture that skipped the encoding and
/// handed over raw pixels would not exercise the decode path at all.
///
/// The four texels are distinct primary-ish colours in a known order, so a test can tell a correct
/// decode from a channel swap or a flipped row, which a uniform image cannot.
pub(crate) fn textured_triangle_glb() -> Vec<u8> {
    let positions: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let uvs: [[f32; 2]; 3] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];

    let mut binary = Vec::new();
    for position in positions {
        for value in position {
            binary.extend_from_slice(&value.to_le_bytes());
        }
    }
    let uvs_offset = binary.len();
    for uv in uvs {
        for value in uv {
            binary.extend_from_slice(&value.to_le_bytes());
        }
    }
    // The image view must start on a four-byte boundary like every other one.
    while !binary.len().is_multiple_of(4) {
        binary.push(0);
    }
    let image_offset = binary.len();
    let image = png_rgba(2, 2, TEXEL_COLOURS.as_flattened());
    let image_length = image.len();
    binary.extend_from_slice(&image);

    let document = json!({
        "asset": { "version": "2.0" },
        "scene": 0,
        "scenes": [ { "name": "textured", "nodes": [0] } ],
        "nodes": [ { "mesh": 0 } ],
        "meshes": [ { "primitives": [
            { "attributes": { "POSITION": 0, "TEXCOORD_0": 1 }, "material": 0 }
        ] } ],
        "materials": [ {
            "name": "painted",
            "pbrMetallicRoughness": {
                "baseColorFactor": [1.0, 1.0, 1.0, 1.0],
                "baseColorTexture": { "index": 0 }
            }
        } ],
        "textures": [ { "source": 0 } ],
        "images": [ { "bufferView": 2, "mimeType": "image/png" } ],
        "accessors": [
            {
                "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0]
            },
            { "bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC2" },
        ],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0, "byteLength": 36 },
            { "buffer": 0, "byteOffset": uvs_offset, "byteLength": 24 },
            { "buffer": 0, "byteOffset": image_offset, "byteLength": image_length },
        ],
        "buffers": [ { "byteLength": binary.len() } ],
    });

    glb(&document, &binary)
}

/// The four texels of the fixture image, row-major from the top-left, as opaque RGBA.
pub(crate) const TEXEL_COLOURS: [[u8; 4]; 4] = [
    [255, 0, 0, 255],
    [0, 255, 0, 255],
    [0, 0, 255, 255],
    [255, 255, 0, 255],
];

/// Builds a one-triangle `.glb` carrying a full PBR map set: base colour, a normal map, and a
/// metallic-roughness map that doubles as the occlusion map.
///
/// Three distinct images rather than one repeated, because the failure worth catching is a slot
/// resolved to the wrong index — every map is an index into one shared image list, and a shift by one
/// puts a roughness map where a normal map belongs with nothing reporting it.
///
/// `TANGENT` is deliberately absent. That is what an exporter usually produces, and it is the case the
/// importer's derivation exists for.
pub(crate) fn normal_mapped_triangle_glb() -> Vec<u8> {
    // UVs increase with +X and +Y, so the derived tangent must come out along +X. A transposed 2x2
    // solve would put it along +Y instead, which rotates every normal map a quarter turn.
    let positions: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let normals: [[f32; 3]; 3] = [[0.0, 0.0, 1.0]; 3];
    let uvs: [[f32; 2]; 3] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];

    let mut binary = Vec::new();
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

    // A flat tangent-space normal is (0.5, 0.5, 1.0) once encoded, and a roughness-metallic texel puts
    // roughness in green and metallic in blue. Both are data rather than colour, which is why the
    // renderer uploads them without an sRGB decode.
    let mut views = vec![
        json!({ "buffer": 0, "byteOffset": 0, "byteLength": 36 }),
        json!({ "buffer": 0, "byteOffset": normals_offset, "byteLength": 36 }),
        json!({ "buffer": 0, "byteOffset": uvs_offset, "byteLength": 24 }),
    ];
    for image in [
        png_rgba(1, 1, &[200, 180, 160, 255]),
        png_rgba(1, 1, &[128, 128, 255, 255]),
        png_rgba(1, 1, &[255, 96, 32, 255]),
    ] {
        while !binary.len().is_multiple_of(4) {
            binary.push(0);
        }
        let offset = binary.len();
        let length = image.len();
        binary.extend_from_slice(&image);
        views.push(json!({ "buffer": 0, "byteOffset": offset, "byteLength": length }));
    }

    let document = json!({
        "asset": { "version": "2.0" },
        "scene": 0,
        "scenes": [ { "name": "mapped", "nodes": [0] } ],
        "nodes": [ { "mesh": 0 } ],
        "meshes": [ { "primitives": [
            { "attributes": { "POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2 }, "material": 0 }
        ] } ],
        "materials": [ {
            "name": "plated",
            "pbrMetallicRoughness": {
                "baseColorTexture": { "index": 0 },
                "metallicRoughnessTexture": { "index": 2 },
                "metallicFactor": 1.0,
                "roughnessFactor": 1.0
            },
            "normalTexture": { "index": 1, "scale": 0.75 },
            "occlusionTexture": { "index": 2, "strength": 0.5 }
        } ],
        "textures": [ { "source": 0 }, { "source": 1 }, { "source": 2 } ],
        "images": [
            { "bufferView": 3, "mimeType": "image/png" },
            { "bufferView": 4, "mimeType": "image/png" },
            { "bufferView": 5, "mimeType": "image/png" },
        ],
        "accessors": [
            {
                "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0]
            },
            { "bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3" },
            { "bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC2" },
        ],
        "bufferViews": views,
        "buffers": [ { "byteLength": binary.len() } ],
    });

    glb(&document, &binary)
}

/// Builds a one-triangle `.glb` with a masked, double-sided material — the way foliage is authored.
pub(crate) fn foliage_triangle_glb() -> Vec<u8> {
    let positions: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let mut binary = Vec::new();
    for position in positions {
        for value in position {
            binary.extend_from_slice(&value.to_le_bytes());
        }
    }

    let document = json!({
        "asset": { "version": "2.0" },
        "scene": 0,
        "scenes": [ { "name": "foliage", "nodes": [0] } ],
        "nodes": [ { "mesh": 0 } ],
        "meshes": [ { "primitives": [
            { "attributes": { "POSITION": 0 }, "material": 0 }
        ] } ],
        // A cutoff other than the default, so the test distinguishes reading it from assuming it.
        "materials": [ {
            "name": "leaf",
            "alphaMode": "MASK",
            "alphaCutoff": 0.4,
            "doubleSided": true,
            "pbrMetallicRoughness": { "baseColorFactor": [0.35, 0.55, 0.25, 1.0] }
        } ],
        "accessors": [ {
            "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
            "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0]
        } ],
        "bufferViews": [ { "buffer": 0, "byteOffset": 0, "byteLength": 36 } ],
        "buffers": [ { "byteLength": binary.len() } ],
    });

    glb(&document, &binary)
}

/// Encodes RGBA bytes as a PNG.
fn png_rgba(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut output, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("write png header");
        writer.write_image_data(rgba).expect("write png data");
        writer.finish().expect("finish png");
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
