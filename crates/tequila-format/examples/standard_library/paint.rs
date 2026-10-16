//! Paints and coatings: car finishes, enamels, powder coats, primers.

use crate::{base, micro, orange_peel, plain, preset, rgb, srgb, wear, Preset, Recipe};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::{MaterialType, Param};

/// A car's gloss finish: colour under a clear coat, with the fine orange
/// peel every sprayed finish has, seen in the reflections' edges.
fn gloss(name: &'static str, tags: &'static [&'static str], colour: Spectrum, roughness: f32) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Paint, |m| {
        m.base_color = colour;
        m.specular_roughness = roughness;
        m.coat_roughness = 0.02;
    }));
    orange_peel(&mut r, 520.0, 5e-6);
    let coat = wear(&mut r, 0.012, 0.035, 20.0);
    r.to(Param::CoatRoughness, coat);
    preset(name, "Paint", tags, MaterialType::Paint, r)
}

/// A metallic finish: flakes in the colour coat, a clear coat over them.
#[allow(clippy::too_many_arguments)]
fn metallic(name: &'static str, tags: &'static [&'static str], colour: Spectrum, flakes: Spectrum, weight: f32, size: f32, spread: f32, roughness: f32) -> Preset {
    let mut r = Recipe::new(base(MaterialType::MetallicPaint, |m| {
        m.base_color = colour;
        m.specular_roughness = roughness;
        m.coat_roughness = 0.02;
        m.flake_weight = weight;
        m.flake_color = flakes;
        m.flake_size = size;
        m.flake_spread = spread;
        m.flake_roughness = 0.18;
    }));
    orange_peel(&mut r, 520.0, 5e-6);
    preset(name, "Paint", tags, MaterialType::MetallicPaint, r)
}

/// A powder coat: thick, cured, with a pronounced orange peel.
fn powder(name: &'static str, tags: &'static [&'static str], colour: Spectrum, low: f32, high: f32) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Plastic, |m| {
        m.base_color = colour;
        m.specular_roughness = (low + high) * 0.5;
    }));
    let peel = orange_peel(&mut r, 260.0, 2.2e-5);
    // The tops of the peel catch the light a touch more.
    let gloss = r.range(peel, (0.3, 0.7), (high, low));
    r.to(Param::SpecularRoughness, gloss);
    preset(name, "Paint", tags, MaterialType::Plastic, r)
}

pub fn all() -> Vec<Preset> {
    let mut out = vec![
        gloss("Automotive Red", &["car", "gloss", "clearcoat", "red"], rgb(0.2426, 0.0332, 0.0167), 0.22),
        gloss("Automotive Blue", &["car", "gloss", "clearcoat", "blue"], rgb(0.0248, 0.0345, 0.1673), 0.22),
        gloss("Automotive White", &["car", "gloss", "clearcoat", "white"], rgb(0.9055, 0.9049, 0.9194), 0.2),
        gloss("Automotive Racing Green", &["car", "gloss", "clearcoat", "green", "classic"], srgb("#0E3B2A"), 0.22),
        gloss("Automotive Yellow", &["car", "gloss", "clearcoat", "yellow"], srgb("#F2B80A"), 0.2),
        metallic("Automotive Black Metallic", &["car", "metallic", "flake", "black"], rgb(0.0081, 0.0081, 0.01), rgb(0.5514, 0.5504, 0.5761), 0.3, 4e-5, 0.35, 0.24),
        metallic("Automotive Silver Metallic", &["car", "metallic", "flake", "silver"], rgb(0.4506, 0.4559, 0.4761), rgb(0.9205, 0.9201, 0.9287), 0.75, 5e-5, 0.25, 0.26),
        metallic("Automotive Blue Metallic", &["car", "metallic", "flake", "blue"], srgb("#0D2A66"), srgb("#8FB0E8"), 0.45, 4.5e-5, 0.3, 0.24),
        metallic("Automotive Red Metallic", &["car", "metallic", "flake", "red", "candy"], srgb("#6E0A0E"), srgb("#E86A5A"), 0.4, 4.5e-5, 0.3, 0.24),
        metallic("Automotive Pearl White", &["car", "pearl", "flake", "white"], srgb("#ECEBE6"), srgb("#F4F0FF"), 0.35, 3e-5, 0.4, 0.2),
        metallic("Automotive Champagne", &["car", "metallic", "flake", "gold", "beige"], srgb("#8C7656"), srgb("#E8D6B0"), 0.6, 5e-5, 0.3, 0.25),
        powder("Powder Coat Grey", &["powder", "satin", "industrial", "grey"], rgb(0.1647, 0.1679, 0.1737), 0.45, 0.6),
        powder("Powder Coat Matte Black", &["powder", "matte", "industrial", "black"], rgb(0.0104, 0.0103, 0.0114), 0.7, 0.84),
        powder("Powder Coat White", &["powder", "satin", "industrial", "white"], srgb("#EDEDEA"), 0.4, 0.55),
        powder("Powder Coat Signal Red", &["powder", "satin", "industrial", "red", "machinery"], srgb("#B3191C"), 0.4, 0.55),
    ];

    // Gloss enamel: an appliance's hard white, its peel almost levelled.
    let mut r = Recipe::new(base(MaterialType::Paint, |m| {
        m.base_color = rgb(0.956, 0.956, 0.956);
        m.specular_roughness = 0.08;
        m.coat_roughness = 0.01;
    }));
    orange_peel(&mut r, 400.0, 3e-6);
    out.push(preset("Enamel Gloss White", "Paint", &["enamel", "appliance", "gloss", "white"], MaterialType::Paint, r));

    // A satin furniture finish.
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = rgb(0.0181, 0.0229, 0.0591);
        m.coat_weight = 0.3;
        m.specular_roughness = 0.42;
    }));
    let gloss = wear(&mut r, 0.36, 0.48, 15.0);
    r.to(Param::SpecularRoughness, gloss);
    micro(&mut r, 1200.0, 3e-6);
    out.push(preset("Paint Satin Navy", "Paint", &["satin", "furniture", "navy", "blue"], MaterialType::Generic, r));

    // Flat paint: dry and chalky, but no paint is without a sheen - a very
    // rough one, which is all a matte black has to show its shape by, and
    // the reason a flat black part never photographs as a hole.
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = srgb("#1E1E20");
        m.base_diffuse_roughness = 0.6;
        m.specular_roughness = 0.72;
    }));
    let gloss = wear(&mut r, 0.66, 0.78, 14.0);
    r.to(Param::SpecularRoughness, gloss);
    micro(&mut r, 2500.0, 4e-6);
    out.push(preset("Paint Matte Black", "Paint", &["matte", "flat", "black", "chalky"], MaterialType::Generic, r));

    // Primer: a soft, sanded matte with a faint sheen.
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = rgb(0.2966, 0.2963, 0.3038);
        m.base_diffuse_roughness = 0.5;
        m.specular_roughness = 0.78;
    }));
    let gloss = wear(&mut r, 0.72, 0.84, 16.0);
    r.to(Param::SpecularRoughness, gloss);
    micro(&mut r, 3000.0, 5e-6);
    out.push(preset("Primer Grey", "Paint", &["primer", "matte", "prototype", "grey"], MaterialType::Generic, r));

    // Paint on top of a texture: a rough, sandy finish for tools and housings.
    let mut r = Recipe::new(base(MaterialType::Plastic, |m| {
        m.base_color = srgb("#2B2E33");
        m.specular_roughness = 0.6;
    }));
    let sand = r.node("voronoi_texture").f("Scale", 1600.0).f("Randomness", 1.0).out("Distance");
    r.bump(sand, 3e-5);
    out.push(preset("Paint Textured Charcoal", "Paint", &["textured", "wrinkle", "industrial", "charcoal"], MaterialType::Plastic, r));

    out.push(plain("Paint Satin White", "Paint", &["satin", "wall", "white"], MaterialType::Plastic, |m| {
        m.base_color = srgb("#F0EFEA");
        m.specular_roughness = 0.45;
    }));
    out
}
