//! The bundled sample model imports, and arrives as five named parts.
//!
//! A sample that does not open is worse than no sample: it is the first thing
//! anyone tries, and a failure there reads as the application being broken
//! rather than the file.

use tequila_scene::tree::NodeKind;
use tequila_scene::SceneDocument;

fn sample() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("the workspace root")
        .join("assets/samples/glasses.gltf")
}

#[test]
fn the_glasses_sample_imports_as_five_named_parts() {
    let document = SceneDocument::new();
    let (command, report) = tequila_import::import(
        &document,
        None,
        &sample(),
        &tequila_import::ImportOptions::default(),
    )
    .expect("the sample imports");

    let tequila_scene::Command::Import { payload } = &command else {
        panic!("import produced something other than an import command");
    };

    let parts: Vec<&str> = payload
        .nodes
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Part { .. }))
        .map(|n| n.name.as_str())
        .collect();

    assert_eq!(parts.len(), 5, "expected five parts, got {parts:?}");
    for expected in [
        "Lens Left",
        "Lens Right",
        "Frame",
        "Temple Left",
        "Temple Right",
    ] {
        assert!(parts.contains(&expected), "missing `{expected}`: {parts:?}");
    }

    // Every part has geometry and a material to replace.
    for node in payload.nodes.iter() {
        if let NodeKind::Part { mesh } = node.kind {
            let geometry = payload
                .meshes
                .iter()
                .find(|(key, _)| *key == mesh)
                .map(|(_, m)| m)
                .unwrap_or_else(|| panic!("`{}` references a missing mesh", node.name));
            assert!(
                geometry.indices.len() >= 3,
                "`{}` has no triangles",
                node.name
            );
            assert!(
                node.material.is_some(),
                "`{}` has no material, so nothing can be dropped onto it",
                node.name
            );
        }
    }

    assert!(report.nodes >= 5);
}

/// Built at life size, so the importer's unit guess lands on metres and the
/// camera frames it without anyone reaching for a scale factor.
#[test]
fn the_sample_is_life_sized() {
    let document = SceneDocument::new();
    let (command, _) = tequila_import::import(
        &document,
        None,
        &sample(),
        &tequila_import::ImportOptions::default(),
    )
    .expect("imports");

    let tequila_scene::Command::Import { payload } = &command else {
        panic!("not an import");
    };

    let mut lowest = f32::INFINITY;
    let mut highest = f32::NEG_INFINITY;
    for (_, mesh) in payload.meshes.iter() {
        for p in &mesh.positions {
            lowest = lowest.min(p.x);
            highest = highest.max(p.x);
        }
    }
    let width = highest - lowest;
    assert!(
        (0.10..0.20).contains(&width),
        "the glasses are {width:.3} units across; real ones are about 0.14 m"
    );
}
