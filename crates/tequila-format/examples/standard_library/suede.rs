//! Suede, two ways: fine calf suede, the nap raised on the grain side and
//! buffed short, and roughout, the flesh side of a full-grain hide worn
//! outward, as work boots have it.
//!
//! Both are velvet underneath - a sheen lighter and paler than the leather,
//! strongest where the surface is seen edge on - with the nap drawn in the
//! graph: how it stands and lies, how it tufts and clumps, and where it is
//! pressed flat. Lengths are real ones, in metres, read in the part's own
//! space: 7000 a metre is a feature a seventh of a millimetre across.
//!
//! Curvature is measured once a vertex, so an edge counts as burnished only
//! when it is clearly sharp; a gentler threshold traces a coarse mesh's edge
//! loops across a panel. Neither grows real fibres: grown, a hairy nap
//! reads as fur on a shoe, and a calf suede's, a few tenths of a millimetre,
//! is drawn better by the sheen than by fibres capped at 250,000 a part.

use crate::recipe::Out;
use crate::{base, preset, srgb, Preset, Recipe};
use tequila_core::spectrum::Spectrum;
use tequila_materials::graph::{CurvatureMode, Node, NodeKind};
use tequila_materials::material_type::{MaterialType, Param};

/// `colour` times `factor`, a node's output.
pub(crate) fn tint(r: &mut Recipe, colour: Spectrum, factor: Out) -> Out {
    r.node("mix").set("blend_type", "multiply").f("Factor", 1.0).c("A", colour).w("B", factor).out("Result")
}

pub(crate) fn noise(r: &mut Recipe, coords: Out, scale: f32, detail: f32, roughness: f32, distortion: f32) -> Out {
    r.node("noise_texture")
        .w("Vector", coords)
        .f("Scale", scale)
        .f("Detail", detail)
        .f("Roughness", roughness)
        .f("Distortion", distortion)
        .out("Fac")
}

pub(crate) fn product(r: &mut Recipe, factors: &[Out]) -> Out {
    let mut out = factors[0];
    for &f in &factors[1..] {
        out = r.math("multiply", out, f);
    }
    out
}

pub(crate) fn curvature(r: &mut Recipe, radius: f32, contrast: f32) -> Out {
    Out(r.graph.add_node(Node::new(NodeKind::Curvature { mode: CurvatureMode::Convex, radius, contrast })), 0)
}

pub(crate) fn occlusion(r: &mut Recipe, radius: f32, contrast: f32) -> Out {
    Out(r.graph.add_node(Node::new(NodeKind::Occlusion { radius, contrast })), 0)
}


pub fn all() -> Vec<Preset> {
    let roughout_tags = |colour: &'static [&'static str]| -> &'static [&'static str] {
        let mut tags = vec!["suede", "roughout", "rough-out", "hairy suede", "reverse leather", "nubuck", "leather", "nap", "boot", "footwear"];
        tags.extend_from_slice(colour);
        Vec::leak(tags)
    };
    vec![
        fine(),
        roughout("Roughout Suede Tobacco", roughout_tags(&["brown", "tobacco"]), &TOBACCO),
        roughout("Roughout Suede Cream", roughout_tags(&["cream", "beige", "off-white"]), &CREAM),
    ]
}

/// A roughout's colours, as sRGB: the tufts'; the sheen over them, paler
/// and greyer than the leather, as a scanned suede's is; the flat colour
/// under the graph; and the stray hairs'.
struct Palette {
    body: &'static str,
    sheen: &'static str,
    base: &'static str,
    hairs: &'static str,
}

const TOBACCO: Palette = Palette { body: "#86593A", sheen: "#A08C7A", base: "#7A5232", hairs: "#9A6A3C" };

/// Paler and warmer than cream is to the eye: the sheen layer and the
/// shading take it down, and a pale yellow taken down turns olive.
const CREAM: Palette = Palette { body: "#F4E2C4", sheen: "#F8EEDD", base: "#EEDCBD", hairs: "#F2E4CB" };

/// Calf suede: a nap so short and fine the surface reads as velvet, a
/// frosted pile up close, laid one way here and another there by brushing.
fn fine() -> Preset {
    let leather = srgb("#846E58");
    let sheen = srgb("#D4C6B2");
    let mut r = Recipe::new(base(MaterialType::Velvet, |m| {
        m.base_color = leather;
        m.fuzz_color = sheen;
        m.fuzz_weight = 1.0;
        m.fuzz_roughness = 0.62;
    }));
    let coords = r.node("texture_coordinate").out("Object");
    // Brushed patches: soft, irregular, a few centimetres across and only a
    // little longer one way than the other, so on a shoe they never read as
    // bands; faint.
    let turned = r.node("mapping").w("Vector", coords).v("Rotation", [0.35, 0.9, 0.2]).v("Scale", [1.6, 1.0, 1.15]).out("Vector");
    let brushed = noise(&mut r, turned, 7.0, 3.0, 0.55, 1.5);
    let nap = r.smooth(brushed, (0.32, 0.68), (0.0, 1.0));
    // Dye taken up a little unevenly, in clouds a few centimetres wide.
    let cloud_field = noise(&mut r, coords, 14.0, 4.0, 0.55, 0.0);
    let cloud = r.smooth(cloud_field, (0.3, 0.7), (0.96, 1.04));
    // Fibre tips at a tenth of a millimetre, tufts at half a millimetre, and
    // the pile standing and lying in patches between: suede's frost.
    let grain_field = noise(&mut r, coords, 7000.0, 2.0, 0.6, 0.0);
    let grain = r.smooth(grain_field, (0.3, 0.7), (0.96, 1.04));
    let tufts = r.node("voronoi_texture").w("Vector", coords).f("Scale", 1700.0).out("Distance");
    let pile_field = noise(&mut r, coords, 2000.0, 3.0, 0.65, 0.0);
    let pile = r.smooth(pile_field, (0.3, 0.7), (0.0, 1.0));
    // Rubbed edges: the nap lies flat, darker, its sheen tighter.
    let curv = curvature(&mut r, 0.0025, 1.0);
    let burnish = r.smooth(curv, (0.5, 0.95), (1.0, 0.88));

    let nap_tone = r.smooth(nap, (0.0, 1.0), (0.99, 1.01));
    let pile_tone = r.smooth(pile, (0.0, 1.0), (0.93, 1.05));
    let shade = product(&mut r, &[cloud, grain, nap_tone, pile_tone, burnish]);
    let colour = tint(&mut r, leather, shade);
    r.to(Param::BaseColor, colour);
    let nap_sheen = r.smooth(nap, (0.0, 1.0), (0.94, 1.05));
    let pile_sheen = r.smooth(pile, (0.0, 1.0), (0.84, 1.12));
    let glow = product(&mut r, &[cloud, nap_sheen, pile_sheen, burnish]);
    let glow = tint(&mut r, sheen, glow);
    r.to(Param::FuzzColor, glow);
    // A rougher sheen where the nap stands, a tighter one where it lies.
    let stands = r.smooth(nap, (0.0, 1.0), (-0.04, 0.03));
    let flat = r.smooth(curv, (0.5, 0.95), (0.0, -0.1));
    let lie = r.math("add", stands, flat);
    let lie = r.math("add", lie, 0.62);
    r.to(Param::FuzzRoughness, lie);
    // Height: tufts as soft domes, the pile and the grain on top.
    let domes = r.smooth(tufts, (0.0, 0.55), (1.0, 0.0));
    let fine_part = r.math("multiply", grain_field, 0.2);
    let pile_part = r.math3("multiply_add", pile_field, 0.35, fine_part);
    let height = r.math3("multiply_add", domes, 0.45, pile_part);
    r.bump(height, 2.2e-5);
    preset(
        "Fine Suede Sand",
        "Leather",
        &["suede", "fine suede", "calf suede", "nubuck", "leather", "nap", "shoe", "footwear", "sand", "beige"],
        MaterialType::Velvet,
        r,
    )
}

/// Roughout: the flesh side worn out, its nap torn up into ragged tufts a
/// millimetre or so across, as a boot's roughout and a hairy suede are,
/// each tuft's fibres lying their own way, its top catching the light and
/// the cracks between them dark; blotchy where the hide took the dye
/// unevenly, darker where handling has pressed it down, burnished on rubbed
/// edges. The tufts are drawn, in the bump and the colour alike; a sparse
/// halo of real fibres over them gives the stray hairs at the edges.
fn roughout(name: &'static str, tags: &'static [&'static str], palette: &Palette) -> Preset {
    let body = srgb(palette.body);
    let sheen = srgb(palette.sheen);
    let mut r = Recipe::new(base(MaterialType::Velvet, |m| {
        m.base_color = srgb(palette.base);
        // The stray hairs' own colour: a tuft's top's.
        m.fuzz_color = srgb(palette.hairs);
        m.fuzz_weight = 1.0;
        m.fuzz_roughness = 0.5;
        // A sparse halo of real fibres over the drawn tufts: the stray,
        // curling hairs at their edges, not a coat.
        m.fuzz_density = 250.0;
        m.fuzz_length = 0.0015;
        m.fuzz_thickness = 0.00003;
        m.fuzz_lean = 0.8;
        m.fuzz_curl = 0.4;
        m.fuzz_clump = 0.3;
        m.fuzz_comb = 0.5;
        m.fuzz_color_variation = 0.4;
    }));
    let coords = r.node("texture_coordinate").out("Object");
    // The nap is combed one way, as a hide's is brushed, wandering a little
    // over patches a few centimetres across: the space the tufts are drawn
    // in pushed a few millimetres about, then turned so that the way they
    // lie, along the part's diagonal, is its first axis. The diagonal runs
    // along every face of a part modelled along its axes.
    let push = r.node("noise_texture").w("Vector", coords).f("Scale", 25.0).f("Detail", 2.0).f("Roughness", 0.5).out("Color");
    let wandered = r.node("vector_math").set("operation", "multiply_add").w("Vector", push).v("Vector 2", [0.004, 0.004, 0.004]).w("Vector 3", coords).out("Vector");
    // Torn: pushed about again at the tufts' own size, so their edges rip,
    // and much finer, so they fray.
    let tear = r.node("noise_texture").w("Vector", wandered).f("Scale", 500.0).f("Detail", 3.0).f("Roughness", 0.55).out("Color");
    let torn = r.node("vector_math").set("operation", "multiply_add").w("Vector", tear).v("Vector 2", [0.0005, 0.0005, 0.0005]).w("Vector 3", wandered).out("Vector");
    let fray = r.node("noise_texture").w("Vector", torn).f("Scale", 3500.0).f("Detail", 2.0).f("Roughness", 0.6).out("Color");
    let frayed = r.node("vector_math").set("operation", "multiply_add").w("Vector", fray).v("Vector 2", [0.0001, 0.0001, 0.0001]).w("Vector 3", torn).out("Vector");
    let turned = r.node("mapping").w("Vector", frayed).v("Rotation", [0.0, 0.0, -0.785_398]).out("Vector");
    let along = r.node("mapping").w("Vector", turned).v("Rotation", [0.0, 0.615_480, 0.0]).out("Vector");
    // The tufts: torn flakes a millimetre or so across and three times as
    // long the way the nap lies, ragged at every size; the strands over
    // them a tenth of a millimetre wide and twelve times as long.
    let flake_space = r.node("mapping").w("Vector", along).v("Scale", [1.0 / 3.0, 1.0, 1.0]).out("Vector");
    let flake_field = noise(&mut r, flake_space, 1100.0, 6.0, 0.62, 0.6);
    let strand_space = r.node("mapping").w("Vector", along).v("Scale", [1.0 / 12.0, 1.0, 1.0]).out("Vector");
    let strand_field = noise(&mut r, strand_space, 4500.0, 2.0, 0.6, 0.0);
    let fibre = r.smooth(strand_field, (0.3, 0.7), (0.0, 1.0));
    let grain = noise(&mut r, coords, 6000.0, 2.0, 0.6, 0.0);
    // Frayed along the strands.
    let fibre_fray = r.math("subtract", fibre, 0.5);
    let flake_frayed = r.math3("multiply_add", fibre_fray, 0.1, flake_field);
    let dome = r.smooth(flake_frayed, (0.36, 0.62), (0.0, 1.0));
    // Each patch its own shade, a little.
    let tone_field = noise(&mut r, coords, 300.0, 2.0, 0.5, 0.0);
    let tuft_tone = r.smooth(tone_field, (0.3, 0.7), (0.96, 1.03));
    let fibre_part = r.math("multiply", fibre, 0.2);
    let top_field = r.math3("multiply_add", dome, 0.8, fibre_part);
    let lit = r.smooth(top_field, (0.05, 0.95), (0.0, 1.0));
    // Only a little darker down in the cracks and lighter on the tops: the
    // bump's own shading gives the depth, as a scanned suede's normals do.
    let tone = r.smooth(lit, (0.0, 1.0), (0.92, 1.04));
    let hair = tint(&mut r, body, tone);
    // The hide's own unevenness, a hand's width and a thumb's.
    let blotch_field = noise(&mut r, coords, 8.0, 4.0, 0.6, 0.8);
    let blotch = r.smooth(blotch_field, (0.35, 0.65), (0.9, 1.06));
    let mottle_field = noise(&mut r, coords, 60.0, 5.0, 0.65, 0.0);
    let mottle = r.smooth(mottle_field, (0.3, 0.7), (0.97, 1.03));
    // Pressed-down patches from handling: the nap flattened, darker and
    // smoother.
    let press_field = noise(&mut r, coords, 5.0, 3.0, 0.5, 0.0);
    let press = r.smooth(press_field, (0.58, 0.74), (0.0, 1.0));
    let pressed = r.smooth(press, (0.0, 1.0), (1.0, 0.82));
    // Burnish on rubbed edges; dirt and shadow down in the creases.
    let curv = curvature(&mut r, 0.004, 1.2);
    let burnish = r.smooth(curv, (0.35, 0.9), (1.0, 0.72));
    let occ = occlusion(&mut r, 0.012, 1.0);
    let crease = r.smooth(occ, (0.0, 1.0), (0.8, 1.0));

    let shade = product(&mut r, &[tuft_tone, blotch, mottle, pressed, burnish, crease]);
    let colour = r.mix("multiply", 1.0, hair, shade);
    r.to(Param::BaseColor, colour);
    // The sheen strongest on the tufts' tops, where the fibre tips stand.
    let tip_sheen = r.smooth(lit, (0.0, 1.0), (0.92, 1.04));
    let glow = product(&mut r, &[tip_sheen, blotch, pressed, burnish, crease]);
    let glow = tint(&mut r, sheen, glow);
    r.to(Param::FuzzColor, glow);
    // The sheen tightens where the nap is pressed or rubbed flat.
    let held = r.smooth(press, (0.0, 1.0), (0.0, -0.15));
    let rubbed = r.smooth(curv, (0.35, 0.9), (0.0, -0.2));
    let lie = r.math("add", held, rubbed);
    let lie = r.math("add", lie, 0.5);
    r.to(Param::FuzzRoughness, lie);
    // Height: the tufts' domes, the fibres on them and a grain under both,
    // falling into the cracks; flattened where the nap is pressed down.
    // Shallow, as a hairy suede's scanned normals are: tilted about 12
    // degrees on average and under 30 nearly everywhere, not stood on end,
    // so the light runs over each tuft rather than scattering off it.
    let grain_part = r.math("multiply", grain, 0.1);
    let fibre_height = r.math3("multiply_add", fibre, 0.25, grain_part);
    // Rounded, as a tuft is: the flakes' own field, not the edge the
    // colour is cut at, which would stand each one up as a mesa.
    let mound = r.range(flake_frayed, (0.25, 0.75), (0.0, 1.0));
    let height = r.math3("multiply_add", mound, 0.65, fibre_height);
    let standing = r.smooth(press, (0.0, 1.0), (1.0, 0.5));
    let height = r.math("multiply", height, standing);
    r.bump(height, 6e-4);
    preset(name, "Leather", tags, MaterialType::Velvet, r)
}
