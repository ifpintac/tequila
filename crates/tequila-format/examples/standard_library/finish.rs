//! What the finishes share: patterns laid on any part from its three
//! planes, the directions brushing, turning and milling leave in a metal,
//! and the lattices that stampings and engravings repeat on.

use crate::{Out, Recipe};
use tequila_materials::material_type::Param;

/// A flat pattern drawn on each of the part's three planes and blended by
/// how squarely the surface faces each, so it lies on a part of any shape
/// without UVs. `draw` gets the plane's (u, v) in metres - u across and v
/// up on the planes facing X and Z, and (x, z) on the one facing Y - and
/// the plane's index (0 facing X, 1 facing Y, 2 facing Z), and returns
/// numbers; each comes back blended. `sharp` is how sharply the planes hand
/// over: higher keeps a pattern from doubling where two meet.
pub fn triplanar(r: &mut Recipe, sharp: f32, mut draw: impl FnMut(&mut Recipe, Out, Out, usize) -> Vec<Out>) -> Vec<Out> {
    let coords = r.node("texture_coordinate").out("Object");
    let normal = r.node("texture_coordinate").out("Normal");
    let xyz = r.node("separate_xyz").w("Vector", coords).add();
    let nxyz = r.node("separate_xyz").w("Vector", normal).add();
    let mut weights = Vec::new();
    for axis in 0..3u8 {
        let a = r.math("absolute", Out(nxyz, axis), 0.0);
        weights.push(r.math("power", a, sharp));
    }
    let sum = r.math("add", weights[0], weights[1]);
    let sum = r.math("add", sum, weights[2]);
    let weights: Vec<Out> = weights.into_iter().map(|w| r.math("divide", w, sum)).collect();
    let planes = [(Out(xyz, 2), Out(xyz, 1)), (Out(xyz, 0), Out(xyz, 2)), (Out(xyz, 0), Out(xyz, 1))];
    let mut blended: Vec<Out> = Vec::new();
    for (i, (u, v)) in planes.into_iter().enumerate() {
        let outs = draw(r, u, v, i);
        for (k, out) in outs.into_iter().enumerate() {
            let weighed = r.math("multiply", out, weights[i]);
            if k < blended.len() {
                blended[k] = r.math("add", blended[k], weighed);
            } else {
                blended.push(weighed);
            }
        }
    }
    blended
}

/// A direction on plane `plane` of [`triplanar`], (gu, gv) along its u and
/// v, as the part's x, y and z: for drawing a groove's direction there.
pub fn on_plane(r: &mut Recipe, plane: usize, gu: Out, gv: Out) -> [Out; 3] {
    let zero = r.math("multiply", gu, 0.0);
    match plane {
        0 => [zero, gv, gu],
        1 => [gu, zero, gv],
        _ => [gu, gv, zero],
    }
}

/// Grooves running along `groove`, a direction in the part's space: the
/// metal is rougher across them than along them, so its lobe's tangent is
/// the normal crossed with the groove. A highlight on brushed metal
/// stretches across the brushing, as it does here.
pub fn grooves_along(r: &mut Recipe, groove: Out) {
    let normal = r.node("texture_coordinate").out("Normal");
    let across = r.node("vector_math").set("operation", "cross_product").w("Vector", normal).w("Vector 2", groove).out("Vector");
    r.to(Param::Tangent, across);
}

/// [`grooves_along`] one fixed axis of the part.
pub fn grooves_along_axis(r: &mut Recipe, axis: [f32; 3]) {
    let normal = r.node("texture_coordinate").out("Normal");
    let across = r.node("vector_math").set("operation", "cross_product").w("Vector", normal).v("Vector 2", axis).out("Vector");
    r.to(Param::Tangent, across);
}

/// Grooves in circles about the part's Y axis, as a lathe or a spinning
/// disc leaves them: on the face, concentric rings; down the side, rings
/// round it.
pub fn turned_about_y(r: &mut Recipe) {
    let at = r.node("texture_coordinate").out("Object");
    let around = r.node("vector_math").set("operation", "cross_product").v("Vector", [0.0, 1.0, 0.0]).w("Vector 2", at).out("Vector");
    grooves_along(r, around);
}

/// The groove a brush leaves: fine streaks of varied length running along
/// the part's X, `across` of them a metre, `length` times as long as they
/// are wide. A height, about 0 to 1.
pub fn brush_lines(r: &mut Recipe, across: f32, length: f32) -> Out {
    let along = r.stretched([1.0 / length, 1.0, 1.0]);
    let fine = r.node("noise_texture").w("Vector", along).f("Scale", across).f("Detail", 3.0).f("Roughness", 0.62).out("Fac");
    let coarse = r.node("noise_texture").w("Vector", along).f("Scale", across * 0.18).f("Detail", 2.0).f("Roughness", 0.5).out("Fac");
    crate::plus(r, coarse, 0.45, fine)
}

/// The nearest point of a square lattice `pitch` apart, along one axis:
/// where `t` is in its cell, -0.5 to 0.5 of the pitch, and the cell's index.
pub fn cell(r: &mut Recipe, t: impl Into<crate::recipe::In>, pitch: f32, offset: f32) -> (Out, Out) {
    let scaled = r.math("divide", t, pitch);
    let shifted = r.math("add", scaled, 0.5 - offset);
    let index = r.math("floor", shifted, 0.0);
    let fraction = r.math("fraction", shifted, 0.0);
    let local = r.math("subtract", fraction, 0.5);
    (local, index)
}

/// The length of (a, b).
pub fn length(r: &mut Recipe, a: Out, b: Out) -> Out {
    let aa = r.math("multiply", a, a);
    let bb = r.math("multiply", b, b);
    let sum = r.math("add", aa, bb);
    r.math("sqrt", sum, 0.0)
}

/// Distance to the nearest centre of a staggered lattice - rows `pitch`
/// apart along u, every other row moved half a pitch, rows close enough
/// that every centre has six neighbours at `pitch` - in metres, by the
/// shape `metric` gives a hole: round, square or hexagonal.
pub fn staggered(r: &mut Recipe, u: Out, v: Out, pitch: f32, metric: Hole) -> Out {
    let row = pitch * 0.866_025_4;
    let mut nearest = None;
    for (du, dv) in [(0.0, 0.0), (0.5, 0.5)] {
        let (cu, _) = cell(r, u, pitch, du);
        let (cv, _) = cell(r, v, row * 2.0, dv);
        let au = r.math("multiply", cu, pitch);
        let av = r.math("multiply", cv, row * 2.0);
        let d = match metric {
            Hole::Round => length(r, au, av),
            Hole::Square => {
                let a = r.math("absolute", au, 0.0);
                let b = r.math("absolute", av, 0.0);
                r.math("maximum", a, b)
            }
            Hole::Hex => {
                let a = r.math("absolute", au, 0.0);
                let b = r.math("absolute", av, 0.0);
                let slant = r.math("multiply", b, 0.5);
                let slant = r.math3("multiply_add", a, 0.866_025_4, slant);
                // Measured to a flat: a hexagon's inradius is its size.
                r.math("maximum", slant, b)
            }
        };
        nearest = Some(match nearest {
            None => d,
            Some(n) => r.math("minimum", n, d),
        });
    }
    nearest.expect("two lattices")
}

#[derive(Clone, Copy)]
pub enum Hole {
    Round,
    Square,
    Hex,
}
