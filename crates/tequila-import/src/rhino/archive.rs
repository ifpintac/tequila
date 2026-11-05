//! The 3dm container as openNURBS writes it: a 32-byte start, then
//! typecoded chunks, inside which every value is little-endian.
//!
//! A chunk is a 4-byte typecode and a value: 4 bytes in archives before
//! version 50 (Rhino 4 and the 32-bit Rhino 5), 8 bytes from version 50 on
//! (Rhino 5 64-bit to Rhino 8). A typecode with the short bit set carries
//! its data in that value and has no body; any other's value is the length
//! of the body that follows, which ends with a CRC when the typecode has
//! the CRC bit. Everything here reads forwards from a body's start and
//! jumps over the rest by its length, so neither a CRC nor a field added by
//! a later Rhino gets in the way.
//!
//! Two ways of stamping a version recur, and they are easy to confuse:
//! openNURBS's `Read3dmChunkVersion` is one byte, major in the high nibble
//! and minor in the low ([`Reader::version`]), and `BeginRead3dmChunk` with
//! a version is an anonymous chunk whose body starts with two 32-bit
//! integers ([`Reader::anonymous`]).

pub(super) type R<T> = Result<T, String>;

pub(super) const TCODE_SHORT: u32 = 0x8000_0000;
pub(super) const TCODE_ENDOFFILE: u32 = 0x0000_7FFF;
pub(super) const TCODE_ENDOFTABLE: u32 = 0xFFFF_FFFF;
pub(super) const TCODE_ANONYMOUS_CHUNK: u32 = 0x4000_8000;
pub(super) const TCODE_MODEL_ATTRIBUTES_CHUNK: u32 = 0x4000_8002;

pub(super) const TCODE_MATERIAL_TABLE: u32 = 0x1000_0010;
pub(super) const TCODE_LAYER_TABLE: u32 = 0x1000_0011;
pub(super) const TCODE_OBJECT_TABLE: u32 = 0x1000_0013;
pub(super) const TCODE_PROPERTIES_TABLE: u32 = 0x1000_0014;
pub(super) const TCODE_SETTINGS_TABLE: u32 = 0x1000_0015;
pub(super) const TCODE_INSTANCE_DEFINITION_TABLE: u32 = 0x1000_0021;

pub(super) const TCODE_PROPERTIES_OPENNURBS_VERSION: u32 = 0xA000_0026;
pub(super) const TCODE_SETTINGS_UNITSANDTOLS: u32 = 0x2000_8031;
pub(super) const TCODE_MATERIAL_RECORD: u32 = 0x2000_8040;
pub(super) const TCODE_LAYER_RECORD: u32 = 0x2000_8050;
pub(super) const TCODE_OBJECT_RECORD: u32 = 0x2000_8070;
pub(super) const TCODE_INSTANCE_DEFINITION_RECORD: u32 = 0x2000_8076;
pub(super) const TCODE_OBJECT_RECORD_ATTRIBUTES: u32 = 0x0200_8072;
pub(super) const TCODE_OBJECT_RECORD_END: u32 = 0x8200_007F;

pub(super) const TCODE_OPENNURBS_CLASS: u32 = 0x0002_7FFA;
const TCODE_OPENNURBS_CLASS_UUID: u32 = 0x0002_FFFB;
const TCODE_OPENNURBS_CLASS_DATA: u32 = 0x0002_FFFC;
const TCODE_OPENNURBS_CLASS_USERDATA: u32 = 0x0002_7FFD;
const TCODE_OPENNURBS_CLASS_USERDATA_HEADER: u32 = 0x0002_FFF9;
const TCODE_OPENNURBS_CLASS_END: u32 = 0x8002_7FFF;

/// The most items any one count in the file may claim. A count is checked
/// against the bytes left as well; this only keeps a corrupt one from
/// asking for an allocation the bytes could never fill.
const MOST_ITEMS: usize = 1 << 28;

/// A class UUID, in the order its text is written
/// (`4ED7D4E4-E947-11d3-...` is `[0x4e, 0xd7, 0xd4, 0xe4, 0xe9, 0x47, ...]`).
/// The file stores the first three fields little-endian, as Windows'
/// `GUID` lays them out in memory.
pub(super) type Uuid = [u8; 16];

pub(super) const NIL: Uuid = [0; 16];

/// A UUID from its text, for the class constants.
pub(super) const fn uuid(text: &str) -> Uuid {
    let bytes = text.as_bytes();
    let mut out = [0u8; 16];
    let (mut i, mut n) = (0, 0);
    while i < bytes.len() {
        let c = bytes[i];
        i += 1;
        let v = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => continue,
        };
        out[n / 2] |= if n % 2 == 0 { v << 4 } else { v };
        n += 1;
    }
    out
}

/// A cursor over one chunk body, or the file.
#[derive(Clone, Copy)]
pub(super) struct Reader<'a> {
    data: &'a [u8],
    pub pos: usize,
    pub end: usize,
    /// Whether chunk values are eight bytes: archives of version 50 on.
    pub big: bool,
    /// The archive's version: 2 to 5, then 50, 60, 70, 80.
    pub archive: u32,
    /// The openNURBS version that wrote the file, which a few fields are
    /// gated on (`ArchiveOpenNURBSVersion()`). Zero when unknown.
    pub opennurbs: u64,
}

/// One chunk: its typecode, its value, and its body (empty for a short
/// chunk, whose value is its data).
pub(super) struct Chunk<'a> {
    pub tcode: u32,
    pub value: i64,
    pub body: Reader<'a>,
}

/// A polymorphic object as `ReadObject` finds it: its class, its own data,
/// and the user data attached to it.
pub(super) struct Object<'a> {
    pub class: Uuid,
    pub data: Reader<'a>,
    pub userdata: Vec<UserData<'a>>,
}

/// A piece of user data on an object: whose it is and its payload, the
/// body of the anonymous chunk that holds it.
pub(super) struct UserData<'a> {
    pub class: Uuid,
    pub payload: Reader<'a>,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8], pos: usize, archive: u32) -> Reader<'a> {
        Reader { data, pos, end: data.len(), big: archive >= 50, archive, opennurbs: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.end.saturating_sub(self.pos)
    }

    /// The same bytes, empty: what a short chunk's body is.
    fn empty(&self) -> Reader<'a> {
        Reader { pos: self.pos, end: self.pos, ..*self }
    }

    pub fn take(&mut self, n: usize) -> R<&'a [u8]> {
        if n > self.remaining() {
            return Err(format!("the data ends {} bytes early at byte {}", n - self.remaining(), self.pos));
        }
        let out = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }

    /// The next four bytes as a typecode, without moving past them.
    pub fn peek_u32(&self) -> Option<u32> {
        (self.remaining() >= 4).then(|| u32::from_le_bytes(self.data[self.pos..self.pos + 4].try_into().unwrap()))
    }

    pub fn skip(&mut self, n: usize) -> R<()> {
        self.take(n).map(|_| ())
    }

    fn array<const N: usize>(&mut self) -> R<[u8; N]> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    pub fn u8(&mut self) -> R<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn bool(&mut self) -> R<bool> {
        Ok(self.u8()? != 0)
    }

    pub fn u16(&mut self) -> R<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub fn i32(&mut self) -> R<i32> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    pub fn u32(&mut self) -> R<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub fn f32(&mut self) -> R<f32> {
        Ok(f32::from_le_bytes(self.array()?))
    }

    pub fn f64(&mut self) -> R<f64> {
        Ok(f64::from_le_bytes(self.array()?))
    }

    /// A finite double, or the error says which.
    pub fn finite(&mut self, what: &str) -> R<f64> {
        let at = self.pos;
        let v = self.f64()?;
        if v.is_finite() { Ok(v) } else { Err(format!("{what} at byte {at} is not a number")) }
    }

    pub fn uuid(&mut self) -> R<Uuid> {
        let b: [u8; 16] = self.array()?;
        Ok([b[3], b[2], b[1], b[0], b[5], b[4], b[7], b[6], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]])
    }

    pub fn point(&mut self) -> R<[f64; 3]> {
        Ok([self.f64()?, self.f64()?, self.f64()?])
    }

    pub fn interval(&mut self) -> R<[f64; 2]> {
        Ok([self.f64()?, self.f64()?])
    }

    /// `ON_Plane`: origin, x, y and z axes, then the plane equation; the
    /// last two follow from the rest and are skipped.
    pub fn plane(&mut self) -> R<Plane> {
        let plane = Plane { origin: self.point()?, x: self.point()?, y: self.point()? };
        self.skip(24 + 32)?;
        Ok(plane)
    }

    /// `ON_BoundingBox`, which every reader here can do without.
    pub fn bbox(&mut self) -> R<()> {
        self.skip(48)
    }

    /// `ON_Xform`: sixteen doubles, row by row, translation in the last
    /// column - the same layout as Tequila's own matrices.
    pub fn xform(&mut self) -> R<[[f64; 4]; 4]> {
        let mut m = [[0.0; 4]; 4];
        for row in &mut m {
            for v in row.iter_mut() {
                *v = self.f64()?;
            }
        }
        Ok(m)
    }

    /// A count, checked against the bytes left at `size` bytes an item.
    pub fn count(&mut self, size: usize) -> R<usize> {
        let at = self.pos;
        let n = self.i32()?;
        let n = usize::try_from(n).map_err(|_| format!("a negative count ({n}) at byte {at}"))?;
        if n > MOST_ITEMS || n.saturating_mul(size.max(1)) > self.remaining() {
            return Err(format!("a count of {n} at byte {at} claims more than the data holds"));
        }
        Ok(n)
    }

    /// A count that only has to be no more than `limit`: one whose items
    /// are stored compressed, so the bytes left say nothing about it.
    pub fn count_at_most(&mut self, limit: usize) -> R<usize> {
        let at = self.pos;
        let n = self.i32()?;
        usize::try_from(n).ok().filter(|&n| n <= limit).ok_or_else(|| format!("a count of {n} at byte {at}"))
    }

    /// `ReadArray` of 32-bit integers: a count, then the values.
    pub fn int_array(&mut self) -> R<Vec<i32>> {
        let n = self.count(4)?;
        (0..n).map(|_| self.i32()).collect()
    }

    /// `ReadArray` of doubles.
    pub fn double_array(&mut self) -> R<Vec<f64>> {
        let n = self.count(8)?;
        (0..n).map(|_| self.f64()).collect()
    }

    /// `ON_wString`: a count of UTF-16 units including the closing nul,
    /// then the units.
    pub fn string(&mut self) -> R<String> {
        let at = self.pos;
        let n = self.u32()? as usize;
        if n.saturating_mul(2) > self.remaining() {
            return Err(format!("a string at byte {at} claims {n} characters, more than the data holds"));
        }
        let units: Vec<u16> = self.take(n * 2)?.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let text = String::from_utf16_lossy(&units);
        Ok(text.trim_end_matches('\0').to_string())
    }

    /// `Read3dmChunkVersion`: one byte, (major, minor).
    pub fn version(&mut self) -> R<(u8, u8)> {
        let v = self.u8()?;
        Ok((v >> 4, v & 0x0F))
    }

    /// The next chunk, and the cursor past it.
    pub fn chunk(&mut self) -> R<Chunk<'a>> {
        let at = self.pos;
        let tcode = self.u32()?;
        let value = if self.big { i64::from_le_bytes(self.array()?) } else { i64::from(self.i32()?) };
        if tcode & TCODE_SHORT != 0 {
            return Ok(Chunk { tcode, value, body: self.empty() });
        }
        let length = usize::try_from(value).ok().filter(|&n| n <= self.remaining()).ok_or_else(|| {
            format!("a chunk at byte {at} (typecode {tcode:#010x}) claims {value} bytes, more than are left ({})", self.remaining())
        })?;
        let body = Reader { pos: self.pos, end: self.pos + length, ..*self };
        self.pos += length;
        Ok(Chunk { tcode, value, body })
    }

    /// The next chunk, which must have typecode `tcode`.
    pub fn chunk_of(&mut self, tcode: u32, what: &str) -> R<Chunk<'a>> {
        let at = self.pos;
        let chunk = self.chunk()?;
        if chunk.tcode != tcode {
            return Err(format!("expected {what} at byte {at}, found a chunk of typecode {:#010x}", chunk.tcode));
        }
        Ok(chunk)
    }

    /// `BeginRead3dmChunk(TCODE_ANONYMOUS_CHUNK, &major, &minor)`: an
    /// anonymous chunk whose body starts with two integers. Returns the body
    /// past them, and the version.
    pub fn anonymous(&mut self, what: &str) -> R<(Reader<'a>, i32, i32)> {
        let mut body = self.chunk_of(TCODE_ANONYMOUS_CHUNK, what)?.body;
        let major = body.i32()?;
        let minor = body.i32()?;
        Ok((body, major, minor))
    }

    /// `ReadObject`: the class wrapper, its UUID, its data and its user data.
    pub fn object(&mut self) -> R<Object<'a>> {
        let mut wrapper = self.chunk_of(TCODE_OPENNURBS_CLASS, "an object")?.body;
        let class = canonical(wrapper.chunk_of(TCODE_OPENNURBS_CLASS_UUID, "an object's class")?.body.uuid()?);
        if class == NIL {
            return Ok(Object { class, data: wrapper.empty(), userdata: Vec::new() });
        }
        let data = wrapper.chunk_of(TCODE_OPENNURBS_CLASS_DATA, "an object's data")?.body;
        let mut userdata = Vec::new();
        while wrapper.remaining() > 0 {
            let chunk = wrapper.chunk()?;
            match chunk.tcode {
                TCODE_OPENNURBS_CLASS_END => break,
                TCODE_OPENNURBS_CLASS_USERDATA => {
                    // User data nobody here reads must not cost the object.
                    if let Ok(u) = user_data(chunk.body) {
                        userdata.push(u);
                    }
                }
                _ => {}
            }
        }
        Ok(Object { class, data, userdata })
    }

    /// `ReadCompressedBuffer`: the uncompressed size, its CRC, a method
    /// byte, then the bytes as they are (0) or deflated in an anonymous
    /// chunk (1). `None` for a buffer of no bytes. A buffer whose size is
    /// not `expected` is consumed and refused.
    pub fn compressed(&mut self, expected: usize, what: &str) -> R<Option<Vec<u8>>> {
        let size = self.u32()? as usize;
        if size == 0 {
            return Ok(None);
        }
        let crc = self.u32()?;
        let method = self.u8()?;
        let bytes = match method {
            0 => self.take(size)?.to_vec(),
            1 => {
                let chunk = self.chunk_of(TCODE_ANONYMOUS_CHUNK, "a compressed buffer")?;
                let deflated = &chunk.body.data[chunk.body.pos..chunk.body.end];
                if size > MOST_ITEMS || size > deflated.len().saturating_mul(1100) + 4096 {
                    return Err(format!("the {what} claim to inflate to {size} bytes"));
                }
                let mut out = Vec::with_capacity(size);
                use std::io::Read;
                flate2::read::ZlibDecoder::new(deflated)
                    .take(size as u64)
                    .read_to_end(&mut out)
                    .map_err(|e| format!("the {what} do not inflate: {e}"))?;
                out
            }
            m => return Err(format!("the {what} use compression method {m}, which is not openNURBS's")),
        };
        if bytes.len() != size {
            return Err(format!("the {what} inflate to {} bytes, not the {size} they claim", bytes.len()));
        }
        if crc32(&bytes) != crc {
            return Err(format!("the {what} fail their checksum"));
        }
        if size != expected {
            return Ok(None);
        }
        Ok(Some(bytes))
    }
}

/// The class an object is read as. Rhino saves its own subclasses of a few
/// openNURBS classes (`TL_Brep`, `TL_NurbsCurve` and the rest), and older
/// files name retired classes, all with the same data as the openNURBS
/// class: the table openNURBS itself keeps in `ON_ClassId::ClassId`
/// (opennurbs_object.cpp), so a file Rhino wrote reads as one openNURBS
/// wrote.
fn canonical(class: Uuid) -> Uuid {
    const NURBS_CURVE: Uuid = uuid("4ED7D4DD-E947-11d3-BFE5-0010830122F0");
    const NURBS_SURFACE: Uuid = uuid("4ED7D4DE-E947-11d3-BFE5-0010830122F0");
    const POLY_CURVE: Uuid = uuid("4ED7D4E0-E947-11d3-BFE5-0010830122F0");
    const BREP: Uuid = uuid("60B5DBC5-E660-11d3-BFE4-0010830122F0");
    const REV_SURFACE: Uuid = uuid("A16220D3-163B-11d4-8000-0010830122F0");
    const SUM_SURFACE: Uuid = uuid("C4CD5359-446D-4690-9FF5-29059732472B");
    const ALIASES: [(Uuid, Uuid); 10] = [
        // TL_NurbsCurve, and an old NURBS curve.
        (uuid("5EAF1119-0B51-11d4-BFFE-0010830122F0"), NURBS_CURVE),
        (uuid("76A709D5-1550-11d4-8000-0010830122F0"), NURBS_CURVE),
        // TL_NurbsSurface, and an old NURBS surface.
        (uuid("4760C817-0BE3-11d4-BFFE-0010830122F0"), NURBS_SURFACE),
        (uuid("FA4FD4B5-1613-11d4-8000-0010830122F0"), NURBS_SURFACE),
        // An old polycurve.
        (uuid("EF638317-154B-11d4-8000-0010830122F0"), POLY_CURVE),
        // An old trimmed surface, an old B-rep, and TL_Brep.
        (uuid("0705FDEF-3E2A-11d4-800E-0010830122F0"), BREP),
        (uuid("2D4CFEDB-3E2A-11d4-800E-0010830122F0"), BREP),
        (uuid("F06FC243-A32A-4608-9DD8-A7D2C4CE2A36"), BREP),
        // TL_RevSurface and TL_SumSurface.
        (uuid("0A8401B6-4D34-4b99-8615-1B4E723DC4E5"), REV_SURFACE),
        (uuid("665F6331-2A66-4cce-81D0-B5EEBD9B5417"), SUM_SURFACE),
    ];
    ALIASES.iter().find(|(alias, _)| *alias == class).map_or(class, |(_, to)| *to)
}

/// `ON_Plane`'s origin and its axes in the plane.
#[derive(Clone, Copy, Debug)]
pub(super) struct Plane {
    pub origin: [f64; 3],
    pub x: [f64; 3],
    pub y: [f64; 3],
}

/// One `TCODE_OPENNURBS_CLASS_USERDATA` chunk: a version byte, then for
/// version 1 the class and item UUIDs, a copy count and a transform, and
/// for version 2 the same in a header chunk of their own (with the
/// application's UUID and more after them); then the payload, an
/// anonymous chunk.
fn user_data(mut body: Reader<'_>) -> R<UserData<'_>> {
    let (major, _) = body.version()?;
    let class = match major {
        1 => {
            let class = body.uuid()?;
            body.skip(16 + 4 + 128)?;
            class
        }
        2 => body.chunk_of(TCODE_OPENNURBS_CLASS_USERDATA_HEADER, "a user data header")?.body.uuid()?,
        v => return Err(format!("user data of version {v}")),
    };
    let payload = body.chunk_of(TCODE_ANONYMOUS_CHUNK, "user data")?.body;
    Ok(UserData { class, payload })
}

/// The CRC-32 openNURBS checks a compressed buffer with: zlib's.
pub(super) fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = flate2::Crc::new();
    crc.update(bytes);
    crc.sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuids_read_in_the_order_their_text_is_written() {
        // ON_Layer's class, as block.3dm-style files store it on disk.
        let disk = [0x13, 0x98, 0x80, 0x95, 0x85, 0xe9, 0xd3, 0x11, 0xbf, 0xe5, 0x00, 0x10, 0x83, 0x01, 0x22, 0xf0];
        let mut r = Reader::new(&disk, 0, 70);
        assert_eq!(r.uuid().unwrap(), uuid("95809813-E985-11d3-BFE5-0010830122F0"));
    }

    #[test]
    fn rhinos_own_classes_read_as_opennurbs_ones() {
        assert_eq!(canonical(uuid("F06FC243-A32A-4608-9DD8-A7D2C4CE2A36")), uuid("60B5DBC5-E660-11d3-BFE4-0010830122F0"), "TL_Brep");
        assert_eq!(canonical(uuid("5EAF1119-0B51-11d4-BFFE-0010830122F0")), uuid("4ED7D4DD-E947-11d3-BFE5-0010830122F0"), "TL_NurbsCurve");
        let mesh = uuid("4ED7D4E4-E947-11d3-BFE5-0010830122F0");
        assert_eq!(canonical(mesh), mesh);
    }

    #[test]
    fn crc32_is_zlibs() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn a_chunk_that_claims_more_than_is_left_is_refused() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&TCODE_ANONYMOUS_CHUNK.to_le_bytes());
        bytes.extend_from_slice(&1000i64.to_le_bytes());
        bytes.extend_from_slice(&[0; 10]);
        let mut r = Reader::new(&bytes, 0, 70);
        assert!(r.chunk().is_err());
        // A short chunk has no body whatever its value says.
        let mut short = Vec::new();
        short.extend_from_slice(&TCODE_ENDOFTABLE.to_le_bytes());
        short.extend_from_slice(&1000i64.to_le_bytes());
        let mut r = Reader::new(&short, 0, 70);
        let chunk = r.chunk().unwrap();
        assert_eq!((chunk.tcode, chunk.value, chunk.body.remaining()), (TCODE_ENDOFTABLE, 1000, 0));
    }
}
