//! The fast viewport shading, over the real BVH.
//!
//! The mode exists for one reason — placing parts and aiming cameras without
//! waiting for light transport — so the properties worth testing are the ones
//! that decide whether it can be used for that: it must be *much* faster than
//! the path tracer, it must never leave a surface unreadable, and it must frame
//! the shot the same way. Anything else about it is a matter of taste and does
//! not belong in a test.

use tequila_render_core::camera::Camera;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_cpu::{preview_pass, render_pass, Device, EmbreeIntersector, RenderContext};
use tequila_scene::demo::{demo_scene, DemoMaterial};

const WIDTH: usize = 320;
const HEIGHT: usize = 240;
/// The timing test's size: four times the work of the others' a pass.
const TIMED_WIDTH: usize = 640;
const TIMED_HEIGHT: usize = 480;

/// The tests here one at a time. The timing test measures alone, which it
/// could not while the others rendered beside it on a three-core runner;
/// the rest take a fraction of a second each.
static ALONE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn alone() -> std::sync::MutexGuard<'static, ()> {
    ALONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[test]
fn solid_remains_sharp_when_the_render_camera_has_depth_of_field() {
    let _alone = alone();
    let (intersector, mut context) = setup();
    let mut pinhole = Framebuffer::new(64, 48);
    preview_pass(&intersector, &mut pinhole, &context, 1);
    context.camera.f_number = 1.2;
    let mut lens = Framebuffer::new(64, 48);
    preview_pass(&intersector, &mut lens, &context, 1);
    for (a, b) in pinhole.pixels().iter().zip(lens.pixels()) { assert_eq!(a.mean(), b.mean()); }
}

fn setup() -> (EmbreeIntersector, RenderContext) {
    let device = Device::shared().expect("Embree");
    let scene = demo_scene(DemoMaterial::Plastic, 0.2);
    let intersector = EmbreeIntersector::build(&device, scene).expect("BVH");
    let context = RenderContext {
        camera: Camera::look_at(
            tequila_core::math::vec3(3.0, 2.0, 4.0),
            tequila_core::math::Vec3::ZERO,
            tequila_core::math::Vec3::Y,
        ),
        ..Default::default()
    };
    (intersector, context)
}

/// The claim the mode is sold on.
///
/// It measures about **6x** on this scene at 320x240 on an idle machine, and
/// that is the pessimistic end: the
/// demo scene is a handful of objects under one environment, where a path is
/// short and there is little for a shadow ray to hit. The gap widens with
/// scene complexity, which is when somebody reaches for the mode.
///
/// It was 7.3x before the shading was rebuilt to carry a world-space
/// hemisphere and a Fresnel rim. That is a real cost and it bought a real
/// thing — the version that ran faster rendered a sphere as a flat disc — but
/// it is a cost, and pretending the old number still holds would make the next
/// regression invisible.
///
/// # How it is measured, and why the obvious way does not work
///
/// The first version timed each side once and compared. It passed alone and
/// failed inside `cargo test --workspace`, reporting 430 ms of preview against
/// 398 ms of path tracing — a ratio of 0.9 for code that is genuinely seven
/// times faster.
///
/// The reasoning behind that version was that a *ratio* survives contention
/// because both sides are measured under it. That is wrong, and it is worth
/// writing down why: the two sides are not the same length. When a dozen other
/// test binaries are fighting for the cores, a scheduling stall of a fixed size
/// lands on both, and a fixed stall is a rounding error on a 228 ms
/// measurement and a catastrophe on a 31 ms one. Contention does not scale
/// the two sides — it adds to them, and adding to the smaller one hurts more.
///
/// Two things fix that, and both follow from the diagnosis.
///
/// The two sides are given **the same amount of wall time** rather than the
/// same number of passes — sixteen preview passes against four path-traced
/// ones, which on this scene is roughly 150 ms each. A fixed stall is then the
/// same fraction of both, and the comparison is per-pass. Min-of-six on top of
/// that, alternating, because the fastest of several runs is the one that got
/// a clear slice and alternating stops either side being systematically
/// unlucky.
///
/// The bound is three rather than the six or so it measures, so that a slower
/// machine is not a failing build. Three still fails every way this could
/// regress, including the two that already have — rebuilding the studio rig
/// per pixel, and losing the early-out around the specular `powf` — because
/// each of those costs about a factor of three on its own.
///
/// # On a shared runner (November 9)
///
/// GitHub's Mac runners, three cores in a virtual machine, still failed it
/// now and then: once at 47 ms of preview against 108 ms of path tracing a
/// pass, where this scene measures 2.3 ms against 14 ms on an M3 Max, 6.3x,
/// and 6.5x on three of its threads with every test here running. Those
/// numbers fit a machine five times slower plus some 35 ms on every pass, a
/// stall of the same size on both sides: the diagnosis above, at a size it
/// did not foresee. So the test now measures alone (`alone`, which every
/// test here takes), at 640 x 480, four times the work a pass and a quarter
/// the weight of any stall, and tries three times before it fails. The bound
/// stays three.
#[test]
fn the_preview_is_far_cheaper_than_a_path_traced_pass() {
    let _alone = alone();
    let (intersector, context) = setup();
    // Tried again before failing: a regression fails every try, a moment
    // the runner gave to something else does not come three times.
    let mut tries = Vec::new();
    for _ in 0..3 {
        let (fast, traced) = time_both(&intersector, &context);
        println!("best of six, per pass: preview {:.2} ms, path traced {:.2} ms, {:.1}x", fast * 1e3, traced * 1e3, traced / fast);
        if traced > fast * 3.0 {
            return;
        }
        tries.push(format!("{:.1} ms against {:.1} ms ({:.1}x)", fast * 1e3, traced * 1e3, traced / fast));
    }
    panic!(
        "the preview was not three times faster than the path tracer in three tries: {}; \
         a mode that is not dramatically faster is not worth the confusion of having it",
        tries.join(", ")
    );
}

/// The preview's and the path tracer's time for a pass, at `TIMED_WIDTH` by
/// `TIMED_HEIGHT`, the best of six rounds after a warm-up.
fn time_both(intersector: &EmbreeIntersector, context: &RenderContext) -> (f64, f64) {
    let schedule = ProgressiveSchedule::default();

    // Allocated once. A fresh framebuffer per round would pay for its page
    // faults inside the timed region, and that cost is fixed rather than
    // proportional — so it lands far harder on the cheaper side and flatters
    // the path tracer. Exactly the mistake described above, in another shape:
    // the first version of this fix measured 3.9x instead of 7.3x for that
    // reason alone. Clearing happens outside the timing and touches memory
    // that is already resident.
    let mut fast_buffer = Framebuffer::new(TIMED_WIDTH, TIMED_HEIGHT);
    let mut traced_buffer = Framebuffer::new(TIMED_WIDTH, TIMED_HEIGHT);

    // Chosen so the two measurements take about the same wall time.
    const FAST_PASSES: usize = 16;
    const TRACED_PASSES: usize = 4;

    let mut fast = f64::MAX;
    let mut traced = f64::MAX;
    for round in 0..7 {
        fast_buffer.clear();
        let fb = &mut fast_buffer;
        let start = std::time::Instant::now();
        for _ in 0..FAST_PASSES {
            preview_pass(intersector, fb, context, 1);
        }
        let elapsed = start.elapsed().as_secs_f64() / FAST_PASSES as f64;
        // The first round is the warm-up for both and is not counted.
        if round > 0 {
            fast = fast.min(elapsed);
        }

        traced_buffer.clear();
        let fb = &mut traced_buffer;
        let start = std::time::Instant::now();
        for _ in 0..TRACED_PASSES {
            render_pass(intersector, fb, context, &schedule, 1);
        }
        let elapsed = start.elapsed().as_secs_f64() / TRACED_PASSES as f64;
        if round > 0 {
            traced = traced.min(elapsed);
        }
    }
    (fast, traced)
}

/// No pixel on the product is ever black, whatever the scene's lights do —
/// the whole point of a camera-fixed rig. A preview that could go dark would
/// fail at the one job it has.
#[test]
fn the_preview_leaves_nothing_unreadable() {
    let _alone = alone();
    let (intersector, context) = setup();
    let mut fb = Framebuffer::new(WIDTH, HEIGHT);
    preview_pass(&intersector, &mut fb, &context, 4);

    let mut dark = 0usize;
    let mut covered = 0usize;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let pixel = fb.pixel(x, y);
            if pixel.alpha() > 0.5 {
                covered += 1;
                if pixel.mean().luminance() < 1e-3 {
                    dark += 1;
                }
            }
        }
    }
    assert!(covered > 500, "the camera is not looking at the product");
    assert_eq!(
        dark, 0,
        "{dark} of {covered} pixels on the product came out black"
    );
}

/// And it frames the same shot: a pixel that is product in one mode is product
/// in the other. Getting this wrong would make the mode actively misleading —
/// a camera aimed in preview is a camera that has to be aimed twice.
#[test]
fn the_preview_frames_what_the_render_frames() {
    let _alone = alone();
    let (intersector, context) = setup();
    let schedule = ProgressiveSchedule::default();

    let mut fast = Framebuffer::new(WIDTH, HEIGHT);
    preview_pass(&intersector, &mut fast, &context, 4);
    let mut traced = Framebuffer::new(WIDTH, HEIGHT);
    render_pass(&intersector, &mut traced, &context, &schedule, 4);

    // Compared at the pixel centres rather than as coverage, so an
    // anti-aliased edge landing a fraction differently is not counted as a
    // disagreement about where the product is.
    let mut differ = 0usize;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let a = fast.pixel(x, y).alpha() > 0.5;
            let b = traced.pixel(x, y).alpha() > 0.5;
            if a != b {
                differ += 1;
            }
        }
    }
    let fraction = differ as f64 / (WIDTH * HEIGHT) as f64;
    assert!(
        fraction < 0.01,
        "{:.2}% of pixels disagree about whether they are the product",
        fraction * 100.0
    );
}

/// A fast frame must be *finished* at one sample.
///
/// This is the mode's entire promise. Anything stochastic left in the shading
/// shows up as grain that clears over the next few frames, and a viewport that
/// visibly settles after every camera move is a viewport that looks like it is
/// still rendering — which is the one impression this mode exists to avoid.
///
/// Measured as the mean absolute difference between one sample and thirty-two
/// of them. It is not expected to be zero: pixel jitter anti-aliases the
/// silhouette, so edge pixels legitimately change. It is expected to be small,
/// and to be concentrated at edges rather than spread across a surface.
#[test]
fn one_sample_is_already_the_finished_frame() {
    let _alone = alone();
    let (intersector, context) = setup();

    let mut one = Framebuffer::new(WIDTH, HEIGHT);
    preview_pass(&intersector, &mut one, &context, 1);
    let mut many = Framebuffer::new(WIDTH, HEIGHT);
    preview_pass(&intersector, &mut many, &context, 32);

    let mut total = 0.0f64;
    let mut worst = 0.0f32;
    let mut noisy = 0usize;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let a = one.pixel(x, y).mean().luminance();
            let b = many.pixel(x, y).mean().luminance();
            let d = (a - b).abs();
            total += d as f64;
            if d > worst {
                worst = d;
            }
            if d > 0.05 {
                noisy += 1;
            }
        }
    }
    let mean = total / (WIDTH * HEIGHT) as f64;
    let fraction = noisy as f64 / (WIDTH * HEIGHT) as f64;
    println!("mean |1spp - 32spp| = {mean:.4}, worst {worst:.4}, {:.2}% noisy", fraction * 100.0);
    assert!(
        fraction < 0.02,
        "{:.1}% of pixels move by more than 0.05 between one sample and \
         thirty-two; the mode visibly settles instead of being instant",
        fraction * 100.0
    );
}

/// A large polished surface seen at a grazing angle, which is where the
/// previous test's scene is not representative.
///
/// This is the case a product scene actually presents: the ground plane is
/// enormous, it runs away from the camera to the horizon, and it is very often
/// polished. A tight specular lobe on a surface like that changes faster than
/// one sample per pixel can follow, so point-sampling it aliases violently —
/// and because the aliasing is different at each jittered sample position, it
/// averages away as samples accumulate. On screen that is grain that clears
/// after every camera move.
fn polished_floor() -> (EmbreeIntersector, RenderContext) {
    use tequila_core::ids::{InstanceId, MaterialId, MeshId};
    use tequila_core::math::vec3;
    use tequila_core::spectrum::Spectrum;
    use tequila_render_core::environment::Environment;
    use tequila_render_core::material::Material;
    use tequila_render_core::scene::{Instance, Mesh, SceneSnapshot};
    use std::sync::Arc;

    let half = 200.0;
    let floor = Mesh::new(
        vec![
            vec3(-half, 0.0, -half),
            vec3(half, 0.0, -half),
            vec3(half, 0.0, half),
            vec3(-half, 0.0, half),
        ],
        vec![0, 1, 2, 0, 2, 3],
    );
    let polished = Material {
        base_color: Spectrum::splat(0.55),
        base_metalness: 1.0,
        specular_roughness: 0.04,
        ..Material::default()
    };
    let scene = SceneSnapshot::new(
        vec![Arc::new(floor)],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
        vec![polished],
        Arc::new(Environment::uniform(Spectrum::ZERO)),
    );

    let device = Device::shared().expect("Embree");
    let intersector = EmbreeIntersector::build(&device, scene).expect("BVH");
    let context = RenderContext {
        // Low and looking along the floor: the grazing angle is the point.
        camera: Camera::look_at(
            vec3(0.0, 0.6, 6.0),
            vec3(0.0, 0.2, -20.0),
            tequila_core::math::Vec3::Y,
        ),
        ..Default::default()
    };
    (intersector, context)
}

#[test]
fn a_polished_floor_does_not_have_to_settle() {
    let _alone = alone();
    let (intersector, context) = polished_floor();

    let mut one = Framebuffer::new(WIDTH, HEIGHT);
    preview_pass(&intersector, &mut one, &context, 1);
    let mut many = Framebuffer::new(WIDTH, HEIGHT);
    preview_pass(&intersector, &mut many, &context, 32);

    let mut noisy = 0usize;
    let mut covered = 0usize;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            if one.pixel(x, y).alpha() < 0.5 {
                continue;
            }
            covered += 1;
            let a = one.pixel(x, y).mean().luminance();
            let b = many.pixel(x, y).mean().luminance();
            if (a - b).abs() > 0.05 {
                noisy += 1;
            }
        }
    }
    assert!(covered > 1000, "the camera is not looking at the floor");
    let fraction = noisy as f64 / covered as f64;
    println!("polished floor: {:.2}% noisy", fraction * 100.0);
    assert!(
        fraction < 0.02,
        "{:.1}% of the floor moves by more than 0.05 between one sample and \
         thirty-two; the highlight is aliasing and the frame visibly settles",
        fraction * 100.0
    );
}

/// The scene the application actually opens with, ground included.
///
/// `demo_scene` above is the geometry only. The ground is a property of the
/// *document's* environment rather than of the scene tree, so it only exists
/// in a snapshot built from a document — which means every test that starts
/// from `demo_scene` has been testing a scene with no floor in it, and the
/// floor is exactly what the grain was reported on.
#[test]
fn the_scene_the_app_opens_with_does_not_have_to_settle() {
    let _alone = alone();
    let document =
        tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.2);
    let mut cache = Default::default();
    let (scene, _) = tequila_scene::snapshot::snapshot(
        &document,
        std::sync::Arc::new(tequila_render_core::environment::Environment::uniform(
            tequila_core::spectrum::Spectrum::ONE,
        )),
        &mut cache,
    );
    assert!(
        scene.instances.iter().any(|i| i.shadow_catcher.is_some()),
        "this snapshot has no ground in it, so it is not the scene in question"
    );

    let device = Device::shared().expect("Embree");
    let intersector = EmbreeIntersector::build(&device, scene).expect("BVH");
    let context = RenderContext {
        camera: Camera::look_at(
            tequila_core::math::vec3(3.0, 1.2, 4.0),
            tequila_core::math::vec3(0.0, 0.4, 0.0),
            tequila_core::math::Vec3::Y,
        ),
        ..Default::default()
    };

    let mut one = Framebuffer::new(WIDTH, HEIGHT);
    preview_pass(&intersector, &mut one, &context, 1);
    let mut many = Framebuffer::new(WIDTH, HEIGHT);
    preview_pass(&intersector, &mut many, &context, 32);

    let mut noisy = 0usize;
    let mut worst = 0.0f32;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let a = one.pixel(x, y).mean().luminance();
            let b = many.pixel(x, y).mean().luminance();
            let d = (a - b).abs();
            worst = worst.max(d);
            if d > 0.05 {
                noisy += 1;
            }
        }
    }
    let fraction = noisy as f64 / (WIDTH * HEIGHT) as f64;
    println!("app scene: {:.2}% noisy, worst {worst:.3}", fraction * 100.0);
    let fraction = noisy as f64 / (WIDTH * HEIGHT) as f64;
    println!("app scene: {:.2}% noisy, worst {worst:.3}", fraction * 100.0);
    // Per-sample values at one ground pixel, to see whether the variation is
    // smooth or bimodal.
    {
        use tequila_core::sampler::Sampler;
        let (x, y) = (160usize, 200usize);
        let aspect = WIDTH as f32 / HEIGHT as f32;
        let rig = tequila_render_core::preview::Rig::for_view(
            context.camera.forward(),
            tequila_core::math::Vec3::Y,
        );
        let mut registers = tequila_render_core::shade::Registers::new();
        let mut scratch = tequila_render_core::material::Material::default();
        for i in 0..8u32 {
            let mut sampler = Sampler::new(x as u32, y as u32, i, 0);
            let j = sampler.pixel_jitter();
            let film = tequila_core::math::Vec2 {
                x: (x as f32 + j.x) / WIDTH as f32,
                y: (y as f32 + j.y) / HEIGHT as f32,
            };
            let ray = context.camera.generate_ray(film, aspect, &mut sampler);
            let sample = tequila_render_core::preview::shade_ray(
                &intersector,
                ray,
                film,
                &rig,
                &mut registers,
                &mut scratch,
            );
            println!(
                "  s{i} jitter=({:.3},{:.3}) inst={} lum={:.4}",
                j.x, j.y, sample.object_id, sample.radiance.luminance()
            );
        }
    }
    assert!(
        fraction < 0.02,
        "{:.1}% of the frame moves by more than 0.05 between one sample and \
         thirty-two; the mode visibly settles instead of being instant",
        fraction * 100.0
    );
}

/// The UV editor's checker, as the processor's Solid draws it: on the part
/// it is on, in place of its colour, through the first UV set or the set
/// handed beside it; and nowhere else.
#[test]
fn the_checker_is_drawn_through_the_set_it_is_given() {
    let _alone = alone();
    use tequila_core::math::{vec2, vec3, Vec2};
    use tequila_core::spectrum::Spectrum;
    use tequila_render_core::preview::PreviewChecker;
    let (intersector, context) = setup();
    let scene = tequila_render_core::intersect::Intersector::scene(&intersector);
    let instance = scene.instances.iter().find(|i| i.visible_to_camera && i.shadow_catcher.is_none()).expect("a part");
    let vertices = scene.mesh(instance.mesh).positions.len();
    // Two by two: red at the top left, green elsewhere.
    let mut bytes = Vec::new();
    for texel in 0..4 {
        bytes.extend_from_slice(if texel == 0 { &[255, 0, 0, 255] } else { &[0, 255, 0, 255] });
    }
    let red = Spectrum::from_linear_srgb(vec3(1.0, 0.0, 0.0));
    let green = Spectrum::from_linear_srgb(vec3(0.0, 1.0, 0.0));
    let count = |checker: Option<&PreviewChecker>| {
        let mut fb = Framebuffer::new(64, 48);
        tequila_render_cpu::preview_pass_checked(&intersector, &mut fb, &context, 1, checker);
        let near = |a: Spectrum, b: Spectrum| (a.r - b.r).abs() + (a.g - b.g).abs() + (a.b - b.b).abs() < 1e-3;
        let reds = fb.pixels().iter().filter(|p| near(p.mean_albedo(), red)).count();
        let greens = fb.pixels().iter().filter(|p| near(p.mean_albedo(), green)).count();
        (reds, greens)
    };
    assert_eq!(count(None), (0, 0), "no checker, no checker colours");
    let at = |place: Vec2| PreviewChecker::from_srgb(vec![instance.id.0], 2, &bytes, Some(std::sync::Arc::new(vec![place; vertices])));
    let (reds, greens) = count(Some(&at(vec2(0.25, 0.25))));
    assert!(reds > 20 && greens == 0, "through the set handed, all in the red square: {reds} red, {greens} green");
    let (reds, greens) = count(Some(&at(vec2(0.75, 0.25))));
    assert!(greens > 20 && reds == 0, "and all in a green one: {reds} red, {greens} green");
    let (first, _) = count(Some(&PreviewChecker::from_srgb(vec![instance.id.0], 2, &bytes, None)));
    let (none, _) = count(Some(&PreviewChecker::from_srgb(vec![u32::MAX - 1], 2, &bytes, None)));
    assert_eq!(none, 0, "on no part, nowhere");
    let _ = first;
}
