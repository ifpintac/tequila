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
    /// A plain weave with a reinforcing yarn every `every` threads, both
    /// ways: two threads woven as one, standing proud of the ground in a
    /// grid of squares, which is what stops a tear.
    Ripstop { every: u32 },
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
        // What a loudspeaker, a bag and a shirt are covered in.
        Cloth { name: "Speaker Fabric Grey Melange", tags: &["speaker fabric", "acoustic cloth", "melange", "wool blend", "smart speaker", "grey"], pattern: Plain, threads: 1500.0, warp: srgb("#6E7074"), weft: srgb("#5E6064"), variation: 0.25, slub: 0.3, fill: 0.96, fuzz: 0.35, fuzz_colour: srgb("#C4C6C9"), heather: Some((srgb("#B8BABD"), 0.4)), depth: 1.2e-4, roughness: 0.85 },
        Cloth { name: "Speaker Fabric Charcoal Melange", tags: &["speaker fabric", "acoustic cloth", "melange", "wool blend", "smart speaker", "charcoal"], pattern: Plain, threads: 1500.0, warp: srgb("#2E2F32"), weft: srgb("#26272A"), variation: 0.25, slub: 0.3, fill: 0.96, fuzz: 0.35, fuzz_colour: srgb("#7A7C80"), heather: Some((srgb("#6A6C70"), 0.4)), depth: 1.2e-4, roughness: 0.85 },
        Cloth { name: "Speaker Fabric Sand Melange", tags: &["speaker fabric", "acoustic cloth", "melange", "wool blend", "smart speaker", "sand", "beige"], pattern: Plain, threads: 1500.0, warp: srgb("#B9AA8E"), weft: srgb("#A89A80"), variation: 0.25, slub: 0.3, fill: 0.96, fuzz: 0.35, fuzz_colour: srgb("#E8E0CE"), heather: Some((srgb("#E0D6C2"), 0.4)), depth: 1.2e-4, roughness: 0.85 },
        Cloth { name: "Speaker Fabric Navy Melange", tags: &["speaker fabric", "acoustic cloth", "melange", "wool blend", "smart speaker", "navy", "blue"], pattern: Plain, threads: 1500.0, warp: srgb("#1F2A44"), weft: srgb("#1A2238"), variation: 0.25, slub: 0.3, fill: 0.96, fuzz: 0.35, fuzz_colour: srgb("#6676A0"), heather: Some((srgb("#55668E"), 0.4)), depth: 1.2e-4, roughness: 0.85 },
        Cloth { name: "Speaker Fabric Black", tags: &["speaker fabric", "acoustic cloth", "grille cloth", "fine", "black"], pattern: Plain, threads: 1700.0, warp: srgb("#111112"), weft: srgb("#161617"), variation: 0.1, slub: 0.15, fill: 0.95, fuzz: 0.2, fuzz_colour: srgb("#3C3C3E"), heather: none, depth: 1.0e-4, roughness: 0.8 },
        Cloth { name: "Ballistic Nylon Black", tags: &["ballistic nylon", "1680d", "bag", "case", "luggage", "basket weave", "black"], pattern: Basket, threads: 1100.0, warp: srgb("#0E0E0F"), weft: srgb("#151516"), variation: 0.05, slub: 0.05, fill: 0.97, fuzz: 0.03, fuzz_colour: srgb("#3A3A3C"), heather: none, depth: 2.0e-4, roughness: 0.42 },
        Cloth { name: "Pack Nylon Black", tags: &["pack cloth", "nylon", "500d", "cordura", "bag", "case", "gig bag", "black"], pattern: Plain, threads: 900.0, warp: srgb("#131314"), weft: srgb("#19191A"), variation: 0.15, slub: 0.35, fill: 0.95, fuzz: 0.08, fuzz_colour: srgb("#404042"), heather: none, depth: 1.8e-4, roughness: 0.6 },
        Cloth { name: "Pack Nylon Olive", tags: &["pack cloth", "nylon", "500d", "cordura", "bag", "case", "olive", "green"], pattern: Plain, threads: 900.0, warp: srgb("#4A4F35"), weft: srgb("#3F442C"), variation: 0.15, slub: 0.35, fill: 0.95, fuzz: 0.08, fuzz_colour: srgb("#8A9068"), heather: none, depth: 1.8e-4, roughness: 0.6 },
        Cloth { name: "Pack Nylon Coyote", tags: &["pack cloth", "nylon", "500d", "cordura", "bag", "case", "coyote", "tan", "brown"], pattern: Plain, threads: 900.0, warp: srgb("#8A7250"), weft: srgb("#7A6444"), variation: 0.15, slub: 0.35, fill: 0.95, fuzz: 0.08, fuzz_colour: srgb("#C0A880"), heather: none, depth: 1.8e-4, roughness: 0.6 },
        Cloth { name: "Canvas Black", tags: &["canvas", "woven", "bag", "cotton", "duck", "black"], pattern: Plain, threads: 820.0, warp: srgb("#1A1A1B"), weft: srgb("#202021"), variation: 0.1, slub: 0.35, fill: 0.93, fuzz: 0.3, fuzz_colour: srgb("#4A4A4C"), heather: none, depth: 2.2e-4, roughness: 0.85 },
        Cloth { name: "Canvas Waxed Olive", tags: &["canvas", "waxed", "oilcloth", "bag", "jacket", "olive", "green"], pattern: Plain, threads: 820.0, warp: srgb("#4B4A2E"), weft: srgb("#414027"), variation: 0.1, slub: 0.3, fill: 0.96, fuzz: 0.04, fuzz_colour: srgb("#8A8860"), heather: none, depth: 1.6e-4, roughness: 0.5 },
        Cloth { name: "Cotton Twill Khaki", tags: &["cotton", "twill", "chino", "apparel", "khaki", "beige"], pattern: Twill { over: 2, under: 1 }, threads: 2000.0, warp: srgb("#B7A27A"), weft: srgb("#A8946E"), variation: 0.12, slub: 0.3, fill: 0.95, fuzz: 0.25, fuzz_colour: srgb("#D8CCB0"), heather: none, depth: 8e-5, roughness: 0.8 },
        Cloth { name: "Cotton Twill Navy", tags: &["cotton", "twill", "chino", "apparel", "navy", "blue"], pattern: Twill { over: 2, under: 1 }, threads: 2000.0, warp: srgb("#1C2740"), weft: srgb("#232F4C"), variation: 0.12, slub: 0.3, fill: 0.95, fuzz: 0.25, fuzz_colour: srgb("#5A6A90"), heather: none, depth: 8e-5, roughness: 0.8 },
        Cloth { name: "Oxford Cloth Blue", tags: &["oxford", "shirting", "cotton", "basket weave", "apparel", "blue"], pattern: Basket, threads: 2400.0, warp: srgb("#5B7FB8"), weft: srgb("#E8ECF2"), variation: 0.1, slub: 0.2, fill: 0.96, fuzz: 0.2, fuzz_colour: srgb("#D0DAEC"), heather: none, depth: 6e-5, roughness: 0.8 },
        Cloth { name: "Satin Champagne", tags: &["satin", "silk", "woven", "sheen", "gold"], pattern: Twill { over: 4, under: 1 }, threads: 3200.0, warp: srgb("#C9B07E"), weft: srgb("#B39A6A"), variation: 0.05, slub: 0.1, fill: 0.99, fuzz: 0.05, fuzz_colour: srgb("#E8D8B0"), heather: none, depth: 2e-5, roughness: 0.3 },
    ]
    .iter()
    .map(woven)
    .collect();

    // ---- piled, felted, looped ------------------------------------------------------
    // Felt: matted fibres, no weave at all - a fine fibrous tooth, a soft
    // mottle where the felting was uneven, a dry fuzz, and the paler fibres
    // that lie on it.
    out.extend([
        felt("Felt Grey", &["felt", "acoustic", "soft", "grey"], rgb(0.13, 0.133, 0.142), rgb(0.24, 0.243, 0.256)),
        felt("Felt Charcoal", &["felt", "acoustic", "soft", "charcoal", "dark grey"], rgb(0.035, 0.036, 0.039), rgb(0.075, 0.077, 0.082)),
        felt("Felt Black", &["felt", "lining", "pad", "soft", "black"], rgb(0.011, 0.011, 0.012), rgb(0.03, 0.03, 0.032)),
        felt("Felt Cream", &["felt", "wool", "soft", "cream", "natural"], srgb("#C9BFA8"), srgb("#E4DCC8")),
        felt("Felt Red", &["felt", "lining", "case", "piano", "red"], srgb("#7E1418"), srgb("#B42A2E")),
        felt("Felt Green Baize", &["felt", "baize", "lining", "case", "table", "green"], srgb("#14462A"), srgb("#22693E")),
        felt("Felt Navy", &["felt", "lining", "soft", "navy", "blue"], srgb("#141D36"), srgb("#263458")),
    ]);

    // Velvet: a dense pile that catches the light at grazing angles - dark
    // seen end on, bright towards the silhouette - lying this way and that
    // in patches (the crush), with the faint grain of its pile.
    for (name, tags, colour, sheen) in [
        ("Velvet Charcoal", &["velvet", "sheen", "upholstery", "charcoal"] as &'static [&'static str], rgb(0.0117, 0.0116, 0.0128), rgb(0.3061, 0.3053, 0.3248)),
        ("Velvet Deep Blue", &["velvet", "sheen", "upholstery", "blue"], rgb(0.009, 0.0105, 0.0431), rgb(0.2526, 0.3023, 0.6598)),
        ("Velvet Emerald", &["velvet", "sheen", "upholstery", "green"], srgb("#0B2A1C"), srgb("#4FA77E")),
        ("Velvet Blush", &["velvet", "sheen", "upholstery", "pink"], srgb("#B0727A"), srgb("#F2C8C8")),
        ("Velvet Crimson", &["velvet", "velour", "sheen", "red"], srgb("#5E0A16"), srgb("#E0566A")),
        ("Velvet Black", &["velvet", "sheen", "lining", "case", "black"], rgb(0.004, 0.004, 0.0045), rgb(0.16, 0.16, 0.17)),
        ("Velour Black", &["velour", "ear pad", "headphone", "cushion", "black"], rgb(0.008, 0.008, 0.009), rgb(0.11, 0.11, 0.115)),
        ("Velour Grey", &["velour", "ear pad", "headphone", "cushion", "grey"], srgb("#4A4C50"), srgb("#A8AAAE")),
    ] {
        let mut r = Recipe::new(base(MaterialType::Velvet, |m| {
            m.base_color = colour;
            m.fuzz_color = sheen;
            m.fuzz_roughness = 0.4;
        }));
        let crush = r.node("noise_texture").f("Scale", 35.0).f("Detail", 5.0).f("Distortion", 1.2).out("Fac");
        // Gently: a velvet new on a product is brushed one way, and its
        // crush is a shading across it, not the marbling of one slept on.
        let pile = r.range(crush, (0.3, 0.7), (0.37, 0.52));
        r.to(Param::FuzzRoughness, pile);
        // Crushed pile lies over and shows more of its colour, less sheen.
        let laid = r.range(crush, (0.3, 0.7), (0.92, 1.1));
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
    // The strokes of a hand: the nap laid one way and the other, in two
    // tones with a soft edge between, a few centimetres long.
    let coords = r.node("texture_coordinate").out("Object");
    let stroke_space = r.node("mapping").w("Vector", coords).v("Rotation", [0.3, 0.8, 0.2]).v("Scale", [4.0, 1.0, 1.5]).out("Vector");
    let nap = r.node("noise_texture").w("Vector", stroke_space).f("Scale", 20.0).f("Detail", 2.0).f("Distortion", 0.8).out("Fac");
    let lay = r.smooth(nap, (0.25, 0.75), (0.0, 1.0));
    let brushed = r.range(lay, (0.0, 1.0), (0.94, 1.05));
    // The nap itself: fibre tips a quarter of a millimetre across, and
    // drawn out along the way they were brushed.
    let fibres = r.node("noise_texture").f("Scale", 4200.0).f("Detail", 3.0).f("Roughness", 0.7).f("Distortion", 1.5).out("Fac");
    let along = r.node("mapping").w("Vector", coords).v("Rotation", [0.3, 0.8, 0.2]).v("Scale", [0.15, 1.0, 1.0]).out("Vector");
    let streaks = r.node("noise_texture").w("Vector", along).f("Scale", 2400.0).f("Detail", 2.0).out("Fac");
    let mottle_field = noise(&mut r, 120.0, 3.0, 0.5);
    let fibre_tone = r.range(fibres, (0.3, 0.7), (0.86, 1.1));
    let streak_tone = r.range(streaks, (0.3, 0.7), (0.92, 1.06));
    let mottle = r.range(mottle_field, (0.3, 0.7), (0.95, 1.04));
    let shade = r.math("multiply", brushed, fibre_tone);
    let shade = r.math("multiply", shade, streak_tone);
    let shade = r.math("multiply", shade, mottle);
    let colour = tint(&mut r, srgb("#7C5E45"), shade);
    r.to(Param::BaseColor, colour);
    let glow = r.range(lay, (0.0, 1.0), (0.86, 1.1));
    let sheen = tint(&mut r, srgb("#B79A7C"), glow);
    r.to(Param::FuzzColor, sheen);
    let lie = r.range(lay, (0.0, 1.0), (0.6, 0.72));
    r.to(Param::FuzzRoughness, lie);
    let height = plus(&mut r, streaks, 0.5, fibres);
    r.bump(height, 4e-5);
    out.push(preset("Suede Tan", "Fabric", &["suede", "nubuck", "leather", "soft", "brown"], MaterialType::Velvet, r));
    // Ripstop: nylon, light and a little shiny, in a 5 mm grid; and the
    // heavier cotton of a field jacket, matte, its grid closer.
    let nylon_tags = |colour: &'static [&'static str]| -> &'static [&'static str] {
        let mut tags = vec!["ripstop", "nylon", "grid", "technical", "outdoor", "tent", "bag", "jacket", "parachute"];
        tags.extend_from_slice(colour);
        Vec::leak(tags)
    };
    out.extend([
        ripstop("Ripstop Nylon Black", nylon_tags(&["black"]), srgb("#151517"), 4200.0, 0.005, 0.38, 0.03, 9.0),
        ripstop("Ripstop Nylon Grey", nylon_tags(&["grey"]), srgb("#6A6D72"), 4200.0, 0.005, 0.38, 0.03, 9.0),
        ripstop("Ripstop Nylon White", nylon_tags(&["white"]), srgb("#E6E5E0"), 4200.0, 0.005, 0.38, 0.03, 9.0),
        ripstop("Ripstop Nylon Orange", nylon_tags(&["orange"]), srgb("#E4601C"), 4200.0, 0.005, 0.38, 0.03, 9.0),
        ripstop("Ripstop Nylon Red", nylon_tags(&["red"]), srgb("#B31D24"), 4200.0, 0.005, 0.38, 0.03, 9.0),
        ripstop("Ripstop Nylon Yellow", nylon_tags(&["yellow"]), srgb("#E8B414"), 4200.0, 0.005, 0.38, 0.03, 9.0),
        ripstop("Ripstop Nylon Olive", nylon_tags(&["olive", "green"]), srgb("#4D5234"), 4200.0, 0.005, 0.38, 0.03, 9.0),
        ripstop("Ripstop Nylon Navy", nylon_tags(&["navy", "blue"]), srgb("#1B2742"), 4200.0, 0.005, 0.38, 0.03, 9.0),
        ripstop("Ripstop Nylon Blue", nylon_tags(&["blue", "royal"]), srgb("#1F55B0"), 4200.0, 0.005, 0.38, 0.03, 9.0),
        ripstop("Ripstop Nylon Teal", nylon_tags(&["teal", "green"]), srgb("#12606A"), 4200.0, 0.005, 0.38, 0.03, 9.0),
        ripstop("Ripstop Cotton Olive", &["ripstop", "cotton", "grid", "field jacket", "military", "workwear", "olive", "green"], srgb("#5A5C3C"), 2600.0, 0.004, 0.82, 0.25, 5.0),
        ripstop("Ripstop Cotton Khaki", &["ripstop", "cotton", "grid", "field jacket", "military", "workwear", "khaki", "tan"], srgb("#A8946A"), 2600.0, 0.004, 0.82, 0.25, 5.0),
        ripstop("Ripstop Cotton Black", &["ripstop", "cotton", "grid", "field jacket", "workwear", "black"], srgb("#1B1B1C"), 2600.0, 0.004, 0.82, 0.25, 5.0),
    ]);
    let sheer_tags = |colour: &'static [&'static str]| -> &'static [&'static str] {
        let mut tags = vec!["ripstop", "nylon", "translucent", "sheer", "silnylon", "grid", "tent", "kite", "ultralight", "wind jacket", "parachute"];
        tags.extend_from_slice(colour);
        Vec::leak(tags)
    };
    let xply_tags = |colour: &'static [&'static str]| -> &'static [&'static str] {
        let mut tags = vec!["laminate", "sailcloth", "x-ply", "x-pac", "vx21", "diamond", "waterproof", "bag", "pack", "technical"];
        tags.extend_from_slice(colour);
        Vec::leak(tags)
    };
    let film_tags = |colour: &'static [&'static str]| -> &'static [&'static str] {
        let mut tags = vec!["composite", "film", "laminate", "dyneema", "dcf", "cuben fiber", "uhmwpe", "ultralight", "translucent", "tent", "pack", "technical"];
        tags.extend_from_slice(colour);
        Vec::leak(tags)
    };
    let spun_tags = |colour: &'static [&'static str]| -> &'static [&'static str] {
        let mut tags = vec!["spunbond", "nonwoven", "tyvek", "paper fabric", "hdpe", "envelope", "house wrap", "coverall", "tag", "packaging"];
        tags.extend_from_slice(colour);
        Vec::leak(tags)
    };
    out.extend([
        ripstop_sheer("Ripstop Translucent White", sheer_tags(&["white"]), srgb("#E9E8E3")),
        ripstop_sheer("Ripstop Translucent Yellow", sheer_tags(&["yellow"]), srgb("#E8B414")),
        ripstop_sheer("Ripstop Translucent Orange", sheer_tags(&["orange"]), srgb("#E4601C")),
        ripstop_sheer("Ripstop Translucent Red", sheer_tags(&["red"]), srgb("#B31D24")),
        ripstop_sheer("Ripstop Translucent Blue", sheer_tags(&["blue"]), srgb("#1F55B0")),
        ripstop_sheer("Ripstop Translucent Green", sheer_tags(&["green"]), srgb("#2E7A46")),
        ripstop_sheer("Ripstop Translucent Smoke", sheer_tags(&["smoke", "grey", "black"]), srgb("#3A3C40")),
        xply("Laminate X-Ply Black", xply_tags(&["black"]), srgb("#161617")),
        xply("Laminate X-Ply Grey", xply_tags(&["grey"]), srgb("#7A7D82")),
        xply("Laminate X-Ply White", xply_tags(&["white"]), srgb("#E4E3DE")),
        xply("Laminate X-Ply Olive", xply_tags(&["olive", "green"]), srgb("#5A5F40")),
        xply("Laminate X-Ply Coyote", xply_tags(&["coyote", "tan", "brown"]), srgb("#927A56")),
        xply("Laminate X-Ply Blue", xply_tags(&["blue"]), srgb("#2A5298")),
        xply("Laminate X-Ply Orange", xply_tags(&["orange"]), srgb("#DC5E1E")),
        composite_film("Composite Film Fabric Natural", film_tags(&["natural", "white", "clear"]), rgb(0.74, 0.77, 0.73), rgb(0.93, 0.94, 0.92), 0.6),
        composite_film("Composite Film Fabric Grey", film_tags(&["grey"]), srgb("#7C8084"), rgb(0.8, 0.81, 0.82), 0.4),
        composite_film("Composite Film Fabric Black", film_tags(&["black"]), rgb(0.014, 0.014, 0.015), rgb(0.2, 0.2, 0.21), 0.08),
        composite_film("Composite Film Fabric Olive", film_tags(&["olive", "green"]), srgb("#59603F"), rgb(0.7, 0.72, 0.6), 0.35),
        composite_film("Composite Film Fabric Blue", film_tags(&["blue"]), srgb("#2B5C9E"), rgb(0.7, 0.8, 0.92), 0.35),
        spunbond("Spunbond Sheet White", spun_tags(&["white"]), srgb("#EEEEEA"), 0.15),
        spunbond("Spunbond Sheet Grey", spun_tags(&["grey"]), srgb("#8E9092"), 0.08),
        spunbond("Spunbond Sheet Black", spun_tags(&["black"]), srgb("#1A1A1B"), 0.0),
        spunbond("Spunbond Sheet Kraft", spun_tags(&["kraft", "tan", "brown"]), srgb("#B9976A"), 0.08),
    ]);
    out.extend([
        spacer_mesh("Spacer Mesh Black", &["spacer mesh", "air mesh", "3d mesh", "knit", "headband", "headphone", "padding", "black"], rgb(0.012, 0.012, 0.013)),
        spacer_mesh("Spacer Mesh Grey", &["spacer mesh", "air mesh", "3d mesh", "knit", "headband", "headphone", "padding", "grey"], srgb("#5A5C60")),
        crate::footwear::jersey("Neoprene Black", &["neoprene", "jersey", "laminated", "sleeve", "case", "strap", "black"], srgb("#161617")),
    ]);
    out
}

/// `colour` times `factor`, a node's output.
/// Ripstop: a fine plain weave with a grid of reinforcing yarns, `grid`
/// metres from one to the next. `threads` to the metre, `sheen` how
/// polished its filament is (a nylon's is, a cotton's is not), `fuzz` its
/// loose fibre, and `crinkle` how much so thin a cloth has creased: the
/// soft folds a few centimetres apart that a ripstop is never without.
#[allow(clippy::too_many_arguments)]
fn ripstop(name: &'static str, tags: &'static [&'static str], colour: Spectrum, threads: f32, grid: f32, sheen: f32, fuzz: f32, crinkle: f32) -> Preset {
    let every = (grid * threads).round().max(4.0) as u32;
    let cloth = Cloth {
        name,
        tags,
        pattern: Pattern::Ripstop { every },
        threads,
        warp: colour,
        weft: colour * 0.93,
        variation: 0.05,
        slub: 0.06,
        // Packed past touching: a ripstop's ground is so fine and close
        // that it is nearly a smooth film, and its threads are not seen
        // one by one but as a tooth.
        fill: 1.25,
        fuzz,
        fuzz_colour: colour.lerp(Spectrum::ONE, 0.45),
        heather: None,
        depth: 5.5e-5,
        roughness: sheen,
    };
    woven_with(&cloth, |r, height| {
        // In the weave's own units, where a thread's height is 1.
        // Creases, not hills: where a wandering noise crosses its middle
        // the cloth is folded sharp, and between two folds it lies soft.
        let field = r.node("noise_texture").f("Scale", 30.0).f("Detail", 3.0).f("Roughness", 0.5).f("Distortion", 1.6).out("Fac");
        let off = r.math("subtract", field, 0.5);
        let from_fold = r.math("absolute", off, 0.0);
        let folds = r.smooth(from_fold, (0.0, 0.16), (1.0, 0.0));
        plus(r, folds, crinkle, height)
    })
}

/// A ripstop so thin that light comes through it: a tent's fly, a
/// kite, a wind jacket held to the sun. The same cloth, a thin sheet that
/// passes half of what falls on it, softened.
fn ripstop_sheer(name: &'static str, tags: &'static [&'static str], colour: Spectrum) -> Preset {
    let mut p = ripstop(name, tags, colour, 4200.0, 0.005, 0.42, 0.02, 9.0);
    p.ty = MaterialType::Generic;
    let m = p.recipe.material_mut();
    m.geometry_thin_walled = true;
    m.transmission_weight = 0.5;
    m.transmission_color = colour.lerp(Spectrum::ONE, 0.35);
    p
}

/// A sailcloth laminate: a fine woven face, and under it, laid in the
/// glue, reinforcing yarns leaning 22 degrees either way, a centimetre
/// apart - an X of dark lines seen through the face and felt as ridges
/// in it. Stiffer than a cloth: it buckles broadly and does not crease.
fn xply(name: &'static str, tags: &'static [&'static str], colour: Spectrum) -> Preset {
    use crate::finish::triplanar;
    let cloth = Cloth {
        name,
        tags,
        pattern: Pattern::Plain,
        threads: 3000.0,
        warp: colour,
        weft: colour * 0.94,
        variation: 0.06,
        slub: 0.1,
        fill: 1.2,
        fuzz: 0.0,
        fuzz_colour: colour,
        heather: None,
        depth: 4e-5,
        roughness: 0.42,
    };
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = colour;
        m.base_diffuse_roughness = 0.5;
        m.specular_ior = 1.5;
        m.specular_roughness = 0.42;
    }));
    let lean = 22f32.to_radians();
    let spacing = 0.011 / lean.cos();
    let blended = triplanar(&mut r, 6.0, |r, u, v, i| {
        let w = weave(r, u, v, &cloth, i as f32 * 31.7);
        let mut yarn: Option<Out> = None;
        for sign in [1.0, -1.0] {
            let across = r.math3("multiply_add", v, lean.tan() * sign, u);
            let t = r.math("divide", across, spacing);
            let f = r.math("fraction", t, 0.0);
            let centred = r.math("subtract", f, 0.5);
            let away = r.math("absolute", centred, 0.0);
            let line = r.smooth(away, (0.018, 0.04), (1.0, 0.0));
            yarn = Some(match yarn {
                None => line,
                Some(y) => r.math("maximum", y, line),
            });
        }
        vec![w.height, w.tone, yarn.expect("two sets of yarns")]
    });
    let (height, tone, yarn) = (blended[0], blended[1], blended[2]);
    // The yarn is black polyester, and shows through whatever is over it.
    let through = r.range(yarn, (0.0, 1.0), (1.0, 0.4));
    let shade = r.math("multiply", tone, through);
    let toned = tint(&mut r, colour, shade);
    r.to(Param::BaseColor, toned);
    let buckle = noise(&mut r, 22.0, 2.0, 0.5);
    let raised = plus(&mut r, yarn, 3.0, height);
    let raised = plus(&mut r, buckle, 6.0, raised);
    r.bump(raised, 4e-5);
    // The face is pressed hard over each yarn and polished a little there.
    let sheen = r.range(yarn, (0.0, 1.0), (0.44, 0.34));
    r.to(Param::SpecularRoughness, sheen);
    preset(name, "Fabric", tags, MaterialType::Generic, r)
}

/// A composite film fabric: no weave at all, but filaments laid dead
/// straight both ways between two films, a millimetre and a half apart
/// and never quite evenly. It is a film, so it passes light, shines a
/// little, and creases sharp as paper does. `film` is its colour, `fibre`
/// the filaments', `through` how much light it passes.
fn composite_film(name: &'static str, tags: &'static [&'static str], film: Spectrum, fibre: Spectrum, through: f32) -> Preset {
    use crate::finish::triplanar;
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = film;
        m.specular_ior = 1.6;
        m.specular_roughness = 0.34;
        m.geometry_thin_walled = true;
        m.transmission_weight = through;
        m.transmission_color = film.lerp(Spectrum::ONE, 0.5);
    }));
    let fibres = triplanar(&mut r, 6.0, |r, u, v, i| {
        let mut laid: Option<Out> = None;
        for (k, x) in [u, v].into_iter().enumerate() {
            let t = r.math("multiply", x, 700.0);
            let index = r.math("floor", t, 0.0);
            let id = r.node("combine_xyz").w("X", index).f("Y", (i * 17 + k * 5) as f32).f("Z", 0.0).out("Vector");
            let own = r.node("white_noise_texture").w("Vector", id).out("Value");
            let id2 = r.node("combine_xyz").w("X", index).f("Y", (i * 17 + k * 5) as f32).f("Z", 9.0).out("Vector");
            let bright = r.node("white_noise_texture").w("Vector", id2).out("Value");
            // Each a little off where it should lie, and its own weight.
            let f = r.math("fraction", t, 0.0);
            let shifted = r.math3("multiply_add", own, 0.5, f);
            let centred = r.math("subtract", shifted, 0.75);
            let away = r.math("absolute", centred, 0.0);
            let line = r.smooth(away, (0.05, 0.13), (1.0, 0.0));
            let weight = r.range(bright, (0.0, 1.0), (0.3, 1.0));
            let line = r.math("multiply", line, weight);
            laid = Some(match laid {
                None => line,
                Some(l) => r.math("maximum", l, line),
            });
        }
        vec![laid.expect("two ways")]
    })[0];
    let some = r.math("multiply", fibres, 0.8);
    let colour = r.node("mix").f("Factor", some).c("A", film).c("B", fibre).out("Result");
    r.to(Param::BaseColor, colour);
    // Creases at two sizes: where a wandering noise crosses its middle.
    let crease = |r: &mut Recipe, scale: f32| {
        let field = r.node("noise_texture").f("Scale", scale).f("Detail", 2.0).f("Roughness", 0.5).f("Distortion", 1.2).out("Fac");
        let off = r.math("subtract", field, 0.5);
        let from = r.math("absolute", off, 0.0);
        r.smooth(from, (0.0, 0.05), (1.0, 0.0))
    };
    let big = crease(&mut r, 18.0);
    let small = crease(&mut r, 60.0);
    let folds = plus(&mut r, small, 0.5, big);
    let height = plus(&mut r, fibres, 0.12, folds);
    r.bump(height, 2.4e-4);
    // Scuffed dull along a fold; and a filament stops some of the light.
    let sheen = r.range(folds, (0.0, 1.0), (0.32, 0.5));
    r.to(Param::SpecularRoughness, sheen);
    let passes = r.range(fibres, (0.0, 1.0), (through, through * 0.5));
    r.to(Param::TransmissionWeight, passes);
    preset(name, "Fabric", tags, MaterialType::Generic, r)
}

/// A spunbond sheet: fine continuous fibres spun out in swirls and
/// bonded flat under heat, neither paper nor cloth. Its fibres show as
/// paler lines that wander and cross; it is dull with a slight shine on
/// them, creases softly, and lets a little light through.
fn spunbond(name: &'static str, tags: &'static [&'static str], colour: Spectrum, through: f32) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = colour;
        m.base_diffuse_roughness = 0.6;
        m.specular_ior = 1.5;
        m.specular_roughness = 0.5;
        m.specular_weight = 0.7;
        m.geometry_thin_walled = through > 0.0;
        m.transmission_weight = through;
        m.transmission_color = colour;
    }));
    let swirl = |r: &mut Recipe, scale: f32, width: f32| {
        let field = r.node("noise_texture").f("Scale", scale).f("Detail", 2.0).f("Roughness", 0.5).f("Distortion", 4.0).out("Fac");
        let off = r.math("subtract", field, 0.5);
        let from = r.math("absolute", off, 0.0);
        r.smooth(from, (0.0, width), (1.0, 0.0))
    };
    let coarse = swirl(&mut r, 180.0, 0.035);
    let fine = swirl(&mut r, 430.0, 0.03);
    let fine = r.math("multiply", fine, 0.7);
    let fibres = r.math("maximum", coarse, fine);
    let cloud = noise(&mut r, 90.0, 3.0, 0.5);
    let mottle = r.range(cloud, (0.3, 0.7), (0.94, 1.03));
    let ground = tint(&mut r, colour, mottle);
    let some = r.math("multiply", fibres, 0.45);
    let toned = r.node("mix").f("Factor", some).w("A", ground).c("B", colour.lerp(Spectrum::ONE, 0.55)).out("Result");
    r.to(Param::BaseColor, toned);
    let field = r.node("noise_texture").f("Scale", 20.0).f("Detail", 3.0).f("Roughness", 0.5).f("Distortion", 1.4).out("Fac");
    let off = r.math("subtract", field, 0.5);
    let from = r.math("absolute", off, 0.0);
    let folds = r.smooth(from, (0.0, 0.1), (1.0, 0.0));
    let tooth = noise(&mut r, 2600.0, 2.0, 0.5);
    let height = plus(&mut r, fibres, 0.25, folds);
    let height = plus(&mut r, tooth, 0.08, height);
    r.bump(height, 1.3e-4);
    let sheen = r.range(fibres, (0.0, 1.0), (0.56, 0.4));
    r.to(Param::SpecularRoughness, sheen);
    preset(name, "Fabric", tags, MaterialType::Generic, r)
}

/// A felt, from the darkest of its mat to the lightest.
fn felt(name: &'static str, tags: &'static [&'static str], dark: Spectrum, light: Spectrum) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Velvet, |m| {
        m.base_color = dark.lerp(light, 0.5);
        m.fuzz_color = light * 1.7;
        m.fuzz_roughness = 0.75;
    }));
    let fibres = r.node("noise_texture").f("Scale", 2400.0).f("Detail", 8.0).f("Roughness", 0.7).f("Distortion", 1.6).out("Fac");
    let tufts = noise(&mut r, 400.0, 4.0, 0.6);
    let height = plus(&mut r, tufts, 0.5, fibres);
    r.bump(height, 1e-4);
    let mottle = noise(&mut r, 60.0, 4.0, 0.5);
    let tone = plus(&mut r, fibres, 0.5, mottle);
    let colour = r.ramp(tone, Linear, &[(0.4, dark), (1.2, light)]);
    // And the fibres that lie on it, paler than the mat they are part of:
    // what tells a felt from a card of its colour.
    let hairs = loose_fibres(&mut r, 420.0, 0.03);
    let some = r.math("multiply", hairs, 0.5);
    let colour = r.node("mix").f("Factor", some).w("A", colour).c("B", light * 1.75).out("Result");
    r.to(Param::BaseColor, colour);
    preset(name, "Fabric", tags, MaterialType::Velvet, r)
}

/// Spacer mesh: a knitted cushion with a honeycomb of holes through its
/// face, as a headband's underside and a shoe's tongue have. The holes
/// are dark and deep, the knit between them soft.
fn spacer_mesh(name: &'static str, tags: &'static [&'static str], colour: Spectrum) -> Preset {
    use crate::finish::{staggered, triplanar, Hole};
    let mut r = Recipe::new(base(MaterialType::Velvet, |m| {
        m.base_color = colour;
        m.fuzz_color = colour.lerp(Spectrum::ONE, 0.25);
        m.fuzz_roughness = 0.7;
        m.fuzz_weight = 0.5;
    }));
    let (pitch, radius) = (0.0036, 0.0011);
    let solid = triplanar(&mut r, 8.0, |r, u, v, _| {
        let d = staggered(r, u, v, pitch, Hole::Hex);
        vec![r.smooth(d, (radius * 0.75, radius * 1.25), (0.0, 1.0))]
    })[0];
    let knit = r.node("noise_texture").f("Scale", 3000.0).f("Detail", 3.0).f("Roughness", 0.6).out("Fac");
    let height = plus(&mut r, knit, 0.06, solid);
    r.bump(height, 9e-4);
    let knit_tone = r.range(knit, (0.3, 0.7), (0.85, 1.1));
    let shade = r.math("multiply", solid, knit_tone);
    let shade = r.range(shade, (0.0, 1.0), (0.12, 1.0));
    let toned = tint(&mut r, colour, shade);
    r.to(Param::BaseColor, toned);
    preset(name, "Fabric", tags, MaterialType::Velvet, r)
}

/// Loose fibres lying on a felted or napped surface: thin, wavy hairs,
/// 1 on one and 0 off it. `scale` is how many of their bends a metre and
/// `width` how much of the surface they are.
pub fn loose_fibres(r: &mut Recipe, scale: f32, width: f32) -> Out {
    // Where a tangled noise crosses its middle is a line that wanders;
    // kept only in patches, the lines are hairs and not a map's contours.
    let field = r.node("noise_texture").f("Scale", scale).f("Detail", 1.0).f("Roughness", 0.5).f("Distortion", 3.0).out("Fac");
    let off = r.math("subtract", field, 0.5);
    let apart = r.math("absolute", off, 0.0);
    let line = r.smooth(apart, (0.0, width), (1.0, 0.0));
    let patches = r.node("noise_texture").f("Scale", scale * 1.7).f("Detail", 2.0).out("Fac");
    let kept = r.smooth(patches, (0.5, 0.6), (0.0, 1.0));
    r.math("multiply", line, kept)
}

fn tint(r: &mut Recipe, colour: Spectrum, factor: Out) -> Out {
    r.node("mix").set("blend_type", "multiply").f("Factor", 1.0).c("A", colour).w("B", factor).out("Result")
}

/// One woven cloth, made.
pub fn woven(c: &Cloth) -> Preset {
    woven_with(c, |_, height| height)
}

/// One woven cloth, with `relief` given the weave's height to add to
/// before it is bumped: a padded tongue's puckers, say. In the weave's own
/// units, a thread's height its 1.
pub fn woven_with(c: &Cloth, relief: impl FnOnce(&mut Recipe, Out) -> Out) -> Preset {
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
    let height = relief(&mut r, height);
    r.bump(height, c.depth);
    preset(c.name, "Fabric", c.tags, MaterialType::RealCloth, r)
}

pub struct Woven {
    pub height: Out,
    pub warp_on_top: Out,
    pub tone: Out,
}

/// The weave on one plane, (u, v) in metres: `c.threads` to the unit. The
/// warp threads are the columns, constant u, running along v.
pub fn weave(r: &mut Recipe, u: Out, v: Out, c: &Cloth, seed: f32) -> Woven {
    let uu = r.math("multiply", u, c.threads);
    let vv = r.math("multiply", v, c.threads);
    let iu = r.math("floor", uu, 0.0);
    let iv = r.math("floor", vv, 0.0);
    let fu = r.math("fraction", uu, 0.0);
    let fv = r.math("fraction", vv, 0.0);
    // Which is on top at this crossing: 1 for the warp.
    let top = match c.pattern {
        Pattern::Plain | Pattern::Ripstop { .. } => {
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
    let (height, tone) = match c.pattern {
        Pattern::Ripstop { every } => {
            // The reinforcing yarn: three threads wide, round across, and
            // nothing between one and the next.
            let rib_of = |r: &mut Recipe, x: Out| {
                let within = r.math("floored_modulo", x, every as f32);
                let across = r.math("subtract", within, 1.5);
                let across = r.math("divide", across, 1.5);
                let sq = r.math("multiply", across, across);
                let rest = r.math("subtract", 1.0, sq);
                let rest = r.math("maximum", rest, 0.0);
                r.math("sqrt", rest, 0.0)
            };
            let warp_rib = rib_of(r, uu);
            let weft_rib = rib_of(r, vv);
            let rib = r.math("maximum", warp_rib, weft_rib);
            // The ground between the yarns is held at its four sides and
            // free in the middle, so each square pillows a little.
            let pillow_of = |r: &mut Recipe, x: Out| {
                let cell = r.math("divide", x, every as f32);
                let f = r.math("fraction", cell, 0.0);
                let a = r.math("multiply", f, std::f32::consts::PI);
                r.math("sine", a, 0.0)
            };
            let pu = pillow_of(r, uu);
            let pv = pillow_of(r, vv);
            let pillow = r.math("multiply", pu, pv);
            let raised = plus(r, rib, 1.3, height);
            let raised = plus(r, pillow, 2.6, raised);
            // A doubled yarn is denser and catches more light than the
            // ground it runs through.
            let brighter = r.range(rib, (0.0, 1.0), (1.0, 1.15));
            (raised, r.math("multiply", tone, brighter))
        }
        _ => (height, tone),
    };
    Woven { height, warp_on_top: top, tone }
}
