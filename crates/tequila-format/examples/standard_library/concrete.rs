//! Concrete, measured against CC0 scans (ambientCG Concrete016, Concrete030,
//! Concrete034, Concrete046).
//!
//! What makes cast concrete read as concrete rather than grey noise:
//! - Its tone barely changes - a few percent - but over large, soft clouds
//!   where the pour and the cure went a little differently.
//! - Streaks: the trowel's smears, or the formwork's grain, one way.
//! - Bug holes: sparse pinholes from trapped air, a millimetre or a few,
//!   dark inside, the only sharp thing on the surface.
//! - A sandy grain all over, felt more than seen.

use crate::recipe::{Linear, Out};
use crate::{base, preset, srgb, Preset, Recipe};
use tequila_materials::material_type::{MaterialType, Param};

pub struct Concrete {
    pub name: &'static str,
    pub tags: &'static [&'static str],
    /// The cement, as a ramp over its slow clouding.
    pub tones: &'static [(f32, &'static str)],
    /// How the streaks run: the pattern's scale along x, y and z, small
    /// along the streaks.
    pub streak_axes: [f32; 3],
    /// How strong the streaks are, as a share of the tone.
    pub streaks: f32,
    /// Bug holes, 0 to 1.
    pub pinholes: f32,
    /// Exposed aggregate, ground flat: its colours and how much of the
    /// surface it covers, 0 to 1.
    pub aggregate: (&'static [&'static str], f32),
    pub roughness: (f32, f32),
}

const CONCRETES: &[Concrete] = &[
    // Cast in place, light grey, the trowel's smears across it and the odd
    // bug hole (Concrete034, Concrete016).
    Concrete {
        name: "Concrete",
        tags: &["concrete", "architectural", "cast", "grey"],
        tones: &[(0.3, "#ACACAC"), (0.5, "#B6B6B5"), (0.7, "#C0C0BF")],
        streak_axes: [0.12, 1.0, 1.0],
        streaks: 0.05,
        pinholes: 0.6,
        aggregate: (&[], 0.0),
        roughness: (0.6, 0.8),
    },
    // Dark, poured against boards: vertical streaks (Concrete030).
    Concrete {
        name: "Concrete Dark",
        tags: &["concrete", "architectural", "cast", "dark", "charcoal", "grey"],
        tones: &[(0.3, "#575451"), (0.5, "#615E5A"), (0.7, "#6B6864")],
        streak_axes: [1.0, 0.1, 1.0],
        streaks: 0.08,
        pinholes: 0.4,
        aggregate: (&[], 0.0),
        roughness: (0.55, 0.75),
    },
    // Warm off-white, as a weathered wall has it (Concrete046).
    Concrete {
        name: "Concrete Warm",
        tags: &["concrete", "architectural", "cast", "off-white", "warm", "weathered"],
        tones: &[(0.3, "#C1C2AB"), (0.5, "#CACCB6"), (0.7, "#D3D5C2")],
        streak_axes: [1.0, 0.15, 1.0],
        streaks: 0.04,
        pinholes: 0.8,
        aggregate: (&[], 0.0),
        roughness: (0.65, 0.85),
    },
    // Ground and sealed floor: aggregate showing through, a soft sheen.
    Concrete {
        name: "Concrete Polished",
        tags: &["concrete", "polished", "floor", "aggregate", "grey", "sealed"],
        tones: &[(0.3, "#8E8D8A"), (0.5, "#999895"), (0.7, "#A4A3A0")],
        streak_axes: [0.3, 1.0, 0.3],
        streaks: 0.03,
        pinholes: 0.15,
        aggregate: (&["#8A8580", "#ADA8A0", "#77736E", "#9F988E", "#BDB8B0", "#6A6764"], 0.35),
        roughness: (0.18, 0.32),
    },
];

pub fn all() -> Vec<Preset> {
    CONCRETES.iter().map(concrete).collect()
}

pub fn concrete(c: &Concrete) -> Preset {
    let middle = srgb(c.tones[c.tones.len() / 2].1);
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = middle;
        m.base_diffuse_roughness = 0.6;
        m.specular_roughness = c.roughness.1;
    }));
    let coords = r.node("texture_coordinate").out("Object");

    // The cement's clouding, slow and faint.
    let cloud = r.node("noise_texture").w("Vector", coords).f("Scale", 14.0).f("Detail", 7.0).f("Roughness", 0.62).out("Fac");
    let stops: Vec<_> = c.tones.iter().map(|(at, hex)| (*at, srgb(hex))).collect();
    let colour = r.ramp(cloud, Linear, &stops);
    let patches = r.node("noise_texture").w("Vector", coords).f("Scale", 90.0).f("Detail", 5.0).f("Roughness", 0.6).f("Distortion", 0.8).out("Fac");
    // Where the trowel passed last the cement is a shade paler, with an
    // edge you can follow.
    let patch_tone = r.smooth(patches, (0.5, 0.56), (0.965, 1.04));
    let colour = r.mix("multiply", 1.0, colour, patch_tone);
    // Streaks one way.
    let streaked = r.node("mapping").w("Vector", coords).v("Scale", c.streak_axes).out("Vector");
    let streak_field = r.node("noise_texture").w("Vector", streaked).f("Scale", 70.0).f("Detail", 6.0).f("Roughness", 0.6).out("Fac");
    let streak = r.range(streak_field, (0.3, 0.7), (1.0 - c.streaks, 1.0 + c.streaks));
    let colour = r.mix("multiply", 1.0, colour, streak);
    // Sand: each grain a shade of its own, finer than the eye quite sees.
    let sand = r.node("voronoi_texture").w("Vector", coords).f("Scale", 2600.0).f("Randomness", 1.0).add();
    let sand_pick = r.node("separate_color").w("Color", Out(sand, 1)).out("Red");
    let sand_tone = r.range(sand_pick, (0.0, 1.0), (0.95, 1.05));
    let colour = r.mix("multiply", 1.0, colour, sand_tone);

    // Exposed aggregate, cut flat: stones of every size, sharp-edged.
    let mut colour = colour;
    if c.aggregate.1 > 0.0 {
        let nick = r.node("noise_texture").w("Vector", coords).f("Scale", 400.0).f("Detail", 2.0).out("Color");
        let nicked = r.node("vector_math").set("operation", "multiply_add").w("Vector", nick).v("Vector 2", [0.0012; 3]).w("Vector 3", coords).out("Vector");
        for (scale, keep) in [(320.0, c.aggregate.1 * 0.9), (850.0, c.aggregate.1 * 1.3)] {
            let stones = r.node("voronoi_texture").w("Vector", nicked).f("Scale", scale).f("Randomness", 1.0).add();
            let pick = r.node("separate_color").w("Color", Out(stones, 1)).out("Red");
            let kept = r.smooth(pick, (1.0 - keep, 1.0 - keep + 0.01), (0.0, 1.0));
            let edge = r.node("voronoi_texture").set("feature", "distance_to_edge").w("Vector", nicked).f("Scale", scale).f("Randomness", 1.0).out("Distance");
            let size = r.node("separate_color").w("Color", Out(stones, 1)).out("Green");
            let margin = r.range(size, (0.0, 1.0), (0.04, 0.2));
            let inside = r.math("subtract", edge, margin);
            let stone = r.smooth(inside, (0.0, 0.02), (0.0, 1.0));
            let stone = r.math("multiply", stone, kept);
            let which = r.node("separate_color").w("Color", Out(stones, 1)).out("Blue");
            let stops: Vec<_> = c.aggregate.0.iter().enumerate().map(|(i, hex)| (i as f32 / c.aggregate.0.len() as f32, srgb(hex))).collect();
            let stone_colour = r.ramp(which, crate::recipe::Constant, &stops);
            colour = r.node("mix").f("Factor", stone).w("A", colour).w("B", stone_colour).out("Result");
        }
    }

    // Bug holes: sparse, each its own size, dark inside.
    let holes = r.node("voronoi_texture").w("Vector", coords).f("Scale", 160.0).f("Randomness", 1.0).add();
    let hole_pick = r.node("separate_color").w("Color", Out(holes, 1)).out("Red");
    let hole_kept = r.smooth(hole_pick, (1.0 - c.pinholes * 0.18, 1.0 - c.pinholes * 0.18 + 0.005), (0.0, 1.0));
    let hole_size = r.node("separate_color").w("Color", Out(holes, 1)).out("Green");
    let hole_reach = r.range(hole_size, (0.0, 1.0), (0.04, 0.16));
    let hole_inside = r.math("subtract", hole_reach, Out(holes, 0));
    let hole = r.smooth(hole_inside, (0.0, 0.025), (0.0, 1.0));
    let hole = r.math("multiply", hole, hole_kept);
    let shadowed = r.math("multiply", hole, 0.55);
    let dark = r.range(shadowed, (0.0, 1.0), (1.0, 0.0));
    let colour = r.mix("multiply", 1.0, colour, dark);
    r.to(Param::BaseColor, colour);

    // A little gloss where the trowel passed, none in the holes.
    let gloss_field = r.node("noise_texture").w("Vector", streaked).f("Scale", 30.0).f("Detail", 3.0).out("Fac");
    let gloss = r.smooth(gloss_field, (0.3, 0.7), (c.roughness.0, c.roughness.1));
    let gloss = r.node("math").set("operation", "multiply_add").w("Value", hole).f("Value 2", 0.3).w("Value 3", gloss).out("Value");
    r.to(Param::SpecularRoughness, gloss);
    let grit = r.node("noise_texture").w("Vector", coords).f("Scale", 1800.0).f("Detail", 3.0).f("Roughness", 0.6).out("Fac");
    let undulation = r.node("noise_texture").w("Vector", streaked).f("Scale", 40.0).f("Detail", 3.0).out("Fac");
    let smear = r.node("noise_texture").w("Vector", streaked).f("Scale", 160.0).f("Detail", 6.0).f("Roughness", 0.65).f("Distortion", 1.2).out("Fac");
    let grit = r.math("multiply", grit, 0.12);
    let height = r.node("math").set("operation", "multiply_add").w("Value", undulation).f("Value 2", 0.6).w("Value 3", grit).out("Value");
    let height = r.node("math").set("operation", "multiply_add").w("Value", smear).f("Value 2", 1.0).w("Value 3", height).out("Value");
    let height = r.node("math").set("operation", "multiply_add").w("Value", hole).f("Value 2", -1.5).w("Value 3", height).out("Value");
    r.bump(height, if c.aggregate.1 > 0.0 { 4e-5 } else { 3.5e-4 });
    preset(c.name, "Stone", c.tags, MaterialType::Generic, r)
}
