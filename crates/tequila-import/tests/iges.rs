//! IGES: a file read from disk as CAD - its bytes kept with the model, its
//! quality recorded - and divided again from them as an update.

use tequila_import::step::Tessellation;
use tequila_import::{build_read, read_cad, read_file, ImportOptions};
use tequila_scene::tree::NodeKind;
use tequila_scene::{CommandBus, SceneDocument};

/// Two surfaces: a quarter cylinder of radius 10 (an exact rational
/// NURBS) labelled SHELL, and a flat 10 x 10 patch beside it labelled
/// PLATE. Millimetres.
fn two_surfaces() -> Vec<u8> {
    let h = std::f64::consts::FRAC_1_SQRT_2;
    let entities = [
        ("SHELL", format!("128,2,1,2,1,0,0,0,0,0,0.,0.,0.,1.,1.,1.,0.,0.,1.,1.,1.,{h},1.,1.,{h},1.,10.,0.,0.,10.,10.,0.,0.,10.,0.,10.,0.,10.,10.,10.,10.,0.,10.,10.,0.,1.,0.,1.;")),
        ("PLATE", "128,1,1,1,1,0,0,1,0,0,0.,0.,1.,1.,0.,0.,1.,1.,1.,1.,1.,1.,20.,0.,0.,30.,0.,0.,20.,10.,0.,30.,10.,0.,0.,1.,0.,1.;".to_string()),
    ];
    let mut out = String::new();
    let line = |out: &mut String, data: &str, letter: char, n: usize| out.push_str(&format!("{data:<72}{letter}{n:>7}\n"));
    line(&mut out, "Tequila test", 'S', 1);
    let global = "1H,,1H;,4Htest,8Htest.igs,8HTequila,3H1.0,32,38,6,308,15,4Htest,1.,2,2HMM,1,0.1,15H20261011.120000,1.E-06,1000.,4Htest,4Htest,11,0,15H20261011.120000;";
    let g: Vec<&str> = global.as_bytes().chunks(72).map(|c| std::str::from_utf8(c).unwrap()).collect();
    for (i, chunk) in g.iter().enumerate() {
        line(&mut out, chunk, 'G', i + 1);
    }
    let mut p: Vec<String> = Vec::new();
    let mut directory = Vec::new();
    for (i, (label, record)) in entities.iter().enumerate() {
        let de = 2 * i + 1;
        let start = p.len() + 1;
        let mut current = String::new();
        for token in record.split_inclusive(',') {
            if current.len() + token.len() > 64 {
                p.push(format!("{current:<64} {de:>7}"));
                current.clear();
            }
            current.push_str(token);
        }
        p.push(format!("{current:<64} {de:>7}"));
        let count = p.len() + 1 - start;
        directory.push(format!("{:>8}{start:>8}{:>8}{:>8}{:>8}{:>8}{:>8}{:>8}{:>8}", 128, 0, 0, 0, 0, 0, 0, "00000000"));
        directory.push(format!("{:>8}{:>8}{:>8}{count:>8}{:>8}{:>8}{:>8}{label:>8}{:>8}", 128, 0, 0, 0, "", "", 0));
    }
    for (i, d) in directory.iter().enumerate() {
        line(&mut out, d, 'D', i + 1);
    }
    for (i, l) in p.iter().enumerate() {
        line(&mut out, l, 'P', i + 1);
    }
    line(&mut out, &format!("S{:>7}G{:>7}D{:>7}P{:>7}", 1, g.len(), directory.len(), p.len()), 'T', 1);
    out.into_bytes()
}

#[test]
fn an_iges_file_is_read_as_cad_and_kept() {
    let folder = std::env::temp_dir().join(format!("tequila-iges-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("Bracket.IGS");
    std::fs::write(&path, two_surfaces()).unwrap();
    let model = read_file(&path).expect("reads by its extension, whatever its case");
    std::fs::remove_dir_all(&folder).ok();
    assert!(model.scene.warnings.is_empty(), "{:?}", model.scene.warnings);
    assert_eq!(model.scene.tessellation.as_deref(), Some("Medium"));
    assert!(model.brep.is_some(), "the file's bytes were not kept to divide again");
    let names: Vec<&str> = model.scene.meshes.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["SHELL", "PLATE"]);
}

/// Dividing again from the kept bytes is an update: the shell keeps its
/// material and takes the finer triangles.
#[test]
fn re_dividing_an_iges_model_keeps_the_work_on_its_parts() {
    let bytes = two_surfaces();
    let mut document = SceneDocument::new();
    let mut bus = CommandBus::new();
    let model = read_cad(&bytes, "Bracket", None, Tessellation::Draft).expect("known as IGES by its content");
    let (command, _) = build_read(&document, None, model, &ImportOptions::default()).unwrap();
    bus.apply(&mut document, command).unwrap();
    let root = document.tree.roots()[0];
    let shell = document.tree.iter().find(|n| n.name == "SHELL").map(|n| n.id).unwrap();
    let gold = document.materials.insert(tequila_scene::material::MaterialDefinition::preset("Gold", tequila_materials::material_type::MaterialType::Metal));
    bus.apply(&mut document, tequila_scene::Command::AssignMaterial { nodes: vec![shell], material: gold }).unwrap();
    let triangles = |d: &SceneDocument| match d.tree.get(shell).unwrap().kind {
        NodeKind::Part { mesh } => d.geometry.get(mesh).unwrap().indices.len() / 3,
        _ => 0,
    };
    let before = triangles(&document);

    let finer = read_cad(&bytes, "Bracket", None, Tessellation::Ultra).unwrap();
    let options = ImportOptions { place_on_ground: false, ..Default::default() };
    let (command, _) = build_read(&document, None, finer, &options).unwrap();
    let tequila_scene::Command::Import { payload } = command else { panic!("an import") };
    let (update, report) = tequila_scene::reimport::plan(&document, &[root], *payload);
    bus.apply(&mut document, update).unwrap();
    assert!(report.added.is_empty() && report.removed.is_empty(), "the same parts: {report:?}");
    assert_eq!(document.tree.get(shell).unwrap().material, Some(gold), "the shell lost its material");
    assert!(triangles(&document) > before * 2, "{} triangles, from {before}", triangles(&document));
    let metadata = &document.tree.get(root).unwrap().metadata;
    assert_eq!(metadata.get(tequila_import::step::TESSELLATION).map(String::as_str), Some("Ultra"));
}
