//! The tile-parallel CPU renderer.
//!
//! `docs/01-RENDERER-CORE.md` §10.2: Rayon over 16x16 tiles. §4.6 constrains
//! how: each tile owns its pixels outright and sums their samples in sample
//! order, so the result never depends on how work was scheduled.

use tequila_core::math::Vec2;
use tequila_core::sampler::Sampler;
use tequila_render_core::camera::Camera;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::aov::AovSet;
use tequila_render_core::framebuffer::AovBuffers;
use tequila_render_core::integrator::caustics::{self, CausticMap};
use tequila_render_core::integrator::{trace_path_with_caustics, PathAovs, PathResult};
use tequila_render_core::intersect::Intersector;
use tequila_render_core::preview;
use tequila_render_core::scheduler::{tiles, ProgressiveSchedule, Tile};
use tequila_render_core::settings::IntegratorConfig;
use rayon::prelude::*;

/// Everything a render pass needs beyond the scene.
#[derive(Clone, Debug)]
pub struct RenderContext {
    pub camera: Camera,
    pub config: IntegratorConfig,
    /// Fixed for a render. Changing it is how a user asks for a different
    /// noise pattern without changing anything else.
    pub scene_seed: u32,
    /// Whether to run firefly rejection during accumulation (§3.5).
    pub reject_fireflies: bool,
    /// Limits sampling to a rectangle (§9.5). `None` renders everything.
    pub region: Option<Tile>,
    /// How far §12's Ambient Occlusion pass looks. In scene units, so it means
    /// the same thing whatever the model was authored in.
    pub ambient_occlusion_radius: f32,
    /// §3.5's optional hard ceiling on one sample's luminance. `None` is the
    /// default and the unbiased choice.
    pub clamp_sample: Option<f32>,
}

impl Default for RenderContext {
    fn default() -> Self {
        RenderContext {
            camera: Camera::default(),
            config: IntegratorConfig::product(),
            scene_seed: 0,
            reject_fireflies: true,
            region: None,
            // A hand's breadth at the scale most products are modelled in.
            // Large enough to find the floor under a part, small enough not to
            // shade one side of a room.
            ambient_occlusion_radius: 0.1,
            clamp_sample: None,
        }
    }
}

/// Add `samples_per_pixel` samples to every live pixel of `framebuffer`.
///
/// Tiles are disjoint, so each is handed an exclusive slice of the buffer and
/// no synchronization is needed inside the loop. Rayon may schedule them in
/// any order; the result is identical either way, which is what §4.6 requires.
pub fn render_pass(
    intersector: &dyn Intersector,
    framebuffer: &mut Framebuffer,
    context: &RenderContext,
    schedule: &ProgressiveSchedule,
    samples_per_pixel: u32,
) {
    render_pass_with_aovs(
        intersector,
        framebuffer,
        None,
        context,
        schedule,
        samples_per_pixel,
    )
}

/// The same, filling the render passes of §12 as it goes.
///
/// A separate entry point rather than an option on the first, because the
/// difference is not a flag: with passes the loop carries a light-path matcher,
/// routes every contribution through it, and writes a dozen extra buffers.
/// Charging the viewport for that machinery to keep one function would slow
/// down the path that runs sixty times a second for the sake of the one that
/// runs when a person presses Render.
pub fn render_pass_with_aovs(
    intersector: &dyn Intersector,
    framebuffer: &mut Framebuffer,
    aovs: Option<(&mut AovBuffers, &AovSet)>,
    context: &RenderContext,
    schedule: &ProgressiveSchedule,
    samples_per_pixel: u32,
) {
    // With caustics on, one sample at a time: each gets its own photons and
    // its own, smaller radius. Sharing one set across a batch would give a
    // render an eighth as many photon passes as samples.
    if context.config.caustics && samples_per_pixel > 1 {
        let mut aovs = aovs;
        for _ in 0..samples_per_pixel {
            render_pass_with_aovs(
                intersector,
                framebuffer,
                aovs.as_mut().map(|(buffers, set)| (&mut **buffers, *set)),
                context,
                schedule,
                1,
            );
        }
        return;
    }
    let (width, height) = (framebuffer.width, framebuffer.height);
    if width == 0 || height == 0 {
        return;
    }
    let aspect = width as f32 / height as f32;
    let work = tiles(width, height, context.region);
    if work.is_empty() {
        return;
    }
    // The pixel's angular size, for this camera at this resolution, so a
    // texture is looked up at the level that matches a pixel of *this*
    // image. An orthographic camera has no angle to give; it keeps the
    // default.
    let fov = context.camera.vertical_fov(aspect);
    let config = tequila_render_core::settings::IntegratorConfig {
        pixel_spread: if fov > 0.0 {
            fov / height as f32
        } else {
            tequila_render_core::settings::DEFAULT_PIXEL_SPREAD
        },
        ..context.config.clone()
    };
    let context = &RenderContext {
        config,
        ..context.clone()
    };

    // Hand each tile a raw pointer to the shared buffer. The tiles partition
    // the image, so no two ever touch the same pixel.
    struct Shared(*mut tequila_render_core::framebuffer::Pixel);
    // SAFETY: tiles are disjoint by construction (`tiles` walks a grid), so
    // each pixel is written by exactly one task.
    unsafe impl Send for Shared {}
    unsafe impl Sync for Shared {}

    let base = Shared(framebuffer.pixels_mut().as_mut_ptr());
    let adaptive_active = schedule.adaptive_sampling_active(framebuffer.max_samples());
    let threshold = context.config.adaptive_sampling_threshold;

    // The pass buffers ride along under the same safety argument as the beauty
    // pixels: the tiles partition the image, so no two tasks ever touch the
    // same pixel of any of them.
    let (aov_buffers, aov_set) = match aovs {
        Some((buffers, set)) => (Some(buffers), Some(set)),
        None => (None, None),
    };
    let expressions: Vec<tequila_render_core::lpe::Lpe> =
        aov_set.map(|set| set.expressions()).unwrap_or_default();
    let occlusion_radius = context.ambient_occlusion_radius;
    let want_occlusion = aov_set.is_some_and(|set| set.data.ambient_occlusion);
    struct SharedAovs(*mut AovBuffers);
    // SAFETY: as above. Every method reached through this pointer writes only
    // at the pixel it is given, and the tiles are disjoint.
    unsafe impl Send for SharedAovs {}
    unsafe impl Sync for SharedAovs {}
    let aov_base = aov_buffers.map(|buffers| SharedAovs(buffers as *mut AovBuffers));

    // This pass's caustic photons (§6.5), numbered by how far the image has
    // got, so each pass reads a fresh set with a smaller radius.
    let caustic_map = if context.config.caustics {
        build_caustics(intersector, &context.config, framebuffer.max_samples(), context.scene_seed)
    } else {
        None
    };
    let caustic_map = caustic_map.as_ref();

    work.par_iter().for_each(|tile| {
        let base = &base;
        for y in tile.y0..tile.y1 {
            for x in tile.x0..tile.x1 {
                // SAFETY: `(x, y)` lies inside this tile, and tiles are
                // disjoint, so this reference is exclusive.
                let pixel = unsafe { &mut *base.0.add(y * width + x) };

                if adaptive_active && pixel.retired {
                    continue;
                }

                let mut path_aovs = PathAovs::new(expressions.clone());
                for s in 0..samples_per_pixel {
                    let sample_index = pixel.samples;
                    let mut sampler =
                        Sampler::new(x as u32, y as u32, sample_index, context.scene_seed);

                    // Filter importance sampling (§4.5): jitter the film point
                    // by the filter kernel rather than weighting afterwards,
                    // which keeps accumulation a plain running mean.
                    let jitter = sampler.pixel_jitter();
                    let film = Vec2 {
                        x: (x as f32 + jitter.x) / width as f32,
                        y: (y as f32 + jitter.y) / height as f32,
                    };

                    let ray = context.camera.generate_ray(film, aspect, &mut sampler);
                    let mut result = trace_path_with_caustics(
                        intersector,
                        &context.config,
                        ray,
                        film,
                        &mut sampler,
                        &mut path_aovs,
                        caustic_map,
                    );
                    if want_occlusion {
                        result.occlusion = ambient_occlusion(
                            intersector,
                            &result,
                            occlusion_radius,
                            &mut sampler,
                        );
                    }
                    pixel.accumulate_with(
                        &result,
                        context.reject_fireflies,
                        context.clamp_sample,
                    );
                    if let Some(aov_base) = &aov_base {
                        // SAFETY: the tile owns `(x, y)`, and every write below
                        // is at that pixel.
                        let buffers = unsafe { &mut *aov_base.0 };
                        buffers.accumulate(x, y, &result, &path_aovs.passes);
                        let object = buffers.object_id_of(result.object_id);
                        let material = buffers.material_id_of(result.material_id);
                        if let (Some(matte), Some(id)) = (&mut buffers.objects, object) {
                            matte.accumulate(x, y, id, 1.0);
                        }
                        if let (Some(matte), Some(id)) = (&mut buffers.materials, material) {
                            matte.accumulate(x, y, id, 1.0);
                        }
                    }
                    let _ = s;
                }
            }
        }
    });

    if schedule.adaptive_sampling_active(framebuffer.max_samples()) {
        framebuffer.update_adaptive_retirement(threshold, 16);
    }
}

/// Trace a pass's caustic photons and index them: `None` where the scene
/// has nothing mirror-smooth or nothing to light it. In chunks across the
/// pool, gathered in chunk order, so the map is the same on any number of
/// threads.
pub fn build_caustics(
    intersector: &dyn Intersector,
    config: &IntegratorConfig,
    pass: u32,
    seed: u32,
) -> Option<CausticMap> {
    let plan = caustics::plan(intersector)?;
    let total = caustics::PHOTONS_PER_PASS;
    const CHUNK: u32 = 2048;
    let chunks: Vec<Vec<caustics::Photon>> = (0..total.div_ceil(CHUNK))
        .into_par_iter()
        .map(|chunk| {
            let mut out = Vec::new();
            let range = chunk * CHUNK..((chunk + 1) * CHUNK).min(total);
            caustics::trace_photons(intersector, config, &plan, pass, seed, range, total, &mut out);
            out
        })
        .collect();
    Some(CausticMap::build(intersector, chunks.concat(), plan.radius(pass)))
}

/// Fill `framebuffer` with [`tequila_render_core::preview`]'s fast shading.
///
/// Shaped like [`render_pass`] and sharing its tiling and its safety argument,
/// because the difference is only what happens at a pixel — but it takes no
/// [`ProgressiveSchedule`]: adaptive sampling retires pixels by *variance*, and
/// a shading with no random component has none, so every pixel would retire on
/// the second sample and the image would stop anti-aliasing itself halfway.
///
/// Samples still accumulate. One sample per pixel is instant but aliased, and
/// a jittered sample added per frame turns a hard edge into a clean one within
/// a second of the user holding still — free, because the frames were going to
/// be drawn anyway.
pub fn preview_pass(
    intersector: &dyn Intersector,
    framebuffer: &mut Framebuffer,
    context: &RenderContext,
    samples_per_pixel: u32,
) {
    let (width, height) = (framebuffer.width, framebuffer.height);
    if width == 0 || height == 0 {
        return;
    }
    let aspect = width as f32 / height as f32;
    let work = tiles(width, height, context.region);
    if work.is_empty() {
        return;
    }

    struct Shared(*mut tequila_render_core::framebuffer::Pixel);
    // SAFETY: tiles are disjoint by construction, so each pixel is written by
    // exactly one task. The same argument as `render_pass_with_aovs`.
    unsafe impl Send for Shared {}
    unsafe impl Sync for Shared {}
    let base = Shared(framebuffer.pixels_mut().as_mut_ptr());
    // The studio rig is a property of the view, not of the ray, so it is built
    // once for the frame.
    let rig = preview::Rig::for_view(context.camera.forward(), tequila_core::math::Vec3::Y);
    // Likewise the materials, cut down to what Solid shading reads.
    let programs = preview::SolidPrograms::for_scene(intersector.scene());

    work.par_iter().for_each(|tile| {
        let base = &base;
        // One set of shading registers per tile rather than per pixel: a
        // material graph evaluation needs scratch, and allocating it eight
        // million times a frame would cost more than the shading.
        let mut registers = tequila_render_core::shade::Registers::new();
        let mut scratch = tequila_render_core::material::Material::default();

        for y in tile.y0..tile.y1 {
            for x in tile.x0..tile.x1 {
                // SAFETY: `(x, y)` lies inside this tile, and tiles are
                // disjoint, so this reference is exclusive.
                let pixel = unsafe { &mut *base.0.add(y * width + x) };

                for _ in 0..samples_per_pixel {
                    let mut sampler =
                        Sampler::new(x as u32, y as u32, pixel.samples, context.scene_seed);
                    let jitter = sampler.pixel_jitter();
                    let film = Vec2 {
                        x: (x as f32 + jitter.x) / width as f32,
                        y: (y as f32 + jitter.y) / height as f32,
                    };
                    let ray = context.camera.pick_ray(film, aspect);
                    let sample = preview::shade_ray_with(
                        intersector,
                        ray,
                        film,
                        &rig,
                        Some(&programs),
                        &mut registers,
                        &mut scratch,
                    );
                    pixel.accumulate(&result_from(sample), false);
                }
            }
        }
    });
}

/// Dress a preview sample as a path result so it can go through the same
/// accumulation, the same develop pass and the same display pipeline.
///
/// The fields this mode cannot honestly fill are left at what "not measured"
/// means for each: no clamped energy, no split of direct lighting, no
/// occlusion. Guessing at them would put numbers in the AOVs that no light
/// transport produced.
fn result_from(sample: preview::PreviewSample) -> PathResult {
    PathResult {
        radiance: sample.radiance,
        albedo: sample.albedo,
        normal: sample.normal,
        depth: sample.depth,
        escaped: sample.escaped,
        coverage: if sample.escaped { 0.0 } else { 1.0 },
        clamped_energy: 0.0,
        position: sample.position,
        object_id: sample.object_id,
        material_id: sample.material_id,
        direct_unshadowed: sample.radiance,
        direct_visible: sample.radiance,
        occlusion: 0.0,
        labels: tequila_core::spectrum::Spectrum::ZERO,
        labels_alpha: 0.0,
        wavelength: None,
        // An escaped preview ray is the background and nothing else.
        background: if sample.escaped { sample.radiance } else { tequila_core::spectrum::Spectrum::ZERO },
    }
}

/// §12's Ambient Occlusion pass: an independent short-ray estimate.
///
/// Independent on purpose. It is not derived from the lighting, so it is the
/// same image whatever the environment does — which is what a compositor wants
/// it for, as a contact mask to multiply into a grade rather than as a
/// measurement of the scene's light.
///
/// Through [`Intersector::occluded`], so what casts no shadow — a light's
/// visible stand-in — occludes nothing here either, as in Cycles, whose AO
/// rays are shadow rays.
fn ambient_occlusion(
    intersector: &dyn Intersector,
    result: &PathResult,
    radius: f32,
    sampler: &mut Sampler,
) -> f32 {
    if !result.depth.is_finite() || result.normal.length_squared() < 1e-6 {
        return 0.0;
    }
    let normal = result.normal.normalize();
    let onb = tequila_core::math::Onb::from_normal(normal);
    let direction = onb.to_world(tequila_core::sampling::sample_cosine_hemisphere(
        sampler.light_sample(),
    ));
    // Bounded by the radius: occlusion is about what is *near*, and letting it
    // reach the whole scene turns a contact mask into a second, wrong shadow.
    let reach = radius.max(1e-4);
    let target = result.position + direction * reach;
    if intersector.occluded(result.position, normal, target, -direction) {
        1.0
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::ids::MaterialId;
    use tequila_core::math::{vec3, Vec3};
    use tequila_core::spectrum::Spectrum;
    use tequila_render_core::environment::Environment;
    use tequila_render_core::material::Material;
    use tequila_render_core::scene::SceneSnapshot;
    use tequila_render_core::test_support::{AnalyticIntersector, Plane, Sphere};
    use std::sync::Arc;

    fn test_scene() -> AnalyticIntersector {
        let scene = SceneSnapshot::new(
            Vec::new(),
            Vec::new(),
            vec![Material::plastic(Spectrum::splat(0.6), 0.3)],
            Arc::new(Environment::uniform(Spectrum::ONE)),
        );
        AnalyticIntersector::new(scene)
            .with_sphere(Sphere::new(vec3(0.0, 1.0, 0.0), 1.0, MaterialId(0)))
            .with_plane(Plane::new(Vec3::ZERO, Vec3::Y, MaterialId(0)))
    }

    fn context() -> RenderContext {
        RenderContext {
            camera: Camera::look_at(vec3(0.0, 1.5, 5.0), vec3(0.0, 1.0, 0.0), Vec3::Y),
            config: IntegratorConfig::product(),
            scene_seed: 42,
            reject_fireflies: false,
            region: None,
            ..Default::default()
        }
    }

    #[test]
    fn a_pass_accumulates_the_requested_samples_everywhere() {
        let mut fb = Framebuffer::new(48, 32);
        let schedule = ProgressiveSchedule::default();
        render_pass(&test_scene(), &mut fb, &context(), &schedule, 4);
        assert_eq!(fb.min_samples(), 4);
        assert_eq!(fb.max_samples(), 4);
        // And it produced an actual image, not a black one.
        let mean: f32 = fb
            .pixels()
            .iter()
            .map(|p| p.mean().luminance())
            .sum::<f32>()
            / fb.pixels().len() as f32;
        assert!(mean > 0.05, "image is essentially black: {mean}");
    }

    /// The property everything in §4.6 exists to protect: the same scene at
    /// the same seed must produce a **bit-identical** image, no matter how
    /// many threads rendered it.
    #[test]
    fn rendering_is_bit_identical_across_thread_counts() {
        let scene = test_scene();
        let context = context();
        let schedule = ProgressiveSchedule::default();

        let render_with = |threads: usize| {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap();
            pool.install(|| {
                let mut fb = Framebuffer::new(40, 24);
                render_pass(&scene, &mut fb, &context, &schedule, 8);
                fb.pixels()
                    .iter()
                    .map(|p| p.mean().to_array())
                    .collect::<Vec<_>>()
            })
        };

        let one = render_with(1);
        let four = render_with(4);
        let sixteen = render_with(16);
        assert_eq!(one, four, "1 vs 4 threads diverged");
        assert_eq!(one, sixteen, "1 vs 16 threads diverged");
    }

    /// Rendering the same buffer twice must be identical to rendering it once
    /// with twice the samples: accumulation must be resumable, which is what
    /// makes the whole progressive architecture work.
    #[test]
    fn accumulation_is_resumable() {
        let scene = test_scene();
        let context = context();
        let schedule = ProgressiveSchedule::default();

        let mut incremental = Framebuffer::new(24, 16);
        render_pass(&scene, &mut incremental, &context, &schedule, 3);
        render_pass(&scene, &mut incremental, &context, &schedule, 5);

        let mut all_at_once = Framebuffer::new(24, 16);
        render_pass(&scene, &mut all_at_once, &context, &schedule, 8);

        for (a, b) in incremental.pixels().iter().zip(all_at_once.pixels()) {
            assert_eq!(a.samples, b.samples);
            assert_eq!(a.mean().to_array(), b.mean().to_array());
        }
    }

    /// Region rendering must leave everything outside the region untouched
    /// (§9.5).
    #[test]
    fn region_rendering_touches_only_its_region() {
        let mut fb = Framebuffer::new(64, 64);
        let mut context = context();
        context.region = Some(Tile {
            x0: 16,
            y0: 16,
            x1: 48,
            y1: 48,
        });
        render_pass(
            &test_scene(),
            &mut fb,
            &context,
            &ProgressiveSchedule::default(),
            2,
        );
        for y in 0..64 {
            for x in 0..64 {
                let inside = (16..48).contains(&x) && (16..48).contains(&y);
                let samples = fb.pixel(x, y).samples;
                assert_eq!(
                    samples,
                    if inside { 2 } else { 0 },
                    "pixel ({x},{y}) has {samples} samples"
                );
            }
        }
    }

    #[test]
    fn crop_adaptive_sampling_ignores_unrendered_border() {
        let mut fb = Framebuffer::new(16, 16);
        let mut context = context();
        context.region = Some(Tile { x0: 4, y0: 4, x1: 12, y1: 12 });
        context.config.adaptive_sampling_threshold = 100.0;
        let mut schedule = ProgressiveSchedule::default();
        schedule.update(60_000);
        render_pass(&test_scene(), &mut fb, &context, &schedule, 32);
        assert_eq!(fb.pixel(0, 0).samples, 0);
        assert!((4..12).any(|y| (4..12).any(|x| fb.pixel(x,y).retired)), "the unsampled border prevented adaptive retirement");
    }

    /// The camera must be pointing where the image says it is: the sphere sits
    /// above the horizon, so the upper middle of the frame must show it and
    /// the lower edge must show ground.
    #[test]
    fn the_image_matches_the_scene_layout() {
        let mut fb = Framebuffer::new(64, 64);
        render_pass(
            &test_scene(),
            &mut fb,
            &context(),
            &ProgressiveSchedule::default(),
            8,
        );
        // Coverage: the sphere and ground fill the frame's centre and bottom;
        // the top corners see open sky.
        assert!(fb.pixel(32, 32).alpha() > 0.9, "centre should be geometry");
        assert!(fb.pixel(32, 60).alpha() > 0.9, "bottom should be ground");
        assert!(fb.pixel(2, 2).alpha() < 0.1, "top corner should be sky");
    }

    /// The energy table must be buildable from inside a Rayon worker.
    ///
    /// It lives behind a `OnceLock`, and the first shading call that needs it
    /// can easily come from a render thread. If its construction were itself
    /// Rayon-parallel, that worker would block on nested work only its own
    /// starved pool could steal, while every other worker blocked on the
    /// `OnceLock` — a deadlock that surfaces or hides depending on which test
    /// or frame happens to touch the table first.
    ///
    /// Building it directly (rather than through the `OnceLock`) is what makes
    /// this test meaningful: it reproduces the nesting regardless of whether
    /// the process-wide table already exists.
    #[test]
    fn the_energy_table_builds_from_inside_a_worker_pool() {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(2)
            .build()
            .unwrap();
        pool.install(|| {
            (0..4).into_par_iter().for_each(|_| {
                let table = tequila_materials::energy::GgxEnergyTable::build();
                assert!(table.white_directional(0.7, 0.25) > 0.0);
            });
        });
    }

    #[test]
    fn an_empty_framebuffer_is_handled_gracefully() {
        let mut fb = Framebuffer::new(0, 0);
        render_pass(
            &test_scene(),
            &mut fb,
            &context(),
            &ProgressiveSchedule::default(),
            1,
        );
        assert_eq!(fb.pixels().len(), 0);
    }

    /// Adaptive sampling must stop spending samples on converged pixels once
    /// it is active (§4.4).
    #[test]
    fn adaptive_sampling_retires_converged_pixels() {
        // A scene that is entirely flat background converges immediately.
        let scene = SceneSnapshot::new(
            Vec::new(),
            Vec::new(),
            vec![Material::default()],
            Arc::new(Environment::uniform(Spectrum::ONE)),
        );
        let flat = AnalyticIntersector::new(scene);

        let mut schedule = ProgressiveSchedule::default();
        schedule.update(10_000); // refining
        let mut fb = Framebuffer::new(16, 16);
        let context = context();

        render_pass(&flat, &mut fb, &context, &schedule, 32);
        assert!(
            fb.converged_fraction() > 0.9,
            "a noiseless image should converge: {}",
            fb.converged_fraction()
        );

        let before = fb.max_samples();
        render_pass(&flat, &mut fb, &context, &schedule, 8);
        assert_eq!(
            fb.max_samples(),
            before,
            "retired pixels must stop consuming samples"
        );
    }

    /// Adaptive sampling must not change the picture.
    ///
    /// §4.4 halts a pixel once its relative error falls below a threshold, which
    /// is only safe if the estimate is honest: a variance estimate that runs
    /// low retires pixels while they are still noisy, and the result is an
    /// image that *reports* convergence while visibly differing from the truth.
    /// The reported percentage cannot catch that, because it is computed from
    /// the same estimate.
    ///
    /// So this compares against a reference rendered with adaptive sampling
    /// off — the only check that does not assume the thing it is testing.
    #[test]
    fn adaptive_sampling_converges_to_the_same_image_as_brute_force() {
        // A sphere on a plane: the shadow edge and the terminator are exactly
        // where a variance estimate is most likely to be flattered by a run of
        // similar samples, so they are where premature retirement shows.
        let intersector = test_scene();
        let camera = Camera::look_at(vec3(3.2, 2.1, 4.4), vec3(0.0, 0.95, 0.0), Vec3::Y);
        let (width, height) = (96, 64);
        let samples = 512;

        let render = |threshold: f32| {
            let mut framebuffer = Framebuffer::new(width, height);
            let mut schedule = ProgressiveSchedule::default();
            // At rest, which is the regime adaptive sampling runs in: it never
            // retires anything while the user is still moving the camera.
            schedule.update(60_000);
            let context = RenderContext {
                camera,
                config: IntegratorConfig {
                    adaptive_sampling_threshold: threshold,
                    ..IntegratorConfig::product()
                },
                scene_seed: 7,
                reject_fireflies: true,
                region: None,
                ..Default::default()
            };
            // Eight at a time, so the retirement pass between batches has
            // something to act on — which is how the real renderer drives it.
            for _ in 0..samples / 8 {
                render_pass(&intersector, &mut framebuffer, &context, &schedule, 8);
            }
            framebuffer
        };

        // A threshold of zero never retires anything: every pixel gets all of
        // them, which makes this run the reference.
        let reference = render(0.0);
        let adaptive = render(0.01);

        let retired: Vec<(usize, usize)> = (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .filter(|&(x, y)| adaptive.pixel(x, y).retired)
            .collect();
        assert!(
            retired.len() > width * height / 50,
            "too few pixels retired for this to prove anything: {}",
            retired.len()
        );

        // Only the retired pixels are compared. They are the ones the
        // criterion made a claim about — the rest received every sample and
        // would differ from the reference only by ordinary Monte Carlo noise,
        // which would dilute exactly the signal being looked for.
        let (mut worst, mut sum_squared) = (0.0f32, 0.0f64);
        for &(x, y) in &retired {
            let truth = reference.pixel(x, y).mean();
            let early = adaptive.pixel(x, y).mean();
            // Relative, so a bright highlight cannot hide a large absolute
            // error the way an RMS over raw radiance would.
            let scale = truth.luminance().max(0.02);
            let error = (early.r - truth.r)
                .abs()
                .max((early.g - truth.g).abs())
                .max((early.b - truth.b).abs())
                / scale;
            worst = worst.max(error);
            sum_squared += (error * error) as f64;
        }
        let rms = (sum_squared / retired.len() as f64).sqrt();

        // Both runs are Monte Carlo estimates, so they differ by their own
        // noise even where neither stopped early. The bounds sit about five
        // times above what this scene actually produces (rms 0.002, worst
        // 0.013) — enough headroom that an unrelated sampler change does not
        // trip it, tight enough that a criterion which started retiring pixels
        // while they were still noisy would.
        assert!(
            rms < 0.01,
            "retired pixels differ from the converged image: rms {rms:.4}, worst {worst:.3}, \
             over {} pixels",
            retired.len()
        );
        assert!(
            worst < 0.06,
            "a pixel retired far from its converged value: worst {worst:.3}, rms {rms:.4}"
        );
    }
}
