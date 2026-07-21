use std::fs;
use std::process::Command;

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
    append_w3d_chunk(&mut bytes, 0x3A, false, &0_u32.to_le_bytes());
    let mut stage = Vec::new();
    append_w3d_chunk(&mut stage, 0x49, false, &0_u32.to_le_bytes());
    let mut uv = Vec::new();
    for value in [0.0_f32, 0.0, 1.0, 0.0, 0.0, 1.0] {
        uv.extend_from_slice(&value.to_le_bytes());
    }
    append_w3d_chunk(&mut stage, 0x4A, false, &uv);
    append_w3d_chunk(&mut bytes, 0x48, true, &stage);
    bytes[356..360].copy_from_slice(&(76_u32 | 0x8000_0000).to_le_bytes());
    append_w3d_chunk(
        &mut bytes,
        0x29,
        false,
        &[3, 1, 0, 0, 0, 1, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0],
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
    bytes[4..8].copy_from_slice(&(540_u32 | 0x8000_0000).to_le_bytes());
    bytes
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

fn split_textured_model_fixture() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
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
    let mut model_file = textured_mesh_fixture();
    append_w3d_chunk(&mut model_file, 0x700, true, &hlod);
    (model_file, hierarchy_file, animation_file)
}

#[test]
fn installed_profile_composes_split_w3d_and_texture_archives_into_animated_gltf() {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("textured-w3d-cli");
    if root.exists() {
        fs::remove_dir_all(&root).expect("remove stale test tree");
    }
    fs::create_dir_all(&root).expect("create test tree");
    let mesh_archive = root.join("W3D.big");
    let texture_archive = root.join("Textures.big");
    let (model, hierarchy, animation) = split_textured_model_fixture();
    fs::write(
        &mesh_archive,
        big_with_entries(&[
            (r"Art\W3D\textured_skn.w3d", &model),
            (r"Art\W3D\testhierarchy.w3d", &hierarchy),
            (r"Art\W3D\testhierarchy_move.w3d", &animation),
        ]),
    )
    .expect("write mesh archive");
    let tga = checker_tga_fixture();
    fs::write(
        &texture_archive,
        big_with_entry(r"Art\Textures\checker.tga", &tga),
    )
    .expect("write texture archive");
    let gltf_path = root.join("textured.gltf");

    let output = Command::new(env!("CARGO_BIN_EXE_cic-inspect"))
        .arg("--game-dir")
        .arg(&root)
        .arg("w3d-gltf")
        .arg("art/w3d/textured_skn.w3d")
        .arg(&gltf_path)
        .output()
        .expect("run glTF export");
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
    assert_eq!(document["animations"].as_array().map(Vec::len), Some(1));
    assert_eq!(document["materials"].as_array().map(Vec::len), Some(1));
    assert_eq!(document["skins"].as_array().map(Vec::len), Some(1));
    assert!(document["meshes"][0]["primitives"][0]["attributes"]["JOINTS_0"].is_number());
    assert!(document["meshes"][0]["primitives"][0]["attributes"]["WEIGHTS_0"].is_number());
    assert_eq!(
        document["images"][0]["uri"],
        "textured_textures/m000_t0000_checker.png"
    );
    assert!(
        !fs::read(root.join("textured.bin"))
            .expect("read glTF buffer")
            .is_empty()
    );
    let png = fs::read(root.join("textured_textures/m000_t0000_checker.png"))
        .expect("read converted PNG");
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1A\n");
    fs::remove_dir_all(root).expect("remove test tree");
}
