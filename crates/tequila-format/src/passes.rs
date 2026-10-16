//! Assembling a finished render into a multi-layer EXR (`docs/01` §12).
//!
//! The other half of M4's exit criterion. The passes are correct by the time
//! they reach here — `tequila-render-cpu`'s `tests/passes.rs` proves Beauty is
//! the sum of its components — and what is left is getting them into a file a
//! compositor reads without being told how.
//!
//! Almost all of that is naming and precision, and both have a right answer:
//!
//! - The beauty pass is bare `R`/`G`/`B`, because that is what a viewer shows
//!   when it opens a file. Every other pass is prefixed.
//! - Colour passes are half precision. They are going to be graded and looked
//!   at, and three decimal digits is more than a display has.
//! - Depth, Position and every Cryptomatte channel are full float. The first
//!   two are distances in scene units; the third are *bit patterns*, and a
//!   Cryptomatte identifier rounded to half names a different object.

use crate::exr::{Channels, ExrImage, Layer, Precision};
use tequila_render_core::aov::{clown_color, AovSet};
use tequila_render_core::cryptomatte::Cryptomatte;
use tequila_render_core::framebuffer::{AovBuffers, Framebuffer};
use tequila_core::spectrum::Spectrum;

/// Build the image to write from a finished render.
///
/// `aovs` must already have had `finish` called on it, which ranks the
/// Cryptomattes. Taking it by reference rather than doing the ranking here is
/// deliberate: ranking is destructive, and a caller that wanted to write two
/// files would otherwise rank twice and halve every coverage the second time.
pub fn exr_from_render(
    framebuffer: &Framebuffer,
    aovs: Option<&AovBuffers>,
    set: &AovSet,
) -> ExrImage {
    let (width, height) = (framebuffer.width, framebuffer.height);
    let mut image = ExrImage::new(width, height);
    let samples = framebuffer.min_samples();
    let pixels = width * height;
    let each = |f: &dyn Fn(usize, usize) -> Spectrum| -> Vec<Spectrum> {
        let mut out = Vec::with_capacity(pixels);
        for y in 0..height {
            for x in 0..width {
                out.push(f(x, y));
            }
        }
        out
    };

    // The beauty pass, unprefixed, so a viewer shows the render.
    image.push(Layer {
        name: String::new(),
        channels: Channels::Color(each(&|x, y| framebuffer.pixel(x, y).mean())),
        precision: Precision::Half,
    });
    // Alpha beside it, which is what makes a shadow catcher composite.
    image.push(Layer {
        name: "A".into(),
        channels: Channels::Scalar(
            (0..pixels)
                .map(|i| framebuffer.pixel(i % width, i / width).alpha())
                .collect(),
        ),
        precision: Precision::Half,
    });

    let Some(aovs) = aovs else {
        return image;
    };

    for (index, pass) in set.light_paths.iter().enumerate() {
        // The beauty pass is already written unprefixed; writing it twice would
        // double the file for no one's benefit.
        if pass.name == "Beauty" {
            continue;
        }
        image.push(Layer {
            name: pass.name.clone(),
            channels: Channels::Color(each(&|x, y| aovs.passes.mean(index, x, y, samples))),
            precision: Precision::Half,
        });
    }

    if set.data.depth {
        image.push(Layer {
            name: "Depth".into(),
            channels: Channels::Scalar(
                (0..pixels)
                    .map(|i| {
                        let depth = framebuffer.pixel(i % width, i / width).mean_depth();
                        // A ray that hit nothing is infinitely far away, and
                        // §12 says to store the true distance. Infinity is the
                        // true distance and every compositor handles it; a
                        // large finite stand-in would be a surface nothing put
                        // there.
                        if depth > 0.0 {
                            depth
                        } else {
                            f32::INFINITY
                        }
                    })
                    .collect(),
            ),
            precision: Precision::Full,
        });
    }
    if set.data.normal {
        image.push(Layer {
            name: "Normal".into(),
            channels: Channels::Vector(
                (0..pixels)
                    .map(|i| framebuffer.pixel(i % width, i / width).mean_normal())
                    .collect(),
            ),
            precision: Precision::Half,
        });
    }
    if set.data.position {
        image.push(Layer {
            name: "Position".into(),
            channels: Channels::Vector(
                (0..pixels)
                    .map(|i| aovs.mean_position(i % width, i / width, samples))
                    .collect(),
            ),
            precision: Precision::Full,
        });
    }
    if set.data.albedo {
        image.push(Layer {
            name: "Albedo".into(),
            channels: Channels::Color(each(&|x, y| framebuffer.pixel(x, y).mean_albedo())),
            precision: Precision::Half,
        });
    }
    if set.data.clown {
        image.push(Layer {
            name: "Clown".into(),
            channels: Channels::Color(
                (0..pixels)
                    .map(|i| {
                        let id = aovs.object_id[i];
                        if id == u32::MAX {
                            Spectrum::ZERO
                        } else {
                            clown_color(id)
                        }
                    })
                    .collect(),
            ),
            precision: Precision::Half,
        });
    }
    if set.data.shadow {
        image.push(Layer {
            name: "Shadow".into(),
            channels: Channels::Scalar(
                (0..pixels).map(|i| aovs.shadow(i % width, i / width)).collect(),
            ),
            precision: Precision::Half,
        });
    }
    if set.data.ambient_occlusion {
        image.push(Layer {
            name: "AmbientOcclusion".into(),
            channels: Channels::Scalar(
                (0..pixels)
                    .map(|i| aovs.mean_occlusion(i % width, i / width, samples))
                    .collect(),
            ),
            precision: Precision::Half,
        });
    }

    if set.data.labels {
        // The labels alone, premultiplied, with their coverage as alpha: laid
        // over the beauty in a compositor, or swapped for new artwork there.
        image.push(Layer {
            name: "Labels".into(),
            channels: Channels::Rgba(
                (0..pixels)
                    .map(|i| {
                        let (colour, alpha) = aovs.mean_labels(i % width, i / width, samples);
                        let rgb = colour.to_rendering_rgb();
                        [rgb.x, rgb.y, rgb.z, alpha]
                    })
                    .collect(),
            ),
            precision: Precision::Half,
        });
    }

    // Energy removed by firefly rejection, so the bias is auditable (§3.5).
    image.push(Layer {
        name: "ClampedEnergy".into(),
        channels: Channels::Scalar(
            (0..pixels)
                .map(|i| framebuffer.pixel(i % width, i / width).mean_clamped_energy())
                .collect(),
        ),
        precision: Precision::Half,
    });

    if let Some(objects) = &aovs.objects {
        write_cryptomatte(&mut image, objects);
    }
    if let Some(materials) = &aovs.materials {
        write_cryptomatte(&mut image, materials);
    }

    image
}

/// One Cryptomatte: its ranked layers, and the four metadata attributes the
/// standard requires.
fn write_cryptomatte(image: &mut ExrImage, matte: &Cryptomatte) {
    let key = matte.metadata_key();
    image
        .attribute(format!("cryptomatte/{key}/name"), matte.name.clone())
        .attribute(format!("cryptomatte/{key}/hash"), "MurmurHash3_32")
        .attribute(format!("cryptomatte/{key}/conversion"), "uint32_to_float32")
        .attribute(format!("cryptomatte/{key}/manifest"), matte.manifest_json());

    for layer in 0..matte.layer_count() {
        let mut samples = Vec::with_capacity(matte.width * matte.height);
        for y in 0..matte.height {
            for x in 0..matte.width {
                samples.push(matte.layer_at(layer, x, y));
            }
        }
        image.push(Layer {
            // `CryptoObject00`, `CryptoObject01`, `CryptoObject02` — the names
            // a compositor looks for, zero-padded to two digits because that is
            // what the standard writes.
            name: format!("{}{:02}", matte.name, layer),
            channels: Channels::Rgba(samples),
            precision: Precision::Full,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_render_core::aov::{AovSet, DataPasses};

    fn render() -> (Framebuffer, AovBuffers, AovSet) {
        let set = AovSet {
            light_paths: tequila_render_core::aov::standard_passes(),
            data: DataPasses {
                depth: true,
                normal: true,
                position: true,
                albedo: true,
                clown: true,
                ambient_occlusion: true,
                shadow: true,
                labels: true,
            },
            cryptomatte: true,
        };
        let mut framebuffer = Framebuffer::new(4, 4);
        let mut aovs = AovBuffers::new(4, 4, &set);
        aovs.name_things(
            &["Housing".to_string(), "Lens".to_string()],
            &["Anodized".to_string()],
        );

        // One sample of something, so the buffers are not all zero.
        let result = tequila_render_core::integrator::PathResult {
            radiance: Spectrum::splat(0.5),
            depth: 3.0,
            coverage: 1.0,
            object_id: 0,
            material_id: 0,
            labels: Spectrum::splat(0.25),
            labels_alpha: 0.5,
            ..Default::default()
        };
        let passes = tequila_render_core::aov::PathPasses::new(set.light_paths.len());
        let housing = aovs.object_id_of(0).expect("a named object");
        for y in 0..4 {
            for x in 0..4 {
                framebuffer.pixel_mut(x, y).accumulate(&result, false);
                aovs.accumulate(x, y, &result, &passes);
                if let Some(matte) = &mut aovs.objects {
                    matte.accumulate(x, y, housing, 1.0);
                }
            }
        }
        aovs.finish(framebuffer.min_samples());
        (framebuffer, aovs, set)
    }

    /// The layers a compositor is going to look for, by the names it looks for
    /// them under. A pass that is present under the wrong name is a pass the
    /// user cannot find, which is the same as one that is missing.
    #[test]
    fn every_requested_pass_reaches_the_file() {
        let (framebuffer, aovs, set) = render();
        let image = exr_from_render(&framebuffer, Some(&aovs), &set);
        let channels = image.channels().unwrap();
        let names: Vec<&str> = channels.iter().map(|(n, _, _)| n.as_str()).collect();

        for expected in [
            "R",
            "G",
            "B",
            "A",
            "Diffuse.R",
            "Reflection.R",
            "Refraction.R",
            "Caustics.R",
            "Lighting.R",
            "GlobalIllumination.R",
            "Depth",
            "Normal.X",
            "Position.X",
            "Albedo.R",
            "Clown.R",
            "Shadow",
            "AmbientOcclusion",
            "Labels.R",
            "Labels.A",
            "ClampedEnergy",
            "CryptoObject00.R",
            "CryptoObject00.A",
            "CryptoObject02.A",
            "CryptoMaterial00.R",
        ] {
            assert!(names.contains(&expected), "`{expected}` is missing: {names:?}");
        }
        // And the beauty pass is not written twice under two names.
        assert!(!names.contains(&"Beauty.R"), "{names:?}");
    }

    /// §12 asks for depth as a true distance. A ray that hit nothing is
    /// infinitely far away, and clamping it to a large number would put a
    /// surface in the depth pass that is not in the render.
    /// The Labels pass carries the labels' colour and their coverage as alpha.
    #[test]
    fn the_labels_pass_is_colour_with_alpha() {
        let (framebuffer, aovs, set) = render();
        let image = exr_from_render(&framebuffer, Some(&aovs), &set);
        let channels = image.channels().unwrap();
        let channel = |name: &str| {
            channels.iter().find(|(n, _, _)| n == name).map(|(_, _, v)| v[5]).expect(name)
        };
        assert!((channel("Labels.A") - 0.5).abs() < 1e-3, "{}", channel("Labels.A"));
        let red = Spectrum::splat(0.25).to_rendering_rgb().x;
        assert!((channel("Labels.R") - red).abs() < 1e-3, "{}", channel("Labels.R"));
    }

    #[test]
    fn depth_is_a_true_distance_and_infinite_where_nothing_was_hit() {
        let set = AovSet::standard();
        let framebuffer = Framebuffer::new(2, 1);
        let aovs = AovBuffers::new(2, 1, &set);
        let image = exr_from_render(&framebuffer, Some(&aovs), &set);
        let channels = image.channels().unwrap();
        let depth = channels
            .iter()
            .find(|(name, _, _)| name == "Depth")
            .expect("a depth channel");
        assert!(depth.2.iter().all(|v| v.is_infinite()));
        assert_eq!(depth.1, Precision::Full);
    }

    /// The four attributes the standard requires, keyed by the hash of the
    /// layer's name. Missing any one of them leaves a file whose channels are
    /// right and which no compositor will offer a matte list for.
    #[test]
    fn the_cryptomatte_metadata_is_complete() {
        let (framebuffer, aovs, set) = render();
        let image = exr_from_render(&framebuffer, Some(&aovs), &set);
        let key = aovs.objects.as_ref().unwrap().metadata_key();

        let get = |suffix: &str| {
            image
                .attributes
                .iter()
                .find(|(name, _)| *name == format!("cryptomatte/{key}/{suffix}"))
                .map(|(_, value)| value.as_str())
        };
        assert_eq!(get("name"), Some("CryptoObject"));
        assert_eq!(get("hash"), Some("MurmurHash3_32"));
        assert_eq!(get("conversion"), Some("uint32_to_float32"));
        let manifest = get("manifest").expect("a manifest");
        assert!(manifest.contains("Housing"), "{manifest}");
        assert!(manifest.contains("Lens"), "{manifest}");

        // Two Cryptomattes, two sets of attributes, under different keys.
        let materials = aovs.materials.as_ref().unwrap().metadata_key();
        assert_ne!(key, materials);
        assert!(image
            .attributes
            .iter()
            .any(|(name, _)| name == &format!("cryptomatte/{materials}/manifest")));
    }

    /// A Cryptomatte identifier is a bit pattern. Rounding one to half
    /// precision names a different object, so those channels have to be full
    /// float — and so do depth and position, which are distances.
    #[test]
    fn precision_follows_what_a_channel_means() {
        let (framebuffer, aovs, set) = render();
        let image = exr_from_render(&framebuffer, Some(&aovs), &set);
        for (name, precision, _) in image.channels().unwrap() {
            let full = name.starts_with("Crypto")
                || name == "Depth"
                || name.starts_with("Position.");
            assert_eq!(
                precision == Precision::Full,
                full,
                "`{name}` is {precision:?}"
            );
        }
    }
}
