//! Pictures of lamps in mirror-smooth surfaces and of parts that cast no
//! shadow, written to the folder `TEQUILA_PICTURES` names, each named after
//! what it shows with `TEQUILA_PICTURE_LABEL` in front, for comparing a
//! build with another. Run by hand (`cargo test -p tequila-render-cpu
//! --release --test glossy_finished_pictures -- --ignored --nocapture`).
//! `TEQUILA_PICTURE_SAMPLES` sets the samples (256).

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Transform, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::fresnel::metals;
use tequila_materials::material_type::MaterialType;
use tequila_materials::openpbr::Material;
use tequila_render_core::environment::Environment;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::light::{AreaLight, DistantLight, Light, LightKind, LightSet, Visibility};
use tequila_render_core::scene::{Instance, Mesh, SceneSnapshot};
use tequila_render_core::settings::IntegratorConfig;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use std::sync::Arc;

const SIZE: (usize, usize) = (480, 320);

fn quad(corners: [Vec3; 4], facing: Vec3) -> Mesh {
    let (a, b, c) = (corners[0], corners[1], corners[2]);
    let indices = if (b - a).cross(c - a).dot(facing) > 0.0 { vec![0, 1, 2, 0, 2, 3] } else { vec![0, 2, 1, 0, 3, 2] };
    Mesh::new(corners.to_vec(), indices).with_normals(vec![facing; 4])
}

fn lamp(at: Vec3, facing: Vec3, half_u: f32, half_v: f32, radiance: f32, disc: bool) -> AreaLight {
    let facing = facing.normalize();
    let across = if facing.y.abs() < 0.9 { Vec3::Y.cross(facing).normalize() } else { Vec3::X.cross(facing).normalize() };
    let up = facing.cross(across).normalize();
    let (u, v) = if across.cross(up).dot(facing) > 0.0 { (across * half_u, up * half_v) } else { (up * half_v, across * half_u) };
    AreaLight { position: at, half_u: u, half_v: v, radiance: Spectrum::splat(radiance), disc, two_sided: false, spread: std::f32::consts::PI }
}

fn render(scene: SceneSnapshot, camera: tequila_render_core::camera::Camera, samples: u32) -> Vec<Spectrum> {
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let mut framebuffer = Framebuffer::new(SIZE.0, SIZE.1);
    let context = RenderContext { camera, config: IntegratorConfig::product(), reject_fireflies: false, ..RenderContext::default() };
    let start = std::time::Instant::now();
    for _ in 0..samples {
        render_pass(&intersector, &mut framebuffer, &context, &Default::default(), 1);
    }
    println!("  {samples} samples in {:.1} s", start.elapsed().as_secs_f32());
    let mut out = Vec::with_capacity(SIZE.0 * SIZE.1);
    for y in 0..SIZE.1 {
        for x in 0..SIZE.0 {
            out.push(framebuffer.pixel(x, y).mean().sanitized());
        }
    }
    out
}

fn picture(name: &str, image: &[Spectrum]) {
    let Some(folder) = std::env::var_os("TEQUILA_PICTURES").map(std::path::PathBuf::from) else { return };
    std::fs::create_dir_all(&folder).ok();
    let label = std::env::var("TEQUILA_PICTURE_LABEL").unwrap_or_default();
    let pixels = tequila_color::ImageStyle::default().develop_through(SIZE.0, SIZE.1, image, &tequila_color::DisplayPipeline::default());
    let path = folder.join(format!("{label}{name}.png"));
    let file = std::fs::File::create(&path).expect("create");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), SIZE.0 as u32, SIZE.1 as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().expect("header").write_image_data(&pixels).expect("data");
    println!("  -> {}", path.display());
}

fn samples() -> u32 {
    std::env::var("TEQUILA_PICTURE_SAMPLES").ok().and_then(|s| s.parse().ok()).unwrap_or(256)
}

/// A white floor before a mirror wall, a chrome ball on it, a disc softbox
/// the camera does not see aimed at the wall, a square one beside the ball,
/// and a low sun with a disc, in a dark sky: the softboxes and the sun in
/// the ball, the disc softbox in the wall and in the wall seen in the ball,
/// and the light the wall throws back onto the floor.
#[test]
#[ignore]
fn lamps_in_mirrors_studio_picture() {
    let floor = quad([vec3(-5.0, 0.0, -1.5), vec3(5.0, 0.0, -1.5), vec3(5.0, 0.0, 5.0), vec3(-5.0, 0.0, 5.0)], Vec3::Y);
    let wall = quad([vec3(-5.0, 0.0, -1.5), vec3(5.0, 0.0, -1.5), vec3(5.0, 2.5, -1.5), vec3(-5.0, 2.5, -1.5)], Vec3::Z);
    let ball = tequila_scene::demo::uv_sphere(0.4, 128, 64);
    let mut white = MaterialType::Diffuse.defaults();
    white.base_color = Spectrum::splat(0.6);
    let mut chrome = Material::metal(metals::SILVER, 0.0);
    chrome.base_color = Spectrum::ONE;
    let scene = SceneSnapshot::new(
        vec![Arc::new(floor), Arc::new(wall), Arc::new(ball)],
        vec![
            Instance::new(InstanceId(0), MeshId(0), MaterialId(0)),
            Instance::new(InstanceId(1), MeshId(1), MaterialId(1)),
            Instance::new(InstanceId(2), MeshId(2), MaterialId(1)).with_transform(Transform::translation(vec3(0.7, 0.4, 0.3))),
        ],
        vec![white, chrome],
        Arc::new(Environment::uniform(Spectrum::splat(0.03))),
    );
    let hidden = Light::new(LightKind::Area(lamp(vec3(-1.0, 1.4, 1.0), vec3(0.1, -0.35, -1.0), 0.4, 0.4, 6.0, true))).with_visibility(Visibility::HIDDEN);
    let square = Light::new(LightKind::Area(lamp(vec3(2.2, 1.0, 1.4), vec3(-1.0, -0.3, -0.6), 0.3, 0.45, 6.0, false))).with_visibility(Visibility::HIDDEN);
    let sun = Light::new(LightKind::Distant(DistantLight { direction: vec3(-0.35, -0.3, -1.0).normalize(), irradiance: Spectrum::splat(0.6), half_angle: 1.5f32.to_radians() }));
    let scene = scene.with_lights(Arc::new(LightSet::new(vec![hidden, square, sun])));
    let camera = tequila_render_core::camera::Camera::look_at(vec3(0.3, 1.0, 3.6), vec3(0.2, 0.6, -0.6), Vec3::Y);
    println!("lamps in mirrors:");
    picture("lamps-in-mirrors-studio", &render(scene, camera, samples()));
}

/// Left, a dark glossy floor and a softbox low behind a grey plate that
/// casts no shadow, rising above it; right, a white floor in a pale sky
/// under two dark balls, the near one casting no shadow: the floor mirrors
/// the softbox through the plate's reflection, and is lit under the ball
/// that casts no shadow as in the open.
#[test]
#[ignore]
fn parts_that_cast_no_shadow_studio_picture() {
    let floor = quad([vec3(-5.0, 0.0, -4.0), vec3(5.0, 0.0, -4.0), vec3(5.0, 0.0, 5.0), vec3(-5.0, 0.0, 5.0)], Vec3::Y);
    let white_floor = quad([vec3(0.3, 0.001, -2.5), vec3(3.5, 0.001, -2.5), vec3(3.5, 0.001, 2.0), vec3(0.3, 0.001, 2.0)], Vec3::Y);
    let plate = quad([vec3(-1.5, 0.0, -0.8), vec3(-0.3, 0.0, -0.8), vec3(-0.3, 0.45, -0.8), vec3(-1.5, 0.45, -0.8)], Vec3::Z);
    let ball = tequila_scene::demo::uv_sphere(0.3, 96, 48);
    let mut white = MaterialType::Diffuse.defaults();
    white.base_color = Spectrum::splat(0.75);
    let glossy = Material::plastic(Spectrum::splat(0.03), 0.06);
    let grey = Material::plastic(Spectrum::splat(0.12), 0.5);
    let mut instances = vec![
        Instance::new(InstanceId(0), MeshId(0), MaterialId(0)),
        Instance::new(InstanceId(1), MeshId(1), MaterialId(1)),
        Instance::new(InstanceId(2), MeshId(2), MaterialId(2)),
        Instance::new(InstanceId(3), MeshId(3), MaterialId(2)).with_transform(Transform::translation(vec3(1.2, 0.55, 0.6))),
        Instance::new(InstanceId(4), MeshId(3), MaterialId(2)).with_transform(Transform::translation(vec3(2.4, 0.55, -0.6))),
    ];
    instances[2].casts_shadows = false;
    instances[3].casts_shadows = false;
    let scene = SceneSnapshot::new(
        vec![Arc::new(floor), Arc::new(white_floor), Arc::new(plate), Arc::new(ball)],
        instances,
        vec![glossy, white, grey],
        Arc::new(Environment::uniform(Spectrum::splat(0.5))),
    );
    let softbox = Light::new(LightKind::Area(lamp(vec3(-0.9, 0.55, -1.8), Vec3::Z, 0.5, 0.35, 6.0, false)));
    let scene = scene.with_lights(Arc::new(LightSet::new(vec![softbox])));
    let camera = tequila_render_core::camera::Camera::look_at(vec3(0.5, 1.3, 3.6), vec3(0.5, 0.25, -0.5), Vec3::Y);
    println!("parts that cast no shadow:");
    picture("casts-no-shadow-studio", &render(scene, camera, samples()));
}

/// A chrome ball and a mirror wall on a white floor, a disc softbox the
/// camera does not see, a square one it does, and a low sun with a disc:
/// the softboxes and the sun in the ball, the softbox in the wall seen in
/// the ball and on the floor, and the light the wall throws onto the floor.
#[test]
#[ignore]
fn lamps_in_mirrors_picture() {
    let floor = quad([vec3(-4.0, 0.0, -2.0), vec3(4.0, 0.0, -2.0), vec3(4.0, 0.0, 4.0), vec3(-4.0, 0.0, 4.0)], Vec3::Y);
    let wall = quad([vec3(-2.5, 0.0, -1.2), vec3(0.5, 0.0, -1.2), vec3(0.5, 1.6, -1.2), vec3(-2.5, 1.6, -1.2)], Vec3::Z);
    let ball = tequila_scene::demo::uv_sphere(0.35, 128, 64);
    let mut white = MaterialType::Diffuse.defaults();
    white.base_color = Spectrum::splat(0.7);
    let mut chrome = Material::metal(metals::SILVER, 0.0);
    chrome.base_color = Spectrum::ONE;
    let scene = SceneSnapshot::new(
        vec![Arc::new(floor), Arc::new(wall), Arc::new(ball)],
        vec![
            Instance::new(InstanceId(0), MeshId(0), MaterialId(0)),
            Instance::new(InstanceId(1), MeshId(1), MaterialId(1)),
            Instance::new(InstanceId(2), MeshId(2), MaterialId(1)).with_transform(Transform::translation(vec3(0.6, 0.35, 0.2))),
        ],
        vec![white, chrome],
        Arc::new(Environment::uniform(Spectrum::splat(0.05))),
    );
    let hidden = Light::new(LightKind::Area(lamp(vec3(1.6, 1.3, 1.6), vec3(-1.0, -0.4, -1.2), 0.45, 0.45, 8.0, true))).with_visibility(Visibility::HIDDEN);
    let shown = Light::new(LightKind::Area(lamp(vec3(-1.4, 1.1, 0.9), vec3(0.4, -0.3, -1.0), 0.35, 0.25, 8.0, false)));
    let sun = Light::new(LightKind::Distant(DistantLight { direction: vec3(-0.5, -0.35, -1.0).normalize(), irradiance: Spectrum::splat(1.2), half_angle: 2f32.to_radians() }));
    let scene = scene.with_lights(Arc::new(LightSet::new(vec![hidden, shown, sun])));
    let camera = tequila_render_core::camera::Camera::look_at(vec3(1.2, 1.1, 3.4), vec3(0.0, 0.45, -0.2), Vec3::Y);
    println!("lamps in mirrors:");
    picture("lamps-in-mirrors", &render(scene, camera, samples()));
}

/// A dark glossy floor with a softbox low behind a black plate that casts
/// no shadow, and beside it a white floor under a dark ball that casts no
/// shadow, in a grey sky: the floor mirrors the softbox past the plate, and
/// the ball leaves the floor under it lit.
#[test]
#[ignore]
fn parts_that_cast_no_shadow_picture() {
    let floor = quad([vec3(-4.0, 0.0, -3.0), vec3(4.0, 0.0, -3.0), vec3(4.0, 0.0, 4.0), vec3(-4.0, 0.0, 4.0)], Vec3::Y);
    let white_floor = quad([vec3(0.4, 0.001, -1.5), vec3(2.6, 0.001, -1.5), vec3(2.6, 0.001, 1.5), vec3(0.4, 0.001, 1.5)], Vec3::Y);
    let plate = quad([vec3(-1.3, 0.0, -0.9), vec3(-0.3, 0.0, -0.9), vec3(-0.3, 0.55, -0.9), vec3(-1.3, 0.55, -0.9)], Vec3::Z);
    let ball = tequila_scene::demo::uv_sphere(0.35, 96, 48);
    let mut white = MaterialType::Diffuse.defaults();
    white.base_color = Spectrum::splat(0.75);
    let glossy = Material::plastic(Spectrum::splat(0.03), 0.08);
    let dark = Material::plastic(Spectrum { r: 0.15, g: 0.03, b: 0.02 }, 0.4);
    let mut instances = vec![
        Instance::new(InstanceId(0), MeshId(0), MaterialId(0)),
        Instance::new(InstanceId(1), MeshId(1), MaterialId(1)),
        Instance::new(InstanceId(2), MeshId(2), MaterialId(2)),
        Instance::new(InstanceId(3), MeshId(3), MaterialId(2)).with_transform(Transform::translation(vec3(1.5, 0.9, 0.0))),
    ];
    instances[2].casts_shadows = false;
    instances[3].casts_shadows = false;
    let scene = SceneSnapshot::new(
        vec![Arc::new(floor), Arc::new(white_floor), Arc::new(plate), Arc::new(ball)],
        instances,
        vec![glossy, white, dark],
        Arc::new(Environment::uniform(Spectrum::splat(0.6))),
    );
    let softbox = Light::new(LightKind::Area(lamp(vec3(-0.8, 0.35, -1.6), Vec3::Z, 0.45, 0.25, 12.0, false)));
    let scene = scene.with_lights(Arc::new(LightSet::new(vec![softbox])));
    let camera = tequila_render_core::camera::Camera::look_at(vec3(0.2, 1.0, 3.2), vec3(0.2, 0.25, -0.4), Vec3::Y);
    println!("parts that cast no shadow:");
    picture("casts-no-shadow", &render(scene, camera, samples()));
}

/// A hash of what `scene` renders to, for telling one build from another:
/// each pixel's value to the bit.
fn hash_of(scene: SceneSnapshot, camera: tequila_render_core::camera::Camera, samples: u32) -> u64 {
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let mut framebuffer = Framebuffer::new(64, 48);
    let context = RenderContext { camera, config: IntegratorConfig::product(), ..RenderContext::default() };
    for _ in 0..samples {
        render_pass(&intersector, &mut framebuffer, &context, &Default::default(), 1);
    }
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for y in 0..48 {
        for x in 0..64 {
            let c = framebuffer.pixel(x, y).mean();
            for v in [c.r, c.g, c.b] {
                for byte in v.to_bits().to_le_bytes() {
                    hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
                }
            }
        }
    }
    hash
}

/// Scenes none of whose surfaces is mirror-smooth where a lamp can show,
/// none of whose parts casts no shadow, and none of whose lamps is weak or
/// casts no shadow, rendered to a hash: the same in a build before this
/// work and after. The demo scene; a glossy floor and plastic ball under a
/// softbox, and under two of about the same power; a chrome ball and a
/// glass one lit by a point light and the sky, which no ray can meet.
#[test]
#[ignore]
fn unaffected_scenes_render_to_the_bit_as_before() {
    let camera = tequila_render_core::camera::Camera::look_at(vec3(2.0, 1.5, 3.0), vec3(0.0, 0.5, 0.0), Vec3::Y);
    let demo = tequila_scene::demo::demo_scene(tequila_scene::demo::DemoMaterial::Plastic, 0.2);
    let mut scene = SceneSnapshot::new(demo.meshes.clone(), demo.instances.clone(), demo.materials.clone(), demo.environment.clone()).with_lights(demo.lights.clone());
    scene.background = demo.background;
    println!("demo: {:016x}", hash_of(scene, camera, 16));
    let floor = quad([vec3(-4.0, 0.0, -4.0), vec3(4.0, 0.0, -4.0), vec3(4.0, 0.0, 4.0), vec3(-4.0, 0.0, 4.0)], Vec3::Y);
    let ball = tequila_scene::demo::uv_sphere(0.5, 64, 32);
    let lit = |materials: Vec<Material>, lights: Vec<Light>| {
        SceneSnapshot::new(
            vec![Arc::new(floor.clone()), Arc::new(ball.clone())],
            vec![
                Instance::new(InstanceId(0), MeshId(0), MaterialId(0)),
                Instance::new(InstanceId(1), MeshId(1), MaterialId(1)).with_transform(Transform::translation(vec3(0.0, 0.5, 0.0))),
            ],
            materials,
            Arc::new(Environment::uniform(Spectrum::splat(0.3))),
        )
        .with_lights(Arc::new(LightSet::new(lights)))
    };
    let glossy = || vec![Material::plastic(Spectrum::splat(0.1), 0.2), Material::plastic(Spectrum { r: 0.7, g: 0.2, b: 0.1 }, 0.3)];
    let softbox = |at: Vec3, radiance: f32| Light::new(LightKind::Area(lamp(at, vec3(0.0, 0.5, 0.0) - at, 0.5, 0.3, radiance, false)));
    println!("glossy floor, one softbox: {:016x}", hash_of(lit(glossy(), vec![softbox(vec3(1.5, 2.0, 1.0), 8.0)]), camera, 16));
    println!("glossy floor, two softboxes: {:016x}", hash_of(lit(glossy(), vec![softbox(vec3(1.5, 2.0, 1.0), 8.0), softbox(vec3(-1.5, 1.5, 0.5), 5.0)]), camera, 16));
    let mut chrome = Material::metal(metals::SILVER, 0.0);
    chrome.base_color = Spectrum::ONE;
    let point = Light::new(LightKind::Point(tequila_render_core::light::PointLight { position: vec3(1.0, 2.5, 1.5), intensity: Spectrum::splat(6.0), radius: 0.0 }));
    println!("chrome ball, point light: {:016x}", hash_of(lit(vec![Material::plastic(Spectrum::splat(0.5), 0.4), chrome], vec![point.clone()]), camera, 16));
    let glass = Material { transmission_color: Spectrum::ONE, transmission_depth: 0.0, ..MaterialType::GlassSolid.defaults() };
    println!("glass ball, point light: {:016x}", hash_of(lit(vec![Material::plastic(Spectrum::splat(0.5), 0.4), glass], vec![point]), camera, 16));
}
