//! Plastics: moulded, textured, rubbery, clear and cloudy.

use crate::{base, micro, noise, plain, plus, preset, rgb, srgb, wear, Out, Preset, Recipe};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::{MaterialType, Param};

/// A moulded part: the polish of its tool, a gloss that varies with
/// handling, and the tool's faint texture.
fn moulded(name: &'static str, tags: &'static [&'static str], colour: Spectrum, low: f32, high: f32) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Plastic, |m| {
        m.base_color = colour;
        m.specular_roughness = (low + high) * 0.5;
    }));
    let gloss = wear(&mut r, low, high, 30.0);
    r.to(Param::SpecularRoughness, gloss);
    micro(&mut r, 2600.0, 3e-6);
    preset(name, "Plastic", tags, MaterialType::Plastic, r)
}

/// Diamond knurling: two sets of grooves at right angles, `scale` grooves
/// a metre. Low in the grooves, high on the diamonds' peaks.
pub fn knurl(r: &mut Recipe, scale: f32) -> Out {
    let coords = r.node("texture_coordinate").out("Object");
    let a = r.node("mapping").w("Vector", coords).v("Rotation", [0.0, 0.0, 0.785]).out("Vector");
    let b = r.node("mapping").w("Vector", coords).v("Rotation", [0.0, 0.0, -0.785]).out("Vector");
    let ga = r.node("wave_texture").set("profile", "tri").w("Vector", a).f("Scale", scale).out("Fac");
    let gb = r.node("wave_texture").set("profile", "tri").w("Vector", b).f("Scale", scale).out("Fac");
    r.math("minimum", ga, gb)
}

pub fn all() -> Vec<Preset> {
    let mut out = vec![
        moulded("ABS White", &["abs", "injection", "enclosure", "white"], rgb(0.8535, 0.8546, 0.827), 0.24, 0.32),
        moulded("ABS Black", &["abs", "injection", "enclosure", "black"], rgb(0.0092, 0.0092, 0.0102), 0.25, 0.34),
        moulded("ABS Grey", &["abs", "injection", "grey"], rgb(0.2118, 0.2155, 0.2221), 0.28, 0.36),
        moulded("ABS Blue", &["abs", "blue", "toy"], rgb(0.0441, 0.062, 0.3565), 0.2, 0.28),
        moulded("ABS Yellow", &["abs", "yellow", "toy"], rgb(0.735, 0.536, 0.0813), 0.2, 0.28),
        moulded("PVC Grey", &["pvc", "pipe", "industrial"], rgb(0.3447, 0.3506, 0.3347), 0.34, 0.44),
        moulded("Plastic Orange", &["abs", "orange", "safety", "toy"], srgb("#E8641E"), 0.18, 0.26),
        moulded("Plastic Mint", &["abs", "green", "pastel", "consumer"], srgb("#A8D8C0"), 0.22, 0.3),
        moulded("Plastic Sand", &["abs", "beige", "warm", "consumer"], srgb("#D8CDB8"), 0.26, 0.34),
    ];

    // A gloss moulding: a mirror finish with the gentle waviness every
    // moulded panel shows in its reflections.
    let mut r = Recipe::new(base(MaterialType::Plastic, |m| {
        m.base_color = rgb(0.3196, 0.0452, 0.023);
        m.specular_roughness = 0.08;
    }));
    let gloss = wear(&mut r, 0.05, 0.11, 25.0);
    r.to(Param::SpecularRoughness, gloss);
    let wave = noise(&mut r, 40.0, 2.0, 0.4);
    r.bump(wave, 1.5e-5);
    out.push(preset("ABS Red Gloss", "Plastic", &["abs", "red", "gloss", "toy"], MaterialType::Plastic, r));

    // Piano black: deep gloss, every waviness of the moulding on show.
    let mut r = Recipe::new(base(MaterialType::Plastic, |m| {
        m.base_color = srgb("#0B0B0C");
        m.specular_roughness = 0.03;
    }));
    let gloss = wear(&mut r, 0.02, 0.06, 18.0);
    r.to(Param::SpecularRoughness, gloss);
    let wave = noise(&mut r, 35.0, 2.0, 0.4);
    r.bump(wave, 1.2e-5);
    out.push(preset("Piano Black", "Plastic", &["gloss", "black", "trim", "electronics"], MaterialType::Plastic, r));

    // A texture-moulded enclosure: a stipple of fine dimples, glossier on
    // the tops the hand wears.
    let mut r = Recipe::new(base(MaterialType::Plastic, |m| {
        m.base_color = rgb(0.0424, 0.0423, 0.0448);
        m.specular_roughness = 0.62;
    }));
    let cells = r.node("voronoi_texture").f("Scale", 3400.0).f("Randomness", 0.9).out("Distance");
    let grain = noise(&mut r, 9000.0, 1.0, 0.5);
    let height = plus(&mut r, grain, 0.35, cells);
    r.bump(height, 1.4e-5);
    let gloss = r.range(cells, (0.0, 0.6), (0.52, 0.7));
    r.to(Param::SpecularRoughness, gloss);
    out.push(preset("ABS Textured", "Plastic", &["abs", "grained", "stipple", "enclosure"], MaterialType::Plastic, r));

    // A leather-grain moulding, as on dashboards and camera bodies.
    let mut r = Recipe::new(base(MaterialType::Plastic, |m| {
        m.base_color = srgb("#1B1B1C");
        m.specular_roughness = 0.55;
    }));
    let cells = r.node("voronoi_texture").set("feature", "smooth_f1").f("Scale", 900.0).f("Smoothness", 0.4).out("Distance");
    let folds = r.node("noise_texture").set("type", "ridged_multifractal").f("Scale", 140.0).f("Detail", 4.0).f("Offset", 1.0).f("Gain", 2.0).out("Fac");
    let height = plus(&mut r, folds, -0.25, cells);
    r.bump(height, 3e-5);
    let gloss = r.range(cells, (0.0, 0.7), (0.42, 0.62));
    r.to(Param::SpecularRoughness, gloss);
    out.push(preset("Plastic Leather Grain", "Plastic", &["grained", "leatherette", "automotive", "interior"], MaterialType::Plastic, r));

    // Soft touch: a rubbery coating, flat and velvety, with a fine tooth.
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = rgb(0.0081, 0.008, 0.009);
        m.specular_roughness = 0.62;
        m.specular_weight = 0.75;
    }));
    let gloss = wear(&mut r, 0.56, 0.68, 20.0);
    r.to(Param::SpecularRoughness, gloss);
    micro(&mut r, 6000.0, 2.5e-6);
    out.push(preset("Soft Touch Black", "Plastic", &["soft touch", "rubberised", "matte", "black"], MaterialType::Generic, r));

    // Sintered nylon, as printed: grainy and dry.
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = rgb(0.0131, 0.013, 0.0143);
        m.base_diffuse_roughness = 0.4;
        m.specular_roughness = 0.72;
    }));
    let grains = r.node("voronoi_texture").f("Scale", 7000.0).out("Distance");
    r.bump(grains, 1.2e-5);
    out.push(preset("Nylon Black", "Plastic", &["nylon", "sintered", "3d print", "matte"], MaterialType::Generic, r));

    // Rubber: dull, with a fine tooth and the odd shinier patch.
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = rgb(0.0061, 0.0061, 0.0065);
        m.base_diffuse_roughness = 0.6;
        m.specular_ior = 1.52;
        m.specular_roughness = 0.8;
    }));
    let gloss = wear(&mut r, 0.7, 0.86, 18.0);
    r.to(Param::SpecularRoughness, gloss);
    micro(&mut r, 4500.0, 6e-6);
    out.push(preset("Rubber Black", "Plastic", &["rubber", "epdm", "seal", "grip"], MaterialType::Generic, r));

    // A knurled rubber grip: raised diamonds.
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = srgb("#1E1E20");
        m.base_diffuse_roughness = 0.5;
        m.specular_roughness = 0.62;
    }));
    let k = knurl(&mut r, 260.0);
    r.bump(k, 3e-4);
    out.push(preset("Rubber Grip", "Plastic", &["rubber", "grip", "knurled", "handle"], MaterialType::Generic, r));

    // The clear and cloudy plastics keep their measured optics.
    out.push(plain("Acrylic Blue Tint", "Plastic", &["acrylic", "pmma", "blue", "tint"], MaterialType::PlasticTransparent, |m| {
        m.specular_ior = 1.49;
        m.transmission_depth = 0.01;
        m.transmission_color = rgb(0.4609, 0.5865, 0.8558);
    }));
    out.push(plain("Polycarbonate Clear", "Plastic", &["pc", "clear", "lens"], MaterialType::PlasticTransparent, |m| {
        m.specular_ior = 1.585;
        m.specular_roughness = 0.02;
        m.transmission_color = rgb(0.9619, 0.972, 0.9579);
    }));
    out.push(plain("Polycarbonate Red Lens", "Plastic", &["polycarbonate", "lens", "tail light", "red"], MaterialType::PlasticTransparent, |m| {
        m.specular_ior = 1.585;
        m.transmission_depth = 0.003;
        // A tail light passes nearly all of its red and almost nothing else:
        // a deep, clear red, not the near-black a weaker red channel made.
        m.transmission_color = rgb(0.86, 0.035, 0.02);
    }));
    let mut r = Recipe::new(base(MaterialType::PlasticCloudy, |m| {
        m.base_color = rgb(0.92, 0.9211, 0.8923);
        m.specular_roughness = 0.44;
        m.subsurface_radius = rgb(0.155, 0.142, 0.117);
    }));
    let gloss = wear(&mut r, 0.38, 0.5, 20.0);
    r.to(Param::SpecularRoughness, gloss);
    out.push(preset("HDPE Milk White", "Plastic", &["hdpe", "translucent", "bottle"], MaterialType::PlasticCloudy, r));
    let mut r = Recipe::new(base(MaterialType::PlasticCloudy, |m| {
        m.base_color = rgb(0.7973, 0.7914, 0.7259);
        m.specular_roughness = 0.36;
        m.subsurface_radius = rgb(0.078, 0.066, 0.053);
    }));
    let gloss = wear(&mut r, 0.3, 0.42, 20.0);
    r.to(Param::SpecularRoughness, gloss);
    out.push(preset("Polypropylene Natural", "Plastic", &["pp", "translucent", "living hinge"], MaterialType::PlasticCloudy, r));
    out.push(plain("Silicone Translucent", "Plastic", &["silicone", "soft", "seal"], MaterialType::Translucent, |m| {
        m.base_color = rgb(0.8201, 0.8082, 0.7804);
        m.specular_roughness = 0.3;
        m.subsurface_color = rgb(0.9225, 0.8771, 0.8589);
        // The five centimetres it was tuned at as a profile width, as a
        // mean free path read the way Cycles reads it.
        m.subsurface_radius = rgb(0.647, 0.656, 0.662);
    }));
    out
}
