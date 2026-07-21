//! Stable diagnostic report formatting.

use std::fmt::Write;

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

#[cfg(test)]
mod tests {
    use cic_vfs::{Vfs, VirtualPath};

    use super::render_manifest;

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
}
