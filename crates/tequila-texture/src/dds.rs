//! DirectDraw Surfaces (`.dds`).
//!
//! The texture format of games and of the tools that make assets for them,
//! so a model from that world brings its maps as DDS - block-compressed,
//! the way a GPU reads them. The container is a fixed header and is read
//! here; the block formats BC1 to BC7 are decoded by `bcdec_rs`, a port of
//! bcdec, because BC6H and BC7 are each a small specification of their own.
//!
//! What is read: the top level of a flat texture (the first, for an array).
//! BC1 to BC5 under their legacy FourCCs (DXT1 to DXT5, ATI1, ATI2, BC4U/S,
//! BC5U/S) and, through the DX10 header, BC1 to BC7 including BC6H, which
//! is floating point and so linear. Uncompressed: any layout the legacy
//! header describes with channel masks (RGB, luminance, alpha), the legacy
//! float and 16-bit codes, and the common DXGI formats - 8 bits a channel
//! in either order, 10:10:10:2, 5:6:5, 5:5:5:1, 4:4:4:4, 8- and 16-bit one-
//! and two-channel, and 16- and 32-bit floats. Cube maps and volume
//! textures are refused; a texture is one picture.
//!
//! A DDS can say its colour is sRGB (the `_SRGB` DXGI formats), and that
//! is not used: Tequila decides by the map's role, as it does for a PNG
//! (`docs/02-MATERIAL-SYSTEM.md` §5.2).
//!
//! Two channels, red and green with no blue, is how BC5 (and R8G8) store a
//! normal map: the third component is left out because it follows from the
//! other two. It is put back here, so that the map reads as a normal map
//! does everywhere else rather than as one pointing into the surface.

use crate::image::{Decoded, Pixels};

/// Decode a DDS to RGBA: unassociated, from the top row down.
pub(crate) fn decode(bytes: &[u8]) -> Result<Decoded, String> {
    if bytes.len() < 128 || &bytes[..4] != b"DDS " {
        return Err("not a DDS".into());
    }
    let u32_at = |at: usize| -> Result<u32, String> {
        Ok(u32::from_le_bytes(bytes.get(at..at + 4).ok_or("the DDS is cut short")?.try_into().unwrap()))
    };
    let (height, width) = (u32_at(12)? as usize, u32_at(16)? as usize);
    if width == 0 || height == 0 {
        return Err("the DDS has no pixels".into());
    }
    let flags = u32_at(80)?;
    let fourcc = &bytes[84..88];
    let caps2 = u32_at(112)?;
    if caps2 & 0x200 != 0 {
        return Err("this DDS is a cube map; a texture here is one flat picture".into());
    }
    if caps2 & 0x20_0000 != 0 {
        return Err("this DDS is a volume texture; a texture here is one flat picture".into());
    }

    let mut start = 128;
    let layout = if flags & FOURCC != 0 {
        match fourcc {
            b"DX10" => {
                start = 148;
                if u32_at(132)? == 4 {
                    return Err("this DDS is a volume texture; a texture here is one flat picture".into());
                }
                if u32_at(136)? & 0x4 != 0 {
                    return Err("this DDS is a cube map; a texture here is one flat picture".into());
                }
                dxgi(u32_at(128)?)?
            }
            b"DXT1" => Layout::Block(Bc::One),
            b"DXT2" => Layout::Premultiplied(Bc::Two),
            b"DXT3" => Layout::Block(Bc::Two),
            b"DXT4" => Layout::Premultiplied(Bc::Three),
            b"DXT5" => Layout::Block(Bc::Three),
            b"ATI1" | b"BC4U" => Layout::Block(Bc::Four { signed: false }),
            b"BC4S" => Layout::Block(Bc::Four { signed: true }),
            b"ATI2" | b"BC5U" => Layout::Block(Bc::Five { signed: false }),
            b"BC5S" => Layout::Block(Bc::Five { signed: true }),
            // Direct3D 9 format codes stored where the FourCC goes.
            code => match u32::from_le_bytes(code.try_into().unwrap()) {
                36 => Layout::Wide { channels: 4, float: false, bits: 16 },
                111 => Layout::Wide { channels: 1, float: true, bits: 16 },
                112 => Layout::Wide { channels: 2, float: true, bits: 16 },
                113 => Layout::Wide { channels: 4, float: true, bits: 16 },
                114 => Layout::Wide { channels: 1, float: true, bits: 32 },
                115 => Layout::Wide { channels: 2, float: true, bits: 32 },
                116 => Layout::Wide { channels: 4, float: true, bits: 32 },
                _ => {
                    return Err(format!(
                        "DDS format {:?} is not one this reader knows",
                        String::from_utf8_lossy(code)
                    ))
                }
            },
        }
    } else if flags & (RGB | LUMINANCE | ALPHA) != 0 {
        let bits = u32_at(88)? as usize;
        if !matches!(bits, 8 | 16 | 24 | 32) {
            return Err(format!("a {bits}-bit uncompressed DDS is not one this reader knows"));
        }
        let mut masks = [u32_at(92)?, u32_at(96)?, u32_at(100)?, u32_at(104)?];
        if flags & (ALPHA_PIXELS | ALPHA) == 0 {
            masks[3] = 0;
        }
        if flags & (RGB | LUMINANCE) == 0 {
            masks[..3].fill(0);
        }
        Layout::Masked { bytes: bits / 8, masks, grey: flags & LUMINANCE != 0 }
    } else {
        return Err("this DDS's pixel format is not one this reader knows (YUV or bump data)".into());
    };

    let data = &bytes[start.min(bytes.len())..];
    let count = width * height;
    let short = || format!("the DDS is cut short: {width}x{height} needs more pixels than it holds");
    let pixels = match layout {
        Layout::Block(bc) | Layout::Premultiplied(bc) => {
            let (across, down) = (width.div_ceil(4), height.div_ceil(4));
            let size = bc.block_bytes();
            if data.len() < across * down * size {
                return Err(short());
            }
            let mut rgba = vec![0f32; count * 4];
            for by in 0..down {
                for bx in 0..across {
                    let block = &data[(by * across + bx) * size..][..size];
                    let texels = bc.decode(block);
                    for (i, texel) in texels.iter().enumerate() {
                        let (x, y) = (bx * 4 + i % 4, by * 4 + i / 4);
                        if x < width && y < height {
                            rgba[(y * width + x) * 4..][..4].copy_from_slice(texel);
                        }
                    }
                }
            }
            if matches!(layout, Layout::Premultiplied(_)) {
                for p in rgba.chunks_exact_mut(4) {
                    if p[3] > 0.0 {
                        for c in 0..3 {
                            p[c] = (p[c] / p[3]).min(1.0);
                        }
                    }
                }
            }
            if matches!(bc, Bc::Five { .. }) {
                rgba.chunks_exact_mut(4).for_each(rebuild_z);
            }
            if matches!(bc, Bc::Six | Bc::SixSigned) {
                Pixels::Linear(rgba)
            } else {
                Pixels::Display(rgba)
            }
        }
        Layout::Masked { bytes: size, masks, grey } => {
            if data.len() < count * size {
                return Err(short());
            }
            let channels = masks.map(Channel::new);
            let two = masks[1] != 0 && masks[2] == 0 && !grey;
            let mut rgba = Vec::with_capacity(count * 4);
            for p in data[..count * size].chunks_exact(size) {
                let mut word = [0u8; 4];
                word[..size].copy_from_slice(p);
                let v = u32::from_le_bytes(word);
                let r = channels[0].read(v, 0.0);
                let (g, b) = if grey { (r, r) } else { (channels[1].read(v, 0.0), channels[2].read(v, 0.0)) };
                rgba.extend_from_slice(&[r, g, b, channels[3].read(v, 1.0)]);
            }
            if two {
                rgba.chunks_exact_mut(4).for_each(rebuild_z);
            }
            Pixels::Display(rgba)
        }
        Layout::Wide { channels, float, bits } => {
            let step = channels * bits / 8;
            if data.len() < count * step {
                return Err(short());
            }
            let mut rgba = Vec::with_capacity(count * 4);
            for p in data[..count * step].chunks_exact(step) {
                let value = |c: usize| -> f32 {
                    match (bits, float) {
                        (16, false) => u16::from_le_bytes([p[2 * c], p[2 * c + 1]]) as f32 / 65535.0,
                        (16, true) => half(u16::from_le_bytes([p[2 * c], p[2 * c + 1]])),
                        _ => f32::from_le_bytes(p[4 * c..4 * c + 4].try_into().unwrap()),
                    }
                };
                rgba.extend_from_slice(&match channels {
                    1 => [value(0), value(0), value(0), 1.0],
                    2 => [value(0), value(1), 0.0, 1.0],
                    3 => [value(0), value(1), value(2), 1.0],
                    _ => [value(0), value(1), value(2), value(3)],
                });
            }
            if float {
                Pixels::Linear(rgba)
            } else {
                Pixels::Display(rgba)
            }
        }
    };
    Ok(Decoded { width, height, pixels })
}

const ALPHA_PIXELS: u32 = 0x1;
const ALPHA: u32 = 0x2;
const FOURCC: u32 = 0x4;
const RGB: u32 = 0x40;
const LUMINANCE: u32 = 0x2_0000;

#[derive(Clone, Copy, Debug)]
enum Layout {
    Block(Bc),
    /// DXT2 and DXT4: BC2 and BC3 with the colour multiplied by alpha.
    Premultiplied(Bc),
    /// One to four bytes a pixel, its channels found by mask.
    Masked { bytes: usize, masks: [u32; 4], grey: bool },
    /// A whole 16- or 32-bit number a channel: unsigned and normalized, or
    /// a float. One channel is a grey.
    Wide { channels: usize, float: bool, bits: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Bc {
    One,
    Two,
    Three,
    Four { signed: bool },
    Five { signed: bool },
    Six,
    SixSigned,
    Seven,
}

impl Bc {
    fn block_bytes(self) -> usize {
        match self {
            Bc::One | Bc::Four { .. } => 8,
            _ => 16,
        }
    }

    /// A block's sixteen texels as RGBA, row by row. Signed BC4 and BC5
    /// run from -1 to 1 and are brought to 0 to 1, how an unsigned map
    /// holds the same numbers.
    fn decode(self, block: &[u8]) -> [[f32; 4]; 16] {
        let mut out = [[0.0, 0.0, 0.0, 1.0]; 16];
        let unsign = |v: f32, signed: bool| if signed { v * 0.5 + 0.5 } else { v };
        match self {
            Bc::One | Bc::Two | Bc::Three | Bc::Seven => {
                let mut texels = [0u8; 64];
                match self {
                    Bc::One => bcdec_rs::bc1(block, &mut texels, 16),
                    Bc::Two => bcdec_rs::bc2(block, &mut texels, 16),
                    Bc::Three => bcdec_rs::bc3(block, &mut texels, 16),
                    _ => bcdec_rs::bc7(block, &mut texels, 16),
                }
                for (texel, p) in out.iter_mut().zip(texels.chunks_exact(4)) {
                    *texel = [p[0], p[1], p[2], p[3]].map(|v| v as f32 / 255.0);
                }
            }
            Bc::Four { signed } => {
                let mut texels = [0f32; 16];
                bcdec_rs::bc4_float(block, &mut texels, 4, signed);
                for (texel, v) in out.iter_mut().zip(texels) {
                    let v = unsign(v, signed);
                    *texel = [v, v, v, 1.0];
                }
            }
            Bc::Five { signed } => {
                let mut texels = [0f32; 32];
                bcdec_rs::bc5_float(block, &mut texels, 8, signed);
                for (texel, p) in out.iter_mut().zip(texels.chunks_exact(2)) {
                    *texel = [unsign(p[0], signed), unsign(p[1], signed), 0.0, 1.0];
                }
            }
            Bc::Six | Bc::SixSigned => {
                let mut texels = [0f32; 48];
                bcdec_rs::bc6h_float(block, &mut texels, 12, self == Bc::SixSigned);
                for (texel, p) in out.iter_mut().zip(texels.chunks_exact(3)) {
                    *texel = [p[0], p[1], p[2], 1.0];
                }
            }
        }
        out
    }
}

/// The DX10 header's format, as a layout.
fn dxgi(format: u32) -> Result<Layout, String> {
    use Layout::{Block, Masked, Wide};
    let masked = |bytes: usize, masks: [u32; 4]| Masked { bytes, masks, grey: false };
    Ok(match format {
        2 => Wide { channels: 4, float: true, bits: 32 },
        6 => Wide { channels: 3, float: true, bits: 32 },
        10 => Wide { channels: 4, float: true, bits: 16 },
        11 => Wide { channels: 4, float: false, bits: 16 },
        16 => Wide { channels: 2, float: true, bits: 32 },
        23 | 24 => masked(4, [0x3FF, 0xF_FC00, 0x3FF0_0000, 0xC000_0000]),
        27..=29 => masked(4, [0xFF, 0xFF00, 0xFF_0000, 0xFF00_0000]),
        34 => Wide { channels: 2, float: true, bits: 16 },
        35 => masked(4, [0xFFFF, 0xFFFF_0000, 0, 0]),
        41 => Wide { channels: 1, float: true, bits: 32 },
        49 => masked(2, [0xFF, 0xFF00, 0, 0]),
        54 => Wide { channels: 1, float: true, bits: 16 },
        56 => Masked { bytes: 2, masks: [0xFFFF, 0, 0, 0], grey: true },
        61 => Masked { bytes: 1, masks: [0xFF, 0, 0, 0], grey: true },
        65 => masked(1, [0, 0, 0, 0xFF]),
        70..=72 => Block(Bc::One),
        73..=75 => Block(Bc::Two),
        76..=78 => Block(Bc::Three),
        79 | 80 => Block(Bc::Four { signed: false }),
        81 => Block(Bc::Four { signed: true }),
        82 | 83 => Block(Bc::Five { signed: false }),
        84 => Block(Bc::Five { signed: true }),
        85 => masked(2, [0xF800, 0x07E0, 0x001F, 0]),
        86 => masked(2, [0x7C00, 0x03E0, 0x001F, 0x8000]),
        87 | 90 | 91 => masked(4, [0xFF_0000, 0xFF00, 0xFF, 0xFF00_0000]),
        88 | 92 | 93 => masked(4, [0xFF_0000, 0xFF00, 0xFF, 0]),
        94 | 95 => Block(Bc::Six),
        96 => Block(Bc::SixSigned),
        97..=99 => Block(Bc::Seven),
        115 => masked(2, [0x0F00, 0x00F0, 0x000F, 0xF000]),
        other => return Err(format!("DXGI format {other} is not one this reader knows")),
    })
}

/// A two-channel normal's third component, from the other two.
fn rebuild_z(p: &mut [f32]) {
    let (x, y) = (p[0] * 2.0 - 1.0, p[1] * 2.0 - 1.0);
    p[2] = (1.0 - x * x - y * y).max(0.0).sqrt() * 0.5 + 0.5;
}

/// IEEE half to single.
fn half(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = ((h >> 10) & 0x1F) as i32;
    let mantissa = (h & 0x3FF) as f32;
    sign * match exponent {
        0 => mantissa * 2f32.powi(-24),
        31 if mantissa == 0.0 => f32::INFINITY,
        31 => f32::NAN,
        e => (1.0 + mantissa / 1024.0) * 2f32.powi(e - 15),
    }
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

    fn read(self, pixel: u32, absent: f32) -> f32 {
        if self.max == 0 {
            return absent;
        }
        ((pixel & self.mask) >> self.shift) as f32 / self.max as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A DDS header for `width` by `height`, with the pixel format's flags,
    /// FourCC, bit count and masks as given.
    fn header(width: u32, height: u32, flags: u32, fourcc: &[u8; 4], bits: u32, masks: [u32; 4]) -> Vec<u8> {
        let mut out = b"DDS ".to_vec();
        let mut words = [0u32; 31];
        words[0] = 124;
        words[1] = 0x1 | 0x2 | 0x4 | 0x1000;
        words[2] = height;
        words[3] = width;
        words[18] = 32;
        words[19] = flags;
        words[21] = bits;
        words[22..26].copy_from_slice(&masks);
        words[26] = 0x1000;
        for w in words {
            out.extend_from_slice(&w.to_le_bytes());
        }
        out[84..88].copy_from_slice(fourcc);
        out
    }

    fn dx10(width: u32, height: u32, format: u32) -> Vec<u8> {
        let mut out = header(width, height, FOURCC, b"DX10", 0, [0; 4]);
        for w in [format, 3, 0, 1, 0] {
            out.extend_from_slice(&w.to_le_bytes());
        }
        out
    }

    fn rgba(decoded: &Decoded, x: usize, y: usize) -> [f32; 4] {
        let data = match &decoded.pixels {
            Pixels::Display(d) | Pixels::Linear(d) => d,
            Pixels::Eight(_) => unreachable!(),
        };
        data[(y * decoded.width + x) * 4..][..4].try_into().unwrap()
    }

    fn close(a: [f32; 4], b: [f32; 4]) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1.5 / 255.0)
    }

    /// A BC1 block of two colours: red and blue as 5:6:5 endpoints, each
    /// row of indices picking the first, second or a blend.
    fn bc1_block() -> [u8; 8] {
        let red = 0xF800u16.to_le_bytes();
        let blue = 0x001Fu16.to_le_bytes();
        // Rows: all red (0), all blue (1), all 2/3 red (2), all 1/3 red (3).
        [red[0], red[1], blue[0], blue[1], 0x00, 0x55, 0xAA, 0xFF]
    }

    #[test]
    fn bc1_decodes_its_endpoints_and_blends() {
        let mut file = header(4, 4, FOURCC, b"DXT1", 0, [0; 4]);
        file.extend_from_slice(&bc1_block());
        let image = decode(&file).unwrap();
        assert_eq!((image.width, image.height), (4, 4));
        assert!(close(rgba(&image, 0, 0), [1.0, 0.0, 0.0, 1.0]), "{:?}", rgba(&image, 0, 0));
        assert!(close(rgba(&image, 3, 1), [0.0, 0.0, 1.0, 1.0]), "{:?}", rgba(&image, 3, 1));
        let blend = rgba(&image, 2, 2);
        assert!((blend[0] - 2.0 / 3.0).abs() < 0.01 && (blend[2] - 1.0 / 3.0).abs() < 0.01, "{blend:?}");
        assert!(matches!(image.pixels, Pixels::Display(_)));
    }

    /// An image that is not a whole number of blocks is cropped from the
    /// blocks that cover it, and the blocks run across, then down.
    #[test]
    fn blocks_cover_an_image_in_rows_and_are_cropped_at_its_edge() {
        let mut file = header(5, 2, FOURCC, b"DXT1", 0, [0; 4]);
        let white = [0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0];
        file.extend_from_slice(&bc1_block());
        file.extend_from_slice(&white);
        let image = decode(&file).unwrap();
        assert_eq!((image.width, image.height), (5, 2));
        assert!(close(rgba(&image, 3, 0), [1.0, 0.0, 0.0, 1.0]));
        assert!(close(rgba(&image, 4, 0), [1.0, 1.0, 1.0, 1.0]), "the second block is to the right");
        assert!(close(rgba(&image, 4, 1), [1.0, 1.0, 1.0, 1.0]));
    }

    /// BC3: a BC1 colour block after an interpolated alpha block, here
    /// alpha 255 and 0 as endpoints with the first row at each.
    #[test]
    fn bc3_carries_interpolated_alpha() {
        let mut file = header(4, 4, FOURCC, b"DXT5", 0, [0; 4]);
        // Endpoints 255 and 0; indices: texels 0-1 at 0 (255), 2-3 at 1 (0),
        // and the rest at 0, three bits each.
        let indices: u64 = (1 << 6) | (1 << 9);
        let mut alpha = vec![255u8, 0];
        alpha.extend_from_slice(&indices.to_le_bytes()[..6]);
        file.extend_from_slice(&alpha);
        file.extend_from_slice(&bc1_block());
        let image = decode(&file).unwrap();
        assert_eq!(rgba(&image, 0, 0)[3], 1.0);
        assert_eq!(rgba(&image, 2, 0)[3], 0.0);
        assert_eq!(rgba(&image, 3, 0)[3], 0.0);
        assert_eq!(rgba(&image, 0, 1)[3], 1.0);
    }

    /// BC5 is a normal map's X and Y; the Z comes back so a flat normal is
    /// flat, rather than pointing into the surface.
    #[test]
    fn bc5_rebuilds_the_normal_it_leaves_out() {
        let mut file = header(4, 4, FOURCC, b"ATI2", 0, [0; 4]);
        // Both channels at 128 everywhere: endpoints equal, indices zero.
        for _ in 0..2 {
            file.extend_from_slice(&[128, 128, 0, 0, 0, 0, 0, 0]);
        }
        let image = decode(&file).unwrap();
        let p = rgba(&image, 1, 1);
        assert!((p[0] - 0.502).abs() < 0.01 && (p[1] - 0.502).abs() < 0.01, "{p:?}");
        assert!(p[2] > 0.99, "the normal's Z should be rebuilt as about 1: {p:?}");
    }

    /// BC7 mode 6, built by hand: both endpoints the same colour, so every
    /// texel is that colour whatever its index.
    #[test]
    fn bc7_through_the_dx10_header() {
        let mut bits: u128 = 0;
        let mut at = 0;
        let mut put = |value: u128, count: u32| {
            bits |= value << at;
            at += count;
        };
        // Mode 6 is the seventh bit; then each channel's two endpoints at
        // seven bits, two p-bits (the endpoints' shared low bit, here 1),
        // and the indices, left at zero.
        put(1 << 6, 7);
        for v in [100u128, 100, 50, 50, 77, 77, 127, 127] {
            put(v, 7);
        }
        put(0b11, 2);
        let mut file = dx10(4, 4, 98);
        file.extend_from_slice(&bits.to_le_bytes());
        let image = decode(&file).unwrap();
        let p = rgba(&image, 2, 3);
        assert_eq!(p, rgba(&image, 0, 0), "a block of one colour should be one colour");
        assert!(close(p, [201.0 / 255.0, 101.0 / 255.0, 155.0 / 255.0, 1.0]), "{p:?}");
    }

    /// Uncompressed by mask: legacy A8R8G8B8, and luminance.
    #[test]
    fn masked_layouts_put_each_channel_where_its_mask_says() {
        let mut file = header(2, 1, RGB | ALPHA_PIXELS, &[0; 4], 32, [0xFF_0000, 0xFF00, 0xFF, 0xFF00_0000]);
        file.extend_from_slice(&[30, 20, 10, 255, 0, 0, 255, 51]);
        let image = decode(&file).unwrap();
        assert!(close(rgba(&image, 0, 0), [10.0 / 255.0, 20.0 / 255.0, 30.0 / 255.0, 1.0]));
        assert!(close(rgba(&image, 1, 0), [1.0, 0.0, 0.0, 0.2]));

        // The same masks without the alpha flag: the alpha mask is ignored.
        let mut file = header(1, 1, RGB, &[0; 4], 32, [0xFF_0000, 0xFF00, 0xFF, 0xFF00_0000]);
        file.extend_from_slice(&[0, 0, 255, 0]);
        assert_eq!(rgba(&decode(&file).unwrap(), 0, 0)[3], 1.0);

        let mut file = header(2, 1, LUMINANCE, &[0; 4], 16, [0xFFFF, 0, 0, 0]);
        file.extend_from_slice(&[0, 0, 0xFF, 0x7F]);
        let p = rgba(&decode(&file).unwrap(), 1, 0);
        assert!((p[0] - 32767.0 / 65535.0).abs() < 1e-5 && p[0] == p[1] && p[1] == p[2], "sixteen-bit grey: {p:?}");
    }

    /// Half floats through the legacy code, linear and past one; and a
    /// 32-bit float grey through DX10.
    #[test]
    fn floats_are_linear_and_keep_their_range() {
        let mut file = header(1, 1, FOURCC, &113u32.to_le_bytes(), 0, [0; 4]);
        // 4.0, 0.5, 0.0, 1.0 as halves.
        for h in [0x4400u16, 0x3800, 0x0000, 0x3C00] {
            file.extend_from_slice(&h.to_le_bytes());
        }
        let image = decode(&file).unwrap();
        assert!(matches!(image.pixels, Pixels::Linear(_)));
        assert_eq!(rgba(&image, 0, 0), [4.0, 0.5, 0.0, 1.0]);

        let mut file = dx10(1, 1, 41);
        file.extend_from_slice(&2.5f32.to_le_bytes());
        assert_eq!(rgba(&decode(&file).unwrap(), 0, 0), [2.5, 2.5, 2.5, 1.0]);
    }

    #[test]
    fn cube_maps_and_short_files_are_refused() {
        let mut file = header(4, 4, FOURCC, b"DXT1", 0, [0; 4]);
        file[112..116].copy_from_slice(&0xFE00u32.to_le_bytes());
        file.extend_from_slice(&[0; 48]);
        assert!(decode(&file).unwrap_err().contains("cube map"));

        let mut file = header(8, 8, FOURCC, b"DXT1", 0, [0; 4]);
        file.extend_from_slice(&[0; 8]);
        assert!(decode(&file).unwrap_err().contains("cut short"));
    }

    #[test]
    fn halves_convert() {
        assert_eq!(half(0x3C00), 1.0);
        assert_eq!(half(0xC000), -2.0);
        assert_eq!(half(0x0001), 2f32.powi(-24));
        assert!(half(0x7C00).is_infinite());
    }
}
