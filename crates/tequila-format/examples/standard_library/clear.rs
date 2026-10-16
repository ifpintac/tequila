//! What light goes into: glass, gems, liquids, the translucent solids, and
//! what gives light out.

use crate::{base, noise, plain, preset, rgb, srgb, Preset, Recipe};
use tequila_materials::material_type::{MaterialType, Param};

pub fn all() -> Vec<Preset> {
    let mut out = Vec::new();

    // ---- glass -------------------------------------------------------------------
    let glass = |name: &'static str, tags: &'static [&'static str], colour, depth, roughness| {
        plain(name, "Glass", tags, MaterialType::GlassSolid, move |m| {
            m.specular_ior = 1.52;
            m.specular_roughness = roughness;
            m.transmission_color = colour;
            m.transmission_depth = depth;
        })
    };
    out.push(glass("Glass Clear", &["glass", "clear", "window"], rgb(0.9799, 0.9897, 0.983), 0.05, 0.0));
    out.push(glass("Glass Amber Bottle", &["glass", "bottle", "amber"], rgb(0.408, 0.2238, 0.0536), 0.02, 0.04));
    out.push(glass("Glass Green Bottle", &["glass", "bottle", "green"], rgb(0.241, 0.3818, 0.2106), 0.02, 0.03));
    out.push(glass("Glass Blue Tint", &["glass", "blue", "tinted", "decorative"], srgb("#6E9AC8"), 0.02, 0.0));
    out.push(plain("Acrylic Clear", "Glass", &["acrylic", "pmma", "clear"], MaterialType::GlassSolid, |m| {
        m.specular_ior = 1.49;
        m.specular_roughness = 0.01;
        m.transmission_color = rgb(0.9734, 0.9734, 0.9734);
        m.transmission_depth = 0.08;
    }));
    out.push(plain("Acrylic Smoked", "Glass", &["acrylic", "smoked", "tinted"], MaterialType::GlassSolid, |m| {
        m.specular_ior = 1.49;
        m.specular_roughness = 0.02;
        m.transmission_color = rgb(0.1027, 0.1024, 0.1107);
        m.transmission_depth = 0.03;
    }));
    out.push(plain("Sapphire Crystal", "Glass", &["sapphire", "watch", "crystal", "clear"], MaterialType::Gem, |m| {
        m.specular_ior = 1.77;
        m.transmission_color = rgb(0.964, 0.9726, 0.9962);
    }));

    // Frosted: acid-etched, a fine satin over the whole face.
    let mut r = Recipe::new(base(MaterialType::GlassSolid, |m| {
        m.specular_ior = 1.52;
        m.specular_roughness = 0.38;
        m.transmission_color = rgb(0.9627, 0.9722, 0.9731);
    }));
    let etch = noise(&mut r, 6000.0, 2.0, 0.5);
    r.bump(etch, 2e-6);
    out.push(preset("Glass Frosted", "Glass", &["glass", "frosted", "diffuser", "etched"], MaterialType::GlassSolid, r));

    // Reeded: parallel flutes pressed into the sheet.
    let mut r = Recipe::new(base(MaterialType::GlassSolid, |m| {
        m.specular_ior = 1.52;
        m.transmission_color = rgb(0.97, 0.985, 0.98);
        m.transmission_depth = 0.05;
    }));
    let flutes = r.node("wave_texture").f("Scale", 52.0).out("Fac");
    r.bump(flutes, 3e-4);
    out.push(preset("Glass Reeded", "Glass", &["glass", "reeded", "fluted", "ribbed", "decorative"], MaterialType::GlassSolid, r));

    // ---- gems ----------------------------------------------------------------------
    let gem = |name: &'static str, tags: &'static [&'static str], ior: f32, abbe: f32, colour| {
        plain(name, "Gem", tags, MaterialType::Gem, move |m| {
            m.specular_ior = ior;
            m.transmission_dispersion_abbe = abbe;
            m.transmission_color = colour;
        })
    };
    out.push(gem("Diamond", &["diamond", "jewellery", "brilliant", "fire"], 2.417, 55.0, rgb(1.0, 1.0, 1.0)));
    out.push(gem("Cubic Zirconia", &["zirconia", "cz", "jewellery", "fire"], 2.16, 30.0, rgb(1.0, 1.0, 1.0)));
    out.push(gem("Ruby", &["ruby", "corundum", "red", "jewellery"], 1.77, 72.0, rgb(0.602, 0.1758, 0.2057)));
    out.push(gem("Sapphire Blue", &["sapphire", "corundum", "blue", "jewellery"], 1.77, 72.0, rgb(0.2011, 0.2496, 0.8127)));
    out.push(gem("Emerald", &["emerald", "beryl", "green", "jewellery"], 1.577, 70.0, rgb(0.3632, 0.7025, 0.3897)));
    out.push(gem("Amethyst", &["amethyst", "quartz", "purple", "jewellery"], 1.544, 70.0, rgb(0.51, 0.3285, 0.7846)));
    out.push(gem("Citrine", &["citrine", "quartz", "yellow", "jewellery"], 1.544, 70.0, rgb(0.7933, 0.6185, 0.2158)));
    out.push(gem("Garnet", &["garnet", "almandine", "red", "jewellery"], 1.79, 56.0, rgb(0.3575, 0.0854, 0.0777)));
    out.push(gem("Peridot", &["peridot", "olivine", "green", "jewellery"], 1.654, 55.0, rgb(0.6145, 0.7733, 0.2034)));
    out.push(gem("Topaz Blue", &["topaz", "blue", "jewellery"], 1.62, 75.0, rgb(0.5755, 0.7316, 0.9178)));
    out.push(gem("Aquamarine", &["aquamarine", "beryl", "blue", "jewellery"], 1.577, 70.0, srgb("#9FD3DA")));
    out.push(gem("Morganite", &["morganite", "beryl", "pink", "jewellery"], 1.577, 70.0, srgb("#F2C4C0")));

    // ---- liquids -------------------------------------------------------------------
    let liquid = |name: &'static str, tags: &'static [&'static str], change: Box<dyn FnOnce(&mut tequila_materials::openpbr::Material)>| plain(name, "Liquid", tags, MaterialType::Liquid, change);
    out.push(liquid("Water", &["water", "liquid", "fill"], Box::new(|m| {
        m.specular_ior = 1.333;
        m.transmission_color = rgb(0.967, 0.9806, 0.9971);
    })));
    out.push(liquid("Beer Lager", &["beer", "lager", "drink", "alcohol"], Box::new(|m| {
        m.specular_ior = 1.34;
        m.transmission_depth = 0.04;
        m.transmission_color = rgb(0.6768, 0.4728, 0.119);
    })));
    out.push(liquid("Red Wine", &["wine", "red", "drink", "alcohol"], Box::new(|m| {
        m.specular_ior = 1.345;
        m.transmission_depth = 0.03;
        m.transmission_color = rgb(0.3123, 0.0449, 0.0549);
    })));
    out.push(liquid("Cola", &["cola", "soda", "drink"], Box::new(|m| {
        m.specular_ior = 1.34;
        m.transmission_depth = 0.012;
        m.transmission_color = rgb(0.2874, 0.1384, 0.0475);
    })));
    out.push(liquid("Coffee Black", &["coffee", "drink", "dark"], Box::new(|m| {
        m.specular_ior = 1.34;
        m.transmission_depth = 0.004;
        m.transmission_color = rgb(0.2572, 0.1351, 0.0552);
        m.transmission_scatter = rgb(0.3, 0.3, 0.3);
    })));
    out.push(liquid("Honey", &["honey", "syrup", "kitchen"], Box::new(|m| {
        m.specular_ior = 1.49;
        m.transmission_depth = 0.015;
        m.transmission_color = rgb(0.6763, 0.4727, 0.1103);
    })));
    out.push(liquid("Olive Oil", &["oil", "olive", "kitchen"], Box::new(|m| {
        m.specular_ior = 1.47;
        m.transmission_depth = 0.04;
        m.transmission_color = rgb(0.7522, 0.7454, 0.2291);
    })));
    out.push(liquid("Milk Whole", &["milk", "dairy", "drink", "cloudy"], Box::new(|m| {
        m.specular_ior = 1.35;
        m.transmission_depth = 0.01;
        m.transmission_color = rgb(0.9791, 0.9757, 0.8835);
        m.transmission_scatter = rgb(28.3189, 31.7121, 36.8349);
    })));
    out.push(liquid("Milk Skimmed", &["milk", "dairy", "drink", "cloudy"], Box::new(|m| {
        m.specular_ior = 1.35;
        m.transmission_depth = 0.01;
        m.transmission_color = rgb(0.9768, 0.9746, 0.8819);
        m.transmission_scatter = rgb(9.3341, 11.9265, 18.0075);
    })));
    out.push(liquid("Orange Juice", &["orange", "juice", "drink", "cloudy"], Box::new(|m| {
        m.specular_ior = 1.35;
        m.transmission_depth = 0.01;
        // Cloudy with pulp: scattering strongly enough to be opaque over a
        // glass's width, the way juice is, not an orange glass.
        m.transmission_color = rgb(0.86, 0.43, 0.05);
        m.transmission_scatter = rgb(22.0, 20.0, 18.0);
        m.transmission_scatter_anisotropy = 0.5;
    })));

    // ---- translucent solids -----------------------------------------------------------
    let soft = |name: &'static str, tags: &'static [&'static str], change: Box<dyn FnOnce(&mut tequila_materials::openpbr::Material)>| plain(name, "Translucent", tags, MaterialType::Translucent, change);
    out.push(soft("Beeswax", &["wax", "beeswax", "yellow"], Box::new(|m| {
        m.base_color = rgb(0.5872, 0.4205, 0.1124);
        m.subsurface_color = rgb(0.6672, 0.4774, 0.1204);
        m.subsurface_radius = rgb(0.098, 0.067, 0.043);
        m.specular_roughness = 0.4;
    })));
    out.push(soft("Wax Candle", &["wax", "candle", "paraffin"], Box::new(|m| {
        m.base_color = rgb(0.8201, 0.7865, 0.6521);
        m.subsurface_color = rgb(0.825, 0.7799, 0.6054);
        m.subsurface_radius = rgb(0.130, 0.112, 0.128);
        m.specular_roughness = 0.35;
    })));
    out.push(soft("Gummy Red", &["gummy", "candy", "jelly", "red"], Box::new(|m| {
        m.base_color = rgb(0.3571, 0.0471, 0.024);
        m.subsurface_color = rgb(0.4939, 0.0754, 0.0464);
        m.subsurface_radius = rgb(0.271, 0.042, 0.054);
        m.specular_roughness = 0.08;
    })));
    // Jade: polished, glowing with light from inside it, and never one
    // green - clouds of paler and deeper colour through it and the odd dark
    // fleck.
    let mut r = Recipe::new(base(MaterialType::Translucent, |m| {
        m.base_color = rgb(0.1588, 0.2868, 0.1459);
        m.subsurface_color = rgb(0.281, 0.4858, 0.2538);
        m.subsurface_radius = rgb(0.087, 0.368, 0.241);
        m.specular_roughness = 0.1;
    }));
    let cloud = noise(&mut r, 30.0, 6.0, 0.6);
    let colour = r.ramp(cloud, crate::recipe::Linear, &[(0.0, srgb("#2D5A38")), (0.45, srgb("#4E8659")), (0.75, srgb("#7CAF84")), (1.0, srgb("#C4D8BC"))]);
    let specks = r.node("voronoi_texture").f("Scale", 260.0).f("Randomness", 1.0).out("Distance");
    let speck = r.smooth(specks, (0.02, 0.05), (1.0, 0.0));
    let colour = r.node("mix").f("Factor", speck).w("A", colour).c("B", srgb("#1C3424")).out("Result");
    r.to(Param::BaseColor, colour);
    r.to(Param::SubsurfaceColor, colour);
    let gloss = crate::wear(&mut r, 0.06, 0.14, 20.0);
    r.to(Param::SpecularRoughness, gloss);
    out.push(preset("Jade", "Translucent", &["jade", "stone", "green", "carving"], MaterialType::Translucent, r));

    // Soap: a bar's satin surface, worn smooth in use.
    let mut r = Recipe::new(base(MaterialType::Translucent, |m| {
        m.base_color = rgb(0.8628, 0.8555, 0.8135);
        m.subsurface_color = rgb(0.9006, 0.8889, 0.8387);
        m.subsurface_radius = rgb(0.052, 0.046, 0.040);
        m.specular_roughness = 0.3;
    }));
    let gloss = crate::wear(&mut r, 0.22, 0.38, 25.0);
    r.to(Param::SpecularRoughness, gloss);
    out.push(preset("Soap White", "Translucent", &["soap", "bar", "bathroom"], MaterialType::Translucent, r));

    // ---- lights ------------------------------------------------------------------------
    let light = |name: &'static str, tags: &'static [&'static str], colour, luminance| {
        plain(name, "Light", tags, MaterialType::Emissive, move |m| {
            m.emission_color = colour;
            m.emission_luminance = luminance;
        })
    };
    out.push(light("LED Cool White", &["led", "light", "cool", "white"], rgb(0.9217, 0.9472, 0.9925), 30.0));
    out.push(light("LED Warm White", &["led", "light", "warm", "white"], rgb(0.9026, 0.7919, 0.5584), 30.0));
    out.push(light("Screen Glow", &["screen", "display", "backlight"], rgb(0.9592, 0.969, 0.9957), 5.0));
    out.push(light("Neon Pink", &["neon", "light", "pink", "sign"], srgb("#FF3EA5"), 20.0));
    out.push(light("Neon Blue", &["neon", "light", "blue", "sign"], srgb("#2E8CFF"), 20.0));
    out
}
