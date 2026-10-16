//! M7's exit criterion from the command line: a saved project with a
//! configurator renders every combination unattended, and leaves a web
//! configurator whose photos are the files it rendered.
//!
//! Through the binary, as `headless_lighting.rs` does: what is under test is
//! the glue between the flags, the document and the writers.

use tequila_scene::configurator::{Choice, ConfigOption, Configurator, Drives, Rule, RuleKind, Style};
use tequila_scene::studio::{ModelSet, ModelSets, MultiMaterial};
use tequila_scene::tree::NodeKind;
use tequila_scene::{Command, CommandBus};
use std::path::{Path, PathBuf};
use std::process::Command as Process;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tequila-headless-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// The demo, a part in two colours and shown in two model sets, with a
/// configurator over both where blue is not sold on its own.
fn project(dir: &Path) -> PathBuf {
    let mut document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.3);
    let mut bus = CommandBus::new();
    let part = document.tree.iter().find(|n| matches!(n.kind, NodeKind::Part { .. })).map(|n| n.id).unwrap();
    let red = document.tree.get(part).unwrap().material.unwrap();
    bus.apply(&mut document, Command::UnlinkMaterial { node: part }).unwrap();
    let blue = document.tree.get(part).unwrap().material.unwrap();
    bus.apply(&mut document, Command::SetMaterialParameter {
        material: blue,
        parameter: tequila_scene::command::MaterialParameter::new(
            tequila_materials::material_type::Param::BaseColor,
            tequila_materials::material_type::ParamValue::Color(tequila_core::spectrum::Spectrum::from_linear_srgb(tequila_core::math::vec3(0.05, 0.1, 0.6))),
        ),
    })
    .unwrap();
    bus.apply(&mut document, Command::SetMultiMaterials {
        multi_materials: vec![MultiMaterial { id: 1, name: "Colour".into(), variants: vec![red, blue], active: 0, parts: vec![part] }],
    })
    .unwrap();
    bus.apply(&mut document, Command::SetModelSets {
        model_sets: Box::new(ModelSets {
            sets: vec![
                ModelSet { name: "Alone".into(), nodes: vec![part], always_visible: false },
                ModelSet { name: "Boxed".into(), nodes: vec![part], always_visible: false },
            ],
            active: Some(0),
        }),
    })
    .unwrap();
    let choice = |label: &str, target| Choice { label: label.into(), target, swatch: None };
    bus.apply(&mut document, Command::SetConfigurator {
        configurator: Box::new(Configurator {
            title: "Orb".into(),
            options: vec![
                ConfigOption { id: 1, name: "Colour".into(), drives: Drives::Material { multi_material: 1 }, choices: vec![choice("Red", 0), choice("Blue", 1)], style: Style::Swatches },
                ConfigOption { id: 2, name: "Pack".into(), drives: Drives::ModelSet, choices: vec![choice("Alone", 0), choice("Boxed", 1)], style: Style::Buttons },
            ],
            rules: vec![Rule { option: 1, choice: 1, kind: RuleKind::Excludes, other: 2, other_choice: 0 }],
            current: vec![0, 0],
            ..Default::default()
        }),
    })
    .unwrap();
    // A camera framed on the part, as a saved project has: a document made
    // in code keeps the default one, which sits inside the sphere.
    let bounds = tequila_scene::snapshot::visible_bounds(&document);
    let centre = bounds.center();
    let radius = bounds.diagonal().length() * 0.5;
    let eye = centre + tequila_core::math::vec3(0.6, 0.35, 1.0).normalize() * radius * 3.2;
    let camera = document.cameras.active_mut();
    camera.camera = tequila_render_core::camera::Camera::look_at(eye, centre, tequila_core::math::Vec3::Y);
    camera.target = centre;
    let path = dir.join("orb.tequila");
    tequila_format::save(&path, &document, &Default::default()).expect("saves");
    path
}

fn tequila(args: &[&str]) {
    let status = Process::new(env!("CARGO_BIN_EXE_tequila"))
        .args(args)
        .args(["-w", "64", "-h", "48", "-s", "4", "--no-denoise"])
        .status()
        .expect("the renderer runs");
    assert!(status.success(), "tequila {args:?} failed");
}

fn rgba(path: &Path) -> Vec<u8> {
    let bytes = std::fs::read(path).unwrap_or_else(|_| panic!("{} was written", path.display()));
    let mut reader = png::Decoder::new(std::io::Cursor::new(&bytes)).read_info().expect("a PNG");
    let mut pixels = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels).expect("pixels");
    assert_eq!(info.color_type, png::ColorType::Rgba);
    pixels[..info.buffer_size()].to_vec()
}

/// Every valid combination is rendered, named by its choices: three of the
/// four, blue alone being ruled out - and the two colours come out apart.
#[test]
fn every_configuration_renders_to_its_own_file() {
    let dir = scratch("configurations");
    let project = project(&dir);
    let template = dir.join("orb {configuration}.png");
    tequila(&["-i", &project.to_string_lossy(), "--all-configurations", "-o", &template.to_string_lossy()]);
    let mut names: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".png")).collect();
    names.sort();
    assert_eq!(names, ["orb Blue - Boxed.png", "orb Red - Alone.png", "orb Red - Boxed.png"]);
    let mean_blue = |pixels: &[u8]| pixels.chunks_exact(4).map(|p| p[2] as f64 - p[0] as f64).sum::<f64>();
    let red = rgba(&dir.join("orb Red - Boxed.png"));
    let blue = rgba(&dir.join("orb Blue - Boxed.png"));
    assert!(mean_blue(&blue) > mean_blue(&red), "the blue variant did not render bluer");
    let _ = std::fs::remove_dir_all(&dir);
}

/// One run leaves a web configurator whose photos are where its page looks.
#[test]
fn a_web_configurator_is_left_behind_with_its_photos() {
    let dir = scratch("web");
    let project = project(&dir);
    let site = dir.join("site");
    tequila(&["-i", &project.to_string_lossy(), "--export-web", &site.to_string_lossy(), "--photos"]);
    for file in ["index.html", "viewer.js", "data.js"] {
        assert!(site.join(file).exists(), "{file} is missing");
    }
    let data = std::fs::read_to_string(site.join("data.js")).unwrap();
    for photo in ["images/Red - Alone.jpg", "images/Red - Boxed.jpg", "images/Blue - Boxed.jpg"] {
        assert!(data.contains(photo), "the page does not name {photo}");
        assert!(site.join(photo).exists(), "{photo} was not rendered");
    }
    assert!(!dir.join("tequila.png").exists(), "an export-only run rendered the default image too");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A chosen configuration renders cut out: the corners, all background,
/// go transparent, the product in the middle stays.
#[test]
fn a_chosen_configuration_renders_cut_out() {
    let dir = scratch("silhouette");
    let project = project(&dir);
    let output = dir.join("cut.png");
    tequila(&["-i", &project.to_string_lossy(), "--configuration", "Blue,Boxed", "--silhouette", "-o", &output.to_string_lossy()]);
    let pixels = rgba(&output);
    let alpha = |x: usize, y: usize| pixels[(y * 64 + x) * 4 + 3];
    assert_eq!(alpha(0, 0), 0, "a corner kept its background");
    assert_eq!(alpha(63, 0), 0);
    assert!(alpha(32, 24) > 200, "the product was cut away");

    // A configuration the rules forbid is refused, not rendered.
    let status = Process::new(env!("CARGO_BIN_EXE_tequila"))
        .args(["-i", &project.to_string_lossy(), "--configuration", "Blue,Alone", "-o", &dir.join("no.png").to_string_lossy(), "-w", "16", "-h", "16", "-s", "1"])
        .status()
        .unwrap();
    assert!(!status.success());
    let _ = std::fs::remove_dir_all(&dir);
}
