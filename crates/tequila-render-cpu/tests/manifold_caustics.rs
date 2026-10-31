//! Manifold next event estimation (`integrator::manifold`): a lamp's light
//! through glass that bends it, solved for at each surface.
//!
//! The claims, each one way it can be wrong. Through a slab of glass, whose
//! light onto the floor below has a closed form, it must bring exactly that
//! light, from a point light and from the sun - too much, and the straight
//! shadow ray is letting the light by as well; too little, or spread wrong,
//! and the walk or its geometric term is off. Through a lens it must agree
//! with brute-force path tracing of a glowing panel where the lamp is,
//! which finds the same light by bounces through the glass. Under a sky and
//! a glowing panel, which it leaves to the path tracer, it must change
//! nothing: the shadow rays it stops at the glass are what the bounces
//! through it make up. And a thin wall, which bends nothing, is passed as
//! before.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec2, vec3, Ray, Vec3};
use tequila_core::sampler::Sampler;
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::MaterialType;
use tequila_materials::openpbr::Material;
use tequila_render_core::environment::Environment;
use tequila_render_core::integrator::trace_path;
use tequila_render_core::light::{AreaLight, DistantLight, Light, LightKind, LightSet, PointLight};
use tequila_render_core::scene::{Instance, Mesh, SceneSnapshot, ShadowCatcher};
use tequila_render_core::settings::{CausticsMethod, IntegratorConfig};
use tequila_render_cpu::{Device, EmbreeIntersector};
use std::sync::Arc;

/// Manifold caustics on, over the everyday preset.
fn manifold() -> IntegratorConfig {
    IntegratorConfig { caustics: true, caustics_method: CausticsMethod::Manifold, ..IntegratorConfig::product() }
}

/// Plain path tracing, to measure against: caustics on but no photons read
/// (every picture here is traced without them), so shadow rays stop at all
/// glass and light through it is found by bounces alone; and no
/// regularization, whose roughened glass is a bias of its own.
fn plain() -> IntegratorConfig {
    IntegratorConfig { caustics: true, caustics_method: CausticsMethod::Photons, regularization_start_bounce: u32::MAX, ..IntegratorConfig::product() }
}

/// With the bounces the closed forms below describe, through glass the
/// path crosses `crossings` times: the floor lit straight through the
/// glass, and none of the light the floor and the glass reflect between
/// them.
fn single(config: IntegratorConfig, crossings: u32) -> IntegratorConfig {
    IntegratorConfig { max_bounces: crossings + 1, ..config }
}

/// A box with a normal per face, for glass whose faces the walk steps
/// along: flat, so its normals do not turn.
fn block(min: Vec3, max: Vec3) -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    let corner = |x: usize, y: usize, z: usize| vec3([min.x, max.x][x], [min.y, max.y][y], [min.z, max.z][z]);
    // Each face as four corners wound counter-clockwise seen from outside.
    let faces = [
        (Vec3::X, [corner(1, 0, 0), corner(1, 1, 0), corner(1, 1, 1), corner(1, 0, 1)]),
        (-Vec3::X, [corner(0, 0, 1), corner(0, 1, 1), corner(0, 1, 0), corner(0, 0, 0)]),
        (Vec3::Y, [corner(0, 1, 0), corner(0, 1, 1), corner(1, 1, 1), corner(1, 1, 0)]),
        (-Vec3::Y, [corner(0, 0, 0), corner(1, 0, 0), corner(1, 0, 1), corner(0, 0, 1)]),
        (Vec3::Z, [corner(0, 0, 1), corner(1, 0, 1), corner(1, 1, 1), corner(0, 1, 1)]),
        (-Vec3::Z, [corner(0, 1, 0), corner(1, 1, 0), corner(1, 0, 0), corner(0, 0, 0)]),
    ];
    for (normal, quad) in faces {
        let base = positions.len() as u32;
        positions.extend_from_slice(&quad);
        normals.extend_from_slice(&[normal; 4]);
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    Mesh::new(positions, indices).with_normals(normals)
}

/// A plano-convex lens, its flat face down at `bottom`: a disc of radius
/// `aperture`, a rim `rim` high, and a cap of a sphere of radius `curve`
/// on top, smooth where it is curved.
fn lens(bottom: f32, aperture: f32, rim: f32, curve: f32) -> Mesh {
    const AROUND: usize = 96;
    const RINGS: usize = 24;
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    let top = bottom + rim;
    // Where the cap's sphere is centred, below the cap.
    let centre = top - (curve * curve - aperture * aperture).sqrt();
    let around = |i: usize| {
        let phi = i as f32 / AROUND as f32 * std::f32::consts::TAU;
        (phi.cos(), phi.sin())
    };
    // The cap: rings from its apex out to the rim.
    let apex_angle = (aperture / curve).asin();
    let cap = positions.len() as u32;
    for ring in 0..=RINGS {
        let theta = apex_angle * ring as f32 / RINGS as f32;
        for i in 0..AROUND {
            let (c, s) = around(i);
            let normal = vec3(theta.sin() * c, theta.cos(), theta.sin() * s);
            positions.push(vec3(0.0, centre, 0.0) + normal * curve);
            normals.push(normal);
        }
    }
    for ring in 0..RINGS {
        for i in 0..AROUND {
            let j = (i + 1) % AROUND;
            let a = cap + (ring * AROUND + i) as u32;
            let b = cap + (ring * AROUND + j) as u32;
            let c = cap + ((ring + 1) * AROUND + i) as u32;
            let d = cap + ((ring + 1) * AROUND + j) as u32;
            indices.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }
    // The rim, facing out.
    let wall = positions.len() as u32;
    for i in 0..AROUND {
        let (c, s) = around(i);
        positions.push(vec3(aperture * c, top, aperture * s));
        positions.push(vec3(aperture * c, bottom, aperture * s));
        normals.push(vec3(c, 0.0, s));
        normals.push(vec3(c, 0.0, s));
    }
    for i in 0..AROUND {
        let j = (i + 1) % AROUND;
        let (a, b, c, d) = (wall + 2 * i as u32, wall + 2 * i as u32 + 1, wall + 2 * j as u32, wall + 2 * j as u32 + 1);
        indices.extend_from_slice(&[a, c, b, c, d, b]);
    }
    // The flat face, facing down.
    let floor = positions.len() as u32;
    positions.push(vec3(0.0, bottom, 0.0));
    normals.push(-Vec3::Y);
    for i in 0..AROUND {
        let (c, s) = around(i);
        positions.push(vec3(aperture * c, bottom, aperture * s));
        normals.push(-Vec3::Y);
    }
    for i in 0..AROUND {
        let j = (i + 1) % AROUND;
        indices.extend_from_slice(&[floor, floor + 1 + j as u32, floor + 1 + i as u32]);
    }
    Mesh::new(positions, indices).with_normals(normals)
}

fn floor_mesh(half: f32) -> Mesh {
    Mesh::new(vec![vec3(-half, 0.0, -half), vec3(half, 0.0, -half), vec3(half, 0.0, half), vec3(-half, 0.0, half)], vec![0, 2, 1, 0, 3, 2])
        .with_normals(vec![Vec3::Y; 4])
}

fn glass() -> Material {
    Material { transmission_color: Spectrum::ONE, transmission_depth: 0.0, ..MaterialType::GlassSolid.defaults() }
}

/// Dark, so that little of its light comes back to it off the glass.
const ALBEDO: f32 = 0.1;

fn floor_material() -> Material {
    Material { base_color: Spectrum::splat(ALBEDO), ..MaterialType::Diffuse.defaults() }
}

/// A floor, the meshes on it, and lights.
fn scene(parts: Vec<(Mesh, Material)>, lights: Vec<Light>, sky: Spectrum) -> SceneSnapshot {
    let mut meshes = vec![Arc::new(floor_mesh(8.0))];
    let mut materials = vec![floor_material()];
    let mut instances = vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))];
    for (mesh, material) in parts {
        let i = meshes.len() as u32;
        meshes.push(Arc::new(mesh));
        materials.push(material);
        instances.push(Instance::new(InstanceId(i), MeshId(i), MaterialId(i)));
    }
    SceneSnapshot::new(meshes, instances, materials, Arc::new(Environment::uniform(sky))).with_lights(Arc::new(LightSet::new(lights)))
}

/// The floor's radiance at `(x, z)`, seen from straight above at `height`,
/// averaged over `samples` paths.
fn floor_at(intersector: &EmbreeIntersector, config: &IntegratorConfig, x: f32, z: f32, height: f32, samples: u32) -> f32 {
    let mut sum = 0.0f64;
    for i in 0..samples {
        let mut sampler = Sampler::new(7, 11, i, 3);
        let ray = Ray::new(vec3(x, height, z), -Vec3::Y);
        sum += f64::from(trace_path(intersector, config, ray, vec2(0.5, 0.5), &mut sampler).radiance.luminance());
    }
    (sum / f64::from(samples)) as f32
}

/// What two faces of glass at index 1.5 let by, met at `cos`.
fn two_faces(cos: f32) -> f32 {
    let face = 1.0 - tequila_materials::fresnel::dielectric(cos, 1.5);
    face * face
}

/// The slab the closed-form tests look through: 0.1 thick, its underside
/// 0.5 above the floor.
const SLAB: (f32, f32) = (0.5, 0.6);

/// The irradiance a point light `height` above the floor brings through the
/// slab to a point `r` from the foot of it: its intensity, through the two
/// faces, times the solid angle a unit of floor takes of it, which the slab
/// widens. A ray leaving `theta` off the vertical lands at
/// `r = (h + g) tan theta + t tan theta_t`, so a ring `dr` wide takes
/// `sin theta d theta / (r dr)`.
fn through_slab(intensity: f32, height: f32, r: f32) -> f32 {
    let (g, t) = (SLAB.0, SLAB.1 - SLAB.0);
    let h = height - SLAB.1;
    let inside = |theta: f32| (theta.sin() / 1.5).asin();
    let reach = |theta: f32| (h + g) * theta.tan() + t * inside(theta).tan();
    if r <= 0.0 {
        return intensity * two_faces(1.0) / ((h + g + t / 1.5) * (h + g + t / 1.5));
    }
    let (mut low, mut high) = (0.0f64, 1.5f64);
    for _ in 0..80 {
        let mid = 0.5 * (low + high);
        if f64::from(reach(mid as f32)) < f64::from(r) {
            low = mid;
        } else {
            high = mid;
        }
    }
    let theta = (0.5 * (low + high)) as f32;
    let step = 1e-3f32;
    let dr = (reach(theta + step) - reach(theta - step)) / (2.0 * step);
    intensity * two_faces(theta.cos()) * theta.sin() / (r * dr)
}

/// Off the lines the slab's and the floor's triangles meet along, which a
/// ray straight down one can slip through.
const ASIDE: f32 = 0.0137;

#[test]
fn a_point_light_through_a_slab_of_glass_brings_what_it_should() {
    let height = 2.0;
    let lamp = Light::new(LightKind::Point(PointLight { position: vec3(0.0, height, 0.0), intensity: Spectrum::splat(10.0), radius: 0.0 }));
    let slab = block(vec3(-3.0, SLAB.0, -3.0), vec3(3.0, SLAB.1, 3.0));
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene(vec![(slab, glass())], vec![lamp], Spectrum::ZERO)).expect("builds");
    for x in [0.0f32, 0.3, 0.8, 1.5] {
        let r = (x * x + ASIDE * ASIDE).sqrt();
        let seen = floor_at(&intersector, &single(manifold(), 2), x, ASIDE, 0.25, 16);
        let expected = ALBEDO / std::f32::consts::PI * through_slab(10.0, height, r);
        println!("r {r}: {seen:.5} against {expected:.5}");
        assert!((seen / expected - 1.0).abs() < 0.01, "at {r} from the foot of the lamp the floor had {seen:.5}, not {expected:.5}");
        // Without, the slab's shadow: nothing a bounce can bring.
        let without = floor_at(&intersector, &plain(), x, ASIDE, 0.25, 16);
        assert!(without < expected * 0.05, "the slab let a point light by without the manifold sample: {without:.5}");
    }
}

/// The same through a slab of glass with a layer of water inside it: four
/// crossings, two of them between glass and water, whose index each takes
/// from the media the path is in (1.5 to 1.33, not 1.33 to air). Each layer
/// widens the cone a ray at `theta` leaves in by its depth times the
/// tangent it crosses it at, and each interface lets by what its Fresnel
/// does not reflect.
#[test]
fn a_point_light_through_water_in_glass_brings_what_it_should() {
    let height = 2.0;
    let lamp = Light::new(LightKind::Point(PointLight { position: vec3(0.0, height, 0.0), intensity: Spectrum::splat(10.0), radius: 0.0 }));
    let glass_block = block(vec3(-3.0, 0.5, -3.0), vec3(3.0, 0.62, 3.0));
    let water_block = block(vec3(-2.5, 0.53, -2.5), vec3(2.5, 0.59, 2.5));
    let water = Material { transmission_color: Spectrum::ONE, transmission_depth: 0.0, specular_ior: 1.33, ..MaterialType::Liquid.defaults() };
    let device = Device::shared().expect("an Embree device");
    let intersector =
        EmbreeIntersector::build(&device, scene(vec![(glass_block, glass()), (water_block, water)], vec![lamp], Spectrum::ZERO)).expect("builds");
    // From the floor up: air, glass, water, glass, air, each with its depth
    // and index.
    let layers = [(0.5f32, 1.0f32), (0.03, 1.5), (0.06, 1.33), (0.03, 1.5), (height - 0.62, 1.0)];
    let angle_in = |theta: f32, n: f32| (theta.sin() / n).asin();
    let reach = |theta: f32| layers.iter().map(|&(depth, n)| depth * angle_in(theta, n).tan()).sum::<f32>();
    let through = |theta: f32| {
        layers.windows(2).map(|pair| 1.0 - tequila_materials::fresnel::dielectric(angle_in(theta, pair[0].1).cos(), pair[1].1 / pair[0].1)).product::<f32>()
    };
    for x in [0.0f32, 0.25, 0.6] {
        let r = (x * x + ASIDE * ASIDE).sqrt();
        let (mut low, mut high) = (0.0f32, 1.5f32);
        for _ in 0..60 {
            let mid = 0.5 * (low + high);
            if reach(mid) < r {
                low = mid;
            } else {
                high = mid;
            }
        }
        let theta = 0.5 * (low + high);
        let step = 1e-3f32;
        let dr = (reach(theta + step) - reach((theta - step).max(0.0))) / (theta + step - (theta - step).max(0.0));
        let expected = ALBEDO / std::f32::consts::PI * 10.0 * through(theta) * theta.sin() / (r * dr);
        let seen = floor_at(&intersector, &single(manifold(), 4), x, ASIDE, 0.25, 8);
        println!("r {r}: {seen:.5} against {expected:.5}");
        assert!((seen / expected - 1.0).abs() < 0.01, "through water in glass at {r} the floor had {seen:.5}, not {expected:.5}");
    }
}

/// A surface of revolution about the vertical: `profile`, (radius, height)
/// from a point on the axis to another, smooth.
fn lathe(profile: &[(f32, f32)], around: usize) -> Mesh {
    let last = profile.len() - 1;
    let mut positions = Vec::new();
    let mut rings = Vec::new();
    for (i, &(r, y)) in profile.iter().enumerate() {
        rings.push(positions.len() as u32);
        if i == 0 || i == last {
            positions.push(vec3(0.0, y, 0.0));
        } else {
            for j in 0..around {
                let phi = j as f32 / around as f32 * std::f32::consts::TAU;
                positions.push(vec3(r * phi.cos(), y, r * phi.sin()));
            }
        }
    }
    let at = |ring: usize, j: usize| if ring == 0 || ring == last { rings[ring] } else { rings[ring] + j as u32 };
    let mut indices = Vec::new();
    for i in 0..last {
        for j in 0..around {
            let k = (j + 1) % around;
            let (a, b, c, d) = (at(i, j), at(i, k), at(i + 1, j), at(i + 1, k));
            if i != 0 {
                indices.extend_from_slice(&[a, c, b]);
            }
            if i + 1 != last {
                indices.extend_from_slice(&[b, c, d]);
            }
        }
    }
    let mut mesh = Mesh::new(positions, indices);
    mesh.compute_smooth_normals();
    mesh
}

/// A quarter circle about `centre` from angle `from` to `to`, in degrees.
fn arc(centre: (f32, f32), radius: f32, from: f32, to: f32) -> Vec<(f32, f32)> {
    (0..=6)
        .map(|i| {
            let a = (from + (to - from) * i as f32 / 6.0).to_radians();
            (centre.0 + radius * a.cos(), centre.1 + radius * a.sin())
        })
        .collect()
}

/// A lamp shines through a small perfume flacon - a rounded block of glass,
/// a centimetre and a half thick at the sides, with a scent inside - onto
/// the floor behind it, and the manifold sample finds a good part of the
/// light path tracing finds there, bouncing off the floor and through the
/// flacon into a glowing ball where the lamp is; never more. Not all of
/// it: the flacon is a cylinder lens, and where two paths through it join
/// the same two points the walk finds the one its seed leads to, 45% of the
/// light here (measured). Four crossings, two of them between glass and
/// liquid, where landing each a few hundredths of a millimetre off moves the
/// answer a long way: aimed from beside where it started, near the origin,
/// every projection did, the walk stalled, and it found 0.6%.
#[test]
fn a_lamp_shines_through_a_perfume_flacon() {
    let mut outer = vec![(0.0, 0.0)];
    outer.extend(arc((0.05, 0.012), 0.012, -90.0, 0.0));
    outer.extend(arc((0.05, 0.128), 0.012, 0.0, 90.0));
    outer.extend([(0.03, 0.142), (0.0, 0.142)]);
    let mut inner = vec![(0.0, 0.016)];
    inner.extend(arc((0.034, 0.03), 0.014, -90.0, 0.0));
    inner.extend(arc((0.034, 0.1), 0.014, 0.0, 90.0));
    inner.push((0.0, 0.114));
    let scent = Material { transmission_color: Spectrum::ONE, transmission_depth: 0.0, specular_ior: 1.36, ..MaterialType::Liquid.defaults() };
    let flacon = || vec![(lathe(&outer, 128), glass()), (lathe(&inner, 128), scent.clone())];
    // A ball of light a centimetre and a half across, and the lamp the same
    // size and as bright: a sphere of radiance L has intensity L pi r^2.
    let (lamp_at, radius, radiance) = (vec3(0.42, 0.5, -0.52), 0.015f32, 2000.0f32);
    let lamp = Light::new(LightKind::Point(PointLight { position: lamp_at, intensity: Spectrum::splat(radiance * std::f32::consts::PI * radius * radius), radius }));
    let mut ball = tequila_scene::demo::uv_sphere(radius, 32, 16);
    for p in &mut ball.positions {
        *p += lamp_at;
    }
    let glow = Material { emission_color: Spectrum::ONE, emission_luminance: radiance, ..MaterialType::Emissive.defaults() };
    let device = Device::shared().expect("an Embree device");
    let solved = EmbreeIntersector::build(&device, scene(flacon(), vec![lamp], Spectrum::ZERO)).expect("builds");
    let mut parts = flacon();
    parts.push((ball, glow));
    let traced = EmbreeIntersector::build(&device, scene(parts, Vec::new(), Spectrum::ZERO)).expect("builds");
    // Points on the floor in the flacon's shadow: where no light reaches it
    // with the glass solid.
    let through = vec3(0.0, 0.06, 0.0);
    let towards = (through - lamp_at).normalize();
    let centre = lamp_at + towards * (-lamp_at.y / towards.y);
    let (mut by_manifold, mut by_paths, mut points) = (0.0f64, 0.0f64, 0);
    for i in 0..16 {
        for j in 0..16 {
            let (x, z) = (centre.x + (i as f32 - 7.5) * 0.012, centre.z + (j as f32 - 7.5) * 0.012);
            if x * x + z * z < 0.07 * 0.07 || floor_at(&solved, &plain(), x, z, 0.002, 4) > 1e-4 {
                continue;
            }
            points += 1;
            by_manifold += f64::from(floor_at(&solved, &manifold(), x, z, 0.002, 64));
            by_paths += f64::from(floor_at(&traced, &plain(), x, z, 0.002, 4096));
        }
    }
    println!("behind the flacon ({points} points): {by_manifold:.4} by the manifold sample, {by_paths:.4} by path tracing");
    assert!(points > 20, "no shadow to look in: {points} points");
    let share = by_manifold / by_paths;
    assert!(share > 0.35 && share < 1.05, "behind the flacon the manifold sample found {share:.3} of the light path tracing does");
}

#[test]
fn the_sun_through_a_slab_of_glass_brings_what_it_should() {
    // A slab shifts the sun's beam and leaves it as dense as it was: what
    // reaches the floor is the illuminance less what the two faces take.
    let angle = 30f32.to_radians();
    let sun = Light::new(LightKind::Distant(DistantLight {
        direction: vec3(-angle.sin(), -angle.cos(), 0.0),
        irradiance: Spectrum::splat(5.0),
        half_angle: 0.0,
    }));
    let slab = block(vec3(-3.0, SLAB.0, -3.0), vec3(3.0, SLAB.1, 3.0));
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene(vec![(slab, glass())], vec![sun], Spectrum::ZERO)).expect("builds");
    let expected = ALBEDO / std::f32::consts::PI * 5.0 * angle.cos() * two_faces(angle.cos());
    for x in [-0.5f32, 0.0, 0.7] {
        let seen = floor_at(&intersector, &single(manifold(), 2), x, 0.2, 0.25, 8);
        println!("x {x}: {seen:.5} against {expected:.5}");
        assert!((seen / expected - 1.0).abs() < 0.01, "the sun through the slab gave the floor {seen:.5}, not {expected:.5}");
    }
}

/// The mean luminance of a `size` square picture of the floor from a
/// camera at `eye` looking straight down, `samples` paths a pixel, traced
/// without photons whatever `config` says.
fn picture(intersector: &EmbreeIntersector, config: &IntegratorConfig, eye: Vec3, size: usize, samples: u32) -> f32 {
    use rayon::prelude::*;
    let camera = tequila_render_core::camera::Camera::look_at(eye, vec3(eye.x, 0.0, eye.z + 0.01), Vec3::Y);
    let sum: f64 = (0..size * size)
        .into_par_iter()
        .map(|pixel| {
            let (x, y) = (pixel % size, pixel / size);
            let mut sum = 0.0f64;
            for i in 0..samples {
                let mut sampler = Sampler::new(x as u32, y as u32, i, 5);
                let jitter = sampler.pixel_jitter();
                let film = vec2((x as f32 + jitter.x) / size as f32, (y as f32 + jitter.y) / size as f32);
                let ray = camera.generate_ray(film, 1.0, &mut sampler);
                sum += f64::from(trace_path(intersector, config, ray, film, &mut sampler).radiance.luminance());
            }
            sum / f64::from(samples)
        })
        .sum();
    (sum / (size * size) as f64) as f32
}

/// Under a lens, the light of an area lamp as the manifold sample finds it
/// is the light of a glowing panel of the same size and brightness in the
/// same place as path tracing finds it, bouncing off the floor and through
/// the lens into the panel. The lens is weak enough that no two paths
/// through it meet on the floor, so there is one path to find from each
/// point of the lamp; and away from the lens the two lamps light the floor
/// alike, which says they are the same lamp. The everyday preset's
/// regularization on the manifold side: roughened, glass past a manifold
/// sample would sample the lamp again through itself, and light the spot
/// twice over in specks.
#[test]
fn a_lens_focuses_a_lamp_as_path_tracing_finds_it() {
    let (y, half, radiance) = (5.0, 0.6, 40.0);
    let area = Light::new(LightKind::Area(AreaLight {
        position: vec3(0.0, y, 0.0),
        half_u: vec3(half, 0.0, 0.0),
        half_v: vec3(0.0, 0.0, half),
        radiance: Spectrum::splat(radiance),
        disc: false,
        two_sided: false,
        spread: std::f32::consts::PI,
    }));
    let glow = Material { emission_color: Spectrum::ONE, emission_luminance: radiance, ..MaterialType::Emissive.defaults() };
    let panel = Mesh::new(vec![vec3(-half, y, -half), vec3(half, y, -half), vec3(half, y, half), vec3(-half, y, half)], vec![0, 1, 2, 0, 2, 3])
        .with_normals(vec![-Vec3::Y; 4]);
    let lens = || lens(0.8, 0.5, 0.02, 0.8);
    let device = Device::shared().expect("an Embree device");
    let solved = EmbreeIntersector::build(&device, scene(vec![(lens(), glass())], vec![area], Spectrum::ZERO)).expect("builds");
    let traced = EmbreeIntersector::build(&device, scene(vec![(lens(), glass()), (panel, glow)], Vec::new(), Spectrum::ZERO)).expect("builds");

    // The floor under the lens's middle, from just under it.
    let under = vec3(0.0, 0.45, 0.0);
    let by_manifold = picture(&solved, &manifold(), under, 16, 256);
    let by_paths = picture(&traced, &plain(), under, 16, 4096);
    // Beside it, in the open.
    let open = vec3(2.0, 0.45, 0.0);
    let lamp_open = picture(&solved, &manifold(), open, 16, 64);
    let panel_open = picture(&traced, &plain(), open, 16, 1024);
    println!("under the lens: {by_manifold:.5} by the manifold sample, {by_paths:.5} by path tracing; in the open {lamp_open:.5} and {panel_open:.5}");
    assert!((lamp_open / panel_open - 1.0).abs() < 0.03, "the lamp and the panel are not the same light: {lamp_open:.5} against {panel_open:.5}");
    assert!(by_manifold > lamp_open * 1.3, "the lens focused nothing: {by_manifold:.5} under it, {lamp_open:.5} in the open");
    assert!((by_manifold / by_paths - 1.0).abs() < 0.05, "under the lens the manifold sample found {by_manifold:.5}, path tracing {by_paths:.5}");
}

/// Under a uniform sky and a glowing panel, which the manifold sample
/// leaves to the path tracer, a glass ball renders as with no caustics at
/// all: shadow rays stop at the ball, and bounces through it bring the
/// light they stopped, weighed as nothing else found it. Let through as well
/// (as clear glass is with caustics off), or weighed against a light sample
/// that never came, the floor round the ball would come out too bright or
/// too dark.
#[test]
fn the_sky_and_glowing_parts_through_glass_are_neither_lost_nor_doubled() {
    let glow = Material { emission_color: Spectrum::ONE, emission_luminance: 20.0, ..MaterialType::Emissive.defaults() };
    let panel = Mesh::new(vec![vec3(0.6, 2.0, -0.3), vec3(1.2, 2.0, -0.3), vec3(1.2, 2.0, 0.3), vec3(0.6, 2.0, 0.3)], vec![0, 1, 2, 0, 2, 3])
        .with_normals(vec![-Vec3::Y; 4]);
    let mut ball = tequila_scene::demo::uv_sphere(0.4, 64, 32);
    for p in &mut ball.positions {
        p.y += 0.45;
    }
    let device = Device::shared().expect("an Embree device");
    let intersector =
        EmbreeIntersector::build(&device, scene(vec![(ball, glass()), (panel, glow)], Vec::new(), Spectrum::splat(0.5))).expect("builds");
    let eye = vec3(0.3, 2.2, 0.0);
    let reference = picture(&intersector, &plain(), eye, 24, 1024);
    let with = picture(&intersector, &IntegratorConfig { regularization_start_bounce: u32::MAX, ..manifold() }, eye, 24, 1024);
    println!("{reference:.5} plain, {with:.5} with manifold caustics");
    assert!((with / reference - 1.0).abs() < 0.02, "with manifold caustics {with:.5}, without {reference:.5}");
}

/// A thin wall bends nothing: shadow rays pass it with manifold caustics on
/// as with caustics off, weighed against the bounces through it as they
/// were, and no manifold sample is taken through it, so the floor under a
/// thin pane is lit once, as it was - by a lamp and by the sky.
#[test]
fn a_thin_pane_lets_light_by_as_it_did() {
    let lamp = Light::new(LightKind::Point(PointLight { position: vec3(0.0, 2.0, 0.0), intensity: Spectrum::splat(10.0), radius: 0.0 }));
    let pane = Material { geometry_thin_walled: true, ..glass() };
    let device = Device::shared().expect("an Embree device");
    for (lights, sky) in [(vec![lamp], Spectrum::ZERO), (Vec::new(), Spectrum::ONE)] {
        let slab = block(vec3(-3.0, SLAB.0, -3.0), vec3(3.0, SLAB.1, 3.0));
        let intersector = EmbreeIntersector::build(&device, scene(vec![(slab, pane.clone())], lights, sky)).expect("builds");
        let off = floor_at(&intersector, &IntegratorConfig::product(), 0.2, ASIDE, 0.25, 4096);
        let with = floor_at(&intersector, &manifold(), 0.2, ASIDE, 0.25, 4096);
        println!("under a thin pane: {off:.5} with caustics off, {with:.5} with manifold caustics");
        assert!(off > 0.01, "the pane let nothing by: {off:.5}");
        assert!((with / off - 1.0).abs() < 0.02, "under a thin pane manifold caustics gave {with:.5}, caustics off {off:.5}");
    }
}

/// On a shadow catcher, a glass ball's caustic from a lamp brightens the
/// backdrop where the ball focuses it, as it would a real floor; beside it,
/// in the ball's shadow, the backdrop darkens.
#[test]
fn a_caustic_shows_on_a_shadow_catcher() {
    let towards = vec3(-1.0, -1.0, 0.0).normalize();
    let side = vec3(0.0, 0.0, 1.0);
    let across = side.cross(towards).normalize();
    let lamp = Light::new(LightKind::Area(AreaLight {
        position: vec3(3.0, 3.6, 0.0),
        half_u: across * 0.1,
        half_v: side * 0.1,
        radiance: Spectrum::splat(2000.0),
        disc: false,
        two_sided: false,
        spread: std::f32::consts::PI,
    }));
    let mut ball = tequila_scene::demo::uv_sphere(0.5, 96, 48);
    for p in &mut ball.positions {
        p.y += 0.5;
    }
    let mut built = scene(vec![(ball, glass())], vec![lamp], Spectrum::splat(0.3));
    built.instances[0].shadow_catcher = Some(ShadowCatcher::default());
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, built).expect("builds");
    // Along the floor away from the lamp, from beside the ball: the
    // caustic and the shadow round it.
    let mut brightest = 0.0f32;
    let mut darkest = f32::MAX;
    for i in 0..40 {
        let x = -0.55 - i as f32 * 0.025;
        let seen = floor_at(&intersector, &manifold(), x, 0.0, 0.2, 32);
        brightest = brightest.max(seen);
        darkest = darkest.min(seen);
    }
    let backdrop = floor_at(&intersector, &manifold(), 2.0, 2.0, 0.2, 8);
    println!("on the catcher: brightest {brightest:.4}, darkest {darkest:.4}, the open backdrop {backdrop:.4}");
    assert!(brightest > backdrop * 1.5, "no caustic on the catcher: at most {brightest:.4} against the backdrop's {backdrop:.4}");
    assert!(darkest < backdrop * 0.7, "no shadow on the catcher: at least {darkest:.4} against the backdrop's {backdrop:.4}");
}
