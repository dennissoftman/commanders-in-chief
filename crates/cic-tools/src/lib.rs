//! Stable diagnostic report formatting.

use std::fmt::Write;

use cic_formats::{CsfFile, W3dChunk, W3dFile, W3dStaticMesh, W3dVector3, w3d_chunk_name};
use cic_vfs::Vfs;

/// Formats winning VFS entries as deterministic tab-separated records.
#[must_use]
pub fn render_manifest(vfs: &Vfs) -> String {
    let mut output = String::from("path\tbytes\tprovider\n");
    for (path, entry) in vfs.iter_resolved() {
        let provider = entry.provider();
        writeln!(
            output,
            "{}\t{}\t{}:{}",
            path,
            entry.bytes().len(),
            provider.kind(),
            provider.name()
        )
        .expect("writing to a String cannot fail");
    }
    output
}

/// Formats a decoded CSF as a deterministic, lossless tab-separated report.
///
/// Labels are ordered by ASCII case-insensitive name with file order as the tie-breaker.
/// Text controls and raw non-ASCII name bytes are escaped so every record occupies one
/// physical line.
#[must_use]
pub fn render_csf(csf: &CsfFile) -> String {
    let header = csf.header();
    let mut output = String::from("version\tlanguage\tlabels\tstrings\n");
    writeln!(
        output,
        "{}\t{}\t{}\t{}",
        header.version(),
        header.language_id(),
        header.label_count(),
        header.string_count()
    )
    .expect("writing to a String cannot fail");
    output.push_str("label\tvariant\ttext\twave\n");

    let mut labels = csf.labels().iter().enumerate().collect::<Vec<_>>();
    labels.sort_by(|(left_index, left), (right_index, right)| {
        ascii_fold(left.name_bytes())
            .cmp(&ascii_fold(right.name_bytes()))
            .then_with(|| left_index.cmp(right_index))
    });

    for (_, label) in labels {
        let name = escape_bytes(label.name_bytes());
        if label.strings().is_empty() {
            writeln!(output, "{name}\t-\t\t").expect("writing to a String cannot fail");
            continue;
        }
        for (variant, string) in label.strings().iter().enumerate() {
            let text = escape_text(string.text());
            let wave = string
                .wave_name_bytes()
                .map_or_else(String::new, escape_bytes);
            writeln!(output, "{name}\t{variant}\t{text}\t{wave}")
                .expect("writing to a String cannot fail");
        }
    }
    output
}

/// Formats a W3D chunk tree as a stable, depth-first tab-separated inventory.
#[must_use]
pub fn render_w3d(w3d: &W3dFile) -> String {
    let mut output = String::from("path\tdepth\toffset\tid\tkind\tpayload\tname\n");
    let mut path = Vec::new();
    render_w3d_level(&mut output, w3d.chunks(), &mut path);
    output
}

/// Formats immutable static mesh geometry with exact floating-point bit patterns.
#[must_use]
pub fn render_w3d_mesh(mesh: &W3dStaticMesh) -> String {
    let header = mesh.header();
    let mesh_name = escape_bytes(fixed_name(header.mesh_name_bytes()));
    let container_name = escape_bytes(fixed_name(header.container_name_bytes()));
    let mut output = String::from(
        "version\tattributes\tmesh\tcontainer\tvertices\ttriangles\tmaterials\tdamage_stages\tsort_level\tprelit\tvertex_channels\tface_channels\n",
    );
    writeln!(
        output,
        "0x{:08X}\t0x{:08X}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t0x{:08X}\t0x{:08X}\t0x{:08X}",
        header.version(),
        header.attributes(),
        mesh_name,
        container_name,
        header.vertex_count(),
        header.triangle_count(),
        header.material_count(),
        header.damage_stage_count(),
        header.sort_level(),
        header.prelit_version(),
        header.vertex_channels(),
        header.face_channels()
    )
    .expect("writing to a String cannot fail");

    output.push_str("bound\tx\ty\tz\tradius\n");
    render_bound(&mut output, "minimum", header.minimum(), None);
    render_bound(&mut output, "maximum", header.maximum(), None);
    render_bound(
        &mut output,
        "sphere",
        header.sphere_center(),
        Some(header.sphere_radius()),
    );

    output.push_str("vertex\tx\ty\tz\tnx\tny\tnz\n");
    for (index, (vertex, normal)) in mesh.vertices().iter().zip(mesh.normals()).enumerate() {
        writeln!(
            output,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            index,
            float_bits(vertex.x()),
            float_bits(vertex.y()),
            float_bits(vertex.z()),
            float_bits(normal.x()),
            float_bits(normal.y()),
            float_bits(normal.z())
        )
        .expect("writing to a String cannot fail");
    }

    output.push_str("triangle\tv0\tv1\tv2\tattributes\tnx\tny\tnz\tdistance\n");
    for (index, triangle) in mesh.triangles().iter().enumerate() {
        let vertices = triangle.vertex_indices();
        let normal = triangle.normal();
        writeln!(
            output,
            "{}\t{}\t{}\t{}\t0x{:08X}\t{}\t{}\t{}\t{}",
            index,
            vertices[0],
            vertices[1],
            vertices[2],
            triangle.attributes(),
            float_bits(normal.x()),
            float_bits(normal.y()),
            float_bits(normal.z()),
            float_bits(triangle.distance())
        )
        .expect("writing to a String cannot fail");
    }
    output
}

/// Converts immutable static mesh geometry to deterministic Wavefront OBJ text.
///
/// Object-space coordinates, vertex normals, triangle order, and winding are preserved.
/// Resolved first-pass diffuse colors use the widely supported `v x y z r g b` extension.
/// Texture-coordinate records are intentionally omitted until their W3D semantics are
/// implemented.
#[must_use]
pub fn render_w3d_obj(mesh: &W3dStaticMesh) -> String {
    let header = mesh.header();
    let mesh_name = obj_name(fixed_name(header.mesh_name_bytes()));
    let container_name = obj_name(fixed_name(header.container_name_bytes()));
    let object_name = if container_name.is_empty() {
        mesh_name
    } else if mesh_name.is_empty() {
        container_name
    } else {
        format!("{container_name}.{mesh_name}")
    };

    let mut output = String::from(
        "# Commanders in Chief W3D static geometry export\n\
         # Object-space coordinates and winding are preserved; UVs are omitted.\n",
    );
    writeln!(
        output,
        "o {}",
        if object_name.is_empty() {
            "mesh"
        } else {
            &object_name
        }
    )
    .expect("writing to a String cannot fail");

    let colors = mesh.preview_vertex_colors();
    for (index, vertex) in mesh.vertices().iter().enumerate() {
        if let Some(color) = colors.as_ref().and_then(|colors| colors.get(index)) {
            writeln!(
                output,
                "v {} {} {} {} {} {}",
                vertex.x(),
                vertex.y(),
                vertex.z(),
                normalized_color(color.red()),
                normalized_color(color.green()),
                normalized_color(color.blue())
            )
            .expect("writing to a String cannot fail");
        } else {
            writeln!(output, "v {} {} {}", vertex.x(), vertex.y(), vertex.z())
                .expect("writing to a String cannot fail");
        }
    }
    for normal in mesh.normals() {
        writeln!(output, "vn {} {} {}", normal.x(), normal.y(), normal.z())
            .expect("writing to a String cannot fail");
    }
    for triangle in mesh.triangles() {
        let [v0, v1, v2] = triangle.vertex_indices().map(|index| u64::from(index) + 1);
        writeln!(output, "f {v0}//{v0} {v1}//{v1} {v2}//{v2}")
            .expect("writing to a String cannot fail");
    }
    output
}

fn normalized_color(value: u8) -> f32 {
    f32::from(value) / f32::from(u8::MAX)
}

fn obj_name(bytes: &[u8]) -> String {
    let mut name = String::new();
    for byte in bytes {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.' => {
                name.push(char::from(*byte));
            }
            _ => write!(name, "_{byte:02X}").expect("writing to a String cannot fail"),
        }
    }
    name
}

fn render_bound(output: &mut String, name: &str, value: W3dVector3, radius: Option<f32>) {
    let radius = radius.map_or_else(String::new, float_bits);
    writeln!(
        output,
        "{}\t{}\t{}\t{}\t{}",
        name,
        float_bits(value.x()),
        float_bits(value.y()),
        float_bits(value.z()),
        radius
    )
    .expect("writing to a String cannot fail");
}

fn float_bits(value: f32) -> String {
    format!("0x{:08X}", value.to_bits())
}

fn fixed_name(bytes: &[u8; 16]) -> &[u8] {
    let length = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    &bytes[..length]
}

fn render_w3d_level(output: &mut String, chunks: &[W3dChunk], path: &mut Vec<usize>) {
    for (index, chunk) in chunks.iter().enumerate() {
        path.push(index);
        let path_text = path
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join("/");
        let kind = if chunk.is_container() {
            "container"
        } else {
            "data"
        };
        let name = w3d_chunk_name(chunk.id()).unwrap_or("unknown");
        writeln!(
            output,
            "{}\t{}\t{}\t0x{:08X}\t{}\t{}\t{}",
            path_text,
            path.len() - 1,
            chunk.offset(),
            chunk.id(),
            kind,
            chunk.payload_length(),
            name
        )
        .expect("writing to a String cannot fail");
        if let Some(children) = chunk.children() {
            render_w3d_level(output, children, path);
        }
        path.pop();
    }
}

fn ascii_fold(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().map(u8::to_ascii_lowercase).collect()
}

fn escape_bytes(bytes: &[u8]) -> String {
    let mut escaped = String::new();
    for byte in bytes {
        match byte {
            b'\\' => escaped.push_str("\\\\"),
            b'\t' => escaped.push_str("\\t"),
            b'\n' => escaped.push_str("\\n"),
            b'\r' => escaped.push_str("\\r"),
            0x20..=0x7e => escaped.push(char::from(*byte)),
            _ => write!(escaped, "\\x{byte:02X}").expect("writing to a String cannot fail"),
        }
    }
    escaped
}

fn escape_text(text: &str) -> String {
    let mut escaped = String::new();
    for character in text.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '\t' => escaped.push_str("\\t"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            value if value.is_control() => write!(escaped, "\\u{{{:X}}}", u32::from(value))
                .expect("writing to a String cannot fail"),
            value => escaped.push(value),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use cic_formats::{
        CsfLimits, W3dLimits, W3dMeshLimits, decode_static_mesh, parse_csf, parse_w3d,
    };
    use cic_vfs::{Vfs, VirtualPath};

    use super::{render_csf, render_manifest, render_w3d, render_w3d_mesh, render_w3d_obj};

    #[test]
    fn manifest_is_sorted_and_reports_winning_provenance() {
        let mut vfs = Vfs::new();
        vfs.mount_memory(
            "base",
            [
                (
                    VirtualPath::new("z.txt").expect("valid path"),
                    b"z".to_vec(),
                ),
                (
                    VirtualPath::new("a.txt").expect("valid path"),
                    b"old".to_vec(),
                ),
            ],
        )
        .expect("base mount");
        vfs.mount_memory(
            "override",
            [(
                VirtualPath::new("A.TXT").expect("valid path"),
                b"new!".to_vec(),
            )],
        )
        .expect("override mount");

        assert_eq!(
            render_manifest(&vfs),
            "path\tbytes\tprovider\na.txt\t4\tmemory:override\nz.txt\t1\tmemory:base\n"
        );
    }

    #[test]
    fn csf_report_is_sorted_and_includes_zero_string_labels() {
        let hex = include_str!("../../cic-formats/tests/fixtures/minimal.csf.hex");
        let digits = hex
            .bytes()
            .filter(u8::is_ascii_hexdigit)
            .collect::<Vec<_>>();
        let bytes = digits
            .chunks_exact(2)
            .map(|pair| {
                let pair = std::str::from_utf8(pair).expect("ASCII fixture");
                u8::from_str_radix(pair, 16).expect("valid hex fixture")
            })
            .collect::<Vec<_>>();
        let csf = parse_csf(&bytes, "minimal.csf", CsfLimits::default()).expect("valid CSF");

        assert_eq!(
            render_csf(&csf),
            "version\tlanguage\tlabels\tstrings\n\
             3\t0\t3\t2\n\
             label\tvariant\ttext\twave\n\
             GUI:HELLO\t0\tHello\t\n\
             SPEECH:READY\t0\tReady\tready.wav\n\
             TOOLTIP:EMPTY\t-\t\t\n"
        );
    }

    #[test]
    fn w3d_report_uses_stable_slash_separated_tree_paths() {
        let hex = include_str!("../../cic-formats/tests/fixtures/minimal.w3d.hex");
        let digits = hex
            .bytes()
            .filter(u8::is_ascii_hexdigit)
            .collect::<Vec<_>>();
        let bytes = digits
            .chunks_exact(2)
            .map(|pair| {
                let pair = std::str::from_utf8(pair).expect("ASCII fixture");
                u8::from_str_radix(pair, 16).expect("valid hex fixture")
            })
            .collect::<Vec<_>>();
        let w3d = parse_w3d(&bytes, "minimal.w3d", W3dLimits::default()).expect("valid W3D");

        assert_eq!(
            render_w3d(&w3d),
            "path\tdepth\toffset\tid\tkind\tpayload\tname\n\
             0\t0\t0\t0x00000000\tcontainer\t29\tW3D_CHUNK_MESH\n\
             0/0\t1\t8\t0x11111111\tdata\t3\tunknown\n\
             0/1\t1\t19\t0x22222222\tcontainer\t10\tunknown\n\
             0/1/0\t2\t27\t0x33333333\tdata\t2\tunknown\n\
             1\t0\t37\t0xDEADBEEF\tdata\t4\tunknown\n"
        );
    }

    #[test]
    fn static_mesh_report_preserves_exact_geometry_bits() {
        let hex = include_str!("../../cic-formats/tests/fixtures/static-mesh.w3d.hex");
        let digits = hex
            .bytes()
            .filter(u8::is_ascii_hexdigit)
            .collect::<Vec<_>>();
        let bytes = digits
            .chunks_exact(2)
            .map(|pair| {
                let pair = std::str::from_utf8(pair).expect("ASCII fixture");
                u8::from_str_radix(pair, 16).expect("valid hex fixture")
            })
            .collect::<Vec<_>>();
        let w3d = parse_w3d(&bytes, "static-mesh.w3d", W3dLimits::default()).expect("valid W3D");
        let mesh = decode_static_mesh(&w3d.chunks()[0], W3dMeshLimits::default())
            .expect("valid static mesh");
        let report = render_w3d_mesh(&mesh);

        assert!(report.starts_with(
            "version\tattributes\tmesh\tcontainer\tvertices\ttriangles\tmaterials\tdamage_stages\tsort_level\tprelit\tvertex_channels\tface_channels\n\
             0x00040002\t0x00000000\tTri\tTest\t3\t1\t0\t0\t0\t0x00000000\t0x00000001\t0x00000001\n"
        ));
        assert!(report.contains(
            "vertex\tx\ty\tz\tnx\tny\tnz\n\
             0\t0x00000000\t0x00000000\t0x00000000\t0x00000000\t0x00000000\t0x3F800000\n"
        ));
        assert!(report.ends_with(
            "triangle\tv0\tv1\tv2\tattributes\tnx\tny\tnz\tdistance\n\
             0\t0\t1\t2\t0x00000000\t0x00000000\t0x00000000\t0x3F800000\t0x00000000\n"
        ));
    }

    #[test]
    fn static_mesh_obj_preserves_coordinates_normals_and_winding() {
        let hex = include_str!("../../cic-formats/tests/fixtures/static-mesh.w3d.hex");
        let digits = hex
            .bytes()
            .filter(u8::is_ascii_hexdigit)
            .collect::<Vec<_>>();
        let bytes = digits
            .chunks_exact(2)
            .map(|pair| {
                let pair = std::str::from_utf8(pair).expect("ASCII fixture");
                u8::from_str_radix(pair, 16).expect("valid hex fixture")
            })
            .collect::<Vec<_>>();
        let w3d = parse_w3d(&bytes, "static-mesh.w3d", W3dLimits::default()).expect("valid W3D");
        let mesh = decode_static_mesh(&w3d.chunks()[0], W3dMeshLimits::default())
            .expect("valid static mesh");

        assert_eq!(
            render_w3d_obj(&mesh),
            "# Commanders in Chief W3D static geometry export\n\
             # Object-space coordinates and winding are preserved; UVs are omitted.\n\
             o Test.Tri\n\
             v 0 0 0\n\
             v 1 0 0\n\
             v 0 1 0\n\
             vn 0 0 1\n\
             vn 0 0 1\n\
             vn 0 0 1\n\
             f 1//1 2//2 3//3\n"
        );
    }

    #[test]
    fn static_mesh_obj_emits_resolved_diffuse_vertex_colors() {
        let hex = include_str!("../../cic-formats/tests/fixtures/colored-mesh.w3d.hex");
        let digits = hex
            .bytes()
            .filter(u8::is_ascii_hexdigit)
            .collect::<Vec<_>>();
        let bytes = digits
            .chunks_exact(2)
            .map(|pair| {
                let pair = std::str::from_utf8(pair).expect("ASCII fixture");
                u8::from_str_radix(pair, 16).expect("valid hex fixture")
            })
            .collect::<Vec<_>>();
        let w3d = parse_w3d(&bytes, "colored-mesh.w3d", W3dLimits::default()).expect("valid W3D");
        let mesh = decode_static_mesh(&w3d.chunks()[0], W3dMeshLimits::default())
            .expect("valid colored mesh");
        let obj = render_w3d_obj(&mesh);

        assert!(obj.contains("v 0 0 0 1 0 0\nv 1 0 0 1 0 0\nv 0 1 0 1 0 0\n"));
    }
}
