//! Spectral rendering (`docs/01` §7.4): paths that carry four wavelengths.
//!
//! The claims a spectral renderer has to make before anyone can switch it on
//! by default. It must not change a colour it has no reason to change: a
//! white furnace stays white, a coloured surface comes out the colour the
//! RGB renderer gives it, a saturated light the colour it was set to — each
//! is the round trip through Jakob and Hanika's upsampling, measured in the
//! render rather than only in the table. It must do what it is for: split
//! white light through a dispersive ball. And the companions must earn their
//! keep, with less colour noise through a gem than one wavelength gives at
//! the same samples. The table's own error and spectral's cost are measured
//! and printed (`cargo test --test spectral -- --nocapture`).

use std::sync::Arc;
use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Transform, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::MaterialType;
use tequila_materials::openpbr::Material;
use tequila_render_core::environment::Environment;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::pins::{composite, studio_preset};
use tequila_render_core::scene::{Instance, SceneSnapshot};
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_core::settings::{IntegratorConfig, SpectralMode};
use tequila_render_core::spectral;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use tequila_texture::image::{ColorSpace, Image};

const SIZE: usize = 40;

fn sphere_scene(material: Material, environment: Environment) -> SceneSnapshot {
    SceneSnapshot::new(
        vec![Arc::new(tequila_scene::demo::uv_sphere(1.0, 96, 48))],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0)).with_transform(Transform::IDENTITY)],
        vec![material],
        Arc::new(environment),
    )
}

fn render(scene: SceneSnapshot, config: IntegratorConfig, samples: u32, seed: u32) -> Framebuffer {
    render_rejecting(scene, config, samples, seed, false)
}

fn render_rejecting(scene: SceneSnapshot, config: IntegratorConfig, samples: u32, seed: u32, reject_fireflies: bool) -> Framebuffer {
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let mut framebuffer = Framebuffer::new(SIZE, SIZE);
    let context = RenderContext {
        camera: tequila_render_core::camera::Camera::look_at(vec3(0.0, 0.0, 3.4), Vec3::ZERO, Vec3::Y),
        config,
        scene_seed: seed,
        reject_fireflies,
        ..RenderContext::default()
    };
    render_pass(&intersector, &mut framebuffer, &context, &ProgressiveSchedule::default(), samples);
    framebuffer
}

fn with(spectral: SpectralMode) -> IntegratorConfig {
    IntegratorConfig { spectral, ..IntegratorConfig::product() }
}

/// The mean of a box well inside the sphere's silhouette.
fn centre(image: &Framebuffer) -> Spectrum {
    let (lo, hi) = (SIZE * 3 / 8, SIZE * 5 / 8);
    let mut total = Spectrum::ZERO;
    for y in lo..hi {
        for x in lo..hi {
            total += image.pixel(x, y).mean();
        }
    }
    total * (1.0 / ((hi - lo) * (hi - lo)) as f32)
}

/// CIELAB distance, D65 white: how far apart two colours look.
fn delta_e(a: Spectrum, b: Spectrum) -> f32 {
    let (a, b) = (spectral::lab_of(a), spectral::lab_of(b));
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn diffuse(colour: Spectrum) -> Material {
    Material { base_color: colour, ..MaterialType::Diffuse.defaults() }
}

/// A white diffuse ball in a uniformly white world reflects exactly the
/// world: white through every wavelength. The film's white point is what
/// makes the image as a whole exact rather than nearly so; one pixel is
/// white only to within the noise of where its paths' wavelengths fell,
/// which the four wavelengths' spread and the sampling's share for blue
/// keep to a few percent at 64 samples.
#[test]
fn a_white_furnace_stays_white() {
    let image = render(sphere_scene(diffuse(Spectrum::ONE), Environment::uniform(Spectrum::ONE)), with(SpectralMode::On), 64, 0);
    let mut total = Spectrum::ZERO;
    let mut tint = 0.0f32;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let c = image.pixel(x, y).mean();
            total += c;
            tint = tint.max((c.max_component() - c.min_component()) / c.max_component());
        }
    }
    let mean = total * (1.0 / (SIZE * SIZE) as f32);
    println!("white furnace, spectral: mean ({:.4}, {:.4}, {:.4}), the most tinted pixel {:.1}% saturated", mean.r, mean.g, mean.b, tint * 100.0);
    for c in mean.to_array() {
        assert!((c - 1.0).abs() < 0.002, "a white furnace came out {mean:?} spectrally");
    }
    assert!(tint < 0.08, "a pixel of the white furnace is {:.1}% saturated at 64 samples", tint * 100.0);
}

/// A coloured diffuse ball under a white sky is its colour, in RGB exactly
/// and spectrally through the upsampled spectrum: the two renders agree to
/// within what the eye could tell apart.
#[test]
fn a_coloured_ball_renders_the_colour_rgb_gives_it() {
    for srgb in [vec3(0.8, 0.2, 0.1), vec3(0.1, 0.5, 0.8), vec3(0.2, 0.7, 0.2), vec3(0.9, 0.8, 0.1), vec3(0.6, 0.1, 0.7)] {
        let colour = Spectrum::from_linear_srgb(srgb);
        let scene = || sphere_scene(diffuse(colour), Environment::uniform(Spectrum::ONE));
        let rgb = centre(&render(scene(), with(SpectralMode::Off), 64, 0));
        let spectral = centre(&render(scene(), with(SpectralMode::On), 64, 0));
        let difference = delta_e(rgb, spectral);
        println!(
            "diffuse {srgb:?}: RGB ({:.4}, {:.4}, {:.4}), spectral ({:.4}, {:.4}, {:.4}), dE {difference:.3}",
            rgb.r, rgb.g, rgb.b, spectral.r, spectral.g, spectral.b
        );
        assert!(difference < 1.0, "a ball of {srgb:?} renders dE {difference:.2} apart: RGB {rgb:?}, spectral {spectral:?}");
    }
}

/// A light of a saturated colour, seen directly, is that colour: the
/// unbounded upsampling round trip, for the primaries of sRGB, which are as
/// saturated as a colour picker goes.
#[test]
fn a_saturated_light_keeps_its_colour() {
    for srgb in [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0), vec3(1.0, 0.5, 0.0)] {
        let colour = Spectrum::from_linear_srgb(srgb);
        let glow = Material { emission_color: colour, emission_luminance: 4.0, ..MaterialType::Flat.defaults() };
        let scene = || sphere_scene(glow.clone(), Environment::uniform(Spectrum::ZERO));
        // More samples than for a surface: a saturated colour's spectrum is
        // mostly in a few wavelengths, which few paths land on.
        let rgb = centre(&render(scene(), with(SpectralMode::Off), 16, 0));
        let spectral = centre(&render(scene(), with(SpectralMode::On), 256, 0));
        let difference = delta_e(rgb * 0.25, spectral * 0.25);
        let relative = (spectral.luminance() / rgb.luminance() - 1.0).abs();
        println!(
            "light {srgb:?}: RGB ({:.4}, {:.4}, {:.4}), spectral ({:.4}, {:.4}, {:.4}), dE {difference:.3}, luminance off by {:.2}%",
            rgb.r, rgb.g, rgb.b, spectral.r, spectral.g, spectral.b, relative * 100.0
        );
        assert!(difference < 1.5, "a light of {srgb:?} comes out dE {difference:.2} off: RGB {rgb:?}, spectral {spectral:?}");
        // sRGB's blue is the hardest: the nearest smooth spectrum to it has a
        // little less green, 1.2% of its luminance.
        assert!(relative < 0.02, "a light of {srgb:?} changed brightness by {:.2}%", relative * 100.0);
    }
}

fn flint(abbe: f32) -> Material {
    Material {
        specular_ior: 1.72,
        transmission_dispersion_abbe: abbe,
        transmission_color: Spectrum::ONE,
        ..MaterialType::GlassSolid.defaults()
    }
}

/// Dark, but for one small bright light behind and to the side of the ball.
fn spot() -> Environment {
    let (width, height) = (256, 128);
    let towards = vec3(0.35, 0.15, -1.0).normalize();
    let mut data = Vec::with_capacity(width * height * 3);
    for y in 0..height {
        for x in 0..width {
            let d = tequila_render_core::pins::equirect_direction((x as f32 + 0.5) / width as f32, (y as f32 + 0.5) / height as f32);
            let lit = if d.dot(towards) > 0.995 { 40.0 } else { 0.0 };
            data.extend_from_slice(&[lit, lit, lit]);
        }
    }
    Environment::new(Arc::new(Image { width, height, channels: 3, data, color_space: ColorSpace::LinearSrgb }), 1.0)
}

/// The most saturated pixel among those bright enough to be the light seen
/// through the ball, as `dispersion.rs` measures it.
fn fire(image: &Framebuffer) -> f32 {
    let mut most = 0.0f32;
    for y in 0..image.height {
        for x in 0..image.width {
            let c = image.pixel(x, y).mean().to_linear_srgb();
            let (high, low) = (c.x.max(c.y).max(c.z), c.x.min(c.y).min(c.z));
            if high > 0.5 {
                most = most.max((high - low) / high);
            }
        }
    }
    most
}

/// Spectrally, a dispersive ball still splits a small white light into its
/// colours and a plain one does not, and a dispersive ball in a white world
/// is still white: the hero carries on alone through the glass and
/// stands for all four.
#[test]
fn dispersion_still_splits_white_light() {
    let plain = fire(&render(sphere_scene(flint(0.0), spot()), with(SpectralMode::On), 256, 0));
    let spectral = fire(&render(sphere_scene(flint(20.0), spot()), with(SpectralMode::On), 256, 0));
    let rgb = fire(&render(sphere_scene(flint(20.0), spot()), with(SpectralMode::Off), 256, 0));
    println!("a light through flint: {spectral:.3} saturated spectrally, {rgb:.3} by one RGB wavelength, {plain:.3} without dispersion");
    assert!(plain < 0.15, "without dispersion the light should stay white; it is {plain:.3} saturated");
    assert!(spectral > 0.5, "a strongly dispersive ball shows a light at most {spectral:.3} saturated");

    let white = centre(&render(sphere_scene(flint(20.0), Environment::uniform(Spectrum::ONE)), with(SpectralMode::On), 128, 0));
    println!("a dispersive ball in a white world, spectrally: ({:.4}, {:.4}, {:.4})", white.r, white.g, white.b);
    let linear = white.to_linear_srgb();
    for c in [linear.x, linear.y, linear.z] {
        assert!((c - 1.0).abs() < 0.03, "a dispersive ball in a white world reads {white:?}");
    }
}

/// Firefly rejection judges a spectral path by the light it carries, not by
/// how luminous its wavelengths happen to be: with it on, a dispersive ball
/// in a white world is still white, where judging by luminance alone clips
/// the paths whose hero is green hardest and leaves the ball lavender.
#[test]
fn firefly_rejection_does_not_tint_a_spectral_gem() {
    let image = render_rejecting(sphere_scene(flint(20.0), Environment::uniform(Spectrum::ONE)), with(SpectralMode::On), 256, 0, true);
    let white = centre(&image).to_linear_srgb();
    println!("a dispersive ball in a white world, spectrally, rejecting fireflies: ({:.4}, {:.4}, {:.4})", white.x, white.y, white.z);
    for c in [white.x, white.y, white.z] {
        assert!((c - 1.0).abs() < 0.03, "with firefly rejection the ball reads {white:?}");
    }
}

/// Every material in the catalog renders in the studio spectrally within
/// sight of the colour it has in RGB, so an Auto render that turns spectral
/// for a gem does not recolour the rest of the scene. The Gem itself is
/// left out: its fire is what spectral is for, and RGB makes it by one
/// wavelength a path. Media are made per wavelength from their colours'
/// spectra; made per channel and then upsampled, a milky medium came out
/// dE 7 from its RGB render.
#[test]
fn every_material_keeps_its_colour_spectrally() {
    let mut worst = (0.0f32, MaterialType::Generic);
    for kind in MaterialType::ALL {
        let scene = || sphere_scene(kind.defaults(), studio());
        let rgb = centre(&render(scene(), with(SpectralMode::Off), 64, 0));
        let spectral = centre(&render(scene(), with(SpectralMode::On), 64, 0));
        // At a common exposure, so a bright light and a dark velvet are
        // judged alike.
        let scale = 1.0 / rgb.max_component().max(spectral.max_component()).max(1e-6);
        let difference = delta_e(rgb * scale, spectral * scale);
        println!("{kind:?}: dE {difference:.2}");
        if kind != MaterialType::Gem && difference > worst.0 {
            worst = (difference, kind);
        }
    }
    println!("the furthest apart, Gem aside: {:?} at dE {:.2}", worst.1, worst.0);
    assert!(worst.0 < 1.5, "{:?} renders dE {:.2} apart spectrally", worst.1, worst.0);
}

/// The three-point studio, whose softboxes a glass ball both reflects and
/// bends.
fn studio() -> Environment {
    let studio = tequila_scene::demo::studio_environment();
    let lit = composite(studio.image(), &studio_preset("three-point").expect("a shipped preset"));
    Environment::new(Arc::new(lit), 1.0)
}

/// Colour noise over the ball: across renders with different seeds, how far
/// each pixel's chroma (red and blue against green, which a change of
/// brightness alone leaves still) strays from its own mean, against the
/// pixel's luminance, averaged over the pixels the ball covers.
fn colour_noise(config: IntegratorConfig, samples: u32) -> f32 {
    let seeds = 8;
    let renders: Vec<Framebuffer> = (0..seeds).map(|seed| render(sphere_scene(flint(20.0), studio()), config.clone(), samples, seed)).collect();
    let (lo, hi) = (SIZE / 4, SIZE * 3 / 4);
    let mut total = 0.0f64;
    let mut pixels = 0;
    for y in lo..hi {
        for x in lo..hi {
            let values: Vec<Spectrum> = renders.iter().map(|r| r.pixel(x, y).mean()).collect();
            let chroma: Vec<(f64, f64)> = values.iter().map(|c| (f64::from(c.r - c.g), f64::from(c.b - c.g))).collect();
            let mean = chroma.iter().fold((0.0, 0.0), |a, c| (a.0 + c.0, a.1 + c.1));
            let mean = (mean.0 / seeds as f64, mean.1 / seeds as f64);
            let spread = chroma.iter().map(|c| (c.0 - mean.0).powi(2) + (c.1 - mean.1).powi(2)).sum::<f64>() / (seeds - 1) as f64;
            let luminance = values.iter().map(|c| f64::from(c.luminance())).sum::<f64>() / seeds as f64;
            if luminance > 1e-3 {
                total += spread.sqrt() / luminance;
                pixels += 1;
            }
        }
    }
    (total / pixels.max(1) as f64) as f32
}

/// Four wavelengths make less colour noise through a gem than one does at
/// the same samples. Through the glass the hero goes on alone either way;
/// what the companions buy is everything else the ball shows — the studio
/// it reflects, which they see in all four wavelengths at once.
#[test]
fn four_wavelengths_make_less_colour_noise_than_one() {
    let samples = 16;
    let four = colour_noise(with(SpectralMode::On), samples);
    let one = colour_noise(IntegratorConfig { spectral_companions: false, ..with(SpectralMode::On) }, samples);
    let rgb = colour_noise(with(SpectralMode::Off), samples);
    println!(
        "colour noise through a flint ball at {samples} samples: {four:.4} with four wavelengths, {one:.4} with one ({:.2}x), {rgb:.4} by RGB's one wavelength at the glass ({:.2}x)",
        one / four,
        rgb / four
    );
    assert!(four < one, "four wavelengths gave {four:.4} colour noise, one gave {one:.4}");
}

/// The table's round trip over grids of colours, measured in CIELAB against
/// D65, by integrating each fitted spectrum finely along the film's response:
/// reflectances across sRGB's gamut, which is where a colour picker's
/// colours live; the whole AP1 cube, much of which is no colour at all (its
/// green primary lies outside the spectral locus), so its worst is the
/// distance to the nearest real one; and lights, which are unbounded.
#[test]
fn the_table_round_trips_colours() {
    // What a first spectral render waits for, built afresh rather than
    // taken from the one the process already has.
    let start = std::time::Instant::now();
    let _ = spectral::Table::build();
    println!(
        "the table builds in {:.2} s on {} threads",
        start.elapsed().as_secs_f64(),
        std::thread::available_parallelism().map_or(1, |n| n.get())
    );
    let measure = |colours: &mut dyn Iterator<Item = Spectrum>, read: &dyn Fn(Spectrum) -> (spectral::Sigmoid, f32)| {
        let (mut worst, mut sum, mut n, mut at) = (0.0f32, 0.0f64, 0usize, Spectrum::ZERO);
        for target in colours {
            let (sigmoid, scale) = read(target);
            let back = spectral::colour_of(|lambda| sigmoid.at(spectral::normalized(lambda)) * scale);
            // Lights are compared at the brightness of a reflectance, so a
            // bright one's error is not inflated by CIELAB's lightness.
            let norm = target.max_component().max(1.0);
            let error = delta_e(target * (1.0 / norm), back * (1.0 / norm));
            if error > worst {
                worst = error;
                at = target;
            }
            sum += f64::from(error);
            n += 1;
        }
        (worst, (sum / n as f64) as f32, at)
    };
    let steps = 16u32;
    let grid = |scale: f32, srgb: bool| {
        (0..=steps).flat_map(move |r| {
            (0..=steps).flat_map(move |g| {
                (0..=steps).map(move |b| {
                    let v = vec3(r as f32, g as f32, b as f32) * (1.0 / steps as f32);
                    let c = if srgb { Spectrum::from_linear_srgb(v) } else { Spectrum::from_rendering_rgb(v) };
                    c * scale
                })
            })
        })
    };
    let (srgb_worst, srgb_mean, srgb_at) = measure(&mut grid(1.0, true), &spectral::reflectance);
    let (ap1_worst, ap1_mean, ap1_at) = measure(&mut grid(1.0, false), &spectral::reflectance);
    let (light_worst, light_mean, light_at) = measure(&mut grid(8.0, true).filter(|c| !c.is_black()), &spectral::unbounded);
    println!("round trip, sRGB-gamut reflectances ({} colours): mean dE {srgb_mean:.4}, max {srgb_worst:.3} at {srgb_at:?}", (steps + 1).pow(3));
    println!("round trip, the AP1 cube: mean dE {ap1_mean:.4}, max {ap1_worst:.3} at {ap1_at:?}");
    println!("round trip, sRGB-gamut lights: mean dE {light_mean:.4}, max {light_worst:.3} at {light_at:?}");
    assert!(srgb_mean < 0.1 && srgb_worst < 3.0, "sRGB reflectances round-trip with mean dE {srgb_mean} and max {srgb_worst}");
    assert!(light_mean < 0.1 && light_worst < 1.5, "sRGB lights round-trip with mean dE {light_mean} and max {light_worst}");
}

/// What spectral costs on the benchmark scene (`bench/simple_product`, the
/// interactivity baseline): the same pass in RGB and spectrally, timed, and
/// the two images agreeing on how bright the product is.
#[test]
fn spectral_cost_on_the_benchmark_scene() {
    let document = tequila_scene::bench::simple_product();
    let environment = Arc::new(tequila_scene::demo::studio_environment());
    let (scene, _) = tequila_scene::snapshot::snapshot(&document, environment, &mut tequila_scene::snapshot::MeshCache::new());
    let bounds = tequila_scene::snapshot::visible_bounds(&document);
    let centre = bounds.center();
    let radius = (bounds.diagonal().length() * 0.5).max(1e-4);
    let camera = tequila_render_core::camera::Camera::look_at(centre + vec3(0.75, 0.45, 1.0).normalize() * radius * 2.6, centre, Vec3::Y);
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    spectral::prewarm();
    let (width, height, samples) = (320, 180, 16);
    let time = |spectral: SpectralMode| {
        let context = RenderContext { camera, config: with(spectral), ..RenderContext::default() };
        let mut framebuffer = Framebuffer::new(width, height);
        // One pass first, so neither is charged for warming caches.
        render_pass(&intersector, &mut Framebuffer::new(width, height), &context, &ProgressiveSchedule::default(), 1);
        let start = std::time::Instant::now();
        render_pass(&intersector, &mut framebuffer, &context, &ProgressiveSchedule::default(), samples);
        let elapsed = start.elapsed().as_secs_f64();
        let mut total = 0.0f64;
        for pixel in framebuffer.pixels() {
            total += f64::from(pixel.mean().luminance());
        }
        (elapsed, total / (width * height) as f64)
    };
    let (rgb_seconds, rgb_luminance) = time(SpectralMode::Off);
    let (spectral_seconds, spectral_luminance) = time(SpectralMode::On);
    let rate = |seconds: f64| (width * height) as f64 * samples as f64 / seconds / 1e6;
    println!(
        "simple_product at {width}x{height}, {samples} samples: RGB {:.3} s ({:.2} Mspp/s), spectral {:.3} s ({:.2} Mspp/s), {:.2}x the time",
        rgb_seconds,
        rate(rgb_seconds),
        spectral_seconds,
        rate(spectral_seconds),
        spectral_seconds / rgb_seconds
    );
    println!("mean luminance: RGB {rgb_luminance:.4}, spectral {spectral_luminance:.4}");
    assert!(
        (spectral_luminance / rgb_luminance - 1.0).abs() < 0.02,
        "the benchmark scene is {spectral_luminance} bright spectrally and {rgb_luminance} in RGB"
    );
}

/// Auto is spectral exactly where something disperses.
#[test]
fn auto_is_spectral_where_something_disperses() {
    let gem = sphere_scene(flint(20.0), Environment::uniform(Spectrum::ONE));
    let glass = sphere_scene(flint(0.0), Environment::uniform(Spectrum::ONE));
    assert!(SpectralMode::Auto.resolve(&gem));
    assert!(!SpectralMode::Auto.resolve(&glass));
    assert!(SpectralMode::On.resolve(&glass));
    assert!(!SpectralMode::Off.resolve(&gem));
}
