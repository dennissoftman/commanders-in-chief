//! Stable diagnostic report formatting.

use std::fmt::Write;

use cic_formats::CsfFile;
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
    use cic_formats::{CsfLimits, parse_csf};
    use cic_vfs::{Vfs, VirtualPath};

    use super::{render_csf, render_manifest};

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
}
