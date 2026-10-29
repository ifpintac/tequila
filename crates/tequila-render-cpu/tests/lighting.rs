//! Analytic lights, rendered (`docs/01-RENDERER-CORE.md` §5.1).
//!
//! The claim a light has to satisfy is not "it makes things brighter". It is
//! that it delivers the *right* amount of light, and there is an independent way
//! to check that: a rectangular area light and a rectangle of emissive geometry
//! in the same place with the same radiance are the same physical object,
//! reached by two entirely different code paths — one through next-event
//! estimation and the light's own sampling, the other through BSDF sampling
//! finding the emitter by chance. They must converge to the same image.
//!
//! That comparison is worth far more than a threshold, because it shares almost
//! no code between the two sides. A sampler checked against its own density
//! agrees with itself no matter what it does; a sampler checked against a
//! different estimator of the same quantity does not.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::MaterialType;
use tequila_materials::openpbr::Material;
use tequila_render_core::camera::Camera;
use tequila_render_core::environment::Environment;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::light::{AreaLight, Light, LightKind, LightSet, PointLight, SpotLight};
use tequila_render_core::scene::{Instance, Mesh, SceneSnapshot};
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use std::sync::Arc;

const SIZE: usize = 40;

/// A floor to catch the light, facing up.
fn floor(half: f32) -> Mesh {
    let positions = vec![
        vec3(-half, 0.0, -half),
        vec3(half, 0.0, -half),
        vec3(half, 0.0, half),
        vec3(-half, 0.0, half),
    ];
    Mesh::new(positions, vec![0, 2, 1, 0, 3, 2]).with_normals(vec![Vec3::Y; 4])
}

/// A rectangle in the plane `y = height`, facing down.
fn panel(half_u: f32, half_v: f32, height: f32) -> Mesh {
    let positions = vec![
        vec3(-half_u, height, -half_v),
        vec3(half_u, height, -half_v),
        vec3(half_u, height, half_v),
        vec3(-half_u, height, half_v),
    ];
    Mesh::new(positions, vec![0, 1, 2, 0, 2, 3]).with_normals(vec![-Vec3::Y; 4])
}

fn matte() -> Material {
    Material {
        base_color: Spectrum::splat(0.5),
        ..MaterialType::Diffuse.defaults()
    }
}

fn camera() -> Camera {
    Camera::look_at(vec3(0.0, 2.6, 3.4), vec3(0.0, 0.0, 0.0), Vec3::Y)
}

fn render(scene: SceneSnapshot, samples: u32) -> Framebuffer {
    render_from(scene, samples, camera())
}

fn render_from(scene: SceneSnapshot, samples: u32, camera: Camera) -> Framebuffer {
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let mut framebuffer = Framebuffer::new(SIZE, SIZE);
    let context = RenderContext {
        camera,
        ..RenderContext::default()
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

fn dark() -> Arc<Environment> {
    Arc::new(Environment::uniform(Spectrum::ZERO))
}

fn mean(image: &Framebuffer) -> f32 {
    let mut total = 0.0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            total += image.pixel(x, y).mean().luminance();
        }
    }
    total / (SIZE * SIZE) as f32
}

/// A floor lit by an analytic area light.
fn with_area_light(radiance: f32, half: f32, height: f32) -> SceneSnapshot {
    let light = AreaLight {
        position: vec3(0.0, height, 0.0),
        half_u: vec3(half, 0.0, 0.0),
        half_v: vec3(0.0, 0.0, half),
        radiance: Spectrum::splat(radiance),
        disc: false,
        two_sided: false,
        spread: std::f32::consts::PI,
    };
    SceneSnapshot::new(
        vec![Arc::new(floor(6.0))],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
        vec![matte()],
        dark(),
    )
    .with_lights(Arc::new(LightSet::new(vec![Light::new(LightKind::Area(
        light,
    ))])))
}

/// The same floor, lit by a rectangle of emissive geometry in the same place.
fn with_emissive_panel(radiance: f32, half: f32, height: f32) -> SceneSnapshot {
    let emitter = Material {
        emission_color: Spectrum::ONE,
        emission_luminance: radiance,
        ..MaterialType::Emissive.defaults()
    };
    SceneSnapshot::new(
        vec![Arc::new(floor(6.0)), Arc::new(panel(half, half, height))],
        vec![
            Instance::new(InstanceId(0), MeshId(0), MaterialId(0)),
            Instance::new(InstanceId(1), MeshId(1), MaterialId(1)),
        ],
        vec![matte(), emitter],
        dark(),
    )
}

/// The comparison this file exists for.
///
/// Two independent routes to the same physical answer: next-event estimation
/// through the light's spherical-rectangle sampler, and BSDF sampling stumbling
/// onto an emissive triangle. If the light's radiometry is wrong by a factor —
/// a missing cosine, a solid angle mistaken for an area — these disagree, and
/// nothing else in the suite would notice.
#[test]
fn an_area_light_matches_the_same_light_as_emissive_geometry() {
    let (radiance, half, height) = (6.0, 1.0, 2.0);

    let analytic = render(with_area_light(radiance, half, height), 256);
    let geometric = render(with_emissive_panel(radiance, half, height), 4096);

    // Compare only the floor, and only where both routes see it: the emissive
    // panel is visible to the camera in one image and absent from the other,
    // and that difference is expected rather than an error.
    let mut compared = 0;
    let mut total_error = 0.0f32;
    for y in (SIZE / 2)..SIZE {
        for x in 0..SIZE {
            let a = analytic.pixel(x, y).mean().luminance();
            let b = geometric.pixel(x, y).mean().luminance();
            if b < 1e-4 {
                continue;
            }
            total_error += (a - b).abs() / b;
            compared += 1;
        }
    }

    assert!(compared > 200, "only {compared} pixels were comparable");
    let error = total_error / compared as f32;
    assert!(
        error < 0.06,
        "an analytic area light and the same light as geometry differ by {:.1}% \
         on average across {compared} pixels; one of the two routes has the \
         radiometry wrong",
        error * 100.0
    );
}

/// Doubling a light's radiance doubles what arrives. Linear, with no threshold
/// or falloff hiding in the middle.
#[test]
fn light_output_is_linear_in_radiance() {
    let single = mean(&render(with_area_light(4.0, 1.0, 2.0), 128));
    let double = mean(&render(with_area_light(8.0, 1.0, 2.0), 128));

    assert!(single > 1e-4, "the single-strength render is black");
    assert!(
        (double / single - 2.0).abs() < 0.02,
        "doubling the radiance multiplied the image by {:.3}",
        double / single
    );
}

/// A point light obeys the inverse-square law in a render, not only in a unit
/// test of its own sampler.
#[test]
fn a_point_light_falls_off_with_distance_in_a_render() {
    let build = |height: f32| {
        SceneSnapshot::new(
            vec![Arc::new(floor(6.0))],
            vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
            vec![matte()],
            dark(),
        )
        .with_lights(Arc::new(LightSet::new(vec![Light::new(LightKind::Point(
            PointLight {
                position: vec3(0.0, height, 0.0),
                intensity: Spectrum::splat(40.0),
                radius: 0.0,
            },
        ))])))
    };

    let near = mean(&render(build(1.5), 64));
    let far = mean(&render(build(3.0), 64));
    assert!(near > 1e-4, "the near render is black");

    // Not exactly four: the floor is a plane, so moving the light up also
    // spreads its light over more of the frame. It must be substantially
    // dimmer, and in the right direction.
    let ratio = near / far;
    assert!(
        ratio > 2.0 && ratio < 5.0,
        "doubling the height changed the image by {ratio:.2}x; the inverse-square \
         law puts it near 4"
    );
}

/// A spotlight lights inside its cone and not outside it, and the edge is soft.
#[test]
fn a_spotlight_lights_its_cone_and_not_beyond() {
    let scene = SceneSnapshot::new(
        vec![Arc::new(floor(6.0))],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
        vec![matte()],
        dark(),
    )
    .with_lights(Arc::new(LightSet::new(vec![Light::new(LightKind::Spot(
        SpotLight {
            position: vec3(0.0, 2.0, 0.0),
            direction: -Vec3::Y,
            intensity: Spectrum::splat(60.0),
            inner_angle: 0.25,
            outer_angle: 0.45,
            falloff: 1.0,
        },
    ))])));

    let image = render(scene, 64);


    // Found rather than assumed: where the pool lands in frame depends on the
    // projection, and a test that hard-codes a pixel is testing the camera.
    let mut brightest = 0.0f32;
    let mut lit = 0usize;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let value = image.pixel(x, y).mean().luminance();
            brightest = brightest.max(value);
            if value > 1e-3 {
                lit += 1;
            }
        }
    }

    assert!(brightest > 1e-3, "nothing in the frame is lit at all");
    // A pool, not a wash: a 26-degree cone two units up lights a small part of
    // a six-unit floor.
    let fraction = lit as f32 / (SIZE * SIZE) as f32;
    assert!(
        fraction > 0.005 && fraction < 0.5,
        "the spotlight lit {:.1}% of the frame",
        fraction * 100.0
    );

    // The far corners of the floor are well outside the cone.
    for (x, y) in [(0usize, SIZE - 1), (SIZE - 1, SIZE - 1)] {
        let corner = image.pixel(x, y).mean().luminance();
        assert!(
            corner < brightest * 0.05,
            "({x}, {y}) is outside the cone but received {corner:.5} against a \
             peak of {brightest:.5}"
        );
    }
}

/// A light hidden from the camera still lights the scene (§5.4). This is the
/// invisible fill every product photograph uses.
#[test]
fn a_light_hidden_from_the_camera_still_lights() {
    let visible = with_area_light(6.0, 1.0, 2.0);
    let hidden = {
        let light = Light::new(LightKind::Area(AreaLight {
            position: vec3(0.0, 2.0, 0.0),
            half_u: vec3(1.0, 0.0, 0.0),
            half_v: vec3(0.0, 0.0, 1.0),
            radiance: Spectrum::splat(6.0),
            disc: false,
            two_sided: false,
            spread: std::f32::consts::PI,
        }))
        .with_visibility(tequila_render_core::light::Visibility::HIDDEN);
        SceneSnapshot::new(
            vec![Arc::new(floor(6.0))],
            vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
            vec![matte()],
            dark(),
        )
        .with_lights(Arc::new(LightSet::new(vec![light])))
    };

    let a = mean(&render(visible, 96));
    let b = mean(&render(hidden, 96));
    assert!(a > 1e-4);
    assert!(
        (a - b).abs() < a * 0.02,
        "hiding a light from the camera changed the lighting: {a:.5} against {b:.5}"
    );
}

/// Many lights of equal power must give the same result as one light of their
/// combined power in the same place — the selection must not lose or duplicate
/// energy as the count crosses §5.3's threshold.
#[test]
fn splitting_a_light_into_many_conserves_its_output() {
    let make = |count: usize| {
        // `count` coincident lights, each with a fraction of the radiance.
        let each = 6.0 / count as f32;
        let lights: Vec<Light> = (0..count)
            .map(|_| {
                Light::new(LightKind::Area(AreaLight {
                    position: vec3(0.0, 2.0, 0.0),
                    half_u: vec3(1.0, 0.0, 0.0),
                    half_v: vec3(0.0, 0.0, 1.0),
                    radiance: Spectrum::splat(each),
                    disc: false,
                    two_sided: false,
                    spread: std::f32::consts::PI,
                }))
            })
            .collect();
        SceneSnapshot::new(
            vec![Arc::new(floor(6.0))],
            vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
            vec![matte()],
            dark(),
        )
        .with_lights(Arc::new(LightSet::new(lights)))
    };

    let one = mean(&render(make(1), 192));
    // Either side of the eight-light threshold, so both selection strategies
    // are covered.
    for count in [4usize, 12] {
        let many = mean(&render(make(count), 192));
        assert!(
            (many / one - 1.0).abs() < 0.05,
            "{count} lights sharing one light's power gave {many:.5} against \
             {one:.5}; the selection is losing or duplicating energy"
        );
    }
}

/// A scene with no lights and no environment renders black rather than
/// producing something from nothing.
#[test]
fn a_scene_with_no_lights_is_black() {
    let scene = SceneSnapshot::new(
        vec![Arc::new(floor(6.0))],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
        vec![matte()],
        dark(),
    );
    let image = render(scene, 32);
    assert!(
        mean(&image) < 1e-6,
        "an unlit scene rendered at {:.6}",
        mean(&image)
    );
}


/// The two selection strategies are estimators of the *same* quantity, so they
/// have to agree on the image.
///
/// §5.3 switches from a power-weighted alias table to an importance-traversed
/// hierarchy at eight lights, and the switch is invisible to the integrator.
/// This checks that it is invisible in the result too — that the hierarchy is
/// a different route to the same answer rather than a different answer.
///
/// Worth being precise about what this does *not* catch. A hierarchy whose
/// reported probabilities are not a distribution can still produce the right
/// mean here, because the error cancels across a symmetric ring of lights;
/// `hierarchy_probabilities_sum_to_one` is what catches that, and it does —
/// re-introducing the bug it was written for fails that test and leaves this
/// one green.
#[test]
fn the_two_light_selection_strategies_agree() {
    use tequila_render_core::light::SelectionStrategy;

    // Spread out, at different heights and distances, so the hierarchy's
    // importance heuristic has something to discriminate on. Sixteen identical
    // co-located lights would agree trivially.
    let lights: Vec<Light> = (0..16)
        .map(|i| {
            let angle = i as f32 / 16.0 * std::f32::consts::TAU;
            Light::new(LightKind::Point(PointLight {
                position: vec3(angle.cos() * 3.0, 1.0 + (i % 4) as f32 * 0.7, angle.sin() * 3.0),
                intensity: Spectrum::splat(2.0 + (i % 3) as f32),
                radius: 0.0,
            }))
        })
        .collect();

    let build = |strategy: SelectionStrategy| {
        SceneSnapshot::new(
            vec![Arc::new(floor(6.0))],
            vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
            vec![matte()],
            dark(),
        )
        .with_lights(Arc::new(LightSet::with_strategy(lights.clone(), strategy)))
    };

    let hierarchy = render(build(SelectionStrategy::Hierarchy), 512);
    let alias = render(build(SelectionStrategy::Alias), 512);

    let a = mean(&hierarchy);
    let b = mean(&alias);
    assert!(a > 1e-4, "nothing was lit");
    assert!(
        (a / b - 1.0).abs() < 0.02,
        "the hierarchy renders {a} where the alias table renders {b}"
    );
}


/// §5.4's visibility flags, which exist for a specific studio move: a big soft
/// key that fills the reflections in a product without washing it out
/// diffusely, or a fill that lights it without adding a second shadow.
///
/// The flags restrict which *lobes* a light reaches, so the check is that a
/// surface with both a diffuse and a specular lobe loses the right part of its
/// shading — not that it goes black, which is what refusing the whole light
/// would do and what a cruder implementation would give.
#[test]
fn light_visibility_flags_restrict_the_lobes_a_light_reaches() {
    use tequila_render_core::light::Visibility;

    // Glossy plastic: a diffuse base under a specular reflection, so there are
    // two lobes to tell apart.
    let plastic = Material {
        base_color: Spectrum::splat(0.5),
        specular_roughness: 0.25,
        ..MaterialType::Plastic.defaults()
    };
    let build = |visibility: Visibility| {
        SceneSnapshot::new(
            vec![Arc::new(floor(6.0))],
            vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
            vec![plastic.clone()],
            dark(),
        )
        .with_lights(Arc::new(LightSet::new(vec![Light::new(LightKind::Area(
            AreaLight {
                position: vec3(0.0, 3.0, 0.0),
                half_u: vec3(1.5, 0.0, 0.0),
                half_v: vec3(0.0, 0.0, 1.5),
                radiance: Spectrum::splat(8.0),
                disc: false,
                two_sided: false,
                spread: std::f32::consts::PI,
            },
        ))
        .with_visibility(visibility)])))
    };

    let both = mean(&render(build(Visibility::default()), 256));
    let specular_only = mean(&render(
        build(Visibility {
            diffuse: false,
            ..Visibility::default()
        }),
        256,
    ));
    let diffuse_only = mean(&render(
        build(Visibility {
            specular: false,
            ..Visibility::default()
        }),
        256,
    ));

    assert!(both > 1e-4, "nothing was lit at all");
    // Each flag removes something, and neither removes everything.
    for (name, value) in [("specular only", specular_only), ("diffuse only", diffuse_only)] {
        assert!(value > 0.0, "{name} removed the whole light");
        assert!(value < both * 0.999, "{name} removed nothing: {value} of {both}");
    }
    // And together they account for the whole: the layer stack splits the
    // shading, it does not duplicate or lose part of it.
    let split = specular_only + diffuse_only;
    assert!(
        (split / both - 1.0).abs() < 0.02,
        "the two halves sum to {split} against {both} for the whole light"
    );

    // Switching both off leaves the light sampled but contributing nothing.
    // (`Visibility::HIDDEN` is a different thing — hidden from the *camera*,
    // which is the softbox that lights the scene without appearing in it.)
    let neither = mean(&render(
        build(Visibility {
            diffuse: false,
            specular: false,
            ..Visibility::default()
        }),
        64,
    ));
    assert!(
        neither < 1e-6,
        "a light that reaches no lobe still lit the scene: {neither}"
    );
}


/// A light the camera can see needs geometry to see, and that geometry must not
/// light the scene a second time.
///
/// This is the trap the `light_proxy` flag exists for. An emissive surface
/// placed where an analytic light is would be sampled by BSDF rays *and* by
/// next-event estimation against the analytic light, so a softbox would come
/// out twice as bright the moment it was made visible — a change that looks
/// like it should only affect what is in frame.
#[test]
fn a_visible_light_is_seen_without_being_counted_twice() {
    let light = AreaLight {
        position: vec3(0.0, 2.0, 0.0),
        half_u: vec3(1.0, 0.0, 0.0),
        half_v: vec3(0.0, 0.0, 1.0),
        radiance: Spectrum::splat(6.0),
        disc: false,
        two_sided: false,
        spread: std::f32::consts::PI,
    };
    let proxy = Mesh::new(
        vec![
            light.position - light.half_u - light.half_v,
            light.position + light.half_u - light.half_v,
            light.position + light.half_u + light.half_v,
            light.position - light.half_u + light.half_v,
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
    .with_normals(vec![light.normal(); 4]);

    let proxy = Arc::new(proxy);
    let visible_scene = move || SceneSnapshot::new(
        vec![Arc::new(floor(6.0)), proxy.clone()],
        vec![
            Instance::new(InstanceId(0), MeshId(0), MaterialId(0)),
            Instance::new(InstanceId(1), MeshId(1), MaterialId(1)).as_light_proxy(),
        ],
        vec![
            matte(),
            Material {
                emission_color: Spectrum::ONE,
                emission_luminance: 6.0,
                ..MaterialType::Emissive.defaults()
            },
        ],
        dark(),
    )
    .with_lights(Arc::new(LightSet::new(vec![Light::new(LightKind::Area(
        light,
    ))])));

    // The floor is lit exactly as it was: the proxy adds nothing to it.
    let lit = |image: &Framebuffer| -> f32 {
        // The lower rows, which see floor rather than the panel above it.
        let mut total = 0.0;
        for y in (SIZE * 3 / 4)..SIZE {
            for x in 0..SIZE {
                total += image.pixel(x, y).mean().luminance();
            }
        }
        total
    };
    let without = render(with_area_light(6.0, 1.0, 2.0), 192);
    let with = render(visible_scene(), 192);
    let (a, b) = (lit(&with), lit(&without));
    assert!(b > 1e-4, "the floor was not lit at all");
    assert!(
        (a / b - 1.0).abs() < 0.03,
        "making the light visible changed how it lights: {a} against {b}"
    );

    // And it is actually in frame, or the flag is being tested on something
    // nobody can see. From below, because the panel emits downwards — seen
    // from above it is a back face and emits nothing, which is correct and
    // would make this half of the test pass for the wrong reason.
    let below = Camera::look_at(vec3(0.0, 0.2, 2.2), vec3(0.0, 2.0, 0.0), Vec3::Y);
    let brightest = |image: &Framebuffer| -> f32 {
        let mut brightest = 0.0f32;
        for y in 0..SIZE {
            for x in 0..SIZE {
                brightest = brightest.max(image.pixel(x, y).mean().luminance());
            }
        }
        brightest
    };
    let seen = render_from(visible_scene(), 32, below);
    let unseen = render_from(with_area_light(6.0, 1.0, 2.0), 32, below);
    assert!(
        brightest(&seen) > brightest(&unseen) * 2.0,
        "the proxy is not visible: {} against {}",
        brightest(&seen),
        brightest(&unseen)
    );
}

/// Nor may making a light visible shadow the rest of the lighting. Its
/// stand-in is there to be seen, so a softbox hung over the product does not
/// blot out the sky behind it the moment it is made visible. It did, twice
/// over: shadow rays stopped at it outside a cutaway, and every bounce that
/// met it ended there. Cycles' lights, which it stands in for there, block
/// nothing.
#[test]
fn a_visible_light_does_not_shadow_the_rest_of_the_lighting() {
    // Dim, so the floor's light is mostly the sky, which is what a panel
    // hung there could take away.
    let light = AreaLight {
        position: vec3(0.0, 2.0, 0.0),
        half_u: vec3(1.0, 0.0, 0.0),
        half_v: vec3(0.0, 0.0, 1.0),
        radiance: Spectrum::splat(1.0),
        disc: false,
        two_sided: false,
        spread: std::f32::consts::PI,
    };
    // Under a sky as well as the light, so there is something to shadow.
    // `Some(true)` hangs the light's stand-in where the light is, and
    // `Some(false)` an ordinary black panel of the same shape.
    let scene = |stand_in: Option<bool>| {
        let mut meshes = vec![Arc::new(floor(6.0))];
        let mut instances = vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))];
        if let Some(stand_in) = stand_in {
            meshes.push(Arc::new(panel(1.0, 1.0, 2.0)));
            instances.push(if stand_in {
                Instance::new(InstanceId(1), MeshId(1), MaterialId(1)).as_light_proxy()
            } else {
                Instance::new(InstanceId(1), MeshId(1), MaterialId(2))
            });
        }
        SceneSnapshot::new(
            meshes,
            instances,
            vec![
                matte(),
                Material {
                    emission_color: Spectrum::ONE,
                    emission_luminance: 1.0,
                    ..MaterialType::Emissive.defaults()
                },
                Material {
                    base_color: Spectrum::ZERO,
                    ..MaterialType::Diffuse.defaults()
                },
            ],
            Arc::new(Environment::uniform(Spectrum::ONE)),
        )
        .with_lights(Arc::new(LightSet::new(vec![Light::new(LightKind::Area(light))])))
    };

    // The floor right under the panel, which the camera sees past its edge.
    let film = camera()
        .project(vec3(0.0, 0.0, 0.0), 1.0)
        .expect("the floor's centre is in frame");
    let px = ((film.x * SIZE as f32) as usize).min(SIZE - 1);
    let py = ((film.y * SIZE as f32) as usize).min(SIZE - 1);
    let under = |image: &Framebuffer| -> f32 {
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

    let unseen = under(&render(scene(None), 128));
    let seen = under(&render(scene(Some(true)), 128));
    assert!(
        (seen / unseen - 1.0).abs() < 0.03,
        "making the light visible darkened the floor under it: {seen} against {unseen}"
    );
    // And something there that did shadow the floor would be caught here.
    let shadowing = under(&render(scene(Some(false)), 128));
    assert!(
        shadowing < unseen * 0.9,
        "a real panel took nothing away either, so this proves nothing: \
         {shadowing} against {unseen}"
    );
}


/// An IES luminaire, rendered (§5.1).
///
/// The claim a photometric file has to satisfy is that the *shape* of the beam
/// on the floor comes from the measurement rather than from a formula. So the
/// profile here is one nothing analytic produces — bright on the axis, dark at
/// 30°, bright again at 60° — and what is checked is that the floor shows those
/// rings. A light that ignored the table would light the floor monotonically
/// however bright it was.
#[test]
fn an_ies_luminaire_puts_its_measured_pattern_on_the_floor() {
    use tequila_render_core::ies::IesProfile;
    use tequila_render_core::light::IesLight;

    // Ten degrees apart, rotationally symmetric, with a hole in the middle of
    // the beam.
    let mut text = String::from("TILT=NONE
1 -1 1 10 1 1 2 0 0 0
1 1 100
");
    for i in 0..10 {
        text.push_str(&format!("{} ", i as f32 * 10.0));
    }
    text.push_str("
0
");
    for i in 0..10 {
        let degrees = i as f32 * 10.0;
        // A ring: full on the axis, out at 30°, back at 60°, out past 70°.
        let value = match i {
            0..=1 => 400.0,
            2..=4 => 10.0,
            5..=7 => 400.0,
            _ => 0.0,
        };
        let _ = degrees;
        text.push_str(&format!("{value} "));
    }
    text.push_str("
");
    let profile = Arc::new(IesProfile::parse(&text).expect("a valid profile"));

    let scene = SceneSnapshot::new(
        vec![Arc::new(floor(6.0))],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
        vec![matte()],
        dark(),
    )
    .with_lights(Arc::new(LightSet::new(vec![Light::new(LightKind::Ies(
        IesLight {
            position: vec3(0.0, 2.0, 0.0),
            direction: -Vec3::Y,
            right: Vec3::X,
            intensity: Spectrum::ONE,
            profile: profile.clone(),
        },
    ))])));

    // Straight down at the floor, so the rings are concentric in the frame.
    let overhead = Camera::look_at(vec3(0.0, 6.0, 0.001), Vec3::ZERO, Vec3::Y);
    let image = render_from(scene, 128, overhead);

    // Sample the floor along a radius, converting each world radius to the
    // pixel it lands on rather than guessing at screen positions.
    let sample = |radius: f32| -> f32 {
        let film = overhead
            .project(vec3(radius, 0.0, 0.0), 1.0)
            .unwrap_or(tequila_core::math::vec2(0.5, 0.5));
        let x = ((film.x * SIZE as f32) as usize).min(SIZE - 1);
        let y = ((film.y * SIZE as f32) as usize).min(SIZE - 1);
        image.pixel(x, y).mean().luminance()
    };

    // The luminaire is 2 units up, so a vertical angle of `a` lands at
    // `2 * tan(a)` from the origin.
    let at = |degrees: f32| sample(2.0 * degrees.to_radians().tan());

    let axis = at(5.0);
    let hole = at(30.0);
    let ring = at(60.0);
    assert!(axis > 0.0, "the luminaire lit nothing at all");
    assert!(
        hole < axis * 0.2,
        "the dark band of the measurement is not dark: {hole} against {axis} on \
         the axis"
    );
    assert!(
        ring > hole * 4.0,
        "the outer ring of the measurement is missing: {ring} against {hole} in \
         the dark band"
    );
}

/// The aiming axis, end to end. A profile applied about the wrong axis turns
/// every downlight into an uplight, and the scene merely goes dark — which
/// reads as a light that is too dim rather than as one pointing the wrong way.
#[test]
fn an_ies_luminaire_aims_where_the_node_points() {
    use tequila_render_core::ies::IesProfile;
    use tequila_render_core::light::IesLight;

    let text = "TILT=NONE
1 -1 1 3 1 1 2 0 0 0
1 1 100
0 90 180
0
500 0 0
";
    let profile = Arc::new(IesProfile::parse(text).expect("a valid profile"));

    let build = |direction: Vec3| {
        SceneSnapshot::new(
            vec![Arc::new(floor(6.0))],
            vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
            vec![matte()],
            dark(),
        )
        .with_lights(Arc::new(LightSet::new(vec![Light::new(LightKind::Ies(
            IesLight {
                position: vec3(0.0, 2.0, 0.0),
                direction,
                right: Vec3::X,
                intensity: Spectrum::ONE,
                profile: profile.clone(),
            },
        ))])))
    };

    let down = mean(&render(build(-Vec3::Y), 64));
    let up = mean(&render(build(Vec3::Y), 64));
    assert!(down > 1e-4, "aimed at the floor, the floor is unlit");
    assert!(
        up < down * 0.05,
        "aimed at the ceiling, the floor still gets {up} against {down}"
    );
}
