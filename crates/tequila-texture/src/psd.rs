//! Photoshop documents, read for their flattened image.
//!
//! A label's artwork very often arrives as the designer's own `.psd`.
//! Tequila does not need the layers: a PSD (and a PSB, Photoshop's large
//! document) also stores the whole image composited - the "merged image
//! data" at the end of the file, which is what any viewer that is not
//! Photoshop shows - and that is the label. Its resolution resource says
//! what it was made to print at.
//!
//! What is read: 8, 16 and 32 bits a channel; greyscale and RGB, with the
//! merged transparency when there is one; raw or PackBits (RLE)
//! compression, which is all Photoshop writes for the merged image.
//!
//! A channel past the colour ones is the composite's transparency only
//! sometimes - it can as well be a selection someone saved, and taking that
//! for alpha would cut holes in the label. The format says a negative layer
//! count marks transparency; failing that, extra channels whose identifiers
//! (resource 1053) are all set are saved selections and anything else is
//! transparency, which is how psd-tools decides. CMYK, Lab, indexed and
//! duotone documents are refused with a message saying so. A document saved
//! with "Maximize Compatibility" off holds no real composite - Photoshop
//! writes a blank one - and comes out blank; there is no way to tell it
//! apart from a blank image.

/// A PSD's composite: RGBA, unassociated, 0 to 1 - still display-encoded at
/// 8 and 16 bits, and linear at 32, as Photoshop keeps its 32-bit documents
/// (where a value may also pass 1).
#[derive(Debug)]
pub struct Composite {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<f32>,
    pub linear: bool,
}

/// Read the merged image of a PSD or PSB.
pub fn composite(bytes: &[u8]) -> Result<Composite, String> {
    let mut r = Reader { bytes, at: 0 };
    if r.take(4)? != b"8BPS" {
        return Err("not a Photoshop document".into());
    }
    let version = r.u16()?;
    let big = match version {
        1 => false,
        2 => true,
        other => return Err(format!("Photoshop file version {other} is not one this reads")),
    };
    r.skip(6)?;
    let channels = r.u16()? as usize;
    let height = r.u32()? as usize;
    let width = r.u32()? as usize;
    let depth = r.u16()?;
    let mode = r.u16()?;
    let colour_channels = match mode {
        1 => 1,
        3 => 3,
        other => {
            return Err(format!(
                "a {} Photoshop document; save the label as RGB or greyscale",
                match other {
                    0 => "bitmap",
                    2 => "indexed-colour",
                    4 => "CMYK",
                    7 => "multichannel",
                    8 => "duotone",
                    9 => "Lab",
                    _ => "differently coloured",
                }
            ))
        }
    };
    if !matches!(depth, 8 | 16 | 32) {
        return Err(format!("{depth} bits a channel is not a depth this reads"));
    }
    if width == 0 || height == 0 || channels < colour_channels {
        return Err("the document has no image".into());
    }
    // Colour mode data: past it.
    let length = r.u32()? as usize;
    r.skip(length)?;
    // Image resources: whether the extra channels are saved selections.
    let length = r.u32()? as usize;
    let selections = resources(r.take(length)?)
        .find(|(id, _)| *id == 1053)
        .is_some_and(|(_, data)| data.len() >= 4 && data.chunks_exact(4).all(|id| id != [0, 0, 0, 0]));
    // Layers: past them, but for the sign of their count.
    let length = if big { r.u64()? as usize } else { r.u32()? as usize };
    let layers = r.take(length)?;
    let info = if big { 8 } else { 4 };
    let marked = layers.len() >= info + 2 && {
        let count = i16::from_be_bytes([layers[info], layers[info + 1]]);
        layers[..info].iter().any(|&b| b != 0) && count < 0
    };

    let compression = r.u16()?;
    let bytes_per = (depth / 8) as usize;
    let row_bytes = width * bytes_per;
    // Planar: every channel's rows in turn.
    let mut planes: Vec<Vec<u8>> = Vec::with_capacity(channels);
    match compression {
        0 => {
            for _ in 0..channels {
                planes.push(r.take(row_bytes * height)?.to_vec());
            }
        }
        1 => {
            // A table of every row's packed length, then the rows.
            let mut lengths = Vec::with_capacity(channels * height);
            for _ in 0..channels * height {
                lengths.push(if big { r.u32()? as usize } else { r.u16()? as usize });
            }
            for c in 0..channels {
                let mut plane = Vec::with_capacity(row_bytes * height);
                for y in 0..height {
                    let packed = r.take(lengths[c * height + y])?;
                    unpack_bits(packed, row_bytes, &mut plane)?;
                }
                planes.push(plane);
            }
        }
        other => return Err(format!("merged-image compression {other} is not one this reads")),
    }

    // Each sample as 0 to 1.
    let sample = |plane: &[u8], i: usize| -> f32 {
        match bytes_per {
            1 => plane[i] as f32 / 255.0,
            2 => u16::from_be_bytes([plane[2 * i], plane[2 * i + 1]]) as f32 / 65535.0,
            _ => {
                let v = f32::from_be_bytes([plane[4 * i], plane[4 * i + 1], plane[4 * i + 2], plane[4 * i + 3]]);
                if v.is_finite() { v.max(0.0) } else { 0.0 }
            }
        }
    };
    let has_alpha = channels > colour_channels && (marked || !selections);
    let mut rgba = Vec::with_capacity(width * height * 4);
    for i in 0..width * height {
        let alpha = if has_alpha { sample(&planes[colour_channels], i).min(1.0) } else { 1.0 };
        for k in 0..3 {
            let mut c = sample(&planes[if colour_channels == 1 { 0 } else { k }], i);
            // The merged image is matted against white where it is
            // transparent; take the white back out.
            if has_alpha && alpha > 0.0 && alpha < 1.0 {
                c = ((c - (1.0 - alpha)) / alpha).max(0.0);
            }
            rgba.push(if depth == 32 { c } else { c.min(1.0) });
        }
        rgba.push(alpha);
    }
    Ok(Composite { width, height, rgba, linear: depth == 32 })
}

/// Pixels per metre across and down, from the resolution resource (1005),
/// with the image's size in pixels.
pub fn resolution(bytes: &[u8]) -> Option<(f32, f32, f32, f32)> {
    let mut r = Reader { bytes, at: 0 };
    if r.take(4).ok()? != b"8BPS" {
        return None;
    }
    // Version, reserved, channels.
    r.skip(10).ok()?;
    let height = r.u32().ok()? as f32;
    let width = r.u32().ok()? as f32;
    r.skip(4).ok()?;
    let length = r.u32().ok()? as usize;
    r.skip(length).ok()?;
    let length = r.u32().ok()? as usize;
    let (_, data) = resources(r.take(length).ok()?).find(|(id, data)| *id == 1005 && data.len() >= 16)?;
    let mut r = Reader { bytes: data, at: 0 };
    let fixed = |r: &mut Reader| r.u32().map(|v| v as f32 / 65536.0);
    let horizontal = fixed(&mut r).ok()?;
    let horizontal_unit = r.u16().ok()?;
    r.skip(2).ok()?;
    let vertical = fixed(&mut r).ok()?;
    let vertical_unit = r.u16().ok()?;
    // Units: 1 pixels per inch, 2 pixels per centimetre.
    let per_metre = |value: f32, unit: u16| if unit == 2 { value * 100.0 } else { value / 0.0254 };
    Some((width, height, per_metre(horizontal, horizontal_unit), per_metre(vertical, vertical_unit)))
}

/// The image resources in `section`, each its id and data, until one is
/// malformed.
fn resources(section: &[u8]) -> impl Iterator<Item = (u16, &[u8])> {
    let mut r = Reader { bytes: section, at: 0 };
    std::iter::from_fn(move || {
        if r.take(4).ok()? != b"8BIM" {
            return None;
        }
        let id = r.u16().ok()?;
        // A Pascal name, padded to an even length.
        let name = r.take(1).ok()?[0] as usize;
        r.skip(if (name + 1) % 2 == 0 { name } else { name + 1 }).ok()?;
        let size = r.u32().ok()? as usize;
        let data = r.take(size).ok()?;
        // Data padded to an even length; the last may stop short.
        let _ = r.skip(size % 2);
        Some((id, data))
    })
}

/// PackBits: a byte n, then n+1 bytes as they are when n < 128, or the next
/// byte 257-n times when n > 128.
fn unpack_bits(packed: &[u8], length: usize, out: &mut Vec<u8>) -> Result<(), String> {
    let end = out.len() + length;
    let mut i = 0;
    while out.len() < end && i < packed.len() {
        let n = packed[i] as i8;
        i += 1;
        if n >= 0 {
            let count = n as usize + 1;
            let run = packed.get(i..i + count).ok_or("a packed row runs short")?;
            out.extend_from_slice(run);
            i += count;
        } else if n != -128 {
            let count = (1 - n as isize) as usize;
            let value = *packed.get(i).ok_or("a packed row runs short")?;
            out.extend(std::iter::repeat_n(value, count));
            i += 1;
        }
    }
    if out.len() != end {
        return Err("a packed row is the wrong length".into());
    }
    Ok(())
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let slice = self.bytes.get(self.at..self.at + n).ok_or("the file ends early")?;
        self.at += n;
        Ok(slice)
    }
    fn skip(&mut self, n: usize) -> Result<(), String> {
        self.take(n).map(|_| ())
    }
    fn u16(&mut self) -> Result<u16, String> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32, String> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&mut self) -> Result<u64, String> {
        let b = self.take(8)?;
        Ok(u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A PSD of `pixels` (RGBA, 8-bit), RLE-packed when `rle`, at `dpi`.
    pub(crate) fn write(width: usize, height: usize, pixels: &[[u8; 4]], rle: bool, dpi: Option<f32>) -> Vec<u8> {
        write_with(width, height, pixels, rle, dpi, &[], 0)
    }

    /// The same, saying the fourth channel's identifier (resource 1053)
    /// when `alpha_ids` has one, and with a layer section holding only a
    /// count when `layer_count` is not 0.
    pub(crate) fn write_with(
        width: usize,
        height: usize,
        pixels: &[[u8; 4]],
        rle: bool,
        dpi: Option<f32>,
        alpha_ids: &[u32],
        layer_count: i16,
    ) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"8BPS");
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&[0; 6]);
        out.extend_from_slice(&4u16.to_be_bytes());
        out.extend_from_slice(&(height as u32).to_be_bytes());
        out.extend_from_slice(&(width as u32).to_be_bytes());
        out.extend_from_slice(&8u16.to_be_bytes());
        out.extend_from_slice(&3u16.to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes());
        let mut resources = Vec::new();
        if let Some(dpi) = dpi {
            resources.extend_from_slice(b"8BIM");
            resources.extend_from_slice(&1005u16.to_be_bytes());
            resources.extend_from_slice(&[0, 0]);
            resources.extend_from_slice(&16u32.to_be_bytes());
            for _ in 0..2 {
                resources.extend_from_slice(&((dpi * 65536.0) as u32).to_be_bytes());
                resources.extend_from_slice(&1u16.to_be_bytes());
                resources.extend_from_slice(&2u16.to_be_bytes());
            }
        }
        if !alpha_ids.is_empty() {
            resources.extend_from_slice(b"8BIM");
            resources.extend_from_slice(&1053u16.to_be_bytes());
            resources.extend_from_slice(&[0, 0]);
            resources.extend_from_slice(&(alpha_ids.len() as u32 * 4).to_be_bytes());
            for id in alpha_ids {
                resources.extend_from_slice(&id.to_be_bytes());
            }
        }
        out.extend_from_slice(&(resources.len() as u32).to_be_bytes());
        out.extend_from_slice(&resources);
        if layer_count == 0 {
            out.extend_from_slice(&0u32.to_be_bytes());
        } else {
            out.extend_from_slice(&6u32.to_be_bytes());
            out.extend_from_slice(&2u32.to_be_bytes());
            out.extend_from_slice(&layer_count.to_be_bytes());
        }
        // Matted against white, as Photoshop writes the merged image.
        let plane = |c: usize| -> Vec<u8> {
            pixels
                .iter()
                .map(|p| {
                    if c == 3 {
                        p[3]
                    } else {
                        let a = p[3] as f32 / 255.0;
                        (p[c] as f32 * a + 255.0 * (1.0 - a)).round() as u8
                    }
                })
                .collect()
        };
        if rle {
            out.extend_from_slice(&1u16.to_be_bytes());
            let mut rows = Vec::new();
            let mut lengths = Vec::new();
            for c in 0..4 {
                let p = plane(c);
                for y in 0..height {
                    // Every row as literal runs of up to 128.
                    let row = &p[y * width..(y + 1) * width];
                    let mut packed = Vec::new();
                    for chunk in row.chunks(128) {
                        packed.push((chunk.len() - 1) as u8);
                        packed.extend_from_slice(chunk);
                    }
                    lengths.push(packed.len() as u16);
                    rows.extend_from_slice(&packed);
                }
            }
            for l in lengths {
                out.extend_from_slice(&l.to_be_bytes());
            }
            out.extend_from_slice(&rows);
        } else {
            out.extend_from_slice(&0u16.to_be_bytes());
            for c in 0..4 {
                out.extend_from_slice(&plane(c));
            }
        }
        out
    }

    #[test]
    fn a_psd_composite_comes_back_as_it_was() {
        let pixels: Vec<[u8; 4]> = (0..12).map(|i| [(i * 20) as u8, 200 - (i * 10) as u8, 90, if i % 3 == 0 { 128 } else { 255 }]).collect();
        for rle in [false, true] {
            let file = write(4, 3, &pixels, rle, None);
            let c = composite(&file).expect("reads");
            assert_eq!((c.width, c.height), (4, 3));
            assert!(!c.linear);
            for (i, p) in pixels.iter().enumerate() {
                let got: Vec<i32> = c.rgba[i * 4..i * 4 + 4].iter().map(|v| (v * 255.0).round() as i32).collect();
                assert_eq!(got[3], p[3] as i32);
                for k in 0..3 {
                    assert!((got[k] - p[k] as i32).abs() <= 2, "rle {rle}, pixel {i}: {got:?} for {p:?}");
                }
            }
        }
    }

    #[test]
    fn a_psd_says_its_resolution() {
        let file = write(600, 300, &vec![[255, 255, 255, 255]; 600 * 300], true, Some(300.0));
        let (w, h, x, y) = resolution(&file).expect("a resolution");
        assert_eq!((w, h), (600.0, 300.0));
        assert!((x - 300.0 / 0.0254).abs() < 1.0 && (y - x).abs() < 1e-3);
    }

    /// A fourth channel that is a saved selection is not the label's
    /// transparency - unless the layer count says the composite has one.
    #[test]
    fn a_saved_selection_is_not_transparency() {
        let pixels = vec![[200, 100, 50, 0]; 4];
        let alpha = |file: Vec<u8>| composite(&file).expect("reads").rgba[3];
        assert_eq!(alpha(write_with(2, 2, &pixels, false, None, &[7], 0)), 1.0, "a selection was taken for alpha");
        assert_eq!(alpha(write_with(2, 2, &pixels, false, None, &[7], 3)), 1.0);
        assert_eq!(alpha(write_with(2, 2, &pixels, false, None, &[7], -3)), 0.0, "marked transparency was missed");
        assert_eq!(alpha(write_with(2, 2, &pixels, false, None, &[], 0)), 0.0);
        assert_eq!(alpha(write_with(2, 2, &pixels, false, None, &[0], 0)), 0.0);
    }

    #[test]
    fn packbits_unpacks_runs_and_literals() {
        let mut out = Vec::new();
        // Three literal bytes, then 4 repeated.
        unpack_bits(&[2, 1, 2, 3, (-3i8) as u8, 9], 7, &mut out).unwrap();
        assert_eq!(out, vec![1, 2, 3, 9, 9, 9, 9]);
    }

    #[test]
    fn a_cmyk_document_says_so() {
        let mut file = write(2, 2, &vec![[0, 0, 0, 255]; 4], false, None);
        file[25] = 4;
        let error = composite(&file).err().expect("refused");
        assert!(error.contains("CMYK"), "{error}");
    }
}
