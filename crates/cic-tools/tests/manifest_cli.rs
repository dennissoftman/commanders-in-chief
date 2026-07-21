use std::fs;
use std::process::Command;

#[test]
fn mounted_directories_produce_a_stable_overlay_manifest() {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("manifest-cli");
    if root.exists() {
        fs::remove_dir_all(&root).expect("remove stale test tree");
    }
    let base = root.join("base");
    let overlay = root.join("overlay");
    fs::create_dir_all(base.join("Data")).expect("create base tree");
    fs::create_dir_all(overlay.join("data")).expect("create overlay tree");
    fs::write(base.join("Data").join("Z.TXT"), b"z").expect("write base resource");
    fs::write(base.join("Data").join("A.txt"), b"old").expect("write base resource");
    fs::write(overlay.join("data").join("a.TXT"), b"new!").expect("write overlay resource");

    let output = Command::new(env!("CARGO_BIN_EXE_cic-inspect"))
        .arg("manifest")
        .arg(&base)
        .arg(&overlay)
        .output()
        .expect("run cic-inspect");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("UTF-8 output"),
        "path\tbytes\tprovider\ndata/a.txt\t4\tdirectory:mount-1\ndata/z.txt\t1\tdirectory:mount-0\n"
    );

    fs::remove_dir_all(root).expect("remove test tree");
}
