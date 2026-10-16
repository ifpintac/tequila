//! Marble and travertine, measured against CC0 scans (ambientCG Marble005,
//! Marble012, Marble016, Marble009, Marble024, Travertine009).
//!
//! What the scans have that the contour-line marble before this did not:
//! - Veins are healed cracks: a network of polygons branching at every
//!   scale, the big ones wandering and the small ones jagged.
//! - Each vein fades in and out along its length, swelling into a cloudy
//!   stain here and vanishing there; none runs as an even line.
//! - Between them the stone is not flat but a crystalline grain - every
//!   crystal a shade of its own - over a slow mottle, and it varies by only
//!   a few percent: Carrara's ground is #E1E5E8 to #EBEEF3 from its quarter
//!   to its three-quarter point.

use crate::recipe::{Linear, Out};
use crate::{base, noise, preset, srgb, Preset, Recipe};
use tequila_materials::material_type::{MaterialType, Param};

pub struct Marble {
    pub name: &'static str,
    pub tags: &'static [&'static str],
    /// The stone between the veins, as a ramp over a slow mottle.
    pub ground: &'static [(f32, &'static str)],
    /// The main veins, from where they are faint to where they are strong.
    pub vein: (&'static str, &'static str),
    /// The fine network and the hairlines.
    pub thread: &'static str,
    /// The stain the veins bleed into the stone, and its blotches.
    pub cloud: &'static str,
    /// Main vein cells a metre - sized so a part a few centimetres to a few
    /// decimetres across shows the stone's character, as a swatch does.
    pub scale: f32,
    /// A main vein's half-width where it is widest, in metres.
    pub width: f32,
    /// The fine network's, in metres.
    pub fine: f32,
    /// How many of the main veins show, 0 to 1.
    pub veins: f32,
    /// How much of the fine network shows, 0 to 1.
    pub network: f32,
    /// How strongly the veins stain the stone either side, 0 to 1.
    pub cloud_amount: f32,
    /// Blotches of the cloud colour anywhere in the stone, 0 to 1.
    pub patches: f32,
    /// Mineral specks: their colour, and how many, 0 to 1.
    pub specks: (&'static str, f32),
    /// How far the pattern is drawn out along x: 1 for veins running any
    /// way, more for a block cut along its bedding.
    pub flow: f32,
    pub roughness: f32,
    pub subsurface: f32,
}

const MARBLES: &[Marble] = &[
    // Carrara: a cool white, grey veins with grey clouds, blotched all
    // over, a dense fine network between (Marble005).
    Marble {
        name: "Marble White",
        tags: &["marble", "stone", "polished", "white", "veined", "carrara"],
        ground: &[(0.25, "#DADEE1"), (0.5, "#E4E8EB"), (0.75, "#EDF0F3")],
        vein: ("#B4B8BD", "#8A8E94"),
        thread: "#9A9EA4",
        cloud: "#C4C8CC",
        scale: 16.0,
        width: 0.6e-3,
        fine: 0.15e-3,
        veins: 0.6,
        network: 0.9,
        cloud_amount: 0.7,
        patches: 0.8,
        specks: ("#A4A8AD", 0.4),
        flow: 1.3,
        roughness: 0.06,
        subsurface: 0.4,
    },
    // Calacatta: a warmer white with fewer, bolder veins in grey and
    // gold, long and sweeping.
    Marble {
        name: "Marble Calacatta",
        tags: &["marble", "stone", "polished", "white", "veined", "calacatta", "gold"],
        ground: &[(0.25, "#E9E6E0"), (0.5, "#F0EEE9"), (0.75, "#F6F4F0")],
        vein: ("#9C9286", "#5A544E"),
        thread: "#A89F93",
        cloud: "#D2CABE",
        scale: 9.0,
        width: 2.6e-3,
        fine: 0.15e-3,
        veins: 0.85,
        network: 0.4,
        cloud_amount: 0.75,
        patches: 0.25,
        specks: ("#B3ABA0", 0.1),
        flow: 1.8,
        roughness: 0.05,
        subsurface: 0.4,
    },
    // Bardiglio: blue-grey, its veins darker grey, clouded and blotched
    // heavily, with pale flecks (Marble012).
    Marble {
        name: "Marble Grey",
        tags: &["marble", "stone", "polished", "grey", "veined", "bardiglio"],
        ground: &[(0.25, "#A3A4AC"), (0.5, "#AEAFB9"), (0.75, "#B8B9C3")],
        vein: ("#878892", "#5A5C65"),
        thread: "#7E7F89",
        cloud: "#83858F",
        scale: 14.0,
        width: 1.2e-3,
        fine: 0.18e-3,
        veins: 0.8,
        network: 0.8,
        cloud_amount: 0.9,
        patches: 0.9,
        specks: ("#CACBD3", 0.5),
        flow: 1.5,
        roughness: 0.07,
        subsurface: 0.25,
    },
    // Nero Marquina: a charcoal ground, never quite black, flecked paler,
    // with a crisp network of white veins at every scale (Marble016).
    Marble {
        name: "Marble Black",
        tags: &["marble", "stone", "polished", "black", "veined", "nero marquina"],
        ground: &[(0.25, "#262727"), (0.5, "#2E302F"), (0.75, "#383A39")],
        vein: ("#A2AEA8", "#E8F0EC"),
        thread: "#D0D8D4",
        cloud: "#3C3F3E",
        scale: 18.0,
        width: 0.3e-3,
        fine: 0.13e-3,
        veins: 0.5,
        network: 1.0,
        cloud_amount: 0.4,
        patches: 0.5,
        specks: ("#575C5B", 0.9),
        flow: 1.2,
        roughness: 0.05,
        subsurface: 0.08,
    },
    // Verde: near-black serpentine, broken up by clouded green veins
    // (Marble009).
    Marble {
        name: "Marble Verde",
        tags: &["marble", "stone", "polished", "green", "dark", "verde", "serpentine"],
        ground: &[(0.25, "#050706"), (0.5, "#0B0F0E"), (0.75, "#142019")],
        vein: ("#2B463E", "#5E8C7E"),
        thread: "#3F6359",
        cloud: "#22403A",
        scale: 20.0,
        width: 1.2e-3,
        fine: 0.45e-3,
        veins: 0.9,
        network: 1.0,
        cloud_amount: 1.0,
        patches: 0.9,
        specks: ("#3B5A51", 0.6),
        flow: 1.4,
        roughness: 0.06,
        subsurface: 0.05,
    },
    // A greige limestone marble, Botticino's quieter cousin: an even ground
    // with only hairline veins in a rusty brown, honed (Marble024).
    Marble {
        name: "Marble Taupe",
        tags: &["marble", "stone", "honed", "beige", "greige", "taupe"],
        ground: &[(0.25, "#8B8178"), (0.5, "#8F857B"), (0.75, "#93897F")],
        vein: ("#857565", "#73604F"),
        thread: "#867869",
        cloud: "#897D72",
        scale: 14.0,
        width: 0.3e-3,
        fine: 0.1e-3,
        veins: 0.4,
        network: 0.7,
        cloud_amount: 0.2,
        patches: 0.15,
        specks: ("#7E746A", 0.2),
        flow: 1.3,
        roughness: 0.16,
        subsurface: 0.2,
    },
];

pub fn all() -> Vec<Preset> {
    let mut out: Vec<Preset> = MARBLES.iter().map(marble).collect();
    out.push(travertine());
    out
}

/// `v * reach + to`, per component.
fn nudge(r: &mut Recipe, v: Out, reach: f32, to: Out) -> Out {
    r.node("vector_math").set("operation", "multiply_add").w("Vector", v).v("Vector 2", [reach; 3]).w("Vector 3", to).out("Vector")
}

fn shifted(r: &mut Recipe, v: Out, by: [f32; 3]) -> Out {
    r.node("vector_math").set("operation", "add").w("Vector", v).v("Vector 2", by).out("Vector")
}

/// 1 along the line where a noise `scale` features a metre crosses its
/// middle, about `half_width` metres either side - wider where the noise is
/// flat, narrower where it is steep - falling to 0.
fn level_line(r: &mut Recipe, space: Out, scale: f32, half_width: f32, detail: f32) -> Out {
    let field = r.node("noise_texture").w("Vector", space).f("Scale", scale).f("Detail", detail).f("Roughness", 0.55).out("Fac");
    let off = r.math("subtract", field, 0.5);
    let apart = r.math("absolute", off, 0.0);
    // The noise climbs about 0.6 of its range over a feature.
    let width = half_width * scale * 0.6;
    r.smooth(apart, (0.0, width), (1.0, 0.0))
}

pub fn marble(m: &Marble) -> Preset {
    let middle = srgb(m.ground[m.ground.len() / 2].1);
    let mut r = Recipe::new(base(MaterialType::Translucent, |mat| {
        mat.base_color = middle;
        mat.subsurface_color = middle;
        mat.subsurface_weight = m.subsurface;
        mat.specular_roughness = m.roughness;
    }));
    let s = m.scale;
    let coords = r.node("texture_coordinate").out("Object");
    let flowed = r.node("mapping").w("Vector", coords).v("Scale", [1.0 / m.flow, 1.0, 1.0]).out("Vector");
    // A slow warp, so a cell's straight sides wander as veins do...
    let drift = r.node("noise_texture").w("Vector", flowed).f("Scale", s * 0.7).f("Detail", 4.0).f("Roughness", 0.55).out("Color");
    let wandering = nudge(&mut r, drift, 0.4 / s, flowed);
    // ...and a fine one, so they are jagged rather than smooth.
    let jag = r.node("noise_texture").w("Vector", wandering).f("Scale", s * 16.0).f("Detail", 3.0).f("Roughness", 0.6).out("Color");
    // Kept shallow: a warp steeper than about one stretches a vein into a blob.
    let jagged = nudge(&mut r, jag, 0.01 / s, wandering);
    let rag = r.node("noise_texture").w("Vector", jagged).f("Scale", s * 70.0).f("Detail", 2.0).f("Roughness", 0.6).out("Color");
    let jagged = nudge(&mut r, rag, 0.002 / s, jagged);

    // Main veins: the cells' edges, as wide as the swell field allows
    // there - nothing at all where it falls to zero. Distances are in
    // cells, so widths in metres are scaled by the cells a metre.
    let edge = r.node("voronoi_texture").set("feature", "distance_to_edge").w("Vector", jagged).f("Scale", s).f("Randomness", 1.0).out("Distance");
    let swell_field = r.node("noise_texture").w("Vector", wandering).f("Scale", s * 1.6).f("Detail", 3.0).f("Roughness", 0.5).out("Fac");
    let w = m.width * s;
    let swell = r.range(swell_field, (0.52 - 0.16 * m.veins, 0.72), (0.0, w * 1.8));
    let inside = r.math("subtract", swell, edge);
    let vein = r.smooth(inside, (-w * 0.3, w * 0.6), (0.0, 1.0));
    let strand_field = r.node("noise_texture").w("Vector", jagged).f("Scale", s * 30.0).f("Detail", 3.0).f("Roughness", 0.6).out("Fac");
    let strands = r.range(strand_field, (0.3, 0.62), (0.35, 1.0));
    let vein = r.math("multiply", vein, strands);

    // The stain either side of them, patchy.
    let presence = r.range(swell_field, (0.32, 0.6), (0.0, 1.0));
    let halo = r.smooth(edge, (0.0, w * 12.0), (1.0, 0.0));
    let blot_field = r.node("noise_texture").w("Vector", jagged).f("Scale", s * 5.0).f("Detail", 5.0).f("Roughness", 0.6).out("Fac");
    let blot = r.range(blot_field, (0.4, 0.66), (0.0, m.cloud_amount));
    let stain = r.math("multiply", halo, presence);
    let stain = r.math("multiply", stain, blot);
    // And blotches of it anywhere, coarse and fine: the stone's impurities.
    let patch_field = r.node("noise_texture").w("Vector", jagged).f("Scale", s * 7.0).f("Detail", 6.0).f("Roughness", 0.65).f("Distortion", 0.6).out("Fac");
    let patch = r.smooth(patch_field, (0.6, 0.635), (0.0, m.patches));
    let fleck_field = r.node("noise_texture").w("Vector", jagged).f("Scale", s * 26.0).f("Detail", 4.0).f("Roughness", 0.6).out("Fac");
    let fleck = r.smooth(fleck_field, (0.64, 0.67), (0.0, m.patches * 0.8));
    let stain = r.math("maximum", stain, patch);
    let stain = r.math("maximum", stain, fleck);

    // The fine network: not cells - a dense Voronoi network reads as a
    // crazed glaze - but the level lines of deep noises, which wander,
    // branch and end as veins do, and swell where the field is flat.
    let fine_scale = s * 4.0;
    let drawn = r.node("mapping").w("Vector", jagged).v("Scale", [0.5, 1.0, 1.0]).out("Vector");
    let fine_space = shifted(&mut r, drawn, [17.3, 5.1, 11.7]);
    let fine_a = level_line(&mut r, fine_space, fine_scale, m.fine, 5.0);
    let other_space = shifted(&mut r, drawn, [-6.2, 13.9, 30.4]);
    let fine_b = level_line(&mut r, other_space, fine_scale * 1.7, m.fine * 0.8, 4.0);
    let lines = r.math("maximum", fine_a, fine_b);
    let fine_swell_space = shifted(&mut r, wandering, [3.7, 9.2, 1.3]);
    let fine_swell_field = r.node("noise_texture").w("Vector", fine_swell_space).f("Scale", s * 3.0).f("Detail", 2.0).out("Fac");
    let fine_presence = r.smooth(fine_swell_field, (0.5 - 0.22 * m.network, 0.64), (0.0, m.network.min(1.0)));
    let thread = r.math("multiply", lines, fine_presence);
    let thread = r.math("multiply", thread, strands);

    // Hairlines: finer again, fainter, here and there.
    let hair_space = shifted(&mut r, jagged, [-8.1, 21.4, -4.6]);
    let hair_line = level_line(&mut r, hair_space, fine_scale * 3.5, m.fine * 0.5, 3.0);
    let hair_bits = r.node("noise_texture").w("Vector", hair_space).f("Scale", s * 14.0).f("Detail", 3.0).out("Fac");
    let hair_presence = r.smooth(hair_bits, (0.56 - 0.14 * m.network, 0.66), (0.0, m.network.min(1.0) * 0.6));
    let hair = r.math("multiply", hair_line, hair_presence);

    // The stone: a slow mottle and a finer one, its crystals each a shade
    // of their own, and the odd speck of another mineral.
    let mottle = r.node("noise_texture").w("Vector", wandering).f("Scale", s * 2.4).f("Detail", 6.0).f("Roughness", 0.6).out("Fac");
    let stops: Vec<_> = m.ground.iter().map(|(at, hex)| (*at, srgb(hex))).collect();
    let ground = r.ramp(mottle, Linear, &stops);
    let fine_mottle = r.node("noise_texture").w("Vector", jagged).f("Scale", s * 9.0).f("Detail", 5.0).f("Roughness", 0.6).out("Fac");
    let fine_tone = r.range(fine_mottle, (0.3, 0.7), (0.955, 1.045));
    let ground = r.mix("multiply", 1.0, ground, fine_tone);
    let crystals = r.node("voronoi_texture").w("Vector", coords).f("Scale", 1400.0).f("Randomness", 1.0).add();
    let crystal_pick = r.node("separate_color").w("Color", Out(crystals, 1)).out("Red");
    let crystal_tone = r.range(crystal_pick, (0.0, 1.0), (0.965, 1.035));
    let ground = r.mix("multiply", 1.0, ground, crystal_tone);
    let specks = r.node("voronoi_texture").w("Vector", jagged).f("Scale", 600.0).f("Randomness", 1.0).add();
    let speck_pick = r.node("separate_color").w("Color", Out(specks, 1)).out("Green");
    let speck_kept = r.smooth(speck_pick, (1.0 - m.specks.1 * 0.6, 1.0 - m.specks.1 * 0.6 + 0.01), (0.0, 1.0));
    let speck_dot = r.smooth(Out(specks, 0), (0.06, 0.22), (0.85, 0.0));
    let speck = r.math("multiply", speck_kept, speck_dot);
    let ground = r.node("mix").f("Factor", speck).w("A", ground).c("B", srgb(m.specks.0)).out("Result");

    let cloud = r.node("mix").f("Factor", stain).w("A", ground).c("B", srgb(m.cloud)).out("Result");
    let with_hair = r.node("mix").f("Factor", hair).w("A", cloud).c("B", srgb(m.thread)).out("Result");
    let with_threads = r.node("mix").f("Factor", thread).w("A", with_hair).c("B", srgb(m.thread)).out("Result");
    let vein_tone = noise(&mut r, s * 4.0, 3.0, 0.5);
    let vein_strength = r.range(vein_tone, (0.35, 0.65), (0.0, 1.0));
    let vein_colour = r.node("mix").f("Factor", vein_strength).c("A", srgb(m.vein.0)).c("B", srgb(m.vein.1)).out("Result");
    let colour = r.node("mix").f("Factor", vein).w("A", with_threads).w("B", vein_colour).out("Result");
    r.to(Param::BaseColor, colour);
    r.to(Param::SubsurfaceColor, colour);

    // Polished, the veins a touch softer than the stone round them and
    // dipping a little where the polish took them down.
    let soft = r.math("add", vein, thread);
    let rough = r.node("math").set("operation", "multiply_add").w("Value", soft).f("Value 2", 0.05).f("Value 3", m.roughness).out("Value");
    r.to(Param::SpecularRoughness, rough);
    let dip = r.math("multiply", soft, -1.0);
    r.bump(dip, 3e-6);
    preset(m.name, "Stone", m.tags, MaterialType::Translucent, r)
}

/// Travertine, cut along its bedding: band upon band of deposit, wavy,
/// each a little paler or darker, with the long thin voids between layers
/// filled and honed (Travertine009).
fn travertine() -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = srgb("#DECBAD");
        m.base_diffuse_roughness = 0.3;
        m.specular_roughness = 0.28;
    }));
    let coords = r.node("texture_coordinate").out("Object");
    let warp = r.node("noise_texture").w("Vector", coords).f("Scale", 12.0).f("Detail", 3.0).out("Color");
    let warped = nudge(&mut r, warp, 0.006, coords);
    // Layers are planes: they change across y only, drawn out along x and z.
    let layered = r.node("mapping").w("Vector", warped).v("Scale", [0.025, 1.0, 0.025]).out("Vector");
    let bands = r.node("noise_texture").w("Vector", layered).f("Scale", 340.0).f("Detail", 8.0).f("Roughness", 0.66).f("Distortion", 0.0).out("Fac");
    let fine_layers = r.node("mapping").w("Vector", warped).v("Scale", [0.02, 1.0, 0.02]).out("Vector");
    let fine = r.node("noise_texture").w("Vector", fine_layers).f("Scale", 1100.0).f("Detail", 4.0).f("Roughness", 0.6).out("Fac");
    let tone = r.node("math").set("operation", "multiply_add").w("Value", fine).f("Value 2", 0.35).w("Value 3", bands).out("Value");
    let colour = r.ramp(tone, Linear, &[
        (0.42, srgb("#C2AA86")),
        (0.56, srgb("#D1BC9B")),
        (0.68, srgb("#DDCAAC")),
        (0.8, srgb("#E6D4B6")),
        (0.92, srgb("#F0DEC0")),
    ]);
    // The pale lines where a layer's top was: thin and bright, broken.
    let seam_layers = r.node("mapping").w("Vector", warped).v("Scale", [0.025, 1.0, 0.025]).out("Vector");
    let seam_field = r.node("noise_texture").w("Vector", seam_layers).f("Scale", 420.0).f("Detail", 3.0).out("Fac");
    let seam_off = r.math("subtract", seam_field, 0.5);
    let seam_apart = r.math("absolute", seam_off, 0.0);
    let seam = r.smooth(seam_apart, (0.0, 0.018), (1.0, 0.0));
    let seam_presence = noise(&mut r, 12.0, 2.0, 0.5);
    let seam_kept = r.range(seam_presence, (0.4, 0.6), (0.0, 0.8));
    let seam = r.math("multiply", seam, seam_kept);
    let colour = r.node("mix").f("Factor", seam).w("A", colour).c("B", srgb("#F4E4C6")).out("Result");
    // The filled voids: long, thin and a shade darker, slightly dull.
    let void_space = r.node("mapping").w("Vector", warped).v("Scale", [0.15, 1.0, 0.15]).out("Vector");
    let voids = r.node("voronoi_texture").w("Vector", void_space).f("Scale", 300.0).f("Randomness", 1.0).add();
    let void_pick = r.node("separate_color").w("Color", Out(voids, 1)).out("Red");
    let void_kept = r.smooth(void_pick, (0.86, 0.9), (0.0, 1.0));
    let void_shape = r.smooth(Out(voids, 0), (0.08, 0.2), (1.0, 0.0));
    let void = r.math("multiply", void_kept, void_shape);
    let colour = r.node("mix").f("Factor", void).w("A", colour).c("B", srgb("#C4AE8C")).out("Result");
    r.to(Param::BaseColor, colour);
    let rough = r.range(void, (0.0, 1.0), (0.28, 0.5));
    r.to(Param::SpecularRoughness, rough);
    let dip = r.math("multiply", void, -1.0);
    let height = r.node("math").set("operation", "multiply_add").w("Value", fine).f("Value 2", 0.15).w("Value 3", dip).out("Value");
    r.bump(height, 3e-5);
    preset("Travertine", "Stone", &["travertine", "stone", "honed", "beige", "vein cut", "limestone"], MaterialType::Generic, r)
}
