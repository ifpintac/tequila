//! Leather beyond the pebbled hide: smooth and nappa, where the grain is
//! the animal's own follicles and creases and nothing is pressed into it;
//! patent, under its gloss coat; Saffiano, cross-hatched in the press;
//! perforated, as a seat or an ear cushion is; and the coated cloth that
//! stands in for leather on an ear pad.

use crate::finish::{staggered, triplanar, Hole};
use crate::recipe::Out;
use crate::{base, noise, plus, preset, rgb, srgb, wear, Preset, Recipe};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::{MaterialType, Param};

/// What every hide has under whatever is done to it: follicles a tenth
/// of a millimetre across, creases wandering a centimetre apart, and a
/// dye that never took quite evenly. Returns the height of the grain and
/// the colour's shade.
fn hide(r: &mut Recipe, creases: f32) -> (Out, Out) {
    let pores = r.node("voronoi_texture").f("Scale", 5200.0).f("Randomness", 1.0).out("Distance");
    let pits = r.smooth(pores, (0.0, 0.3), (-1.0, 0.0));
    let field = r.node("noise_texture").f("Scale", 45.0).f("Detail", 3.0).f("Roughness", 0.5).f("Distortion", 0.4).out("Fac");
    let off = r.math("subtract", field, 0.5);
    let apart = r.math("absolute", off, 0.0);
    let crease = r.smooth(apart, (0.0, 0.04), (-1.0, 0.0));
    let fine = noise(r, 1400.0, 3.0, 0.6);
    let height = plus(r, crease, creases, pits);
    let height = plus(r, fine, 0.5, height);
    let dye = noise(r, 35.0, 3.0, 0.5);
    let mottle = r.range(dye, (0.3, 0.7), (0.9, 1.03));
    // Barely darker in a crease: a line drawn on it is a crack, not a fold.
    let in_crease = r.range(crease, (-1.0, 0.0), (0.95, 1.0));
    let shade = r.math("multiply", mottle, in_crease);
    (height, shade)
}

/// A smooth leather: `gloss` from the most polished of it to the least,
/// `depth` how deep its grain is, metres. `patent` puts it under a gloss
/// coat.
fn smooth(name: &'static str, tags: &'static [&'static str], colour: Spectrum, gloss: (f32, f32), depth: f32, patent: bool) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = colour;
        m.base_diffuse_roughness = 0.5;
        m.specular_ior = 1.5;
        m.specular_roughness = (gloss.0 + gloss.1) * 0.5;
        m.fuzz_weight = 0.05;
        m.fuzz_color = rgb(1.0, 1.0, 1.0);
        m.fuzz_roughness = 0.5;
        if patent {
            // Under the coat the leather has no shine of its own.
            m.specular_weight = 0.0;
            m.fuzz_weight = 0.0;
            m.coat_weight = 1.0;
            m.coat_roughness = 0.03;
        }
    }));
    let (height, shade) = hide(&mut r, if patent { 0.25 } else { 0.5 });
    r.bump(height, depth);
    let toned = r.node("mix").set("blend_type", "multiply").f("Factor", 1.0).c("A", colour).w("B", shade).out("Result");
    r.to(Param::BaseColor, toned);
    // Handled: polished where a hand goes, duller between.
    let sheen = wear(&mut r, gloss.0, gloss.1, 18.0);
    r.to(Param::SpecularRoughness, sheen);
    preset(name, "Leather", tags, MaterialType::Generic, r)
}

/// Saffiano: a fine cross-hatch pressed into the leather and sealed, so
/// it is ridges at two slants with a waxy shine on their tops.
fn saffiano(name: &'static str, tags: &'static [&'static str], colour: Spectrum) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = colour;
        m.base_diffuse_roughness = 0.4;
        m.specular_ior = 1.5;
        m.specular_roughness = 0.36;
    }));
    let pitch = 0.0011;
    let h = triplanar(&mut r, 8.0, |r, u, v, _| {
        let mut ridges = Vec::new();
        for sign in [1.0, -1.0] {
            // Along one slant: 0 in a groove, 1 on a ridge.
            let across = r.math3("multiply_add", v, 0.55 * sign, u);
            let t = r.math("divide", across, pitch);
            let f = r.math("fraction", t, 0.0);
            let centred = r.math("subtract", f, 0.5);
            let away = r.math("absolute", centred, 0.0);
            ridges.push(r.smooth(away, (0.12, 0.46), (1.0, 0.0)));
        }
        vec![r.math("minimum", ridges[0], ridges[1])]
    })[0];
    r.bump(h, 9e-5);
    let sheen = r.range(h, (0.0, 1.0), (0.5, 0.26));
    r.to(Param::SpecularRoughness, sheen);
    let shade = r.range(h, (0.0, 1.0), (0.84, 1.0));
    let toned = r.node("mix").set("blend_type", "multiply").f("Factor", 1.0).c("A", colour).w("B", shade).out("Result");
    r.to(Param::BaseColor, toned);
    preset(name, "Leather", tags, MaterialType::Generic, r)
}

/// Perforated: a smooth leather punched with rows of small holes, each a
/// dark pit with its rim pulled down.
fn perforated(name: &'static str, tags: &'static [&'static str], colour: Spectrum) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = colour;
        m.base_diffuse_roughness = 0.5;
        m.specular_ior = 1.5;
        m.specular_roughness = 0.4;
        m.fuzz_weight = 0.05;
        m.fuzz_color = rgb(1.0, 1.0, 1.0);
        m.fuzz_roughness = 0.5;
    }));
    let (pitch, radius) = (0.0045, 0.0006);
    let solid = triplanar(&mut r, 8.0, |r, u, v, _| {
        let d = staggered(r, u, v, pitch, Hole::Round);
        vec![r.smooth(d, (radius * 0.7, radius * 1.5), (0.0, 1.0))]
    })[0];
    let (grain, shade) = hide(&mut r, 0.3);
    // A hole is a millimetre deep to the eye; the grain is a tenth of that.
    let height = plus(&mut r, grain, 0.08, solid);
    r.bump(height, 6e-4);
    let shade = r.math("multiply", shade, solid);
    let toned = r.node("mix").set("blend_type", "multiply").f("Factor", 1.0).c("A", colour).w("B", shade).out("Result");
    r.to(Param::BaseColor, toned);
    let sheen = wear(&mut r, 0.34, 0.46, 18.0);
    r.to(Param::SpecularRoughness, sheen);
    preset(name, "Leather", tags, MaterialType::Generic, r)
}

pub fn all() -> Vec<Preset> {
    vec![
        smooth("Leather Smooth Black", &["leather", "smooth", "full grain", "calf", "black"], srgb("#161514"), (0.3, 0.44), 2.2e-5, false),
        smooth("Leather Smooth Brown", &["leather", "smooth", "full grain", "calf", "brown"], srgb("#5A3420"), (0.3, 0.44), 2.2e-5, false),
        smooth("Leather Smooth Tan", &["leather", "smooth", "vegetable tanned", "natural", "tan", "strap"], srgb("#B98654"), (0.34, 0.48), 2.2e-5, false),
        smooth("Leather Nappa Black", &["leather", "nappa", "soft", "smooth", "headphone", "ear pad", "black"], srgb("#171615"), (0.42, 0.54), 1.4e-5, false),
        smooth("Leather Nappa Cream", &["leather", "nappa", "soft", "smooth", "cream", "white"], srgb("#DDD3C0"), (0.42, 0.54), 1.4e-5, false),
        smooth("Leather Nappa Tan", &["leather", "nappa", "soft", "smooth", "headphone", "tan"], srgb("#A87448"), (0.42, 0.54), 1.4e-5, false),
        smooth("Leather Patent Black", &["leather", "patent", "gloss", "lacquered", "black"], srgb("#0C0B0B"), (0.3, 0.4), 1.0e-5, true),
        smooth("Leather Patent Red", &["leather", "patent", "gloss", "lacquered", "red"], srgb("#8C1016"), (0.3, 0.4), 1.0e-5, true),
        saffiano("Leather Saffiano Black", &["leather", "saffiano", "cross-hatch", "embossed", "wallet", "case", "black"], srgb("#151414")),
        saffiano("Leather Saffiano Tan", &["leather", "saffiano", "cross-hatch", "embossed", "wallet", "case", "tan"], srgb("#A9744A")),
        perforated("Leather Perforated Black", &["leather", "perforated", "punched", "seat", "ear pad", "headphone", "black"], srgb("#171615")),
        perforated("Leather Perforated Tan", &["leather", "perforated", "punched", "seat", "headphone", "tan"], srgb("#A87448")),
        // The coated cloth an ear pad or a headband is covered in: a
        // finer, shallower grain than a cabinet's vinyl.
        crate::equipment::vinyl("Leatherette Black", &["leatherette", "protein leather", "pu leather", "faux leather", "ear pad", "headband", "headphone", "black"], srgb("#151515"), 1500.0, 2.4e-5),
        crate::equipment::vinyl("Leatherette Grey", &["leatherette", "protein leather", "pu leather", "faux leather", "ear pad", "headband", "headphone", "grey"], srgb("#5F6062"), 1500.0, 2.4e-5),
        crate::equipment::vinyl("Leatherette White", &["leatherette", "protein leather", "pu leather", "faux leather", "ear pad", "headband", "headphone", "white"], srgb("#DEDCD6"), 1500.0, 2.4e-5),
    ]
}
