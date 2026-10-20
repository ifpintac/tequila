//! FDM 3D Print: PLA as a filament printer lays it down, from the PLA 3D
//! Print materials for Blender, node for node. The surface is the FDM Layer
//! Lines group (`tequila_materials::groups`): layer lines up the walls, the
//! seam, raster lines on the top and bottom skins, 0.2 mm layers at their
//! real size. Each preset builds the rest as the Blender material does: the
//! filament's colour darkened in the creases and varied a little from layer
//! to layer, a satin that is rougher in the creases and glossier on the bed
//! side, a sheen along the layers, and for the translucent ones a tinted,
//! slightly hazy body.
//!
//! The colours are the Blender materials' own, converted from linear sRGB.
//! A part drawn Y up (glTF, OBJ, FBX) wants the group's Build Along Z off.

use crate::{base, preset, Out, Preset, Recipe};
use tequila_core::math::vec3;
use tequila_core::spectrum::Spectrum;
use tequila_materials::graph::{Node, NodeId, NodeKind};
use tequila_materials::groups::NodeGroup;
use tequila_materials::material_type::{MaterialType, Param};

const CATEGORY: &str = "FDM 3D Print";

/// A colour from Blender, linear sRGB, in the rendering space.
fn blender(r: f32, g: f32, b: f32) -> Spectrum {
    Spectrum::from_linear_srgb(vec3(r, g, b))
}

/// The layer lines, and its outputs by name.
struct Lines(NodeId);

impl Lines {
    fn new(r: &mut Recipe) -> Lines {
        Lines(r.graph.add_node(Node::new(NodeKind::Group(NodeGroup::FdmLayerLines))))
    }

    fn out(&self, name: &str) -> Out {
        let index = NodeGroup::FdmLayerLines.outputs().iter().position(|o| o.name == name).unwrap_or_else(|| panic!("no output {name}"));
        Out(self.0, index as u8)
    }
}

/// The roughness the Blender materials share: `roughness`, rougher by
/// `in_creases` in the creases, varied by `variation` over a few
/// millimetres - a satin as a print has it.
fn satin(r: &mut Recipe, lines: &Lines, roughness: f32, in_creases: f32, detail: f32, variation: f32) -> Out {
    let crease = r.math("multiply", lines.out("Cavity"), in_creases);
    let rough = r.math("add", roughness, crease);
    let noise = r.node("noise_texture").w("Vector", lines.out("Position (mm)")).f("Scale", 1.6).f("Detail", detail).f("Roughness", 0.5).out("Fac");
    let centred = r.math("subtract", noise, 0.5);
    let varied = r.math("multiply", centred, variation);
    r.math("add", rough, varied)
}

/// A solid colour: PLA's satin and the sheen along the layers. The Blender
/// material's 6% of light under the surface is left out: it moved no part
/// of a rendered print by a level.
fn solid(name: &'static str, tags: &'static [&'static str], rgb: Spectrum, roughness: f32) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Plastic, |m| {
        m.base_color = rgb;
        m.specular_ior = 1.46;
        m.specular_roughness = roughness;
    }));
    let lines = Lines::new(&mut r);

    // Darker in the creases, and each layer a shade lighter or darker.
    let dark = r.math("multiply", lines.out("Cavity"), 0.3);
    let lit = r.math("subtract", 1.0, dark);
    let layer = r.math("subtract", lines.out("Layer Random"), 0.5);
    let layer = r.math("multiply", layer, 0.05);
    let shade = r.math("add", lit, layer);
    let colour_node = r.node("mix").set("data_type", "color").set("blend_type", "multiply").f("Factor", 1.0).c("A", rgb).w("B", shade).out("Result");
    r.to(Param::BaseColor, colour_node);

    // Glossier where it lay on the bed.
    let walls = satin(&mut r, &lines, roughness, 0.12, 4.0, 0.1);
    let to_bed = r.math("subtract", 0.16, walls);
    let on_bed = r.math("multiply", to_bed, lines.out("Bottom Mask"));
    let rough = r.node("math").set("operation", "add").set("clamp", true).w("Value", walls).w("Value 2", on_bed).out("Value");
    r.to(Param::SpecularRoughness, rough);

    // The sheen along the layers: Blender's anisotropy of 0.3 on the walls,
    // as Tequila's stretches the same highlight, sqrt(1 - (1 - 0.9 a)^2).
    let short = r.node("math").set("operation", "multiply_add").w("Value", lines.out("Wall Factor")).f("Value 2", -0.27).f("Value 3", 1.0).out("Value");
    let square = r.math("multiply", short, short);
    let rest = r.math("subtract", 1.0, square);
    let anisotropy = r.node("math").set("operation", "sqrt").w("Value", rest).out("Value");
    r.to(Param::Anisotropy, anisotropy);
    r.to(Param::Tangent, lines.out("Tangent"));
    r.to(Param::Bump, lines.out("Bump"));

    preset(name, CATEGORY, tags, MaterialType::Plastic, r)
}

/// Absorption per millimetre in Tequila's rendering space that looks, over
/// the thicknesses a print has, as `srgb` does in Blender's.
///
/// Converting the colour a millimetre keeps is not enough. Blender absorbs
/// in sRGB's primaries and Tequila in ACEScg's, which are wider: a deep
/// orange that is nearly all sRGB red is, in ACEScg, partly green, and the
/// green channel is absorbed fast. Compounded over 20 mm of plastic, the
/// colour converted at one millimetre came out several times too dark. So
/// the three coefficients are fitted, by a plain search, to what Blender
/// shows from a millimetre to four and a half centimetres - light that
/// scatters inside a part goes further than its thickness - compared as
/// the eye does, on the square root. Through 20 mm of the orange, that is
/// within three levels of Blender's red and seven of its blue.
fn absorbed_in_rendering_space(srgb: [f32; 3]) -> [f32; 3] {
    const DEPTHS: [f32; 9] = [1.0, 2.0, 3.0, 5.0, 8.0, 12.0, 20.0, 30.0, 45.0];
    let error = |sigma: [f32; 3]| -> f32 {
        DEPTHS
            .iter()
            .map(|&d| {
                let wanted = srgb.map(|s| (-s * d).exp());
                let kept = Spectrum { r: (-sigma[0] * d).exp(), g: (-sigma[1] * d).exp(), b: (-sigma[2] * d).exp() }.to_linear_srgb();
                [kept.x, kept.y, kept.z]
                    .iter()
                    .zip(wanted)
                    .map(|(k, w)| (k.clamp(0.0, 1.0).sqrt() - w.sqrt()).powi(2))
                    .sum::<f32>()
            })
            .sum()
    };
    // From the conversion at a centimetre, then a coordinate search.
    let at = 10.0;
    let mut sigma = {
        let c = Spectrum::from_linear_srgb(vec3((-srgb[0] * at).exp(), (-srgb[1] * at).exp(), (-srgb[2] * at).exp()));
        [c.r, c.g, c.b].map(|v| -v.max(1e-6).ln() / at)
    };
    let mut best = error(sigma);
    let mut step = 0.5f32;
    while step > 1e-4 {
        let mut improved = false;
        for channel in 0..3 {
            for direction in [1.0, -1.0] {
                let mut tried = sigma;
                tried[channel] = (tried[channel] + direction * step * tried[channel].max(0.01)).max(0.0);
                let e = error(tried);
                if e < best {
                    (sigma, best, improved) = (tried, e, true);
                }
            }
        }
        if !improved {
            step *= 0.5;
        }
    }
    sigma
}

/// A translucent colour: a dyed, slightly hazy body that light goes
/// through, tinted at its skin as well as through its thickness.
///
/// As the Blender material has it: `absorption` and `haze` per millimetre,
/// absorbing towards `tint` and scattering a paler version of it, and the
/// surface `skin` of the way to the tint.
fn translucent(name: &'static str, tags: &'static [&'static str], tint: [f32; 3], absorption: f32, haze: f32, skin: f32) -> Preset {
    // What Blender's volume absorbs and scatters, per millimetre, and the
    // colour of the skin.
    let absorbed = tint.map(|t| absorption * (1.0 - t));
    let scatter = tint.map(|t| haze * (0.1 + 0.9 * t));
    let surface = tint.map(|t| 1.0 - skin + skin * t);
    let kept = absorbed_in_rendering_space(absorbed).map(|a| (-a).exp());
    let mut r = Recipe::new(base(MaterialType::PlasticTransparent, |m| {
        m.transmission_weight = 1.0;
        m.specular_ior = 1.46;
        m.specular_roughness = 0.08;
        m.transmission_depth = 0.001;
        m.volume_density = 1.0;
        m.transmission_color = Spectrum { r: kept[0], g: kept[1], b: kept[2] };
        m.transmission_scatter = blender(scatter[0], scatter[1], scatter[2]);
        m.transmission_scatter_anisotropy = 0.45;
        m.transmission_tint = blender(surface[0], surface[1], surface[2]);
    }));
    let lines = Lines::new(&mut r);
    let walls = satin(&mut r, &lines, 0.08, 0.2, 2.0, 0.06);
    let rough = r.node("math").set("operation", "add").set("clamp", true).w("Value", walls).f("Value 2", 0.0).out("Value");
    r.to(Param::SpecularRoughness, rough);
    r.to(Param::Bump, lines.out("Bump"));

    preset(name, CATEGORY, tags, MaterialType::PlasticTransparent, r)
}

pub fn all() -> Vec<Preset> {
    vec![
        // The everyday filaments: white, black and grey, and the primaries.
        solid("FDM PLA White", &["3d print", "fdm", "pla", "layer lines", "filament", "white"], blender(0.838, 0.838, 0.807), 0.37),
        solid("FDM PLA Black", &["3d print", "fdm", "pla", "layer lines", "filament", "black"], blender(0.0129, 0.0129, 0.0144), 0.30),
        solid("FDM PLA Grey", &["3d print", "fdm", "pla", "layer lines", "filament", "grey", "gray"], blender(0.15, 0.157, 0.165), 0.34),
        solid("FDM PLA Red", &["3d print", "fdm", "pla", "layer lines", "filament", "red"], blender(0.552, 0.0116, 0.0159), 0.34),
        solid("FDM PLA Blue", &["3d print", "fdm", "pla", "layer lines", "filament", "blue"], blender(0.0091, 0.0762, 0.412), 0.34),
        solid("FDM PLA Yellow", &["3d print", "fdm", "pla", "layer lines", "filament", "yellow"], blender(0.888, 0.552, 0.0037), 0.35),
        // Translucent: natural (undyed, milky), and the clear colours people
        // print light pipes, lamp shades and cases in.
        translucent("FDM PLA Translucent Natural", &["3d print", "fdm", "pla", "layer lines", "translucent", "natural", "clear", "milky"], [0.9047, 0.9047, 0.8714], 0.0, 0.14, 0.0),
        translucent("FDM PLA Translucent Red", &["3d print", "fdm", "pla", "layer lines", "translucent", "transparent", "red"], [0.4621, 0.0, 0.0056], 0.45, 0.07, 0.55),
        translucent("FDM PLA Translucent Orange", &["3d print", "fdm", "pla", "layer lines", "translucent", "transparent", "orange"], [0.9301, 0.0762, 0.0006], 0.42, 0.07, 0.55),
        translucent("FDM PLA Translucent Green", &["3d print", "fdm", "pla", "layer lines", "translucent", "transparent", "green"], [0.0, 0.3515, 0.0513], 0.45, 0.07, 0.55),
        translucent("FDM PLA Translucent Blue", &["3d print", "fdm", "pla", "layer lines", "translucent", "transparent", "blue"], [0.0, 0.063, 0.4969], 0.45, 0.07, 0.55),
    ]
}
