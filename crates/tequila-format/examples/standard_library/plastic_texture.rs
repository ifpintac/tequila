//! Plastics by finish and texture: the polish ladder of a moulding tool
//! (high gloss to blasted), spark-eroded textures by their VDI 3400 grade,
//! stipples and grains, moulded grip textures (hexagons, diamonds, dots,
//! ribs, hairline), and plastics with something in them - metallic flake,
//! pearl, glitter, recycled confetti and swirls - or made some other way:
//! printed, cast in resin, foamed.

use crate::finish::{brush_lines, cell, staggered, triplanar, Hole};
use crate::plastic::knurl;
use crate::recipe::{Constant, Linear};
use crate::{base, micro, noise, pattern, plain, plus, preset, rgb, srgb, wear, Out, Preset, Recipe};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::{MaterialType, Param};
use tequila_texture::procedural::Procedural;

fn plastic(colour: Spectrum, roughness: f32) -> tequila_materials::openpbr::Material {
    base(MaterialType::Plastic, |m| {
        m.base_color = colour;
        m.specular_roughness = roughness;
    })
}

/// A rung of the polish ladder a mould is finished to: from a diamond
/// buffed mirror, through paper and stone polishes that leave faint lines,
/// to a blast that leaves a matte tooth. `lines` is the polishing's
/// direction marks, if it leaves any.
#[allow(clippy::too_many_arguments)]
fn polish(name: &'static str, tags: &'static [&'static str], colour: Spectrum, low: f32, high: f32, lines: Option<(f32, f32)>, tooth: Option<(f32, f32)>, wave: f32) -> Preset {
    let mut r = Recipe::new(plastic(colour, (low + high) * 0.5));
    let gloss = wear(&mut r, low, high, 25.0);
    let mut height: Option<Out> = None;
    if wave > 0.0 {
        // Every moulded panel's reflection wanders a little.
        let w = noise(&mut r, 35.0, 2.0, 0.4);
        let w = r.math("multiply", w, wave);
        height = Some(w);
    }
    if let Some((across, length)) = lines {
        let l = brush_lines(&mut r, across, length);
        let l = r.math("multiply", l, 4e-7);
        height = Some(match height {
            Some(h) => r.math("add", h, l),
            None => l,
        });
    }
    if let Some((scale, depth)) = tooth {
        let t = r.node("voronoi_texture").f("Scale", scale).f("Randomness", 1.0).out("Distance");
        let t = r.math("multiply", t, depth);
        height = Some(match height {
            Some(h) => r.math("add", h, t),
            None => t,
        });
    }
    if let Some(h) = height {
        r.bump(h, 1.0);
    }
    r.to(Param::SpecularRoughness, gloss);
    preset(name, "Plastic", tags, MaterialType::Plastic, r)
}

/// A spark-eroded texture by its VDI 3400 grade: the mould burned by an
/// electrode into overlapping craters, as much rougher with each grade as
/// the standard's roughness (Ra = 0.1 µm × 10^(grade / 20)) says - from a
/// fine satin at VDI 18 to sandpaper at VDI 45.
fn vdi(grade: u32, name: &'static str, tags: &'static [&'static str], colour: Spectrum) -> Preset {
    let ra = 1e-7 * 10f32.powf(grade as f32 / 20.0);
    let roughness = (0.3 + 0.019 * (grade as f32 - 24.0)).clamp(0.2, 0.74);
    let mut r = Recipe::new(plastic(colour, roughness));
    let crater = ra * 25.0;
    let big = r.node("voronoi_texture").set("feature", "smooth_f1").f("Scale", 1.0 / crater).f("Smoothness", 0.25).f("Randomness", 1.0).out("Distance");
    let small = r.node("voronoi_texture").set("feature", "smooth_f1").f("Scale", 2.7 / crater).f("Smoothness", 0.25).f("Randomness", 1.0).out("Distance");
    let height = plus(&mut r, small, 0.5, big);
    r.bump(height, ra * 3.0);
    let gloss = wear(&mut r, roughness * 0.94, roughness * 1.06, 20.0);
    r.to(Param::SpecularRoughness, gloss);
    preset(name, "Plastic", tags, MaterialType::Plastic, r)
}

/// A chemically etched stipple: raised rounded dots at two sizes, the tops
/// a little glossier where hands have worn them.
fn stipple(name: &'static str, tags: &'static [&'static str], colour: Spectrum, scale: f32, depth: f32, low: f32, high: f32) -> Preset {
    let mut r = Recipe::new(plastic(colour, (low + high) * 0.5));
    let big = r.node("voronoi_texture").set("feature", "smooth_f1").f("Scale", scale).f("Smoothness", 0.5).f("Randomness", 1.0).out("Distance");
    let small = r.node("voronoi_texture").set("feature", "smooth_f1").f("Scale", scale * 2.6).f("Smoothness", 0.5).f("Randomness", 1.0).out("Distance");
    let domes = r.math("subtract", 1.0, big);
    let fine = r.math("subtract", 1.0, small);
    let height = plus(&mut r, fine, 0.45, domes);
    r.bump(height, depth);
    let gloss = r.range(domes, (0.3, 1.0), (high, low));
    r.to(Param::SpecularRoughness, gloss);
    preset(name, "Plastic", tags, MaterialType::Plastic, r)
}

/// A raised grip texture laid on any part: `height` from (u, v) on each of
/// its planes, `depth` metres for its 1, the raised tops glossier.
fn grip(name: &'static str, tags: &'static [&'static str], ty: MaterialType, material: tequila_materials::openpbr::Material, depth: f32, tops: f32, height: impl Fn(&mut Recipe, Out, Out) -> Out) -> Preset {
    let valleys = material.specular_roughness;
    let mut r = Recipe::new(material);
    let h = triplanar(&mut r, 8.0, |r, u, v, _| vec![height(r, u, v)])[0];
    r.bump(h, depth);
    let gloss = r.range(h, (0.0, 1.0), (valleys, tops));
    r.to(Param::SpecularRoughness, gloss);
    preset(name, "Plastic", tags, ty, r)
}

/// A moulding with metal flake in it, or mica, or glitter: the resin's own
/// polish over flakes that each catch the light their own way.
#[allow(clippy::too_many_arguments)]
fn flaked(name: &'static str, tags: &'static [&'static str], colour: Spectrum, flake: Spectrum, weight: f32, size: f32, spread: f32, flake_roughness: f32) -> Preset {
    plain(name, "Plastic", tags, MaterialType::MetallicPaint, |m| {
        m.base_color = colour;
        m.flake_color = flake;
        m.flake_weight = weight;
        m.flake_size = size;
        m.flake_spread = spread;
        m.flake_roughness = flake_roughness;
        m.specular_roughness = 0.3;
        m.coat_roughness = 0.04;
    })
}

/// Recycled sheet pressed from flakes of coloured bottles: chips of a
/// few millimetres in `chips` colours, irregular, scattered over `ground`,
/// with a fine speckle of the darker bits between.
fn confetti(name: &'static str, tags: &'static [&'static str], ground: Spectrum, chips: &[(f32, Spectrum)], size: f32, cover: f32) -> Preset {
    let mut r = Recipe::new(plastic(ground, 0.36));
    let coords = r.node("texture_coordinate").out("Object");
    let warp = r.node("noise_texture").w("Vector", coords).f("Scale", 0.6 / size).f("Detail", 3.0).out("Color");
    let warped = r.node("vector_math").set("operation", "multiply_add").w("Vector", warp).v("Vector 2", [size * 0.55, size * 0.55, size * 0.55]).w("Vector 3", coords).out("Vector");
    let mut colour = None;
    for (layer, scale) in [(0u32, 1.0 / size), (1, 2.3 / size)] {
        let cells = r.node("voronoi_texture").w("Vector", warped).f("Scale", scale).f("Randomness", 1.0).add();
        let pick = r.node("separate_color").w("Color", Out(cells, 1)).add();
        let edge = r.node("noise_texture").w("Vector", coords).f("Scale", scale * 3.0).f("Detail", 2.0).out("Fac");
        let radius = r.range(edge, (0.3, 0.7), (0.24, 0.4));
        let inside = r.math("less_than", Out(cells, 0), radius);
        let present = r.math("less_than", Out(pick, 1), cover * if layer == 0 { 1.0 } else { 0.6 });
        let chip = r.math("multiply", inside, present);
        let tint = r.ramp(Out(pick, 2), Constant, chips);
        let under = match colour {
            Some(c) => c,
            None => r.node("mix").f("Factor", 0.0).c("A", ground).c("B", ground).out("Result"),
        };
        colour = Some(r.node("mix").w("Factor", chip).w("A", under).w("B", tint).out("Result"));
    }
    // The fine speckle: crumbs of dark flake.
    let crumbs = r.node("voronoi_texture").f("Scale", 0.12 / (size * 0.02)).f("Randomness", 1.0).add();
    let crumb_pick = r.node("separate_color").w("Color", Out(crumbs, 1)).add();
    let small = r.math("less_than", Out(crumbs, 0), 0.22);
    let some = r.math("less_than", Out(crumb_pick, 0), 0.18);
    let crumb = r.math("multiply", small, some);
    let colour = r.node("mix").w("Factor", crumb).w("A", colour.expect("chips")).c("B", srgb("#2B2B2D")).out("Result");
    r.to(Param::BaseColor, colour);
    let gloss = wear(&mut r, 0.32, 0.42, 20.0);
    r.to(Param::SpecularRoughness, gloss);
    micro(&mut r, 5000.0, 2e-6);
    preset(name, "Plastic", tags, MaterialType::Plastic, r)
}

/// Recycled sheet pressed from a few colours at once that never quite
/// mixed: bands that swirl and fold, with a few flecks.
fn swirl(name: &'static str, tags: &'static [&'static str], colours: &[(f32, Spectrum)]) -> Preset {
    let mut r = Recipe::new(plastic(colours[0].1, 0.34));
    let bands = r
        .node("wave_texture")
        .set("type", "bands")
        .set("direction", "diagonal")
        .f("Scale", 70.0)
        .f("Distortion", 6.0)
        .f("Detail", 3.0)
        .f("Detail Scale", 0.7)
        .f("Detail Roughness", 0.6)
        .out("Fac");
    let colour = r.ramp(bands, Linear, colours);
    r.to(Param::BaseColor, colour);
    let gloss = wear(&mut r, 0.3, 0.4, 20.0);
    r.to(Param::SpecularRoughness, gloss);
    micro(&mut r, 5000.0, 2e-6);
    preset(name, "Plastic", tags, MaterialType::Plastic, r)
}

/// Printed in layers: each a rounded bead `layer` high, stacked up the
/// part, a little uneven from one layer to the next, and glossier along
/// each bead's crown than in the seam between.
fn printed(name: &'static str, tags: &'static [&'static str], colour: Spectrum, layer: f32, low: f32, high: f32) -> Preset {
    let mut r = Recipe::new(plastic(colour, (low + high) * 0.5));
    let coords = r.node("texture_coordinate").out("Object");
    let y = r.node("separate_xyz").w("Vector", coords).out("Y");
    let (f, index) = cell(&mut r, y, layer, 0.5);
    let f2 = r.math("multiply", f, 2.0);
    let sq = r.math("multiply", f2, f2);
    let bead = r.math("subtract", 1.0, sq);
    let bead = r.math("maximum", bead, 0.0);
    let bead = r.math("sqrt", bead, 0.0);
    // Each layer laid a hair to one side of the last.
    let which = r.node("combine_xyz").w("X", index).f("Y", 3.0).f("Z", 1.0).out("Vector");
    let offset = r.node("white_noise_texture").w("Vector", which).out("Value");
    let height = plus(&mut r, offset, 0.25, bead);
    r.bump(height, layer * 0.18);
    let gloss = r.range(bead, (0.0, 1.0), (high, low));
    r.to(Param::SpecularRoughness, gloss);
    preset(name, "Plastic", tags, MaterialType::Plastic, r)
}

/// Soft touch: a rubbery paint, flat and velvety, with a fine tooth.
fn soft_touch(name: &'static str, tags: &'static [&'static str], colour: Spectrum) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = colour;
        m.specular_roughness = 0.62;
        m.specular_weight = 0.75;
    }));
    let gloss = wear(&mut r, 0.56, 0.68, 20.0);
    r.to(Param::SpecularRoughness, gloss);
    micro(&mut r, 6000.0, 2.5e-6);
    preset(name, "Plastic", tags, MaterialType::Generic, r)
}

pub fn all() -> Vec<Preset> {
    let mut out = vec![
        // The polish ladder.
        polish("Plastic High Gloss White", &["gloss", "white", "mirror polish", "spi a", "consumer"], rgb(0.84, 0.84, 0.83), 0.02, 0.05, None, None, 1.2e-5),
        polish("Plastic Semi-Gloss Grey", &["semi gloss", "grey", "paper polish", "spi b", "enclosure"], srgb("#6E7177"), 0.1, 0.15, Some((5000.0, 40.0)), None, 0.0),
        polish("Plastic Satin Black", &["satin", "black", "stone polish", "spi c", "enclosure"], srgb("#151517"), 0.26, 0.32, None, Some((8000.0, 1.5e-6)), 0.0),
        polish("Plastic Matte White", &["matte", "white", "blasted", "spi d", "appliance"], srgb("#E4E3DE"), 0.5, 0.56, None, Some((9000.0, 3e-6)), 0.0),
        // Spark-eroded, by grade.
        vdi(18, "Plastic VDI 18 Black", &["vdi", "edm", "spark eroded", "fine", "satin", "black"], srgb("#141416")),
        vdi(24, "Plastic VDI 24 Grey", &["vdi", "edm", "spark eroded", "satin", "grey"], srgb("#5A5D62")),
        vdi(30, "Plastic VDI 30 Black", &["vdi", "edm", "spark eroded", "matte", "black"], srgb("#161618")),
        vdi(36, "Plastic VDI 36 Light Grey", &["vdi", "edm", "spark eroded", "matte", "grey"], srgb("#B6B8B9")),
        vdi(42, "Plastic VDI 42 Charcoal", &["vdi", "edm", "spark eroded", "coarse", "charcoal"], srgb("#2E3033")),
        // Stipples and grains.
        stipple("Plastic Stipple Coarse", &["stipple", "textured", "grained", "enclosure", "tool"], srgb("#2A2B2E"), 1400.0, 4e-5, 0.46, 0.62),
        stipple("Plastic Sand Texture", &["sand", "textured", "grained", "fine", "beige"], srgb("#BDB8AE"), 4200.0, 1.4e-5, 0.54, 0.64),
    ];

    // Pebble grain: the coarse leather grain of dashboards and luggage.
    let mut r = Recipe::new(plastic(srgb("#26262A"), 0.55));
    let grain = pattern(&mut r, Procedural::Leather { frequency: 430.0, crack_width: 0.14, wrinkle: 0.35 }, 7);
    r.bump(grain, 7e-5);
    let gloss = r.range(grain, (0.2, 0.9), (0.62, 0.44));
    r.to(Param::SpecularRoughness, gloss);
    out.push(preset("Plastic Pebble Grain", "Plastic", &["pebble grain", "leatherette", "automotive", "luggage", "grained"], MaterialType::Plastic, r));

    // Hairline: the brushed-metal look moulded in, as on a television's
    // bezel.
    let mut r = Recipe::new(plastic(srgb("#0E0E10"), 0.22));
    let lines = brush_lines(&mut r, 5000.0, 200.0);
    r.bump(lines, 6e-7);
    let gloss = r.range(lines, (0.3, 0.7), (0.17, 0.3));
    r.to(Param::SpecularRoughness, gloss);
    out.push(preset("Plastic Hairline Black", "Plastic", &["hairline", "brushed look", "black", "electronics", "bezel"], MaterialType::Plastic, r));

    // Moulded grips.
    out.push(grip("Plastic Hex Grip", &["hexagon", "grip", "textured", "tool", "handle"], MaterialType::Plastic, plastic(srgb("#1C1C1E"), 0.55), 2.5e-4, 0.4, |r, u, v| {
        let d = staggered(r, u, v, 0.0022, Hole::Hex);
        // Moulded, so its edges are rounded, not cut.
        r.smooth(d, (0.0006, 0.0009), (1.0, 0.0))
    }));
    out.push(grip("Rubber Dot Grip", &["rubber", "dots", "grip", "non slip", "handle"], MaterialType::Generic, base(MaterialType::Generic, |m| {
        m.base_color = srgb("#1A1A1B");
        m.base_diffuse_roughness = 0.5;
        m.specular_roughness = 0.78;
    }), 4e-4, 0.6, |r, u, v| {
        let d = staggered(r, u, v, 0.0026, Hole::Round);
        let t = r.math("divide", d, 0.0009);
        let tt = r.math("multiply", t, t);
        let dome = r.math("subtract", 1.0, tt);
        let dome = r.math("maximum", dome, 0.0);
        r.math("sqrt", dome, 0.0)
    }));
    let mut r = Recipe::new(plastic(srgb("#2E2F33"), 0.5));
    let k = knurl(&mut r, 480.0);
    r.bump(k, 1.5e-4);
    let gloss = r.range(k, (0.0, 1.0), (0.55, 0.38));
    r.to(Param::SpecularRoughness, gloss);
    out.push(preset("Plastic Diamond Texture", "Plastic", &["diamond", "knurled", "pyramid", "grip", "textured"], MaterialType::Plastic, r));
    let mut r = Recipe::new(plastic(srgb("#3A3D42"), 0.36));
    let coords = r.node("texture_coordinate").out("Object");
    let x = r.node("separate_xyz").w("Vector", coords).out("X");
    let (f, _) = cell(&mut r, x, 0.0015, 0.5);
    let f2 = r.math("multiply", f, 2.0);
    let sq = r.math("multiply", f2, f2);
    let rib = r.math("subtract", 1.0, sq);
    let rib = r.math("maximum", rib, 0.0);
    let rib = r.math("sqrt", rib, 0.0);
    r.bump(rib, 1.6e-4);
    out.push(preset("Plastic Ribbed", "Plastic", &["ribbed", "fluted", "grip", "lid", "textured"], MaterialType::Plastic, r));

    // Flake, mica and glitter in the resin.
    out.push(flaked("Plastic Metallic Silver", &["metallic", "silver", "flake", "moulded", "consumer"], rgb(0.3, 0.31, 0.32), rgb(0.86, 0.86, 0.87), 0.6, 4.5e-5, 0.4, 0.3));
    out.push(flaked("Plastic Metallic Blue", &["metallic", "blue", "flake", "moulded", "consumer"], srgb("#1C3366"), rgb(0.42, 0.5, 0.78), 0.5, 4.5e-5, 0.3, 0.25));
    out.push(flaked("Plastic Pearl White", &["pearl", "pearlescent", "mica", "white", "cosmetic"], srgb("#E6E2DA"), srgb("#F7D6C4"), 0.7, 5e-5, 0.8, 0.2));
    out.push(flaked("Plastic Glitter Purple", &["glitter", "sparkle", "purple", "toy", "case"], srgb("#3A1B5C"), rgb(0.9, 0.9, 0.92), 0.16, 3e-4, 0.6, 0.12));

    // Recycled.
    out.push(confetti(
        "Recycled Plastic Confetti",
        &["recycled", "confetti", "hdpe", "terrazzo", "sheet", "sustainable"],
        srgb("#E9E5DA"),
        &[(0.0, srgb("#2D5DA8")), (0.18, srgb("#3E8E5C")), (0.34, srgb("#1C1C1C")), (0.48, srgb("#E07A2E")), (0.62, srgb("#F4F3EE")), (0.76, srgb("#C23B32")), (0.88, srgb("#8FB8DE"))],
        0.006,
        0.42,
    ));
    out.push(confetti(
        "Recycled Plastic Speckled",
        &["recycled", "speckled", "granite look", "sheet", "sustainable"],
        srgb("#CFCDC6"),
        &[(0.0, srgb("#1E1E20")), (0.5, srgb("#3A3B3E")), (0.8, srgb("#7A7D80"))],
        0.0022,
        0.3,
    ));
    out.push(swirl(
        "Recycled Plastic Swirl",
        &["recycled", "swirl", "marbled", "sheet", "sustainable"],
        &[(0.0, srgb("#26272A")), (0.28, srgb("#34363A")), (0.4, srgb("#CFCFC9")), (0.55, srgb("#E2E2DC")), (0.68, srgb("#8C8F94")), (0.84, srgb("#2A2B2F")), (1.0, srgb("#26272A"))],
    ));

    // Clear, smoked and frosted.
    out.push(plain("Acrylic Frosted", "Plastic", &["acrylic", "pmma", "frosted", "sandblasted", "diffuser"], MaterialType::PlasticTransparent, |m| {
        m.specular_ior = 1.49;
        m.specular_roughness = 0.42;
        m.transmission_color = rgb(0.95, 0.96, 0.96);
    }));
    out.push(plain("Acrylic Smoke", "Plastic", &["acrylic", "pmma", "smoke", "tinted", "cover"], MaterialType::PlasticTransparent, |m| {
        m.specular_ior = 1.49;
        m.specular_roughness = 0.02;
        m.transmission_depth = 0.004;
        m.transmission_color = rgb(0.3, 0.3, 0.31);
    }));

    // Printed, cast and foamed.
    out.push(printed("PLA 3D Print Orange", &["pla", "3d print", "fdm", "layer lines", "orange"], srgb("#E8622A"), 0.00025, 0.3, 0.44));
    out.push(printed("PETG 3D Print Black", &["petg", "3d print", "fdm", "layer lines", "black"], srgb("#151517"), 0.0002, 0.16, 0.3));
    let mut r = Recipe::new(plastic(srgb("#8E9196"), 0.4));
    let coords = r.node("texture_coordinate").out("Object");
    let y = r.node("separate_xyz").w("Vector", coords).out("Y");
    let layer = r.math("multiply", y, std::f32::consts::TAU / 0.00005);
    let layers = r.math("sine", layer, 0.0);
    let grain = noise(&mut r, 9000.0, 2.0, 0.5);
    let height = plus(&mut r, layers, 0.3, grain);
    r.bump(height, 1e-6);
    let gloss = wear(&mut r, 0.36, 0.44, 20.0);
    r.to(Param::SpecularRoughness, gloss);
    out.push(preset("Resin SLA Grey", "Plastic", &["resin", "sla", "3d print", "prototype", "grey"], MaterialType::Plastic, r));
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = srgb("#1F1F1F");
        m.base_diffuse_roughness = 0.5;
        m.specular_roughness = 0.72;
        m.specular_weight = 0.7;
    }));
    let pores = r.node("voronoi_texture").f("Scale", 1500.0).f("Randomness", 1.0).out("Distance");
    let pits = r.smooth(pores, (0.0, 0.22), (-1.0, 0.0));
    let skin = noise(&mut r, 1200.0, 3.0, 0.55);
    let height = plus(&mut r, skin, 0.3, pits);
    r.bump(height, 1.4e-4);
    out.push(preset("EVA Foam Black", "Plastic", &["eva", "foam", "closed cell", "black", "sole", "padding"], MaterialType::Generic, r));

    // Engineering plastics, as they look.
    out.push(plain("Silicone Soft Grey", "Plastic", &["silicone", "soft", "rubber", "grey", "kitchen"], MaterialType::Translucent, |m| {
        m.base_color = srgb("#7C8085");
        m.subsurface_color = srgb("#9A9EA3");
        m.subsurface_weight = 0.3;
        m.subsurface_radius = rgb(0.002, 0.002, 0.002);
        m.specular_roughness = 0.46;
    }));
    out.push(plain("POM White", "Plastic", &["pom", "acetal", "delrin", "white", "engineering", "waxy"], MaterialType::PlasticCloudy, |m| {
        m.base_color = srgb("#ECEBE5");
        m.specular_roughness = 0.3;
        m.subsurface_radius = rgb(0.03, 0.028, 0.025);
    }));
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = srgb("#1A1A1C");
        m.specular_roughness = 0.42;
    }));
    let flow = r.node("noise_texture").f("Scale", 260.0).f("Detail", 4.0).f("Roughness", 0.55).f("Distortion", 2.5).out("Fac");
    let fibres = r.stretched([0.05, 1.0, 1.0]);
    let fibres = r.node("noise_texture").w("Vector", fibres).f("Scale", 3000.0).f("Detail", 2.0).out("Fac");
    let colour = r.node("mix").w("Factor", flow).c("A", srgb("#18181A")).c("B", srgb("#232326")).out("Result");
    r.to(Param::BaseColor, colour);
    let gloss = r.range(flow, (0.3, 0.7), (0.36, 0.5));
    r.to(Param::SpecularRoughness, gloss);
    r.bump(fibres, 1.2e-6);
    out.push(preset("Nylon Glass Filled Black", "Plastic", &["nylon", "pa66", "glass filled", "engineering", "black"], MaterialType::Generic, r));

    // Tortoiseshell acetate: amber and brown blotches deep in a glossy
    // translucent slab.
    let mut r = Recipe::new(base(MaterialType::PlasticCloudy, |m| {
        m.base_color = srgb("#8A4A1E");
        m.specular_roughness = 0.05;
        m.subsurface_radius = rgb(0.008, 0.004, 0.0018);
    }));
    let blotch = r.node("noise_texture").f("Scale", 70.0).f("Detail", 6.0).f("Roughness", 0.55).f("Distortion", 1.2).out("Fac");
    let fine = noise(&mut r, 400.0, 3.0, 0.6);
    let tone = plus(&mut r, fine, 0.22, blotch);
    let colour = r.ramp(tone, Linear, &[(0.32, srgb("#1E0D06")), (0.45, srgb("#4A220C")), (0.56, srgb("#8C4614")), (0.66, srgb("#C27726")), (0.76, srgb("#DDA54E"))]);
    r.to(Param::BaseColor, colour);
    out.push(preset("Acetate Tortoiseshell", "Plastic", &["acetate", "tortoiseshell", "eyewear", "amber", "translucent"], MaterialType::PlasticCloudy, r));

    out.push(soft_touch("Soft Touch Grey", &["soft touch", "rubberised", "matte", "grey"], srgb("#55585D")));
    out.push(soft_touch("Soft Touch White", &["soft touch", "rubberised", "matte", "white"], srgb("#DCDBD6")));
    out
}
