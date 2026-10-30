//! Truevision TGA (`.tga`) decoder.
//!
//! Still the format game engines and a good many texture painters export
//! by default, so a model from that world very often brings its maps as
//! TGAs. The format is small and fully specified, which is why it is read
//! here rather than through a crate.
//!
//! What is read: true-colour at 15, 16, 24 and 32 bits, greyscale at 8 bits
//! (and 16, as grey with alpha), and colour-mapped images with 8- or 16-bit
//! indices into a palette of any of the true-colour depths; each raw or
//! run-length encoded. Both origins are honoured - TGA stores its rows from
//! the bottom up unless the descriptor says otherwise - and so is the rare
//! right-to-left flag.
//!
//! Alpha is the part writers disagree about. The descriptor's count of
//! alpha bits is meant to say whether there is any, and a TGA 2.0 file's
//! extension area says outright (none, meaningful, or premultiplied); that
//! is believed when it is there. Plenty of writers leave the alpha bits at
//! zero over a real alpha channel, and a few fill a 32-bit file's fourth
//! byte with zeros meaning nothing, so failing an extension area a fourth
//! byte that is zero everywhere is read as opaque rather than as an image
//! nobody can see.

/// Decode a TGA file to 8-bit RGBA, unassociated, from the top row down.
pub fn decode(bytes: &[u8]) -> Result<(usize, usize, Vec<u8>), String> {
    if bytes.len() < 18 {
        return Err("too short to be a TGA".into());
    }
    let u16_at = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]) as usize;
    let id_length = bytes[0] as usize;
    let map_type = bytes[1];
    let image_type = bytes[2];
    let (map_first, map_length, map_bits) = (u16_at(3), u16_at(5), bytes[7] as usize);
    let (width, height) = (u16_at(12), u16_at(14));
    let depth = bytes[16] as usize;
    let descriptor = bytes[17];
    let alpha_bits = descriptor & 0x0F;
    let right_to_left = descriptor & 0x10 != 0;
    let top_down = descriptor & 0x20 != 0;

    let (kind, rle) = match image_type {
        1 => (Kind::Mapped, false),
        2 => (Kind::TrueColour, false),
        3 => (Kind::Grey, false),
        9 => (Kind::Mapped, true),
        10 => (Kind::TrueColour, true),
        11 => (Kind::Grey, true),
        0 => return Err("the TGA holds no image".into()),
        other => return Err(format!("TGA image type {other} is not one this reader knows")),
    };
    if width == 0 || height == 0 {
        return Err("the TGA has no pixels".into());
    }
    let pixel_bytes = match (kind, depth) {
        (Kind::TrueColour, 15 | 16) | (Kind::Mapped, 16) | (Kind::Grey, 16) => 2,
        (Kind::TrueColour, 24) => 3,
        (Kind::TrueColour, 32) => 4,
        (Kind::Grey | Kind::Mapped, 8) => 1,
        _ => return Err(format!("a {depth}-bit {kind:?} TGA is not one this reader knows")),
    };

    let mut at = 18 + id_length;
    // The palette, as RGBA, indexed from `map_first`.
    let mut palette = Vec::new();
    if map_type == 1 {
        let entry_bytes = map_bits.div_ceil(8);
        let length = map_length * entry_bytes;
        let entries = bytes.get(at..at + length).ok_or("the TGA's palette is cut short")?;
        if kind == Kind::Mapped {
            if !matches!(map_bits, 15 | 16 | 24 | 32) {
                return Err(format!("a {map_bits}-bit TGA palette is not one this reader knows"));
            }
            palette = entries.chunks_exact(entry_bytes).map(true_colour).collect();
        }
        // A palette on a true-colour image is allowed and means nothing.
        at += length;
    } else if kind == Kind::Mapped {
        return Err("the TGA is colour-mapped but has no palette".into());
    }

    // The pixel values as stored, `pixel_bytes` each, in file order.
    let count = width * height;
    let stored: Vec<u8> = if rle {
        let mut out = Vec::with_capacity(count * pixel_bytes);
        // Packets may run across the end of a row; the spec says they should
        // not, and writers do it anyway, so the stream is read whole.
        while out.len() < count * pixel_bytes {
            let header = *bytes.get(at).ok_or("the TGA's pixels are cut short")?;
            at += 1;
            let run = (header & 0x7F) as usize + 1;
            if header & 0x80 != 0 {
                let pixel = bytes.get(at..at + pixel_bytes).ok_or("the TGA's pixels are cut short")?;
                for _ in 0..run {
                    out.extend_from_slice(pixel);
                }
                at += pixel_bytes;
            } else {
                let pixels = bytes.get(at..at + run * pixel_bytes).ok_or("the TGA's pixels are cut short")?;
                out.extend_from_slice(pixels);
                at += run * pixel_bytes;
            }
        }
        out.truncate(count * pixel_bytes);
        out
    } else {
        bytes.get(at..at + count * pixel_bytes).ok_or("the TGA's pixels are cut short")?.to_vec()
    };

    let mut rgba: Vec<[u8; 4]> = match kind {
        Kind::TrueColour => stored.chunks_exact(pixel_bytes).map(true_colour).collect(),
        Kind::Grey if pixel_bytes == 2 => stored.chunks_exact(2).map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        Kind::Grey => stored.iter().map(|&v| [v, v, v, 255]).collect(),
        Kind::Mapped => {
            let mut out = Vec::with_capacity(count);
            for p in stored.chunks_exact(pixel_bytes) {
                let index = if pixel_bytes == 2 { u16::from_le_bytes([p[0], p[1]]) as usize } else { p[0] as usize };
                let entry = index
                    .checked_sub(map_first)
                    .and_then(|i| palette.get(i))
                    .ok_or_else(|| format!("the TGA uses colour {index}, which its palette does not have"))?;
                out.push(*entry);
            }
            out
        }
    };

    // Whether the alpha in the file means anything.
    let carries_alpha = match kind {
        Kind::TrueColour => matches!(depth, 16 | 32),
        Kind::Mapped => matches!(map_bits, 16 | 32),
        Kind::Grey => pixel_bytes == 2,
    };
    match (carries_alpha, extension_alpha(bytes)) {
        (false, _) | (true, Some(Alpha::None)) => rgba.iter_mut().for_each(|p| p[3] = 255),
        (true, Some(Alpha::Premultiplied)) => {
            for p in &mut rgba {
                if p[3] > 0 && p[3] < 255 {
                    for c in 0..3 {
                        p[c] = ((p[c] as u32 * 255 + p[3] as u32 / 2) / p[3] as u32).min(255) as u8;
                    }
                }
            }
        }
        (true, Some(Alpha::Straight)) => {}
        (true, None) => {
            // A 16-bit pixel's top bit is alpha only when the descriptor
            // claims it; it is as often a leftover of 15-bit colour.
            let claimed = alpha_bits > 0 || depth == 32 || (kind == Kind::Mapped && map_bits == 32);
            if !claimed || rgba.iter().all(|p| p[3] == 0) {
                rgba.iter_mut().for_each(|p| p[3] = 255);
            }
        }
    }

    // Into the top-down, left-to-right order the renderer keeps.
    let mut out = Vec::with_capacity(count * 4);
    for y in 0..height {
        let row = if top_down { y } else { height - 1 - y };
        for x in 0..width {
            let column = if right_to_left { width - 1 - x } else { x };
            out.extend_from_slice(&rgba[row * width + column]);
        }
    }
    Ok((width, height, out))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    TrueColour,
    Grey,
    Mapped,
}

/// What a TGA 2.0 extension area says the alpha is.
enum Alpha {
    None,
    Straight,
    Premultiplied,
}

/// The extension area's attributes type, if the file has a TGA 2.0 footer
/// pointing at one.
fn extension_alpha(bytes: &[u8]) -> Option<Alpha> {
    let footer = bytes.len().checked_sub(26)?;
    if &bytes[footer + 8..] != b"TRUEVISION-XFILE.\0" {
        return None;
    }
    let offset = u32::from_le_bytes(bytes[footer..footer + 4].try_into().ok()?) as usize;
    if offset == 0 || offset + 495 > footer {
        return None;
    }
    // 0: no alpha; 1 and 2: undefined data, to ignore or keep; 3: alpha;
    // 4: premultiplied alpha.
    match bytes[offset + 494] {
        0..=2 => Some(Alpha::None),
        3 => Some(Alpha::Straight),
        4 => Some(Alpha::Premultiplied),
        _ => None,
    }
}

/// One stored true-colour value, little-endian BGR(A), as RGBA. Fifteen-
/// and sixteen-bit values are five bits a channel, widened by repeating
/// their top bits so that 31 comes out as 255.
fn true_colour(p: &[u8]) -> [u8; 4] {
    match p.len() {
        2 => {
            let v = u16::from_le_bytes([p[0], p[1]]);
            let five = |shift: u16| {
                let c = ((v >> shift) & 0x1F) as u8;
                (c << 3) | (c >> 2)
            };
            [five(10), five(5), five(0), if v & 0x8000 != 0 { 255 } else { 0 }]
        }
        3 => [p[2], p[1], p[0], 255],
        _ => [p[2], p[1], p[0], p[3]],
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A TGA header for an image of `kind` (2, 3, 10 ...), `width` by
    /// `height` at `depth` bits, with `descriptor` as given.
    pub(crate) fn header(kind: u8, width: u16, height: u16, depth: u8, descriptor: u8) -> Vec<u8> {
        let mut out = vec![0u8, 0, kind];
        out.extend_from_slice(&[0; 5]);
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&width.to_le_bytes());
        out.extend_from_slice(&height.to_le_bytes());
        out.extend_from_slice(&[depth, descriptor]);
        out
    }

    fn pixel(image: &(usize, usize, Vec<u8>), x: usize, y: usize) -> [u8; 4] {
        let i = (y * image.0 + x) * 4;
        image.2[i..i + 4].try_into().unwrap()
    }

    /// Bottom-up is the default: the first row in the file is the bottom
    /// of the picture. Getting that wrong turns every map upside down.
    #[test]
    fn rows_are_read_from_the_bottom_unless_the_file_says_top() {
        // Two by two, 24-bit BGR: the file's first row is red then green,
        // its second blue then white.
        let rows = [[0, 0, 255], [0, 255, 0], [255, 0, 0], [255, 255, 255]];
        let body: Vec<u8> = rows.iter().flatten().copied().collect();

        let mut bottom_up = header(2, 2, 2, 24, 0);
        bottom_up.extend_from_slice(&body);
        let image = decode(&bottom_up).unwrap();
        assert_eq!((image.0, image.1), (2, 2));
        assert_eq!(pixel(&image, 0, 1), [255, 0, 0, 255], "the first stored row is the bottom one");
        assert_eq!(pixel(&image, 1, 1), [0, 255, 0, 255]);
        assert_eq!(pixel(&image, 0, 0), [0, 0, 255, 255]);

        let mut top_down = header(2, 2, 2, 24, 0x20);
        top_down.extend_from_slice(&body);
        let image = decode(&top_down).unwrap();
        assert_eq!(pixel(&image, 0, 0), [255, 0, 0, 255], "the top-origin bit puts it at the top");
        assert_eq!(pixel(&image, 1, 1), [255, 255, 255, 255]);

        let mut mirrored = header(2, 2, 2, 24, 0x30);
        mirrored.extend_from_slice(&body);
        let image = decode(&mirrored).unwrap();
        assert_eq!(pixel(&image, 0, 0), [0, 255, 0, 255], "right to left");
    }

    /// Run-length packets, a run and a literal, with one crossing the end
    /// of a row as many writers produce; and 32-bit alpha kept as stored.
    #[test]
    fn run_length_true_colour_keeps_its_alpha() {
        let mut file = header(10, 3, 2, 32, 0x28);
        // A run of four half-transparent reds, then two literals.
        file.push(0x80 | 3);
        file.extend_from_slice(&[0, 0, 255, 128]);
        file.push(1);
        file.extend_from_slice(&[255, 0, 0, 255, 0, 255, 0, 0]);
        let image = decode(&file).unwrap();
        assert_eq!(pixel(&image, 0, 0), [255, 0, 0, 128]);
        assert_eq!(pixel(&image, 2, 0), [255, 0, 0, 128]);
        assert_eq!(pixel(&image, 0, 1), [255, 0, 0, 128], "the run crossed into the second row");
        assert_eq!(pixel(&image, 1, 1), [0, 0, 255, 255]);
        assert_eq!(pixel(&image, 2, 1), [0, 255, 0, 0], "fully transparent stays transparent");
    }

    /// A 32-bit file whose fourth byte is zero throughout carries no alpha,
    /// whatever its descriptor claims; and a TGA 2.0 extension that says
    /// "no alpha" is believed over the bytes.
    #[test]
    fn an_empty_or_disowned_alpha_reads_as_opaque() {
        let mut file = header(2, 1, 1, 32, 0);
        file.extend_from_slice(&[10, 20, 30, 0]);
        assert_eq!(pixel(&decode(&file).unwrap(), 0, 0), [30, 20, 10, 255]);

        let mut file = header(2, 2, 1, 32, 8);
        file.extend_from_slice(&[10, 20, 30, 0, 10, 20, 30, 77]);
        let offset = file.len() as u32;
        let mut extension = vec![0u8; 495];
        extension[0..2].copy_from_slice(&495u16.to_le_bytes());
        extension[494] = 0;
        file.extend_from_slice(&extension);
        file.extend_from_slice(&offset.to_le_bytes());
        file.extend_from_slice(&[0; 4]);
        file.extend_from_slice(b"TRUEVISION-XFILE.\0");
        let image = decode(&file).unwrap();
        assert_eq!(pixel(&image, 1, 0), [30, 20, 10, 255], "the extension said there is no alpha");

        // The same file saying its alpha is premultiplied.
        let at = offset as usize + 494;
        file[at] = 4;
        let image = decode(&file).unwrap();
        assert_eq!(pixel(&image, 1, 0)[3], 77);
        assert_eq!(pixel(&image, 1, 0)[0], ((30u32 * 255 + 38) / 77) as u8, "the colour is divided by its alpha");
    }

    #[test]
    fn greyscale_and_grey_with_alpha() {
        let mut file = header(3, 2, 1, 8, 0x20);
        file.extend_from_slice(&[0, 200]);
        let image = decode(&file).unwrap();
        assert_eq!(pixel(&image, 1, 0), [200, 200, 200, 255]);

        let mut file = header(11, 2, 1, 16, 0x28);
        file.extend_from_slice(&[0x81, 90, 40]);
        let image = decode(&file).unwrap();
        assert_eq!(pixel(&image, 0, 0), [90, 90, 90, 40]);
        assert_eq!(pixel(&image, 1, 0), [90, 90, 90, 40]);
    }

    /// Colour-mapped, with a palette that starts past zero and 24-bit
    /// entries; run-length encoded.
    #[test]
    fn colour_mapped_indices_go_through_the_palette() {
        let mut file = header(9, 3, 1, 8, 0x20);
        file[1] = 1;
        file[3..5].copy_from_slice(&5u16.to_le_bytes());
        file[5..7].copy_from_slice(&2u16.to_le_bytes());
        file[7] = 24;
        // Entries 5 and 6: blue, then orange (BGR).
        file.extend_from_slice(&[255, 0, 0, 0, 128, 255]);
        file.extend_from_slice(&[0x81, 6, 0, 5]);
        let image = decode(&file).unwrap();
        assert_eq!(pixel(&image, 0, 0), [255, 128, 0, 255]);
        assert_eq!(pixel(&image, 1, 0), [255, 128, 0, 255]);
        assert_eq!(pixel(&image, 2, 0), [0, 0, 255, 255]);

        // An index the palette does not have is an error, not a black.
        let at = file.len() - 1;
        file[at] = 9;
        assert!(decode(&file).is_err());
    }

    /// Sixteen-bit colour widens 31 to 255, and its top bit is alpha only
    /// when the descriptor says there is one bit of it.
    #[test]
    fn sixteen_bit_colour_widens_to_eight() {
        // Pure red with the top bit clear.
        let red = (0x1Fu16 << 10).to_le_bytes();
        let mut file = header(2, 1, 1, 16, 0);
        file.extend_from_slice(&red);
        assert_eq!(pixel(&decode(&file).unwrap(), 0, 0), [255, 0, 0, 255], "no alpha bits: opaque");

        let mut file = header(2, 2, 1, 16, 1);
        file.extend_from_slice(&red);
        file.extend_from_slice(&(0x8000u16 | 0x1F).to_le_bytes());
        let image = decode(&file).unwrap();
        assert_eq!(pixel(&image, 0, 0), [255, 0, 0, 0], "one alpha bit, clear: transparent");
        assert_eq!(pixel(&image, 1, 0), [0, 0, 255, 255]);
    }

    #[test]
    fn a_truncated_file_is_an_error() {
        let mut file = header(2, 4, 4, 24, 0);
        file.extend_from_slice(&[0; 10]);
        assert!(decode(&file).is_err());
        let mut file = header(10, 4, 4, 24, 0);
        file.extend_from_slice(&[0x83, 1, 2]);
        assert!(decode(&file).is_err());
    }
}
