//! Granite, measured against CC0 scans (ambientCG Granite001A, Granite003A,
//! Granite005A, Poly Haven granite_tile).
//!
//! A granite is a mosaic of mineral grains a few millimetres across, set
//! hard against one another with angular, irregular outlines: feldspar
//! (white, cream, salmon or pink), quartz (grey and glassy), and the black
//! micas and hornblende in small flakes. What the scans show besides:
//! - Grains of more than one size, the small ones filling between the big.
//! - Each mineral gathers into loose clots rather than spreading evenly.
//! - Inside a grain the colour is not flat: it clouds a little, and quartz
//!   is darker at its heart where the stone behind shows through.

use crate::recipe::{Constant, Out};
use crate::{base, preset, srgb, Preset, Recipe};
use tequila_materials::material_type::{MaterialType, Param};

pub struct Granite {
    pub name: &'static str,
    pub tags: &'static [&'static str],
    /// The minerals: each one's share of the grains and its colour.
    pub minerals: &'static [(f32, &'static str)],
    /// The dark flakes, mica and hornblende: colour and how many, 0 to 1.
    pub flakes: (&'static str, f32),
    /// A large grain's size, in metres.
    pub grain: f32,
    /// How strongly each mineral gathers into clots, 0 to 1.
    pub clotting: f32,
    pub roughness: f32,
}

const GRANITES: &[Granite] = &[
    // Salt and pepper: white and grey with black flakes, the kitchen
    // worktop granite.
    Granite {
        name: "Granite Polished",
        tags: &["granite", "stone", "polished", "speckled", "countertop", "salt and pepper", "grey"],
        minerals: &[(0.38, "#C9C7C2"), (0.22, "#DAD8D3"), (0.26, "#9C9A97"), (0.14, "#7C7A78")],
        flakes: ("#262524", 0.75),
        grain: 1.5e-3,
        clotting: 0.25,
        roughness: 0.06,
    },
    // Pink: salmon feldspar, grey quartz, black mica (Granite001A).
    Granite {
        name: "Granite Rosa",
        tags: &["granite", "stone", "polished", "pink", "salmon", "rosa", "countertop"],
        minerals: &[(0.3, "#A98E7C"), (0.2, "#BCA493"), (0.2, "#D2C8BE"), (0.2, "#8F8A86"), (0.1, "#726C68")],
        flakes: ("#332D2A", 0.75),
        grain: 1.8e-3,
        clotting: 0.3,
        roughness: 0.07,
    },
    // Blue-grey with blue feldspar flecks (Granite005A).
    Granite {
        name: "Granite Blue Grey",
        tags: &["granite", "stone", "polished", "blue", "grey", "countertop"],
        minerals: &[(0.42, "#A49D98"), (0.26, "#BDB5AC"), (0.2, "#88858C"), (0.08, "#7A7F91"), (0.04, "#9095A6")],
        flakes: ("#34333B", 0.7),
        grain: 1.3e-3,
        clotting: 0.25,
        roughness: 0.07,
    },
    // Rust and blue-grey in clots on white, a honed rustic granite
    // (Granite003A).
    Granite {
        name: "Granite Rustic",
        tags: &["granite", "stone", "honed", "rust", "brown", "grey", "rustic"],
        minerals: &[(0.32, "#C3C2C5"), (0.2, "#D9D8DB"), (0.2, "#9D806E"), (0.14, "#8E8F96"), (0.06, "#7E8699"), (0.08, "#5F534B")],
        flakes: ("#332E2B", 0.7),
        grain: 1.7e-3,
        clotting: 0.45,
        roughness: 0.22,
    },
    // Absolute black: fine and even, the faintest grain in it.
    Granite {
        name: "Granite Absolute Black",
        tags: &["granite", "stone", "polished", "black", "absolute black", "countertop"],
        minerals: &[(0.6, "#141414"), (0.25, "#1C1C1D"), (0.15, "#2A2A2C")],
        flakes: ("#3A3A3D", 0.35),
        grain: 0.9e-3,
        clotting: 0.15,
        roughness: 0.05,
    },
];

pub fn all() -> Vec<Preset> {
    GRANITES.iter().map(granite).collect()
}

pub fn granite(g: &Granite) -> Preset {
    let first = srgb(g.minerals[0].1);
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = first;
        m.specular_roughness = g.roughness;
        m.base_diffuse_roughness = 0.2;
    }));
    let per_metre = 1.0 / g.grain;
    let coords = r.node("texture_coordinate").out("Object");
    // Angular, irregular outlines: cells whose straight sides a small warp
    // breaks up.
    let nick = r.node("noise_texture").w("Vector", coords).f("Scale", per_metre * 0.8).f("Detail", 3.0).f("Roughness", 0.6).out("Color");
    let warped = r.node("vector_math").set("operation", "multiply_add").w("Vector", nick).v("Vector 2", [g.grain * 0.45; 3]).w("Vector 3", coords).out("Vector");

    // Large grains, and small ones filling between them here and there.
    let large = r.node("voronoi_texture").w("Vector", warped).f("Scale", per_metre).f("Randomness", 1.0).add();
    let small = r.node("voronoi_texture").w("Vector", warped).f("Scale", per_metre * 2.4).f("Randomness", 1.0).add();
    let large_pick = r.node("separate_color").w("Color", Out(large, 1)).out("Red");
    let small_pick = r.node("separate_color").w("Color", Out(small, 1)).out("Red");
    let large_tone = r.node("separate_color").w("Color", Out(large, 1)).out("Green");
    let small_tone = r.node("separate_color").w("Color", Out(small, 1)).out("Green");
    let fill_field = r.node("noise_texture").w("Vector", coords).f("Scale", per_metre * 0.35).f("Detail", 3.0).out("Fac");
    let use_small = r.smooth(fill_field, (0.52, 0.56), (0.0, 1.0));
    let pick = r.mix_float(use_small, large_pick, small_pick);
    let tone = r.mix_float(use_small, large_tone, small_tone);

    // Clots: the pick drifts with a slow field, so neighbours tend to be
    // the same mineral.
    let clot_field = r.node("noise_texture").w("Vector", coords).f("Scale", 40.0).f("Detail", 5.0).f("Roughness", 0.6).out("Fac");
    let clot = r.range(clot_field, (0.25, 0.75), (0.0, 1.0));
    let pick = r.mix_float(g.clotting, pick, clot);
    // The clot field bunches near its middle; spread the pick back over the
    // whole range so the shares hold.
    let pick = r.smooth(pick, (0.08 * g.clotting, 1.0 - 0.08 * g.clotting), (0.0, 1.0));

    let mut stops = Vec::new();
    let mut at = 0.0;
    for (share, hex) in g.minerals {
        stops.push((at, srgb(hex)));
        at += share;
    }
    let mineral = r.ramp(pick, Constant, &stops);
    let grain_tone = r.range(tone, (0.0, 1.0), (0.86, 1.12));
    let mineral = r.mix("multiply", 1.0, mineral, grain_tone);
    // Clouding inside each grain.
    let cloud = r.node("noise_texture").w("Vector", coords).f("Scale", per_metre * 3.0).f("Detail", 3.0).f("Roughness", 0.6).out("Fac");
    let cloud_tone = r.range(cloud, (0.3, 0.7), (0.88, 1.1));
    let mineral = r.mix("multiply", 1.0, mineral, cloud_tone);
    let fine = r.node("noise_texture").w("Vector", coords).f("Scale", per_metre * 9.0).f("Detail", 2.0).out("Fac");
    let cloud_tone = r.range(fine, (0.3, 0.7), (0.93, 1.06));
    let mineral = r.mix("multiply", 1.0, mineral, cloud_tone);

    // The dark flakes: small, irregular, gathered a little.
    let ragged = r.node("noise_texture").w("Vector", coords).f("Scale", per_metre * 6.0).f("Detail", 2.0).out("Color");
    let flake_space = r.node("vector_math").set("operation", "multiply_add").w("Vector", ragged).v("Vector 2", [g.grain * 0.35; 3]).w("Vector 3", warped).out("Vector");
    let flake_cells = r.node("voronoi_texture").w("Vector", flake_space).f("Scale", per_metre * 2.2).f("Randomness", 1.0).add();
    let flake_pick = r.node("separate_color").w("Color", Out(flake_cells, 1)).out("Blue");
    let flake_gather = r.range(clot_field, (0.3, 0.7), (-0.15, 0.15));
    let flake_pick = r.math("add", flake_pick, flake_gather);
    let flake_kept = r.smooth(flake_pick, (1.0 - g.flakes.1 * 0.45, 1.0 - g.flakes.1 * 0.45 + 0.02), (0.0, 1.0));
    let flake_size = r.node("separate_color").w("Color", Out(flake_cells, 1)).out("Green");
    let flake_reach = r.range(flake_size, (0.0, 1.0), (0.12, 0.4));
    let flake_inside = r.math("subtract", flake_reach, Out(flake_cells, 0));
    let flake_shape = r.smooth(flake_inside, (0.0, 0.05), (0.0, 1.0));
    let flake = r.math("multiply", flake_kept, flake_shape);
    let colour = r.node("mix").f("Factor", flake).w("A", mineral).c("B", srgb(g.flakes.0)).out("Result");
    r.to(Param::BaseColor, colour);

    // Polished: mica takes the polish less well, and the grain boundaries
    // hold the odd fine pit.
    let rough = r.range(flake, (0.0, 1.0), (g.roughness, (g.roughness * 3.0).min(0.6)));
    r.to(Param::SpecularRoughness, rough);
    let edge = r.node("voronoi_texture").set("feature", "distance_to_edge").w("Vector", warped).f("Scale", per_metre).f("Randomness", 1.0).out("Distance");
    let seam = r.smooth(edge, (0.0, 0.03), (-1.0, 0.0));
    let height = r.node("math").set("operation", "multiply_add").w("Value", flake).f("Value 2", -0.5).w("Value 3", seam).out("Value");
    r.bump(height, 4e-6);
    preset(g.name, "Stone", g.tags, MaterialType::Generic, r)
}
