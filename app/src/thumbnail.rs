//! A rendered material thumbnail, made into a picture (`docs/04` §9).
//!
//! One place for the library's tiles and the Material tab's sphere, so a
//! material looks the same on the shelf as in the panel.

use eframe::egui;
use tequila_render_cpu::Thumbnail;

/// The thumbnail as a texture.
pub fn develop(thumbnail: &Thumbnail) -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied(
        [thumbnail.width, thumbnail.height],
        &develop_rgba(thumbnail),
    )
}

/// Pixels a small preview is rendered at per point it is shown at: two on a
/// Retina screen, or any as dense, where a preview of one showed each of its
/// pixels as four and every sphere on the shelf came out soft; one elsewhere.
pub fn density(ctx: &egui::Context) -> usize {
    if ctx.pixels_per_point() > 1.25 {
        2
    } else {
        1
    }
}

/// A thumbnail's key in the disk cache at `density`: the key itself at one,
/// so the thumbnails already kept stay kept.
pub fn dense_key(key: u64, density: usize) -> u64 {
    match density {
        1 => key,
        n => key ^ (0x9e37_79b9_7f4a_7c15u64.wrapping_mul(n as u64)),
    }
}

/// A library tile's key in the disk cache when it is `pixels` across: those
/// of 96 and 192 under the keys a tile of density one and two always had, so
/// the thumbnails already kept stay kept.
pub fn sized_key(key: u64, pixels: usize) -> u64 {
    if pixels % 96 == 0 {
        dense_key(key, pixels / 96)
    } else {
        key ^ (0xc2b2_ae3d_27d4_eb4fu64.wrapping_mul(pixels as u64))
    }
}

/// RGBA8, row-major from the top-left.
///
/// Denoised, because a thumbnail's sample count leaves glass and clear
/// plastic speckled however it is lit, and developed through the same
/// display transform as a final render, so a colour on the shelf is the
/// colour it renders.
pub fn develop_rgba(thumbnail: &Thumbnail) -> Vec<u8> {
    use tequila_core::spectrum::Spectrum;
    let (width, height) = (thumbnail.width, thumbnail.height);
    let denoised = tequila_denoise::Denoiser::shared().ok().and_then(|denoiser| {
        denoiser
            .denoise(
                width,
                height,
                &thumbnail.pixels,
                &thumbnail.albedo,
                &thumbnail.normal,
                tequila_denoise::DenoiseSettings {
                    quality: tequila_denoise::Quality::High,
                    strength: 1.0,
                    prefilter_guides: true,
                },
            )
            .ok()
    });
    let colour: Vec<Spectrum> = match &denoised {
        Some(image) => image
            .data
            .chunks_exact(3)
            .map(|c| Spectrum::from_rendering_rgb(tequila_core::math::vec3(c[0], c[1], c[2])))
            .collect(),
        None => (0..width * height).map(|i| thumbnail.pixel(i % width, i / width)).collect(),
    };
    tequila_color::ImageStyle::default().develop(width, height, &colour)
}

/// Developed thumbnails kept on disk between launches (§9's "cached ... keyed
/// by content hash"), so the library is rendered once rather than every time
/// the application opens.
///
/// Kept across builds, in a folder named after [`THUMBNAIL_LOOK`]. It was a
/// folder per build, cleared by the next one, on the grounds that a
/// thumbnail is only as right as the renderer that made it. But then every
/// update path traced the whole library again — three hundred spheres, the
/// best part of an hour of a core — and the shelf was grey and the machine
/// busy each time a new version opened. A tile's key is its material's
/// content and its pictures' files, so a material that changes is rendered
/// again whatever the folder; what a build can change without the material
/// changing is the look, and that is what the number is for.
pub struct DiskCache {
    dir: Option<std::path::PathBuf>,
}

/// Which look the kept thumbnails have. Raise it with any change that shows
/// on the shelf without changing a material — the renderer's shading, the
/// preview scene or its lighting, the thumbnail's size or samples, the
/// denoising or the developing — and the next launch clears the old ones and
/// renders the library afresh. Leave it alone otherwise: raising it costs
/// everyone that hour of rendering.
pub const THUMBNAIL_LOOK: u32 = 1;

impl DiskCache {
    /// The cache, under the per-user cache directory.
    pub fn persistent() -> DiskCache {
        // A test binary is a build of its own, and opening its cache would
        // clear the application's; tests keep theirs apart.
        #[cfg(test)]
        let root = std::env::temp_dir().join("tequila-test-thumbnails");
        #[cfg(not(test))]
        let root = tequila_format::library::cache_directory().join("thumbnails");
        let look = format!("look-{THUMBNAIL_LOOK}");
        adopt_last_build(&root, &look);
        DiskCache::in_folder(&root, &look)
    }

    /// The cache for the look `look`, under `root`, clearing any other
    /// look's beside it.
    pub fn in_folder(root: &std::path::Path, look: &str) -> DiskCache {
        if let Ok(others) = std::fs::read_dir(root) {
            for other in others.flatten() {
                if other.file_name() != look {
                    let _ = std::fs::remove_dir_all(other.path());
                }
            }
        }
        let dir = root.join(look);
        DiskCache { dir: std::fs::create_dir_all(&dir).ok().map(|_| dir) }
    }

    fn path(&self, key: u64) -> Option<std::path::PathBuf> {
        self.dir.as_ref().map(|dir| dir.join(format!("{key:016x}.png")))
    }

    /// A kept thumbnail, if this build made one for `key`.
    pub fn load(&self, key: u64) -> Option<egui::ColorImage> {
        let file = std::fs::File::open(self.path(key)?).ok()?;
        let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
        decoder.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = decoder.read_info().ok()?;
        let mut rgba = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut rgba).ok()?;
        if info.color_type != png::ColorType::Rgba {
            return None;
        }
        rgba.truncate(info.buffer_size());
        Some(egui::ColorImage::from_rgba_unmultiplied([info.width as usize, info.height as usize], &rgba))
    }

    /// Keep a developed thumbnail. Best effort: a cache that cannot be
    /// written only means rendering again next time. Written beside its name
    /// and renamed into place, so a launch that dies mid-write leaves nothing
    /// half a picture.
    pub fn store(&self, key: u64, width: usize, height: usize, rgba: &[u8]) {
        let Some(path) = self.path(key) else {
            return;
        };
        let partial = path.with_extension("partial");
        let written = (|| -> Option<()> {
            let file = std::fs::File::create(&partial).ok()?;
            let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width as u32, height as u32);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().ok()?.write_image_data(rgba).ok()
        })();
        if written.is_some() {
            let _ = std::fs::rename(&partial, &path);
        } else {
            let _ = std::fs::remove_file(&partial);
        }
    }
}

/// Builds before the cache was kept across them left their thumbnails in a
/// folder named after the build: the executable's size and time, two hex
/// numbers. The newest of those was made by the renderer [`THUMBNAIL_LOOK`]
/// 1 describes, so it becomes that look's folder rather than the first
/// launch of this build rendering the library once more.
fn adopt_last_build(root: &std::path::Path, look: &str) {
    if THUMBNAIL_LOOK != 1 || root.join(look).exists() {
        return;
    }
    let is_build = |name: &str| {
        name.split_once('-')
            .is_some_and(|(a, b)| [a, b].iter().all(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_hexdigit())))
    };
    let newest = std::fs::read_dir(root).into_iter().flatten().flatten()
        .filter(|folder| folder.file_name().to_str().is_some_and(is_build))
        .filter_map(|folder| Some((folder.metadata().ok()?.modified().ok()?, folder.path())))
        .max_by_key(|(modified, _)| *modified);
    if let Some((_, folder)) = newest {
        let _ = std::fs::rename(folder, root.join(look));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("tequila-thumbnail-cache-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// What is stored is what comes back, and only for its own key.
    #[test]
    fn a_kept_thumbnail_comes_back_as_it_was() {
        let root = scratch("round-trip");
        let cache = DiskCache::in_folder(&root, "build-a");
        let rgba: Vec<u8> = (0..4 * 4 * 4).map(|i| (i * 7 % 256) as u8).map(|v| v | 1).collect();
        let opaque: Vec<u8> = rgba.chunks(4).flat_map(|p| [p[0], p[1], p[2], 255]).collect();
        cache.store(42, 4, 4, &opaque);
        let back = cache.load(42).expect("the thumbnail was kept");
        assert_eq!(back.size, [4, 4]);
        assert_eq!(back, egui::ColorImage::from_rgba_unmultiplied([4, 4], &opaque));
        assert!(cache.load(43).is_none(), "another key has nothing kept");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A new look does not show the old one's thumbnails, and clears them.
    #[test]
    fn a_new_look_starts_afresh() {
        let root = scratch("looks");
        let old = DiskCache::in_folder(&root, "look-1");
        old.store(7, 1, 1, &[10, 20, 30, 255]);
        assert!(old.load(7).is_some());
        let new = DiskCache::in_folder(&root, "look-2");
        assert!(new.load(7).is_none(), "the new look showed the old look's thumbnail");
        assert!(!root.join("look-1").exists(), "the old look's thumbnails were left behind");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The thumbnails the last build before this cache made are kept, not
    /// rendered again: those of the newest build, and only those.
    #[test]
    fn the_last_builds_thumbnails_are_kept() {
        let root = scratch("adopt");
        let older = DiskCache::in_folder(&root, "1592a00-18dd95f0201bedf0");
        older.store(1, 1, 1, &[1, 1, 1, 255]);
        std::thread::sleep(std::time::Duration::from_millis(50));
        // Made directly, as in_folder would clear the older one.
        let newer = root.join("15a3f00-18dd9d1e0c7a2b40");
        std::fs::create_dir_all(&newer).unwrap();
        DiskCache { dir: Some(newer.clone()) }.store(2, 1, 1, &[2, 2, 2, 255]);
        std::fs::create_dir_all(root.join("not-a-build")).unwrap();

        adopt_last_build(&root, "look-1");
        let cache = DiskCache::in_folder(&root, "look-1");
        assert!(cache.load(2).is_some(), "the newest build's thumbnail was rendered again");
        assert!(cache.load(1).is_none(), "an older build's thumbnail was kept");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1, "the other folders were left behind");

        // Once the look has its folder, nothing more is adopted into it.
        std::fs::create_dir_all(root.join("16b0000-18dda00000000000")).unwrap();
        DiskCache { dir: Some(root.join("16b0000-18dda00000000000")) }.store(3, 1, 1, &[3, 3, 3, 255]);
        adopt_last_build(&root, "look-1");
        let cache = DiskCache::in_folder(&root, "look-1");
        assert!(cache.load(3).is_none() && cache.load(2).is_some());
        let _ = std::fs::remove_dir_all(&root);
    }
    use tequila_render_cpu::{render_thumbnail_graph, Device};
    use std::sync::Arc;

    /// How speckled the middle of a thumbnail is: the mean difference between
    /// each pixel and its four neighbours, in eight-bit levels. Refraction
    /// has edges, but few; noise is everywhere.
    fn speckle(rgba: &[u8], size: usize) -> f32 {
        let lum = |x: usize, y: usize| {
            let at = (y * size + x) * 4;
            (rgba[at] as f32 + rgba[at + 1] as f32 + rgba[at + 2] as f32) / 3.0
        };
        let (from, to) = (size * 3 / 10, size * 6 / 10);
        let mut total = 0.0;
        let mut n = 0.0;
        for y in from..to {
            for x in from..to {
                let around = (lum(x - 1, y) + lum(x + 1, y) + lum(x, y - 1) + lum(x, y + 1)) / 4.0;
                total += (lum(x, y) - around).abs();
                n += 1.0;
            }
        }
        total / n
    }

    /// Nothing on the shelf previews as a field of specks. Glass and clear
    /// or smoked plastic are the hard ones — a few paths through two
    /// refractions carry all of it — and rendered at a thumbnail's sample
    /// count and shown as they came out they were: 7 to 22 levels from pixel
    /// to pixel in the middle of the sphere, where an opaque material is 2
    /// to 4.
    ///
    /// Measured on each preset's material without its patterns: a polished
    /// granite or a speckled glaze is meant to change from pixel to pixel,
    /// and that is the picture, not noise in it.
    #[test]
    fn no_library_thumbnail_is_speckled() {
        let device = Device::shared().expect("an Embree device");
        let (entries, _) = tequila_format::library::read_directory_from(
            &crate::library_tab::standard_library_path(),
            tequila_format::library::Source::Standard,
        );
        assert!(entries.len() > 40, "the standard library did not load");
        let environment = Arc::new(tequila_scene::demo::preview_environment());
        let settings = crate::library_tab::thumbnail_settings_at(1);
        let mut worst = (0.0f32, String::new());
        for entry in &entries {
            // Glitter is a pattern too: flakes a few tenths of a millimetre
            // across, sparse enough that a pixel catches none, one or two,
            // sparkle from pixel to pixel because that is what glitter does.
            // Metallic paint's fine, dense flakes average out and are
            // measured as they are.
            let mut material = entry.material.clone();
            if material.flake_size >= 1e-4 && material.flake_weight < 0.3 {
                material.flake_weight = 0.0;
            }
            let plain = tequila_materials::graph::MaterialGraph::from_material(material);
            let thumbnail = render_thumbnail_graph(&device, &plain, environment.clone(), settings)
                .expect("renders");
            let noise = speckle(&develop_rgba(&thumbnail), thumbnail.width);
            if noise > worst.0 {
                worst = (noise, entry.name.clone());
            }
        }
        println!("worst: {} at {:.2}", worst.1, worst.0);
        assert!(worst.0 < 4.5, "`{}` previews speckled: {:.2} levels from pixel to pixel", worst.1, worst.0);
    }
}
