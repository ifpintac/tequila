//! Where a label lands when it is first placed.
//!
//! The Labels section shipped requiring UV coordinates, and the handoff note
//! recorded planar placement and automatic fitting as future work. That leaves
//! the feature complete and unusable for the formats this project actually
//! imports: an STL carries positions and nothing else, and most exported OBJ
//! meshes have no UV set either. A label control that does nothing on the file
//! the user opened is indistinguishable, from the outside, from a broken one.
//!
//! So a label with no UVs to sit on is projected instead, flat onto the part's
//! widest face, sized and centred to the part. §5.3 already has the projection;
//! what was missing was anything to choose a sensible frame for it, which is
//! all this module does.
//!
//! ## What "fitted" has to mean
//!
//! Three things, and leaving out any one of them puts the artwork somewhere the
//! user then has to hunt for:
//!
//! 1. **A face.** Projected down the part's *shortest* axis, so the artwork
//!    lands on the largest face rather than edge-on to it. On a phone back or a
//!    packaging panel that is the face anyone would have chosen.
//! 2. **A size.** In scene units, not repeats — a repeat count means the same
//!    label is a different size on every part it is applied to. Sized to a
//!    fraction of the face so it reads as placed artwork rather than as a
//!    full-surface wrap.
//! 3. **A centre.** Both the offset *and* the pivot, so the artwork sits in the
//!    middle of the face and the rotation control turns it where it sits. A
//!    part modelled away from the origin — which is most imported geometry — is
//!    where a pivot left at zero becomes obvious, because the label swings away
//!    across the surface as soon as it is rotated.

use crate::document::SceneDocument;
use crate::tree::{MaterialKey, NodeId, NodeKind};
use tequila_core::math::{vec2, vec3, Aabb, Vec3};
use tequila_texture::projection::{Axis, Mapping, Projection};

/// How much of the chosen face a freshly placed label covers, across its
/// narrower side. Big enough to be visible without hunting, small enough to
/// read as a label rather than as a wrap.
const COVERAGE: f32 = 0.45;

/// The part a label placed on `material` should be fitted to.
///
/// The first part in the tree using the material, by node id, so the result
/// does not depend on iteration order. A material shared across several parts
/// has no single answer — the parts have different object spaces, and a box
/// unioned with the bracket beside it describes neither — and the part the
/// user had selected when they reached for the button is almost always the
/// first one anyway.
fn part_for(document: &SceneDocument, material: MaterialKey) -> Option<(NodeId, Aabb, bool)> {
    let mut best: Option<(NodeId, Aabb, bool)> = None;
    for node in document.tree.iter() {
        if node.material != Some(material) {
            continue;
        }
        let NodeKind::Part { mesh } = node.kind else {
            continue;
        };
        let Some(mesh) = document.geometry.get(mesh) else {
            continue;
        };
        let has_uvs = mesh
            .uv_sets
            .first()
            .is_some_and(|uvs| uvs.len() == mesh.positions.len() && !uvs.is_empty());
        if best.as_ref().is_none_or(|(id, _, _)| node.id.0 < id.0) {
            best = Some((node.id, mesh.bounds(), has_uvs));
        }
    }
    best
}

/// The part a label on this material is placed against: its node, so the
/// viewport can find its transform, and its object-space bounds.
pub fn part_of(document: &SceneDocument, material: MaterialKey) -> Option<(NodeId, Aabb)> {
    part_for(document, material).map(|(node, bounds, _)| (node, bounds))
}

/// The mapping a newly placed label should start with.
///
/// `None` when the part has usable UV coordinates, or when there is no part to
/// measure: the existing UV placement is then left exactly as it was, so a
/// scene that worked before this existed behaves identically.
pub fn fit(document: &SceneDocument, material: MaterialKey) -> Option<Mapping> {
    let (_, bounds, has_uvs) = part_for(document, material)?;
    if has_uvs {
        return None;
    }
    fit_to_bounds(bounds)
}

/// A projected placement fitted to the part, whether or not it has UVs.
///
/// What the "Fit to part" button and a switch to projected placement both
/// call. [`fit`] declines on a UV-mapped part because overriding an authored
/// layout unasked would be wrong; asking for it explicitly is not unasked.
pub fn fit_projected(document: &SceneDocument, material: MaterialKey) -> Option<Mapping> {
    let (_, bounds, _) = part_for(document, material)?;
    fit_to_bounds(bounds)
}

/// Whether the part wearing this material has UV coordinates to place a label
/// on — so the UI can say why UV placement is not offered rather than showing
/// a control that quietly does nothing.
pub fn has_uvs(document: &SceneDocument, material: MaterialKey) -> bool {
    part_for(document, material).is_some_and(|(_, _, has_uvs)| has_uvs)
}

/// The same fit, from bounds alone — the part of this that is arithmetic
/// rather than document lookup, and the part worth testing directly.
pub fn fit_to_bounds(bounds: Aabb) -> Option<Mapping> {
    let extent = bounds.max - bounds.min;
    if !extent.x.is_finite() || !extent.y.is_finite() || !extent.z.is_finite() {
        return None;
    }
    // A degenerate part — a flat sheet, or a single triangle — still has two
    // usable axes, but one with no size at all cannot be measured against.
    let largest = extent.x.max(extent.y).max(extent.z);
    if largest <= 0.0 {
        return None;
    }

    // Down the shortest axis, so the projection lands on the widest face.
    let axis = if extent.x <= extent.y && extent.x <= extent.z {
        Axis::X
    } else if extent.y <= extent.z {
        Axis::Y
    } else {
        Axis::Z
    };

    let centre = (bounds.min + bounds.max) * 0.5;
    let (cu, cv) = axis.across(centre);
    let (eu, ev) = axis.across(extent);

    // Square artwork: a logo is not stretched to the proportions of the panel
    // it is printed on. The narrower side of the face sets the size, so the
    // label fits within the face in both directions.
    let across = eu.min(ev);
    let size = if across > 0.0 {
        across * COVERAGE
    } else {
        largest * COVERAGE
    };

    let centred = Mapping {
        projection: Projection::Planar,
        axis,
        // A size in scene units rather than a repeat count, so the same label
        // is the same size wherever it is applied.
        real_world_units: true,
        scale: vec3(size, size, 1.0),
        // The projected coordinate of the part's centre, which is where the
        // artwork's centre and its pivot both belong.
        offset: vec2(0.5 - cu / size, 0.5 - cv / size),
        pivot: vec2(cu / size, cv / size),
        // Turned as a whole, as artwork is.
        rigid: true,
        // Artwork, not a tiling texture: outside its own extent there is
        // nothing, and the material below shows through.
        wrap_u: tequila_texture::mip::Wrap::Clamp,
        wrap_v: tequila_texture::mip::Wrap::Clamp,
        ..Default::default()
    };
    // The right way up on that face, not the way the projection's own
    // coordinates happen to run.
    Some(reorient(&centred, upright(Projection::Planar, axis)))
}

/// A linear map in the texture plane, rows first: from a projection's own
/// coordinates (scaled) to the artwork's, u across the picture and v down it.
type Plane = [[f32; 2]; 2];

fn product(a: Plane, b: Plane) -> Plane {
    [
        [a[0][0] * b[0][0] + a[0][1] * b[1][0], a[0][0] * b[0][1] + a[0][1] * b[1][1]],
        [a[1][0] * b[0][0] + a[1][1] * b[1][0], a[1][0] * b[0][1] + a[1][1] * b[1][1]],
    ]
}

/// The inverse, for the turns and flips this module deals in.
fn transpose(a: Plane) -> Plane {
    [[a[0][0], a[1][0]], [a[0][1], a[1][1]]]
}

fn apply(a: Plane, p: [f32; 2]) -> [f32; 2] {
    [a[0][0] * p[0] + a[0][1] * p[1], a[1][0] * p[0] + a[1][1] * p[1]]
}

fn turn(turns: f32) -> Plane {
    let (sin, cos) = (turns * std::f32::consts::TAU).sin_cos();
    [[cos, -sin], [sin, cos]]
}

fn flips(flip_u: bool, flip_v: bool) -> Plane {
    [[if flip_u { -1.0 } else { 1.0 }, 0.0], [0.0, if flip_v { -1.0 } else { 1.0 }]]
}

/// A label's flips and turn together, in the order `Mapping` applies them.
fn orientation(mapping: &Mapping) -> Plane {
    product(flips(mapping.flip_u, mapping.flip_v), turn(mapping.rotation))
}

/// How artwork reads the right way up on a projection, seen from outside the
/// part.
///
/// A flat one from the positive side of its axis, with the world's +Y up -
/// and on a top face, the picture's top away from someone standing at +Z, the
/// front. A wrap round Y from outside, reading to the right. The
/// projections' coordinates were chosen as maths rather than for reading: v
/// runs up the part while it runs down the picture, and the angle round a
/// cylinder grows to the left. So a flat label used to land upside down, on
/// its side on a side face, and wrapped round a bottle upside down and
/// backwards. The other projections keep their own coordinates.
fn upright(projection: Projection, axis: Axis) -> Plane {
    match (projection, axis) {
        // (x, y): across is x, down is -y.
        (Projection::Planar, Axis::Z) => [[1.0, 0.0], [0.0, -1.0]],
        // (y, z): across is -z, down is -y.
        (Projection::Planar, Axis::X) => [[0.0, -1.0], [-1.0, 0.0]],
        // (angle, height): across is the angle backwards, down is -height.
        (Projection::Cylindrical, Axis::Y) => [[-1.0, 0.0], [0.0, -1.0]],
        _ => [[1.0, 0.0], [0.0, 1.0]],
    }
}

/// `mapping` turned and flipped to `to`, keeping its size, its pivot and the
/// point on the part the artwork is centred on.
///
/// Its flips are kept where they still fit, so a mirrored label stays
/// mirrored the way it was rather than becoming one turned half round.
fn reorient(mapping: &Mapping, to: Plane) -> Mapping {
    let half = [0.5, 0.5];
    let c = [mapping.pivot.x, mapping.pivot.y];
    let o = [mapping.offset.x, mapping.offset.y];
    // A rigid turn is the turn with the scale taken out and put back:
    // S R S^-1 on the scaled coordinates this works in.
    let (su, sv) = scale_factors(mapping);
    let spun = |turns: f32| -> Plane {
        let r = turn(turns);
        if mapping.rigid && su.abs() > 1e-12 && sv.abs() > 1e-12 {
            [[r[0][0], r[0][1] * su / sv], [r[1][0] * sv / su, r[1][1]]]
        } else {
            r
        }
    };
    // The artwork's centre q, from F(R(q - c) + c + o) = (0.5, 0.5).
    let fh = apply(flips(mapping.flip_u, mapping.flip_v), half);
    let back = apply(spun(-mapping.rotation), [fh[0] - c[0] - o[0], fh[1] - c[1] - o[1]]);
    let q = [c[0] + back[0], c[1] + back[1]];

    let mirrored = to[0][0] * to[1][1] - to[0][1] * to[1][0] < 0.0;
    let (flip_u, mut flip_v) = (mapping.flip_u, mapping.flip_v);
    if (flip_u != flip_v) != mirrored {
        flip_v = !flip_v;
    }
    let f = flips(flip_u, flip_v);
    let r = product(f, to);
    let rotation = r[1][0].atan2(r[0][0]) / std::f32::consts::TAU;
    let fh = apply(f, half);
    let ahead = apply(spun(rotation), [q[0] - c[0], q[1] - c[1]]);
    Mapping {
        flip_u,
        flip_v,
        rotation,
        offset: vec2(fh[0] - c[0] - ahead[0], fh[1] - c[1] - ahead[1]),
        ..*mapping
    }
}

/// The same label on another projection or axis: centred where it was, and
/// turned to read on the new one as it read on the old - upright if it was
/// upright, mirrored or turned by as much as it was.
pub fn rebase(mapping: &Mapping, projection: Projection, axis: Axis) -> Mapping {
    if projection == mapping.projection && axis == mapping.axis {
        return *mapping;
    }
    let own = product(orientation(mapping), transpose(upright(mapping.projection, mapping.axis)));
    reorient(&Mapping { projection, axis, ..*mapping }, product(own, upright(projection, axis)))
}

/// The rotation an upright label has on this projection: what the Rotation
/// control counts from, so it reads 0 on a label the right way up.
pub fn upright_turn(projection: Projection, axis: Axis) -> f32 {
    reorient(&Mapping { projection, axis, ..Mapping::default() }, upright(projection, axis)).rotation
}

/// Whether the artwork's width runs along the projection's second coordinate
/// - a label on its side, which an upright one on a side face is - so the
/// Size control can put width and height on the fields they belong to.
pub fn sideways(mapping: &Mapping) -> bool {
    let m = orientation(mapping);
    m[0][1].abs() > m[0][0].abs()
}

/// The principal axis a surface most nearly faces.
///
/// A projection here is axis-aligned, so a click on a surface has to be
/// resolved to one of three answers. The dominant component of the normal is
/// that answer, and on the flat faces labels are usually placed on it is not an
/// approximation at all.
pub fn axis_facing(normal: Vec3) -> Axis {
    let (x, y, z) = (normal.x.abs(), normal.y.abs(), normal.z.abs());
    if x >= y && x >= z {
        Axis::X
    } else if y >= z {
        Axis::Y
    } else {
        Axis::Z
    }
}

/// Move a label so its centre sits on `point`, keeping its size and rotation.
///
/// `point` and `normal` are in the part's **object space** — the same space the
/// renderer shades in — so a label placed on a part stays where it was put when
/// the part moves.
///
/// For a flat projection the face is re-chosen from the normal, because a click
/// on the side of a box means the side of the box. A wrapped one keeps its axis:
/// the axis of a cylinder is a property of the part, not of where on it the
/// user happened to click, and re-deriving it from the normal would flip the
/// wrap every time the label crossed the halfway point.
pub fn place_at(mapping: Mapping, point: Vec3, normal: Vec3) -> Mapping {
    let mut placed = match mapping.projection {
        // UV placement has no frame to move: the coordinates belong to the
        // mesh. Nothing sensible to do, so nothing is done.
        Projection::Uv => return mapping,
        // A new face, read the same way round on it as on the old one.
        Projection::Planar => rebase(&mapping, Projection::Planar, axis_facing(normal)),
        _ => mapping,
    };

    let probe = tequila_texture::projection::SurfacePoint {
        position: point,
        normal,
        uv: vec2(0.0, 0.0),
        footprint: 0.0,
        scale: 1.0,
        uv_footprint: None,
        curvature: 0.0,
        occlusion: 0.0,
        vertex_color: tequila_core::spectrum::Spectrum::ONE,
        world: Default::default(),
    };
    // Through the renderer's own formula rather than a second copy of it, so
    // the artwork lands under the cursor and not beside it.
    let Some((raw_u, raw_v)) = placed.raw_coords(&probe) else {
        return mapping;
    };

    let (su, sv) = scale_factors(&placed);
    let (pu, pv) = (raw_u * su, raw_v * sv);
    if !pu.is_finite() || !pv.is_finite() {
        return mapping;
    }

    // The pivot goes on the clicked point too, so the rotation control keeps
    // turning the artwork where it sits rather than about wherever it was
    // last placed.
    placed.pivot = vec2(pu, pv);
    // A flipped coordinate is negated after the offset, so a mirrored label
    // is centred on -0.5 rather than 0.5 before its flip.
    let (hu, hv) = (if placed.flip_u { -0.5 } else { 0.5 }, if placed.flip_v { -0.5 } else { 0.5 });
    placed.offset = vec2(hu - pu, hv - pv);
    placed
}

/// What the coordinates are multiplied by before rotation and offset.
///
/// `scale` means a size when real-world units are on and a repeat count when
/// they are off, and the coordinates take the reciprocal in the first case.
/// One function, because placing a label and drawing its frame both need this
/// and getting it backwards in either puts the artwork off the part by a factor
/// of the scale squared.
fn scale_factors(mapping: &Mapping) -> (f32, f32) {
    if mapping.real_world_units {
        (
            1.0 / mapping.scale.x.abs().max(1e-6),
            1.0 / mapping.scale.y.abs().max(1e-6),
        )
    } else {
        (mapping.scale.x, mapping.scale.y)
    }
}

/// The artwork's border, in the part's object space, as a closed polyline.
///
/// This is the projection run backwards: the four edges of the artwork are
/// walked in texture space and each sample is carried back through the offset,
/// the rotation and the scale to a point on the part. Drawn in the viewport it
/// is the frame that says where the label is, how big it is and which way up.
///
/// A polyline rather than four corners, because a wrapped label's edges are
/// arcs. `segments` is how many samples each edge gets; four corners is the
/// degenerate case and looks like a rectangle cutting through the bottle.
///
/// `None` for UV placement, which has no frame in space to draw — the artwork
/// is wherever the mesh's coordinates put it, which can be several disjoint
/// patches — and for tri-planar, which has three frames rather than one.
///
/// `viewer`, in object space, decides which side of the part a flat frame is
/// drawn on. Given one, the frame sits on the face turned towards it rather
/// than on the plane through the middle of the part — where it is half buried
/// in the geometry and reads as unrelated to the artwork on the surface.
pub fn outline(
    mapping: &Mapping,
    bounds: Aabb,
    viewer: Option<Vec3>,
    segments: usize,
) -> Option<Vec<Vec3>> {
    if !matches!(
        mapping.projection,
        Projection::Planar | Projection::Cylindrical
    ) {
        return None;
    }
    let extent = bounds.max - bounds.min;
    if !extent.x.is_finite() || !extent.y.is_finite() || !extent.z.is_finite() {
        return None;
    }

    let axis = mapping.axis;
    let centre = (bounds.min + bounds.max) * 0.5;
    // Where along the third dimension the frame is drawn. For a flat
    // projection that is a depth — the middle of the part, so the frame reads
    // as sitting in it rather than floating beside it — and for a wrapped one
    // it is a radius.
    let (eu, ev) = axis.across(extent);
    let radius = ((eu + ev) * 0.25).max(1e-4);
    let depth = match viewer {
        // The near face, plus a hair of clearance so the frame is not coplanar
        // with the surface it describes.
        Some(viewer) => {
            let (near, far) = (axis.of(bounds.min), axis.of(bounds.max));
            let clearance = axis.of(extent).abs().max(1e-3) * 0.01;
            if axis.of(viewer) >= axis.of(centre) {
                far + clearance
            } else {
                near - clearance
            }
        }
        None => axis.of(centre),
    };

    let segments = segments.max(1);
    let mut points = Vec::with_capacity(segments * 4);
    for (u0, v0, u1, v1) in [
        (0.0, 0.0, 1.0, 0.0),
        (1.0, 0.0, 1.0, 1.0),
        (1.0, 1.0, 0.0, 1.0),
        (0.0, 1.0, 0.0, 0.0),
    ] {
        for step in 0..segments {
            let t = step as f32 / segments as f32;
            let (u, v) = (u0 + (u1 - u0) * t, v0 + (v1 - v0) * t);
            let (a, b) = unfinish(mapping, u, v);
            points.push(match mapping.projection {
                Projection::Planar => axis.compose(a, b, depth),
                // `a` came back as the normalized angle and `b` as the
                // distance along the axis, which is what `raw_coords` produced
                // going the other way.
                _ => {
                    let angle = (a - 0.5) * std::f32::consts::TAU;
                    let (sin, cos) = angle.sin_cos();
                    axis.compose(radius * cos, radius * sin, b)
                }
            });
        }
    }
    points.iter().all(|p| p.is_finite()).then_some(points)
}

/// Undo [`Mapping`]'s scale, rotation, offset and flips, returning the
/// coordinates the projection produced before any of them were applied.
///
/// The order is the forward order reversed, and getting it wrong is not
/// visible in the arithmetic — it shows up as a frame that is right until the
/// label is rotated and then drifts.
fn unfinish(mapping: &Mapping, u: f32, v: f32) -> (f32, f32) {
    mapping.unapply(u, v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_texture::projection::SurfacePoint;

    fn coords(mapping: &Mapping, position: tequila_core::math::Vec3) -> (f32, f32) {
        let contributions = mapping.project(&SurfacePoint {
            position,
            normal: tequila_core::math::Vec3::Y,
            uv: vec2(0.0, 0.0),
            footprint: 0.0,
            scale: 1.0,
            uv_footprint: None,
            curvature: 0.0,
            occlusion: 0.0,
            vertex_color: tequila_core::spectrum::Spectrum::ONE,
            world: Default::default(),
        });
        assert_eq!(contributions.len(), 1);
        (contributions[0].coords.u, contributions[0].coords.v)
    }

    /// Where on the artwork a point lands: (across, down) of the picture.
    fn reads(mapping: &Mapping, at: Vec3) -> (f32, f32) {
        coords(mapping, at)
    }

    /// Asserts that, from the centre, `right` reads further across the
    /// picture and `up` further up it - the artwork the right way up and the
    /// right way round, seen from where those directions are right and up.
    fn assert_upright(mapping: &Mapping, centre: Vec3, right: Vec3, up: Vec3, what: &str) {
        let (u0, v0) = reads(mapping, centre);
        assert!((u0 - 0.5).abs() < 1e-4 && (v0 - 0.5).abs() < 1e-4, "{what}: the centre read ({u0}, {v0})");
        let (ur, vr) = reads(mapping, centre + right);
        let (uu, vu) = reads(mapping, centre + up);
        // Linear steps: across moves u alone and up moves v alone, the
        // right way.
        let (du, dv) = (ur - u0, vr - v0);
        assert!(
            du > 0.0 && dv.abs() < 0.01 * du,
            "{what}: to the right read ({ur}, {vr}) from ({u0}, {v0}); the artwork is mirrored or turned"
        );
        let (du, dv) = (uu - u0, vu - v0);
        assert!(
            dv < 0.0 && du.abs() < 0.01 * -dv,
            "{what}: upwards read ({uu}, {vu}) from ({u0}, {v0}); the artwork is upside down or on its side"
        );
    }

    /// A fitted label reads the right way up on the face it lands on, seen
    /// from outside: the front (+Z), the side (+X) and the top, looked down
    /// on from the front. It used to land upside down on the front and on its
    /// side on the side.
    #[test]
    fn a_fitted_label_reads_the_right_way_up() {
        let d = 0.002;
        let front = Aabb { min: vec3(10.0, 4.0, -0.5), max: vec3(14.0, 8.0, 0.5) };
        let fitted = fit_to_bounds(front).expect("fits");
        assert_eq!(fitted.axis, Axis::Z);
        assert_upright(&fitted, vec3(12.0, 6.0, 0.5), vec3(d, 0.0, 0.0), vec3(0.0, d, 0.0), "the front");

        let side = Aabb { min: vec3(-0.5, 4.0, 10.0), max: vec3(0.5, 8.0, 14.0) };
        let fitted = fit_to_bounds(side).expect("fits");
        assert_eq!(fitted.axis, Axis::X);
        assert_upright(&fitted, vec3(0.5, 6.0, 12.0), vec3(0.0, 0.0, -d), vec3(0.0, d, 0.0), "the side");
        assert!(sideways(&fitted), "on a side face the width runs along the projection's second coordinate");

        let top = Aabb { min: vec3(10.0, -0.5, 4.0), max: vec3(14.0, 0.5, 8.0) };
        let fitted = fit_to_bounds(top).expect("fits");
        assert_eq!(fitted.axis, Axis::Y);
        assert_upright(&fitted, vec3(12.0, 0.5, 6.0), vec3(d, 0.0, 0.0), vec3(0.0, 0.0, -d), "the top");

        // Wrapped round a bottle and seen from the front.
        let wrapped = rebase(&fitted, Projection::Cylindrical, Axis::Y);
        let r = 1.0;
        let at = vec3(0.0, 0.3, r);
        let wrapped = place_at(Mapping { real_world_units: false, scale: vec3(4.0, 4.0, 1.0), ..wrapped }, at, Vec3::Z);
        assert_upright(&wrapped, at, vec3(d, 0.0, 0.0), vec3(0.0, d, 0.0), "round a bottle");
    }

    /// Dragged from the front of a box onto its side, a label stays the right
    /// way up - and a mirrored or turned one stays mirrored or turned by as
    /// much, and still lands under the cursor. The mirror and turn are the
    /// person's; the face's own coordinates are not.
    #[test]
    fn a_label_moved_to_another_face_keeps_reading_the_same_way() {
        let d = 0.002;
        let start = fit_to_bounds(Aabb { min: vec3(-1.0, -1.0, -0.2), max: vec3(1.0, 1.0, 0.2) }).expect("fits");
        let on_side = place_at(start, vec3(1.0, 0.3, 0.1), Vec3::X);
        assert_eq!(on_side.axis, Axis::X);
        assert_upright(&on_side, vec3(1.0, 0.3, 0.1), vec3(0.0, 0.0, -d), vec3(0.0, d, 0.0), "moved onto the side");

        for (name, mine) in [("mirrored", start.mirrored(true)), ("turned", Mapping { rotation: start.rotation + 0.1, ..start })] {
            let front = vec3(0.2, -0.1, 0.2);
            let here = place_at(mine, front, Vec3::Z);
            let there = place_at(here, vec3(1.0, 0.3, 0.1), Vec3::X);
            let (u, v) = reads(&there, vec3(1.0, 0.3, 0.1));
            assert!((u - 0.5).abs() < 1e-4 && (v - 0.5).abs() < 1e-4, "{name}: landed at ({u}, {v}), not under the click");
            // The same small step right and up reads the same on both faces.
            let step = |m: &Mapping, at: Vec3, right: Vec3, up: Vec3| {
                let (u0, v0) = reads(m, at);
                let (ur, vr) = reads(m, at + right);
                let (uu, vu) = reads(m, at + up);
                [ur - u0, vr - v0, uu - u0, vu - v0]
            };
            let a = step(&here, front, vec3(d, 0.0, 0.0), vec3(0.0, d, 0.0));
            let b = step(&there, vec3(1.0, 0.3, 0.1), vec3(0.0, 0.0, -d), vec3(0.0, d, 0.0));
            for k in 0..4 {
                assert!((a[k] - b[k]).abs() < 1e-4, "{name}: read {a:?} on the front but {b:?} on the side");
            }
        }
    }

    /// A mirrored label clicked somewhere lands there, mirrored. Placing
    /// used to centre it as if it were not flipped, a whole artwork-width
    /// away.
    #[test]
    fn a_mirrored_label_lands_where_it_was_placed() {
        let start = fit_to_bounds(Aabb { min: vec3(-1.0, -1.0, -0.1), max: vec3(1.0, 1.0, 0.1) }).expect("fits");
        for mine in [start.mirrored(true), start.mirrored(false), start.mirrored(true).mirrored(false)] {
            let at = vec3(0.4, -0.3, 0.1);
            let placed = place_at(mine, at, Vec3::Z);
            assert_eq!((placed.flip_u, placed.flip_v), (mine.flip_u, mine.flip_v));
            let (u, v) = reads(&placed, at);
            assert!((u - 0.5).abs() < 1e-4 && (v - 0.5).abs() < 1e-4, "landed at ({u}, {v})");
        }
    }

    /// A panel: wide in X and Z, thin in Y. The artwork belongs on the big
    /// face, which means projecting down Y.
    #[test]
    fn a_flat_panel_is_projected_onto_its_widest_face() {
        let panel = Aabb {
            min: vec3(-2.0, -0.05, -1.0),
            max: vec3(2.0, 0.05, 1.0),
        };
        let mapping = fit_to_bounds(panel).expect("a panel can be fitted");
        assert_eq!(mapping.axis, Axis::Y);
        assert_eq!(mapping.projection, Projection::Planar);
    }

    /// The artwork's centre lands on the part's centre, and one unit of
    /// texture spans the size that was chosen.
    ///
    /// Tested through `project` rather than by reading the fields back,
    /// because the fields are only right if the arithmetic that consumes them
    /// agrees — an offset computed against one convention and applied under
    /// another is exactly the sort of thing that reads correctly in both
    /// places and puts the label somewhere else.
    #[test]
    fn the_artwork_is_centred_on_the_part() {
        // Deliberately nowhere near the origin: a part that happens to be
        // centred would make a missing offset invisible.
        let part = Aabb {
            min: vec3(10.0, 4.0, -0.5),
            max: vec3(14.0, 8.0, 0.5),
        };
        let mapping = fit_to_bounds(part).expect("fits");
        // Thinnest in Z, so the face is XY and its centre is (12, 6).
        assert_eq!(mapping.axis, Axis::Z);

        let (u, v) = coords(&mapping, vec3(12.0, 6.0, 0.5));
        assert!(
            (u - 0.5).abs() < 1e-4 && (v - 0.5).abs() < 1e-4,
            "the centre of the part projected to ({u}, {v}), not the centre of \
             the artwork"
        );

        // ...and the artwork covers the fraction of the face it claims to.
        // Half a size either side of the centre is the artwork's own edge.
        let size = mapping.scale.x;
        let (edge, _) = coords(&mapping, vec3(12.0 + size * 0.5, 6.0, 0.5));
        assert!(
            (edge - 1.0).abs() < 1e-4,
            "half a size from the centre reached u = {edge} rather than the \
             artwork's edge at 1.0"
        );
        assert!(
            (size - 4.0 * COVERAGE).abs() < 1e-4,
            "a 4 x 4 face gave a label {size} across, not {} of it",
            COVERAGE
        );
    }

    /// Rotating a fitted label turns it where it sits.
    ///
    /// This is the pivot doing its job, and the part is far from the origin
    /// precisely because that is when a missing pivot throws the artwork off
    /// the part rather than merely tilting it.
    #[test]
    fn a_fitted_label_rotates_in_place() {
        let part = Aabb {
            min: vec3(20.0, -1.0, 6.0),
            max: vec3(24.0, 1.0, 10.0),
        };
        let fitted = fit_to_bounds(part).expect("fits");
        let centre = (part.min + part.max) * 0.5;

        for turns in [0.0, 0.05, 0.25, 0.5, -0.125] {
            let rotated = Mapping {
                rotation: turns,
                ..fitted
            };
            let (u, v) = coords(&rotated, centre);
            assert!(
                (u - 0.5).abs() < 1e-4 && (v - 0.5).abs() < 1e-4,
                "turned {turns} of a turn, the artwork's centre moved to \
                 ({u}, {v})"
            );
        }
    }

    /// The whole claim of click-to-place: wherever you click on the part, the
    /// middle of the artwork ends up there.
    ///
    /// Checked by projecting the clicked point back through the mapping, which
    /// is the only way to state it that does not just restate the arithmetic
    /// that produced it — the offset is right if and only if the renderer,
    /// reading it, puts the artwork's centre on the point.
    #[test]
    fn a_label_lands_where_it_was_placed() {
        let start = fit_to_bounds(Aabb {
            min: vec3(-1.0, -1.0, -0.1),
            max: vec3(1.0, 1.0, 0.1),
        })
        .expect("fits");

        // Points scattered over a face, none of them at the origin or the
        // centre, where a missing offset would be invisible.
        for at in [
            vec3(0.4, 0.7, 0.1),
            vec3(-0.9, 0.2, 0.1),
            vec3(0.0, -0.6, 0.1),
            vec3(-0.35, -0.85, 0.1),
        ] {
            let placed = place_at(start, at, vec3(0.0, 0.0, 1.0));
            let (u, v) = coords(&placed, at);
            assert!(
                (u - 0.5).abs() < 1e-4 && (v - 0.5).abs() < 1e-4,
                "clicked at {at:?}, artwork centre landed at ({u}, {v})"
            );
        }
    }

    /// Placing keeps the size and the rotation. Moving a label is moving it,
    /// not starting it again.
    #[test]
    fn placing_keeps_the_size_and_rotation() {
        let mine = Mapping {
            rotation: 0.125,
            scale: vec3(0.3, 0.45, 1.0),
            ..fit_to_bounds(Aabb {
                min: vec3(-1.0, -1.0, -0.1),
                max: vec3(1.0, 1.0, 0.1),
            })
            .expect("fits")
        };
        let placed = place_at(mine, vec3(0.6, -0.2, 0.1), vec3(0.0, 0.0, 1.0));
        assert_eq!(placed.scale, mine.scale);
        assert_eq!(placed.rotation, mine.rotation);

        // ...and it still lands where it was put, with the rotation applied —
        // which is the pivot moving with the placement rather than staying
        // behind at the old centre.
        let (u, v) = coords(&placed, vec3(0.6, -0.2, 0.1));
        assert!(
            (u - 0.5).abs() < 1e-4 && (v - 0.5).abs() < 1e-4,
            "a rotated label landed at ({u}, {v}) rather than under the click"
        );
    }

    /// A click on the side of a box means the side of the box.
    #[test]
    fn a_flat_label_takes_the_face_it_was_clicked_on() {
        let start = fit_to_bounds(Aabb {
            min: vec3(-1.0, -1.0, -1.0),
            max: vec3(1.0, 1.0, 1.0),
        })
        .expect("fits");
        for (normal, expected) in [
            (vec3(1.0, 0.0, 0.0), Axis::X),
            (vec3(-0.98, 0.1, 0.05), Axis::X),
            (vec3(0.0, -1.0, 0.0), Axis::Y),
            (vec3(0.2, 0.1, 0.97), Axis::Z),
        ] {
            assert_eq!(
                place_at(start, vec3(0.5, 0.5, 0.5), normal).axis,
                expected,
                "a face pointing {normal:?} chose the wrong axis"
            );
        }
    }

    /// A wrapped label keeps the axis it wraps about.
    ///
    /// That axis is a property of the part — the bottle stands up whichever
    /// side of it was clicked — and re-deriving it from the normal would flip
    /// the wrap as the label was dragged around the cylinder.
    #[test]
    fn a_wrapped_label_keeps_its_axis_wherever_it_is_clicked() {
        let wrapped = Mapping {
            projection: Projection::Cylindrical,
            axis: Axis::Y,
            real_world_units: false,
            scale: vec3(1.0, 1.0, 1.0),
            ..Default::default()
        };
        for normal in [
            vec3(1.0, 0.0, 0.0),
            vec3(0.0, 0.0, -1.0),
            vec3(-0.7, 0.0, 0.7),
        ] {
            let placed = place_at(wrapped, vec3(normal.x, 0.4, normal.z), normal);
            assert_eq!(placed.axis, Axis::Y, "the wrap axis moved");
            let (u, v) = coords(&placed, vec3(normal.x, 0.4, normal.z));
            assert!(
                (u - 0.5).abs() < 1e-4 && (v - 0.5).abs() < 1e-4,
                "a wrapped label landed at ({u}, {v})"
            );
        }
    }

    /// A UV label has no frame to move, so a click leaves it exactly alone
    /// rather than silently corrupting an authored layout.
    #[test]
    fn placing_a_uv_label_does_nothing() {
        let uv = Mapping::default();
        assert_eq!(place_at(uv, vec3(3.0, 4.0, 5.0), vec3(0.0, 1.0, 0.0)), uv);
    }

    /// The frame drawn in the viewport is the artwork's border, and the only
    /// way to say so without restating the arithmetic is to run it forwards
    /// again: every point on the outline must project back onto an edge of the
    /// artwork, where u or v is 0 or 1.
    ///
    /// This is what catches an inverse applied in the wrong order. Undoing the
    /// rotation before the offset instead of after gives a frame that is
    /// exactly right until the label is rotated and then drifts away from it —
    /// visible only if you rotate one and look carefully, which is to say, not
    /// visible.
    #[test]
    fn the_outline_traces_the_edge_of_the_artwork() {
        let bounds = Aabb {
            min: vec3(-1.0, -2.0, -0.2),
            max: vec3(1.0, 2.0, 0.2),
        };
        let base = fit_to_bounds(bounds).expect("fits");

        let cases = [
            ("as fitted", base),
            (
                "rotated",
                Mapping {
                    rotation: 0.17,
                    ..base
                },
            ),
            (
                "moved and rotated",
                place_at(
                    Mapping {
                        rotation: -0.3,
                        ..base
                    },
                    vec3(0.4, -1.1, 0.2),
                    vec3(0.0, 0.0, 1.0),
                ),
            ),
            (
                "flipped",
                Mapping {
                    flip_u: true,
                    rotation: 0.1,
                    ..base
                },
            ),
            (
                // A wrapped label covering a quarter turn, placed facing +X so
                // it sits well away from the seam.
                //
                // Away from the seam on purpose: a cylindrical projection is
                // periodic, so a label straddling the point where the angle
                // wraps has edges whose forward projection lands a full turn
                // apart. The frame drawn in the viewport is still continuous —
                // it is built in object space — but "project it forward and
                // check the coordinate" cannot express that, and a test that
                // pretended otherwise would be asserting against the
                // projection's own definition.
                "wrapped",
                place_at(
                    Mapping {
                        projection: Projection::Cylindrical,
                        axis: Axis::Y,
                        real_world_units: false,
                        scale: vec3(4.0, 1.0, 1.0),
                        rotation: 0.0,
                        pivot: vec2(0.0, 0.0),
                        offset: vec2(0.0, 0.0),
                        ..Default::default()
                    },
                    vec3(0.6, 0.3, 0.0),
                    vec3(1.0, 0.0, 0.0),
                ),
            ),
        ];

        for (name, mapping) in cases {
            // Every viewer choice, because a planar projection ignores
            // the depth entirely: the frame may be drawn on either face
            // and still trace the same artwork. A depth that leaked into
            // the coordinates would fail one of these.
            for viewer in [None, Some(vec3(0.0, 0.0, 50.0)), Some(vec3(0.0, 0.0, -50.0))] {
            let points = outline(&mapping, bounds, viewer, 8).expect("a frame to draw");
            assert_eq!(points.len(), 32, "{name}: wrong number of samples");
            for point in points {
                let (u, v) = coords(&mapping, point);
                let on_edge = [u, v]
                    .iter()
                    .any(|c| c.abs() < 1e-3 || (c - 1.0).abs() < 1e-3);
                assert!(
                    on_edge,
                    "{name}: an outline point projected to ({u}, {v}), which is \
                     not on the artwork's edge"
                );
                // ...and inside it, so the frame is the border rather than a
                // line running off across the part.
                assert!(
                    (-1e-3..=1.0 + 1e-3).contains(&u) && (-1e-3..=1.0 + 1e-3).contains(&v),
                    "{name}: an outline point projected to ({u}, {v}), outside \
                     the artwork altogether"
                );
            }
            }
        }
    }

    /// A frame is drawn only where there is one frame to draw.
    #[test]
    fn placements_without_a_single_frame_have_no_outline() {
        let bounds = Aabb {
            min: vec3(-1.0, -1.0, -1.0),
            max: vec3(1.0, 1.0, 1.0),
        };
        for projection in [Projection::Uv, Projection::TriPlanar, Projection::Spherical] {
            assert!(
                outline(
                    &Mapping {
                        projection,
                        ..Default::default()
                    },
                    bounds,
                    None,
                    4
                )
                .is_none(),
                "{projection:?} produced a frame it cannot have"
            );
        }
    }

    /// Nothing here may produce a mapping that cannot be sampled.
    #[test]
    fn degenerate_parts_are_refused_rather_than_fitted_to_nothing() {
        assert!(fit_to_bounds(Aabb::EMPTY).is_none());
        assert!(fit_to_bounds(Aabb {
            min: vec3(1.0, 1.0, 1.0),
            max: vec3(1.0, 1.0, 1.0),
        })
        .is_none());

        // A sheet with no thickness is *not* degenerate: it has a face, and it
        // is exactly what a label is most often put on.
        let sheet = Aabb {
            min: vec3(-1.0, 0.0, -1.0),
            max: vec3(1.0, 0.0, 1.0),
        };
        let mapping = fit_to_bounds(sheet).expect("a sheet has a face");
        assert_eq!(mapping.axis, Axis::Y);
        assert!(mapping.scale.x > 0.0 && mapping.scale.x.is_finite());
        let (u, v) = coords(&mapping, vec3(0.0, 0.0, 0.0));
        assert!(u.is_finite() && v.is_finite());
    }
}
