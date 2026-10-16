//! Volume Map data (`docs/02` §6): a density grid, read from an OpenVDB file.
//!
//! VDB is what Houdini, Blender, EmberGen and every simulation tool write for
//! smoke, clouds and fog, and it is sparse — a tree of 8³ leaves under larger
//! nodes, with whole regions stored as single tiles. The renderer wants to ask
//! "how dense is it here" millions of times a frame, so the file is read once
//! into a dense grid over the bounding box of what is active in it, and
//! sampled trilinearly. Dense costs memory that sparse would not, so the
//! grid is held to [`MAX_VOXELS`], averaged down by whole factors to fit.
//!
//! Positions are in the file's own units (its voxel size and offset), which
//! is how the caller fits the grid to a part.

use tequila_core::math::{vec3, Vec3};
use std::io::Cursor;

/// The largest dense grid a volume is held to: 256³ floats, 64 MiB.
pub const MAX_VOXELS: usize = 256 * 256 * 256;

/// A scalar field over a box, dense, `x` fastest.
#[derive(Clone, Debug, PartialEq)]
pub struct VolumeGrid {
    /// The grid read from the file, by name.
    pub name: String,
    pub data: Vec<f32>,
    pub resolution: [usize; 3],
    /// Where voxel (0, 0, 0)'s centre is, in the file's units.
    pub origin: Vec3,
    /// A voxel's size along each axis, in the file's units.
    pub voxel_size: Vec3,
    /// The largest value anywhere in the grid, which bounds a medium's
    /// density for the tracking that samples it.
    pub max: f32,
    /// The largest value a trilinear lookup can return inside each block of
    /// [`BLOCK`]³ voxels — the local bound the tracking steps through, so
    /// that empty space costs nothing and thin wisps are not tracked at the
    /// density of the thickest part. See [`VolumeGrid::block_of`].
    pub blocks: Vec<f32>,
    pub block_resolution: [usize; 3],
}

/// Voxels along a side of one block of the majorant grid.
pub const BLOCK: usize = 8;

#[derive(Debug, Clone, PartialEq)]
pub enum VolumeError {
    /// Not a VDB file this reader understands.
    Unreadable(String),
    /// A VDB file with no scalar grid in it.
    NoScalarGrid,
    /// A grid with nothing active in it.
    Empty,
}

impl std::fmt::Display for VolumeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VolumeError::Unreadable(why) => write!(f, "not a readable OpenVDB file: {why}"),
            VolumeError::NoScalarGrid => write!(f, "the file has no scalar (float) grid"),
            VolumeError::Empty => write!(f, "the grid is empty"),
        }
    }
}

impl std::error::Error for VolumeError {}

/// Read the density grid from a VDB file: the grid named `density` if there
/// is one, as the tools name a fog volume's, else the first scalar grid.
pub fn read_vdb(bytes: &[u8]) -> Result<VolumeGrid, VolumeError> {
    let mut reader =
        vdb_rs::VdbReader::new(Cursor::new(bytes)).map_err(|e| VolumeError::Unreadable(e.to_string()))?;
    let mut names = reader.available_grids();
    names.sort();
    if let Some(at) = names.iter().position(|n| n == "density") {
        let density = names.remove(at);
        names.insert(0, density);
    }
    let mut why = None;
    for name in names {
        let grid = match reader.read_grid::<f32>(&name) {
            Ok(grid) => grid,
            Err(e) => {
                why.get_or_insert_with(|| format!("`{name}`: {e}"));
                continue;
            }
        };
        let (voxel_size, translation) = match &grid.transform {
            vdb_rs::Map::UniformScaleMap { voxel_size, .. } => ([voxel_size.x, voxel_size.y, voxel_size.z], [0.0; 3]),
            vdb_rs::Map::ScaleTranslateMap { voxel_size, translation, .. } => (
                [voxel_size.x, voxel_size.y, voxel_size.z],
                [translation.x, translation.y, translation.z],
            ),
        };
        let voxels: Vec<([i64; 3], i64, f32)> = grid
            .iter()
            .map(|(p, v, level)| ([p.x as i64, p.y as i64, p.z as i64], level.scale() as i64, v))
            .collect();
        return densify(name, &voxels, voxel_size, translation);
    }
    Err(why.map_or(VolumeError::NoScalarGrid, VolumeError::Unreadable))
}

/// Lay active voxels and tiles (`[index], edge, value`) into a dense grid.
fn densify(
    name: String,
    voxels: &[([i64; 3], i64, f32)],
    voxel_size: [f64; 3],
    translation: [f64; 3],
) -> Result<VolumeGrid, VolumeError> {
    if voxels.is_empty() {
        return Err(VolumeError::Empty);
    }
    let mut low = [i64::MAX; 3];
    let mut high = [i64::MIN; 3];
    for (at, edge, _) in voxels {
        for a in 0..3 {
            low[a] = low[a].min(at[a]);
            high[a] = high[a].max(at[a] + edge - 1);
        }
    }
    let full = [(high[0] - low[0] + 1) as usize, (high[1] - low[1] + 1) as usize, (high[2] - low[2] + 1) as usize];
    // The smallest whole factor that brings the grid under the cap.
    let mut factor = 1usize;
    while full.iter().map(|n| n.div_ceil(factor)).product::<usize>() > MAX_VOXELS {
        factor += 1;
    }
    let resolution = [full[0].div_ceil(factor), full[1].div_ceil(factor), full[2].div_ceil(factor)];
    let mut sum = vec![0.0f64; resolution[0] * resolution[1] * resolution[2]];
    for (at, edge, value) in voxels {
        for dz in 0..*edge {
            for dy in 0..*edge {
                for dx in 0..*edge {
                    let x = ((at[0] + dx - low[0]) as usize) / factor;
                    let y = ((at[1] + dy - low[1]) as usize) / factor;
                    let z = ((at[2] + dz - low[2]) as usize) / factor;
                    sum[(z * resolution[1] + y) * resolution[0] + x] += *value as f64;
                }
            }
        }
    }
    let per_cell = (factor * factor * factor) as f64;
    let data: Vec<f32> = sum.iter().map(|s| (s / per_cell) as f32).collect();
    let size = vec3(voxel_size[0] as f32, voxel_size[1] as f32, voxel_size[2] as f32);
    // A coarse cell's centre sits halfway across the fine voxels it averages.
    let first = vec3(low[0] as f32, low[1] as f32, low[2] as f32) + Vec3::splat((factor as f32 - 1.0) * 0.5);
    Ok(VolumeGrid::new(
        name,
        data,
        resolution,
        vec3(
            first.x * size.x + translation[0] as f32,
            first.y * size.y + translation[1] as f32,
            first.z * size.z + translation[2] as f32,
        ),
        size * factor as f32,
    ))
}

impl VolumeGrid {
    /// A grid of `data` (`x` fastest), with its maximum and its majorant
    /// blocks worked out.
    pub fn new(name: String, data: Vec<f32>, resolution: [usize; 3], origin: Vec3, voxel_size: Vec3) -> VolumeGrid {
        let max = data.iter().copied().fold(0.0f32, f32::max);
        // Blocks tile the voxel index range from -1 (where a lookup first
        // reaches voxel 0) upwards, BLOCK voxels a side. A lookup whose lower
        // corner is voxel v reads v and v + 1, so a block's bound is taken
        // over its own voxels and one more on the far side.
        let block_resolution = [
            (resolution[0] + 2).div_ceil(BLOCK),
            (resolution[1] + 2).div_ceil(BLOCK),
            (resolution[2] + 2).div_ceil(BLOCK),
        ];
        let mut blocks = vec![0.0f32; block_resolution.iter().product()];
        let [rx, ry, rz] = resolution;
        for bz in 0..block_resolution[2] {
            for by in 0..block_resolution[1] {
                for bx in 0..block_resolution[0] {
                    let span = |b: usize, n: usize| {
                        let first = (b * BLOCK) as i64 - 1;
                        (first.max(0) as usize)..((first + BLOCK as i64 + 1).clamp(0, n as i64) as usize)
                    };
                    let mut most = 0.0f32;
                    for z in span(bz, rz) {
                        for y in span(by, ry) {
                            for x in span(bx, rx) {
                                most = most.max(data[(z * ry + y) * rx + x]);
                            }
                        }
                    }
                    blocks[(bz * block_resolution[1] + by) * block_resolution[0] + bx] = most;
                }
            }
        }
        VolumeGrid { name, data, resolution, origin, voxel_size, max, blocks, block_resolution }
    }

    /// Where `position` (the file's units) is in block coordinates: block
    /// `(i, j, k)` spans `[i, i + 1)` on each axis. Linear in the position,
    /// so a straight ray is a straight line through the blocks.
    pub fn block_coordinates(&self, position: Vec3) -> Vec3 {
        vec3(
            ((position.x - self.origin.x) / self.voxel_size.x + 1.0) / BLOCK as f32,
            ((position.y - self.origin.y) / self.voxel_size.y + 1.0) / BLOCK as f32,
            ((position.z - self.origin.z) / self.voxel_size.z + 1.0) / BLOCK as f32,
        )
    }

    /// The bound in block `(i, j, k)`, and zero outside the blocks, where
    /// every lookup is zero.
    pub fn block_of(&self, i: i64, j: i64, k: i64) -> f32 {
        let [bx, by, bz] = self.block_resolution;
        if i < 0 || j < 0 || k < 0 || i >= bx as i64 || j >= by as i64 || k >= bz as i64 {
            return 0.0;
        }
        self.blocks[(k as usize * by + j as usize) * bx + i as usize]
    }

    /// The value at `position`, in the file's units: trilinear between voxel
    /// centres, and zero outside the grid.
    pub fn sample(&self, position: Vec3) -> f32 {
        let local = vec3(
            (position.x - self.origin.x) / self.voxel_size.x,
            (position.y - self.origin.y) / self.voxel_size.y,
            (position.z - self.origin.z) / self.voxel_size.z,
        );
        let base = [local.x.floor(), local.y.floor(), local.z.floor()];
        let t = [local.x - base[0], local.y - base[1], local.z - base[2]];
        let at = |x: f32, y: f32, z: f32| -> f32 {
            let (x, y, z) = (x as i64, y as i64, z as i64);
            let [rx, ry, rz] = self.resolution;
            if x < 0 || y < 0 || z < 0 || x >= rx as i64 || y >= ry as i64 || z >= rz as i64 {
                return 0.0;
            }
            self.data[(z as usize * ry + y as usize) * rx + x as usize]
        };
        let mut value = 0.0;
        for corner in 0..8 {
            let (dx, dy, dz) = ((corner & 1) as f32, ((corner >> 1) & 1) as f32, ((corner >> 2) & 1) as f32);
            let w = (if dx > 0.0 { t[0] } else { 1.0 - t[0] })
                * (if dy > 0.0 { t[1] } else { 1.0 - t[1] })
                * (if dz > 0.0 { t[2] } else { 1.0 - t[2] });
            if w > 0.0 {
                value += w * at(base[0] + dx, base[1] + dy, base[2] + dz);
            }
        }
        value
    }

    /// The box the grid's values reach, in the file's units: half a voxel
    /// past the outermost centres, where the trilinear filter reaches zero.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        let half = self.voxel_size * 0.5;
        let far = vec3(
            self.origin.x + (self.resolution[0] as f32 - 1.0) * self.voxel_size.x,
            self.origin.y + (self.resolution[1] as f32 - 1.0) * self.voxel_size.y,
            self.origin.z + (self.resolution[2] as f32 - 1.0) * self.voxel_size.z,
        );
        (self.origin - half, far + half)
    }
}
