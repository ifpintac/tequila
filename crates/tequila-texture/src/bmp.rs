//! Windows bitmaps (`.bmp`).
//!
//! Rarely anyone's first choice for a texture, but common enough in the
//! files that arrive with older models and from Windows tools that a map
//! saved as one should not render flat. Small and fully specified, so read
//! here rather than through a crate.
//!
//! What is read: 1-, 4- and 8-bit palette images, 16-bit (five bits a
//! channel by default), 24-bit and 32-bit, uncompressed (`BI_RGB`) or with
//! channel masks (`BI_BITFIELDS`, `BI_ALPHABITFIELDS`); every header from
//! the old 12-byte core header to V5. Rows run from the bottom up unless the
//! height is negative, and each is padded to four bytes.
//!
//! Alpha: masks say where it is when there are masks. A plain 32-bit file's
//! fourth byte is "reserved", and in practice is either zero throughout or
//! real alpha, which is how browsers read it too - so it is taken as alpha
//! unless it is zero everywhere. Run-length compressed bitmaps, and the
//! JPEG- or PNG-in-a-BMP variants, are refused with a message saying so.

/// Decode a BMP to 8-bit RGBA, unassociated, from the top row down.
pub fn decode(bytes: &[u8]) -> Result<(usize, usize, Vec<u8>), String> {
    let header = Header::read(bytes)?;
    let Header { width, height, top_down, bits, .. } = header;

    let masks: Option<[u32; 4]> = match header.compression {
        0 => None,
        3 | 6 => {
            // Straight after the 40-byte header, or inside a longer one at
            // the same place; alpha only where there is room for it.
            let at = 14 + 40;
            let mask = |i: usize| u32_at(bytes, at + 4 * i).ok_or("the BMP's channel masks are cut short");
            let alpha = if header.compression == 6 || header.size >= 56 { mask(3)? } else { 0 };
            Some([mask(0)?, mask(1)?, mask(2)?, alpha])
        }
        1 | 2 => return Err("this BMP is run-length compressed, which this reader does not do; re-save it uncompressed".into()),
        4 | 5 => return Err("this BMP wraps a JPEG or PNG; use that picture itself".into()),
        other => return Err(format!("BMP compression {other} is not one this reader knows")),
    };
    if masks.is_some() && !matches!(bits, 16 | 32) {
        return Err(format!("a {bits}-bit BMP cannot have channel masks"));
    }

    let palette: Vec<[u8; 4]> = if bits <= 8 {
        let entry = if header.size == 12 { 3 } else { 4 };
        let after_masks = match (header.size, header.compression) {
            (40, 3) => 12,
            (40, 6) => 16,
            _ => 0,
        };
        let at = 14 + header.size + after_masks;
        let count = if header.colours_used == 0 { 1usize << bits } else { header.colours_used.min(1 << bits) };
        let table = bytes.get(at..at + count * entry).ok_or("the BMP's palette is cut short")?;
        table.chunks_exact(entry).map(|p| [p[2], p[1], p[0], 255]).collect()
    } else {
        Vec::new()
    };

    let stride = (width * bits as usize).div_ceil(32) * 4;
    let start = u32_at(bytes, 10).ok_or("the BMP is cut short")? as usize;
    let data = bytes.get(start..start + stride * height).ok_or("the BMP's pixels are cut short")?;

    // Five bits a channel is what a 16-bit file without masks means.
    let masks = masks.unwrap_or(match bits {
        16 => [0x7C00, 0x03E0, 0x001F, 0],
        _ => [0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000],
    });
    let channels = masks.map(Channel::new);

    let mut out = vec![0u8; width * height * 4];
    for y in 0..height {
        let row = &data[(if top_down { y } else { height - 1 - y }) * stride..][..stride];
        for x in 0..width {
            let pixel: [u8; 4] = match bits {
                1 | 2 | 4 | 8 => {
                    let bit = x * bits as usize;
                    let index = (row[bit / 8] >> (8 - bits as usize - bit % 8)) & ((1u16 << bits) - 1) as u8;
                    *palette
                        .get(index as usize)
                        .ok_or_else(|| format!("the BMP uses colour {index}, which its palette does not have"))?
                }
                24 => [row[x * 3 + 2], row[x * 3 + 1], row[x * 3], 255],
                16 | 32 => {
                    let n = bits as usize / 8;
                    let mut word = [0u8; 4];
                    word[..n].copy_from_slice(&row[x * n..x * n + n]);
                    let v = u32::from_le_bytes(word);
                    [channels[0].read(v, 0), channels[1].read(v, 0), channels[2].read(v, 0), channels[3].read(v, 255)]
                }
                other => return Err(format!("a {other}-bit BMP is not one this reader knows")),
            };
            out[(y * width + x) * 4..][..4].copy_from_slice(&pixel);
        }
    }
    if bits == 32 && out.chunks_exact(4).all(|p| p[3] == 0) {
        out.chunks_exact_mut(4).for_each(|p| p[3] = 255);
    }
    Ok((width, height, out))
}

/// Pixel size and pixels per metre across and down, for the print size.
pub fn resolution(bytes: &[u8]) -> Option<(f32, f32, f32, f32)> {
    let header = Header::read(bytes).ok()?;
    if header.size < 40 {
        return None;
    }
    let x = i32::from_le_bytes(bytes.get(38..42)?.try_into().ok()?);
    let y = i32::from_le_bytes(bytes.get(42..46)?.try_into().ok()?);
    Some((header.width as f32, header.height as f32, x as f32, y as f32))
}

struct Header {
    /// Of the DIB header: 12 for the old core header, 40 and up for the rest.
    size: usize,
    width: usize,
    height: usize,
    top_down: bool,
    bits: u16,
    compression: u32,
    colours_used: usize,
}

impl Header {
    fn read(bytes: &[u8]) -> Result<Header, String> {
        if bytes.len() < 26 || &bytes[..2] != b"BM" {
            return Err("not a BMP".into());
        }
        let size = u32_at(bytes, 14).ok_or("not a BMP")? as usize;
        let u16_at = |at: usize| bytes.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or("the BMP's header is cut short");
        let (width, height, bits, compression, colours_used) = if size == 12 {
            (u16_at(18)? as i64, u16_at(20)? as i64, u16_at(24)?, 0, 0)
        } else if size >= 40 {
            let i32_at = |at: usize| u32_at(bytes, at).map(|v| v as i32 as i64).ok_or("the BMP's header is cut short");
            (i32_at(18)?, i32_at(22)?, u16_at(28)?, u32_at(bytes, 30).ok_or("the BMP's header is cut short")?, u32_at(bytes, 46).unwrap_or(0) as usize)
        } else {
            return Err(format!("a BMP header of {size} bytes is not one this reader knows"));
        };
        if width <= 0 || height == 0 {
            return Err("the BMP has no pixels".into());
        }
        Ok(Header {
            size,
            width: width as usize,
            height: height.unsigned_abs() as usize,
            top_down: height < 0,
            bits,
            compression,
            colours_used,
        })
    }
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// One channel's place in a pixel, from its mask.
#[derive(Clone, Copy)]
struct Channel {
    mask: u32,
    shift: u32,
    max: u32,
}

impl Channel {
    fn new(mask: u32) -> Channel {
        let shift = if mask == 0 { 0 } else { mask.trailing_zeros() };
        Channel { mask, shift, max: mask.checked_shr(shift).unwrap_or(0) }
    }

    /// The channel as eight bits, widened or narrowed from however many it
    /// has; `absent` when the mask is empty.
    fn read(self, pixel: u32, absent: u8) -> u8 {
        if self.max == 0 {
            return absent;
        }
        let v = ((pixel & self.mask) >> self.shift) as u64;
        ((v * 255 + self.max as u64 / 2) / self.max as u64) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A BMP with a 40-byte header (or `size` bytes), `extra` between the
    /// header and the pixels (masks, a palette), and rows as given, each
    /// padded here to four bytes.
    fn bmp(size: usize, width: i32, height: i32, bits: u16, compression: u32, extra: &[u8], rows: &[Vec<u8>]) -> Vec<u8> {
        let mut dib = vec![0u8; size];
        dib[0..4].copy_from_slice(&(size as u32).to_le_bytes());
        dib[4..8].copy_from_slice(&width.to_le_bytes());
        dib[8..12].copy_from_slice(&height.to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&bits.to_le_bytes());
        dib[16..20].copy_from_slice(&compression.to_le_bytes());
        // 2835 pixels a metre: 72 dpi.
        dib[24..28].copy_from_slice(&2835i32.to_le_bytes());
        dib[28..32].copy_from_slice(&2835i32.to_le_bytes());
        let mut pixels = Vec::new();
        for row in rows {
            let mut row = row.clone();
            while row.len() % 4 != 0 {
                row.push(0);
            }
            pixels.extend_from_slice(&row);
        }
        let offset = 14 + size + extra.len();
        let mut out = b"BM".to_vec();
        out.extend_from_slice(&((offset + pixels.len()) as u32).to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        out.extend_from_slice(&dib);
        out.extend_from_slice(extra);
        out.extend_from_slice(&pixels);
        out
    }

    fn pixel(image: &(usize, usize, Vec<u8>), x: usize, y: usize) -> [u8; 4] {
        let i = (y * image.0 + x) * 4;
        image.2[i..i + 4].try_into().unwrap()
    }

    /// 24-bit, three pixels wide so every row carries three bytes of
    /// padding; bottom-up, the default.
    #[test]
    fn twenty_four_bit_rows_are_padded_and_run_from_the_bottom() {
        let bottom = vec![0, 0, 255, 0, 255, 0, 255, 0, 0];
        let top = vec![255, 255, 255, 0, 0, 0, 128, 128, 128];
        let file = bmp(40, 3, 2, 24, 0, &[], &[bottom.clone(), top.clone()]);
        let image = decode(&file).unwrap();
        assert_eq!((image.0, image.1), (3, 2));
        assert_eq!(pixel(&image, 0, 1), [255, 0, 0, 255], "the first stored row is the bottom one");
        assert_eq!(pixel(&image, 1, 1), [0, 255, 0, 255]);
        assert_eq!(pixel(&image, 2, 1), [0, 0, 255, 255], "BGR order, and the padding skipped");
        assert_eq!(pixel(&image, 0, 0), [255, 255, 255, 255]);
        assert_eq!(pixel(&image, 2, 0), [128, 128, 128, 255]);

        // A negative height is top-down.
        let file = bmp(40, 3, -2, 24, 0, &[], &[bottom, top]);
        let image = decode(&file).unwrap();
        assert_eq!(pixel(&image, 0, 0), [255, 0, 0, 255]);
        assert_eq!(pixel(&image, 0, 1), [255, 255, 255, 255]);
    }

    /// A plain 32-bit file's fourth byte is alpha when it holds anything,
    /// and nothing at all when it is zero throughout.
    #[test]
    fn thirty_two_bit_alpha_where_there_is_some() {
        let row = vec![10, 20, 30, 0, 10, 20, 30, 200];
        let image = decode(&bmp(40, 2, 1, 32, 0, &[], &[row])).unwrap();
        assert_eq!(pixel(&image, 0, 0), [30, 20, 10, 0]);
        assert_eq!(pixel(&image, 1, 0), [30, 20, 10, 200]);

        let row = vec![10, 20, 30, 0, 40, 50, 60, 0];
        let image = decode(&bmp(40, 2, 1, 32, 0, &[], &[row])).unwrap();
        assert_eq!(pixel(&image, 1, 0), [60, 50, 40, 255], "an all-zero fourth byte is not alpha");
    }

    /// Masks: a V5 header with RGBA in an unusual order, and a 40-byte
    /// header with 5-6-5 masks after it.
    #[test]
    fn bitfields_put_each_channel_where_its_mask_says() {
        // V5: masks inside the header, here red in the low byte and alpha in
        // the high one (RGBA in memory).
        let mut file = bmp(124, 1, 1, 32, 3, &[], &[vec![200, 100, 50, 25]]);
        let masks = [0x0000_00FFu32, 0x0000_FF00, 0x00FF_0000, 0xFF00_0000];
        for (i, m) in masks.iter().enumerate() {
            file[54 + 4 * i..58 + 4 * i].copy_from_slice(&m.to_le_bytes());
        }
        let image = decode(&file).unwrap();
        assert_eq!(pixel(&image, 0, 0), [200, 100, 50, 25]);

        let mut masks = Vec::new();
        for m in [0xF800u32, 0x07E0, 0x001F] {
            masks.extend_from_slice(&m.to_le_bytes());
        }
        // Full red and half green (32 of 63) in 5-6-5.
        let v: u16 = (0x1F << 11) | (32 << 5);
        let image = decode(&bmp(40, 1, 1, 16, 3, &masks, &[v.to_le_bytes().to_vec()])).unwrap();
        assert_eq!(pixel(&image, 0, 0), [255, 130, 0, 255]);
    }

    /// Eight- and four-bit palettes, with a palette shorter than the depth
    /// allows (`biClrUsed`).
    #[test]
    fn palette_images_index_their_palette() {
        // Two entries, BGRX: blue, then orange.
        let palette = [255, 0, 0, 0, 0, 128, 255, 0];
        let mut file = bmp(40, 3, 1, 8, 0, &palette, &[vec![1, 0, 1]]);
        file[14 + 32..14 + 36].copy_from_slice(&2u32.to_le_bytes());
        let image = decode(&file).unwrap();
        assert_eq!(pixel(&image, 0, 0), [255, 128, 0, 255]);
        assert_eq!(pixel(&image, 1, 0), [0, 0, 255, 255]);

        // Four bits: two pixels a byte, high nibble first.
        let mut file = bmp(40, 3, 1, 4, 0, &palette, &[vec![0x10, 0x10]]);
        file[14 + 32..14 + 36].copy_from_slice(&2u32.to_le_bytes());
        let image = decode(&file).unwrap();
        assert_eq!(pixel(&image, 0, 0), [255, 128, 0, 255]);
        assert_eq!(pixel(&image, 1, 0), [0, 0, 255, 255]);
        assert_eq!(pixel(&image, 2, 0), [255, 128, 0, 255]);

        // An index past the palette is an error.
        let mut file = bmp(40, 1, 1, 8, 0, &palette, &[vec![7]]);
        file[14 + 32..14 + 36].copy_from_slice(&2u32.to_le_bytes());
        assert!(decode(&file).is_err());
    }

    #[test]
    fn run_length_and_truncated_files_are_refused_with_reasons() {
        let error = decode(&bmp(40, 1, 1, 8, 1, &[0; 1024], &[vec![0]])).unwrap_err();
        assert!(error.contains("run-length"), "{error}");
        let mut file = bmp(40, 4, 4, 24, 0, &[], &vec![vec![0; 12]; 4]);
        file.truncate(file.len() - 10);
        assert!(decode(&file).is_err());
    }

    #[test]
    fn the_resolution_is_in_pixels_per_metre() {
        let file = bmp(40, 3, -2, 24, 0, &[], &[vec![0; 9], vec![0; 9]]);
        assert_eq!(resolution(&file), Some((3.0, 2.0, 2835.0, 2835.0)));
    }
}
