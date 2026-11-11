//! What audio gear is made of: the enamel on a pedal's die-cast box, the
//! powder coat on a chassis, the knobs, the connectors' plastic, the
//! plated nuts and screws.
//!
//! These are the finishes of small things seen close, so they are built
//! the way a product renderer builds them by hand: a colour that is not
//! quite even, a gloss that wanders over a few centimetres, a grain a
//! fraction of a millimetre across that only the highlights show, and
//! edges that are never knife sharp - every one of them rounds its edges
//! by a tenth of a millimetre or three, which is where a real part catches
//! the light and a computed one does not.

//!
//! The numbers are published ones where there are any. Indices of
//! refraction: polycarbonate 1.585, polystyrene 1.59, PMMA 1.49 and SAN,
//! which is ABS's glassy part, 1.566 (Sultanova, Kasarova and Nikolov
//! 2009, by refractiveindex.info); nylon 1.53; acetal about 1.49; cured
//! enamels and polyurethanes 1.51 to 1.53. Gloss: an alkyd gloss enamel
//! measures 80 to 95 gloss units at 60 degrees, a fine-texture black
//! powder coat 7 to 11, a wrinkle finish about 10, a hammertone powder 15
//! to 55 (makers' data sheets). Orange peel is read in two bands, 0.3 to
//! 1.2 mm and 1.2 to 12 mm (BYK's wave-scan), which are the two grains an
//! enamel has here. Edges: a moulding's or a zinc casting's outside edge
//! is not made sharper than about 0.4 mm; a machined chamfer on anodised
//! aluminium is 0.1 to 0.5 mm. Grilles are to DIN 24041's patterns, round
//! holes staggered: Rv 3-5 and Rv 1.5-2.5 are a third open, and a PA
//! speaker's grille two thirds.

use crate::recipe::Out;
use crate::{base, metal_ior, noise, plus, preset, rgb, srgb, wear, Preset, Recipe};
use tequila_core::spectrum::Spectrum;
use tequila_materials::groups::NodeGroup;
use tequila_materials::material_type::{MaterialType, Param};

/// A fine grain and a coarser waviness summed into one bump: `fine`
/// features a metre and `coarse`, the coarse `weight` times the fine, the
/// whole `depth` metres for the fine's full height.
fn two_grains(r: &mut Recipe, fine: f32, coarse: f32, weight: f32, depth: f32) -> Out {
    let small = noise(r, fine, 2.0, 0.5);
    let large = noise(r, coarse, 3.0, 0.45);
    let both = plus(r, large, weight, small);
    r.bump(both, depth);
    both
}

/// The enamel on a die-cast enclosure: sprayed and baked, semi-gloss, with
/// the fine tooth of the spray and a faint peel under it.
fn enamel(name: &'static str, tags: &'static [&'static str], colour: Spectrum, gloss: (f32, f32)) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Plastic, |m| {
        m.base_color = colour;
        m.specular_roughness = (gloss.0 + gloss.1) * 0.5;
        m.specular_ior = 1.52;
        m.edge_radius = 4e-4;
    }));
    let gloss = wear(&mut r, gloss.0, gloss.1, 22.0);
    r.to(Param::SpecularRoughness, gloss);
    two_grains(&mut r, 3200.0, 240.0, 2.5, 2.5e-6);
    preset(name, "Paint", tags, MaterialType::Plastic, r)
}

/// The same enamel sprayed thick over a rough casting, or with a texture
/// in the paint: bumps half a millimetre to a millimetre across, each with
/// a glint of its own.
fn enamel_textured(name: &'static str, tags: &'static [&'static str], colour: Spectrum) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Plastic, |m| {
        m.base_color = colour;
        m.specular_roughness = 0.3;
        m.specular_ior = 1.52;
        m.edge_radius = 4e-4;
    }));
    let gloss = wear(&mut r, 0.26, 0.34, 18.0);
    r.to(Param::SpecularRoughness, gloss);
    let bumps = r.group(NodeGroup::OrangePeel, &[("Size (mm)", 0.8), ("Detail", 1.0)]);
    let bumps = r.of(bumps, "Height");
    r.bump(bumps, 4.5e-5);
    preset(name, "Paint", tags, MaterialType::Plastic, r)
}

/// A powder coat on sheet steel: `low` to `high` in gloss, with `grain`
/// features a metre `depth` metres high - fine for a satin, coarse and deep
/// for a sandy texture.
fn powder(name: &'static str, tags: &'static [&'static str], colour: Spectrum, gloss: (f32, f32), grain: f32, depth: f32) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Plastic, |m| {
        m.base_color = colour;
        m.specular_roughness = (gloss.0 + gloss.1) * 0.5;
        m.edge_radius = 3e-4;
    }));
    let sheen = wear(&mut r, gloss.0, gloss.1, 45.0);
    r.to(Param::SpecularRoughness, sheen);
    let tooth = r.node("noise_texture").f("Scale", grain).f("Detail", 5.0).f("Roughness", 0.5).out("Fac");
    r.bump(tooth, depth);
    preset(name, "Paint", tags, MaterialType::Plastic, r)
}

/// A moulded plastic with a fine grain from the tool: a knob, a cap, a
/// bezel. A dull sheen at a glancing angle, as a textured black has.
fn fine_grain(name: &'static str, tags: &'static [&'static str], colour: Spectrum, gloss: (f32, f32)) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = colour;
        m.specular_roughness = (gloss.0 + gloss.1) * 0.5;
        m.specular_ior = 1.566;
        m.fuzz_weight = 0.07;
        m.fuzz_roughness = 0.5;
        m.fuzz_color = rgb(0.55, 0.55, 0.56);
        m.edge_radius = 3e-4;
    }));
    // Three sizes of grain: the tool's texture, its finer tooth, and a
    // slow unevenness in how deep the texture took.
    let texture = noise(&mut r, 5200.0, 2.0, 0.5);
    let tooth = noise(&mut r, 21000.0, 2.0, 0.5);
    let uneven = noise(&mut r, 1500.0, 2.5, 0.5);
    let grain = plus(&mut r, tooth, 0.3, texture);
    let grain = plus(&mut r, uneven, 0.45, grain);
    r.bump(grain, 7e-6);
    let sheen = r.smooth(texture, (0.3, 0.7), (gloss.0, gloss.1));
    r.to(Param::SpecularRoughness, sheen);
    preset(name, "Plastic", tags, MaterialType::Generic, r)
}

/// A hard, glossy moulding as it leaves a polished tool: a connector's
/// body, a switch's slider, the plastic inside a jack.
fn glossy(name: &'static str, tags: &'static [&'static str], colour: Spectrum, gloss: (f32, f32), ior: f32) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Plastic, |m| {
        m.base_color = colour;
        m.specular_roughness = (gloss.0 + gloss.1) * 0.5;
        m.specular_ior = ior;
        m.edge_radius = 3e-4;
    }));
    let gloss = wear(&mut r, gloss.0, gloss.1, 40.0);
    r.to(Param::SpecularRoughness, gloss);
    preset(name, "Plastic", tags, MaterialType::Plastic, r)
}

/// Plating over brass or steel: nickel, a little warm, bright or dulled.
/// `handled` adds what fingers leave - patches a little duller, finely
/// streaked - which is what makes a plated nut look held rather than
/// drawn.
fn nickel(name: &'static str, tags: &'static [&'static str], gloss: (f32, f32), handled: bool, pebble: Option<(f32, f32)>) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Metal, |m| {
        m.metal_ior = metal_ior("Nickel");
        m.base_color = rgb(0.97, 0.93, 0.87);
        m.specular_roughness = (gloss.0 + gloss.1) * 0.5;
        m.edge_radius = 2e-4;
    }));
    let mut sheen = wear(&mut r, gloss.0, gloss.1, 35.0);
    if handled {
        let smudges = r.group(NodeGroup::Smudges, &[("Size (mm)", 9.0), ("Amount", 0.55), ("Streaks", 0.5)]);
        let smudge = r.of(smudges, "Smudge");
        sheen = plus(&mut r, smudge, 0.16, sheen);
    }
    r.to(Param::SpecularRoughness, sheen);
    match pebble {
        // Plate over a die casting: the casting's skin shows through as a
        // fine pebble, which is why such a shell never shows a mirror.
        Some((grains, depth)) => {
            let skin = r.group(NodeGroup::MouldTexture, &[("Grain (mm)", 1000.0 / grains), ("Unevenness", 0.5)]);
            let skin = r.of(skin, "Height");
            r.bump(skin, depth);
        }
        // The plating's own fine tooth.
        None => {
            let tooth = noise(&mut r, 16000.0, 2.0, 0.5);
            r.bump(tooth, 6e-7);
        }
    }
    preset(name, "Metal", tags, MaterialType::Metal, r)
}

/// A moulded plastic with the satin of a lightly textured tool: most of
/// an enclosure's inside and many a small part.
fn moulded_satin(name: &'static str, tags: &'static [&'static str], colour: Spectrum, gloss: (f32, f32), ior: f32) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Plastic, |m| {
        m.base_color = colour;
        m.specular_roughness = (gloss.0 + gloss.1) * 0.5;
        m.specular_ior = ior;
        m.edge_radius = 3.5e-4;
    }));
    let sheen = wear(&mut r, gloss.0, gloss.1, 30.0);
    r.to(Param::SpecularRoughness, sheen);
    two_grains(&mut r, 7000.0, 900.0, 1.5, 1.5e-6);
    preset(name, "Plastic", tags, MaterialType::Plastic, r)
}

/// Nylon with glass fibre in it: duller and paler where the fibres lie
/// at the surface, in the swirls the melt flowed in.
fn glass_filled(name: &'static str, tags: &'static [&'static str], resin: Spectrum, fibre: Spectrum) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = resin;
        m.specular_ior = 1.53;
        m.specular_roughness = 0.46;
        m.edge_radius = 3.5e-4;
    }));
    let flow = r.node("noise_texture").f("Scale", 220.0).f("Detail", 4.0).f("Roughness", 0.55).f("Distortion", 2.2).out("Fac");
    let colour = r.node("mix").w("Factor", flow).c("A", resin).c("B", fibre).out("Result");
    r.to(Param::BaseColor, colour);
    let sheen = r.range(flow, (0.3, 0.7), (0.38, 0.54));
    r.to(Param::SpecularRoughness, sheen);
    let along = r.stretched([0.06, 1.0, 1.0]);
    let fibres = r.node("noise_texture").w("Vector", along).f("Scale", 2600.0).f("Detail", 2.0).out("Fac");
    r.bump(fibres, 1.5e-6);
    preset(name, "Plastic", tags, MaterialType::Generic, r)
}

/// Chrome over a plastic moulding: a mirror, but one laid on plastic, so
/// its reflections wander a little where the moulding sank and flowed -
/// which is how a plated trim ring is told from a turned one.
fn plated_plastic(name: &'static str, tags: &'static [&'static str], gloss: (f32, f32)) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Metal, |m| {
        m.metal_ior = metal_ior("Chromium");
        m.base_color = rgb(0.98, 0.98, 0.99);
        m.specular_roughness = (gloss.0 + gloss.1) * 0.5;
        m.edge_radius = 3.5e-4;
    }));
    let sheen = wear(&mut r, gloss.0, gloss.1, 25.0);
    r.to(Param::SpecularRoughness, sheen);
    let flow = r.group(NodeGroup::OrangePeel, &[("Size (mm)", 7.0), ("Detail", 0.5)]);
    let flow = r.of(flow, "Height");
    r.bump(flow, 2.5e-6);
    preset(name, "Metal", tags, MaterialType::Metal, r)
}

/// A die casting as it leaves the tool or the tumbler: a dull grey skin,
/// finely pebbled, patchy where the metal flowed and chilled.
fn die_cast(name: &'static str, tags: &'static [&'static str], ior: &str, tint: Spectrum, gloss: (f32, f32), grain_mm: f32) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Metal, |m| {
        m.metal_ior = metal_ior(ior);
        m.base_color = tint;
        m.specular_roughness = (gloss.0 + gloss.1) * 0.5;
        m.edge_radius = 4e-4;
    }));
    let patches = r.group(NodeGroup::Patches, &[("Size (mm)", 18.0), ("Amount", 0.5), ("Softness", 0.8), ("Detail", 2.0)]);
    let patch = r.of(patches, "Patch");
    let sheen = r.mix_float(patch, gloss.0, gloss.1);
    r.to(Param::SpecularRoughness, sheen);
    let skin = r.group(NodeGroup::MouldTexture, &[("Grain (mm)", grain_mm), ("Unevenness", 0.6)]);
    let skin = r.of(skin, "Height");
    r.bump(skin, 9e-6);
    preset(name, "Metal", tags, MaterialType::Metal, r)
}

/// A paint that pools into a pattern as it cures: `height` from a pattern,
/// `depth` metres deep, under a gloss of `gloss`. Wrinkle finish and
/// hammertone are both this.
fn patterned_paint(name: &'static str, tags: &'static [&'static str], colour: Spectrum, metal: f32, gloss: (f32, f32), height: impl FnOnce(&mut Recipe) -> Out, depth: f32) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = colour;
        m.base_metalness = metal;
        m.specular_roughness = (gloss.0 + gloss.1) * 0.5;
        m.edge_radius = 3e-4;
    }));
    let relief = height(&mut r);
    r.bump(relief, depth);
    // Thinner on the rises, so a touch duller there.
    let sheen = r.range(relief, (0.2, 0.8), (gloss.0, gloss.1));
    r.to(Param::SpecularRoughness, sheen);
    preset(name, "Paint", tags, MaterialType::Generic, r)
}

/// Wrinkle finish: fine ridges that wander and join, a millimetre or so
/// apart.
fn wrinkle(r: &mut Recipe) -> Out {
    let noise = r.node("noise_texture").f("Scale", 850.0).f("Detail", 1.0).f("Roughness", 0.4).f("Distortion", 2.6).out("Fac");
    // The ridge where the noise crosses its middle.
    let away = r.math3("multiply_add", noise, 2.0, -1.0);
    let away = r.math("absolute", away, 0.0);
    let ridge = r.math("subtract", 1.0, away);
    r.smooth(ridge, (0.55, 0.98), (0.0, 1.0))
}

/// Hammertone: shallow dishes a few millimetres across.
fn dishes(r: &mut Recipe) -> Out {
    let hammered = r.group(NodeGroup::Hammered, &[("Size (mm)", 4.5)]);
    r.of(hammered, "Height")
}

/// A gloss enamel's roughness, 80 to 95 gloss units, and a satin's.
const GLOSS: (f32, f32) = (0.09, 0.13);
const SATIN: (f32, f32) = (0.2, 0.3);

pub fn all() -> Vec<Preset> {
    let mut out = vec![
        enamel("Enclosure Enamel Orange", &["enamel", "pedal", "stompbox", "die-cast", "enclosure", "orange", "paint"], srgb("#DE5E12"), GLOSS),
        enamel("Enclosure Enamel Red", &["enamel", "pedal", "stompbox", "die-cast", "enclosure", "red", "paint"], srgb("#B8161A"), GLOSS),
        enamel("Enclosure Enamel Yellow", &["enamel", "pedal", "stompbox", "die-cast", "enclosure", "yellow", "paint"], srgb("#F0B400"), GLOSS),
        enamel("Enclosure Enamel Green", &["enamel", "pedal", "stompbox", "die-cast", "enclosure", "green", "paint"], srgb("#1F7A3D"), GLOSS),
        enamel("Enclosure Enamel Blue", &["enamel", "pedal", "stompbox", "die-cast", "enclosure", "blue", "paint"], srgb("#0E4FA6"), GLOSS),
        enamel("Enclosure Enamel Purple", &["enamel", "pedal", "stompbox", "die-cast", "enclosure", "purple", "paint"], srgb("#5A2A82"), GLOSS),
        enamel("Enclosure Enamel White", &["enamel", "pedal", "stompbox", "die-cast", "enclosure", "white", "paint"], srgb("#ECECE7"), GLOSS),
        enamel("Enclosure Enamel Black", &["enamel", "pedal", "stompbox", "die-cast", "enclosure", "black", "paint"], srgb("#0D0D0E"), GLOSS),
        enamel("Enclosure Enamel Satin Orange", &["enamel", "pedal", "stompbox", "die-cast", "enclosure", "orange", "satin", "paint"], srgb("#DE5E12"), SATIN),
        enamel("Enclosure Enamel Satin Blue", &["enamel", "pedal", "stompbox", "die-cast", "enclosure", "blue", "satin", "paint"], srgb("#0E4FA6"), SATIN),
        enamel("Enclosure Enamel Satin White", &["enamel", "pedal", "stompbox", "die-cast", "enclosure", "white", "satin", "paint"], srgb("#ECECE7"), SATIN),
        enamel("Enclosure Enamel Satin Black", &["enamel", "pedal", "stompbox", "die-cast", "enclosure", "black", "satin", "paint"], srgb("#0D0D0E"), SATIN),
        enamel_textured("Enclosure Enamel Textured Orange", &["enamel", "pedal", "stompbox", "textured", "orange peel", "orange", "paint"], srgb("#E06A10")),
        enamel_textured("Enclosure Enamel Textured Black", &["enamel", "pedal", "stompbox", "textured", "orange peel", "black", "paint"], srgb("#101011")),
        crate::paint::metallic("Enclosure Metallic Gold", &["metallic", "pedal", "stompbox", "die-cast", "flake", "gold", "paint"], srgb("#8A6A2A"), srgb("#F0D890"), 0.55, 5e-5, 0.3, 0.28),
        crate::paint::metallic("Enclosure Metallic Charcoal", &["metallic", "pedal", "stompbox", "die-cast", "flake", "charcoal", "gunmetal", "paint"], srgb("#141518"), srgb("#7C8088"), 0.22, 6e-5, 0.3, 0.3),
        crate::paint::metallic("Enclosure Metallic Silver", &["metallic", "painted plastic", "side panel", "flake", "silver", "paint"], srgb("#8E9296"), srgb("#F2F4F6"), 0.6, 4e-5, 0.3, 0.32),
        crate::paint::metallic("Enclosure Metallic Dark Green", &["metallic", "pedal", "stompbox", "die-cast", "flake", "green", "sparkle", "paint"], srgb("#0A1A12"), srgb("#8FE0B0"), 0.35, 4e-5, 0.3, 0.16),
        powder("Powder Coat Satin Black", &["powder", "satin", "chassis", "cover", "black", "sheet steel"], rgb(0.009, 0.009, 0.0095), (0.34, 0.48), 1900.0, 4e-6),
        powder("Powder Coat Fine Texture Black", &["powder", "texture", "sandy", "rack", "chassis", "black"], rgb(0.008, 0.008, 0.0085), (0.5, 0.64), 2400.0, 2.2e-5),
        powder("Powder Coat Fine Texture Grey", &["powder", "texture", "sandy", "rack", "chassis", "grey"], srgb("#5B5D60"), (0.5, 0.62), 2400.0, 2.2e-5),
        fine_grain("Knob Plastic Black", &["knob", "abs", "fine grain", "textured", "black", "moulded"], rgb(0.011, 0.011, 0.0118), (0.44, 0.6)),
        fine_grain("Knob Plastic Cream", &["knob", "abs", "fine grain", "textured", "cream", "moulded"], srgb("#D9D0BC"), (0.42, 0.56)),
        fine_grain("Knob Pointer Grey", &["knob", "pointer", "line", "indicator", "grey", "moulded"], srgb("#8E9092"), (0.5, 0.6)),
        fine_grain("Knob Pointer White", &["knob", "pointer", "line", "indicator", "white", "moulded"], srgb("#E9E9E6"), (0.5, 0.6)),
        glossy("Connector Plastic Black", &["connector", "jack", "socket", "pbt", "nylon", "black", "gloss"], rgb(0.004, 0.004, 0.0045), (0.15, 0.26), 1.53),
        glossy("Connector Plastic Grey", &["connector", "jack", "socket", "pbt", "nylon", "grey", "gloss"], srgb("#8A8C8E"), (0.15, 0.26), 1.53),
        glossy("Knob Plastic Gloss Black", &["knob", "gloss", "black", "pointer knob", "moulded", "polished"], rgb(0.008, 0.008, 0.0085), (0.04, 0.09), 1.566),
        nickel("Nickel Plated Bright", &["nickel", "plated", "jack", "nut", "bushing", "hardware", "bright"], (0.1, 0.2), false, None),
        nickel("Nickel Plated Satin", &["nickel", "plated", "jack", "sleeve", "thread", "hardware", "satin"], (0.26, 0.36), false, None),
        nickel("Nickel Plated Handled", &["nickel", "plated", "connector", "smudged", "fingerprints", "hardware"], (0.12, 0.22), true, None),
        crate::metal_pattern::spun("Nickel Plated Spun", &["nickel", "plated", "spun", "footswitch", "cap", "concentric", "hardware"], "Nickel", rgb(0.97, 0.93, 0.87), 0.22),
        crate::metal_pattern::turned("Nickel Plated Turned", &["nickel", "plated", "turned", "bushing", "thread", "lathe", "hardware"], "Nickel", rgb(0.97, 0.93, 0.87), 0.24, 0.00012),
        nickel("Nickel Plated Matte", &["nickel", "matte", "die-cast", "connector shell", "xlr", "plug", "satin"], (0.42, 0.52), false, Some((4000.0, 6e-6))),
    ];

    out.extend([
        moulded_satin("Polycarbonate Satin Black", &["polycarbonate", "pc", "pc/abs", "housing", "satin", "black"], rgb(0.011, 0.011, 0.012), (0.3, 0.4), 1.585),
        moulded_satin("Polycarbonate Satin White", &["polycarbonate", "pc", "pc/abs", "housing", "satin", "white"], srgb("#E8E8E5"), (0.3, 0.4), 1.585),
        moulded_satin("Polycarbonate Satin Grey", &["polycarbonate", "pc", "pc/abs", "housing", "satin", "grey"], srgb("#6E7074"), (0.3, 0.4), 1.585),
        moulded_satin("HIPS Matte White", &["hips", "polystyrene", "housing", "tray", "matte", "white"], srgb("#E4E2DC"), (0.5, 0.62), 1.59),
        moulded_satin("HIPS Matte Black", &["hips", "polystyrene", "housing", "tray", "matte", "black"], rgb(0.014, 0.014, 0.015), (0.5, 0.62), 1.59),
        moulded_satin("POM Black", &["pom", "acetal", "delrin", "slider", "gear", "black", "engineering"], rgb(0.012, 0.012, 0.013), (0.24, 0.34), 1.49),
        glossy("Polycarbonate Gloss Black", &["polycarbonate", "pc", "gloss", "bezel", "lens surround", "black"], rgb(0.005, 0.005, 0.0055), (0.03, 0.08), 1.585),
        glossy("Polycarbonate Gloss White", &["polycarbonate", "pc", "gloss", "housing", "white"], srgb("#EDEDEA"), (0.04, 0.1), 1.585),
        plated_plastic("Chrome Plated Plastic Bright", &["chrome", "plated plastic", "trim", "ring", "button", "logo", "bright"], (0.03, 0.07)),
        plated_plastic("Chrome Plated Plastic Satin", &["chrome", "plated plastic", "trim", "ring", "button", "satin", "pearl chrome"], (0.24, 0.32)),
        die_cast("Zinc Die Cast Raw", &["zinc", "zamak", "die-cast", "raw", "pedal box", "housing"], "Zinc", rgb(0.72, 0.73, 0.75), (0.42, 0.56), 0.35),
        die_cast("Aluminium Die Cast Tumbled", &["aluminium", "die-cast", "tumbled", "vibratory", "enclosure", "housing"], "Aluminium", rgb(0.78, 0.78, 0.8), (0.44, 0.58), 0.5),
        crate::metal::brushed("Aluminium Anodised Brushed Black", &["aluminium", "anodised", "brushed", "black", "front panel"], "Aluminium", srgb("#1E1F22"), 0.32),
        crate::metal::brushed("Aluminium Anodised Brushed Silver", &["aluminium", "anodised", "brushed", "silver", "front panel"], "Aluminium", srgb("#D6D8DA"), 0.3),
        crate::metal::brushed("Aluminium Anodised Brushed Blue", &["aluminium", "anodised", "brushed", "blue", "front panel"], "Aluminium", srgb("#2A5CB0"), 0.32),
        crate::metal::brushed("Aluminium Anodised Brushed Red", &["aluminium", "anodised", "brushed", "red", "front panel"], "Aluminium", srgb("#A8222C"), 0.32),
        crate::metal::brushed("Aluminium Anodised Brushed Gold", &["aluminium", "anodised", "brushed", "gold", "champagne", "front panel"], "Aluminium", srgb("#C8A968"), 0.32),
        crate::metal::brushed("Nickel Plated Brushed", &["nickel", "plated", "brushed", "nut", "hardware"], "Nickel", rgb(0.97, 0.93, 0.87), 0.3),
        patterned_paint("Powder Coat Wrinkle Black", &["powder", "wrinkle", "crackle finish", "chassis", "amplifier", "black"], rgb(0.01, 0.01, 0.0105), 0.0, (0.64, 0.5), wrinkle, 5.5e-5),
        patterned_paint("Hammertone Grey", &["hammertone", "hammer finish", "hammered paint", "chassis", "grey"], srgb("#5E6468"), 0.55, (0.44, 0.3), dishes, 8e-5),
        patterned_paint("Hammertone Silver", &["hammertone", "hammer finish", "hammered paint", "chassis", "silver"], srgb("#A9AEB2"), 0.6, (0.44, 0.3), dishes, 8e-5),
        patterned_paint("Hammertone Blue", &["hammertone", "hammer finish", "hammered paint", "chassis", "blue"], srgb("#2C4C78"), 0.5, (0.44, 0.3), dishes, 8e-5),
    ]);

    // Grilles: sheet punched to DIN 24041's staggered round patterns and
    // powder coated black. Pitch and hole radius in metres.
    out.extend([
        crate::metal_pattern::perforated_coated("Grille Black Round 3 mm", &["grille", "perforated", "vent", "speaker", "black", "rv 3-5", "powder coat"], rgb(0.01, 0.01, 0.0105), 0.5, 0.005, 0.0015, crate::finish::Hole::Round),
        crate::metal_pattern::perforated_coated("Grille Black Round 1.5 mm", &["grille", "perforated", "vent", "fine", "black", "rv 1.5-2.5", "powder coat"], rgb(0.01, 0.01, 0.0105), 0.5, 0.0025, 0.00075, crate::finish::Hole::Round),
        crate::metal_pattern::perforated_coated("Grille Black Hex 6 mm", &["grille", "perforated", "speaker", "pa", "hexagon", "black", "powder coat"], rgb(0.01, 0.01, 0.0105), 0.5, 0.007, 0.003, crate::finish::Hole::Hex),
    ]);

    // The rest of what a chassis and its small parts are moulded and
    // folded from. Polypropylene's index is 1.49; the sheet steels are a
    // chassis's usual two, zinc plated by electrolysis, which has no
    // spangle, and bare cold rolled; tinplate is what a shield can on a
    // circuit board is pressed from.
    out.extend([
        moulded_satin("Polypropylene Black", &["polypropylene", "pp", "housing", "moulded", "case", "black"], rgb(0.012, 0.012, 0.013), (0.34, 0.46), 1.49),
        moulded_satin("Polypropylene Grey", &["polypropylene", "pp", "housing", "moulded", "case", "grey"], srgb("#74777A"), (0.34, 0.46), 1.49),
        moulded_satin("Polypropylene White", &["polypropylene", "pp", "housing", "moulded", "case", "white"], srgb("#E6E5E0"), (0.34, 0.46), 1.49),
        glossy("Connector Plastic White", &["connector", "jack", "socket", "pbt", "nylon", "white", "gloss"], srgb("#E4E3DD"), (0.15, 0.26), 1.53),
        glass_filled("Nylon Glass Filled Grey", &["nylon", "pa66", "glass filled", "gf30", "engineering", "bracket", "grey"], srgb("#55585B"), srgb("#6C6F73")),
        glass_filled("Nylon Glass Filled Natural", &["nylon", "pa66", "glass filled", "gf30", "engineering", "bracket", "natural", "cream"], srgb("#C9C0A8"), srgb("#D9D2BE")),
        crate::metal_pattern::sheet("Steel Electrogalvanised", &["steel", "electrogalvanised", "secc", "zinc", "chassis", "sheet", "bracket"], "Zinc", rgb(0.72, 0.73, 0.75), (0.46, 0.58)),
        crate::metal_pattern::sheet("Steel Cold Rolled", &["steel", "cold rolled", "crs", "spcc", "bare", "chassis", "sheet"], "Iron", rgb(0.9, 0.9, 0.91), (0.28, 0.4)),
        crate::metal_pattern::sheet("Tinplate Bright", &["tinplate", "tin", "shield", "shielding can", "rf shield", "circuit board", "sheet"], "Aluminium", rgb(0.9, 0.89, 0.85), (0.14, 0.24)),
        crate::metal_pattern::chromated("Aluminium Chromate Yellow", &["aluminium", "chromate", "conversion coating", "alodine", "iridite", "chassis", "yellow", "gold"], &[(0.25, srgb("#BCA662")), (0.5, srgb("#CDBB7E")), (0.75, srgb("#B3AE78"))]),
    ]);

    // More grilles: slots, expanded metal, and the woven wire of a
    // microphone's head.
    out.extend([
        crate::metal_pattern::slotted_coated("Grille Black Slotted", &["grille", "vent", "slots", "slotted", "obround", "black", "powder coat"], rgb(0.01, 0.01, 0.0105), 0.5, 0.012, 0.002, 0.016, 0.004),
        crate::metal_pattern::slotted_coated("Grille Black Slotted Fine", &["grille", "vent", "slots", "slotted", "fine", "black", "powder coat"], rgb(0.01, 0.01, 0.0105), 0.5, 0.006, 0.0012, 0.0085, 0.0026),
        crate::metal_pattern::expanded_coated("Grille Black Expanded", &["grille", "expanded metal", "mesh", "diamond", "speaker", "amplifier", "black"], rgb(0.01, 0.01, 0.0105), 0.5, 0.008, 0.004, 0.3),
        crate::metal_pattern::wire_mesh("Wire Mesh Nickel", &["wire mesh", "woven", "microphone", "grille", "screen", "nickel", "plated"], crate::metal_pattern::plated_wire("Nickel", rgb(0.97, 0.93, 0.87), 0.2), 0.0013, 0.0004),
        crate::metal_pattern::wire_mesh("Wire Mesh Nickel Fine", &["wire mesh", "woven", "microphone", "grille", "screen", "fine", "nickel", "plated"], crate::metal_pattern::plated_wire("Nickel", rgb(0.97, 0.93, 0.87), 0.2), 0.0005, 0.00016),
        crate::metal_pattern::wire_mesh("Wire Mesh Black", &["wire mesh", "woven", "microphone", "grille", "screen", "black"], base(MaterialType::Generic, |m| {
            m.base_color = rgb(0.01, 0.01, 0.0105);
            m.specular_roughness = 0.42;
        }), 0.0013, 0.0004),
    ]);

    // A diffuser: white plastic the light goes through and is scattered
    // by, so what is behind it is a glow and never a shape.
    out.push(crate::plain("Diffuser White", "Plastic", &["diffuser", "opal", "milky", "led window", "light ring", "translucent", "polycarbonate"], MaterialType::PlasticCloudy, |m| {
        m.base_color = rgb(0.9, 0.9, 0.88);
        m.specular_roughness = 0.34;
        m.edge_radius = 3e-4;
    }));
    // The window over an infrared receiver: black to the eye, a deep red
    // against the light.
    out.push(crate::plain("Window Infrared Dark Red", "Plastic", &["infrared", "ir window", "remote", "sensor", "dark red", "tinted"], MaterialType::PlasticTransparent, |m| {
        m.specular_ior = 1.49;
        m.specular_roughness = 0.03;
        m.transmission_color = rgb(0.09, 0.004, 0.006);
        m.edge_radius = 2e-4;
    }));

    // A knurled nut or thumbscrew: straight ridges round it, plated.
    let mut r = Recipe::new(base(MaterialType::Metal, |m| {
        m.metal_ior = metal_ior("Nickel");
        m.base_color = rgb(0.97, 0.93, 0.87);
        m.specular_roughness = 0.24;
        m.edge_radius = 1e-4;
    }));
    let knurl = r.group(NodeGroup::Knurl, &[("Pitch (mm)", 0.7), ("Angle", 0.0), ("Flat", 0.2), ("Plane", 4.0)]);
    let ridges = r.of(knurl, "Height");
    r.bump(ridges, 1.2e-4);
    // Brighter on the ridges' flats, duller down the flanks.
    let gloss = r.range(ridges, (0.0, 1.0), (0.34, 0.18));
    r.to(Param::SpecularRoughness, gloss);
    out.push(preset("Nickel Plated Knurled", "Metal", &["nickel", "plated", "knurled", "nut", "thumbscrew", "jack nut", "hardware"], MaterialType::Metal, r));

    // Black zinc on a screw: dark metal under a thin, slightly dull film.
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_metalness = 1.0;
        m.base_color = rgb(0.04, 0.042, 0.048);
        m.specular_roughness = 0.36;
        m.coat_weight = 0.3;
        m.coat_roughness = 0.25;
        m.edge_radius = 1e-4;
    }));
    let gloss = wear(&mut r, 0.3, 0.44, 220.0);
    r.to(Param::SpecularRoughness, gloss);
    out.push(preset("Steel Black Zinc", "Metal", &["screw", "fastener", "black zinc", "passivated", "hardware"], MaterialType::Generic, r));

    // A light guide: clear plastic frosted where it shows, so an LED
    // behind it is a soft, even glow and not a point.
    let mut r = Recipe::new(base(MaterialType::PlasticTransparent, |m| {
        m.specular_ior = 1.49;
        m.specular_roughness = 0.36;
        m.transmission_color = rgb(0.94, 0.95, 0.95);
        m.edge_radius = 2.5e-4;
    }));
    let frost = noise(&mut r, 5600.0, 2.0, 0.5);
    r.bump(frost, 3e-6);
    out.push(preset("Light Guide Frosted", "Plastic", &["light guide", "light pipe", "led", "diffuser", "frosted", "indicator"], MaterialType::PlasticTransparent, r));
    out
}
