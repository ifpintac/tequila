//! What one frame of a gizmo drag or a material slider costs on a big
//! assembly: the snapshot rebuilt with every mesh already converted.
//!
//! `cargo run --release -p tequila-scene --example snapshot_bench`

use tequila_core::math::{vec3, Transform};
use tequila_scene::command::{reserve_keys, Command, ImportPayload};
use tequila_scene::material::MaterialDefinition;
use tequila_scene::snapshot::{snapshot_with, MeshCache, ViewOverrides};
use tequila_scene::tree::{NodeKind, SceneNode};
use std::sync::Arc;
use std::time::Instant;

fn main() {
    // 5,000 parts, five to a group, groups five to a group, four levels deep:
    // the shape of a CAD assembly.
    let parts = 5000usize;
    let fan = 5usize;
    let mut document = tequila_scene::SceneDocument::new();
    let groups_per_level: Vec<usize> = (1..=3).map(|level| parts.div_ceil(fan.pow(level))).collect();
    let groups: usize = groups_per_level.iter().sum();
    let (meshes, materials, ids) = reserve_keys(&document, 1, 8, groups + parts);
    let mut nodes: Vec<SceneNode> = Vec::new();
    let mut next = 0usize;
    let mut level_ids: Vec<Vec<_>> = Vec::new();
    for (level, count) in groups_per_level.iter().rev().enumerate() {
        let mut this = Vec::new();
        for i in 0..*count {
            let mut group = SceneNode::new(ids[next], format!("Group {level}.{i}"), NodeKind::Group);
            next += 1;
            if let Some(parents) = level_ids.last() {
                group.parent = Some(parents[i / fan]);
            }
            group.transform = Transform::translation(vec3(0.01 * i as f32, 0.0, 0.0));
            this.push(group.id);
            nodes.push(group);
        }
        level_ids.push(this);
    }
    let leaves = level_ids.last().unwrap().clone();
    for i in 0..parts {
        let mut part = SceneNode::new(ids[next], format!("Part {i}"), NodeKind::Part { mesh: meshes[0] });
        next += 1;
        part.parent = Some(leaves[i / fan]);
        part.material = Some(materials[i % materials.len()]);
        part.transform = Transform::translation(vec3(0.0, 0.002 * i as f32, 0.0));
        nodes.push(part);
    }
    // Children lists, from the parents.
    let by_id: std::collections::HashMap<_, usize> = nodes.iter().enumerate().map(|(i, n)| (n.id, i)).collect();
    let links: Vec<(usize, _)> = nodes.iter().filter_map(|n| n.parent.map(|p| (by_id[&p], n.id))).collect();
    for (parent, child) in links {
        nodes[parent].children.push(child);
    }
    let roots = level_ids[0].clone();
    // A part of about a thousand vertices: a 32 by 32 patch, 1 cm across.
    let side = 32u32;
    let corners: Vec<_> = (0..side * side).map(|i| vec3((i % side) as f32 * 0.01 / side as f32, 0.0, (i / side) as f32 * 0.01 / side as f32)).collect();
    let mut faces = Vec::new();
    for y in 0..side - 1 {
        for x in 0..side - 1 {
            let i = y * side + x;
            faces.extend_from_slice(&[i, i + side, i + 1, i + 1, i + side, i + side + 1]);
        }
    }
    let cube = tequila_scene::geometry::Mesh::new(corners, faces);
    let mut bus = tequila_scene::CommandBus::new();
    bus.apply(
        &mut document,
        Command::Import {
            payload: Box::new(ImportPayload {
                meshes: vec![(meshes[0], cube)],
                textures: Vec::new(),
                materials: materials
                    .iter()
                    .enumerate()
                    .map(|(i, &key)| {
                        let plastic = tequila_render_core::material::Material::plastic(tequila_core::spectrum::Spectrum::splat(0.1 * i as f32), 0.3);
                        (key, MaterialDefinition::new(format!("M{i}"), tequila_materials::material_type::MaterialType::Plastic, plastic))
                    })
                    .collect(),
                nodes,
                roots,
            }),
        },
    )
    .expect("the assembly is well formed");

    let environment = Arc::new(tequila_scene::demo::studio_environment());
    let mut cache = MeshCache::new();
    let view = ViewOverrides::default();
    let _ = snapshot_with(&document, environment.clone(), &mut cache, &view);
    let mut best = f64::INFINITY;
    let mut total = 0.0;
    let rounds = 20;
    for _ in 0..rounds {
        let start = Instant::now();
        let (snapshot, _) = snapshot_with(&document, environment.clone(), &mut cache, &view);
        let ms = start.elapsed().as_secs_f64() * 1e3;
        std::hint::black_box(snapshot);
        best = best.min(ms);
        total += ms;
    }
    let best_of = |run: &dyn Fn()| {
        (0..rounds)
            .map(|_| {
                let start = Instant::now();
                run();
                start.elapsed().as_secs_f64() * 1e3
            })
            .fold(f64::INFINITY, f64::min)
    };
    println!("placements alone: {:.2} ms best", best_of(&|| {
        std::hint::black_box(document.placements());
    }));
    println!("node by node, as before: {:.2} ms best", best_of(&|| {
        for node in document.tree.iter() {
            std::hint::black_box((document.tree.world_transform(node.id), document.is_shown(node.id)));
        }
    }));
    let near = vec3(0.0, 0.5, 0.0);
    println!("vertex snap near one part: {:.3} ms best", best_of(&|| {
        std::hint::black_box(tequila_scene::gizmo::nearest_vertex(&document, near, 0.005));
    }));
    println!("vertex snap, every vertex as before: {:.3} ms best", best_of(&|| {
        let mut best = f32::INFINITY;
        for node in document.tree.iter() {
            let Some(mesh) = node.mesh() else { continue };
            if !document.tree.is_effectively_visible(node.id) {
                continue;
            }
            let world = document.tree.world_transform(node.id);
            for p in &document.geometry.get(mesh).unwrap().positions {
                best = best.min((world.point(*p) - near).length());
            }
        }
        std::hint::black_box(best);
    }));
    println!(
        "{} nodes ({parts} parts): snapshot rebuilt in {best:.2} ms best, {:.2} ms mean",
        document.tree.len(),
        total / rounds as f64
    );
}
