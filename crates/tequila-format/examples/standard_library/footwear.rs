//! Footwear: what a sneaker is made of besides its leather and suede - the
//! tongue's nylon, the jersey lining, the cupsole's rubber, and the parts
//! that run along a strip: laces, webbing tape and stitching thread.
//!
//! Panels and soles are read in object space, in metres, as every other
//! preset is. A lace, a tape and a thread are read along their own UVs
//! instead, since a braid, a rib or a twist has to follow the strip, which
//! object space cannot know. Those take UVs that run along the strip in U
//! and across it (a tape) or round it (a lace, a thread) in V, 0 to 1, with
//! U in proportion to V: a strip unwrapped without stretching, as a curve's
//! UVs are once their length is set to match. One unwrapped 0 to 1 along
//! its whole length wants its Mapping node's U scale raised to its length
//! over its width; one without UVs shows the yarn's colour alone.

use crate::fabric::{weave, woven_with, Cloth, Pattern};
use crate::recipe::Out;
use crate::{base, micro, plus, preset, srgb, wear, Preset, Recipe};
use tequila_core::spectrum::Spectrum;
use tequila_materials::graph::{Node, NodeKind};
use tequila_materials::material_type::{MaterialType, Param};

pub fn all() -> Vec<Preset> {
    vec![
        nylon("Nylon Taffeta Grey", &["nylon", "taffeta", "tongue", "sneaker", "footwear", "jacket", "grey"], srgb("#5E6166"), srgb("#55585D")),
        nylon("Nylon Taffeta Black", &["nylon", "taffeta", "tongue", "sneaker", "footwear", "jacket", "black"], srgb("#202124"), srgb("#1B1C1F")),
        jersey("Jersey Lining Grey", &["jersey", "knit", "lining", "sneaker", "footwear", "tricot", "grey"], srgb("#4D5055")),
        jersey("Jersey Lining White", &["jersey", "knit", "lining", "sneaker", "footwear", "tricot", "white"], srgb("#DCDAD4")),
        outsole_white(),
        outsole_gum(),
        lace("Shoelace Flat White", &["shoelace", "lace", "braid", "flat lace", "sneaker", "footwear", "white"], srgb("#E9E6DF")),
        lace("Shoelace Flat Black", &["shoelace", "lace", "braid", "flat lace", "sneaker", "footwear", "black"], srgb("#1E1E20")),
        webbing("Webbing Nylon Black", &["webbing", "tape", "strap", "pull tab", "nylon", "sneaker", "bag", "black"], srgb("#1F2023")),
        thread("Thread Bonded White", &["thread", "stitching", "stitch", "seam", "sewing", "nylon", "white"], srgb("#ECE9E1")),
        thread("Thread Bonded Black", &["thread", "stitching", "stitch", "seam", "sewing", "nylon", "black"], srgb("#1C1C1E")),
    ]
}

/// `colour` times `factor`, a node's output.
fn tint(r: &mut Recipe, colour: Spectrum, factor: Out) -> Out {
    r.node("mix").set("blend_type", "multiply").f("Factor", 1.0).c("A", colour).w("B", factor).out("Result")
}

fn noise(r: &mut Recipe, coords: Out, scale: f32, detail: f32, roughness: f32) -> Out {
    r.node("noise_texture").w("Vector", coords).f("Scale", scale).f("Detail", detail).f("Roughness", roughness).out("Fac")
}

fn occlusion(r: &mut Recipe, radius: f32, contrast: f32) -> Out {
    Out(r.graph.add_node(Node::new(NodeKind::Occlusion { radius, contrast })), 0)
}

/// A strip's own coordinates: U along it, V across or round it, through a
/// Mapping node whose U scale fits a strip unwrapped 0 to 1 along its
/// length.
fn strip(r: &mut Recipe) -> (Out, Out) {
    let uv = r.node("texture_coordinate").out("UV");
    let fitted = r.node("mapping").w("Vector", uv).v("Scale", [1.0, 1.0, 1.0]).out("Vector");
    let parts = r.node("separate_xyz").w("Vector", fitted).add();
    (Out(parts, 0), Out(parts, 1))
}

/// Nylon taffeta, the padded tongue's: a fine, tight plain weave of
/// filament yarn with a soft sheen, puckered over its padding where it is
/// quilted down.
fn nylon(name: &'static str, tags: &'static [&'static str], warp: Spectrum, weft: Spectrum) -> Preset {
    let cloth = Cloth {
        name,
        tags,
        pattern: Pattern::Plain,
        threads: 4200.0,
        warp,
        weft,
        variation: 0.04,
        slub: 0.05,
        fill: 0.97,
        fuzz: 0.04,
        fuzz_colour: warp.lerp(Spectrum::ONE, 0.3),
        heather: None,
        depth: 2.5e-5,
        roughness: 0.42,
    };
    woven_with(&cloth, |r, height| {
        // The padding under it: soft puckers a centimetre across, half a
        // millimetre deep, twenty times the weave's own depth.
        let puckers = r.node("noise_texture").f("Scale", 90.0).f("Detail", 2.0).f("Roughness", 0.5).f("Distortion", 0.4).out("Fac");
        plus(r, puckers, 20.0, height)
    })
}

/// A jersey knit, as sneakers are lined with: columns (wales) of
/// interlocked loops, each loop's two legs leaning in towards the column's
/// centre, so the face is rows of small Vs; a little fuzz, and the yarn's
/// own shade varying column by column.
pub(crate) fn jersey(name: &'static str, tags: &'static [&'static str], colour: Spectrum) -> Preset {
    let mut r = Recipe::new(base(MaterialType::RealCloth, |m| {
        m.base_color = colour;
        m.fuzz_weight = 0.35;
        m.fuzz_color = colour.lerp(Spectrum::ONE, 0.25);
        m.specular_roughness = 0.75;
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
    let mut tone: Option<Out> = None;
    for (i, (u, v)) in planes.into_iter().enumerate() {
        let (h, t) = knit(&mut r, u, v, i as f32 * 17.3);
        let hw = r.math("multiply", h, weights[i]);
        let tw = r.math("multiply", t, weights[i]);
        height = Some(match height {
            Some(a) => r.math("add", a, hw),
            None => hw,
        });
        tone = Some(match tone {
            Some(a) => r.math("add", a, tw),
            None => tw,
        });
    }
    let (height, tone) = (height.unwrap(), tone.unwrap());
    let colour_out = tint(&mut r, colour, tone);
    r.to(Param::BaseColor, colour_out);
    r.bump(height, 1.2e-4);
    preset(name, "Fabric", tags, MaterialType::RealCloth, r)
}

/// One plane of jersey, (u, v) in metres: 1250 wales and 1600 courses to
/// the metre. The height of the loop's leg at the point, and its shade.
fn knit(r: &mut Recipe, u: Out, v: Out, seed: f32) -> (Out, Out) {
    let uu = r.math("multiply", u, 1250.0);
    let vv = r.math("multiply", v, 1600.0);
    let iu = r.math("floor", uu, 0.0);
    let x = r.math("fraction", uu, 0.0);
    let y = r.math("fraction", vv, 0.0);
    // The two legs, leaning in: centred at a quarter and three quarters
    // across the wale, tilted by a fifth of it over a course.
    let tilt = r.math("subtract", y, 0.5);
    let tilt = r.math("multiply", tilt, 0.2);
    let left_centre = r.math("add", tilt, 0.27);
    let right_centre = r.math("subtract", 0.73, tilt);
    let leg = |r: &mut Recipe, centre: Out| {
        let d = r.math("subtract", x, centre);
        let d = r.math("divide", d, 0.22);
        let sq = r.math("multiply", d, d);
        let rest = r.math("subtract", 1.0, sq);
        let rest = r.math("maximum", rest, 0.0);
        r.math("sqrt", rest, 0.0)
    };
    let left = leg(r, left_centre);
    let right = leg(r, right_centre);
    let legs = r.math("maximum", left, right);
    // Each loop arches as it goes over the one below it.
    let a = r.math("multiply", y, std::f32::consts::PI);
    let arch = r.math("sine", a, 0.0);
    let arch = r.math("multiply", arch, 0.3);
    let arch = r.math("add", arch, 0.7);
    let shaped = r.math("multiply", legs, arch);
    // The plies twisting along each leg.
    let ply_phase = r.math("multiply", vv, 3.0);
    let ply_phase = r.math("add", ply_phase, x);
    let ply_phase = r.math("multiply", ply_phase, std::f32::consts::TAU);
    let ply = r.math("sine", ply_phase, 0.0);
    let height = plus(r, ply, 0.06, shaped);
    // Column by column, the yarn's own shade; the gaps between legs dark.
    let id = r.node("combine_xyz").w("X", iu).f("Y", seed).f("Z", 0.0).out("Vector");
    let pick = r.node("white_noise_texture").w("Vector", id).out("Value");
    let own = r.range(pick, (0.0, 1.0), (0.94, 1.04));
    let shade = r.range(shaped, (0.0, 1.0), (0.55, 1.0));
    let tone = r.math("multiply", own, shade);
    (height, tone)
}

/// Cupsole rubber, white: moulded, so its surface is the mould's fine
/// matte tooth, a little shinier where it has been rubbed; warm, not pure
/// white, and greyer with dust down in the grooves.
fn outsole_white() -> Preset {
    let colour = srgb("#E7E1D3");
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = colour;
        m.base_diffuse_roughness = 0.5;
        m.specular_ior = 1.52;
        m.specular_roughness = 0.62;
    }));
    let coords = r.node("texture_coordinate").out("Object");
    let mottle_field = noise(&mut r, coords, 40.0, 4.0, 0.55);
    let mottle = r.range(mottle_field, (0.3, 0.7), (0.97, 1.02));
    let occ = occlusion(&mut r, 0.004, 1.0);
    let dust = r.range(occ, (0.0, 1.0), (0.84, 1.0));
    let shade = r.math("multiply", mottle, dust);
    let toned = tint(&mut r, colour, shade);
    r.to(Param::BaseColor, toned);
    let gloss = wear(&mut r, 0.5, 0.68, 14.0);
    r.to(Param::SpecularRoughness, gloss);
    micro(&mut r, 5000.0, 7e-6);
    preset("Outsole Rubber White", "Plastic", &["rubber", "outsole", "cupsole", "sole", "sneaker", "footwear", "white"], MaterialType::Generic, r)
}

/// Gum rubber: translucent and amber, so light goes into it and comes back
/// warmer and deeper; satin where it is moulded smooth.
fn outsole_gum() -> Preset {
    let colour = srgb("#8A5428");
    let mut r = Recipe::new(base(MaterialType::Translucent, |m| {
        m.base_color = colour;
        m.subsurface_color = srgb("#9C5E2A");
        m.subsurface_radius = tequila_core::spectrum::Spectrum { r: 0.006, g: 0.003, b: 0.0012 };
        m.specular_roughness = 0.45;
    }));
    let coords = r.node("texture_coordinate").out("Object");
    let cloud = noise(&mut r, coords, 25.0, 4.0, 0.55);
    let shade = r.range(cloud, (0.3, 0.7), (0.92, 1.05));
    let toned = tint(&mut r, colour, shade);
    r.to(Param::BaseColor, toned);
    r.to(Param::SubsurfaceColor, toned);
    let gloss = wear(&mut r, 0.38, 0.55, 14.0);
    r.to(Param::SpecularRoughness, gloss);
    micro(&mut r, 5000.0, 6e-6);
    preset("Outsole Rubber Gum", "Translucent", &["gum", "rubber", "outsole", "sole", "sneaker", "footwear", "brown", "amber"], MaterialType::Translucent, r)
}

/// A flat lace: a tube braided from sixteen yarns, eight each way round,
/// pressed flat. Seen from above, a 2/2 diamond braid, each yarn going
/// over two and under two of the other way's at 45 degrees to the lace.
fn lace(name: &'static str, tags: &'static [&'static str], colour: Spectrum) -> Preset {
    let mut r = Recipe::new(base(MaterialType::RealCloth, |m| {
        m.base_color = colour;
        m.fuzz_weight = 0.25;
        m.fuzz_color = colour.lerp(Spectrum::ONE, 0.3);
        m.specular_roughness = 0.72;
    }));
    let (u, v) = strip(&mut r);
    // The two ways round: eight yarns each, at 45 degrees.
    let carriers = 8.0;
    let along = r.math("multiply", u, carriers);
    let round = r.math("multiply", v, carriers);
    let a = r.math("add", round, along);
    let b = r.math("subtract", round, along);
    let yarn = Cloth {
        name,
        tags,
        pattern: Pattern::Twill { over: 2, under: 2 },
        threads: 1.0,
        warp: colour,
        weft: colour,
        variation: 0.06,
        slub: 0.2,
        fill: 0.94,
        fuzz: 0.25,
        fuzz_colour: colour,
        heather: None,
        depth: 0.0,
        roughness: 0.72,
    };
    let woven = weave(&mut r, a, b, &yarn, 3.0);
    let toned = tint(&mut r, colour, woven.tone);
    r.to(Param::BaseColor, toned);
    r.bump(woven.height, 3e-4);
    // Its thumbnail a lace's size: a UV unit, once round it, two centimetres.
    preset(name, "Fabric", tags, MaterialType::RealCloth, r).previewed_as_strip(0.02)
}

/// Webbing tape: a dense twill of heavy filament warp running along the
/// tape, forty ends across its width, with a soft sheen along the ribs.
fn webbing(name: &'static str, tags: &'static [&'static str], colour: Spectrum) -> Preset {
    let mut r = Recipe::new(base(MaterialType::RealCloth, |m| {
        m.base_color = colour;
        m.fuzz_weight = 0.08;
        m.fuzz_color = colour.lerp(Spectrum::ONE, 0.25);
        m.specular_roughness = 0.5;
    }));
    let (u, v) = strip(&mut r);
    // The warp runs along the tape, so it is the weave's columns: v across.
    let ends = r.math("multiply", v, 40.0);
    let picks = r.math("multiply", u, 30.0);
    let tape = Cloth {
        name,
        tags,
        pattern: Pattern::Twill { over: 2, under: 1 },
        threads: 1.0,
        warp: colour,
        weft: colour,
        variation: 0.05,
        slub: 0.1,
        fill: 0.96,
        fuzz: 0.08,
        fuzz_colour: colour,
        heather: None,
        depth: 0.0,
        roughness: 0.5,
    };
    let woven = weave(&mut r, ends, picks, &tape, 9.0);
    let toned = tint(&mut r, colour, woven.tone);
    r.to(Param::BaseColor, toned);
    r.bump(woven.height, 1.2e-4);
    // A tape two and a half centimetres wide.
    preset(name, "Fabric", tags, MaterialType::RealCloth, r).previewed_as_strip(0.025)
}

/// Bonded nylon thread, as sneakers are stitched with: three plies twisted
/// together, so it is a spiral of round ridges, each ply's filaments
/// streaking along it, with a little sheen.
fn thread(name: &'static str, tags: &'static [&'static str], colour: Spectrum) -> Preset {
    let mut r = Recipe::new(base(MaterialType::RealCloth, |m| {
        m.base_color = colour;
        m.fuzz_weight = 0.15;
        m.fuzz_color = colour.lerp(Spectrum::ONE, 0.25);
        m.specular_roughness = 0.5;
    }));
    let (u, v) = strip(&mut r);
    // Three plies round, each turning once every three thread widths.
    let twist = r.math("multiply", u, 1.0);
    let phase = r.math("add", v, twist);
    let phase = r.math("multiply", phase, 3.0);
    let f = r.math("fraction", phase, 0.0);
    let centred = r.math("subtract", f, 0.5);
    let scaled = r.math("multiply", centred, 2.0 / 0.95);
    let sq = r.math("multiply", scaled, scaled);
    let rest = r.math("subtract", 1.0, sq);
    let rest = r.math("maximum", rest, 0.0);
    let ply = r.math("sqrt", rest, 0.0);
    // Filaments along each ply.
    let fibre_at = r.node("combine_xyz").w("X", phase).w("Y", u).f("Z", 0.0).out("Vector");
    let fibre_space = r.node("mapping").w("Vector", fibre_at).v("Scale", [12.0, 0.6, 1.0]).out("Vector");
    let fibre = noise(&mut r, fibre_space, 1.0, 2.0, 0.5);
    let height = plus(&mut r, fibre, 0.08, ply);
    let shade = r.range(ply, (0.0, 1.0), (0.6, 1.0));
    let streak = r.range(fibre, (0.3, 0.7), (0.95, 1.04));
    let shade = r.math("multiply", shade, streak);
    let toned = tint(&mut r, colour, shade);
    r.to(Param::BaseColor, toned);
    r.bump(height, 1e-4);
    // A thread under a millimetre thick: three millimetres round.
    preset(name, "Fabric", tags, MaterialType::RealCloth, r).previewed_as_strip(0.003)
}
