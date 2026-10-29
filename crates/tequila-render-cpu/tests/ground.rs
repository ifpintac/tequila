//! The ground plane, rendered (`TEQUILA_SPEC.md` §1.5).
//!
//! A shadow catcher makes three claims that are all testable and all easy to
//! get subtly wrong:
//!
//! 1. Where nothing shadows it, it is **exactly** the background. Not nearly —
//!    exactly. A catcher that tinted the empty floor by a percent would be
//!    visible as a seam the moment the render was composited over the plate it
//!    is supposed to match.
//! 2. Where the product shadows it, it darkens *and* becomes opaque. The alpha
//!    is the point: a shadow that composited away to nothing would leave the
//!    product floating in whatever it was dropped onto.
//! 3. It does not light the scene. A real floor bounces light back up; a
//!    catcher must not, or turning shadows on would change how the product
//!    itself is lit.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Transform, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::MaterialType;
use tequila_materials::openpbr::Material;
use tequila_render_core::camera::Camera;
use tequila_render_core::environment::{Background, Environment};
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::light::{Light, LightKind, LightSet, PointLight};
use tequila_render_core::scene::{Instance, Mesh, SceneSnapshot, ShadowCatcher};
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use std::sync::Arc;

const SIZE: usize = 48;

fn ground(half: f32) -> Mesh {
    Mesh::new(
        vec![
            vec3(-half, 0.0, -half),
            vec3(half, 0.0, -half),
            vec3(half, 0.0, half),
            vec3(-half, 0.0, half),
        ],
        vec![0, 2, 1, 0, 3, 2],
    )
    .with_normals(vec![Vec3::Y; 4])
}

/// A panel standing on the floor at `z = 0`, facing the camera.
///
/// Vertical rather than a box on purpose. It has to do two jobs: cast a shadow
/// backwards onto the floor, and face a surface the floor can bounce light
/// onto. A horizontal plate does the first and not the second — nothing below
/// an upward-facing surface can ever light it — which makes a bounce test over
/// one pass whatever the renderer does.
fn blocker() -> Mesh {
    Mesh::new(
        vec![
            vec3(-1.0, 0.0, 0.0),
            vec3(1.0, 0.0, 0.0),
            vec3(1.0, 2.0, 0.0),
            vec3(-1.0, 2.0, 0.0),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
    .with_normals(vec![Vec3::Z; 4])
}

fn matte() -> Material {
    Material {
        base_color: Spectrum::splat(0.8),
        ..MaterialType::Diffuse.defaults()
    }
}

fn camera() -> Camera {
    // Looking down at the floor beside the blocker, so most of the frame is
    // catcher rather than product.
    Camera::look_at(vec3(0.0, 4.0, 4.0), vec3(0.0, 0.0, 0.0), Vec3::Y)
}

fn render(scene: SceneSnapshot, samples: u32) -> Framebuffer {
    render_with(scene, samples, false)
}

fn render_with(scene: SceneSnapshot, samples: u32, ground_illumination: bool) -> Framebuffer {
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let mut framebuffer = Framebuffer::new(SIZE, SIZE);
    let default = RenderContext::default();
    let context = RenderContext {
        camera: camera(),
        config: tequila_render_core::settings::IntegratorConfig {
            ground_illumination,
            ..default.config.clone()
        },
        ..default
    };
    render_pass(
        &intersector,
        &mut framebuffer,
        &context,
        &ProgressiveSchedule::default(),
        samples,
    );
    framebuffer
}

fn render_seen_from(scene: SceneSnapshot, camera: Camera, samples: u32) -> Framebuffer {
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let mut framebuffer = Framebuffer::new(SIZE, SIZE);
    let context = RenderContext { camera, ..RenderContext::default() };
    render_pass(&intersector, &mut framebuffer, &context, &ProgressiveSchedule::default(), samples);
    framebuffer
}

/// A scene with a coloured background, a catcher, and — optionally — something
/// standing on it.
fn scene(catcher: Option<ShadowCatcher>, with_blocker: bool) -> SceneSnapshot {
    let mut meshes = vec![Arc::new(ground(8.0))];
    let mut instances = vec![match catcher {
        Some(catcher) => Instance::new(InstanceId(0), MeshId(0), MaterialId(0))
            .with_shadow_catcher(catcher),
        None => Instance::new(InstanceId(0), MeshId(0), MaterialId(0)),
    }];
    if with_blocker {
        meshes.push(Arc::new(blocker()));
        instances.push(
            Instance::new(InstanceId(1), MeshId(1), MaterialId(0))
                .with_transform(Transform::IDENTITY),
        );
    }

    let mut scene = SceneSnapshot::new(
        meshes,
        instances,
        vec![matte()],
        Arc::new(Environment::uniform(Spectrum::ZERO)),
    )
    .with_lights(Arc::new(LightSet::new(vec![Light::new(LightKind::Point(
        // In front of the panel and above it, so the panel is lit on the face
        // the camera sees and throws its shadow backwards onto the floor.
        PointLight {
            position: vec3(0.0, 4.0, 3.0),
            intensity: Spectrum::splat(60.0),
            radius: 0.0,
        },
    ))])));
    // A colour, so "the catcher shows the background" is checkable against
    // something other than black.
    scene.background = Background::Color(tequila_core::spectrum::spectrum(0.2, 0.4, 0.9));
    scene
}

/// Claim 1, and the one that has to be exact.
#[test]
fn an_unshadowed_catcher_is_exactly_the_background() {
    let image = render(scene(Some(ShadowCatcher::default()), false), 16);
    let expected = tequila_core::spectrum::spectrum(0.2, 0.4, 0.9);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let pixel = image.pixel(x, y).mean();
            assert!(
                (pixel.r - expected.r).abs() < 1e-5
                    && (pixel.g - expected.g).abs() < 1e-5
                    && (pixel.b - expected.b).abs() < 1e-5,
                "at ({x}, {y}): {pixel:?} is not the background"
            );
        }
    }
    // And it composites away, because there is nothing there.
    assert!(image.pixel(SIZE / 2, SIZE / 2).alpha() < 1e-6);
}

/// Claim 2: the shadow has to be visible *and* opaque.
#[test]
fn a_shadow_darkens_the_catcher_and_becomes_opaque() {
    let image = render(scene(Some(ShadowCatcher::default()), true), 64);

    // Behind the panel, which from a camera looking down at the origin is the
    // upper half of the frame.
    let mut shadowed = None;
    for y in 0..(SIZE / 2) {
        for x in 0..SIZE {
            let alpha = image.pixel(x, y).alpha();
            if alpha > 0.5 && image.pixel(x, y).mean().luminance() < 0.1 {
                shadowed = Some((x, y));
            }
        }
    }
    let (x, y) = shadowed.expect("no shadowed pixel anywhere in the lower frame");
    let pixel = image.pixel(x, y).mean();
    assert!(
        pixel.b < 0.2,
        "the shadow did not darken the background: {pixel:?}"
    );

    // A corner in front of the panel, far from the shadow, is still open sky.
    let corner = image.pixel(1, SIZE - 2);
    assert!(
        corner.alpha() < 0.2,
        "the whole floor went opaque: alpha {} in a corner",
        corner.alpha()
    );
}

/// A part with its shadow switched off throws none onto the catcher, and is
/// still in the picture itself. The toggle used to work only in a scene with
/// a cutaway in it: anywhere else the shadow came out the same either way.
#[test]
fn a_part_that_casts_no_shadow_leaves_the_catcher_clear() {
    // Dark and opaque in the half of the frame behind the panel, which is
    // what claim 2 finds a shadow by.
    let shadowed = |image: &Framebuffer| {
        (0..SIZE / 2)
            .flat_map(|y| (0..SIZE).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let pixel = image.pixel(x, y);
                pixel.alpha() > 0.5 && pixel.mean().luminance() < 0.1
            })
            .count()
    };
    let casting = render(scene(Some(ShadowCatcher::default()), true), 32);
    let mut shadowless = scene(Some(ShadowCatcher::default()), true);
    shadowless.instances[1].casts_shadows = false;
    let shadowless = render(shadowless, 32);

    assert!(shadowed(&casting) > 0, "the panel cast no shadow to begin with");
    assert_eq!(
        shadowed(&shadowless),
        0,
        "a panel with casts_shadows = false still shadowed the catcher"
    );

    let film = camera()
        .project(vec3(0.0, 1.0, 0.0), 1.0)
        .expect("the panel is in frame");
    let px = ((film.x * SIZE as f32) as usize).min(SIZE - 1);
    let py = ((film.y * SIZE as f32) as usize).min(SIZE - 1);
    assert!(
        shadowless.pixel(px, py).alpha() > 0.99,
        "switching the shadow off took the panel out of the picture"
    );
}

/// Claim 3. Switching the ground on must not change how the product is lit,
/// which a real diffuse floor would.
#[test]
fn a_catcher_does_not_bounce_light_onto_the_product() {
    let with = render(scene(Some(ShadowCatcher::default()), true), 96);
    let without = render(scene(None, true), 96);

    // The blocker's top face, found by projecting it rather than by guessing
    // at a screen region — a threshold over the whole frame would compare the
    // product in one render against the floor in the other, which is a
    // different quantity and would pass or fail for the wrong reason.
    // Low on the panel's face, where a bounce off the floor in front of it
    // lands hardest.
    let film = camera()
        .project(vec3(0.0, 0.25, 0.0), 1.0)
        .expect("the panel is in frame");
    let px = ((film.x * SIZE as f32) as usize).min(SIZE - 1);
    let py = ((film.y * SIZE as f32) as usize).min(SIZE - 1);

    let sample = |image: &Framebuffer| -> f32 {
        let mut total = 0.0;
        let mut counted = 0;
        for y in py.saturating_sub(2)..(py + 3).min(SIZE) {
            for x in px.saturating_sub(2)..(px + 3).min(SIZE) {
                total += image.pixel(x, y).mean().luminance();
                counted += 1;
            }
        }
        total / counted.max(1) as f32
    };

    let a = sample(&with);
    let b = sample(&without);
    assert!(a > 0.0 && b > 0.0, "nothing lit in either render at ({px}, {py})");
    assert!(
        a <= b * 1.02,
        "the catcher bounced light back: {a} against an ordinary floor's {b}"
    );
    // And the comparison is not vacuous: the floor really does light this
    // point, so a catcher that failed to suppress the bounce would be caught.
    assert!(
        b > a * 1.2,
        "an ordinary floor bounced nothing here either, so this test proves \
         nothing: {b} against {a}"
    );

    // Ground illumination is the deliberate opposite, and it has to work:
    // with it on, the catcher *is* a floor again and the bounce comes back.
    let illuminating = render_with(scene(Some(ShadowCatcher::default()), true), 96, true);
    let c = sample(&illuminating);
    assert!(
        c > a * 1.2,
        "ground illumination did nothing: {c} against {a} without it"
    );
}

/// A catcher with reflections shows what stands on it. Without this the
/// control is a slider that does nothing, which is worse than not having it.
#[test]
fn ground_reflections_show_the_product() {
    let plain = render(scene(Some(ShadowCatcher::default()), true), 64);
    let mirrored = render(
        scene(
            Some(ShadowCatcher {
                reflections: 1.0,
                ..Default::default()
            }),
            true,
        ),
        64,
    );

    // Floor pixels only, the same ones in both: what the camera sees as
    // fully opaque without reflections is the product itself. (Measured on
    // each image, a reflection strong enough to be all but opaque took its
    // own pixels out of the count.)
    let floor_light = |image: &Framebuffer| -> f32 {
        let mut total = 0.0;
        for y in 0..SIZE {
            for x in 0..SIZE {
                if plain.pixel(x, y).alpha() < 0.9 {
                    total += image.pixel(x, y).mean().luminance();
                }
            }
        }
        total
    };

    assert!(
        floor_light(&mirrored) > floor_light(&plain),
        "reflections added nothing: {} against {}",
        floor_light(&mirrored),
        floor_light(&plain)
    );
}

/// The floor reflects only what stands on it. Under a sky brighter than the
/// ground it stands on - any HDRI, the studio - a reflection of the
/// environment showed the sky where the camera looks at the ground, so a
/// strong one made the floor a bright rectangle on the background: what a
/// floor reflection must never do. Where the reflection misses, the floor
/// is exactly the background; where it meets the panel, the panel shows.
#[test]
fn a_reflecting_floor_shows_only_what_stands_on_it() {
    let two_tone = |catcher: ShadowCatcher| {
        let mut scene = scene(Some(catcher), true);
        // An equirectangular sky of two rows: bright above, dark below.
        let mut rgb = Vec::new();
        for (row, level) in [(0, 220u8), (1, 30u8)] {
            let _ = row;
            for _ in 0..4 {
                rgb.extend_from_slice(&[level, level, level]);
            }
        }
        let image = tequila_texture::image::Image::from_srgb8(4, 2, 3, &rgb);
        scene.environment = Arc::new(Environment::new(Arc::new(image), 1.0));
        scene.background = Background::Environment;
        scene
    };
    let plain = render(two_tone(ShadowCatcher::default()), 32);
    let mirror = ShadowCatcher { reflections: 1.0, ..Default::default() };
    let mirrored = render(two_tone(mirror), 32);
    for (x, y) in [(0, SIZE - 1), (SIZE - 1, SIZE - 1), (0, SIZE / 2), (SIZE - 1, SIZE / 2)] {
        let (a, b) = (plain.pixel(x, y).mean(), mirrored.pixel(x, y).mean());
        assert!(
            (a.luminance() - b.luminance()).abs() < 1e-4,
            "the floor at ({x}, {y}) changed where nothing is reflected: {a:?} became {b:?}"
        );
    }
    let changed = changed_pixels(&plain, &mirrored);
    assert!(changed > SIZE, "the panel's reflection did not show: {changed} pixels changed");
}

/// Blur spreads the reflection: the panel's mirror image smears past its
/// edges, onto floor a sharp reflection leaves alone.
#[test]
fn a_blurred_reflection_spreads() {
    let plain = render(scene(Some(ShadowCatcher::default()), true), 64);
    let sharp = render(scene(Some(ShadowCatcher { reflections: 1.0, ..Default::default() }), true), 64);
    let blurred = render(
        scene(Some(ShadowCatcher { reflections: 1.0, reflection_roughness: 0.6, ..Default::default() }), true),
        64,
    );
    let (sharp, blurred) = (changed_pixels(&plain, &sharp), changed_pixels(&plain, &blurred));
    assert!(blurred > sharp + SIZE / 2, "blur did not spread the reflection: {blurred} pixels against {sharp} sharp");
}

/// The reflection rendered alone (`floor_reflection_only`), laid over the
/// image without it, is the image with it: how Cycles gets Tequila's floor
/// reflection, checked here where both halves are Tequila's.
#[test]
fn the_reflection_alone_lays_over_the_rest() {
    let mirror = ShadowCatcher { reflections: 0.7, ..Default::default() };
    let full = render(scene(Some(mirror), true), 128);
    let base = render(scene(Some(ShadowCatcher::default()), true), 128);
    let layer = render_layer(scene(Some(mirror), true), 128);
    let (mut composed, mut whole) = (0.0f32, 0.0f32);
    for y in 0..SIZE {
        for x in 0..SIZE {
            if base.pixel(x, y).alpha() >= 0.9 {
                continue;
            }
            let (b, l) = (base.pixel(x, y), layer.pixel(x, y));
            composed += (b.mean() * (1.0 - l.alpha()) + l.mean()).luminance();
            whole += full.pixel(x, y).mean().luminance();
        }
    }
    assert!(
        (composed - whole).abs() < 0.03 * whole,
        "laid over the rest, the reflection came to {composed} where the whole render has {whole}"
    );
    // And the layer is nothing where there is no floor to reflect in.
    assert!(layer.pixels().iter().filter(|p| p.alpha() > 0.0).count() < SIZE * SIZE, "the layer covered everything");
}

/// A short reflection meets the product at the floor at full strength and
/// fades with height: with a fade at a quarter of the panel's height, the
/// floor still mirrors its foot as strongly, and far less of it in all.
#[test]
fn a_short_reflection_fades_with_height() {
    let full = render_layer(scene(Some(ShadowCatcher { reflections: 1.0, ..Default::default() }), true), 64);
    let short = render_layer(scene(Some(ShadowCatcher { reflections: 1.0, reflection_fade: Some(0.5), ..Default::default() }), true), 64);
    let total = |image: &Framebuffer| image.pixels().iter().map(|p| p.alpha()).sum::<f32>();
    let strongest = |image: &Framebuffer| image.pixels().iter().map(|p| p.alpha()).fold(0.0f32, f32::max);
    assert!(total(&short) < 0.6 * total(&full), "the reflection did not shorten: {} of {}", total(&short), total(&full));
    assert!(strongest(&short) > 0.9 * strongest(&full), "the reflection weakened at the floor too: {} of {}", strongest(&short), strongest(&full));
}

/// The floor's reflection shows the product as it is, lit by the floor's
/// bounce: with a bounce on, the mirrored panel is brighter, as the panel
/// itself is. Left out of the reflection, a face the bounce lifted was
/// mirrored as dark as without it.
#[test]
fn the_reflection_shows_the_product_lit_by_the_bounce() {
    let mirror = ShadowCatcher { reflections: 1.0, ..Default::default() };
    let bounced = ShadowCatcher { bounce: Some(Spectrum::splat(0.8)), ..mirror.clone() };
    let plain = render_layer(scene(Some(mirror), true), 32);
    let lit = render_layer(scene(Some(bounced), true), 32);
    let light = |image: &Framebuffer| image.pixels().iter().map(|p| p.mean().luminance()).sum::<f32>();
    assert!(light(&lit) > 1.2 * light(&plain), "the bounce did not reach the reflection: {} against {}", light(&lit), light(&plain));
}

/// A short reflection's layer spends its samples where there is a
/// reflection: after a first pass, only a band around what it found takes
/// the rest, and the reflection comes to what sampling the whole frame
/// makes of it.
#[test]
fn the_reflection_pass_samples_only_where_there_is_one() {
    let short = || scene(Some(ShadowCatcher { reflections: 1.0, reflection_fade: Some(0.5), ..Default::default() }), true);
    let whole = render_layer(short(), 64);
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, short()).expect("builds");
    let mut banded = Framebuffer::new(SIZE, SIZE);
    tequila_render_cpu::floor::render_floor_reflection(&intersector, &mut banded, camera(), &tequila_render_core::settings::IntegratorConfig::default(), 64);
    let total = |image: &Framebuffer| image.pixels().iter().map(|p| p.alpha()).sum::<f32>();
    assert!((total(&banded) - total(&whole)).abs() < 0.05 * total(&whole), "the band's reflection is {} where the whole frame's is {}", total(&banded), total(&whole));
    let skipped = banded.pixels().iter().filter(|p| p.samples < 64).count();
    assert!(skipped > SIZE * SIZE / 4, "only {skipped} pixels were spared the full count");
}

fn changed_pixels(a: &Framebuffer, b: &Framebuffer) -> usize {
    let mut changed = 0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            if (a.pixel(x, y).mean().luminance() - b.pixel(x, y).mean().luminance()).abs() > 1e-3 {
                changed += 1;
            }
        }
    }
    changed
}

fn render_layer(scene: SceneSnapshot, samples: u32) -> Framebuffer {
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let mut framebuffer = Framebuffer::new(SIZE, SIZE);
    let default = RenderContext::default();
    let context = RenderContext {
        camera: camera(),
        config: tequila_render_core::settings::IntegratorConfig { floor_reflection_only: true, ..default.config.clone() },
        ..default
    };
    render_pass(&intersector, &mut framebuffer, &context, &ProgressiveSchedule::default(), samples);
    framebuffer
}

/// A reflecting floor under a uniform sky is the sky: its reflection takes
/// its share from the backdrop the floor shows, as a glossy floor's does,
/// rather than adding to it. Added on top, a floor seen low — where Fresnel
/// reflects most — showed the backdrop and most of the sky again, and a floor
/// fitted to the product read as a glowing rectangle with a hard edge.
#[test]
fn a_reflecting_floor_under_a_uniform_sky_is_seamless() {
    let sky = 0.5;
    let mut scene = SceneSnapshot::new(
        vec![Arc::new(ground(3.0))],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0)).with_shadow_catcher(ShadowCatcher {
            reflections: 1.0,
            ..Default::default()
        })],
        vec![matte()],
        Arc::new(Environment::uniform(Spectrum::splat(sky))),
    );
    scene.background = Background::Environment;
    // Low over the floor, outside it, so the frame holds its near corner and
    // the open backdrop around it.
    let camera = Camera::look_at(vec3(4.0, 0.4, 4.0), vec3(0.0, 0.6, 0.0), Vec3::Y);
    let image = render_seen_from(scene, camera, 16);

    // A point on the floor near its corner, seen eight degrees below level.
    let film = camera.project(vec3(2.0, 0.0, 2.0), 1.0).expect("the floor point is in view");
    let (x, y) = ((film.x * SIZE as f32) as usize, (film.y * SIZE as f32) as usize);
    assert!(image.pixel(x, y).mean_depth() > 0.0, "({x}, {y}) is not the floor");
    let (x, y) = (0..SIZE * SIZE)
        .map(|i| (i % SIZE, i / SIZE))
        .max_by(|&(ax, ay), &(bx, by)| {
            let off = |x, y| (image.pixel(x, y).mean().g - sky).abs();
            off(ax, ay).total_cmp(&off(bx, by))
        })
        .unwrap();
    let worst = image.pixel(x, y).mean().g;
    assert!(
        (worst / sky - 1.0).abs() < 0.02,
        "at ({x}, {y}) the frame reads {worst:.3} against a sky of {sky}: the floor shows"
    );
}

/// The share of the floor a reflection takes is opaque in an image with
/// alpha, as it is in the beauty: the floor over the backdrop it was rendered
/// against is the floor in the beauty. Counted as see-through, a reflection
/// darker than the backdrop it replaces — a black part's — composited away.
#[test]
fn a_dark_reflection_stays_in_an_image_with_alpha() {
    let sky = 0.5;
    let black = Material {
        base_color: Spectrum::ZERO,
        ..MaterialType::Diffuse.defaults()
    };
    let mut scene = SceneSnapshot::new(
        vec![Arc::new(ground(8.0)), Arc::new(blocker())],
        vec![
            Instance::new(InstanceId(0), MeshId(0), MaterialId(0)).with_shadow_catcher(ShadowCatcher {
                reflections: 1.0,
                ..Default::default()
            }),
            Instance::new(InstanceId(1), MeshId(1), MaterialId(0)),
        ],
        vec![black],
        Arc::new(Environment::uniform(Spectrum::splat(sky))),
    );
    scene.background = Background::Environment;
    let eye = vec3(0.0, 0.4, 4.0);
    let camera = Camera::look_at(eye, vec3(0.0, 0.6, 0.0), Vec3::Y);
    let image = render_seen_from(scene, camera, 64);

    // Where the floor mirrors the panel's lower middle: the camera's ray to
    // that point's image below the floor crosses the floor here.
    let mirrored = vec3(0.0, -0.3, 0.0);
    let point = eye + (mirrored - eye) * (eye.y / (eye.y - mirrored.y));
    let film = camera.project(point, 1.0).expect("the floor point is in view");
    let floor = image.pixel((film.x * SIZE as f32) as usize, (film.y * SIZE as f32) as usize);
    let (beauty, alpha) = (floor.mean().g, floor.alpha());
    assert!(beauty < 0.6 * sky, "the panel's reflection does not show: {beauty:.3}");
    let over = floor.mean_foreground().g + (1.0 - alpha) * sky;
    assert!(
        (over - beauty).abs() < 1e-3,
        "with its alpha of {alpha:.3} the floor composites to {over:.3} over the sky, \
         and reads {beauty:.3} in the beauty"
    );
}

/// Claim 3, towards the environment: a catcher that is not a surface hides
/// nothing. What a product sees below the horizon is the environment's own
/// ground (§1.5's Flatten Ground projects exactly that onto the floor), so a
/// metal sphere above a catcher looks as it does with no floor at all. The
/// catcher used to end every path that reached it by a bounce, which made it
/// a black floor to reflections: a chrome part's lower half rendered black.
#[test]
fn a_catcher_hides_nothing_below_the_horizon() {
    let sphere = || Arc::new(tequila_scene::demo::uv_sphere(0.8, 64, 32));
    let metal = Material {
        base_metalness: 1.0,
        base_color: Spectrum::ONE,
        specular_roughness: 0.15,
        ..MaterialType::Metal.defaults()
    };
    let lifted = Transform::from_matrix(tequila_core::math::Mat4::translation(vec3(0.0, 1.0, 0.0))).unwrap();
    let render_sphere = |with_catcher: bool| {
        let mut meshes = vec![sphere()];
        let mut instances = vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0)).with_transform(lifted)];
        if with_catcher {
            meshes.push(Arc::new(ground(8.0)));
            instances.push(Instance::new(InstanceId(1), MeshId(1), MaterialId(0)).with_shadow_catcher(ShadowCatcher::default()));
        }
        let scene = SceneSnapshot::new(meshes, instances, vec![metal.clone()], Arc::new(Environment::uniform(Spectrum::ONE)));
        let device = Device::shared().expect("an Embree device");
        let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
        let mut framebuffer = Framebuffer::new(SIZE, SIZE);
        let context = RenderContext {
            camera: Camera::look_at(vec3(0.0, 1.0, 4.0), vec3(0.0, 1.0, 0.0), Vec3::Y),
            reject_fireflies: false,
            ..RenderContext::default()
        };
        render_pass(&intersector, &mut framebuffer, &context, &ProgressiveSchedule::default(), 128);
        framebuffer
    };
    let (with, without) = (render_sphere(true), render_sphere(false));
    // The sphere's lower half, which faces the floor.
    let lower = |image: &Framebuffer| -> f32 {
        let (mut total, mut n) = (0.0, 0.0);
        for y in SIZE / 2 + 3..SIZE / 2 + 9 {
            for x in SIZE / 2 - 3..SIZE / 2 + 3 {
                total += image.pixel(x, y).mean().luminance();
                n += 1.0;
            }
        }
        total / n
    };
    let (a, b) = (lower(&with), lower(&without));
    assert!(b > 0.5, "the control is not lit: {b}");
    assert!((a - b).abs() < 0.04 * b, "the sphere's lower half reads {a} above a catcher and {b} with no floor");
}
