//! The material lab: tools for making the library's materials look real,
//! run by hand (`cargo test -p tequila-app --release material_lab --
//! --ignored`), each driven by environment variables.
//!
//! - [`reference_board`]: real texture sets and library presets rendered
//!   side by side, on the same ball, in the same light, at the same scale:
//!   the comparison a material has to survive.
//! - [`reference_palette`]: the colours a photographed material is made of,
//!   as quantiles of its colour map in sRGB hex, to build a procedural's
//!   ramps from measured colour rather than guessed.
//!
//! The reference sets are CC0 scans (Poly Haven, ambientCG), downloaded by
//! hand for comparison; nothing of theirs ships.

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    fn var(name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|v| !v.trim().is_empty())
    }

    /// A tile's RGBA, rendered from a library entry.
    fn tile(entry: &tequila_format::library::LibraryEntry, px: usize, samples: u32, shape: tequila_render_cpu::PreviewShape) -> Vec<u8> {
        let device = tequila_render_cpu::Device::shared().expect("a device");
        let settings = tequila_render_cpu::PreviewSettings { size: px, samples, shape, ..tequila_render_cpu::PreviewSettings::default() };
        let thumbnail = tequila_render_cpu::render_thumbnail_textured(
            &device,
            &entry.graph,
            crate::library_tab::picture_table(&entry.textures),
            Arc::new(tequila_scene::demo::preview_environment()),
            settings,
        )
        .expect("renders");
        let rgba = crate::thumbnail::develop_rgba(&thumbnail);
        // Transmissive materials render at twice the size: every other pixel.
        let step = (thumbnail.width / px).max(1);
        let mut out = Vec::with_capacity(px * px * 4);
        for y in 0..px {
            for x in 0..px {
                let i = ((y * step) * thumbnail.width + x * step) * 4;
                out.extend_from_slice(&rgba[i..i + 4]);
            }
        }
        out
    }

    /// A texture set as a library entry, `size` metres to a repeat, the way
    /// the application loads a dropped pack.
    fn pack_entry(folder: &Path, size: f32) -> tequila_format::library::LibraryEntry {
        use tequila_format::library::{read_directory_from, save_entry, LibraryEntry, Source};
        let mut app = crate::TequilaApp::for_tests();
        let part = app.project.document().tree.iter().find(|n| n.material.is_some()).map(|n| n.id).unwrap();
        let loaded = crate::texture_pack::load(&mut app.project, folder, vec![part]).expect("loads");
        let mut definition = app.project.document().materials.get(loaded.report.materials[0].0).unwrap().clone();
        let ids: Vec<_> = definition.graph.nodes().map(|(id, _)| id).collect();
        for id in ids {
            if let Some(tequila_materials::graph::NodeKind::Mapping2d { mapping }) = definition.graph.node_mut(id).map(|n| &mut n.kind) {
                mapping.scale.x = size;
                mapping.scale.y = size;
            }
        }
        let entry = LibraryEntry::from_graph_with_pictures(
            definition.name.clone(),
            "Reference".into(),
            definition.material_type,
            &definition.graph,
            &app.project.document().textures,
            Source::User,
        );
        let library = std::env::temp_dir().join(format!("tequila-lab-{}-{}", std::process::id(), folder.file_name().unwrap().to_string_lossy()));
        let _ = std::fs::remove_dir_all(&library);
        save_entry(&library, &entry).expect("saves");
        let (mut entries, _) = read_directory_from(&library, Source::User);
        // The pictures are read from the copies when the tile renders, so
        // the folder stays until the process ends.
        entries.remove(0)
    }

    fn write_png(path: &Path, width: usize, height: usize, rgba: &[u8]) {
        let file = std::fs::File::create(path).expect("writable");
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(rgba).unwrap();
    }

    /// References over presets, side by side.
    ///
    /// - `TEQUILA_LAB_PACKS`: texture-set folders, `;` apart. A folder's
    ///   `size_m.txt`, if it has one, is how many metres one repeat covers.
    /// - `TEQUILA_LAB_PRESETS`: library preset names, `|` apart, exact.
    /// - `TEQUILA_LAB_OUT`: the PNG. `TEQUILA_LAB_PX` (360),
    ///   `TEQUILA_LAB_SPP` (96), `TEQUILA_LAB_SHAPE` (`sphere` or `slab`),
    ///   `TEQUILA_LAB_COLUMNS` (presets a row; all in one by default),
    ///   `TEQUILA_LAB_LIBRARY` (another folder of presets).
    #[test]
    #[ignore = "a comparison to look at; run by hand"]
    fn reference_board() {
        use rayon::prelude::*;
        let Some(out) = var("TEQUILA_LAB_OUT") else { return };
        let px: usize = var("TEQUILA_LAB_PX").and_then(|s| s.parse().ok()).unwrap_or(360);
        let samples: u32 = var("TEQUILA_LAB_SPP").and_then(|s| s.parse().ok()).unwrap_or(96);
        let shape = match var("TEQUILA_LAB_SHAPE").as_deref() {
            Some("slab") => tequila_render_cpu::PreviewShape::Slab,
            _ => tequila_render_cpu::PreviewShape::Sphere,
        };
        let packs: Vec<PathBuf> = var("TEQUILA_LAB_PACKS").map(|s| s.split(';').map(|p| PathBuf::from(p.trim())).collect()).unwrap_or_default();
        let names: Vec<String> = var("TEQUILA_LAB_PRESETS").map(|s| s.split('|').map(|n| n.trim().to_string()).collect()).unwrap_or_default();

        let mut rows: Vec<Vec<Vec<u8>>> = Vec::new();
        if !packs.is_empty() {
            let entries: Vec<_> = packs
                .iter()
                .map(|folder| {
                    let size = std::fs::read_to_string(folder.join("size_m.txt")).ok().and_then(|s| s.trim().parse::<f32>().ok()).unwrap_or(1.0);
                    pack_entry(folder, size)
                })
                .collect();
            rows.push(entries.par_iter().map(|e| tile(e, px, samples, shape)).collect());
        }
        if !names.is_empty() {
            let folder = var("TEQUILA_LAB_LIBRARY").map(PathBuf::from).unwrap_or_else(crate::library_tab::standard_library_path);
            let (entries, errors) = tequila_format::library::read_directory(&folder);
            assert!(errors.is_empty(), "{errors:?}");
            let chosen: Vec<_> = names
                .iter()
                .map(|n| entries.iter().find(|e| &e.name == n).unwrap_or_else(|| panic!("no preset called {n:?}")).clone())
                .collect();
            let tiles: Vec<Vec<u8>> = chosen.par_iter().map(|e| tile(e, px, samples, shape)).collect();
            // TEQUILA_LAB_COLUMNS wraps a long list into rows.
            let wrap = var("TEQUILA_LAB_COLUMNS").and_then(|s| s.parse::<usize>().ok()).filter(|&n| n > 0).unwrap_or(tiles.len().max(1));
            for chunk in tiles.chunks(wrap) {
                rows.push(chunk.to_vec());
            }
        }
        let columns = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        let gap = 4;
        let width = columns * (px + gap) + gap;
        let height = rows.len() * (px + gap) + gap;
        let mut image = vec![40u8; width * height * 4];
        for chunk in image.chunks_exact_mut(4) {
            chunk[3] = 255;
        }
        for (r, row) in rows.iter().enumerate() {
            for (c, tile) in row.iter().enumerate() {
                let (x0, y0) = (gap + c * (px + gap), gap + r * (px + gap));
                for y in 0..px {
                    let dst = ((y0 + y) * width + x0) * 4;
                    image[dst..dst + px * 4].copy_from_slice(&tile[y * px * 4..(y + 1) * px * 4]);
                }
            }
        }
        write_png(Path::new(&out), width, height, &image);
        println!("wrote {out}: {} references over {} presets", packs.len(), names.len());
    }

    /// The colours of a photographed material: its colour map's pixels
    /// sorted by lightness, read at quantiles, in sRGB hex; their mean and
    /// spread; and the same along and across the image, to tell how much of
    /// the variation is grain. `TEQUILA_LAB_IMAGE` is the map.
    #[test]
    #[ignore = "a measurement to read; run by hand"]
    fn reference_palette() {
        let Some(path) = var("TEQUILA_LAB_IMAGE") else { return };
        let image = tequila_texture::image::Image::load(&path).expect("an image");
        let to_srgb = |v: f32| {
            let v = v.clamp(0.0, 1.0);
            let s = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
            (s * 255.0).round() as u8
        };
        let hex = |c: tequila_core::math::Vec3| format!("#{:02X}{:02X}{:02X}", to_srgb(c.x), to_srgb(c.y), to_srgb(c.z));
        let luma = |c: tequila_core::math::Vec3| 0.2126 * c.x + 0.7152 * c.y + 0.0722 * c.z;
        let mut texels = Vec::with_capacity(image.width * image.height);
        // Every other texel is plenty, and keeps a 4K map quick.
        for y in (0..image.height).step_by(2) {
            for x in (0..image.width).step_by(2) {
                texels.push(image.texel(x, y));
            }
        }
        let n = texels.len() as f32;
        let mean = texels.iter().fold(tequila_core::math::Vec3::ZERO, |a, c| a + *c) / n;
        let mean_l = texels.iter().map(|c| luma(*c)).sum::<f32>() / n;
        let spread = (texels.iter().map(|c| (luma(*c) - mean_l).powi(2)).sum::<f32>() / n).sqrt();
        texels.sort_by(|a, b| luma(*a).total_cmp(&luma(*b)));
        println!("{path}");
        println!("  mean {}  lightness {:.3} +- {:.3} (linear)", hex(mean), mean_l, spread);
        for q in [0.02, 0.1, 0.25, 0.5, 0.75, 0.9, 0.98] {
            // A small window around the quantile, averaged, so one odd
            // texel does not stand for it.
            let i = ((n - 1.0) * q) as usize;
            let lo = i.saturating_sub(20);
            let hi = (i + 20).min(texels.len() - 1);
            let avg = texels[lo..=hi].iter().fold(tequila_core::math::Vec3::ZERO, |a, c| a + *c) / (hi - lo + 1) as f32;
            println!("  q{:>4.0}% {}", q * 100.0, hex(avg));
        }
        // How fast lightness changes along each axis: grain runs along the
        // axis it changes slowly on.
        let step = |dx: usize, dy: usize| {
            let mut total = 0.0;
            let mut count = 0;
            for y in (0..image.height - dy).step_by(3) {
                for x in (0..image.width - dx).step_by(3) {
                    total += (luma(image.texel(x + dx, y + dy)) - luma(image.texel(x, y))).abs();
                    count += 1;
                }
            }
            total / count as f32
        };
        println!("  neighbour change: along x {:.4}, along y {:.4}", step(1, 0), step(0, 1));
    }
}
