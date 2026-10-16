//! What an auto-save costs on a big scene: copying the document (which stays
//! on the window's thread) against writing it (which can go elsewhere).
//! `cargo run --release -p tequila-format --example autosave_cost`

use tequila_core::math::{Vec2, Vec3};
use tequila_scene::geometry::Mesh;
use std::time::Instant;

fn main() {
    let side = 600usize;
    let mut positions = Vec::with_capacity(side * side);
    let mut normals = Vec::with_capacity(side * side);
    let mut uvs = Vec::with_capacity(side * side);
    for y in 0..side {
        for x in 0..side {
            let (u, v) = (x as f32 / side as f32, y as f32 / side as f32);
            positions.push(Vec3 { x: u, y: (u * 40.0).sin() * 0.01, z: v });
            normals.push(Vec3 { x: 0.0, y: 1.0, z: 0.0 });
            uvs.push(Vec2 { x: u, y: v });
        }
    }
    let mut indices = Vec::with_capacity((side - 1) * (side - 1) * 6);
    for y in 0..side - 1 {
        for x in 0..side - 1 {
            let i = (y * side + x) as u32;
            let s = side as u32;
            indices.extend_from_slice(&[i, i + s, i + 1, i + 1, i + s, i + s + 1]);
        }
    }
    let triangles = indices.len() / 3;
    let mut document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.3);
    document.geometry.insert_deduplicated(Mesh::new(positions, indices).with_normals(normals).with_uvs(uvs));

    let path = std::env::temp_dir().join(format!("tequila-autosave-cost-{}.tequila", std::process::id()));
    for round in 0..3 {
        let start = Instant::now();
        let copy = document.clone();
        let copied = start.elapsed();
        let start = Instant::now();
        tequila_format::save(&path, &copy, &tequila_format::Preserved::default()).unwrap();
        let written = start.elapsed();
        println!(
            "round {round}: {triangles} triangles - copy {:.1} ms, write {:.1} ms ({:.1} MB)",
            copied.as_secs_f64() * 1e3,
            written.as_secs_f64() * 1e3,
            std::fs::metadata(&path).unwrap().len() as f64 / 1e6
        );
    }
    let _ = std::fs::remove_file(&path);
}
