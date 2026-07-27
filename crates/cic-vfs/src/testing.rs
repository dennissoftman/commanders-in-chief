//! Archive fixture builders for unit tests.
//!
//! These write real containers rather than recorded blobs, so a test can express the *structural*
//! case it cares about — a straddled block boundary, a trailing comment, a bomb's declared size —
//! and stay readable. Nothing here validates its own output; that is the reader's job.

use std::io::Write;

/// Compresses with raw DEFLATE, as zip method 8 stores it (no zlib or gzip wrapper).
pub(crate) fn deflate(bytes: &[u8]) -> Vec<u8> {
    let mut encoder =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).expect("deflate fixture");
    encoder.finish().expect("finish deflate fixture")
}

/// Wraps bytes in a gzip stream.
pub(crate) fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).expect("gzip fixture");
    encoder.finish().expect("finish gzip fixture")
}

struct ZipMember {
    name: String,
    stored: Vec<u8>,
    uncompressed_size: u32,
    method: u16,
    flags: u16,
    local_offset: u32,
}

/// Builds a zip archive member by member.
pub(crate) struct ZipBuilder {
    body: Vec<u8>,
    members: Vec<ZipMember>,
    comment: Vec<u8>,
}

impl ZipBuilder {
    pub(crate) fn new() -> Self {
        Self {
            body: Vec::new(),
            members: Vec::new(),
            comment: Vec::new(),
        }
    }

    /// Adds a member stored verbatim.
    pub(crate) fn stored(self, name: &str, payload: &[u8]) -> Self {
        let length = u32::try_from(payload.len()).expect("fixture payload fits u32");
        self.with_method(name, payload, 0, length)
    }

    /// Adds a member whose payload is already raw-DEFLATE compressed.
    pub(crate) fn deflated(self, name: &str, compressed: &[u8], uncompressed_size: usize) -> Self {
        let declared = u32::try_from(uncompressed_size).expect("fixture size fits u32");
        self.with_method(name, compressed, 8, declared)
    }

    /// Adds a member with an explicit compression method, so unsupported methods can be tested.
    pub(crate) fn with_method(
        mut self,
        name: &str,
        stored: &[u8],
        method: u16,
        uncompressed_size: u32,
    ) -> Self {
        self.push_member(name, stored, method, uncompressed_size, 0);
        self
    }

    /// Adds a member with the encryption flag set.
    pub(crate) fn encrypted(mut self, name: &str, stored: &[u8]) -> Self {
        let length = u32::try_from(stored.len()).expect("fixture payload fits u32");
        self.push_member(name, stored, 0, length, 1);
        self
    }

    /// Sets a trailing archive comment, which displaces the end-of-central-directory record.
    pub(crate) fn comment(mut self, comment: &[u8]) -> Self {
        self.comment = comment.to_vec();
        self
    }

    fn push_member(
        &mut self,
        name: &str,
        stored: &[u8],
        method: u16,
        uncompressed_size: u32,
        flags: u16,
    ) {
        let local_offset = u32::try_from(self.body.len()).expect("fixture offset fits u32");
        let compressed_size = u32::try_from(stored.len()).expect("fixture payload fits u32");
        let name_bytes = name.as_bytes();

        self.body.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        self.body.extend_from_slice(&20u16.to_le_bytes()); // version needed
        self.body.extend_from_slice(&flags.to_le_bytes());
        self.body.extend_from_slice(&method.to_le_bytes());
        self.body.extend_from_slice(&0u16.to_le_bytes()); // time
        self.body.extend_from_slice(&0u16.to_le_bytes()); // date
        self.body.extend_from_slice(&0u32.to_le_bytes()); // crc32, ignored by the reader
        self.body.extend_from_slice(&compressed_size.to_le_bytes());
        self.body
            .extend_from_slice(&uncompressed_size.to_le_bytes());
        self.body.extend_from_slice(
            &u16::try_from(name_bytes.len())
                .expect("fixture name fits u16")
                .to_le_bytes(),
        );
        self.body.extend_from_slice(&0u16.to_le_bytes()); // extra length
        self.body.extend_from_slice(name_bytes);
        self.body.extend_from_slice(stored);

        self.members.push(ZipMember {
            name: name.to_owned(),
            stored: stored.to_vec(),
            uncompressed_size,
            method,
            flags,
            local_offset,
        });
    }

    /// Emits the central directory and end record, returning the whole archive.
    pub(crate) fn finish(self) -> Vec<u8> {
        let mut archive = self.body;
        let directory_offset = u32::try_from(archive.len()).expect("fixture offset fits u32");

        for member in &self.members {
            let name_bytes = member.name.as_bytes();
            let compressed_size =
                u32::try_from(member.stored.len()).expect("fixture payload fits u32");
            archive.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            archive.extend_from_slice(&20u16.to_le_bytes()); // version made by
            archive.extend_from_slice(&20u16.to_le_bytes()); // version needed
            archive.extend_from_slice(&member.flags.to_le_bytes());
            archive.extend_from_slice(&member.method.to_le_bytes());
            archive.extend_from_slice(&0u16.to_le_bytes()); // time
            archive.extend_from_slice(&0u16.to_le_bytes()); // date
            archive.extend_from_slice(&0u32.to_le_bytes()); // crc32
            archive.extend_from_slice(&compressed_size.to_le_bytes());
            archive.extend_from_slice(&member.uncompressed_size.to_le_bytes());
            archive.extend_from_slice(
                &u16::try_from(name_bytes.len())
                    .expect("fixture name fits u16")
                    .to_le_bytes(),
            );
            archive.extend_from_slice(&0u16.to_le_bytes()); // extra length
            archive.extend_from_slice(&0u16.to_le_bytes()); // comment length
            archive.extend_from_slice(&0u16.to_le_bytes()); // disk start
            archive.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
            archive.extend_from_slice(&0u32.to_le_bytes()); // external attributes
            archive.extend_from_slice(&member.local_offset.to_le_bytes());
            archive.extend_from_slice(name_bytes);
        }

        let directory_size =
            u32::try_from(archive.len()).expect("fixture size fits u32") - directory_offset;
        let count = u16::try_from(self.members.len()).expect("fixture count fits u16");

        archive.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        archive.extend_from_slice(&0u16.to_le_bytes()); // this disk
        archive.extend_from_slice(&0u16.to_le_bytes()); // directory disk
        archive.extend_from_slice(&count.to_le_bytes());
        archive.extend_from_slice(&count.to_le_bytes());
        archive.extend_from_slice(&directory_size.to_le_bytes());
        archive.extend_from_slice(&directory_offset.to_le_bytes());
        archive.extend_from_slice(
            &u16::try_from(self.comment.len())
                .expect("fixture comment fits u16")
                .to_le_bytes(),
        );
        archive.extend_from_slice(&self.comment);
        archive
    }
}

const TAR_BLOCK: usize = 512;

/// Builds a tar archive member by member.
pub(crate) struct TarBuilder {
    body: Vec<u8>,
}

impl TarBuilder {
    pub(crate) fn new() -> Self {
        Self { body: Vec::new() }
    }

    /// Adds a regular file.
    pub(crate) fn file(self, name: &str, payload: &[u8]) -> Self {
        self.entry(name, payload, payload.len(), b'0', "")
    }

    /// Adds a regular file whose header declares a size other than its payload length.
    pub(crate) fn file_with_size(self, name: &str, payload: &[u8], declared: usize) -> Self {
        self.entry(name, payload, declared, b'0', "")
    }

    /// Adds a directory marker, which carries no payload.
    pub(crate) fn directory(self, name: &str) -> Self {
        self.entry(name, &[], 0, b'5', "")
    }

    /// Adds a symbolic link, which a reader must skip rather than resolve.
    pub(crate) fn symlink(self, name: &str, target: &str) -> Self {
        self.entry(name, &[], 0, b'2', target)
    }

    fn entry(
        mut self,
        name: &str,
        payload: &[u8],
        declared_size: usize,
        type_flag: u8,
        link_target: &str,
    ) -> Self {
        let mut header = [0u8; TAR_BLOCK];

        // Split long paths across the ustar prefix field at a component boundary, which is what
        // real tar does rather than truncating.
        let (prefix, short) = split_ustar(name);
        header[0..short.len()].copy_from_slice(short.as_bytes());
        write_octal(&mut header[100..108], 0o644);
        write_octal(&mut header[108..116], 0);
        write_octal(&mut header[116..124], 0);
        write_octal(&mut header[124..136], declared_size as u64);
        write_octal(&mut header[136..148], 0);
        header[156] = type_flag;
        header[157..157 + link_target.len()].copy_from_slice(link_target.as_bytes());
        header[257..262].copy_from_slice(b"ustar");
        header[263..265].copy_from_slice(b"00");
        header[345..345 + prefix.len()].copy_from_slice(prefix.as_bytes());

        // The checksum is computed with its own field read as spaces.
        header[148..156].fill(b' ');
        let sum: u64 = header.iter().map(|byte| u64::from(*byte)).sum();
        let checksum = format!("{sum:06o}\0 ");
        header[148..156].copy_from_slice(checksum.as_bytes());

        self.body.extend_from_slice(&header);
        self.body.extend_from_slice(payload);
        let padding = payload.len().next_multiple_of(TAR_BLOCK) - payload.len();
        self.body.extend_from_slice(&vec![0u8; padding]);
        self
    }

    /// Appends the two terminating zero blocks a real archive ends with.
    pub(crate) fn finish(mut self) -> Vec<u8> {
        self.body.extend_from_slice(&[0u8; TAR_BLOCK * 2]);
        self.body
    }
}

/// Splits a path into a ustar `(prefix, name)` pair at a component boundary.
fn split_ustar(name: &str) -> (&str, &str) {
    if name.len() <= 100 {
        return ("", name);
    }
    // The last separator that leaves the tail within the 100-byte name field.
    let split = name
        .char_indices()
        .filter(|(index, character)| *character == '/' && name.len() - index - 1 <= 100)
        .map(|(index, _)| index)
        .next()
        .expect("fixture path must have a separator that fits the ustar split");
    (&name[..split], &name[split + 1..])
}

fn write_octal(field: &mut [u8], value: u64) {
    let text = format!("{:0width$o}\0", value, width = field.len() - 1);
    field.copy_from_slice(text.as_bytes());
}
