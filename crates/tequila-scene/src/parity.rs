//! The parity set (`docs/07` R0.3): the scenes Tequila's renderer on the
//! graphics card is held to its renderer on the processor with.
//!
//! Small, so they render in seconds on a processor and can be checked on
//! every pull request, and fixed - size, samples and seed - so the two
//! backends trace the same paths with the same random numbers and their
//! pictures can be held to a tight tolerance rather than a statistical
//! one (`docs/07` D7, R2.6). Between them they are to use every feature in
//! R3's tiers: R2 starts the set with one scene, and each R3 step adds the
//! scene for the feature it ports, here, with what it covers.
//!
//! Generated, as the benchmark scenes are (`bench`), so they are the same
//! on every machine and in every version.

use crate::bench::{lamp_lit, product_camera, simple_product, simple_product_mapped};
use crate::command::{Command, EnvParam, EnvValue, EnvironmentParameter};
use crate::{CommandBus, SceneDocument};
use tequila_core::math::{vec2, vec3, Transform};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::MaterialType;
use tequila_render_core::camera::Camera;
use tequila_render_core::material::Material;
use tequila_render_core::settings::IntegratorConfig;
use std::sync::Arc;
use tequila_materials::graph::{Node, NodeKind as GraphKind, Socket, TextureId, Value};
use tequila_materials::material_type::Param;
use tequila_texture::image::{ColorSpace, Image};
use tequila_texture::mip::Wrap;
use tequila_texture::projection::{Axis, Mapping, Projection};
use tequila_texture::texture::{MapRole, Texture};

/// One scene of the set, and how it is rendered.
#[derive(Debug)]
pub struct ParityScene {
    pub name: &'static str,
    /// What in `docs/07` R3 it exercises.
    pub covers: &'static str,
    pub width: usize,
    pub height: usize,
    pub samples: u32,
    pub seed: u32,
    build: fn() -> SceneDocument,
    /// What it does to the product's camera as framed: a lens, a projection.
    lens: fn(Camera) -> Camera,
    /// The pictures its materials sample, made here (no image file is
    /// shipped), by the ids its document gives them.
    pictures: fn() -> Vec<(TextureId, Texture)>,
}

impl ParityScene {
    pub fn document(&self) -> SceneDocument {
        (self.build)()
    }

    /// The camera it is seen from: framed on its product, through its lens.
    pub fn camera(&self, document: &SceneDocument) -> Camera {
        (self.lens)(product_camera(document, self.width as f32 / self.height as f32))
    }

    /// A cache to snapshot the scene through, holding its pictures: the
    /// snapshot takes them from it as it would a file's.
    pub fn cache(&self) -> crate::snapshot::MeshCache {
        let mut cache = crate::snapshot::MeshCache::new();
        for (id, texture) in (self.pictures)() {
            cache.insert_texture(id, Arc::new(texture));
        }
        cache
    }
}

/// The set.
pub const SET: &[ParityScene] = &[ParityScene {
    name: "grey_product",
    covers: "R2.5: the camera, instanced triangles, one grey diffuse surface, the environment sampled with next-event \
             estimation and MIS, Russian roulette and the bounce limit",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: grey_product,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "product_materials",
    covers: "R3 tier A, the surface: the OpenPBR stack's opaque lobes - plastic (a specular interface over a \
             diffuse substrate, with two mirror-smooth ones until the roughness floor), aluminium and gold \
             metal, a diffuse foot - with the stack's lobe choice and aim, mirror bounces' MIS and the \
             specular chain's clamps",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: simple_product,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "product_floor",
    covers: "R3 tier A, the floor: product_materials standing on the environment's ground, a shadow catcher with \
             the Headphones' settings - shadows in black, reflections at 0.35, 3 m across - its shadow drawn from \
             the sky's light onto a floor facing up, its mirror reflection a path of its own (three bounces, the \
             same sampler), the backdrop behind it and the coverage it takes, the catcher's view through mirrors \
             and near mirrors (a draw of the open stream), and a floor a bounce meets passed straight through",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: product_floor,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "product_floor_glossy",
    covers: "R3 tier A, the floor's other reflection: blurred (a GGX facet of roughness 0.3 drawn on the floor's \
             bounce dimensions), weakened by Fresnel, fading out by half the product's height, over a tinted \
             shadow",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: product_floor_glossy,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "product_floor_studio",
    covers: "R3 tier A, the studio floor: contact shadows by ambient occlusion rather than the sky, a bounce the \
             product sees and the camera does not (glowing, darker toward the contact), and a reflection fading by \
             how near the product is to the floor point",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: product_floor_studio,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "product_floor_lit",
    covers: "R3 tier A, ground illumination: the default ground's shadow, with the floor a bounce meets shaded as \
             the unassigned grey rather than passed",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: product_floor_lit,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "lamp_lit",
    covers: "R3 tier A, the lamps: the bench's lamp-lit product, the environment turned down to nothing and three              lamps in its place - a rectangular key and a disc fill, each with its stand-in the camera sees (the disc's              a square), and a spot rim weak enough beside them that the light hierarchy chooses - next-event              estimation of one lamp chosen down the hierarchy by importance, the rectangle by its spherical              rectangle (by area near its plane), the disc by area, the spot as a delta; shadow rays that pass the              stand-ins; the bounces off sharp reflections that meet the panels along their way (a draw of the open              stream between roughness 0.5 and 0.8), shared with the light sample by its choice from there; the              mirrors' views of them; Filter Glossy's blur of the mirror-smooth screen and lens past a rough bounce",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: lamp_lit,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "studio_lamps",
    covers: "R3 tier A, a studio: the product (no floor) under the environment and the studio kit's softbox, strip              and card - panels in nits, falling off to their edges (0.55, 0.45, 0.2), out of the camera's sight so              no stand-ins, yet shown in mirrors - three lamps of like power chosen by the alias table; the sky's and              a lamp's light samples summed and clamped as one",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: studio_lamps,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "lamps_in_view",
    covers: "R3 tier A, the stand-ins: the product on its modelled ground under the environment, lit by lamp_lit's              key and by two lamps the camera sees - a softbox behind the product facing the camera, falling off to              its edges and fading down its face, its stand-in glowing as the lamp lights (and mirrored in the              screen, the bezel and the ground by the lamp itself, the stand-in passed), and a disc turned away,              whose stand-in the camera sees from behind: a square, unlit",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: lamps_in_view,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "glass_ball",
    covers: "R3 tier B, glass: the product on its modelled ground and a smooth ball of solid glass in front of it              (depth 0.3, IOR 1.5) - the split at the first glass the camera sees, its reflection a path of its own              traced first on the same sampler and the camera's going on through by what Fresnel leaves; refraction              in and out, total internal reflection, the medium entered and left and the media a path starts in,              Beer-Lambert absorption inside; shadow rays from the ground and the body through the ball, tinted at              each face and absorbed between; the bounces that pass the ball as the light samples did and those that              see it as glass, met past it; the ball roughened deep in a path and sampling the whole sky",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: glass_ball,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "rough_glass_floor",
    covers: "R3 tier B, rough glass: product_floor and a block of rough (0.25) tinted glass on the floor under a              mirror-smooth coat, a tenth of it diffuse - the rough transmission lobe's evaluation, density and              sampling from both sides, compensated by the interface's table; the coat over glass trapping light              against the transmission colour; the interface and the diffuse sharing what the transmission leaves;              light sampled from both sides of a surface that transmits, the whole sky and the shadow rays leaving              from the side the light is on; the catcher's view through the reflections and not through the              refractions; no clear glass, so shadow rays stop at it",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: rough_glass_floor,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "pane_and_lamps",
    covers: "R3 tier B, thin glass and lamps: the product on its modelled ground under the environment, lamp_lit's key              and a softbox behind it facing the camera, seen through two thin-walled panes - one clear and tinted,              one frosted (0.3) - straight through the clear one (the thin split), off the frosted one's mirrored              half vector; lamps chosen from both sides of glass and lit through it, shadow rays tinted by the clear              pane at each face; the camera's view through glass meeting the softbox and the key, the split              reflection's own view of the lamps from its start, the stand-in passed past the first surface",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: pane_and_lamps,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "liquid_in_glass",
    covers: "R3 tier B, nested media: a hollow ball of glass, its outer and inner surfaces one mesh, holding a ball of              orange liquid modelled into the glass, which outranks it (priority 1) - the medium present by              priority, the glass's inner wall inside the liquid crossed as no interface, the index between liquid              and glass both ways, absorption by whichever medium is present, the camera parked at its first glass              while the reflection's path has media of its own, and a shadow ray's walk pairing one volume at a time",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: liquid_in_glass,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "glass_ball_floor",
    covers: "R3 tier B, glass on the floor: product_floor and glass_ball's ball on it - the floor's shadow drawn from              the sky through the ball, tinted at each face and absorbed between, the ball split at and refracting              the floor and its reflection, the floor's reflection a path of its own seeing the ball",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: glass_ball_floor,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "textured_product",
    covers: "R3 tier A, image textures through the mesh's UVs: the product on its modelled ground, its body wearing a \
             colour map of sRGB bytes (512 x 512, its mip levels sRGB bytes too) and a grey roughness map of bytes \
             (one channel, its mip levels sixteen bits), each repeated and the roughness turned; the ground a colour \
             map of half floats repeated forty times and the roughness map again, seen from the product's feet to \
             the horizon so the footprint picks every level and blends between them; colour maps brought from \
             linear sRGB into the rendering space, the roughness read as a number",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: textured_product,
    lens: framed,
    pictures: parity_pictures,
}, ParityScene {
    name: "projected_product",
    covers: "R3 tier A/B, projected pictures: the body tri-planar in real-world units, the bezel's roughness planar \
             from above, the screen a clamped planar label of sRGB bytes with alpha (its alpha the coat) leaving the \
             screen black outside it, the lens spherical, the buttons cylindrical about their axis, the gold button \
             a checker on a Mapping 2D's coordinates (a projection a node reads), and the ground a picture at the \
             place a Mapping node made of its UVs (sample_at, the sharpest level)",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: projected_product,
    lens: framed,
    pictures: parity_pictures,
}, ParityScene {
    name: "bumped_product",
    covers: "R3 tier A, bump and normal maps from pictures: the product on its modelled ground, its body bumped by a \
             height map of bytes through a Bump node (the bump's program, a picture lookup in it, run twice more a \
             tenth of a footprint along the surface), the ground by a tangent-space normal map through its UVs \
             (decoded in the frame the UVs run in), the bezel by the normal map laid down tri-planar in real-world \
             units and the buttons by it planar from above (each contribution decoded in its projection's own \
             frame, read off the projection a step along each axis)",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: bumped_product,
    lens: framed,
    pictures: parity_pictures,
}, ParityScene {
    name: "glowing_pattern",
    covers: "R3 tier A, emission a program drives: the product on its modelled ground under the environment, its \
             screen glowing in a checker planar from above (the glow's colour), its lens orange by the colour \
             picture through its UVs (the glow's luminance) - glows no light sample takes, found by the camera \
             and by bounces alone, weighed whole, lighting the bezel, the body and the ground",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: glowing_pattern,
    lens: framed,
    pictures: parity_pictures,
}, ParityScene {
    name: "glowing_screen",
    covers: "R3 tier A, glowing parts as lights: the product on its modelled ground under the environment turned \
             down to a sixth, its screen glowing blue, its lens and its feet orange, the same all over - every \
             triangle of them in the tree of glowing triangles, a light sample choosing one down it from each \
             point the path scatters at and a point on it, weighed against the bounces that hit the same point; \
             the screen lighting the bezel and the body, the feet the ground under the product",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: glowing_screen,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "glow_under_glass",
    covers: "R3 tier A/B, a glow seen through glass: glowing_screen with a block of solid clear glass lying over the \
             screen - the camera seeing the glow split at and refracted through the glass, the light samples of the \
             glowing triangles passing the glass, the glossy bezel's bounces seeing the glass as glass and the glow \
             past it weighed against the glowing surface their straight line meets",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: glow_under_glass,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "cutout_grille",
    covers: "R3 tier A, cutout opacity: the product on the Headphones' floor under the environment, a perforated \
             grille lying over its screen - its holes a checker a program draws into its opacity, a hole or a \
             wall - and a sheet of smoked plastic standing on the floor in front, its opacity 0.45 everywhere: \
             the camera and the bounces crossing the holes by a draw of the open stream, the camera's coverage \
             averaged along its ray (the program run at every crossing, the floor's shadow added behind the \
             sheet), the shadow rays of the sky through both, the floor's shadow of them, its reflection \
             crossing them",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: cutout_grille,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "cut_glass",
    covers: "R3 tier A/B, glass with a cutout's holes: glass_ball_floor's ball 0.55 opaque - the path crossing its \
             holes without entering it, refracting where it meets the glass, no split at the first glass that is \
             not plain; shadow rays through it taking its holes with its tint (Pane::crossing), the floor's shadow \
             of it, the camera's coverage through it",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: cut_glass,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "glowing_volume",
    covers: "R3 tier B, a glowing volume: glass_ball's ball of solid glass, its volume glowing orange and absorbing \
             - the glow given off along every stretch a path travels inside it, less what it absorbs on the way, \
             times the square of its index, seen through the glass by the camera and lighting what is round it \
             by the bounces that pass through",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: glowing_volume,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "bevelled_product",
    covers: "R3 tier B, rounded edges (Cycles' Bevel, bevel.rs): sharp-edged parts as a CAD model brings them, rounded \
             by their materials at the surface a path meets first - a block of brushed aluminium (anisotropic, its UVs \
             drawing the tangent), a red cylinder standing on it whose probes meet the block and are refused as \
             another part's, a brass hex nut, a blue plate thinner than its radius whose far side leans away, an \
             L-shaped bracket with no normals of its own whose program bumps it over the rounded normal - each \
             sample a disc of probes cast from both sides on three draws of the open stream, before the cutout's \
             draw; under three lamps and a dim sky, on a modelled ground",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: bevelled_product,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "driven_film",
    covers: "R3 tier B, glass whose transmission a program drives: the product on its modelled ground under the \
             environment, a thin-walled, frosted film standing in front of it whose program draws a checker into \
             how much light it lets through (a tenth or seven tenths) - the stack's transmission lobe weighed at \
             each hit by the program, the film no clear glass to a shadow ray and no medium, as the scene's own \
             material has it",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: driven_film,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "background_colour",
    covers: "R3 tier A, a background colour: glass_ball_floor and a mirror-smooth chrome ball beside the glass, the              camera seeing a flat blue past the parts where its ray escapes, through the glass and in the chrome              (the specular chain), the floor standing in front of the colour with its shadow and its reflection on              it, the floor's reflection seeing the colour through the glass; every other path still lit by the              environment (past_the_scene)",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: background_colour,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "backplate",
    covers: "R3 tier A, a backplate: background_colour's parts in front of a picture made here (a gradient under a              grid, 96 by 54 texels, stretched over the frame and read bilinearly at each sample's film point), seen              past the parts, through the glass and in the chrome at the film point the camera's sample set out from,              behind the floor, and through the glass in the floor's reflection at the film's corner, as the              processor's floor reflection traces it",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: backplate,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "shallow_focus",
    covers: "R3, depth of field: product_floor through a thin lens at f/1.4, focused on the product's front - each \
             camera ray from a point on the round opening drawn on the lens dimensions (the concentric disc) \
             towards its pinhole ray's point on the focus plane, the floor's far edge and the horizon blurred, the \
             floor's reflection and shadow through the lens",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: product_floor,
    lens: shallow,
    pictures: no_pictures,
}, ParityScene {
    name: "bladed_bokeh",
    covers: "R3, depth of field through a bladed opening: lamps_in_view at f/1.2 focused well in front of the \
             product, through six blades turned a fifth of a radian - a wedge by the lens draw, a point in its \
             triangle - the product, the lamps' stand-ins and their reflections out of focus through the six-sided opening",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: lamps_in_view,
    lens: bladed,
    pictures: no_pictures,
}, ParityScene {
    name: "orthographic",
    covers: "R3, the orthographic camera: product_floor seen by parallel rays from the film plane, the frame as tall \
             as the perspective one's at the product - each ray's start on the film plane in scene units, every \
             ray the camera's forward",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: product_floor,
    lens: orthographic,
    pictures: no_pictures,
}, ParityScene {
    name: "all_round",
    covers: "R3, the 360-degree camera: glass_ball_floor seen every way round from the product's camera, the film's \
             width round the horizon and its height from the zenith down - the sky above, the floor below, the \
             product and the ball in front, the backdrop the camera sees behind",
    width: 320,
    height: 160,
    samples: 256,
    seed: 7,
    build: glass_ball_floor,
    lens: all_round,
    pictures: no_pictures,
}, ParityScene {
    name: "shifted_lens",
    covers: "R3, the shifted lens: lamp_lit through the product camera's lens with its film shifted an eighth across \
             and a sixth down - the frame moved without turning the camera, verticals kept parallel",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: lamp_lit,
    lens: shifted,
    pictures: no_pictures,
}];

/// The product's camera as framed.
fn framed(camera: Camera) -> Camera {
    camera
}

/// A shallow focus: f/1.4, focused on the near side of the product.
fn shallow(camera: Camera) -> Camera {
    Camera { f_number: 1.4, focus_distance: camera.focus_distance * 0.85, ..camera }
}

/// A bladed opening wide open, focused in front of the product.
fn bladed(camera: Camera) -> Camera {
    use tequila_render_core::camera::Aperture;
    Camera { f_number: 1.2, focus_distance: camera.focus_distance * 0.6, aperture: Aperture::Bladed { blades: 6, rotation: 0.2 }, ..camera }
}

/// Parallel rays, the frame as tall as the lens's is at the product.
fn orthographic(camera: Camera) -> Camera {
    use tequila_render_core::camera::Projection;
    let scale = 2.0 * camera.focus_distance * (camera.vertical_fov(320.0 / 180.0) * 0.5).tan();
    Camera { projection: Projection::Orthographic { scale }, ..camera }
}

/// Every way round from where the product's camera stands.
fn all_round(camera: Camera) -> Camera {
    Camera { projection: tequila_render_core::camera::Projection::Equirectangular, ..camera }
}

/// The product's lens with its film shifted across and down.
fn shifted(camera: Camera) -> Camera {
    use tequila_render_core::camera::Projection;
    let Projection::Perspective { focal_length_mm } = camera.projection else { return camera };
    Camera { projection: Projection::Shift { focal_length_mm, shift_x: 0.125, shift_y: -1.0 / 6.0 }, ..camera }
}

/// The path [`background`] takes for the set's own backplate
/// ([`backplate_picture`]) rather than a file's.
pub const BACKPLATE: &str = "parity:backplate";

/// What a snapshot of `document` shows past its parts, as the command line
/// sets it (`EnvironmentCache::background`), with [`BACKPLATE`] the set's
/// own picture: the parity tests set it on their snapshots.
pub fn background(document: &SceneDocument) -> tequila_render_core::environment::Background {
    let settings = &document.environment;
    if settings.background == crate::document::BackgroundMode::Backplate && settings.backplate.as_deref() == Some(BACKPLATE) {
        return tequila_render_core::environment::Background::Backplate { image: Arc::new(backplate_picture()) };
    }
    crate::environment::EnvironmentCache::new().background(settings)
}

/// The set's backplate: a warm-to-cool gradient under a grid of lighter
/// lines, linear values, 96 by 54 texels - coarser than the frame, so a
/// sample reads between texels.
pub fn backplate_picture() -> Image {
    let (width, height) = (96usize, 54usize);
    let mut data = Vec::with_capacity(width * height * 3);
    for y in 0..height {
        for x in 0..width {
            let u = x as f32 / (width - 1) as f32;
            let v = y as f32 / (height - 1) as f32;
            let line = if x % 12 == 0 || y % 9 == 0 { 0.25 } else { 0.0 };
            data.extend_from_slice(&[0.15 + 0.6 * u + line, 0.2 + 0.3 * v + line, 0.65 - 0.45 * u + line]);
        }
    }
    Image { width, height, channels: 3, data, color_space: ColorSpace::LinearSrgb }
}

/// The scene of the set by `name`.
pub fn scene(name: &str) -> Option<&'static ParityScene> {
    SET.iter().find(|s| s.name == name)
}

/// `simple_product` with every material grey and diffuse, under the
/// default environment: what R2.5's first transport can render.
fn grey_product() -> SceneDocument {
    let grey = Material::diffuse(Spectrum::splat(0.5));
    simple_product_mapped(&|(name, _, _)| (name, MaterialType::Diffuse, grey.clone())).0
}

/// `simple_product` in its own materials with its modelled ground taken
/// away, standing on the environment's ground instead, set as `ground` says.
/// The two floors lie in one plane, so only one may be there (see
/// `simple_product_mapped`).
fn on_the_environments_ground(ground: &[EnvironmentParameter]) -> SceneDocument {
    on_the_environments_ground_with(ground, None)
}

/// The same, and under `config` when given rather than the Product preset.
fn on_the_environments_ground_with(ground: &[EnvironmentParameter], config: Option<IntegratorConfig>) -> SceneDocument {
    let (mut document, mut bus) = simple_product_mapped(&|material| material);
    let modelled: Vec<_> = document
        .tree
        .iter()
        .filter(|n| n.material.and_then(|m| document.materials.get(m)).is_some_and(|m| m.name == "Ground"))
        .map(|n| n.id)
        .collect();
    assert_eq!(modelled.len(), 1, "simple_product has one modelled ground");
    bus.apply(&mut document, Command::DeleteNodes { nodes: modelled }).expect("the modelled ground goes");
    let on = [EnvironmentParameter::flag(EnvParam::GroundEnabled, true), EnvironmentParameter::flag(EnvParam::GroundShadows, true)];
    for parameter in on.into_iter().chain(ground.iter().cloned()) {
        bus.apply(&mut document, Command::SetEnvironmentParameter { parameter }).expect("the environment's ground is set");
    }
    if let Some(config) = config {
        bus.apply(&mut document, Command::SetLightingConfig { config: Box::new(config) }).expect("the lighting is set");
    }
    document
}

/// The Headphones' floor: black shadows, reflections at 0.35, 3 m across.
fn product_floor() -> SceneDocument {
    on_the_environments_ground(&[
        EnvironmentParameter::color(EnvParam::GroundShadowColor, Spectrum::ZERO),
        EnvironmentParameter::scalar(EnvParam::GroundReflections, 0.35),
        EnvironmentParameter::scalar(EnvParam::GroundSize, 3.0),
    ])
}

/// A glossy floor: a blurred reflection, weakened by Fresnel, fading out by
/// half the product's height, over a tinted shadow.
fn product_floor_glossy() -> SceneDocument {
    on_the_environments_ground(&[
        EnvironmentParameter::color(EnvParam::GroundShadowColor, Spectrum { r: 0.05, g: 0.03, b: 0.02 }),
        EnvironmentParameter::scalar(EnvParam::GroundReflections, 0.8),
        EnvironmentParameter::scalar(EnvParam::GroundReflectionBlur, 0.3),
        EnvironmentParameter::scalar(EnvParam::GroundReflectionLength, 0.5),
        EnvironmentParameter::flag(EnvParam::GroundReflectionFresnel, true),
        EnvironmentParameter::scalar(EnvParam::GroundSize, 3.0),
    ])
}

/// A studio floor: contact shadows by ambient occlusion, a bounce glowing
/// for the product and darker toward the contact, and a reflection fading by
/// distance.
fn product_floor_studio() -> SceneDocument {
    on_the_environments_ground(&[
        EnvironmentParameter::flag(EnvParam::GroundOcclusion, true),
        EnvironmentParameter::flag(EnvParam::GroundBounce, true),
        EnvironmentParameter::scalar(EnvParam::GroundBounceStrength, 0.8),
        EnvironmentParameter::scalar(EnvParam::GroundBounceContactFalloff, 0.15),
        EnvironmentParameter::scalar(EnvParam::GroundReflections, 0.5),
        EnvironmentParameter::scalar(EnvParam::GroundReflectionFadeDistance, 0.08),
        EnvironmentParameter::new(EnvParam::GroundReflectionFadeBy, EnvValue::Choice("distance")),
        EnvironmentParameter::scalar(EnvParam::GroundSize, 3.0),
    ])
}

/// The default ground under the Product preset with ground illumination on.
fn product_floor_lit() -> SceneDocument {
    on_the_environments_ground_with(&[], Some(IntegratorConfig { ground_illumination: true, ..IntegratorConfig::product() }))
}

/// `simple_product` under the environment and lamp_lit's key, with two lamps
/// in the camera's view: a softbox behind the product facing the camera,
/// up to its right, and a disc up to its left turned away from it.
fn lamps_in_view() -> SceneDocument {
    use crate::lighting::{Intensity, LightSettings, LightShape};
    use tequila_core::math::{vec3, Mat4, Transform, Vec3};
    let mut document = simple_product();
    let mut bus = crate::command::CommandBus::new();
    let camera = product_camera(&document, 320.0 / 180.0);
    let eye = camera.position();
    let product: Vec<_> = document
        .tree
        .iter()
        .filter(|n| matches!(n.kind, crate::tree::NodeKind::Part { .. }))
        .filter(|n| n.material.and_then(|m| document.materials.get(m)).is_none_or(|m| m.name != "Ground"))
        .map(|n| n.id)
        .collect();
    let bounds = crate::snapshot::visible_bounds_under(&document, &product);
    let (middle, radius) = (bounds.center(), (bounds.diagonal().length() * 0.5).max(1e-4));
    let toward = vec3(eye.x - middle.x, 0.0, eye.z - middle.z).normalize();
    let right = Vec3::Y.cross(toward).normalize();
    // A frame whose +Z (the way a panel shines) is `facing`.
    let placed = |at: Vec3, facing: Vec3| {
        let z = facing.normalize();
        let x = Vec3::Y.cross(z).normalize();
        let y = z.cross(x);
        Transform::from_matrix(Mat4::translation(at) * Mat4::from_columns(x, y, z)).expect("a frame")
    };
    let quarter = std::f32::consts::FRAC_PI_2;
    let softbox_at = middle - toward * (1.8 * radius) + right * (1.2 * radius) + Vec3::Y * (0.6 * radius);
    let disc_at = middle - toward * (1.5 * radius) - right * (1.3 * radius) + Vec3::Y * (0.5 * radius);
    let lamps = [
        (
            "Key",
            LightSettings {
                shape: LightShape::Area { width: 1.5, height: 1.0, disc: false, two_sided: false },
                intensity: Intensity::Lumens(400.0),
                ..LightSettings::default()
            },
            Transform::translation(vec3(0.9, 2.2, 1.1)) * Transform::rotation(Vec3::X, quarter),
        ),
        (
            "Softbox",
            LightSettings {
                shape: LightShape::Area { width: radius, height: 0.6 * radius, disc: false, two_sided: false },
                intensity: Intensity::Nits(8.0),
                edge_falloff: 0.55,
                fade: 0.3,
                ..LightSettings::default()
            },
            placed(softbox_at, eye - softbox_at),
        ),
        (
            "Turned away",
            LightSettings {
                shape: LightShape::Area { width: 0.5 * radius, height: 0.5 * radius, disc: true, two_sided: false },
                intensity: Intensity::Nits(20.0),
                ..LightSettings::default()
            },
            placed(disc_at, disc_at - eye),
        ),
    ];
    for (name, settings, transform) in lamps {
        let before: Vec<_> = document.tree.iter().filter(|n| n.kind.light().is_some()).map(|n| n.id).collect();
        bus.apply(&mut document, Command::AddLight { parent: None, name: name.into(), settings: Box::new(settings) }).expect("a lamp");
        let id = document.tree.iter().find(|n| n.kind.light().is_some() && !before.contains(&n.id)).expect("the lamp just added").id;
        bus.apply(&mut document, Command::set_transform(id, transform)).expect("the lamp placed");
    }
    document
}

/// `simple_product` with no floor at all, under the environment, lit by the
/// studio kit's softbox, strip light and reflector card, each placed for the
/// set's camera as Lighting › Add to the studio places it.
fn studio_lamps() -> SceneDocument {
    use crate::studio_kit::{add, Piece};
    let (mut document, mut bus) = simple_product_mapped(&|material| material);
    let modelled: Vec<_> = document
        .tree
        .iter()
        .filter(|n| n.material.and_then(|m| document.materials.get(m)).is_some_and(|m| m.name == "Ground"))
        .map(|n| n.id)
        .collect();
    bus.apply(&mut document, Command::DeleteNodes { nodes: modelled }).expect("the modelled ground goes");
    let eye = product_camera(&document, 320.0 / 180.0).position();
    for piece in [Piece::Softbox, Piece::Strip, Piece::Card] {
        let (commands, _) = add(&document, piece, eye);
        for command in commands {
            bus.apply(&mut document, command).expect("the studio kit's piece goes in");
        }
    }
    document
}

/// Parts added to `document` through one import, as `simple_product_mapped`
/// adds its own: each a mesh of its own in a material of its own at its
/// place, under a group of their own.
fn add_parts(document: &mut SceneDocument, bus: &mut CommandBus, parts: Vec<(&'static str, crate::geometry::Mesh, MaterialType, Material, Transform)>) {
    use crate::command::{reserve_keys, ImportPayload};
    use crate::material::MaterialDefinition;
    use crate::tree::{NodeKind, SceneNode};
    let (mesh_keys, material_keys, ids) = reserve_keys(document, parts.len(), parts.len(), parts.len() + 1);
    let mut root = SceneNode::new(ids[0], "Glass", NodeKind::Group);
    root.children = ids[1..].to_vec();
    let mut nodes = vec![root];
    let mut meshes = Vec::new();
    let mut materials = Vec::new();
    for (index, (name, mesh, material_type, material, transform)) in parts.into_iter().enumerate() {
        let mut node = SceneNode::new(ids[index + 1], name, NodeKind::Part { mesh: mesh_keys[index] });
        node.parent = Some(ids[0]);
        node.transform = transform;
        node.material = Some(material_keys[index]);
        nodes.push(node);
        meshes.push((mesh_keys[index], mesh));
        materials.push((material_keys[index], MaterialDefinition::new(name, material_type, material)));
    }
    bus.apply(document, Command::Import { payload: Box::new(ImportPayload { meshes, textures: Vec::new(), materials, nodes, roots: vec![ids[0]] }) })
        .expect("the parts go in");
}

/// A ball of glass: a sphere of radius `radius`.
fn ball(radius: f32) -> crate::geometry::Mesh {
    crate::demo::to_document_mesh(&crate::demo::uv_sphere(radius, 96, 48))
}

/// A hollow ball as one mesh: a sphere of radius `outer`, and inside it one
/// of radius `inner` turned inside out, its faces and normals to the hollow.
fn hollow_ball(outer: f32, inner: f32) -> crate::geometry::Mesh {
    let mut shell = crate::demo::uv_sphere(outer, 96, 48);
    let hollow = crate::demo::uv_sphere(inner, 96, 48);
    let base = shell.positions.len() as u32;
    shell.positions.extend_from_slice(&hollow.positions);
    shell.normals.extend(hollow.normals.iter().map(|n| -*n));
    shell.uvs.extend_from_slice(&hollow.uvs);
    for triangle in hollow.indices.chunks(3) {
        shell.indices.extend_from_slice(&[base + triangle[0], base + triangle[2], base + triangle[1]]);
    }
    crate::demo::to_document_mesh(&shell)
}

/// `simple_product` on its modelled ground, a smooth ball of solid glass on
/// the ground in front of the body.
fn glass_ball() -> SceneDocument {
    let (mut document, mut bus) = simple_product_mapped(&|material| material);
    let glass = Material {
        transmission_depth: 0.3,
        transmission_color: Spectrum { r: 0.85, g: 0.95, b: 0.92 },
        specular_ior: 1.5,
        ..MaterialType::GlassSolid.defaults()
    };
    add_parts(&mut document, &mut bus, vec![("Glass Ball", ball(0.15), MaterialType::GlassSolid, glass, Transform::translation(vec3(0.3, 0.151, 0.85)))]);
    document
}

/// The Headphones' floor and glass_ball's ball on it.
fn glass_ball_floor() -> SceneDocument {
    let mut document = product_floor();
    let mut bus = crate::command::CommandBus::new();
    let glass = Material {
        transmission_depth: 0.3,
        transmission_color: Spectrum { r: 0.85, g: 0.95, b: 0.92 },
        specular_ior: 1.5,
        ..MaterialType::GlassSolid.defaults()
    };
    add_parts(&mut document, &mut bus, vec![("Glass Ball", ball(0.15), MaterialType::GlassSolid, glass, Transform::translation(vec3(0.3, 0.151, 0.85)))]);
    document
}

/// glass_ball_floor, a mirror-smooth chrome ball beside the glass, and a
/// flat blue past the parts.
fn background_colour() -> SceneDocument {
    let mut document = glass_ball_floor();
    let mut bus = crate::command::CommandBus::new();
    let chrome = Material::metal(tequila_materials::fresnel::metals::ALUMINIUM, 0.0);
    add_parts(&mut document, &mut bus, vec![("Chrome Ball", ball(0.1), MaterialType::Metal, chrome, Transform::translation(vec3(-0.35, 0.101, 0.8)))]);
    for parameter in [
        EnvironmentParameter::new(EnvParam::Background, EnvValue::Mode(crate::document::BackgroundMode::Color)),
        EnvironmentParameter::color(EnvParam::BackgroundColor, Spectrum { r: 0.18, g: 0.32, b: 0.62 }),
    ] {
        bus.apply(&mut document, Command::SetEnvironmentParameter { parameter }).expect("the background is set");
    }
    document
}

/// background_colour's parts in front of the set's backplate.
fn backplate() -> SceneDocument {
    let mut document = background_colour();
    let mut bus = crate::command::CommandBus::new();
    for parameter in [
        EnvironmentParameter::new(EnvParam::Background, EnvValue::Mode(crate::document::BackgroundMode::Backplate)),
        EnvironmentParameter::new(EnvParam::Backplate, EnvValue::Path(Some(BACKPLATE.to_string()))),
    ] {
        bus.apply(&mut document, Command::SetEnvironmentParameter { parameter }).expect("the backplate is set");
    }
    document
}

/// The Headphones' floor, a rough block of tinted glass on it under a
/// mirror-smooth coat, a tenth of it diffuse.
fn rough_glass_floor() -> SceneDocument {
    let mut document = product_floor();
    let mut bus = crate::command::CommandBus::new();
    let block = Material {
        transmission_weight: 0.9,
        transmission_color: Spectrum { r: 0.55, g: 0.75, b: 0.95 },
        transmission_depth: 0.15,
        specular_roughness: 0.25,
        specular_ior: 1.5,
        coat_weight: 1.0,
        coat_roughness: 0.02,
        ..MaterialType::GlassSolid.defaults()
    };
    let mesh = crate::bench::rounded_box(vec3(0.2, 0.12, 0.16), 0.03, 24);
    add_parts(&mut document, &mut bus, vec![("Glass Block", mesh, MaterialType::GlassSolid, block, Transform::translation(vec3(0.45, 0.121, 0.9)))]);
    document
}

/// `simple_product` on its modelled ground under the environment and
/// lamps_in_view's key and softbox, the softbox behind the product facing
/// the camera, both placed for the product's own camera; a thin clear pane
/// of tinted glass hanging between that camera and the softbox, and a thin
/// frosted pane standing on the ground in front of the product.
fn pane_and_lamps() -> SceneDocument {
    use crate::lighting::{Intensity, LightSettings, LightShape};
    use tequila_core::math::{Mat4, Vec3};
    let (mut document, mut bus) = simple_product_mapped(&|material| material);
    let camera = product_camera(&document, 320.0 / 180.0);
    let eye = camera.position();
    let product: Vec<_> = document
        .tree
        .iter()
        .filter(|n| matches!(n.kind, crate::tree::NodeKind::Part { .. }))
        .filter(|n| n.material.and_then(|m| document.materials.get(m)).is_none_or(|m| m.name != "Ground"))
        .map(|n| n.id)
        .collect();
    let bounds = crate::snapshot::visible_bounds_under(&document, &product);
    let (middle, radius) = (bounds.center(), (bounds.diagonal().length() * 0.5).max(1e-4));
    let toward = vec3(eye.x - middle.x, 0.0, eye.z - middle.z).normalize();
    let right = Vec3::Y.cross(toward).normalize();
    // A frame whose +Z (the way a panel shines, and a pane's thickness) is
    // `facing`.
    let placed = |at: Vec3, facing: Vec3| {
        let z = facing.normalize();
        let x = Vec3::Y.cross(z).normalize();
        let y = z.cross(x);
        Transform::from_matrix(Mat4::translation(at) * Mat4::from_columns(x, y, z)).expect("a frame")
    };
    let softbox_at = middle - toward * (1.4 * radius) + right * (0.5 * radius) + Vec3::Y * (0.4 * radius);
    let lamps = [
        (
            "Key",
            LightSettings {
                shape: LightShape::Area { width: 1.5, height: 1.0, disc: false, two_sided: false },
                intensity: Intensity::Lumens(400.0),
                ..LightSettings::default()
            },
            Transform::translation(vec3(0.9, 2.2, 1.1)) * Transform::rotation(Vec3::X, std::f32::consts::FRAC_PI_2),
        ),
        (
            "Softbox",
            LightSettings {
                shape: LightShape::Area { width: radius, height: 0.6 * radius, disc: false, two_sided: false },
                intensity: Intensity::Nits(8.0),
                edge_falloff: 0.55,
                fade: 0.3,
                ..LightSettings::default()
            },
            placed(softbox_at, eye - softbox_at),
        ),
    ];
    for (name, settings, transform) in lamps {
        let before: Vec<_> = document.tree.iter().filter(|n| n.kind.light().is_some()).map(|n| n.id).collect();
        bus.apply(&mut document, Command::AddLight { parent: None, name: name.into(), settings: Box::new(settings) }).expect("a lamp");
        let id = document.tree.iter().find(|n| n.kind.light().is_some() && !before.contains(&n.id)).expect("the lamp just added").id;
        bus.apply(&mut document, Command::set_transform(id, transform)).expect("the lamp placed");
    }
    let clear = Material { transmission_color: Spectrum { r: 0.8, g: 0.9, b: 1.0 }, ..MaterialType::Glass.defaults() };
    let frosted = Material { specular_roughness: 0.3, ..MaterialType::Glass.defaults() };
    let hanging = eye + (softbox_at - eye) * 0.75;
    add_parts(
        &mut document,
        &mut bus,
        vec![
            ("Clear Pane", crate::bench::rounded_box(vec3(0.6 * radius, 0.4 * radius, 0.003), 0.002, 8), MaterialType::Glass, clear, placed(hanging, eye - hanging)),
            (
                "Frosted Pane",
                crate::bench::rounded_box(vec3(0.32, 0.22, 0.003), 0.002, 8),
                MaterialType::Glass,
                frosted,
                Transform::translation(vec3(0.55, 0.25, 1.35)) * Transform::rotation(Vec3::Y, 0.6),
            ),
        ],
    );
    document
}

/// `simple_product` on its modelled ground, a hollow ball of glass in front
/// of the body - its outer and inner surfaces one mesh - holding a ball of
/// orange liquid modelled two millimetres into the glass, which outranks
/// it there.
fn liquid_in_glass() -> SceneDocument {
    let (mut document, mut bus) = simple_product_mapped(&|material| material);
    let glass = Material {
        transmission_depth: 0.3,
        transmission_color: Spectrum { r: 0.85, g: 0.95, b: 0.92 },
        specular_ior: 1.5,
        ..MaterialType::GlassSolid.defaults()
    };
    let liquid = Material {
        transmission_color: Spectrum { r: 0.9, g: 0.5, b: 0.2 },
        transmission_depth: 0.2,
        medium_priority: 1.0,
        ..MaterialType::Liquid.defaults()
    };
    let at = Transform::translation(vec3(0.3, 0.151, 0.85));
    add_parts(
        &mut document,
        &mut bus,
        vec![
            ("Glass Shell", hollow_ball(0.15, 0.14), MaterialType::GlassSolid, glass, at),
            ("Liquid", ball(0.142), MaterialType::Liquid, liquid, at),
        ],
    );
    document
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scene_is_named_once_and_builds() {
        for (i, scene) in SET.iter().enumerate() {
            assert!(SET[..i].iter().all(|s| s.name != scene.name), "{} twice", scene.name);
            assert!(!scene.covers.is_empty());
            let document = scene.document();
            document.validate().unwrap();
            assert!(document.difference_from(&scene.document()).is_none(), "{} differs between builds", scene.name);
        }
        assert!(super::scene("grey_product").is_some());
        assert!(super::scene("product_materials").is_some());
        assert!(super::scene("product_floor").is_some());
    }

    /// The floor scenes stand on the environment's ground alone, as set.
    #[test]
    fn the_floor_scenes_stand_on_the_environments_ground() {
        for name in ["product_floor", "product_floor_glossy", "product_floor_studio", "product_floor_lit"] {
            let document = scene(name).unwrap().document();
            let ground = &document.environment.ground;
            assert!(ground.enabled && ground.shadows, "{name}");
            assert!(
                document.tree.iter().all(|n| n.material.and_then(|m| document.materials.get(m)).is_none_or(|m| m.name != "Ground")),
                "{name} keeps no modelled ground"
            );
        }
        let floor = scene("product_floor").unwrap().document();
        assert_eq!((floor.environment.ground.reflections, floor.environment.ground.size), (0.35, 3.0));
        assert!(scene("product_floor_lit").unwrap().document().lighting.config().ground_illumination);
    }

    /// The lamp scenes are lit as they say: lamp_lit by its three lamps
    /// alone, the hierarchy choosing; the studio by three panels out of the
    /// camera's sight, falling off to their edges, the alias table choosing.
    #[test]
    fn the_lamp_scenes_are_lit_as_they_say() {
        use tequila_render_core::light::{LightKind, SelectionStrategy};
        let lit = |name: &str| {
            let document = scene(name).unwrap().document();
            let mut environments = crate::environment::EnvironmentCache::new();
            let environment = environments.environment(&document.environment);
            crate::snapshot::snapshot(&document, environment, &mut crate::snapshot::MeshCache::new()).0
        };
        let lamps = lit("lamp_lit");
        assert_eq!(lamps.lights.len(), 3);
        assert_eq!(lamps.lights.strategy(), SelectionStrategy::Hierarchy);
        assert_eq!(lamps.instances.iter().filter(|i| i.light_proxy).count(), 2, "the key's and the fill's stand-ins");
        assert!(lamps.environment.intensity() <= 0.0);
        let studio = lit("studio_lamps");
        assert_eq!(studio.lights.len(), 3);
        assert_eq!(studio.lights.strategy(), SelectionStrategy::Alias);
        assert!(studio.instances.iter().all(|i| !i.light_proxy && i.shadow_catcher.is_none()));
        for light in studio.lights.lights() {
            let LightKind::Area(area) = &light.kind else { panic!("a panel") };
            assert!(!area.is_even() && !light.visibility.camera && light.visibility.specular);
        }
        assert!(studio.environment.intensity() > 0.0);
        let in_view = lit("lamps_in_view");
        assert_eq!(in_view.lights.len(), 3);
        assert_eq!(in_view.instances.iter().filter(|i| i.light_proxy).count(), 3);
        assert!(in_view.environment.intensity() > 0.0);
    }

    #[test]
    fn the_grey_product_is_grey_and_diffuse_everywhere() {
        let document = scene("grey_product").unwrap().document();
        assert!(document.materials.iter().count() >= 8);
        for (_, material) in document.materials.iter() {
            assert_eq!(material.material_type, MaterialType::Diffuse, "{}", material.name);
            assert_eq!(material.material().base_color, Spectrum::splat(0.5), "{}", material.name);
        }
    }
}

// ---- Pictures --------------------------------------------------------------

fn no_pictures() -> Vec<(TextureId, Texture)> {
    Vec::new()
}

/// Bytes from a fixed sequence: detail at every scale, the same everywhere.
fn noise(count: usize, seed: u32) -> Vec<u8> {
    let mut state = seed | 1;
    (0..count)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state >> 24) as u8
        })
        .collect()
}

fn byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// The textured scenes' pictures, by id:
/// 1. a colour map of sRGB bytes, 512 x 512: terracotta and cream squares
///    ruled by darker lines, grained by noise;
/// 2. a grey roughness map of bytes, 256 x 256: soft stripes, grained;
/// 3. a colour map of half floats, 300 x 300: planks in linear light;
/// 4. a label of sRGB bytes with alpha, 256 x 128: a ring and a bar, clear
///    around them (small enough to be kept lossless: its mip levels floats);
/// 5. a height of bytes, 256 x 256: rounded bumps and fine grain;
/// 6. a tangent-space normal map of bytes, 256 x 256, from a height of
///    ripples.
pub fn parity_pictures() -> Vec<(TextureId, Texture)> {
    let uv = Mapping::default();
    let mut pictures = Vec::new();

    let grain = noise(512 * 512, 11);
    let colour: Vec<u8> = (0..512 * 512)
        .flat_map(|i| {
            let (x, y) = (i % 512, i / 512);
            let square = ((x / 64) + (y / 64)) % 2 == 0;
            let line = x % 32 == 0 || y % 32 == 0;
            let base: [f32; 3] = if square { [0.75, 0.36, 0.24] } else { [0.9, 0.84, 0.7] };
            let g = (grain[i] as f32 / 255.0 - 0.5) * 0.2;
            let dark = if line { 0.55 } else { 1.0 };
            base.map(|c| byte((c + g) * dark))
        })
        .collect();
    pictures.push((TextureId(1), Texture::from_linear(Image::from_srgb8(512, 512, 3, &colour), MapRole::Color, uv)));

    let grain = noise(256 * 256, 12);
    let roughness: Vec<u8> = (0..256 * 256)
        .map(|i| {
            let (x, y) = ((i % 256) as f32, (i / 256) as f32);
            let stripes = 0.5 + 0.5 * ((x + 0.3 * y) * 0.15).sin();
            byte(0.08 + 0.55 * stripes + (grain[i] as f32 / 255.0 - 0.5) * 0.15)
        })
        .collect();
    pictures.push((TextureId(2), Texture::from_linear(Image::from_raw8(256, 256, 1, &roughness), MapRole::Data, uv)));

    let grain = noise(300 * 300, 13);
    let planks: Vec<f32> = (0..300 * 300)
        .flat_map(|i| {
            let (x, y) = ((i % 300) as f32, (i / 300) as f32);
            let plank = (y / 50.0).floor();
            let rings = 0.5 + 0.5 * ((x * 0.07 + plank * 1.7 + (y * 0.05).sin() * 2.0).sin());
            let seam = if (y as usize) % 50 < 2 { 0.4 } else { 1.0 };
            let g = grain[i] as f32 / 255.0 * 0.05;
            let v = (0.12 + 0.3 * rings + g) * seam;
            [v * 1.1, v * 0.75, v * 0.45]
        })
        .collect();
    let planks = Image { width: 300, height: 300, channels: 3, data: planks, color_space: ColorSpace::LinearSrgb };
    pictures.push((TextureId(3), Texture::from_linear(planks, MapRole::Color, uv)));

    let label: Vec<u8> = (0..256 * 128)
        .flat_map(|i| {
            let (x, y) = ((i % 256) as f32 + 0.5, (i / 256) as f32 + 0.5);
            let r = ((x - 70.0).powi(2) + (y - 64.0).powi(2)).sqrt();
            let ring = (1.0 - ((r - 40.0).abs() - 10.0).clamp(0.0, 1.5) / 1.5).max(0.0);
            let bar = if (130.0..240.0).contains(&x) && (50.0..78.0).contains(&y) { 1.0 } else { 0.0 };
            let alpha = ring.max(bar);
            let colour = if bar > 0.0 { [0.95, 0.85, 0.2] } else { [0.15, 0.45, 0.85] };
            [byte(colour[0]), byte(colour[1]), byte(colour[2]), byte(alpha)]
        })
        .collect();
    pictures.push((TextureId(4), Texture::from_linear(Image::from_srgb8(256, 128, 4, &label), MapRole::Color, uv)));

    let grain = noise(256 * 256, 14);
    let height: Vec<u8> = (0..256 * 256)
        .map(|i| {
            let (x, y) = ((i % 256) as f32, (i / 256) as f32);
            let bumps = ((x * 0.098).sin() * (y * 0.098).sin()).abs();
            byte(0.2 + 0.6 * bumps + (grain[i] as f32 / 255.0 - 0.5) * 0.1)
        })
        .collect();
    pictures.push((TextureId(5), Texture::from_linear(Image::from_raw8(256, 256, 1, &height), MapRole::Height, uv)));

    let ripple = |x: f32, y: f32| (x * 0.2).sin() * 0.5 + (y * 0.13 + x * 0.05).sin() * 0.5;
    let normals: Vec<u8> = (0..256 * 256)
        .flat_map(|i| {
            let (x, y) = ((i % 256) as f32, (i / 256) as f32);
            let (dx, dy) = ((ripple(x + 0.5, y) - ripple(x - 0.5, y)) * 1.5, (ripple(x, y + 0.5) - ripple(x, y - 0.5)) * 1.5);
            let l = (dx * dx + dy * dy + 1.0).sqrt();
            // Green up the image: v runs down it.
            [byte(-dx / l * 0.5 + 0.5), byte(dy / l * 0.5 + 0.5), byte(1.0 / l * 0.5 + 0.5)]
        })
        .collect();
    pictures.push((TextureId(6), Texture::from_linear(Image::from_raw8(256, 256, 3, &normals), MapRole::Normal, uv)));
    pictures
}

/// The textured scenes' pictures in `document`'s store, by the ids
/// `parity_pictures` makes them under, at paths nothing reads (the cache
/// holds them).
fn store_pictures(document: &mut SceneDocument) {
    use crate::textures::TextureSource;
    for (id, name, role) in [
        (1, "colour", MapRole::Color),
        (2, "roughness", MapRole::Data),
        (3, "planks", MapRole::Color),
        (4, "label", MapRole::Color),
        (5, "height", MapRole::Height),
        (6, "normal", MapRole::Normal),
    ] {
        document.textures.insert_with_id(TextureId(id), TextureSource::new(format!("parity/{name}.generated"), role));
    }
}

/// One picture wired into a material's graph: through `mapping` (a Mapping
/// 2D node, when given) into `param`.
struct Wire {
    texture: u32,
    mapping: Option<Mapping>,
    param: Param,
    alpha: bool,
}

fn wire(texture: u32, mapping: Option<Mapping>, param: Param) -> Wire {
    Wire { texture, mapping, param, alpha: false }
}

/// Material `name` of `document` with `wires` added to its graph.
fn texture_material(document: &mut SceneDocument, name: &str, wires: &[Wire]) {
    let key = document.materials.iter().find(|(_, m)| m.name == name).map(|(k, _)| k).expect("the material");
    let definition = document.materials.get_mut(key).expect("the material");
    let graph = &mut definition.graph;
    let output = graph.output();
    for w in wires {
        let kind = if w.alpha {
            GraphKind::ImageAlpha { texture: TextureId(w.texture), fallback: Value::Float(0.0) }
        } else {
            GraphKind::ImageTexture { texture: TextureId(w.texture), fallback: Value::Color(Spectrum::splat(0.5)) }
        };
        let image = graph.add_node(Node::new(kind));
        if let Some(mapping) = w.mapping {
            let m = graph.add_node(Node::new(GraphKind::Mapping2d { mapping }));
            graph.connect(m, image, Socket::Input(0));
        }
        graph.connect(image, output, Socket::Param(w.param));
    }
}

fn uv_repeats(u: f32, v: f32) -> Mapping {
    Mapping { scale: vec3(u, v, 1.0), ..Mapping::default() }
}

/// `simple_product` on its modelled ground, its body and its ground
/// textured through their UVs.
fn textured_product() -> SceneDocument {
    let (mut document, _) = simple_product_mapped(&|material| material);
    store_pictures(&mut document);
    let turned = Mapping { rotation: 0.1, ..uv_repeats(2.0, 2.0) };
    texture_material(&mut document, "Body", &[wire(1, Some(uv_repeats(3.0, 3.0)), Param::BaseColor), wire(2, Some(turned), Param::SpecularRoughness)]);
    texture_material(&mut document, "Ground", &[wire(3, Some(uv_repeats(40.0, 40.0)), Param::BaseColor), wire(2, Some(uv_repeats(25.0, 25.0)), Param::SpecularRoughness)]);
    document
}

/// `simple_product` on its modelled ground, its parts wearing pictures
/// projected every way a Mapping 2D projects.
fn projected_product() -> SceneDocument {
    let (mut document, _) = simple_product_mapped(&|material| material);
    store_pictures(&mut document);
    let tri_planar = Mapping { projection: Projection::TriPlanar, real_world_units: true, scale: vec3(0.35, 0.35, 1.0), blend_sharpness: 4.0, ..Mapping::default() };
    texture_material(&mut document, "Body", &[wire(1, Some(tri_planar), Param::BaseColor)]);
    let from_above = Mapping { projection: Projection::Planar, axis: Axis::Y, real_world_units: true, scale: vec3(0.3, 0.3, 1.0), ..Mapping::default() };
    texture_material(&mut document, "Bezel", &[wire(2, Some(from_above), Param::SpecularRoughness)]);
    // The screen is 1.76 across and 1.02 deep: the label 0.9 by 0.45 in
    // its middle, clear around.
    let label = Mapping {
        projection: Projection::Planar,
        axis: Axis::Y,
        scale: vec3(1.0 / 0.9, 1.0 / 0.45, 1.0),
        offset: vec2(0.5, 0.5),
        wrap_u: Wrap::Clamp,
        wrap_v: Wrap::Clamp,
        ..Mapping::default()
    };
    texture_material(&mut document, "Screen", &[wire(4, Some(label), Param::BaseColor), Wire { texture: 4, mapping: Some(label), param: Param::CoatWeight, alpha: true }]);
    let spherical = Mapping { projection: Projection::Spherical, scale: vec3(2.0, 1.0, 1.0), ..Mapping::default() };
    texture_material(&mut document, "Lens", &[wire(3, Some(spherical), Param::BaseColor)]);
    let around = Mapping { projection: Projection::Cylindrical, axis: Axis::Y, scale: vec3(1.0, 20.0, 1.0), ..Mapping::default() };
    texture_material(&mut document, "Button", &[wire(1, Some(around), Param::BaseColor)]);
    // A checker on a Mapping 2D's coordinates: the projection read by a node.
    {
        let key = document.materials.iter().find(|(_, m)| m.name == "Trim").map(|(k, _)| k).expect("the trim");
        let graph = &mut document.materials.get_mut(key).expect("the trim").graph;
        let output = graph.output();
        let planar = Mapping { projection: Projection::Planar, axis: Axis::Y, scale: vec3(8.0, 8.0, 1.0), ..Mapping::default() };
        let m = graph.add_node(Node::new(GraphKind::Mapping2d { mapping: planar }));
        let checker = graph.add_node(Node::new(GraphKind::Node(tequila_materials::nodes::ShaderNode::Checker)));
        graph.connect(m, checker, Socket::Input(0));
        graph.connect(checker, output, Socket::Param(Param::BaseColor));
    }
    // The ground: a picture at the place a Mapping node makes of its UVs.
    {
        let key = document.materials.iter().find(|(_, m)| m.name == "Ground").map(|(k, _)| k).expect("the ground");
        let graph = &mut document.materials.get_mut(key).expect("the ground").graph;
        let output = graph.output();
        let coordinates = graph.add_node(Node::new(GraphKind::Node(tequila_materials::nodes::ShaderNode::TextureCoordinate)));
        let mut scale = Node::new(GraphKind::Node(tequila_materials::nodes::ShaderNode::Mapping { kind: tequila_texture::cycles::ops::MappingKind::Point }));
        scale.literals.insert(Socket::Input(3), Value::Vector(vec3(30.0, 30.0, 1.0)));
        let scale = graph.add_node(scale);
        let image = graph.add_node(Node::new(GraphKind::ImageTexture { texture: TextureId(3), fallback: Value::Color(Spectrum::splat(0.5)) }));
        graph.connect_output(coordinates, 2, scale, Socket::Input(0));
        graph.connect(scale, image, Socket::Input(0));
        graph.connect(image, output, Socket::Param(Param::BaseColor));
    }
    document
}

/// `simple_product` on its modelled ground, its screen glowing in a checker
/// and its lens by a picture, both driven by programs.
fn glowing_pattern() -> SceneDocument {
    let (mut document, _) = simple_product_mapped(&|(name, material_type, material)| match name {
        "Screen" => (
            name,
            MaterialType::Emissive,
            Material { emission_color: Spectrum { r: 0.5, g: 0.7, b: 1.0 }, emission_luminance: 3.0, ..MaterialType::Emissive.defaults() },
        ),
        "Lens" => (
            name,
            MaterialType::Emissive,
            Material { emission_color: Spectrum { r: 1.0, g: 0.45, b: 0.15 }, emission_luminance: 2.0, ..MaterialType::Emissive.defaults() },
        ),
        _ => (name, material_type, material),
    });
    store_pictures(&mut document);
    {
        let key = document.materials.iter().find(|(_, m)| m.name == "Screen").map(|(k, _)| k).expect("the screen");
        let graph = &mut document.materials.get_mut(key).expect("the screen").graph;
        let output = graph.output();
        let planar = Mapping { projection: Projection::Planar, axis: Axis::Y, scale: vec3(1.2, 1.2, 1.0), ..Mapping::default() };
        let m = graph.add_node(Node::new(GraphKind::Mapping2d { mapping: planar }));
        let checker = graph.add_node(Node::new(GraphKind::Node(tequila_materials::nodes::ShaderNode::Checker)));
        graph.connect(m, checker, Socket::Input(0));
        graph.connect(checker, output, Socket::Param(Param::EmissionColor));
    }
    texture_material(&mut document, "Lens", &[wire(1, Some(uv_repeats(2.0, 2.0)), Param::EmissionLuminance)]);
    document
}

/// `simple_product` on its modelled ground, the environment down to a sixth,
/// its screen glowing blue, its lens and its feet orange, each the same all
/// over: glowing parts a light sample chooses.
fn glowing_screen() -> SceneDocument {
    glowing_screen_and_bus().0
}

fn glowing_screen_and_bus() -> (SceneDocument, CommandBus) {
    let (mut document, mut bus) = simple_product_mapped(&|(name, material_type, material)| match name {
        "Screen" => (
            name,
            MaterialType::Emissive,
            Material { emission_color: Spectrum { r: 0.45, g: 0.65, b: 1.0 }, emission_luminance: 4.0, ..MaterialType::Emissive.defaults() },
        ),
        "Lens" => (
            name,
            MaterialType::Emissive,
            Material { emission_color: Spectrum { r: 1.0, g: 0.6, b: 0.3 }, emission_luminance: 8.0, ..MaterialType::Emissive.defaults() },
        ),
        "Foot" => (
            name,
            MaterialType::Emissive,
            Material { emission_color: Spectrum { r: 1.0, g: 0.5, b: 0.2 }, emission_luminance: 12.0, ..MaterialType::Emissive.defaults() },
        ),
        _ => (name, material_type, material),
    });
    bus.apply(&mut document, Command::SetEnvironmentParameter { parameter: EnvironmentParameter::scalar(EnvParam::Brightness, 1.0 / 6.0) })
        .expect("the environment turns down");
    (document, bus)
}

/// glowing_screen with a block of solid clear glass over the screen.
fn glow_under_glass() -> SceneDocument {
    let (mut document, mut bus) = glowing_screen_and_bus();
    let glass = Material { specular_ior: 1.5, ..MaterialType::GlassSolid.defaults() };
    let block = crate::bench::rounded_box(vec3(0.5, 0.012, 0.3), 0.004, 12);
    add_parts(&mut document, &mut bus, vec![("Cover Glass", block, MaterialType::GlassSolid, glass, Transform::translation(vec3(0.15, 0.345, 0.08)))]);
    document
}

/// The Headphones' floor, a perforated grille over the product's screen,
/// its holes a checker a program draws into its opacity, and a sheet of
/// smoked plastic, 0.45 opaque, standing on the floor in front.
fn cutout_grille() -> SceneDocument {
    use tequila_core::math::Vec3;
    let mut document = product_floor();
    let mut bus = crate::command::CommandBus::new();
    let grille = Material::plastic(Spectrum { r: 0.7, g: 0.68, b: 0.62 }, 0.3);
    let sheet = Material { geometry_opacity: 0.45, ..Material::plastic(Spectrum { r: 0.12, g: 0.14, b: 0.18 }, 0.08) };
    add_parts(
        &mut document,
        &mut bus,
        vec![
            ("Grille", crate::bench::rounded_box(vec3(0.8, 0.004, 0.46), 0.002, 8), MaterialType::Plastic, grille, Transform::translation(vec3(0.0, 0.42, 0.0))),
            (
                "Sheet",
                crate::bench::rounded_box(vec3(0.3, 0.2, 0.004), 0.002, 8),
                MaterialType::Plastic,
                sheet,
                Transform::translation(vec3(0.75, 0.2, 1.0)) * Transform::rotation(Vec3::Y, 0.5),
            ),
        ],
    );
    let key = document.materials.iter().find(|(_, m)| m.name == "Grille").map(|(k, _)| k).expect("the grille");
    let graph = &mut document.materials.get_mut(key).expect("the grille").graph;
    let output = graph.output();
    let planar = Mapping { projection: Projection::Planar, axis: Axis::Y, scale: vec3(1.6, 1.6, 1.0), ..Mapping::default() };
    let m = graph.add_node(Node::new(GraphKind::Mapping2d { mapping: planar }));
    let checker = graph.add_node(Node::new(GraphKind::Node(tequila_materials::nodes::ShaderNode::Checker)));
    graph.connect(m, checker, Socket::Input(0));
    graph.connect_output(checker, 1, output, Socket::Param(Param::Opacity));
    document
}

/// The Headphones' floor and glass_ball's ball on it, 0.55 opaque.
fn cut_glass() -> SceneDocument {
    let mut document = product_floor();
    let mut bus = crate::command::CommandBus::new();
    let glass = Material {
        transmission_depth: 0.3,
        transmission_color: Spectrum { r: 0.85, g: 0.95, b: 0.92 },
        specular_ior: 1.5,
        geometry_opacity: 0.55,
        ..MaterialType::GlassSolid.defaults()
    };
    add_parts(&mut document, &mut bus, vec![("Cut Glass Ball", ball(0.15), MaterialType::GlassSolid, glass, Transform::translation(vec3(0.3, 0.151, 0.85)))]);
    document
}

/// `simple_product` on its modelled ground, a ball of solid glass on the
/// ground in front of the body, its volume glowing orange.
fn glowing_volume() -> SceneDocument {
    let (mut document, mut bus) = simple_product_mapped(&|material| material);
    let glass = Material {
        transmission_depth: 0.3,
        transmission_color: Spectrum { r: 0.95, g: 0.8, b: 0.6 },
        specular_ior: 1.5,
        volume_emission_color: Spectrum { r: 1.0, g: 0.55, b: 0.2 },
        volume_emission_strength: 6.0,
        ..MaterialType::GlassSolid.defaults()
    };
    add_parts(&mut document, &mut bus, vec![("Glowing Ball", ball(0.15), MaterialType::GlassSolid, glass, Transform::translation(vec3(0.3, 0.151, 0.85)))]);
    document
}

/// An L-shaped bracket as a CAD model brings it: the L `width` across and
/// `height` up, `thickness` thick, drawn out `depth` along Z; every face
/// flat, with corners of its own, and no normals.
fn bracket(width: f32, height: f32, depth: f32, thickness: f32) -> crate::geometry::Mesh {
    // The L in the XY plane, anticlockwise seen from +Z.
    let outline = [vec2(0.0, 0.0), vec2(width, 0.0), vec2(width, thickness), vec2(thickness, thickness), vec2(thickness, height), vec2(0.0, height)];
    let mut positions = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for i in 0..outline.len() {
        let (a, b) = (outline[i], outline[(i + 1) % outline.len()]);
        let base = positions.len() as u32;
        positions.extend([vec3(a.x, a.y, 0.0), vec3(b.x, b.y, 0.0), vec3(b.x, b.y, depth), vec3(a.x, a.y, depth)]);
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    // Each end as two rectangles, anticlockwise seen from outside.
    for (z, out) in [(0.0, false), (depth, true)] {
        for (lo, hi) in [(vec2(0.0, 0.0), vec2(width, thickness)), (vec2(0.0, thickness), vec2(thickness, height))] {
            let base = positions.len() as u32;
            positions.extend([vec3(lo.x, lo.y, z), vec3(hi.x, lo.y, z), vec3(hi.x, hi.y, z), vec3(lo.x, hi.y, z)]);
            if out {
                indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
            } else {
                indices.extend([base, base + 2, base + 1, base, base + 3, base + 2]);
            }
        }
    }
    crate::geometry::Mesh::new(positions, indices)
}

/// Sharp-edged parts rounded by their materials (`edge_radius`), on a
/// modelled ground under three lamps and a dim sky: a block of brushed
/// aluminium, a cylinder standing on it, a hex nut, a plate thinner than
/// its radius, and a bracket with no normals that a program bumps.
fn bevelled_product() -> SceneDocument {
    bevelled_product_rounded(true)
}

/// [`bevelled_product`], its parts' edges rounded or, `rounded` false, as
/// sharp as their meshes: what rounding them shows.
pub fn bevelled_product_rounded(rounded: bool) -> SceneDocument {
    use crate::command::{EnvParam, EnvironmentParameter};
    use crate::lighting::{Intensity, LightSettings, LightShape};
    use tequila_core::math::Vec3;
    use tequila_materials::fresnel::metals;
    let mut document = SceneDocument::new();
    let mut bus = CommandBus::new();
    let edge = |m: Material, radius: f32, samples: f32| Material { edge_radius: if rounded { radius } else { 0.0 }, edge_samples: samples, ..m };
    let aluminium = Material { specular_roughness_anisotropy: 0.5, ..Material::metal(metals::ALUMINIUM, 0.22) };
    let red = Material::plastic(Spectrum { r: 0.55, g: 0.06, b: 0.04 }, 0.3);
    let brass = Material::metal(metals::GOLD, 0.3);
    let blue = Material::plastic(Spectrum { r: 0.05, g: 0.12, b: 0.4 }, 0.45);
    let graphite = Material::plastic(Spectrum::splat(0.06), 0.35);
    let ground = Material::plastic(Spectrum::splat(0.3), 0.5);
    let sharp_box = |half: Vec3| crate::bench::rounded_box(half, 0.0, 4);
    add_parts(
        &mut document,
        &mut bus,
        vec![
            ("Block", sharp_box(vec3(0.3, 0.1, 0.2)), MaterialType::Metal, edge(aluminium, 0.02, 8.0), Transform::translation(vec3(0.0, 0.102, 0.0))),
            ("Cylinder", crate::bench::cylinder(0.08, 0.18, 48), MaterialType::Plastic, edge(red, 0.015, 4.0), Transform::translation(vec3(0.1, 0.204, -0.02))),
            ("Nut", crate::bench::cylinder(0.05, 0.04, 6), MaterialType::Metal, edge(brass, 0.008, 8.0), Transform::translation(vec3(-0.17, 0.204, 0.07))),
            ("Plate", sharp_box(vec3(0.18, 0.006, 0.12)), MaterialType::Plastic, edge(blue, 0.012, 12.0), Transform::translation(vec3(-0.05, 0.008, 0.4))),
            (
                "Bracket",
                bracket(0.22, 0.26, 0.16, 0.04),
                MaterialType::Plastic,
                edge(graphite, 0.015, 16.0),
                Transform::translation(vec3(0.4, 0.002, 0.16)) * Transform::rotation(Vec3::Y, 0.35),
            ),
            ("Ground", crate::demo::to_document_mesh(&crate::demo::ground_plane(40.0)), MaterialType::Ground, ground, Transform::IDENTITY),
        ],
    );
    // The bracket grained by a noise its program bumps it with.
    {
        let key = document.materials.iter().find(|(_, m)| m.name == "Bracket").map(|(k, _)| k).expect("the bracket");
        let graph = &mut document.materials.get_mut(key).expect("the bracket").graph;
        let output = graph.output();
        let mut noise = Node::new(GraphKind::Node(tequila_materials::nodes::ShaderNode::Noise {
            dims: 3,
            fractal: tequila_texture::cycles::noise::FractalKind::Fbm,
            normalize: true,
        }));
        noise.literals.insert(Socket::Input(2), Value::Float(60.0));
        let noise = graph.add_node(noise);
        let bump = graph.add_node(Node::new(GraphKind::Bump { depth: 0.0006 }));
        graph.connect(noise, bump, Socket::Input(0));
        graph.connect(bump, output, Socket::Param(Param::Bump));
    }
    for parameter in [EnvironmentParameter::flag(EnvParam::GroundEnabled, false), EnvironmentParameter::scalar(EnvParam::Brightness, 0.4)] {
        bus.apply(&mut document, Command::SetEnvironmentParameter { parameter }).expect("the environment is set");
    }
    // A key panel and a disc fill shining down and a spot rim from behind,
    // as lamp_lit has them, brought in to the parts.
    let quarter = std::f32::consts::FRAC_PI_2;
    let (panel_down, spot_down) = (Transform::rotation(Vec3::X, quarter), Transform::rotation(Vec3::X, -quarter));
    let lamps = [
        ("Key", LightShape::Area { width: 0.8, height: 0.6, disc: false, two_sided: false }, 110.0, vec3(0.55, 1.2, 0.75), panel_down),
        ("Fill", LightShape::Area { width: 0.5, height: 0.5, disc: true, two_sided: false }, 45.0, vec3(-0.9, 0.9, 0.35), panel_down),
        ("Rim", LightShape::Spot { inner_degrees: 20.0, outer_degrees: 35.0, falloff: 1.0 }, 60.0, vec3(0.0, 1.4, -0.9), spot_down),
    ];
    for (name, shape, lumens, at, down) in lamps {
        let before: Vec<_> = document.tree.iter().filter(|n| n.kind.light().is_some()).map(|n| n.id).collect();
        bus.apply(
            &mut document,
            Command::AddLight { parent: None, name: name.into(), settings: Box::new(LightSettings { shape, intensity: Intensity::Lumens(lumens), ..LightSettings::default() }) },
        )
        .expect("a lamp");
        let id = document.tree.iter().find(|n| n.kind.light().is_some() && !before.contains(&n.id)).expect("the lamp just added").id;
        bus.apply(&mut document, Command::set_transform(id, Transform::translation(at) * down)).expect("the lamp placed");
    }
    document
}

/// `simple_product` on its modelled ground, a frosted film standing in
/// front of it whose program draws a checker into how much light it lets
/// through.
fn driven_film() -> SceneDocument {
    let (mut document, mut bus) = simple_product_mapped(&|material| material);
    let film = Material {
        base_color: Spectrum::splat(0.05),
        specular_roughness: 0.35,
        transmission_weight: 0.4,
        transmission_color: Spectrum { r: 0.8, g: 0.85, b: 0.9 },
        geometry_thin_walled: true,
        ..Material::default()
    };
    let sheet = crate::bench::rounded_box(vec3(0.55, 0.3, 0.003), 0.002, 8);
    add_parts(&mut document, &mut bus, vec![("Film", sheet, MaterialType::Glass, film, Transform::translation(vec3(-0.2, 0.31, 0.85)))]);
    let key = document.materials.iter().find(|(_, m)| m.name == "Film").map(|(k, _)| k).expect("the film");
    let graph = &mut document.materials.get_mut(key).expect("the film").graph;
    let output = graph.output();
    let planar = Mapping { projection: Projection::Planar, axis: Axis::Z, scale: vec3(6.0, 6.0, 1.0), ..Mapping::default() };
    let m = graph.add_node(Node::new(GraphKind::Mapping2d { mapping: planar }));
    let mut checker = Node::new(GraphKind::Node(tequila_materials::nodes::ShaderNode::Checker));
    checker.literals.insert(Socket::Input(1), Value::Color(Spectrum::splat(0.1)));
    checker.literals.insert(Socket::Input(2), Value::Color(Spectrum::splat(0.7)));
    let checker = graph.add_node(checker);
    graph.connect(m, checker, Socket::Input(0));
    graph.connect(checker, output, Socket::Param(Param::TransmissionWeight));
    document
}

/// `simple_product` on its modelled ground, bumped by pictures every way:
/// a height map, a normal map through the UVs, and the normal map laid
/// down by projections.
fn bumped_product() -> SceneDocument {
    let (mut document, _) = simple_product_mapped(&|material| material);
    store_pictures(&mut document);
    {
        let key = document.materials.iter().find(|(_, m)| m.name == "Body").map(|(k, _)| k).expect("the body");
        let graph = &mut document.materials.get_mut(key).expect("the body").graph;
        let output = graph.output();
        let image = graph.add_node(Node::new(GraphKind::ImageTexture { texture: TextureId(5), fallback: Value::Float(0.0) }));
        let m = graph.add_node(Node::new(GraphKind::Mapping2d { mapping: uv_repeats(4.0, 4.0) }));
        let bump = graph.add_node(Node::new(GraphKind::Bump { depth: 0.004 }));
        graph.connect(m, image, Socket::Input(0));
        graph.connect(image, bump, Socket::Input(0));
        graph.connect(bump, output, Socket::Param(Param::Bump));
    }
    texture_material(&mut document, "Ground", &[wire(6, Some(uv_repeats(30.0, 30.0)), Param::Bump)]);
    let tri_planar = Mapping { projection: Projection::TriPlanar, real_world_units: true, scale: vec3(0.15, 0.15, 1.0), ..Mapping::default() };
    texture_material(&mut document, "Bezel", &[wire(6, Some(tri_planar), Param::Bump)]);
    let from_above = Mapping { projection: Projection::Planar, axis: Axis::Y, scale: vec3(12.0, 12.0, 1.0), ..Mapping::default() };
    texture_material(&mut document, "Button", &[wire(6, Some(from_above), Param::Bump)]);
    document
}
