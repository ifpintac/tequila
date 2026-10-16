//! Fabrics: woven, knitted, piled and felted, measured against scanned cloth
//! (Poly Haven's denim, linen, hessian, herringbone wool, boucle, velour,
//! corduroy, suede, fleece and satin).
//!
//! A woven cloth is built thread by thread rather than drawn as a pattern:
//! at each crossing of warp and weft, the weave says which is on top; each
//! yarn is round across its width and arches over the crossing it floats
//! across, twisted from plies that show as fine slanted lines; threads
//! differ a little from one another, thicken and thin along their length
//! (slubs), and leave dark gaps where they do not quite meet. The warp and
//! the weft can be different colours - denim's indigo warp over a white
//! weft is most of what denim looks like. And over it all, a fuzz of loose
//! fibre that catches the light at the edges.
//!
//! The cloth is read in object space, in metres - real thread counts - and
//! projected three ways, blended by the surface's facing (tri-planar), so a
//! part with no UVs, a CAD body, takes it as readily as a cushion does.

use crate::recipe::{Linear, Out};
use crate::{base, noise, plus, preset, rgb, srgb, Preset, Recipe};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::{MaterialType, Param};

/// Which thread is on top where.
#[derive(Clone, Copy)]
pub enum Pattern {
    /// Over one, under one.
    Plain,
    /// Warp over `over` wefts, under `under`, stepping one each pick: the
    /// diagonal of denim (3/1) and of tweed (2/2).
    Twill { over: u32, under: u32 },
    /// Pairs of threads woven as one: a basket.
    Basket,
    /// A 2/2 twill reversing every `period` warp threads.
    Herringbone { period: u32 },
}

pub struct Cloth {
    pub name: &'static str,
    pub tags: &'static [&'static str],
    pub pattern: Pattern,
    /// Threads to the metre.
    pub threads: f32,
    pub warp: Spectrum,
    pub weft: Spectrum,
    /// How much one thread differs from the next, 0 to 1.
    pub variation: f32,
    /// How much a thread thickens and thins along its length.
    pub slub: f32,
    /// How much of the pitch a thread fills; below 1 leaves gaps.
    pub fill: f32,
    /// Loose fibre over the surface, 0 to 1, and its colour.
    pub fuzz: f32,
    pub fuzz_colour: Spectrum,
    /// A mélange: fibres of another colour spun into the yarn.
    pub heather: Option<(Spectrum, f32)>,
    /// Relief, metres.
    pub depth: f32,
    pub roughness: f32,
}

pub fn all() -> Vec<Preset> {
    use Pattern::*;
    let none = None;
    let mut out: Vec<Preset> = [
        Cloth { name: "Canvas Natural", tags: &["canvas", "woven", "bag", "cotton", "duck"], pattern: Plain, threads: 820.0, warp: srgb("#CDBF9E"), weft: srgb("#C4B593"), variation: 0.1, slub: 0.35, fill: 0.93, fuzz: 0.35, fuzz_colour: srgb("#E8E0CC"), heather: none, depth: 2.2e-4, roughness: 0.85 },
        Cloth { name: "Denim Indigo", tags: &["denim", "woven", "apparel", "twill", "blue", "jeans"], pattern: Twill { over: 3, under: 1 }, threads: 2300.0, warp: srgb("#2A3D63"), weft: srgb("#8E97A8"), variation: 0.22, slub: 0.7, fill: 0.95, fuzz: 0.3, fuzz_colour: srgb("#8A97B0"), heather: Some((srgb("#4C6390"), 0.35)), depth: 1.4e-4, roughness: 0.82 },
        Cloth { name: "Denim Black", tags: &["denim", "woven", "apparel", "twill", "black", "jeans"], pattern: Twill { over: 3, under: 1 }, threads: 2300.0, warp: srgb("#262628"), weft: srgb("#58585A"), variation: 0.15, slub: 0.45, fill: 0.95, fuzz: 0.3, fuzz_colour: srgb("#58585C"), heather: Some((srgb("#3A3A3E"), 0.3)), depth: 1.4e-4, roughness: 0.84 },
        Cloth { name: "Linen Natural", tags: &["linen", "woven", "upholstery", "flax"], pattern: Plain, threads: 950.0, warp: srgb("#BDB096"), weft: srgb("#C6BAA0"), variation: 0.28, slub: 1.0, fill: 0.86, fuzz: 0.28, fuzz_colour: srgb("#D9CFBA"), heather: none, depth: 1.6e-4, roughness: 0.8 },
        Cloth { name: "Linen White", tags: &["linen", "woven", "white", "bedding"], pattern: Plain, threads: 900.0, warp: srgb("#E8E5DD"), weft: srgb("#E2DED5"), variation: 0.12, slub: 0.7, fill: 0.86, fuzz: 0.3, fuzz_colour: srgb("#F4F2EC"), heather: none, depth: 1.5e-4, roughness: 0.8 },
        Cloth { name: "Hessian", tags: &["hessian", "burlap", "jute", "sack", "coarse"], pattern: Plain, threads: 260.0, warp: srgb("#9C7F55"), weft: srgb("#A8895C"), variation: 0.25, slub: 0.8, fill: 0.72, fuzz: 0.55, fuzz_colour: srgb("#C2A57A"), heather: none, depth: 6e-4, roughness: 0.88 },
        Cloth { name: "Tweed Grey", tags: &["tweed", "wool", "woven", "upholstery", "grey"], pattern: Twill { over: 2, under: 2 }, threads: 700.0, warp: srgb("#555558"), weft: srgb("#7E7E80"), variation: 0.2, slub: 0.5, fill: 0.95, fuzz: 0.6, fuzz_colour: srgb("#A6A6A8"), heather: Some((srgb("#D8D6D0"), 0.3)), depth: 2.4e-4, roughness: 0.88 },
        Cloth { name: "Herringbone Wool", tags: &["herringbone", "wool", "woven", "suiting", "grey"], pattern: Herringbone { period: 6 }, threads: 900.0, warp: srgb("#4C4B4A"), weft: srgb("#85837F"), variation: 0.15, slub: 0.35, fill: 0.95, fuzz: 0.55, fuzz_colour: srgb("#8C8B88"), heather: Some((srgb("#6A6966"), 0.35)), depth: 2e-4, roughness: 0.88 },
        Cloth { name: "Upholstery Charcoal", tags: &["upholstery", "woven", "sofa", "charcoal"], pattern: Basket, threads: 1100.0, warp: srgb("#2F3033"), weft: srgb("#3E3F43"), variation: 0.14, slub: 0.4, fill: 0.94, fuzz: 0.5, fuzz_colour: srgb("#5E6066"), heather: Some((srgb("#6B6D73"), 0.3)), depth: 1.6e-4, roughness: 0.86 },
        Cloth { name: "Upholstery Mustard", tags: &["upholstery", "woven", "sofa", "yellow"], pattern: Basket, threads: 1100.0, warp: srgb("#B58A22"), weft: srgb("#A17A1C"), variation: 0.14, slub: 0.4, fill: 0.94, fuzz: 0.5, fuzz_colour: srgb("#D8B65C"), heather: Some((srgb("#7E6018"), 0.3)), depth: 1.6e-4, roughness: 0.86 },
        Cloth { name: "Satin Champagne", tags: &["satin", "silk", "woven", "sheen", "gold"], pattern: Twill { over: 4, under: 1 }, threads: 3200.0, warp: srgb("#C9B07E"), weft: srgb("#B39A6A"), variation: 0.05, slub: 0.1, fill: 0.99, fuzz: 0.05, fuzz_colour: srgb("#E8D8B0"), heather: none, depth: 2e-5, roughness: 0.3 },
    ]
    .iter()
    .map(woven)
    .collect();

    // ---- piled, felted, looped ------------------------------------------------------
    // Felt: matted fibres, no weave at all - a fine fibrous tooth, a soft
    // mottle where the felting was uneven, and a dry fuzz.
    let mut r = Recipe::new(base(MaterialType::Velvet, |m| {
        m.base_color = rgb(0.1844, 0.1875, 0.1996);
        m.fuzz_color = rgb(0.4071, 0.4121, 0.4311);
        m.fuzz_roughness = 0.75;
    }));
    let fibres = r.node("noise_texture").f("Scale", 2400.0).f("Detail", 8.0).f("Roughness", 0.7).f("Distortion", 1.6).out("Fac");
    let tufts = noise(&mut r, 400.0, 4.0, 0.6);
    let height = plus(&mut r, tufts, 0.5, fibres);
    r.bump(height, 1e-4);
    let mottle = noise(&mut r, 60.0, 4.0, 0.5);
    let tone = plus(&mut r, fibres, 0.5, mottle);
    let colour = r.ramp(tone, Linear, &[(0.4, rgb(0.13, 0.133, 0.142)), (1.2, rgb(0.24, 0.243, 0.256))]);
    r.to(Param::BaseColor, colour);
    out.push(preset("Felt Grey", "Fabric", &["felt", "acoustic", "soft", "grey"], MaterialType::Velvet, r));

    // Velvet: a dense pile that catches the light at grazing angles - dark
    // seen end on, bright towards the silhouette - lying this way and that
    // in patches (the crush), with the faint grain of its pile.
    for (name, tags, colour, sheen) in [
        ("Velvet Charcoal", &["velvet", "sheen", "upholstery", "charcoal"] as &'static [&'static str], rgb(0.0117, 0.0116, 0.0128), rgb(0.3061, 0.3053, 0.3248)),
        ("Velvet Deep Blue", &["velvet", "sheen", "upholstery", "blue"], rgb(0.009, 0.0105, 0.0431), rgb(0.2526, 0.3023, 0.6598)),
        ("Velvet Emerald", &["velvet", "sheen", "upholstery", "green"], srgb("#0B2A1C"), srgb("#4FA77E")),
        ("Velvet Blush", &["velvet", "sheen", "upholstery", "pink"], srgb("#B0727A"), srgb("#F2C8C8")),
        ("Velvet Crimson", &["velvet", "velour", "sheen", "red"], srgb("#5E0A16"), srgb("#E0566A")),
    ] {
        let mut r = Recipe::new(base(MaterialType::Velvet, |m| {
            m.base_color = colour;
            m.fuzz_color = sheen;
            m.fuzz_roughness = 0.4;
        }));
        let crush = r.node("noise_texture").f("Scale", 35.0).f("Detail", 5.0).f("Distortion", 1.2).out("Fac");
        let pile = r.range(crush, (0.3, 0.7), (0.28, 0.62));
        r.to(Param::FuzzRoughness, pile);
        // Crushed pile lies over and shows more of its colour, less sheen.
        let laid = r.range(crush, (0.3, 0.7), (0.8, 1.25));
        let toned = tint(&mut r, colour, laid);
        let grain = r.node("noise_texture").f("Scale", 5000.0).f("Detail", 3.0).f("Roughness", 0.7).out("Fac");
        let grain_tone = r.range(grain, (0.3, 0.7), (0.9, 1.08));
        let toned = r.mix("multiply", 1.0, toned, grain_tone);
        r.to(Param::BaseColor, toned);
        let height = plus(&mut r, crush, 0.2, grain);
        r.bump(height, 3e-5);
        out.push(preset(name, "Fabric", tags, MaterialType::Velvet, r));
    }

    // Corduroy: ribs of cut pile (wales) a few millimetres apart, each a
    // little velvet stripe, with the woven ground showing in the grooves.
    let mut r = Recipe::new(base(MaterialType::Velvet, |m| {
        m.base_color = srgb("#56603E");
        m.fuzz_color = srgb("#A3AC80");
        m.fuzz_roughness = 0.45;
    }));
    let coords = r.node("texture_coordinate").out("Object");
    let x = r.node("separate_xyz").w("Vector", coords).out("X");
    let wales = r.math("multiply", x, 400.0);
    let f = r.math("fraction", wales, 0.0);
    let centred = r.math("subtract", f, 0.5);
    let scaled = r.math("multiply", centred, 2.0 / 0.86);
    let sq = r.math("multiply", scaled, scaled);
    let rest = r.math("subtract", 1.0, sq);
    let rest = r.math("maximum", rest, 0.0);
    let rib = r.math("sqrt", rest, 0.0);
    let grain = r.node("noise_texture").f("Scale", 4500.0).f("Detail", 3.0).out("Fac");
    // The pile's fibres lie along the wale, so its texture streaks that way.
    let along = r.node("mapping").w("Vector", coords).v("Scale", [1.0, 0.3, 0.3]).out("Vector");
    let coarse = r.node("noise_texture").w("Vector", along).f("Scale", 1400.0).f("Detail", 3.0).f("Roughness", 0.5).out("Fac");
    let fine = r.node("noise_texture").w("Vector", along).f("Scale", 3800.0).f("Detail", 2.0).f("Roughness", 0.5).out("Fac");
    let streak = r.mix_float(0.45, coarse, fine);
    // Each wale cut and brushed a little differently from the next.
    let wale_index = r.math("floor", wales, 0.0);
    let wale_id = r.node("combine_xyz").w("X", wale_index).f("Y", 71.0).f("Z", 0.0).out("Vector");
    let wale_pick = r.node("white_noise_texture").w("Vector", wale_id).out("Value");
    let wale_tone = r.range(wale_pick, (0.0, 1.0), (0.93, 1.06));
    let height = plus(&mut r, grain, 0.15, rib);
    let height = plus(&mut r, streak, 0.12, height);
    r.bump(height, 4e-4);
    let groove = r.smooth(rib, (0.0, 0.55), (0.32, 1.0));
    let tone = r.range(grain, (0.3, 0.7), (0.92, 1.06));
    let streak_tone = r.range(streak, (0.35, 0.65), (0.66, 1.28));
    let shade = r.math("multiply", groove, tone);
    let shade = r.math("multiply", shade, streak_tone);
    let shade = r.math("multiply", shade, wale_tone);
    let colour = tint(&mut r, srgb("#56603E"), shade);
    r.to(Param::BaseColor, colour);
    // The pile is mostly seen by its sheen, so that is where its streaks
    // have to be.
    let sheen_streak = r.range(streak, (0.35, 0.65), (0.55, 1.3));
    let sheen_shade = r.math("multiply", sheen_streak, wale_tone);
    let sheen = tint(&mut r, srgb("#A3AC80"), sheen_shade);
    r.to(Param::FuzzColor, sheen);
    let pile = r.range(rib, (0.0, 1.0), (0.8, 0.45));
    r.to(Param::FuzzRoughness, pile);
    out.push(preset("Corduroy Olive", "Fabric", &["corduroy", "cord", "ribbed", "pile", "green"], MaterialType::Velvet, r));

    // Bouclé: yarn spun with loops, woven loosely, so the surface is a mass
    // of small curls rather than a grid - creamy, soft and deep.
    let mut r = Recipe::new(base(MaterialType::RealCloth, |m| {
        m.base_color = srgb("#E3DCCE");
        m.fuzz_weight = 0.7;
        m.fuzz_color = srgb("#F2EEE6");
        m.specular_roughness = 0.88;
    }));
    let loops = r.node("voronoi_texture").set("feature", "smooth_f1").f("Scale", 650.0).f("Smoothness", 0.5).f("Randomness", 1.0).out("Distance");
    let curl = r.node("voronoi_texture").set("feature", "distance_to_edge").f("Scale", 1300.0).f("Randomness", 1.0).out("Distance");
    let curl_ring = r.smooth(curl, (0.0, 0.12), (1.0, 0.0));
    let knot = r.range(loops, (0.0, 0.6), (1.0, 0.0));
    let height = plus(&mut r, curl_ring, 0.35, knot);
    r.bump(height, 5e-4);
    let deep = r.range(height, (0.0, 1.2), (0.62, 1.05));
    let colour = tint(&mut r, srgb("#E3DCCE"), deep);
    r.to(Param::BaseColor, colour);
    out.push(preset("Boucle Cream", "Fabric", &["boucle", "bouclé", "wool", "loop", "upholstery", "cream"], MaterialType::RealCloth, r));

    // Fleece: a brushed, open pile in soft clumps.
    let mut r = Recipe::new(base(MaterialType::Velvet, |m| {
        m.base_color = srgb("#B8A88E");
        m.fuzz_color = srgb("#E6DAC4");
        m.fuzz_roughness = 0.7;
    }));
    let clumps = r.node("noise_texture").f("Scale", 350.0).f("Detail", 6.0).f("Roughness", 0.65).f("Distortion", 2.5).out("Fac");
    let fibres = r.node("noise_texture").f("Scale", 6000.0).f("Detail", 4.0).f("Roughness", 0.7).f("Distortion", 2.5).out("Fac");
    let height = plus(&mut r, fibres, 0.4, clumps);
    r.bump(height, 6e-4);
    let deep = r.range(height, (0.3, 1.1), (0.68, 1.1));
    let colour = tint(&mut r, srgb("#B8A88E"), deep);
    r.to(Param::BaseColor, colour);
    out.push(preset("Fleece Oat", "Fabric", &["fleece", "polar", "soft", "fuzzy", "beige"], MaterialType::Velvet, r));

    // Suede: a short, soft nap with a dusty sheen, brushed one way in one
    // place and the other way in the next, so it lies in lighter and darker
    // patches; a fine fibrous tooth, mottled.
    let mut r = Recipe::new(base(MaterialType::Velvet, |m| {
        m.base_color = srgb("#7C5E45");
        m.fuzz_weight = 0.7;
        m.fuzz_color = srgb("#B79A7C");
        m.fuzz_roughness = 0.65;
    }));
    let nap = r.node("noise_texture").f("Scale", 25.0).f("Detail", 3.0).f("Distortion", 0.6).out("Fac");
    let brushed = r.range(nap, (0.3, 0.7), (0.72, 1.12));
    let fibres = r.node("noise_texture").f("Scale", 7000.0).f("Detail", 4.0).f("Roughness", 0.7).f("Distortion", 1.5).out("Fac");
    let mottle_field = noise(&mut r, 300.0, 4.0, 0.6);
    let fibre_tone = r.range(fibres, (0.3, 0.7), (0.84, 1.1));
    let mottle = r.range(mottle_field, (0.3, 0.7), (0.86, 1.08));
    let shade = r.math("multiply", brushed, fibre_tone);
    let shade = r.math("multiply", shade, mottle);
    let colour = tint(&mut r, srgb("#7C5E45"), shade);
    r.to(Param::BaseColor, colour);
    let lie = r.range(nap, (0.3, 0.7), (0.55, 0.8));
    r.to(Param::FuzzRoughness, lie);
    r.bump(fibres, 5e-5);
    out.push(preset("Suede Tan", "Fabric", &["suede", "nubuck", "leather", "soft", "brown"], MaterialType::Velvet, r));
    out
}

/// `colour` times `factor`, a node's output.
fn tint(r: &mut Recipe, colour: Spectrum, factor: Out) -> Out {
    r.node("mix").set("blend_type", "multiply").f("Factor", 1.0).c("A", colour).w("B", factor).out("Result")
}

/// One woven cloth, made.
pub fn woven(c: &Cloth) -> Preset {
    let mut r = Recipe::new(base(MaterialType::RealCloth, |m| {
        m.base_color = c.warp.lerp(c.weft, 0.5);
        m.fuzz_weight = c.fuzz;
        m.fuzz_color = c.fuzz_colour;
        m.specular_roughness = c.roughness;
    }));
    let coords = r.node("texture_coordinate").out("Object");
    let normal = r.node("texture_coordinate").out("Normal");
    let xyz = r.node("separate_xyz").w("Vector", coords).add();
    let nxyz = r.node("separate_xyz").w("Vector", normal).add();
    // Tri-planar weights: the facing along each axis, sharpened, summing to 1.
    let mut weights = Vec::new();
    for axis in 0..3u8 {
        let a = r.math("absolute", Out(nxyz, axis), 0.0);
        weights.push(r.math("power", a, 6.0));
    }
    let sum = r.math("add", weights[0], weights[1]);
    let sum = r.math("add", sum, weights[2]);
    let weights: Vec<Out> = weights.into_iter().map(|w| r.math("divide", w, sum)).collect();
    // Each plane: (u, v) along the two axes it contains.
    let planes = [(Out(xyz, 1), Out(xyz, 2)), (Out(xyz, 0), Out(xyz, 2)), (Out(xyz, 0), Out(xyz, 1))];
    let mut height: Option<Out> = None;
    let mut top: Option<Out> = None;
    let mut tone: Option<Out> = None;
    for (i, (u, v)) in planes.into_iter().enumerate() {
        let w = weave(&mut r, u, v, c, i as f32 * 31.7);
        let weigh = |r: &mut Recipe, acc: Option<Out>, x: Out| {
            let wx = r.math("multiply", x, weights[i]);
            match acc {
                Some(a) => r.math("add", a, wx),
                None => wx,
            }
        };
        height = Some(weigh(&mut r, height, w.height));
        top = Some(weigh(&mut r, top, w.warp_on_top));
        tone = Some(weigh(&mut r, tone, w.tone));
    }
    let (height, top, tone) = (height.unwrap(), top.unwrap(), tone.unwrap());
    let yarn = r.node("mix").f("Factor", top).c("A", c.weft).c("B", c.warp).out("Result");
    let yarn = match c.heather {
        Some((fleck, amount)) => {
            // Fibres of another colour spun through the yarn: fine and many.
            let flecks = r.node("noise_texture").w("Vector", coords).f("Scale", c.threads * 3.0).f("Detail", 2.0).out("Fac");
            let take = r.smooth(flecks, (0.62, 0.7), (0.0, amount));
            r.node("mix").f("Factor", take).w("A", yarn).c("B", fleck).out("Result")
        }
        None => yarn,
    };
    let colour = r.mix("multiply", 1.0, yarn, tone);
    r.to(Param::BaseColor, colour);
    r.bump(height, c.depth);
    preset(c.name, "Fabric", c.tags, MaterialType::RealCloth, r)
}

struct Woven {
    height: Out,
    warp_on_top: Out,
    tone: Out,
}

/// The weave on one plane, (u, v) in metres.
fn weave(r: &mut Recipe, u: Out, v: Out, c: &Cloth, seed: f32) -> Woven {
    let uu = r.math("multiply", u, c.threads);
    let vv = r.math("multiply", v, c.threads);
    let iu = r.math("floor", uu, 0.0);
    let iv = r.math("floor", vv, 0.0);
    let fu = r.math("fraction", uu, 0.0);
    let fv = r.math("fraction", vv, 0.0);
    // Which is on top at this crossing: 1 for the warp.
    let top = match c.pattern {
        Pattern::Plain => {
            let s = r.math("add", iu, iv);
            let m = r.math("floored_modulo", s, 2.0);
            r.math("less_than", m, 1.0)
        }
        Pattern::Twill { over, under } => {
            let s = r.math("subtract", iu, iv);
            let m = r.math("floored_modulo", s, (over + under) as f32);
            r.math("less_than", m, over as f32)
        }
        Pattern::Basket => {
            let pu = r.math("divide", iu, 2.0);
            let pu = r.math("floor", pu, 0.0);
            let pv = r.math("divide", iv, 2.0);
            let pv = r.math("floor", pv, 0.0);
            let s = r.math("add", pu, pv);
            let m = r.math("floored_modulo", s, 2.0);
            r.math("less_than", m, 1.0)
        }
        Pattern::Herringbone { period } => {
            let block = r.math("divide", iu, period as f32);
            let block = r.math("floor", block, 0.0);
            let flip = r.math("floored_modulo", block, 2.0);
            // +1 or -1: the twill's direction in this block.
            let sign = r.math("multiply", flip, -2.0);
            let sign = r.math("add", sign, 1.0);
            let step = r.math("multiply", iv, sign);
            let s = r.math("subtract", iu, step);
            let m = r.math("floored_modulo", s, 4.0);
            r.math("less_than", m, 2.0)
        }
    };
    // Each yarn round across its width; a thread that does not fill its
    // pitch leaves a gap.
    let across = |r: &mut Recipe, f: Out| {
        let centred = r.math("subtract", f, 0.5);
        let scaled = r.math("multiply", centred, 2.0 / c.fill);
        let sq = r.math("multiply", scaled, scaled);
        let rest = r.math("subtract", 1.0, sq);
        let rest = r.math("maximum", rest, 0.0);
        r.math("sqrt", rest, 0.0)
    };
    let warp_round = across(r, fu);
    let weft_round = across(r, fv);
    // Arching over the crossing it floats across.
    let arch = |r: &mut Recipe, f: Out| {
        let a = r.math("multiply", f, std::f32::consts::PI);
        let s = r.math("sine", a, 0.0);
        r.math("multiply", s, 0.35)
    };
    let warp_arch = arch(r, fv);
    let warp_arch = r.math("add", warp_arch, 0.65);
    let weft_arch = arch(r, fu);
    let weft_arch = r.math("add", weft_arch, 0.65);
    let warp_h = r.math("multiply", warp_round, warp_arch);
    let weft_h = r.math("multiply", weft_round, weft_arch);
    let surface = r.mix_float(top, weft_h, warp_h);
    // The plies: slanted lines across each yarn.
    let twist_w = {
        let a = r.math("multiply", vv, 2.5);
        let b = r.math("multiply", fu, 1.2);
        let s = r.math("add", a, b);
        let s = r.math("multiply", s, std::f32::consts::TAU);
        r.math("sine", s, 0.0)
    };
    let twist_f = {
        let a = r.math("multiply", uu, 2.5);
        let b = r.math("multiply", fv, 1.2);
        let s = r.math("add", a, b);
        let s = r.math("multiply", s, std::f32::consts::TAU);
        r.math("sine", s, 0.0)
    };
    let twist = r.mix_float(top, twist_f, twist_w);
    let height = plus(r, twist, 0.08, surface);
    // Thread by thread: each its own shade; slubs along each.
    let warp_id = r.node("combine_xyz").w("X", iu).f("Y", seed).f("Z", 0.0).out("Vector");
    let weft_id = r.node("combine_xyz").w("X", iv).f("Y", seed + 500.0).f("Z", 0.0).out("Vector");
    let warp_rand = r.node("white_noise_texture").w("Vector", warp_id).out("Value");
    let weft_rand = r.node("white_noise_texture").w("Vector", weft_id).out("Value");
    let own = r.mix_float(top, weft_rand, warp_rand);
    let own = r.range(own, (0.0, 1.0), (1.0 - c.variation, 1.0 + c.variation * 0.5));
    let warp_slub_at = r.node("combine_xyz").w("X", iu).w("Y", vv).f("Z", seed).out("Vector");
    let weft_slub_at = r.node("combine_xyz").w("X", uu).w("Y", iv).f("Z", seed + 7.0).out("Vector");
    let warp_slub_space = r.node("mapping").w("Vector", warp_slub_at).v("Scale", [3.0, 0.08, 1.0]).out("Vector");
    let weft_slub_space = r.node("mapping").w("Vector", weft_slub_at).v("Scale", [0.08, 3.0, 1.0]).out("Vector");
    let warp_slub = r.node("noise_texture").w("Vector", warp_slub_space).f("Scale", 1.0).f("Detail", 2.0).out("Fac");
    let weft_slub = r.node("noise_texture").w("Vector", weft_slub_space).f("Scale", 1.0).f("Detail", 2.0).out("Fac");
    let slub = r.mix_float(top, weft_slub, warp_slub);
    let slub = r.range(slub, (0.3, 0.7), (1.0 - c.slub * 0.25, 1.0 + c.slub * 0.12));
    // Gaps and the shade of the yarn's own roundness.
    let shade = r.range(surface, (0.0, 1.0), (0.45, 1.0));
    let tone = r.math("multiply", own, slub);
    let tone = r.math("multiply", tone, shade);
    let ply = r.range(twist, (-1.0, 1.0), (0.94, 1.04));
    let tone = r.math("multiply", tone, ply);
    let height = r.math("multiply", height, slub);
    Woven { height, warp_on_top: top, tone }
}
