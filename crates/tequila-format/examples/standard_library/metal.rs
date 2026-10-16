//! Metals: polished, satin, brushed, blasted, cast, beaten, coated, aged.

use crate::finish::{brush_lines, grooves_along_axis};
use crate::plastic::knurl;
use crate::{base, metal_ior, micro, noise, pattern, plus, preset, rgb, srgb, wear, Preset, Recipe};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::{MaterialType, Param};
use tequila_texture::procedural::Procedural;

pub fn metal(ior: &str, tint: Spectrum, roughness: f32) -> tequila_materials::openpbr::Material {
    base(MaterialType::Metal, |m| {
        m.metal_ior = metal_ior(ior);
        m.base_color = tint;
        m.specular_roughness = roughness;
    })
}

/// Hairline scratches, sparse and every way, the marks any handled metal
/// gathers: each a little rougher than the metal around it and the faintest
/// groove, which is all it takes for a highlight to show them.
pub fn hairlines(r: &mut Recipe, gloss: crate::Out, density: f32) -> crate::Out {
    let scratches = pattern(r, Procedural::Scratches { frequency: 110.0, width: 0.012, density, spread: 0.25 }, 11);
    let sunk = r.math("multiply", scratches, -1.0);
    r.bump(sunk, 4e-7);
    plus(r, scratches, 0.12, gloss)
}

/// A polished metal: a mirror, with the faint haze of handling and the
/// odd hairline scratch.
pub fn polished(name: &'static str, tags: &'static [&'static str], ior: &str, tint: Spectrum, low: f32, high: f32) -> Preset {
    let mut r = Recipe::new(metal(ior, tint, low));
    let gloss = wear(&mut r, low, high, 22.0);
    let gloss = hairlines(&mut r, gloss, 0.18);
    r.to(Param::SpecularRoughness, gloss);
    preset(name, "Metal", tags, MaterialType::Metal, r)
}

/// A satin metal: polished then dulled by a fine abrasive, which leaves a
/// tooth too fine to see and a sheen that varies with the hand that did it.
pub fn satin(name: &'static str, tags: &'static [&'static str], ior: &str, tint: Spectrum, low: f32, high: f32) -> Preset {
    let mut r = Recipe::new(metal(ior, tint, (low + high) * 0.5));
    let gloss = wear(&mut r, low, high, 15.0);
    let gloss = hairlines(&mut r, gloss, 0.25);
    r.to(Param::SpecularRoughness, gloss);
    preset(name, "Metal", tags, MaterialType::Metal, r)
}

/// Anodised aluminium: usually bead blasted before it is dyed, so a fine,
/// even satin rather than a polish, the colour in the metal's own
/// reflection.
pub fn anodised(name: &'static str, tags: &'static [&'static str], tint: Spectrum, low: f32, high: f32) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Metal, |m| {
        m.metal_ior = metal_ior("Aluminium");
        m.base_color = tint;
        m.specular_roughness = (low + high) * 0.5;
    }));
    let gloss = wear(&mut r, low, high, 12.0);
    r.to(Param::SpecularRoughness, gloss);
    micro(&mut r, 9000.0, 2.5e-6);
    preset(name, "Metal", tags, MaterialType::Metal, r)
}

/// A brushed metal: fine lines along the part's X, streaks of slightly
/// different polish along them, and the sheen they make - a highlight
/// drawn out across the brushing, on every part, whatever its UVs.
pub fn brushed(name: &'static str, tags: &'static [&'static str], ior: &str, tint: Spectrum, roughness: f32) -> Preset {
    brushed_with(name, tags, ior, tint, roughness, 0.92, (1800.0, 60.0))
}

/// [`brushed`], with how strongly it is directional and its lines' size:
/// `lines` across a metre, and how many times longer than wide.
pub fn brushed_with(name: &'static str, tags: &'static [&'static str], ior: &str, tint: Spectrum, roughness: f32, anisotropy: f32, lines: (f32, f32)) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Anisotropic, |m| {
        m.metal_ior = metal_ior(ior);
        m.base_color = tint;
        m.specular_roughness = roughness;
        m.specular_roughness_anisotropy = anisotropy;
    }));
    let grooves = brush_lines(&mut r, lines.0, lines.1);
    r.bump(grooves, 1.5e-6);
    let along = r.stretched([0.03, 1.0, 1.0]);
    let streaks = r.node("noise_texture").w("Vector", along).f("Scale", 600.0).f("Detail", 3.0).out("Fac");
    // Each pass of the brush cuts a little deeper or shallower than the
    // last: streaks of brighter and duller metal along the grain, as a
    // brushed sheet shows at any distance.
    let both = plus(&mut r, grooves, 0.6, streaks);
    let gloss = r.range(both, (0.45, 1.15), (roughness * 0.8, roughness * 1.25));
    r.to(Param::SpecularRoughness, gloss);
    let shade = r.range(both, (0.45, 1.15), (0.86, 1.04));
    let colour = r.node("mix").set("blend_type", "multiply").f("Factor", 1.0).c("A", tint).w("B", shade).out("Result");
    r.to(Param::BaseColor, colour);
    grooves_along_axis(&mut r, [1.0, 0.0, 0.0]);
    preset(name, "Metal", tags, MaterialType::Anisotropic, r)
}

/// A metal with something growing on it - verdigris, rust - gathering where
/// `field` is high, in `colours`, rough and dull where it has. Its edges
/// are broken and ragged, as corrosion's are, not a blob's smooth outline,
/// and the metal is tarnished well beyond it before it is eaten: `bare`,
/// then `tarnish` from `tarnish_from` on, then the growth.
#[allow(clippy::too_many_arguments)]
fn aged(r: &mut Recipe, scale: f32, coverage: (f32, f32), tarnish_from: f32, bare: Spectrum, tarnish: Spectrum, colours: &[(f32, Spectrum)], depth: f32) {
    let field = r.node("noise_texture").f("Scale", scale).f("Detail", 8.0).f("Roughness", 0.62).f("Distortion", 0.3).out("Fac");
    let grit = r.node("noise_texture").f("Scale", scale * 7.0).f("Detail", 6.0).f("Roughness", 0.65).out("Fac");
    let broken = plus(r, grit, 0.22, field);
    let lifted = r.math("subtract", broken, 0.11);
    let crust = r.smooth(lifted, coverage, (0.0, 1.0));
    let near = r.smooth(lifted, (tarnish_from, coverage.0), (0.0, 1.0));
    let metalness = r.range(crust, (0.0, 1.0), (1.0, 0.0));
    r.to(Param::BaseMetalness, metalness);
    let fine = noise(r, scale * 18.0, 3.0, 0.6);
    let growth = r.ramp(fine, crate::recipe::Linear, colours);
    let tarnished = r.node("mix").f("Factor", near).c("A", bare).c("B", tarnish).out("Result");
    let colour = r.node("mix").f("Factor", crust).w("A", tarnished).w("B", growth).out("Result");
    r.to(Param::BaseColor, colour);
    let dull = r.range(near, (0.0, 1.0), (0.28, 0.42));
    let gloss = r.node("mix").set("data_type", "float").f("Factor", crust).w("A", dull).f("B", 0.9).out("Result");
    r.to(Param::SpecularRoughness, gloss);
    let height = plus(r, fine, 0.3, crust);
    r.bump(height, depth);
}

pub fn all() -> Vec<Preset> {
    let white = Spectrum::ONE;
    let mut out = vec![
        polished("Chrome", &["chrome", "mirror", "trim", "plating"], "Chromium", white, 0.012, 0.035),
        polished("Chrome Black", &["chrome", "black", "trim"], "Chromium", rgb(0.3005, 0.3001, 0.3087), 0.03, 0.06),
        polished("Aluminium Polished", &["aluminium", "aluminum", "mirror"], "Aluminium", white, 0.03, 0.07),
        polished("Gold Polished", &["gold", "jewellery", "mirror"], "Gold", white, 0.02, 0.05),
        polished("Silver Polished", &["silver", "jewellery", "mirror"], "Silver", white, 0.02, 0.05),
        polished("Platinum", &["platinum", "jewellery", "white metal"], "Platinum", white, 0.035, 0.07),
        polished("Rose Gold", &["gold", "rose", "jewellery"], "Copper", rgb(0.9854, 0.9979, 0.9561), 0.03, 0.06),
        polished("Copper Polished", &["copper", "warm"], "Copper", white, 0.05, 0.1),
        polished("Brass Polished", &["brass", "gold", "fitting"], "Gold", rgb(0.9335, 0.8663, 0.6367), 0.03, 0.07),
        polished("Stainless Polished", &["steel", "stainless", "mirror"], "Nickel", rgb(0.9462, 0.955, 0.9708), 0.04, 0.08),
        satin("Copper Satin", &["copper", "warm", "satin"], "Copper", white, 0.24, 0.34),
        satin("Brass Satin", &["brass", "gold", "fitting", "satin"], "Gold", rgb(0.9223, 0.8458, 0.5604), 0.2, 0.3),
        satin("Nickel Satin", &["nickel", "plating", "hardware"], "Nickel", white, 0.2, 0.3),
        satin("Titanium Satin", &["titanium", "medical", "satin"], "Titanium", rgb(0.8643, 0.8559, 0.8412), 0.26, 0.36),
        satin("Gunmetal", &["gunmetal", "dark", "steel"], "Iron", rgb(0.5601, 0.5693, 0.6131), 0.24, 0.34),
        satin("Steel Blued", &["steel", "gun", "dark", "blued"], "Iron", rgb(0.0667, 0.0718, 0.0941), 0.14, 0.22),
        anodised("Aluminium Anodised Silver", &["aluminium", "anodised", "satin"], rgb(0.693, 0.7003, 0.7135), 0.34, 0.44),
        anodised("Aluminium Anodised Black", &["aluminium", "anodised", "matte", "black"], rgb(0.0274, 0.0274, 0.0293), 0.4, 0.5),
        anodised("Aluminium Anodised Blue", &["aluminium", "anodised", "blue", "colour"], srgb("#2F5FA8"), 0.34, 0.44),
        anodised("Aluminium Anodised Red", &["aluminium", "anodised", "red", "colour"], srgb("#A82A30"), 0.34, 0.44),
        satin("Titanium Anodised Bronze", &["titanium", "anodised", "bronze", "watch"], "Titanium", srgb("#B8895A"), 0.22, 0.3),
        brushed("Aluminium Brushed", &["aluminium", "brushed", "satin"], "Aluminium", rgb(0.9734, 0.9734, 0.9734), 0.28),
        brushed("Stainless Brushed", &["steel", "stainless", "brushed"], "Nickel", rgb(0.8458, 0.8541, 0.8689), 0.24),
        brushed("Gold Brushed", &["gold", "brushed", "watch", "jewellery"], "Gold", white, 0.22),
    ];

    // Bead blasted: a fine, even grain and no direction at all.
    let mut r = Recipe::new(metal("Aluminium", Spectrum::ONE, 0.48));
    let gloss = wear(&mut r, 0.44, 0.52, 12.0);
    r.to(Param::SpecularRoughness, gloss);
    micro(&mut r, 9000.0, 3e-6);
    out.push(preset("Aluminium Bead Blasted", "Metal", &["aluminium", "aluminum", "blasted", "matte"], MaterialType::Metal, r));

    // Cast aluminium: the sand mould's grain, pitted here and there.
    let mut r = Recipe::new(metal("Aluminium", rgb(0.6438, 0.6443, 0.6325), 0.56));
    let sand = noise(&mut r, 1800.0, 4.0, 0.55);
    let pits = r.node("voronoi_texture").f("Scale", 300.0).f("Randomness", 1.0).out("Distance");
    let pitted = r.smooth(pits, (0.0, 0.12), (-1.0, 0.0));
    let height = plus(&mut r, pitted, 0.6, sand);
    r.bump(height, 3.5e-5);
    let gloss = wear(&mut r, 0.5, 0.64, 14.0);
    r.to(Param::SpecularRoughness, gloss);
    out.push(preset("Aluminium Cast", "Metal", &["aluminium", "cast", "rough"], MaterialType::Metal, r));

    // Cast iron: dark and coarse, the high spots worn brighter.
    let mut r = Recipe::new(metal("Iron", rgb(0.3012, 0.2966, 0.2888), 0.68));
    let sand = noise(&mut r, 1200.0, 5.0, 0.6);
    r.bump(sand, 6e-5);
    let tint = r.range(sand, (0.35, 0.65), (0.2, 0.36));
    r.to(Param::BaseColor, tint);
    let gloss = r.range(sand, (0.35, 0.7), (0.74, 0.58));
    r.to(Param::SpecularRoughness, gloss);
    out.push(preset("Cast Iron", "Metal", &["iron", "cast", "dark", "rough"], MaterialType::Metal, r));

    // Hammered copper: dimples beaten into a sheet, each its own facet.
    let mut r = Recipe::new(metal("Copper", Spectrum::ONE, 0.16));
    let dimples = r.node("voronoi_texture").set("feature", "smooth_f1").f("Scale", 180.0).f("Smoothness", 0.6).f("Randomness", 0.85).out("Distance");
    r.bump(dimples, 4.5e-4);
    let gloss = wear(&mut r, 0.12, 0.22, 20.0);
    r.to(Param::SpecularRoughness, gloss);
    out.push(preset("Copper Hammered", "Metal", &["copper", "hammered", "beaten", "craft"], MaterialType::Metal, r));

    // Galvanised steel: the zinc's spangles, each crystal a slightly
    // different grey and polish.
    let mut r = Recipe::new(metal("Nickel", rgb(0.75, 0.76, 0.77), 0.25));
    let crystals = r.node("voronoi_texture").f("Scale", 55.0).f("Randomness", 1.0).out("Color");
    let shade = r.node("separate_color").w("Color", crystals).out("Red");
    let grey = r.range(shade, (0.0, 1.0), (0.52, 0.92));
    r.to(Param::BaseColor, grey);
    let gloss = r.range(shade, (0.0, 1.0), (0.08, 0.5));
    r.to(Param::SpecularRoughness, gloss);
    out.push(preset("Steel Galvanised", "Metal", &["steel", "galvanised", "zinc", "spangle", "industrial"], MaterialType::Metal, r));

    // Knurled steel: a machined diamond grip.
    let mut r = Recipe::new(metal("Nickel", rgb(0.86, 0.865, 0.87), 0.22));
    let k = knurl(&mut r, 320.0);
    r.bump(k, 2.2e-4);
    let gloss = r.range(k, (0.0, 1.0), (0.32, 0.16));
    r.to(Param::SpecularRoughness, gloss);
    out.push(preset("Steel Knurled", "Metal", &["steel", "knurled", "grip", "machined"], MaterialType::Metal, r));

    // Copper gone green: years outdoors darken the whole of it to a brown
    // tarnish, and the verdigris over that is chalky and pale - a murky
    // green-brown where it is thin, blue-green where it is thick.
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.metal_ior = metal_ior("Copper");
        m.base_metalness = 1.0;
        m.specular_roughness = 0.4;
    }));
    aged(&mut r, 60.0, (0.4, 0.56), 0.12, rgb(0.17, 0.19, 0.21), rgb(0.1, 0.11, 0.12), &[(0.0, srgb("#4D6355")), (0.35, srgb("#618F7D")), (0.7, srgb("#80AF9C")), (1.0, srgb("#A6C9B9"))], 6e-5);
    out.push(preset("Copper Verdigris", "Metal", &["copper", "patina", "verdigris", "aged", "green"], MaterialType::Generic, r));

    // Rusted steel: orange-brown scale eating into the metal.
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.metal_ior = metal_ior("Iron");
        m.base_metalness = 1.0;
        m.base_color = rgb(0.55, 0.55, 0.56);
        m.specular_roughness = 0.35;
    }));
    aged(&mut r, 35.0, (0.26, 0.44), 0.1, rgb(0.42, 0.42, 0.43), rgb(0.26, 0.2, 0.17), &[(0.0, srgb("#4A2412")), (0.45, srgb("#7A3A1A")), (0.8, srgb("#A5582A")), (1.0, srgb("#C27A3E"))], 1.2e-4);
    out.push(preset("Steel Rusted", "Metal", &["steel", "rust", "corroded", "aged", "weathered"], MaterialType::Generic, r));
    out
}
