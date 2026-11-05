//! Pins on a part's UV layout: vertices held where they are when an island
//! is laid out flat again (`uv.reflatten`), relaxed or reshaped round them.
//!
//! Kept on the part (`SceneNode::uv_pins`), as its guides are, rather than
//! in the editor: a pin put down is still there when the project is opened
//! again, and putting it down or taking it up is a step to undo like any
//! other edit (`uv.set_pins`). Nothing that renders reads them.
//!
//! A pin is a mesh vertex, by its number, on one of the part's UV sets - a
//! vertex standing for every corner on the same place of that layout, as
//! the `uv.*` commands name vertices. So the pins follow the layout's
//! edits: a cut keeps every vertex's number and the pins with them; an
//! unwrap or a new set numbers the vertices anew, and the pins go to the
//! new numbers of the same corners ([`carried`]); a set copied takes its
//! pins with it, a set deleted takes them away, a set moved keeps them.

use crate::geometry::Mesh;
use std::collections::BTreeMap;

/// A part's pins: for each UV set that has any, its pinned mesh vertices,
/// in order and each once.
pub type UvPins = BTreeMap<usize, Vec<u32>>;

/// The pins put in order: each set's vertices sorted and once each, and a
/// set with none left out, so two lists of the same pins are equal.
pub fn tidy(pins: UvPins) -> UvPins {
    pins.into_iter()
        .filter_map(|(set, mut vertices)| {
            vertices.sort_unstable();
            vertices.dedup();
            (!vertices.is_empty()).then_some((set, vertices))
        })
        .collect()
}

/// The pins after an edit made `after` of `before`, with each old set
/// going to the sets `sets` gives it - none for a set deleted, two for one
/// copied.
///
/// Where the edit kept the triangles in their order, as every edit of a
/// layout does and an unwrap too, a pin goes to the vertices now at the
/// corners its vertex was at: the same vertex where the numbers were kept,
/// both sides of a cut through it, the new numbers after an unwrap. An edit
/// that made other triangles keeps the pins that are still vertices.
pub fn carried(pins: &UvPins, before: &Mesh, after: &Mesh, sets: impl Fn(usize) -> Vec<usize>) -> UvPins {
    let vertices = after.vertex_count() as u32;
    let same_corners = before.indices.len() == after.indices.len();
    let mut out = UvPins::new();
    for (&set, pinned) in pins {
        let moved: Vec<u32> = if same_corners {
            let held: std::collections::HashSet<u32> = pinned.iter().copied().collect();
            before.indices.iter().zip(&after.indices).filter(|(old, _)| held.contains(old)).map(|(_, &new)| new).collect()
        } else {
            pinned.iter().copied().filter(|&v| v < vertices).collect()
        };
        for to in sets(set) {
            if to < after.uv_sets.len() {
                out.entry(to).or_default().extend(moved.iter().copied());
            }
        }
    }
    tidy(out)
}

/// Where each set goes in a change to the sets: the set's own new number,
/// and the copy's too for the set copied; nothing for the set deleted.
pub fn through_change(change: super::SetChange) -> impl Fn(usize) -> Vec<usize> {
    move |old| match change {
        super::SetChange::Delete { set } if set == old => Vec::new(),
        super::SetChange::Copy { from, to } if from == old => vec![change.renumber(old), to],
        _ => vec![change.renumber(old)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::tests_support::*;
    use crate::uv::SetChange;
    use tequila_core::math::vec2;

    #[test]
    fn pins_are_kept_in_order_and_once_each() {
        let pins = tidy(UvPins::from([(1, vec![5, 2, 5]), (0, Vec::new())]));
        assert_eq!(pins, UvPins::from([(1, vec![2, 5])]));
    }

    /// Copied with their set, gone with it, moved with it.
    #[test]
    fn pins_follow_their_set() {
        let mut mesh = cube();
        mesh.uv_sets = (0..3).map(|s| vec![vec2(s as f32, 0.0); mesh.positions.len()]).collect();
        let pins = UvPins::from([(0, vec![1]), (2, vec![3])]);
        let change = SetChange::Copy { from: 2, to: 0 };
        let copied = change.apply(&mesh).unwrap();
        assert_eq!(carried(&pins, &mesh, &copied, through_change(change)), UvPins::from([(0, vec![3]), (1, vec![1]), (3, vec![3])]));
        let change = SetChange::Delete { set: 0 };
        let deleted = change.apply(&mesh).unwrap();
        assert_eq!(carried(&pins, &mesh, &deleted, through_change(change)), UvPins::from([(1, vec![3])]));
        let change = SetChange::Move { from: 2, to: 0 };
        let moved = change.apply(&mesh).unwrap();
        assert_eq!(carried(&pins, &mesh, &moved, through_change(change)), UvPins::from([(0, vec![3]), (1, vec![1])]));
    }

    /// A new set numbers the vertices anew: each pin goes to the vertices
    /// at the corners it was at, which sit where it sat on its layout.
    #[test]
    fn pins_go_to_the_same_corners_when_the_vertices_are_numbered_anew() {
        let mut mesh = tube(8, 2);
        mesh.uv_sets = vec![mesh.positions.iter().map(|p| vec2(p.x * 0.3 + 0.5, p.y * 0.3 + 0.2)).collect()];
        let pins = UvPins::from([(0, vec![4, 11])]);
        let change = SetChange::Add;
        let added = change.apply(&mesh).unwrap();
        let after = carried(&pins, &mesh, &added, through_change(change));
        let moved = &after[&0];
        assert!(moved.len() >= 2, "{moved:?}");
        for &v in moved {
            let place = added.uv_sets[0][v as usize];
            assert!([4usize, 11].iter().any(|&old| (mesh.uv_sets[0][old] - place).length() < 1e-6), "a pin sits where one sat");
        }
        for (c, &old) in mesh.indices.iter().enumerate() {
            if pins[&0].contains(&old) {
                assert!(moved.contains(&added.indices[c]), "every corner a pin was at is pinned");
            }
        }
    }
}
