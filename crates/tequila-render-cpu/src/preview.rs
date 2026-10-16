//! Material preview thumbnails (`docs/04-UX-SPEC.md` §9).
//!
//! §9 asks for thumbnails "rendered by the actual renderer on a standard
//! preview scene", and the word that matters is *actual*. A library of
//! thumbnails drawn by some cheaper approximation is a library of pictures that
//! do not predict what a material will look like — the gold reads too flat, the
//! glass reads as grey plastic, and people learn to ignore the thumbnails and
//! apply materials one at a time to find out. Rendering them with the path
//! tracer that will render the final image means a thumbnail is a promise.
//!
//! The cost is managed by the other half of §9: generated lazily on first
//! display, and cached keyed by content hash, so a material is previewed once
//! however many times it is scrolled past, and re-previewed exactly when it
//! changes.
//!
//! ## The preview scene
//!
//! A sphere on a sweep, lit from two sides. Deliberately not a "shader ball"
//! with bevels, holes and a cloth drape: those exist to show off a renderer,
//! and at 96 pixels the extra geometry is a few grey pixels that crowd out the
//! thing being previewed. A sphere shows the highlight shape, the Fresnel falloff
//! at the silhouette, and the colour, which is what distinguishes one material
//! from another at this size.

use crate::embree::{Device, EmbreeIntersector};
use crate::renderer::{render_pass, RenderContext};
use tequila_core::math::{vec3, Transform, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::graph::MaterialGraph;
use tequila_materials::openpbr::Material;
use tequila_render_core::shade::TextureTable;
use tequila_render_core::camera::Camera;
use tequila_render_core::environment::Environment;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_render_core::scene::{Instance, Mesh, SceneSnapshot};
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_core::settings::IntegratorConfig;
use std::sync::Arc;

/// A rendered thumbnail: linear RGB, row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct Thumbnail {
    pub width: usize,
    pub height: usize,
    /// `width * height * 3` scene-linear values.
    pub pixels: Vec<f32>,
    /// The denoiser's guides, laid out as `pixels` is: at a thumbnail's
    /// sample count the picture needs denoising, and the denoiser needs
    /// these to keep edges and texture.
    pub albedo: Vec<f32>,
    pub normal: Vec<f32>,
}

impl Thumbnail {
    pub fn pixel(&self, x: usize, y: usize) -> Spectrum {
        let at = (y * self.width + x) * 3;
        Spectrum {
            r: self.pixels[at],
            g: self.pixels[at + 1],
            b: self.pixels[at + 2],
        }
    }

    /// Mean luminance, for tests and for deciding whether a preview came out
    /// black because the material is black or because something went wrong.
    pub fn mean_luminance(&self) -> f32 {
        if self.pixels.is_empty() {
            return 0.0;
        }
        let mut total = 0.0;
        for y in 0..self.height {
            for x in 0..self.width {
                total += self.pixel(x, y).luminance();
            }
        }
        total / (self.width * self.height) as f32
    }
}

/// What the preview is rendered on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PreviewShape {
    /// A sphere on a sweep. Reads clearly at thumbnail size.
    #[default]
    Sphere,
    /// A rounded slab, which shows anisotropy and flake orientation better
    /// because it has flat area at a range of angles.
    Slab,
}

/// How much work a preview is worth.
#[derive(Clone, Copy, Debug)]
pub struct PreviewSettings {
    pub size: usize,
    pub samples: u32,
    pub shape: PreviewShape,
    /// The sphere's radius in metres, the slab sized to match: the library's
    /// five-centimetre ball by default, or a part's own size to see a
    /// material as it will look on that part.
    pub radius: f32,
}

impl Default for PreviewSettings {
    fn default() -> Self {
        PreviewSettings {
            // Large enough to read a highlight, small enough that sixty of them
            // are not a stall. The browser scales this down for its grid.
            size: 96,
            // Enough for the denoiser to work from; clear materials get more,
            // below.
            samples: 48,
            shape: PreviewShape::Sphere,
            radius: PREVIEW_RADIUS,
        }
    }
}

/// Render one material on the preview scene.
///
/// The device is passed in rather than created here: building an Embree device
/// is not free, and a library browser renders a great many of these.
pub fn render_thumbnail(
    device: &Arc<Device>,
    material: &Material,
    environment: Arc<Environment>,
    settings: PreviewSettings,
) -> Result<Thumbnail, String> {
    render_thumbnail_graph(device, &MaterialGraph::from_material(material.clone()), environment, settings)
}

/// Render a material with its graph: patterns and all, as it will render.
///
/// Patterns only — with no textures to look up, a graph that reads a
/// picture shows the picture's fallback. See [`render_thumbnail_textured`].
pub fn render_thumbnail_graph(
    device: &Arc<Device>,
    graph: &MaterialGraph,
    environment: Arc<Environment>,
    settings: PreviewSettings,
) -> Result<Thumbnail, String> {
    render_thumbnail_textured(device, graph, Arc::new(TextureTable::new()), environment, settings)
}

/// The same, with the pictures the graph reads: a document's material,
/// previewed with the project's own textures and labels rather than their
/// fallbacks.
pub fn render_thumbnail_textured(
    device: &Arc<Device>,
    graph: &MaterialGraph,
    textures: Arc<TextureTable>,
    environment: Arc<Environment>,
    settings: PreviewSettings,
) -> Result<Thumbnail, String> {
    let material = graph.material();
    let radius = if settings.radius.is_finite() && settings.radius > 0.0 { settings.radius } else { PREVIEW_RADIUS };
    let mut scene = preview_scene(material, settings.shape, environment, radius);
    if graph.len() > 1 || !graph.decals.is_empty() {
        if let Ok(program) = graph.compile() {
            // A Volume Map, fitted to the sphere as the render fits it to a
            // part, so a VDB loaded onto Density shows its shape here.
            let field = tequila_render_core::density::volume_of(&program)
                .and_then(|volume| textures.volume(volume).cloned())
                .and_then(|grid| {
                    let mesh = scene.meshes.first()?;
                    let mut low = tequila_core::math::Vec3::splat(f32::INFINITY);
                    let mut high = tequila_core::math::Vec3::splat(f32::NEG_INFINITY);
                    for p in &mesh.positions {
                        low = low.min(*p);
                        high = high.max(*p);
                    }
                    let placement = scene.instances.first()?.transform;
                    tequila_render_core::density::DensityField::fitted(grid, (low, high), placement)
                });
            scene = scene.with_programs(vec![Some(Arc::new(program)), None], textures);
            if let Some(field) = field {
                scene = scene.with_densities(vec![Some(Arc::new(field)), None]);
            }
        }
    }
    let intersector = EmbreeIntersector::build(device, scene)?;

    // Glass and clear plastic reflect a few percent and refract the rest, and
    // which a sample does is chosen at random, so the reflection of a light
    // arrives as a few very bright samples among many dark ones: at a
    // thumbnail's sample count it is thirty-percent noise, which the
    // denoiser smears into blotches. Twice the resolution and three times the
    // samples puts twelve times the samples behind every pixel the tile
    // shows, and gives the denoiser room; the tile is drawn at its own size,
    // so the extra resolution costs nothing on screen. For the few clear
    // materials only — everything else converges at the plain count.
    let transmissive = material.transmission_weight > 0.0;
    let (size, samples) = if transmissive {
        (settings.size * 2, settings.samples * 3)
    } else {
        (settings.size, settings.samples)
    };
    let mut framebuffer = Framebuffer::new(size, size);
    let context = RenderContext {
        camera: preview_camera(radius),
        // No adaptive sampling: at this sample count the variance estimate it
        // needs is itself too noisy to be worth acting on.
        // The floor is a shadow catcher that is also a floor for the light
        // bouncing off it, so polished metal mirrors a surface below it
        // rather than a black void.
        config: IntegratorConfig {
            adaptive_sampling_threshold: 0.0,
            ground_illumination: true,
            ..IntegratorConfig::product()
        },
        region: None,
        ..RenderContext::default()
    };
    let schedule = ProgressiveSchedule::default();
    render_pass(&intersector, &mut framebuffer, &context, &schedule, samples);

    let guides = framebuffer.to_denoiser_buffers();
    Ok(Thumbnail {
        width: size,
        height: size,
        pixels: guides.color,
        albedo: guides.albedo,
        normal: guides.normal,
    })
}

/// The preview sphere's radius, in metres: a ball five centimetres across,
/// built at that size.
///
/// A material's lengths are real ones — how far light goes through it before
/// reaching its colour, how far it scatters under the surface, how big its
/// flakes are, how many rings of wood or threads of weave there are to the
/// metre — and mean nothing on a sphere of no particular size. It was two
/// metres across: tinted glass previewed as a black ball and every pattern
/// forty times too coarse. This is roughly the scale of the parts materials
/// are made for, so a swatch looks the way the material will.
pub const PREVIEW_RADIUS: f32 = 0.025;

/// Three-quarter view, slightly above: the angle that shows a highlight, a
/// shaded side and the silhouette at once.
fn preview_camera(radius: f32) -> Camera {
    Camera::look_at(vec3(2.1, 1.5, 3.0) * radius, vec3(0.0, -0.05, 0.0) * radius, Vec3::Y)
}

/// The standard preview scene: the subject on a floor that catches its
/// shadow and is otherwise the backdrop — a seamless sweep with no horizon.
fn preview_scene(
    material: &Material,
    shape: PreviewShape,
    environment: Arc<Environment>,
    radius: f32,
) -> SceneSnapshot {
    let subject = Arc::new(match shape {
        PreviewShape::Sphere => sphere(radius, 96, 48),
        PreviewShape::Slab => slab(1.3 * radius, 0.45 * radius, radius),
    });
    let sweep = Arc::new(ground(20.0 * radius));

    // What the floor is to light bouncing off it. Mid-grey rather than
    // white: a white floor blows out under a dark material and the
    // thumbnail becomes a silhouette, which is the one thing it must not be.
    let backdrop = Material {
        base_color: Spectrum::splat(0.22),
        specular_roughness: 0.6,
        ..Material::default()
    };

    SceneSnapshot::new(
        vec![subject, sweep],
        vec![
            Instance::new(InstanceId(0), MeshId(0), MaterialId(0)),
            Instance::new(InstanceId(1), MeshId(1), MaterialId(1))
                .with_transform(Transform::translation(vec3(0.0, -radius, 0.0)))
                .with_shadow_catcher(tequila_render_core::scene::ShadowCatcher {
                    shadow_color: Spectrum::ZERO,
                    reflections: 0.08,
                    // A hint of the sphere in the floor, fainter seen from
                    // above, as it always was on the shelf.
                    reflection_fresnel: true,
                    occlusion_only: false,
                    occlusion_radius: 0.5 * radius,
                    ..Default::default()
                }),
        ],
        vec![material.clone(), backdrop],
        environment,
    )
}

fn sphere(radius: f32, segments: usize, rings: usize) -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();

    for ring in 0..=rings {
        let v = ring as f32 / rings as f32;
        let theta = v * std::f32::consts::PI;
        for segment in 0..=segments {
            let u = segment as f32 / segments as f32;
            let phi = u * std::f32::consts::TAU;
            let n = vec3(
                theta.sin() * phi.cos(),
                theta.cos(),
                theta.sin() * phi.sin(),
            );
            positions.push(n * radius);
            normals.push(n);
            // Latitude and longitude, for a pattern mapped by UV.
            uvs.push(tequila_core::math::vec2(u, v));
        }
    }
    for ring in 0..rings {
        for segment in 0..segments {
            let a = (ring * (segments + 1) + segment) as u32;
            let b = a + segments as u32 + 1;
            // Wound counter-clockwise seen from outside. Getting this
            // backwards makes every ray hit a backface, and the sphere renders
            // pure black whatever material is on it — which reads as a broken
            // material rather than as inside-out geometry.
            indices.extend_from_slice(&[a, a + 1, b, a + 1, b + 1, b]);
        }
    }
    Mesh::new(positions, indices).with_normals(normals).with_uvs(uvs)
}

fn slab(width: f32, height: f32, depth: f32) -> Mesh {
    let (x, y, z) = (width * 0.5, height * 0.5, depth * 0.5);
    let corners = [
        vec3(-x, -y, -z),
        vec3(x, -y, -z),
        vec3(x, y, -z),
        vec3(-x, y, -z),
        vec3(-x, -y, z),
        vec3(x, -y, z),
        vec3(x, y, z),
        vec3(-x, y, z),
    ];
    // Faces wound outwards, each with its own vertices so the normals are flat
    // rather than averaged across an edge.
    let faces: [[usize; 4]; 6] = [
        [0, 3, 2, 1],
        [4, 5, 6, 7],
        [0, 1, 5, 4],
        [2, 3, 7, 6],
        [0, 4, 7, 3],
        [1, 2, 6, 5],
    ];
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    for face in faces {
        let base = positions.len() as u32;
        let (a, b, c) = (corners[face[0]], corners[face[1]], corners[face[2]]);
        let normal = (b - a).cross(c - a).normalize();
        for index in face {
            positions.push(corners[index]);
            normals.push(normal);
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    Mesh::new(positions, indices).with_normals(normals)
}

fn ground(half: f32) -> Mesh {
    let positions = vec![
        vec3(-half, 0.0, -half),
        vec3(half, 0.0, -half),
        vec3(half, 0.0, half),
        vec3(-half, 0.0, half),
    ];
    let normals = vec![Vec3::Y; 4];
    Mesh::new(positions, vec![0, 2, 1, 0, 3, 2]).with_normals(normals)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_materials::material_type::MaterialType;

    fn studio() -> Arc<Environment> {
        Arc::new(Environment::uniform(Spectrum::splat(1.0)))
    }

    fn device() -> Arc<Device> {
        Device::shared().expect("an Embree device")
    }

    fn preview(material: &Material) -> Thumbnail {
        render_thumbnail(
            &device(),
            material,
            studio(),
            PreviewSettings {
                size: 32,
                samples: 24,
                radius: PREVIEW_RADIUS,
                    shape: PreviewShape::Sphere,
            },
        )
        .expect("the preview scene always builds")
    }


    /// The basic promise: a preview shows *something*, and it is finite.
    /// A material's lengths are real ones: amber bottle glass reaches its
    /// colour over two centimetres. Previewed on a sphere two *metres*
    /// across, the light never got through and the swatch was a black ball;
    /// the preview has to be an object of a size such a material is made for.
    #[test]
    fn tinted_glass_previews_its_colour_not_black() {
        let amber = Material {
            transmission_color: Spectrum { r: 0.55, g: 0.2, b: 0.023 },
            transmission_depth: 0.02,
            ..MaterialType::GlassSolid.defaults()
        };
        let preview = render_thumbnail(&device(), &amber, studio(), PreviewSettings::default()).unwrap();
        let (size, mut through) = (preview.width, Spectrum::ZERO);
        for y in size * 2 / 5..size * 3 / 5 {
            for x in size * 2 / 5..size * 3 / 5 {
                through = through + preview.pixel(x, y);
            }
        }
        let rgb = through.to_linear_srgb();
        assert!(rgb.x > 2.0 * rgb.y && rgb.x > 0.5, "amber glass previews as {rgb:?}");
    }

    /// A document's material previews with the document's pictures. With no
    /// texture table the Material tab's sphere showed a textured material's
    /// fallback — a plain grey stand-in for a model's painted surface.
    #[test]
    fn a_textured_material_previews_with_its_texture() {
        use tequila_materials::graph::{Node, NodeKind, Socket, TextureId, Value};
        use tequila_materials::material_type::Param;
        use tequila_texture::image::{ColorSpace, Image};
        use tequila_texture::texture::{MapRole, Texture};

        let mut graph = MaterialGraph::from_material(MaterialType::Diffuse.defaults());
        let picture = graph.add_node(Node::new(NodeKind::ImageTexture {
            texture: TextureId(7),
            fallback: Value::Color(Spectrum::splat(0.5)),
        }));
        let output = graph.output();
        graph.connect(picture, output, Socket::Param(Param::BaseColor));

        let red = Image { width: 2, height: 2, channels: 3, data: [0.8, 0.02, 0.02].repeat(4), color_space: ColorSpace::LinearSrgb };
        let mut textures = TextureTable::new();
        textures.insert(TextureId(7), Arc::new(Texture::from_linear(red, MapRole::Color, Default::default())));

        let redness = |t: &Thumbnail| {
            let c = t.pixel(t.width / 2, t.height / 2).to_linear_srgb();
            c.x - c.y
        };
        let with = render_thumbnail_textured(&device(), &graph, Arc::new(textures), studio(), PreviewSettings::default()).unwrap();
        let without = render_thumbnail_graph(&device(), &graph, studio(), PreviewSettings::default()).unwrap();
        assert!(redness(&with) > 0.2, "the texture did not reach the preview: {}", redness(&with));
        assert!(redness(&without) < 0.05, "the control is red without its texture");
    }

    #[test]
    fn a_preview_renders_a_finite_image() {
        let thumbnail = preview(&MaterialType::Plastic.defaults());
        assert_eq!(thumbnail.width, 32);
        assert_eq!(thumbnail.pixels.len(), 32 * 32 * 3);
        assert!(
            thumbnail.pixels.iter().all(|v| v.is_finite() && *v >= 0.0),
            "the preview contains a non-finite or negative value"
        );
        assert!(
            thumbnail.mean_luminance() > 0.01,
            "the preview came out black"
        );
    }

    /// The point of rendering previews with the real renderer: materials that
    /// look different must preview differently. A cheaper approximation that
    /// collapsed them would make the library useless without being obviously
    /// broken.
    #[test]
    fn materials_that_differ_preview_differently() {
        let white = preview(&Material {
            base_color: Spectrum::splat(0.9),
            ..MaterialType::Plastic.defaults()
        });
        let black = preview(&Material {
            base_color: Spectrum::splat(0.02),
            ..MaterialType::Plastic.defaults()
        });
        // Over the sphere, not the frame: the floor is a shadow catcher
        // showing the backdrop, which is the same behind every material.
        let subject = |t: &Thumbnail| {
            let (from, to) = (t.width * 2 / 5, t.width * 3 / 5);
            let mut total = 0.0;
            for y in from..to {
                for x in from..to {
                    total += t.pixel(x, y).luminance();
                }
            }
            total / ((to - from) * (to - from)) as f32
        };
        assert!(
            subject(&white) > subject(&black) * 2.0,
            "white previewed at {:.4} and black at {:.4}",
            subject(&white),
            subject(&black)
        );

        // ...and a metal must not look like a plastic of the same colour.
        let metal = preview(&MaterialType::Metal.defaults());
        let plastic = preview(&MaterialType::Plastic.defaults());
        let difference: f32 = (0..32 * 32 * 3)
            .map(|i| (metal.pixels[i] - plastic.pixels[i]).abs())
            .sum::<f32>()
            / (32.0 * 32.0 * 3.0);
        assert!(
            difference > 0.01,
            "a metal and a plastic previewed almost identically ({difference:.5} \
             mean difference), so the thumbnail does not predict the render"
        );
    }

    /// Every catalog type has to preview without panicking — a library that
    /// crashes on one entry crashes on scroll.
    #[test]
    fn every_catalog_type_previews() {
        for material_type in MaterialType::ALL {
            let thumbnail = render_thumbnail(
                &device(),
                &material_type.defaults(),
                studio(),
                PreviewSettings {
                    size: 16,
                    samples: 8,
                    radius: PREVIEW_RADIUS,
                    shape: PreviewShape::Sphere,
                },
            )
            .expect("builds");
            assert!(
                thumbnail.pixels.iter().all(|v| v.is_finite()),
                "{material_type:?} previewed with a non-finite value"
            );
        }
    }

    /// A mirror and a metal a hair rougher look the same: below a roughness
    /// of about 0.032 a lobe is treated as a perfect mirror, and the floor it
    /// reflects must not change across that line. It did - the mirror saw
    /// the backdrop composited with the floor's shadow and the near-mirror a
    /// lit floor - and a polished part with any roughness pattern showed dark
    /// patches with hard edges where the pattern crossed it.
    #[test]
    fn a_mirror_and_a_near_mirror_see_the_same_floor() {
        let render = |roughness: f32| {
            let mut gold = MaterialType::Metal.defaults();
            gold.metal_ior = tequila_materials::fresnel::metals::ALL[2].1;
            gold.specular_roughness = roughness;
            let settings = PreviewSettings { size: 48, samples: 64, radius: PREVIEW_RADIUS, shape: PreviewShape::Sphere };
            render_thumbnail(&device(), &gold, studio(), settings).expect("builds")
        };
        // The lower half of the ball, where it reflects the floor.
        let floor_in_ball = |t: &Thumbnail| {
            let (mut sum, mut n) = (0.0, 0);
            for y in 26..36 {
                for x in 12..36 {
                    sum += t.pixel(x, y).luminance();
                    n += 1;
                }
            }
            sum / n as f32
        };
        let (mirror, near) = (floor_in_ball(&render(0.0315)), floor_in_ball(&render(0.0317)));
        assert!((mirror / near - 1.0).abs() < 0.1, "the floor in a mirror reads {mirror:.4}, in a near-mirror {near:.4}");
    }

    /// Both shapes work. The slab exists for anisotropy, which a sphere shows
    /// poorly.
    #[test]
    fn both_preview_shapes_render() {
        for shape in [PreviewShape::Sphere, PreviewShape::Slab] {
            let thumbnail = render_thumbnail(
                &device(),
                &MaterialType::Metal.defaults(),
                studio(),
                PreviewSettings {
                    size: 24,
                    samples: 16,
                    shape,
                    radius: PREVIEW_RADIUS,
                },
            )
            .expect("builds");
            assert!(
                thumbnail.mean_luminance() > 0.01,
                "{shape:?} rendered black"
            );
        }
    }

    /// Rendering the same material twice must give the same image, or the
    /// content-hash cache §9 asks for would be pointless.
    #[test]
    fn previews_are_deterministic() {
        let material = MaterialType::Paint.defaults();
        assert_eq!(
            preview(&material),
            preview(&material),
            "two previews of one material differed, so caching them by content \
             hash would serve a stale or arbitrary image"
        );
    }
}
