//! Writing a finished render (`TEQUILA_SPEC.md` §1.8, `docs/01` §12).
//!
//! Everything a render turns into on the way out: the file formats, the three
//! alpha modes, the filename template, the print-size arithmetic and the
//! metadata sidecar.
//!
//! # The alpha modes are not cosmetic
//!
//! §12 asks for Straight, Premultiplied and Aliased, and the difference between
//! the first two is the single most common way a composite comes out with a
//! dark fringe. A premultiplied pixel holds `colour × coverage`; a straight one
//! holds the colour the surface would have if it covered the pixel entirely.
//! The renderer produces the former — an edge pixel is the average of samples
//! that hit and samples that did not — so Straight has to *divide it back out*,
//! and where coverage is zero there is nothing to divide, which is why the
//! colour has to be carried in from a neighbour instead of left at black.

use crate::exr::{Channels, ExrImage, Layer as ExrLayer, Precision};
use crate::psd::{Depth, PsdImage, PsdLayer};
use tequila_color::{DisplayPipeline, ImageStyle};
use tequila_core::spectrum::Spectrum;
use std::path::{Path, PathBuf};

/// How the coverage is folded into the colour (§12).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AlphaMode {
    /// No transparency: the picture as it looks on screen, background and
    /// all, every pixel opaque. What an image viewer, a web page or a slide
    /// shows best — a file with alpha shows its background as whatever the
    /// viewer puts behind it, which is white or black or a checkerboard, and
    /// never the studio the product was lit in.
    None,
    /// Colour as if the pixel were fully covered, with the edge colour carried
    /// outwards. What a compositor expects when it multiplies by alpha itself.
    Straight,
    /// Colour already multiplied by coverage — matted against black. What the
    /// renderer produces, and what "over" composites directly.
    #[default]
    Premultiplied,
    /// Coverage rounded to zero or one. For a pipeline that cannot handle a
    /// soft edge at all, at the cost of a hard one.
    Aliased,
}

/// The file formats §1.8 lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    /// Quality 1 to 100.
    Jpeg(u8),
    /// Eight or sixteen bits per sample, or full float for TIFF32.
    Tiff(Depth),
    Exr(Precision),
    Psd(Depth),
}

impl ImageFormat {
    /// The extension a file of this format gets.
    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg(_) => "jpg",
            ImageFormat::Tiff(_) => "tif",
            ImageFormat::Exr(_) => "exr",
            ImageFormat::Psd(_) => "psd",
        }
    }

    /// Whether the format can carry coverage at all.
    ///
    /// JPEG cannot, and that is worth knowing *before* the render rather than
    /// after: a shadow-catcher setup written to JPEG loses the one thing it was
    /// set up for.
    pub fn carries_alpha(self) -> bool {
        !matches!(self, ImageFormat::Jpeg(_))
    }

    /// Whether the format keeps values above one.
    pub fn is_high_dynamic_range(self) -> bool {
        matches!(
            self,
            ImageFormat::Exr(_)
                | ImageFormat::Tiff(Depth::ThirtyTwo)
                | ImageFormat::Psd(Depth::ThirtyTwo)
        )
    }
}

/// A finished render, before it becomes a file.
#[derive(Clone, Debug)]
pub struct RenderOutput {
    pub width: usize,
    pub height: usize,
    /// Scene-linear beauty, premultiplied by coverage as the renderer produced
    /// it.
    pub color: Vec<Spectrum>,
    pub alpha: Vec<f32>,
    /// Named passes, for the layered formats.
    /// Named passes, and the coverage of those that have one of their own
    /// (the Labels pass: artwork over nothing); `None` is opaque.
    pub layers: Vec<(String, Vec<Spectrum>, Option<Vec<f32>>)>,
    pub style: ImageStyle,
    pub display: DisplayPipeline,
    pub alpha_mode: AlphaMode,
}

#[derive(Debug)]
pub enum OutputError {
    Io(String),
    Encode(String),
}

impl std::fmt::Display for OutputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputError::Io(m) | OutputError::Encode(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for OutputError {}

impl RenderOutput {
    /// The colour and coverage a file should carry, in the chosen alpha mode.
    ///
    /// Returns scene-linear colour: the eight-bit formats grade it afterwards,
    /// and the float formats want it as it is.
    pub fn matted(&self) -> (Vec<Spectrum>, Vec<f32>) {
        match self.alpha_mode {
            // The colour is already the finished picture: what a camera ray
            // saw, the background included.
            AlphaMode::None => (self.color.clone(), vec![1.0; self.alpha.len()]),
            AlphaMode::Premultiplied => (self.color.clone(), self.alpha.clone()),
            AlphaMode::Aliased => {
                // A hard edge: coverage rounds, and a pixel that is kept is
                // un-premultiplied so it does not come out half-dark.
                let alpha: Vec<f32> = self
                    .alpha
                    .iter()
                    .map(|a| if *a >= 0.5 { 1.0 } else { 0.0 })
                    .collect();
                let color = self
                    .color
                    .iter()
                    .zip(&self.alpha)
                    .zip(&alpha)
                    .map(|((c, original), kept)| {
                        if *kept > 0.0 && *original > 1e-6 {
                            *c / *original
                        } else {
                            Spectrum::ZERO
                        }
                    })
                    .collect();
                (color, alpha)
            }
            AlphaMode::Straight => (self.unmatted(), self.alpha.clone()),
        }
    }

    /// Colour with the coverage divided back out, and carried outwards where
    /// there is none.
    ///
    /// The carrying is the part that matters. A straight-alpha edge pixel with
    /// coverage 0.02 divides by 0.02 and amplifies its own noise enormously; a
    /// pixel with coverage 0 has no colour at all and leaving it black produces
    /// exactly the dark fringe straight alpha exists to avoid. So below a
    /// threshold the colour comes from the nearest neighbour that has one.
    pub fn unmatted(&self) -> Vec<Spectrum> {
        const FLOOR: f32 = 0.05;
        let mut out = vec![Spectrum::ZERO; self.color.len()];
        for (index, (color, alpha)) in self.color.iter().zip(&self.alpha).enumerate() {
            if *alpha >= FLOOR {
                out[index] = *color / *alpha;
            }
        }
        // One pass outwards per ring, repeated until the edge has been carried
        // far enough to survive any resampling a compositor does.
        let mut filled: Vec<bool> = self.alpha.iter().map(|a| *a >= FLOOR).collect();
        for _ in 0..4 {
            let previous = filled.clone();
            let snapshot = out.clone();
            for y in 0..self.height {
                for x in 0..self.width {
                    let index = y * self.width + x;
                    if previous[index] {
                        continue;
                    }
                    let mut sum = Spectrum::ZERO;
                    let mut count = 0.0;
                    for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                        let nx = x as i32 + dx;
                        let ny = y as i32 + dy;
                        if nx < 0 || ny < 0 || nx as usize >= self.width || ny as usize >= self.height
                        {
                            continue;
                        }
                        let neighbour = ny as usize * self.width + nx as usize;
                        if previous[neighbour] {
                            sum = sum + snapshot[neighbour];
                            count += 1.0;
                        }
                    }
                    if count > 0.0 {
                        out[index] = sum / count;
                        filled[index] = true;
                    }
                }
            }
        }
        out
    }

    /// Eight-bit RGBA, graded and display-encoded.
    pub fn to_rgba8(&self) -> Vec<u8> {
        let (color, alpha) = self.matted();
        let mut out = self
            .style
            .develop_through(self.width, self.height, &color, &self.display);
        for (index, coverage) in alpha.iter().enumerate() {
            out[index * 4 + 3] = (coverage.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
        out
    }

    pub fn write(&self, path: impl AsRef<Path>, format: ImageFormat) -> Result<(), OutputError> {
        let path = path.as_ref();
        match format {
            ImageFormat::Png => self.write_png(path),
            ImageFormat::Jpeg(quality) => self.write_jpeg(path, quality),
            ImageFormat::Tiff(depth) => self.write_tiff(path, depth),
            ImageFormat::Exr(precision) => self.write_exr(path, precision),
            ImageFormat::Psd(depth) => self.write_psd(path, depth),
        }
    }

    fn write_png(&self, path: &Path) -> Result<(), OutputError> {
        let file = std::fs::File::create(path).map_err(|e| OutputError::Io(e.to_string()))?;
        let mut encoder = png::Encoder::new(
            std::io::BufWriter::new(file),
            self.width as u32,
            self.height as u32,
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| OutputError::Encode(e.to_string()))?;
        writer
            .write_image_data(&self.to_rgba8())
            .map_err(|e| OutputError::Encode(e.to_string()))
    }

    fn write_jpeg(&self, path: &Path, quality: u8) -> Result<(), OutputError> {
        // No alpha channel exists in a baseline JPEG, so the image is matted
        // against black rather than silently dropping the coverage — a
        // half-covered edge pixel has to become *something*, and black is what
        // "premultiplied" already means.
        let rgba = self.to_rgba8();
        let rgb: Vec<u8> = rgba
            .chunks_exact(4)
            .flat_map(|p| {
                let a = p[3] as f32 / 255.0;
                [
                    (p[0] as f32 * a) as u8,
                    (p[1] as f32 * a) as u8,
                    (p[2] as f32 * a) as u8,
                ]
            })
            .collect();
        let encoder = jpeg_encoder::Encoder::new_file(path, quality.clamp(1, 100))
            .map_err(|e| OutputError::Io(e.to_string()))?;
        encoder
            .encode(
                &rgb,
                self.width as u16,
                self.height as u16,
                jpeg_encoder::ColorType::Rgb,
            )
            .map_err(|e| OutputError::Encode(e.to_string()))
    }

    fn write_tiff(&self, path: &Path, depth: Depth) -> Result<(), OutputError> {
        use tiff::encoder::{colortype, TiffEncoder};
        let file = std::fs::File::create(path).map_err(|e| OutputError::Io(e.to_string()))?;
        let mut encoder =
            TiffEncoder::new(std::io::BufWriter::new(file)).map_err(|e| OutputError::Encode(e.to_string()))?;
        let (width, height) = (self.width as u32, self.height as u32);
        match depth {
            Depth::Eight => {
                let rgba = self.to_rgba8();
                encoder
                    .write_image::<colortype::RGBA8>(width, height, &rgba)
                    .map_err(|e| OutputError::Encode(e.to_string()))
            }
            Depth::Sixteen => {
                let rgba = self.to_rgba8();
                // Sixteen bits from eight would be a lie about the precision,
                // so the grade is redone at full width instead.
                let (color, alpha) = self.matted();
                let mut samples = Vec::with_capacity(color.len() * 4);
                for (c, a) in color.iter().zip(&alpha) {
                    let graded = self.display.apply(self.style.grade(*c));
                    for channel in [graded.x, graded.y, graded.z] {
                        samples.push((channel.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16);
                    }
                    samples.push((a.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16);
                }
                let _ = rgba;
                encoder
                    .write_image::<colortype::RGBA16>(width, height, &samples)
                    .map_err(|e| OutputError::Encode(e.to_string()))
            }
            Depth::ThirtyTwo => {
                // TIFF32: scene-linear, ungraded. The point of asking for it.
                let (color, alpha) = self.matted();
                let mut samples = Vec::with_capacity(color.len() * 4);
                for (c, a) in color.iter().zip(&alpha) {
                    let rgb = c.to_rendering_rgb();
                    samples.extend_from_slice(&[rgb.x, rgb.y, rgb.z, *a]);
                }
                encoder
                    .write_image::<colortype::RGBA32Float>(width, height, &samples)
                    .map_err(|e| OutputError::Encode(e.to_string()))
            }
        }
    }

    fn write_exr(&self, path: &Path, precision: Precision) -> Result<(), OutputError> {
        let (color, alpha) = self.matted();
        let mut image = ExrImage::new(self.width, self.height);
        image.push(ExrLayer {
            name: String::new(),
            channels: Channels::Color(color),
            precision,
        });
        image.push(ExrLayer {
            name: "A".into(),
            channels: Channels::Scalar(alpha),
            precision,
        });
        for (name, pixels, alpha) in &self.layers {
            let channels = match alpha {
                // Premultiplied, as EXR is.
                Some(alpha) => Channels::Rgba(
                    pixels
                        .iter()
                        .zip(alpha)
                        .map(|(c, a)| {
                            let rgb = c.to_rendering_rgb();
                            [rgb.x * a, rgb.y * a, rgb.z * a, *a]
                        })
                        .collect(),
                ),
                None => Channels::Color(pixels.clone()),
            };
            image.push(ExrLayer { name: name.clone(), channels, precision });
        }
        image
            .write(path)
            .map_err(|e| OutputError::Encode(e.to_string()))
    }

    fn write_psd(&self, path: &Path, depth: Depth) -> Result<(), OutputError> {
        let (color, alpha) = self.matted();
        let opaque = vec![1.0; color.len()];
        let document = PsdImage {
            width: self.width,
            height: self.height,
            depth,
            composite: PsdLayer {
                name: "Beauty".into(),
                pixels: color,
                alpha,
            },
            layers: self
                .layers
                .iter()
                .map(|(name, pixels, alpha)| PsdLayer {
                    name: name.clone(),
                    pixels: pixels.clone(),
                    alpha: alpha.clone().unwrap_or_else(|| opaque.clone()),
                })
                .collect(),
            style: self.style,
        };
        let bytes = document
            .encode()
            .map_err(|e| OutputError::Encode(e.to_string()))?;
        std::fs::write(path, bytes).map_err(|e| OutputError::Io(e.to_string()))
    }
}

// ---------------------------------------------------------------------------
// Filenames, print sizes and sidecars
// ---------------------------------------------------------------------------

/// What a filename template may refer to.
#[derive(Clone, Debug, Default)]
pub struct NameFields {
    pub project: String,
    pub camera: String,
    /// A configurator combination's choices, "Red - Leather - Front"
    /// (`TEQUILA_SPEC.md` §1.13); empty for an ordinary render.
    pub configuration: String,
    pub model_set: String,
    pub studio: String,
    pub frame: Option<u32>,
    pub width: usize,
    pub height: usize,
    pub samples: u32,
    /// `YYYY-MM-DD` and `HH-MM-SS`, supplied rather than read from the clock so
    /// a whole queue's filenames share one timestamp.
    pub date: String,
    pub time: String,
}

/// Expand `{field}` placeholders in a filename template (§1.8).
///
/// An unknown field is left as written rather than dropped. A user who typos
/// `{cammera}` gets a file with `{cammera}` in its name, which is a mistake
/// they can see; silently removing it produces two renders that overwrite each
/// other and no clue why.
pub fn expand_template(template: &str, fields: &NameFields) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('}') else {
            // An unclosed brace is literal text.
            out.push_str(&rest[start..]);
            return sanitize(&out);
        };
        let name = &rest[start + 1..start + end];
        let value = match name {
            "project" => Some(fields.project.clone()),
            "camera" => Some(fields.camera.clone()),
            "configuration" => Some(fields.configuration.clone()),
            "model_set" => Some(fields.model_set.clone()),
            "studio" => Some(fields.studio.clone()),
            "frame" => fields.frame.map(|f| format!("{f:04}")),
            "width" => Some(fields.width.to_string()),
            "height" => Some(fields.height.to_string()),
            "resolution" => Some(format!("{}x{}", fields.width, fields.height)),
            "samples" => Some(fields.samples.to_string()),
            "date" => Some(fields.date.clone()),
            "time" => Some(fields.time.clone()),
            _ => None,
        };
        match value {
            Some(value) => out.push_str(&value),
            None => out.push_str(&rest[start..start + end + 1]),
        }
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    sanitize(&out)
}

/// Replace what a filesystem will not take.
///
/// Applied after expansion, not before: a camera called `Hero 3/4` is a
/// perfectly good camera name and a terrible path component, and the user
/// should not have to know that when they name it.
fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if (c as u32) < 0x20 => '-',
            c => c,
        })
        .collect::<String>()
        .trim()
        .trim_end_matches('.')
        .to_string()
}

/// Give a render a filename, with its extension.
pub fn output_path(
    directory: &Path,
    template: &str,
    fields: &NameFields,
    format: ImageFormat,
) -> PathBuf {
    let stem = expand_template(template, fields);
    let stem = if stem.is_empty() {
        "render".to_string()
    } else {
        stem
    };
    directory.join(format!("{stem}.{}", format.extension()))
}

/// The path itself if nothing is there, otherwise the first `_2`, `_3`, …
/// beside it that is free.
///
/// The default name is `{project}_{camera}_{date}`, so two renders of one
/// camera on one day resolve to the same file, and the second used to replace
/// the first without a word — the gallery showed two renders, the folder held
/// one. A render never overwrites; a person who wants one name per shot can
/// still get it by putting `{time}` in the template.
pub fn first_free(path: PathBuf) -> PathBuf {
    // Taken if the image is there, or a sidecar a render writes beside it
    // under the same name: an EXR rendered after a PNG of the same name was
    // free itself, and its metadata files replaced the PNG's.
    let taken = |p: &Path| p.exists() || p.with_extension("txt").exists() || p.with_extension("xmp").exists();
    if !taken(&path) {
        return path;
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "render".into());
    let extension = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let directory = path.parent().map(Path::to_path_buf).unwrap_or_default();
    (2..)
        .map(|n| directory.join(format!("{stem}_{n}{extension}")))
        .find(|candidate| !taken(candidate))
        .expect("an unbounded range always has a free name")
}

/// §1.8's print-size calculator: pixels, inches and dots per inch.
///
/// Two of the three determine the third, and the calculator's whole job is to
/// let a user fix whichever two they care about. A print shop asks for inches
/// and a DPI; a web page asks for pixels; nobody wants to do the division
/// twice and find they rounded differently.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrintSize {
    pub pixels: (usize, usize),
    pub dpi: f32,
}

impl PrintSize {
    pub fn from_pixels(width: usize, height: usize, dpi: f32) -> PrintSize {
        PrintSize {
            pixels: (width, height),
            dpi: dpi.max(1.0),
        }
    }

    /// The pixel count a physical size at a given density needs.
    pub fn from_inches(width: f32, height: f32, dpi: f32) -> PrintSize {
        let dpi = dpi.max(1.0);
        PrintSize {
            pixels: (
                (width.max(0.0) * dpi).round().max(1.0) as usize,
                (height.max(0.0) * dpi).round().max(1.0) as usize,
            ),
            dpi,
        }
    }

    pub fn from_millimetres(width: f32, height: f32, dpi: f32) -> PrintSize {
        PrintSize::from_inches(width / 25.4, height / 25.4, dpi)
    }

    pub fn inches(&self) -> (f32, f32) {
        (
            self.pixels.0 as f32 / self.dpi,
            self.pixels.1 as f32 / self.dpi,
        )
    }

    pub fn millimetres(&self) -> (f32, f32) {
        let (w, h) = self.inches();
        (w * 25.4, h * 25.4)
    }

    /// Bytes an uncompressed image of this size takes, for the warning a 4-metre
    /// poster at 300 DPI deserves before the render starts rather than after.
    pub fn uncompressed_bytes(&self, bytes_per_sample: usize, channels: usize) -> u64 {
        self.pixels.0 as u64 * self.pixels.1 as u64 * bytes_per_sample as u64 * channels as u64
    }
}

/// The sidecar written beside a render (§1.8).
#[derive(Clone, Debug, Default)]
pub struct Metadata {
    pub fields: Vec<(String, String)>,
}

impl Metadata {
    pub fn set(&mut self, name: impl Into<String>, value: impl Into<String>) -> &mut Self {
        self.fields.push((name.into(), value.into()));
        self
    }

    /// The plain form: one `name: value` per line, for a person.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for (name, value) in &self.fields {
            out.push_str(name);
            out.push_str(": ");
            out.push_str(value);
            out.push('\n');
        }
        out
    }

    /// XMP, for a pipeline.
    ///
    /// A real XMP packet rather than a file of tags that look like one: the
    /// wrapper, the RDF, and the namespace declaration are what make it
    /// readable by anything that reads XMP, and without them it is a text file
    /// with angle brackets.
    pub fn to_xmp(&self) -> String {
        let mut out = String::from(
            "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
             <x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n\
             \x20<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
             \x20\x20<rdf:Description rdf:about=\"\"\n\
             \x20\x20\x20\x20xmlns:tequila=\"https://tequila.dev/ns/1.0/\">\n",
        );
        for (name, value) in &self.fields {
            let tag = name
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect::<String>();
            out.push_str(&format!(
                "    <tequila:{tag}>{}</tequila:{tag}>\n",
                escape_xml(value)
            ));
        }
        out.push_str(
            "\x20\x20</rdf:Description>\n\
             \x20</rdf:RDF>\n\
             </x:xmpmeta>\n\
             <?xpacket end=\"w\"?>\n",
        );
        out
    }
}

fn escape_xml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two renders that resolve to one name both survive.
    #[test]
    fn a_render_never_overwrites_an_earlier_one() {
        let directory = std::env::temp_dir().join(format!("tequila-first-free-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let wanted = directory.join("quest3_Camera 1_2026-09-24.png");

        assert_eq!(first_free(wanted.clone()), wanted, "a free name is used as it is");
        std::fs::write(&wanted, b"first").unwrap();
        let second = first_free(wanted.clone());
        assert_eq!(second, directory.join("quest3_Camera 1_2026-09-24_2.png"));
        std::fs::write(&second, b"second").unwrap();
        assert_eq!(first_free(wanted.clone()), directory.join("quest3_Camera 1_2026-09-24_3.png"));

        // Nor another format's render the first one's metadata: the EXR of
        // the same name would write its text file over the PNG's.
        std::fs::write(wanted.with_extension("txt"), b"the PNG's").unwrap();
        let exr = directory.join("quest3_Camera 1_2026-09-24.exr");
        assert_eq!(first_free(exr), directory.join("quest3_Camera 1_2026-09-24_2.exr"));

        let _ = std::fs::remove_dir_all(&directory);
    }

    fn output(alpha_mode: AlphaMode) -> RenderOutput {
        // A two-by-two image: one covered pixel, one half-covered, two empty.
        RenderOutput {
            width: 2,
            height: 2,
            color: vec![
                Spectrum::splat(0.4),
                Spectrum::splat(0.2),
                Spectrum::ZERO,
                Spectrum::ZERO,
            ],
            alpha: vec![1.0, 0.5, 0.0, 0.0],
            layers: Vec::new(),
            style: ImageStyle::default(),
            display: DisplayPipeline::default(),
            alpha_mode,
        }
    }

    /// The difference between the two alpha modes, stated as the arithmetic it
    /// is. A half-covered pixel holds half the colour; straight alpha divides
    /// that back out so the compositor's own multiply does not do it twice.
    #[test]
    fn straight_alpha_divides_the_coverage_back_out() {
        let (color, alpha) = output(AlphaMode::Straight).matted();
        assert_eq!(alpha, vec![1.0, 0.5, 0.0, 0.0]);
        assert!((color[0].r - 0.4).abs() < 1e-6);
        assert!(
            (color[1].r - 0.4).abs() < 1e-6,
            "the half-covered pixel came out at {}, not the full colour",
            color[1].r
        );
    }

    /// No alpha: every pixel opaque, and the colour — which is already the
    /// picture the camera saw, background included — untouched. With alpha,
    /// a viewer shows the background as whatever it puts behind the image.
    #[test]
    fn no_alpha_keeps_the_picture_and_covers_every_pixel() {
        let source = output(AlphaMode::None);
        let (color, alpha) = source.matted();
        assert_eq!(color, source.color);
        assert_eq!(alpha, vec![1.0; 4]);
    }

    /// And premultiplied leaves it alone, because that is what the renderer
    /// already produced.
    #[test]
    fn premultiplied_alpha_changes_nothing() {
        let source = output(AlphaMode::Premultiplied);
        let (color, alpha) = source.matted();
        assert_eq!(color, source.color);
        assert_eq!(alpha, source.alpha);
    }

    /// The uncovered pixels have no colour of their own, and leaving them black
    /// is exactly the dark fringe straight alpha exists to prevent. They take
    /// their colour from a neighbour that has one.
    #[test]
    fn straight_alpha_carries_the_edge_colour_outwards() {
        let (color, _) = output(AlphaMode::Straight).matted();
        for index in 2..4 {
            assert!(
                color[index].r > 0.3,
                "uncovered pixel {index} came out at {} — a dark fringe",
                color[index].r
            );
        }
    }

    /// Aliased rounds the coverage and un-premultiplies what it keeps, so a
    /// kept edge pixel is the surface's colour rather than half of it.
    #[test]
    fn aliased_alpha_is_hard_edged_and_not_half_dark() {
        let (color, alpha) = output(AlphaMode::Aliased).matted();
        assert_eq!(alpha, vec![1.0, 1.0, 0.0, 0.0]);
        assert!((color[1].r - 0.4).abs() < 1e-6, "{}", color[1].r);
        assert_eq!(color[2], Spectrum::ZERO);
    }

    /// A template that names nothing known is left as written. Dropping it
    /// silently produces two renders with the same name and no clue why one
    /// overwrote the other.
    #[test]
    fn an_unknown_field_survives_rather_than_vanishing() {
        let fields = NameFields {
            project: "Kettle".into(),
            camera: "Hero".into(),
            ..Default::default()
        };
        assert_eq!(
            expand_template("{project}_{cammera}", &fields),
            "Kettle_{cammera}"
        );
        assert_eq!(expand_template("{project}_{camera}", &fields), "Kettle_Hero");
    }

    /// A camera called `Hero 3/4` is a good camera name and a bad path
    /// component. The user should not have to know that when they name it.
    #[test]
    fn a_name_that_a_filesystem_would_refuse_is_repaired() {
        let fields = NameFields {
            camera: "Hero 3/4".into(),
            project: "A:B".into(),
            ..Default::default()
        };
        let name = expand_template("{project}_{camera}", &fields);
        assert_eq!(name, "A-B_Hero 3-4");
        assert!(!name.contains('/') && !name.contains(':'));
    }

    #[test]
    fn templates_fill_in_the_documented_fields() {
        let fields = NameFields {
            project: "Kettle".into(),
            camera: "Hero".into(),
            frame: Some(12),
            width: 1920,
            height: 1080,
            samples: 256,
            date: "2026-09-12".into(),
            ..Default::default()
        };
        assert_eq!(
            expand_template("{project}_{camera}_{frame}_{resolution}_{samples}spp_{date}", &fields),
            "Kettle_Hero_0012_1920x1080_256spp_2026-09-12"
        );
        // An absent frame leaves its placeholder, which is what tells a user
        // they asked for a frame number on a still.
        let still = NameFields {
            frame: None,
            ..fields
        };
        assert!(expand_template("{frame}", &still).contains("frame"));
    }

    /// The extension follows the format, and never the other way round: a user
    /// who types `render.png` and picks EXR gets an EXR.
    #[test]
    fn the_extension_follows_the_format() {
        let fields = NameFields {
            project: "render.png".into(),
            ..Default::default()
        };
        let path = output_path(
            Path::new("/tmp"),
            "{project}",
            &fields,
            ImageFormat::Exr(Precision::Half),
        );
        assert!(path.to_string_lossy().ends_with("render.png.exr"));
    }

    /// Two of pixels, inches and DPI fix the third, and the round trip has to
    /// close — a calculator that disagrees with itself is worse than none.
    #[test]
    fn print_sizes_round_trip() {
        let size = PrintSize::from_inches(8.0, 10.0, 300.0);
        assert_eq!(size.pixels, (2400, 3000));
        let (w, h) = size.inches();
        assert!((w - 8.0).abs() < 1e-3 && (h - 10.0).abs() < 1e-3);

        let metric = PrintSize::from_millimetres(210.0, 297.0, 300.0);
        let (w, h) = metric.millimetres();
        assert!((w - 210.0).abs() < 0.2 && (h - 297.0).abs() < 0.2, "{w}x{h}");
        // A4 at 300 DPI is the number a print shop will quote.
        assert_eq!(metric.pixels, (2480, 3508));
    }

    /// The warning a big print deserves before the render rather than after.
    #[test]
    fn a_large_print_reports_its_size() {
        // Four metres by three at 300 DPI is 47,244 by 35,433 pixels — 1.67
        // gigapixels, which at four four-byte channels is about 27 GB before
        // any compression. Worth saying out loud before the render starts.
        let poster = PrintSize::from_millimetres(4000.0, 3000.0, 300.0);
        assert_eq!(poster.pixels, (47244, 35433));
        let bytes = poster.uncompressed_bytes(4, 4);
        assert!(
            (25_000_000_000..30_000_000_000).contains(&bytes),
            "a four-metre poster at 300 DPI came out at {bytes} bytes"
        );
    }

    /// XMP is a format, not a shape. Without the packet wrapper and the RDF it
    /// is a text file with angle brackets that nothing reading XMP will find.
    #[test]
    fn the_sidecar_is_real_xmp() {
        let mut metadata = Metadata::default();
        metadata
            .set("Renderer", "Tequila")
            .set("Notes", "A & B <test>");
        let xmp = metadata.to_xmp();
        assert!(xmp.starts_with("<?xpacket begin="));
        assert!(xmp.contains("x:xmpmeta"));
        assert!(xmp.contains("rdf:Description"));
        assert!(xmp.contains("xmlns:tequila="));
        assert!(xmp.trim_end().ends_with("<?xpacket end=\"w\"?>"));
        // And the content is escaped, or one ampersand in a note makes the
        // whole packet unparseable.
        assert!(xmp.contains("A &amp; B &lt;test&gt;"), "{xmp}");
        assert!(!xmp.contains("A & B <test>"));
    }

    #[test]
    fn the_plain_sidecar_is_readable() {
        let mut metadata = Metadata::default();
        metadata.set("Samples", "256").set("Renderer", "Tequila");
        assert_eq!(metadata.to_text(), "Samples: 256\nRenderer: Tequila\n");
    }

    /// Every format actually writes a file, and the file starts with what its
    /// readers look for. A format that is offered in the UI and produces
    /// something unopenable is worse than one that is not offered.
    #[test]
    fn every_format_writes_a_file_its_readers_recognize() {
        let directory = std::env::temp_dir().join("tequila-output-test");
        std::fs::create_dir_all(&directory).unwrap();

        let mut source = output(AlphaMode::Premultiplied);
        source.width = 8;
        source.height = 8;
        source.color = (0..64)
            .map(|i| Spectrum::splat(i as f32 / 64.0))
            .collect();
        source.alpha = vec![1.0; 64];
        source.layers = vec![
            ("Diffuse".into(), vec![Spectrum::splat(0.3); 64], None),
            ("Labels".into(), vec![Spectrum::splat(0.6); 64], Some(vec![0.5; 64])),
        ];

        // Magic bytes as numbers rather than as string escapes. The escapes
        // do not survive being written through several layers of quoting, and a
        // test that checks the wrong bytes is worse than no test at all.
        let cases: [(ImageFormat, &[u8]); 7] = [
            (ImageFormat::Png, &[0x89, b'P', b'N', b'G']),
            (ImageFormat::Jpeg(90), &[0xff, 0xd8, 0xff]),
            (ImageFormat::Tiff(Depth::Eight), &[b'I', b'I', 42, 0]),
            (ImageFormat::Tiff(Depth::Sixteen), &[b'I', b'I', 42, 0]),
            (ImageFormat::Tiff(Depth::ThirtyTwo), &[b'I', b'I', 42, 0]),
            (ImageFormat::Exr(Precision::Half), &[0x76, 0x2f, 0x31, 0x01]),
            (ImageFormat::Psd(Depth::Sixteen), &[b'8', b'B', b'P', b'S']),
        ];
        for (format, magic) in cases {
            let path = directory.join(format!("probe.{}", format.extension()));
            source
                .write(&path, format)
                .unwrap_or_else(|e| panic!("{format:?} failed to write: {e}"));
            let bytes = std::fs::read(&path).expect("the file exists");
            assert!(
                bytes.starts_with(magic),
                "{format:?} wrote a file starting {:?}",
                &bytes[..magic.len().min(bytes.len())]
            );
            assert!(bytes.len() > 64, "{format:?} wrote {} bytes", bytes.len());
            let _ = std::fs::remove_file(&path);
        }
    }

    /// The one format that keeps the renderer's own numbers has to actually
    /// keep them: a TIFF32 written through the tone map is a picture, which is
    /// what PNG is for.
    #[test]
    fn thirty_two_bit_tiff_keeps_scene_linear_values() {
        let directory = std::env::temp_dir().join("tequila-output-test");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("linear.tif");

        let mut source = output(AlphaMode::Premultiplied);
        // A value well past one, which only a float format can hold.
        source.color = vec![Spectrum::splat(12.0); 4];
        source.alpha = vec![1.0; 4];
        source.write(&path, ImageFormat::Tiff(Depth::ThirtyTwo)).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        let expected = Spectrum::splat(12.0).to_rendering_rgb().x.to_le_bytes();
        assert!(
            bytes.windows(4).any(|w| w == expected),
            "the scene-linear value is not in the file"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// JPEG cannot carry coverage, and knowing that before the render is what
    /// stops a shadow-catcher setup being written to a format that throws away
    /// the one thing it was set up for.
    #[test]
    fn formats_declare_what_they_can_carry() {
        assert!(!ImageFormat::Jpeg(90).carries_alpha());
        assert!(ImageFormat::Png.carries_alpha());
        assert!(ImageFormat::Exr(Precision::Half).is_high_dynamic_range());
        assert!(ImageFormat::Tiff(Depth::ThirtyTwo).is_high_dynamic_range());
        assert!(!ImageFormat::Tiff(Depth::Eight).is_high_dynamic_range());
        assert!(!ImageFormat::Png.is_high_dynamic_range());
    }
}
