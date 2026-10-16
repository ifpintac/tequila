//! Composites: fibre in resin.
//!
//! A woven composite is a cloth of tows - flat bundles of thousands of
//! straight filaments - set in resin under a clear coat, and it reads as
//! itself through what the tows do with light. Each tow reflects along its
//! filaments, so warp and weft, a quarter turn apart, catch a highlight in
//! turn and the weave shimmers as the part turns. Besides that:
//! - a tow is flat, not round, and fills its pitch all but a hair of resin;
//! - it dives under its neighbour at each crossing, darker there;
//! - its filaments streak it lengthwise, and no two tows are quite alike
//!   in shade or in how straight they lie;
//! - the resin over it is a coat of its own: glass-deep on a gloss part,
//!   a soft sheen on a satin or matte one, absent on dry fabric.
//!
//! Forged carbon is chopped tow pressed in a mould: flakes lying every way,
//! each with its own highlight. Chopped strand mat is glass fibre laid at
//! random in a translucent resin. Both get their own generators.

use crate::recipe::{Linear, Out};
use crate::{base, noise, plus, preset, srgb, Preset, Recipe};
use tequila_materials::material_type::{MaterialType, Param};

#[derive(Clone, Copy)]
pub enum Weave {
    /// Over one, under one: a check.
    Plain,
    /// Over two, under two, stepping one each row: the diagonal.
    Twill,
    /// Tows all one way.
    Unidirectional,
}

#[derive(Clone, Copy)]
pub enum Finish {
    /// A clear coat polished to a mirror.
    Gloss,
    /// A clear coat with a soft sheen.
    Satin,
    /// A clear coat sanded flat.
    Matte,
    /// No resin at all: the fabric itself.
    Dry,
}

impl Finish {
    /// The coat's weight and roughness.
    fn coat(self) -> (f32, f32) {
        match self {
            Finish::Gloss => (1.0, 0.015),
            Finish::Satin => (1.0, 0.18),
            Finish::Matte => (1.0, 0.38),
            Finish::Dry => (0.0, 0.0),
        }
    }
}

pub struct Composite {
    pub name: &'static str,
    pub tags: &'static [&'static str],
    pub weave: Weave,
    /// A tow's width, in metres: 3K carbon about 2.2 mm, 12K about 5.
    pub tow: f32,
    /// The warp's and the weft's fibre, sRGB: different for a hybrid.
    pub warp: &'static str,
    pub weft: &'static str,
    /// How metallic each fibre's reflection is: carbon's graphite reflects
    /// like a dark metal, which is what makes its weave shimmer; aramid
    /// and glass are polymers and glass, with only a silky sheen.
    pub metal: (f32, f32),
    pub finish: Finish,
    /// The filaments' own roughness under the coat, and how strongly they
    /// pull the highlight along themselves (0 to 1).
    pub roughness: f32,
    pub anisotropy: f32,
    /// How much the filaments streak a tow, 0 to 1.
    pub streaks: f32,
}

const COMPOSITES: &[Composite] = &[
    // ---- carbon ----------------------------------------------------------------------
    // 3K 2x2 twill under a gloss coat: the carbon fibre of car parts and
    // phone cases.
    Composite {
        name: "Carbon Fibre",
        tags: &["carbon", "composite", "weave", "twill", "3k", "gloss", "cfrp"],
        weave: Weave::Twill,
        tow: 2.2e-3,
        warp: "#77777D",
        weft: "#77777D",
        metal: (0.9, 0.9),
        finish: Finish::Gloss,
        roughness: 0.2,
        anisotropy: 0.95,
        streaks: 0.6,
    },
    Composite {
        name: "Carbon Fibre Plain Weave",
        tags: &["carbon", "composite", "weave", "plain", "3k", "gloss", "cfrp"],
        weave: Weave::Plain,
        tow: 2.2e-3,
        warp: "#77777D",
        weft: "#77777D",
        metal: (0.9, 0.9),
        finish: Finish::Gloss,
        roughness: 0.2,
        anisotropy: 0.95,
        streaks: 0.6,
    },
    Composite {
        name: "Carbon Fibre Satin",
        tags: &["carbon", "composite", "weave", "twill", "3k", "satin", "cfrp"],
        weave: Weave::Twill,
        tow: 2.2e-3,
        warp: "#77777D",
        weft: "#77777D",
        metal: (0.9, 0.9),
        finish: Finish::Satin,
        roughness: 0.24,
        anisotropy: 0.95,
        streaks: 0.6,
    },
    Composite {
        name: "Carbon Fibre Matte",
        tags: &["carbon", "composite", "weave", "twill", "3k", "matte", "cfrp"],
        weave: Weave::Twill,
        tow: 2.2e-3,
        warp: "#77777D",
        weft: "#77777D",
        metal: (0.9, 0.9),
        finish: Finish::Matte,
        roughness: 0.3,
        anisotropy: 0.8,
        streaks: 0.6,
    },
    // 12K: tows four times the filaments, about twice as wide - bolder.
    Composite {
        name: "Carbon Fibre 12K Twill",
        tags: &["carbon", "composite", "weave", "twill", "12k", "gloss", "cfrp"],
        weave: Weave::Twill,
        tow: 4.8e-3,
        warp: "#77777D",
        weft: "#77777D",
        metal: (0.9, 0.9),
        finish: Finish::Gloss,
        roughness: 0.18,
        anisotropy: 0.95,
        streaks: 0.7,
    },
    // Unidirectional: every tow one way, the sheen running with them.
    Composite {
        name: "Carbon Fibre Unidirectional",
        tags: &["carbon", "composite", "unidirectional", "ud", "gloss", "cfrp"],
        weave: Weave::Unidirectional,
        tow: 3.0e-3,
        warp: "#77777D",
        weft: "#77777D",
        metal: (0.9, 0.9),
        finish: Finish::Gloss,
        roughness: 0.16,
        anisotropy: 0.97,
        streaks: 0.85,
    },
    // Spread tow: tows spread into flat tapes a couple of centimetres wide,
    // woven as a plain check - big, flat squares.
    Composite {
        name: "Carbon Fibre Spread Tow",
        tags: &["carbon", "composite", "spread tow", "plain", "tape", "gloss", "cfrp"],
        weave: Weave::Plain,
        tow: 17.0e-3,
        warp: "#75757B",
        weft: "#75757B",
        metal: (0.9, 0.9),
        finish: Finish::Gloss,
        roughness: 0.16,
        anisotropy: 0.96,
        streaks: 0.8,
    },
    // ---- aramid --------------------------------------------------------------------
    // Kevlar: golden aramid, silky, deepened to amber by the resin.
    Composite {
        name: "Kevlar Plain Weave",
        tags: &["kevlar", "aramid", "composite", "weave", "plain", "gold", "yellow"],
        weave: Weave::Plain,
        tow: 2.0e-3,
        warp: "#B8892A",
        weft: "#B8892A",
        metal: (0.3, 0.3),
        finish: Finish::Gloss,
        roughness: 0.28,
        anisotropy: 0.7,
        streaks: 0.5,
    },
    Composite {
        name: "Kevlar Twill",
        tags: &["kevlar", "aramid", "composite", "weave", "twill", "gold", "yellow"],
        weave: Weave::Twill,
        tow: 2.0e-3,
        warp: "#B8892A",
        weft: "#B8892A",
        metal: (0.3, 0.3),
        finish: Finish::Gloss,
        roughness: 0.28,
        anisotropy: 0.7,
        streaks: 0.5,
    },
    // The fabric before it meets resin: paler, matte, soft.
    Composite {
        name: "Kevlar Dry Fabric",
        tags: &["kevlar", "aramid", "fabric", "dry", "plain", "gold", "yellow"],
        weave: Weave::Plain,
        tow: 1.6e-3,
        warp: "#D4AC45",
        weft: "#D4AC45",
        metal: (0.2, 0.2),
        finish: Finish::Dry,
        roughness: 0.45,
        anisotropy: 0.55,
        streaks: 0.6,
    },
    // ---- hybrids: carbon one way, aramid the other ----------------------------------
    Composite {
        name: "Carbon Kevlar Hybrid",
        tags: &["carbon", "kevlar", "aramid", "hybrid", "composite", "twill", "yellow", "black"],
        weave: Weave::Twill,
        tow: 2.2e-3,
        warp: "#77777D",
        weft: "#B8892A",
        metal: (0.9, 0.3),
        finish: Finish::Gloss,
        roughness: 0.22,
        anisotropy: 0.8,
        streaks: 0.6,
    },
    Composite {
        name: "Carbon Aramid Blue",
        tags: &["carbon", "aramid", "hybrid", "composite", "twill", "blue", "black"],
        weave: Weave::Twill,
        tow: 2.2e-3,
        warp: "#77777D",
        weft: "#1E3F8C",
        metal: (0.9, 0.3),
        finish: Finish::Gloss,
        roughness: 0.22,
        anisotropy: 0.8,
        streaks: 0.6,
    },
    Composite {
        name: "Carbon Aramid Red",
        tags: &["carbon", "aramid", "hybrid", "composite", "twill", "red", "black"],
        weave: Weave::Twill,
        tow: 2.2e-3,
        warp: "#77777D",
        weft: "#8E1A1E",
        metal: (0.9, 0.3),
        finish: Finish::Gloss,
        roughness: 0.22,
        anisotropy: 0.8,
        streaks: 0.6,
    },
    // ---- other fibres --------------------------------------------------------------
    // Basalt: dark, with a bronze sheen along its filaments.
    Composite {
        name: "Basalt Fibre",
        tags: &["basalt", "composite", "weave", "twill", "bronze", "dark"],
        weave: Weave::Twill,
        tow: 2.4e-3,
        warp: "#8C7862",
        weft: "#8C7862",
        metal: (0.65, 0.65),
        finish: Finish::Gloss,
        roughness: 0.22,
        anisotropy: 0.75,
        streaks: 0.6,
    },
    // Flax: a natural fibre in clear resin, tan and a little uneven.
    Composite {
        name: "Flax Fibre Composite",
        tags: &["flax", "linen", "natural fibre", "composite", "twill", "tan", "bio"],
        weave: Weave::Twill,
        tow: 2.6e-3,
        warp: "#8F6E44",
        weft: "#8F6E44",
        metal: (0.0, 0.0),
        finish: Finish::Satin,
        roughness: 0.32,
        anisotropy: 0.45,
        streaks: 0.8,
    },
];

pub fn all() -> Vec<Preset> {
    let mut out: Vec<Preset> = COMPOSITES.iter().map(woven).collect();
    out.push(forged("Forged Carbon", &["carbon", "forged", "chopped", "composite", "marbled", "gloss"], Finish::Gloss));
    out.push(forged("Forged Carbon Matte", &["carbon", "forged", "chopped", "composite", "marbled", "matte"], Finish::Matte));
    out.push(fiberglass_woven());
    out.push(chopped_strand_mat());
    out.push(gelcoat());
    out
}

/// The weave on one plane, (u, v) in metres: which tow is on top (1 the
/// warp, which runs along v), its height across its width and along its
/// length, and its shade.
struct Tows {
    top: Out,
    height: Out,
    shade: Out,
    /// Each tow's own small turn away from straight, as a rotation.
    wander: Out,
}

fn tows(r: &mut Recipe, u: Out, v: Out, c: &Composite, seed: f32) -> Tows {
    let pitch = 1.0 / c.tow;
    let uu = r.math("multiply", u, pitch);
    let vv = r.math("multiply", v, pitch);
    let iu = r.math("floor", uu, 0.0);
    let iv = r.math("floor", vv, 0.0);
    let fu = r.math("fraction", uu, 0.0);
    let fv = r.math("fraction", vv, 0.0);
    let top = match c.weave {
        Weave::Plain => {
            let s = r.math("add", iu, iv);
            let m = r.math("floored_modulo", s, 2.0);
            r.math("less_than", m, 1.0)
        }
        Weave::Twill => {
            let s = r.math("subtract", iu, iv);
            let m = r.math("floored_modulo", s, 4.0);
            r.math("less_than", m, 2.0)
        }
        Weave::Unidirectional => r.math("add", 1.0, 0.0),
    };
    // Flat across its width - a tow is a tape, not a yarn - falling off
    // only at the edge, where a hair of resin shows between it and the next.
    let across = |r: &mut Recipe, f: Out| {
        let centred = r.math("subtract", f, 0.5);
        let off = r.math("absolute", centred, 0.0);
        r.smooth(off, (0.43, 0.5), (1.0, 0.25))
    };
    let warp_across = across(r, fu);
    let weft_across = across(r, fv);
    // Along its float it rises from where it surfaced and dives where it
    // goes under; a unidirectional tow never dives.
    let along = |r: &mut Recipe, f: Out, run: f32| {
        if run <= 0.0 {
            return r.math("add", 1.0, 0.0);
        }
        let a = r.math("multiply", f, std::f32::consts::PI);
        let s = r.math("sine", a, 0.0);
        r.range(s, (0.0, 1.0), (1.0 - run, 1.0))
    };
    let float = match c.weave {
        Weave::Unidirectional => 0.0,
        _ => 0.35,
    };
    let warp_float = along(r, fv, float);
    let weft_float = along(r, fu, float);
    let warp_h = r.math("multiply", warp_across, warp_float);
    let weft_h = r.math("multiply", weft_across, weft_float);
    let height = r.mix_float(top, weft_h, warp_h);
    // Each tow its own shade and its own small wander from straight.
    let warp_id = r.node("combine_xyz").w("X", iu).f("Y", seed).f("Z", 3.0).out("Vector");
    let weft_id = r.node("combine_xyz").w("X", iv).f("Y", seed + 500.0).f("Z", 7.0).out("Vector");
    let warp_rand = r.node("white_noise_texture").w("Vector", warp_id).add();
    let weft_rand = r.node("white_noise_texture").w("Vector", weft_id).add();
    let own = r.mix_float(top, Out(weft_rand, 0), Out(warp_rand, 0));
    let own_tone = r.range(own, (0.0, 1.0), (0.9, 1.08));
    let wander_pick = r.node("mix").set("data_type", "float").w("Factor", top).w("A", Out(weft_rand, 1)).w("B", Out(warp_rand, 1)).out("Result");
    let wander = r.range(wander_pick, (0.0, 1.0), (-0.012, 0.012));
    // Filaments: streaks along the tow, fine across it and long along it.
    let warp_streak_at = r.node("combine_xyz").w("X", uu).w("Y", vv).f("Z", seed).out("Vector");
    let warp_streak_space = r.node("mapping").w("Vector", warp_streak_at).v("Scale", [24.0, 0.6, 1.0]).out("Vector");
    let weft_streak_space = r.node("mapping").w("Vector", warp_streak_at).v("Scale", [0.6, 24.0, 1.0]).out("Vector");
    let warp_streak = r.node("noise_texture").w("Vector", warp_streak_space).f("Scale", 1.0).f("Detail", 3.0).f("Roughness", 0.6).out("Fac");
    let weft_streak = r.node("noise_texture").w("Vector", weft_streak_space).f("Scale", 1.0).f("Detail", 3.0).f("Roughness", 0.6).out("Fac");
    let streak = r.mix_float(top, weft_streak, warp_streak);
    let streak_tone = r.range(streak, (0.3, 0.7), (1.0 - 0.18 * c.streaks, 1.0 + 0.12 * c.streaks));
    // Darker where a tow dives, and in the resin between tows.
    let depth_shade = r.range(height, (0.0, 1.0), (0.62, 1.0));
    let shade = r.math("multiply", own_tone, streak_tone);
    let shade = r.math("multiply", shade, depth_shade);
    let height = plus(r, streak, 0.06 * c.streaks, height);
    Tows { top, height, shade, wander }
}

/// A woven composite, tri-planar in object space so it lies on any part.
pub fn woven(c: &Composite) -> Preset {
    let (coat, coat_roughness) = c.finish.coat();
    let warp = srgb(c.warp);
    let weft = srgb(c.weft);
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = warp.lerp(weft, 0.5);
        m.coat_weight = coat;
        m.coat_roughness = coat_roughness;
        m.specular_roughness = c.roughness;
        m.specular_roughness_anisotropy = c.anisotropy;
        m.base_metalness = (c.metal.0 + c.metal.1) * 0.5;
    }));
    let coords = r.node("texture_coordinate").out("Object");
    let normal = r.node("texture_coordinate").out("Normal");
    let xyz = r.node("separate_xyz").w("Vector", coords).add();
    let nxyz = r.node("separate_xyz").w("Vector", normal).add();
    let mut weights = Vec::new();
    for axis in 0..3u8 {
        let a = r.math("absolute", Out(nxyz, axis), 0.0);
        weights.push(r.math("power", a, 6.0));
    }
    let sum = r.math("add", weights[0], weights[1]);
    let sum = r.math("add", sum, weights[2]);
    let weights: Vec<Out> = weights.into_iter().map(|w| r.math("divide", w, sum)).collect();
    let planes = [(Out(xyz, 1), Out(xyz, 2)), (Out(xyz, 0), Out(xyz, 2)), (Out(xyz, 0), Out(xyz, 1))];
    let (mut top, mut height, mut shade, mut wander): (Option<Out>, Option<Out>, Option<Out>, Option<Out>) = (None, None, None, None);
    for (i, (u, v)) in planes.into_iter().enumerate() {
        let t = tows(&mut r, u, v, c, i as f32 * 17.3);
        let weigh = |r: &mut Recipe, acc: Option<Out>, x: Out| {
            let wx = r.math("multiply", x, weights[i]);
            match acc {
                Some(a) => r.math("add", a, wx),
                None => wx,
            }
        };
        top = Some(weigh(&mut r, top, t.top));
        height = Some(weigh(&mut r, height, t.height));
        shade = Some(weigh(&mut r, shade, t.shade));
        wander = Some(weigh(&mut r, wander, t.wander));
    }
    let (top, height, shade, wander) = (top.unwrap(), height.unwrap(), shade.unwrap(), wander.unwrap());
    // Warp and weft a quarter turn apart: half of the rotation's range.
    let turn = r.range(top, (0.0, 1.0), (0.5, 0.0));
    let turn = r.math("add", turn, wander);
    r.to(Param::AnisotropyRotation, turn);
    let fibre = r.node("mix").w("Factor", top).c("A", weft).c("B", warp).out("Result");
    if c.metal.0 != c.metal.1 {
        let metal = r.range(top, (0.0, 1.0), (c.metal.1, c.metal.0));
        r.to(Param::BaseMetalness, metal);
    }
    // Warp and weft in turn: one direction catches the light where the
    // other does not. The anisotropic lobe does this by itself where the
    // light is right; the tint keeps the weave legible everywhere else, and
    // in views that have no anisotropy at all.
    let facing = r.range(top, (0.0, 1.0), (0.55, 1.0));
    let shade = r.math("multiply", shade, facing);
    let colour = r.mix("multiply", 1.0, fibre, shade);
    r.to(Param::BaseColor, colour);
    // The resin between tows is glassier than the fibre.
    let rough = r.range(height, (0.0, 0.6), (c.roughness * 0.5, c.roughness));
    r.to(Param::SpecularRoughness, rough);
    let depth = match c.finish {
        // Under a thick coat the weave is only a whisper of relief.
        Finish::Gloss | Finish::Satin => 6e-6,
        Finish::Matte => 1.2e-5,
        Finish::Dry => 4e-5,
    } * (c.tow / 2.2e-3).sqrt();
    r.bump(height, depth);
    preset(c.name, "Composite", c.tags, MaterialType::Generic, r)
}

/// Forged carbon: chopped tow pressed in a mould, flakes lying every way,
/// each catching the light by its own direction - a marbling of black and
/// grey - with the filaments streaking each flake.
fn forged(name: &'static str, tags: &'static [&'static str], finish: Finish) -> Preset {
    let (coat, coat_roughness) = finish.coat();
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = srgb("#77777D");
        m.coat_weight = coat;
        m.coat_roughness = coat_roughness;
        m.specular_roughness = 0.26;
        m.specular_roughness_anisotropy = 0.9;
        m.base_metalness = 0.9;
    }));
    let coords = r.node("texture_coordinate").out("Object");
    // Flakes with ragged edges: cells under a warp.
    let warp = r.node("noise_texture").w("Vector", coords).f("Scale", 90.0).f("Detail", 3.0).f("Roughness", 0.55).out("Color");
    let warped = r.node("vector_math").set("operation", "multiply_add").w("Vector", warp).v("Vector 2", [0.006; 3]).w("Vector 3", coords).out("Vector");
    // Two sizes of flake, the small ones lying over the large here and there.
    let large = r.node("voronoi_texture").w("Vector", warped).f("Scale", 85.0).f("Randomness", 1.0).add();
    let small = r.node("voronoi_texture").w("Vector", warped).f("Scale", 210.0).f("Randomness", 1.0).add();
    let lay = noise(&mut r, 30.0, 3.0, 0.5);
    let small_on_top = r.smooth(lay, (0.5, 0.56), (0.0, 1.0));
    let large_angle = r.node("separate_color").w("Color", Out(large, 1)).out("Red");
    let small_angle = r.node("separate_color").w("Color", Out(small, 1)).out("Red");
    let angle = r.mix_float(small_on_top, large_angle, small_angle);
    r.to(Param::AnisotropyRotation, angle);
    // Each flake's filaments, streaking it along its own direction.
    let theta = r.math("multiply", angle, std::f32::consts::PI);
    let c = r.math("cosine", theta, 0.0);
    let s = r.math("sine", theta, 0.0);
    let xyz = r.node("separate_xyz").w("Vector", coords).add();
    let along_a = r.math("multiply", Out(xyz, 0), c);
    let along = r.node("math").set("operation", "multiply_add").w("Value", Out(xyz, 1)).w("Value 2", s).w("Value 3", along_a).out("Value");
    let across_a = r.math("multiply", Out(xyz, 1), c);
    let neg_s = r.math("multiply", s, -1.0);
    let across = r.node("math").set("operation", "multiply_add").w("Value", Out(xyz, 0)).w("Value 2", neg_s).w("Value 3", across_a).out("Value");
    let streak_at = r.node("combine_xyz").w("X", across).w("Y", along).w("Z", Out(xyz, 2)).out("Vector");
    let streak_space = r.node("mapping").w("Vector", streak_at).v("Scale", [9000.0, 300.0, 300.0]).out("Vector");
    let streak = r.node("noise_texture").w("Vector", streak_space).f("Scale", 1.0).f("Detail", 3.0).f("Roughness", 0.6).out("Fac");
    let streak_tone = r.range(streak, (0.3, 0.7), (0.82, 1.14));
    // Flakes a shade apart, and the resin between them darker.
    let large_tone = r.node("separate_color").w("Color", Out(large, 1)).out("Green");
    let small_tone = r.node("separate_color").w("Color", Out(small, 1)).out("Green");
    let tone = r.mix_float(small_on_top, large_tone, small_tone);
    let tone = r.range(tone, (0.0, 1.0), (0.2, 1.3));
    let large_edge = r.node("voronoi_texture").set("feature", "distance_to_edge").w("Vector", warped).f("Scale", 85.0).f("Randomness", 1.0).out("Distance");
    let small_edge = r.node("voronoi_texture").set("feature", "distance_to_edge").w("Vector", warped).f("Scale", 210.0).f("Randomness", 1.0).out("Distance");
    let edge = r.mix_float(small_on_top, large_edge, small_edge);
    let seam = r.smooth(edge, (0.0, 0.03), (0.85, 1.0));
    let shade = r.math("multiply", tone, streak_tone);
    let shade = r.math("multiply", shade, seam);
    let black = r_colour(&mut r, "#77777D");
    let colour = r.mix("multiply", 1.0, black, shade);
    r.to(Param::BaseColor, colour);
    let height = plus(&mut r, streak, 0.2, seam);
    r.bump(height, if matches!(finish, Finish::Matte) { 1.0e-5 } else { 5e-6 });
    preset(name, "Composite", tags, MaterialType::Generic, r)
}

/// A constant colour as a node output, for a mix to multiply.
fn r_colour(r: &mut Recipe, hex: &str) -> Out {
    r.node("mix").f("Factor", 0.0).c("A", srgb(hex)).c("B", srgb(hex)).out("Result")
}

/// Woven glass roving in clear resin: the laminate's own translucent
/// green-white, the weave a little whiter where the bundles lie thick, and
/// the resin between them clearer.
fn fiberglass_woven() -> Preset {
    let c = Composite {
        name: "Fiberglass Woven Roving",
        tags: &["fiberglass", "fibreglass", "glass fibre", "grp", "frp", "composite", "woven roving", "translucent"],
        weave: Weave::Plain,
        tow: 4.5e-3,
        warp: "#D9DED3",
        weft: "#D9DED3",
        metal: (0.0, 0.0),
        finish: Finish::Gloss,
        roughness: 0.3,
        anisotropy: 0.0,
        streaks: 0.7,
    };
    let mut r = Recipe::new(base(MaterialType::Translucent, |m| {
        m.base_color = srgb("#C9D1C2");
        m.subsurface_color = srgb("#C9D1C2");
        m.subsurface_weight = 0.6;
        m.subsurface_radius = tequila_core::spectrum::spectrum(0.06, 0.07, 0.06);
        m.specular_roughness = 0.04;
    }));
    let coords = r.node("texture_coordinate").out("Object");
    let normal = r.node("texture_coordinate").out("Normal");
    let xyz = r.node("separate_xyz").w("Vector", coords).add();
    let nxyz = r.node("separate_xyz").w("Vector", normal).add();
    let mut weights = Vec::new();
    for axis in 0..3u8 {
        let a = r.math("absolute", Out(nxyz, axis), 0.0);
        weights.push(r.math("power", a, 6.0));
    }
    let sum = r.math("add", weights[0], weights[1]);
    let sum = r.math("add", sum, weights[2]);
    let weights: Vec<Out> = weights.into_iter().map(|w| r.math("divide", w, sum)).collect();
    let planes = [(Out(xyz, 1), Out(xyz, 2)), (Out(xyz, 0), Out(xyz, 2)), (Out(xyz, 0), Out(xyz, 1))];
    let mut height: Option<Out> = None;
    let mut shade: Option<Out> = None;
    for (i, (u, v)) in planes.into_iter().enumerate() {
        let t = tows(&mut r, u, v, &c, i as f32 * 23.1);
        let weigh = |r: &mut Recipe, acc: Option<Out>, x: Out| {
            let wx = r.math("multiply", x, weights[i]);
            match acc {
                Some(a) => r.math("add", a, wx),
                None => wx,
            }
        };
        height = Some(weigh(&mut r, height, t.height));
        shade = Some(weigh(&mut r, shade, t.shade));
    }
    let (height, shade) = (height.unwrap(), shade.unwrap());
    // Thick bundles whiter, the resin between them a clearer green.
    let colour = r.ramp(height, Linear, &[(0.4, srgb("#C2CBBD")), (1.0, srgb("#DCE1D6"))]);
    let lift = r.range(shade, (0.5, 1.1), (0.93, 1.04));
    let colour = r.mix("multiply", 1.0, colour, lift);
    r.to(Param::BaseColor, colour);
    r.to(Param::SubsurfaceColor, colour);
    r.bump(height, 1.2e-5);
    preset("Fiberglass Woven Roving", "Composite", c.tags, MaterialType::Translucent, r)
}

/// Chopped strand mat: glass strands a few centimetres long laid every way
/// in a translucent resin, white where they lie thick.
fn chopped_strand_mat() -> Preset {
    let mut r = Recipe::new(base(MaterialType::Translucent, |m| {
        m.base_color = srgb("#B9C4B4");
        m.subsurface_color = srgb("#B9C4B4");
        m.subsurface_weight = 0.65;
        m.subsurface_radius = tequila_core::spectrum::spectrum(0.08, 0.09, 0.08);
        m.specular_roughness = 0.06;
    }));
    let coords = r.node("texture_coordinate").out("Object");
    let xyz = r.node("separate_xyz").w("Vector", coords).add();
    let mut strands: Option<Out> = None;
    // Layers of strands, each layer's patches lying their own way.
    for (k, (cells, density)) in [(25.0f32, 380.0f32), (40.0, 520.0), (60.0, 450.0)].into_iter().enumerate() {
        let shift = r.node("vector_math").set("operation", "add").w("Vector", coords).v("Vector 2", [k as f32 * 3.1, k as f32 * 1.7, k as f32 * 2.3]).out("Vector");
        let patch = r.node("voronoi_texture").w("Vector", shift).f("Scale", cells).f("Randomness", 1.0).add();
        let angle = r.node("separate_color").w("Color", Out(patch, 1)).out("Red");
        let theta = r.math("multiply", angle, std::f32::consts::PI);
        let c = r.math("cosine", theta, 0.0);
        let s = r.math("sine", theta, 0.0);
        let a = r.math("multiply", Out(xyz, 1), c);
        let neg_s = r.math("multiply", s, -1.0);
        let across = r.node("math").set("operation", "multiply_add").w("Value", Out(xyz, 0)).w("Value 2", neg_s).w("Value 3", a).out("Value");
        let lines = r.math("multiply", across, density);
        let f = r.math("fraction", lines, 0.0);
        let off = r.math("subtract", f, 0.5);
        let off = r.math("absolute", off, 0.0);
        let strand = r.smooth(off, (0.1, 0.32), (1.0, 0.0));
        // Strands come and go: chopped, and not every line is filled.
        let gap_field = r.node("noise_texture").w("Vector", shift).f("Scale", cells * 6.0).f("Detail", 2.0).out("Fac");
        let present = r.smooth(gap_field, (0.48, 0.58), (0.0, 1.0));
        let strand = r.math("multiply", strand, present);
        strands = Some(match strands {
            Some(prev) => r.math("maximum", prev, strand),
            None => strand,
        });
    }
    let strands = strands.unwrap();
    let cloud = noise(&mut r, 20.0, 4.0, 0.55);
    let resin = r.ramp(cloud, Linear, &[(0.3, srgb("#A3B39F")), (0.7, srgb("#B8C5B2"))]);
    let colour = r.node("mix").w("Factor", strands).w("A", resin).c("B", srgb("#E8ECE3")).out("Result");
    r.to(Param::BaseColor, colour);
    r.to(Param::SubsurfaceColor, colour);
    r.bump(strands, 8e-6);
    preset("Fiberglass Chopped Strand Mat", "Composite", &["fiberglass", "fibreglass", "chopped strand mat", "csm", "grp", "composite", "translucent"], MaterialType::Translucent, r)
}

/// A boat's gelcoat: a hard, glossy white skin over the laminate, the
/// weave beneath printing through it as the faintest ripple.
fn gelcoat() -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = srgb("#F1F0EA");
        m.coat_weight = 1.0;
        m.coat_roughness = 0.03;
        m.specular_roughness = 0.12;
    }));
    let coords = r.node("texture_coordinate").out("Object");
    let xyz = r.node("separate_xyz").w("Vector", coords).add();
    // Print-through: the roving's check, very soft.
    let u = r.math("multiply", Out(xyz, 0), 220.0);
    let v = r.math("multiply", Out(xyz, 2), 220.0);
    let su = r.math("sine", u, 0.0);
    let sv = r.math("sine", v, 0.0);
    let check = r.math("multiply", su, sv);
    let ripple = noise(&mut r, 40.0, 3.0, 0.5);
    let height = plus(&mut r, check, 0.25, ripple);
    r.bump(height, 3e-6);
    let tone = noise(&mut r, 8.0, 3.0, 0.5);
    let colour = r.ramp(tone, Linear, &[(0.3, srgb("#EDECE5")), (0.7, srgb("#F4F3ED"))]);
    r.to(Param::BaseColor, colour);
    preset("Fiberglass Gelcoat White", "Composite", &["fiberglass", "fibreglass", "gelcoat", "boat", "marine", "gloss", "white", "grp"], MaterialType::Generic, r)
}
