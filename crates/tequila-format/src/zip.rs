//! A ZIP reader and writer. It writes stored entries only; it reads stored
//! and deflated ones, since a ZIP a person made - a plugin, a material
//! package, Windows' Compressed folder - is deflated.
//!
//! §4.1: "A ZIP archive (stored, not compressed, for large binary members so
//! memory-mapping works)." Storing rather than deflating is what lets a
//! `.fsmesh` member be mapped and handed to Embree without a copy, and it also
//! means this needs no compressor — a stored ZIP is headers, payloads, and a
//! central directory.
//!
//! # Determinism
//!
//! M1's exit criterion is a **byte-identical** save/reopen round trip, so
//! nothing here may vary between two writes of the same content. ZIP's
//! per-entry MS-DOS timestamp is the trap: using the wall clock would make
//! every save differ from the last. Every entry is written with the same fixed
//! timestamp, and entries are written in the order given, which the document
//! writer keeps sorted.
//!
//! A file's modification time is metadata about the file, not about the scene;
//! losing it costs nothing, and keeping it would cost the round trip.

use std::collections::BTreeMap;
use std::fmt;

/// 1980-01-01 00:00:00, the earliest MS-DOS timestamp ZIP can express, used
/// for every entry so that two saves of one document agree byte for byte.
const FIXED_DOS_TIME: u16 = 0;
const FIXED_DOS_DATE: u16 = 0x0021;

const LOCAL_HEADER: u32 = 0x0403_4b50;
const CENTRAL_HEADER: u32 = 0x0201_4b50;
const END_OF_CENTRAL_DIRECTORY: u32 = 0x0605_4b50;
const STORED: u16 = 0;
const DEFLATED: u16 = 8;
/// A central-directory entry's fixed-size part, before its variable-length
/// name, extra field and comment.
const CENTRAL_HEADER_SIZE: usize = 46;
/// A local header's fixed-size part, likewise.
const LOCAL_HEADER_SIZE: usize = 30;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ZipError {
    /// The archive is not a ZIP, or its structure does not hold together.
    Malformed(String),
    /// A member's CRC does not match its bytes: the file is damaged.
    Corrupt { name: String },
    /// The archive uses a ZIP feature this reader does not implement.
    Unsupported(String),
}

impl fmt::Display for ZipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ZipError::Malformed(m) => write!(f, "the archive is malformed: {m}"),
            ZipError::Corrupt { name } => {
                write!(f, "`{name}` is damaged: its checksum does not match")
            }
            ZipError::Unsupported(m) => write!(f, "unsupported archive: {m}"),
        }
    }
}

impl std::error::Error for ZipError {}

/// Build a stored ZIP. Members are written in insertion order.
#[derive(Default)]
pub struct ZipWriter {
    entries: Vec<(String, Vec<u8>)>,
}

impl ZipWriter {
    pub fn new() -> ZipWriter {
        ZipWriter::default()
    }

    pub fn add(&mut self, name: impl Into<String>, bytes: Vec<u8>) {
        self.entries.push((name.into(), bytes));
    }

    /// Why the archive would not fit the format this writer writes, if it
    /// would not: plain ZIP counts members in 16 bits and places them in 32,
    /// so past 65,535 members or 4 GB the offsets wrap and the file written
    /// does not open. Asked before writing over a project that does.
    pub fn too_big(&self) -> Option<String> {
        if self.entries.len() > u16::MAX as usize {
            return Some(format!("it has {} parts to store, more than a project file can hold (65,535)", self.entries.len()));
        }
        let size: u64 = self.entries.iter().map(|(name, data)| 76 + 2 * name.len() as u64 + data.len() as u64).sum::<u64>() + 22;
        if size > u32::MAX as u64 {
            return Some(format!("it would be {:.1} GB, more than a project file can hold (4 GB)", size as f64 / 1e9));
        }
        None
    }

    pub fn finish(self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut directory = Vec::new();

        for (name, data) in &self.entries {
            let offset = out.len() as u32;
            let crc = crc32(data);
            let name_bytes = name.as_bytes();

            out.extend_from_slice(&LOCAL_HEADER.to_le_bytes());
            out.extend_from_slice(&20u16.to_le_bytes()); // version needed
            out.extend_from_slice(&0u16.to_le_bytes()); // flags
            out.extend_from_slice(&STORED.to_le_bytes());
            out.extend_from_slice(&FIXED_DOS_TIME.to_le_bytes());
            out.extend_from_slice(&FIXED_DOS_DATE.to_le_bytes());
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // extra field length
            out.extend_from_slice(name_bytes);
            out.extend_from_slice(data);

            directory.extend_from_slice(&CENTRAL_HEADER.to_le_bytes());
            directory.extend_from_slice(&20u16.to_le_bytes()); // version made by
            directory.extend_from_slice(&20u16.to_le_bytes()); // version needed
            directory.extend_from_slice(&0u16.to_le_bytes());
            directory.extend_from_slice(&STORED.to_le_bytes());
            directory.extend_from_slice(&FIXED_DOS_TIME.to_le_bytes());
            directory.extend_from_slice(&FIXED_DOS_DATE.to_le_bytes());
            directory.extend_from_slice(&crc.to_le_bytes());
            directory.extend_from_slice(&(data.len() as u32).to_le_bytes());
            directory.extend_from_slice(&(data.len() as u32).to_le_bytes());
            directory.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
            directory.extend_from_slice(&0u16.to_le_bytes()); // extra
            directory.extend_from_slice(&0u16.to_le_bytes()); // comment
            directory.extend_from_slice(&0u16.to_le_bytes()); // disk number
            directory.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
            directory.extend_from_slice(&0u32.to_le_bytes()); // external attrs
            directory.extend_from_slice(&offset.to_le_bytes());
            directory.extend_from_slice(name_bytes);
        }

        let directory_offset = out.len() as u32;
        let directory_size = directory.len() as u32;
        out.extend_from_slice(&directory);

        out.extend_from_slice(&END_OF_CENTRAL_DIRECTORY.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // this disk
        out.extend_from_slice(&0u16.to_le_bytes()); // disk with directory
        out.extend_from_slice(&(self.entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(self.entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&directory_size.to_le_bytes());
        out.extend_from_slice(&directory_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // comment length
        out
    }
}

/// An archive read into memory, keyed by member name.
///
/// Read through the **central directory**, not by scanning for local headers.
/// The directory is the authority on what an archive contains; a reader that
/// scans forward instead can be shown members the directory does not list,
/// which is the shape of several ZIP-handling vulnerabilities.
#[derive(Debug)]
pub struct ZipArchive {
    members: BTreeMap<String, Vec<u8>>,
    order: Vec<String>,
}

impl ZipArchive {
    pub fn read(bytes: &[u8]) -> Result<ZipArchive, ZipError> {
        let end = find_end_of_central_directory(bytes)?;
        let count = u16::from_le_bytes([bytes[end + 10], bytes[end + 11]]) as usize;
        let directory_size = read_u32(bytes, end + 12)? as usize;
        let directory_offset = read_u32(bytes, end + 16)? as usize;

        if directory_offset + directory_size > bytes.len() {
            return Err(ZipError::Malformed(
                "the central directory runs past the end of the file".into(),
            ));
        }

        let mut members = BTreeMap::new();
        let mut order = Vec::new();
        let mut at = directory_offset;

        for _ in 0..count {
            // A directory entry is 46 fixed bytes before its variable-length
            // name. Checking the signature alone is not enough: an archive
            // whose last four bytes happen to be a directory signature would
            // send every field read below past the end of the buffer.
            if at + CENTRAL_HEADER_SIZE > bytes.len() {
                return Err(ZipError::Malformed(format!(
                    "a directory entry at byte {at} runs past the end of the file"
                )));
            }
            if read_u32(bytes, at)? != CENTRAL_HEADER {
                return Err(ZipError::Malformed(format!(
                    "expected a directory entry at byte {at}"
                )));
            }
            let method = u16::from_le_bytes([bytes[at + 10], bytes[at + 11]]);
            let crc = read_u32(bytes, at + 16)?;
            let compressed = read_u32(bytes, at + 20)? as usize;
            let uncompressed = read_u32(bytes, at + 24)? as usize;
            let name_length = u16::from_le_bytes([bytes[at + 28], bytes[at + 29]]) as usize;
            let extra_length = u16::from_le_bytes([bytes[at + 30], bytes[at + 31]]) as usize;
            let comment_length = u16::from_le_bytes([bytes[at + 32], bytes[at + 33]]) as usize;
            let local_offset = read_u32(bytes, at + 42)? as usize;

            let name_end = at + 46 + name_length;
            if name_end > bytes.len() {
                return Err(ZipError::Malformed(
                    "a member name runs past the file".into(),
                ));
            }
            let name = String::from_utf8_lossy(&bytes[at + 46..name_end]).into_owned();

            if method != STORED && method != DEFLATED {
                return Err(ZipError::Unsupported(format!(
                    "`{name}` is compressed with method {method}; only stored and deflated members are read"
                )));
            }
            // A member naming `../` would escape the extraction directory. This
            // reader never writes to disk, but the names index a map that
            // callers turn into paths, so it is refused here where it is
            // cheap.
            if name.contains("..") || name.starts_with('/') || name.contains('\\') {
                return Err(ZipError::Malformed(format!(
                    "`{name}` is not a safe member name"
                )));
            }

            // Payload position comes from the local header, whose own
            // variable-length fields may differ from the directory's.
            if local_offset + LOCAL_HEADER_SIZE > bytes.len() {
                return Err(ZipError::Malformed(format!(
                    "`{name}` has a local header past the end of the file"
                )));
            }
            if read_u32(bytes, local_offset)? != LOCAL_HEADER {
                return Err(ZipError::Malformed(format!(
                    "`{name}` has no local header at byte {local_offset}"
                )));
            }
            let local_name_length =
                u16::from_le_bytes([bytes[local_offset + 26], bytes[local_offset + 27]]) as usize;
            let local_extra_length =
                u16::from_le_bytes([bytes[local_offset + 28], bytes[local_offset + 29]]) as usize;
            let start = local_offset + 30 + local_name_length + local_extra_length;
            let finish = start
                .checked_add(compressed)
                .ok_or_else(|| ZipError::Malformed(format!("`{name}` has an implausible size")))?;
            if finish > bytes.len() {
                return Err(ZipError::Malformed(format!(
                    "`{name}` runs past the end of the file"
                )));
            }
            let data = if method == STORED {
                if compressed != uncompressed {
                    return Err(ZipError::Malformed(format!(
                        "`{name}` is stored but its sizes disagree"
                    )));
                }
                bytes[start..finish].to_vec()
            } else {
                // Inflated to no more than the size the directory states, so
                // a member that claims a little and inflates to a lot is
                // refused rather than filling memory.
                use std::io::Read;
                let mut data = Vec::with_capacity(uncompressed.min(1 << 26));
                flate2::read::DeflateDecoder::new(&bytes[start..finish])
                    .take(uncompressed as u64 + 1)
                    .read_to_end(&mut data)
                    .map_err(|_| ZipError::Corrupt { name: name.clone() })?;
                if data.len() != uncompressed {
                    return Err(ZipError::Malformed(format!(
                        "`{name}` inflates to {} bytes, not the {uncompressed} its directory says",
                        data.len()
                    )));
                }
                data
            };
            if crc32(&data) != crc {
                return Err(ZipError::Corrupt { name });
            }

            order.push(name.clone());
            members.insert(name, data);
            at += 46 + name_length + extra_length + comment_length;
        }

        Ok(ZipArchive { members, order })
    }

    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.members.get(name).map(|v| v.as_slice())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.members.contains_key(name)
    }

    /// Member names in the order the archive lists them.
    pub fn names(&self) -> &[String] {
        &self.order
    }

    pub fn len(&self) -> usize {
        self.members.len()
    }

    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }
}

fn read_u32(bytes: &[u8], at: usize) -> Result<u32, ZipError> {
    bytes
        .get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| ZipError::Malformed(format!("the file ends before byte {}", at + 4)))
}

/// The end-of-central-directory record is at the end, but a trailing comment
/// may follow it, so it has to be searched for backwards.
/// One member of the archive in `file`, read through its central directory
/// without reading the rest: `None` when there is no member of that name.
///
/// For a small member of a large archive, a project's preview, which a start
/// screen shows ten of where each project can be hundreds of megabytes. The
/// member is at most `limit` bytes, and refused when it says it is more.
pub fn read_member(
    file: &mut (impl std::io::Read + std::io::Seek),
    name: &str,
    limit: usize,
) -> Result<Option<Vec<u8>>, ZipError> {
    use std::io::{Read, SeekFrom};
    let io = |e: std::io::Error| ZipError::Malformed(format!("the archive could not be read: {e}"));
    let length = file.seek(SeekFrom::End(0)).map_err(io)? as usize;
    // The end record, within a comment's length of the end.
    let tail_start = length.saturating_sub(22 + 65535);
    let mut tail = vec![0u8; length - tail_start];
    file.seek(SeekFrom::Start(tail_start as u64)).map_err(io)?;
    file.read_exact(&mut tail).map_err(io)?;
    let end = find_end_of_central_directory(&tail)?;
    let count = u16::from_le_bytes([tail[end + 10], tail[end + 11]]) as usize;
    let directory_size = read_u32(&tail, end + 12)? as usize;
    let directory_offset = read_u32(&tail, end + 16)? as usize;
    if directory_offset + directory_size > length {
        return Err(ZipError::Malformed("the central directory runs past the end of the file".into()));
    }
    let mut directory = vec![0u8; directory_size];
    file.seek(SeekFrom::Start(directory_offset as u64)).map_err(io)?;
    file.read_exact(&mut directory).map_err(io)?;

    let mut at = 0usize;
    for _ in 0..count {
        if at + CENTRAL_HEADER_SIZE > directory.len() || read_u32(&directory, at)? != CENTRAL_HEADER {
            return Err(ZipError::Malformed(format!("expected a directory entry at byte {}", directory_offset + at)));
        }
        let field = |offset: usize| u16::from_le_bytes([directory[at + offset], directory[at + offset + 1]]) as usize;
        let (method, name_length, extra_length, comment_length) = (field(10) as u16, field(28), field(30), field(32));
        let name_end = at + 46 + name_length;
        if name_end > directory.len() {
            return Err(ZipError::Malformed("a member name runs past the directory".into()));
        }
        if &directory[at + 46..name_end] == name.as_bytes() {
            let crc = read_u32(&directory, at + 16)?;
            let compressed = read_u32(&directory, at + 20)? as usize;
            let uncompressed = read_u32(&directory, at + 24)? as usize;
            let local_offset = read_u32(&directory, at + 42)? as usize;
            if method != STORED && method != DEFLATED {
                return Err(ZipError::Unsupported(format!("`{name}` is compressed with method {method}")));
            }
            if compressed > limit || uncompressed > limit {
                return Err(ZipError::Malformed(format!("`{name}` is larger than the {limit} bytes asked for")));
            }
            let mut header = [0u8; LOCAL_HEADER_SIZE];
            file.seek(SeekFrom::Start(local_offset as u64)).map_err(io)?;
            file.read_exact(&mut header).map_err(io)?;
            if read_u32(&header, 0)? != LOCAL_HEADER {
                return Err(ZipError::Malformed(format!("`{name}` has no local header at byte {local_offset}")));
            }
            let skip = u16::from_le_bytes([header[26], header[27]]) as u64 + u16::from_le_bytes([header[28], header[29]]) as u64;
            let start = local_offset as u64 + LOCAL_HEADER_SIZE as u64 + skip;
            if start as usize + compressed > length {
                return Err(ZipError::Malformed(format!("`{name}` runs past the end of the file")));
            }
            let mut stored = vec![0u8; compressed];
            file.seek(SeekFrom::Start(start)).map_err(io)?;
            file.read_exact(&mut stored).map_err(io)?;
            let data = if method == STORED {
                stored
            } else {
                let mut data = Vec::with_capacity(uncompressed);
                flate2::read::DeflateDecoder::new(stored.as_slice())
                    .take(uncompressed as u64 + 1)
                    .read_to_end(&mut data)
                    .map_err(|_| ZipError::Corrupt { name: name.to_string() })?;
                data
            };
            if data.len() != uncompressed || crc32(&data) != crc {
                return Err(ZipError::Corrupt { name: name.to_string() });
            }
            return Ok(Some(data));
        }
        at = name_end + extra_length + comment_length;
    }
    Ok(None)
}

fn find_end_of_central_directory(bytes: &[u8]) -> Result<usize, ZipError> {
    if bytes.len() < 22 {
        return Err(ZipError::Malformed(
            "the file is too short to be a ZIP".into(),
        ));
    }
    // The comment is at most 65535 bytes, so the record is within that of the
    // end.
    let earliest = bytes.len().saturating_sub(22 + 65535);
    for at in (earliest..=bytes.len() - 22).rev() {
        if read_u32(bytes, at)? == END_OF_CENTRAL_DIRECTORY {
            return Ok(at);
        }
    }
    Err(ZipError::Malformed(
        "no end-of-central-directory record; this is not a ZIP archive".into(),
    ))
}

/// CRC-32, the ZIP variant (reflected, polynomial 0xEDB88320).
pub fn crc32(bytes: &[u8]) -> u32 {
    // Built once per call rather than kept in a table: the archives here have
    // a handful of members, and a static table is a lock or a lazy cell for no
    // measurable gain.
    let mut crc = 0xffff_ffffu32;
    for &byte in bytes {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    /// One member read on its own is the member the whole archive holds,
    /// and a member that is not there is `None`, not an error.
    #[test]
    fn one_member_is_read_without_the_rest() {
        let mut writer = super::ZipWriter::new();
        writer.add("scene.json", b"{}".to_vec());
        writer.add("geometry/m.fsmesh", vec![7u8; 100_000]);
        writer.add("preview.png", b"a picture".to_vec());
        let bytes = writer.finish();
        let mut file = std::io::Cursor::new(bytes.clone());
        assert_eq!(super::read_member(&mut file, "preview.png", 1 << 20).unwrap().as_deref(), Some(&b"a picture"[..]));
        assert_eq!(super::read_member(&mut file, "thumbnail.png", 1 << 20).unwrap(), None);
        assert!(super::read_member(&mut file, "geometry/m.fsmesh", 1000).is_err(), "a member over the limit was read");
        let whole = super::ZipArchive::read(&bytes).unwrap();
        assert_eq!(super::read_member(&mut file, "geometry/m.fsmesh", 1 << 20).unwrap().as_deref(), whole.get("geometry/m.fsmesh"));
    }

    use super::*;

    /// More members than plain ZIP can count is refused before writing,
    /// rather than written with the count wrapped round.
    #[test]
    fn an_archive_past_what_zip_can_count_says_so() {
        let mut writer = ZipWriter::new();
        for i in 0..u16::MAX as usize {
            writer.add(format!("m{i}"), Vec::new());
        }
        assert!(writer.too_big().is_none(), "65,535 members fit");
        writer.add("one more", Vec::new());
        assert!(writer.too_big().is_some_and(|why| why.contains("65,535")));
    }

    fn archive() -> Vec<u8> {
        let mut writer = ZipWriter::new();
        writer.add("manifest.json", b"{\"version\":1}".to_vec());
        writer.add("geometry/m_1.fsmesh", vec![0xde, 0xad, 0xbe, 0xef]);
        writer.add("empty", Vec::new());
        writer.finish()
    }

    #[test]
    fn a_written_archive_reads_back_unchanged() {
        let read = ZipArchive::read(&archive()).unwrap();

        assert_eq!(read.len(), 3);
        assert_eq!(read.get("manifest.json").unwrap(), b"{\"version\":1}");
        assert_eq!(
            read.get("geometry/m_1.fsmesh").unwrap(),
            &[0xde, 0xad, 0xbe, 0xef]
        );
        assert_eq!(read.get("empty").unwrap(), b"");
        assert_eq!(read.get("absent"), None);
        assert_eq!(
            read.names(),
            ["manifest.json", "geometry/m_1.fsmesh", "empty"]
        );
    }

    /// The exit criterion is a byte-identical round trip, so two writes of the
    /// same content must agree exactly — including the timestamps, which is
    /// where a wall-clock would leak in.
    #[test]
    fn two_writes_of_the_same_content_are_byte_identical() {
        assert_eq!(archive(), archive());
    }

    /// Known-answer, against the standard CRC-32 check value.
    #[test]
    fn the_checksum_matches_the_reference_values() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(b"a"), 0xe8b7_be43);
    }

    /// A damaged member must be reported, not returned. Silently handing back
    /// corrupted geometry is worse than refusing to open the file.
    #[test]
    fn a_flipped_bit_in_a_member_is_caught() {
        let mut bytes = archive();
        // Find the payload of the first member and change it.
        let at = bytes
            .windows(13)
            .position(|w| w == b"{\"version\":1}")
            .expect("the payload is in there");
        bytes[at] = b'[';

        match ZipArchive::read(&bytes) {
            Err(ZipError::Corrupt { name }) => assert_eq!(name, "manifest.json"),
            other => panic!("expected a corruption error, got {other:?}"),
        }
    }

    #[test]
    fn a_member_that_escapes_its_directory_is_refused() {
        let mut writer = ZipWriter::new();
        writer.add("../../etc/passwd", b"x".to_vec());
        let error = ZipArchive::read(&writer.finish()).unwrap_err();
        assert!(matches!(error, ZipError::Malformed(m) if m.contains("safe member name")));
    }

    /// A ZIP made by an ordinary tool deflates its members, and they read
    /// back as they were written. Made by Python's zipfile.
    #[test]
    fn deflated_members_are_read() {
        const DEFLATED_ZIP: &[u8] = &[80, 75, 3, 4, 20, 0, 0, 0, 8, 0, 251, 133, 83, 93, 167, 57, 72, 47, 73, 0, 0, 0, 89, 0, 0, 0, 18, 0, 0, 0, 115, 104, 97, 114, 101, 100, 47, 112, 108, 117, 103, 105, 110, 46, 106, 115, 111, 110, 171, 86, 202, 76, 81, 178, 82, 74, 206, 207, 213, 75, 173, 72, 204, 45, 200, 73, 213, 43, 46, 44, 77, 77, 173, 74, 77, 81, 210, 81, 202, 75, 204, 77, 85, 178, 82, 10, 70, 136, 228, 38, 102, 230, 41, 89, 129, 41, 189, 130, 74, 37, 29, 144, 206, 220, 196, 188, 148, 98, 37, 171, 232, 106, 136, 89, 233, 249, 74, 181, 177, 181, 0, 80, 75, 3, 4, 20, 0, 0, 0, 8, 0, 251, 133, 83, 93, 107, 135, 187, 190, 30, 0, 0, 0, 184, 1, 0, 0, 14, 0, 0, 0, 115, 104, 97, 114, 101, 100, 47, 109, 97, 105, 110, 46, 112, 121, 203, 204, 45, 200, 47, 42, 81, 40, 73, 45, 44, 205, 204, 73, 140, 47, 200, 41, 77, 207, 204, 227, 202, 28, 21, 45, 29, 220, 162, 0, 80, 75, 1, 2, 20, 0, 20, 0, 0, 0, 8, 0, 251, 133, 83, 93, 167, 57, 72, 47, 73, 0, 0, 0, 89, 0, 0, 0, 18, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 128, 1, 0, 0, 0, 0, 115, 104, 97, 114, 101, 100, 47, 112, 108, 117, 103, 105, 110, 46, 106, 115, 111, 110, 80, 75, 1, 2, 20, 0, 20, 0, 0, 0, 8, 0, 251, 133, 83, 93, 107, 135, 187, 190, 30, 0, 0, 0, 184, 1, 0, 0, 14, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 128, 1, 121, 0, 0, 0, 115, 104, 97, 114, 101, 100, 47, 109, 97, 105, 110, 46, 112, 121, 80, 75, 5, 6, 0, 0, 0, 0, 2, 0, 2, 0, 124, 0, 0, 0, 195, 0, 0, 0, 0, 0];
        let archive = ZipArchive::read(DEFLATED_ZIP).expect("reads");
        assert_eq!(archive.names(), ["shared/plugin.json", "shared/main.py"]);
        assert!(String::from_utf8_lossy(archive.get("shared/plugin.json").unwrap()).contains("com.example.squeezed"));
        assert_eq!(archive.get("shared/main.py").unwrap(), "import tequila_plugin\n".repeat(20).as_bytes());
    }

    /// A member that claims to be deflated but is not is refused, not read
    /// as whatever bytes it holds; a method with no decoder says so.
    #[test]
    fn a_member_that_does_not_inflate_is_refused() {
        let mut bytes = archive();
        // Set the compression method to deflate in the first directory entry.
        let directory = bytes
            .windows(4)
            .rposition(|w| w == CENTRAL_HEADER.to_le_bytes())
            .unwrap();
        // rposition finds the last entry; walk back to the first.
        let first = bytes
            .windows(4)
            .position(|w| w == CENTRAL_HEADER.to_le_bytes())
            .unwrap();
        assert!(first <= directory);
        bytes[first + 10] = 8;
        assert!(ZipArchive::read(&bytes).is_err(), "stored bytes read as deflated");

        bytes[first + 10] = 12;
        let error = ZipArchive::read(&bytes).unwrap_err();
        assert!(matches!(error, ZipError::Unsupported(_)), "{error}");
    }

    #[test]
    fn malformed_archives_are_errors_and_never_panics() {
        let good = archive();
        assert!(ZipArchive::read(b"").is_err());
        assert!(ZipArchive::read(b"not a zip at all, not even close").is_err());
        // Truncated at every length: none may panic.
        for cut in 0..good.len() {
            let _ = ZipArchive::read(&good[..cut]);
        }
        // And with garbage appended.
        let mut extended = good.clone();
        extended.extend_from_slice(&[0xff; 64]);
        let _ = ZipArchive::read(&extended);
    }

    /// Four bytes that happen to spell a directory signature, with no room
    /// for the entry that should follow. Checking the signature alone and then
    /// reading the fields would index straight past the buffer.
    #[test]
    fn a_directory_signature_at_the_very_end_is_not_read_past() {
        let mut bytes = archive();
        let end = find_end_of_central_directory(&bytes).unwrap();
        // Point the directory at the last four bytes of the file, and write a
        // signature there.
        let at = bytes.len() - 4;
        bytes[at..].copy_from_slice(&CENTRAL_HEADER.to_le_bytes());
        bytes[end + 16..end + 20].copy_from_slice(&(at as u32).to_le_bytes());
        bytes[end + 12..end + 16].copy_from_slice(&4u32.to_le_bytes());
        bytes[end + 10] = 1;

        let error = ZipArchive::read(&bytes).unwrap_err();
        assert!(
            matches!(error, ZipError::Malformed(m) if m.contains("past the end")),
            "expected a bounds error"
        );
    }

    /// The same for a member whose local header offset points off the end.
    #[test]
    fn a_local_header_offset_past_the_end_is_refused() {
        let mut bytes = archive();
        let directory = bytes
            .windows(4)
            .position(|w| w == CENTRAL_HEADER.to_le_bytes())
            .unwrap();
        let past = (bytes.len() as u32) - 2;
        bytes[directory + 42..directory + 46].copy_from_slice(&past.to_le_bytes());

        assert!(ZipArchive::read(&bytes).is_err());
    }

    /// A directory entry claiming more members than exist must not read past
    /// the end.
    #[test]
    fn a_lying_member_count_is_caught() {
        let mut bytes = archive();
        let end = find_end_of_central_directory(&bytes).unwrap();
        bytes[end + 10] = 99;
        assert!(ZipArchive::read(&bytes).is_err());
    }
}
