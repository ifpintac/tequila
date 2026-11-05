//! The self-test's scene: small, quick, and holding a little of everything a
//! graphics card can get wrong - clear glass (refraction, the deepest
//! paths), polished gold (a conductor's Fresnel), a cube with a picture on
//! it (a texture sent to the card and sampled there), a ball of fibres (hair
//! curves, which Cycles traces with their own kernels) and a softbox lamp
//! beside the dim sky, on a plain floor.
//!
//! Built straight as a render scene rather than a document, so it is the
//! same scene in every version that renders it, whatever the document's
//! defaults become; and from nothing on disk, so it renders anywhere.

use std::sync::Arc;
use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec2, vec3, Transform, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::graph::{FuzzRequest, MaterialGraph, Node, NodeKind, Socket, TextureId, Value};
use tequila_materials::material_type::{MaterialType, Param};
use tequila_materials::openpbr::Material;
use tequila_render_core::camera::Camera;
use tequila_render_core::environment::{Background, Environment};
use tequila_render_core::light::{AreaLight, Light, LightKind, LightSet};
use tequila_render_core::scene::{Instance, Mesh, SceneSnapshot};
use tequila_render_core::settings::IntegratorConfig;
use tequila_render_core::shade::TextureTable;
use tequila_texture::projection::{Mapping, Projection};
use tequila_texture::texture::{MapRole, Texture};

/// The picture's size: small enough that the processor renders it in a
/// second or two, large enough that a wrong refraction or a missing texture
/// is many pixels.
pub const WIDTH: usize = 256;
pub const HEIGHT: usize = 160;

/// The scene, the camera it is seen from, and the lighting it is rendered
/// with.
pub struct Reference {
    pub scene: SceneSnapshot,
    pub camera: Camera,
    pub config: IntegratorConfig,
}

/// The scene, built.
pub fn build() -> Reference {
    let floor = Mesh::new(vec![vec3(-2.0, 0.0, -2.0), vec3(2.0, 0.0, -2.0), vec3(2.0, 0.0, 2.0), vec3(-2.0, 0.0, 2.0)], vec![0, 2, 1, 0, 3, 2])
        .with_normals(vec![Vec3::Y; 4]);
    let glass_ball = tequila_scene::demo::uv_sphere(0.032, 96, 48);
    let gold_ball = tequila_scene::demo::uv_sphere(0.026, 96, 48);
    let cube = cube(0.045);
    let fibre_core = tequila_scene::demo::uv_sphere(0.022, 64, 32);

    let floor_material = Material::diffuse(Spectrum::from_srgb_encoded(Vec3::splat(0.62)));
    let glass = Material { transmission_color: Spectrum::ONE, transmission_depth: 0.0, ..MaterialType::GlassSolid.defaults() };
    let gold = Material::metal(tequila_materials::fresnel::metals::GOLD, 0.18);
    // The cube's colour comes from its picture (the program below); this is
    // what it would be without one.
    let painted = Material::diffuse(Spectrum::splat(0.5));
    // A tennis ball's felt: a few thousand fibres three millimetres long.
    let felt_colour = Spectrum::from_srgb_encoded(vec3(0.80, 0.86, 0.22));
    let felt = Material {
        base_color: felt_colour,
        fuzz_weight: 1.0,
        fuzz_color: felt_colour,
        fuzz_roughness: 0.5,
        fuzz_density: 60.0,
        fuzz_length: 0.003,
        fuzz_thickness: 0.00012,
        ..MaterialType::Diffuse.defaults()
    };
    let fibres = FuzzRequest::of_material(&felt)
        .and_then(|request| tequila_scene::fuzz::grow(&fibre_core, 1.0, &request))
        .expect("the felt asks for fibres, and a ball has area to grow them on");

    let meshes = vec![Arc::new(floor), Arc::new(glass_ball), Arc::new(gold_ball), Arc::new(cube), Arc::new(fibre_core), Arc::new(fibres)];
    let place = |id: u32, mesh: u32, material: u32, transform: Transform| {
        Instance::new(InstanceId(id), MeshId(mesh), MaterialId(material)).with_transform(transform)
    };
    let fibre_place = Transform::translation(vec3(0.028, 0.022, 0.062));
    let instances = vec![
        place(0, 0, 0, Transform::IDENTITY),
        place(1, 1, 1, Transform::translation(vec3(-0.085, 0.032, 0.0))),
        place(2, 2, 2, Transform::translation(vec3(-0.012, 0.026, -0.045))),
        place(3, 3, 3, Transform::translation(vec3(0.07, 0.0225, -0.01)) * Transform::rotation(Vec3::Y, 0.55)),
        place(4, 4, 4, fibre_place),
        place(5, 5, 4, fibre_place),
    ];

    // The softbox, up to the left and in front, facing the middle of the
    // set.
    let at = vec3(-0.22, 0.32, 0.24);
    let facing = (vec3(0.0, 0.02, 0.0) - at).normalize();
    let side = facing.cross(Vec3::Y).normalize();
    let up = side.cross(facing).normalize();
    let softbox = Light::new(LightKind::Area(AreaLight {
        position: at,
        half_u: up * 0.07,
        half_v: side * 0.09,
        radiance: Spectrum::splat(26.0),
        disc: false,
        two_sided: false,
        spread: std::f32::consts::PI,
    }));

    let mut textures = TextureTable::new();
    textures.insert(TextureId(1), Arc::new(checker()));
    let mut programs = vec![None; 5];
    programs[3] = Some(Arc::new(textured().compile().expect("the cube's graph compiles")));

    let mut scene = SceneSnapshot::new(meshes, instances, vec![floor_material, glass, gold, painted, felt], Arc::new(Environment::uniform(Spectrum::splat(0.32))))
        .with_lights(Arc::new(LightSet::new(vec![softbox])))
        .with_programs(programs, Arc::new(textures));
    scene.background = Background::Environment;

    let mut camera = Camera::look_at(vec3(0.0, 0.11, 0.30), vec3(-0.004, 0.024, 0.0), Vec3::Y);
    camera.set_vertical_fov(0.52, WIDTH as f32 / HEIGHT as f32);
    camera.keep_focus = true;

    // Product lighting, with adaptive sampling off: every check takes the
    // same number of samples, so the times compare.
    let config = IntegratorConfig { adaptive_sampling_threshold: 0.0, ..IntegratorConfig::product() };
    Reference { scene, camera, config }
}

/// A cube `size` across, standing on the floor at the origin, each face
/// with the whole of the picture on it.
fn cube(size: f32) -> Mesh {
    let h = size * 0.5;
    // Each face's normal, and the directions of its picture's u and v.
    let faces: [(Vec3, Vec3, Vec3); 6] = [
        (Vec3::X, vec3(0.0, 0.0, -1.0), Vec3::Y),
        (vec3(-1.0, 0.0, 0.0), Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::X, vec3(0.0, 0.0, -1.0)),
        (vec3(0.0, -1.0, 0.0), Vec3::X, Vec3::Z),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (vec3(0.0, 0.0, -1.0), vec3(-1.0, 0.0, 0.0), Vec3::Y),
    ];
    let (mut positions, mut normals, mut uvs, mut indices) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (normal, u, v) in faces {
        let start = positions.len() as u32;
        for (a, b) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            positions.push(vec3(0.0, h, 0.0) + (normal + u * a + v * b) * h);
            normals.push(normal);
            uvs.push(vec2((a + 1.0) * 0.5, (b + 1.0) * 0.5));
        }
        indices.extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
    }
    Mesh::new(positions, indices).with_normals(normals).with_uvs(uvs)
}

/// A red and white checker of eight squares a side with a blue bar along one
/// edge, so a picture sampled upside down or not at all shows.
fn checker() -> Texture {
    const SIZE: usize = 64;
    let mut bytes = Vec::with_capacity(SIZE * SIZE * 3);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let rgb = if y < SIZE / 8 {
                [40, 70, 200]
            } else if (x / 8 + y / 8) % 2 == 0 {
                [205, 40, 35]
            } else {
                [235, 232, 225]
            };
            bytes.extend_from_slice(&rgb);
        }
    }
    Texture::from_bytes(SIZE, SIZE, 3, &bytes, MapRole::Color, Mapping { projection: Projection::Uv, ..Default::default() })
}

/// The cube's material: its base colour from the checker, through its UVs.
fn textured() -> MaterialGraph {
    let mut graph = MaterialGraph::from_material(MaterialType::Diffuse.defaults());
    let output = graph.output();
    let mapping = graph.add_node(Node::new(NodeKind::Mapping2d { mapping: Mapping { projection: Projection::Uv, ..Default::default() } }));
    let texture = graph.add_node(Node::new(NodeKind::ImageTexture { texture: TextureId(1), fallback: Value::Color(Spectrum::splat(0.5)) }));
    graph.connect(mapping, texture, Socket::Input(0));
    graph.connect(texture, output, Socket::Param(Param::BaseColor));
    graph
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Everything the scene is for is in it: glass, a conductor, a
    /// texture, fibres and a lamp - and the camera sees the set.
    #[test]
    fn the_scene_holds_what_it_tests() {
        let reference = build();
        let scene = &reference.scene;
        assert_eq!(scene.instances.len(), 6);
        assert!(scene.meshes.iter().any(|m| m.fibres.is_some()), "the felt grew no fibres");
        let fibres = scene.meshes.iter().find_map(|m| m.fibres.as_ref()).unwrap();
        assert!((1_000..20_000).contains(&fibres.count()), "{} fibres", fibres.count());
        assert!(scene.programs.iter().flatten().count() == 1, "the cube's texture");
        assert_eq!(scene.lights.lights().len(), 1);
        let aspect = WIDTH as f32 / HEIGHT as f32;
        for at in [vec3(-0.085, 0.032, 0.0), vec3(-0.012, 0.026, -0.045), vec3(0.07, 0.0225, -0.01), vec3(0.028, 0.022, 0.062)] {
            let film = reference.camera.project(at, aspect).expect("in front of the camera");
            assert!((0.05..0.95).contains(&film.x) && (0.05..0.95).contains(&film.y), "{at:?} is at {film:?}");
        }
    }
}
