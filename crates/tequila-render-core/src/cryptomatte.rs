//! Cryptomatte (`docs/01-RENDERER-CORE.md` §12, [Friedman & Jones 2015]).
//!
//! A Cryptomatte is a set of per-pixel coverage mattes that a compositor can
//! use to isolate any object or material in a render *after* the fact, with
//! correct partial coverage at the edges and through motion blur and depth of
//! field. §12 requires it "spec-compliant so Nuke/After Effects/Fusion read it
//! directly", and that word is doing real work: every part of the encoding is
//! fixed by the standard, and a plausible-looking variation is a file those
//! applications open and show nothing useful in.
//!
//! The parts that are not negotiable:
//!
//! - **The hash is MurmurHash3_x86_32** of the object's name, with a fixup that
//!   keeps the result out of the NaN and infinity exponents — a float that is
//!   not a number cannot be compared, and comparison is how a compositor picks
//!   a matte.
//! - **The layers are ranked**, most coverage first, packed two ranks to a
//!   four-channel image: `(id0, coverage0, id1, coverage1)`.
//! - **The manifest** maps names to hashes, so a compositor can offer a list to
//!   pick from rather than making the user click a pixel.
//! - **The metadata keys** are derived from the first seven hex digits of the
//!   hash of the layer's own name. That indirection looks pointless until two
//!   Cryptomattes are in one file, which is exactly the case here: objects and
//!   materials.

use std::collections::BTreeMap;

/// How many ranks are kept per pixel.
///
/// Six, which is three four-channel layers — the number the standard's
/// reference implementations write and the number a compositor expects. A pixel
/// covered by more than six objects loses the least-covering ones, which is the
/// standard's own behaviour rather than a shortcut.
pub const RANKS: usize = 6;

/// MurmurHash3 x86 32-bit, as the Cryptomatte standard specifies.
pub fn murmur3_32(key: &[u8], seed: u32) -> u32 {
    const C1: u32 = 0xcc9e_2d51;
    const C2: u32 = 0x1b87_3593;

    let mut h = seed;
    let blocks = key.len() / 4;
    for i in 0..blocks {
        let mut k = u32::from_le_bytes([
            key[i * 4],
            key[i * 4 + 1],
            key[i * 4 + 2],
            key[i * 4 + 3],
        ]);
        k = k.wrapping_mul(C1);
        k = k.rotate_left(15);
        k = k.wrapping_mul(C2);
        h ^= k;
        h = h.rotate_left(13);
        h = h.wrapping_mul(5).wrapping_add(0xe654_6b64);
    }

    let tail = &key[blocks * 4..];
    let mut k = 0u32;
    if tail.len() >= 3 {
        k ^= (tail[2] as u32) << 16;
    }
    if tail.len() >= 2 {
        k ^= (tail[1] as u32) << 8;
    }
    if !tail.is_empty() {
        k ^= tail[0] as u32;
        k = k.wrapping_mul(C1);
        k = k.rotate_left(15);
        k = k.wrapping_mul(C2);
        h ^= k;
    }

    h ^= key.len() as u32;
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    h
}

/// The float a name hashes to.
///
/// The standard's exact procedure: hash, then force the exponent away from the
/// all-ones and all-zeros patterns. A NaN cannot be compared for equality and
/// an infinity cannot be interpolated, and a compositor does both — so an ID
/// that landed on either would be an object nobody could select.
pub fn name_to_float(name: &str) -> f32 {
    let hash = murmur3_32(name.as_bytes(), 0);
    hash_to_float(hash)
}

/// The same fixup, for a hash that is already in hand.
pub fn hash_to_float(hash: u32) -> f32 {
    let exponent = (hash >> 23) & 0xff;
    let fixed = if exponent == 0 || exponent == 0xff {
        // Flip the highest exponent bit, which moves an all-ones exponent to
        // something finite and an all-zeros one out of the denormals, while
        // leaving every other hash untouched.
        hash ^ (1 << 23)
    } else {
        hash
    };
    f32::from_bits(fixed)
}

/// A name and the hash it is known by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestEntry {
    pub name: String,
    pub hash: u32,
}

/// One Cryptomatte: a name, a manifest, and the ranked coverage per pixel.
#[derive(Clone, Debug)]
pub struct Cryptomatte {
    /// `CryptoObject`, `CryptoMaterial`, and so on. The layer names in the EXR
    /// are this plus `00`, `01`, `02`.
    pub name: String,
    pub width: usize,
    pub height: usize,
    /// Every identifier that touched each pixel, with its accumulated
    /// coverage, ranked by [`Cryptomatte::finish`].
    ///
    /// A growable list per pixel rather than a fixed set of slots, and the
    /// reason is worth stating because the fixed version is the obvious one and
    /// is wrong. With `RANKS` slots and a displacement rule, which identifiers
    /// survive depends on the order the samples arrive in: once six of them
    /// hold one sample each, a seventh that goes on to cover most of the pixel
    /// can never displace any, because each of its samples is compared against
    /// an accumulated total. Ranking at the end from a complete tally is the
    /// only version that gives the same answer whatever order the sampler
    /// visits things in — and order-independence is what §4.6 requires of this
    /// buffer as much as of the beauty pass.
    ///
    /// The cost is a `Vec` per pixel. Most pixels hold one or two entries, and
    /// the memory sits alongside the pass buffers, which at 4K are already far
    /// larger.
    pixels: Vec<Vec<(f32, f32)>>,
    /// Every name that appeared, so the manifest can list them.
    manifest: BTreeMap<String, u32>,
}

impl Cryptomatte {
    pub fn new(name: impl Into<String>, width: usize, height: usize) -> Cryptomatte {
        Cryptomatte {
            name: name.into(),
            width,
            height,
            pixels: vec![Vec::new(); width * height],
            manifest: BTreeMap::new(),
        }
    }

    /// Record that a name exists, so it appears in the manifest even where it
    /// never won a rank.
    pub fn register(&mut self, name: &str) {
        self.manifest
            .entry(name.to_string())
            .or_insert_with(|| murmur3_32(name.as_bytes(), 0));
    }

    /// Add one sample's worth of coverage at a pixel.
    ///
    /// Coverage is *summed*, not averaged, and divided by the sample count at
    /// the end. That is what makes a pixel on a silhouette come out at the
    /// fraction of samples that landed on each side, which is the whole reason
    /// a Cryptomatte beats an ID pass.
    pub fn accumulate(&mut self, x: usize, y: usize, id: f32, coverage: f32) {
        if x >= self.width || y >= self.height || coverage <= 0.0 {
            return;
        }
        let entries = &mut self.pixels[y * self.width + x];
        for entry in entries.iter_mut() {
            if entry.0 == id {
                entry.1 += coverage;
                return;
            }
        }
        entries.push((id, coverage));
    }

    /// Normalize by the sample count and sort each pixel's ranks.
    ///
    /// Called once before writing. Sorting is not cosmetic: a compositor reads
    /// rank 0 first and stops when the coverage it has gathered reaches one, so
    /// unsorted ranks give a matte that is missing its largest contributor.
    pub fn finish(&mut self, samples: u32) {
        let scale = if samples == 0 {
            0.0
        } else {
            1.0 / samples as f32
        };
        for entries in &mut self.pixels {
            for entry in entries.iter_mut() {
                entry.1 *= scale;
            }
            // Descending by coverage, and by identifier where two are equal —
            // which happens on an evenly split pixel and would otherwise let
            // the ranks depend on sample order after all.
            entries.sort_by(|a, b| {
                b.1.partial_cmp(&a.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| {
                        b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal)
                    })
            });
            entries.truncate(RANKS);
            entries.shrink_to_fit();
        }
    }

    /// The (id, coverage) pairs at a pixel, ranked, padded to `RANKS`.
    pub fn ranks_at(&self, x: usize, y: usize) -> [(f32, f32); RANKS] {
        let mut out = [(0.0f32, 0.0f32); RANKS];
        let entries = &self.pixels[y.min(self.height - 1) * self.width + x.min(self.width - 1)];
        for (slot, entry) in out.iter_mut().zip(entries) {
            *slot = *entry;
        }
        out
    }

    /// The coverage of one name at one pixel, which is what a compositor
    /// computes when it picks a matte.
    pub fn coverage_of(&self, name: &str, x: usize, y: usize) -> f32 {
        let id = name_to_float(name);
        self.ranks_at(x, y)
            .iter()
            .filter(|(entry, _)| *entry == id)
            .map(|(_, coverage)| *coverage)
            .sum()
    }

    pub fn manifest(&self) -> Vec<ManifestEntry> {
        self.manifest
            .iter()
            .map(|(name, hash)| ManifestEntry {
                name: name.clone(),
                hash: *hash,
            })
            .collect()
    }

    /// The manifest as the JSON the standard puts in the EXR header.
    pub fn manifest_json(&self) -> String {
        let mut out = String::from("{");
        for (index, entry) in self.manifest().iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            // The name is JSON-escaped; a part called `1/2"` is not unusual in
            // CAD and would otherwise produce a manifest nothing can parse.
            out.push('"');
            for c in entry.name.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    c if (c as u32) < 0x20 => {
                        out.push_str(&format!("\\u{:04x}", c as u32));
                    }
                    c => out.push(c),
                }
            }
            out.push_str("\":\"");
            out.push_str(&format!("{:08x}", entry.hash));
            out.push('"');
        }
        out.push('}');
        out
    }

    /// The seven hex digits that key this Cryptomatte's metadata.
    ///
    /// The standard derives them from the hash of the *layer's* name, which
    /// looks like an odd indirection until a file holds two Cryptomattes — and
    /// this one holds objects and materials both.
    pub fn metadata_key(&self) -> String {
        format!("{:07x}", murmur3_32(self.name.as_bytes(), 0))[..7].to_string()
    }

    /// How many four-channel layers this writes.
    pub fn layer_count(&self) -> usize {
        RANKS / 2
    }

    /// The four channel values at a pixel for one layer.
    pub fn layer_at(&self, layer: usize, x: usize, y: usize) -> [f32; 4] {
        let ranks = self.ranks_at(x, y);
        let first = layer * 2;
        let a = ranks.get(first).copied().unwrap_or((0.0, 0.0));
        let b = ranks.get(first + 1).copied().unwrap_or((0.0, 0.0));
        [a.0, a.1, b.0, b.1]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MurmurHash3 against the reference vectors. The whole format hangs off
    /// this function agreeing with everyone else's, and a hash that is merely
    /// *a* hash produces a file Nuke opens and finds nothing in.
    #[test]
    fn the_hash_matches_the_reference_vectors() {
        // From the canonical MurmurHash3 test suite.
        assert_eq!(murmur3_32(b"", 0), 0);
        assert_eq!(murmur3_32(b"", 1), 0x514E28B7);
        assert_eq!(murmur3_32(b"", 0xffff_ffff), 0x81F16F39);
        assert_eq!(murmur3_32(b"aaaa", 0x9747b28c), 0x5A97808A);
        assert_eq!(murmur3_32(b"aaa", 0x9747b28c), 0x283E0130);
        assert_eq!(murmur3_32(b"aa", 0x9747b28c), 0x5D211726);
        assert_eq!(murmur3_32(b"a", 0x9747b28c), 0x7FA09EA6);
        assert_eq!(murmur3_32(b"abcd", 0x9747b28c), 0xF0478627);
        assert_eq!(murmur3_32(b"Hello, world!", 0x9747b28c), 0x24884CBA);
    }

    /// An ID has to be a number a compositor can compare and interpolate. A NaN
    /// compares unequal to itself, so an object whose name happened to hash
    /// into the NaN exponent would be one nobody could ever select.
    #[test]
    fn every_id_is_a_usable_float() {
        for i in 0..20000u32 {
            let id = hash_to_float(i.wrapping_mul(2_654_435_761));
            assert!(id.is_finite(), "hash {i} produced {id}");
            assert!(!id.is_nan());
        }
        // Including the patterns the fixup exists for.
        assert!(hash_to_float(0x7f80_0000).is_finite(), "infinity");
        assert!(hash_to_float(0x7fc0_0000).is_finite(), "a quiet NaN");
        assert!(hash_to_float(0x0000_0001).is_finite(), "a denormal");
    }

    /// The property a Cryptomatte exists for: partial coverage on a silhouette.
    /// An ID pass gives one object per pixel and a hard edge; this has to give
    /// the fraction of samples that landed on each.
    #[test]
    fn coverage_is_fractional_on_an_edge() {
        let mut matte = Cryptomatte::new("CryptoObject", 4, 4);
        matte.register("front");
        matte.register("back");
        for _ in 0..3 {
            matte.accumulate(1, 1, name_to_float("front"), 1.0);
        }
        matte.accumulate(1, 1, name_to_float("back"), 1.0);
        matte.finish(4);

        assert!((matte.coverage_of("front", 1, 1) - 0.75).abs() < 1e-5);
        assert!((matte.coverage_of("back", 1, 1) - 0.25).abs() < 1e-5);
        // And the two account for the whole pixel, which is what makes the
        // matte composite without a seam.
        let total: f32 = matte.ranks_at(1, 1).iter().map(|(_, c)| *c).sum();
        assert!((total - 1.0).abs() < 1e-5);
    }

    /// Ranks are sorted by coverage. A compositor reads rank 0 first and stops
    /// once it has gathered full coverage, so unsorted ranks give a matte
    /// missing its largest contributor.
    #[test]
    fn ranks_come_out_sorted_by_coverage() {
        let mut matte = Cryptomatte::new("CryptoObject", 1, 1);
        for (name, count) in [("a", 1), ("b", 5), ("c", 3)] {
            matte.register(name);
            for _ in 0..count {
                matte.accumulate(0, 0, name_to_float(name), 1.0);
            }
        }
        matte.finish(9);
        let ranks = matte.ranks_at(0, 0);
        assert_eq!(ranks[0].0, name_to_float("b"));
        assert_eq!(ranks[1].0, name_to_float("c"));
        assert_eq!(ranks[2].0, name_to_float("a"));
        for pair in ranks.windows(2) {
            assert!(pair[0].1 >= pair[1].1);
        }
    }

    /// Past six objects in one pixel the standard keeps the most-covering, not
    /// the first seen. Keeping the first seen would make a busy pixel's matte
    /// depend on sampling order.
    #[test]
    fn a_crowded_pixel_keeps_the_largest_contributors() {
        let mut matte = Cryptomatte::new("CryptoObject", 1, 1);
        // Eight objects, the last two by far the largest.
        for i in 0..8u32 {
            let name = format!("object{i}");
            matte.register(&name);
            let count = if i >= 6 { 20 } else { 1 };
            for _ in 0..count {
                matte.accumulate(0, 0, name_to_float(&name), 1.0);
            }
        }
        matte.finish(46);
        assert!(matte.coverage_of("object7", 0, 0) > 0.4);
        assert!(matte.coverage_of("object6", 0, 0) > 0.4);
    }

    /// The manifest is JSON a compositor parses. A CAD part name with a quote
    /// or a backslash in it — `1/2" bolt` is an ordinary name — must not break
    /// the file.
    #[test]
    fn the_manifest_escapes_awkward_names() {
        let mut matte = Cryptomatte::new("CryptoObject", 1, 1);
        matte.register("plain");
        matte.register("1/2\" bolt");
        matte.register("back\\slash");
        let json = matte.manifest_json();
        assert!(json.starts_with('{') && json.ends_with('}'));
        assert!(json.contains("1/2\\\" bolt"), "{json}");
        assert!(json.contains("back\\\\slash"), "{json}");
        // And every entry names its hash in hex.
        assert!(json.contains(&format!("{:08x}", murmur3_32(b"plain", 0))));
    }

    /// Two Cryptomattes in one file need different metadata keys, which is the
    /// entire reason the standard keys them by the hash of the layer name.
    #[test]
    fn two_cryptomattes_have_different_keys() {
        let objects = Cryptomatte::new("CryptoObject", 1, 1);
        let materials = Cryptomatte::new("CryptoMaterial", 1, 1);
        assert_ne!(objects.metadata_key(), materials.metadata_key());
        assert_eq!(objects.metadata_key().len(), 7);
    }

    /// The layers pack two ranks each, in the order the standard reads them.
    #[test]
    fn layers_pack_two_ranks_each() {
        let mut matte = Cryptomatte::new("CryptoObject", 1, 1);
        for (name, count) in [("a", 4), ("b", 3), ("c", 2), ("d", 1)] {
            matte.register(name);
            for _ in 0..count {
                matte.accumulate(0, 0, name_to_float(name), 1.0);
            }
        }
        matte.finish(10);
        assert_eq!(matte.layer_count(), 3);
        let first = matte.layer_at(0, 0, 0);
        assert_eq!(first[0], name_to_float("a"));
        assert!((first[1] - 0.4).abs() < 1e-5);
        assert_eq!(first[2], name_to_float("b"));
        assert!((first[3] - 0.3).abs() < 1e-5);
        let second = matte.layer_at(1, 0, 0);
        assert_eq!(second[0], name_to_float("c"));
        assert_eq!(second[2], name_to_float("d"));
        // The unused ranks are zero, which reads as "nothing here".
        let third = matte.layer_at(2, 0, 0);
        assert_eq!(third, [0.0, 0.0, 0.0, 0.0]);
    }
}
