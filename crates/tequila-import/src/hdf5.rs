//! Just enough HDF5 to read what X-Rite AxF files keep their measured
//! materials in: groups old and new (symbol tables, links in the object
//! header, and links in a fractal heap indexed by a v2 B-tree), and
//! datasets of numbers and strings - compact, contiguous, or chunked under
//! a v1 B-tree, shuffled and deflated.
//!
//! Read-only and bounds-checked: a file that is not what it should be is
//! an error, never a panic. Written against the HDF5 file format
//! specification, version 3.0; nothing of the HDF Group's library is used.

use std::io::Read;

const UNDEFINED: u64 = u64::MAX;

/// An HDF5 file, in memory.
pub struct File<'a> {
    data: &'a [u8],
    /// Sizes of addresses and of lengths, in bytes.
    offsets: usize,
    lengths: usize,
    root: u64,
}

/// A dataset's values.
#[derive(Clone, Debug, PartialEq)]
pub enum Values {
    Numbers(Vec<f64>),
    Strings(Vec<String>),
}

/// A dataset: its shape and values.
#[derive(Clone, Debug, PartialEq)]
pub struct Dataset {
    pub shape: Vec<usize>,
    pub values: Values,
}

impl Dataset {
    pub fn numbers(&self) -> Option<&[f64]> {
        match &self.values {
            Values::Numbers(v) => Some(v),
            Values::Strings(_) => None,
        }
    }

    pub fn text(&self) -> Option<&str> {
        match &self.values {
            Values::Strings(v) => v.first().map(String::as_str),
            Values::Numbers(_) => None,
        }
    }
}

type Result<T> = std::result::Result<T, String>;

struct Message {
    kind: u16,
    at: usize,
    size: usize,
}

impl<'a> File<'a> {
    pub fn open(data: &'a [u8]) -> Result<File<'a>> {
        // The superblock is at 0, or at 512, 1024, ... in a file with a
        // user block before it.
        let mut base = 0usize;
        while data.get(base..base + 8) != Some(b"\x89HDF\r\n\x1a\n".as_slice()) {
            base = if base == 0 { 512 } else { base * 2 };
            if base >= data.len() {
                return Err("not an HDF5 file".into());
            }
        }
        let version = *data.get(base + 8).ok_or("truncated superblock")?;
        let mut file = File { data, offsets: 8, lengths: 8, root: 0 };
        match version {
            0 | 1 => {
                file.offsets = file.byte(base + 13)? as usize;
                file.lengths = file.byte(base + 14)? as usize;
                // Then the group leaf and internal K, flags, (v1: indexed
                // storage K and two reserved), four addresses, and the root
                // group's symbol table entry, whose second field is its
                // object header.
                let addresses = base + 24 + if version == 1 { 4 } else { 0 };
                let entry = addresses + 4 * file.offsets;
                file.root = file.uint(entry + file.offsets, file.offsets)?;
            }
            2 | 3 => {
                file.offsets = file.byte(base + 9)? as usize;
                file.lengths = file.byte(base + 10)? as usize;
                // Base, extension, end of file, root object header.
                file.root = file.uint(base + 12 + 3 * file.offsets, file.offsets)?;
            }
            other => return Err(format!("HDF5 superblock version {other} is not known")),
        }
        if !matches!(file.offsets, 2 | 4 | 8) || !matches!(file.lengths, 2 | 4 | 8) {
            return Err("HDF5 sizes of addresses or lengths are not 2, 4 or 8".into());
        }
        Ok(file)
    }

    // ---- bytes -----------------------------------------------------------------

    fn bytes(&self, at: usize, n: usize) -> Result<&'a [u8]> {
        at.checked_add(n).and_then(|end| self.data.get(at..end)).ok_or_else(|| format!("HDF5 data runs past the end of the file at {at}"))
    }

    fn byte(&self, at: usize) -> Result<u8> {
        self.data.get(at).copied().ok_or_else(|| "HDF5 data runs past the end of the file".to_string())
    }

    fn uint(&self, at: usize, n: usize) -> Result<u64> {
        let b = self.bytes(at, n)?;
        Ok(b.iter().rev().fold(0u64, |v, &x| (v << 8) | x as u64))
    }

    fn address(&self, at: usize) -> Result<u64> {
        let a = self.uint(at, self.offsets)?;
        Ok(if self.offsets < 8 && a == (1u64 << (8 * self.offsets)) - 1 { UNDEFINED } else { a })
    }

    fn signature(&self, at: u64, expected: &[u8; 4]) -> Result<usize> {
        let at = usize::try_from(at).map_err(|_| "HDF5 address out of range".to_string())?;
        if self.bytes(at, 4)? != expected {
            return Err(format!("HDF5 structure at {at} is not a {}", String::from_utf8_lossy(expected)));
        }
        Ok(at)
    }

    // ---- object headers --------------------------------------------------------------

    fn messages(&self, address: u64) -> Result<Vec<Message>> {
        let at = usize::try_from(address).map_err(|_| "HDF5 address out of range".to_string())?;
        let mut out = Vec::new();
        if self.bytes(at, 4)? == b"OHDR" {
            let flags = self.byte(at + 5)?;
            let mut p = at + 6;
            if flags & 0x20 != 0 {
                p += 16;
            }
            if flags & 0x10 != 0 {
                p += 4;
            }
            let size_bytes = 1usize << (flags & 3);
            let first = self.uint(p, size_bytes)? as usize;
            p += size_bytes;
            let order = flags & 0x04 != 0;
            let mut chunks = vec![(p, first)];
            let mut guard = 0;
            while let Some((start, length)) = chunks.pop() {
                guard += 1;
                if guard > 10_000 {
                    return Err("HDF5 object header continues without end".into());
                }
                let end = start + length;
                let mut q = start;
                while q + 4 <= end {
                    let kind = self.byte(q)? as u16;
                    let size = self.uint(q + 1, 2)? as usize;
                    q += 4 + if order { 2 } else { 0 };
                    if q + size > end {
                        break;
                    }
                    if kind == 0x10 {
                        let address = self.address(q)?;
                        let length = self.uint(q + self.offsets, self.lengths)? as usize;
                        let block = self.signature(address, b"OCHK")?;
                        // Its own signature in, its checksum out.
                        chunks.push((block + 4, length.saturating_sub(8)));
                    } else if kind != 0 {
                        out.push(Message { kind, at: q, size });
                    }
                    q += size;
                }
            }
        } else {
            if self.byte(at)? != 1 {
                return Err(format!("HDF5 object header at {at} has an unknown version"));
            }
            let size = self.uint(at + 8, 4)? as usize;
            let mut chunks = vec![(at + 16, size)];
            let mut guard = 0;
            while let Some((start, length)) = chunks.pop() {
                guard += 1;
                if guard > 10_000 {
                    return Err("HDF5 object header continues without end".into());
                }
                let end = start + length;
                let mut q = start;
                while q + 8 <= end {
                    let kind = self.uint(q, 2)? as u16;
                    let size = self.uint(q + 2, 2)? as usize;
                    q += 8;
                    if kind == 0x10 {
                        let address = self.address(q)?;
                        let length = self.uint(q + self.offsets, self.lengths)? as usize;
                        chunks.push((address as usize, length));
                    } else if kind != 0 {
                        out.push(Message { kind, at: q, size });
                    }
                    q += size;
                }
            }
        }
        Ok(out)
    }

    // ---- groups ----------------------------------------------------------------

    /// The object at `path` (names apart by `/`) from the root.
    pub fn find(&self, path: &str) -> Result<u64> {
        let mut at = self.root;
        for name in path.split('/').filter(|n| !n.is_empty()) {
            at = self
                .members(at)?
                .into_iter()
                .find(|(n, _)| n == name)
                .map(|(_, a)| a)
                .ok_or_else(|| format!("no `{name}` in the file's `{path}`"))?;
        }
        Ok(at)
    }

    /// Whether `path` names something.
    pub fn has(&self, path: &str) -> bool {
        self.find(path).is_ok()
    }

    /// A group's members, by name.
    pub fn members(&self, group: u64) -> Result<Vec<(String, u64)>> {
        let mut out = Vec::new();
        for m in self.messages(group)? {
            match m.kind {
                0x06 => {
                    if let Some(link) = self.link(self.bytes(m.at, m.size)?)? {
                        out.push(link);
                    }
                }
                0x02 => {
                    let flags = self.byte(m.at + 1)?;
                    let mut p = m.at + 2;
                    if flags & 1 != 0 {
                        p += 8;
                    }
                    let heap = self.address(p)?;
                    let names = self.address(p + self.offsets)?;
                    if heap != UNDEFINED && names != UNDEFINED {
                        let heap = Heap::open(self, heap)?;
                        for record in self.btree2(names)? {
                            // A name's record: its hash, then its heap id.
                            let id = record.get(4..).ok_or("short link record")?;
                            if let Some(link) = self.link(&heap.object(self, id)?)? {
                                out.push(link);
                            }
                        }
                    }
                }
                0x11 => {
                    let tree = self.address(m.at)?;
                    let heap = self.address(m.at + self.offsets)?;
                    self.symbol_table(tree, heap, &mut out, 0)?;
                }
                _ => {}
            }
        }
        Ok(out)
    }

    /// A link message's name and the object it is a hard link to.
    fn link(&self, b: &[u8]) -> Result<Option<(String, u64)>> {
        let get = |i: usize| b.get(i).copied().ok_or_else(|| "short link".to_string());
        let flags = get(1)?;
        let mut p = 2;
        let mut kind = 0;
        if flags & 0x08 != 0 {
            kind = get(p)?;
            p += 1;
        }
        if flags & 0x04 != 0 {
            p += 8;
        }
        if flags & 0x10 != 0 {
            p += 1;
        }
        let size = 1usize << (flags & 3);
        let length = (0..size).try_fold(0usize, |v, i| Ok::<_, String>(v | (get(p + i)? as usize) << (8 * i)))?;
        p += size;
        let name = String::from_utf8_lossy(b.get(p..p + length).ok_or("short link name")?).into_owned();
        p += length;
        if kind != 0 {
            return Ok(None);
        }
        let address = (0..self.offsets).try_fold(0u64, |v, i| Ok::<_, String>(v | (get(p + i)? as u64) << (8 * i)))?;
        Ok(Some((name, address)))
    }

    /// An old-style group: a v1 B-tree of symbol table nodes, names in a
    /// local heap.
    fn symbol_table(&self, node: u64, heap: u64, out: &mut Vec<(String, u64)>, depth: u32) -> Result<()> {
        if depth > 64 {
            return Err("HDF5 group tree too deep".into());
        }
        let heap_at = self.signature(heap, b"HEAP")?;
        let names = self.address(heap_at + 8 + 2 * self.lengths)?;
        let at = self.signature(node, b"TREE")?;
        let level = self.byte(at + 5)?;
        let entries = self.uint(at + 6, 2)? as usize;
        let mut p = at + 8 + 2 * self.offsets + self.lengths;
        for _ in 0..entries {
            let child = self.address(p)?;
            p += self.offsets + self.lengths;
            if level > 0 {
                self.symbol_table(child, heap, out, depth + 1)?;
                continue;
            }
            let snod = self.signature(child, b"SNOD")?;
            let count = self.uint(snod + 6, 2)? as usize;
            let mut e = snod + 8;
            for _ in 0..count {
                let name_offset = self.uint(e, self.offsets)? as usize;
                let object = self.address(e + self.offsets)?;
                let start = names as usize + name_offset;
                let tail = self.data.get(start..).ok_or("HDF5 name out of the file")?;
                let end = tail.iter().position(|&c| c == 0).unwrap_or(tail.len());
                out.push((String::from_utf8_lossy(&tail[..end]).into_owned(), object));
                e += 2 * self.offsets + 8 + 16;
            }
        }
        Ok(())
    }

    /// Every record of a v2 B-tree, in order.
    fn btree2(&self, address: u64) -> Result<Vec<&'a [u8]>> {
        let at = self.signature(address, b"BTHD")?;
        let node_size = self.uint(at + 6, 4)? as usize;
        let record = self.uint(at + 10, 2)? as usize;
        let depth = self.uint(at + 12, 2)? as usize;
        let root = self.address(at + 16)?;
        let root_records = self.uint(at + 16 + self.offsets, 2)? as usize;
        if record == 0 || depth > 32 {
            return Err("an HDF5 B-tree of no records".into());
        }
        // How many records a node holds at each depth, and how many bytes
        // its counts take, as the library sizes them.
        let encoded = |n: u64| -> usize { (64 - n.max(1).leading_zeros() as usize).div_ceil(8).max(1) };
        let leaf_max = (node_size.saturating_sub(10) / record) as u64;
        let count_size = encoded(leaf_max);
        let mut cumulative = vec![leaf_max];
        let mut cumulative_size = vec![0usize];
        for d in 1..=depth {
            let pointer = self.offsets + count_size + if d > 1 { cumulative_size[d - 1] } else { 0 };
            let max = (node_size.saturating_sub(10 + pointer) / (record + pointer)) as u64;
            let total = (max + 1) * cumulative[d - 1] + max;
            cumulative.push(total);
            cumulative_size.push(encoded(total));
        }
        let mut out = Vec::new();
        self.btree2_node(root, root_records, depth, record, count_size, &cumulative_size, &mut out)?;
        Ok(out)
    }

    #[allow(clippy::too_many_arguments)]
    fn btree2_node(&self, address: u64, records: usize, depth: usize, record: usize, count_size: usize, cumulative_size: &[usize], out: &mut Vec<&'a [u8]>) -> Result<()> {
        if depth == 0 {
            let at = self.signature(address, b"BTLF")?;
            for i in 0..records {
                out.push(self.bytes(at + 6 + i * record, record)?);
            }
            return Ok(());
        }
        let at = self.signature(address, b"BTIN")?;
        let own: Vec<&[u8]> = (0..records).map(|i| self.bytes(at + 6 + i * record, record)).collect::<Result<_>>()?;
        let mut p = at + 6 + records * record;
        for i in 0..=records {
            let child = self.address(p)?;
            p += self.offsets;
            let count = self.uint(p, count_size)? as usize;
            p += count_size;
            if depth > 1 {
                p += cumulative_size[depth - 1];
            }
            self.btree2_node(child, count, depth - 1, record, count_size, cumulative_size, out)?;
            if i < records {
                out.push(own[i]);
            }
        }
        Ok(())
    }

    // ---- datasets ------------------------------------------------------------

    /// The dataset at `path`.
    pub fn dataset(&self, path: &str) -> Result<Dataset> {
        self.read(self.find(path)?)
    }

    /// The dataset whose object header is at `address`.
    pub fn read(&self, address: u64) -> Result<Dataset> {
        let messages = self.messages(address)?;
        let find = |kind: u16| messages.iter().find(|m| m.kind == kind);
        let space = find(0x01).ok_or("a dataset without a dataspace")?;
        let kind = find(0x03).ok_or("a dataset without a datatype")?;
        let layout = find(0x08).ok_or("a dataset without a layout")?;
        let filters = match find(0x0B) {
            Some(m) => self.filters(m)?,
            None => Vec::new(),
        };
        let shape = self.dataspace(space.at)?;
        let count: usize = shape.iter().product::<usize>().max(1);
        if count > 1 << 28 {
            return Err("an HDF5 dataset too large to read".into());
        }
        let class = self.byte(kind.at)? & 0x0F;
        let bits = self.uint(kind.at + 1, 3)?;
        let size = self.uint(kind.at + 4, 4)? as usize;
        if size == 0 || size > 64 {
            return Err("an HDF5 datatype of no size".into());
        }
        let raw = self.raw(layout.at, &shape, size, &filters)?;
        let values = match class {
            1 => {
                if bits & 1 != 0 {
                    return Err("big-endian HDF5 numbers are not read".into());
                }
                Values::Numbers(match size {
                    4 => raw.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f64).collect(),
                    8 => raw.chunks_exact(8).map(|c| f64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]])).collect(),
                    _ => return Err("an HDF5 float of an unknown size".into()),
                })
            }
            0 => {
                let signed = bits & 0x08 != 0;
                Values::Numbers(
                    raw.chunks_exact(size)
                        .map(|c| {
                            let v = c.iter().rev().fold(0u64, |v, &x| (v << 8) | x as u64);
                            if signed && size < 8 && v >> (size * 8 - 1) & 1 == 1 {
                                (v as i64 - (1i64 << (size * 8))) as f64
                            } else if signed {
                                v as i64 as f64
                            } else {
                                v as f64
                            }
                        })
                        .collect(),
                )
            }
            3 => Values::Strings(
                raw.chunks_exact(size).map(|c| String::from_utf8_lossy(&c[..c.iter().position(|&x| x == 0).unwrap_or(c.len())]).into_owned()).collect(),
            ),
            9 => {
                // A sequence or string of variable length: each a length,
                // then where in the global heap it is.
                let each = 4 + self.offsets + 4;
                let mut strings = Vec::with_capacity(count);
                for i in 0..count {
                    let c = raw.get(i * each..(i + 1) * each).ok_or("short HDF5 variable-length data")?;
                    let collection = c[4..4 + self.offsets].iter().rev().fold(0u64, |v, &x| (v << 8) | x as u64);
                    let index = u32::from_le_bytes([c[4 + self.offsets], c[5 + self.offsets], c[6 + self.offsets], c[7 + self.offsets]]);
                    let bytes = self.global(collection, index)?;
                    strings.push(String::from_utf8_lossy(&bytes[..bytes.iter().position(|&x| x == 0).unwrap_or(bytes.len())]).into_owned());
                }
                Values::Strings(strings)
            }
            other => return Err(format!("HDF5 datatype class {other} is not read")),
        };
        Ok(Dataset { shape, values })
    }

    fn dataspace(&self, at: usize) -> Result<Vec<usize>> {
        let version = self.byte(at)?;
        let rank = self.byte(at + 1)? as usize;
        let start = if version == 1 { at + 8 } else { at + 4 };
        (0..rank).map(|i| self.uint(start + i * self.lengths, self.lengths).map(|v| v as usize)).collect()
    }

    /// The filters in a filter pipeline message, by id.
    fn filters(&self, m: &Message) -> Result<Vec<u16>> {
        let version = self.byte(m.at)?;
        let count = self.byte(m.at + 1)? as usize;
        let mut p = m.at + if version == 1 { 8 } else { 2 };
        let mut out = Vec::new();
        for _ in 0..count {
            let id = self.uint(p, 2)? as u16;
            out.push(id);
            if version == 1 || id >= 256 {
                let name = self.uint(p + 2, 2)? as usize;
                let values = self.uint(p + 6, 2)? as usize;
                let name = if version == 1 { name.div_ceil(8) * 8 } else { name };
                p += 8 + name + 4 * values + if version == 1 && values % 2 == 1 { 4 } else { 0 };
            } else {
                let values = self.uint(p + 4, 2)? as usize;
                p += 6 + 4 * values;
            }
        }
        Ok(out)
    }

    /// A dataset's bytes, its layout followed.
    fn raw(&self, at: usize, shape: &[usize], element: usize, filters: &[u16]) -> Result<Vec<u8>> {
        let version = self.byte(at)?;
        let count: usize = shape.iter().product::<usize>().max(1);
        let total = count * element;
        if version < 3 {
            return Err("HDF5 layout messages older than version 3 are not read".into());
        }
        match self.byte(at + 1)? {
            0 => {
                let size = self.uint(at + 2, 2)? as usize;
                Ok(self.bytes(at + 4, size.min(total))?.to_vec())
            }
            1 => {
                let address = self.address(at + 2)?;
                if address == UNDEFINED {
                    return Ok(vec![0; total]);
                }
                Ok(self.bytes(address as usize, total)?.to_vec())
            }
            2 if version == 3 => {
                let rank = self.byte(at + 2)? as usize;
                let tree = self.address(at + 3)?;
                let dims: Vec<usize> = (0..rank).map(|i| self.uint(at + 3 + self.offsets + 4 * i, 4).map(|v| v as usize)).collect::<Result<_>>()?;
                // The last of a chunk's dimensions is the element's size.
                let chunk = &dims[..rank.saturating_sub(1)];
                if chunk.len() != shape.len() {
                    return Err("an HDF5 chunk of a different rank from its dataset".into());
                }
                let mut out = vec![0u8; total];
                if tree != UNDEFINED {
                    self.chunks(tree, shape, chunk, element, filters, &mut out, 0)?;
                }
                Ok(out)
            }
            2 => Err("HDF5 version 4 chunk indexes are not read".into()),
            other => Err(format!("HDF5 layout class {other} is not read")),
        }
    }

    /// Every chunk under a v1 B-tree node, unfiltered and put in place.
    #[allow(clippy::too_many_arguments)]
    fn chunks(&self, node: u64, shape: &[usize], chunk: &[usize], element: usize, filters: &[u16], out: &mut [u8], depth: u32) -> Result<()> {
        if depth > 64 {
            return Err("HDF5 chunk tree too deep".into());
        }
        let at = self.signature(node, b"TREE")?;
        let level = self.byte(at + 5)?;
        let entries = self.uint(at + 6, 2)? as usize;
        let rank = shape.len();
        let key = 8 + 8 * (rank + 1);
        let mut p = at + 8 + 2 * self.offsets;
        for _ in 0..entries {
            let size = self.uint(p, 4)? as usize;
            let mask = self.uint(p + 4, 4)? as u32;
            let origin: Vec<usize> = (0..rank).map(|i| self.uint(p + 8 + 8 * i, 8).map(|v| v as usize)).collect::<Result<_>>()?;
            let child = self.address(p + key)?;
            p += key + self.offsets;
            if level > 0 {
                self.chunks(child, shape, chunk, element, filters, out, depth + 1)?;
                continue;
            }
            let mut data = self.bytes(child as usize, size)?.to_vec();
            for (i, filter) in filters.iter().enumerate().rev() {
                if mask >> i & 1 == 1 {
                    continue;
                }
                data = match filter {
                    1 => {
                        let mut inflated = Vec::new();
                        flate2::read::ZlibDecoder::new(&data[..]).read_to_end(&mut inflated).map_err(|e| format!("HDF5 deflate: {e}"))?;
                        inflated
                    }
                    2 => {
                        let n = data.len() / element;
                        let mut unshuffled = vec![0u8; data.len()];
                        for b in 0..element {
                            for j in 0..n {
                                unshuffled[j * element + b] = data[b * n + j];
                            }
                        }
                        unshuffled
                    }
                    3 => {
                        data.truncate(data.len().saturating_sub(4));
                        data
                    }
                    other => return Err(format!("HDF5 filter {other} is not read")),
                };
            }
            place(&data, &origin, chunk, shape, element, out);
        }
        Ok(())
    }

    /// An object in the global heap.
    fn global(&self, collection: u64, index: u32) -> Result<Vec<u8>> {
        let at = self.signature(collection, b"GCOL")?;
        let size = self.uint(at + 8, self.lengths)? as usize;
        let end = at + size;
        let mut p = at + 8 + self.lengths;
        while p + 8 + self.lengths <= end {
            let i = self.uint(p, 2)? as u32;
            let length = self.uint(p + 8, self.lengths)? as usize;
            if i == 0 {
                break;
            }
            if i == index {
                return Ok(self.bytes(p + 8 + self.lengths, length)?.to_vec());
            }
            p += 8 + self.lengths + length.div_ceil(8) * 8;
        }
        Err("an HDF5 string not in its heap".into())
    }
}

/// A chunk's elements copied to where they belong in the whole, row-major,
/// what lies past the dataset's edge left out.
fn place(data: &[u8], origin: &[usize], chunk: &[usize], shape: &[usize], element: usize, out: &mut [u8]) {
    let rank = shape.len();
    if rank == 0 {
        let n = data.len().min(out.len());
        out[..n].copy_from_slice(&data[..n]);
        return;
    }
    let row = chunk[rank - 1];
    let mut index = vec![0usize; rank - 1];
    loop {
        // Where this row of the chunk starts, in the chunk and the whole.
        let mut from = 0usize;
        let mut to = 0usize;
        let mut inside = true;
        for d in 0..rank - 1 {
            from = from * chunk[d] + index[d];
            let at = origin[d] + index[d];
            inside &= at < shape[d];
            to = to * shape[d] + at;
        }
        if inside && origin[rank - 1] < shape[rank - 1] {
            let n = row.min(shape[rank - 1] - origin[rank - 1]);
            let from = (from * row) * element;
            let to = (to * shape[rank - 1] + origin[rank - 1]) * element;
            if let (Some(src), Some(dst)) = (data.get(from..from + n * element), out.get_mut(to..to + n * element)) {
                dst.copy_from_slice(src);
            }
        }
        // The next row.
        let mut d = rank - 1;
        loop {
            if d == 0 {
                return;
            }
            d -= 1;
            index[d] += 1;
            if index[d] < chunk[d] {
                break;
            }
            index[d] = 0;
        }
    }
}

/// A fractal heap, for the objects its ids name.
struct Heap {
    id_offset: usize,
    id_length: usize,
    width: usize,
    start: u64,
    max_direct: u64,
    root: u64,
    rows: usize,
    offset_bytes: usize,
    checksummed: bool,
}

impl Heap {
    fn open(file: &File, address: u64) -> Result<Heap> {
        let at = file.signature(address, b"FRHP")?;
        let (o, l) = (file.offsets, file.lengths);
        let filters = file.uint(at + 7, 2)? as usize;
        let flags = file.byte(at + 9)?;
        let max_managed = file.uint(at + 10, 4)?;
        // Past the counts and addresses to the doubling table.
        let p = at + 14 + l + o + l + o + 8 * l;
        let width = file.uint(p, 2)? as usize;
        let start = file.uint(p + 2, l)?;
        let max_direct = file.uint(p + 2 + l, l)?;
        let max_heap_bits = file.uint(p + 2 + 2 * l, 2)? as usize;
        let root = file.address(p + 6 + 2 * l)?;
        let rows = file.uint(p + 6 + 2 * l + o, 2)? as usize;
        if filters != 0 {
            return Err("a filtered HDF5 fractal heap is not read".into());
        }
        if width == 0 || start == 0 || !start.is_power_of_two() || !max_direct.is_power_of_two() {
            return Err("an HDF5 fractal heap of no size".into());
        }
        let offset_bytes = max_heap_bits.div_ceil(8);
        let length_bytes = {
            let direct = (63 - max_direct.leading_zeros() as usize).div_ceil(8);
            let managed = (64 - max_managed.max(1).leading_zeros() as usize).div_ceil(8);
            direct.min(managed)
        };
        Ok(Heap { id_offset: offset_bytes, id_length: length_bytes, width, start, max_direct, root, rows, offset_bytes, checksummed: flags & 2 != 0 })
    }

    fn object(&self, file: &File, id: &[u8]) -> Result<Vec<u8>> {
        let first = *id.first().ok_or("an empty HDF5 heap id")?;
        match first >> 4 & 3 {
            // Tiny: the object is in the id.
            2 => {
                let n = (first & 0x0F) as usize + 1;
                return Ok(id.get(1..1 + n).ok_or("short HDF5 tiny object")?.to_vec());
            }
            0 => {}
            _ => return Err("huge HDF5 heap objects are not read".into()),
        }
        let le = |b: &[u8]| b.iter().rev().fold(0u64, |v, &x| (v << 8) | x as u64);
        let offset = le(id.get(1..1 + self.id_offset).ok_or("short HDF5 heap id")?);
        let length = le(id.get(1 + self.id_offset..1 + self.id_offset + self.id_length).ok_or("short HDF5 heap id")?) as usize;
        let (block, block_offset) = if self.rows == 0 { (self.root, 0) } else { self.find(file, self.root, self.rows, 0, offset, 0)? };
        let at = file.signature(block, b"FHDB")?;
        let _ = self.checksummed;
        // Offsets count from the block's start, header and all.
        let start = at + (offset - block_offset) as usize;
        Ok(file.bytes(start, length)?.to_vec())
    }

    /// The direct block holding heap offset `offset`, under the indirect
    /// block at `address` of `rows` rows starting at `base`.
    fn find(&self, file: &File, address: u64, rows: usize, base: u64, offset: u64, depth: u32) -> Result<(u64, u64)> {
        if depth > 16 {
            return Err("HDF5 heap too deep".into());
        }
        let at = file.signature(address, b"FHIB")?;
        let mut p = at + 5 + file.offsets + self.offset_bytes;
        let max_direct_rows = (self.max_direct.trailing_zeros() - self.start.trailing_zeros()) as usize + 2;
        let mut position = base;
        for row in 0..rows {
            let size = if row == 0 { self.start } else { self.start << (row - 1) };
            for _ in 0..self.width {
                let child = file.address(p)?;
                p += file.offsets;
                if child != UNDEFINED && offset >= position && offset < position + size {
                    if row < max_direct_rows {
                        return Ok((child, position));
                    }
                    let child_rows = (size / (self.start * self.width as u64)).trailing_zeros() as usize + 1;
                    return self.find(file, child, child_rows, position, offset, depth + 1);
                }
                position += size;
            }
        }
        Err("an HDF5 heap offset in no block".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A chunk's rows land where they belong, cut at the dataset's edge.
    #[test]
    fn a_chunk_is_placed_and_cut_at_the_edge() {
        let mut out = vec![0u8; 3 * 3];
        // A 2x2 chunk at (2, 2) of a 3x3 dataset of bytes: one element in.
        place(&[1, 2, 3, 4], &[2, 2], &[2, 2], &[3, 3], 1, &mut out);
        assert_eq!(out, vec![0, 0, 0, 0, 0, 0, 0, 0, 1]);
        place(&[5, 6, 7, 8], &[0, 0], &[2, 2], &[3, 3], 1, &mut out);
        assert_eq!(out, vec![5, 6, 0, 7, 8, 0, 0, 0, 1]);
    }

    /// AxF files on this computer, named in `TEQUILA_AXF`: their material
    /// groups read.
    #[test]
    #[ignore = "reads files named in TEQUILA_AXF"]
    fn files_named_in_the_environment_read() {
        let Ok(list) = std::env::var("TEQUILA_AXF") else { return };
        for path in list.split(';').filter(|p| !p.is_empty()) {
            let bytes = std::fs::read(path).unwrap();
            let file = File::open(&bytes).unwrap();
            let materials = file.find("com.xrite.Materials").unwrap();
            for (name, _) in file.members(materials).unwrap() {
                println!("{path}: {name}");
            }
        }
    }
}
