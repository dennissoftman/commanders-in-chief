use std::fs;
use std::process::Command;

use cic_formats::{W3dChunk, W3dLimits, parse_w3d};
use serde_json::json;

#[test]
fn directory_and_big_archive_produce_a_stable_overlay_manifest() {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("manifest-cli");
    if root.exists() {
        fs::remove_dir_all(&root).expect("remove stale test tree");
    }
    let base = root.join("base");
    let archive_path = root.join("overlay.big");
    fs::create_dir_all(base.join("Data")).expect("create base tree");
    fs::write(base.join("Data").join("Z.TXT"), b"z").expect("write base resource");
    fs::write(base.join("Data").join("A.txt"), b"old").expect("write base resource");
    fs::write(&archive_path, big_fixture()).expect("write BIG fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_cic-inspect"))
        .arg("manifest")
        .arg(&base)
        .arg(&archive_path)
        .output()
        .expect("run cic-inspect");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("UTF-8 output"),
        "path\tbytes\tprovider\ndata/a.txt\t4\tbig:mount-1\ndata/z.bin\t3\tbig:mount-1\ndata/z.txt\t1\tdirectory:mount-0\n"
    );

    fs::remove_dir_all(root).expect("remove test tree");
}

#[test]
fn custom_profile_and_mod_layer_do_not_require_retail_archive_names() {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("custom-profile-cli");
    if root.exists() {
        fs::remove_dir_all(&root).expect("remove stale custom-profile tree");
    }
    let archive_path = root.join("total-conversion.assets");
    let mod_root = root.join("my-mod");
    fs::create_dir_all(mod_root.join("data")).expect("create mod tree");
    fs::write(&archive_path, big_fixture()).expect("write custom base archive");
    fs::write(mod_root.join("data/a.txt"), b"modded").expect("write mod override");
    let profile_path = root.join("custom.cic-profile");
    fs::write(
        &profile_path,
        "version=1\nmount=total-conversion.assets\noptional=missing-extras\n",
    )
    .expect("write custom profile");

    let output = Command::new(env!("CARGO_BIN_EXE_cic-inspect"))
        .arg("--profile")
        .arg(&profile_path)
        .arg("--mod")
        .arg(&mod_root)
        .arg("manifest")
        .output()
        .expect("run custom profile manifest");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("UTF-8 output"),
        "path\tbytes\tprovider\ndata/a.txt\t6\tdirectory:mount-1\ndata/z.bin\t3\tbig:mount-0\n"
    );
    fs::remove_dir_all(root).expect("remove custom-profile tree");
}

fn big_fixture() -> Vec<u8> {
    let hex = include_str!("../../cic-vfs/tests/fixtures/minimal.big.hex");
    let digits = hex
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    digits
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).expect("ASCII hex");
            u8::from_str_radix(pair, 16).expect("valid hex fixture")
        })
        .collect()
}

#[test]
fn csf_inside_big_produces_a_stable_localization_report() {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("csf-cli");
    if root.exists() {
        fs::remove_dir_all(&root).expect("remove stale test tree");
    }
    fs::create_dir_all(&root).expect("create test tree");
    let archive_path = root.join("localization.big");
    let csf = csf_fixture();
    fs::write(
        &archive_path,
        big_with_entry(r"Data\English\minimal.csf", &csf),
    )
    .expect("write synthetic archive");

    let output = Command::new(env!("CARGO_BIN_EXE_cic-inspect"))
        .arg("csf")
        .arg(r"DATA\ENGLISH\MINIMAL.CSF")
        .arg(&archive_path)
        .output()
        .expect("run cic-inspect");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("UTF-8 output"),
        "version\tlanguage\tlabels\tstrings\n\
         3\t0\t3\t2\n\
         label\tvariant\ttext\twave\n\
         GUI:HELLO\t0\tHello\t\n\
         SPEECH:READY\t0\tReady\tready.wav\n\
         TOOLTIP:EMPTY\t-\t\t\n"
    );

    fs::remove_dir_all(root).expect("remove test tree");
}

fn csf_fixture() -> Vec<u8> {
    let hex = include_str!("../../cic-formats/tests/fixtures/minimal.csf.hex");
    let digits = hex
        .bytes()
        .filter(u8::is_ascii_hexdigit)
        .collect::<Vec<_>>();
    digits
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).expect("ASCII hex");
            u8::from_str_radix(pair, 16).expect("valid hex fixture")
        })
        .collect()
}

fn big_with_entry(name: &str, bytes: &[u8]) -> Vec<u8> {
    let data_start = 16 + 8 + name.len() + 1;
    let archive_size = data_start + bytes.len();
    let mut archive = Vec::with_capacity(archive_size);
    archive.extend_from_slice(b"BIGF");
    archive.extend_from_slice(
        &u32::try_from(archive_size)
            .expect("fixture size fits u32")
            .to_le_bytes(),
    );
    archive.extend_from_slice(&1_u32.to_be_bytes());
    archive.extend_from_slice(
        &u32::try_from(data_start)
            .expect("fixture offset fits u32")
            .to_be_bytes(),
    );
    archive.extend_from_slice(
        &u32::try_from(data_start)
            .expect("fixture offset fits u32")
            .to_be_bytes(),
    );
    archive.extend_from_slice(
        &u32::try_from(bytes.len())
            .expect("fixture length fits u32")
            .to_be_bytes(),
    );
    archive.extend_from_slice(name.as_bytes());
    archive.push(0);
    archive.extend_from_slice(bytes);
    archive
}

fn big_with_entries(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let table_bytes = entries
        .iter()
        .map(|(name, _)| 8 + name.len() + 1)
        .sum::<usize>();
    let data_start = 16 + table_bytes;
    let archive_size = data_start + entries.iter().map(|(_, bytes)| bytes.len()).sum::<usize>();
    let mut archive = Vec::with_capacity(archive_size);
    archive.extend_from_slice(b"BIGF");
    archive.extend_from_slice(
        &u32::try_from(archive_size)
            .expect("fixture size fits u32")
            .to_le_bytes(),
    );
    archive.extend_from_slice(
        &u32::try_from(entries.len())
            .expect("fixture entry count fits u32")
            .to_be_bytes(),
    );
    archive.extend_from_slice(
        &u32::try_from(data_start)
            .expect("fixture table fits u32")
            .to_be_bytes(),
    );
    let mut offset = data_start;
    for (name, bytes) in entries {
        archive.extend_from_slice(
            &u32::try_from(offset)
                .expect("fixture offset fits u32")
                .to_be_bytes(),
        );
        archive.extend_from_slice(
            &u32::try_from(bytes.len())
                .expect("fixture length fits u32")
                .to_be_bytes(),
        );
        archive.extend_from_slice(name.as_bytes());
        archive.push(0);
        offset += bytes.len();
    }
    for (_, bytes) in entries {
        archive.extend_from_slice(bytes);
    }
    archive
}

#[test]
fn w3d_inside_big_produces_a_stable_nested_inventory() {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("w3d-cli");
    if root.exists() {
        fs::remove_dir_all(&root).expect("remove stale test tree");
    }
    fs::create_dir_all(&root).expect("create test tree");
    let archive_path = root.join("art.big");
    let w3d = w3d_fixture();
    fs::write(&archive_path, big_with_entry(r"Art\W3D\minimal.w3d", &w3d))
        .expect("write synthetic archive");

    let output = Command::new(env!("CARGO_BIN_EXE_cic-inspect"))
        .arg("w3d")
        .arg("ART/W3D/MINIMAL.W3D")
        .arg(&archive_path)
        .output()
        .expect("run cic-inspect");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("UTF-8 output"),
        "path\tdepth\toffset\tid\tkind\tpayload\tname\n\
         0\t0\t0\t0x00000000\tcontainer\t29\tW3D_CHUNK_MESH\n\
         0/0\t1\t8\t0x11111111\tdata\t3\tunknown\n\
         0/1\t1\t19\t0x22222222\tcontainer\t10\tunknown\n\
         0/1/0\t2\t27\t0x33333333\tdata\t2\tunknown\n\
         1\t0\t37\t0xDEADBEEF\tdata\t4\tunknown\n"
    );

    fs::remove_dir_all(root).expect("remove test tree");
}

#[test]
fn map_inside_big_produces_stable_inventory_and_height_reports() {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("map-cli");
    if root.exists() {
        fs::remove_dir_all(&root).expect("remove stale test tree");
    }
    fs::create_dir_all(&root).expect("create test tree");
    let archive_path = root.join("maps.big");
    let map = map_fixture();
    fs::write(
        &archive_path,
        big_with_entry(r"Maps\Synthetic Valley\synthetic valley.map", &map),
    )
    .expect("write synthetic archive");

    let inventory = Command::new(env!("CARGO_BIN_EXE_cic-inspect"))
        .arg("map")
        .arg("MAPS/SYNTHETIC VALLEY/SYNTHETIC VALLEY.MAP")
        .arg(&archive_path)
        .output()
        .expect("run MAP inventory");
    assert!(inventory.status.success());
    assert_eq!(
        String::from_utf8(inventory.stdout).expect("UTF-8 inventory"),
        "compression\tnone\n\
         symbol\toffset\tid\tname\n\
         0\t8\t0x00000007\tHeightMapData\n\
         1\t26\t0x00000009\tMystery\n\
         chunk\toffset\tid\tversion\tpayload\tname\n\
         0\t38\t0x00000007\t4\t34\tHeightMapData\n\
         1\t82\t0x00000009\t2\t3\tMystery\n\
         2\t95\t0xFEEDBEEF\t9\t2\tunknown\n"
    );

    let heights = Command::new(env!("CARGO_BIN_EXE_cic-inspect"))
        .arg("map-height")
        .arg("--report")
        .arg("maps/synthetic valley/synthetic valley.map")
        .arg(&archive_path)
        .output()
        .expect("run MAP height report");
    assert!(heights.status.success());
    assert_eq!(
        String::from_utf8(heights.stdout).expect("UTF-8 height report"),
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

    let png_path = root.join("height.png");
    let png = Command::new(env!("CARGO_BIN_EXE_cic-inspect"))
        .arg("map-height")
        .arg("--png")
        .arg(&png_path)
        .arg("maps/synthetic valley/synthetic valley.map")
        .arg(&archive_path)
        .output()
        .expect("run MAP height PNG export");
    assert!(png.status.success());
    let image = image::open(&png_path).expect("open height PNG").to_luma8();
    assert_eq!(image.dimensions(), (3, 2));
    assert_eq!(image.as_raw(), &[0, 16, 32, 48, 64, 255]);

    let default_png = Command::new(env!("CARGO_BIN_EXE_cic-inspect"))
        .current_dir(&root)
        .arg("map-height")
        .arg("maps/synthetic valley/synthetic valley.map")
        .arg(&archive_path)
        .output()
        .expect("run default MAP height PNG export");
    assert!(default_png.status.success());
    assert!(root.join("synthetic valley.png").is_file());

    fs::remove_dir_all(root).expect("remove test tree");
}

#[test]
fn map_blend_inside_big_produces_stable_semantic_report() {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("map-blend-cli");
    if root.exists() {
        fs::remove_dir_all(&root).expect("remove stale test tree");
    }
    fs::create_dir_all(&root).expect("create test tree");
    let archive_path = root.join("maps.big");
    fs::write(
        &archive_path,
        big_with_entry(r"Maps\Synthetic\blend.map", &map_blend_fixture()),
    )
    .expect("write synthetic archive");

    let output = Command::new(env!("CARGO_BIN_EXE_cic-inspect"))
        .arg("map-blend")
        .arg("maps/synthetic/blend.map")
        .arg(&archive_path)
        .output()
        .expect("run MAP blend report");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 blend report");
    assert!(stdout.starts_with(
        "version\twidth\theight\tcells\tbitmap_tiles\tblended_tiles\tcliff_info\ttexture_classes\tedge_tiles\tedge_texture_classes\tcliff_stride\n\
         7\t8\t2\t16\t4\t2\t2\t1\t2\t1\t1\n"
    ));
    assert!(stdout.contains("0\tterrain\t0\t4\t2\t0\tBase\n"));
    assert!(stdout.contains("0\tedge\t0\t2\t1\t\tShore\n"));
    assert!(stdout.ends_with(
        "1\t3\t0x00000000\t0x00000000\t0x00000000\t0x3F800000\t0x3F800000\t0x3F800000\t0x3F800000\t0x00000000\t1\t0\n"
    ));

    fs::remove_dir_all(root).expect("remove test tree");
}

#[test]
fn map_render_inside_big_writes_a_textured_png() {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("map-render-cli");
    if root.exists() {
        fs::remove_dir_all(&root).expect("remove stale test tree");
    }
    let texture_dir = root.join("art/terrain");
    let ini_dir = root.join("data/ini");
    fs::create_dir_all(&texture_dir).expect("create texture tree");
    fs::create_dir_all(&ini_dir).expect("create INI tree");
    let archive_path = root.join("maps.big");
    fs::write(
        &archive_path,
        big_with_entry(r"Maps\Synthetic\blend.map", &map_blend_fixture()),
    )
    .expect("write synthetic archive");
    image::RgbaImage::from_raw(128, 128, vec![96; 128 * 128 * 4])
        .expect("terrain image")
        .save(texture_dir.join("SyntheticGround.png"))
        .expect("write terrain image");
    let mut edge = vec![0_u8; 64 * 64 * 4];
    for y in 0..64 {
        for x in 0..64 {
            let color = match x % 16 {
                0..=3 => [255, 255, 255, 255],
                4..=11 => [240, 48, 192, 255],
                _ => [0, 0, 0, 255],
            };
            let offset = (y * 64 + x) * 4;
            edge[offset..offset + 4].copy_from_slice(&color);
        }
    }
    image::RgbaImage::from_raw(64, 64, edge)
        .expect("edge image")
        .save(texture_dir.join("SyntheticEdge.png"))
        .expect("write edge image");
    fs::write(
        ini_dir.join("terrain.ini"),
        b"Terrain DefaultTerrain\n  Texture = SyntheticGround.png\nEnd\nTerrain Base\n  BlendEdges = Yes\nEnd\nTerrain Shore\n  Texture = SyntheticEdge.png\nEnd\n",
    )
    .expect("write terrain catalog");
    let output_path = root.join("terrain.png");

    let output = Command::new(env!("CARGO_BIN_EXE_cic-inspect"))
        .arg("map-render")
        .arg("--size")
        .arg("128")
        .arg("maps/synthetic/blend.map")
        .arg(&output_path)
        .arg(&archive_path)
        .arg(&root)
        .output()
        .expect("run MAP terrain renderer");
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("requesting a graphics adapter"), "{stderr}");
        fs::remove_dir_all(root).expect("remove test tree");
        return;
    }
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 terrain report");
    assert!(stdout.contains("grid\t8\t2\n"));
    assert!(stdout.contains("primary_layers\t1\n"));
    assert!(stdout.contains("extra_layers\t1\n"));
    assert!(stdout.contains("custom_edge_cells\t1\n"));
    assert!(stdout.contains("edge_indices\t6\n"));
    assert!(stdout.contains("terrain_policy\tlegacy\n"));
    let image = image::open(&output_path)
        .expect("open terrain PNG")
        .to_rgba8();
    assert_eq!(image.dimensions(), (128, 128));

    fs::remove_dir_all(root).expect("remove test tree");
}

fn map_fixture() -> Vec<u8> {
    let hex = include_str!("../../cic-formats/tests/fixtures/minimal.map.hex");
    let digits = hex
        .bytes()
        .filter(u8::is_ascii_hexdigit)
        .collect::<Vec<_>>();
    digits
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).expect("ASCII hex");
            u8::from_str_radix(pair, 16).expect("valid hex fixture")
        })
        .collect()
}

fn map_blend_fixture() -> Vec<u8> {
    let hex = include_str!("../../cic-formats/tests/fixtures/blend.map.hex");
    let digits = hex
        .bytes()
        .filter(u8::is_ascii_hexdigit)
        .collect::<Vec<_>>();
    digits
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).expect("ASCII hex");
            u8::from_str_radix(pair, 16).expect("valid hex fixture")
        })
        .collect()
}

fn w3d_fixture() -> Vec<u8> {
    let hex = include_str!("../../cic-formats/tests/fixtures/minimal.w3d.hex");
    let digits = hex
        .bytes()
        .filter(u8::is_ascii_hexdigit)
        .collect::<Vec<_>>();
    digits
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).expect("ASCII hex");
            u8::from_str_radix(pair, 16).expect("valid hex fixture")
        })
        .collect()
}

#[test]
fn static_mesh_inside_big_produces_exact_geometry_report() {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("w3d-mesh-cli");
    if root.exists() {
        fs::remove_dir_all(&root).expect("remove stale test tree");
    }
    fs::create_dir_all(&root).expect("create test tree");
    let archive_path = root.join("mesh.big");
    let mesh = static_mesh_fixture();
    fs::write(
        &archive_path,
        big_with_entry(r"Art\W3D\static-mesh.w3d", &mesh),
    )
    .expect("write synthetic archive");

    let output = Command::new(env!("CARGO_BIN_EXE_cic-inspect"))
        .arg("w3d-mesh")
        .arg("ART/W3D/STATIC-MESH.W3D")
        .arg("0")
        .arg(&archive_path)
        .output()
        .expect("run cic-inspect");

    assert!(output.status.success());
    let report = String::from_utf8(output.stdout).expect("UTF-8 output");
    assert!(report.contains(
        "0x00040002\t0x00000000\tTri\tTest\t3\t1\t0\t0\t0\t0x00000000\t0x00000001\t0x00000001\n"
    ));
    assert!(
        report
            .ends_with("0\t0\t1\t2\t0x00000000\t0x00000000\t0x00000000\t0x3F800000\t0x00000000\n")
    );

    fs::remove_dir_all(root).expect("remove test tree");
}

fn static_mesh_fixture() -> Vec<u8> {
    let hex = include_str!("../../cic-formats/tests/fixtures/static-mesh.w3d.hex");
    let digits = hex
        .bytes()
        .filter(u8::is_ascii_hexdigit)
        .collect::<Vec<_>>();
    digits
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).expect("ASCII hex");
            u8::from_str_radix(pair, 16).expect("valid hex fixture")
        })
        .collect()
}

fn colored_mesh_fixture() -> Vec<u8> {
    let hex = include_str!("../../cic-formats/tests/fixtures/colored-mesh.w3d.hex");
    let digits = hex
        .bytes()
        .filter(u8::is_ascii_hexdigit)
        .collect::<Vec<_>>();
    digits
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).expect("ASCII hex");
            u8::from_str_radix(pair, 16).expect("valid hex fixture")
        })
        .collect()
}

fn append_w3d_chunk(output: &mut Vec<u8>, id: u32, container: bool, payload: &[u8]) {
    output.extend_from_slice(&id.to_le_bytes());
    let length = u32::try_from(payload.len()).expect("fixture payload fits u32")
        | if container { 0x8000_0000 } else { 0 };
    output.extend_from_slice(&length.to_le_bytes());
    output.extend_from_slice(payload);
}

fn textured_mesh_fixture() -> Vec<u8> {
    let mut bytes = colored_mesh_fixture();
    bytes[20..24].copy_from_slice(&0x0002_0000_u32.to_le_bytes());
    bytes[84..88].copy_from_slice(&0x0000_0011_u32.to_le_bytes());
    bytes[276..280].copy_from_slice(&1_u32.to_le_bytes());
    bytes[280..284].copy_from_slice(&1_u32.to_le_bytes());
    bytes[268..272].copy_from_slice(&2_u32.to_le_bytes());
    append_w3d_chunk(&mut bytes, 0x3A, false, &0_u32.to_le_bytes());
    let mut stage = Vec::new();
    append_w3d_chunk(&mut stage, 0x49, false, &0_u32.to_le_bytes());
    let mut uv = Vec::new();
    for value in [0.0_f32, 0.0, 1.0, 0.0, 0.0, 1.0] {
        uv.extend_from_slice(&value.to_le_bytes());
    }
    append_w3d_chunk(&mut stage, 0x4A, false, &uv);
    append_w3d_chunk(&mut bytes, 0x48, true, &stage);
    append_w3d_chunk(&mut bytes, 0x48, true, &stage);
    bytes[356..360].copy_from_slice(&(128_u32 | 0x8000_0000).to_le_bytes());
    let mut second_pass = Vec::new();
    append_w3d_chunk(&mut second_pass, 0x39, false, &0_u32.to_le_bytes());
    append_w3d_chunk(&mut second_pass, 0x3A, false, &0_u32.to_le_bytes());
    append_w3d_chunk(&mut second_pass, 0x48, true, &stage);
    append_w3d_chunk(&mut bytes, 0x38, true, &second_pass);
    append_w3d_chunk(
        &mut bytes,
        0x29,
        false,
        &[3, 1, 0, 1, 0, 1, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0],
    );
    let mut texture = Vec::new();
    append_w3d_chunk(&mut texture, 0x32, false, b"checker.tga\0");
    let mut texture_info = Vec::new();
    texture_info.extend_from_slice(&[0; 12]);
    texture_info[4..8].copy_from_slice(&1_u32.to_le_bytes());
    append_w3d_chunk(&mut texture, 0x33, false, &texture_info);
    let mut entry = Vec::new();
    append_w3d_chunk(&mut entry, 0x31, true, &texture);
    append_w3d_chunk(&mut bytes, 0x30, true, &entry);
    let mut influences = Vec::new();
    for _ in 0..3 {
        influences.extend_from_slice(&1_u16.to_le_bytes());
        influences.extend_from_slice(&[0; 6]);
    }
    append_w3d_chunk(&mut bytes, 0x0E, false, &influences);
    let mesh_payload = u32::try_from(bytes.len() - 8).expect("mesh fixture payload fits u32");
    bytes[4..8].copy_from_slice(&(mesh_payload | 0x8000_0000).to_le_bytes());
    add_linear_mapper(&mut bytes);
    bytes
}

fn find_w3d_chunk(chunks: &[W3dChunk], id: u32) -> Option<&W3dChunk> {
    for chunk in chunks {
        if chunk.id() == id {
            return Some(chunk);
        }
        if let Some(found) = chunk
            .children()
            .and_then(|children| find_w3d_chunk(children, id))
        {
            return Some(found);
        }
    }
    None
}

fn increase_w3d_container(bytes: &mut [u8], header: usize, addition: usize) {
    let size = u32::from_le_bytes(
        bytes[header + 4..header + 8]
            .try_into()
            .expect("container size word"),
    );
    let payload = (size & 0x7FFF_FFFF)
        .checked_add(u32::try_from(addition).expect("fixture addition fits u32"))
        .expect("fixture container remains bounded");
    bytes[header + 4..header + 8].copy_from_slice(&(payload | 0x8000_0000).to_le_bytes());
}

fn add_linear_mapper(bytes: &mut Vec<u8>) {
    let (mesh, wrapper, material, info, insertion) = {
        let file = parse_w3d(bytes, "textured-mapper.w3d", W3dLimits::default())
            .expect("valid fixture before mapper insertion");
        let mesh = file.chunks()[0].offset();
        let wrapper = find_w3d_chunk(file.chunks(), 0x2A)
            .expect("vertex-material wrapper")
            .offset();
        let material_chunk = find_w3d_chunk(file.chunks(), 0x2B).expect("vertex material");
        let material = material_chunk.offset();
        let insertion = material + 8 + material_chunk.payload_length();
        let info = find_w3d_chunk(file.chunks(), 0x2D)
            .expect("vertex-material info")
            .offset()
            + 8;
        (mesh, wrapper, material, info, insertion)
    };
    bytes[info..info + 4].copy_from_slice(&0x0004_0000_u32.to_le_bytes());
    let mut mapper = Vec::new();
    append_w3d_chunk(
        &mut mapper,
        0x2E,
        false,
        b"UPerSec=0.5;VPerSec=-0.25;UScale=1.0;VScale=1.0;\0",
    );
    let addition = mapper.len();
    bytes.splice(insertion..insertion, mapper);
    increase_w3d_container(bytes, material, addition);
    increase_w3d_container(bytes, wrapper, addition);
    increase_w3d_container(bytes, mesh, addition);
}

fn checker_tga_fixture() -> Vec<u8> {
    let hex = include_str!("fixtures/checker.tga.hex");
    let digits = hex
        .bytes()
        .filter(u8::is_ascii_hexdigit)
        .collect::<Vec<_>>();
    digits
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).expect("ASCII hex");
            u8::from_str_radix(pair, 16).expect("valid hex fixture")
        })
        .collect()
}

fn fixed_name<const N: usize>(name: &[u8]) -> [u8; N] {
    assert!(name.len() < N);
    let mut result = [0; N];
    result[..name.len()].copy_from_slice(name);
    result
}

fn split_textured_model_fixture() -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
    let hierarchy_name = fixed_name::<16>(b"TestHierarchy");
    let mut hierarchy_header = Vec::new();
    hierarchy_header.extend_from_slice(&0x0004_0001_u32.to_le_bytes());
    hierarchy_header.extend_from_slice(&hierarchy_name);
    hierarchy_header.extend_from_slice(&2_u32.to_le_bytes());
    hierarchy_header.extend_from_slice(&[0; 12]);

    let mut pivots = Vec::new();
    for (name, parent) in [
        (fixed_name::<16>(b"RootTransform"), u32::MAX),
        (fixed_name::<16>(b"Bone"), 0),
    ] {
        pivots.extend_from_slice(&name);
        pivots.extend_from_slice(&parent.to_le_bytes());
        pivots.extend_from_slice(&[0; 24]);
        for value in [0.0_f32, 0.0, 0.0, 1.0] {
            pivots.extend_from_slice(&value.to_le_bytes());
        }
    }
    let mut hierarchy = Vec::new();
    append_w3d_chunk(&mut hierarchy, 0x101, false, &hierarchy_header);
    append_w3d_chunk(&mut hierarchy, 0x102, false, &pivots);

    let mut animation_header = Vec::new();
    animation_header.extend_from_slice(&0x0004_0001_u32.to_le_bytes());
    animation_header.extend_from_slice(&fixed_name::<16>(b"Move"));
    animation_header.extend_from_slice(&hierarchy_name);
    animation_header.extend_from_slice(&2_u32.to_le_bytes());
    animation_header.extend_from_slice(&30_u32.to_le_bytes());
    let mut animation_channel = Vec::new();
    for value in [0_u16, 1, 1, 0, 1, 0] {
        animation_channel.extend_from_slice(&value.to_le_bytes());
    }
    for value in [0.0_f32, 1.0] {
        animation_channel.extend_from_slice(&value.to_le_bytes());
    }
    let mut animation = Vec::new();
    append_w3d_chunk(&mut animation, 0x201, false, &animation_header);
    append_w3d_chunk(&mut animation, 0x202, false, &animation_channel);

    let mut compressed_header = Vec::new();
    compressed_header.extend_from_slice(&1_u32.to_le_bytes());
    compressed_header.extend_from_slice(&fixed_name::<16>(b"CompressedMove"));
    compressed_header.extend_from_slice(&hierarchy_name);
    compressed_header.extend_from_slice(&2_u32.to_le_bytes());
    compressed_header.extend_from_slice(&30_u16.to_le_bytes());
    compressed_header.extend_from_slice(&0_u16.to_le_bytes());
    let mut compressed_channel = Vec::new();
    compressed_channel.extend_from_slice(&2_u32.to_le_bytes());
    compressed_channel.extend_from_slice(&1_u16.to_le_bytes());
    compressed_channel.extend_from_slice(&[1, 2]);
    compressed_channel.extend_from_slice(&0_u32.to_le_bytes());
    compressed_channel.extend_from_slice(&0.0_f32.to_le_bytes());
    compressed_channel.extend_from_slice(&1_u32.to_le_bytes());
    compressed_channel.extend_from_slice(&1.0_f32.to_le_bytes());
    let mut compressed_animation = Vec::new();
    append_w3d_chunk(&mut compressed_animation, 0x281, false, &compressed_header);
    append_w3d_chunk(&mut compressed_animation, 0x282, false, &compressed_channel);

    let mut hlod_header = Vec::new();
    hlod_header.extend_from_slice(&0x0001_0000_u32.to_le_bytes());
    hlod_header.extend_from_slice(&1_u32.to_le_bytes());
    hlod_header.extend_from_slice(&fixed_name::<16>(b"Test"));
    hlod_header.extend_from_slice(&hierarchy_name);
    let mut lod_header = Vec::new();
    lod_header.extend_from_slice(&1_u32.to_le_bytes());
    lod_header.extend_from_slice(&1.0_f32.to_le_bytes());
    let mut sub_object = Vec::new();
    sub_object.extend_from_slice(&1_u32.to_le_bytes());
    sub_object.extend_from_slice(&fixed_name::<32>(b"Test.Tri"));
    let mut lod = Vec::new();
    append_w3d_chunk(&mut lod, 0x703, false, &lod_header);
    append_w3d_chunk(&mut lod, 0x704, false, &sub_object);
    let mut hlod = Vec::new();
    append_w3d_chunk(&mut hlod, 0x701, false, &hlod_header);
    append_w3d_chunk(&mut hlod, 0x702, true, &lod);

    let mut hierarchy_file = Vec::new();
    append_w3d_chunk(&mut hierarchy_file, 0x100, true, &hierarchy);
    let mut animation_file = Vec::new();
    append_w3d_chunk(&mut animation_file, 0x200, true, &animation);
    let mut compressed_animation_file = Vec::new();
    append_w3d_chunk(
        &mut compressed_animation_file,
        0x280,
        true,
        &compressed_animation,
    );
    let mut model_file = textured_mesh_fixture();
    append_w3d_chunk(&mut model_file, 0x700, true, &hlod);
    (
        model_file,
        hierarchy_file,
        animation_file,
        compressed_animation_file,
    )
}

#[test]
#[allow(clippy::too_many_lines)]
fn installed_profile_exports_single_glb_by_default_and_optional_gltf() {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("textured-w3d-cli");
    if root.exists() {
        fs::remove_dir_all(&root).expect("remove stale test tree");
    }
    fs::create_dir_all(&root).expect("create test tree");
    let mesh_archive = root.join("W3D.big");
    let texture_archive = root.join("Textures.big");
    let (model, hierarchy, animation, compressed_animation) = split_textured_model_fixture();
    fs::write(
        &mesh_archive,
        big_with_entries(&[
            (r"Art\W3D\textured_skn.w3d", &model),
            (r"Art\W3D\testhierarchy.w3d", &hierarchy),
            (r"Art\W3D\testhierarchy_move.w3d", &animation),
            (
                r"Art\W3D\testhierarchy_compressed.w3d",
                &compressed_animation,
            ),
        ]),
    )
    .expect("write mesh archive");
    let tga = checker_tga_fixture();
    fs::write(
        &texture_archive,
        big_with_entry(r"Art\Textures\checker.tga", &tga),
    )
    .expect("write texture archive");
    let glb_path = root.join("textured_skn.glb");

    let output = run_model_export(&root, None, false);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!root.join("textured.bin").exists());
    assert!(!root.join("textured_textures").exists());
    let glb = fs::read(&glb_path).expect("read GLB");
    let (document, binary) = parse_glb(&glb);
    assert_eq!(document["asset"]["version"], "2.0");
    assert!(document["buffers"][0].get("uri").is_none());
    assert_eq!(document["meshes"].as_array().map(Vec::len), Some(1));
    assert_eq!(document["animations"].as_array().map(Vec::len), Some(2));
    let encodings = document["animations"]
        .as_array()
        .expect("animation array")
        .iter()
        .map(|animation| {
            animation["extras"]["w3dEncoding"]
                .as_str()
                .expect("encoding")
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        encodings,
        std::collections::BTreeSet::from(["raw", "time-coded"])
    );
    assert_eq!(document["skins"].as_array().map(Vec::len), Some(1));
    assert!(document["skins"][0].get("inverseBindMatrices").is_none());
    assert!(document["materials"][0].get("alphaCutoff").is_none());
    assert_eq!(document["materials"][0]["alphaMode"], "BLEND");
    assert_eq!(
        document["materials"][0]["extras"]["w3dPreviewBlend"],
        "additive-alpha-coverage-v1"
    );
    assert_eq!(
        document["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["index"],
        1
    );
    let material_extras = &document["meshes"][0]["extras"];
    assert_eq!(
        material_extras["w3dMaterialPolicy"],
        "fixed-function-metadata-v1"
    );
    assert_eq!(material_extras["passes"].as_array().map(Vec::len), Some(2));
    assert_eq!(
        material_extras["passes"][0]["textureStages"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    assert_eq!(
        material_extras["vertexMaterials"][0]["mappers"][0]["modeName"],
        "linear_offset"
    );
    assert_eq!(material_extras["textures"][0]["info"]["frameCount"], 1);
    assert_eq!(material_extras["textures"][0]["gltfTexture"], 0);
    assert_skinned_mesh_is_scene_root(&document);
    assert_eq!(document["images"].as_array().map(Vec::len), Some(2));
    assert_eq!(document["images"][0]["mimeType"], "image/png");
    assert!(document["images"][0].get("uri").is_none());
    let image_view = document["images"][0]["bufferView"]
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .expect("image buffer view index");
    let offset = document["bufferViews"][image_view]["byteOffset"]
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .expect("image byte offset");
    let length = document["bufferViews"][image_view]["byteLength"]
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .expect("image byte length");
    let embedded_png = &binary[offset..offset + length];
    assert_png_preserves_srgb_texels(&tga, embedded_png);

    let gltf_path = root.join("textured.gltf");
    let output = run_model_export(&root, Some(&gltf_path), true);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document: serde_json::Value =
        serde_json::from_slice(&fs::read(&gltf_path).expect("read glTF document"))
            .expect("parse glTF JSON");
    assert_eq!(document["asset"]["version"], "2.0");
    assert_eq!(document["buffers"][0]["uri"], "textured.bin");
    assert_eq!(document["meshes"].as_array().map(Vec::len), Some(1));
    assert_eq!(document["animations"].as_array().map(Vec::len), Some(2));
    assert_eq!(document["materials"].as_array().map(Vec::len), Some(1));
    assert_eq!(document["skins"].as_array().map(Vec::len), Some(1));
    assert!(document["meshes"][0]["primitives"][0]["attributes"]["JOINTS_0"].is_number());
    assert!(document["meshes"][0]["primitives"][0]["attributes"]["WEIGHTS_0"].is_number());
    assert_eq!(
        document["images"][0]["uri"],
        "textured_textures/m000_t0000_checker.png"
    );
    assert_eq!(
        document["images"][1]["uri"],
        "textured_textures/m000_t0000_checker_additive-preview.png"
    );
    assert!(
        !fs::read(root.join("textured.bin"))
            .expect("read glTF buffer")
            .is_empty()
    );
    let png = fs::read(root.join("textured_textures/m000_t0000_checker.png"))
        .expect("read converted PNG");
    assert_png_preserves_srgb_texels(&tga, &png);
    assert!(
        root.join("textured_textures/m000_t0000_checker_additive-preview.png")
            .is_file()
    );
    let capture_path = root.join("textured.ppm");
    let output = run_model_render(&root, &capture_path);
    if output.status.success() {
        let capture = fs::read(&capture_path).expect("read W3D render capture");
        assert!(capture.starts_with(b"P6\n512 512\n255\n"));
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("animation\t0\n"));
        assert!(stdout.contains("frame\t1\n"));
        assert!(stdout.contains("mapper_time_seconds\t0.5\n"));
        assert!(stdout.contains("vertices\t9\n"));
        assert!(stdout.contains("indices\t9\n"));
        assert!(stdout.contains("draws\t3\n"));
        assert!(stdout.contains("materials\t2\n"));
        assert!(stdout.contains("textures\t1\n"));
        let expected = include_str!("fixtures/textured-animated.rgba.sha256").trim();
        assert!(
            stdout.contains(&format!("rgba_sha256\t{expected}\n")),
            "{stdout}"
        );
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("requesting a graphics adapter"), "{stderr}");
    }
    fs::remove_dir_all(root).expect("remove test tree");
}

fn assert_skinned_mesh_is_scene_root(document: &serde_json::Value) {
    let skinned_node = document["nodes"]
        .as_array()
        .and_then(|nodes| nodes.iter().position(|node| node.get("skin").is_some()))
        .expect("skinned mesh node");
    assert!(
        document["scenes"][0]["nodes"]
            .as_array()
            .is_some_and(|nodes| nodes.contains(&json!(skinned_node)))
    );
    assert!(!document["nodes"].as_array().is_some_and(|nodes| {
        nodes.iter().any(|node| {
            node["children"]
                .as_array()
                .is_some_and(|children| children.contains(&json!(skinned_node)))
        })
    }));
}

fn run_model_export(
    root: &std::path::Path,
    output: Option<&std::path::Path>,
    gltf: bool,
) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cic-inspect"));
    command
        .current_dir(root)
        .arg("--game-dir")
        .arg(root)
        .arg("w3d-export");
    if gltf {
        command.arg("--gltf");
    }
    command.arg("art/w3d/textured_skn.w3d");
    if let Some(output) = output {
        command.arg(output);
    }
    command.output().expect("run model export")
}

fn run_model_render(root: &std::path::Path, output: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cic-inspect"))
        .current_dir(root)
        .arg("--game-dir")
        .arg(root)
        .arg("w3d-render")
        .arg("--animation")
        .arg("0")
        .arg("--frame")
        .arg("1")
        .arg("--time")
        .arg("0.5")
        .arg("art/w3d/textured_skn.w3d")
        .arg(output)
        .output()
        .expect("run W3D render")
}

fn parse_glb(bytes: &[u8]) -> (serde_json::Value, &[u8]) {
    assert_eq!(&bytes[..4], b"glTF");
    assert_eq!(
        u32::from_le_bytes(bytes[4..8].try_into().expect("GLB version")),
        2
    );
    assert_eq!(
        usize::try_from(u32::from_le_bytes(
            bytes[8..12].try_into().expect("GLB total length")
        ))
        .expect("GLB length fits usize"),
        bytes.len()
    );
    let json_length = usize::try_from(u32::from_le_bytes(
        bytes[12..16].try_into().expect("GLB JSON length"),
    ))
    .expect("JSON length fits usize");
    assert_eq!(&bytes[16..20], b"JSON");
    let json_end = 20 + json_length;
    let json = bytes[20..json_end]
        .strip_suffix(b" ")
        .unwrap_or(&bytes[20..json_end]);
    let document = serde_json::from_slice(json).expect("parse embedded glTF JSON");
    let binary_length = usize::try_from(u32::from_le_bytes(
        bytes[json_end..json_end + 4]
            .try_into()
            .expect("GLB BIN length"),
    ))
    .expect("BIN length fits usize");
    assert_eq!(&bytes[json_end + 4..json_end + 8], b"BIN\0");
    let binary_start = json_end + 8;
    (document, &bytes[binary_start..binary_start + binary_length])
}

fn assert_png_preserves_srgb_texels(tga: &[u8], png: &[u8]) {
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1A\n");
    assert!(png_has_chunk(png, *b"sRGB"));
    let source = image::load_from_memory_with_format(tga, image::ImageFormat::Tga)
        .expect("decode source TGA")
        .to_rgba8();
    let converted = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .expect("decode converted PNG")
        .to_rgba8();
    assert_eq!(converted.dimensions(), source.dimensions());
    assert_eq!(converted.as_raw(), source.as_raw());
}

fn png_has_chunk(bytes: &[u8], expected: [u8; 4]) -> bool {
    let mut offset = 8;
    while offset + 12 <= bytes.len() {
        let length = usize::try_from(u32::from_be_bytes(
            bytes[offset..offset + 4]
                .try_into()
                .expect("PNG chunk length"),
        ))
        .expect("PNG chunk length fits usize");
        if bytes[offset + 4..offset + 8] == expected {
            return true;
        }
        let Some(next) = offset
            .checked_add(12)
            .and_then(|value| value.checked_add(length))
        else {
            return false;
        };
        offset = next;
    }
    false
}
