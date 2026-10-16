//! M4's exit criterion, end to end:
//!
//! > render a 4K image with 12 passes; open the EXR in a compositor and confirm
//! > Cryptomatte mattes select correctly and that Beauty = sum of its component
//! > passes within floating-point tolerance.
//!
//! "Open it in a compositor" is the part a test cannot do, and pretending
//! otherwise would be worse than saying so. What it does instead is open the
//! file with an independent, complete implementation of the format — the same
//! one a compositor's reader is standing in for — and then do exactly what a
//! compositor's Cryptomatte node does: read the manifest, hash a name, walk the
//! ranked layers, and gather coverage. If that produces the right matte, a
//! compositor will too, because that procedure *is* the format.
//!
//! The resolution is the other compromise, and it is only in the default test.
//! `the_full_4k_render` does the criterion's own 4K at twelve passes; it is
//! `#[ignore]`d because it needs about two gigabytes and half a minute, which
//! is not something to put in the path of every `cargo test`. It is run by
//! hand, and what it produced is recorded in `docs/05`.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Transform, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_format::exr::Precision;
use tequila_format::passes::exr_from_render;
use tequila_materials::material_type::MaterialType;
use tequila_materials::openpbr::Material;
use tequila_render_core::aov::{AovSet, LightPathPass};
use tequila_render_core::camera::Camera;
use tequila_render_core::cryptomatte::{murmur3_32, name_to_float};
use tequila_render_core::environment::Environment;
use tequila_render_core::framebuffer::{AovBuffers, Framebuffer};
use tequila_render_core::scene::{Instance, Mesh, SceneSnapshot};
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_cpu::{render_pass_with_aovs, Device, EmbreeIntersector, RenderContext};
use std::sync::Arc;

fn sphere(radius: f32) -> Mesh {
    let (rings, segments) = (24usize, 32usize);
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    for ring in 0..=rings {
        let theta = ring as f32 / rings as f32 * std::f32::consts::PI;
        for segment in 0..=segments {
            let phi = segment as f32 / segments as f32 * std::f32::consts::TAU;
            let n = vec3(theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin());
            positions.push(n * radius);
            normals.push(n);
        }
    }
    let mut indices = Vec::new();
    for ring in 0..rings {
        for segment in 0..segments {
            let a = (ring * (segments + 1) + segment) as u32;
            let b = a + segments as u32 + 1;
            indices.extend_from_slice(&[a, a + 1, b, a + 1, b + 1, b]);
        }
    }
    Mesh::new(positions, indices).with_normals(normals)
}

fn quad(half: f32, y: f32, up: bool) -> Mesh {
    let n = if up { Vec3::Y } else { -Vec3::Y };
    let indices = if up {
        vec![0, 2, 1, 0, 3, 2]
    } else {
        vec![0, 1, 2, 0, 2, 3]
    };
    Mesh::new(
        vec![
            vec3(-half, y, -half),
            vec3(half, y, -half),
            vec3(half, y, half),
            vec3(-half, y, half),
        ],
        indices,
    )
    .with_normals(vec![n; 4])
}

/// The names the Cryptomatte manifest will carry, in instance order.
const OBJECTS: [&str; 4] = ["Floor", "Mirror ball", "Glass ball", "Softbox"];
const MATERIALS: [&str; 4] = ["Matte", "Chrome", "Glass", "Emitter"];

fn scene() -> SceneSnapshot {
    let matte = Material {
        base_color: Spectrum::splat(0.6),
        ..MaterialType::Diffuse.defaults()
    };
    let mirror = Material {
        specular_roughness: 0.0,
        ..MaterialType::Metal.defaults()
    };
    let glass = Material {
        specular_roughness: 0.0,
        ..MaterialType::GlassSolid.defaults()
    };
    let emitter = Material {
        emission_color: Spectrum::ONE,
        emission_luminance: 30.0,
        ..MaterialType::Emissive.defaults()
    };
    let ball = Arc::new(sphere(0.5));
    SceneSnapshot::new(
        vec![
            Arc::new(quad(4.0, 0.0, true)),
            ball.clone(),
            ball,
            Arc::new(quad(1.2, 3.0, false)),
        ],
        vec![
            Instance::new(InstanceId(0), MeshId(0), MaterialId(0)),
            Instance::new(InstanceId(1), MeshId(1), MaterialId(1))
                .with_transform(Transform::translation(vec3(-0.8, 0.5, 0.0))),
            Instance::new(InstanceId(2), MeshId(2), MaterialId(2))
                .with_transform(Transform::translation(vec3(0.8, 0.5, 0.0))),
            Instance::new(InstanceId(3), MeshId(3), MaterialId(3)),
        ],
        vec![matte, mirror, glass, emitter],
        Arc::new(Environment::uniform(Spectrum::ZERO)),
    )
}

/// The twelve passes the criterion asks for.
fn twelve_passes() -> AovSet {
    let mut set = AovSet::standard();
    // Seven light-path passes from §12's table, plus the glossy first bounce
    // that completes the partition.
    set.light_paths
        .push(LightPathPass::new("Glossy", "C<RG>.*L").unwrap());
    set
}

fn render(width: usize, height: usize, samples: u32) -> (Framebuffer, AovBuffers, AovSet) {
    let set = twelve_passes();
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene()).expect("builds");
    let mut framebuffer = Framebuffer::new(width, height);
    let mut aovs = AovBuffers::new(width, height, &set);
    aovs.name_things(
        &OBJECTS.map(String::from),
        &MATERIALS.map(String::from),
    );
    let context = RenderContext {
        camera: Camera::look_at(vec3(0.0, 1.6, 3.6), vec3(0.0, 0.5, 0.0), Vec3::Y),
        // The criterion is an energy identity, and firefly rejection removes
        // energy on purpose. Auditing that removal is the Clamped Energy pass's
        // job; here it would be a term the sum is missing.
        reject_fireflies: false,
        ..RenderContext::default()
    };
    render_pass_with_aovs(
        &intersector,
        &mut framebuffer,
        Some((&mut aovs, &set)),
        &context,
        &ProgressiveSchedule::default(),
        samples,
    );
    aovs.finish(framebuffer.min_samples());
    (framebuffer, aovs, set)
}

/// What a compositor's Cryptomatte node does, written out.
///
/// Nothing here reaches into Tequila's own structures: it takes a file's
/// channels and a name, hashes the name the way the metadata says to, and
/// gathers coverage from the ranked layers. That is the entire procedure, and
/// implementing it against the file rather than against the renderer is what
/// makes this a check on the *format* rather than a restatement of how the
/// buffers were filled.
fn matte_from_file(
    channels: &std::collections::HashMap<String, Vec<f32>>,
    prefix: &str,
    name: &str,
    index: usize,
) -> f32 {
    let id = name_to_float(name);
    let mut coverage = 0.0;
    for layer in 0..3 {
        let base = format!("{prefix}{layer:02}");
        let rank_id = |suffix: &str| {
            channels
                .get(&format!("{base}.{suffix}"))
                .map(|values| values[index])
                .unwrap_or(0.0)
        };
        if rank_id("R") == id {
            coverage += rank_id("G");
        }
        if rank_id("B") == id {
            coverage += rank_id("A");
        }
    }
    coverage
}

fn read_channels(path: &std::path::Path) -> std::collections::HashMap<String, Vec<f32>> {
    let read = exr::prelude::read_all_flat_layers_from_file(path).expect("the file reads back");
    let layer = &read.layer_data[0];
    let mut out = std::collections::HashMap::new();
    for channel in &layer.channel_data.list {
        let values: Vec<f32> = match &channel.sample_data {
            exr::prelude::FlatSamples::F16(v) => v.iter().map(|h| h.to_f32()).collect(),
            exr::prelude::FlatSamples::F32(v) => v.clone(),
            exr::prelude::FlatSamples::U32(v) => v.iter().map(|u| *u as f32).collect(),
        };
        out.insert(channel.name.to_string(), values);
    }
    out
}

fn read_attributes(path: &std::path::Path) -> std::collections::HashMap<String, String> {
    let read = exr::prelude::read_all_flat_layers_from_file(path).expect("the file reads back");
    let layer = &read.layer_data[0];
    let mut out = std::collections::HashMap::new();
    for (name, value) in &layer.attributes.other {
        if let exr::prelude::AttributeValue::Text(text) = value {
            out.insert(name.to_string(), text.to_string());
        }
    }
    out
}

fn write_to(path: &std::path::Path, width: usize, height: usize, samples: u32) {
    let (framebuffer, aovs, set) = render(width, height, samples);
    assert_eq!(
        set.light_paths.len() + 5,
        13,
        "the criterion asks for twelve passes and this is what is being written"
    );
    exr_from_render(&framebuffer, Some(&aovs), &set)
        .write(path)
        .expect("the EXR is written");
}

fn scratch(name: &str) -> std::path::PathBuf {
    let directory = std::env::temp_dir().join("tequila-m4");
    std::fs::create_dir_all(&directory).expect("a scratch directory");
    directory.join(name)
}

#[test]
fn beauty_is_the_sum_of_its_components_in_the_written_file() {
    let path = scratch("passes.exr");
    write_to(&path, 160, 120, 96);
    let channels = read_channels(&path);

    // Read out of the file, not out of the renderer.
    let beauty = channels.get("R").expect("a red channel");
    let components: Vec<&Vec<f32>> = ["Diffuse", "Reflection", "Refraction", "Glossy"]
        .iter()
        .map(|name| {
            channels
                .get(&format!("{name}.R"))
                .unwrap_or_else(|| panic!("no {name} pass in the file"))
        })
        .collect();

    let mut worst = 0.0f32;
    let mut total = 0.0f64;
    for i in 0..beauty.len() {
        let whole = beauty[i];
        let sum: f32 = components.iter().map(|c| c[i]).sum();
        total += whole as f64;
        worst = worst.max((whole - sum).abs() / whole.abs().max(1e-3));
    }
    assert!(total > 1.0, "the render is empty");
    // Half precision, which is what a colour pass is written in: about three
    // decimal digits, so the tolerance is the format's rather than the
    // renderer's. `tequila-render-cpu`'s `passes.rs` holds the same identity
    // to 1e-4 before quantization.
    assert!(
        worst < 5e-3,
        "the components differ from the beauty pass by {worst} of it at worst"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_compositor_can_select_by_name_from_the_manifest() {
    let path = scratch("cryptomatte.exr");
    write_to(&path, 160, 120, 64);
    let channels = read_channels(&path);
    let attributes = read_attributes(&path);

    // Find the object Cryptomatte the way a compositor does: by scanning the
    // metadata for a `name` attribute, not by knowing what it is called.
    let key = attributes
        .iter()
        .find(|(_, value)| *value == "CryptoObject")
        .map(|(name, _)| {
            name.trim_start_matches("cryptomatte/")
                .trim_end_matches("/name")
                .to_string()
        })
        .expect("an object Cryptomatte in the metadata");
    assert_eq!(
        attributes.get(&format!("cryptomatte/{key}/hash")).map(|s| s.as_str()),
        Some("MurmurHash3_32")
    );
    assert_eq!(
        attributes
            .get(&format!("cryptomatte/{key}/conversion"))
            .map(|s| s.as_str()),
        Some("uint32_to_float32")
    );

    // The manifest names the parts, with the hashes the standard's conversion
    // produces.
    let manifest = attributes
        .get(&format!("cryptomatte/{key}/manifest"))
        .expect("a manifest");
    for name in OBJECTS {
        assert!(manifest.contains(name), "`{name}` is not in {manifest}");
        assert!(
            manifest.contains(&format!("{:08x}", murmur3_32(name.as_bytes(), 0))),
            "`{name}`'s hash is not in the manifest"
        );
    }

    // And the matte selects. The mirror ball is somewhere left of centre; the
    // glass ball is right of it. Rather than guess at pixels, check the whole
    // image: each named object covers some of it, and no pixel is covered by
    // more than all of it.
    let pixels = channels.get("R").expect("a red channel").len();
    let mut totals = [0.0f32; 4];
    for i in 0..pixels {
        let mut at_pixel = 0.0;
        for (slot, name) in OBJECTS.iter().enumerate() {
            let coverage = matte_from_file(&channels, "CryptoObject", name, i);
            totals[slot] += coverage;
            at_pixel += coverage;
        }
        assert!(
            at_pixel <= 1.0 + 1e-3,
            "pixel {i} is {at_pixel} covered, which is more than all of it"
        );
    }
    // The three objects in frame each cover some of it. The softbox is the
    // light above the camera's view: it is in the manifest, so a compositor can
    // offer it in a list, and it covers nothing, because it is not in the
    // picture. Those two facts are separate on purpose — a manifest that only
    // listed what happened to be visible would be a list that changes with the
    // camera.
    for name in ["Floor", "Mirror ball", "Glass ball"] {
        let slot = OBJECTS.iter().position(|o| *o == name).unwrap();
        assert!(
            totals[slot] > 1.0,
            "`{name}` covers {} pixels, so it cannot be selected",
            totals[slot]
        );
    }
    let softbox = OBJECTS.iter().position(|o| *o == "Softbox").unwrap();
    assert_eq!(
        totals[softbox], 0.0,
        "the softbox is out of frame and should cover nothing"
    );

    // A name that is not in the scene selects nothing, which is what stops a
    // matte from being a coincidence.
    let absent: f32 = (0..pixels)
        .map(|i| matte_from_file(&channels, "CryptoObject", "Not in this scene", i))
        .sum();
    assert_eq!(absent, 0.0);

    let _ = std::fs::remove_file(&path);
}

/// The data passes carry what a compositor expects, in the precision it needs.
#[test]
fn the_data_passes_are_usable() {
    let path = scratch("data.exr");
    write_to(&path, 96, 72, 32);
    let channels = read_channels(&path);

    let depth = channels.get("Depth").expect("a depth pass");
    assert!(
        depth.iter().any(|v| v.is_finite() && *v > 0.0),
        "nothing in the depth pass is a distance"
    );
    assert!(
        depth.iter().any(|v| v.is_infinite()),
        "nothing in the depth pass escaped, so the background is being given a \
         distance it does not have"
    );

    // Alpha is what makes the render composite at all.
    let alpha = channels.get("A").expect("an alpha channel");
    assert!(alpha.iter().any(|v| *v > 0.9));
    assert!(alpha.iter().any(|v| *v < 0.1));

    for name in ["Normal.X", "Position.X", "Albedo.R", "Clown.R", "Shadow"] {
        assert!(channels.contains_key(name), "`{name}` is missing");
    }

    let _ = std::fs::remove_file(&path);
}

/// The criterion's own resolution. Ignored by default: about two gigabytes and
/// half a minute, which does not belong in every `cargo test`.
///
/// Run it with `cargo test --release -p tequila-format --test
/// m4_exit_criterion -- --ignored --nocapture`.
#[test]
#[ignore = "4K with twelve passes needs ~2 GB; run it by hand"]
fn the_full_4k_render() {
    let path = scratch("4k.exr");
    let start = std::time::Instant::now();
    write_to(&path, 3840, 2160, 8);
    let elapsed = start.elapsed();
    let bytes = std::fs::metadata(&path).expect("the file exists").len();
    println!(
        "4K, twelve passes: {:.1} s, {:.1} MB",
        elapsed.as_secs_f64(),
        bytes as f64 / (1024.0 * 1024.0)
    );

    let channels = read_channels(&path);
    assert_eq!(channels.get("R").map(|c| c.len()), Some(3840 * 2160));
    assert!(channels.contains_key("CryptoObject02.A"));

    let beauty = &channels["R"];
    let components: Vec<&Vec<f32>> = ["Diffuse", "Reflection", "Refraction", "Glossy"]
        .iter()
        .map(|name| &channels[&format!("{name}.R")])
        .collect();
    let mut worst = 0.0f32;
    for i in 0..beauty.len() {
        let sum: f32 = components.iter().map(|c| c[i]).sum();
        worst = worst.max((beauty[i] - sum).abs() / beauty[i].abs().max(1e-3));
    }
    println!("worst component mismatch: {worst}");
    assert!(worst < 5e-3);

    let _ = std::fs::remove_file(&path);
}

/// Precision is a property of what a channel *means*, and the file has to carry
/// it — a Cryptomatte identifier written as half names a different object.
#[test]
fn identifiers_survive_the_file_exactly() {
    let path = scratch("precision.exr");
    write_to(&path, 64, 48, 16);
    let read = exr::prelude::read_all_flat_layers_from_file(&path).expect("it reads");
    let layer = &read.layer_data[0];

    for channel in &layer.channel_data.list {
        let name = channel.name.to_string();
        let is_full = matches!(channel.sample_data, exr::prelude::FlatSamples::F32(_));
        let should_be_full =
            name.starts_with("Crypto") || name == "Depth" || name.starts_with("Position.");
        assert_eq!(
            is_full, should_be_full,
            "`{name}` is stored as {:?}",
            if is_full { Precision::Full } else { Precision::Half }
        );
    }

    // And an identifier read back out of the file is bit-identical to the one
    // hashing the name produces, which is the only way a matte can select.
    let channels = read_channels(&path);
    let ids: Vec<f32> = channels["CryptoObject00.R"].clone();
    let expected: Vec<f32> = OBJECTS.iter().map(|n| name_to_float(n)).collect();
    assert!(
        ids.iter().any(|id| expected.contains(id)),
        "no identifier in the file matches any object's name hash"
    );

    let _ = std::fs::remove_file(&path);
}
