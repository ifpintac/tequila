//! A downloaded PBR texture pack, made into materials.
//!
//! One route for every way in — the File menu, the top bar, the Library
//! panel, the Material tab, a file dropped on the window and the scripting
//! channel — so the parts a pack lands on and the words that report it cannot
//! drift apart between them.

use std::path::{Path, PathBuf};

use tequila_import::texture_set::{import_sets, SetOptions, SetReport};
use tequila_scene::tree::NodeId;
use tequila_scene::Command;

use crate::project::Project;

/// What a pack became.
pub struct Loaded {
    pub report: SetReport,
    /// How wide one repeat of it is, in scene units.
    pub size: f32,
    /// The parts the first material went on. Empty when nothing was chosen:
    /// the materials are then in the project, waiting to be picked.
    pub assigned: Vec<NodeId>,
}

/// Read the pack at `source` and put its first material on `targets`.
///
/// One undo step for the lot.
pub fn load(project: &mut Project, source: &Path, targets: Vec<NodeId>) -> Result<Loaded, String> {
    if !source.exists() {
        return Err(format!("{} does not exist", source.display()));
    }
    let size = fitted_size(project.document(), &targets);
    let options = match uv_layout(project.document(), &targets) {
        Some(uv_repeats) => SetOptions {
            size,
            projection: tequila_texture::projection::Projection::Uv,
            uv_repeats,
            ..SetOptions::default()
        },
        None => SetOptions { size, ..SetOptions::default() },
    };
    // The parts first, before anything is added: a part that is not there
    // or is locked used to fail the assignment after the pack's materials
    // and textures were already in the project.
    for &node in &targets {
        match project.document().tree.get(node) {
            None => return Err(format!("there is no node {}", node.0)),
            Some(_) if project.document().tree.is_effectively_locked(node) => {
                return Err(format!("{} is locked", project.document().tree.get(node).map(|n| n.name.as_str()).unwrap_or("the part")))
            }
            Some(_) => {}
        }
    }
    let (command, report) = import_sets(project.document(), source, options).map_err(|e| e.to_string())?;
    project.begin_group();
    let applied = project.apply(command).map_err(|e| e.to_string()).and_then(|_| match report.materials.first() {
        Some(&(key, _)) if !targets.is_empty() => project
            .apply(Command::AssignMaterial { nodes: targets.clone(), material: key })
            .map(|_| targets)
            .map_err(|e| e.to_string()),
        _ => Ok(Vec::new()),
    });
    let assigned = match applied {
        Ok(assigned) => {
            project.end_group();
            assigned
        }
        Err(error) => {
            project.abandon_group();
            return Err(error);
        }
    };
    project.end_interaction();
    Ok(Loaded { report, assigned, size })
}

/// How wide one repeat of a pack is on `targets`: as wide as they are, so it
/// lies across them the way the pack's own preview lies across its ball.
///
/// A fixed half metre magnified every pack five times on the ten-centimetre
/// ball the application starts with — a polystyrene's fine pores rendered as
/// craters, a brick wall as part of a single brick — which read as the
/// textures being wrong rather than as a size to change. With nothing to fit
/// to, the half metre stands.
fn fitted_size(document: &tequila_scene::SceneDocument, targets: &[NodeId]) -> f32 {
    const UNFITTED: f32 = 0.5;
    if targets.is_empty() {
        return UNFITTED;
    }
    let Some(bounds) = tequila_scene::gizmo::selection_bounds(document, targets).filter(|b| !b.is_empty()) else {
        return UNFITTED;
    };
    let d = bounds.diagonal();
    let widest = d.x.max(d.y).max(d.z);
    if !widest.is_finite() || widest <= 0.0 {
        return UNFITTED;
    }
    // Two significant figures, so the Material tab shows 10 cm rather than
    // 10.0374 cm.
    let widest = widest.clamp(0.001, 1000.0);
    let magnitude = 10f32.powf(widest.log10().floor() - 1.0);
    (widest / magnitude).round() * magnitude
}

/// Whether a pack should follow `targets`' own UVs, and how many times it
/// should repeat across and down them.
///
/// A model that was laid out for textures — the starter ball, a glTF or OBJ
/// from a modelling package — carries UVs made for exactly this, and a pack
/// follows them without the seams and fans where tri-planar's three
/// projections meet on a curve. That is how every pack's own preview is
/// rendered. A STEP body's are the surfaces' own parameters, a different
/// scale on every face, so a CAD part keeps tri-planar, as does anything
/// without UVs.
///
/// The repeats keep the texture's proportions: UVs that run further one way
/// than the other on the surface (a sphere's go twice as far round as from
/// pole to pole) get proportionally more repeats that way, rounded to whole
/// ones so the texture meets itself at the seam.
fn uv_layout(document: &tequila_scene::SceneDocument, targets: &[NodeId]) -> Option<[f32; 2]> {
    if targets.is_empty() {
        return None;
    }
    let tree = &document.tree;
    let from_cad = |node: NodeId| {
        tree.ancestors(node).into_iter().chain([node]).any(|id| {
            tree.get(id)
                .and_then(|n| n.metadata.get(tequila_scene::reimport::SOURCE_FILE))
                .is_some_and(|file| {
                    let file = file.to_ascii_lowercase();
                    file.ends_with(".step") || file.ends_with(".stp")
                })
        })
    };
    let (mut along_u, mut along_v) = (0.0f64, 0.0f64);
    let mut parts = 0;
    for &target in targets {
        for id in tree.descendants_and_self(target) {
            let Some(mesh) = tree.get(id).and_then(|n| n.mesh()) else { continue };
            if from_cad(id) {
                return None;
            }
            let mesh = document.geometry.get(mesh)?;
            let uvs = mesh.uv_sets.first().filter(|uvs| uvs.len() == mesh.positions.len())?;
            parts += 1;
            for triangle in mesh.indices.chunks_exact(3) {
                let [a, b, c] = [triangle[0] as usize, triangle[1] as usize, triangle[2] as usize];
                let (e1, e2) = (mesh.positions[b] - mesh.positions[a], mesh.positions[c] - mesh.positions[a]);
                let (d1, d2) = (uvs[b] - uvs[a], uvs[c] - uvs[a]);
                let det = d1.x * d2.y - d1.y * d2.x;
                if det.abs() < 1e-12 {
                    continue;
                }
                let area = e1.cross(e2).length() as f64;
                let dpdu = (e1 * d2.y - e2 * d1.y) * (1.0 / det);
                let dpdv = (e2 * d1.x - e1 * d2.x) * (1.0 / det);
                along_u += area * dpdu.length() as f64;
                along_v += area * dpdv.length() as f64;
            }
        }
    }
    if parts == 0 || along_u <= 0.0 || along_v <= 0.0 {
        return None;
    }
    let ratio = (along_u / along_v) as f32;
    Some(if ratio >= 1.0 {
        [ratio.round().max(1.0), 1.0]
    } else {
        [1.0, (1.0 / ratio).round().max(1.0)]
    })
}

/// A length in scene units (metres), as a person reads it.
pub fn length_label(metres: f32) -> String {
    if metres < 0.01 {
        format!("{:.1} mm", metres * 1000.0)
    } else if metres < 1.0 {
        format!("{:.1} cm", metres * 100.0)
    } else {
        format!("{:.2} m", metres)
    }
}

/// The status line: what was made, from which maps, and where it went — or,
/// when it went nowhere, where to find it.
pub fn status(loaded: &Loaded) -> String {
    let report = &loaded.report;
    let Some((_, name)) = report.materials.first() else {
        return "The pack held no material".into();
    };
    let maps: Vec<&str> = report.maps[0].iter().map(|(slot, _)| slot.label()).collect();
    let parts = loaded.assigned.len();
    let repeat = length_label(loaded.size);
    let mut status = if parts == 0 {
        format!(
            "Added `{name}` ({}) to the project. Select a part, then pick it from \
             the material name at the top of the Material tab",
            maps.join(", ")
        )
    } else {
        format!(
            "`{name}` from {} maps ({}) on {parts} part{}, one repeat every {repeat} (Texture size in the Material tab)",
            maps.len(),
            maps.join(", "),
            if parts == 1 { "" } else { "s" }
        )
    };
    if report.materials.len() > 1 {
        let others: Vec<&str> = report.materials[1..].iter().map(|(_, n)| n.as_str()).collect();
        status.push_str(&format!(". Also added: {}", others.join(", ")));
    }
    if let Some(note) = report.notes.iter().find(|n| n.contains("DirectX")) {
        status.push_str(&format!(". {note}"));
    }
    status
}

/// Ask for a pack: a folder, or a ZIP (or any one of the maps inside a
/// folder, for people who open the folder and see pictures).
pub fn pick(folder: bool) -> Option<PathBuf> {
    let dialog = rfd::FileDialog::new().set_title("Import texture pack");
    if folder {
        dialog.pick_folder()
    } else {
        dialog
            .add_filter("Texture pack", &["zip", "png", "jpg", "jpeg", "tif", "tiff", "exr", "hdr"])
            .pick_file()
    }
}

/// The picker's two ways in, as menu entries. `Some(true)` for a folder.
pub fn menu(ui: &mut egui::Ui) -> Option<bool> {
    let mut chosen = None;
    ui.label(
        egui::RichText::new(
            "A PBR pack from ambientCG, Poly Haven, Megascans and the like. Its \
             colour, roughness, metalness, normal and height maps become one \
             material, put on the selected parts.",
        )
        .small()
        .color(crate::theme::dim()),
    );
    if ui.button("From a folder…").clicked() {
        chosen = Some(true);
        ui.close_menu();
    }
    if ui.button("From a ZIP or one of its maps…").clicked() {
        chosen = Some(false);
        ui.close_menu();
    }
    chosen
}

/// What a file dropped on the window is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dropped {
    Model,
    Project,
    TexturePack,
    /// A material somebody shared: a package, or a preset's `.json`.
    Material,
    Unknown,
}

/// Sort a dropped path by what it is. A folder or a ZIP is a pack; so is a
/// loose picture, which stands for the folder it is in. An HDR or EXR is left
/// alone: dropped on its own it is as likely an environment as a map.
pub fn classify_drop(path: &Path) -> Dropped {
    if path.is_dir() {
        return Dropped::TexturePack;
    }
    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "obj" | "stl" | "gltf" | "glb" | "step" | "stp" | "igs" | "iges" | "usda" | "usd" | "ifc" => Dropped::Model,
        "tequila" | "freeshot" => Dropped::Project,
        // A ZIP is a pack of maps unless it holds a material.
        "fsmat" | "json" | "zip" if tequila_format::library::is_material_file(path) => Dropped::Material,
        // KeyShot's material packages.
        "kmp" => Dropped::Material,
        "zip" | "png" | "jpg" | "jpeg" | "tif" | "tiff" => Dropped::TexturePack,
        _ => Dropped::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_png(path: &Path, rgb: [u8; 3]) {
        let file = std::fs::File::create(path).unwrap();
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), 4, 4);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let data: Vec<u8> = (0..16).flat_map(|_| rgb).collect();
        encoder.write_header().unwrap().write_image_data(&data).unwrap();
    }

    /// With parts selected the pack goes on them; with none it is only added,
    /// and the status line says where to find it.
    #[test]
    fn a_pack_goes_on_the_selection_or_says_where_it_went() {
        let folder = std::env::temp_dir().join(format!("tequila-app-pack-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        write_png(&folder.join("Bricks090_1K-PNG_Color.png"), [150, 60, 40]);
        write_png(&folder.join("Bricks090_1K-PNG_Roughness.png"), [200, 200, 200]);

        let mut app = crate::TequilaApp::for_tests();
        let before = app.project.document().materials.iter().count();
        let loaded = load(&mut app.project, &folder, Vec::new()).expect("loads");
        assert!(loaded.assigned.is_empty());
        assert_eq!(loaded.size, 0.5, "with nothing to fit to, the half metre stands");
        assert_eq!(app.project.document().materials.iter().count(), before + 1);
        let line = status(&loaded);
        assert!(line.contains("top of the Material tab"), "{line}");

        let part = app
            .project
            .document()
            .tree
            .iter()
            .find(|n| n.material.is_some())
            .map(|n| n.id)
            .expect("the demo has a part");
        let loaded = load(&mut app.project, &folder, vec![part]).expect("loads");
        assert_eq!(loaded.assigned, vec![part]);
        // As wide as the part: one repeat across it, as across the pack's
        // own preview ball — not half a metre on a ten-centimetre ball.
        let bounds = tequila_scene::gizmo::selection_bounds(app.project.document(), &[part]).unwrap();
        let d = bounds.diagonal();
        let widest = d.x.max(d.y).max(d.z);
        assert!((loaded.size - widest).abs() < widest * 0.06, "{} against a part {widest} wide", loaded.size);
        assert!(status(&loaded).contains("one repeat every"), "{}", status(&loaded));
        let (key, _) = loaded.report.materials[0];
        assert_eq!(app.project.document().tree.get(part).unwrap().material, Some(key));
        assert!(status(&loaded).contains("on 1 part"), "{}", status(&loaded));

        assert!(load(&mut app.project, &folder.join("missing"), Vec::new()).is_err());
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// The whole round: a pack onto a part, saved to the library with its
    /// maps, and dropped from the library onto the part twice. One material and
    /// one set of pictures come of it, and the pictures are the library's
    /// copies, not the pack's files.
    #[test]
    fn a_pack_saved_to_the_library_comes_back_with_its_maps() {
        use tequila_format::library::{read_directory_from, save_entry, LibraryEntry, Source};
        let root = std::env::temp_dir().join(format!("tequila-app-pack-library-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let folder = root.join("Leather037");
        std::fs::create_dir_all(&folder).unwrap();
        write_png(&folder.join("Leather037_1K-PNG_Color.png"), [90, 50, 30]);
        write_png(&folder.join("Leather037_1K-PNG_Roughness.png"), [160, 160, 160]);

        let mut app = crate::TequilaApp::for_tests();
        let part = app
            .project
            .document()
            .tree
            .iter()
            .find(|n| n.material.is_some())
            .map(|n| n.id)
            .expect("the demo has a part");
        let loaded = load(&mut app.project, &folder, vec![part]).expect("loads");
        let (key, _) = loaded.report.materials[0];
        let definition = app.project.document().materials.get(key).unwrap().clone();
        let entry = LibraryEntry::from_graph_with_pictures(
            definition.name.clone(),
            "Leather".into(),
            definition.material_type,
            &definition.graph,
            &app.project.document().textures,
            Source::User,
        );
        assert_eq!(entry.textures.len(), 2);
        let library = root.join("library");
        save_entry(&library, &entry).expect("saves");
        let (entries, problems) = read_directory_from(&library, Source::User);
        assert!(problems.is_empty(), "{problems:?}");

        let materials = app.project.document().materials.iter().count();
        let textures = app.project.document().textures.len();
        app.apply_library_material(&entries[0], &[part], false);
        app.apply_library_material(&entries[0], &[part], false);
        let document = app.project.document();
        assert_eq!(document.materials.iter().count(), materials + 1, "applying twice made two materials");
        assert_eq!(document.textures.len(), textures + 2, "the maps were added twice, or not at all");
        let applied = document.tree.get(part).unwrap().material.unwrap();
        assert_ne!(applied, key, "the part still has the pack's own material");
        for id in document.materials.get(applied).unwrap().graph.compile().unwrap().requirements.textures {
            let path = &document.textures.get(id).expect("a texture the graph reads").path;
            assert!(path.starts_with(&library), "{} is not the library's copy", path.display());
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A real pack's library tile, for looking at: TEQUILA_PACK_EVAL names
    /// the pack, TEQUILA_PACK_EVAL_OUT the PNG to write.
    #[test]
    #[ignore = "an evaluation on a downloaded pack; run by hand"]
    fn eval_library_tile_of_a_real_pack() {
        use tequila_format::library::{read_directory_from, save_entry, LibraryEntry, Source};
        let (Ok(pack), Ok(out)) = (std::env::var("TEQUILA_PACK_EVAL"), std::env::var("TEQUILA_PACK_EVAL_OUT")) else {
            return;
        };
        let mut app = crate::TequilaApp::for_tests();
        let part = app.project.document().tree.iter().find(|n| n.material.is_some()).map(|n| n.id).unwrap();
        let loaded = load(&mut app.project, Path::new(&pack), vec![part]).expect("loads");
        // The whole scene too, for rendering as the application renders it.
        if let Ok(project) = std::env::var("TEQUILA_PACK_EVAL_PROJECT") {
            let mut document = app.project.document().clone();
            document.collect_garbage();
            tequila_format::save(Path::new(&project), &document, &tequila_format::Preserved::default()).expect("saves");
        }
        let mut definition = app.project.document().materials.get(loaded.report.materials[0].0).unwrap().clone();
        // A size to show it at, as the Material tab's Texture size sets it.
        if let Some(size) = std::env::var("TEQUILA_PACK_EVAL_SIZE").ok().and_then(|s| s.parse::<f32>().ok()) {
            let ids: Vec<_> = definition.graph.nodes().map(|(id, _)| id).collect();
            for id in ids {
                if let Some(tequila_materials::graph::NodeKind::Mapping2d { mapping }) = definition.graph.node_mut(id).map(|n| &mut n.kind) {
                    mapping.scale.x = size;
                    mapping.scale.y = size;
                }
            }
        }
        let entry = LibraryEntry::from_graph_with_pictures(
            definition.name.clone(),
            "Eval".into(),
            definition.material_type,
            &definition.graph,
            &app.project.document().textures,
            Source::User,
        );
        let library = std::env::temp_dir().join(format!("tequila-eval-library-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&library);
        save_entry(&library, &entry).expect("saves");
        let (entries, _) = read_directory_from(&library, Source::User);
        let entry = &entries[0];
        let device = tequila_render_cpu::Device::shared().expect("a device");
        let settings = tequila_render_cpu::PreviewSettings { size: std::env::var("TEQUILA_PACK_EVAL_PX").ok().and_then(|s| s.parse().ok()).unwrap_or(256), samples: 64, ..crate::library_tab::thumbnail_settings_at(1) };
        let thumbnail = tequila_render_cpu::render_thumbnail_textured(
            &device,
            &entry.graph,
            crate::library_tab::picture_table(&entry.textures),
            std::sync::Arc::new(tequila_scene::demo::preview_environment()),
            settings,
        )
        .expect("renders");
        let rgba = crate::thumbnail::develop_rgba(&thumbnail);
        let file = std::fs::File::create(&out).unwrap();
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), thumbnail.width as u32, thumbnail.height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&rgba).unwrap();
        println!("{} maps, written to {out}", entry.textures.len());
        let _ = std::fs::remove_dir_all(&library);
    }

    /// On a model with UVs made for texturing — the starter ball — a pack
    /// follows them, repeated twice around and once from pole to pole so a
    /// brick stays a brick, as every pack's own preview is rendered. On a
    /// STEP body, whose UVs are its surfaces' own parameters, it stays
    /// tri-planar.
    #[test]
    fn a_pack_follows_a_models_uvs_and_not_a_cad_parts() {
        use tequila_texture::projection::Projection;
        let folder = std::env::temp_dir().join(format!("tequila-app-pack-uv-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        write_png(&folder.join("Bricks090_1K-PNG_Color.png"), [150, 60, 40]);
        let mapping_of = |app: &crate::TequilaApp, loaded: &Loaded| {
            let graph = &app.project.document().materials.get(loaded.report.materials[0].0).unwrap().graph;
            graph
                .nodes()
                .find_map(|(_, n)| match &n.kind {
                    tequila_materials::graph::NodeKind::Mapping2d { mapping } => Some(mapping.clone()),
                    _ => None,
                })
                .expect("a mapping")
        };

        let mut app = crate::TequilaApp::for_tests();
        let part = app.project.document().tree.iter().find(|n| n.material.is_some()).map(|n| n.id).unwrap();
        let loaded = load(&mut app.project, &folder, vec![part]).expect("loads");
        let mapping = mapping_of(&app, &loaded);
        assert_eq!(mapping.projection, Projection::Uv);
        assert_eq!((mapping.scale.x, mapping.scale.y), (2.0, 1.0), "a sphere's UVs run twice as far round");

        app.project
            .apply(tequila_scene::Command::SetMetadata {
                node: part,
                key: tequila_scene::reimport::SOURCE_FILE.into(),
                value: Some("C:/cad/bracket.STEP".into()),
            })
            .unwrap();
        let loaded = load(&mut app.project, &folder, vec![part]).expect("loads");
        assert_eq!(mapping_of(&app, &loaded).projection, Projection::TriPlanar);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_dropped_file_is_sorted_by_what_it_is() {
        let folder = std::env::temp_dir();
        assert_eq!(classify_drop(&folder), Dropped::TexturePack);
        assert_eq!(classify_drop(Path::new("C:/packs/Leather037_1K-JPG.zip")), Dropped::TexturePack);
        assert_eq!(classify_drop(Path::new("C:/packs/wood/wood_diff_2k.JPG")), Dropped::TexturePack);
        assert_eq!(classify_drop(Path::new("C:/models/bracket.STEP")), Dropped::Model);
        assert_eq!(classify_drop(Path::new("C:/models/headset.glb")), Dropped::Model);
        assert_eq!(classify_drop(Path::new("C:/work/shot.tequila")), Dropped::Project);
        assert_eq!(classify_drop(Path::new("C:/notes.txt")), Dropped::Unknown);
        assert_eq!(classify_drop(Path::new("C:/hdri/studio_small_08_4k.exr")), Dropped::Unknown);
    }

    /// A material somebody shared - a package, or a ZIP or .json holding a
    /// material - goes to the library; a ZIP of maps is still a pack, and a
    /// .json that is not a material is nothing Tequila opens.
    #[test]
    fn a_dropped_material_is_told_from_a_pack() {
        let folder = std::env::temp_dir().join(format!("tequila-drop-material-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let entry = tequila_format::library::LibraryEntry::from_graph(
            "Shared".into(),
            "Plastic".into(),
            tequila_materials::material_type::MaterialType::Plastic,
            &tequila_materials::graph::MaterialGraph::from_material(tequila_materials::material_type::MaterialType::Plastic.defaults()),
            tequila_format::library::Source::User,
        );
        let package = folder.join("shared.fsmat");
        tequila_format::library::export_package(&entry, &package).unwrap();
        assert_eq!(classify_drop(&package), Dropped::Material);
        let zipped = folder.join("shared.zip");
        std::fs::copy(&package, &zipped).unwrap();
        assert_eq!(classify_drop(&zipped), Dropped::Material);
        let preset = folder.join("shared.json");
        std::fs::write(&preset, tequila_format::library::write_entry(&entry)).unwrap();
        assert_eq!(classify_drop(&preset), Dropped::Material);
        let other = folder.join("package.json");
        std::fs::write(&other, "{\"dependencies\": {}}").unwrap();
        assert_eq!(classify_drop(&other), Dropped::Unknown);
        let _ = std::fs::remove_dir_all(&folder);
    }
}
