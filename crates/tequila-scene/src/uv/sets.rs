//! A part's UV sets: added, copied, deleted and put in another order.
//!
//! A set is read by number - a label's or a texture's `Mapping.uv_set` -
//! so a change that renumbers them says, by [`SetChange::renumber`], which
//! number each old one has after it; the command that makes the change
//! renumbers the mappings of what the part wears with it.
//!
//! The first set is the one the tangent frames follow, and the one a
//! Blender-style Texture Coordinate node reads: a change that puts another
//! set first changes what those read.

use super::edit::UvError;
use crate::geometry::Mesh;
use tequila_core::math::{vec2, Vec2, Vec3};

/// A change to a part's list of UV sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetChange {
    /// A new set at the end: the part seen flat down its thinnest axis,
    /// fitted into the unit square.
    Add,
    /// A copy of set `from`, put at `to` among them; those from there on
    /// move up one.
    Copy { from: usize, to: usize },
    /// Set `set` gone; those after it move down one.
    Delete { set: usize },
    /// Set `from` taken out and put back at `to`.
    Move { from: usize, to: usize },
}

impl SetChange {
    /// The number set `old` has after the change. A mapping on a deleted
    /// set reads the first set after it, which is what a part with no such
    /// set gave it anyway.
    pub fn renumber(&self, old: usize) -> usize {
        match *self {
            SetChange::Add => old,
            SetChange::Copy { to, .. } => if old >= to { old + 1 } else { old },
            SetChange::Delete { set } => {
                if old == set {
                    0
                } else if old > set {
                    old - 1
                } else {
                    old
                }
            }
            SetChange::Move { from, to } => {
                if old == from {
                    to
                } else if from < old && old <= to {
                    old - 1
                } else if to <= old && old < from {
                    old + 1
                } else {
                    old
                }
            }
        }
    }

    /// The mesh with its sets changed.
    pub fn apply(&self, mesh: &Mesh) -> Result<Mesh, UvError> {
        let count = mesh.uv_sets.len();
        let mut out = mesh.clone();
        let first_before = mesh.uv_sets.first().cloned();
        match *self {
            SetChange::Add => out.uv_sets.push(flat_projection(&mesh.positions)),
            SetChange::Copy { from, to } => {
                if from >= count {
                    return Err(UvError::NoSuchSet(from));
                }
                if to > count {
                    return Err(UvError::NoSuchSet(to));
                }
                let copy = mesh.uv_sets[from].clone();
                out.uv_sets.insert(to, copy);
            }
            SetChange::Delete { set } => {
                if set >= count {
                    return Err(UvError::NoSuchSet(set));
                }
                out.uv_sets.remove(set);
            }
            SetChange::Move { from, to } => {
                if from >= count {
                    return Err(UvError::NoSuchSet(from));
                }
                if to >= count {
                    return Err(UvError::NoSuchSet(to));
                }
                let set = out.uv_sets.remove(from);
                out.uv_sets.insert(to, set);
            }
        }
        // The tangent frames follow the first set.
        if out.uv_sets.first() != first_before.as_ref() {
            if out.uv_sets.is_empty() {
                out.tangents.clear();
            } else if !mesh.tangents.is_empty() && !out.generate_tangents() {
                out.tangents.clear();
            }
        }
        Ok(out)
    }
}

/// The part seen flat down its thinnest axis, from the positive side, fitted
/// into the unit square with its proportions kept and reading the right way.
fn flat_projection(positions: &[Vec3]) -> Vec<Vec2> {
    let (mut low, mut high) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for p in positions {
        low = low.min(*p);
        high = high.max(*p);
    }
    if positions.is_empty() {
        return Vec::new();
    }
    let size = high - low;
    // Across and down, seen from the positive side of the thinnest axis.
    let (across, down): (fn(Vec3) -> f32, fn(Vec3) -> f32) = if size.x <= size.y && size.x <= size.z {
        (|p| -p.z, |p| -p.y)
    } else if size.y <= size.z {
        (|p| p.x, |p| p.z)
    } else {
        (|p| p.x, |p| -p.y)
    };
    let flat: Vec<Vec2> = positions.iter().map(|p| vec2(across(*p), down(*p))).collect();
    let (mut lo, mut hi) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
    for p in &flat {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    let side = (hi - lo).x.max((hi - lo).y).max(1e-12);
    flat.into_iter().map(|p| (p - lo) / side).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::tests_support::*;

    fn with_sets(n: usize) -> Mesh {
        let mut mesh = cube();
        mesh.uv_sets = (0..n).map(|s| vec![vec2(s as f32, 0.0); mesh.positions.len()]).collect();
        mesh
    }

    #[test]
    fn sets_are_added_copied_deleted_and_moved() {
        let mesh = with_sets(3);
        let added = SetChange::Add.apply(&mesh).unwrap();
        assert_eq!(added.uv_sets.len(), 4);
        assert!(added.uv_sets[3].iter().all(|p| (0.0..=1.0).contains(&p.x) && (0.0..=1.0).contains(&p.y)));
        let copied = SetChange::Copy { from: 2, to: 0 }.apply(&mesh).unwrap();
        assert_eq!(copied.uv_sets.iter().map(|s| s[0].x).collect::<Vec<_>>(), vec![2.0, 0.0, 1.0, 2.0]);
        let deleted = SetChange::Delete { set: 1 }.apply(&mesh).unwrap();
        assert_eq!(deleted.uv_sets.iter().map(|s| s[0].x).collect::<Vec<_>>(), vec![0.0, 2.0]);
        let moved = SetChange::Move { from: 0, to: 2 }.apply(&mesh).unwrap();
        assert_eq!(moved.uv_sets.iter().map(|s| s[0].x).collect::<Vec<_>>(), vec![1.0, 2.0, 0.0]);
        assert_eq!(SetChange::Delete { set: 3 }.apply(&mesh), Err(UvError::NoSuchSet(3)));
        assert_eq!(SetChange::Move { from: 0, to: 3 }.apply(&mesh), Err(UvError::NoSuchSet(3)));
    }

    /// Every set that survives keeps being read where it went.
    #[test]
    fn renumbering_follows_each_set_to_its_new_place() {
        let mesh = with_sets(4);
        let changes = [
            SetChange::Add,
            SetChange::Copy { from: 1, to: 1 },
            SetChange::Copy { from: 3, to: 4 },
            SetChange::Delete { set: 0 },
            SetChange::Delete { set: 2 },
            SetChange::Move { from: 0, to: 3 },
            SetChange::Move { from: 3, to: 1 },
            SetChange::Move { from: 2, to: 2 },
        ];
        for change in changes {
            let out = change.apply(&mesh).unwrap();
            for old in 0..4 {
                if change == (SetChange::Delete { set: old }) {
                    continue;
                }
                let new = change.renumber(old);
                assert_eq!(out.uv_sets[new][0].x, old as f32, "{change:?}: set {old} went to {new}");
            }
        }
    }

    /// A plate thin along each axis in turn, its new set seen from the
    /// positive side of that axis: the face there reads the right way.
    #[test]
    fn a_new_set_reads_the_right_way_round() {
        use tequila_core::math::vec3;
        for (axis, squash) in [(0usize, vec3(0.1, 1.0, 2.0)), (1, vec3(2.0, 0.1, 1.0)), (2, vec3(2.0, 1.0, 0.1))] {
            let mut mesh = cube();
            mesh.positions.iter_mut().for_each(|p| *p = *p * squash);
            mesh.uv_sets.clear();
            let out = SetChange::Add.apply(&mesh).unwrap();
            let uv = &out.uv_sets[0];
            let mut checked = 0;
            for t in 0..out.triangle_count() {
                let n = out.geometric_normal_scaled(t);
                if [n.x, n.y, n.z][axis] > 0.0 && n.length() > 0.0 && [n.x, n.y, n.z][axis] / n.length() > 0.9 {
                    let [a, b, c] = out.triangle(t).map(|v| uv[v as usize]);
                    assert!(crate::uv::flatten::facing(a, b, c) > 0.0, "axis {axis}, triangle {t}");
                    checked += 1;
                }
            }
            assert_eq!(checked, 2);
        }
    }
}
