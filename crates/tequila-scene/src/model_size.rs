//! A model's size put right: what its numbers are in, when the file did
//! not say (`docs/03-SCENE-AND-IO.md` §3.4).
//!
//! An STL written in millimetres comes in a thousand times too large,
//! because a document's lengths are metres and the file named no unit.
//! Putting it right changes the model's size and nothing else: its top
//! nodes are scaled about the middle of its base, so it stands where it
//! stood. What the Model units window does, and the `model_units` request.

use crate::tree::NodeId;
use crate::{Command, SceneDocument};
use tequila_core::math::{vec3, Aabb, Mat4, Transform, Vec3};

/// The world bounds of the parts under `roots`: `None` when there is no
/// geometry there to measure.
pub fn bounds_of(document: &SceneDocument, roots: &[NodeId]) -> Option<Aabb> {
    let mut bounds = Aabb::EMPTY;
    for &root in roots {
        bounds.extend(&document.tree.bounds(root, &|mesh| document.geometry.get(mesh).map(|m| m.bounds())));
    }
    (!bounds.is_empty() && (bounds.max - bounds.min).length() > 0.0).then_some(bounds)
}

/// The middle of a model's base: what scaling it leaves where it is.
pub fn base_of(bounds: &Aabb) -> Vec3 {
    let centre = bounds.center();
    vec3(centre.x, bounds.min.y, centre.z)
}

/// Scale the model under `roots` by `ratio` about `pivot`, in the world: one
/// command for all of them, so it is one step to undo.
pub fn rescale(document: &SceneDocument, roots: &[NodeId], pivot: Vec3, ratio: f32) -> Command {
    let about = Mat4::translation(pivot) * Mat4::scale(Vec3::splat(ratio)) * Mat4::translation(-pivot);
    let transforms = roots
        .iter()
        .filter_map(|&root| {
            let local = document.tree.get(root)?.transform;
            let world = document.tree.world_transform(root);
            // world = parent · local, so the new local is parent⁻¹ · about ·
            // world, and parent⁻¹ is local · world⁻¹.
            Transform::from_matrix(local.matrix * world.inverse * about * world.matrix).map(|t| (root, t))
        })
        .collect();
    Command::SetTransform { transforms }
}

/// A size as a person would say it: in millimetres, centimetres, metres or
/// kilometres by how big it is, to three figures.
pub fn real_size(metres: Vec3) -> String {
    let largest = metres.x.max(metres.y).max(metres.z);
    let (per, symbol) = match largest {
        l if l < 0.1 => (0.001, "mm"),
        l if l < 2.0 => (0.01, "cm"),
        l if l < 2000.0 => (1.0, "m"),
        _ => (1000.0, "km"),
    };
    let figure = |metres: f32| {
        let value = metres / per;
        let decimals = if value >= 100.0 { 0 } else if value >= 10.0 { 1 } else { 2 };
        let text = format!("{value:.decimals$}");
        if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.').to_string() } else { text }
    };
    format!("{} × {} × {} {symbol}", figure(metres.x), figure(metres.y), figure(metres.z))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_as_people_say_them() {
        assert_eq!(real_size(vec3(0.121, 0.110, 0.025)), "12.1 × 11 × 2.5 cm");
        assert_eq!(real_size(vec3(121.0, 110.0, 25.0)), "121 × 110 × 25 m");
        assert_eq!(real_size(vec3(0.004, 0.0025, 0.001)), "4 × 2.5 × 1 mm");
        assert_eq!(real_size(vec3(3.07, 2.79, 0.64)), "3.07 × 2.79 × 0.64 m");
    }
}
