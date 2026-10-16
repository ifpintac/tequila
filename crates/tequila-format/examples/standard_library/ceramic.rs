//! Ceramics: glazes over clay.
//!
//! A glaze is a thin glass, so a glazed piece is a clear coat over a
//! coloured body, and what makes it read as ceramic rather than plastic is
//! what the glass does as it melts: it runs a little thicker here and
//! thinner there, which shifts its colour; it levels to a faint waviness;
//! iron in the clay boils up through it as specks with bled halos; it
//! crazes as it cools; and a reactive glaze breaks into flows and pools.

use crate::recipe::{Linear, Out};
use crate::{base, noise, plus, preset, rgb, srgb, Preset, Recipe};
use tequila_materials::material_type::{MaterialType, Param};

pub fn all() -> Vec<Preset> {
    vec![glazed_white(), matte(), stoneware(), porcelain(), crackle(), reactive_blue()]
}

/// Specks of iron in the body come up through the glaze: dark centres,
/// each with a browner halo where it bled, of every size; `per_metre`
/// cells, `share` of them speckled.
fn specks(r: &mut Recipe, colour: Out, per_metre: f32, share: f32, dark: &str, halo: &str) -> (Out, Out) {
    let coords = r.node("texture_coordinate").out("Object");
    let nick = r.node("noise_texture").w("Vector", coords).f("Scale", per_metre * 2.0).f("Detail", 2.0).out("Color");
    let nicked = r.node("vector_math").set("operation", "multiply_add").w("Vector", nick).v("Vector 2", [0.3 / per_metre; 3]).w("Vector 3", coords).out("Vector");
    let cells = r.node("voronoi_texture").w("Vector", nicked).f("Scale", per_metre).f("Randomness", 1.0).add();
    let pick = r.node("separate_color").w("Color", Out(cells, 1)).out("Red");
    let kept = r.smooth(pick, (1.0 - share, 1.0 - share + 0.01), (0.0, 1.0));
    let size = r.node("separate_color").w("Color", Out(cells, 1)).out("Green");
    let size = r.math("power", size, 2.5);
    let reach = r.range(size, (0.0, 1.0), (0.03, 0.2));
    let inside = r.math("subtract", reach, Out(cells, 0));
    let core = r.smooth(inside, (0.0, 0.02), (0.0, 1.0));
    let core = r.math("multiply", core, kept);
    let wide = r.math("multiply", reach, 2.6);
    let bleed_inside = r.math("subtract", wide, Out(cells, 0));
    let bleed = r.smooth(bleed_inside, (0.0, 0.12), (0.0, 0.55));
    let bleed = r.math("multiply", bleed, kept);
    let haloed = r.node("mix").f("Factor", bleed).w("A", colour).c("B", srgb(halo)).out("Result");
    let speckled = r.node("mix").f("Factor", core).w("A", haloed).c("B", srgb(dark)).out("Result");
    (speckled, core)
}

/// The glaze running thicker and thinner: `thin` where it ran off, `thick`
/// where it pooled, over a few centimetres.
fn glaze_depth(r: &mut Recipe, base: &str, thin: &str, thick: &str, amount: f32) -> Out {
    let flow = noise(r, 9.0, 4.0, 0.55);
    let thick_share = r.smooth(flow, (0.42, 0.72), (0.0, amount));
    let thin_share = r.smooth(flow, (0.28, 0.48), (amount * 0.7, 0.0));
    let thinned = r.node("mix").f("Factor", thin_share).c("A", srgb(base)).c("B", srgb(thin)).out("Result");
    r.node("mix").f("Factor", thick_share).w("A", thinned).c("B", srgb(thick)).out("Result")
}

/// The faint waviness a glaze levels to, and its odd pinhole.
fn glaze_surface(r: &mut Recipe, depth: f32) {
    let wave = noise(r, 28.0, 3.0, 0.45);
    let ripple = noise(r, 140.0, 2.0, 0.5);
    let height = plus(r, ripple, 0.25, wave);
    r.bump(height, depth);
}

fn glazed_white() -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = srgb("#F1F1EE");
        m.coat_weight = 1.0;
        m.coat_roughness = 0.015;
        m.specular_roughness = 0.12;
    }));
    let colour = glaze_depth(&mut r, "#F1F1EE", "#F4F3EF", "#E6E9EA", 0.6);
    r.to(Param::BaseColor, colour);
    glaze_surface(&mut r, 1.6e-5);
    preset("Ceramic Glazed White", "Ceramic", &["ceramic", "glaze", "sanitary", "white", "tableware", "gloss"], MaterialType::Generic, r)
}

fn matte() -> Preset {
    // A satin-matte glaze, never as dead as paint: a soft, wide sheen, the
    // clay's grain faintly through it, and the odd fleck.
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = srgb("#DCD7CC");
        m.base_diffuse_roughness = 0.45;
        m.specular_roughness = 0.55;
    }));
    let colour = glaze_depth(&mut r, "#DCD7CC", "#E2DDD3", "#D2CDC2", 0.5);
    let grain = noise(&mut r, 2200.0, 3.0, 0.6);
    let grain_tone = r.range(grain, (0.3, 0.7), (0.96, 1.04));
    let colour = r.mix("multiply", 1.0, colour, grain_tone);
    let (colour, _) = specks(&mut r, colour, 260.0, 0.06, "#6E5E4E", "#C4B7A4");
    r.to(Param::BaseColor, colour);
    let sheen = noise(&mut r, 14.0, 4.0, 0.55);
    let rough = r.smooth(sheen, (0.3, 0.7), (0.48, 0.62));
    r.to(Param::SpecularRoughness, rough);
    let tooth = noise(&mut r, 2600.0, 2.0, 0.5);
    let wave = noise(&mut r, 28.0, 3.0, 0.45);
    let height = plus(&mut r, tooth, 0.4, wave);
    r.bump(height, 8e-6);
    preset("Ceramic Matte", "Ceramic", &["ceramic", "bisque", "matte", "satin", "stoneware"], MaterialType::Generic, r)
}

fn stoneware() -> Preset {
    // Speckled stoneware: a cream glaze over an iron-rich clay, the iron
    // boiling up as specks with bled, browner halos, the glaze breaking
    // paler where it ran thin.
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = srgb("#E2D8C4");
        m.coat_weight = 0.85;
        m.coat_roughness = 0.06;
        m.specular_roughness = 0.25;
    }));
    let colour = glaze_depth(&mut r, "#E2D8C4", "#EBE3D2", "#D3C7AE", 0.8);
    let (colour, _) = specks(&mut r, colour, 180.0, 0.32, "#4E3A2A", "#B9A07F");
    let (colour, _) = specks(&mut r, colour, 520.0, 0.28, "#5A4636", "#C8B293");
    r.to(Param::BaseColor, colour);
    glaze_surface(&mut r, 2.4e-5);
    preset("Stoneware Speckled", "Ceramic", &["ceramic", "stoneware", "speckled", "glaze", "pottery", "cream"], MaterialType::Generic, r)
}

fn porcelain() -> Preset {
    // Fine white porcelain: light goes into it and comes back softened,
    // glowing at a thin edge, under a clear glaze.
    let mut r = Recipe::new(base(MaterialType::Translucent, |m| {
        m.base_color = rgb(0.9119, 0.9128, 0.8912);
        m.subsurface_color = rgb(0.9342, 0.9304, 0.9082);
        m.subsurface_weight = 0.75;
        m.subsurface_radius = rgb(0.03, 0.026, 0.02);
        m.specular_roughness = 0.03;
    }));
    let colour = glaze_depth(&mut r, "#F3F2EC", "#F5F4EF", "#EAEDEE", 0.5);
    r.to(Param::BaseColor, colour);
    r.to(Param::SubsurfaceColor, colour);
    glaze_surface(&mut r, 1.0e-5);
    preset("Porcelain", "Ceramic", &["porcelain", "ceramic", "china", "translucent", "white"], MaterialType::Translucent, r)
}

fn crackle() -> Preset {
    // Celadon crackle: a pale grey-green glaze crazed as it cooled into a
    // network of fine cracks at two scales, the cracks stained darker.
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = srgb("#BFCCBC");
        m.coat_weight = 1.0;
        m.coat_roughness = 0.02;
        m.specular_roughness = 0.15;
    }));
    let colour = glaze_depth(&mut r, "#BFCCBC", "#C9D4C6", "#A8BBA9", 0.8);
    let coords = r.node("texture_coordinate").out("Object");
    let nick = r.node("noise_texture").w("Vector", coords).f("Scale", 120.0).f("Detail", 3.0).out("Color");
    let nicked = r.node("vector_math").set("operation", "multiply_add").w("Vector", nick).v("Vector 2", [0.0015; 3]).w("Vector 3", coords).out("Vector");
    let mut colour = colour;
    let mut cracks = None;
    for (scale, width, strength) in [(70.0f32, 0.006f32, 0.55f32), (210.0, 0.01, 0.35)] {
        let edge = r.node("voronoi_texture").set("feature", "distance_to_edge").w("Vector", nicked).f("Scale", scale).f("Randomness", 1.0).out("Distance");
        let line = r.smooth(edge, (0.0, width), (strength, 0.0));
        colour = r.node("mix").f("Factor", line).w("A", colour).c("B", srgb("#5E6B5E")).out("Result");
        cracks = Some(match cracks {
            None => line,
            Some(other) => r.math("maximum", other, line),
        });
    }
    r.to(Param::BaseColor, colour);
    let cracks = cracks.expect("two scales");
    let dip = r.math("multiply", cracks, -1.0);
    let wave = noise(&mut r, 28.0, 3.0, 0.45);
    let height = plus(&mut r, wave, 0.2, dip);
    r.bump(height, 6e-6);
    preset("Ceramic Crackle Celadon", "Ceramic", &["ceramic", "crackle", "craquelure", "celadon", "glaze", "green"], MaterialType::Generic, r)
}

fn reactive_blue() -> Preset {
    // A reactive glaze: deep blue breaking to a paler, mottled blue where it
    // ran thin, in flows down the piece, with fine crystals and specks.
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = srgb("#1E3556");
        m.coat_weight = 1.0;
        m.coat_roughness = 0.03;
        m.specular_roughness = 0.2;
    }));
    let coords = r.node("texture_coordinate").out("Object");
    // Flows run down: drawn out along y.
    let down = r.node("mapping").w("Vector", coords).v("Scale", [1.0, 0.25, 1.0]).out("Vector");
    let flow = r.node("noise_texture").w("Vector", down).f("Scale", 30.0).f("Detail", 8.0).f("Roughness", 0.62).f("Distortion", 0.5).out("Fac");
    let colour = r.ramp(flow, Linear, &[
        (0.3, srgb("#14253E")),
        (0.45, srgb("#1E3556")),
        (0.58, srgb("#36577F")),
        (0.7, srgb("#6F8FAE")),
        (0.8, srgb("#9DB3C4")),
    ]);
    let crystals = noise(&mut r, 900.0, 3.0, 0.6);
    let crystal_tone = r.range(crystals, (0.3, 0.7), (0.9, 1.12));
    let colour = r.mix("multiply", 1.0, colour, crystal_tone);
    let (colour, _) = specks(&mut r, colour, 300.0, 0.05, "#2A2018", "#5C6670");
    r.to(Param::BaseColor, colour);
    glaze_surface(&mut r, 2.0e-5);
    preset("Ceramic Reactive Blue", "Ceramic", &["ceramic", "reactive", "glaze", "blue", "pottery", "variegated"], MaterialType::Generic, r)
}
