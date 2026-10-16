//! The transform gizmo's arithmetic.
//!
//! `docs/04-UX-SPEC.md` §5. The drawing belongs to the app; everything that
//! decides *what a drag means* is here, because that is the part with a right
//! answer and therefore the part worth testing. A gizmo that moves the object
//! a little differently from where the cursor went is the kind of bug that is
//! obvious in use and invisible in a screenshot.
//!
//! The model throughout: a drag is defined by two rays through the cursor —
//! where it went down, and where it is now — and the handle turns that pair
//! into a delta transform applied about the pivot.

use crate::document::SceneDocument;
use crate::tree::{NodeId, NodeKind};
use tequila_core::math::{vec3, Aabb, Mat4, Ray, Transform, Vec3};

/// `W` / `E` / `R` (§5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GizmoMode {
    #[default]
    Move,
    Rotate,
    Scale,
}

impl GizmoMode {
    pub fn label(self) -> &'static str {
        match self {
            GizmoMode::Move => "Move",
            GizmoMode::Rotate => "Rotate",
            GizmoMode::Scale => "Scale",
        }
    }

    /// The hotkey §6 binds to this mode.
    pub fn hotkey(self) -> char {
        match self {
            GizmoMode::Move => 'W',
            GizmoMode::Rotate => 'E',
            GizmoMode::Scale => 'R',
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub fn direction(self) -> Vec3 {
        match self {
            Axis::X => Vec3::X,
            Axis::Y => Vec3::Y,
            Axis::Z => Vec3::Z,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Axis::X => "X",
            Axis::Y => "Y",
            Axis::Z => "Z",
        }
    }

    pub const ALL: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];
}

/// A part of the gizmo the cursor can grab (§5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    /// One arrow or one rotation ring.
    Axis(Axis),
    /// A plane quad, named by its *normal*: `Plane(Y)` slides in XZ, which is
    /// the ground plane and the one product staging uses most.
    Plane(Axis),
    /// The screen-space handle: move parallel to the film, or scale
    /// uniformly.
    Screen,
}

impl Handle {
    pub fn label(self) -> String {
        match self {
            Handle::Axis(a) => a.label().to_string(),
            Handle::Plane(Axis::X) => "YZ".into(),
            Handle::Plane(Axis::Y) => "XZ".into(),
            Handle::Plane(Axis::Z) => "XY".into(),
            Handle::Screen => "Screen".into(),
        }
    }
}

/// Where the gizmo sits and how it is oriented (§5's world/local toggle).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GizmoFrame {
    pub pivot: Vec3,
    /// Column vectors of the basis the handles point along. World space is the
    /// identity; local space is the selection's rotation.
    pub basis: [Vec3; 3],
    /// World-space length of an axis handle. The app keeps this proportional
    /// to distance so the gizmo has a constant size on screen.
    pub handle_length: f32,
}

impl Default for GizmoFrame {
    fn default() -> Self {
        GizmoFrame {
            pivot: Vec3::ZERO,
            basis: [Vec3::X, Vec3::Y, Vec3::Z],
            handle_length: 1.0,
        }
    }
}

impl GizmoFrame {
    pub fn world(pivot: Vec3, handle_length: f32) -> GizmoFrame {
        GizmoFrame {
            pivot,
            basis: [Vec3::X, Vec3::Y, Vec3::Z],
            handle_length,
        }
    }

    /// Local space: the handles follow the node's own rotation.
    ///
    /// The basis is orthonormalized. A node with a non-uniform scale has a
    /// non-orthogonal matrix, and handles that are not perpendicular would let
    /// a drag along X change Y.
    pub fn local(pivot: Vec3, transform: Transform, handle_length: f32) -> GizmoFrame {
        let column = |i: usize| {
            vec3(
                transform.matrix.m[0][i],
                transform.matrix.m[1][i],
                transform.matrix.m[2][i],
            )
        };
        let x = column(0).try_normalize().unwrap_or(Vec3::X);
        let y = column(1)
            .try_normalize()
            .map(|y| (y - x * x.dot(y)).try_normalize().unwrap_or(Vec3::Y))
            .unwrap_or(Vec3::Y);
        let z = x.cross(y);
        GizmoFrame {
            pivot,
            basis: [x, y, z],
            handle_length,
        }
    }

    pub fn direction(&self, axis: Axis) -> Vec3 {
        match axis {
            Axis::X => self.basis[0],
            Axis::Y => self.basis[1],
            Axis::Z => self.basis[2],
        }
    }
}

/// Snapping increments (§5).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snapping {
    /// Grid increment in scene units; `None` for free movement.
    pub grid: Option<f32>,
    /// Angle increment in radians. §5 gives 15° as the default.
    pub angle: Option<f32>,
    /// Scale increment, e.g. 0.1 for tenths.
    pub scale: Option<f32>,
    /// Snap a dragged point to the nearest vertex within this world-space
    /// distance, when hovering geometry.
    pub vertex_within: Option<f32>,
}

impl Default for Snapping {
    fn default() -> Self {
        Snapping {
            grid: None,
            angle: Some(15f32.to_radians()),
            scale: None,
            vertex_within: None,
        }
    }
}

impl Snapping {
    /// Everything off, for a free drag.
    pub const OFF: Snapping = Snapping {
        grid: None,
        angle: None,
        scale: None,
        vertex_within: None,
    };
}

/// A drag in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drag {
    pub handle: Handle,
    pub mode: GizmoMode,
    pub frame: GizmoFrame,
    /// Where the cursor grabbed, in world space. Kept rather than recomputed
    /// so the object does not jump on the first frame by the distance between
    /// the cursor and the handle's centre.
    pub grab: Vec3,
    /// The node's transform when the drag began. Deltas are always measured
    /// from here, never accumulated frame to frame — accumulating drifts, and
    /// a snapped drag would ratchet.
    pub start: Transform,
    /// Camera forward, for the screen-space handle and for rotation rings seen
    /// edge-on.
    pub view: Vec3,
}

/// Which handle a ray hits, or `None`.
///
/// Tested against the handles in order of how hard they are to hit: the
/// screen handle is a small disc at the centre, plane quads sit inside the
/// axes, and axis shafts are last. Picking the axis first would make the
/// plane quads unreachable, since an axis shaft passes through both.
pub fn pick(ray: &Ray, frame: &GizmoFrame, mode: GizmoMode, view: Vec3) -> Option<Handle> {
    let length = frame.handle_length.max(1e-6);
    // Generous relative to the drawn geometry: a 6-pixel arrow needs a
    // forgiving hit region or the gizmo is unusable with a trackpad.
    let shaft_radius = length * 0.09;

    if mode != GizmoMode::Rotate {
        // The screen handle: a small disc at the pivot facing the camera.
        if let Some(hit) = intersect_plane(ray, frame.pivot, view) {
            if (hit - frame.pivot).length() <= length * 0.16 {
                return Some(Handle::Screen);
            }
        }
        // Plane quads, in the inner corner between two axes.
        let mut best: Option<(f32, Handle)> = None;
        for axis in Axis::ALL {
            let normal = frame.direction(axis);
            let Some(hit) = intersect_plane(ray, frame.pivot, normal) else {
                continue;
            };
            let local = hit - frame.pivot;
            let (u, v) = plane_axes(frame, axis);
            let (a, b) = (local.dot(u), local.dot(v));
            let (low, high) = (length * 0.22, length * 0.55);
            if (low..=high).contains(&a) && (low..=high).contains(&b) {
                let distance = (hit - ray.origin).length();
                if best.is_none_or(|(d, _)| distance < d) {
                    best = Some((distance, Handle::Plane(axis)));
                }
            }
        }
        if let Some((_, handle)) = best {
            return Some(handle);
        }
    }

    let mut best: Option<(f32, Handle)> = None;
    for axis in Axis::ALL {
        let direction = frame.direction(axis);
        let hit = match mode {
            // A rotation ring: the circle of radius `length` in the plane
            // normal to the axis.
            GizmoMode::Rotate => {
                let Some(point) = intersect_plane(ray, frame.pivot, direction) else {
                    continue;
                };
                let radius = (point - frame.pivot).length();
                if (radius - length).abs() > shaft_radius {
                    continue;
                }
                Some((point, (point - ray.origin).length()))
            }
            // An arrow or a scale handle: the segment from the pivot outward.
            _ => {
                let (on_axis, on_ray) = closest_points(frame.pivot, direction, ray);
                let along = (on_axis - frame.pivot).dot(direction);
                if !(0.0..=length).contains(&along) {
                    continue;
                }
                if (on_axis - on_ray).length() > shaft_radius {
                    continue;
                }
                Some((on_axis, (on_ray - ray.origin).length()))
            }
        };
        if let Some((_, distance)) = hit {
            if best.is_none_or(|(d, _)| distance < d) {
                best = Some((distance, Handle::Axis(axis)));
            }
        }
    }
    best.map(|(_, handle)| handle)
}

/// The two frame axes that span the plane normal to `axis`.
fn plane_axes(frame: &GizmoFrame, axis: Axis) -> (Vec3, Vec3) {
    match axis {
        Axis::X => (frame.basis[1], frame.basis[2]),
        Axis::Y => (frame.basis[2], frame.basis[0]),
        Axis::Z => (frame.basis[0], frame.basis[1]),
    }
}

/// Begin a drag: work out where on the handle the cursor grabbed.
pub fn begin(
    handle: Handle,
    mode: GizmoMode,
    frame: GizmoFrame,
    start: Transform,
    ray: &Ray,
    view: Vec3,
) -> Drag {
    let grab = grab_point(handle, mode, &frame, ray, view).unwrap_or(frame.pivot);
    Drag {
        handle,
        mode,
        frame,
        grab,
        start,
        view,
    }
}

fn grab_point(
    handle: Handle,
    mode: GizmoMode,
    frame: &GizmoFrame,
    ray: &Ray,
    view: Vec3,
) -> Option<Vec3> {
    match (handle, mode) {
        (Handle::Axis(axis), GizmoMode::Rotate) => {
            intersect_plane(ray, frame.pivot, frame.direction(axis))
        }
        (Handle::Axis(axis), _) => {
            let direction = frame.direction(axis);
            Some(closest_points(frame.pivot, direction, ray).0)
        }
        (Handle::Plane(axis), _) => intersect_plane(ray, frame.pivot, frame.direction(axis)),
        (Handle::Screen, _) => intersect_plane(ray, frame.pivot, view),
    }
}

/// The transform the node should have, given where the cursor is now.
///
/// Always computed from [`Drag::start`], never composed onto the previous
/// frame's result: accumulating would drift over a long drag, and with
/// snapping on it would ratchet the object one increment per frame.
pub fn update(drag: &Drag, ray: &Ray, snapping: &Snapping) -> Transform {
    let delta = match drag.mode {
        GizmoMode::Move => translation_delta(drag, ray, snapping),
        GizmoMode::Rotate => rotation_delta(drag, ray, snapping),
        GizmoMode::Scale => scale_delta(drag, ray, snapping),
    };
    // About the pivot, in the pivot's own frame: a rotation applied at the
    // origin would swing an off-centre object across the scene.
    let to_pivot = Mat4::translation(drag.frame.pivot);
    let from_pivot = Mat4::translation(-drag.frame.pivot);
    let matrix = to_pivot * delta * from_pivot * drag.start.matrix;
    Transform::from_matrix(matrix).unwrap_or(drag.start)
}

fn translation_delta(drag: &Drag, ray: &Ray, snapping: &Snapping) -> Mat4 {
    let now = match drag.handle {
        Handle::Axis(axis) => {
            let direction = drag.frame.direction(axis);
            closest_points(drag.frame.pivot, direction, ray).0
        }
        Handle::Plane(axis) => match intersect_plane(ray, drag.grab, drag.frame.direction(axis)) {
            Some(point) => point,
            None => return Mat4::IDENTITY,
        },
        Handle::Screen => match intersect_plane(ray, drag.grab, drag.view) {
            Some(point) => point,
            None => return Mat4::IDENTITY,
        },
    };

    let mut offset = now - drag.grab;
    // An axis drag may only move along that axis, even when the closest-point
    // solve wanders off it near a grazing view.
    if let Handle::Axis(axis) = drag.handle {
        let direction = drag.frame.direction(axis);
        offset = direction * offset.dot(direction);
    }

    if let Some(step) = snapping.grid.filter(|s| *s > 0.0) {
        // Snap the *resulting position*, not the offset: snapping the offset
        // leaves an object that started off-grid off-grid forever, which is
        // not what a grid is for.
        let start = position_of(&drag.start);
        let target = start + offset;
        let snapped = vec3(
            (target.x / step).round() * step,
            (target.y / step).round() * step,
            (target.z / step).round() * step,
        );
        offset = match drag.handle {
            // Only the dragged axis is snapped; the others must stay exactly
            // where they were.
            Handle::Axis(axis) => {
                let direction = drag.frame.direction(axis);
                let target_axis = target.dot(direction);
                direction * ((target_axis / step).round() * step - start.dot(direction))
            }
            Handle::Plane(axis) => {
                let (u, v) = plane_axes(&drag.frame, axis);
                u * ((target.dot(u) / step).round() * step - start.dot(u))
                    + v * ((target.dot(v) / step).round() * step - start.dot(v))
            }
            _ => snapped - start,
        };
    }
    Mat4::translation(offset)
}

fn rotation_delta(drag: &Drag, ray: &Ray, snapping: &Snapping) -> Mat4 {
    let axis = match drag.handle {
        Handle::Axis(axis) => drag.frame.direction(axis),
        // The screen handle rotates about the view direction, which is the
        // trackball gesture.
        _ => drag.view,
    };
    let Some(now) = intersect_plane(ray, drag.frame.pivot, axis) else {
        return Mat4::IDENTITY;
    };

    let from = (drag.grab - drag.frame.pivot).try_normalize();
    let to = (now - drag.frame.pivot).try_normalize();
    let (Some(from), Some(to)) = (from, to) else {
        return Mat4::IDENTITY;
    };

    // Signed angle about the axis: `acos` alone loses the direction, and the
    // gizmo would only ever turn one way.
    let angle = from.cross(to).dot(axis).atan2(from.dot(to));
    let angle = match snapping.angle.filter(|s| *s > 0.0) {
        Some(step) => (angle / step).round() * step,
        None => angle,
    };
    Mat4::rotation(axis, angle)
}

fn scale_delta(drag: &Drag, ray: &Ray, snapping: &Snapping) -> Mat4 {
    let factor = match drag.handle {
        Handle::Axis(axis) => {
            let direction = drag.frame.direction(axis);
            let now = closest_points(drag.frame.pivot, direction, ray).0;
            ratio(
                (now - drag.frame.pivot).dot(direction),
                (drag.grab - drag.frame.pivot).dot(direction),
            )
        }
        Handle::Plane(axis) => {
            let Some(now) = intersect_plane(ray, drag.frame.pivot, drag.frame.direction(axis)) else { return Mat4::IDENTITY; };
            ratio((now - drag.frame.pivot).length(), (drag.grab - drag.frame.pivot).length())
        }
        Handle::Screen => {
            let Some(now) = intersect_plane(ray, drag.frame.pivot, drag.view) else {
                return Mat4::IDENTITY;
            };
            ratio(
                (now - drag.frame.pivot).length(),
                (drag.grab - drag.frame.pivot).length(),
            )
        }
    };
    let factor = match snapping.scale.filter(|s| *s > 0.0) {
        Some(step) => (factor / step).round() * step,
        None => factor,
    };
    // Never zero or negative: a zero scale collapses the part to a plane and
    // makes its transform non-invertible, which the renderer cannot use for
    // normals. The gizmo refuses rather than producing a broken document.
    let factor = factor.max(1e-4);

    match drag.handle {
        Handle::Axis(axis) => {
            // Non-uniform scale along one frame axis, which in local space is
            // not a world axis: R * S * R⁻¹.
            let direction = drag.frame.direction(axis);
            let basis = Mat4::from_columns(
                drag.frame.basis[0],
                drag.frame.basis[1],
                drag.frame.basis[2],
            );
            let mut scale = Vec3::ONE;
            match axis {
                Axis::X => scale.x = factor,
                Axis::Y => scale.y = factor,
                Axis::Z => scale.z = factor,
            }
            let _ = direction;
            basis * Mat4::scale(scale) * basis.transpose()
        }
        Handle::Plane(axis) => {
            let basis = Mat4::from_columns(drag.frame.basis[0], drag.frame.basis[1], drag.frame.basis[2]);
            let mut scale = Vec3::ONE * factor;
            match axis { Axis::X => scale.x = 1.0, Axis::Y => scale.y = 1.0, Axis::Z => scale.z = 1.0 }
            basis * Mat4::scale(scale) * basis.transpose()
        }
        Handle::Screen => Mat4::scale(Vec3::ONE * factor),
    }
}

/// A scale factor from two distances, guarding the degenerate grab.
fn ratio(now: f32, then: f32) -> f32 {
    if then.abs() < 1e-6 {
        1.0
    } else {
        now / then
    }
}

fn position_of(transform: &Transform) -> Vec3 {
    vec3(
        transform.matrix.m[0][3],
        transform.matrix.m[1][3],
        transform.matrix.m[2][3],
    )
}

/// Where a ray meets a plane, or `None` when it runs parallel to it.
pub fn intersect_plane(ray: &Ray, point: Vec3, normal: Vec3) -> Option<Vec3> {
    let denominator = ray.direction.dot(normal);
    // A near-parallel ray gives a distance in the millions and a hit position
    // off in space; refusing is better than snapping the object to infinity.
    if denominator.abs() < 1e-6 {
        return None;
    }
    let t = (point - ray.origin).dot(normal) / denominator;
    if t < 0.0 {
        return None;
    }
    Some(ray.origin + ray.direction * t)
}

/// Closest points between the line `origin + t*direction` and a ray.
///
/// Returns `(on the line, on the ray)`. The standard two-line solve, with the
/// parallel case falling back to the line's origin.
pub fn closest_points(origin: Vec3, direction: Vec3, ray: &Ray) -> (Vec3, Vec3) {
    let (u, v) = (direction, ray.direction);
    let w = origin - ray.origin;
    let (a, b, c) = (u.dot(u), u.dot(v), v.dot(v));
    let (d, e) = (u.dot(w), v.dot(w));
    let denominator = a * c - b * b;
    if denominator.abs() < 1e-8 {
        return (origin, ray.origin + v * (e / c.max(1e-8)));
    }
    let s = (b * e - c * d) / denominator;
    let t = (a * e - b * d) / denominator;
    (origin + u * s, ray.origin + v * t)
}

// --- pivots --------------------------------------------------------------

/// §03 §2.4. `Corner`, `CylinderAxis` and `SphereCenter` derive from picked
/// geometry and arrive with M8's BREP support; the first three are what M1
/// needs and are exact.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PivotMode {
    /// The node's own origin — where the CAD system put it, which is often
    /// what a mechanical part should rotate about.
    Origin,
    /// The centre of the selection's bounds. The sensible default for staging.
    #[default]
    BoundingBoxCenter,
    /// The min corner of the selection's bounds.
    Corner,
    /// Set by the user with `Insert`+drag.
    Custom,
}

impl PivotMode {
    pub fn label(self) -> &'static str {
        match self {
            PivotMode::Origin => "Origin",
            PivotMode::BoundingBoxCenter => "Bounding box centre",
            PivotMode::Corner => "Corner",
            PivotMode::Custom => "Custom",
        }
    }
}

/// Where the gizmo sits for a selection.
///
/// Multi-selection always uses the combined bounds, whatever the mode says:
/// "the origin" of six parts is not a point, and picking one of them
/// arbitrarily would move the gizmo when the selection order changed.
pub fn pivot_for(
    document: &SceneDocument,
    selection: &[NodeId],
    mode: PivotMode,
    custom: Option<Vec3>,
) -> Option<Vec3> {
    if selection.is_empty() {
        return None;
    }
    if mode == PivotMode::Custom {
        // An explicit override wins; otherwise the node's own stored pivot.
        // Only a single selection has one — several nodes have several, and
        // silently picking one of them would move the gizmo when the
        // selection order changed.
        if let Some(point) = custom {
            return Some(point);
        }
        if selection.len() == 1 {
            if let Some(local) = document.tree.get(selection[0]).and_then(|n| n.pivot) {
                return Some(document.tree.world_transform(selection[0]).point(local));
            }
        }
    }
    if mode == PivotMode::Origin && selection.len() == 1 {
        let id = selection[0];
        return Some(document.tree.world_transform(id).point(Vec3::ZERO));
    }

    let bounds = selection_bounds(document, selection)?;
    Some(match mode {
        PivotMode::Corner => bounds.min,
        _ => bounds.center(),
    })
}

/// World-space bounds of a selection, or `None` if none of it has geometry.
pub fn selection_bounds(document: &SceneDocument, selection: &[NodeId]) -> Option<Aabb> {
    let mut bounds = Aabb::EMPTY;
    for &id in selection {
        bounds.extend(
            &document
                .tree
                .bounds(id, &|mesh| document.geometry.get(mesh).map(|m| m.bounds())),
        );
    }
    (!bounds.is_empty()).then_some(bounds)
}

/// §5's snap-to-ground: "drop the object onto the ground plane — a one-click
/// operation people use constantly in product staging".
///
/// Returns the transform the node should have, or `None` when it has no
/// geometry to sit on. Moves in Y only: dropping something should not slide it
/// sideways.
pub fn snap_to_ground(document: &SceneDocument, node: NodeId, ground_y: f32) -> Option<Transform> {
    let bounds = selection_bounds(document, &[node])?;
    let lift = ground_y - bounds.min.y;
    if lift.abs() < 1e-7 {
        return None;
    }
    // The lift is in world space; the node's transform is relative to its
    // parent, so it has to be expressed in the parent's frame or a rotated
    // parent would send the part sideways.
    let parent = document.tree.get(node).and_then(|n| n.parent);
    let local_lift = match parent {
        Some(parent) => {
            let inverse = document.tree.world_transform(parent).inverted();
            inverse.matrix.transform_vector(vec3(0.0, lift, 0.0))
        }
        None => vec3(0.0, lift, 0.0),
    };
    let current = document.tree.get(node)?.transform;
    Transform::from_matrix(Mat4::translation(local_lift) * current.matrix)
}

/// Nearest vertex to `point` within `radius`, for §5's vertex snapping.
///
/// Searches world-space positions across every visible part. Linear, which is
/// fine for a single hover query on an M1-sized scene and would need the BVH
/// for a 4,000-part assembly — noted, not yet needed.
pub fn nearest_vertex(document: &SceneDocument, point: Vec3, radius: f32) -> Option<Vec3> {
    // Run on every frame of a drag with vertex snapping on. A part whose
    // box is farther than the radius has no vertex within it, so only the
    // parts near the point have their vertices looked at - not every vertex
    // of the assembly, each frame.
    let placed = document.placements();
    let mut best: Option<(f32, Vec3)> = None;
    for node in document.tree.iter() {
        let NodeKind::Part { mesh } = node.kind else {
            continue;
        };
        // Only what is on screen: not a part a model set hides either.
        if !placed.shown(node.id) {
            continue;
        }
        let Some(mesh) = document.geometry.get(mesh) else {
            continue;
        };
        let to_world = placed.world(node.id);
        let reach = best.map_or(radius, |(d, _)| d.min(radius));
        // With a hair of slack, so rounding at the box's edge cannot pass
        // over a vertex on it.
        if distance_to_box(&mesh.bounds().transformed(&to_world), point) > reach * (1.0 + 1e-5) + 1e-6 {
            continue;
        }
        for local in &mesh.positions {
            let world = to_world.point(*local);
            let distance = (world - point).length();
            if distance <= radius && best.is_none_or(|(d, _)| distance < d) {
                best = Some((distance, world));
            }
        }
    }
    best.map(|(_, point)| point)
}

/// How far `point` is from the box: 0 inside it.
fn distance_to_box(bounds: &Aabb, point: Vec3) -> f32 {
    if bounds.is_empty() {
        return f32::INFINITY;
    }
    let outside = (bounds.min - point).max(Vec3::ZERO).max(point - bounds.max);
    outside.length()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{reserve_keys, Command, ImportPayload};
    use crate::geometry::Mesh;
    use crate::tree::SceneNode;
    use crate::CommandBus;

    /// Passing over the parts whose boxes are out of reach finds what
    /// looking at every vertex finds, near a part, between parts, inside
    /// one, and out of reach of all.
    #[test]
    fn the_nearest_vertex_is_the_one_every_vertex_would_give() {
        let mut document = crate::demo::demo_document(crate::demo::DemoMaterial::Plastic, 0.3);
        let mut bus = CommandBus::new();
        let sphere = document.tree.iter().find(|n| n.mesh().is_some()).unwrap().id;
        let demo = document.tree.roots()[0];
        // Copies of the sphere, spread out, one turned.
        for i in 0..6 {
            bus.apply(&mut document, Command::AddGroup { parent: None, name: format!("Copy {i}") }).unwrap();
        }
        let groups: Vec<NodeId> = document.tree.roots().iter().copied().filter(|&r| r != demo).collect();
        let mesh = document.tree.get(sphere).unwrap().mesh().unwrap();
        let (_, _, ids) = reserve_keys(&document, 0, 0, groups.len());
        let nodes: Vec<SceneNode> = groups
            .iter()
            .zip(&ids)
            .map(|(&group, &id)| {
                let mut part = SceneNode::new(id, "Part", NodeKind::Part { mesh });
                part.parent = Some(group);
                part
            })
            .collect();
        for (node, group) in nodes.into_iter().zip(&groups) {
            let mut payload = ImportPayload { meshes: Vec::new(), textures: Vec::new(), materials: Vec::new(), nodes: vec![node.clone()], roots: Vec::new() };
            payload.nodes[0].parent = Some(*group);
            bus.apply(&mut document, Command::Import { payload: Box::new(payload) }).unwrap();
        }
        let moves: Vec<(NodeId, Transform)> = groups
            .iter()
            .enumerate()
            .map(|(i, &g)| (g, Transform::translation(vec3(0.3 * i as f32, 0.1 * (i % 3) as f32, -0.2 * i as f32)) * Transform::rotation(vec3(0.0, 1.0, 0.0), 0.4 * i as f32)))
            .collect();
        bus.apply(&mut document, Command::SetTransform { transforms: moves }).unwrap();
        let every = |document: &SceneDocument, point: Vec3, radius: f32| -> Option<Vec3> {
            let mut best: Option<(f32, Vec3)> = None;
            for node in document.tree.iter() {
                let NodeKind::Part { mesh } = node.kind else { continue };
                if !document.is_shown(node.id) {
                    continue;
                }
                let world = document.tree.world_transform(node.id);
                for local in &document.geometry.get(mesh).unwrap().positions {
                    let at = world.point(*local);
                    let distance = (at - point).length();
                    if distance <= radius && best.is_none_or(|(d, _)| distance < d) {
                        best = Some((distance, at));
                    }
                }
            }
            best.map(|(_, p)| p)
        };
        let mut found = 0;
        for i in 0..400 {
            let t = i as f32 * 0.37;
            let point = vec3((t * 1.3).sin() * 1.2 + 0.8, (t * 0.7).cos() * 0.4, (t * 0.9).sin() * 1.1 - 0.6);
            for radius in [0.01, 0.05, 0.3] {
                let expected = every(&document, point, radius);
                found += expected.is_some() as usize;
                assert_eq!(nearest_vertex(&document, point, radius), expected, "at {point:?} within {radius}");
            }
        }
        assert!(found > 50, "the points were not near enough to anything to test: {found}");

        // A part a model set hides is not snapped to: it is not on screen.
        let sets = crate::studio::ModelSets {
            sets: vec![
                crate::studio::ModelSet { name: "The first copies".into(), nodes: groups[..3].to_vec(), always_visible: false },
                crate::studio::ModelSet { name: "The others".into(), nodes: groups[3..].to_vec(), always_visible: false },
            ],
            active: Some(0),
        };
        bus.apply(&mut document, Command::SetModelSets { model_sets: Box::new(sets) }).unwrap();
        let hidden = document.tree.get(groups[5]).unwrap().children[0];
        let on_it = document.tree.world_transform(hidden).point(document.geometry.get(mesh).unwrap().positions[0]);
        assert!(!document.is_shown(hidden));
        assert_ne!(nearest_vertex(&document, on_it, 0.001), Some(on_it), "snapped to a part the model set hides");
        for i in 0..100 {
            let t = i as f32 * 0.61;
            let point = vec3((t * 1.3).sin() * 1.2 + 0.8, (t * 0.7).cos() * 0.4, (t * 0.9).sin() * 1.1 - 0.6);
            assert_eq!(nearest_vertex(&document, point, 0.3), every(&document, point, 0.3), "with a model set, at {point:?}");
        }
    }

    /// Looking down -Z from (0, 0, 10), a ray through a point on the z=0 plane.
    fn ray_at(x: f32, y: f32) -> Ray {
        let origin = vec3(0.0, 0.0, 10.0);
        Ray::new(origin, (vec3(x, y, 0.0) - origin).normalize())
    }

    const VIEW: Vec3 = vec3(0.0, 0.0, -1.0);

    #[test]
    fn plane_scale_changes_only_the_two_plane_axes() {
        for axis in Axis::ALL {
            let frame = GizmoFrame::default();
            let (u, v) = plane_axes(&frame, axis);
            let normal = frame.direction(axis);
            let origin = normal * 10.0;
            let ray = |p: Vec3| Ray::new(origin, (p - origin).normalize());
            let drag = begin(Handle::Plane(axis), GizmoMode::Scale, frame,
                Transform::IDENTITY, &ray((u + v) * 0.3), -normal);
            let result = update(&drag, &ray((u + v) * 0.6), &Snapping::OFF);
            assert!((result.vector(u) - u * 2.0).length() < 1e-4);
            assert!((result.vector(v) - v * 2.0).length() < 1e-4);
            assert!((result.vector(normal) - normal).length() < 1e-4);
        }
    }

    #[test]
    fn plane_snapping_does_not_move_the_normal_axis() {
        let start = Transform::translation(vec3(0.13, 0.27, 0.37));
        let drag = begin(Handle::Plane(Axis::Z), GizmoMode::Move,
            GizmoFrame::world(start.point(Vec3::ZERO), 1.0), start,
            &ray_at(0.2, 0.3), VIEW);
        let moved = update(&drag, &ray_at(1.2, 1.3), &Snapping { grid: Some(1.0), ..Snapping::OFF });
        assert!((moved.point(Vec3::ZERO).z - 0.37).abs() < 1e-6);
    }

    #[test]
    fn an_axis_arrow_is_picked_along_its_shaft_and_not_past_the_tip() {
        let frame = GizmoFrame::world(Vec3::ZERO, 1.0);

        assert_eq!(
            pick(&ray_at(0.6, 0.0), &frame, GizmoMode::Move, VIEW),
            Some(Handle::Axis(Axis::X))
        );
        assert_eq!(
            pick(&ray_at(0.0, 0.6), &frame, GizmoMode::Move, VIEW),
            Some(Handle::Axis(Axis::Y))
        );
        assert_eq!(
            pick(&ray_at(1.6, 0.0), &frame, GizmoMode::Move, VIEW),
            None,
            "past the tip of the arrow"
        );
        assert_eq!(
            pick(&ray_at(0.6, 0.5), &frame, GizmoMode::Move, VIEW),
            None,
            "well off any handle"
        );
    }

    /// The axis shafts pass through the plane quads, so testing axes first
    /// would make the quads unreachable.
    #[test]
    fn a_plane_quad_wins_over_the_axis_that_passes_through_it() {
        let frame = GizmoFrame::world(Vec3::ZERO, 1.0);
        // Inside the XY quad, and also within the X shaft's radius? No — but
        // squarely inside the quad, which is the case that matters.
        assert_eq!(
            pick(&ray_at(0.35, 0.35), &frame, GizmoMode::Move, VIEW),
            Some(Handle::Plane(Axis::Z)),
            "the XY quad has normal Z"
        );
    }

    #[test]
    fn the_screen_handle_sits_at_the_centre() {
        let frame = GizmoFrame::world(Vec3::ZERO, 1.0);
        assert_eq!(
            pick(&ray_at(0.02, 0.02), &frame, GizmoMode::Move, VIEW),
            Some(Handle::Screen)
        );
    }

    /// Rotation rings are circles at the handle radius, not filled discs:
    /// clicking the middle of a ring must not grab it.
    #[test]
    fn a_rotation_ring_is_picked_at_its_radius_only() {
        let frame = GizmoFrame::world(Vec3::ZERO, 1.0);
        // The Z ring lies in the XY plane, so it is the one facing the camera.
        assert_eq!(
            pick(&ray_at(1.0, 0.0), &frame, GizmoMode::Rotate, VIEW),
            Some(Handle::Axis(Axis::Z))
        );
        assert_eq!(
            pick(&ray_at(0.4, 0.0), &frame, GizmoMode::Rotate, VIEW),
            None,
            "inside the ring is not on it"
        );
    }

    #[test]
    fn an_axis_drag_moves_along_that_axis_and_no_other() {
        let frame = GizmoFrame::world(Vec3::ZERO, 1.0);
        let drag = begin(
            Handle::Axis(Axis::X),
            GizmoMode::Move,
            frame,
            Transform::IDENTITY,
            &ray_at(0.5, 0.0),
            VIEW,
        );

        // The cursor moves diagonally; only the component along the handle
        // may be taken.
        let moved = update(&drag, &ray_at(2.5, 1.7), &Snapping::OFF).point(Vec3::ZERO);
        assert!(moved.x > 0.0, "the object moved the wrong way: {moved:?}");
        assert!(moved.y.abs() < 1e-4, "the drag leaked into y: {}", moved.y);
        assert!(moved.z.abs() < 1e-4, "the drag leaked into z: {}", moved.z);

        // Further along the axis is further, and the mapping is monotonic —
        // the property that makes a drag feel connected to the cursor.
        let further = update(&drag, &ray_at(4.0, 1.7), &Snapping::OFF).point(Vec3::ZERO);
        assert!(further.x > moved.x);
        let back = update(&drag, &ray_at(-1.0, 1.7), &Snapping::OFF).point(Vec3::ZERO);
        assert!(back.x < 0.0);
    }

    /// How far along the axis a drag lands is the point on the axis closest to
    /// the cursor ray — the standard construction, and the one that stays
    /// well-behaved when the axis points nearly at the camera. Under a
    /// perspective camera it is not the same as the cursor's position on some
    /// plane, so this pins the geometry down with a distant camera, where the
    /// two agree and the expected answer can be read off directly.
    #[test]
    fn an_axis_drag_tracks_the_cursor_along_the_handle() {
        let frame = GizmoFrame::world(Vec3::ZERO, 1.0);
        // Far enough away that the rays are all but parallel.
        let far = |x: f32| {
            let origin = vec3(0.0, 0.0, 10_000.0);
            Ray::new(origin, (vec3(x, 0.0, 0.0) - origin).normalize())
        };
        let drag = begin(
            Handle::Axis(Axis::X),
            GizmoMode::Move,
            frame,
            Transform::IDENTITY,
            &far(0.5),
            VIEW,
        );

        let moved = update(&drag, &far(2.5), &Snapping::OFF).point(Vec3::ZERO);
        assert!(
            (moved.x - 2.0).abs() < 1e-3,
            "expected the object to follow the cursor's 2 units, got {}",
            moved.x
        );
    }

    /// The object must not jump by the distance between the cursor and the
    /// handle centre on the first frame.
    #[test]
    fn a_drag_that_has_not_moved_changes_nothing() {
        let frame = GizmoFrame::world(Vec3::ZERO, 1.0);
        let ray = ray_at(0.5, 0.0);
        let drag = begin(
            Handle::Axis(Axis::X),
            GizmoMode::Move,
            frame,
            Transform::IDENTITY,
            &ray,
            VIEW,
        );
        let result = update(&drag, &ray, &Snapping::OFF);
        assert!(
            result.point(Vec3::ZERO).length() < 1e-5,
            "the object jumped to {:?}",
            result.point(Vec3::ZERO)
        );
    }

    /// Grid snapping snaps the resulting *position*, so an object that starts
    /// off-grid ends up on it.
    #[test]
    fn grid_snapping_lands_on_the_grid_not_a_grid_multiple_away() {
        let frame = GizmoFrame::world(Vec3::ZERO, 1.0);
        let start = Transform::translation(vec3(0.3, 0.0, 0.0));
        let drag = begin(
            Handle::Axis(Axis::X),
            GizmoMode::Move,
            frame,
            start,
            &ray_at(0.5, 0.0),
            VIEW,
        );
        let snapping = Snapping {
            grid: Some(1.0),
            ..Snapping::OFF
        };

        let moved = update(&drag, &ray_at(2.4, 0.0), &snapping).point(Vec3::ZERO);
        assert!(
            (moved.x - 2.0).abs() < 1e-4,
            "expected a whole grid position, got {}",
            moved.x
        );
    }

    /// Snapping must not ratchet: holding still after a snapped move keeps the
    /// object where it is rather than nudging it another increment per frame.
    #[test]
    fn a_snapped_drag_does_not_ratchet_while_the_cursor_is_still() {
        let frame = GizmoFrame::world(Vec3::ZERO, 1.0);
        let drag = begin(
            Handle::Axis(Axis::X),
            GizmoMode::Move,
            frame,
            Transform::IDENTITY,
            &ray_at(0.5, 0.0),
            VIEW,
        );
        let snapping = Snapping {
            grid: Some(0.5),
            ..Snapping::OFF
        };

        let held = ray_at(1.7, 0.0);
        let first = update(&drag, &held, &snapping).point(Vec3::ZERO);
        let second = update(&drag, &held, &snapping).point(Vec3::ZERO);
        let third = update(&drag, &held, &snapping).point(Vec3::ZERO);
        assert_eq!(first, second);
        assert_eq!(second, third);
    }

    #[test]
    fn rotation_turns_by_the_angle_the_cursor_swept() {
        let frame = GizmoFrame::world(Vec3::ZERO, 1.0);
        let drag = begin(
            Handle::Axis(Axis::Z),
            GizmoMode::Rotate,
            frame,
            Transform::IDENTITY,
            &ray_at(1.0, 0.0),
            VIEW,
        );

        // Drag from +X round to +Y: a quarter turn about Z.
        let result = update(&drag, &ray_at(0.0, 1.0), &Snapping::OFF);
        let turned = result.point(vec3(1.0, 0.0, 0.0));
        assert!(
            (turned - vec3(0.0, 1.0, 0.0)).length() < 1e-4,
            "+X should have turned to +Y, got {turned:?}"
        );

        // And back the other way, which a plain `acos` would get wrong.
        let back = update(&drag, &ray_at(0.0, -1.0), &Snapping::OFF);
        let turned = back.point(vec3(1.0, 0.0, 0.0));
        assert!(
            (turned - vec3(0.0, -1.0, 0.0)).length() < 1e-4,
            "rotation only turns one way: {turned:?}"
        );
    }

    /// §5's 15° default.
    #[test]
    fn angle_snapping_quantizes_the_turn() {
        let frame = GizmoFrame::world(Vec3::ZERO, 1.0);
        let drag = begin(
            Handle::Axis(Axis::Z),
            GizmoMode::Rotate,
            frame,
            Transform::IDENTITY,
            &ray_at(1.0, 0.0),
            VIEW,
        );
        // 20° of cursor sweep snaps to 15°.
        let twenty = 20f32.to_radians();
        let result = update(
            &drag,
            &ray_at(twenty.cos(), twenty.sin()),
            &Snapping::default(),
        );
        let turned = result.point(vec3(1.0, 0.0, 0.0));
        let angle = turned.y.atan2(turned.x).to_degrees();
        assert!((angle - 15.0).abs() < 0.1, "snapped to {angle}°");
    }

    #[test]
    fn scaling_grows_with_the_distance_from_the_pivot() {
        let frame = GizmoFrame::world(Vec3::ZERO, 1.0);
        let drag = begin(
            Handle::Axis(Axis::X),
            GizmoMode::Scale,
            frame,
            Transform::IDENTITY,
            &ray_at(1.0, 0.0),
            VIEW,
        );
        let result = update(&drag, &ray_at(2.0, 0.0), &Snapping::OFF);
        let point = result.point(vec3(1.0, 1.0, 0.0));
        assert!((point.x - 2.0).abs() < 1e-4, "x scaled to {}", point.x);
        assert!(
            (point.y - 1.0).abs() < 1e-4,
            "y must not scale: {}",
            point.y
        );
    }

    /// A zero scale makes the transform non-invertible, which the renderer
    /// cannot use for normals. The gizmo must refuse rather than produce it.
    #[test]
    fn scaling_never_reaches_zero_or_flips() {
        let frame = GizmoFrame::world(Vec3::ZERO, 1.0);
        let drag = begin(
            Handle::Axis(Axis::X),
            GizmoMode::Scale,
            frame,
            Transform::IDENTITY,
            &ray_at(1.0, 0.0),
            VIEW,
        );
        for x in [0.0, -0.5, -3.0] {
            let result = update(&drag, &ray_at(x, 0.0), &Snapping::OFF);
            let point = result.point(vec3(1.0, 0.0, 0.0));
            assert!(
                point.x > 0.0,
                "dragging to {x} flipped or collapsed the part: {point:?}"
            );
            // And it stays invertible.
            assert!(Transform::from_matrix(result.matrix).is_some());
        }
    }

    /// Rotating about a pivot away from the origin must turn the object in
    /// place, not swing it across the scene.
    #[test]
    fn a_rotation_happens_about_the_pivot_not_the_origin() {
        let pivot = vec3(5.0, 0.0, 0.0);
        let frame = GizmoFrame::world(pivot, 1.0);
        let start = Transform::translation(pivot);
        let ray = Ray::new(
            vec3(5.0, 0.0, 10.0),
            (vec3(6.0, 0.0, 0.0) - vec3(5.0, 0.0, 10.0)).normalize(),
        );
        let drag = begin(
            Handle::Axis(Axis::Z),
            GizmoMode::Rotate,
            frame,
            start,
            &ray,
            VIEW,
        );

        let to = Ray::new(
            vec3(5.0, 0.0, 10.0),
            (vec3(5.0, 1.0, 0.0) - vec3(5.0, 0.0, 10.0)).normalize(),
        );
        let result = update(&drag, &to, &Snapping::OFF);
        let centre = result.point(Vec3::ZERO);
        assert!(
            (centre - pivot).length() < 1e-3,
            "the object left its pivot: {centre:?}"
        );
    }

    #[test]
    fn a_local_frame_is_orthonormal_even_under_a_non_uniform_scale() {
        let matrix = Mat4::rotation(Vec3::Y, 0.7) * Mat4::scale(vec3(3.0, 1.0, 0.2));
        let transform = Transform::from_matrix(matrix).unwrap();
        let frame = GizmoFrame::local(Vec3::ZERO, transform, 1.0);

        for axis in Axis::ALL {
            assert!((frame.direction(axis).length() - 1.0).abs() < 1e-5);
        }
        assert!(frame.basis[0].dot(frame.basis[1]).abs() < 1e-5);
        assert!(frame.basis[1].dot(frame.basis[2]).abs() < 1e-5);
        assert!(frame.basis[0].dot(frame.basis[2]).abs() < 1e-5);
    }

    // --- pivots and ground snapping --------------------------------------

    /// A part whose mesh sits from y=1 to y=3, placed at `offset`.
    fn scene_with_a_box(offset: Vec3) -> (SceneDocument, NodeId) {
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();
        let mesh = Mesh::new(
            vec![
                vec3(-1.0, 1.0, -1.0),
                vec3(1.0, 1.0, -1.0),
                vec3(1.0, 3.0, -1.0),
            ],
            vec![0, 1, 2],
        );
        let (meshes, _, ids) = reserve_keys(&document, 1, 0, 1);
        let mut node = SceneNode::new(ids[0], "Box", NodeKind::Part { mesh: meshes[0] });
        node.transform = Transform::translation(offset);

        bus.apply(
            &mut document,
            Command::Import {
                payload: Box::new(ImportPayload {
                    meshes: vec![(meshes[0], mesh)],
                    textures: Vec::new(),
                    materials: Vec::new(),
                    nodes: vec![node],
                    roots: vec![ids[0]],
                }),
            },
        )
        .unwrap();
        (document, ids[0])
    }

    #[test]
    fn the_default_pivot_is_the_centre_of_the_bounds() {
        let (document, id) = scene_with_a_box(Vec3::ZERO);
        let pivot = pivot_for(&document, &[id], PivotMode::BoundingBoxCenter, None).unwrap();
        assert!((pivot.y - 2.0).abs() < 1e-5, "pivot at {pivot:?}");

        let corner = pivot_for(&document, &[id], PivotMode::Corner, None).unwrap();
        assert!((corner.y - 1.0).abs() < 1e-5);

        // Origin mode uses where the CAD system put the part, which here is
        // two units below the geometry.
        let origin = pivot_for(&document, &[id], PivotMode::Origin, None).unwrap();
        assert!(origin.length() < 1e-5, "origin pivot at {origin:?}");
    }

    /// "The origin" of several parts is not a point, so a multi-selection uses
    /// the combined bounds whatever the mode says.
    #[test]
    fn a_multi_selection_pivots_on_the_combined_bounds() {
        let (mut document, first) = scene_with_a_box(Vec3::ZERO);
        let mut bus = CommandBus::new();
        let mesh = document.geometry.keys().next().unwrap();
        let (_, _, ids) = reserve_keys(&document, 0, 0, 1);
        let mut node = SceneNode::new(ids[0], "Box 2", NodeKind::Part { mesh });
        node.transform = Transform::translation(vec3(10.0, 0.0, 0.0));
        bus.apply(
            &mut document,
            Command::Import {
                payload: Box::new(ImportPayload {
                    meshes: Vec::new(),
                    textures: Vec::new(),
                    materials: Vec::new(),
                    nodes: vec![node],
                    roots: vec![ids[0]],
                }),
            },
        )
        .unwrap();

        let pivot = pivot_for(&document, &[first, ids[0]], PivotMode::Origin, None).unwrap();
        assert!(
            (pivot.x - 5.0).abs() < 1e-4,
            "expected the midpoint, got {pivot:?}"
        );
    }

    /// §04 §5's custom pivot, stored on the node so it survives a save.
    #[test]
    fn a_custom_pivot_comes_from_the_node_and_moves_with_it() {
        let (mut document, id) = scene_with_a_box(vec3(3.0, 0.0, 0.0));
        let mut bus = CommandBus::new();

        // Without one, Custom falls back to the derived centre.
        let fallback = pivot_for(&document, &[id], PivotMode::Custom, None).unwrap();
        let centre = pivot_for(&document, &[id], PivotMode::BoundingBoxCenter, None).unwrap();
        assert_eq!(fallback, centre);

        // Set one in the node's local space.
        bus.apply(
            &mut document,
            Command::set_pivot(id, Some(vec3(1.0, 1.0, 0.0))),
        )
        .unwrap();
        let pivot = pivot_for(&document, &[id], PivotMode::Custom, None).unwrap();
        assert!(
            (pivot - vec3(4.0, 1.0, 0.0)).length() < 1e-5,
            "the local pivot was not taken into world space: {pivot:?}"
        );

        // Moving the part moves its pivot with it, which is the point of
        // storing it in local space.
        bus.apply(
            &mut document,
            Command::set_transform(id, Transform::translation(vec3(10.0, 0.0, 0.0))),
        )
        .unwrap();
        let pivot = pivot_for(&document, &[id], PivotMode::Custom, None).unwrap();
        assert!(
            (pivot - vec3(11.0, 1.0, 0.0)).length() < 1e-5,
            "the pivot stayed behind when the part moved: {pivot:?}"
        );

        // And clearing returns it to the derived pivot.
        bus.apply(&mut document, Command::set_pivot(id, None))
            .unwrap();
        assert_eq!(
            pivot_for(&document, &[id], PivotMode::Custom, None),
            pivot_for(&document, &[id], PivotMode::BoundingBoxCenter, None)
        );
    }

    /// Several nodes have several pivots, and picking one arbitrarily would
    /// move the gizmo when the user changed the order they clicked in.
    #[test]
    fn a_multi_selection_ignores_stored_pivots() {
        let (mut document, first) = scene_with_a_box(Vec3::ZERO);
        let mut bus = CommandBus::new();
        let mesh = document.geometry.keys().next().unwrap();
        let (_, _, ids) = reserve_keys(&document, 0, 0, 1);
        let mut node = SceneNode::new(ids[0], "Box 2", NodeKind::Part { mesh });
        node.transform = Transform::translation(vec3(10.0, 0.0, 0.0));
        bus.apply(
            &mut document,
            Command::Import {
                payload: Box::new(ImportPayload {
                    meshes: Vec::new(),
                    textures: Vec::new(),
                    materials: Vec::new(),
                    nodes: vec![node],
                    roots: vec![ids[0]],
                }),
            },
        )
        .unwrap();
        bus.apply(
            &mut document,
            Command::set_pivot(first, Some(vec3(0.0, 5.0, 0.0))),
        )
        .unwrap();

        let both = [first, ids[0]];
        assert_eq!(
            pivot_for(&document, &both, PivotMode::Custom, None),
            pivot_for(&document, &both, PivotMode::BoundingBoxCenter, None),
            "one node's pivot was used for the whole selection"
        );
    }

    /// The one-click drop of §5.
    #[test]
    fn snap_to_ground_drops_the_part_onto_the_plane_without_sliding_it() {
        let (mut document, id) = scene_with_a_box(vec3(3.0, 7.0, -2.0));
        let before = document.tree.world_transform(id).point(Vec3::ZERO);

        let dropped = snap_to_ground(&document, id, 0.0).expect("it is above the ground");
        let mut bus = CommandBus::new();
        bus.apply(&mut document, Command::set_transform(id, dropped))
            .unwrap();

        let bounds = selection_bounds(&document, &[id]).unwrap();
        assert!(
            bounds.min.y.abs() < 1e-4,
            "the part did not land on the ground: {}",
            bounds.min.y
        );
        let after = document.tree.world_transform(id).point(Vec3::ZERO);
        assert!(
            (after.x - before.x).abs() < 1e-5 && (after.z - before.z).abs() < 1e-5,
            "dropping slid the part sideways: {before:?} -> {after:?}"
        );
    }

    #[test]
    fn snapping_a_part_already_on_the_ground_is_a_no_op() {
        let (document, id) = scene_with_a_box(vec3(0.0, -1.0, 0.0));
        assert_eq!(snap_to_ground(&document, id, 0.0), None);
    }

    #[test]
    fn vertex_snapping_finds_the_nearest_vertex_within_range() {
        let (document, _) = scene_with_a_box(Vec3::ZERO);

        let found = nearest_vertex(&document, vec3(0.9, 1.1, -1.0), 0.5);
        assert_eq!(found, Some(vec3(1.0, 1.0, -1.0)));

        assert_eq!(
            nearest_vertex(&document, vec3(50.0, 0.0, 0.0), 0.5),
            None,
            "nothing is within range"
        );
    }

    /// Hidden parts are not snapping targets: snapping to something the user
    /// cannot see is indistinguishable from the gizmo misbehaving.
    #[test]
    fn vertex_snapping_ignores_hidden_geometry() {
        let (mut document, id) = scene_with_a_box(Vec3::ZERO);
        let mut bus = CommandBus::new();
        bus.apply(&mut document, Command::set_visibility(id, false))
            .unwrap();

        assert_eq!(nearest_vertex(&document, vec3(1.0, 1.0, -1.0), 0.5), None);
    }
}
