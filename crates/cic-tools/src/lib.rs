//! Stable diagnostic report formatting.

mod gltf;
pub mod resource;

pub use gltf::{GltfTextureRequest, W3dGlbError, W3dGltfBundle, pack_w3d_glb, render_w3d_gltf};

use std::fmt::Write;

use cic_formats::{
    CsfFile, MapBlendData, MapFile, MapHeightField, MapLightingData, MapWaterData, W3dChunk,
    W3dFile, W3dStaticMesh, W3dVector3, w3d_chunk_name,
};
use cic_render::Capture;
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
            entry.len(),
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

/// Formats a MAP symbol table and top-level chunk stream as a stable inventory.
#[must_use]
pub fn render_map(map: &MapFile) -> String {
    let mut output = format!("compression\t{}\n", map.compression());
    output.push_str("symbol\toffset\tid\tname\n");
    for (index, symbol) in map.symbols().iter().enumerate() {
        writeln!(
            output,
            "{}\t{}\t0x{:08X}\t{}",
            index,
            symbol.offset(),
            symbol.id(),
            escape_bytes(symbol.name_bytes())
        )
        .expect("writing to a String cannot fail");
    }
    output.push_str("chunk\toffset\tid\tversion\tpayload\tname\n");
    for (index, chunk) in map.chunks().iter().enumerate() {
        let name = map
            .symbol_name(chunk.id())
            .map_or_else(|| "unknown".to_owned(), escape_bytes);
        writeln!(
            output,
            "{}\t{}\t0x{:08X}\t{}\t{}\t{}",
            index,
            chunk.offset(),
            chunk.id(),
            chunk.version(),
            chunk.data().len(),
            name
        )
        .expect("writing to a String cannot fail");
    }
    output
}

/// Formats decoded MAP terrain heights in stable row-major order.
///
/// # Panics
///
/// Panics only if a validated MAP width cannot fit the current platform's address size.
#[must_use]
pub fn render_map_height(height: &MapHeightField) -> String {
    let mut output =
        String::from("version\twidth\theight\tborder\tcell_size\tboundaries\tsamples\n");
    writeln!(
        output,
        "{}\t{}\t{}\t{}\t{}\t{}\t{}",
        height.version(),
        height.width(),
        height.height(),
        height.border_size(),
        height.cell_size_world_units(),
        height.boundaries().len(),
        height.samples().len()
    )
    .expect("writing to a String cannot fail");
    output.push_str("boundary\tx\ty\n");
    for (index, boundary) in height.boundaries().iter().enumerate() {
        writeln!(output, "{}\t{}\t{}", index, boundary.x(), boundary.y())
            .expect("writing to a String cannot fail");
    }
    output.push_str("sample\tx\ty\tvalue\n");
    let width = usize::try_from(height.width()).expect("validated MAP width fits usize");
    for (index, sample) in height.samples().iter().enumerate() {
        writeln!(
            output,
            "{}\t{}\t{}\t{}",
            index,
            index % width,
            index / width,
            sample
        )
        .expect("writing to a String cannot fail");
    }
    output
}

/// Encodes MAP height samples as a deterministic 8-bit grayscale PNG in stored row order.
///
/// Height samples are scalar data, so the PNG carries no sRGB or gamma declaration.
///
/// # Errors
///
/// Returns a PNG encoding error if the validated dimensions or sample stream cannot be encoded.
pub fn encode_map_height_png(height: &MapHeightField) -> Result<Vec<u8>, png::EncodingError> {
    let mut output = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut output, height.width(), height.height());
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(height.samples())?;
    }
    Ok(output)
}

/// Encodes a headless renderer capture as an sRGB RGBA8 PNG.
///
/// # Errors
///
/// Returns a PNG encoding error if the validated capture cannot be encoded.
pub fn encode_capture_png(capture: &Capture) -> Result<Vec<u8>, png::EncodingError> {
    let mut output = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut output, capture.width(), capture.height());
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(capture.rgba())?;
    }
    Ok(output)
}

/// Formats decoded MAP blend, edge, and cliff values in stable source order.
///
/// # Panics
///
/// Panics only if validated MAP dimensions cannot fit the current platform's address size.
#[must_use]
pub fn render_map_blend(blend: &MapBlendData) -> String {
    let mut output = String::from(
        "version\twidth\theight\tcells\tbitmap_tiles\tblended_tiles\tcliff_info\ttexture_classes\tedge_tiles\tedge_texture_classes\tcliff_stride\n",
    );
    writeln!(
        output,
        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        blend.version(),
        blend.width(),
        blend.height(),
        blend.tile_indices().len(),
        blend.bitmap_tile_count(),
        blend.blended_tile_count(),
        blend.cliff_info_count(),
        blend.texture_classes().len(),
        blend.edge_tile_count(),
        blend.edge_texture_classes().len(),
        blend.cliff_flag_stride()
    )
    .expect("writing to a String cannot fail");

    render_map_blend_cells(&mut output, blend);
    render_map_texture_classes(&mut output, blend);
    render_map_blend_tiles(&mut output, blend);
    render_map_cliff_info(&mut output, blend);
    output
}

/// Formats water-only polygon data in stable source order.
#[must_use]
pub fn render_map_water(water: &MapWaterData) -> String {
    let point_count = water
        .areas()
        .iter()
        .map(|area| area.points().len())
        .sum::<usize>();
    let mut output = String::from("version\tsource_triggers\twater_areas\twater_points\n");
    writeln!(
        output,
        "{}\t{}\t{}\t{}",
        water.version(),
        water.source_trigger_count(),
        water.areas().len(),
        point_count
    )
    .expect("writing to a String cannot fail");
    output.push_str("area\tsource_index\tid\triver\triver_start\tpoints\tname\n");
    for (index, area) in water.areas().iter().enumerate() {
        writeln!(
            output,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            index,
            area.source_index(),
            area.trigger_id(),
            u8::from(area.is_river()),
            area.river_start(),
            area.points().len(),
            escape_bytes(area.name_bytes())
        )
        .expect("writing to a String cannot fail");
        for (point_index, point) in area.points().iter().enumerate() {
            let [x, y, z] = point.coordinates();
            writeln!(output, "point\t{index}\t{point_index}\t{x}\t{y}\t{z}")
                .expect("writing to a String cannot fail");
        }
    }
    output
}

/// Formats separate terrain/object MAP lights in stable time and source-light order.
#[must_use]
pub fn render_map_lighting(lighting: &MapLightingData) -> String {
    const NAMES: [&str; 4] = ["morning", "afternoon", "evening", "night"];
    let shadow = lighting
        .shadow_color()
        .map_or_else(|| "none".to_owned(), |color| format!("0x{color:08X}"));
    let mut output = String::from("version\tselected_time\tshadow_color\n");
    writeln!(
        output,
        "{}\t{}\t{}",
        lighting.version(),
        lighting.selected_time().name(),
        shadow
    )
    .expect("writing to a String cannot fail");
    output.push_str(
        "period\ttime\tset\tlight\tambient_r\tambient_g\tambient_b\tdiffuse_r\tdiffuse_g\tdiffuse_b\tdirection_x\tdirection_y\tdirection_z\n",
    );
    for (period_index, (period, name)) in lighting.periods().iter().zip(NAMES).enumerate() {
        for (set_name, lights) in [
            ("terrain", period.terrain_lights()),
            ("objects", period.object_lights()),
        ] {
            for (light_index, light) in lights.iter().enumerate() {
                let ambient = light.ambient().map(float_bits);
                let diffuse = light.diffuse().map(float_bits);
                let direction = light.direction().map(float_bits);
                writeln!(
                    output,
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    period_index,
                    name,
                    set_name,
                    light_index,
                    ambient[0],
                    ambient[1],
                    ambient[2],
                    diffuse[0],
                    diffuse[1],
                    diffuse[2],
                    direction[0],
                    direction[1],
                    direction[2]
                )
                .expect("writing to a String cannot fail");
            }
        }
    }
    output
}

fn render_map_blend_cells(output: &mut String, blend: &MapBlendData) {
    output.push_str("cell\tx\ty\ttile\tblend\textra_blend\tcliff_info\tcliff_flag\n");
    let width = usize::try_from(blend.width()).expect("validated MAP width fits usize");
    for index in 0..blend.tile_indices().len() {
        let x = index % width;
        let y = index / width;
        let cliff = blend
            .is_cliff(
                u32::try_from(x).expect("validated X fits u32"),
                u32::try_from(y).expect("validated Y fits u32"),
            )
            .expect("row-major cell is in range");
        writeln!(
            output,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            index,
            x,
            y,
            blend.tile_indices()[index],
            blend.blend_indices()[index],
            blend.extra_blend_indices()[index],
            blend.cliff_info_indices()[index],
            u8::from(cliff)
        )
        .expect("writing to a String cannot fail");
    }
}

fn render_map_texture_classes(output: &mut String, blend: &MapBlendData) {
    output.push_str("texture\tkind\tfirst\tcount\twidth\tlegacy\tname\n");
    for (kind, classes) in [
        ("terrain", blend.texture_classes()),
        ("edge", blend.edge_texture_classes()),
    ] {
        for (index, class) in classes.iter().enumerate() {
            let legacy = class
                .legacy()
                .map_or_else(String::new, |value| value.to_string());
            writeln!(
                output,
                "{}\t{}\t{}\t{}\t{}\t{}\t{}",
                index,
                kind,
                class.first_tile(),
                class.tile_count(),
                class.width(),
                legacy,
                escape_bytes(class.name_bytes())
            )
            .expect("writing to a String cannot fail");
        }
    }
}

fn render_map_blend_tiles(output: &mut String, blend: &MapBlendData) {
    output.push_str(
        "blend\tblend_index\thorizontal\tvertical\tright_diagonal\tleft_diagonal\tinverted\tlong_diagonal\tcustom_edge_class\n",
    );
    for tile in blend.blend_tiles() {
        writeln!(
            output,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            tile.table_index(),
            tile.blend_index(),
            tile.horizontal(),
            tile.vertical(),
            tile.right_diagonal(),
            tile.left_diagonal(),
            tile.inverted(),
            tile.long_diagonal(),
            tile.custom_edge_class()
        )
        .expect("writing to a String cannot fail");
    }
}

fn render_map_cliff_info(output: &mut String, blend: &MapBlendData) {
    output.push_str("cliff\ttile\tu0\tv0\tu1\tv1\tu2\tv2\tu3\tv3\tflip\tmutant\n");
    for cliff in blend.cliff_info() {
        let uv = cliff.uv().map(float_bits);
        writeln!(
            output,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            cliff.table_index(),
            cliff.tile_index(),
            uv[0],
            uv[1],
            uv[2],
            uv[3],
            uv[4],
            uv[5],
            uv[6],
            uv[7],
            cliff.flip(),
            cliff.mutant()
        )
        .expect("writing to a String cannot fail");
    }
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
        CsfLimits, MapLimits, W3dLimits, W3dMeshLimits, decode_map_blend, decode_map_height,
        decode_static_mesh, parse_csf, parse_map, parse_w3d,
    };
    use cic_vfs::{Vfs, VirtualPath};

    use super::{
        encode_map_height_png, render_csf, render_manifest, render_map, render_map_blend,
        render_map_height, render_w3d, render_w3d_mesh,
    };

    fn hex_fixture(hex: &str) -> Vec<u8> {
        let digits = hex
            .bytes()
            .filter(u8::is_ascii_hexdigit)
            .collect::<Vec<_>>();
        digits
            .chunks_exact(2)
            .map(|pair| {
                let pair = std::str::from_utf8(pair).expect("ASCII fixture");
                u8::from_str_radix(pair, 16).expect("valid hex fixture")
            })
            .collect()
    }

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
    fn map_reports_preserve_inventory_and_emit_row_major_heights() {
        let bytes = hex_fixture(include_str!(
            "../../cic-formats/tests/fixtures/minimal.map.hex"
        ));
        let map = parse_map(&bytes, "minimal.map", MapLimits::default()).expect("valid MAP");

        assert_eq!(
            render_map(&map),
            "compression\tnone\n\
             symbol\toffset\tid\tname\n\
             0\t8\t0x00000007\tHeightMapData\n\
             1\t26\t0x00000009\tMystery\n\
             chunk\toffset\tid\tversion\tpayload\tname\n\
             0\t38\t0x00000007\t4\t34\tHeightMapData\n\
             1\t82\t0x00000009\t2\t3\tMystery\n\
             2\t95\t0xFEEDBEEF\t9\t2\tunknown\n"
        );

        let height = decode_map_height(&map, MapLimits::default()).expect("valid heights");
        assert_eq!(
            render_map_height(&height),
            "version\twidth\theight\tborder\tcell_size\tboundaries\tsamples\n\
             4\t3\t2\t0\t10\t1\t6\n\
             boundary\tx\ty\n\
             0\t3\t2\n\
             sample\tx\ty\tvalue\n\
             0\t0\t0\t0\n\
             1\t1\t0\t16\n\
             2\t2\t0\t32\n\
             3\t0\t1\t48\n\
             4\t1\t1\t64\n\
             5\t2\t1\t255\n"
        );

        let png = encode_map_height_png(&height).expect("encode height PNG");
        let image = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
            .expect("decode height PNG")
            .to_luma8();
        assert_eq!(image.dimensions(), (3, 2));
        assert_eq!(image.as_raw(), &[0, 16, 32, 48, 64, 255]);
    }

    #[test]
    fn map_blend_report_is_stable_and_preserves_exact_uv_bits() {
        let bytes = hex_fixture(include_str!(
            "../../cic-formats/tests/fixtures/blend.map.hex"
        ));
        let map = parse_map(&bytes, "blend.map", MapLimits::default()).expect("valid MAP");
        let height = decode_map_height(&map, MapLimits::default()).expect("valid heights");
        let blend = decode_map_blend(&map, &height, MapLimits::default()).expect("valid blend");

        assert_eq!(
            render_map_blend(&blend),
            "version\twidth\theight\tcells\tbitmap_tiles\tblended_tiles\tcliff_info\ttexture_classes\tedge_tiles\tedge_texture_classes\tcliff_stride\n\
             7\t8\t2\t16\t4\t2\t2\t1\t2\t1\t1\n\
             cell\tx\ty\ttile\tblend\textra_blend\tcliff_info\tcliff_flag\n\
             0\t0\t0\t0\t0\t0\t1\t1\n\
             1\t1\t0\t1\t0\t0\t0\t0\n\
             2\t2\t0\t2\t0\t0\t0\t0\n\
             3\t3\t0\t3\t0\t0\t0\t0\n\
             4\t4\t0\t0\t0\t0\t0\t0\n\
             5\t5\t0\t1\t1\t0\t0\t0\n\
             6\t6\t0\t2\t0\t1\t0\t0\n\
             7\t7\t0\t3\t0\t0\t0\t0\n\
             8\t0\t1\t0\t0\t0\t0\t0\n\
             9\t1\t1\t1\t0\t0\t0\t0\n\
             10\t2\t1\t2\t0\t0\t0\t0\n\
             11\t3\t1\t3\t0\t0\t0\t0\n\
             12\t4\t1\t0\t0\t0\t0\t0\n\
             13\t5\t1\t1\t0\t0\t0\t0\n\
             14\t6\t1\t2\t0\t0\t0\t0\n\
             15\t7\t1\t3\t0\t0\t0\t1\n\
             texture\tkind\tfirst\tcount\twidth\tlegacy\tname\n\
             0\tterrain\t0\t4\t2\t0\tBase\n\
             0\tedge\t0\t2\t1\t\tShore\n\
             blend\tblend_index\thorizontal\tvertical\tright_diagonal\tleft_diagonal\tinverted\tlong_diagonal\tcustom_edge_class\n\
             1\t1\t1\t0\t1\t0\t3\t1\t0\n\
             cliff\ttile\tu0\tv0\tu1\tv1\tu2\tv2\tu3\tv3\tflip\tmutant\n\
             1\t3\t0x00000000\t0x00000000\t0x00000000\t0x3F800000\t0x3F800000\t0x3F800000\t0x3F800000\t0x00000000\t1\t0\n"
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
}
