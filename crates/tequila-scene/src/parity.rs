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

use crate::bench::{lamp_lit, product_camera, simple_product, simple_product_mapped, subsurface_product};
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

    /// A cache to snapshot the scene through, holding its pictures and the
    /// set's photometric profiles ([`parity_profile`]): the snapshot takes
    /// them from it as it would a file's.
    pub fn cache(&self) -> crate::snapshot::MeshCache {
        let mut cache = crate::snapshot::MeshCache::new();
        for (id, texture) in (self.pictures)() {
            cache.insert_texture(id, Arc::new(texture));
        }
        for path in PROFILES {
            if let Some(profile) = parity_profile(path) {
                cache.insert_photometry(path, Arc::new(profile));
            }
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
}, ParityScene {
    name: "soft_filter",
    covers: "R3, the pixel filter: product_materials under Blender's own filter, Blackman-Harris 1.5 px - each \
             sample's jitter placed through the inverse of the filter's distribution over one half, a table of \
             1025 steps read between them, so samples land up to a pixel and a half from their pixel's centre",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: soft_filter,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "wide_box",
    covers: "R3, the pixel filter: product_floor under a box two pixels wide - each sample's jitter spread about \
             its pixel's centre by the width, over the floor's shadow and reflection",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: wide_box,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "finite_dome",
    covers: "R3 tier A, a dome of a size: glass_ball_floor under the environment on a dome 8 m across, its centre \
             a metre up, round below as above (the floor well inside it) - the sky looked up from where each path is (the camera's \
             backdrop, the floor's, what escapes through the glass), its light samples drawn from the centre and \
             seen from the point with their jacobian, their shadow rays stopping at the dome, the bounces weighed \
             against them from where they left, and the floor's shadow drawn by the light onto it there",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: finite_dome,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "hidden_parts",
    covers: "R3 tier A, visibility flags and holdout: the Headphones' floor with a glass ball and a chrome one \
             before the product, and five parts standing clear of everything: a ball hidden from the camera \
             (seen in the chrome, gone from the floor's mirror and from the camera's coverage), one hidden from \
             reflections, a block from diffuse bounces, a ball behind the glass from paths through it, and one \
             held out - a hole with no alpha where the camera sees it, black in the floor's mirror",

    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: hidden_parts,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "shadow_links",
    covers: "R3 tier A, shadow linking: lamp_lit with the key's shadow left off the body, the rim spot's shadow \
             cast by the feet and the buttons alone, and a point lamp at the side whose shadow the bezel does not \
             cast - shadow rays walking the parts rather than asking whether any is in the way, and the key, a \
             panel a ray can meet, met past the body by the bounces that stop at it",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: shadow_links,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "shadowless_parts",
    covers: "R3 tier A, the light past a part its shadow rays pass: the studio floor (occlusion, a bounce, a \
             reflection fading by distance measured past such parts) under the sky, a key panel and a fill disc \
             that casts no shadow; a glossy ball and a chrome one casting none before the product, and a card \
             casting none between a glowing panel and the product - the lamps met past them by a seeing bounce \
             that stops at them, and the sky and the glow past them by a bounce weighed against the light \
             samples",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: shadowless_parts,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "floor_lamps",
    covers: "R3 tier A, the floor's shadows from the lamps: product_floor under the sky, a key panel, a spot, a \
             point with a radius, a sun with a disc, a spot whose shadow the feet alone cast, a point that casts \
             none and a disc that lights the product alone - each drawn for the floor's shadow by its bound onto \
             the point, kept or not against it, its shadow ray through its shadow links",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: floor_lamps,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "ies_lamps",
    covers: "R3 tier A, IES lamps: product_floor under a dim sky, lit by a downlight's profile on a point lamp \
             above, an asymmetric profile measured over a quadrant on a spot at the side (folded into the other \
             three), and one over a half on a panel facing down - light samples, the light hierarchy's choice, \
             and the floor's shadows from them",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: ies_lamps,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "direct_off",
    covers: "R3 tier A, direct lighting off: lamps_in_view with its feet glowing and the lights sampled by \
             nothing - the sky and the glows found by bounces alone and weighed whole, the lamps lighting nothing \
             and their stand-ins shown wherever the camera sees along",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: direct_off,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "metallic_paint",
    covers: "R3 tier A, flakes: lamp_lit's product in metallic paints - the body candy red under a clear coat, \
             flakes 3 mm across over half of it leaning up to half of sixty degrees; the buttons mirror-smooth \
             flakes all over, no coat, leaning anywhere to sixty degrees; the bezel a half metal under a coat and a \
             fuzz, flakes 2 mm over a third of it - the flake field (Worley cells in the part's object space, a \
             flake's presence and lean from its cell), the metal lobe in the flake's frame under the coat in place \
             of that share of the base, its share of the base's response the coat traps light against, mirror flakes \
             as delta lobes and Filter Glossy's blur of them, the stack's choice of a flake by its albedo where it faces",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: metallic_paint,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "thin_film",
    covers: "R3 tier A, thin film: the product under the environment, films over every kind of interface - a \
             rough plastic body under a film 450 nm thick, an aluminium bezel anodised under one 320 nm thick of \
             index 2, a mirror-smooth lens coated against reflection (110 nm of index 1.38), the trim mirror-smooth \
             gold under a thick film (1.2 um) at seven tenths, the buttons half metal half plastic under a film \
             at half its weight and a coat with none - the Airy sum at 600, 550 and 450 nm over a dielectric and \
             over a metal's complex index, both polarizations, blended with the bare interface by the film's weight, \
             on rough and mirror-smooth lobes",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: thin_film,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "dispersion_rgb",
    covers: "R3 tier B, dispersion in an RGB render (spectral off): lamp_lit's product with a smooth ball of \
             dispersive glass (index 1.9, Abbe number 18) and a rough one (0.2, Abbe number 30) in front of it - \
             a wavelength drawn at the first such glass a path meets (a draw of the open stream), the path \
             weighed by the colour it stands for, the index at that wavelength at every dispersive surface after, \
             entering and leaving alike; no split at dispersive glass; the firefly rejection judging a dispersed \
             path by the light it carries",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: dispersion_rgb,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "dispersion_spectral",
    covers: "R3 tier B, spectral mode: dispersion_rgb's balls rendered by wavelength, as Auto renders a scene that \
             disperses - four wavelengths a path from the sample's wavelength dimension (the hero and three spaced \
             evenly in the sampling distribution's measure), every colour read at them by the processor's \
             Jakob-Hanika table, the hero's index at a dispersive surface, the hero alone through it (and for the \
             light sampled through the rough one), the film's response at the end, the firefly rejection judging \
             a path by how luminous its wavelengths are",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: dispersion_spectral,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "spectral_floor",
    covers: "R3 tier B, spectral mode on the floor: glass_ball_floor and a small dispersive gem beside the ball, so \
             the picture is traced by wavelength - the sky at an escape and in light samples, the floor's shadow \
             and its reflection (a path of its own on the same wavelengths), the split at the plain glass ball \
             (its reflection filtered by the weight the wavelengths share), Beer-Lambert inside it, the clamps \
             judged by the colour the wavelengths register as",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: spectral_floor,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "spectral_lamps",
    covers: "R3 tier B, spectral mode on: lamps_in_view rendered by wavelength though nothing in it disperses, as              the render's Spectral On renders it - the spectral tables put on the card at the first frame that asks              for them; the lamps met along the way and their stand-ins, the sky, the lamps' and the sky's light              samples summed as one colour and read at the four wavelengths, plastics, metals and the ground",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: spectral_lamps,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "spectral_options",
    covers: "R3 tier B with tier A's options, spectral mode on: shadowless_parts rendered by wavelength - the lamps,              the sky and the glowing panel met past the balls and the card that cast no shadow, through a fill disc              that casts none, and the studio floor's nearness measured past them - with an IES downlight over the              floor drawn for its shadows, a ball the camera does not see and one held out; every colour that meets              a path there read at the four wavelengths",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: spectral_options,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "milk_in_glass",
    covers: "R3 tier B, a volume that scatters: liquid_in_glass with its liquid cloudy - a walk through the medium              it fills, each flight drawn in a channel chosen by the path's throughput times the medium's albedo and              weighed over all three, a collision turning the path by Henyey-Greenstein's phase function (forward a              little), a flight that reaches the surface weighed by what survives it, Russian roulette in a long              walk; refracting where the liquid meets the glass, the glass's own absorption between",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: milk_in_glass,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "scattering_volumes",
    covers: "R3 tier B, volumes whose surfaces bend nothing (Scattering Medium): a haze and a forward-scattering wax              under the sky, a key panel and a spot - at each collision the lights sampled directly (one draw of the              whole sky, one lamp chosen by power), each weighed by the phase function and by what of the medium              lies between (medium_transmittance, the medium's own surface passed), the sky past it weighed by the              phase function's density against the sky's own",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: scattering_volumes,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "subsurface_product",
    covers: "R3 tier B, subsurface scattering: the bench's subsurface_product, simple_product with a translucent body \
             - the stack's subsurface lobe beside the diffuse, the walk's doorway: a path that takes it starts a \
             random walk under the surface, guided back towards where it came in (Dwivedi's turns, half of them, \
             each flight weighed by every way it could have been drawn), its first flight looking ten mean free \
             paths ahead for the surface opposite, and leaves through the part's own surface, spread over the \
             hemisphere outside, sampling the sky that side of it and the lamps",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: subsurface_product,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "backlit_translucency",
    covers: "R3 tier B, subsurface scattering lit from behind: simple_product on its modelled ground under the sky, \
             a translucent slab a centimetre thick and a thin-walled translucent sheet standing in front of it, a \
             panel lamp behind them - the slab's walks guided across to its far side once the first flight has \
             seen it, the sheet's light through it in its subsurface colour (no walk), both lit from the side the \
             lamp is on",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: backlit_translucency,
    lens: framed,
    pictures: no_pictures,
}, ParityScene {
    name: "split_shell",
    covers: "the processor's #169 on the card: simple_product on its modelled ground, and in front of it a clear lime \
             tray - closed towards the camera, open towards the product - modelled as the palma's shell halves come \
             in, split by material slot into two open parts, its outside with the rim and its inside, that close \
             each other's open edges: one solid (SceneSnapshot::solid), so a path that goes in through one part and \
             out through the other leaves the volume, which scatters a little; the media a path starts inside \
             counted along the line behind the camera",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: split_shell,
    lens: framed,
    pictures: no_pictures,
}];

/// A lamp added to `document`: its name, its settings and its place.
fn add_lamp(document: &mut SceneDocument, bus: &mut CommandBus, name: &str, settings: crate::lighting::LightSettings, transform: Transform) {
    let before: Vec<_> = document.tree.iter().filter(|n| n.kind.light().is_some()).map(|n| n.id).collect();
    bus.apply(document, Command::AddLight { parent: None, name: name.into(), settings: Box::new(settings) }).expect("a lamp");
    let id = document.tree.iter().find(|n| n.kind.light().is_some() && !before.contains(&n.id)).expect("the lamp just added").id;
    bus.apply(document, Command::set_transform(id, transform)).expect("the lamp placed");
}

/// `document`'s lamp called `name`, its settings changed by `change`.
fn change_lamp(document: &mut SceneDocument, bus: &mut CommandBus, name: &str, change: &dyn Fn(&mut crate::lighting::LightSettings)) {
    let node = document.tree.iter().find(|n| n.name == name && n.kind.light().is_some()).expect("the lamp");
    let id = node.id;
    let mut settings = node.kind.light().expect("a lamp").clone();
    change(&mut settings);
    bus.apply(document, Command::SetLight { node: id, settings: Box::new(settings) }).expect("the lamp is set");
}

/// Shadow links naming the parts wearing the materials `names`: the only
/// parts that cast the lamp's shadow, or with `exclude` the parts that do
/// not.
fn shadow_linked(document: &SceneDocument, names: &[&str], exclude: bool) -> Option<crate::lighting::LightLinking> {
    let nodes = names.iter().flat_map(|name| parts_named(document, name)).collect();
    Some(crate::lighting::LightLinking { exclude, nodes, product: false })
}

/// A frame placing a lamp at `at`, its +Z (the way a panel shines) towards
/// `facing`.
fn facing_frame(at: tequila_core::math::Vec3, facing: tequila_core::math::Vec3) -> Transform {
    use tequila_core::math::{Mat4, Vec3};
    let z = facing.normalize();
    let x = Vec3::Y.cross(z).normalize();
    let y = z.cross(x);
    Transform::from_matrix(Mat4::translation(at) * Mat4::from_columns(x, y, z)).expect("a frame")
}

/// lamp_lit with shadow links: the key's shadow left off the body, the rim's
/// cast by the feet and the buttons alone, and a point lamp at the side
/// whose shadow the bezel does not cast.
fn shadow_links() -> SceneDocument {
    use crate::lighting::{Intensity, LightSettings, LightShape};
    let mut document = lamp_lit();
    let mut bus = crate::command::CommandBus::new();
    let key = shadow_linked(&document, &["Body"], true);
    change_lamp(&mut document, &mut bus, "Key", &|s| s.shadow_links = key.clone());
    let rim = shadow_linked(&document, &["Foot", "Button"], false);
    change_lamp(&mut document, &mut bus, "Rim", &|s| s.shadow_links = rim.clone());
    let side = LightSettings {
        shape: LightShape::Point { radius: 0.0 },
        intensity: Intensity::Lumens(120.0),
        shadow_links: shadow_linked(&document, &["Bezel"], true),
        ..LightSettings::default()
    };
    add_lamp(&mut document, &mut bus, "Side", side, Transform::translation(vec3(1.6, 0.9, 0.2)));
    document
}

/// The studio floor under the sky, a key panel and a fill disc that casts no
/// shadow; a glossy ball and a chrome one casting none, standing clear of
/// the floor and the product (see hidden_parts), and a card casting none
/// between a glowing panel and the product.
fn shadowless_parts() -> SceneDocument {
    use crate::lighting::{Intensity, LightSettings, LightShape};
    use crate::tree::Rays;
    let mut document = product_floor_studio();
    let mut bus = crate::command::CommandBus::new();
    let quarter = std::f32::consts::FRAC_PI_2;
    let panel_down = Transform::rotation(tequila_core::math::Vec3::X, quarter);
    add_lamp(
        &mut document,
        &mut bus,
        "Key",
        LightSettings { shape: LightShape::Area { width: 1.5, height: 1.0, disc: false, two_sided: false }, intensity: Intensity::Lumens(400.0), ..LightSettings::default() },
        Transform::translation(vec3(0.9, 2.2, 1.1)) * panel_down,
    );
    add_lamp(
        &mut document,
        &mut bus,
        "Fill",
        LightSettings {
            shape: LightShape::Area { width: 1.0, height: 1.0, disc: true, two_sided: false },
            intensity: Intensity::Lumens(130.0),
            visibility: tequila_render_core::light::Visibility { shadow: false, ..Default::default() },
            ..LightSettings::default()
        },
        Transform::translation(vec3(-1.6, 1.6, 0.6)) * panel_down,
    );
    let glow = Material { emission_color: Spectrum { r: 1.0, g: 0.7, b: 0.4 }, emission_luminance: 6.0, ..MaterialType::Emissive.defaults() };
    let card = Material::diffuse(Spectrum::splat(0.6));
    let panel_at = vec3(-0.95, 0.25, 0.55);
    add_parts(
        &mut document,
        &mut bus,
        vec![
            ("Glow Panel", crate::bench::rounded_box(vec3(0.004, 0.18, 0.3), 0.002, 8), MaterialType::Emissive, glow, Transform::translation(panel_at)),
            ("Card", crate::bench::rounded_box(vec3(0.003, 0.12, 0.2), 0.002, 8), MaterialType::Diffuse, card, Transform::translation(vec3(-0.8, 0.13, 0.55))),
        ],
    );
    let chrome = Material::metal(tequila_materials::fresnel::metals::ALUMINIUM, 0.0);
    add_parts(
        &mut document,
        &mut bus,
        vec![
            ("Glossy Ball", ball(0.1), MaterialType::Plastic, Material::plastic(Spectrum { r: 0.7, g: 0.2, b: 0.15 }, 0.15), Transform::translation(vec3(0.42, 0.102, 0.88))),
            ("Chrome Ball", ball(0.08), MaterialType::Metal, chrome, Transform::translation(vec3(-0.3, 0.082, 0.9))),
        ],
    );
    for name in ["Glossy Ball", "Chrome Ball", "Card"] {
        for id in parts_named(&document, name) {
            bus.apply(&mut document, Command::set_rays(id, Rays { shadow: false, ..Rays::default() })).expect("the part's rays are set");
        }
    }
    document
}

/// product_floor under the sky and seven lamps of every kind.
fn floor_lamps() -> SceneDocument {
    use crate::lighting::{Intensity, LightLinking, LightSettings, LightShape};
    use tequila_core::math::Vec3;
    let mut document = product_floor();
    let mut bus = crate::command::CommandBus::new();
    let quarter = std::f32::consts::FRAC_PI_2;
    let (panel_down, spot_down) = (Transform::rotation(Vec3::X, quarter), Transform::rotation(Vec3::X, -quarter));
    let feet = shadow_linked(&document, &["Foot"], false);
    let lamps = [
        ("Key", LightSettings { shape: LightShape::Area { width: 1.5, height: 1.0, disc: false, two_sided: false }, intensity: Intensity::Lumens(300.0), ..LightSettings::default() }, Transform::translation(vec3(0.9, 2.2, 1.1)) * panel_down),
        ("Spot", LightSettings { shape: LightShape::Spot { inner_degrees: 15.0, outer_degrees: 30.0, falloff: 1.0 }, intensity: Intensity::Lumens(150.0), ..LightSettings::default() }, Transform::translation(vec3(-0.6, 2.0, 1.2)) * Transform::rotation(Vec3::X, -1.2)),
        ("Bulb", LightSettings { shape: LightShape::Point { radius: 0.05 }, intensity: Intensity::Lumens(80.0), ..LightSettings::default() }, Transform::translation(vec3(-1.0, 1.0, -0.4))),
        ("Sun", LightSettings { shape: LightShape::Sun { angle_degrees: 3.0 }, intensity: Intensity::Lux(1.5), ..LightSettings::default() }, Transform::rotation(Vec3::Y, 0.7) * Transform::rotation(Vec3::X, -0.9)),
        ("Feet Spot", LightSettings { shape: LightShape::Spot { inner_degrees: 25.0, outer_degrees: 40.0, falloff: 1.0 }, intensity: Intensity::Lumens(100.0), shadow_links: feet, ..LightSettings::default() }, Transform::translation(vec3(0.3, 2.4, -1.3)) * spot_down * Transform::rotation(Vec3::X, 0.4)),
        ("Shadowless", LightSettings { shape: LightShape::Point { radius: 0.0 }, intensity: Intensity::Lumens(60.0), visibility: tequila_render_core::light::Visibility { shadow: false, ..Default::default() }, ..LightSettings::default() }, Transform::translation(vec3(1.2, 0.8, -0.6))),
        ("Product Only", LightSettings { shape: LightShape::Area { width: 0.6, height: 0.6, disc: true, two_sided: false }, intensity: Intensity::Lumens(90.0), links: Some(LightLinking { exclude: false, nodes: Vec::new(), product: true }), ..LightSettings::default() }, facing_frame(vec3(0.2, 0.9, 1.8), vec3(-0.1, -0.5, -1.0))),
    ];
    for (name, settings, transform) in lamps {
        add_lamp(&mut document, &mut bus, name, settings, transform);
    }
    document
}

/// The path the set's photometric profiles are read from, by name
/// ([`parity_profile`]): the parity cache holds them (`ParityScene::cache`).
pub const PROFILES: [&str; 3] = ["parity:downlight", "parity:quadrant", "parity:half"];

/// One of the set's photometric profiles, made here as an IES file's text
/// (no file is shipped): a downlight - vertical angles 0-90°, rotationally
/// symmetric, its intensity falling off by its angle; one measured over a
/// quadrant of horizontal angles (0-90°, which the profile folds into the
/// other three) and every vertical angle, brighter one way round; and one
/// over a half (0-180°).
pub fn parity_profile(path: &str) -> Option<tequila_render_core::ies::IesProfile> {
    let (vertical, horizontal): (Vec<f32>, Vec<f32>) = match path {
        "parity:downlight" => ((0..10).map(|i| i as f32 * 10.0).collect(), vec![0.0]),
        "parity:quadrant" => ((0..13).map(|i| i as f32 * 15.0).collect(), vec![0.0, 30.0, 60.0, 90.0]),
        "parity:half" => ((0..7).map(|i| i as f32 * 15.0).collect(), vec![0.0, 45.0, 90.0, 135.0, 180.0]),
        _ => return None,
    };
    let mut text = String::from("IESNA:LM-63-2002\n[TEST] parity\nTILT=NONE\n");
    text.push_str(&format!("1 -1 1 {} {} 1 2 0 0 0\n1 1 100\n", vertical.len(), horizontal.len()));
    for v in &vertical {
        text.push_str(&format!("{v} "));
    }
    text.push('\n');
    for h in &horizontal {
        text.push_str(&format!("{h} "));
    }
    text.push('\n');
    for (j, h) in horizontal.iter().enumerate() {
        for v in &vertical {
            let theta = v.to_radians();
            let value = match path {
                "parity:downlight" => 1000.0 * theta.cos().max(0.0).powf(1.5),
                "parity:quadrant" => 600.0 * (0.35 + 0.65 * (0.5 + 0.5 * theta.cos())) * (1.0 - 0.6 * h.to_radians().sin()) + 40.0 * j as f32,
                _ => 800.0 * (theta.cos().max(0.0) + 0.1) * (0.4 + 0.6 * (h.to_radians() * 0.5).cos().abs()),
            };
            text.push_str(&format!("{value} "));
        }
        text.push('\n');
    }
    tequila_render_core::ies::IesProfile::parse(&text).ok()
}

/// product_floor under a dim sky, lit by the set's three profiles: the
/// downlight on a point lamp above, the quadrant on a spot at the side, the
/// half on a panel facing down.
fn ies_lamps() -> SceneDocument {
    use crate::lighting::{Intensity, LightSettings, LightShape, Photometry};
    use tequila_core::math::Vec3;
    let mut document = product_floor();
    let mut bus = crate::command::CommandBus::new();
    bus.apply(&mut document, Command::SetEnvironmentParameter { parameter: EnvironmentParameter::scalar(EnvParam::Brightness, 0.25) }).expect("the environment turns down");
    let quarter = std::f32::consts::FRAC_PI_2;
    let profile = |path: &str| Some(Photometry { path: path.to_string() });
    let lamps = [
        ("Downlight", LightSettings { shape: LightShape::Point { radius: 0.0 }, photometry: profile(PROFILES[0]), intensity: Intensity::Candela(0.02), ..LightSettings::default() }, Transform::translation(vec3(0.2, 1.8, 0.4))),
        ("Quadrant", LightSettings { shape: LightShape::Spot { inner_degrees: 20.0, outer_degrees: 30.0, falloff: 1.0 }, photometry: profile(PROFILES[1]), intensity: Intensity::Candela(0.015), ..LightSettings::default() }, Transform::translation(vec3(-1.4, 1.3, 0.9)) * Transform::rotation(Vec3::Y, -0.9) * Transform::rotation(Vec3::X, -0.5)),
        ("Half", LightSettings { shape: LightShape::Area { width: 0.5, height: 0.5, disc: false, two_sided: false }, photometry: profile(PROFILES[2]), intensity: Intensity::Candela(0.01), ..LightSettings::default() }, Transform::translation(vec3(1.0, 1.6, -0.5)) * Transform::rotation(Vec3::X, quarter) * Transform::rotation(Vec3::Z, 0.4)),
    ];
    for (name, settings, transform) in lamps {
        add_lamp(&mut document, &mut bus, name, settings, transform);
    }
    document
}

/// lamps_in_view, its feet glowing, under direct lighting off.
fn direct_off() -> SceneDocument {
    let mut document = lamps_in_view();
    let mut bus = crate::command::CommandBus::new();
    let key = document.materials.iter().find(|(_, m)| m.name == "Foot").map(|(k, _)| k).expect("the feet's material");
    let mut definition = document.materials.get(key).expect("the feet's material").clone();
    *definition.graph.material_mut() = Material { emission_color: Spectrum { r: 1.0, g: 0.5, b: 0.2 }, emission_luminance: 12.0, ..MaterialType::Emissive.defaults() };
    document.materials.insert_with_key(key, definition);
    let config = IntegratorConfig { direct_lighting: false, ..document.lighting.config() };
    bus.apply(&mut document, Command::SetLightingConfig { config: Box::new(config) }).expect("the lighting is set");
    document
}

/// The parts of `document` wearing the material called `name`.
fn parts_named(document: &SceneDocument, name: &str) -> Vec<crate::tree::NodeId> {
    document
        .tree
        .iter()
        .filter(|n| matches!(n.kind, crate::tree::NodeKind::Part { .. }))
        .filter(|n| n.material.and_then(|m| document.materials.get(m)).is_some_and(|m| m.name == name))
        .map(|n| n.id)
        .collect()
}

/// glass_ball_floor, a chrome ball beside the glass, and five parts each
/// hidden from one kind of ray or held out, standing clear of the floor and
/// of every other part: where two faces touch, which one a ray meets is a
/// tie that Embree and the ray query break differently, and passing one
/// part's face then skips the other's.
fn hidden_parts() -> SceneDocument {
    use crate::tree::Rays;
    let mut document = glass_ball_floor();
    let mut bus = crate::command::CommandBus::new();
    let chrome = Material::metal(tequila_materials::fresnel::metals::ALUMINIUM, 0.0);
    let matte = |r: f32, g: f32, b: f32| Material::diffuse(Spectrum { r, g, b });
    let gloss = |r: f32, g: f32, b: f32| Material::plastic(Spectrum { r, g, b }, 0.2);
    add_parts(
        &mut document,
        &mut bus,
        vec![
            ("Chrome Ball", ball(0.1), MaterialType::Metal, chrome, Transform::translation(vec3(-0.35, 0.101, 0.8))),
            ("Unseen Ball", ball(0.06), MaterialType::Diffuse, matte(0.8, 0.15, 0.1), Transform::translation(vec3(-0.12, 0.062, 0.98))),
            ("Unreflected Ball", ball(0.07), MaterialType::Plastic, gloss(0.1, 0.5, 0.8), Transform::translation(vec3(-0.6, 0.072, 0.98))),
            ("Undiffused Block", crate::bench::rounded_box(vec3(0.06, 0.05, 0.06), 0.01, 8), MaterialType::Diffuse, matte(0.9, 0.85, 0.2), Transform::translation(vec3(0.62, 0.052, 0.72))),
            ("Behind Glass", ball(0.06), MaterialType::Plastic, gloss(0.2, 0.8, 0.25), Transform::translation(vec3(0.1, 0.062, 0.7))),
            ("Held Out", ball(0.07), MaterialType::Plastic, gloss(0.9, 0.9, 0.9), Transform::translation(vec3(0.62, 0.072, 1.08))),
        ],
    );
    for (name, rays) in [
        ("Unseen Ball", Rays { camera: false, ..Rays::default() }),
        ("Unreflected Ball", Rays { glossy: false, ..Rays::default() }),
        ("Undiffused Block", Rays { diffuse: false, ..Rays::default() }),
        ("Behind Glass", Rays { transmission: false, ..Rays::default() }),
        ("Held Out", Rays { holdout: true, ..Rays::default() }),
    ] {
        let parts = parts_named(&document, name);
        assert!(!parts.is_empty(), "the scene has a {name}");
        for id in parts {
            bus.apply(&mut document, Command::set_rays(id, rays)).expect("the part's rays are set");
        }
    }
    document
}

/// glass_ball_floor on a dome 8 m across, a metre up, round below as
/// above: the floor's 3 m well inside it.
fn finite_dome() -> SceneDocument {
    let mut document = glass_ball_floor();
    let mut bus = crate::command::CommandBus::new();
    for parameter in [
        EnvironmentParameter::scalar(EnvParam::Size, 8.0),
        EnvironmentParameter::scalar(EnvParam::Height, 1.0),
    ] {
        bus.apply(&mut document, Command::SetEnvironmentParameter { parameter }).expect("the dome is set");
    }
    document
}

/// `simple_product` under Blackman-Harris 1.5 px, Blender's own filter.
fn soft_filter() -> SceneDocument {
    use tequila_render_core::settings::{PixelFilter, PixelFilterKind};
    let mut document = simple_product();
    document.render.pixel_filter = Some(PixelFilter { kind: PixelFilterKind::BlackmanHarris, width: 1.5 });
    document
}

/// product_floor under a box two pixels wide.
fn wide_box() -> SceneDocument {
    use tequila_render_core::settings::{PixelFilter, PixelFilterKind};
    let mut document = product_floor();
    document.render.pixel_filter = Some(PixelFilter { kind: PixelFilterKind::Box, width: 2.0 });
    document
}

/// What a render of `document` runs with, as the application's renders set
/// it: the lighting's configuration and the render's pixel filter (one
/// pixel's box where unset).
pub fn config(document: &SceneDocument) -> IntegratorConfig {
    let mut config = document.lighting.config();
    config.pixel_filter = document.render.pixel_filter.unwrap_or(tequila_render_core::settings::PixelFilter::BOX);
    config.spectral = document.render.spectral;
    config
}

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

/// `document`'s material `name` changed by `change`.
fn repainted(document: &mut SceneDocument, name: &str, change: &dyn Fn(&mut Material)) {
    let key = document.materials.iter().find(|(_, m)| m.name == name).map(|(k, _)| k).expect("the material");
    change(document.materials.get_mut(key).expect("the material").graph.material_mut());
}

/// lamp_lit's product in metallic paints (`flakes.rs`): flakes of a few
/// millimetres, a pixel or so across here, so each sparkles on its own.
fn metallic_paint() -> SceneDocument {
    let mut document = lamp_lit();
    repainted(&mut document, "Body", &|m| {
        *m = Material {
            base_color: Spectrum { r: 0.3, g: 0.02, b: 0.025 },
            flake_weight: 0.5,
            flake_size: 0.003,
            flake_spread: 0.5,
            flake_roughness: 0.15,
            flake_color: Spectrum { r: 0.9, g: 0.55, b: 0.45 },
            ..MaterialType::MetallicPaint.defaults()
        }
    });
    repainted(&mut document, "Button", &|m| {
        *m = Material {
            base_color: Spectrum { r: 0.05, g: 0.1, b: 0.3 },
            coat_weight: 0.0,
            flake_weight: 1.0,
            flake_size: 0.004,
            flake_spread: 1.0,
            flake_roughness: 0.0,
            flake_color: Spectrum::splat(0.95),
            ..MaterialType::MetallicPaint.defaults()
        }
    });
    repainted(&mut document, "Bezel", &|m| {
        m.base_color = Spectrum { r: 0.2, g: 0.6, b: 0.3 };
        m.base_metalness = 0.5;
        m.coat_weight = 1.0;
        m.fuzz_weight = 0.5;
        m.fuzz_color = Spectrum { r: 0.9, g: 0.9, b: 1.0 };
        m.flake_weight = 0.3;
        m.flake_size = 0.002;
        m.flake_spread = 0.8;
        m.flake_roughness = 0.3;
        m.flake_color = Spectrum { r: 0.8, g: 0.85, b: 0.9 };
    });
    document
}

/// `simple_product` with thin films over its interfaces (`thin_film.rs`):
/// over a rough dielectric, a rough metal, a smooth dielectric, a smooth
/// metal and a half metal under a coat.
fn thin_film() -> SceneDocument {
    use tequila_materials::fresnel::metals;
    let film = |m: &mut Material, weight: f32, thickness: f32, ior: f32| {
        m.thin_film_weight = weight;
        m.thin_film_thickness = thickness;
        m.thin_film_ior = ior;
    };
    simple_product_mapped(&|(name, material_type, mut m)| {
        match name {
            "Body" => film(&mut m, 1.0, 450.0, 1.45),
            "Bezel" => film(&mut m, 1.0, 320.0, 2.0),
            "Lens" => {
                m.specular_roughness = 0.0;
                film(&mut m, 1.0, 110.0, 1.38);
            }
            "Trim" => {
                m = Material::metal(metals::GOLD, 0.0);
                film(&mut m, 0.7, 1200.0, 1.6);
            }
            "Button" => {
                m.base_metalness = 0.5;
                m.coat_weight = 1.0;
                m.coat_roughness = 0.1;
                film(&mut m, 0.5, 600.0, 1.33);
            }
            _ => {}
        }
        (name, material_type, m)
    })
    .0
}

/// lamp_lit's product with two balls of dispersive glass in front of it,
/// one smooth and one rough, rendered as `spectral` says.
fn dispersive_balls(spectral: tequila_render_core::settings::SpectralMode) -> SceneDocument {
    let mut document = lamp_lit();
    let mut bus = crate::command::CommandBus::new();
    let smooth = Material {
        transmission_depth: 0.4,
        transmission_color: Spectrum { r: 0.95, g: 0.97, b: 0.9 },
        specular_ior: 1.9,
        specular_roughness: 0.0,
        transmission_dispersion_abbe: 18.0,
        ..MaterialType::GlassSolid.defaults()
    };
    let rough = Material { specular_roughness: 0.2, specular_ior: 1.6, transmission_dispersion_abbe: 30.0, ..smooth.clone() };
    add_parts(
        &mut document,
        &mut bus,
        vec![
            ("Smooth Gem", ball(0.12), MaterialType::GlassSolid, smooth, Transform::translation(vec3(0.25, 0.121, 0.85))),
            ("Rough Gem", ball(0.08), MaterialType::GlassSolid, rough, Transform::translation(vec3(-0.25, 0.081, 0.9))),
        ],
    );
    document.render.spectral = spectral;
    document
}

/// The dispersive balls in an RGB render.
fn dispersion_rgb() -> SceneDocument {
    dispersive_balls(tequila_render_core::settings::SpectralMode::Off)
}

/// lamps_in_view rendered by wavelength, though nothing in it disperses.
fn spectral_lamps() -> SceneDocument {
    let mut document = lamps_in_view();
    document.render.spectral = tequila_render_core::settings::SpectralMode::On;
    document
}

/// shadowless_parts rendered by wavelength (Spectral On), though nothing in
/// it disperses, with an IES downlight over the floor, a ball the camera
/// does not see and one held out, standing clear of every other part.
fn spectral_options() -> SceneDocument {
    use crate::lighting::{Intensity, LightSettings, LightShape, Photometry};
    use crate::tree::Rays;
    let mut document = shadowless_parts();
    let mut bus = crate::command::CommandBus::new();
    add_lamp(
        &mut document,
        &mut bus,
        "Downlight",
        LightSettings { shape: LightShape::Point { radius: 0.0 }, photometry: Some(Photometry { path: PROFILES[0].to_string() }), intensity: Intensity::Candela(0.02), ..LightSettings::default() },
        Transform::translation(vec3(0.2, 1.8, 0.4)),
    );
    add_parts(
        &mut document,
        &mut bus,
        vec![
            ("Unseen Ball", ball(0.06), MaterialType::Diffuse, Material::diffuse(Spectrum { r: 0.2, g: 0.3, b: 0.85 }), Transform::translation(vec3(0.05, 0.062, 0.95))),
            ("Held Out", ball(0.07), MaterialType::Plastic, Material::plastic(Spectrum { r: 0.9, g: 0.85, b: 0.2 }, 0.2), Transform::translation(vec3(0.75, 0.072, 0.95))),
        ],
    );
    for (name, rays) in [("Unseen Ball", Rays { camera: false, ..Rays::default() }), ("Held Out", Rays { holdout: true, ..Rays::default() })] {
        for id in parts_named(&document, name) {
            bus.apply(&mut document, Command::set_rays(id, rays)).expect("the part's rays are set");
        }
    }
    document.render.spectral = tequila_render_core::settings::SpectralMode::On;
    document
}

/// The dispersive balls rendered by wavelength.
fn dispersion_spectral() -> SceneDocument {
    dispersive_balls(tequila_render_core::settings::SpectralMode::Auto)
}

/// glass_ball_floor and a small dispersive gem beside the ball, so the
/// picture is traced by wavelength.
fn spectral_floor() -> SceneDocument {
    let mut document = glass_ball_floor();
    let mut bus = crate::command::CommandBus::new();
    let gem = Material {
        transmission_depth: 0.2,
        transmission_color: Spectrum { r: 0.9, g: 0.95, b: 1.0 },
        specular_ior: 2.0,
        specular_roughness: 0.0,
        transmission_dispersion_abbe: 25.0,
        ..MaterialType::GlassSolid.defaults()
    };
    add_parts(&mut document, &mut bus, vec![("Gem", ball(0.05), MaterialType::GlassSolid, gem, Transform::translation(vec3(-0.1, 0.051, 0.95)))]);
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

/// liquid_in_glass with its liquid cloudy: a medium that scatters as well
/// as absorbs (`Material::volume_scattering`), nested in the glass that
/// outranks it nowhere but where they overlap.
fn milk_in_glass() -> SceneDocument {
    let (mut document, mut bus) = simple_product_mapped(&|material| material);
    let glass = Material {
        transmission_depth: 0.3,
        transmission_color: Spectrum { r: 0.85, g: 0.95, b: 0.92 },
        specular_ior: 1.5,
        ..MaterialType::GlassSolid.defaults()
    };
    let milky = Material {
        transmission_color: Spectrum { r: 0.96, g: 0.9, b: 0.75 },
        transmission_depth: 0.1,
        transmission_scatter: Spectrum { r: 2.0, g: 2.5, b: 3.0 },
        transmission_scatter_anisotropy: 0.3,
        medium_priority: 1.0,
        ..MaterialType::Liquid.defaults()
    };
    let at = Transform::translation(vec3(0.3, 0.151, 0.85));
    add_parts(
        &mut document,
        &mut bus,
        vec![
            ("Glass Shell", hollow_ball(0.15, 0.14), MaterialType::GlassSolid, glass, at),
            ("Milky Liquid", ball(0.142), MaterialType::Liquid, milky, at),
        ],
    );
    document
}

/// `simple_product` on its modelled ground under the sky and two lamps - a
/// key panel and a spot from behind - with two volumes whose surfaces bend
/// nothing (Scattering Medium's index of 1) in front of the body: a haze
/// that scatters every way, and a waxy one that scatters forward.
fn scattering_volumes() -> SceneDocument {
    use crate::lighting::{Intensity, LightSettings, LightShape};
    use tequila_core::math::Vec3;
    let (mut document, mut bus) = simple_product_mapped(&|material| material);
    let quarter = std::f32::consts::FRAC_PI_2;
    let (panel_down, spot_down) = (Transform::rotation(Vec3::X, quarter), Transform::rotation(Vec3::X, -quarter));
    let lamps = [
        ("Key", LightShape::Area { width: 0.8, height: 0.6, disc: false, two_sided: false }, 160.0, vec3(0.55, 1.2, 0.75), panel_down),
        ("Rim", LightShape::Spot { inner_degrees: 20.0, outer_degrees: 35.0, falloff: 1.0 }, 80.0, vec3(0.0, 1.4, -0.9), spot_down),
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
    let haze = Material { transmission_color: Spectrum { r: 0.9, g: 0.92, b: 0.97 }, ..MaterialType::ScatteringMedium.defaults() };
    let wax = MaterialType::TranslucentMedium.defaults();
    add_parts(
        &mut document,
        &mut bus,
        vec![
            ("Haze", ball(0.15), MaterialType::ScatteringMedium, haze, Transform::translation(vec3(0.3, 0.151, 0.85))),
            ("Wax", ball(0.1), MaterialType::TranslucentMedium, wax, Transform::translation(vec3(-0.3, 0.101, 0.95))),
        ],
    );
    document
}

/// `simple_product` on its modelled ground under the sky, a translucent
/// slab a centimetre thick and a thin-walled translucent sheet standing in
/// front of it, facing the camera, and a panel lamp behind them shining
/// through towards it.
fn backlit_translucency() -> SceneDocument {
    use crate::lighting::{Intensity, LightSettings, LightShape};
    use tequila_core::math::{Mat4, Vec3};
    let (mut document, mut bus) = simple_product_mapped(&|material| material);
    let eye = product_camera(&document, 320.0 / 180.0).position();
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
    let placed = |at: Vec3, facing: Vec3| {
        let z = facing.normalize();
        let x = Vec3::Y.cross(z).normalize();
        let y = z.cross(x);
        Transform::from_matrix(Mat4::translation(at) * Mat4::from_columns(x, y, z)).expect("a frame")
    };
    let slab_at = middle + toward * (0.7 * radius) - right * (0.3 * radius);
    let sheet_at = middle + toward * (0.7 * radius) + right * (0.35 * radius);
    let lamp_at = middle + toward * (0.45 * radius) + Vec3::Y * (0.05 * radius);
    let before: Vec<_> = document.tree.iter().filter(|n| n.kind.light().is_some()).map(|n| n.id).collect();
    let settings = LightSettings {
        shape: LightShape::Area { width: 0.9 * radius, height: 0.4 * radius, disc: false, two_sided: false },
        intensity: Intensity::Nits(30.0),
        ..LightSettings::default()
    };
    bus.apply(&mut document, Command::AddLight { parent: None, name: "Back".into(), settings: Box::new(settings) }).expect("a lamp");
    let id = document.tree.iter().find(|n| n.kind.light().is_some() && !before.contains(&n.id)).expect("the lamp just added").id;
    bus.apply(&mut document, Command::set_transform(id, placed(lamp_at, eye - lamp_at))).expect("the lamp placed");
    let slab = Material {
        subsurface_color: Spectrum { r: 0.95, g: 0.7, b: 0.45 },
        subsurface_radius: Spectrum { r: 0.03, g: 0.02, b: 0.012 },
        ..MaterialType::Translucent.defaults()
    };
    let sheet = Material {
        base_color: Spectrum { r: 0.8, g: 0.8, b: 0.75 },
        subsurface_weight: 0.7,
        subsurface_color: Spectrum { r: 0.9, g: 0.85, b: 0.6 },
        specular_roughness: 0.5,
        geometry_thin_walled: true,
        ..Material::default()
    };
    add_parts(
        &mut document,
        &mut bus,
        vec![
            ("Slab", crate::bench::rounded_box(vec3(0.4 * radius, 0.45 * radius, 0.012), 0.003, 8), MaterialType::Translucent, slab, placed(slab_at, eye - slab_at)),
            ("Sheet", crate::bench::rounded_box(vec3(0.35 * radius, 0.5 * radius, 0.002), 0.0008, 8), MaterialType::Translucent, sheet, placed(sheet_at, eye - sheet_at)),
        ],
    );
    document
}

/// A tray `half` across each way from its middle and `depth` deep, as the
/// palma P8's shell halves come in: closed towards +Z, open towards -Z,
/// walls a tenth of `half` thick, modelled as two open surfaces that close
/// each other along the inner edge of the rim - its outside with the rim,
/// then its inside. Every corner is worked out the same way in both, so the
/// edges they share meet exactly (tequila-render-cpu's split_solid test).
fn split_tray(half: f32, depth: f32) -> (crate::geometry::Mesh, crate::geometry::Mesh) {
    use tequila_core::math::Vec3;
    let (outer, inner) = (half, 0.9 * half);
    let (front, inside_front, back) = (0.5 * depth, 0.5 * depth - 0.1 * half, -0.5 * depth);
    let p = |x: f32, y: f32, z: f32| vec3(x, y, z);
    let side = |r: f32| [(-r, -r), (r, -r), (r, r), (-r, r)];
    let (o, i) = (side(outer), side(inner));
    let mut outside = vec![[p(-outer, -outer, front), p(outer, -outer, front), p(outer, outer, front), p(-outer, outer, front)]];
    let mut inside = vec![[p(-inner, -inner, inside_front), p(-inner, inner, inside_front), p(inner, inner, inside_front), p(inner, -inner, inside_front)]];
    for k in 0..4 {
        let (a, b) = (o[k], o[(k + 1) % 4]);
        let (c, d) = (i[k], i[(k + 1) % 4]);
        // Outer wall, facing out; the rim between the walls, facing back;
        // the inner wall, facing into the tray.
        outside.push([p(a.0, a.1, back), p(b.0, b.1, back), p(b.0, b.1, front), p(a.0, a.1, front)]);
        outside.push([p(a.0, a.1, back), p(c.0, c.1, back), p(d.0, d.1, back), p(b.0, b.1, back)]);
        inside.push([p(c.0, c.1, back), p(c.0, c.1, inside_front), p(d.0, d.1, inside_front), p(d.0, d.1, back)]);
    }
    let mesh = |quads: &[[Vec3; 4]]| {
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        for quad in quads {
            let first = positions.len() as u32;
            positions.extend_from_slice(quad);
            indices.extend_from_slice(&[first, first + 1, first + 2, first, first + 2, first + 3]);
        }
        crate::geometry::Mesh::new(positions, indices)
    };
    (mesh(&outside), mesh(&inside))
}

/// `simple_product` on its modelled ground under the sky, and a clear lime
/// tray standing in front of it facing the camera, split by material slot
/// into its outside and its inside ([`split_tray`]): the palma P8's shell,
/// which rendered an opaque dark green while its media were told apart by
/// part.
fn split_shell() -> SceneDocument {
    use tequila_core::math::{Mat4, Vec3};
    let (mut document, mut bus) = simple_product_mapped(&|material| material);
    let eye = product_camera(&document, 320.0 / 180.0).position();
    let (middle, toward) = (vec3(0.0, 0.0, 0.85), vec3(eye.x, 0.0, eye.z - 0.85).normalize());
    let half = 0.11;
    let x = Vec3::Y.cross(toward).normalize();
    let at = middle + vec3(-0.28, half + 0.002, 0.0);
    let placed = Transform::from_matrix(Mat4::translation(at) * Mat4::from_columns(x, Vec3::Y, toward)).expect("a frame");
    // The palma's shell material at this size: a lime all transmission,
    // its volume reaching its colour in 3 cm and faintly scattering.
    let lime = Material {
        transmission_weight: 1.0,
        transmission_color: Spectrum { r: 0.76, g: 0.96, b: 0.04 },
        transmission_depth: 0.03,
        transmission_scatter: Spectrum { r: 0.07, g: 0.09, b: 0.02 },
        specular_ior: 1.585,
        medium_priority: 1.0,
        ..MaterialType::Plastic.defaults()
    };
    let (outside, inside) = split_tray(half, 0.08);
    add_parts(
        &mut document,
        &mut bus,
        vec![
            ("Shell Outside", outside, MaterialType::Plastic, lime.clone(), placed),
            ("Shell Inside", inside, MaterialType::Plastic, lime, placed),
        ],
    );
    document
}
