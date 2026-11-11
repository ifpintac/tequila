//! Metals with a pattern in them: stamped (diamond and tread plate,
//! perforated sheet, embossing), formed (ribbed extrusion, straight knurl),
//! and machined or hand-finished (spun, turned, face milled, Geneva
//! stripes, perlage, Clous de Paris). Sizes are the real ones, in metres.

use crate::finish::{cell, grooves_along, grooves_along_axis, length, on_plane, staggered, triplanar, turned_about_y, Hole};
use crate::{base, metal_ior, noise, plus, preset, rgb, wear, Out, Preset, Recipe};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::{MaterialType, Param};
use tequila_texture::procedural::Procedural;
use std::f32::consts::TAU;

fn metal(ty: MaterialType, ior: &str, tint: Spectrum, roughness: f32, anisotropy: f32) -> tequila_materials::openpbr::Material {
    base(ty, |m| {
        m.metal_ior = metal_ior(ior);
        m.base_color = tint;
        m.specular_roughness = roughness;
        if anisotropy > 0.0 {
            m.specular_roughness_anisotropy = anisotropy;
        }
        if ty == MaterialType::Generic {
            m.base_metalness = 1.0;
        }
    })
}

/// The sheen a rolling mill leaves in a sheet: faint streaks along X, a
/// little rougher and a touch darker in some than in others.
fn mill_streaks(r: &mut Recipe, low: f32, high: f32) -> (Out, Out) {
    let along = r.stretched([0.02, 1.0, 1.0]);
    let streak = r.node("noise_texture").w("Vector", along).f("Scale", 260.0).f("Detail", 4.0).f("Roughness", 0.6).out("Fac");
    let gloss = r.range(streak, (0.3, 0.7), (low, high));
    (streak, gloss)
}


/// Diamond (lozenge) plate: lens-shaped bars a couple of centimetres long,
/// pointed at both ends, laid in a checkerboard with each at right angles
/// to its neighbours - the non-slip floor of every van and stair. The bar
/// tops are worn brighter than the plate between them.
fn diamond_plate(name: &'static str, tags: &'static [&'static str], ior: &str, tint: Spectrum, roughness: f32) -> Preset {
    let mut r = Recipe::new(metal(MaterialType::Metal, ior, tint, roughness, 0.0));
    let pitch = 0.014;
    let height = triplanar(&mut r, 6.0, |r, u, v, _| {
        let (fu, iu) = cell(r, u, pitch, 0.5);
        let (fv, iv) = cell(r, v, pitch, 0.5);
        let sum = r.math("add", iu, iv);
        let parity = r.math("floored_modulo", sum, 2.0);
        // Along the cell one way, or the other.
        let a = r.mix_float(parity, fu, fv);
        let b = r.mix_float(parity, fv, fu);
        // A lens: pointed along it, rounded across.
        let along = r.math("absolute", a, 0.0);
        let along = r.math("divide", along, 0.43);
        let across = r.math("divide", b, 0.08);
        let across = r.math("multiply", across, across);
        let q = r.math("add", along, across);
        // Domed across its width, as a rolled bar is.
        let inside = r.math("subtract", 1.0, q);
        let inside = r.math("maximum", inside, 0.0);
        vec![r.math("sqrt", inside, 0.0)]
    })[0];
    r.bump(height, 6e-4);
    let (streak, gloss) = mill_streaks(&mut r, roughness * 0.85, roughness * 1.15);
    let worn = r.range(height, (0.0, 1.0), (1.0, 0.62));
    let gloss = r.math("multiply", gloss, worn);
    r.to(Param::SpecularRoughness, gloss);
    let shade = r.range(streak, (0.3, 0.7), (0.94, 1.03));
    let colour = r.node("mix").set("blend_type", "multiply").f("Factor", 1.0).c("A", tint).w("B", shade).out("Result");
    r.to(Param::BaseColor, colour);
    preset(name, "Metal", tags, MaterialType::Metal, r)
}

/// Five-bar tread plate: blocks of five short bars, each block at right
/// angles to the next, like a basket weave.
fn tread_plate(name: &'static str, tags: &'static [&'static str], ior: &str, tint: Spectrum, roughness: f32) -> Preset {
    let mut r = Recipe::new(metal(MaterialType::Metal, ior, tint, roughness, 0.0));
    let pitch = 0.013;
    let height = triplanar(&mut r, 6.0, |r, u, v, _| {
        let (fu, iu) = cell(r, u, pitch, 0.5);
        let (fv, iv) = cell(r, v, pitch, 0.5);
        let sum = r.math("add", iu, iv);
        let parity = r.math("floored_modulo", sum, 2.0);
        let along = r.mix_float(parity, fu, fv);
        let across = r.mix_float(parity, fv, fu);
        // Five bars across the block, each a narrow lens.
        let (bar, _) = cell(r, across, 0.17, 0.5);
        let ends = r.math("absolute", along, 0.0);
        let ends = r.math("divide", ends, 0.4);
        let side = r.math("divide", bar, 0.3);
        let side = r.math("multiply", side, side);
        let q = r.math("add", ends, side);
        let lens = r.smooth(q, (0.35, 1.0), (1.0, 0.0));
        // Only the five in the middle of the block.
        let off = r.math("absolute", across, 0.0);
        let inside = r.smooth(off, (0.41, 0.43), (1.0, 0.0));
        vec![r.math("multiply", lens, inside)]
    })[0];
    r.bump(height, 4.5e-4);
    let (_, gloss) = mill_streaks(&mut r, roughness * 0.85, roughness * 1.15);
    let worn = r.range(height, (0.0, 1.0), (1.0, 0.6));
    let gloss = r.math("multiply", gloss, worn);
    r.to(Param::SpecularRoughness, gloss);
    preset(name, "Metal", tags, MaterialType::Metal, r)
}

/// Perforated sheet: holes punched on a staggered pitch, cut right through
/// (an opacity), each hole's edge rolled a little by the punch.
#[allow(clippy::too_many_arguments)]
fn perforated(name: &'static str, tags: &'static [&'static str], ior: &str, tint: Spectrum, roughness: f32, pitch: f32, radius: f32, hole: Hole) -> Preset {
    let mut r = Recipe::new(metal(MaterialType::Generic, ior, tint, roughness, 0.0));
    let blended = triplanar(&mut r, 8.0, |r, u, v, _| {
        let d = staggered(r, u, v, pitch, hole);
        let solid = r.smooth(d, (radius - pitch * 0.015, radius + pitch * 0.015), (0.0, 1.0));
        let rim = r.smooth(d, (radius, radius + pitch * 0.12), (0.0, 1.0));
        vec![solid, rim]
    });
    r.to(Param::Opacity, blended[0]);
    r.bump(blended[1], 1.5e-4);
    let (_, gloss) = mill_streaks(&mut r, roughness * 0.85, roughness * 1.2);
    r.to(Param::SpecularRoughness, gloss);
    preset(name, "Metal", tags, MaterialType::Generic, r)
}

/// The same sheet coated: a speaker's or a vent's grille, punched and then
/// powder coated, the coat a little thicker round each hole.
pub fn perforated_coated(name: &'static str, tags: &'static [&'static str], colour: Spectrum, roughness: f32, pitch: f32, radius: f32, hole: Hole) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = colour;
        m.base_metalness = 0.0;
        m.specular_roughness = roughness;
    }));
    let blended = triplanar(&mut r, 8.0, |r, u, v, _| {
        let d = staggered(r, u, v, pitch, hole);
        let solid = r.smooth(d, (radius - pitch * 0.015, radius + pitch * 0.015), (0.0, 1.0));
        let rim = r.smooth(d, (radius, radius + pitch * 0.12), (0.0, 1.0));
        vec![solid, rim]
    });
    r.to(Param::Opacity, blended[0]);
    r.bump(blended[1], 1.2e-4);
    let gloss = wear(&mut r, roughness * 0.85, roughness * 1.15, 40.0);
    r.to(Param::SpecularRoughness, gloss);
    preset(name, "Metal", tags, MaterialType::Generic, r)
}

/// A sheet as a chassis is folded from: the rolling mill's faint streaks
/// under whatever it was plated with, `gloss` from the most polished
/// streak to the dullest.
pub fn sheet(name: &'static str, tags: &'static [&'static str], ior: &str, tint: Spectrum, gloss: (f32, f32)) -> Preset {
    let mut r = Recipe::new(metal(MaterialType::Metal, ior, tint, (gloss.0 + gloss.1) * 0.5, 0.0));
    r.material_mut().edge_radius = 1e-4;
    let (streak, sheen) = mill_streaks(&mut r, gloss.0, gloss.1);
    r.to(Param::SpecularRoughness, sheen);
    let shade = r.range(streak, (0.3, 0.7), (0.93, 1.0));
    let toned = r.node("mix").set("blend_type", "multiply").f("Factor", 1.0).c("A", tint).w("B", shade).out("Result");
    r.to(Param::BaseColor, toned);
    let tooth = noise(&mut r, 12000.0, 2.0, 0.5);
    r.bump(tooth, 8e-7);
    preset(name, "Metal", tags, MaterialType::Metal, r)
}

/// Aluminium sheet with a chromate conversion coat: a thin film in straws
/// and golds that never comes out one colour, `stops` its colours.
pub fn chromated(name: &'static str, tags: &'static [&'static str], stops: &[(f32, Spectrum)]) -> Preset {
    let mut r = Recipe::new(metal(MaterialType::Metal, "Aluminium", stops[0].1, 0.36, 0.0));
    r.material_mut().edge_radius = 1e-4;
    let patches = r.node("noise_texture").f("Scale", 28.0).f("Detail", 3.0).f("Roughness", 0.5).f("Distortion", 0.8).out("Fac");
    let colour = r.ramp(patches, crate::recipe::Linear, stops);
    r.to(Param::BaseColor, colour);
    let (_, sheen) = mill_streaks(&mut r, 0.3, 0.42);
    r.to(Param::SpecularRoughness, sheen);
    let tooth = noise(&mut r, 12000.0, 2.0, 0.5);
    r.bump(tooth, 8e-7);
    preset(name, "Metal", tags, MaterialType::Metal, r)
}

/// Where the point is from the nearest centre of each of a staggered
/// pattern's two lattices, `pu` by `pv` and the second half a step along
/// both: across and up, in metres.
fn staggered_cells(r: &mut Recipe, u: Out, v: Out, pu: f32, pv: f32) -> [(Out, Out); 2] {
    [(0.0, 0.0), (0.5, 0.5)].map(|(du, dv)| {
        let (cu, _) = cell(r, u, pu, du);
        let (cv, _) = cell(r, v, pv, dv);
        (r.math("multiply", cu, pu), r.math("multiply", cv, pv))
    })
}

/// A coated sheet with openings cut in it: open where `distance`, which
/// is given the plane's u and v, is under `radius`, the coat a little
/// thicker round each opening over `rim` metres.
fn cut_coated(name: &'static str, tags: &'static [&'static str], colour: Spectrum, roughness: f32, radius: f32, rim: f32, distance: impl Fn(&mut Recipe, Out, Out) -> Out) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = colour;
        m.base_metalness = 0.0;
        m.specular_roughness = roughness;
    }));
    let blended = triplanar(&mut r, 8.0, |r, u, v, _| {
        let d = distance(r, u, v);
        let solid = r.smooth(d, (radius - rim * 0.12, radius + rim * 0.12), (0.0, 1.0));
        let raised = r.smooth(d, (radius, radius + rim), (0.0, 1.0));
        vec![solid, raised]
    });
    r.to(Param::Opacity, blended[0]);
    r.bump(blended[1], 1.2e-4);
    let gloss = wear(&mut r, roughness * 0.85, roughness * 1.15, 40.0);
    r.to(Param::SpecularRoughness, gloss);
    preset(name, "Metal", tags, MaterialType::Generic, r)
}

/// A vent of slots: round-ended, `length` by `width`, `along` from one's
/// middle to the next in its row and the rows `across` apart, every
/// other row half a step along.
pub fn slotted_coated(name: &'static str, tags: &'static [&'static str], colour: Spectrum, roughness: f32, length: f32, width: f32, along: f32, across: f32) -> Preset {
    cut_coated(name, tags, colour, roughness, width * 0.5, width * 0.25, move |r, u, v| {
        let mut nearest = None;
        for (au, av) in staggered_cells(r, u, v, along, across * 2.0) {
            let a = r.math("absolute", au, 0.0);
            let past = r.math("subtract", a, (length - width) * 0.5);
            let past = r.math("maximum", past, 0.0);
            let d = crate::finish::length(r, past, av);
            nearest = Some(match nearest {
                None => d,
                Some(n) => r.math("minimum", n, d),
            });
        }
        nearest.expect("two lattices")
    })
}

/// Expanded metal: sheet slit and pulled into diamonds `long` by `short`
/// from middle to middle, `strand` of each left as metal.
pub fn expanded_coated(name: &'static str, tags: &'static [&'static str], colour: Spectrum, roughness: f32, long: f32, short: f32, strand: f32) -> Preset {
    cut_coated(name, tags, colour, roughness, (1.0 - strand) * short * 0.5, short * 0.06, move |r, u, v| {
        let mut nearest = None;
        for (au, av) in staggered_cells(r, u, v, long, short) {
            let a = r.math("absolute", au, 0.0);
            let b = r.math("absolute", av, 0.0);
            // A diamond's own measure, in metres up its short way.
            let d = r.math3("multiply_add", a, short / long, b);
            nearest = Some(match nearest {
                None => d,
                Some(n) => r.math("minimum", n, d),
            });
        }
        nearest.expect("two lattices")
    })
}

/// Woven wire mesh: round wires `wire` thick, `pitch` apart both ways,
/// each over one and under the next, and nothing between them.
pub fn wire_mesh(name: &'static str, tags: &'static [&'static str], material: tequila_materials::openpbr::Material, pitch: f32, wire: f32) -> Preset {
    let mut r = Recipe::new(material);
    let half = wire * 0.5 / pitch;
    let blended = triplanar(&mut r, 8.0, |r, u, v, _| {
        let (fu, iu) = cell(r, u, pitch, 0.0);
        let (fv, iv) = cell(r, v, pitch, 0.0);
        let column = r.math("add", iu, iv);
        let mut wires = Vec::new();
        // The wire running up, at the middle of its cell across, and the
        // wire running across: `off` is how far from the wire's middle,
        // `along` how far along it, and one rises where the other falls.
        for (off, along, sign) in [(fu, fv, 1.0), (fv, fu, -1.0)] {
            let away = r.math("absolute", off, 0.0);
            let on = r.smooth(away, (half * 0.85, half * 1.15), (1.0, 0.0));
            let t = r.math("divide", away, half);
            let tt = r.math("multiply", t, t);
            let round = r.math("subtract", 1.0, tt);
            let round = r.math("maximum", round, 0.0);
            let round = r.math("sqrt", round, 0.0);
            let turn = r.math("add", along, column);
            let turn = r.math("multiply", turn, std::f32::consts::PI);
            let wave = r.math("cosine", turn, 0.0);
            let lift = r.math3("multiply_add", wave, 0.25 * sign, 0.25);
            let height = r.math3("multiply_add", round, 0.5, lift);
            wires.push((on, r.math("multiply", height, on)));
        }
        let solid = r.math("maximum", wires[0].0, wires[1].0);
        let height = r.math("maximum", wires[0].1, wires[1].1);
        vec![solid, height]
    });
    r.to(Param::Opacity, blended[0]);
    r.bump(blended[1], wire);
    preset(name, "Metal", tags, MaterialType::Generic, r)
}

/// The metal of a [`wire_mesh`], plated.
pub fn plated_wire(ior: &str, tint: Spectrum, roughness: f32) -> tequila_materials::openpbr::Material {
    metal(MaterialType::Generic, ior, tint, roughness, 0.0)
}

/// An extruded profile: rounded ribs `pitch` apart down its length, and the
/// die lines the extrusion drew along it.
fn ribbed(name: &'static str, tags: &'static [&'static str], tint: Spectrum, roughness: f32, pitch: f32) -> Preset {
    let mut r = Recipe::new(metal(MaterialType::Anisotropic, "Aluminium", tint, roughness, 0.55));
    let coords = r.node("texture_coordinate").out("Object");
    let x = r.node("separate_xyz").w("Vector", coords).out("X");
    let (f, _) = cell(&mut r, x, pitch, 0.5);
    let f2 = r.math("multiply", f, 2.0);
    let sq = r.math("multiply", f2, f2);
    let rib = r.math("subtract", 1.0, sq);
    let rib = r.math("maximum", rib, 0.0);
    let rib = r.math("sqrt", rib, 0.0);
    // Die lines, along the extrusion: up the part.
    let along = r.stretched([1.0, 1.0 / 60.0, 1.0]);
    let lines = r.node("noise_texture").w("Vector", along).f("Scale", 2200.0).f("Detail", 2.0).out("Fac");
    let height = plus(&mut r, lines, 0.004, rib);
    r.bump(height, pitch * 0.18);
    let gloss = r.range(lines, (0.3, 0.7), (roughness * 0.85, roughness * 1.15));
    r.to(Param::SpecularRoughness, gloss);
    grooves_along_axis(&mut r, [0.0, 1.0, 0.0]);
    preset(name, "Metal", tags, MaterialType::Anisotropic, r)
}

/// Straight knurling: sharp ridges along the part, a fraction of a
/// millimetre apart.
fn knurled_straight(name: &'static str, tags: &'static [&'static str], ior: &str, tint: Spectrum, pitch: f32) -> Preset {
    let mut r = Recipe::new(metal(MaterialType::Metal, ior, tint, 0.2, 0.0));
    let coords = r.node("texture_coordinate").out("Object");
    let x = r.node("separate_xyz").w("Vector", coords).out("X");
    let (f, _) = cell(&mut r, x, pitch, 0.5);
    let off = r.math("absolute", f, 0.0);
    let ridge = r.math3("multiply_add", off, -2.0, 1.0);
    r.bump(ridge, pitch * 0.3);
    let gloss = r.range(ridge, (0.0, 1.0), (0.32, 0.14));
    r.to(Param::SpecularRoughness, gloss);
    preset(name, "Metal", tags, MaterialType::Metal, r)
}

/// Spun: a disc turned against a polishing tool, fine grooves in rings
/// about its axis - the face of a speaker cone, a knob, a watch dial - and
/// the radial bow-tie of light they throw.
pub fn spun(name: &'static str, tags: &'static [&'static str], ior: &str, tint: Spectrum, roughness: f32) -> Preset {
    let mut r = Recipe::new(metal(MaterialType::Anisotropic, ior, tint, roughness, 0.97));
    let rings = crate::pattern(&mut r, Procedural::BrushedRadial { axis: tequila_texture::projection::Axis::Y, frequency: 2600.0, stretch: 90.0 }, 5);
    r.bump(rings, 9e-7);
    let gloss = r.range(rings, (0.3, 0.7), (roughness * 0.85, roughness * 1.15));
    r.to(Param::SpecularRoughness, gloss);
    turned_about_y(&mut r);
    preset(name, "Metal", tags, MaterialType::Anisotropic, r)
}

/// Turned on a lathe: the tool's feed marks, rings `feed` apart - round the
/// side of the part, and across its face from the centre out.
pub fn turned(name: &'static str, tags: &'static [&'static str], ior: &str, tint: Spectrum, roughness: f32, feed: f32) -> Preset {
    let mut r = Recipe::new(metal(MaterialType::Anisotropic, ior, tint, roughness, 0.95));
    let coords = r.node("texture_coordinate").out("Object");
    let normal = r.node("texture_coordinate").out("Normal");
    let p = r.node("separate_xyz").w("Vector", coords).add();
    let n = r.node("separate_xyz").w("Vector", normal).add();
    let radius = length(&mut r, Out(p, 0), Out(p, 2));
    let face = r.math("absolute", Out(n, 1), 0.0);
    let face = r.math("power", face, 4.0);
    let travel = r.mix_float(face, Out(p, 1), radius);
    let phase = r.math("multiply", travel, TAU / feed);
    let marks = r.math("sine", phase, 0.0);
    let marks = r.math3("multiply_add", marks, 0.5, 0.5);
    let marks = r.math("power", marks, 3.0);
    r.bump(marks, feed * 0.012);
    let gloss = wear(&mut r, roughness * 0.9, roughness * 1.1, 30.0);
    r.to(Param::SpecularRoughness, gloss);
    turned_about_y(&mut r);
    preset(name, "Metal", tags, MaterialType::Anisotropic, r)
}

/// Côtes de Genève: stripes a few millimetres wide, each finished by a
/// turning disc drawn along it, so each is a band of curved grooves - and
/// a band of light that bends across it - side by side down a watch
/// movement's plates.
fn geneva(name: &'static str, tags: &'static [&'static str], ior: &str, tint: Spectrum, roughness: f32) -> Preset {
    let mut r = Recipe::new(metal(MaterialType::Anisotropic, ior, tint, roughness, 0.97));
    let width = 0.0036;
    let tool = width * 0.78;
    let out = triplanar(&mut r, 8.0, |r, u, v, plane| {
        let (dv, _) = cell(r, v, width, 0.5);
        let dy = r.math("multiply", dv, width);
        // Where the disc last passed over: its edge, ahead of the point.
        let dyy = r.math("multiply", dy, dy);
        let rest = r.math("subtract", tool * tool, dyy);
        let rest = r.math("maximum", rest, 0.0);
        let s = r.math("sqrt", rest, 0.0);
        let [gx, gy, gz] = on_plane(r, plane, dy, s);
        // The disc's own fine grooves, and a hair of a step between stripes.
        let lead = r.math("add", u, s);
        let phase = r.math("multiply", lead, TAU / 0.00006);
        let fine = r.math("sine", phase, 0.0);
        let edge = r.math("absolute", dv, 0.0);
        let step = r.smooth(edge, (0.46, 0.5), (0.0, -1.0));
        let height = plus(r, fine, 0.02, step);
        vec![height, gx, gy, gz]
    });
    r.bump(out[0], 1.2e-5);
    let groove = r.node("combine_xyz").w("X", out[1]).w("Y", out[2]).w("Z", out[3]).out("Vector");
    grooves_along(&mut r, groove);
    preset(name, "Metal", tags, MaterialType::Anisotropic, r)
}

/// Perlage: overlapping spots, each ground in circles by the end of a
/// turning peg, laid in rows across a plate like fish scales - every spot
/// with its own little cross of light.
fn perlage(name: &'static str, tags: &'static [&'static str], ior: &str, tint: Spectrum, roughness: f32) -> Preset {
    let mut r = Recipe::new(metal(MaterialType::Anisotropic, ior, tint, roughness, 0.97));
    let spot = 0.0009;
    let step = spot * 0.72;
    let out = triplanar(&mut r, 8.0, |r, u, v, plane| {
        // The last row of spots that could reach this point, and the one
        // before it: the later one wins where it covers.
        let reach = r.math("add", v, spot);
        let reach = r.math("divide", reach, step);
        let row = r.math("floor", reach, 0.0);
        let mut picks = Vec::new();
        for back in [0.0, 1.0] {
            let j = r.math("subtract", row, back);
            let cy = r.math("multiply", j, step);
            let dy = r.math("subtract", v, cy);
            let dyy = r.math("multiply", dy, dy);
            let rest = r.math("subtract", spot * spot, dyy);
            let rest = r.math("maximum", rest, 0.0);
            let half = r.math("sqrt", rest, 0.0);
            let lead = r.math("add", u, half);
            let lead = r.math("divide", lead, step);
            let i = r.math("floor", lead, 0.0);
            let cx = r.math("multiply", i, step);
            let dx = r.math("subtract", u, cx);
            let d = length(r, dx, dy);
            let covered = r.math("less_than", d, spot);
            picks.push((covered, dx, dy, d));
        }
        let (covered, dx0, dy0, d0) = picks[0];
        let (_, dx1, dy1, d1) = picks[1];
        let dx = r.mix_float(covered, dx1, dx0);
        let dy = r.mix_float(covered, dy1, dy0);
        let d = r.mix_float(covered, d1, d0);
        // Round the spot's centre.
        let gu = r.math("multiply", dy, -1.0);
        let [gx, gy, gz] = on_plane(r, plane, gu, dx);
        let phase = r.math("multiply", d, TAU / 0.00004);
        let rings = r.math("sine", phase, 0.0);
        // Each spot ground a hair deeper at its middle.
        let dish = r.math("divide", d, spot);
        let height = plus(r, rings, 0.04, dish);
        vec![height, gx, gy, gz]
    });
    r.bump(out[0], 8e-6);
    let groove = r.node("combine_xyz").w("X", out[1]).w("Y", out[2]).w("Z", out[3]).out("Vector");
    grooves_along(&mut r, groove);
    preset(name, "Metal", tags, MaterialType::Anisotropic, r)
}

/// Clous de Paris: a grid of tiny square pyramids cut in a polished metal,
/// the hobnail of a dress watch's dial.
fn clous_de_paris(name: &'static str, tags: &'static [&'static str], ior: &str, tint: Spectrum, pitch: f32) -> Preset {
    let mut r = Recipe::new(metal(MaterialType::Metal, ior, tint, 0.05, 0.0));
    let height = triplanar(&mut r, 8.0, |r, u, v, _| {
        let (fu, _) = cell(r, u, pitch, 0.5);
        let (fv, _) = cell(r, v, pitch, 0.5);
        let a = r.math("absolute", fu, 0.0);
        let b = r.math("absolute", fv, 0.0);
        let m = r.math("maximum", a, b);
        vec![r.math3("multiply_add", m, -2.0, 1.0)]
    })[0];
    r.bump(height, pitch * 0.32);
    let gloss = r.range(height, (0.0, 0.15), (0.12, 0.05));
    r.to(Param::SpecularRoughness, gloss);
    preset(name, "Metal", tags, MaterialType::Metal, r)
}

/// Face milled: a cutter swept across a block in passes, leaving overlapping
/// arcs of its feed marks - the machined look of a billet part.
fn face_milled(name: &'static str, tags: &'static [&'static str], ior: &str, tint: Spectrum, roughness: f32) -> Preset {
    let mut r = Recipe::new(metal(MaterialType::Anisotropic, ior, tint, roughness, 0.95));
    let cutter = 0.008;
    let pass = cutter * 1.6;
    let out = triplanar(&mut r, 8.0, |r, u, v, plane| {
        let (dv, _) = cell(r, v, pass, 0.5);
        let dy = r.math("multiply", dv, pass);
        let dyy = r.math("multiply", dy, dy);
        let rest = r.math("subtract", cutter * cutter, dyy);
        let rest = r.math("maximum", rest, 0.0);
        let s = r.math("sqrt", rest, 0.0);
        // The arc through this point: centred `s` behind it on the pass.
        let centre = r.math("subtract", u, s);
        let gu = r.math("multiply", dy, -1.0);
        let [gx, gy, gz] = on_plane(r, plane, gu, s);
        let phase = r.math("multiply", centre, TAU / 0.0005);
        let marks = r.math("sine", phase, 0.0);
        let marks = r.math3("multiply_add", marks, 0.5, 0.5);
        vec![marks, gx, gy, gz]
    });
    r.bump(out[0], 6e-6);
    let gloss = wear(&mut r, roughness * 0.9, roughness * 1.12, 35.0);
    r.to(Param::SpecularRoughness, gloss);
    let groove = r.node("combine_xyz").w("X", out[1]).w("Y", out[2]).w("Z", out[3]).out("Vector");
    grooves_along(&mut r, groove);
    preset(name, "Metal", tags, MaterialType::Anisotropic, r)
}

/// Stucco embossed: a sheet rolled between dimpled rollers, soft rounded
/// bumps a few millimetres across, as on appliance panels and insulation
/// cladding.
fn stucco(name: &'static str, tags: &'static [&'static str], tint: Spectrum, roughness: f32) -> Preset {
    let mut r = Recipe::new(metal(MaterialType::Metal, "Aluminium", tint, roughness, 0.0));
    let cells = r.node("voronoi_texture").set("feature", "smooth_f1").f("Scale", 450.0).f("Smoothness", 1.0).f("Randomness", 1.0).out("Distance");
    let dome = r.math("subtract", 1.0, cells);
    let lumps = noise(&mut r, 900.0, 2.0, 0.5);
    let height = plus(&mut r, lumps, 0.25, dome);
    r.bump(height, 2.5e-4);
    let gloss = r.range(dome, (0.2, 0.9), (roughness * 1.15, roughness * 0.85));
    r.to(Param::SpecularRoughness, gloss);
    preset(name, "Metal", tags, MaterialType::Metal, r)
}

pub fn all() -> Vec<Preset> {
    let white = Spectrum::ONE;
    let aluminium = rgb(0.9734, 0.9734, 0.9734);
    let steel = rgb(0.86, 0.865, 0.87);
    vec![
        diamond_plate("Aluminium Diamond Plate", &["aluminium", "diamond plate", "checker plate", "tread", "industrial", "floor"], "Aluminium", aluminium, 0.3),
        diamond_plate("Steel Diamond Plate", &["steel", "diamond plate", "checker plate", "tread", "industrial", "floor"], "Iron", rgb(0.52, 0.53, 0.55), 0.4),
        tread_plate("Aluminium Tread Plate", &["aluminium", "five bar", "tread plate", "industrial", "floor"], "Aluminium", aluminium, 0.32),
        perforated("Steel Perforated Round", &["steel", "perforated", "holes", "grille", "sheet"], "Nickel", steel, 0.3, 0.005, 0.0016, Hole::Round),
        perforated("Aluminium Perforated Hex", &["aluminium", "perforated", "hexagon", "speaker", "grille"], "Aluminium", aluminium, 0.32, 0.004, 0.0016, Hole::Hex),
        perforated("Steel Perforated Square", &["steel", "perforated", "square", "grille", "sheet"], "Nickel", steel, 0.3, 0.006, 0.002, Hole::Square),
        ribbed("Aluminium Ribbed Extrusion", &["aluminium", "ribbed", "fluted", "extruded", "heatsink"], aluminium, 0.26, 0.0025),
        knurled_straight("Aluminium Knurled Straight", &["aluminium", "knurled", "straight", "grip", "machined"], "Aluminium", aluminium, 0.0008),
        spun("Aluminium Spun", &["aluminium", "spun", "radial", "concentric", "speaker", "dial"], "Aluminium", aluminium, 0.34),
        spun("Stainless Spun", &["steel", "stainless", "spun", "radial", "concentric"], "Nickel", rgb(0.88, 0.89, 0.9), 0.3),
        turned("Steel Turned", &["steel", "turned", "lathe", "machined", "rings"], "Iron", rgb(0.78, 0.79, 0.8), 0.32, 0.00025),
        geneva("Steel Geneva Stripes", &["steel", "geneva stripes", "cotes de geneve", "watch", "movement"], "Nickel", rgb(0.92, 0.925, 0.93), 0.28),
        geneva("Gold Geneva Stripes", &["gold", "geneva stripes", "cotes de geneve", "watch", "movement"], "Gold", white, 0.28),
        perlage("Steel Perlage", &["steel", "perlage", "circular graining", "watch", "engine turned"], "Nickel", rgb(0.92, 0.925, 0.93), 0.28),
        clous_de_paris("Silver Clous de Paris", &["silver", "clous de paris", "hobnail", "guilloche", "watch dial"], "Silver", white, 0.0006),
        face_milled("Aluminium Face Milled", &["aluminium", "milled", "machined", "billet", "fly cut"], "Aluminium", aluminium, 0.3),
        stucco("Aluminium Stucco Embossed", &["aluminium", "stucco", "embossed", "appliance", "cladding"], rgb(0.8, 0.8, 0.79), 0.34),
    ]
}
