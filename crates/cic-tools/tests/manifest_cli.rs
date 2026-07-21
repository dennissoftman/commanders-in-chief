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
