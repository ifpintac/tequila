//! A medium whose density varies through a part: a Volume Map's grid,
//! fitted to the part (`docs/02` §6).
//!
//! A part whose material drives Density from a Volume Map node gets one of
//! these per instance, in the snapshot. The walk through its medium samples
//! it with null collisions — tracking against the medium at the grid's
//! densest, and at each tentative collision keeping it with probability
//! `density here / densest` — which is unbiased however the density varies,
//! and needs nothing of the grid but a lookup and its maximum.

use tequila_core::math::{Transform, Vec3};
use tequila_materials::graph::{CompiledGraph, Instruction, TextureId};
use tequila_materials::material_type::Param;
use tequila_texture::volume::VolumeGrid;
use std::sync::Arc;

/// A density grid placed in one part.
#[derive(Clone, Debug)]
pub struct DensityField {
    grid: Arc<VolumeGrid>,
    /// The part's placement, to take world positions into its own space.
    transform: Transform,
    /// The part's own bounding box, which the grid's box is stretched over.
    object_low: Vec3,
    object_size: Vec3,
    grid_low: Vec3,
    grid_size: Vec3,
}

impl DensityField {
    /// `grid` fitted to the box `object` (the part's, in its own space), for
    /// a part placed by `transform`. `None` for a box or grid with no extent.
    pub fn fitted(grid: Arc<VolumeGrid>, object: (Vec3, Vec3), transform: Transform) -> Option<DensityField> {
        let (grid_low, grid_high) = grid.bounds();
        let object_size = object.1 - object.0;
        let grid_size = grid_high - grid_low;
        let positive = |v: Vec3| v.x > 0.0 && v.y > 0.0 && v.z > 0.0 && v.x.is_finite() && v.y.is_finite() && v.z.is_finite();
        (positive(object_size) && positive(grid_size)).then_some(DensityField {
            grid,
            transform,
            object_low: object.0,
            object_size,
            grid_low,
            grid_size,
        })
    }

    /// Where a world position is in the grid's own units.
    fn in_grid(&self, world: Vec3) -> Vec3 {
        let local = self.transform.inv_point(world);
        let t = Vec3 {
            x: (local.x - self.object_low.x) / self.object_size.x,
            y: (local.y - self.object_low.y) / self.object_size.y,
            z: (local.z - self.object_low.z) / self.object_size.z,
        };
        Vec3 {
            x: self.grid_low.x + t.x * self.grid_size.x,
            y: self.grid_low.y + t.y * self.grid_size.y,
            z: self.grid_low.z + t.z * self.grid_size.z,
        }
    }

    /// Walk the stretch `from..to` of a ray through the grid's majorant
    /// blocks, calling `visit(start, end, bound)` for each piece inside the
    /// blocks in order, until it returns false. Outside them the density is
    /// zero and nothing is visited. The map from world to grid is affine, so
    /// the ray is a straight line in block coordinates and the walk is the
    /// ordinary one through a regular grid.
    pub fn march(&self, origin: Vec3, direction: Vec3, from: f32, to: f32, mut visit: impl FnMut(f32, f32, f32) -> bool) {
        let u0 = self.grid.block_coordinates(self.in_grid(origin));
        let u1 = self.grid.block_coordinates(self.in_grid(origin + direction));
        let du = [u1.x - u0.x, u1.y - u0.y, u1.z - u0.z];
        let start = [u0.x, u0.y, u0.z];
        let size = self.grid.block_resolution;
        // Clip to the blocks' box.
        let (mut low, mut high) = (from, to);
        for a in 0..3 {
            let n = size[a] as f32;
            if du[a].abs() < 1e-12 {
                if start[a] < 0.0 || start[a] >= n {
                    return;
                }
            } else {
                let (t0, t1) = ((0.0 - start[a]) / du[a], (n - start[a]) / du[a]);
                low = low.max(t0.min(t1));
                high = high.min(t0.max(t1));
            }
        }
        if !(low < high) {
            return;
        }
        let mid = low + (high - low) * 1e-6;
        let mut cell = [0i64; 3];
        let mut next = [f32::INFINITY; 3];
        let mut delta = [f32::INFINITY; 3];
        let mut step = [0i64; 3];
        for a in 0..3 {
            cell[a] = ((start[a] + du[a] * mid).floor() as i64).clamp(0, size[a] as i64 - 1);
            if du[a] > 0.0 {
                step[a] = 1;
                next[a] = ((cell[a] + 1) as f32 - start[a]) / du[a];
                delta[a] = 1.0 / du[a];
            } else if du[a] < 0.0 {
                step[a] = -1;
                next[a] = (cell[a] as f32 - start[a]) / du[a];
                delta[a] = -1.0 / du[a];
            }
        }
        let mut t = low;
        for _ in 0..(size[0] + size[1] + size[2] + 3) {
            let axis = if next[0] <= next[1] && next[0] <= next[2] {
                0
            } else if next[1] <= next[2] {
                1
            } else {
                2
            };
            let end = next[axis].min(high);
            if end > t && !visit(t, end, self.grid.block_of(cell[0], cell[1], cell[2])) {
                return;
            }
            if end >= high {
                return;
            }
            t = end;
            cell[axis] += step[axis];
            next[axis] += delta[axis];
            if cell[axis] < 0 || cell[axis] >= size[axis] as i64 {
                return;
            }
        }
    }

    /// The density at a world position.
    pub fn at(&self, world: Vec3) -> f32 {
        let local = self.transform.inv_point(world);
        let t = Vec3 {
            x: (local.x - self.object_low.x) / self.object_size.x,
            y: (local.y - self.object_low.y) / self.object_size.y,
            z: (local.z - self.object_low.z) / self.object_size.z,
        };
        let in_grid = Vec3 {
            x: self.grid_low.x + t.x * self.grid_size.x,
            y: self.grid_low.y + t.y * self.grid_size.y,
            z: self.grid_low.z + t.z * self.grid_size.z,
        };
        self.grid.sample(in_grid).max(0.0)
    }

    /// The densest the field gets, which bounds [`DensityField::at`].
    pub fn max(&self) -> f32 {
        self.grid.max.max(0.0)
    }
}

/// The Volume Map a program's Density is wired to, if it is: directly, as the
/// Material tab wires it.
pub fn volume_of(program: &CompiledGraph) -> Option<TextureId> {
    let (_, register) = program.driven.iter().find(|(p, _)| *p == Param::VolumeDensity)?;
    program.instructions.iter().rev().find(|i| i.destination() == *register).and_then(|i| match i {
        Instruction::VolumeMap { volume, .. } => Some(*volume),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::{vec3, Mat4};

    /// A bound below the density anywhere makes a collision's "probability"
    /// exceed one, and the tracking quietly biased. So along random rays
    /// through a random grid, placed at an angle in a scaled part, every
    /// point of every piece the march visits must sit at or under that
    /// piece's bound — including the half-voxel rims where a lookup reaches
    /// across a block's edge.
    #[test]
    fn no_lookup_exceeds_its_block_bound() {
        let mut state = 12345u64;
        let mut random = || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((state >> 33) as f32) / (1u64 << 31) as f32
        };
        let resolution = [19, 23, 17];
        let data: Vec<f32> = (0..resolution.iter().product::<usize>())
            .map(|_| if random() < 0.7 { 0.0 } else { random() * 3.0 })
            .collect();
        let grid = Arc::new(VolumeGrid::new("density".into(), data, resolution, vec3(0.1, -0.2, 0.3), vec3(0.05, 0.07, 0.04)));
        let placement = Transform::from_matrix(
            Mat4::translation(vec3(0.5, 1.0, -0.3)) * Mat4::rotation(vec3(0.3, 1.0, 0.2).normalize(), 0.7),
        )
        .unwrap();
        let field = DensityField::fitted(grid, (vec3(-1.0, -0.5, -0.8), vec3(1.2, 0.9, 0.6)), placement).unwrap();
        let mut visited = 0;
        for _ in 0..300 {
            let origin = placement.point(vec3(random() * 4.0 - 2.0, random() * 4.0 - 2.0, random() * 4.0 - 2.0));
            let direction = vec3(random() - 0.5, random() - 0.5, random() - 0.5).normalize();
            field.march(origin, direction, 0.0, 6.0, |start, end, bound| {
                for i in 0..=16 {
                    let t = start + (end - start) * i as f32 / 16.0;
                    let density = field.at(origin + direction * t);
                    assert!(density <= bound + 1e-4, "{density} above its block's bound {bound}");
                }
                visited += 1;
                true
            });
        }
        assert!(visited > 100, "the rays hardly met the grid: {visited} pieces");
    }
}
