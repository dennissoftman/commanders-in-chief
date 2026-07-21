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
