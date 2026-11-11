//! What audio gear is made of: the enamel on a pedal's die-cast box, the
//! powder coat on a chassis, the knobs, the connectors' plastic, the
//! plated nuts and screws, the anodised extrusion with its bright cut edge.
//!
//! These are the finishes of small things seen close, so they are built
//! the way a product renderer builds them by hand: a colour that is not
//! quite even, a gloss that wanders over a few centimetres, a grain a
//! fraction of a millimetre across that only the highlights show, and
//! edges that are never knife sharp - every one of them rounds its edges
//! by a tenth of a millimetre or three, which is where a real part catches
//! the light and a computed one does not.

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
fn enamel(name: &'static str, tags: &'static [&'static str], colour: Spectrum) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Plastic, |m| {
        m.base_color = colour;
        m.specular_roughness = 0.24;
        m.specular_ior = 1.6;
        m.edge_radius = 2e-4;
    }));
    let gloss = wear(&mut r, 0.2, 0.28, 18.0);
    r.to(Param::SpecularRoughness, gloss);
    two_grains(&mut r, 3900.0, 300.0, 3.0, 2.5e-6);
    preset(name, "Paint", tags, MaterialType::Plastic, r)
}

/// The same enamel sprayed thick over a rough casting, or with a texture
/// in the paint: bumps half a millimetre to a millimetre across, each with
/// a glint of its own.
fn enamel_textured(name: &'static str, tags: &'static [&'static str], colour: Spectrum) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Plastic, |m| {
        m.base_color = colour;
        m.specular_roughness = 0.3;
        m.specular_ior = 1.6;
        m.edge_radius = 3e-4;
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
    let sheen = wear(&mut r, gloss.0, gloss.1, 60.0);
    r.to(Param::SpecularRoughness, sheen);
    let tooth = r.node("noise_texture").f("Scale", grain).f("Detail", 6.0).f("Roughness", 0.55).out("Fac");
    r.bump(tooth, depth);
    preset(name, "Paint", tags, MaterialType::Plastic, r)
}

/// A moulded plastic with a fine grain from the tool: a knob, a cap, a
/// bezel. A dull sheen at a glancing angle, as a textured black has.
fn fine_grain(name: &'static str, tags: &'static [&'static str], colour: Spectrum, gloss: (f32, f32)) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = colour;
        m.specular_roughness = (gloss.0 + gloss.1) * 0.5;
        m.fuzz_weight = 0.2;
        m.fuzz_roughness = 0.45;
        m.fuzz_color = rgb(0.5, 0.5, 0.52);
        m.edge_radius = 1e-4;
    }));
    // Three sizes of grain: the tool's texture, its finer tooth, and a
    // slow unevenness in how deep the texture took.
    let texture = noise(&mut r, 4500.0, 1.5, 0.5);
    let tooth = noise(&mut r, 25000.0, 2.0, 0.5);
    let uneven = noise(&mut r, 1250.0, 3.0, 0.5);
    let grain = plus(&mut r, tooth, 0.35, texture);
    let grain = plus(&mut r, uneven, 0.5, grain);
    r.bump(grain, 7e-6);
    let sheen = r.smooth(texture, (0.3, 0.7), (gloss.0, gloss.1));
    r.to(Param::SpecularRoughness, sheen);
    preset(name, "Plastic", tags, MaterialType::Generic, r)
}

/// A hard, glossy moulding as it leaves a polished tool: a connector's
/// body, a switch's slider, the plastic inside a jack.
fn glossy(name: &'static str, tags: &'static [&'static str], colour: Spectrum, gloss: (f32, f32)) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Plastic, |m| {
        m.base_color = colour;
        m.specular_roughness = (gloss.0 + gloss.1) * 0.5;
        m.specular_ior = 1.6;
        m.edge_radius = 1e-4;
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
            let tooth = noise(&mut r, 20000.0, 2.0, 0.5);
            r.bump(tooth, 6e-7);
        }
    }
    preset(name, "Metal", tags, MaterialType::Metal, r)
}

/// Anodised aluminium whose edges were cut after it was dyed: the faces a
/// fine satin in the dye's colour, and every outward edge bare, bright
/// metal - the line of light round a machined case.
fn diamond_cut(name: &'static str, tags: &'static [&'static str], tint: Spectrum) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Metal, |m| {
        m.metal_ior = metal_ior("Aluminium");
        m.base_color = tint;
        m.specular_roughness = 0.38;
    }));
    // Where the surface turns outward within a few tenths of a millimetre.
    let edge = r.edges(4e-4, 1.0);
    let cut = r.smooth(edge, (0.25, 0.6), (0.0, 1.0));
    let faces = wear(&mut r, 0.34, 0.44, 12.0);
    let gloss = r.mix_float(cut, faces, 0.07);
    r.to(Param::SpecularRoughness, gloss);
    let dyed = r.node("combine_color").f("Red", tint.r).f("Green", tint.g).f("Blue", tint.b).out("Color");
    let bare = r.node("combine_color").f("Red", 0.96).f("Green", 0.965).f("Blue", 0.97).out("Color");
    let colour = r.mix("mix", cut, dyed, bare);
    r.to(Param::BaseColor, colour);
    // The blast under the dye; the cut is smooth.
    let blast = noise(&mut r, 6500.0, 3.0, 0.5);
    let blast = r.math("multiply", blast, 1.0);
    let off = r.math("subtract", 1.0, cut);
    let blast = r.math("multiply", blast, off);
    r.bump(blast, 2.5e-6);
    preset(name, "Metal", tags, MaterialType::Metal, r)
}

pub fn all() -> Vec<Preset> {
    let mut out = vec![
        enamel("Enclosure Enamel Orange", &["enamel", "pedal", "stompbox", "die-cast", "enclosure", "orange", "paint"], srgb("#E05A00")),
        enamel("Enclosure Enamel Red", &["enamel", "pedal", "stompbox", "die-cast", "enclosure", "red", "paint"], srgb("#B8161A")),
        enamel("Enclosure Enamel Yellow", &["enamel", "pedal", "stompbox", "die-cast", "enclosure", "yellow", "paint"], srgb("#F0B400")),
        enamel("Enclosure Enamel Green", &["enamel", "pedal", "stompbox", "die-cast", "enclosure", "green", "paint"], srgb("#1F7A3D")),
        enamel("Enclosure Enamel Blue", &["enamel", "pedal", "stompbox", "die-cast", "enclosure", "blue", "paint"], srgb("#0E4FA6")),
        enamel("Enclosure Enamel Purple", &["enamel", "pedal", "stompbox", "die-cast", "enclosure", "purple", "paint"], srgb("#5A2A82")),
        enamel("Enclosure Enamel White", &["enamel", "pedal", "stompbox", "die-cast", "enclosure", "white", "paint"], srgb("#ECECE7")),
        enamel("Enclosure Enamel Black", &["enamel", "pedal", "stompbox", "die-cast", "enclosure", "black", "paint"], srgb("#0D0D0E")),
        enamel_textured("Enclosure Enamel Textured Orange", &["enamel", "pedal", "stompbox", "textured", "orange peel", "orange", "paint"], srgb("#E06A10")),
        enamel_textured("Enclosure Enamel Textured Black", &["enamel", "pedal", "stompbox", "textured", "orange peel", "black", "paint"], srgb("#101011")),
        crate::paint::metallic("Enclosure Metallic Gold", &["metallic", "pedal", "stompbox", "die-cast", "flake", "gold", "paint"], srgb("#8A6A2A"), srgb("#F0D890"), 0.55, 5e-5, 0.3, 0.28),
        crate::paint::metallic("Enclosure Metallic Charcoal", &["metallic", "pedal", "stompbox", "die-cast", "flake", "charcoal", "gunmetal", "paint"], srgb("#1C1D20"), srgb("#B8BCC4"), 0.45, 6e-5, 0.3, 0.3),
        crate::paint::metallic("Enclosure Metallic Silver", &["metallic", "painted plastic", "side panel", "flake", "silver", "paint"], srgb("#8E9296"), srgb("#F2F4F6"), 0.6, 4e-5, 0.3, 0.32),
        crate::paint::metallic("Enclosure Metallic Dark Green", &["metallic", "pedal", "stompbox", "die-cast", "flake", "green", "sparkle", "paint"], srgb("#0A1A12"), srgb("#8FE0B0"), 0.35, 4e-5, 0.3, 0.16),
        powder("Powder Coat Satin Black", &["powder", "satin", "chassis", "cover", "black", "sheet steel"], rgb(0.006, 0.006, 0.0063), (0.36, 0.5), 1500.0, 4e-6),
        powder("Powder Coat Fine Texture Black", &["powder", "texture", "sandy", "rack", "chassis", "black"], rgb(0.008, 0.008, 0.0085), (0.5, 0.64), 2400.0, 2.2e-5),
        powder("Powder Coat Fine Texture Grey", &["powder", "texture", "sandy", "rack", "chassis", "grey"], srgb("#5B5D60"), (0.5, 0.62), 2400.0, 2.2e-5),
        fine_grain("Knob Plastic Black", &["knob", "abs", "fine grain", "textured", "black", "moulded"], rgb(0.012, 0.012, 0.0125), (0.42, 0.58)),
        fine_grain("Knob Plastic Cream", &["knob", "abs", "fine grain", "textured", "cream", "moulded"], srgb("#D9D0BC"), (0.42, 0.56)),
        fine_grain("Knob Pointer Grey", &["knob", "pointer", "line", "indicator", "grey", "moulded"], rgb(0.309, 0.319, 0.328), (0.5, 0.6)),
        fine_grain("Knob Pointer White", &["knob", "pointer", "line", "indicator", "white", "moulded"], srgb("#E9E9E6"), (0.5, 0.6)),
        glossy("Connector Plastic Black", &["connector", "jack", "socket", "pbt", "nylon", "black", "gloss"], rgb(0.004, 0.004, 0.0045), (0.15, 0.26)),
        glossy("Connector Plastic Grey", &["connector", "jack", "socket", "pbt", "nylon", "grey", "gloss"], srgb("#8A8C8E"), (0.15, 0.26)),
        glossy("Knob Plastic Gloss Black", &["knob", "gloss", "black", "pointer knob", "moulded", "polished"], rgb(0.006, 0.006, 0.0065), (0.04, 0.09)),
        nickel("Nickel Plated Bright", &["nickel", "plated", "jack", "nut", "bushing", "hardware", "bright"], (0.1, 0.2), false, None),
        nickel("Nickel Plated Satin", &["nickel", "plated", "jack", "sleeve", "thread", "hardware", "satin"], (0.26, 0.36), false, None),
        nickel("Nickel Plated Handled", &["nickel", "plated", "connector", "smudged", "fingerprints", "hardware"], (0.12, 0.22), true, None),
        nickel("Nickel Plated Matte", &["nickel", "matte", "die-cast", "connector shell", "xlr", "plug", "satin"], (0.42, 0.52), false, Some((4000.0, 6e-6))),
        diamond_cut("Aluminium Anodised Blue Diamond Cut", &["aluminium", "anodised", "blue", "diamond cut", "chamfer", "bright edge"], srgb("#1F5FD0")),
        diamond_cut("Aluminium Anodised Black Diamond Cut", &["aluminium", "anodised", "black", "diamond cut", "chamfer", "bright edge"], srgb("#1B1C1F")),
        diamond_cut("Aluminium Anodised Silver Diamond Cut", &["aluminium", "anodised", "silver", "diamond cut", "chamfer", "bright edge"], srgb("#D8DADC")),
        diamond_cut("Aluminium Anodised Red Diamond Cut", &["aluminium", "anodised", "red", "diamond cut", "chamfer", "bright edge"], srgb("#B5202A")),
    ];

    // Black zinc on a screw: dark metal under a thin, slightly dull film.
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_metalness = 1.0;
        m.base_color = rgb(0.055, 0.055, 0.06);
        m.specular_roughness = 0.34;
        m.coat_weight = 0.25;
        m.coat_roughness = 0.2;
        m.edge_radius = 1e-4;
    }));
    let gloss = wear(&mut r, 0.28, 0.42, 300.0);
    r.to(Param::SpecularRoughness, gloss);
    out.push(preset("Steel Black Zinc", "Metal", &["screw", "fastener", "black zinc", "passivated", "hardware"], MaterialType::Generic, r));

    // A light guide: clear plastic frosted where it shows, so an LED
    // behind it is a soft, even glow and not a point.
    let mut r = Recipe::new(base(MaterialType::PlasticTransparent, |m| {
        m.specular_ior = 1.49;
        m.specular_roughness = 0.3;
        m.transmission_color = rgb(0.96, 0.96, 0.96);
        m.edge_radius = 3e-4;
    }));
    let frost = noise(&mut r, 4900.0, 2.0, 0.5);
    r.bump(frost, 3e-6);
    out.push(preset("Light Guide Frosted", "Plastic", &["light guide", "light pipe", "led", "diffuser", "frosted", "indicator"], MaterialType::PlasticTransparent, r));
    out
}
