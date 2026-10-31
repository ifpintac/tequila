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
    /// All eight `docs/02-MATERIAL-SYSTEM.md` §5.1 asks for (PSD by its
    /// flattened image, see [`crate::psd`]), and BMP, GIF (its first frame),
    /// DDS, JPEG XL and SVG (rasterized, see [`crate::svg`]) besides. What
    /// is still refused — HEIC and AVIF, whose decoders are C libraries, and
    /// KTX2 among others — produces that warning instead of a flat render,
    /// which is the difference between a feature that is missing and one
    /// that appears to be broken.
    pub const DECODABLE: [&str; 17] = [
        "png", "jpg", "jpeg", "tif", "tiff", "tga", "bmp", "webp", "gif", "jxl", "dds", "svg", "svgz", "hdr", "exr",
        "psd", "psb",
    ];

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
    /// transfer function is removed. Nor is one from a floating-point TIFF,
    /// DDS or JPEG XL; every integer format has its sRGB curve removed, as a
    /// PNG does.
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
            other => Ok(Self::decode(path, other)?.into_image(false)),
        }
    }

    /// Read an image whose 8-bit values are numbers rather than colours —
    /// a normal map, a roughness or metalness map, a height map — keeping
    /// them as they are stored.
    ///
    /// [`Image::load`] removes the sRGB curve from PNG, JPEG and every other
    /// integer format, which is
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
            "hdr" | "exr" => Self::load(path),
            other => Ok(Self::decode(path, other)?.into_image(true)),
        }
    }

    /// The formats past the first few, by extension, each read to pixels
    /// that [`Decoded::into_image`] then decodes — or not, for a raw map —
    /// in one place for all of them.
    fn decode(path: &Path, ext: &str) -> Result<Decoded, ImageError> {
        let read = || std::fs::read(path).map_err(ImageError::Io);
        Ok(match ext {
            "tif" | "tiff" => Self::tiff_pixels(path)?,
            "webp" => Self::webp_pixels(path)?,
            "gif" => Self::gif_pixels(path)?,
            "jxl" => Self::jxl_pixels(path)?,
            "tga" => Decoded::eight(crate::tga::decode(&read()?).map_err(ImageError::Decode)?),
            "bmp" => Decoded::eight(crate::bmp::decode(&read()?).map_err(ImageError::Decode)?),
            "dds" => crate::dds::decode(&read()?).map_err(ImageError::Decode)?,
            "svg" | "svgz" => Decoded::eight(crate::svg::rasterize(path).map_err(ImageError::Decode)?),
            other => return Err(ImageError::UnsupportedFormat(other.to_string())),
        })
    }

    /// A Photoshop document's flattened image as RGBA, unassociated: linear
    /// with the sRGB curve removed, or as stored when `raw`. A 32-bit
    /// document is linear already and loads the same either way.
    pub fn load_psd(path: impl AsRef<Path>, raw: bool) -> Result<Image, ImageError> {
        let bytes = std::fs::read(path.as_ref()).map_err(ImageError::Io)?;
        let composite = crate::psd::composite(&bytes).map_err(ImageError::Decode)?;
        let pixels = if composite.linear { Pixels::Linear(composite.rgba) } else { Pixels::Display(composite.rgba) };
        Ok(Decoded { width: composite.width, height: composite.height, pixels }.into_image(raw))
    }

    /// A TIFF's first image, by the `tiff` crate: 8 or 16 bits a channel
    /// (display-encoded, so decoded like a PNG's) or 32- and 64-bit floats
    /// (linear, as a TIFF32 render is written); greyscale, RGB or a
    /// palette; with alpha; uncompressed, LZW, Deflate or PackBits, with or
    /// without a predictor.
    ///
    /// A channel past the colour ones is the alpha when `ExtraSamples`
    /// says so (premultiplied or not) or says nothing at all, which is how
    /// the `tiff` crate writes one, Tequila's own renders included. One
    /// marked "unspecified" is a saved selection or a spot channel, and
    /// taking it for alpha would cut holes in the picture. The orientation
    /// tag is honoured. CMYK, YCbCr, Lab and channels stored as separate
    /// planes are refused with a message saying so.
    fn tiff_pixels(path: &Path) -> Result<Decoded, ImageError> {
        use tiff::decoder::{Decoder, DecodingResult, Limits};
        use tiff::tags::Tag;
        let fail = |e: tiff::TiffError| ImageError::Decode(e.to_string());
        let refuse = |why: &str| ImageError::Decode(format!("this TIFF {why}"));
        let file = std::io::BufReader::new(std::fs::File::open(path)?);
        // The default limits stop at 256 MB, which an 8K texture at sixteen
        // bits passes; the file is one the user chose.
        let mut decoder = Decoder::new(file).map_err(fail)?.with_limits(Limits::unlimited());
        let (width, height) = decoder.dimensions().map_err(fail)?;
        let (width, height) = (width as usize, height as usize);
        let mut tag = |tag: Tag| decoder.find_tag_unsigned::<u32>(tag).map_err(fail);
        let samples = tag(Tag::SamplesPerPixel)?.unwrap_or(1) as usize;
        let photometric = tag(Tag::PhotometricInterpretation)?.unwrap_or(1);
        let planar = tag(Tag::PlanarConfiguration)?.unwrap_or(1);
        let orientation = tag(Tag::Orientation)?.unwrap_or(1);
        let extra = decoder.find_tag_unsigned_vec::<u32>(Tag::ExtraSamples).map_err(fail)?.unwrap_or_default();
        let colours = match photometric {
            0 | 1 | 3 => 1,
            2 => 3,
            5 => return Err(refuse("is CMYK; re-save it as RGB")),
            6 => return Err(refuse("is YCbCr; re-save it as RGB")),
            8..=10 => return Err(refuse("is Lab; re-save it as RGB")),
            other => return Err(refuse(&format!("has photometric interpretation {other}, which this reader does not know"))),
        };
        if planar == 2 && samples > 1 {
            return Err(refuse("stores each channel separately; re-save it with interleaved pixels"));
        }
        if samples < colours {
            return Err(refuse("has fewer channels than its colour needs"));
        }
        // Some(premultiplied) when the first extra channel is alpha.
        let alpha = (samples > colours)
            .then(|| match extra.first() {
                None | Some(2) => Some(false),
                Some(1) => Some(true),
                _ => None,
            })
            .flatten();
        let palette = if photometric == 3 {
            let map = decoder.find_tag_unsigned_vec::<u16>(Tag::ColorMap).map_err(fail)?;
            Some(map.ok_or_else(|| refuse("is a palette image with no palette"))?)
        } else {
            None
        };

        let (values, float): (Vec<f32>, bool) = match decoder.read_image().map_err(fail)? {
            // Indices stay indices; everything else comes to 0 to 1.
            DecodingResult::U8(v) if palette.is_some() => (v.into_iter().map(|v| v as f32).collect(), false),
            DecodingResult::U16(v) if palette.is_some() => (v.into_iter().map(|v| v as f32).collect(), false),
            DecodingResult::U8(v) => (v.into_iter().map(|v| v as f32 / 255.0).collect(), false),
            DecodingResult::U16(v) => (v.into_iter().map(|v| v as f32 / 65535.0).collect(), false),
            DecodingResult::U32(v) => (v.into_iter().map(|v| (v as f64 / u32::MAX as f64) as f32).collect(), false),
            DecodingResult::F32(v) => (v, true),
            DecodingResult::F64(v) => (v.into_iter().map(|v| v as f32).collect(), true),
            _ => return Err(refuse("holds signed integers, which this reader does not read")),
        };
        let count = width * height;
        if values.len() < count * samples {
            return Err(refuse("holds fewer pixels than its size"));
        }
        let mut rgba = Vec::with_capacity(count * 4);
        for p in values.chunks_exact(samples).take(count) {
            let (r, g, b) = match (photometric, &palette) {
                (3, Some(map)) => {
                    let n = map.len() / 3;
                    let i = p[0] as usize;
                    if i >= n {
                        return Err(refuse(&format!("uses colour {i}, which its palette does not have")));
                    }
                    let c = |k: usize| map[k * n + i] as f32 / 65535.0;
                    (c(0), c(1), c(2))
                }
                // White is zero: a scanner's or a fax's grey, inverted.
                (0, _) if !float => (1.0 - p[0], 1.0 - p[0], 1.0 - p[0]),
                (0 | 1, _) => (p[0], p[0], p[0]),
                _ => (p[0], p[1], p[2]),
            };
            let a = if alpha.is_some() { p[colours] } else { 1.0 };
            rgba.extend_from_slice(&[r, g, b, a]);
        }
        if alpha == Some(true) {
            unpremultiply(&mut rgba);
        }
        let (width, height, rgba) = upright(rgba, width, height, orientation);
        Ok(Decoded { width, height, pixels: if float { Pixels::Linear(rgba) } else { Pixels::Display(rgba) } })
    }

    /// A WebP, lossy or lossless, with its alpha; the first frame of an
    /// animated one.
    fn webp_pixels(path: &Path) -> Result<Decoded, ImageError> {
        let fail = |e: image_webp::DecodingError| ImageError::Decode(e.to_string());
        let file = std::io::BufReader::new(std::fs::File::open(path)?);
        let mut decoder = image_webp::WebPDecoder::new(file).map_err(fail)?;
        let (width, height) = decoder.dimensions();
        let size = decoder.output_buffer_size().ok_or_else(|| ImageError::Decode("the WebP is too large to hold".into()))?;
        let mut buffer = vec![0; size];
        decoder.read_image(&mut buffer).map_err(fail)?;
        let rgba = if decoder.has_alpha() {
            buffer
        } else {
            buffer.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect()
        };
        Ok(Decoded { width: width as usize, height: height as usize, pixels: Pixels::Eight(rgba) })
    }

    /// A GIF's first frame, placed where it sits on the picture's canvas
    /// and transparent around it, as a viewer shows it before it plays.
    fn gif_pixels(path: &Path) -> Result<Decoded, ImageError> {
        let fail = |e: gif::DecodingError| ImageError::Decode(e.to_string());
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::RGBA);
        let mut decoder = options.read_info(std::io::BufReader::new(std::fs::File::open(path)?)).map_err(fail)?;
        let (screen_width, screen_height) = (decoder.width() as usize, decoder.height() as usize);
        let frame = decoder.read_next_frame().map_err(fail)?.ok_or_else(|| ImageError::Decode("the GIF has no frames".into()))?;
        let (left, top) = (frame.left as usize, frame.top as usize);
        let (frame_width, frame_height) = (frame.width as usize, frame.height as usize);
        // A canvas smaller than its own frame is a broken header; the frame
        // is the better witness.
        let width = screen_width.max(left + frame_width);
        let height = screen_height.max(top + frame_height);
        let mut rgba = vec![0u8; width * height * 4];
        if frame_width > 0 {
            for (y, row) in frame.buffer.chunks_exact(frame_width * 4).take(frame_height).enumerate() {
                let at = ((top + y) * width + left) * 4;
                rgba[at..at + row.len()].copy_from_slice(row);
            }
        }
        Ok(Decoded { width, height, pixels: Pixels::Eight(rgba) })
    }

    /// A JPEG XL's first frame, by `jxl-oxide`: rendered to sRGB, or to
    /// linear sRGB when its samples are floats, so that it is decoded the
    /// way every other format of the same kind is. Its orientation is
    /// applied; CMYK is refused.
    fn jxl_pixels(path: &Path) -> Result<Decoded, ImageError> {
        use jxl_oxide::{EnumColourEncoding, ExtraChannelType, JxlImage, RenderingIntent};
        let fail = |e: Box<dyn std::error::Error + Send + Sync>| ImageError::Decode(e.to_string());
        let mut image = JxlImage::builder().open(path).map_err(fail)?;
        let metadata = &image.image_header().metadata;
        let float = matches!(metadata.bit_depth, jxl_oxide::image::BitDepth::FloatSample { .. });
        let premultiplied = metadata
            .ec_info
            .iter()
            .any(|ec| matches!(ec.ty, ExtraChannelType::Alpha { alpha_associated: true }));
        let intent = RenderingIntent::Relative;
        let mut encoding =
            if metadata.grayscale() { EnumColourEncoding::gray_srgb(intent) } else { EnumColourEncoding::srgb(intent) };
        if float {
            encoding.tf = jxl_oxide::color::TransferFunction::Linear;
        }
        image.request_color_encoding(encoding);
        if image.pixel_format().has_black() {
            return Err(ImageError::Decode("this JPEG XL is CMYK; re-save it as RGB".into()));
        }
        let render = image.render_frame(0).map_err(fail)?;
        let mut stream = render.stream();
        let (width, height) = (stream.width() as usize, stream.height() as usize);
        let channels = stream.channels() as usize;
        let mut samples = vec![0f32; width * height * channels];
        stream.write_to_buffer(&mut samples);
        let mut rgba = Vec::with_capacity(width * height * 4);
        for p in samples.chunks_exact(channels.max(1)) {
            rgba.extend_from_slice(&match channels {
                1 => [p[0], p[0], p[0], 1.0],
                2 => [p[0], p[0], p[0], p[1]],
                3 => [p[0], p[1], p[2], 1.0],
                _ => [p[0], p[1], p[2], p[3]],
            });
        }
        if premultiplied {
            unpremultiply(&mut rgba);
        }
        Ok(Decoded { width, height, pixels: if float { Pixels::Linear(rgba) } else { Pixels::Display(rgba) } })
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

/// Pixels as a reader hands them over, before any colour decoding: RGBA,
/// unassociated, from the top row down.
#[derive(Debug)]
pub(crate) struct Decoded {
    pub width: usize,
    pub height: usize,
    pub pixels: Pixels,
}

#[derive(Debug)]
pub(crate) enum Pixels {
    /// Eight bits a channel, display-encoded: what most files hold.
    Eight(Vec<u8>),
    /// 0 to 1 and display-encoded, from a format with more than eight bits.
    Display(Vec<f32>),
    /// Scene-linear floats, which may pass 1.
    Linear(Vec<f32>),
}

impl Decoded {
    pub(crate) fn eight((width, height, rgba): (usize, usize, Vec<u8>)) -> Decoded {
        Decoded { width, height, pixels: Pixels::Eight(rgba) }
    }

    /// As an [`Image`]: display-encoded values with the sRGB curve taken
    /// off, or kept as stored when `raw`; linear values are the same either
    /// way. The one place every format past PNG and JPEG is decoded, so
    /// that a map's role means the same thing whatever file it came in.
    pub(crate) fn into_image(self, raw: bool) -> Image {
        let Decoded { width, height, pixels } = self;
        let image = |data: Vec<f32>, color_space| Image { width, height, channels: 4, data, color_space };
        match pixels {
            Pixels::Eight(bytes) if raw => Image::from_raw8(width, height, 4, &bytes),
            Pixels::Eight(bytes) => Image::from_srgb8(width, height, 4, &bytes),
            Pixels::Display(data) if raw => image(data, ColorSpace::Raw),
            Pixels::Display(mut data) => {
                for (i, v) in data.iter_mut().enumerate() {
                    // Alpha is never gamma-encoded.
                    if i % 4 != 3 {
                        *v = srgb_eotf(*v);
                    }
                }
                image(data, ColorSpace::LinearSrgb)
            }
            Pixels::Linear(data) => image(data, ColorSpace::LinearSrgb),
        }
    }
}

/// Premultiplied RGBA to unassociated, in place.
fn unpremultiply(rgba: &mut [f32]) {
    for p in rgba.chunks_exact_mut(4) {
        if p[3] > 0.0 {
            for c in 0..3 {
                p[c] /= p[3];
            }
        }
    }
}

/// RGBA stored in TIFF (and EXIF) orientation `orientation`, turned
/// upright: 1 is as stored, 2 to 4 mirror and turn it half round, and 5 to
/// 8 lie on their side, so their width and height trade places.
fn upright(rgba: Vec<f32>, width: usize, height: usize, orientation: u32) -> (usize, usize, Vec<f32>) {
    if !(2..=8).contains(&orientation) {
        return (width, height, rgba);
    }
    let (w, h) = if orientation >= 5 { (height, width) } else { (width, height) };
    let mut out = vec![0.0; rgba.len()];
    for y in 0..h {
        for x in 0..w {
            let (sx, sy) = match orientation {
                2 => (width - 1 - x, y),
                3 => (width - 1 - x, height - 1 - y),
                4 => (x, height - 1 - y),
                5 => (y, x),
                6 => (y, height - 1 - x),
                7 => (width - 1 - y, height - 1 - x),
                _ => (width - 1 - y, x),
            };
            out[(y * w + x) * 4..][..4].copy_from_slice(&rgba[(sy * width + sx) * 4..][..4]);
        }
    }
    (w, h, out)
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
        let e = Image::load("nope.heic").unwrap_err();
        assert!(matches!(e, ImageError::UnsupportedFormat(_)), "{e:?}");
        let e = Image::load_raw("nope.heic").unwrap_err();
        assert!(matches!(e, ImageError::UnsupportedFormat(_)), "{e:?}");
    }

    /// A file of `bytes` under `name` in the temporary folder, removed
    /// when dropped.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(name: &str, bytes: &[u8]) -> Scratch {
            let path = std::env::temp_dir().join(format!("tequila-{}-{name}", std::process::id()));
            std::fs::write(&path, bytes).unwrap();
            Scratch(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn rgba_at(image: &Image, x: usize, y: usize) -> [f32; 4] {
        assert_eq!(image.channels, 4);
        let i = (y * image.width + x) * 4;
        image.data[i..i + 4].try_into().unwrap()
    }

    fn near(a: [f32; 4], b: [f32; 4], tolerance: f32) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() <= tolerance)
    }

    /// The hand-read formats arrive the way a PNG does: the sRGB curve off
    /// for artwork, the stored numbers kept for data, alpha untouched.
    #[test]
    fn hand_read_formats_decode_by_role_like_a_png() {
        let mut tga = crate::tga::tests::header(2, 1, 1, 32, 0x28);
        tga.extend_from_slice(&[0, 128, 255, 64]);
        let mut bmp = b"BM".to_vec();
        let mut dib = vec![0u8; 40];
        dib[0] = 40;
        dib[4] = 1;
        dib[8] = 1;
        dib[12] = 1;
        dib[14] = 32;
        bmp.extend_from_slice(&(14u32 + 40 + 4).to_le_bytes());
        bmp.extend_from_slice(&[0; 4]);
        bmp.extend_from_slice(&54u32.to_le_bytes());
        bmp.extend_from_slice(&dib);
        bmp.extend_from_slice(&[0, 128, 255, 64]);
        for (name, bytes) in [("pixel.tga", tga), ("pixel.BMP", bmp)] {
            let file = Scratch::new(name, &bytes);
            let art = Image::load(&file.0).unwrap();
            assert_eq!(art.color_space, ColorSpace::LinearSrgb);
            let expected = [1.0, srgb_eotf(128.0 / 255.0), 0.0, 64.0 / 255.0];
            assert!(near(rgba_at(&art, 0, 0), expected, 1e-5), "{name}: {:?}", rgba_at(&art, 0, 0));
            let data = Image::load_raw(&file.0).unwrap();
            assert_eq!(data.color_space, ColorSpace::Raw);
            assert!(near(rgba_at(&data, 0, 0), [1.0, 128.0 / 255.0, 0.0, 64.0 / 255.0], 1e-5), "{name} raw");
        }
    }

    /// TIFFs at each depth and compression the `tiff` crate writes, read
    /// back through `Image::load`: pixels, rows in order, alpha, and the
    /// precision of sixteen bits and of floats.
    #[test]
    fn tiffs_read_at_every_depth_and_compression() {
        use tiff::encoder::{colortype, compression, TiffEncoder};
        let write = |name: &str, f: &dyn Fn(&mut TiffEncoder<&mut std::io::Cursor<Vec<u8>>>)| {
            let mut bytes = std::io::Cursor::new(Vec::new());
            f(&mut TiffEncoder::new(&mut bytes).unwrap());
            Scratch::new(name, bytes.get_ref())
        };

        // 8-bit RGB, LZW: red, green on the top row; blue, white below.
        let rgb8 = [255u8, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];
        let file = write("rgb8.tif", &|e| e.write_image_with_compression::<colortype::RGB8, _>(2, 2, compression::Lzw, &rgb8).unwrap());
        let image = Image::load(&file.0).unwrap();
        assert_eq!((image.width, image.height), (2, 2));
        assert_eq!(rgba_at(&image, 1, 0), [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(rgba_at(&image, 0, 1), [0.0, 0.0, 1.0, 1.0], "the second row is the bottom one");

        // 8-bit RGBA, PackBits, as Tequila's own renders are written: no
        // ExtraSamples tag, and the fourth channel is alpha.
        let rgba8 = [200u8, 100, 50, 128];
        let file = write("rgba8.tiff", &|e| e.write_image_with_compression::<colortype::RGBA8, _>(1, 1, compression::Packbits, &rgba8).unwrap());
        let raw = Image::load_raw(&file.0).unwrap();
        assert!(near(rgba_at(&raw, 0, 0), [200.0 / 255.0, 100.0 / 255.0, 50.0 / 255.0, 128.0 / 255.0], 1e-6));
        let art = Image::load(&file.0).unwrap();
        assert!((rgba_at(&art, 0, 0)[0] - srgb_eotf(200.0 / 255.0)).abs() < 1e-6, "the sRGB curve is removed");

        // 16-bit grey, Deflate: a value eight bits cannot hold survives.
        let grey16 = [0u16, 1000, 40000, 65535];
        let file = write("grey16.tif", &|e| {
            e.write_image_with_compression::<colortype::Gray16, _>(4, 1, compression::Deflate::default(), &grey16).unwrap()
        });
        let raw = Image::load_raw(&file.0).unwrap();
        let p = rgba_at(&raw, 1, 0);
        assert!((p[0] - 1000.0 / 65535.0).abs() < 1e-7 && p[0] == p[1] && p[1] == p[2] && p[3] == 1.0, "{p:?}");

        // 32-bit float RGBA: linear, past one, the same whichever way it is
        // loaded.
        let float = [12.0f32, 0.5, 0.25, 1.0, 0.0, 0.0, 0.0, 0.0];
        let file = write("float.tif", &|e| e.write_image::<colortype::RGBA32Float>(2, 1, &float).unwrap());
        for image in [Image::load(&file.0).unwrap(), Image::load_raw(&file.0).unwrap()] {
            assert_eq!(image.color_space, ColorSpace::LinearSrgb);
            assert_eq!(rgba_at(&image, 0, 0), [12.0, 0.5, 0.25, 1.0]);
            assert_eq!(rgba_at(&image, 1, 0)[3], 0.0);
        }
    }

    /// The tags the `tiff` crate leaves to its callers: an orientation, a
    /// premultiplied alpha, and an extra channel that is not alpha at all.
    #[test]
    fn tiff_orientation_and_extra_channels_are_honoured() {
        use tiff::encoder::{colortype, TiffEncoder};
        use tiff::tags::Tag;
        let write = |name: &str, width: u32, height: u32, data: &[u8], tags: &[(Tag, u16)]| {
            let mut bytes = std::io::Cursor::new(Vec::new());
            let mut encoder = TiffEncoder::new(&mut bytes).unwrap();
            let mut image = encoder.new_image::<colortype::RGBA8>(width, height).unwrap();
            for (tag, value) in tags {
                image.encoder().write_tag(*tag, *value).unwrap();
            }
            image.write_data(data).unwrap();
            Scratch::new(name, bytes.get_ref())
        };
        // Stored as two rows of three: red green blue, then black white grey.
        let stored = [
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, //
            0, 0, 0, 255, 255, 255, 255, 255, 128, 128, 128, 255,
        ];

        // Orientation 4: stored bottom row first.
        let file = write("flipped.tif", 3, 2, &stored, &[(Tag::Orientation, 4)]);
        let image = Image::load_raw(&file.0).unwrap();
        assert_eq!(rgba_at(&image, 0, 0), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(rgba_at(&image, 0, 1), [1.0, 0.0, 0.0, 1.0]);

        // Orientation 6: turned a quarter clockwise to stand upright, so
        // three across becomes three down, and the first stored row ends
        // up as the right-hand column.
        let file = write("turned.tif", 3, 2, &stored, &[(Tag::Orientation, 6)]);
        let image = Image::load_raw(&file.0).unwrap();
        assert_eq!((image.width, image.height), (2, 3));
        assert_eq!(rgba_at(&image, 1, 0), [1.0, 0.0, 0.0, 1.0], "red: top right");
        assert_eq!(rgba_at(&image, 1, 2), [0.0, 0.0, 1.0, 1.0], "blue: bottom right");
        assert_eq!(rgba_at(&image, 0, 0), [0.0, 0.0, 0.0, 1.0], "black: top left");

        // Premultiplied: 100 at alpha 0.5 is 200 as a colour.
        let file = write("associated.tif", 1, 1, &[100, 50, 0, 128], &[(Tag::ExtraSamples, 1)]);
        let p = rgba_at(&Image::load_raw(&file.0).unwrap(), 0, 0);
        assert!(near(p, [100.0 / 128.0, 50.0 / 128.0, 0.0, 128.0 / 255.0], 1e-5), "{p:?}");

        // Unspecified: a saved selection, not alpha.
        let file = write("selection.tif", 1, 1, &[100, 50, 0, 0], &[(Tag::ExtraSamples, 0)]);
        assert_eq!(rgba_at(&Image::load_raw(&file.0).unwrap(), 0, 0)[3], 1.0);
    }

    /// Lossless WebP, encoded here, comes back bit for bit with its alpha.
    #[test]
    fn a_lossless_webp_keeps_every_value_and_its_alpha() {
        let rgba: Vec<u8> = (0..4 * 3).flat_map(|i: u32| [(i * 20) as u8, 255 - (i * 20) as u8, 7, (i * 21) as u8]).collect();
        let mut bytes = Vec::new();
        image_webp::WebPEncoder::new(&mut bytes).encode(&rgba, 4, 3, image_webp::ColorType::Rgba8).unwrap();
        let file = Scratch::new("lossless.webp", &bytes);
        let image = Image::load_raw(&file.0).unwrap();
        assert_eq!((image.width, image.height), (4, 3));
        for (i, (got, wanted)) in image.data.iter().zip(&rgba).enumerate() {
            assert!((got - *wanted as f32 / 255.0).abs() < 1e-6, "value {i}: {got} for {wanted}");
        }

        let mut bytes = Vec::new();
        image_webp::WebPEncoder::new(&mut bytes).encode(&[10, 20, 30, 40, 50, 60], 2, 1, image_webp::ColorType::Rgb8).unwrap();
        let file = Scratch::new("opaque.webp", &bytes);
        let image = Image::load_raw(&file.0).unwrap();
        assert!(near(rgba_at(&image, 1, 0), [40.0 / 255.0, 50.0 / 255.0, 60.0 / 255.0, 1.0], 1e-6));
    }

    /// Lossy WebP. No pure-Rust encoder writes it, so these are written
    /// out in full: a browser's own encoder (Chromium's canvas, quality 0.9)
    /// given a 32 by 32 picture of four 16-pixel quadrants, with its ICC
    /// profile chunk taken out. The first is red (220, 30, 30) and green
    /// (30, 200, 40) over blue (30, 40, 220) at half alpha and nothing at
    /// all, so it is lossy colour with a lossless alpha chunk (VP8X, ALPH,
    /// VP8); the second the same with an opaque near-white (250) for the
    /// last quadrant, a bare VP8 stream.
    const LOSSY_WITH_ALPHA: [u8; 178] = [
        0x52, 0x49, 0x46, 0x46, 0xaa, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50, 0x56, 0x50, 0x38, 0x58,
        0x0a, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x1f, 0x00, 0x00, 0x1f, 0x00, 0x00, 0x41, 0x4c,
        0x50, 0x48, 0x1d, 0x00, 0x00, 0x00, 0x01, 0x17, 0x20, 0x10, 0x48, 0x52, 0x9f, 0x7c, 0x8d, 0x88,
        0x08, 0x07, 0x32, 0x69, 0x9b, 0x98, 0x9c, 0xc9, 0x7a, 0xdd, 0x1f, 0xd1, 0xff, 0x09, 0xd8, 0xfd,
        0x2a, 0xf8, 0x05, 0x00, 0x56, 0x50, 0x38, 0x20, 0x66, 0x00, 0x00, 0x00, 0x90, 0x05, 0x00, 0x9d,
        0x01, 0x2a, 0x20, 0x00, 0x20, 0x00, 0x3e, 0x2d, 0x10, 0x86, 0x42, 0xa1, 0xa1, 0x0d, 0xfa, 0x00,
        0x0c, 0x01, 0x62, 0x5a, 0x00, 0x27, 0x4c, 0xa1, 0x1c, 0x0d, 0xe0, 0x1f, 0x80, 0x1f, 0xb0, 0x1f,
        0xda, 0xf9, 0xc3, 0x34, 0x0b, 0xb0, 0x1f, 0xb7, 0x3f, 0xd9, 0x33, 0xfe, 0xc2, 0xe0, 0x2d, 0x58,
        0x00, 0x00, 0xfe, 0xf5, 0x58, 0xbf, 0xfe, 0x89, 0x9b, 0x9c, 0xc6, 0xff, 0xed, 0x50, 0x90, 0x44,
        0x17, 0x82, 0x26, 0xef, 0x99, 0x6f, 0x73, 0x2e, 0x87, 0xd7, 0x7f, 0x7f, 0x88, 0x80, 0x2c, 0x4f,
        0x1b, 0xda, 0x6a, 0x4b, 0xd2, 0xe7, 0x5f, 0xf1, 0x6e, 0xdd, 0x26, 0x42, 0x7e, 0xb8, 0x6f, 0x64,
        0x00, 0x00,
    ];

    const LOSSY_OPAQUE: [u8; 122] = [
        0x52, 0x49, 0x46, 0x46, 0x72, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50, 0x56, 0x50, 0x38, 0x20,
        0x66, 0x00, 0x00, 0x00, 0x90, 0x05, 0x00, 0x9d, 0x01, 0x2a, 0x20, 0x00, 0x20, 0x00, 0x3e, 0x2d,
        0x10, 0x86, 0x42, 0xa1, 0xa1, 0x0d, 0xfa, 0x00, 0x0c, 0x01, 0x62, 0x5a, 0x00, 0x27, 0x4c, 0xa1,
        0x1c, 0x0d, 0xe0, 0x1f, 0x80, 0x1f, 0xb0, 0x1f, 0xda, 0xf9, 0xc3, 0x34, 0x0b, 0xb0, 0x1f, 0xb7,
        0x3f, 0xd9, 0x33, 0xfe, 0xc2, 0xe0, 0x2c, 0xbe, 0x00, 0x00, 0xfe, 0xf5, 0x58, 0xbf, 0xfe, 0x89,
        0x9b, 0x9c, 0xc6, 0xff, 0xed, 0x50, 0x90, 0x44, 0x17, 0x82, 0x26, 0xef, 0x99, 0x6f, 0x73, 0x2e,
        0x87, 0xd7, 0x7f, 0x7f, 0x88, 0x80, 0x57, 0xaf, 0xec, 0x41, 0x20, 0xd0, 0x03, 0x0b, 0x9f, 0xfe,
        0x4b, 0x80, 0xd0, 0x13, 0xe9, 0x96, 0x1c, 0x1f, 0x00, 0x00,
    ];

    #[test]
    fn lossy_webp_decodes_near_the_colours_it_was_made_from() {
        let file = Scratch::new("lossy-alpha.webp", &LOSSY_WITH_ALPHA);
        let image = Image::load_raw(&file.0).unwrap();
        assert_eq!((image.width, image.height), (32, 32));
        let at = |x: usize, y: usize| rgba_at(&image, x, y).map(|v| v * 255.0);
        // Lossy colour, so near; at the middle of each quadrant, away from
        // the blur at its edges.
        let tolerance = 12.0;
        assert!(near(at(8, 8), [220.0, 30.0, 30.0, 255.0], tolerance), "top left {:?}", at(8, 8));
        assert!(near(at(24, 8), [30.0, 200.0, 40.0, 255.0], tolerance), "top right {:?}", at(24, 8));
        let blue = at(8, 24);
        assert!((blue[3] - 128.0).abs() <= 1.0, "alpha is coded losslessly: {blue:?}");
        assert!(near([blue[0], blue[1], blue[2], 0.0], [30.0, 40.0, 220.0, 0.0], tolerance), "bottom left {blue:?}");
        assert_eq!(at(24, 24)[3], 0.0, "bottom right is transparent");

        let file = Scratch::new("lossy.webp", &LOSSY_OPAQUE);
        let image = Image::load(&file.0).unwrap();
        assert_eq!(image.color_space, ColorSpace::LinearSrgb);
        let (white, expected) = (rgba_at(&image, 24, 24), srgb_eotf(250.0 / 255.0));
        assert!(near(white, [expected, expected, expected, 1.0], 0.05), "{white:?}");
        assert!(rgba_at(&image, 8, 8)[0] > 0.6, "red decodes as red, the curve removed");
    }

    /// A GIF's first frame, smaller than its canvas and set into it, with
    /// its transparent colour; a second frame is not shown.
    #[test]
    fn a_gif_shows_its_first_frame_in_place() {
        let mut bytes = Vec::new();
        {
            // Red, blue, and a third colour that the frame calls transparent.
            let palette = [255, 0, 0, 0, 0, 255, 9, 9, 9];
            let mut encoder = gif::Encoder::new(&mut bytes, 4, 3, &palette).unwrap();
            let mut first = gif::Frame { left: 1, top: 1, width: 2, height: 2, transparent: Some(2), ..Default::default() };
            first.buffer = std::borrow::Cow::Owned(vec![0, 1, 2, 0]);
            encoder.write_frame(&first).unwrap();
            let second = gif::Frame { width: 4, height: 3, buffer: std::borrow::Cow::Owned(vec![1; 12]), ..Default::default() };
            encoder.write_frame(&second).unwrap();
        }
        let file = Scratch::new("frames.gif", &bytes);
        let image = Image::load_raw(&file.0).unwrap();
        assert_eq!((image.width, image.height), (4, 3), "the canvas, not the frame");
        assert_eq!(rgba_at(&image, 1, 1), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(rgba_at(&image, 2, 1), [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(rgba_at(&image, 1, 2)[3], 0.0, "the transparent colour");
        assert_eq!(rgba_at(&image, 2, 2), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(rgba_at(&image, 0, 0)[3], 0.0, "outside the first frame is transparent");
        assert_eq!(rgba_at(&image, 3, 2)[3], 0.0);
    }

    /// JPEG XL, encoded here losslessly, comes back exactly: eight-bit RGBA
    /// with its alpha, and sixteen-bit grey with its precision.
    #[test]
    fn a_jpeg_xl_decodes_exactly() {
        use zune_core::{bit_depth::BitDepth, colorspace::ColorSpace as Layout, options::EncoderOptions};
        let encode = |data: &[u8], width: usize, height: usize, layout: Layout, depth: BitDepth| {
            let mut out = Vec::new();
            zune_jpegxl::JxlSimpleEncoder::new(data, EncoderOptions::new(width, height, layout, depth))
                .encode(&mut out)
                .unwrap();
            out
        };

        let rgba: Vec<u8> = (0..4 * 3u32).flat_map(|i| [(i * 20) as u8, 200 - (i * 10) as u8, 7, (255 - i * 15) as u8]).collect();
        let file = Scratch::new("pixels.jxl", &encode(&rgba, 4, 3, Layout::RGBA, BitDepth::Eight));
        let raw = Image::load_raw(&file.0).unwrap();
        assert_eq!((raw.width, raw.height), (4, 3));
        assert_eq!(raw.color_space, ColorSpace::Raw);
        for (i, (got, wanted)) in raw.data.iter().zip(&rgba).enumerate() {
            assert!((got * 255.0 - *wanted as f32).abs() < 0.01, "value {i}: {} for {wanted}", got * 255.0);
        }
        let art = Image::load(&file.0).unwrap();
        assert!((art.data[4] - srgb_eotf(20.0 / 255.0)).abs() < 1e-4, "the sRGB curve is removed");

        let grey: Vec<u16> = vec![0, 1000, 40000, 65535, 12345, 54321];
        let bytes: Vec<u8> = grey.iter().flat_map(|v| v.to_ne_bytes()).collect();
        let file = Scratch::new("grey16.jxl", &encode(&bytes, 3, 2, Layout::Luma, BitDepth::Sixteen));
        let raw = Image::load_raw(&file.0).unwrap();
        let p = rgba_at(&raw, 1, 0);
        assert!((p[0] - 1000.0 / 65535.0).abs() < 1e-6 && p[0] == p[2] && p[3] == 1.0, "{p:?}");
        assert!((rgba_at(&raw, 2, 1)[0] - 54321.0 / 65535.0).abs() < 1e-6);
    }

    /// DDS and SVG through `Image::load`, by role like every other format.
    #[test]
    fn dds_and_svg_load_by_extension() {
        let mut dds = b"DDS ".to_vec();
        let mut words = [0u32; 31];
        words[0] = 124;
        words[2] = 1;
        words[3] = 1;
        words[18] = 32;
        words[19] = 0x41;
        words[21] = 32;
        words[22..26].copy_from_slice(&[0xFF, 0xFF00, 0xFF_0000, 0xFF00_0000]);
        for w in words {
            dds.extend_from_slice(&w.to_le_bytes());
        }
        dds.extend_from_slice(&[255, 128, 0, 64]);
        let file = Scratch::new("pixel.dds", &dds);
        let p = rgba_at(&Image::load(&file.0).unwrap(), 0, 0);
        assert!(near(p, [1.0, srgb_eotf(128.0 / 255.0), 0.0, 64.0 / 255.0], 1e-5), "{p:?}");

        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2 1"><rect width="1" height="1" fill="#808080"/></svg>"##;
        let file = Scratch::new("half.svg", svg.as_bytes());
        let image = Image::load_raw(&file.0).unwrap();
        assert_eq!((image.width, image.height), (4096, 2048));
        assert!(near(rgba_at(&image, 1000, 1000), [128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 1.0], 1e-6));
        assert_eq!(rgba_at(&image, 3000, 1000)[3], 0.0);
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
            if let Err(ImageError::UnsupportedFormat(_)) = Image::load_raw(&path) {
                panic!("`{ext}` is listed as decodable but `load_raw` refuses it")
            }
        }
        assert!(Image::is_decodable("photo.tga"));
        assert!(!Image::is_decodable("photo.heic"));
        assert!(!Image::is_decodable("no-extension"));
        // Extensions come off files a person named, so case is not a signal.
        assert!(Image::is_decodable("COLOUR.JPG"));
    }
}
