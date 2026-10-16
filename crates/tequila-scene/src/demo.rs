//! The M0 demonstration scene.
//!
//! `docs/05-ROADMAP-AND-TESTING.md` §1 states the M0 exit criterion as "a
//! sphere on a ground plane under an HDRI, orbitable at >= 30 fps with a
//! denoised image that is visually converged within 2 seconds". This module
//! builds exactly that, as real triangle meshes so the whole pipeline —
//! tessellation, BLAS/TLAS build, interpolated shading normals, instancing —
//! is exercised rather than bypassed with analytic primitives.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec2, vec3, Transform, Vec2, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_render_core::environment::Environment;
use tequila_materials::material_type::MaterialType;
use tequila_render_core::material::Material;
use tequila_render_core::scene::{Instance, Mesh, SceneSnapshot};
use tequila_texture::image::{ColorSpace, Image};
use std::sync::Arc;

/// Tessellate a UV sphere.
///
/// Smooth vertex normals are computed analytically rather than averaged from
/// the faces: for a sphere the normal *is* the normalized position, so there
/// is no reason to approximate it, and the exact values make the shading
/// independent of tessellation density — which is the property CAD imports
/// need from Re-Tessellate at M8.
pub fn uv_sphere(radius: f32, segments: usize, rings: usize) -> Mesh {
    let segments = segments.max(3);
    let rings = rings.max(2);

    let mut positions = Vec::with_capacity((rings + 1) * (segments + 1));
    let mut normals = Vec::with_capacity((rings + 1) * (segments + 1));
    let mut uvs: Vec<Vec2> = Vec::with_capacity((rings + 1) * (segments + 1));

    for ring in 0..=rings {
        let v = ring as f32 / rings as f32;
        let theta = v * std::f32::consts::PI;
        let (sin_theta, cos_theta) = theta.sin_cos();
        for segment in 0..=segments {
            // The seam vertex is duplicated so that `u` can reach 1 there
            // rather than wrapping to 0, which would smear the whole texture
            // backwards across the last column of quads.
            let u = segment as f32 / segments as f32;
            let phi = u * std::f32::consts::TAU;
            let (sin_phi, cos_phi) = phi.sin_cos();
            let normal = vec3(sin_theta * cos_phi, cos_theta, sin_theta * sin_phi);
            positions.push(normal * radius);
            normals.push(normal);
            uvs.push(vec2(u, v));
        }
    }

    let mut indices = Vec::with_capacity(rings * segments * 6);
    let stride = segments + 1;
    for ring in 0..rings {
        for segment in 0..segments {
            let a = (ring * stride + segment) as u32;
            let b = a + stride as u32;
            // Wound counter-clockwise seen from outside, so that
            // `cross(v1 - v0, v2 - v0)` points away from the centre and agrees
            // with the vertex normals. Reversing these two lines inverts every
            // geometric normal on the sphere, which is what an inside-out
            // import looks like; the integrator carries both cases now, and
            // `tequila-render-cpu/tests/inverted_winding.rs` renders this
            // sphere both ways round to say so.
            //
            // Degenerate triangles at the poles are skipped: they contribute
            // nothing and hand the BVH builder zero-area primitives.
            if ring != 0 {
                indices.extend_from_slice(&[a, a + 1, b]);
            }
            if ring != rings - 1 {
                indices.extend_from_slice(&[a + 1, b + 1, b]);
            }
        }
    }

    Mesh::new(positions, indices)
        .with_normals(normals)
        .with_uvs(uvs)
}

/// A flat quad in the XZ plane, facing `+Y`.
pub fn ground_plane(half_size: f32) -> Mesh {
    let positions = vec![
        vec3(-half_size, 0.0, -half_size),
        vec3(half_size, 0.0, -half_size),
        vec3(half_size, 0.0, half_size),
        vec3(-half_size, 0.0, half_size),
    ];
    let normals = vec![Vec3::Y; 4];
    let uvs = vec![
        vec2(0.0, 0.0),
        vec2(1.0, 0.0),
        vec2(1.0, 1.0),
        vec2(0.0, 1.0),
    ];
    // Wound counter-clockwise seen from above, matching the `+Y` normals.
    Mesh::new(positions, vec![0, 2, 1, 0, 3, 2])
        .with_normals(normals)
        .with_uvs(uvs)
}

/// The studio every scene without an environment image is lit by.
///
/// Generated rather than shipped: `TEQUILA_SPEC.md` §0.2 rule 4 forbids
/// bundling another renderer's environment maps, and a studio is a few
/// numbers. What it is built to do is give materials something to show:
///
/// - **A neutral surround** — grey walls brightening towards the ceiling, a
///   darker floor, no colour cast. The first studio graded a blue sky over a
///   warm floor, which tinted every white part and read as a colour fault in
///   the material rather than in the light.
/// - **Lights with edges.** A large overhead softbox and four tall strips
///   around the product, built from the same rectangular pins as the HDRI
///   editor's. The first studio's sources were soft cones with no flat face
///   and no edge, which reflect as shapeless smudges; glossy plastic, paint
///   and metal read as glossy by the crisp shapes they mirror, and strips on
///   every side mean an edge highlight whichever way the camera looks.
/// - **Enough resolution to keep those edges.** 1024 across: three texels
///   a degree, so a reflected softbox has a clean border, without making the
///   pins painted over it slow to drag.
///
/// Tuned so a white product reads white from the front on a light grey
/// seamless floor, with the lights several times brighter than the walls so
/// gloss and chrome keep their contrast. It was first matched to the studio
/// it replaced, which lit from overhead and left the front of a product
/// mid-grey on a nearly black floor.
pub fn studio_environment() -> Environment {
    Environment::new(Arc::new(studio_image()), 1.0)
}

/// The studio's lights, as pins over the surround.
pub fn studio_lights() -> Vec<tequila_render_core::pins::Pin> {
    use tequila_render_core::pins::{softbox, strip};
    let mut lights = vec![
        // The packshot light: big, overhead, a little in front.
        tequila_render_core::pins::Pin {
            angular_size: tequila_core::math::Vec2 { x: 1.3, y: 0.9 },
            ..softbox("Overhead", 0.0, 72.0, 1.3, 0.25, STUDIO_OVERHEAD)
        },
    ];
    for (i, bearing) in [45.0f32, 135.0, 225.0, 315.0].into_iter().enumerate() {
        let mut light = strip(&format!("Strip {}", i + 1), bearing, 18.0, STUDIO_STRIP);
        light.softness = 0.2;
        lights.push(light);
    }
    lights
}

/// Radiance of the overhead softbox and of each strip.
const STUDIO_OVERHEAD: f32 = 3.2;
const STUDIO_STRIP: f32 = 2.4;
/// The surround: walls at the horizon, how much brighter they get towards
/// the ceiling, and the floor.
const STUDIO_WALL: f32 = 0.34;
const STUDIO_CEILING_LIFT: f32 = 0.10;
const STUDIO_FLOOR: f32 = 0.30;

/// The studio as an image: the surround, with its lights painted in.
pub fn studio_image() -> Image {
    tequila_render_core::pins::composite(&studio_surround(), &studio_lights())
}

/// The studio without its lights: what the camera sees behind the product.
///
/// The lights are pins, painted over this for lighting and reflection and
/// left out of the backdrop — a softbox standing in the background of a
/// packshot is a mistake in the photograph, not the look of one.
pub fn studio_surround() -> Image {
    const WIDTH: usize = 1024;
    const HEIGHT: usize = 512;
    let mut data = Vec::with_capacity(WIDTH * HEIGHT * 3);
    for y in 0..HEIGHT {
        let v = (y as f32 + 0.5) / HEIGHT as f32;
        for x in 0..WIDTH {
            let u = (x as f32 + 0.5) / WIDTH as f32;
            let up = tequila_render_core::pins::equirect_direction(u, v).y;
            // Walls brightening to the ceiling, a darker floor, and a soft
            // horizon between them rather than a seam.
            let wall = STUDIO_WALL + STUDIO_CEILING_LIFT * up.max(0.0).powf(0.8);
            let floor = STUDIO_FLOOR;
            let t = ((up + 0.06) / 0.12).clamp(0.0, 1.0);
            let blend = t * t * (3.0 - 2.0 * t);
            let grey = floor + (wall - floor) * blend;
            data.extend_from_slice(&[grey, grey, grey]);
        }
    }
    Image {
        width: WIDTH,
        height: HEIGHT,
        channels: 3,
        data,
        color_space: ColorSpace::LinearSrgb,
    }
}

/// The studio material thumbnails are shot in (`docs/04` §9).
///
/// Not the document's studio: that one lights a product in a room, evenly,
/// and a sphere in it reads as a grey ball whatever it is made of. A
/// thumbnail has to tell materials apart at ninety pixels, so this is lit
/// the way a swatch photograph is: one large soft key above and to the
/// camera's left for the highlight and the shading, a strip behind to the
/// right for a rim that shows gloss and Fresnel, a weak fill so the shadow
/// side is not black — and behind the sphere a pool of light falling off to
/// dark, which is what glass and clear plastic have to refract to look like
/// anything at all. The lights are kept out of the backdrop.
///
/// Built for the preview camera in `tequila_render_cpu::preview`, which
/// looks at the origin from `(2.1, 1.5, 3.0)`.
pub fn preview_environment() -> Environment {
    use tequila_render_core::pins::{softbox, Pin};
    let camera = vec3(2.1, 1.5, 3.0).normalize();
    let right = camera.cross(Vec3::Y).normalize() * -1.0;
    let left = right * -1.0;
    let behind = camera * -1.0;

    let light = |name: &str, direction: Vec3, size: Vec2, softness: f32, intensity: f32| Pin {
        direction: direction.normalize(),
        angular_size: size,
        ..softbox(name, 0.0, 0.0, size.x, softness, intensity)
    };
    let lights = [
        light("Key", camera * 0.5 + left * 0.6 + Vec3::Y * 0.9, vec2(1.1, 0.8), 0.8, 3.8),
        light("Rim", camera * -0.6 + right * 0.8 + Vec3::Y * 0.3, vec2(0.22, 1.2), 0.3, 5.0),
        // A second, slimmer strip behind on the other side: polished metal
        // and glass are drawn by their edges, and one rim leaves the other
        // edge to vanish into the backdrop.
        light("Rim left", camera * -0.5 + left * 0.85 + Vec3::Y * 0.2, vec2(0.12, 1.0), 0.35, 3.2),
        light("Fill", camera * 0.7 + right * 0.7 + Vec3::Y * 0.1, vec2(0.7, 0.7), 0.5, 0.7),
    ];

    const WIDTH: usize = 512;
    const HEIGHT: usize = 256;
    let mut data = Vec::with_capacity(WIDTH * HEIGHT * 3);
    for y in 0..HEIGHT {
        let v = (y as f32 + 0.5) / HEIGHT as f32;
        for x in 0..WIDTH {
            let u = (x as f32 + 0.5) / WIDTH as f32;
            let d = tequila_render_core::pins::equirect_direction(u, v);
            let t = ((d.y + 0.7) / 1.3).clamp(0.0, 1.0);
            let lift = t * t * (3.0 - 2.0 * t);
            let pool = d.dot(behind).max(0.0).powi(8);
            // A faint brighter band at the horizon: what a polished part
            // shows as the line between floor and wall, which is most of
            // what makes chrome read as chrome.
            let horizon = (-((d.y - 0.03) / 0.06).powi(2)).exp();
            let grey = 0.035 + 0.24 * lift + 0.35 * pool + 0.05 * horizon;
            data.extend_from_slice(&[grey, grey, grey]);
        }
    }
    let surround = Image { width: WIDTH, height: HEIGHT, channels: 3, data, color_space: ColorSpace::LinearSrgb };
    let lit = tequila_render_core::pins::composite(&surround, &lights);
    let mut environment = Environment::new(Arc::new(lit), 1.0);
    environment.set_camera_image(Some(Arc::new(surround)));
    environment
}

/// Which demonstration material the sphere wears.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DemoMaterial {
    Plastic,
    Metal,
    Diffuse,
}

impl DemoMaterial {
    pub const ALL: [DemoMaterial; 3] = [
        DemoMaterial::Plastic,
        DemoMaterial::Metal,
        DemoMaterial::Diffuse,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            DemoMaterial::Plastic => "Plastic",
            DemoMaterial::Metal => "Metal",
            DemoMaterial::Diffuse => "Diffuse",
        }
    }

    /// The catalog entry this demo material is an instance of, so the
    /// Material tab opens on the right set of controls.
    pub const fn material_type(self) -> MaterialType {
        match self {
            DemoMaterial::Plastic => MaterialType::Plastic,
            DemoMaterial::Metal => MaterialType::Metal,
            DemoMaterial::Diffuse => MaterialType::Diffuse,
        }
    }

    fn build(self, roughness: f32) -> Material {
        match self {
            DemoMaterial::Plastic => Material::plastic(
                Spectrum::from_srgb_encoded(vec3(0.85, 0.24, 0.20)),
                roughness,
            ),
            DemoMaterial::Metal => {
                Material::metal(tequila_materials::fresnel::metals::GOLD, roughness)
            }
            DemoMaterial::Diffuse => {
                Material::diffuse(Spectrum::from_srgb_encoded(vec3(0.85, 0.24, 0.20)))
            }
        }
    }
}

/// The same scene as a real [`SceneDocument`], built through the command bus.
///
/// This is what the application opens with, and it is deliberately not a
/// special case: it is an ordinary document, assembled by an ordinary
/// `Command::Import`, that can be selected, moved, regrouped, saved and
/// reopened like any other. A first-run scene wired in beside the document
/// would be a second path from geometry to pixels, and §1.1 allows exactly
/// one.
pub fn demo_document(material: DemoMaterial, roughness: f32) -> crate::SceneDocument {
    demo_document_sized(material, roughness, 1.0)
}

/// The radius of the ball the application opens with, in metres: ten
/// centimetres across, the size of a product.
///
/// Materials are real-world: glass reaches its colour over centimetres, and
/// wood has so many rings to the metre. On the two-metre ball the demo
/// scene is, amber bottle glass rendered black and a weave was too fine to
/// see — a first scene that makes the library look broken.
pub const STARTER_RADIUS: f32 = 0.05;

/// The demo scene with a ball of this radius, in metres.
pub fn demo_document_sized(material: DemoMaterial, roughness: f32, radius: f32) -> crate::SceneDocument {
    use crate::command::{reserve_keys, Command, ImportPayload};
    use crate::material::MaterialDefinition;
    use crate::tree::{NodeKind, SceneNode};
    use crate::CommandBus;

    let mut document = crate::SceneDocument::new();
    let mut bus = CommandBus::new();

    let (meshes, materials, ids) = reserve_keys(&document, 1, 1, 2);

    let mut root = SceneNode::new(ids[0], "Demo", NodeKind::Group);
    root.children = vec![ids[1]];

    let mut sphere = SceneNode::new(ids[1], "Sphere", NodeKind::Part { mesh: meshes[0] });
    sphere.parent = Some(ids[0]);
    // Resting *on* the ground plane, not intersecting it: a sphere sunk into
    // the floor has no contact shadow, which is the strongest cue that an
    // object is sitting on a surface.
    sphere.transform = Transform::translation(vec3(0.0, radius, 0.0));
    sphere.material = Some(materials[0]);

    // The floor is the *environment's* ground plane, which is on by default,
    // and this document deliberately does not ship one of its own.
    //
    // It used to, and the two were coplanar at y = 0. Every ray through the
    // floor then landed on whichever of them the sub-pixel jitter happened to
    // favour, so half the frame flickered between two greys and averaged into
    // a third — invisible in a converged path trace, and very visible as grain
    // that clears after every camera move once there was a mode fast enough to
    // show a single sample.
    //
    // A modelled floor is also the wrong floor. It is opaque, so it hides the
    // shadow catcher underneath and makes every control on the Environment
    // tab's Ground section appear to do nothing; it is a real diffuse surface,
    // so it bounces light back into the product that a catcher deliberately
    // does not; and it is a fixed size rather than one fitted to the scene.

    bus.apply(
        &mut document,
        Command::Import {
            payload: Box::new(ImportPayload {
                meshes: vec![(meshes[0], to_document_mesh(&uv_sphere(radius, 96, 48)))],
                textures: Vec::new(),
                materials: vec![(
                    materials[0],
                    MaterialDefinition::new(
                        material.label(),
                        material.material_type(),
                        material.build(roughness),
                    ),
                )],
                nodes: vec![root, sphere],
                roots: vec![ids[0]],
            }),
        },
    )
    .expect("the demo scene is well formed");
    document
}

/// The renderer's mesh in the document's layout.
///
/// The demo geometry is built in the renderer's form because that is what
/// `demo_scene` needs; the document's form carries the extra channels a user
/// can edit. Converting is cheaper than maintaining two builders that can
/// disagree about where the sphere is.
pub(crate) fn to_document_mesh(mesh: &Mesh) -> crate::geometry::Mesh {
    let mut out = crate::geometry::Mesh::new(mesh.positions.clone(), mesh.indices.clone());
    out.normals = mesh.normals.clone();
    if !mesh.uvs.is_empty() {
        out.uv_sets = vec![mesh.uvs.clone()];
    }
    out
}

/// Build the M0 scene: a sphere resting on a ground plane, lit by the studio
/// environment.
pub fn demo_scene(material: DemoMaterial, roughness: f32) -> SceneSnapshot {
    let radius = 1.0;
    let sphere = Arc::new(uv_sphere(radius, 96, 48));
    let ground = Arc::new(ground_plane(40.0));

    let materials = vec![
        material.build(roughness),
        // A neutral, slightly glossy floor. Mid-grey so it reads as a studio
        // sweep rather than tinting the subject's bounce light.
        Material::plastic(Spectrum::from_srgb_encoded(Vec3::splat(0.55)), 0.35),
    ];

    let instances = vec![
        // Resting *on* the plane, not intersecting it: a sphere sunk into the
        // floor has no contact shadow, which is the single strongest cue that
        // an object is actually sitting on a surface.
        Instance::new(InstanceId(0), MeshId(0), MaterialId(0))
            .with_transform(Transform::translation(vec3(0.0, radius, 0.0))),
        Instance::new(InstanceId(1), MeshId(1), MaterialId(1)),
    ];

    SceneSnapshot::new(
        vec![sphere, ground],
        instances,
        materials,
        Arc::new(studio_environment()),
    )
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn the_sphere_is_watertight_and_correctly_sized() {
        let mesh = uv_sphere(2.0, 32, 16);
        assert!(mesh.triangle_count() > 800);
        for p in &mesh.positions {
            assert!(
                (p.length() - 2.0).abs() < 1e-4,
                "vertex off the sphere: {p:?}"
            );
        }
        // Bounds must be the full sphere, not a hemisphere.
        let b = mesh.bounds();
        assert!((b.min.y + 2.0).abs() < 1e-3 && (b.max.y - 2.0).abs() < 1e-3);
    }

    #[test]
    fn sphere_normals_are_exact_and_outward() {
        let mesh = uv_sphere(1.0, 24, 12);
        for (p, n) in mesh.positions.iter().zip(&mesh.normals) {
            assert!((n.length() - 1.0).abs() < 1e-5);
            // For a sphere the normal is the normalized position exactly.
            assert!(
                (p.normalize() - *n).length() < 1e-5,
                "normal is not the radial direction"
            );
        }
    }

    /// Degenerate triangles at the poles would be handed to the BVH builder
    /// and contribute nothing but trouble.
    #[test]
    fn the_sphere_has_no_zero_area_triangles() {
        let mesh = uv_sphere(1.0, 32, 16);
        for t in 0..mesh.triangle_count() {
            let area = mesh.geometric_normal_scaled(t).length() * 0.5;
            assert!(area > 1e-9, "triangle {t} has area {area}");
        }
    }

    /// The seam vertex is duplicated so `u` reaches 1 rather than wrapping,
    /// which would smear the texture across the last column of quads.
    #[test]
    fn the_uv_seam_is_split() {
        let mesh = uv_sphere(1.0, 8, 4);
        let has_zero = mesh.uvs.iter().any(|uv| uv.x == 0.0);
        let has_one = mesh.uvs.iter().any(|uv| uv.x == 1.0);
        assert!(has_zero && has_one, "the UV seam was not split");
    }

    #[test]
    fn triangle_windings_face_outward() {
        let mesh = uv_sphere(1.0, 16, 8);
        for t in 0..mesh.triangle_count() {
            let [a, b, c] = mesh.triangle(t);
            let centroid = (mesh.positions[a as usize]
                + mesh.positions[b as usize]
                + mesh.positions[c as usize])
                / 3.0;
            let normal = mesh.geometric_normal_scaled(t);
            assert!(normal.dot(centroid) > 0.0, "triangle {t} is wound inwards");
        }
    }

    /// Triangle winding must agree with the authored vertex normals.
    ///
    /// This is the invariant the renderer depends on: it orients the shading
    /// normal using the *geometric* one, so if the two disagree the shading
    /// normal ends up pointing into the surface and every path terminates
    /// immediately. The image comes out black with a perfectly correct
    /// coverage mask, which is a confusing thing to debug from.
    #[test]
    fn winding_agrees_with_the_vertex_normals() {
        for mesh in [uv_sphere(1.0, 24, 12), ground_plane(3.0)] {
            for t in 0..mesh.triangle_count() {
                let geometric = mesh.geometric_normal_scaled(t).normalize();
                let [a, b, c] = mesh.triangle(t);
                let shading = (mesh.normals[a as usize]
                    + mesh.normals[b as usize]
                    + mesh.normals[c as usize])
                    .normalize();
                assert!(
                    geometric.dot(shading) > 0.0,
                    "triangle {t}: geometric {geometric:?} opposes shading {shading:?}"
                );
            }
        }
    }

    #[test]
    fn the_ground_faces_up() {
        let mesh = ground_plane(5.0);
        for t in 0..mesh.triangle_count() {
            assert!(mesh.geometric_normal_scaled(t).normalize().y > 0.99);
        }
    }

    /// The environment must have real contrast for the importance sampler to
    /// have anything to do, and no negative or non-finite values.
    #[test]
    fn the_studio_environment_is_bright_above_and_dim_below() {
        let env = studio_environment();
        let up = env.radiance(Vec3::Y).luminance();
        let down = env.radiance(-Vec3::Y).luminance();
        assert!(up > down * 2.0, "sky {up} vs floor {down}");

        // The lights must be much brighter than the surround, or importance
        // sampling has nothing to importance-sample. Measured against the
        // surround rather than as a fixed number: the studio's units are its
        // own, and a larger, dimmer softbox is still a light.
        let mut values = Vec::with_capacity(20_000);
        let mut rng = tequila_core::rng::Pcg32::new(1, 1);
        for _ in 0..20_000 {
            let d = tequila_core::sampling::sample_uniform_sphere(vec2(
                rng.next_f32(),
                rng.next_f32(),
            ));
            let l = env.radiance(d).luminance();
            assert!(l.is_finite() && l >= 0.0, "bad radiance {l}");
            values.push(l);
        }
        values.sort_by(f32::total_cmp);
        let (median, brightest) = (values[values.len() / 2], values[values.len() - 1]);
        assert!(brightest > median * 10.0, "no strong light: peak {brightest} against a median of {median}");
    }

    #[test]
    fn the_sphere_rests_on_the_ground_rather_than_sinking_into_it() {
        let scene = demo_scene(DemoMaterial::Plastic, 0.2);
        let sphere = &scene.meshes[scene.instances[0].mesh.index()];
        let bounds = sphere.bounds().transformed(&scene.instances[0].transform);
        assert!(
            bounds.min.y.abs() < 1e-3,
            "sphere bottom is at {} rather than resting at y = 0",
            bounds.min.y
        );
    }

    /// The demo scene as a document must be a real, valid document — the
    /// application opens with it, and a first-run scene that cannot be saved
    /// or edited would be a lie about what the app does.
    #[test]
    fn the_demo_document_is_a_valid_editable_document() {
        let document = demo_document(DemoMaterial::Plastic, 0.18);

        document.validate().expect("the demo document is valid");
        assert_eq!(document.tree.len(), 2);
        assert_eq!(document.geometry.len(), 1);
        assert_eq!(document.materials.len(), 1);

        // And it renders the same geometry `demo_scene` does.
        let (snapshot, _) = crate::snapshot::snapshot(
            &document,
            std::sync::Arc::new(studio_environment()),
            &mut crate::snapshot::MeshCache::new(),
        );
        // One part plus the shadow catcher, which is on by default and is the
        // scene's only floor.
        assert_eq!(snapshot.instances.len(), 2);
        assert_eq!(
            snapshot
                .instances
                .iter()
                .filter(|i| i.shadow_catcher.is_some())
                .count(),
            1
        );
        let triangles = |s: &tequila_render_core::scene::SceneSnapshot| -> usize {
            s.instances
                .iter()
                .filter_map(|i| s.meshes.get(i.mesh.index()))
                .map(|m| m.triangle_count())
                .sum()
        };
        let modelled = |s: &tequila_render_core::scene::SceneSnapshot| -> usize {
            s.instances
                .iter()
                .filter(|i| i.shadow_catcher.is_none())
                .filter_map(|i| s.meshes.get(i.mesh.index()))
                .map(|m| m.triangle_count())
                .sum()
        };
        let _ = triangles;
        // The snapshot-only demo carries a modelled floor because it is built
        // without a document and therefore without an environment ground; the
        // document form gets its floor from the environment instead. So the
        // two agree on the product and not on the floor, and comparing the
        // product is the comparison that means anything.
        let sphere_triangles = uv_sphere(1.0, 96, 48).triangle_count();
        assert_eq!(modelled(&snapshot), sphere_triangles);
        assert!(modelled(&demo_scene(DemoMaterial::Plastic, 0.18)) > sphere_triangles);
    }

    /// The sphere must rest on the plane in the document form too, or the
    /// contact shadow that makes the scene read is gone.
    #[test]
    fn the_document_sphere_rests_on_the_ground() {
        let document = demo_document(DemoMaterial::Plastic, 0.18);
        let bounds = crate::snapshot::visible_bounds(&document);
        assert!(
            bounds.min.y.abs() < 1e-3,
            "the scene does not sit on y = 0: {bounds:?}"
        );
    }

    #[test]
    fn the_demo_scene_is_well_formed() {
        for material in DemoMaterial::ALL {
            let scene = demo_scene(material, 0.25);
            assert_eq!(scene.instances.len(), 2);
            assert!(scene.triangle_count() > 1000);
            for instance in &scene.instances {
                assert!(instance.mesh.index() < scene.meshes.len());
                assert!(instance.material.index() < scene.materials.len());
            }
            assert!(!material.label().is_empty());
        }
    }
}
