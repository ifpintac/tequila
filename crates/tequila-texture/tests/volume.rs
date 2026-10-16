//! Reading OpenVDB files.
//!
//! The fixtures in `data/` were written by OpenVDB itself (12.0, as shipped
//! inside Blender 5.0) by `data/make_vdb_fixtures.py`, with contents chosen so
//! a test can say what every voxel holds:
//!
//! - `ramp.vdb`: 32³ active voxels at voxel size 0.05, index x from -8 to 23,
//!   y from 0 to 31, z from -4 to 27, each holding `(x + 8) * 0.01 + 0.001`.
//! - `ramp_half.vdb`: the same, stored as half floats.
//! - `sphere.vdb`: 1.0 in every voxel within 10 of the origin, voxel size
//!   0.1, so a ball a metre in radius.

use tequila_core::math::vec3;
use tequila_texture::volume::{read_vdb, VolumeGrid};

fn fixture(name: &str) -> VolumeGrid {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name);
    read_vdb(&std::fs::read(&path).expect("fixture present")).expect("reads")
}

fn ramp_at(x: i32) -> f32 {
    (x + 8) as f32 * 0.01 + 0.001
}

#[test]
fn every_voxel_reads_back_where_the_file_put_it() {
    let grid = fixture("ramp.vdb");
    assert_eq!(grid.name, "density");
    assert_eq!(grid.resolution, [32, 32, 32]);
    for (x, y, z) in [(-8, 0, -4), (0, 5, 0), (7, 31, 27), (23, 16, 11)] {
        let at = vec3(x as f32 * 0.05, y as f32 * 0.05, z as f32 * 0.05);
        let got = grid.sample(at);
        assert!((got - ramp_at(x)).abs() < 1e-5, "voxel ({x}, {y}, {z}) reads {got}, not {}", ramp_at(x));
    }
    assert!((grid.max - ramp_at(23)).abs() < 1e-5);
}

#[test]
fn between_voxels_is_trilinear_and_outside_is_empty() {
    let grid = fixture("ramp.vdb");
    // Halfway between x = 2 and x = 3.
    let got = grid.sample(vec3(2.5 * 0.05, 10.0 * 0.05, 3.0 * 0.05));
    assert!((got - (ramp_at(2) + ramp_at(3)) * 0.5).abs() < 1e-5, "{got}");
    // Well outside, and the bounds are half a voxel past the outermost centres.
    assert_eq!(grid.sample(vec3(5.0, 0.5, 0.5)), 0.0);
    let (low, high) = grid.bounds();
    assert!((low.x - (-8.5 * 0.05)).abs() < 1e-5 && (high.x - (23.5 * 0.05)).abs() < 1e-5, "{low:?} {high:?}");
}

#[test]
fn half_float_grids_read_as_the_same_field() {
    let full = fixture("ramp.vdb");
    let half = fixture("ramp_half.vdb");
    assert_eq!(full.resolution, half.resolution);
    for (a, b) in full.data.iter().zip(&half.data) {
        assert!((a - b).abs() < 1e-3, "{a} against {b}");
    }
}

#[test]
fn a_fog_sphere_is_a_metre_across_where_the_file_says() {
    let grid = fixture("sphere.vdb");
    assert!((grid.sample(vec3(0.0, 0.0, 0.0)) - 1.0).abs() < 1e-6);
    assert!((grid.sample(vec3(0.0, 0.9, 0.0)) - 1.0).abs() < 1e-6);
    assert_eq!(grid.sample(vec3(0.0, 1.2, 0.0)), 0.0);
    assert_eq!(grid.sample(vec3(0.8, 0.8, 0.0)), 0.0, "a corner of the box is outside the ball");
}

#[test]
fn a_file_that_is_not_a_vdb_is_an_error_not_a_panic() {
    assert!(read_vdb(b"definitely not a volume").is_err());
    assert!(read_vdb(&[]).is_err());
}
