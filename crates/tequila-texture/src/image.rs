//! Floating-point image buffers and readers.
//!
//! `docs/02-MATERIAL-SYSTEM.md` §5 specifies OpenImageIO for the production
//! texture pipeline — its tiled MIP cache is what makes scenes with dozens of
//! 4K textures workable, and that arrives with the texture system at M2.
//!
//! M0 needs one thing only: read an HDR environment map. That is served here
//! by pure-Rust decoders, which keeps the M0 build free of a native
//! dependency. The [`Image`] type is deliberately the shape OIIO will hand
//! back, so swapping the reader later does not ripple outwards.

use tequila_core::math::{vec3, Vec3};
use tequila_core::spectrum::{srgb_eotf, Spectrum};
use std::path::Path;

/// How a file's stored values map to scene-linear radiance.
///
/// `docs/01-RENDERER-CORE.md` §7.2: mis-tagging a data map as sRGB is "a
/// common and destructive error", so the importer defaults data maps to raw
/// and requires an explicit override rather than the reverse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorSpace {
    /// Already linear, in Rec.709 primaries. HDR and EXR files.
    LinearSrgb,
    /// sRGB transfer function applied; 8-bit colour textures.
    Srgb,
    /// No transform at all. Roughness, metalness, masks, bump, displacement.
    Raw,
}

/// A 32-bit float image with 3 or 4 channels, stored row-major from the top.
#[derive(Clone, Debug)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub channels: usize,
    /// `width * height * channels` values.
    pub data: Vec<f32>,
    pub color_space: ColorSpace,
}

#[derive(Debug)]
pub enum ImageError {
    Io(std::io::Error),
    UnsupportedFormat(String),
    Decode(String),
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageError::Io(e) => write!(f, "{e}"),
            ImageError::UnsupportedFormat(s) => write!(f, "unsupported image format: {s}"),
            ImageError::Decode(s) => write!(f, "could not decode image: {s}"),
        }
    }
}

impl std::error::Error for ImageError {}

impl From<std::io::Error> for ImageError {
    fn from(e: std::io::Error) -> Self {
        ImageError::Io(e)
    }
}

impl Image {
    /// A solid-colour image, useful as a fallback and in tests.
    pub fn constant(width: usize, height: usize, color: Vec3) -> Self {
        let mut data = Vec::with_capacity(width * height * 3);
        for _ in 0..width * height {
            data.extend_from_slice(&[color.x, color.y, color.z]);
        }
        Image {
            width,
            height,
            channels: 3,
            data,
            color_space: ColorSpace::LinearSrgb,
        }
    }

    /// Texel at `(x, y)`, clamped to the image bounds, as linear RGB in the
    /// file's own primaries (the colour-space transform is applied on load,
    /// so this is always linear).
    #[inline]
    pub fn texel(&self, x: usize, y: usize) -> Vec3 {
        let x = x.min(self.width.saturating_sub(1));
        let y = y.min(self.height.saturating_sub(1));
        let i = (y * self.width + x) * self.channels;
        vec3(self.data[i], self.data[i + 1], self.data[i + 2])
    }

    /// Bilinear sample at normalized coordinates, wrapping in `u` and clamping
    /// in `v` — the correct addressing for an equirectangular environment,
    /// where `u` runs continuously around the horizon but `v` stops at the
    /// poles.
    pub fn sample_bilinear_wrap_u(&self, u: f32, v: f32) -> Vec3 {
        if self.width == 0 || self.height == 0 {
            return Vec3::ZERO;
        }
        let fx = u.rem_euclid(1.0) * self.width as f32 - 0.5;
        let fy = v.clamp(0.0, 1.0) * self.height as f32 - 0.5;
        let x0 = fx.floor();
        let y0 = fy.floor();
        let tx = fx - x0;
        let ty = fy - y0;

        let wrap_x = |x: i64| x.rem_euclid(self.width as i64) as usize;
        let clamp_y = |y: i64| y.clamp(0, self.height as i64 - 1) as usize;

        let (x0i, x1i) = (wrap_x(x0 as i64), wrap_x(x0 as i64 + 1));
        let (y0i, y1i) = (clamp_y(y0 as i64), clamp_y(y0 as i64 + 1));

        let c00 = self.texel(x0i, y0i);
        let c10 = self.texel(x1i, y0i);
        let c01 = self.texel(x0i, y1i);
        let c11 = self.texel(x1i, y1i);
        c00.lerp(c10, tx).lerp(c01.lerp(c11, tx), ty)
    }

    /// Texel as a [`Spectrum`] in the rendering space.
    #[inline]
    pub fn texel_spectrum(&self, x: usize, y: usize) -> Spectrum {
        Spectrum::from_linear_srgb(self.texel(x, y))
    }

    /// The extensions [`Image::load`] can decode.
    ///
    /// Exposed so that a caller holding a path can tell the difference between
    /// a map that will appear and one that will not, *before* the render is
    /// already missing it. The importer uses it: it used to check only that a
    /// texture file existed, so a model whose maps were JPEGs imported with no
    /// complaint and then rendered untextured, with the fallback colour
    /// standing in and nothing anywhere saying why.
    ///
    /// Six of the eight `docs/02-MATERIAL-SYSTEM.md` §5.1 asks for (PSD by
    /// its flattened image, see [`crate::psd`]). TIFF, TGA and WebP are not
    /// implemented; they now produce that warning instead of a flat render,
    /// which is the difference between a feature that is missing and one
    /// that appears to be broken.
    pub const DECODABLE: [&str; 7] = ["png", "jpg", "jpeg", "hdr", "exr", "psd", "psb"];

    /// Whether [`Image::load`] can decode a path, by its extension.
    pub fn is_decodable(path: impl AsRef<Path>) -> bool {
        path.as_ref()
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .is_some_and(|e| Image::DECODABLE.contains(&e.as_str()))
    }

    /// Read an image, dispatching on extension.
    ///
    /// `.hdr` (Radiance RGBE) and `.exr` (OpenEXR) are the environment
    /// formats `TEQUILA_SPEC.md` §1.5 requires. Both store linear data, so no
    /// transfer function is removed.
    pub fn load(path: impl AsRef<Path>) -> Result<Image, ImageError> {
        let path = path.as_ref();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "png" => Self::load_png(path),
            "jpg" | "jpeg" => Self::load_jpeg(path),
            "hdr" => Self::load_radiance(path),
            "exr" => Self::load_exr(path),
            "psd" | "psb" => Self::load_psd(path, false),
            other => Err(ImageError::UnsupportedFormat(other.to_string())),
        }
    }

    /// Read an image whose 8-bit values are numbers rather than colours —
    /// a normal map, a roughness or metalness map, a height map — keeping
    /// them as they are stored.
    ///
    /// [`Image::load`] removes the sRGB curve from PNG and JPEG, which is
    /// right for artwork and wrong for data: a flat normal's 128 comes out as
    /// 0.22 rather than 0.5 and leans every normal the same way, and a
    /// roughness of 0.5 comes out as 0.22. Relabelling the result Raw
    /// afterwards does not put the numbers back. Float formats are linear
    /// either way and load the same as [`Image::load`].
    pub fn load_raw(path: impl AsRef<Path>) -> Result<Image, ImageError> {
        let path = path.as_ref();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "png" => {
                let (width, height, rgba) = Self::png_rgba8(path)?;
                Ok(Self::from_raw8(width, height, 4, &rgba))
            }
            "jpg" | "jpeg" => {
                let (width, height, rgba) = Self::jpeg_rgba8(path)?;
                Ok(Self::from_raw8(width, height, 4, &rgba))
            }
            "psd" | "psb" => Self::load_psd(path, true),
            _ => Self::load(path),
        }
    }

    /// A Photoshop document's flattened image as RGBA, unassociated: linear
    /// with the sRGB curve removed, or as stored when `raw`. A 32-bit
    /// document is linear already and loads the same either way.
    pub fn load_psd(path: impl AsRef<Path>, raw: bool) -> Result<Image, ImageError> {
        let bytes = std::fs::read(path.as_ref()).map_err(ImageError::Io)?;
        let composite = crate::psd::composite(&bytes).map_err(ImageError::Decode)?;
        let decode = !raw && !composite.linear;
        let data = composite
            .rgba
            .iter()
            .enumerate()
            .map(|(i, &v)| if decode && i % 4 != 3 { srgb_eotf(v) } else { v })
            .collect();
        Ok(Image {
            width: composite.width,
            height: composite.height,
            channels: 4,
            data,
            color_space: if raw && !composite.linear { ColorSpace::Raw } else { ColorSpace::LinearSrgb },
        })
    }

    /// Decode PNG artwork to linear RGBA, preserving unassociated alpha.
    pub fn load_png(path: impl AsRef<Path>) -> Result<Image, ImageError> {
        let (width, height, rgba) = Self::png_rgba8(path.as_ref())?;
        Ok(Self::from_srgb8(width, height, 4, &rgba))
    }

    /// A PNG's pixels as 8-bit RGBA, before any colour decoding.
    fn png_rgba8(path: &Path) -> Result<(usize, usize, Vec<u8>), ImageError> {
        let mut decoder = png::Decoder::new(std::fs::File::open(path)?);
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder
            .read_info()
            .map_err(|e| ImageError::Decode(e.to_string()))?;
        let mut buffer = vec![0; reader.output_buffer_size()];
        let info = reader
            .next_frame(&mut buffer)
            .map_err(|e| ImageError::Decode(e.to_string()))?;
        let mut rgba = Vec::with_capacity(info.width as usize * info.height as usize * 4);
        for pixel in buffer[..info.buffer_size()].chunks_exact(info.color_type.samples()) {
            match info.color_type {
                png::ColorType::Rgba => rgba.extend_from_slice(pixel),
                png::ColorType::Rgb => rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]),
                png::ColorType::Grayscale => {
                    rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], 255])
                }
                png::ColorType::GrayscaleAlpha => {
                    rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]])
                }
                _ => return Err(ImageError::Decode("Unsupported PNG color type".into())),
            }
        }
        Ok((info.width as usize, info.height as usize, rgba))
    }

    /// Decode a JPEG to linear RGBA.
    ///
    /// Same treatment as a PNG — sRGB removed on the way in, opaque alpha —
    /// because the two carry the same kind of content and differ only in how
    /// they are packed. A model downloaded from anywhere ships its colour map
    /// as one or the other, and which one is the exporter's choice rather than
    /// anything about the material.
    pub fn load_jpeg(path: impl AsRef<Path>) -> Result<Image, ImageError> {
        let (width, height, rgba) = Self::jpeg_rgba8(path.as_ref())?;
        Ok(Self::from_srgb8(width, height, 4, &rgba))
    }

    /// A JPEG's pixels as 8-bit RGBA, before any colour decoding.
    fn jpeg_rgba8(path: &Path) -> Result<(usize, usize, Vec<u8>), ImageError> {
        let file = std::io::BufReader::new(std::fs::File::open(path)?);
        let mut decoder = jpeg_decoder::Decoder::new(file);
        let pixels = decoder
            .decode()
            .map_err(|e| ImageError::Decode(e.to_string()))?;
        let info = decoder
            .info()
            .ok_or_else(|| ImageError::Decode("the JPEG has no frame header".into()))?;
        let (width, height) = (info.width as usize, info.height as usize);

        let mut rgba = Vec::with_capacity(width * height * 4);
        match info.pixel_format {
            jpeg_decoder::PixelFormat::RGB24 => {
                for p in pixels.chunks_exact(3) {
                    rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
                }
            }
            jpeg_decoder::PixelFormat::L8 => {
                for &v in &pixels {
                    rgba.extend_from_slice(&[v, v, v, 255]);
                }
            }
            // Sixteen-bit greyscale, little-endian pairs. The renderer works
            // in eight-bit sRGB here as it does for PNG, so the low byte goes.
            jpeg_decoder::PixelFormat::L16 => {
                for p in pixels.chunks_exact(2) {
                    let v = (u16::from_le_bytes([p[0], p[1]]) >> 8) as u8;
                    rgba.extend_from_slice(&[v, v, v, 255]);
                }
            }
            // CMYK, and in practice Adobe's inverted variant, which needs the
            // APP14 transform flag to tell apart from the plain one. Refusing
            // is better than guessing: a wrong guess gives a photographic
            // negative, which reads as a broken renderer rather than as an
            // unusual file.
            other => {
                return Err(ImageError::Decode(format!(
                    "this JPEG is {other:?} rather than RGB or greyscale; re-save it as RGB"
                )))
            }
        }
        Ok((width, height, rgba))
    }

    /// Write as Radiance RGBE (`.hdr`).
    pub fn save_radiance(&self, path: impl AsRef<Path>) -> Result<(), ImageError> {
        std::fs::write(path, crate::radiance::encode(self))?;
        Ok(())
    }

    /// Radiance RGBE (`.hdr`).
    pub fn load_radiance(path: impl AsRef<Path>) -> Result<Image, ImageError> {
        let bytes = std::fs::read(path)?;
        crate::radiance::decode(&bytes)
    }

    /// OpenEXR (`.exr`), reading the RGB channels of the first layer — or,
    /// where it has no colour, its one channel as a grey.
    pub fn load_exr(path: impl AsRef<Path>) -> Result<Image, ImageError> {
        Self::load_exr_rgb(path.as_ref()).or_else(|colour| Self::load_exr_grey(path.as_ref()).map_err(|_| colour))
    }

    /// A greyscale EXR: a roughness, height or mask map stored as the one
    /// channel it needs, usually named `Y`. Poly Haven ships its roughness
    /// maps this way, and a reader that asked only for red, green and blue
    /// refused them — so the material fell back to its default roughness,
    /// and a brick wall rendered with a plastic's gloss.
    fn load_exr_grey(path: &Path) -> Result<Image, ImageError> {
        let decoded = exr::image::read::read_first_flat_layer_from_file(path)
            .map_err(|e| ImageError::Decode(e.to_string()))?;
        let layer = decoded.layer_data;
        let (width, height) = (layer.size.0, layer.size.1);
        let channels = &layer.channel_data.list;
        let named = |wanted: &str| channels.iter().position(|c| c.name.to_string() == wanted);
        let pick = named("Y").or_else(|| named("R")).unwrap_or(0);
        let channel = channels
            .get(pick)
            .ok_or_else(|| ImageError::Decode("the EXR has no channels".into()))?;
        let mut data = Vec::with_capacity(width * height * 3);
        for value in channel.sample_data.values_as_f32() {
            data.extend_from_slice(&[value, value, value]);
        }
        if data.len() != width * height * 3 {
            return Err(ImageError::Decode("the EXR's channel does not fill its image".into()));
        }
        Ok(Image { width, height, channels: 3, data, color_space: ColorSpace::LinearSrgb })
    }

    fn load_exr_rgb(path: &Path) -> Result<Image, ImageError> {
        // Not `use exr::prelude::*`: that glob exports its own `Image`, which
        // would shadow ours inside this function and produce baffling errors.
        let decoded = exr::image::read::read_first_rgba_layer_from_file(
            path,
            |resolution: exr::math::Vec2<usize>, _| Image {
                width: resolution.0,
                height: resolution.1,
                channels: 3,
                data: vec![0.0; resolution.0 * resolution.1 * 3],
                color_space: ColorSpace::LinearSrgb,
            },
            |img: &mut Image,
             position: exr::math::Vec2<usize>,
             (r, g, b, _a): (f32, f32, f32, f32)| {
                let i = (position.1 * img.width + position.0) * 3;
                img.data[i] = r;
                img.data[i + 1] = g;
                img.data[i + 2] = b;
            },
        )
        .map_err(|e| ImageError::Decode(e.to_string()))?;
        Ok(decoded.layer_data.channel_data.pixels)
    }

    /// An 8-bit buffer of numbers, as floats from 0 to 1 with nothing
    /// decoded.
    pub fn from_raw8(width: usize, height: usize, channels: usize, bytes: &[u8]) -> Image {
        Image {
            width,
            height,
            channels,
            data: bytes.iter().map(|&b| b as f32 / 255.0).collect(),
            color_space: ColorSpace::Raw,
        }
    }

    /// Decode an 8-bit sRGB buffer into linear floats. Used for backplates and
    /// label textures; kept here so every path into the renderer removes the
    /// transfer function in exactly one place.
    pub fn from_srgb8(width: usize, height: usize, channels: usize, bytes: &[u8]) -> Image {
        let data = bytes
            .iter()
            .enumerate()
            .map(|(i, &b)| {
                let v = b as f32 / 255.0;
                // Alpha is never gamma-encoded.
                if channels == 4 && i % 4 == 3 {
                    v
                } else {
                    srgb_eotf(v)
                }
            })
            .collect();
        Image {
            width,
            height,
            channels,
            data,
            color_space: ColorSpace::LinearSrgb,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(width: usize, height: usize) -> Image {
        let mut data = Vec::new();
        for y in 0..height {
            for x in 0..width {
                data.extend_from_slice(&[x as f32, y as f32, 0.0]);
            }
        }
        Image {
            width,
            height,
            channels: 3,
            data,
            color_space: ColorSpace::LinearSrgb,
        }
    }

    /// A one-channel EXR — how Poly Haven stores roughness — reads as a
    /// grey, rather than failing and leaving the material at its default.
    #[test]
    fn a_greyscale_exr_reads_as_a_grey() {
        let path = std::env::temp_dir().join(format!("tequila-grey-{}.exr", std::process::id()));
        let values: Vec<f32> = (0..16).map(|i| i as f32 / 16.0).collect();
        let channel = exr::prelude::AnyChannel::new("Y", exr::prelude::FlatSamples::F32(values.clone()));
        let image = exr::prelude::Image::from_channels(
            (4, 4),
            exr::prelude::AnyChannels::sort(exr::prelude::SmallVec::from_vec(vec![channel])),
        );
        use exr::prelude::WritableImage;
        image.write().to_file(&path).expect("writes");
        let read = Image::load_raw(&path).expect("a greyscale EXR loads");
        assert_eq!((read.width, read.height), (4, 4));
        let texel = read.texel(3, 1);
        assert!((texel.x - values[7]).abs() < 1e-6 && texel.x == texel.y && texel.y == texel.z, "{texel:?}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn texel_access_clamps() {
        let img = gradient(4, 4);
        assert_eq!(img.texel(0, 0), vec3(0.0, 0.0, 0.0));
        assert_eq!(img.texel(3, 3), vec3(3.0, 3.0, 0.0));
        assert_eq!(img.texel(99, 99), vec3(3.0, 3.0, 0.0));
    }

    #[test]
    fn bilinear_hits_texel_centres_exactly() {
        let img = gradient(4, 4);
        for x in 0..4 {
            for y in 0..4 {
                let u = (x as f32 + 0.5) / 4.0;
                let v = (y as f32 + 0.5) / 4.0;
                let s = img.sample_bilinear_wrap_u(u, v);
                assert!(
                    (s - img.texel(x, y)).length() < 1e-4,
                    "({x},{y}) sampled {s:?}"
                );
            }
        }
    }

    /// `u` must wrap: an equirectangular environment is continuous across the
    /// seam, and clamping there leaves a visible vertical band in reflections.
    #[test]
    fn horizontal_addressing_wraps() {
        let img = gradient(4, 4);
        let just_before = img.sample_bilinear_wrap_u(0.999, 0.5);
        let just_after = img.sample_bilinear_wrap_u(0.001, 0.5);
        // Both straddle the seam, so both blend column 3 with column 0.
        assert!(just_before.x < 3.0 && just_before.x > 0.0);
        assert!(just_after.x < 3.0 && just_after.x > 0.0);
        // Not exact equality: `rem_euclid` on 1.3 and on 0.3 do not produce
        // bit-identical results, and a texture lookup has no business
        // depending on that.
        let a = img.sample_bilinear_wrap_u(0.3, 0.5);
        let b = img.sample_bilinear_wrap_u(1.3, 0.5);
        assert!((a - b).length() < 1e-5, "{a:?} vs {b:?}");
    }

    /// `v` must clamp, not wrap: blending the zenith into the nadir puts the
    /// sky on the floor.
    #[test]
    fn vertical_addressing_clamps() {
        let img = gradient(4, 4);
        let top = img.sample_bilinear_wrap_u(0.5, 0.0);
        let bottom = img.sample_bilinear_wrap_u(0.5, 1.0);
        assert!((top.y - 0.0).abs() < 1e-4, "{top:?}");
        assert!((bottom.y - 3.0).abs() < 1e-4, "{bottom:?}");
    }

    #[test]
    fn srgb8_decode_removes_the_transfer_function() {
        let img = Image::from_srgb8(1, 1, 3, &[255, 128, 0]);
        assert!((img.data[0] - 1.0).abs() < 1e-5);
        // Mid-grey in sRGB is about 0.216 linear, not 0.502.
        assert!((img.data[1] - 0.2158).abs() < 1e-3, "{}", img.data[1]);
        assert_eq!(img.data[2], 0.0);
    }

    #[test]
    fn srgb8_decode_leaves_alpha_linear() {
        let img = Image::from_srgb8(1, 1, 4, &[128, 128, 128, 128]);
        assert!((img.data[3] - 128.0 / 255.0).abs() < 1e-5);
    }

    /// A label saved as a PSD loads by its flattened image: artwork with the
    /// sRGB curve taken off, its transparency kept; numbers left as they are
    /// through `load_raw`.
    #[test]
    fn a_psd_label_loads_by_its_flattened_image() {
        let pixels = [[128, 255, 0, 255], [0, 64, 255, 0], [255, 255, 255, 255], [10, 20, 30, 255]];
        let path = std::env::temp_dir().join(format!("tequila-flattened-{}.PSD", std::process::id()));
        std::fs::write(&path, crate::psd::tests::write(2, 2, &pixels, true, None)).unwrap();
        let image = Image::load(&path);
        let raw = Image::load_raw(&path);
        let _ = std::fs::remove_file(&path);
        let image = image.expect("a PSD should load");
        assert_eq!((image.width, image.height, image.channels), (2, 2, 4));
        assert!((image.data[0] - srgb_eotf(128.0 / 255.0)).abs() < 1e-4, "{}", image.data[0]);
        assert!((image.data[1] - 1.0).abs() < 1e-4 && image.data[2].abs() < 1e-4);
        assert_eq!((image.data[3], image.data[7]), (1.0, 0.0), "the transparency is kept");
        let raw = raw.expect("a PSD should load raw");
        assert!((raw.data[0] - 128.0 / 255.0).abs() < 1e-4, "{}", raw.data[0]);
    }

    #[test]
    fn unknown_extensions_are_reported_not_guessed() {
        let e = Image::load("nope.tga").unwrap_err();
        assert!(matches!(e, ImageError::UnsupportedFormat(_)), "{e:?}");
    }

    /// Write a JPEG of known colours, read it back through `Image::load`.
    ///
    /// Encoded here rather than checked in as a blob, so the colours the test
    /// asserts on are the colours it wrote, visible in this file. The
    /// tolerance is generous because JPEG is lossy — the point is that red
    /// comes back red and in the right corner, not that it survives bit for
    /// bit.
    #[test]
    fn a_jpeg_decodes_to_the_colours_it_was_written_from() {
        use jpeg_encoder::{ColorType, Encoder};

        // Four quadrants of a 16x16: red, green, blue, white. Solid blocks,
        // and the samples are taken well inside them, so that chroma
        // subsampling at the edges cannot decide the answer.
        let (w, h) = (16usize, 16usize);
        let mut rgb = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                let c: [u8; 3] = match (x < w / 2, y < h / 2) {
                    (true, true) => [230, 20, 20],
                    (false, true) => [20, 200, 20],
                    (true, false) => [20, 20, 220],
                    (false, false) => [255, 255, 255],
                };
                rgb.extend_from_slice(&c);
            }
        }

        let path = std::env::temp_dir().join("tequila-quadrants.jpg");
        Encoder::new_file(&path, 95)
            .expect("an encoder")
            .encode(&rgb, w as u16, h as u16, ColorType::Rgb)
            .expect("the JPEG is written");

        let image = Image::load(&path).expect("a .jpg should load");
        assert_eq!((image.width, image.height), (w, h));
        assert_eq!(
            image.color_space,
            ColorSpace::LinearSrgb,
            "a JPEG carries a transfer curve and it should have been removed, as it is for a PNG"
        );

        // Compared in sRGB, where the written values live; `texel` is linear.
        let srgb_at = |x: usize, y: usize| {
            let v = image.texel(x, y);
            [v.x, v.y, v.z].map(|c| {
                let c = c.clamp(0.0, 1.0);
                255.0
                    * if c <= 0.0031308 {
                        12.92 * c
                    } else {
                        1.055 * c.powf(1.0 / 2.4) - 0.055
                    }
            })
        };

        for (x, y, expected, corner) in [
            (4, 4, [230.0, 20.0, 20.0], "top left, red"),
            (12, 4, [20.0, 200.0, 20.0], "top right, green"),
            (4, 12, [20.0, 20.0, 220.0], "bottom left, blue"),
            (12, 12, [255.0, 255.0, 255.0], "bottom right, white"),
        ] {
            let got = srgb_at(x, y);
            for channel in 0..3 {
                assert!(
                    (got[channel] - expected[channel]).abs() < 16.0,
                    "{corner}: read {got:?}, expected about {expected:?} —                      either the channels are out of order or the rows are"
                );
            }
        }
        let _ = std::fs::remove_file(&path);
    }

    /// The list a caller checks a path against has to agree with what `load`
    /// will actually do, or a warning is given for a format that works, or
    /// withheld for one that does not.
    #[test]
    fn the_decodable_list_agrees_with_the_dispatch() {
        for ext in Image::DECODABLE {
            let path = format!("nothing-here.{ext}");
            assert!(Image::is_decodable(&path), "`{ext}` is not recognised");
            // The file does not exist, so the error must be about that rather
            // than about the format.
            match Image::load(&path) {
                Err(ImageError::UnsupportedFormat(_)) => {
                    panic!("`{ext}` is listed as decodable but `load` refuses it")
                }
                _ => {}
            }
        }
        assert!(!Image::is_decodable("photo.tga"));
        assert!(!Image::is_decodable("no-extension"));
        // Extensions come off files a person named, so case is not a signal.
        assert!(Image::is_decodable("COLOUR.JPG"));
    }
}
