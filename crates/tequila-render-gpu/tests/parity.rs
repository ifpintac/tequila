//! The card's picture is the processor's (`docs/07` R2.6).
//!
//! `grey_product` from the parity set (R0.3), at its size, samples and seed,
//! through both renderers. They draw the same samples (R2.3) and trace the
//! same paths, so they differ only where floating point sends a path a
//! different way: a ray query and Embree round a triangle's edges
//! differently, the card's division, square root and trigonometry are not
//! correctly rounded, and an FMA here and there. The mean absolute
//! difference must be under 0.002 in scene-linear values, and at most 0.1%
//! of pixels more than 0.02 apart.

use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use tequila_render_gpu::device::{self, Needs};
use tequila_render_gpu::transport::{FrameSettings, GpuRenderer};
use tequila_render_gpu::variants::KernelFeatures;
use tequila_scene::parity;
use tequila_scene::snapshot::{snapshot, MeshCache};

/// The card's renderer for a scene, one built at a time. Each build compiles
/// the transport kernel; when it has changed, that compile is cold and takes
/// gigabytes, and the suite has died with a dozen of them at once. Its
/// kernel is the scene's own variant (`tequila_render_gpu::variants`).
fn renderer(gpu: tequila_render_gpu::GpuDevice, snapshot: &tequila_render_core::scene::SceneSnapshot) -> GpuRenderer {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    GpuRenderer::new(gpu, snapshot)
}

static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The picture the scene's variant of the kernel gave, `card`, against the
/// whole kernel's: the variant leaves out only what the scene never
/// reaches, so both draw the same samples in the same order, and the two
/// should be one picture to the bit. How many pixels are apart, and by how
/// much; asserted at the end of the shot, once its parity is said.
fn same_as_the_whole_kernel(name: &str, renderer: &mut GpuRenderer, card: &Framebuffer, settings: &FrameSettings, samples: u32) -> usize {
    // The scene's variant, widened to what the frame's camera reaches.
    let variant = renderer.kernel | renderer.frame_features(settings, card.width, card.height);
    {
        let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        renderer.set_kernel(KernelFeatures::ALL);
    }
    let mut whole = Framebuffer::new(card.width, card.height);
    renderer.render_pass(&mut whole, settings, samples);
    let bits = |p: &tequila_render_core::framebuffer::Pixel| {
        [p.sum.r.to_bits(), p.sum.g.to_bits(), p.sum.b.to_bits(), p.background_sum.r.to_bits(), p.background_sum.g.to_bits(), p.background_sum.b.to_bits(), p.coverage_sum.to_bits(), p.clamped_energy.to_bits(), p.samples]
    };
    let differing = card.pixels().iter().zip(whole.pixels()).filter(|(a, b)| bits(a) != bits(b) || a.sum_squared_luminance.to_bits() != b.sum_squared_luminance.to_bits()).count();
    let (mut sum, mut worst, mut mean_variant, mut mean_whole) = (0.0f64, 0.0f32, 0.0f64, 0.0f64);
    for (a, b) in card.pixels().iter().zip(whole.pixels()) {
        let (a, b) = (a.mean(), b.mean());
        let d = (a.r - b.r).abs().max((a.g - b.g).abs()).max((a.b - b.b).abs());
        sum += d as f64;
        worst = worst.max(d);
        mean_variant += (a.r + a.g + a.b) as f64;
        mean_whole += (b.r + b.g + b.b) as f64;
    }
    let n = card.pixels().len() as f64;
    eprintln!(
        "{name}: its kernel keeps {}; {differing} pixels apart from the whole kernel's picture (mean {:.6} against {:.6}, mean |difference| {:.2e}, worst {:.2e})",
        variant.names(),
        mean_variant / (3.0 * n),
        mean_whole / (3.0 * n),
        sum / n,
        worst
    );
    differing
}

#[test]
fn the_grey_product_is_the_processors_picture() {
    same_picture("grey_product");
}

/// R3 tier A's surface: plastics, metals and a diffuse part, with mirror
/// bounces and the stack's lobe choice.
#[test]
fn the_product_in_its_materials_is_the_processors_picture() {
    same_picture("product_materials");
}

/// R3 tier A's floor: the environment's ground as a shadow catcher, its
/// shadow, its mirror reflection and the coverage it takes.
#[test]
fn the_product_on_its_floor_is_the_processors_picture() {
    same_picture("product_floor");
}

/// The floor's blurred reflection, weakened by Fresnel and fading by height,
/// over a tinted shadow.
#[test]
fn the_product_on_a_glossy_floor_is_the_processors_picture() {
    same_picture("product_floor_glossy");
}

/// A studio floor: occlusion shadows, the bounce the product sees, a
/// reflection fading by distance.
#[test]
fn the_product_on_a_studio_floor_is_the_processors_picture() {
    same_picture("product_floor_studio");
}

/// Ground illumination: the floor a bounce meets is shaded, not passed.
#[test]
fn the_product_on_a_lit_floor_is_the_processors_picture() {
    same_picture("product_floor_lit");
}

/// The lamps: a key, a fill and a rim, chosen down the light hierarchy, two
/// with stand-ins; the bounces that meet them, the mirrors that show them,
/// Filter Glossy.
#[test]
fn the_lamp_lit_product_is_the_processors_picture() {
    same_picture("lamp_lit");
}

/// A studio: the sky and three softboxes falling off to their edges, out of
/// the camera's sight, chosen by the alias table.
#[test]
fn the_product_in_a_studio_is_the_processors_picture() {
    same_picture("studio_lamps");
}

/// Image textures through the mesh's UVs: sRGB bytes, grey bytes, half
/// floats, from the product's feet to the horizon so every level shows.
#[test]
fn the_textured_product_is_the_processors_picture() {
    same_picture("textured_product");
}

/// Projected pictures: tri-planar, planar (a clamped label and its alpha),
/// spherical, cylindrical, a Mapping 2D read by a node, a picture at a
/// place the graph worked out.
#[test]
fn the_projected_product_is_the_processors_picture() {
    same_picture("projected_product");
}

/// Bumps from pictures: a height map through a Bump node, a normal map
/// through the UVs, the normal map laid down tri-planar and planar.
#[test]
fn the_bumped_product_is_the_processors_picture() {
    same_picture("bumped_product");
}

/// Lamps the camera sees: a softbox's stand-in glowing as it lights, and
/// mirrored by the lamp itself; a disc's stand-in seen from behind.
#[test]
fn lamps_in_view_are_the_processors_picture() {
    same_picture("lamps_in_view");
}

/// Glass: a smooth ball of solid glass split at, refracted through,
/// absorbing inside, passed by shadow rays and the bounces that pass it,
/// seen as glass by those that see.
#[test]
fn the_glass_ball_is_the_processors_picture() {
    same_picture("glass_ball");
}

/// Glass on the floor: the floor's shadow through it, its reflection
/// seeing it.
#[test]
fn a_glass_ball_on_the_floor_is_the_processors_picture() {
    same_picture("glass_ball_floor");
}

/// Rough glass under a coat on the floor: the rough transmission lobe from
/// both sides, light sampled through it, no clear glass for shadow rays.
#[test]
fn rough_glass_on_the_floor_is_the_processors_picture() {
    same_picture("rough_glass_floor");
}

/// Thin panes, clear and frosted, in front of the product and its lamps.
#[test]
fn panes_in_front_of_the_lamps_are_the_processors_picture() {
    same_picture("pane_and_lamps");
}

/// A liquid in a hollow glass: nested media by priority, the wall inside
/// the liquid no interface.
#[test]
fn a_liquid_in_glass_is_the_processors_picture() {
    same_picture("liquid_in_glass");
}

/// Glows a program drives: a checker's colour, a picture's luminance,
/// found by bounces alone and weighed whole.
#[test]
fn glows_a_program_drives_are_the_processors_picture() {
    same_picture("glowing_pattern");
}

/// Glowing parts as lights: the tree of glowing triangles chosen down from
/// each point, a point on the triangle, MIS against the bounces that hit.
#[test]
fn a_glowing_screen_is_the_processors_picture() {
    same_picture("glowing_screen");
}

/// A glow under a block of glass: seen through it, sampled through it, met
/// past it by the bounces that see the glass.
#[test]
fn a_glow_under_glass_is_the_processors_picture() {
    same_picture("glow_under_glass");
}

/// Cutouts: a grille whose holes a program draws, crossed by the camera and
/// the bounces, its coverage averaged, its shadow cast through by the sky
/// on the product and the floor; a sheet whose opacity is a number.
#[test]
fn a_cut_out_grille_is_the_processors_picture() {
    same_picture("cutout_grille");
}

/// Glass with a cutout's holes: crossed, refracted, cast through.
#[test]
fn cut_glass_is_the_processors_picture() {
    same_picture("cut_glass");
}

/// A glowing volume: glass whose inside gives off light along every stretch
/// a path travels in it.
#[test]
fn a_glowing_volume_is_the_processors_picture() {
    same_picture("glowing_volume");
}

/// Rounded edges: sharp parts rounded by their materials at the surface a
/// path meets first, the probes refusing other parts and the far side of a
/// thin one, a program's bump over the rounded normal.
#[test]
fn the_bevelled_product_is_the_processors_picture() {
    same_picture("bevelled_product");
}

/// A background colour: seen past the parts, through glass and in chrome,
/// behind the floor and in its reflection through the glass.
#[test]
fn a_background_colour_is_the_processors_picture() {
    same_picture("background_colour");
}

/// A backplate: read at each sample's film point past the parts, through
/// glass and in chrome, behind the floor; at the film's corner through the
/// glass in the floor's reflection.
#[test]
fn a_backplate_is_the_processors_picture() {
    same_picture("backplate");
}

/// Depth of field: a thin lens wide open, focused on the product's front,
/// the floor's far edge and the horizon blurred.
#[test]
fn a_shallow_focus_is_the_processors_picture() {
    same_picture("shallow_focus");
}

/// A bladed opening: the lamps and highlights out of focus spread into its
/// shape.
#[test]
fn bladed_bokeh_is_the_processors_picture() {
    same_picture("bladed_bokeh");
}

/// The orthographic camera: parallel rays from the film plane.
#[test]
fn an_orthographic_camera_is_the_processors_picture() {
    same_picture("orthographic");
}

/// The 360-degree camera: every way round.
#[test]
fn a_360_degree_camera_is_the_processors_picture() {
    same_picture("all_round");
}

/// The shifted lens: the frame moved without turning the camera.
#[test]
fn a_shifted_lens_is_the_processors_picture() {
    same_picture("shifted_lens");
}

/// A soft pixel filter: Blackman-Harris 1.5 px, from its table.
#[test]
fn a_soft_pixel_filter_is_the_processors_picture() {
    same_picture("soft_filter");
}

/// A box two pixels wide.
#[test]
fn a_wide_box_filter_is_the_processors_picture() {
    same_picture("wide_box");
}

/// A dome of a size: the sky from where each path is.
#[test]
fn a_dome_of_a_size_is_the_processors_picture() {
    same_picture("finite_dome");
}

/// Parts hidden from some rays, and one held out: passed by those rays and
/// by the camera's coverage, a hole where the camera sees the held-out one.
#[test]
fn hidden_parts_are_the_processors_picture() {
    same_picture("hidden_parts");
}

/// Shadow links: a lamp's shadow cast by the parts its links name, or all
/// but those; a panel met past a part its shadow rays pass.
#[test]
fn shadow_links_are_the_processors_picture() {
    same_picture("shadow_links");
}

/// Parts that cast no shadow, and a lamp that casts none: the light past
/// them met where a bounce stops at them; a floor's nearness past them.
#[test]
fn parts_that_cast_no_shadow_are_the_processors_picture() {
    same_picture("shadowless_parts");
}

/// The floor's shadows from lamps of every kind.
#[test]
fn the_floors_shadows_from_lamps_are_the_processors_picture() {
    same_picture("floor_lamps");
}

/// IES lamps: three profiles on a point, a spot and a panel.
#[test]
fn ies_lamps_are_the_processors_picture() {
    same_picture("ies_lamps");
}

/// Direct lighting off: lights found by bounces alone, weighed whole.
#[test]
fn direct_lighting_off_is_the_processors_picture() {
    same_picture("direct_off");
}

/// Metallic paints: flakes under a coat, mirror flakes with none, flakes
/// over a half metal under a fuzz.
#[test]
fn metallic_paint_is_the_processors_picture() {
    same_picture("metallic_paint");
}

/// Thin films over a rough and a smooth dielectric, a rough and a smooth
/// metal, a half metal under a coat.
#[test]
fn thin_films_are_the_processors_picture() {
    same_picture("thin_film");
}

/// Dispersive glass in an RGB render: a wavelength drawn at the first such
/// glass a path meets, the index at it from there on.
#[test]
fn dispersion_in_rgb_is_the_processors_picture() {
    same_picture("dispersion_rgb");
}

/// Dispersive glass rendered by wavelength, as Auto renders it: four
/// wavelengths a path, the hero alone through the glass.
#[test]
fn dispersion_by_wavelength_is_the_processors_picture() {
    same_picture("dispersion_spectral");
}

/// A floor, the sky, the split at plain glass, all by wavelength.
#[test]
fn a_spectral_floor_is_the_processors_picture() {
    same_picture("spectral_floor");
}

/// A render by wavelength of a scene that disperses nothing: the spectral
/// tables put on the card at the first frame that asks for them.
#[test]
fn spectral_lamps_are_the_processors_picture() {
    same_picture("spectral_lamps");
}

/// Spectral On with the options: the light past parts that cast no shadow,
/// the floor's nearness past them, an IES downlight over the floor, a part
/// the camera does not see and one held out, all read at the wavelengths.
#[test]
fn the_options_by_wavelength_are_the_processors_picture() {
    same_picture("spectral_options");
}

/// The library's presets of glass that disperses light, every one of them,
/// on the Headphones' parts, rendered by wavelength as Auto renders them.
#[test]
fn a_wall_of_dispersive_presets_is_the_processors_picture() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/samples/Headphones.tequila");
    let mut document = tequila_format::load(&path).expect("the Headphones sample loads").document;
    let (entries, _) = tequila_format::library::read_directory(&tequila_format::library::standard_directory());
    let mut pictures = tequila_render_gpu::textures::CardPictures::default();
    pictures.finish();
    let dispersive: Vec<_> = entries
        .into_iter()
        .filter(|entry| {
            let Ok(program) = entry.graph.compile() else { return false };
            let m = &program.constants;
            m.transmission_weight > 0.0
                && m.transmission_dispersion_abbe > 0.0
                && tequila_render_gpu::program::lower_with(&program, tequila_render_gpu::textures::Pictures::On(&pictures)).is_ok()
                && tequila_render_gpu::material::GpuMaterial::supports(m)
                && m.fuzz_density <= 0.0
        })
        .collect();
    let keys: Vec<_> = document.materials.keys().collect();
    assert!(!dispersive.is_empty() && dispersive.len() <= keys.len(), "{} dispersive presets for {} parts", dispersive.len(), keys.len());
    let mut names = Vec::new();
    for (j, key) in keys.iter().enumerate() {
        let entry = &dispersive[j % dispersive.len()];
        names.push(entry.name.clone());
        document.materials.insert_with_key(*key, entry.definition());
    }
    eprintln!("dispersive wall ({} presets): {}", dispersive.len(), names.join(", "));
    let camera = document.cameras.active().camera;
    same_shot(Shot { name: "dispersive_wall".into(), document, camera, width: 320, height: 180, samples: 256, seed: 7, cache: MeshCache::new() });
}

/// The library's presets with metallic flakes (`flakes.rs`) on the
/// Headphones' parts, every one of them, as the presets have them - their
/// programs' bumps and rounded edges kept: the flake field in each part's
/// object space, at flakes far smaller than a pixel here, and the flake's
/// lobe under the coat.
#[test]
fn a_wall_of_flake_presets_is_the_processors_picture() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/samples/Headphones.tequila");
    let mut document = tequila_format::load(&path).expect("the Headphones sample loads").document;
    let (entries, _) = tequila_format::library::read_directory(&tequila_format::library::standard_directory());
    let mut pictures = tequila_render_gpu::textures::CardPictures::default();
    pictures.finish();
    let flaked: Vec<_> = entries
        .into_iter()
        .filter(|entry| {
            let Ok(program) = entry.graph.compile() else { return false };
            let m = &program.constants;
            m.flake_weight > 1e-4
                && tequila_render_gpu::program::lower_with(&program, tequila_render_gpu::textures::Pictures::On(&pictures)).is_ok()
                && tequila_render_gpu::material::GpuMaterial::supports(m)
                && m.fuzz_density <= 0.0
        })
        .collect();
    let keys: Vec<_> = document.materials.keys().collect();
    assert!(flaked.len() <= keys.len(), "{} flake presets, more than the {} parts one wall wears", flaked.len(), keys.len());
    let mut names = Vec::new();
    for (j, key) in keys.iter().enumerate() {
        let entry = &flaked[j % flaked.len()];
        names.push(entry.name.clone());
        document.materials.insert_with_key(*key, entry.definition());
    }
    eprintln!("flake wall ({} presets): {}", flaked.len(), names.join(", "));
    let camera = document.cameras.active().camera;
    same_shot(Shot { name: "flake_wall".into(), document, camera, width: 320, height: 180, samples: 256, seed: 7, cache: MeshCache::new() });
}

/// Glass whose transmission a program drives: a frosted film in front of
/// the product, a checker drawing how much light it lets through.
#[test]
fn a_film_a_program_drives_is_the_processors_picture() {
    same_picture("driven_film");
}

/// The library's presets whose programs drive their transmission, on the
/// Headphones' parts in turn, the other parts as the sample has them. Each
/// is a film with a share of subsurface scattering, which the card did not
/// do when this wall was built: it is taken off, as the first walls took off
/// rounded edges before the card had them (the subsurface walls wear them
/// whole). So is each one's bump, a grid
/// of filaments far finer than a pixel here, which puts 0.13% of the
/// pixels more than 0.02 apart with or without the transmission a program
/// drives (0 without the bump): the forward differences of a pattern that
/// fine move as far for an ulp of the point on the processor alone.
#[test]
fn a_wall_of_driven_transmission_is_the_processors_picture() {
    use tequila_materials::material_type::Param;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/samples/Headphones.tequila");
    let mut document = tequila_format::load(&path).expect("the Headphones sample loads").document;
    let (entries, _) = tequila_format::library::read_directory(&tequila_format::library::standard_directory());
    let driven: Vec<_> = entries
        .into_iter()
        .filter(|entry| entry.graph.compile().is_ok_and(|p| p.driven.iter().any(|(param, _)| *param == Param::TransmissionWeight)))
        .collect();
    assert!(!driven.is_empty(), "no preset drives its transmission");
    let keys: Vec<_> = document.materials.keys().collect();
    let mut names = Vec::new();
    // Every other part, so each preset is seen beside the sample's own.
    for (j, key) in keys.iter().enumerate().filter(|(j, _)| j % 2 == 0) {
        let entry = &driven[(j / 2) % driven.len()];
        names.push(entry.name.clone());
        let mut definition = entry.definition();
        definition.graph.material_mut().subsurface_weight = 0.0;
        let output = definition.graph.output();
        definition.graph.disconnect(output, tequila_materials::graph::Socket::Param(Param::Bump));
        document.materials.insert_with_key(*key, definition);
    }
    eprintln!("driven transmission wall ({} presets): {}", driven.len(), names.join(", "));
    let camera = document.cameras.active().camera;
    same_shot(Shot { name: "driven_transmission_wall".into(), document, camera, width: 320, height: 180, samples: 256, seed: 7, cache: MeshCache::new() });
}

/// A volume that scatters: a cloudy liquid in a glass, walked through,
/// refracting where it meets the glass.
#[test]
fn milk_in_glass_is_the_processors_picture() {
    same_picture("milk_in_glass");
}

/// Volumes whose surfaces bend nothing: a haze and a wax, walked through,
/// the lights sampled directly at each collision.
#[test]
fn scattering_volumes_are_the_processors_picture() {
    same_picture("scattering_volumes");
}

/// The library's presets whose volumes scatter (milk, orange juice,
/// coffee, translucent prints), on every other part of the Headphones, the
/// other parts as the sample has them.
#[test]
fn a_wall_of_scattering_volumes_is_the_processors_picture() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/samples/Headphones.tequila");
    let mut document = tequila_format::load(&path).expect("the Headphones sample loads").document;
    let (entries, _) = tequila_format::library::read_directory(&tequila_format::library::standard_directory());
    let scattering: Vec<_> = entries
        .into_iter()
        .filter(|entry| entry.graph.compile().is_ok_and(|p| p.constants.transmission_weight > 0.0 && p.constants.volume_scattering().is_some()))
        .collect();
    assert!(!scattering.is_empty(), "no preset's volume scatters");
    let keys: Vec<_> = document.materials.keys().collect();
    let mut names = Vec::new();
    for (j, key) in keys.iter().enumerate().filter(|(j, _)| j % 2 == 0) {
        let entry = &scattering[(j / 2) % scattering.len()];
        names.push(entry.name.clone());
        document.materials.insert_with_key(*key, entry.definition());
    }
    eprintln!("scattering wall ({} presets): {}", scattering.len(), names.join(", "));
    let camera = document.cameras.active().camera;
    same_shot(Shot { name: "scattering_wall".into(), document, camera, width: 320, height: 180, samples: 256, seed: 7, cache: MeshCache::new() });
}

/// Subsurface scattering: a translucent body walked under, guided back out.
#[test]
fn the_subsurface_product_is_the_processors_picture() {
    same_picture("subsurface_product");
}

/// Subsurface scattering lit from behind: a slab's walks guided across, a
/// thin sheet's light through it.
#[test]
fn backlit_translucency_is_the_processors_picture() {
    same_picture("backlit_translucency");
}

/// A clear shell split by material into two open parts that close each
/// other (the processor's #169): one solid, its volume left where a path
/// leaves through either part.
#[test]
fn a_shell_split_by_material_is_the_processors_picture() {
    same_picture("split_shell");
}

/// The presets of the standard library with subsurface scattering that the
/// card renders whole (their programs lowered with their pictures none
/// loaded, as no preset of these has one; their surfaces on the card's
/// stack; no fibres, no flakes, no dispersion; their colour their own, not
/// a program's), the Headphones' parts and
/// camera as the other walls have them: wall `variant` of
/// [`SUBSURFACE_WALLS`], which between them wear every one.
fn subsurface_wall(variant: usize) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/samples/Headphones.tequila");
    let mut document = tequila_format::load(&path).expect("the Headphones sample loads").document;
    let (entries, _) = tequila_format::library::read_directory(&tequila_format::library::standard_directory());
    let mut pictures = tequila_render_gpu::textures::CardPictures::default();
    pictures.finish();
    let runnable: Vec<_> = entries
        .into_iter()
        .filter(|entry| {
            let Ok(program) = entry.graph.compile() else { return false };
            let m = &program.constants;
            m.subsurface_weight > 0.0
                && tequila_render_gpu::program::lower_with(&program, tequila_render_gpu::textures::Pictures::On(&pictures)).is_ok()
                && tequila_render_gpu::material::GpuMaterial::supports(m)
                && m.fuzz_density <= 0.0
                && m.flake_weight <= 1e-4
                // Its colour its constants' (the card refuses a program
                // that drives it: the walk follows the colour to the bit,
                // and the interpreter's is not, docs/07).
                && !program.driven.iter().any(|(param, _)| *param == tequila_materials::material_type::Param::SubsurfaceColor)
        })
        .collect();
    assert!(!runnable.is_empty(), "no preset scatters under its surface");
    let keys: Vec<_> = document.materials.keys().collect();
    assert!(SUBSURFACE_WALLS * keys.len() >= runnable.len(), "{} presets, more than {SUBSURFACE_WALLS} walls of {} wear", runnable.len(), keys.len());
    let mut names = Vec::new();
    for (j, key) in keys.iter().enumerate() {
        let entry = &runnable[(SUBSURFACE_WALLS * j + variant) % runnable.len()];
        names.push(entry.name.clone());
        document.materials.insert_with_key(*key, entry.definition());
    }
    eprintln!("subsurface wall {variant} (of {} presets): {}", runnable.len(), names.join(", "));
    let camera = document.cameras.active().camera;
    same_shot(Shot { name: format!("subsurface_wall_{variant}"), document, camera, width: 320, height: 180, samples: 256, seed: 7, cache: MeshCache::new() });
}

/// How many walls the presets with subsurface scattering are spread over.
const SUBSURFACE_WALLS: usize = 3;

#[test]
fn a_subsurface_wall_is_the_processors_picture() {
    subsurface_wall(0);
}

#[test]
fn another_subsurface_wall_is_the_processors_picture() {
    subsurface_wall(1);
}

#[test]
fn a_third_subsurface_wall_is_the_processors_picture() {
    subsurface_wall(2);
}

/// A picture to render on both: a scene, the camera it is seen from, and
/// how it is rendered.
struct Shot {
    name: String,
    document: tequila_scene::SceneDocument,
    camera: tequila_render_core::camera::Camera,
    width: usize,
    height: usize,
    samples: u32,
    seed: u32,
    /// What the snapshot reads its pictures from.
    cache: MeshCache,
}

fn same_picture(name: &str) {
    let scene = parity::scene(name).expect("the parity set has it");
    let document = scene.document();
    let camera = scene.camera(&document);
    same_shot(Shot { name: scene.name.to_string(), document, camera, width: scene.width, height: scene.height, samples: scene.samples, seed: scene.seed, cache: scene.cache() });
}

fn same_shot(mut scene: Shot) {
    let Some(gpu) = device::for_test(Needs::RayQueries) else { return };
    let name = scene.name.as_str();
    let document = &scene.document;
    let mut environments = tequila_scene::environment::EnvironmentCache::new();
    let environment = environments.environment(&document.environment);
    let (mut snapshot, _) = snapshot(&document, environment, &mut scene.cache);
    // What the camera sees past the parts, as the command line sets it.
    snapshot.background = parity::background(document);
    let camera = scene.camera;
    // As the application renders it: the render's pixel filter too.
    let mut config = parity::config(document);
    // TEQUILA_PARITY_BOUNCES="bounces,through glass": both renderers' limits
    // set so, to find the depth at which paths first part (a test hook; the
    // set's own limits are the bar's).
    if let Ok(limits) = std::env::var("TEQUILA_PARITY_BOUNCES") {
        let limits: Vec<u32> = limits.split(',').map(|v| v.trim().parse().expect("TEQUILA_PARITY_BOUNCES is two numbers")).collect();
        config.max_bounces = limits[0];
        config.max_transmission_bounces = limits[1];
        eprintln!("{name}: {} bounces, {} through glass (TEQUILA_PARITY_BOUNCES)", limits[0], limits[1]);
    }

    // Nothing in the set is a case the card renders otherwise.
    let mut limits = tequila_render_gpu::transport::floor_limits(&snapshot, &config);
    limits.extend(tequila_render_gpu::lights::lamp_limits(&snapshot, &config));
    limits.extend(tequila_render_gpu::transport::glass_limits(&snapshot, &config));
    limits.extend(tequila_render_gpu::textures::texture_limits(&snapshot, &config));
    limits.extend(tequila_render_gpu::transport::part_limits(&snapshot, &config));
    limits.extend(tequila_render_gpu::transport::camera_limits(&camera, &config));
    assert!(limits.is_empty(), "{name}: the card renders these otherwise: {limits:?}");
    // And every material's program is one the card runs.
    let refused: Vec<String> = tequila_render_gpu::transport::lower_programs(&snapshot)
        .iter()
        .enumerate()
        .filter_map(|(i, l)| l.as_ref().err().map(|why| format!("material {i}: {why}")))
        .collect();
    assert!(refused.is_empty(), "{name}: the card refuses {refused:?}");

    let started = std::time::Instant::now();
    let mut renderer = renderer(gpu, &snapshot);
    // The processor adds the card's samples up, as it adds up its own: the
    // comparison is of the paths alone.
    renderer.accumulate_on_card = false;
    let mut card = Framebuffer::new(scene.width, scene.height);
    let settings = FrameSettings {
        camera,
        config: config.clone(),
        scene_seed: scene.seed,
        reject_fireflies: true,
        clamp_direct: None,
        clamp_indirect: None,
    };
    renderer.render_pass(&mut card, &settings, scene.samples);
    let card_time = started.elapsed();
    let apart = same_as_the_whole_kernel(name, &mut renderer, &card, &settings, scene.samples);

    let started = std::time::Instant::now();
    let intersector = EmbreeIntersector::build(&Device::shared().expect("an Embree device"), snapshot).expect("builds");
    let context = RenderContext { camera, config, scene_seed: scene.seed, ..RenderContext::default() };
    let mut schedule = ProgressiveSchedule::default();
    schedule.update(60_000);
    let mut processor = Framebuffer::new(scene.width, scene.height);
    render_pass(&intersector, &mut processor, &context, &schedule, scene.samples);
    let processor_time = started.elapsed();

    let mut sum = 0.0f64;
    let mut far = 0usize;
    let mut worst = (0.0f32, 0usize);
    let (mut card_mean, mut processor_mean) = (0.0f64, 0.0f64);
    // The alpha too: a shadow catcher's coverage is the floor's shadow and
    // reflection, which the card has to find as the processor does.
    let mut alpha_sum = 0.0f64;
    let mut alpha_worst = 0.0f32;
    for (a, b) in card.pixels().iter().zip(processor.pixels()) {
        let d = (a.alpha() - b.alpha()).abs();
        alpha_sum += d as f64;
        alpha_worst = alpha_worst.max(d);
    }
    for (i, (a, b)) in card.pixels().iter().zip(processor.pixels()).enumerate() {
        let (a, b) = (a.mean(), b.mean());
        let d = [(a.r - b.r).abs(), (a.g - b.g).abs(), (a.b - b.b).abs()];
        sum += (d[0] + d[1] + d[2]) as f64;
        card_mean += (a.r + a.g + a.b) as f64;
        processor_mean += (b.r + b.g + b.b) as f64;
        let largest = d[0].max(d[1]).max(d[2]);
        if largest > 0.02 {
            far += 1;
        }
        if largest > worst.0 {
            worst = (largest, i);
        }
    }
    // TEQUILA_PARITY_DUMP=folder: both pictures as raw little-endian f32
    // RGB, width x height, to look at, and their alphas.
    if let Ok(folder) = std::env::var("TEQUILA_PARITY_DUMP") {
        for (name, buffer) in [("card", &card), ("processor", &processor)] {
            let bytes: Vec<u8> = buffer
                .pixels()
                .iter()
                .flat_map(|p| {
                    let c = p.mean();
                    [c.r, c.g, c.b]
                })
                .flat_map(f32::to_le_bytes)
                .collect();
            std::fs::write(std::path::Path::new(&folder).join(format!("{}-{name}.f32", scene.name)), bytes).expect("writes");
            // And the alpha, one float a pixel: a shadow catcher's shadow
            // and reflection.
            let alpha: Vec<u8> = buffer.pixels().iter().flat_map(|p| p.alpha().to_le_bytes()).collect();
            std::fs::write(std::path::Path::new(&folder).join(format!("{}-{name}-alpha.f32", scene.name)), alpha).expect("writes");
        }
    }
    // TEQUILA_PARITY_PNG=folder: both pictures to look at, exposed alike
    // (the brightest hundredth of the processor's at white), and their
    // difference ten times over.
    if let Ok(folder) = std::env::var("TEQUILA_PARITY_PNG") {
        let mut levels: Vec<f32> = processor.pixels().iter().map(|p| p.mean().max_component()).collect();
        levels.sort_by(f32::total_cmp);
        let white = levels[levels.len() * 99 / 100].max(1e-6);
        let byte = |v: f32| ((v / white).clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0 + 0.5) as u8;
        let save = |file: &str, rgb: &dyn Fn(usize) -> [u8; 3]| {
            let rgba: Vec<u8> = (0..card.pixels().len()).flat_map(|i| {
                let [r, g, b] = rgb(i);
                [r, g, b, 255]
            }).collect();
            let frame = tequila_texture::video::Frame { width: scene.width, height: scene.height, rgba, seconds: 0.0 };
            tequila_texture::video::save_png(&frame, &std::path::Path::new(&folder).join(file)).expect("writes");
        };
        for (which, buffer) in [("card", &card), ("processor", &processor)] {
            save(&format!("{}-{which}.png", scene.name), &|i| {
                let c = buffer.pixels()[i].mean();
                [byte(c.r), byte(c.g), byte(c.b)]
            });
        }
        save(&format!("{}-difference.png", scene.name), &|i| {
            let (a, b) = (card.pixels()[i].mean(), processor.pixels()[i].mean());
            let d = |x: f32, y: f32| byte((x - y).abs() * 10.0);
            [d(a.r, b.r), d(a.g, b.g), d(a.b, b.b)]
        });
    }
    let pixels = card.pixels().len();
    let mean = sum / (pixels * 3) as f64;
    let share = far as f64 / pixels as f64;
    let alpha_mean = alpha_sum / pixels as f64;
    eprintln!(
        "{} {}x{} at {} samples: card {:.2} s, processor {:.2} s; mean {:.5} against {:.5}; \
         mean |difference| {mean:.6}; {far} pixels ({:.3}%) more than 0.02 apart; worst {:.4} at pixel {}; \
         alpha mean |difference| {alpha_mean:.6}, worst {alpha_worst:.4}",
        scene.name,
        scene.width,
        scene.height,
        scene.samples,
        card_time.as_secs_f64(),
        processor_time.as_secs_f64(),
        card_mean / (pixels * 3) as f64,
        processor_mean / (pixels * 3) as f64,
        100.0 * share,
        worst.0,
        worst.1
    );
    if std::env::var("TEQUILA_PARITY_REPORT_ONLY").is_ok() {
        return;
    }
    assert!(mean < 0.002, "{name}: the mean absolute difference is {mean:.6}, not under 0.002");
    assert!(share <= 0.001, "{name}: {far} pixels ({:.3}%) are more than 0.02 apart, more than 0.1%", 100.0 * share);
    // A sample's coverage is all or nothing where a shadow catcher's shadow
    // ray is (one direction, blocked or not), so the same paths give the
    // same alpha but where a ray query and Embree round an edge apart:
    // drawing the shadow from another bounce's dimensions, statistically
    // the same shadow, is 0.0015 here and passes the colour's bar.
    assert!(alpha_mean < 1e-4, "{name}: the alpha's mean absolute difference is {alpha_mean:.6}, not under 0.0001");
    assert_eq!(apart, 0, "{name}: the scene's variant of the kernel is not the whole kernel's picture at {apart} pixels");
}

/// The card adding its samples up, firefly rejection and all, gives the
/// picture the processor's `Framebuffer` gives adding the same samples up:
/// the two differ only in the squared luminance the rejection judges by,
/// an f64 on the processor and two f32s on the card.
#[test]
fn adding_up_on_the_card_is_adding_up_on_the_processor() {
    let Some(gpu) = device::for_test(Needs::RayQueries) else { return };
    let scene = parity::scene("grey_product").expect("the parity set has it");
    let document = scene.document();
    let mut environments = tequila_scene::environment::EnvironmentCache::new();
    let environment = environments.environment(&document.environment);
    let (snapshot, _) = snapshot(&document, environment, &mut MeshCache::new());
    let settings = FrameSettings {
        camera: scene.camera(&document),
        config: document.lighting.config(),
        scene_seed: scene.seed,
        reject_fireflies: true,
        clamp_direct: None,
        clamp_indirect: None,
    };
    let mut renderer = renderer(gpu, &snapshot);
    let mut on_card = Framebuffer::new(scene.width, scene.height);
    // In two passes, so the sums go to the card and come back between them.
    renderer.render_pass(&mut on_card, &settings, scene.samples / 2);
    renderer.render_pass(&mut on_card, &settings, scene.samples - scene.samples / 2);
    renderer.accumulate_on_card = false;
    let mut on_processor = Framebuffer::new(scene.width, scene.height);
    renderer.render_pass(&mut on_processor, &settings, scene.samples);
    let mut sum = 0.0f64;
    let mut worst = 0.0f32;
    for (a, b) in on_card.pixels().iter().zip(on_processor.pixels()) {
        assert_eq!(a.samples, b.samples);
        let (a, b) = (a.mean(), b.mean());
        let d = (a.r - b.r).abs().max((a.g - b.g).abs()).max((a.b - b.b).abs());
        sum += d as f64;
        worst = worst.max(d);
    }
    let mean = sum / on_card.pixels().len() as f64;
    eprintln!("on the card against on the processor: mean |difference| {mean:.2e}, worst {worst:.2e}");
    assert!(mean < 1e-5 && worst < 0.01, "adding up on the card differs: mean {mean:.2e}, worst {worst:.2e}");
}

/// The Headphones sample (`assets/samples/Headphones.tequila`): its five
/// graph materials - leather, the brushed stainless hairline that draws the
/// anisotropy's tangent, the rest grained by noise into their bump - run as
/// programs on the card (`docs/07` R3, stage 1), from its own camera, at the
/// parity set's size, samples and seed.
#[test]
fn the_headphones_are_the_processors_picture() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/samples/Headphones.tequila");
    let loaded = tequila_format::load(&path).expect("the Headphones sample loads");
    let document = loaded.document;
    let camera = document.cameras.active().camera;
    // Every one of its graphs is one the card runs.
    let mut environments = tequila_scene::environment::EnvironmentCache::new();
    let environment = environments.environment(&document.environment);
    let (snap, _) = snapshot(&document, environment, &mut MeshCache::new());
    let lowered = tequila_render_gpu::transport::lower_programs(&snap);
    let refused: Vec<String> = lowered.iter().enumerate().filter_map(|(i, l)| l.as_ref().err().map(|why| format!("material {i}: {why}"))).collect();
    let runs = lowered.iter().filter(|l| matches!(l, Ok(Some(_)))).count();
    eprintln!("Headphones: {runs} materials run as programs on the card");
    assert!(refused.is_empty(), "the card refuses: {refused:?}");
    same_shot(Shot { name: "headphones".into(), document, camera, width: 320, height: 180, samples: 256, seed: 7, cache: MeshCache::new() });
}

/// A preset wall: the Headphones' parts and camera, every material of it
/// swapped for a preset of the standard library that the card runs as a
/// program - one opaque on the card's stack, glowing nowhere, with no
/// cutout and no fibres - spread over the library by `variant`. Their
/// rounded edges are taken off, as the card has no Bevel yet.
fn preset_wall(variant: usize) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/samples/Headphones.tequila");
    let mut document = tequila_format::load(&path).expect("the Headphones sample loads").document;
    let (entries, _) = tequila_format::library::read_directory(&tequila_format::library::standard_directory());
    let runnable: Vec<_> = entries
        .into_iter()
        .filter(|entry| {
            let Ok(program) = entry.graph.compile() else { return false };
            let m = &program.constants;
            matches!(tequila_render_gpu::program::lower(&program), Ok(Some(_)))
                && tequila_render_gpu::material::GpuMaterial::supports(m)
                // Opaque, as the walls were before the card took glass.
                && m.transmission_weight == 0.0
                // Nor walked under, as the walls were before the card took
                // subsurface: the subsurface walls wear those.
                && m.subsurface_weight == 0.0
                && m.emission_luminance <= 0.0
                && m.geometry_opacity >= 0.999
                && m.fuzz_density <= 0.0
                // Nor glowing or cut out where its program says.
                && !program.driven.iter().any(|(param, _)| {
                    use tequila_materials::material_type::Param;
                    matches!(param, Param::Opacity | Param::EmissionColor | Param::EmissionLuminance)
                })
        })
        .collect();
    let keys: Vec<_> = document.materials.keys().collect();
    let stride = (runnable.len() / (2 * keys.len()).max(1)).max(1);
    let mut names = Vec::new();
    for (j, key) in keys.iter().enumerate() {
        let entry = &runnable[((2 * j + variant) * stride) % runnable.len()];
        let mut definition = entry.definition();
        definition.graph.material_mut().edge_radius = 0.0;
        names.push(entry.name.clone());
        document.materials.insert_with_key(*key, definition);
    }
    eprintln!("preset wall {variant}: {}", names.join(", "));
    let camera = document.cameras.active().camera;
    same_shot(Shot { name: format!("preset_wall_{variant}"), document, camera, width: 320, height: 180, samples: 256, seed: 7, cache: MeshCache::new() });
}

#[test]
fn a_preset_wall_is_the_processors_picture() {
    preset_wall(0);
}

#[test]
fn another_preset_wall_is_the_processors_picture() {
    preset_wall(1);
}

/// Whether the card runs `program` now where stage 1's lowering did not:
/// the transport's lowering takes it (its pictures none loaded, as no
/// preset of these has one), `program::lower` refuses it.
fn newly_on_the_card(program: &tequila_materials::graph::CompiledGraph) -> bool {
    let mut pictures = tequila_render_gpu::textures::CardPictures::default();
    pictures.finish();
    tequila_render_gpu::program::lower(program).is_err()
        && matches!(tequila_render_gpu::program::lower_with(program, tequila_render_gpu::textures::Pictures::On(&pictures)), Ok(Some(_)))
}

/// A wall of the presets the card has run since R3's procedural nodes
/// (`newly_on_the_card`: Tequila's procedurals, Wave, Gabor, the curvature
/// and the baked occlusion), the Headphones' parts and camera as the preset
/// walls have them, opaque and glowing nowhere: wall `variant` of
/// [`NEW_WALLS`], which between them wear every one of those presets.
fn new_preset_wall(variant: usize) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/samples/Headphones.tequila");
    let mut document = tequila_format::load(&path).expect("the Headphones sample loads").document;
    let (entries, _) = tequila_format::library::read_directory(&tequila_format::library::standard_directory());
    let runnable: Vec<_> = entries
        .into_iter()
        .filter(|entry| {
            let Ok(program) = entry.graph.compile() else { return false };
            let m = &program.constants;
            newly_on_the_card(&program)
                && tequila_render_gpu::material::GpuMaterial::supports(m)
                && m.transmission_weight == 0.0
                && m.emission_luminance <= 0.0
                && m.geometry_opacity >= 0.999
                && m.fuzz_density <= 0.0
                // Nor glowing or cut out where its program says, as the
                // older walls: the card took those after these nodes.
                && !program.driven.iter().any(|(param, _)| {
                    use tequila_materials::material_type::Param;
                    matches!(param, Param::Opacity | Param::EmissionColor | Param::EmissionLuminance)
                })
        })
        .collect();
    assert!(!runnable.is_empty(), "no preset is newly on the card");
    let keys: Vec<_> = document.materials.keys().collect();
    let mut names = Vec::new();
    for (j, key) in keys.iter().enumerate() {
        let entry = &runnable[(NEW_WALLS * j + variant) % runnable.len()];
        let mut definition = entry.definition();
        definition.graph.material_mut().edge_radius = 0.0;
        names.push(entry.name.clone());
        document.materials.insert_with_key(*key, definition);
    }
    assert!(NEW_WALLS * keys.len() >= runnable.len(), "{} presets newly on the card, more than {NEW_WALLS} walls of {} wear", runnable.len(), keys.len());
    eprintln!("new preset wall {variant} (of {} newly on the card): {}", runnable.len(), names.join(", "));
    let camera = document.cameras.active().camera;
    same_shot(Shot { name: format!("new_preset_wall_{variant}"), document, camera, width: 320, height: 180, samples: 256, seed: 7, cache: MeshCache::new() });
}

#[test]
fn a_wall_of_the_new_presets_is_the_processors_picture() {
    new_preset_wall(0);
}

#[test]
fn another_wall_of_the_new_presets_is_the_processors_picture() {
    new_preset_wall(1);
}

#[test]
fn a_third_wall_of_the_new_presets_is_the_processors_picture() {
    new_preset_wall(2);
}

/// How many walls the presets newly on the card are spread over.
const NEW_WALLS: usize = 3;

/// The presets of the standard library that round their edges
/// (`edge_radius`) and that the card renders whole: its lowering takes
/// their programs (their pictures none loaded, as no preset of these has
/// one), its stack their surfaces, and they grow no fibres and wear no
/// flakes. Glass, glows and cutouts among them.
fn rounding_presets() -> Vec<tequila_format::library::LibraryEntry> {
    let (entries, _) = tequila_format::library::read_directory(&tequila_format::library::standard_directory());
    let mut pictures = tequila_render_gpu::textures::CardPictures::default();
    pictures.finish();
    entries
        .into_iter()
        .filter(|entry| {
            let Ok(program) = entry.graph.compile() else { return false };
            let m = &program.constants;
            m.edge_radius > 0.0
                && tequila_render_gpu::program::lower_with(&program, tequila_render_gpu::textures::Pictures::On(&pictures)).is_ok()
                && tequila_render_gpu::material::GpuMaterial::supports(m)
                && m.fuzz_density <= 0.0
                // Nor flakes, which the card left out when these walls
                // were chosen: kept out, so the walls keep their members
                // (the flake presets have a wall of their own).
                && m.flake_weight <= 1e-4
                // Nor glass that disperses light, for the same reason.
                && !(m.transmission_weight > 0.0 && m.transmission_dispersion_abbe > 0.0)
                // Nor walked in or under, as the walls were before the card
                // took media: the subsurface and scattering walls wear those.
                && m.subsurface_weight == 0.0
                && !(m.transmission_weight > 0.0 && m.volume_scattering().is_some())
        })
        .collect()
}

/// A bevel wall: the Headphones' parts and camera, every material swapped
/// for a preset that rounds its edges, its radius and probes kept
/// ([`rounding_presets`]), spread over them by `variant` of
/// [`BEVEL_WALLS`].
fn bevel_wall(variant: usize) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/samples/Headphones.tequila");
    let mut document = tequila_format::load(&path).expect("the Headphones sample loads").document;
    let runnable = rounding_presets();
    assert!(!runnable.is_empty(), "no preset rounds its edges");
    let keys: Vec<_> = document.materials.keys().collect();
    let stride = (runnable.len() / (BEVEL_WALLS * keys.len()).max(1)).max(1);
    let mut names = Vec::new();
    for (j, key) in keys.iter().enumerate() {
        let entry = &runnable[((BEVEL_WALLS * j + variant) * stride) % runnable.len()];
        names.push(format!("{} ({} m)", entry.name, entry.graph.material().edge_radius));
        document.materials.insert_with_key(*key, entry.definition());
    }
    eprintln!("bevel wall {variant} (of {} presets that round their edges): {}", runnable.len(), names.join(", "));
    let camera = document.cameras.active().camera;
    same_shot(Shot { name: format!("bevel_wall_{variant}"), document, camera, width: 320, height: 180, samples: 256, seed: 7, cache: MeshCache::new() });
}

#[test]
fn a_bevel_wall_is_the_processors_picture() {
    bevel_wall(0);
}

#[test]
fn another_bevel_wall_is_the_processors_picture() {
    bevel_wall(1);
}

#[test]
fn a_third_bevel_wall_is_the_processors_picture() {
    bevel_wall(2);
}

/// How many walls the presets that round their edges are spread over.
const BEVEL_WALLS: usize = 3;

/// Which presets of the bevel walls the card renders otherwise: each swapped
/// in alone, rounded, and again with its edges sharp. Run by hand,
/// `TEQUILA_PARITY_REPORT_ONLY=1 cargo test --test parity -- --ignored
/// --nocapture bevel_walls_one_by_one`.
#[test]
#[ignore]
fn the_bevel_walls_one_by_one() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/samples/Headphones.tequila");
    let original = tequila_format::load(&path).expect("the Headphones sample loads").document;
    let runnable = rounding_presets();
    let keys: Vec<_> = original.materials.keys().collect();
    let stride = (runnable.len() / (BEVEL_WALLS * keys.len()).max(1)).max(1);
    for variant in 0..BEVEL_WALLS {
        for (j, key) in keys.iter().enumerate() {
            let entry = &runnable[((BEVEL_WALLS * j + variant) * stride) % runnable.len()];
            for rounded in [true, false] {
                let mut document = original.clone();
                let mut definition = entry.definition();
                if !rounded {
                    definition.graph.material_mut().edge_radius = 0.0;
                }
                document.materials.insert_with_key(*key, definition);
                let camera = document.cameras.active().camera;
                eprint!("{} on {key:?}, {}: ", entry.name, if rounded { "rounded" } else { "sharp" });
                same_shot(Shot { name: format!("bevel_one_{variant}_{j}_{rounded}"), document, camera, width: 160, height: 90, samples: 128, seed: 7, cache: MeshCache::new() });
            }
        }
    }
}

/// What the snapshot measures at the mesh's corners, read by programs on the
/// card (R3: the curvature, the baked occlusion, the distance to other parts,
/// vertex colours): the Headphones' parts, every mesh given vertex colours
/// banded over it, every material its constants with one of the measures
/// driving its base colour or its roughness - the barycentrics worked out
/// again at the hit, the measures uploaded and weighed, the curvature by the
/// part's scale, the colours into the rendering space.
#[test]
fn the_measured_headphones_are_the_processors_picture() {
    use tequila_materials::graph::{CurvatureMode, MaterialGraph, Node, NodeKind, Socket, Value};
    use tequila_materials::material_type::Param;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/samples/Headphones.tequila");
    let mut document = tequila_format::load(&path).expect("the Headphones sample loads").document;
    let meshes: Vec<_> = document.geometry.keys().collect();
    for key in meshes {
        let mut mesh = document.geometry.get(key).expect("a mesh").clone();
        let band = |x: f32| 0.5 + 0.5 * (x * 60.0).sin();
        mesh.colors = mesh.positions.iter().map(|p| tequila_core::math::Vec4 { x: band(p.x), y: band(p.y + 0.01), z: band(p.z - 0.02), w: 1.0 }).collect();
        document.geometry.insert_with_key(key, mesh);
    }
    let colour = |r: f32, g: f32, b: f32| Value::Color(tequila_core::spectrum::Spectrum { r, g, b });
    let keys: Vec<_> = document.materials.keys().collect();
    for (j, key) in keys.iter().enumerate() {
        let mut definition = document.materials.get(*key).expect("a material").clone();
        let mut graph = MaterialGraph::from_material(definition.graph.material().clone());
        graph.material_mut().edge_radius = 0.0;
        let output = graph.output();
        // A mask between two colours, into the base colour.
        let mix_into_base = |graph: &mut MaterialGraph, mask: NodeKind| {
            let mask = graph.add_node(Node::new(mask));
            let mut mix = Node::new(NodeKind::Mix);
            mix.literals.insert(Socket::Input(0), colour(0.05, 0.05, 0.06));
            mix.literals.insert(Socket::Input(1), colour(0.9, 0.55, 0.2));
            let mix = graph.add_node(mix);
            graph.connect(mask, mix, Socket::Input(2));
            graph.connect(mix, output, Socket::Param(Param::BaseColor));
        };
        match j % 5 {
            0 => mix_into_base(&mut graph, NodeKind::Curvature { mode: CurvatureMode::Convex, radius: 0.004, contrast: 1.2 }),
            1 => mix_into_base(&mut graph, NodeKind::Occlusion { radius: 0.01, contrast: 1.5 }),
            2 => {
                let distance = graph.add_node(Node::new(NodeKind::Distance { radius: 0.02 }));
                graph.connect(distance, output, Socket::Param(Param::SpecularRoughness));
                let curvature = graph.add_node(Node::new(NodeKind::Curvature { mode: CurvatureMode::Both, radius: 0.003, contrast: 0.7 }));
                graph.connect(curvature, output, Socket::Param(Param::BaseColor));
            }
            3 => {
                let painted = graph.add_node(Node::new(NodeKind::VertexColor));
                graph.connect(painted, output, Socket::Param(Param::BaseColor));
            }
            _ => mix_into_base(&mut graph, NodeKind::Curvature { mode: CurvatureMode::Concave, radius: 0.004, contrast: 1.0 }),
        }
        definition.graph = graph;
        document.materials.insert_with_key(*key, definition);
    }
    let camera = document.cameras.active().camera;
    same_shot(Shot { name: "measured_headphones".into(), document, camera, width: 320, height: 180, samples: 256, seed: 7, cache: MeshCache::new() });
}

/// Every Tequila procedural the card runs, at the menu's settings
/// (`Procedural::catalog`; Wood (Advanced) is not on the card): the
/// Headphones' parts, each material its constants with one pattern mixing
/// its base colour between two and, on every other part, bumping it too;
/// in the part's space, and on every third part through a planar mapping.
#[test]
fn the_patterned_headphones_are_the_processors_picture() {
    use tequila_materials::graph::{MaterialGraph, Node, NodeKind, Socket, Value};
    use tequila_materials::material_type::Param;
    use tequila_texture::procedural::Procedural;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/samples/Headphones.tequila");
    let mut document = tequila_format::load(&path).expect("the Headphones sample loads").document;
    let patterns: Vec<Procedural> = Procedural::catalog().into_iter().filter(|p| !matches!(p, Procedural::WoodAdvanced { .. })).collect();
    let colour = |r: f32, g: f32, b: f32| Value::Color(tequila_core::spectrum::Spectrum { r, g, b });
    let keys: Vec<_> = document.materials.keys().collect();
    let mut names = Vec::new();
    for (j, key) in keys.iter().enumerate() {
        let mut definition = document.materials.get(*key).expect("a material").clone();
        let mut graph = MaterialGraph::from_material(definition.graph.material().clone());
        graph.material_mut().edge_radius = 0.0;
        let output = graph.output();
        let pattern = patterns[j % patterns.len()];
        names.push(pattern.label());
        let node = graph.add_node(Node::new(NodeKind::Procedural { procedural: Box::new(pattern), seed: j as u32 * 7 }));
        if j % 3 == 2 {
            let planar = tequila_texture::projection::Mapping { projection: tequila_texture::projection::Projection::Planar, axis: tequila_texture::projection::Axis::Z, real_world_units: true, scale: tequila_core::math::vec3(0.05, 0.05, 1.0), ..Default::default() };
            let m = graph.add_node(Node::new(NodeKind::Mapping2d { mapping: planar }));
            graph.connect(m, node, Socket::Input(0));
        }
        let mut mix = Node::new(NodeKind::Mix);
        mix.literals.insert(Socket::Input(0), colour(0.06, 0.07, 0.08));
        mix.literals.insert(Socket::Input(1), colour(0.85, 0.6, 0.3));
        let mix = graph.add_node(mix);
        graph.connect(node, mix, Socket::Input(2));
        graph.connect(mix, output, Socket::Param(Param::BaseColor));
        if j % 2 == 0 {
            let bump = graph.add_node(Node::new(NodeKind::Bump { depth: 0.0003 }));
            graph.connect(node, bump, Socket::Input(0));
            graph.connect(bump, output, Socket::Param(Param::Bump));
        }
        definition.graph = graph;
        document.materials.insert_with_key(*key, definition);
    }
    eprintln!("patterned headphones: {}", names.join(", "));
    let camera = document.cameras.active().camera;
    same_shot(Shot { name: "patterned_headphones".into(), document, camera, width: 320, height: 180, samples: 256, seed: 7, cache: MeshCache::new() });
}

/// Which presets of the walls the card renders otherwise: each swapped in
/// alone. Run by hand, `TEQUILA_PARITY_REPORT_ONLY=1 cargo test --test parity
/// -- --ignored --nocapture one_by_one`.
#[test]
#[ignore]
fn the_preset_walls_one_by_one() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/samples/Headphones.tequila");
    let original = tequila_format::load(&path).expect("the Headphones sample loads").document;
    let (entries, _) = tequila_format::library::read_directory(&tequila_format::library::standard_directory());
    let runnable: Vec<_> = entries
        .into_iter()
        .filter(|entry| {
            let Ok(program) = entry.graph.compile() else { return false };
            let m = &program.constants;
            matches!(tequila_render_gpu::program::lower(&program), Ok(Some(_)))
                && tequila_render_gpu::material::GpuMaterial::supports(m)
                // Opaque, as the walls were before the card took glass.
                && m.transmission_weight == 0.0
                // Nor walked under, as the walls were before the card took
                // subsurface: the subsurface walls wear those.
                && m.subsurface_weight == 0.0
                && m.emission_luminance <= 0.0
                && m.geometry_opacity >= 0.999
                && m.fuzz_density <= 0.0
                // Nor glowing or cut out where its program says.
                && !program.driven.iter().any(|(param, _)| {
                    use tequila_materials::material_type::Param;
                    matches!(param, Param::Opacity | Param::EmissionColor | Param::EmissionLuminance)
                })
        })
        .collect();
    let keys: Vec<_> = original.materials.keys().collect();
    let stride = (runnable.len() / (2 * keys.len()).max(1)).max(1);
    for variant in 0..2 {
        for (j, key) in keys.iter().enumerate() {
            let entry = &runnable[((2 * j + variant) * stride) % runnable.len()];
            let mut document = original.clone();
            let mut definition = entry.definition();
            definition.graph.material_mut().edge_radius = 0.0;
            document.materials.insert_with_key(*key, definition);
            let camera = document.cameras.active().camera;
            eprint!("{} on {key:?}: ", entry.name);
            same_shot(Shot { name: format!("one_{variant}_{j}"), document, camera, width: 160, height: 90, samples: 128, seed: 7, cache: MeshCache::new() });
        }
    }
}

/// A preset on the Headphones' cups as its program and as its constants
/// alone: whether what the card renders otherwise is the program's or the
/// surface's. Run by hand, `TEQUILA_PARITY_REPORT_ONLY=1 TEQUILA_PRESETS=..
/// cargo test --test parity -- --ignored --nocapture program_or_surface`.
#[test]
#[ignore]
fn a_preset_as_program_or_surface() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/samples/Headphones.tequila");
    let original = tequila_format::load(&path).expect("the Headphones sample loads").document;
    let (entries, _) = tequila_format::library::read_directory(&tequila_format::library::standard_directory());
    let key = original.materials.keys().nth(12).expect("the cups' material");
    for name in std::env::var("TEQUILA_PRESETS").unwrap_or_default().split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let Some(entry) = entries.iter().find(|e| e.name == name) else { continue };
        let program = entry.graph.compile().expect("compiles");
        eprintln!("{name}: driven {:?}; fuzz {} coat {} metal {} aniso {}", program.driven.iter().map(|(p, _)| p.label()).collect::<Vec<_>>(), program.constants.fuzz_weight, program.constants.coat_weight, program.constants.base_metalness, program.constants.specular_roughness_anisotropy);
        for constant in [false, true] {
            let mut document = original.clone();
            let mut definition = entry.definition();
            if constant {
                definition.graph = tequila_materials::graph::MaterialGraph::from_material(program.constants.clone());
            }
            definition.graph.material_mut().edge_radius = 0.0;
            document.materials.insert_with_key(key, definition);
            let camera = document.cameras.active().camera;
            eprint!("  {}: ", if constant { "constants" } else { "program" });
            same_shot(Shot { name: format!("pos_{constant}"), document, camera, width: 160, height: 90, samples: 128, seed: 7, cache: MeshCache::new() });
        }
    }
}
