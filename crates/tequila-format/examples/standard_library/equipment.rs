//! What is on and in the gear besides its box: the lights that show it is
//! on, the rubber it stands on and is pressed through, the loudspeaker,
//! and what a cabinet is covered in.
//!
//! Published numbers where there are any. Indices of refraction: silicone
//! rubber 1.41, natural and butyl rubber 1.52, plasticised PVC 1.53, a
//! polyurethane elastomer 1.5, polypropylene 1.49, the epoxy of an LED's
//! lens 1.55, a display's cover glass 1.52. An indicator's colour is its
//! die's: red about 625 nm, amber 590, the old yellow-green 570, the
//! newer green 525, blue 470.
//!
//! What is lit is bright by the picture and not by the lamp: a picture's
//! white is a luminance of about one, so an indicator at a hundred, which
//! is nearer what it measures, is a white hole in the picture with its
//! colour gone. These are a little over white at their brightest, as a
//! photograph of gear under studio lights shows them, and keep their
//! colour. To have one light what is round it, raise its Luminance.

use crate::fabric::{woven, Cloth, Pattern};
use crate::recipe::Out;
use crate::{base, noise, plain, plus, preset, rgb, srgb, wear, Preset, Recipe};
use tequila_core::spectrum::Spectrum;
use tequila_materials::groups::NodeGroup;
use tequila_materials::material_type::{MaterialType, Param};

// ---- lit ----------------------------------------------------------------------------

/// An LED lit, seen through its own diffusing lens: brightest where the
/// lens faces the eye, which is over the die, and falling away to the
/// rim. The lens is still polished plastic, and shows the room on it.
fn indicator(name: &'static str, tags: &'static [&'static str], colour: Spectrum, luminance: f32) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = colour * 0.25;
        m.specular_ior = 1.55;
        m.specular_roughness = 0.1;
        m.emission_color = colour;
        m.emission_luminance = luminance;
        m.edge_radius = 2e-4;
    }));
    let facing = r.node("layer_weight").f("Blend", 0.5).out("Facing");
    let glow = r.smooth(facing, (0.1, 0.9), (luminance, luminance * 0.45));
    r.to(Param::EmissionLuminance, glow);
    preset(name, "Light", tags, MaterialType::Generic, r)
}

/// Plastic lit from behind or within: a light guide's end, a legend, a
/// button's cap. Even, but for being a little brighter here than there,
/// as the light inside is nearer here than there; `gloss` is how rough
/// the face is and `frost` how deep its tooth, metres.
fn glowing(name: &'static str, tags: &'static [&'static str], colour: Spectrum, luminance: f32, ior: f32, gloss: f32, frost: f32) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = colour * 0.35;
        m.specular_ior = ior;
        m.specular_roughness = gloss;
        m.emission_color = colour;
        m.emission_luminance = luminance;
        m.edge_radius = 2.5e-4;
    }));
    let uneven = noise(&mut r, 45.0, 2.0, 0.5);
    let glow = r.smooth(uneven, (0.25, 0.75), (luminance * 0.86, luminance * 1.1));
    r.to(Param::EmissionLuminance, glow);
    if frost > 0.0 {
        let tooth = noise(&mut r, 5600.0, 2.0, 0.5);
        r.bump(tooth, frost);
    }
    preset(name, "Light", tags, MaterialType::Generic, r)
}

/// A display that makes its own light, under its glass: the glass's
/// polish over an even glow.
fn display(name: &'static str, tags: &'static [&'static str], colour: Spectrum, luminance: f32) -> Preset {
    plain(name, "Light", tags, MaterialType::Generic, move |m| {
        m.base_color = rgb(0.004, 0.004, 0.005);
        m.specular_ior = 1.52;
        m.specular_roughness = 0.03;
        m.emission_color = colour;
        m.emission_luminance = luminance;
    })
}

/// An LED's lens unlit: tinted epoxy with a diffuser stirred in, milky
/// and a little deep.
fn lens(name: &'static str, tags: &'static [&'static str], colour: Spectrum) -> Preset {
    plain(name, "Plastic", tags, MaterialType::Generic, move |m| {
        m.base_color = colour;
        m.specular_ior = 1.55;
        m.specular_roughness = 0.08;
        m.subsurface_weight = 0.7;
        m.subsurface_color = colour;
        m.subsurface_radius = rgb(0.0012, 0.0012, 0.0012);
        m.edge_radius = 2e-4;
    })
}

// ---- rubber, soft plastic, foam -------------------------------------------------------

/// Dust: pale specks on `colour`, on `amount` of the places one could be.
/// Rubber and silicone hold it, and a clean black one looks computed.
fn dusty(r: &mut Recipe, colour: Spectrum, amount: f32) {
    let specks = r.group(NodeGroup::Speckle, &[("Size (mm)", 0.22), ("Amount", amount), ("Seed", 7.0)]);
    let speck = r.of(specks, "Speck");
    let some = r.math("multiply", speck, 0.45);
    let dusted = r.node("mix").f("Factor", some).c("A", colour).c("B", rgb(0.42, 0.41, 0.39)).out("Result");
    r.to(Param::BaseColor, dusted);
}

/// A moulded soft part: `ior` its material's, `gloss` its sheen from the
/// most polished to the least, `grain` the width in millimetres of the
/// mould's grain and `depth` its height in metres, `dust` how dusty.
#[allow(clippy::too_many_arguments)]
fn moulded(name: &'static str, category: &'static str, tags: &'static [&'static str], colour: Spectrum, ior: f32, gloss: (f32, f32), grain: f32, depth: f32, dust: f32) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = colour;
        m.base_diffuse_roughness = 0.5;
        m.specular_ior = ior;
        m.specular_roughness = (gloss.0 + gloss.1) * 0.5;
        m.edge_radius = 3e-4;
    }));
    let sheen = wear(&mut r, gloss.0, gloss.1, 30.0);
    r.to(Param::SpecularRoughness, sheen);
    let skin = r.group(NodeGroup::MouldTexture, &[("Grain (mm)", grain), ("Unevenness", 0.4)]);
    let skin = r.of(skin, "Height");
    r.bump(skin, depth);
    if dust > 0.0 {
        dusty(&mut r, colour, dust);
    }
    preset(name, category, tags, MaterialType::Generic, r)
}

/// Silicone rubber, as a keypad or a button is moulded of: matte with a
/// soft sheen, and light going a millimetre or so into it, `translucent`
/// of it.
fn silicone(name: &'static str, tags: &'static [&'static str], colour: Spectrum, translucent: f32) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = colour;
        m.base_diffuse_roughness = 0.5;
        m.specular_ior = 1.41;
        m.specular_roughness = 0.58;
        m.subsurface_weight = translucent;
        m.subsurface_color = colour;
        m.subsurface_radius = rgb(0.0015, 0.0015, 0.0015);
        m.edge_radius = 4e-4;
    }));
    let sheen = wear(&mut r, 0.5, 0.66, 40.0);
    r.to(Param::SpecularRoughness, sheen);
    let tooth = noise(&mut r, 9000.0, 2.0, 0.5);
    r.bump(tooth, 2e-6);
    dusty(&mut r, colour, 0.07);
    preset(name, "Plastic", tags, MaterialType::Generic, r)
}

/// A rubber tread: a pattern `depth` metres high moulded into a mat, its
/// tops scuffed a little shinier than what is between them.
fn tread(name: &'static str, tags: &'static [&'static str], depth: f32, height: impl FnOnce(&mut Recipe) -> Out) -> Preset {
    let colour = rgb(0.0075, 0.0075, 0.008);
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = colour;
        m.base_diffuse_roughness = 0.6;
        m.specular_ior = 1.52;
        m.specular_roughness = 0.7;
        m.edge_radius = 3e-4;
    }));
    let pattern = height(&mut r);
    let skin = noise(&mut r, 5000.0, 2.0, 0.5);
    let both = plus(&mut r, skin, 0.03, pattern);
    r.bump(both, depth);
    let sheen = r.range(pattern, (0.0, 1.0), (0.76, 0.58));
    r.to(Param::SpecularRoughness, sheen);
    dusty(&mut r, colour, 0.08);
    preset(name, "Plastic", tags, MaterialType::Generic, r)
}

/// Open foam: `cells` to the metre, each a dark hole between pale
/// struts, and no shine to speak of.
fn foam(name: &'static str, tags: &'static [&'static str], colour: Spectrum, cells: f32, depth: f32) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = colour;
        m.base_diffuse_roughness = 1.0;
        m.specular_weight = 0.25;
        m.specular_roughness = 0.85;
    }));
    let cell = r.node("voronoi_texture").f("Scale", cells).f("Randomness", 1.0).out("Distance");
    let hole = r.smooth(cell, (0.1, 0.5), (-1.0, 0.0));
    let skin = noise(&mut r, cells * 0.2, 3.0, 0.55);
    let height = plus(&mut r, skin, 0.4, hole);
    r.bump(height, depth);
    let shade = r.range(hole, (-1.0, 0.0), (0.45, 1.0));
    let toned = r.node("mix").set("blend_type", "multiply").f("Factor", 1.0).c("A", colour).w("B", shade).out("Result");
    r.to(Param::BaseColor, toned);
    preset(name, "Plastic", tags, MaterialType::Generic, r)
}

// ---- the loudspeaker ------------------------------------------------------------------

/// A paper cone: pulp pressed in a mould, its fibres lying every way.
/// `gloss` is its sheen: a bare cone has next to none, a coated one some.
fn cone_paper(name: &'static str, tags: &'static [&'static str], colour: Spectrum, gloss: (f32, f32), weight: f32) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = colour;
        m.base_diffuse_roughness = 0.8;
        m.specular_ior = 1.5;
        m.specular_weight = weight;
        m.specular_roughness = (gloss.0 + gloss.1) * 0.5;
        m.fuzz_weight = 0.2;
        m.fuzz_color = colour * 2.0;
        m.fuzz_roughness = 0.6;
    }));
    let fibres = r.node("noise_texture").f("Scale", 2200.0).f("Detail", 7.0).f("Roughness", 0.65).f("Distortion", 1.4).out("Fac");
    let clumps = noise(&mut r, 300.0, 4.0, 0.55);
    let height = plus(&mut r, clumps, 0.5, fibres);
    r.bump(height, 2.5e-5);
    let tone = r.range(fibres, (0.3, 0.7), (0.8, 1.0));
    let toned = r.node("mix").set("blend_type", "multiply").f("Factor", 1.0).c("A", colour).w("B", tone).out("Result");
    r.to(Param::BaseColor, toned);
    let sheen = wear(&mut r, gloss.0, gloss.1, 25.0);
    r.to(Param::SpecularRoughness, sheen);
    preset(name, "Other", tags, MaterialType::Generic, r)
}

// ---- cabinets -------------------------------------------------------------------------

/// The vinyl a cabinet is covered in: cloth coated with PVC and embossed
/// with a grain, `pebbles` to the metre and `depth` metres deep, glossier
/// on the tops of the grain than between.
pub(crate) fn vinyl(name: &'static str, tags: &'static [&'static str], colour: Spectrum, pebbles: f32, depth: f32) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = colour;
        m.base_diffuse_roughness = 0.4;
        m.specular_ior = 1.53;
        m.specular_roughness = 0.42;
    }));
    let coords = r.node("texture_coordinate").out("Object");
    let wander = r.node("noise_texture").w("Vector", coords).f("Scale", pebbles * 0.9).f("Detail", 2.0).out("Color");
    let reach = 1.6 / pebbles;
    let warped = r.node("vector_math").set("operation", "multiply_add").w("Vector", wander).v("Vector 2", [reach, reach, reach]).w("Vector 3", coords).out("Vector");
    let edge = r.node("voronoi_texture").set("feature", "distance_to_edge").w("Vector", warped).f("Scale", pebbles).f("Randomness", 1.0).out("Distance");
    let groove = r.smooth(edge, (0.0, 0.4), (0.0, 1.0));
    let centre = r.node("voronoi_texture").w("Vector", warped).f("Scale", pebbles).f("Randomness", 1.0).out("Distance");
    let dome = r.smooth(centre, (0.0, 0.8), (1.0, 0.8));
    let pebble = r.math("multiply", groove, dome);
    let fine = noise(&mut r, pebbles * 6.0, 2.0, 0.5);
    let height = plus(&mut r, fine, 0.08, pebble);
    r.bump(height, depth);
    let sheen = r.range(pebble, (0.0, 1.0), (0.62, 0.44));
    r.to(Param::SpecularRoughness, sheen);
    let shade = r.range(pebble, (0.0, 1.0), (0.9, 1.0));
    let toned = r.node("mix").set("blend_type", "multiply").f("Factor", 1.0).c("A", colour).w("B", shade).out("Result");
    r.to(Param::BaseColor, toned);
    preset(name, "Leather", tags, MaterialType::Generic, r)
}

/// The carpet a touring cabinet is covered in: fibres needled into a
/// felt, coarser and wirier than a felt for a desk.
fn carpet(name: &'static str, tags: &'static [&'static str], dark: Spectrum, light: Spectrum) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Velvet, |m| {
        m.base_color = dark.lerp(light, 0.5);
        m.fuzz_color = light * 2.2;
        m.fuzz_roughness = 0.8;
    }));
    let fibres = r.node("noise_texture").f("Scale", 1500.0).f("Detail", 8.0).f("Roughness", 0.72).f("Distortion", 2.6).out("Fac");
    let tufts = noise(&mut r, 260.0, 4.0, 0.6);
    let height = plus(&mut r, tufts, 0.6, fibres);
    r.bump(height, 2.6e-4);
    let mottle = noise(&mut r, 45.0, 4.0, 0.5);
    let tone = plus(&mut r, fibres, 0.5, mottle);
    let colour = r.ramp(tone, crate::recipe::Linear, &[(0.4, dark), (1.2, light)]);
    // The wiry fibres that stand out of it, paler than the mat.
    let hairs = crate::fabric::loose_fibres(&mut r, 300.0, 0.035);
    let some = r.math("multiply", hairs, 0.55);
    let colour = r.node("mix").f("Factor", some).w("A", colour).c("B", light * 2.6).out("Result");
    r.to(Param::BaseColor, colour);
    preset(name, "Fabric", tags, MaterialType::Velvet, r)
}

/// The sprayed coating on a touring cabinet: a tough paint thrown on in
/// spatters that stand a tenth of a millimetre proud.
fn spatter_coat(name: &'static str, tags: &'static [&'static str], colour: Spectrum) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = colour;
        m.base_diffuse_roughness = 0.4;
        m.specular_ior = 1.5;
        m.specular_roughness = 0.6;
    }));
    let spatters = r.group(NodeGroup::Stucco, &[("Size (mm)", 1.1), ("Amount", 0.62), ("Thickness", 0.7)]);
    let spatters = r.of(spatters, "Height");
    let fine = noise(&mut r, 2600.0, 2.0, 0.5);
    let height = plus(&mut r, fine, 0.12, spatters);
    r.bump(height, 7e-5);
    let sheen = r.range(spatters, (0.0, 1.0), (0.66, 0.54));
    r.to(Param::SpecularRoughness, sheen);
    preset(name, "Paint", tags, MaterialType::Generic, r)
}

/// A cloth with the library's weave, said in few words.
#[allow(clippy::too_many_arguments)]
fn cloth(name: &'static str, tags: &'static [&'static str], pattern: Pattern, threads: f32, warp: Spectrum, weft: Spectrum, fill: f32, fuzz: f32, depth: f32, roughness: f32) -> Preset {
    woven(&Cloth { name, tags, pattern, threads, warp, weft, variation: 0.1, slub: 0.12, fill, fuzz, fuzz_colour: warp.lerp(weft, 0.5) * 2.5, heather: None, depth, roughness })
}

pub fn all() -> Vec<Preset> {
    // An indicator's colours: deeper than they are seen, since a light a
    // little over the picture's white is seen paler than its colour.
    let red = rgb(1.0, 0.012, 0.008);
    let amber = rgb(1.0, 0.27, 0.0);
    let yellow_green = rgb(0.36, 1.0, 0.0);
    let green = rgb(0.0, 1.0, 0.07);
    let blue = rgb(0.006, 0.08, 1.0);
    let white = rgb(0.92, 0.95, 1.0);
    let warm = rgb(1.0, 0.62, 0.3);

    let mut out = vec![
        indicator("LED Indicator Red", &["led", "indicator", "lit", "on", "status", "red", "pilot light"], red, 1.3),
        indicator("LED Indicator Amber", &["led", "indicator", "lit", "on", "status", "amber", "orange", "pilot light"], amber, 1.3),
        indicator("LED Indicator Yellow Green", &["led", "indicator", "lit", "on", "status", "green", "yellow green", "pilot light"], yellow_green, 1.3),
        indicator("LED Indicator Green", &["led", "indicator", "lit", "on", "status", "green", "pilot light"], green, 1.3),
        indicator("LED Indicator Blue", &["led", "indicator", "lit", "on", "status", "blue", "pilot light"], blue, 1.5),
        indicator("LED Indicator White", &["led", "indicator", "lit", "on", "status", "white", "pilot light"], white, 1.3),
        lens("LED Lens Red", &["led", "lens", "unlit", "off", "diffused", "epoxy", "red"], rgb(0.5, 0.03, 0.03)),
        lens("LED Lens Amber", &["led", "lens", "unlit", "off", "diffused", "epoxy", "amber", "orange"], rgb(0.62, 0.24, 0.03)),
        lens("LED Lens Green", &["led", "lens", "unlit", "off", "diffused", "epoxy", "green"], rgb(0.1, 0.42, 0.08)),
        lens("LED Lens White", &["led", "lens", "unlit", "off", "diffused", "epoxy", "white", "milky"], rgb(0.82, 0.83, 0.82)),
        glowing("Light Guide Lit White", &["light guide", "light pipe", "led", "lit", "on", "ring", "halo", "white"], white, 1.0, 1.49, 0.36, 3e-6),
        glowing("Light Guide Lit Red", &["light guide", "light pipe", "led", "lit", "on", "ring", "halo", "red"], red, 1.0, 1.49, 0.36, 3e-6),
        glowing("Light Guide Lit Amber", &["light guide", "light pipe", "led", "lit", "on", "ring", "halo", "amber", "orange"], amber, 1.0, 1.49, 0.36, 3e-6),
        glowing("Light Guide Lit Green", &["light guide", "light pipe", "led", "lit", "on", "ring", "halo", "green"], green, 1.0, 1.49, 0.36, 3e-6),
        glowing("Light Guide Lit Blue", &["light guide", "light pipe", "led", "lit", "on", "ring", "halo", "blue"], blue, 1.2, 1.49, 0.36, 3e-6),
        glowing("Button Backlit White", &["button", "backlit", "legend", "lit", "pad", "silicone", "white"], white, 0.8, 1.41, 0.5, 2e-6),
        glowing("Button Backlit Red", &["button", "backlit", "legend", "lit", "pad", "silicone", "red"], red, 0.8, 1.41, 0.5, 2e-6),
        glowing("Button Backlit Amber", &["button", "backlit", "legend", "lit", "pad", "silicone", "amber", "orange"], amber, 0.8, 1.41, 0.5, 2e-6),
        glowing("Button Backlit Green", &["button", "backlit", "legend", "lit", "pad", "silicone", "green"], green, 0.8, 1.41, 0.5, 2e-6),
        glowing("Button Backlit Blue", &["button", "backlit", "legend", "lit", "pad", "silicone", "blue"], blue, 0.9, 1.41, 0.5, 2e-6),
        glowing("Display Segment Red", &["display", "seven segment", "7 segment", "led display", "digit", "lit", "red"], red, 1.0, 1.55, 0.3, 0.0),
        glowing("Display Segment Green", &["display", "seven segment", "7 segment", "led display", "digit", "lit", "green"], green, 1.0, 1.55, 0.3, 0.0),
        glowing("Display Segment White", &["display", "seven segment", "7 segment", "led display", "digit", "lit", "white"], white, 1.0, 1.55, 0.3, 0.0),
        display("Display OLED White", &["display", "oled", "screen", "pixels", "lit", "white"], white, 0.8),
        display("Display OLED Yellow", &["display", "oled", "screen", "pixels", "lit", "yellow"], rgb(1.0, 0.7, 0.03), 0.8),
        display("Display OLED Blue", &["display", "oled", "screen", "pixels", "lit", "blue", "cyan"], rgb(0.05, 0.4, 1.0), 0.8),
        display("Display LCD Backlit Blue", &["display", "lcd", "screen", "backlit", "character display", "blue"], rgb(0.01, 0.04, 0.8), 0.6),
        display("Display LCD Backlit Green", &["display", "lcd", "screen", "backlit", "character display", "green", "yellow green"], rgb(0.36, 0.75, 0.01), 0.6),
        display("Display LCD Backlit White", &["display", "lcd", "screen", "backlit", "tft", "white"], white, 0.6),
        glowing("Meter Backlight Warm", &["meter", "vu meter", "backlight", "dial", "lit", "warm", "lamp"], warm, 0.8, 1.49, 0.4, 0.0),
        glowing("Valve Heater Glow", &["valve", "tube", "heater", "filament", "glow", "orange", "amplifier"], rgb(1.0, 0.22, 0.02), 0.8, 1.52, 0.5, 0.0),
    ];

    // A display off: black glass, and the grey-green of a liquid crystal
    // with no light behind it.
    out.push(plain("Display Glass Black", "Glass", &["display", "screen", "oled", "off", "black glass", "cover glass"], MaterialType::Generic, |m| {
        m.base_color = rgb(0.003, 0.003, 0.0035);
        m.specular_ior = 1.52;
        m.specular_roughness = 0.02;
    }));
    out.push(plain("Display LCD Unlit", "Glass", &["display", "lcd", "screen", "off", "reflective", "grey green", "segment"], MaterialType::Generic, |m| {
        m.base_color = srgb("#7C8473");
        m.specular_ior = 1.52;
        m.specular_roughness = 0.45;
        m.coat_weight = 1.0;
        m.coat_roughness = 0.03;
    }));
    // The window over a display: clear plastic smoked so what is not lit
    // behind it is not seen.
    out.push(plain("Display Window Smoked", "Plastic", &["display", "window", "lens", "smoked", "tinted", "acrylic", "bezel"], MaterialType::PlasticTransparent, |m| {
        m.specular_ior = 1.49;
        m.specular_roughness = 0.02;
        m.transmission_color = rgb(0.1, 0.1, 0.11);
        m.edge_radius = 2e-4;
    }));

    // ---- rubber, soft plastic, foam ---------------------------------------------------
    let black = rgb(0.0075, 0.0075, 0.008);
    out.extend([
        moulded("Rubber Foot Black", "Plastic", &["rubber", "foot", "feet", "bumper", "moulded", "black"], black, 1.52, (0.5, 0.66), 0.12, 5e-6, 0.08),
        moulded("Rubber Foot Grey", "Plastic", &["rubber", "foot", "feet", "bumper", "moulded", "grey"], srgb("#6A6C6E"), 1.52, (0.5, 0.66), 0.12, 5e-6, 0.0),
        moulded("TPE Overmould Black", "Plastic", &["tpe", "tpu", "overmould", "soft grip", "elastomer", "black"], rgb(0.01, 0.01, 0.0105), 1.5, (0.6, 0.74), 0.2, 8e-6, 0.08),
        moulded("TPE Overmould Grey", "Plastic", &["tpe", "tpu", "overmould", "soft grip", "elastomer", "grey"], srgb("#55585C"), 1.5, (0.6, 0.74), 0.2, 8e-6, 0.0),
        moulded("Cable Jacket Black", "Plastic", &["cable", "wire", "jacket", "pvc", "lead", "cord", "black"], rgb(0.008, 0.008, 0.0085), 1.53, (0.28, 0.4), 0.06, 1.5e-6, 0.0),
        moulded("Cable Jacket Grey", "Plastic", &["cable", "wire", "jacket", "pvc", "lead", "cord", "grey"], srgb("#77797B"), 1.53, (0.28, 0.4), 0.06, 1.5e-6, 0.0),
        moulded("Cable Jacket White", "Plastic", &["cable", "wire", "jacket", "pvc", "lead", "cord", "white"], srgb("#E6E6E2"), 1.53, (0.28, 0.4), 0.06, 1.5e-6, 0.0),
        moulded("Strain Relief Black", "Plastic", &["strain relief", "boot", "cable", "plug", "pvc", "moulded", "black"], rgb(0.009, 0.009, 0.0095), 1.53, (0.4, 0.54), 0.1, 4e-6, 0.05),
        silicone("Silicone Keypad Black", &["silicone", "keypad", "button", "pad", "rubber", "black"], rgb(0.011, 0.011, 0.0115), 0.0),
        silicone("Silicone Keypad Grey", &["silicone", "keypad", "button", "pad", "rubber", "grey"], srgb("#76797C"), 0.25),
        silicone("Silicone Keypad White", &["silicone", "keypad", "button", "pad", "rubber", "white"], srgb("#E4E3DE"), 0.5),
        silicone("Silicone Keypad Red", &["silicone", "keypad", "button", "pad", "rubber", "red"], srgb("#B5201C"), 0.3),
        silicone("Silicone Pad Translucent", &["silicone", "pad", "drum pad", "button", "backlit", "unlit", "milky", "translucent"], srgb("#D8D9D4"), 0.9),
        tread("Rubber Tread Ribbed", &["rubber", "tread", "ribbed", "mat", "pedal", "non slip", "black"], 5e-4, |r| {
            let ribs = r.group(NodeGroup::Stripes, &[("Width (mm)", 2.4), ("Share", 0.62), ("Angle", 0.0), ("Plane", 0.0)]);
            r.of(ribs, "Round")
        }),
        tread("Rubber Tread Diamond", &["rubber", "tread", "diamond", "mat", "pedal", "non slip", "black"], 4e-4, |r| {
            let points = r.group(NodeGroup::Knurl, &[("Pitch (mm)", 2.8), ("Angle", 30.0), ("Flat", 0.3), ("Plane", 0.0)]);
            r.of(points, "Height")
        }),
        tread("Rubber Tread Studded", &["rubber", "tread", "studs", "dots", "mat", "pedal", "non slip", "black"], 5e-4, |r| {
            let studs = r.group(NodeGroup::Dots, &[("Spacing (mm)", 3.6), ("Size", 0.6), ("Staggered", 1.0), ("Plane", 0.0)]);
            r.of(studs, "Dome")
        }),
        foam("Foam Windscreen Black", &["foam", "windscreen", "windshield", "microphone", "pop filter", "open cell", "black"], rgb(0.012, 0.012, 0.013), 2600.0, 2.2e-4),
        foam("Foam Open Cell Charcoal", &["foam", "open cell", "acoustic", "packing", "case", "charcoal", "grey"], srgb("#2E2F31"), 1500.0, 3.5e-4),
    ]);
    // The clear stick-on foot: soft polyurethane, not quite clear.
    out.push(plain("Bumper Foot Clear", "Plastic", &["bumper", "foot", "feet", "stick-on", "polyurethane", "clear", "dome"], MaterialType::PlasticTransparent, |m| {
        m.specular_ior = 1.5;
        m.specular_roughness = 0.14;
        m.transmission_color = rgb(0.88, 0.9, 0.88);
        m.edge_radius = 4e-4;
    }));
    out.push(cloth("Cable Braided Black", &["cable", "braided", "woven", "jacket", "instrument cable", "cloth", "black"], Pattern::Twill { over: 2, under: 2 }, 1800.0, srgb("#161617"), srgb("#222224"), 0.95, 0.2, 1.2e-4, 0.55));

    // ---- the loudspeaker --------------------------------------------------------------
    out.extend([
        cone_paper("Speaker Cone Paper Black", &["speaker", "cone", "paper", "pulp", "woofer", "driver", "black"], srgb("#1B1B1C"), (0.7, 0.82), 0.4),
        cone_paper("Speaker Cone Paper Grey", &["speaker", "cone", "paper", "pulp", "woofer", "driver", "grey"], srgb("#5A5855"), (0.72, 0.84), 0.35),
        cone_paper("Speaker Cone Paper White", &["speaker", "cone", "paper", "pulp", "woofer", "monitor", "white"], srgb("#DEDCD4"), (0.7, 0.82), 0.4),
        cone_paper("Speaker Cone Coated Black", &["speaker", "cone", "paper", "coated", "doped", "treated", "woofer", "black"], srgb("#161617"), (0.4, 0.52), 0.9),
        moulded("Speaker Cone Polypropylene Black", "Plastic", &["speaker", "cone", "polypropylene", "pp", "woofer", "driver", "black"], rgb(0.012, 0.012, 0.013), 1.49, (0.26, 0.36), 0.05, 1e-6, 0.0),
        moulded("Speaker Cone Polypropylene Grey", "Plastic", &["speaker", "cone", "polypropylene", "pp", "mineral filled", "woofer", "grey"], srgb("#44474B"), 1.49, (0.26, 0.36), 0.05, 1e-6, 0.0),
        moulded("Speaker Surround Rubber", "Plastic", &["speaker", "surround", "roll", "butyl", "rubber", "woofer", "black"], rgb(0.009, 0.009, 0.0095), 1.52, (0.44, 0.58), 0.08, 3e-6, 0.06),
        foam("Speaker Surround Foam", &["speaker", "surround", "roll", "foam", "woofer", "charcoal"], srgb("#2A2A2B"), 5200.0, 9e-5),
        cloth("Speaker Surround Cloth", &["speaker", "surround", "accordion", "cloth", "doped", "pa", "black"], Pattern::Plain, 2600.0, srgb("#151516"), srgb("#1B1B1C"), 0.97, 0.05, 4e-5, 0.5),
        cloth("Speaker Dome Silk Black", &["speaker", "tweeter", "dome", "silk", "soft dome", "fabric", "black"], Pattern::Plain, 5000.0, srgb("#121213"), srgb("#161617"), 0.98, 0.05, 1.2e-5, 0.42),
        crate::metal_pattern::spun("Speaker Cone Aluminium", &["speaker", "cone", "dome", "aluminium", "metal cone", "tweeter", "woofer"], "Aluminium", rgb(0.86, 0.87, 0.88), 0.42),
    ]);
    // A ferrite magnet: a dark sintered ceramic, dull, with a fine grain.
    out.push(moulded("Ferrite Magnet", "Ceramic", &["ferrite", "magnet", "speaker", "driver", "sintered", "ceramic", "dark grey"], srgb("#2C2C2F"), 1.9, (0.5, 0.62), 0.15, 6e-6, 0.0));

    // ---- cabinets ---------------------------------------------------------------------
    out.extend([
        vinyl("Cabinet Vinyl Black", &["cabinet", "amplifier", "vinyl", "leatherette", "covering", "levant", "black"], srgb("#151515"), 760.0, 5e-5),
        vinyl("Cabinet Vinyl Black Coarse", &["cabinet", "amplifier", "vinyl", "leatherette", "covering", "elephant", "coarse", "black"], srgb("#141414"), 380.0, 9e-5),
        vinyl("Cabinet Vinyl Blonde", &["cabinet", "amplifier", "vinyl", "leatherette", "covering", "blonde", "cream"], srgb("#CDBE98"), 760.0, 5e-5),
        vinyl("Cabinet Vinyl Brown", &["cabinet", "amplifier", "vinyl", "leatherette", "covering", "brown"], srgb("#4A3122"), 760.0, 5e-5),
        carpet("Cabinet Carpet Charcoal", &["cabinet", "carpet", "covering", "pa", "speaker", "felt", "charcoal", "grey"], rgb(0.03, 0.031, 0.033), rgb(0.075, 0.077, 0.082)),
        carpet("Cabinet Carpet Black", &["cabinet", "carpet", "covering", "pa", "speaker", "felt", "black"], rgb(0.01, 0.01, 0.011), rgb(0.03, 0.03, 0.032)),
        spatter_coat("Cabinet Coating Textured Black", &["cabinet", "coating", "textured paint", "polyurea", "spatter", "pa", "speaker", "black"], rgb(0.009, 0.009, 0.0095)),
        cloth("Grille Cloth Black", &["grille cloth", "speaker cloth", "amplifier", "cabinet", "woven", "black"], Pattern::Basket, 1300.0, srgb("#141414"), srgb("#1C1C1D"), 0.86, 0.15, 2.2e-4, 0.6),
        cloth("Grille Cloth Silver Black", &["grille cloth", "speaker cloth", "amplifier", "cabinet", "woven", "silver", "salt and pepper"], Pattern::Plain, 1500.0, srgb("#17171A"), srgb("#8E8F8C"), 0.88, 0.12, 1.8e-4, 0.55),
        cloth("Grille Cloth Wheat", &["grille cloth", "speaker cloth", "amplifier", "cabinet", "woven", "wheat", "tan"], Pattern::Basket, 1300.0, srgb("#B49A5E"), srgb("#7A5A34"), 0.86, 0.15, 2.2e-4, 0.6),
    ]);
    out
}
