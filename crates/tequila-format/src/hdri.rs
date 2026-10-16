//! Writing a composed environment out as a plain HDRI (`docs/03` §5).
//!
//! "Export to plain .hdr/.exr by rasterizing the composite at a chosen
//! resolution." The rasterizing is the environment cache's; this is only the
//! writing, chosen by the path's extension so a save dialog's filter is the
//! whole of the interface.
//!
//! The environment is held in the rendering space, ACEScg; the file is
//! written in linear sRGB, the primaries every HDRI is taken to be in, so
//! another application opening it — or this one, loading it back — reads
//! what this one lights with.

use crate::exr::{Channels, ExrImage, Layer, Precision};
use tequila_core::spectrum::Spectrum;
use tequila_texture::image::Image;
use std::path::Path;

/// The extensions [`export`] writes.
pub const EXTENSIONS: [&str; 2] = ["exr", "hdr"];

/// Write `image` to `path` as OpenEXR or Radiance, by its extension.
///
/// EXR is written at full 32-bit precision: half floats stop at 65504, and a
/// sun disc or a hot softbox is brighter than that often enough that clipping
/// it would change the lighting of anything that loads the file.
pub fn export(image: &Image, path: &Path) -> Result<(), String> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if image.width == 0 || image.height == 0 || image.channels == 0 {
        return Err("the environment is empty".into());
    }
    // Out of the rendering space. A colour outside sRGB's gamut comes out
    // with a negative component: EXR keeps it, so the colour survives a
    // round trip; Radiance's shared exponent cannot hold one, so there it
    // is clipped at zero.
    let clip = extension == "hdr";
    let mut image = image.clone();
    if image.channels >= 3 {
        let channels = image.channels;
        for pixel in image.data.chunks_exact_mut(channels) {
            let v = tequila_core::spectrum::acescg_to_srgb(tequila_core::math::vec3(pixel[0], pixel[1], pixel[2]));
            let keep = |c: f32| if clip { c.max(0.0) } else { c };
            pixel[0] = keep(v.x);
            pixel[1] = keep(v.y);
            pixel[2] = keep(v.z);
        }
    }
    let image = &image;
    match extension.as_str() {
        "exr" => {
            let channels = image.channels;
            let samples: Vec<Spectrum> = image
                .data
                .chunks(channels)
                .map(|texel| Spectrum {
                    r: texel[0],
                    g: texel[1.min(channels - 1)],
                    b: texel[2.min(channels - 1)],
                })
                .collect();
            let mut out = ExrImage::new(image.width, image.height);
            out.push(Layer {
                name: String::new(),
                channels: Channels::Color(samples),
                precision: Precision::Full,
            });
            out.attribute("tequilaEnvironment", "composed");
            out.write(path).map_err(|e| e.to_string())
        }
        "hdr" => image.save_radiance(path).map_err(|e| e.to_string()),
        other => Err(format!(
            "cannot write an environment as `.{other}`; use .exr or .hdr"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec3;

    /// Both formats read back through the loader environments are opened
    /// with, values beyond half-float range included, and into the rendering
    /// space the same colours they left it as. The file itself is sRGB.
    #[test]
    fn an_exported_environment_loads_back() {
        let directory = std::env::temp_dir().join(format!("tequila-hdri-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let mut image = Image::constant(32, 16, vec3(0.25, 0.5, 1.0));
        // A sun: far past what a half float holds, and white, as a sun is.
        image.data[0] = 200_000.0;
        image.data[1] = 200_000.0;
        image.data[2] = 200_000.0;

        for extension in EXTENSIONS {
            let path = directory.join(format!("sky.{extension}"));
            export(&image, &path).expect("it writes");
            let back = Image::load(&path).expect("it reads");
            assert_eq!((back.width, back.height), (32, 16));
            let in_file = back.texel(5, 5);
            let expected = tequila_core::spectrum::acescg_to_srgb(vec3(0.25, 0.5, 1.0));
            assert!((in_file - expected).length() < 0.01, ".{extension} holds {in_file:?}, not sRGB {expected:?}");
            let texel = tequila_core::spectrum::srgb_to_acescg(in_file);
            assert!(
                (texel - vec3(0.25, 0.5, 1.0)).length() < 0.01,
                ".{extension} read back {texel:?}"
            );
            let sun = tequila_core::spectrum::srgb_to_acescg(back.texel(0, 0)).x;
            assert!(
                (sun - 200_000.0).abs() < 200_000.0 * 0.01,
                ".{extension} clipped the sun to {sun}"
            );
        }
        assert!(export(&image, &directory.join("sky.png")).is_err());
        let _ = std::fs::remove_dir_all(&directory);
    }
}
