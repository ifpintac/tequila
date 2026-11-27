//! Worn finishes: the library's familiar surfaces after use - scratched,
//! swirled, scuffed, dusty, fingerprinted - made with the wear patterns
//! (`tequila_texture::wear`), which need no UVs.
//!
//! Each starts from the clean finish it wears and adds what use leaves on
//! it, as a person would put the wear patterns on one of their own: the
//! pattern on the roughness, where wear shows first in a highlight, then on
//! the colour and the bump where it is deep enough to.

use crate::finish::{brush_lines, grooves_along_axis};
use crate::{base, metal_ior, noise, pattern, plus, preset, rgb, srgb, wear, Preset, Recipe};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::{MaterialType, Param};
use tequila_texture::procedural::Procedural;
use tequila_texture::wear::{Dust, EdgeWear, ScratchDirection, Scratches, Scuffs, Fingerprints};

/// Brushed aluminium, then handled: the brushing's fine lines and streaks
/// as the clean preset has them, crossed by scratches every way - each a
/// sunken groove a few microns deep, rougher than the metal round it - and
/// the finer hairlines among them.
fn aluminium_brushed_scratched() -> Preset {
    let tint = rgb(0.9734, 0.9734, 0.9734);
    let roughness = 0.28;
    let mut r = Recipe::new(base(MaterialType::Anisotropic, |m| {
        m.metal_ior = metal_ior("Aluminium");
        m.base_color = tint;
        m.specular_roughness = roughness;
        m.specular_roughness_anisotropy = 0.92;
    }));
    let grooves = brush_lines(&mut r, 1800.0, 60.0);
    let along = r.stretched([0.03, 1.0, 1.0]);
    let streaks = r.node("noise_texture").w("Vector", along).f("Scale", 600.0).f("Detail", 3.0).out("Fac");
    let both = plus(&mut r, grooves, 0.6, streaks);
    let scratches = pattern(
        &mut r,
        Procedural::WearScratches(Scratches { frequency: 28.0, density: 0.6, width: 0.004, length: 0.85, bend: 0.25, wobble: 0.25, layers: 2, ..Scratches::default() }),
        3,
    );
    // The brushing raises, the scratches sink: one height, the brushing a
    // quarter as deep.
    let sunk = r.math("multiply", scratches, -1.0);
    let height = plus(&mut r, grooves, 0.25, sunk);
    r.bump(height, 6e-6);
    let brushed = r.range(both, (0.45, 1.15), (roughness * 0.8, roughness * 1.25));
    let gloss = plus(&mut r, scratches, 0.22, brushed);
    r.to(Param::SpecularRoughness, gloss);
    let shade = r.range(both, (0.45, 1.15), (0.86, 1.04));
    let colour = r.node("mix").set("blend_type", "multiply").f("Factor", 1.0).c("A", tint).w("B", shade).out("Result");
    r.to(Param::BaseColor, colour);
    grooves_along_axis(&mut r, [1.0, 0.0, 0.0]);
    preset("Aluminium Brushed Scratched", "Metal", &["aluminium", "brushed", "scratched", "worn", "used", "wear"], MaterialType::Anisotropic, r)
}

/// Chrome after years of polishing with the wrong cloth: a mirror, with the
/// fine swirl marks a polishing pad leaves - arcs that ring a highlight in
/// circles.
fn chrome_swirl_marks() -> Preset {
    let mut r = Recipe::new(base(MaterialType::Metal, |m| {
        m.metal_ior = metal_ior("Chromium");
        m.base_color = Spectrum::ONE;
        m.specular_roughness = 0.02;
    }));
    let gloss = wear(&mut r, 0.012, 0.035, 22.0);
    let swirls = pattern(
        &mut r,
        Procedural::WearScratches(Scratches {
            frequency: 140.0,
            density: 1.0,
            length: 0.9,
            length_variation: 0.4,
            width: 0.0025,
            taper: 0.5,
            fade: 0.45,
            wobble: 0.1,
            direction: ScratchDirection::Swirl,
            spread: 0.12,
            depth_variation: 0.7,
            layers: 2,
            ..Scratches::default()
        }),
        5,
    );
    let sunk = r.math("multiply", swirls, -1.0);
    r.bump(sunk, 8e-7);
    let gloss = plus(&mut r, swirls, 0.12, gloss);
    r.to(Param::SpecularRoughness, gloss);
    preset("Chrome Swirl Marks", "Metal", &["chrome", "mirror", "swirl", "polishing marks", "holograms", "worn", "wear"], MaterialType::Metal, r)
}

/// Black ABS after a hard life: scuffed grey and dull where it rubbed,
/// scratched pale here and there, and its edges worn smooth and a little
/// lighter by hands.
fn abs_black_worn() -> Preset {
    let colour = rgb(0.0092, 0.0092, 0.0102);
    let mut r = Recipe::new(base(MaterialType::Plastic, |m| {
        m.base_color = colour;
        m.specular_roughness = 0.3;
        m.specular_ior = 1.54;
    }));
    let gloss = wear(&mut r, 0.25, 0.34, 30.0);
    let scuffs = pattern(&mut r, Procedural::Scuffs(Scuffs { frequency: 14.0, coverage: 0.5, stretch: 0.6, streaks: 0.55, softness: 0.55 }), 2);
    let scratches = pattern(
        &mut r,
        Procedural::WearScratches(Scratches { frequency: 34.0, density: 0.45, width: 0.005, depth_variation: 0.7, layers: 2, ..Scratches::default() }),
        4,
    );
    let edges = pattern(&mut r, Procedural::EdgeWear(EdgeWear { radius: 0.0015, amount: 0.45, breakup: 0.7, frequency: 260.0, softness: 0.35 }), 6);
    // Scuffs and scratches dull it, worn edges polish it.
    let dulled = plus(&mut r, scuffs, 0.3, gloss);
    let dulled = plus(&mut r, scratches, 0.25, dulled);
    let gloss = plus(&mut r, edges, -0.12, dulled);
    r.to(Param::SpecularRoughness, gloss);
    // And both lighten it: abraded plastic scatters, a grey bloom.
    let marks = plus(&mut r, scratches, 0.8, scuffs);
    let marks = plus(&mut r, edges, 0.5, marks);
    let worn = r.node("mix").set("blend_type", "mix").w("Factor", marks).c("A", colour).c("B", srgb("#4A4A4C")).out("Result");
    r.to(Param::BaseColor, worn);
    let grain = noise(&mut r, 2600.0, 2.0, 0.5);
    let sunk = r.math("multiply", scratches, -1.5);
    let height = plus(&mut r, grain, 1.0, sunk);
    r.bump(height, 3e-6);
    preset("ABS Black Worn", "Plastic", &["abs", "black", "worn", "scuffed", "scratched", "used", "wear"], MaterialType::Plastic, r)
}

/// A black car finish left in a garage: dust settled on everything facing
/// up, a soft film in drifts with grains in it, dulling the clear coat and
/// greying the colour under it - less on the sides, none underneath.
fn automotive_black_dusty() -> Preset {
    let colour = rgb(0.0081, 0.0081, 0.01);
    let mut r = Recipe::new(base(MaterialType::Paint, |m| {
        m.base_color = colour;
        m.coat_roughness = 0.02;
    }));
    crate::orange_peel(&mut r, 520.0, 5e-6);
    let dust = pattern(&mut r, Procedural::Dust(Dust { frequency: 18.0, amount: 0.55, patchiness: 0.55, specks: 0.6, speck_frequency: 900.0, up_bias: 0.85 }), 8);
    let coat = r.range(dust, (0.0, 0.8), (0.02, 0.55));
    r.to(Param::CoatRoughness, coat);
    let dusted = r.node("mix").set("blend_type", "mix").w("Factor", dust).c("A", colour).c("B", srgb("#9A958A")).out("Result");
    r.to(Param::BaseColor, dusted);
    preset("Automotive Black Dusty", "Paint", &["car", "gloss", "clearcoat", "black", "dust", "dusty", "garage", "worn", "wear"], MaterialType::Paint, r)
}

/// A switched-off phone or tablet after it has been held: black cover
/// glass, its mirror hazed by fingerprints - each with its ridges - and a
/// wiped smear or two.
fn display_glass_fingerprinted() -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = rgb(0.003, 0.003, 0.0035);
        m.specular_ior = 1.52;
        m.specular_roughness = 0.02;
    }));
    let oil = pattern(&mut r, Procedural::Fingerprints(Fingerprints { frequency: 22.0, prints: 0.6, smears: 0.35, ridges: 0.75, softness: 0.4 }), 9);
    let gloss = r.range(oil, (0.0, 1.0), (0.02, 0.3));
    r.to(Param::SpecularRoughness, gloss);
    preset(
        "Display Glass Black Fingerprinted",
        "Glass",
        &["display", "screen", "off", "black glass", "cover glass", "fingerprints", "smudges", "used", "wear"],
        MaterialType::Generic,
        r,
    )
}

pub fn all() -> Vec<Preset> {
    vec![aluminium_brushed_scratched(), chrome_swirl_marks(), abs_black_worn(), automotive_black_dusty(), display_glass_fingerprinted()]
}
