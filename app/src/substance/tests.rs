use super::*;
use tequila_import::substance::fixture;

/// A folder of its own for one test, emptied first.
fn scratch(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("tequila-app-substance-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

/// The bricks fixture on disk, and renders filed in the test's own folder.
fn setup(name: &str) -> (PathBuf, PathBuf, Renders) {
    let folder = scratch(name);
    let file = folder.join("bricks_tequila.sbsar");
    std::fs::write(&file, fixture::bricks_sbsar()).unwrap();
    let renders = Renders { cache: folder.join("cache"), settle: Duration::from_millis(0), ..Renders::default() };
    (folder, file, renders)
}

/// The stand-in renderer, run in this process, as Adobe's tools.
fn with_stand_in(renders: &mut Renders) {
    renders.tools = bridge::Tools { sbsrender: Some(PathBuf::from("sbsrender")), sbscooker: Some(PathBuf::from("sbscooker")) };
    renders.runner = tequila_import::substance::fake::runner;
}

fn the_part(app: &crate::TequilaApp) -> NodeId {
    app.project.document().tree.iter().find(|n| n.material.is_some()).map(|n| n.id).expect("the demo has a part")
}

fn link_of(app: &crate::TequilaApp, key: MaterialKey) -> SubstanceLink {
    (**app.project.document().materials.get(key).unwrap().substance.as_ref().expect("a Substance material")).clone()
}

/// Without Adobe's tools a Substance file still opens: a material that
/// refers to it, with a texture for each output it uses wired as a pack's
/// would be, on the part, in one undo step — and it says plainly what to
/// install, rather than failing quietly.
#[test]
fn a_substance_file_opens_without_the_tools_and_says_what_to_install() {
    let (folder, file, mut renders) = setup("no-tools");
    let mut app = crate::TequilaApp::for_tests();
    let part = the_part(&app);
    let before = app.project.document().materials.iter().count();
    let opened = open(&mut app.project, &mut renders, &file, vec![part], None).expect("opens");
    assert_eq!(opened.assigned, vec![part]);
    assert!(!opened.can_render);
    assert_eq!(app.project.document().materials.iter().count(), before + 1);
    let line = status(&opened);
    assert!(line.contains("Substance 3D Designer") && line.contains("Automation Toolkit"), "{line}");
    let (key, name) = opened.materials[0].clone();
    assert_eq!(name, "Tequila Test Bricks");
    assert_eq!(app.project.document().tree.get(part).unwrap().material, Some(key));

    let link = link_of(&app, key);
    assert_eq!(link.graph, "pkg://bricks_tequila");
    assert_eq!(link.values.get("$outputsize").map(String::as_str), Some("10,10"), "rendered at 1024 at least");
    let outputs: Vec<&str> = link.maps.keys().map(String::as_str).collect();
    assert_eq!(outputs, ["ambientocclusion", "basecolor", "metallic", "normal", "roughness"], "height is not used beside a normal map");
    // The textures wait where the render of these settings will go; the
    // normal map's is the OpenGL copy of the DirectX one the file draws.
    let expected = renders.folder(&link);
    let normal = app.project.document().textures.get(link.maps["normal"]).unwrap();
    assert_eq!(normal.path, expected.join("normal_opengl.png"));
    assert_eq!(normal.role, tequila_texture::texture::MapRole::Normal);
    let graph = &app.project.document().materials.get(key).unwrap().graph;
    for param in [Param::BaseColor, Param::SpecularRoughness, Param::BaseMetalness, Param::Occlusion, Param::Bump] {
        assert!(graph.driver_of(param).is_some(), "{param:?} is not wired");
    }

    renders.poll(&mut app.project);
    assert_eq!(renders.state(key), Some(&State::NoTools));
    assert!(!renders.busy(), "nothing is waiting on tools that are not there");

    app.project.undo();
    assert_eq!(app.project.document().materials.iter().count(), before, "one undo takes it all back");
    let _ = std::fs::remove_dir_all(&folder);
}

use tequila_materials::material_type::Param;

/// With the tools (here the stand-in), the maps are rendered in the
/// background and the textures pointed at them; a changed setting renders
/// again into a folder of its own once it settles, and undoing it brings
/// the earlier maps straight back, without a render.
#[test]
fn settings_render_in_the_background_and_undo_needs_no_render() {
    let (folder, file, mut renders) = setup("render");
    with_stand_in(&mut renders);
    let mut app = crate::TequilaApp::for_tests();
    let part = the_part(&app);
    let opened = open(&mut app.project, &mut renders, &file, vec![part], None).expect("opens");
    assert!(opened.can_render);
    let key = opened.materials[0].0;
    renders.wait(&mut app.project, Duration::from_secs(60));
    assert_eq!(renders.state(key), Some(&State::Rendered));
    let first = link_of(&app, key);
    let first_folder = renders.folder(&first);
    for texture in first.maps.values() {
        let path = &app.project.document().textures.get(*texture).unwrap().path;
        assert!(path.starts_with(&first_folder) && path.is_file(), "{} was not rendered", path.display());
    }
    let colour_of = |app: &crate::TequilaApp| {
        let path = app.project.document().textures.get(link_of(app, key).maps["basecolor"]).unwrap().path.clone();
        let image = tequila_texture::image::Image::load_raw(&path).unwrap();
        (image.width, image.data[..3].to_vec())
    };
    assert_eq!(colour_of(&app).0, 1024, "rendered at the output size asked for");

    // A new brick colour: one undo step, then a render of its own.
    let mut next = first.clone();
    next.values.insert("brick_color".into(), "0.1,0.2,0.9,1".into());
    next.values.insert("$outputsize".into(), "6,6".into());
    app.project.apply(Command::SetSubstance { material: key, substance: Some(Box::new(next.clone())) }).unwrap();
    app.project.end_interaction();
    renders.poll(&mut app.project);
    assert!(renders.busy(), "the new settings render");
    renders.wait(&mut app.project, Duration::from_secs(60));
    assert_eq!(renders.state(key), Some(&State::Rendered));
    let second_folder = renders.folder(&next);
    assert_ne!(first_folder, second_folder);
    let path = app.project.document().textures.get(next.maps["basecolor"]).unwrap().path.clone();
    assert!(path.starts_with(&second_folder), "{}", path.display());
    assert_eq!(colour_of(&app).0, 64);

    // Undo: the old settings, whose render is still filed, so no render.
    app.project.undo();
    assert_eq!(link_of(&app, key), first);
    renders.poll(&mut app.project);
    assert!(!renders.busy(), "an undo found its render filed");
    let path = app.project.document().textures.get(first.maps["basecolor"]).unwrap().path.clone();
    assert!(path.starts_with(&first_folder), "{}", path.display());
    assert_eq!(colour_of(&app).0, 1024);
    let _ = std::fs::remove_dir_all(&folder);
}

/// A project keeps its Substance maps: saved and opened again where there
/// are no tools — and none of the renders it was made from — it still has
/// them, filed as the render of its settings, so nothing needs rendering.
#[test]
fn a_saved_project_keeps_its_maps_for_a_computer_without_the_tools() {
    let (folder, file, mut renders) = setup("project");
    with_stand_in(&mut renders);
    let mut app = crate::TequilaApp::for_tests();
    let part = the_part(&app);
    let key = open(&mut app.project, &mut renders, &file, vec![part], None).expect("opens").materials[0].0;
    renders.wait(&mut app.project, Duration::from_secs(60));
    assert_eq!(renders.state(key), Some(&State::Rendered));
    let link = link_of(&app, key);

    let saved = folder.join("bricks.tequila");
    app.project.save(&saved).expect("saves");
    let bytes = std::fs::read(&saved).unwrap();
    let archive = tequila_format::zip::ZipArchive::read(&bytes).unwrap();
    let members: Vec<&String> = archive.names().iter().filter(|n| n.starts_with("substance/")).collect();
    assert_eq!(members.len(), link.maps.len(), "{members:?}");

    // Another computer: another cache, empty, and no tools at all.
    let elsewhere = folder.join("elsewhere");
    let loaded = tequila_format::load_into(&saved, &elsewhere).expect("loads");
    let definition = loaded.document.materials.get(key).unwrap();
    let again = definition.substance.as_deref().unwrap();
    assert_eq!(again.values, link.values);
    assert_eq!(again.maps, link.maps);
    for texture in again.maps.values() {
        let path = &loaded.document.textures.get(*texture).unwrap().path;
        assert!(path.starts_with(elsewhere.join(link.settings_key())) && path.is_file(), "{}", path.display());
    }
    let mut project = crate::project::Project::with_document(loaded.document, std::sync::Arc::new(tequila_scene::demo::studio_environment()));
    let mut there = Renders { cache: elsewhere, ..Renders::default() };
    there.poll(&mut project);
    assert_eq!(there.state(key), Some(&State::Rendered), "the maps came with the project");
    assert!(!there.busy());
    // And the file itself gone too: the settings are still the material's,
    // the maps still on it.
    std::fs::remove_file(&file).unwrap();
    there.described.clear();
    there.poll(&mut project);
    assert_eq!(there.state(key), Some(&State::Missing));
    let _ = std::fs::remove_dir_all(&folder);
}

/// An unlinked copy shares its original's textures. Once its settings
/// differ it does not repoint them — the two would fight over them every
/// frame — and says so; given maps of its own (one undo step), each follows
/// its own settings.
#[test]
fn an_unlinked_copy_gets_maps_of_its_own() {
    let (folder, file, mut renders) = setup("copy");
    with_stand_in(&mut renders);
    let mut app = crate::TequilaApp::for_tests();
    let part = the_part(&app);
    let original = open(&mut app.project, &mut renders, &file, vec![part], None).expect("opens").materials[0].0;
    renders.wait(&mut app.project, Duration::from_secs(60));
    app.project.apply(Command::UnlinkMaterial { node: part }).unwrap();
    let copy = app.project.document().tree.get(part).unwrap().material.unwrap();
    assert_ne!(copy, original);
    assert_eq!(link_of(&app, copy).maps, link_of(&app, original).maps, "the copy shares the textures");

    let mut next = link_of(&app, copy);
    next.values.insert("brick_color".into(), "0.1,0.6,0.2,1".into());
    app.project.apply(Command::SetSubstance { material: copy, substance: Some(Box::new(next)) }).unwrap();
    app.project.end_interaction();
    let generation = app.project.generation();
    renders.wait(&mut app.project, Duration::from_secs(60));
    renders.poll(&mut app.project);
    assert!(matches!(renders.state(copy), Some(State::Shared(_))), "{:?}", renders.state(copy));
    assert_eq!(renders.state(original), Some(&State::Rendered));
    assert_eq!(app.project.generation(), generation, "nothing was repointed back and forth");

    own_maps(&mut app.project, copy).expect("gives it maps of its own");
    let (mine, theirs) = (link_of(&app, copy).maps, link_of(&app, original).maps);
    assert!(mine.values().all(|t| !theirs.values().any(|o| o == t)), "the copy still shares a texture");
    renders.wait(&mut app.project, Duration::from_secs(60));
    assert_eq!(renders.state(copy), Some(&State::Rendered));
    assert_eq!(renders.state(original), Some(&State::Rendered));
    let path = |key| app.project.document().textures.get(link_of(&app, key).maps["basecolor"]).unwrap().path.clone();
    assert_ne!(path(copy), path(original));
    app.project.undo();
    assert_eq!(link_of(&app, copy).maps, link_of(&app, original).maps, "one undo step");
    let _ = std::fs::remove_dir_all(&folder);
}

/// Every way of naming a graph, and the refusals that say why.
#[test]
fn opening_refuses_what_it_cannot_use_and_says_why() {
    let (folder, file, mut renders) = setup("refuse");
    let mut app = crate::TequilaApp::for_tests();
    let error = open(&mut app.project, &mut renders, &file, Vec::new(), Some("pkg://nothing")).unwrap_err();
    assert!(error.contains("pkg://bricks_tequila"), "{error}");
    assert!(open(&mut app.project, &mut renders, &folder.join("missing.sbsar"), Vec::new(), None).is_err());
    assert!(open(&mut app.project, &mut renders, &folder.join("notes.txt"), Vec::new(), None).unwrap_err().contains(".sbsar"));
    let opened = open(&mut app.project, &mut renders, &file, Vec::new(), Some("bricks_tequila")).expect("by its identifier");
    assert!(opened.assigned.is_empty());
    assert!(status(&opened).contains("top of the Material tab"), "{}", status(&opened));
    let _ = std::fs::remove_dir_all(&folder);
}

/// Designer's `.sbs` is cooked before it renders, by the cooker beside
/// sbsrender; without the cooker it says so.
#[test]
fn an_sbs_is_cooked_then_rendered() {
    let folder = scratch("sbs");
    let file = folder.join("bricks_tequila.sbs");
    std::fs::write(&file, fixture::bricks_sbs()).unwrap();
    let mut renders = Renders { cache: folder.join("cache"), settle: Duration::from_millis(0), ..Renders::default() };
    with_stand_in(&mut renders);
    let mut app = crate::TequilaApp::for_tests();
    let part = the_part(&app);
    let key = open(&mut app.project, &mut renders, &file, vec![part], None).expect("opens").materials[0].0;
    renders.wait(&mut app.project, Duration::from_secs(60));
    assert_eq!(renders.state(key), Some(&State::Rendered));
    assert!(folder.join("cache").join("cooked").read_dir().unwrap().next().is_some(), "it was cooked");

    let mut no_cooker = Renders { cache: folder.join("cache2"), settle: Duration::from_millis(0), ..Renders::default() };
    with_stand_in(&mut no_cooker);
    no_cooker.tools.sbscooker = None;
    no_cooker.poll(&mut app.project);
    assert_eq!(no_cooker.state(key), Some(&State::NoTools));
    let _ = std::fs::remove_dir_all(&folder);
}

/// The panel draws every kind of input, in every state, without touching
/// the material.
#[test]
fn the_panel_draws_without_editing() {
    let (folder, file, mut renders) = setup("panel");
    let mut app = crate::TequilaApp::for_tests();
    let part = the_part(&app);
    let key = open(&mut app.project, &mut renders, &file, vec![part], None).expect("opens").materials[0].0;
    renders.poll(&mut app.project);
    let before = app.project.document().clone();
    let ctx = egui::Context::default();
    let mut asked = Asked::default();
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            asked = panel(ui, &app.project, &mut renders, key);
        });
    });
    assert!(asked.command.is_none());
    assert!(app.project.document() == &before);
    let _ = std::fs::remove_dir_all(&folder);
}

/// A value set back to the file's own is no longer kept.
#[test]
fn a_default_value_is_not_kept() {
    let graph = fixture::bricks();
    let link = SubstanceLink { graph: graph.url.clone(), ..SubstanceLink::default() };
    let red = with_value(&link, &graph, "brick_color", &Value::Numbers(vec![1.0, 0.0, 0.0, 1.0]));
    assert_eq!(red.values.get("brick_color").map(String::as_str), Some("1,0,0,1"));
    let back = with_value(&red, &graph, "brick_color", &graph.input("brick_color").unwrap().default);
    assert!(back.values.is_empty());
    assert_eq!(starting_values(&graph).get("$outputsize").map(String::as_str), Some("10,10"));
}

#[test]
fn the_tools_folder_preference_round_trips() {
    let chosen = Preferences { tools: Some(PathBuf::from("C:/Adobe/SAT")) };
    assert_eq!(Preferences::from_text(&chosen.to_text()), chosen);
    assert_eq!(Preferences::from_text("garbage"), Preferences::default());
}
