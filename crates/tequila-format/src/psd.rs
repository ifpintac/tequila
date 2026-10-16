//! Layered Photoshop documents (`TEQUILA_SPEC.md` §1.8).
//!
//! Written by hand, unlike the EXR next door, and the reason is the opposite of
//! that one's. There is no maintained Rust *writer* for PSD, and the part of
//! the format this needs is small and completely specified: a header, a layer
//! table, and planar channel data. What makes PSD fiddly is the parts nobody
//! here uses — adjustment layers, smart objects, the resource block zoo — and
//! leaving those out is not an approximation, it is a document that simply
//! does not contain them.
//!
//! # The parts that are easy to get wrong
//!
//! - **Channels are planar and per layer.** Not interleaved: a layer's red
//!   plane is written whole, then its green. Interleaving produces a file
//!   Photoshop opens as noise.
//! - **Every length is big-endian**, including the ones inside the layer
//!   records, and several of them count bytes that come *after* the length
//!   itself. Getting one wrong shifts everything after it.
//! - **Layer names are Pascal strings padded to a multiple of four**, and the
//!   padding is counted in the extra-data length. A name whose length happens
//!   to be a multiple of four still takes a pad byte for the count.
//! - **The order is bottom-up.** The first layer record in the file is the one
//!   at the bottom of the stack, which is the reverse of how a person lists
//!   passes.

use tequila_core::spectrum::Spectrum;
use tequila_color::ImageStyle;

/// How many bits a sample gets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Depth {
    /// What a screen shows and what most people want.
    #[default]
    Eight,
    /// Room to grade without banding.
    Sixteen,
    /// Scene-linear, unclamped. The passes stay the numbers the renderer
    /// produced rather than a picture of them.
    ThirtyTwo,
}

impl Depth {
    fn bits(self) -> u16 {
        match self {
            Depth::Eight => 8,
            Depth::Sixteen => 16,
            Depth::ThirtyTwo => 32,
        }
    }

    fn bytes(self) -> usize {
        (self.bits() / 8) as usize
    }
}

/// One layer of the document.
#[derive(Clone, Debug)]
pub struct PsdLayer {
    pub name: String,
    /// Scene-linear samples, row-major from the top-left.
    pub pixels: Vec<Spectrum>,
    /// Coverage, or all ones for an opaque layer.
    pub alpha: Vec<f32>,
}

/// A document on its way to disk.
#[derive(Clone, Debug)]
pub struct PsdImage {
    pub width: usize,
    pub height: usize,
    pub depth: Depth,
    /// The flattened image every reader shows, even ones that ignore layers.
    pub composite: PsdLayer,
    /// Bottom of the stack first, which is the order the file wants.
    pub layers: Vec<PsdLayer>,
    /// The grade applied on the way to eight and sixteen bits. Ignored at
    /// thirty-two, where the point is to keep the renderer's own numbers.
    pub style: ImageStyle,
}

#[derive(Debug)]
pub enum PsdError {
    /// Photoshop's own limit, and not a small one.
    TooLarge(usize, usize),
    Size {
        layer: String,
        expected: usize,
        found: usize,
    },
}

impl std::fmt::Display for PsdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PsdError::TooLarge(w, h) => write!(
                f,
                "{w}x{h} is past PSD's 30,000 pixel limit; a PSB would be needed"
            ),
            PsdError::Size {
                layer,
                expected,
                found,
            } => write!(
                f,
                "layer `{layer}` holds {found} samples where the document is {expected} pixels"
            ),
        }
    }
}

impl std::error::Error for PsdError {}

const MAX_SIDE: usize = 30_000;

impl PsdImage {
    pub fn encode(&self) -> Result<Vec<u8>, PsdError> {
        if self.width > MAX_SIDE || self.height > MAX_SIDE {
            return Err(PsdError::TooLarge(self.width, self.height));
        }
        let count = self.width * self.height;
        for layer in std::iter::once(&self.composite).chain(&self.layers) {
            if layer.pixels.len() != count || layer.alpha.len() != count {
                return Err(PsdError::Size {
                    layer: layer.name.clone(),
                    expected: count,
                    found: layer.pixels.len().min(layer.alpha.len()),
                });
            }
        }

        let mut out = Vec::new();
        // ---- Header ----------------------------------------------------
        out.extend_from_slice(b"8BPS");
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&[0u8; 6]);
        // Four channels: the alpha is what makes a shadow catcher composite.
        out.extend_from_slice(&4u16.to_be_bytes());
        out.extend_from_slice(&(self.height as u32).to_be_bytes());
        out.extend_from_slice(&(self.width as u32).to_be_bytes());
        out.extend_from_slice(&self.depth.bits().to_be_bytes());
        // Colour mode 3 is RGB.
        out.extend_from_slice(&3u16.to_be_bytes());

        // ---- Colour mode data and image resources ----------------------
        // Both empty, and both still need their length.
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes());

        // ---- Layer and mask information --------------------------------
        let layer_section = self.encode_layers();
        out.extend_from_slice(&(layer_section.len() as u32).to_be_bytes());
        out.extend_from_slice(&layer_section);

        // ---- The flattened image ---------------------------------------
        // Compression 0: raw. Photoshop's RLE would shrink this, and a render
        // is mostly gradient, where RLE gains little and costs a pass over
        // every scanline.
        out.extend_from_slice(&0u16.to_be_bytes());
        for channel in 0..4 {
            self.write_plane(&mut out, &self.composite, channel);
        }
        Ok(out)
    }

    fn encode_layers(&self) -> Vec<u8> {
        if self.layers.is_empty() {
            // A document with no layers still needs the section, and an empty
            // one is four zero bytes rather than nothing at all.
            return Vec::new();
        }
        let mut records = Vec::new();
        records.extend_from_slice(&(self.layers.len() as i16).to_be_bytes());

        // Each layer's planes, kept aside so the records can name their sizes.
        let mut planes: Vec<Vec<Vec<u8>>> = Vec::new();
        for layer in &self.layers {
            let mut per_channel = Vec::new();
            for channel in 0..4 {
                let mut plane = Vec::new();
                // Every plane carries its own compression word.
                plane.extend_from_slice(&0u16.to_be_bytes());
                self.write_plane(&mut plane, layer, channel);
                per_channel.push(plane);
            }
            planes.push(per_channel);
        }

        for (layer, per_channel) in self.layers.iter().zip(&planes) {
            // The rectangle, top-left to bottom-right.
            records.extend_from_slice(&0i32.to_be_bytes());
            records.extend_from_slice(&0i32.to_be_bytes());
            records.extend_from_slice(&(self.height as i32).to_be_bytes());
            records.extend_from_slice(&(self.width as i32).to_be_bytes());

            records.extend_from_slice(&4u16.to_be_bytes());
            // Alpha is channel -1, and it comes first: Photoshop reads the
            // identifiers rather than the order, but every file in the wild
            // writes it here and a reader that assumed otherwise is a reader
            // this file would break.
            for (id, plane) in [(-1i16, 3usize), (0, 0), (1, 1), (2, 2)] {
                records.extend_from_slice(&id.to_be_bytes());
                records.extend_from_slice(&(per_channel[plane].len() as u32).to_be_bytes());
            }

            records.extend_from_slice(b"8BIM");
            records.extend_from_slice(b"norm");
            records.push(255); // opacity
            records.push(0); // clipping: base
            records.push(0); // flags
            records.push(0); // filler

            // Extra data: mask, blending ranges, name.
            let mut extra = Vec::new();
            extra.extend_from_slice(&0u32.to_be_bytes()); // no mask
            extra.extend_from_slice(&0u32.to_be_bytes()); // no blending ranges
            let name = layer.name.as_bytes();
            let length = name.len().min(255);
            extra.push(length as u8);
            extra.extend_from_slice(&name[..length]);
            // Padded to a multiple of four, *including* the length byte — so a
            // name of exactly three characters still takes no padding and one
            // of four takes three.
            while (extra.len() - 8) % 4 != 0 {
                extra.push(0);
            }
            records.extend_from_slice(&(extra.len() as u32).to_be_bytes());
            records.extend_from_slice(&extra);
        }

        // Then every layer's channel data, in the same order.
        for per_channel in &planes {
            for plane in [3usize, 0, 1, 2] {
                records.extend_from_slice(&per_channel[plane]);
            }
        }

        let mut section = Vec::new();
        // Layer info, padded to an even length.
        let padded = records.len() + (records.len() % 2);
        section.extend_from_slice(&(padded as u32).to_be_bytes());
        section.extend_from_slice(&records);
        if records.len() % 2 == 1 {
            section.push(0);
        }
        // Global layer mask info: present and empty.
        section.extend_from_slice(&0u32.to_be_bytes());
        section
    }

    /// One channel plane of one layer, in this document's depth.
    fn write_plane(&self, out: &mut Vec<u8>, layer: &PsdLayer, channel: usize) {
        out.reserve(self.width * self.height * self.depth.bytes());
        for index in 0..self.width * self.height {
            let value = if channel == 3 {
                layer.alpha[index]
            } else {
                match self.depth {
                    // Eight and sixteen bits are a *picture*, so they carry the
                    // graded, display-encoded value — a linear 0.18 written to
                    // eight bits is code 46, which looks black.
                    Depth::Eight | Depth::Sixteen => {
                        let graded = self.style.apply(layer.pixels[index]);
                        [graded.x, graded.y, graded.z][channel]
                    }
                    // Thirty-two bits is the renderer's own number, which is
                    // the entire reason to ask for it.
                    Depth::ThirtyTwo => {
                        let rgb = layer.pixels[index].to_rendering_rgb();
                        [rgb.x, rgb.y, rgb.z][channel]
                    }
                }
            };
            match self.depth {
                Depth::Eight => out.push((value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8),
                Depth::Sixteen => out.extend_from_slice(
                    &((value.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16).to_be_bytes(),
                ),
                Depth::ThirtyTwo => out.extend_from_slice(&value.to_be_bytes()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(name: &str, value: f32, size: usize) -> PsdLayer {
        PsdLayer {
            name: name.into(),
            pixels: vec![Spectrum::splat(value); size],
            alpha: vec![1.0; size],
        }
    }

    fn document(depth: Depth) -> PsdImage {
        PsdImage {
            width: 4,
            height: 3,
            depth,
            composite: layer("Composite", 0.5, 12),
            layers: vec![layer("Diffuse", 0.25, 12), layer("Reflection", 0.1, 12)],
            style: ImageStyle::default(),
        }
    }

    fn be32(bytes: &[u8], at: usize) -> u32 {
        u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
    }

    /// The header a reader looks at first. Every field in it is fixed-width and
    /// big-endian, and one wrong byte makes the file unopenable rather than
    /// wrong — which is at least honest.
    /// The reader the label importer uses reads these documents' flattened
    /// image, past the layers, at every depth.
    #[test]
    fn the_texture_reader_reads_the_composite_back() {
        for depth in [Depth::Eight, Depth::Sixteen, Depth::ThirtyTwo] {
            let mut doc = document(depth);
            doc.composite.alpha[5] = 0.0;
            let bytes = doc.encode().unwrap();
            let read = tequila_texture::psd::composite(&bytes).expect("reads");
            assert_eq!((read.width, read.height), (4, 3));
            assert_eq!(read.linear, depth == Depth::ThirtyTwo);
            assert_eq!(read.rgba[5 * 4 + 3], 0.0);
            assert_eq!(read.rgba[3], 1.0);
            // The composite is grey throughout: every channel the same.
            let grey = read.rgba[0];
            assert!(grey > 0.0 && (read.rgba[1] - grey).abs() < 1e-3 && (read.rgba[2] - grey).abs() < 1e-3);
            if depth == Depth::ThirtyTwo {
                let expected = Spectrum::splat(0.5).to_rendering_rgb().x;
                assert!((grey - expected).abs() < 1e-4, "{grey}");
            }
        }
    }

    #[test]
    fn the_header_says_what_the_document_is() {
        let bytes = document(Depth::Eight).encode().unwrap();
        assert_eq!(&bytes[0..4], b"8BPS");
        assert_eq!(u16::from_be_bytes([bytes[4], bytes[5]]), 1);
        assert_eq!(u16::from_be_bytes([bytes[12], bytes[13]]), 4, "channels");
        assert_eq!(be32(&bytes, 14), 3, "height");
        assert_eq!(be32(&bytes, 18), 4, "width");
        assert_eq!(u16::from_be_bytes([bytes[22], bytes[23]]), 8, "depth");
        assert_eq!(u16::from_be_bytes([bytes[24], bytes[25]]), 3, "RGB");
    }

    /// Every declared length has to be the length of what follows it. This is
    /// the single most common way a hand-written PSD goes wrong, and the
    /// failure is silent up to the point a reader seeks past the end.
    #[test]
    fn every_section_length_matches_its_contents() {
        for depth in [Depth::Eight, Depth::Sixteen, Depth::ThirtyTwo] {
            let bytes = document(depth).encode().unwrap();
            let mut at = 26;
            let colour_mode = be32(&bytes, at) as usize;
            assert_eq!(colour_mode, 0);
            at += 4 + colour_mode;
            let resources = be32(&bytes, at) as usize;
            assert_eq!(resources, 0);
            at += 4 + resources;

            let layer_section = be32(&bytes, at) as usize;
            at += 4;
            // Inside it: the layer info block, then the global mask.
            let layer_info = be32(&bytes, at) as usize;
            assert_eq!(
                layer_section,
                layer_info + 8,
                "the layer section at {depth:?} does not hold its own contents"
            );
            at += layer_section;

            // What is left is the flattened image: a compression word and four
            // planes.
            let expected = 2 + 4 * 4 * 3 * depth.bytes();
            assert_eq!(
                bytes.len() - at,
                expected,
                "the flattened image at {depth:?} is the wrong size"
            );
        }
    }

    /// Layer names are Pascal strings padded to a multiple of four, and the
    /// padding is counted. A name of the wrong length shifts every layer after
    /// it.
    #[test]
    fn layer_names_are_padded_to_a_multiple_of_four() {
        for name in ["a", "ab", "abc", "abcd", "abcde", "Global illumination"] {
            let mut image = document(Depth::Eight);
            image.layers = vec![layer(name, 0.5, 12)];
            let bytes = image.encode().unwrap();
            // Find the name and check what follows it is the channel data
            // rather than padding that was not accounted for.
            let position = bytes
                .windows(name.len())
                .position(|w| w == name.as_bytes())
                .unwrap_or_else(|| panic!("`{name}` is not in the file"));
            // The length byte is immediately before it.
            assert_eq!(bytes[position - 1] as usize, name.len());
            // And the extra-data length, twelve bytes further back, counts the
            // mask, the ranges, the length byte, the name and its padding.
            let extra_at = position - 1 - 8 - 4;
            let extra = be32(&bytes, extra_at) as usize;
            let payload = 8 + 1 + name.len();
            assert_eq!(
                extra,
                payload + ((4 - (payload - 8) % 4) % 4),
                "`{name}` is padded wrongly"
            );
        }
    }

    /// Eight and sixteen bits are a picture and carry the graded value;
    /// thirty-two is the renderer's own number. Writing scene-linear into eight
    /// bits produces an image that looks black, and writing a graded value into
    /// thirty-two throws away the reason to ask for it.
    #[test]
    fn depth_decides_whether_the_grade_is_applied() {
        let mut image = document(Depth::ThirtyTwo);
        image.layers.clear();
        image.composite = layer("Composite", 0.18, 12);
        let bytes = image.encode().unwrap();
        // The last plane is alpha; the first is red.
        let start = bytes.len() - 4 * 12 * 4;
        let red = f32::from_be_bytes([
            bytes[start],
            bytes[start + 1],
            bytes[start + 2],
            bytes[start + 3],
        ]);
        let expected = Spectrum::splat(0.18).to_rendering_rgb().x;
        assert!(
            (red - expected).abs() < 1e-6,
            "32-bit wrote {red} rather than the scene-linear {expected}"
        );

        let mut eight = document(Depth::Eight);
        eight.layers.clear();
        eight.composite = layer("Composite", 0.18, 12);
        let bytes = eight.encode().unwrap();
        let start = bytes.len() - 4 * 12;
        // Mid-grey through the default tone map lands in the middle of the
        // range, not down at code 46 where a linear value would.
        assert!(
            bytes[start] > 80,
            "8-bit wrote code {} for mid-grey, which is a linear value",
            bytes[start]
        );
    }

    /// A mismatched layer is refused rather than written short.
    #[test]
    fn a_mismatched_layer_is_refused() {
        let mut image = document(Depth::Eight);
        image.layers[0].pixels.truncate(3);
        assert!(matches!(image.encode(), Err(PsdError::Size { .. })));
    }

    /// PSD's own size limit. Past it the format is PSB, which is a different
    /// file — saying so is better than writing a header nothing can open.
    #[test]
    fn an_oversized_document_is_refused() {
        let mut image = document(Depth::Eight);
        image.width = 40_000;
        assert!(matches!(image.encode(), Err(PsdError::TooLarge(..))));
    }
}
