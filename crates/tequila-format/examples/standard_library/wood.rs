//! Wood, species by species.
//!
//! One generator, [`wood`], and a description of each species, [`Species`],
//! written against scans of the real thing (Poly Haven's species veneers)
//! and what a woodworker knows of each: how wide its growth rings are and
//! how sharply each year's light earlywood turns into dark latewood; whether
//! its pores are large and crowded into the earlywood (oak, ash: ring
//! porous), small and everywhere (maple, cherry, walnut: diffuse porous) or
//! absent (pine, spruce, fir: softwoods); whether its rays show (oak's
//! silver flecks, beech's freckles); its figure - the ribbon of sapele and
//! mahogany, maple's curl, the dark streaks of rosewood and teak, olive's
//! swirls, pine's knots - and how it is finished.
//!
//! The geometry is a log's. The grain runs along the part's X; the rings
//! are cylinders about a pith half a metre away, so across a part of a few
//! centimetres they lie almost flat, as in a board sawn from the log: long
//! grain along the sides, the rings' arcs - the "cathedrals" - where the
//! surface cuts across them. The pith wanders, the years differ in width,
//! and the whole is read in object space, so a part shows the grain a
//! piece of that wood of that shape would.

use crate::recipe::{Linear, Out};
use crate::{base, preset, srgb, Preset, Recipe};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::{MaterialType, Param};

/// How a species' pores lie.
#[derive(Clone, Copy)]
pub enum Pores {
    /// Softwood: none to see.
    None,
    /// Small, scattered through the whole ring: `size` (0 to 1) and how
    /// dark they read.
    Diffuse { size: f32, dark: f32 },
    /// Large and crowded into the earlywood, a few small in the latewood.
    RingPorous { size: f32, dark: f32 },
}

/// A species' figure: what it does beyond rings and pores.
#[derive(Clone, Copy)]
pub enum Figure {
    Plain,
    /// Interlocked grain: long stripes along the grain that change from
    /// light to dark as the light moves (sapele, mahogany).
    Ribbon { strength: f32 },
    /// Waves across the grain that catch the light (curly maple).
    Curl { strength: f32 },
    /// Dark mineral streaks along the grain (rosewood, teak, ebony, walnut).
    Streaks { colour: Spectrum, amount: f32 },
    /// Wild grain: the rings swirling, dark lines through pale wood (olive).
    Swirl { colour: Spectrum },
    /// Knots: branches the board was sawn through (pine).
    Knots { colour: Spectrum, density: f32 },
}

/// How it is finished.
#[derive(Clone, Copy)]
pub enum Finish {
    /// Raw, sanded: dry and open.
    Raw,
    /// Oiled: a soft sheen, the pores still open.
    Oiled,
    /// A satin lacquer.
    Satin,
    /// A gloss lacquer, deep and clear.
    Gloss,
}

pub struct Species {
    pub name: &'static str,
    pub tags: &'static [&'static str],
    /// Earlywood (light) and latewood (dark), as finished.
    pub early: &'static str,
    pub late: &'static str,
    /// Growth rings to the metre.
    pub rings: f32,
    /// How much of a year is latewood, 0 to 1.
    pub latewood: f32,
    pub pores: Pores,
    /// Ray flecks, 0 to 1.
    pub rays: f32,
    pub figure: Figure,
    pub finish: Finish,
    /// How far the colour drifts across a board, 0 to 1: sapwood and
    /// heartwood, a streak of mineral, the tree's own unevenness.
    pub drift: f32,
}

/// The species, as finished.
pub fn all() -> Vec<Preset> {
    use Figure::*;
    use Finish::*;
    use Pores::*;
    let species = [
        Species { name: "Wood White Oak", tags: &["wood", "oak", "white oak", "furniture", "light"], early: "#C69D6D", late: "#B48A5B", rings: 340.0, latewood: 0.45, pores: RingPorous { size: 0.11, dark: 0.55 }, rays: 0.75, figure: Plain, finish: Oiled, drift: 0.25 },
        Species { name: "Wood Red Oak", tags: &["wood", "oak", "red oak", "furniture", "pink"], early: "#CFA07E", late: "#BE8C69", rings: 300.0, latewood: 0.45, pores: RingPorous { size: 0.13, dark: 0.6 }, rays: 0.25, figure: Plain, finish: Satin, drift: 0.25 },
        Species { name: "Wood Smoked Oak", tags: &["wood", "oak", "smoked", "fumed", "dark"], early: "#7A5A3E", late: "#664833", rings: 340.0, latewood: 0.45, pores: RingPorous { size: 0.11, dark: 0.55 }, rays: 0.6, figure: Plain, finish: Oiled, drift: 0.2 },
        Species { name: "Wood Walnut", tags: &["wood", "walnut", "black walnut", "furniture", "dark"], early: "#6E4C36", late: "#553826", rings: 300.0, latewood: 0.20, pores: Diffuse { size: 0.09, dark: 0.55 }, rays: 0.0, figure: Streaks { colour: srgb("#2A1B13"), amount: 0.35 }, finish: Oiled, drift: 0.45 },
        Species { name: "Wood European Walnut", tags: &["wood", "walnut", "european", "furniture", "figured"], early: "#8E6B4C", late: "#6E4F36", rings: 300.0, latewood: 0.20, pores: Diffuse { size: 0.08, dark: 0.5 }, rays: 0.0, figure: Streaks { colour: srgb("#32221A"), amount: 0.6 }, finish: Oiled, drift: 0.5 },
        Species { name: "Wood Maple", tags: &["wood", "maple", "hard maple", "furniture", "pale"], early: "#EBD6AF", late: "#DCC198", rings: 520.0, latewood: 0.16, pores: Diffuse { size: 0.04, dark: 0.3 }, rays: 0.15, figure: Plain, finish: Satin, drift: 0.15 },
        Species { name: "Wood Curly Maple", tags: &["wood", "maple", "curly", "flame", "figured", "instrument"], early: "#E8CB9A", late: "#E2C493", rings: 520.0, latewood: 0.16, pores: Diffuse { size: 0.04, dark: 0.3 }, rays: 0.1, figure: Curl { strength: 1.0 }, finish: Gloss, drift: 0.2 },
        Species { name: "Wood Cherry", tags: &["wood", "cherry", "furniture", "red"], early: "#B96F47", late: "#A05A36", rings: 460.0, latewood: 0.18, pores: Diffuse { size: 0.05, dark: 0.4 }, rays: 0.05, figure: Plain, finish: Satin, drift: 0.35 },
        Species { name: "Wood Ash", tags: &["wood", "ash", "furniture", "pale", "open grain"], early: "#DEC59D", late: "#C9A87C", rings: 300.0, latewood: 0.45, pores: RingPorous { size: 0.12, dark: 0.55 }, rays: 0.0, figure: Plain, finish: Satin, drift: 0.2 },
        Species { name: "Wood Beech", tags: &["wood", "beech", "steamed", "furniture", "pale"], early: "#DCB089", late: "#CC9C74", rings: 460.0, latewood: 0.18, pores: Diffuse { size: 0.04, dark: 0.3 }, rays: 0.9, figure: Plain, finish: Satin, drift: 0.15 },
        Species { name: "Wood Birch", tags: &["wood", "birch", "plywood", "pale"], early: "#E8D0A8", late: "#D9BC90", rings: 480.0, latewood: 0.16, pores: Diffuse { size: 0.035, dark: 0.3 }, rays: 0.0, figure: Plain, finish: Satin, drift: 0.2 },
        Species { name: "Wood Pine", tags: &["wood", "pine", "softwood", "knots", "yellow"], early: "#E3BF88", late: "#B7743A", rings: 200.0, latewood: 0.40, pores: None, rays: 0.0, figure: Knots { colour: srgb("#6B3A17"), density: 9.0 }, finish: Satin, drift: 0.3 },
        Species { name: "Wood Douglas Fir", tags: &["wood", "fir", "douglas", "softwood", "orange"], early: "#DEA875", late: "#9F5A2C", rings: 260.0, latewood: 0.45, pores: None, rays: 0.0, figure: Plain, finish: Satin, drift: 0.25 },
        Species { name: "Wood Spruce", tags: &["wood", "spruce", "softwood", "pale", "tonewood"], early: "#EFDDB9", late: "#DCC195", rings: 380.0, latewood: 0.32, pores: None, rays: 0.0, figure: Plain, finish: Raw, drift: 0.12 },
        Species { name: "Wood Cedar", tags: &["wood", "cedar", "western red cedar", "softwood", "red"], early: "#BE7A50", late: "#8E4F30", rings: 320.0, latewood: 0.38, pores: None, rays: 0.0, figure: Streaks { colour: srgb("#6A3820"), amount: 0.5 }, finish: Raw, drift: 0.6 },
        Species { name: "Wood Teak", tags: &["wood", "teak", "outdoor", "boat", "golden"], early: "#A86E3E", late: "#8E5A30", rings: 300.0, latewood: 0.20, pores: Diffuse { size: 0.1, dark: 0.5 }, rays: 0.0, figure: Streaks { colour: srgb("#4A2C16"), amount: 0.45 }, finish: Oiled, drift: 0.35 },
        Species { name: "Wood Mahogany", tags: &["wood", "mahogany", "furniture", "red", "ribbon"], early: "#8E4B2F", late: "#7A3D24", rings: 360.0, latewood: 0.18, pores: Diffuse { size: 0.1, dark: 0.5 }, rays: 0.0, figure: Ribbon { strength: 0.6 }, finish: Satin, drift: 0.2 },
        Species { name: "Wood Sapele", tags: &["wood", "sapele", "ribbon", "red", "veneer"], early: "#A9593A", late: "#91472C", rings: 380.0, latewood: 0.18, pores: Diffuse { size: 0.08, dark: 0.45 }, rays: 0.0, figure: Ribbon { strength: 1.0 }, finish: Gloss, drift: 0.2 },
        Species { name: "Wood Rosewood", tags: &["wood", "rosewood", "exotic", "dark", "instrument"], early: "#6E3719", late: "#4A220E", rings: 520.0, latewood: 0.22, pores: Diffuse { size: 0.08, dark: 0.5 }, rays: 0.0, figure: Streaks { colour: srgb("#140805"), amount: 0.7 }, finish: Gloss, drift: 0.45 },
        Species { name: "Wood Ebony", tags: &["wood", "ebony", "exotic", "black", "instrument"], early: "#2C2420", late: "#1E1916", rings: 600.0, latewood: 0.25, pores: Diffuse { size: 0.05, dark: 0.3 }, rays: 0.0, figure: Streaks { colour: srgb("#3B2C22"), amount: 0.35 }, finish: Gloss, drift: 0.3 },
        Species { name: "Wood Wenge", tags: &["wood", "wenge", "exotic", "dark", "striped"], early: "#5C4333", late: "#211611", rings: 300.0, latewood: 0.50, pores: Diffuse { size: 0.14, dark: 0.6 }, rays: 0.0, figure: Plain, finish: Oiled, drift: 0.25 },
        Species { name: "Wood Zebrawood", tags: &["wood", "zebrawood", "exotic", "striped"], early: "#DDBA7E", late: "#3F2A19", rings: 220.0, latewood: 0.40, pores: Diffuse { size: 0.06, dark: 0.4 }, rays: 0.0, figure: Plain, finish: Satin, drift: 0.15 },
        Species { name: "Wood Olive", tags: &["wood", "olive", "figured", "kitchen", "swirl"], early: "#DAB98E", late: "#B8956A", rings: 240.0, latewood: 0.20, pores: Diffuse { size: 0.05, dark: 0.4 }, rays: 0.0, figure: Swirl { colour: srgb("#5A3E28") }, finish: Oiled, drift: 0.3 },
    ];
    species.iter().map(wood).collect()
}

/// One species, made.
pub fn wood(s: &Species) -> Preset {
    let early = srgb(s.early);
    let late = srgb(s.late);
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = early.lerp(late, 0.3);
        m.base_diffuse_roughness = 0.4;
        match s.finish {
            Finish::Raw => m.specular_roughness = 0.62,
            Finish::Oiled => m.specular_roughness = 0.42,
            Finish::Satin => {
                m.specular_roughness = 0.45;
                m.coat_weight = 0.55;
                m.coat_roughness = 0.22;
            }
            Finish::Gloss => {
                m.specular_roughness = 0.4;
                m.coat_weight = 1.0;
                m.coat_roughness = 0.03;
            }
        }
    }));

    // ---- The log: rings about a pith half a metre below, wandering -------
    let coords = r.node("texture_coordinate").out("Object");
    // The stem bends a few millimetres over a few centimetres.
    let bend_space = r.node("mapping").w("Vector", coords).v("Scale", [5.0, 22.0, 22.0]).out("Vector");
    let bend = r.node("noise_texture").w("Vector", bend_space).f("Scale", 1.0).f("Detail", 3.0).f("Roughness", 0.55).out("Color");
    let reach = match s.figure {
        Figure::Swirl { .. } => 0.035,
        _ => 0.006,
    };
    let bent = r.node("vector_math").set("operation", "multiply_add").w("Vector", bend).v("Vector 2", [0.0, reach, reach]).w("Vector 3", coords).out("Vector");
    // And a finer wobble on that: a year's boundary is never a clean line.
    let wobble_space = r.node("mapping").w("Vector", coords).v("Scale", [25.0, 90.0, 90.0]).out("Vector");
    let wobble = r.node("noise_texture").w("Vector", wobble_space).f("Scale", 1.0).f("Detail", 4.0).f("Roughness", 0.6).out("Color");
    let bent = r.node("vector_math").set("operation", "multiply_add").w("Vector", wobble).v("Vector 2", [0.0, 0.0012, 0.0012]).w("Vector 3", bent).out("Vector");
    let xyz = r.node("separate_xyz").w("Vector", bent).add();
    let y = r.math("add", Out(xyz, 1), 0.45);
    let z = r.math("add", Out(xyz, 2), 0.2);
    let yy = r.math("multiply", y, y);
    let zz = r.math("multiply", z, z);
    let rr = r.math("add", yy, zz);
    let radius = r.math("sqrt", rr, 0.0);
    let years = r.math("multiply", radius, s.rings);
    // No two years alike: the count drifts by a year or two either way.
    let along_years = r.node("combine_xyz").w("X", years).f("Y", 0.0).f("Z", 0.0).out("Vector");
    let uneven = r.node("noise_texture").w("Vector", along_years).f("Scale", 0.22).f("Detail", 2.0).out("Fac");
    let mut phase = plus(&mut r, uneven, 2.2, years);

    // Knots: branches the board was sawn through, the rings swirling round.
    let mut knot_core: Option<Out> = None;
    if let Figure::Knots { density, .. } = s.figure {
        let knot_space = r.node("mapping").w("Vector", coords).v("Scale", [1.0, 0.35, 1.0]).out("Vector");
        let knots = r.node("voronoi_texture").w("Vector", knot_space).f("Scale", density).f("Randomness", 1.0).add();
        let pick = r.node("separate_color").w("Color", Out(knots, 1)).out("Red");
        let kept = r.smooth(pick, (0.62, 0.64), (0.0, 1.0));
        let near = r.smooth(Out(knots, 0), (0.0, 0.32), (1.0, 0.0));
        let swirl = r.math("multiply", near, kept);
        phase = plus(&mut r, swirl, 3.5, phase);
        let core = r.smooth(Out(knots, 0), (0.04, 0.12), (1.0, 0.0));
        knot_core = Some(r.math("multiply", core, kept));
    }
    let year = r.math("fraction", phase, 0.0);

    // ---- Colour across a year ------------------------------------------
    let lw = s.latewood.clamp(0.05, 0.9);
    let earliest = early.lerp(Spectrum::ONE, 0.04);
    let latest = late.lerp(Spectrum::ZERO, 0.08);
    let mut colour = r.ramp(year, Linear, &[(0.0, earliest), (1.0 - lw, early), ((1.0 - lw * 0.55).min(0.99), early.lerp(late, 0.6)), ((1.0 - lw * 0.1).min(0.995), late), (1.0, latest)]);
    let late_share = r.smooth(year, (1.0 - lw, 1.0 - lw * 0.35), (0.0, 1.0));

    // Fine lines within the year: the wood's density rises and falls several
    // times a season, faintly - most of the lines the eye actually sees.
    let fine_jitter = r.node("noise_texture").w("Vector", bent).f("Scale", 400.0).f("Detail", 2.0).out("Fac");
    let fine_phase = plus(&mut r, fine_jitter, 0.25, phase);
    let fine_phase = r.math("multiply", fine_phase, 4.0);
    let fine_year = r.math("fraction", fine_phase, 0.0);
    let fine_line = r.smooth(fine_year, (0.84, 0.97), (1.0, 0.88));
    colour = r.mix("multiply", 1.0, colour, fine_line);

    // Drift: heartwood and sapwood, mineral, the tree's unevenness.
    let drift_space = r.node("mapping").w("Vector", coords).v("Scale", [2.0, 14.0, 14.0]).out("Vector");
    let drift_field = r.node("noise_texture").w("Vector", drift_space).f("Scale", 1.0).f("Detail", 3.0).out("Fac");
    let drift = r.range(drift_field, (0.3, 0.7), (1.0 - s.drift * 0.35, 1.0 + s.drift * 0.2));
    colour = r.mix("multiply", 1.0, colour, drift);

    // The fibre: fine streaks along the grain, the most of what says "wood"
    // close up - noise drawn out some forty times along x, at two sizes.
    let combed = r.node("mapping").w("Vector", bent).v("Scale", [18.0, 700.0, 700.0]).out("Vector");
    let coarse_fibre = r.node("noise_texture").w("Vector", combed).f("Scale", 1.0).f("Detail", 3.0).f("Roughness", 0.6).out("Fac");
    let combed_fine = r.node("mapping").w("Vector", bent).v("Scale", [60.0, 2600.0, 2600.0]).out("Vector");
    let fine_fibre = r.node("noise_texture").w("Vector", combed_fine).f("Scale", 1.0).f("Detail", 2.0).out("Fac");
    let fibre = plus(&mut r, fine_fibre, 0.6, coarse_fibre);
    let fibre = r.math("subtract", fibre, 0.8);
    let fibre_tone = r.range(fibre, (-0.35, 0.35), (0.86, 1.1));
    colour = r.mix("multiply", 1.0, colour, fibre_tone);

    let mut gloss_drop = r.math("multiply", fibre, 0.0);
    let mut sink = r.math("multiply", late_share, -0.15);

    // ---- Figure -----------------------------------------------------------
    match s.figure {
        Figure::Plain | Figure::Knots { .. } => {}
        Figure::Ribbon { strength } => {
            // Bands a centimetre or two wide along the grain, alternately
            // lighter and darker, and turning the sheen as they go.
            let band_space = r.node("mapping").w("Vector", bent).v("Scale", [4.0, 70.0, 70.0]).out("Vector");
            let band_field = r.node("noise_texture").w("Vector", band_space).f("Scale", 1.0).f("Detail", 1.0).out("Fac");
            let band = r.smooth(band_field, (0.42, 0.58), (0.0, 1.0));
            let tone = r.range(band, (0.0, 1.0), (1.0 - 0.14 * strength, 1.0 + 0.06 * strength));
            colour = r.mix("multiply", 1.0, colour, tone);
            let turn = r.math("multiply", band, 0.18 * strength);
            r.to(Param::AnisotropyRotation, turn);
            r.material_mut().specular_roughness_anisotropy = 0.5 * strength;
        }
        Figure::Curl { strength } => {
            // Ripples across the grain, a few millimetres apart.
            let across = r.node("mapping").w("Vector", bent).v("Scale", [140.0, 18.0, 18.0]).out("Vector");
            let ripple = r.node("wave_texture").set("type", "bands").set("direction", "x").w("Vector", across).f("Scale", 1.0).f("Distortion", 1.4).f("Detail", 1.0).out("Fac");
            let ripple = r.smooth(ripple, (0.25, 0.75), (0.0, 1.0));
            let tone = r.range(ripple, (0.0, 1.0), (1.0 - 0.16 * strength, 1.0 + 0.04 * strength));
            colour = r.mix("multiply", 1.0, colour, tone);
            let turn = r.math("multiply", ripple, 0.5 * strength);
            r.to(Param::AnisotropyRotation, turn);
            r.material_mut().specular_roughness_anisotropy = 0.6 * strength;
        }
        Figure::Streaks { colour: streak, amount } => {
            let streak_space = r.node("mapping").w("Vector", bent).v("Scale", [3.0, 160.0, 160.0]).out("Vector");
            let field = r.node("noise_texture").w("Vector", streak_space).f("Scale", 1.0).f("Detail", 4.0).f("Roughness", 0.6).out("Fac");
            let line = r.smooth(field, (0.56, 0.68), (0.0, amount));
            colour = r.node("mix").f("Factor", line).w("A", colour).c("B", streak).out("Result");
        }
        Figure::Swirl { colour: dark } => {
            // Dark lines where the twisted years' boundaries crowd.
            let line = r.smooth(year, (0.8, 0.95), (0.0, 0.9));
            colour = r.node("mix").f("Factor", line).w("A", colour).c("B", dark).out("Result");
        }
    }
    if let (Some(core), Figure::Knots { colour: knot, .. }) = (knot_core, s.figure) {
        colour = r.node("mix").f("Factor", core).w("A", colour).c("B", knot).out("Result");
        sink = plus(&mut r, core, -0.6, sink);
    }

    // ---- Pores -------------------------------------------------------------
    let (pore_size, pore_dark, ring_porous) = match s.pores {
        Pores::None => (0.0, 0.0, false),
        Pores::Diffuse { size, dark } => (size, dark, false),
        Pores::RingPorous { size, dark } => (size, dark, true),
    };
    if pore_size > 0.0 {
        // Each pore a vessel cut along: a dash some twenty times as long as
        // it is wide, lying along the grain.
        let density = 2200.0;
        let pore_space = r.node("mapping").w("Vector", bent).v("Scale", [density / 16.0, density, density]).out("Vector");
        let cells = r.node("voronoi_texture").w("Vector", pore_space).f("Scale", 1.0).f("Randomness", 1.0).add();
        let size = pore_size * 1.7;
        let mut pore = r.smooth(Out(cells, 0), (size * 0.6, size * 1.4), (1.0, 0.0));
        let pick = r.node("separate_color").w("Color", Out(cells, 1)).out("Red");
        if ring_porous {
            // Crowded into the earlywood, few and small after it.
            let early_zone = r.smooth(year, (0.14, 0.26), (1.0, 0.15));
            pore = r.math("multiply", pore, early_zone);
            let kept = r.smooth(pick, (0.08, 0.12), (0.0, 1.0));
            pore = r.math("multiply", pore, kept);
        } else {
            let kept = r.smooth(pick, (0.55, 0.6), (0.0, 1.0));
            pore = r.math("multiply", pore, kept);
        }
        let darker = r.mix("multiply", 1.0, colour, colour);
        let deep = r.node("mix").f("Factor", 0.4).w("A", darker).c("B", Spectrum::ZERO).out("Result");
        let pore_mix = r.math("multiply", pore, pore_dark);
        colour = r.node("mix").f("Factor", pore_mix).w("A", colour).w("B", deep).out("Result");
        gloss_drop = plus(&mut r, pore, 0.25, gloss_drop);
        sink = plus(&mut r, pore, -1.0, sink);
    }

    // ---- Rays ----------------------------------------------------------------
    if s.rays > 0.0 {
        // Thin ribbons running out from the pith: on a sawn face, short
        // flecks along the grain, lighter and glossier than the wood round
        // them - oak's silver grain.
        let ray_space = r.node("mapping").w("Vector", bent).v("Scale", [180.0, 60.0, 2400.0]).out("Vector");
        let rays = r.node("voronoi_texture").w("Vector", ray_space).f("Scale", 1.0).f("Randomness", 1.0).add();
        let pick = r.node("separate_color").w("Color", Out(rays, 1)).out("Red");
        let kept = r.smooth(pick, (0.5, 0.55), (0.0, 1.0));
        let body = r.smooth(Out(rays, 0), (0.05, 0.14), (1.0, 0.0));
        let fleck = r.math("multiply", body, kept);
        let fleck = r.math("multiply", fleck, s.rays * 0.6);
        let light = r.node("mix").f("Factor", 0.25).w("A", colour).c("B", Spectrum::ONE).out("Result");
        colour = r.node("mix").f("Factor", fleck).w("A", colour).w("B", light).out("Result");
        gloss_drop = plus(&mut r, fleck, -0.15, gloss_drop);
    }

    r.to(Param::BaseColor, colour);
    let base_roughness = r.material_mut().specular_roughness;
    let gloss = r.math("add", gloss_drop, base_roughness);
    r.to(Param::SpecularRoughness, gloss);
    let fibre_relief = r.math("multiply", fibre, 0.05);
    let height = r.math("add", sink, fibre_relief);
    let depth = match s.finish {
        Finish::Gloss => 1.2e-5,
        Finish::Satin => 2.5e-5,
        Finish::Oiled | Finish::Raw => 4e-5,
    };
    r.bump(height, depth);
    preset(s.name, "Wood", s.tags, MaterialType::Generic, r)
}

fn plus(r: &mut Recipe, a: Out, weight: f32, b: Out) -> Out {
    crate::plus(r, a, weight, b)
}

