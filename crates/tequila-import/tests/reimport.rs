//! An imported model, updated from a newer version of its file (`docs/03`
//! §2.3), end to end through the importer: the keys and the source file it
//! records are what the update matches by.

use tequila_import::{build_read, import, read_file, ImportOptions};
use tequila_materials::material_type::MaterialType;
use tequila_scene::command::ImportPayload;
use tequila_scene::material::MaterialDefinition;
use tequila_scene::reimport::{import_roots, plan, source_file, SOURCE_FILE};
use tequila_scene::tree::MaterialKey;
use tequila_scene::{Command, CommandBus, SceneDocument};

fn obj(parts: &[(&str, f32)]) -> String {
    let mut text = String::new();
    let mut base = 1;
    for (name, size) in parts {
        text.push_str(&format!(
            "o {name}\nv 0 0 0\nv {size} 0 0\nv 0 {size} 0\nv 0 0 {size}\nf {} {} {}\nf {} {} {}\n",
            base,
            base + 1,
            base + 2,
            base,
            base + 2,
            base + 3
        ));
        base += 4;
    }
    text
}

#[test]
fn a_model_updated_from_its_file_keeps_its_materials() {
    let directory = std::env::temp_dir().join(format!("tequila-reimport-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let file = directory.join("bracket.obj");
    std::fs::write(&file, obj(&[("Housing", 1.0), ("Screw", 0.1), ("Clip", 0.2)])).unwrap();

    let mut document = SceneDocument::new();
    let mut bus = CommandBus::new();
    let (command, _) = import(&document, None, &file, &ImportOptions::default()).expect("imports");
    bus.apply(&mut document, command).unwrap();

    let housing = document.tree.iter().find(|n| n.name == "Housing").expect("a housing").id;
    assert!(document.tree.iter().all(|n| n.path_key.is_some()), "a node has no key");
    let roots = import_roots(&document, housing);
    let recorded = source_file(&document, &roots).expect("the file is recorded");
    assert!(recorded.ends_with("bracket.obj"), "{recorded}");
    assert!(roots.iter().all(|&r| document.tree.get(r).unwrap().metadata.contains_key(SOURCE_FILE)));

    // The user's own material on the housing.
    let own = MaterialKey(document.materials.next_key());
    bus.apply(
        &mut document,
        Command::Import {
            payload: Box::new(ImportPayload {
                meshes: Vec::new(),
                textures: Vec::new(),
                nodes: Vec::new(),
                roots: Vec::new(),
                materials: vec![(own, MaterialDefinition::new("Anodised", MaterialType::Metal, MaterialType::Metal.defaults()))],
            }),
        },
    )
    .unwrap();
    bus.apply(&mut document, Command::AssignMaterial { nodes: vec![housing], material: own }).unwrap();

    // The next revision, saved over the same file: a bigger housing, no
    // clip, a new lid.
    std::fs::write(&file, obj(&[("Housing", 2.0), ("Screw", 0.1), ("Lid", 1.0)])).unwrap();
    let parent = document.tree.get(roots[0]).unwrap().parent;
    let options = ImportOptions { place_on_ground: false, ..ImportOptions::default() };
    let (Command::Import { payload }, _) = build_read(&document, parent, read_file(std::path::Path::new(&recorded)).unwrap(), &options).unwrap() else {
        panic!("an import");
    };
    let (update, report) = plan(&document, &roots, *payload);
    assert_eq!(report.added, vec!["Lid".to_string()]);
    assert_eq!(report.removed.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>(), vec!["Clip"]);
    bus.apply(&mut document, update).unwrap();

    let after = document.tree.get(housing).expect("the housing kept its id");
    assert_eq!(after.material, Some(own), "the housing lost its material");
    let mesh = document.geometry.get(after.mesh().unwrap()).unwrap();
    let widest = mesh.positions.iter().map(|p| p.x).fold(0.0f32, f32::max);
    assert!(widest > 1.5, "the housing kept its old shape ({widest})");
    assert!(document.tree.iter().any(|n| n.name == "Lid" && n.visible));
    assert!(document.tree.iter().any(|n| n.name == "Clip" && !n.visible));
    let _ = std::fs::remove_dir_all(&directory);
}

/// A real revision series, for looking at: TEQUILA_REIMPORT_EVAL is
/// `old|new`, two versions of one model. Prints what matched.
#[test]
#[ignore = "an evaluation on files outside the repository; run by hand"]
fn eval_update_between_two_revisions() {
    let Ok(pair) = std::env::var("TEQUILA_REIMPORT_EVAL") else { return };
    let (old, new) = pair.split_once('|').expect("old|new");
    let mut document = SceneDocument::new();
    let mut bus = CommandBus::new();
    let (command, report) = import(&document, None, std::path::Path::new(old), &ImportOptions::default()).expect("imports");
    bus.apply(&mut document, command).unwrap();
    println!("imported {} nodes from {old}", report.nodes);
    let part = document.tree.iter().find(|n| n.mesh().is_some()).expect("a part").id;
    let roots = import_roots(&document, part);
    let parent = document.tree.get(roots[0]).unwrap().parent;
    let options = ImportOptions { place_on_ground: false, ..ImportOptions::default() };
    let started = std::time::Instant::now();
    let (Command::Import { payload }, _) = build_read(&document, parent, read_file(std::path::Path::new(new)).unwrap(), &options).unwrap() else {
        panic!("an import");
    };
    let (update, report) = plan(&document, &roots, *payload);
    bus.apply(&mut document, update).unwrap();
    println!("updated from {new} in {:.2}s", started.elapsed().as_secs_f64());
    println!("matched {}, added {:?}, removed {:?}", report.matched, report.added, report.removed.iter().map(|(_, n)| n).collect::<Vec<_>>());
    for node in document.tree.iter() {
        println!("  {} {:?} visible={}", node.name, node.path_key, node.visible);
    }
}
