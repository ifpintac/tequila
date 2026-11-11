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
    /// A burl: buds that never grew into branches, each a small dark eye
    /// the rings wrap round, in clusters, with the grain between them
    /// going every way. `eyes` to the metre, on `share` of the places one
    /// could be: a walnut burl is crowded with them, a bird's-eye maple
    /// has a few.
    Burl { colour: Spectrum, eyes: f32, share: f32 },
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
        // More of what instruments and hi-fi are made of, and the same woods
        // under a piano lacquer or stained black with the grain left open.
        Species { name: "Wood Alder", tags: &["wood", "alder", "guitar body", "plain", "pale", "reddish"], early: "#C89A72", late: "#B88660", rings: 420.0, latewood: 0.15, pores: Diffuse { size: 0.03, dark: 0.3 }, rays: 0.1, figure: Plain, finish: Satin, drift: 0.2 },
        Species { name: "Wood Koa", tags: &["wood", "koa", "figured", "ukulele", "guitar", "golden", "instrument"], early: "#A9683A", late: "#7E4522", rings: 300.0, latewood: 0.20, pores: Diffuse { size: 0.08, dark: 0.5 }, rays: 0.0, figure: Curl { strength: 0.6 }, finish: Gloss, drift: 0.5 },
        Species { name: "Wood Padauk", tags: &["wood", "padauk", "exotic", "orange", "red"], early: "#B5482A", late: "#8F2F1A", rings: 320.0, latewood: 0.20, pores: Diffuse { size: 0.11, dark: 0.5 }, rays: 0.0, figure: Plain, finish: Oiled, drift: 0.25 },
        Species { name: "Wood Bubinga", tags: &["wood", "bubinga", "exotic", "red", "drum shell", "instrument"], early: "#8E4A3A", late: "#6B2F26", rings: 420.0, latewood: 0.20, pores: Diffuse { size: 0.08, dark: 0.45 }, rays: 0.0, figure: Streaks { colour: srgb("#3B1A16"), amount: 0.5 }, finish: Gloss, drift: 0.35 },
        Species { name: "Wood Macassar Ebony", tags: &["wood", "ebony", "macassar", "exotic", "striped", "dark", "veneer"], early: "#6B4A33", late: "#1C130F", rings: 180.0, latewood: 0.50, pores: Diffuse { size: 0.05, dark: 0.3 }, rays: 0.0, figure: Streaks { colour: srgb("#120C0A"), amount: 0.6 }, finish: Gloss, drift: 0.3 },
        Species { name: "Wood Walnut Figured", tags: &["wood", "walnut", "figured", "crotch", "swirl", "veneer", "hi-fi", "luxury"], early: "#7A5638", late: "#4E3320", rings: 260.0, latewood: 0.20, pores: Diffuse { size: 0.06, dark: 0.45 }, rays: 0.0, figure: Swirl { colour: srgb("#2A1B13") }, finish: Gloss, drift: 0.5 },
        Species { name: "Wood Walnut Burl", tags: &["wood", "walnut", "burl", "burr", "eyes", "veneer", "hi-fi", "dashboard", "luxury"], early: "#8A6240", late: "#5A3A22", rings: 300.0, latewood: 0.25, pores: Diffuse { size: 0.05, dark: 0.4 }, rays: 0.0, figure: Burl { colour: srgb("#24160F"), eyes: 150.0, share: 0.55 }, finish: Gloss, drift: 0.5 },
        Species { name: "Wood Birdseye Maple", tags: &["wood", "maple", "birdseye", "bird's eye", "figured", "eyes", "instrument", "veneer", "pale"], early: "#EBD6AF", late: "#DCC198", rings: 520.0, latewood: 0.16, pores: Diffuse { size: 0.04, dark: 0.3 }, rays: 0.1, figure: Burl { colour: srgb("#8A6A44"), eyes: 210.0, share: 0.22 }, finish: Gloss, drift: 0.15 },
        Species { name: "Wood Walnut Gloss", tags: &["wood", "walnut", "gloss", "piano lacquer", "hi-fi", "speaker", "dark"], early: "#6E4C36", late: "#553826", rings: 300.0, latewood: 0.20, pores: Diffuse { size: 0.09, dark: 0.55 }, rays: 0.0, figure: Streaks { colour: srgb("#2A1B13"), amount: 0.35 }, finish: Gloss, drift: 0.45 },
        Species { name: "Wood Maple Gloss", tags: &["wood", "maple", "gloss", "lacquer", "drum shell", "pale"], early: "#EBD6AF", late: "#DCC198", rings: 520.0, latewood: 0.16, pores: Diffuse { size: 0.04, dark: 0.3 }, rays: 0.15, figure: Plain, finish: Gloss, drift: 0.15 },
        Species { name: "Wood Mahogany Gloss", tags: &["wood", "mahogany", "gloss", "lacquer", "guitar", "red", "ribbon"], early: "#8E4B2F", late: "#7A3D24", rings: 360.0, latewood: 0.18, pores: Diffuse { size: 0.1, dark: 0.5 }, rays: 0.0, figure: Ribbon { strength: 0.6 }, finish: Gloss, drift: 0.2 },
        Species { name: "Wood Oak Black Open Grain", tags: &["wood", "oak", "black", "ebonised", "stained", "open grain", "speaker", "furniture"], early: "#1E1D1C", late: "#151413", rings: 340.0, latewood: 0.45, pores: RingPorous { size: 0.13, dark: 0.8 }, rays: 0.3, figure: Plain, finish: Satin, drift: 0.1 },
        Species { name: "Wood Ash Black Open Grain", tags: &["wood", "ash", "black", "ebonised", "stained", "open grain", "guitar", "furniture"], early: "#201F1E", late: "#141312", rings: 300.0, latewood: 0.45, pores: RingPorous { size: 0.14, dark: 0.8 }, rays: 0.0, figure: Plain, finish: Satin, drift: 0.1 },
        Species { name: "Wood Oak Whitewashed", tags: &["wood", "oak", "whitewashed", "limed", "pale", "scandinavian", "furniture"], early: "#DCD2BF", late: "#CBBFA9", rings: 340.0, latewood: 0.45, pores: RingPorous { size: 0.11, dark: 0.3 }, rays: 0.5, figure: Plain, finish: Raw, drift: 0.12 },
    ];
    let mut out: Vec<Preset> = species.iter().map(wood).collect();
    out.push(plywood());
    out.push(bamboo("Wood Bamboo", &["wood", "bamboo", "laminated", "strips", "nodes", "pale", "natural", "sustainable"], srgb("#DEC388"), srgb("#BF9A58"), srgb("#8A6430")));
    out.push(bamboo("Wood Bamboo Carbonised", &["wood", "bamboo", "laminated", "strips", "nodes", "carbonised", "caramel", "brown"], srgb("#B4804A"), srgb("#8A5628"), srgb("#553616")));
    out.push(fibreboard("MDF Raw", &["mdf", "fibreboard", "raw", "cabinet", "speaker", "brown"], srgb("#A9865E"), srgb("#8E6E4A")));
    out.push(fibreboard("MDF Black", &["mdf", "fibreboard", "black", "through-coloured", "cabinet", "speaker"], srgb("#2A2826"), srgb("#1E1C1B")));
    out
}

/// Birch plywood: a rotary-cut face, its grain wide and wandering, and
/// on every edge the plies, a millimetre and a half each, light where a
/// ply's grain runs along the edge and dark where it is cut across.
fn plywood() -> Preset {
    let face = srgb("#E3CDA6");
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = face;
        m.base_diffuse_roughness = 0.4;
        m.specular_roughness = 0.45;
        m.specular_weight = 0.0;
        m.coat_weight = 1.0;
        m.coat_roughness = 0.3;
    }));
    let coords = r.node("texture_coordinate").out("Object");
    let normal = r.node("texture_coordinate").out("Normal");
    let xyz = r.node("separate_xyz").w("Vector", coords).add();
    let up = r.node("separate_xyz").w("Vector", normal).out("Y");
    // The face: peeled off the log round and round, so its figure is wide
    // flames along the grain and not a board's cathedrals.
    let flame_space = r.node("mapping").w("Vector", coords).v("Scale", [3.0, 1.0, 26.0]).out("Vector");
    let flame = r.node("noise_texture").w("Vector", flame_space).f("Scale", 1.0).f("Detail", 4.0).f("Roughness", 0.55).f("Distortion", 0.8).out("Fac");
    let fibre_space = r.node("mapping").w("Vector", coords).v("Scale", [30.0, 900.0, 900.0]).out("Vector");
    let fibre = r.node("noise_texture").w("Vector", fibre_space).f("Scale", 1.0).f("Detail", 3.0).out("Fac");
    let face_tone = r.range(flame, (0.3, 0.7), (0.86, 1.04));
    let fibre_tone = r.range(fibre, (0.3, 0.7), (0.93, 1.05));
    let face_shade = r.math("multiply", face_tone, fibre_tone);
    let face_colour = r.node("mix").set("blend_type", "multiply").f("Factor", 1.0).c("A", face).w("B", face_shade).out("Result");
    // The edge: plies up the thickness, every other one end grain.
    let ply = r.math("divide", Out(xyz, 1), 0.003);
    let which = r.math("fraction", ply, 0.0);
    let end_grain = r.smooth(which, (0.47, 0.53), (0.0, 1.0));
    let glue = r.math("multiply", which, 2.0);
    let glue = r.math("fraction", glue, 0.0);
    let line = r.smooth(glue, (0.0, 0.07), (1.0, 0.0));
    let edge_colour = r.node("mix").f("Factor", end_grain).c("A", srgb("#DCC29A")).c("B", srgb("#B8935F")).out("Result");
    let edge_colour = r.node("mix").f("Factor", line).w("A", edge_colour).c("B", srgb("#5E4226")).out("Result");
    let speck = r.node("noise_texture").f("Scale", 2600.0).f("Detail", 2.0).out("Fac");
    let speck_tone = r.range(speck, (0.3, 0.7), (0.9, 1.06));
    let edge_colour = r.node("mix").set("blend_type", "multiply").f("Factor", 1.0).w("A", edge_colour).w("B", speck_tone).out("Result");
    // Which is which: the faces look up and down, the edges sideways.
    let facing = r.math("absolute", up, 0.0);
    let is_face = r.smooth(facing, (0.6, 0.8), (0.0, 1.0));
    let colour = r.node("mix").f("Factor", is_face).w("A", edge_colour).w("B", face_colour).out("Result");
    r.to(Param::BaseColor, colour);
    r.bump(fibre, 1.5e-5);
    preset("Plywood Birch", "Wood", &["plywood", "birch", "ply", "baltic birch", "edge", "layers", "cabinet", "speaker", "furniture"], MaterialType::Generic, r)
}

/// Laminated bamboo: the culm split into strips two centimetres wide,
/// planed and glued side by side. Not a wood: no rings, but fibres dead
/// straight along each strip, the dark dashes of its vessels among them,
/// and every thirty centimetres or so a node, the joint of the cane - a
/// dark line across the strip with paler wood either side of it - at a
/// different place in every strip. `light` and `dark` are the strips'
/// colours, `node` a joint's.
fn bamboo(name: &'static str, tags: &'static [&'static str], light: Spectrum, dark: Spectrum, node: Spectrum) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = light.lerp(dark, 0.5);
        m.base_diffuse_roughness = 0.4;
        m.specular_roughness = 0.45;
        m.specular_weight = 0.0;
        m.coat_weight = 1.0;
        m.coat_roughness = 0.28;
    }));
    let coords = r.node("texture_coordinate").out("Object");
    let xyz = r.node("separate_xyz").w("Vector", coords).add();
    let (x, y, z) = (Out(xyz, 0), Out(xyz, 1), Out(xyz, 2));
    // The strips: 20 mm across, in layers 6 mm thick, each its own.
    let across = r.math("divide", z, 0.02);
    let up = r.math("divide", y, 0.006);
    let strip = r.math("floor", across, 0.0);
    let layer = r.math("floor", up, 0.0);
    let id = r.node("combine_xyz").w("X", strip).w("Y", layer).f("Z", 3.0).out("Vector");
    let own = r.node("white_noise_texture").w("Vector", id).out("Value");
    let id2 = r.node("combine_xyz").w("X", strip).w("Y", layer).f("Z", 11.0).out("Vector");
    let shift = r.node("white_noise_texture").w("Vector", id2).out("Value");
    // A strip's colour: its own, from the cane it was split from.
    let strip_colour = r.node("mix").w("Factor", own).c("A", dark).c("B", light).out("Result");
    // The fibre: dead straight along x, fine.
    let combed = r.node("mapping").w("Vector", coords).v("Scale", [6.0, 1800.0, 1800.0]).out("Vector");
    let fibre = r.node("noise_texture").w("Vector", combed).f("Scale", 1.0).f("Detail", 3.0).f("Roughness", 0.6).out("Fac");
    let fibre_tone = r.range(fibre, (0.3, 0.7), (0.9, 1.06));
    // The vessels: short dark dashes along the grain, many.
    let dash_space = r.node("mapping").w("Vector", coords).v("Scale", [110.0, 2400.0, 2400.0]).out("Vector");
    let dashes = r.node("voronoi_texture").w("Vector", dash_space).f("Scale", 1.0).f("Randomness", 1.0).out("Distance");
    let dash = r.smooth(dashes, (0.08, 0.2), (1.0, 0.0));
    let dash_tone = r.range(dash, (0.0, 1.0), (1.0, 0.72));
    // The nodes: one every 32 cm along a strip, wherever that strip's
    // cane had it.
    let along = r.math("divide", x, 0.32);
    let along = r.math("add", along, shift);
    let f = r.math("fraction", along, 0.0);
    let off = r.math("subtract", f, 0.5);
    let from_node = r.math("absolute", off, 0.0);
    // The joint itself, a millimetre and a half; and the paler, denser
    // wood a centimetre either side of it.
    let joint = r.smooth(from_node, (0.002, 0.006), (1.0, 0.0));
    let collar = r.smooth(from_node, (0.006, 0.04), (1.0, 0.0));
    let collar_tone = r.range(collar, (0.0, 1.0), (1.0, 1.1));
    // The glue lines between strips and between layers.
    let fz = r.math("fraction", across, 0.0);
    let fz = r.math("subtract", fz, 0.5);
    let fz = r.math("absolute", fz, 0.0);
    let seam_z = r.smooth(fz, (0.485, 0.5), (0.0, 1.0));
    let fy = r.math("fraction", up, 0.0);
    let fy = r.math("subtract", fy, 0.5);
    let fy = r.math("absolute", fy, 0.0);
    let seam_y = r.smooth(fy, (0.47, 0.5), (0.0, 1.0));
    let seam = r.math("maximum", seam_z, seam_y);
    let seam_tone = r.range(seam, (0.0, 1.0), (1.0, 0.6));
    let shade = r.math("multiply", fibre_tone, dash_tone);
    let shade = r.math("multiply", shade, collar_tone);
    let shade = r.math("multiply", shade, seam_tone);
    let colour = r.node("mix").set("blend_type", "multiply").f("Factor", 1.0).w("A", strip_colour).w("B", shade).out("Result");
    let colour = r.node("mix").f("Factor", joint).w("A", colour).c("B", node).out("Result");
    r.to(Param::BaseColor, colour);
    // The joint stands a little proud under the lacquer, the seams sink.
    let height = plus(&mut r, joint, 0.5, fibre);
    let height = plus(&mut r, seam, -0.8, height);
    r.bump(height, 1.6e-5);
    preset(name, "Wood", tags, MaterialType::Generic, r)
}

/// Fibreboard: wood fibre pressed with resin, the same all through, with
/// no grain - a fine fleck, a dry face, and nothing else.
fn fibreboard(name: &'static str, tags: &'static [&'static str], light: Spectrum, dark: Spectrum) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = light.lerp(dark, 0.5);
        m.base_diffuse_roughness = 0.6;
        m.specular_roughness = 0.62;
        m.specular_weight = 0.4;
    }));
    let fleck = r.node("noise_texture").f("Scale", 1800.0).f("Detail", 5.0).f("Roughness", 0.7).f("Distortion", 1.0).out("Fac");
    let cloud = r.node("noise_texture").f("Scale", 40.0).f("Detail", 3.0).out("Fac");
    let tone = plus(&mut r, cloud, 0.4, fleck);
    let colour = r.ramp(tone, Linear, &[(0.45, dark), (1.0, light)]);
    r.to(Param::BaseColor, colour);
    r.bump(fleck, 1.2e-5);
    preset(name, "Wood", tags, MaterialType::Generic, r)
}

/// One species, made.
pub fn wood(s: &Species) -> Preset {
    let early = srgb(s.early);
    let late = srgb(s.late);
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = early.lerp(late, 0.3);
        m.base_diffuse_roughness = 0.4;
        // Under a lacquer the wood has no shine of its own: there is no
        // air between the two for a surface to reflect, and with one the
        // lacquer's highlight sat on a second, broad one, a milky veil
        // over the grain. A figured wood keeps half of it: the shimmer of
        // a ribbon or a curl is its fibres turning the light, under
        // whatever is over them.
        let under = match s.figure {
            Figure::Ribbon { .. } | Figure::Curl { .. } => 0.5,
            _ => 0.0,
        };
        match s.finish {
            // Bare wood is open and dry: what sheen it has is weak and
            // broad, and an oil only closes it a little.
            Finish::Raw => {
                m.specular_roughness = 0.7;
                m.specular_weight = 0.4;
            }
            Finish::Oiled => {
                m.specular_roughness = 0.5;
                m.specular_weight = 0.6;
            }
            Finish::Satin => {
                m.specular_roughness = 0.45;
                m.specular_weight = under;
                m.coat_weight = 1.0;
                m.coat_roughness = 0.28;
            }
            Finish::Gloss => {
                m.specular_roughness = 0.4;
                m.specular_weight = under;
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
        // A crowded burl's grain is wild between its eyes; a few eyes in
        // a straight board leave it straight.
        Figure::Burl { share, .. } => 0.006 + 0.13 * (share - 0.2).max(0.0),
        _ => 0.006,
    };
    let bent = r.node("vector_math").set("operation", "multiply_add").w("Vector", bend).v("Vector 2", [0.0, reach, reach]).w("Vector 3", coords).out("Vector");
    // And a finer wobble on that: a year's boundary is never a clean line.
    let wobble_space = r.node("mapping").w("Vector", coords).v("Scale", [25.0, 90.0, 90.0]).out("Vector");
    let wobble = r.node("noise_texture").w("Vector", wobble_space).f("Scale", 1.0).f("Detail", 4.0).f("Roughness", 0.6).out("Color");
    let bent = r.node("vector_math").set("operation", "multiply_add").w("Vector", wobble).v("Vector 2", [0.0, 0.0012, 0.0012]).w("Vector 3", bent).out("Vector");
    let xyz = r.node("separate_xyz").w("Vector", bent).add();
    // The pith is off on the diagonal, so the faces a product shows, up
    // and to the front, are both cut across the rings at a slope - the
    // straight, close grain of a board sawn on the rift, which is how a
    // cabinet's show faces are chosen. Under a face, the rings lie flat
    // to it and show as a few wide blotches.
    let y = r.math("add", Out(xyz, 1), 0.33);
    let z = r.math("add", Out(xyz, 2), 0.33);
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
    // A burl's eyes: small, round, in clusters, each with the rings drawn
    // in round it as round a knot, and a dark bud at its middle.
    if let Figure::Burl { eyes, share, .. } = s.figure {
        let cells = r.node("voronoi_texture").w("Vector", coords).f("Scale", eyes).f("Randomness", 1.0).add();
        let pick = r.node("separate_color").w("Color", Out(cells, 1)).out("Red");
        let own = r.node("white_noise_texture").w("Vector", Out(cells, 1)).out("Value");
        // Clusters: more of them here, fewer there.
        let crowd = r.node("noise_texture").w("Vector", coords).f("Scale", eyes * 0.12).f("Detail", 2.0).out("Fac");
        let chance = plus(&mut r, crowd, 1.3, own);
        let _ = pick;
        let kept = r.smooth(chance, (1.5 - share - 0.03, 1.5 - share + 0.03), (0.0, 1.0));
        let near = r.smooth(Out(cells, 0), (0.0, 0.5), (1.0, 0.0));
        let swirl = r.math("multiply", near, kept);
        // One year drawn in round it, and no more: two are a target.
        phase = plus(&mut r, swirl, 0.9, phase);
        let core = r.smooth(Out(cells, 0), (0.0, 0.3), (0.9, 0.0));
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
        Figure::Burl { colour: dark, share, .. } => {
            // Dark lines where the years crowd round the eyes, as in the
            // swirl of an olive, the more of them the more eyes there are.
            let line = r.smooth(year, (0.82, 0.96), (0.0, ((share - 0.2) * 1.2).clamp(0.03, 0.45)));
            colour = r.node("mix").f("Factor", line).w("A", colour).c("B", dark).out("Result");
        }
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
    if let (Some(core), Figure::Knots { colour: knot, .. } | Figure::Burl { colour: knot, .. }) = (knot_core, s.figure) {
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
    if matches!(s.finish, Finish::Satin) {
        // A satin lacquer does not fill the pores: it is duller over them.
        let coat = r.material_mut().coat_roughness;
        let over = r.math3("multiply_add", gloss_drop, 0.6, coat);
        r.to(Param::CoatRoughness, over);
    }
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

