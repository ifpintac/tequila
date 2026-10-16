//! More metal finishes: brushed, blasted and anodised in more metals and
//! colours; platings and coatings (PVD, black oxide, black nickel, zinc);
//! colours heat and chemistry give a metal (temper colours, Corten, bronze
//! patina, mill scale); and pattern-welded and laminated metals (Damascus,
//! mokume gane).

use crate::finish::{brush_lines, grooves_along_axis};
use crate::metal::{anodised, brushed, brushed_with, polished, satin};
use crate::recipe::Linear;
use crate::{base, metal_ior, micro, noise, plus, preset, rgb, srgb, wear, Preset, Recipe};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::{MaterialType, Param};

fn metal(ior: &str, tint: Spectrum, roughness: f32) -> tequila_materials::openpbr::Material {
    crate::metal::metal(ior, tint, roughness)
}

/// A metal blasted with fine beads: an even matte grain with no direction.
fn blasted(name: &'static str, tags: &'static [&'static str], ior: &str, tint: Spectrum, low: f32, high: f32) -> Preset {
    let mut r = Recipe::new(metal(ior, tint, (low + high) * 0.5));
    let gloss = wear(&mut r, low, high, 12.0);
    r.to(Param::SpecularRoughness, gloss);
    micro(&mut r, 9000.0, 3e-6);
    preset(name, "Metal", tags, MaterialType::Metal, r)
}

/// Beaten by hand: overlapping dimples, each a facet of its own, the
/// hammer's polish on them.
fn hammered(name: &'static str, tags: &'static [&'static str], ior: &str, tint: Spectrum, low: f32, high: f32, size: f32) -> Preset {
    let mut r = Recipe::new(metal(ior, tint, low));
    let dimples = r.node("voronoi_texture").set("feature", "smooth_f1").f("Scale", 1.0 / size).f("Smoothness", 0.6).f("Randomness", 0.85).out("Distance");
    r.bump(dimples, size * 0.08);
    let gloss = wear(&mut r, low, high, 20.0);
    r.to(Param::SpecularRoughness, gloss);
    preset(name, "Metal", tags, MaterialType::Metal, r)
}

/// A dark coating over steel - an oxide, a plating, a vapour-deposited film
/// - satin, with the faint machining under it and a little unevenness in
/// its colour, as a coating always has.
#[allow(clippy::too_many_arguments)]
fn coated(name: &'static str, tags: &'static [&'static str], ior: &str, dark: Spectrum, light: Spectrum, low: f32, high: f32, machined: bool) -> Preset {
    let mut r = Recipe::new(metal(ior, dark.lerp(light, 0.5), (low + high) * 0.5));
    let mottle = noise(&mut r, 70.0, 4.0, 0.55);
    let colour = r.node("mix").w("Factor", mottle).c("A", dark).c("B", light).out("Result");
    r.to(Param::BaseColor, colour);
    let gloss = r.range(mottle, (0.3, 0.7), (low, high));
    if machined {
        let grooves = brush_lines(&mut r, 2400.0, 40.0);
        r.bump(grooves, 8e-7);
        let gloss = plus(&mut r, grooves, 0.04, gloss);
        r.to(Param::SpecularRoughness, gloss);
    } else {
        r.to(Param::SpecularRoughness, gloss);
        micro(&mut r, 6000.0, 1.5e-6);
    }
    preset(name, "Metal", tags, MaterialType::Metal, r)
}

/// Hard anodised: thick, dark and matte, a slightly olive grey that is
/// never quite even.
fn hard_anodised() -> Preset {
    coated(
        "Aluminium Hard Anodised",
        &["aluminium", "hard anodised", "cookware", "matte", "dark"],
        "Aluminium",
        srgb("#3E3E39"),
        srgb("#55554E"),
        0.48,
        0.58,
        false,
    )
}

/// Zinc plated: a thin zinc coat, then a chromate that colours it. The
/// yellow one is the iridescent gold-green of a hardware-store bolt; the
/// clear one a bluish silver.
fn zinc(name: &'static str, tags: &'static [&'static str], colours: &[(f32, Spectrum)], roughness: f32) -> Preset {
    let mut r = Recipe::new(metal("Aluminium", colours[0].1, roughness));
    let swirl = r.node("noise_texture").f("Scale", 90.0).f("Detail", 5.0).f("Roughness", 0.6).f("Distortion", 0.6).out("Fac");
    let colour = r.ramp(swirl, Linear, colours);
    r.to(Param::BaseColor, colour);
    let gloss = wear(&mut r, roughness * 0.85, roughness * 1.2, 25.0);
    r.to(Param::SpecularRoughness, gloss);
    micro(&mut r, 7000.0, 1e-6);
    preset(name, "Metal", tags, MaterialType::Metal, r)
}

/// Heat-tinted titanium: the oxide a flame grows, thicker where it was
/// hotter, its colours running from straw through bronze and purple to
/// blue and back to a pale gold - up the part, wavering.
fn heat_tinted() -> Preset {
    let mut r = Recipe::new(metal("Titanium", rgb(0.85, 0.8, 0.7), 0.12));
    let coords = r.node("texture_coordinate").out("Object");
    let y = r.node("separate_xyz").w("Vector", coords).out("Y");
    let waver = noise(&mut r, 60.0, 3.0, 0.5);
    let heat = r.range(y, (-0.025, 0.025), (0.0, 1.0));
    let heat = plus(&mut r, waver, 0.18, heat);
    let colour = r.ramp(
        heat,
        Linear,
        &[
            (0.05, srgb("#C9C2B0")),
            (0.2, srgb("#D8B65E")),
            (0.35, srgb("#B4743C")),
            (0.5, srgb("#7A3F86")),
            (0.62, srgb("#3A55B0")),
            (0.75, srgb("#5E9CC9")),
            (0.88, srgb("#B8C9A4")),
            (1.0, srgb("#D7C27A")),
        ],
    );
    r.to(Param::BaseColor, colour);
    let gloss = wear(&mut r, 0.1, 0.16, 25.0);
    r.to(Param::SpecularRoughness, gloss);
    preset("Titanium Heat Tinted", "Metal", &["titanium", "heat tinted", "anodised", "rainbow", "exhaust", "temper"], MaterialType::Metal, r)
}

/// Damascus: steel and nickel steel forged in layers and folded, then
/// etched - the nickel layers stay bright and polished, the steel ones go
/// dark and a hair lower - so the layers show as contour lines that wander
/// and swirl.
fn damascus() -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.metal_ior = metal_ior("Iron");
        m.base_metalness = 1.0;
        m.base_color = rgb(0.5, 0.5, 0.52);
        m.specular_roughness = 0.22;
    }));
    let layers = r
        .node("wave_texture")
        .set("type", "bands")
        .set("direction", "x")
        .f("Scale", 420.0)
        .f("Distortion", 16.0)
        .f("Detail", 4.0)
        .f("Detail Scale", 0.6)
        .f("Detail Roughness", 0.55)
        .out("Fac");
    let bright = r.smooth(layers, (0.48, 0.72), (0.0, 1.0));
    let colour = r.node("mix").w("Factor", bright).c("A", srgb("#4A4C52")).c("B", srgb("#B4B6B9")).out("Result");
    r.to(Param::BaseColor, colour);
    let gloss = r.range(bright, (0.0, 1.0), (0.36, 0.14));
    r.to(Param::SpecularRoughness, gloss);
    r.bump(bright, 6e-6);
    preset("Damascus Steel", "Metal", &["damascus", "pattern welded", "steel", "knife", "etched"], MaterialType::Generic, r)
}

/// Mokume gane: layers of copper, silver and gold-coloured alloys forged
/// into one and carved, so they show as a wood grain of metals.
fn mokume() -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.metal_ior = metal_ior("Silver");
        m.base_metalness = 1.0;
        m.base_color = rgb(0.8, 0.6, 0.5);
        m.specular_roughness = 0.14;
    }));
    let layers = r
        .node("wave_texture")
        .set("type", "rings")
        .set("direction", "y")
        .f("Scale", 170.0)
        .f("Distortion", 4.0)
        .f("Detail", 3.0)
        .f("Detail Scale", 0.8)
        .out("Fac");
    let colour = r.ramp(
        layers,
        Linear,
        &[
            (0.0, srgb("#A8644A")),
            (0.25, srgb("#C98C70")),
            (0.4, srgb("#D9D6CF")),
            (0.56, srgb("#CFCBC2")),
            (0.68, srgb("#BFA165")),
            (0.86, srgb("#5A463C")),
            (1.0, srgb("#A8644A")),
        ],
    );
    r.to(Param::BaseColor, colour);
    let gloss = wear(&mut r, 0.12, 0.2, 25.0);
    r.to(Param::SpecularRoughness, gloss);
    r.bump(layers, 4e-6);
    preset("Mokume Gane", "Metal", &["mokume gane", "laminated", "copper", "silver", "jewellery"], MaterialType::Generic, r)
}

/// Hot-rolled steel: the blue-black mill scale the rolling leaves, streaked
/// along the roll, flaked off here and there to the grey steel under it.
fn mill_scale() -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.metal_ior = metal_ior("Iron");
        m.base_metalness = 1.0;
        m.base_color = rgb(0.2, 0.2, 0.22);
        m.specular_roughness = 0.5;
    }));
    let along = r.stretched([0.08, 1.0, 1.0]);
    let streak = r.node("noise_texture").w("Vector", along).f("Scale", 140.0).f("Detail", 6.0).f("Roughness", 0.6).out("Fac");
    let patches = r.node("noise_texture").f("Scale", 45.0).f("Detail", 8.0).f("Roughness", 0.65).f("Distortion", 0.4).out("Fac");
    let bare = r.smooth(patches, (0.6, 0.66), (0.0, 1.0));
    let scale_colour = r.ramp(streak, Linear, &[(0.25, srgb("#6E737C")), (0.5, srgb("#858A93")), (0.75, srgb("#9A9EA6"))]);
    let colour = r.node("mix").w("Factor", bare).w("A", scale_colour).c("B", srgb("#B4B6B9")).out("Result");
    r.to(Param::BaseColor, colour);
    let rough = r.range(streak, (0.3, 0.7), (0.42, 0.58));
    let rough = r.mix_float(bare, rough, 0.36);
    r.to(Param::SpecularRoughness, rough);
    let edge = plus(&mut r, streak, 0.2, bare);
    r.bump(edge, 1.2e-5);
    preset("Steel Hot Rolled", "Metal", &["steel", "hot rolled", "mill scale", "raw", "industrial"], MaterialType::Generic, r)
}

/// Weathering steel, its rust grown into a stable, dense patina: a deep,
/// velvety brown-orange, darker and redder in places, fine-grained.
fn corten() -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = srgb("#6B331C");
        m.base_metalness = 0.0;
        m.specular_roughness = 0.82;
        m.base_diffuse_roughness = 0.5;
    }));
    let broad = r.node("noise_texture").f("Scale", 25.0).f("Detail", 6.0).f("Roughness", 0.6).out("Fac");
    let fine = noise(&mut r, 900.0, 4.0, 0.6);
    let tone = plus(&mut r, fine, 0.35, broad);
    let colour = r.ramp(tone, Linear, &[(0.2, srgb("#3E1C12")), (0.45, srgb("#5C2A18")), (0.65, srgb("#7A3A1E")), (0.85, srgb("#985028")), (1.05, srgb("#A8622F"))]);
    r.to(Param::BaseColor, colour);
    let grain = noise(&mut r, 4000.0, 2.0, 0.55);
    let height = plus(&mut r, fine, 0.6, grain);
    r.bump(height, 3e-5);
    preset("Steel Corten", "Metal", &["steel", "corten", "weathering", "rust", "patina", "architectural"], MaterialType::Generic, r)
}

/// Statuary bronze: a chemical patina, brown-black, rubbed back to the
/// bronze where hands and cloths reach.
fn bronze_patina() -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.metal_ior = metal_ior("Copper");
        m.base_color = rgb(0.62, 0.5, 0.32);
        m.base_metalness = 0.5;
        m.specular_roughness = 0.4;
    }));
    let rubbed = r.node("noise_texture").f("Scale", 40.0).f("Detail", 6.0).f("Roughness", 0.6).f("Distortion", 0.3).out("Fac");
    let fine = noise(&mut r, 600.0, 3.0, 0.6);
    let wear_field = plus(&mut r, fine, 0.2, rubbed);
    let bare = r.smooth(wear_field, (0.66, 0.78), (0.0, 1.0));
    r.to(Param::BaseMetalness, bare);
    let patina = r.ramp(fine, Linear, &[(0.25, srgb("#2B2216")), (0.5, srgb("#463823")), (0.75, srgb("#625437")), (0.95, srgb("#7A7258"))]);
    let colour = r.node("mix").w("Factor", bare).w("A", patina).c("B", rgb(0.62, 0.5, 0.32)).out("Result");
    r.to(Param::BaseColor, colour);
    let rough = r.range(bare, (0.0, 1.0), (0.55, 0.22));
    r.to(Param::SpecularRoughness, rough);
    micro(&mut r, 5000.0, 4e-6);
    preset("Bronze Patina", "Metal", &["bronze", "patina", "statuary", "aged", "brown"], MaterialType::Generic, r)
}

/// Printed titanium, as it comes off a laser powder bed: a fine grain of
/// half-melted powder, dull and slightly grey-gold, with the layers a hair
/// apart under it.
fn printed_titanium() -> Preset {
    let mut r = Recipe::new(metal("Titanium", rgb(0.7, 0.68, 0.64), 0.6));
    let powder = r.node("voronoi_texture").set("feature", "smooth_f1").f("Scale", 22000.0).f("Smoothness", 0.8).out("Distance");
    let lumps = noise(&mut r, 3500.0, 3.0, 0.6);
    let coords = r.node("texture_coordinate").out("Object");
    let y = r.node("separate_xyz").w("Vector", coords).out("Y");
    let layer = r.math("multiply", y, std::f32::consts::TAU / 0.00006);
    let layers = r.math("sine", layer, 0.0);
    let height = plus(&mut r, lumps, 0.8, powder);
    let height = plus(&mut r, layers, 0.15, height);
    r.bump(height, 1.6e-5);
    let gloss = r.range(lumps, (0.3, 0.7), (0.66, 0.54));
    r.to(Param::SpecularRoughness, gloss);
    preset("Titanium 3D Printed", "Metal", &["titanium", "3d print", "slm", "powder bed", "additive"], MaterialType::Metal, r)
}

/// Rolled sheet as it leaves the mill: faint streaks along the roll, a
/// dull uneven sheen, a touch darker in some streaks than others.
fn mill_finish() -> Preset {
    let mut r = Recipe::new(base(MaterialType::Anisotropic, |m| {
        m.metal_ior = metal_ior("Aluminium");
        m.base_color = rgb(0.86, 0.86, 0.86);
        m.specular_roughness = 0.34;
        m.specular_roughness_anisotropy = 0.7;
    }));
    let along = r.stretched([0.02, 1.0, 1.0]);
    let streak = r.node("noise_texture").w("Vector", along).f("Scale", 300.0).f("Detail", 5.0).f("Roughness", 0.6).out("Fac");
    let colour = r.node("mix").w("Factor", streak).c("A", rgb(0.78, 0.78, 0.78)).c("B", rgb(0.92, 0.92, 0.92)).out("Result");
    r.to(Param::BaseColor, colour);
    let gloss = r.range(streak, (0.3, 0.7), (0.4, 0.28));
    r.to(Param::SpecularRoughness, gloss);
    let grooves = brush_lines(&mut r, 3000.0, 120.0);
    r.bump(grooves, 6e-7);
    grooves_along_axis(&mut r, [1.0, 0.0, 0.0]);
    preset("Aluminium Mill Finish", "Metal", &["aluminium", "mill finish", "raw", "sheet", "rolled"], MaterialType::Anisotropic, r)
}

/// Cross brushed: brushed one way, then the other, so the lines cross and
/// neither direction wins.
fn cross_brushed() -> Preset {
    let mut r = Recipe::new(metal("Aluminium", rgb(0.95, 0.95, 0.95), 0.3));
    let one = brush_lines(&mut r, 1800.0, 60.0);
    let turned = r.node("texture_coordinate").out("Object");
    let turned = r.node("mapping").w("Vector", turned).v("Rotation", [0.0, 0.0, std::f32::consts::FRAC_PI_2]).out("Vector");
    let turned = r.node("mapping").w("Vector", turned).v("Scale", [1.0 / 60.0, 1.0, 1.0]).out("Vector");
    let other = r.node("noise_texture").w("Vector", turned).f("Scale", 1800.0).f("Detail", 3.0).f("Roughness", 0.62).out("Fac");
    let both = r.math("maximum", one, other);
    r.bump(both, 1.5e-6);
    let gloss = r.range(both, (0.4, 0.8), (0.27, 0.34));
    r.to(Param::SpecularRoughness, gloss);
    preset("Aluminium Cross Brushed", "Metal", &["aluminium", "cross brushed", "crosshatch", "satin"], MaterialType::Metal, r)
}

pub fn all() -> Vec<Preset> {
    let white = Spectrum::ONE;
    let brass = rgb(0.9223, 0.8458, 0.5604);
    vec![
        // Brushed, in more metals, and the finest of them.
        brushed("Copper Brushed", &["copper", "brushed", "warm"], "Copper", white, 0.28),
        brushed("Brass Brushed", &["brass", "brushed", "fitting", "hardware"], "Gold", brass, 0.26),
        brushed("Titanium Brushed", &["titanium", "brushed", "watch"], "Titanium", rgb(0.8643, 0.8559, 0.8412), 0.3),
        brushed("Chrome Brushed", &["chrome", "brushed", "satin chrome", "fixture"], "Chromium", white, 0.22),
        brushed("Bronze Brushed", &["bronze", "brushed", "warm", "hardware"], "Gold", rgb(0.62, 0.5, 0.42), 0.28),
        brushed_with("Stainless Hairline", &["steel", "stainless", "hairline", "fine brushed", "appliance"], "Nickel", rgb(0.8458, 0.8541, 0.8689), 0.2, 0.96, (4200.0, 160.0)),
        brushed_with("Steel PVD Black Brushed", &["steel", "pvd", "black", "brushed", "watch"], "Chromium", rgb(0.05, 0.05, 0.055), 0.24, 0.92, (2400.0, 80.0)),
        mill_finish(),
        cross_brushed(),
        // Blasted.
        blasted("Stainless Bead Blasted", &["steel", "stainless", "blasted", "matte"], "Nickel", rgb(0.84, 0.85, 0.86), 0.42, 0.5),
        blasted("Titanium Bead Blasted", &["titanium", "blasted", "matte", "watch"], "Titanium", rgb(0.78, 0.77, 0.75), 0.46, 0.54),
        // Anodised, in more colours, and hard anodised.
        anodised("Aluminium Anodised Gold", &["aluminium", "anodised", "gold", "colour"], srgb("#C9A24E"), 0.34, 0.44),
        anodised("Aluminium Anodised Green", &["aluminium", "anodised", "green", "colour"], srgb("#3F7F52"), 0.34, 0.44),
        anodised("Aluminium Anodised Purple", &["aluminium", "anodised", "purple", "colour"], srgb("#5E3C8E"), 0.34, 0.44),
        anodised("Aluminium Anodised Orange", &["aluminium", "anodised", "orange", "colour"], srgb("#D86A26"), 0.34, 0.44),
        anodised("Aluminium Anodised Space Grey", &["aluminium", "anodised", "grey", "space grey", "laptop"], srgb("#8B8E93"), 0.36, 0.46),
        anodised("Aluminium Anodised Rose", &["aluminium", "anodised", "rose", "pink", "colour"], srgb("#D9A79C"), 0.34, 0.44),
        anodised("Aluminium Anodised Champagne", &["aluminium", "anodised", "champagne", "gold", "laptop"], srgb("#D8C6A1"), 0.34, 0.44),
        anodised("Aluminium Anodised Midnight", &["aluminium", "anodised", "blue", "midnight", "dark"], srgb("#34405C"), 0.36, 0.46),
        hard_anodised(),
        // Platings and coatings.
        polished("Black Nickel", &["nickel", "black", "plating", "hardware", "dark"], "Nickel", rgb(0.3, 0.31, 0.33), 0.03, 0.06),
        satin("Steel PVD Gunmetal", &["steel", "pvd", "gunmetal", "watch", "dark"], "Titanium", rgb(0.36, 0.36, 0.38), 0.18, 0.24),
        satin("Steel PVD Bronze", &["steel", "pvd", "bronze", "tap", "fixture"], "Gold", rgb(0.62, 0.5, 0.38), 0.16, 0.22),
        coated("Steel Black Oxide", &["steel", "black oxide", "blackened", "tool", "fastener"], "Iron", srgb("#2A2C31"), srgb("#3C3F45"), 0.32, 0.44, true),
        coated("Steel PVD Black", &["steel", "pvd", "dlc", "black", "watch"], "Chromium", rgb(0.04, 0.04, 0.045), rgb(0.06, 0.06, 0.066), 0.14, 0.2, false),
        zinc("Steel Zinc Plated Yellow", &["steel", "zinc", "plated", "chromate", "yellow", "fastener"], &[(0.0, srgb("#C9B45A")), (0.35, srgb("#D8C46A")), (0.55, srgb("#B9C46E")), (0.72, srgb("#D7A2A8")), (0.88, srgb("#A9B6D6")), (1.0, srgb("#D4BC64"))], 0.16),
        zinc("Steel Zinc Plated Clear", &["steel", "zinc", "plated", "blue", "fastener"], &[(0.0, srgb("#C7CFDD")), (0.5, srgb("#DCE0E6")), (1.0, srgb("#BFC9DC"))], 0.18),
        // Metals of their own.
        polished("Tungsten Carbide", &["tungsten", "carbide", "ring", "jewellery", "dark"], "Tungsten", white, 0.03, 0.06),
        satin("Pewter", &["pewter", "tin", "tableware", "dull", "grey"], "Silver", rgb(0.6, 0.61, 0.62), 0.26, 0.36),
        polished("Bronze Polished", &["bronze", "warm", "statue", "mirror"], "Gold", rgb(0.62, 0.5, 0.42), 0.04, 0.08),
        satin("Bronze Satin", &["bronze", "warm", "satin", "hardware"], "Gold", rgb(0.62, 0.5, 0.42), 0.22, 0.3),
        // Hammered.
        hammered("Silver Hammered", &["silver", "hammered", "beaten", "tableware", "jewellery"], "Silver", white, 0.08, 0.16, 0.0045),
        hammered("Brass Hammered", &["brass", "hammered", "beaten", "craft"], "Gold", brass, 0.12, 0.22, 0.0055),
        hammered("Steel Hammered", &["steel", "hammered", "planished", "forged"], "Iron", rgb(0.6, 0.61, 0.62), 0.2, 0.32, 0.006),
        // Heat, chemistry and the forge.
        heat_tinted(),
        damascus(),
        mokume(),
        mill_scale(),
        corten(),
        bronze_patina(),
        printed_titanium(),
    ]
}
