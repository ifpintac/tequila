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

/// An IGES file of `entities` - type, label, level, colour number, the
/// parameter record, and whether another entity uses it - in millimetres.
/// Its strings are counted right, as OpenCASCADE, unlike Tequila's own
/// reader, insists.
fn iges(entities: &[(u32, &str, i64, i64, &str, bool)]) -> Vec<u8> {
    let mut out = String::new();
    let line = |out: &mut String, data: &str, letter: char, n: usize| out.push_str(&format!("{data:<72}{letter}{n:>7}\n"));
    // Free-format records split between parameters, never inside one.
    let split = |record: &str, width: usize| -> Vec<String> {
        let mut lines = vec![String::new()];
        for token in record.split_inclusive(',') {
            if lines.last().unwrap().len() + token.len() > width {
                lines.push(String::new());
            }
            lines.last_mut().unwrap().push_str(token);
        }
        lines
    };
    line(&mut out, "Tequila test", 'S', 1);
    let global = "1H,,1H;,4Htest,8Htest.igs,7HTequila,3H1.0,32,38,6,308,15,4Htest,1.,2,2HMM,1,0.1,15H20261030.120000,1.E-06,1000.,4Htest,4Htest,11,0,15H20261030.120000;";
    let g = split(global, 72);
    for (i, chunk) in g.iter().enumerate() {
        line(&mut out, chunk, 'G', i + 1);
    }
    let (mut p, mut d) = (Vec::new(), Vec::new());
    for (i, (kind, label, level, colour, record, used)) in entities.iter().enumerate() {
        let de = 2 * i + 1;
        let start = p.len() + 1;
        for chunk in split(record, 64) {
            p.push(format!("{chunk:<64} {de:>7}"));
        }
        let count = p.len() + 1 - start;
        let status = if *used { "00010000" } else { "00000000" };
        d.push(format!("{kind:>8}{start:>8}{:>8}{:>8}{level:>8}{:>8}{:>8}{:>8}{status:>8}", 0, 0, 0, 0, 0));
        d.push(format!("{kind:>8}{:>8}{colour:>8}{count:>8}{:>8}{:>8}{:>8}{label:>8}{:>8}", 0, 0, "", "", ""));
    }
    for (i, l) in d.iter().enumerate() {
        line(&mut out, l, 'D', i + 1);
    }
    for (i, l) in p.iter().enumerate() {
        line(&mut out, l, 'P', i + 1);
    }
    line(&mut out, &format!("S{:>7}G{:>7}D{:>7}P{:>7}", 1, g.len(), d.len(), p.len()), 'T', 1);
    out.into_bytes()
}

/// A flat 10 x 10 patch, at x 20 to 30.
const PLATE: &str = "128,1,1,1,1,0,0,1,0,0,0.,0.,1.,1.,0.,0.,1.,1.,1.,1.,1.,1.,20.,0.,0.,30.,0.,0.,20.,10.,0.,30.,10.,0.,0.,1.,0.,1.;";
/// The same patch moved along y, so the two do not meet.
const OTHER_PLATE: &str = "128,1,1,1,1,0,0,1,0,0,0.,0.,1.,1.,0.,0.,1.,1.,1.,1.,1.,1.,20.,40.,0.,30.,40.,0.,20.,50.,0.,30.,50.,0.,0.,1.,0.,1.;";

/// IGES layers are levels: each part is on its surface's level, named as
/// OpenCASCADE names it, and the document shows and hides it by that.
#[test]
fn an_iges_files_levels_are_its_layers() {
    let loose = "128,1,1,1,1,0,0,1,0,0,0.,0.,1.,1.,0.,0.,1.,1.,1.,1.,1.,1.,20.,80.,0.,30.,80.,0.,20.,90.,0.,30.,90.,0.,0.,1.,0.,1.;";
    let bytes = iges(&[(128, "PLATE", 5, 0, PLATE, false), (128, "OTHER", 7, 4, OTHER_PLATE, false), (128, "LOOSE", 0, 0, loose, false)]);
    let model = read_cad(&bytes, "Plates", None, Tessellation::Draft).expect("reads");
    assert!(model.scene.warnings.is_empty(), "{:?}", model.scene.warnings);
    let layers = |name: &str| model.scene.nodes.iter().find(|n| n.name == name).map(|n| n.layers.clone()).unwrap_or_else(|| panic!("no `{name}`"));
    assert_eq!(layers("PLATE"), ["Level 5"]);
    assert_eq!(layers("OTHER"), ["Level 7"]);
    assert!(layers("LOOSE").is_empty(), "level 0 is no layer");

    let mut document = SceneDocument::new();
    let mut bus = CommandBus::new();
    let (command, _) = build_read(&document, None, model, &ImportOptions::default()).unwrap();
    bus.apply(&mut document, command).unwrap();
    let names: Vec<String> = tequila_scene::layers::layers(&document).into_iter().map(|l| l.name).collect();
    assert_eq!(names, ["Level 5", "Level 7"]);
    bus.apply(&mut document, tequila_scene::Command::SetLayerVisibility { layer: "Level 5".into(), visible: false }).unwrap();
    let visible = |name: &str| document.tree.iter().find(|n| n.name == name).unwrap().visible;
    assert!(!visible("PLATE") && visible("OTHER") && visible("LOOSE"));
}

/// Two surfaces that meet but sit on different levels stay two parts, so
/// each can be hidden with its layer.
#[test]
fn surfaces_on_different_levels_are_not_gathered_into_one_body() {
    let touching = "128,1,1,1,1,0,0,1,0,0,0.,0.,1.,1.,0.,0.,1.,1.,1.,1.,1.,1.,30.,0.,0.,40.,0.,0.,30.,10.,0.,40.,10.,0.,0.,1.,0.,1.;";
    let same = read_cad(&iges(&[(128, "A", 5, 0, PLATE, false), (128, "B", 5, 0, touching, false)]), "Plates", None, Tessellation::Draft).unwrap();
    assert_eq!(same.scene.meshes.len(), 1, "on one level they meet and are one body");
    let apart = read_cad(&iges(&[(128, "A", 5, 0, PLATE, false), (128, "B", 6, 0, touching, false)]), "Plates", None, Tessellation::Draft).unwrap();
    assert_eq!(apart.scene.meshes.len(), 2);
}

/// A surface Tequila's own reader does not read - an offset surface - is
/// read by OpenCASCADE where its helper is built, with its level, colour
/// and name, and the scene says so; without the helper the file is
/// refused as before.
#[test]
fn opencascade_reads_an_iges_file_tequila_cannot() {
    let bytes = iges(&[(128, "BASE", 0, 0, PLATE, true), (140, "OFFSET", 9, 2, "140,0.,0.,1.,2.,1;", false)]);
    let name = if cfg!(windows) { "tequila-occt.exe" } else { "tequila-occt" };
    let built = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../native/occt/build/occt").join(name);
    let read = read_cad(&bytes, "Offset", None, Tessellation::Draft);
    if !built.is_file() {
        assert!(read.is_err(), "Tequila's own reader has no offset surfaces");
        eprintln!("OpenCASCADE's helper is not built here; only the refusal was checked");
        return;
    }
    let model = read.expect("OpenCASCADE reads the offset surface");
    let scene = &model.scene;
    assert!(scene.warnings.first().is_some_and(|w| w.contains("OpenCASCADE")), "{:?}", scene.warnings);
    let part = scene.nodes.iter().find(|n| n.mesh.is_some()).expect("a part");
    assert_eq!(part.name, "OFFSET");
    assert_eq!(part.layers, ["Level 9"]);
    let mesh = &scene.meshes[part.mesh.unwrap()];
    assert!(mesh.positions.iter().all(|p| (p.z - 2.0).abs() < 1e-4), "offset 2 mm from the plate");
    assert!(mesh.material.is_some(), "its red came through");
    assert_eq!(scene.tessellation.as_deref(), Some("Draft"), "divided at the quality asked for");
}
