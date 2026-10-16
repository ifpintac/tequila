//! Network rendering through the binary: two render nodes on this machine,
//! and a run that shares a project's configurations out among them.

use tequila_scene::configurator::{Choice, ConfigOption, Configurator, Drives, Style};
use tequila_scene::studio::MultiMaterial;
use tequila_scene::tree::NodeKind;
use tequila_scene::{Command, CommandBus};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as Process, Stdio};

const TOKEN: &str = "a test farm's token";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tequila-farm-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// The demo, its part in three colours under a configurator.
fn project(dir: &Path) -> PathBuf {
    let mut document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.3);
    let mut bus = CommandBus::new();
    let part = document.tree.iter().find(|n| matches!(n.kind, NodeKind::Part { .. })).map(|n| n.id).unwrap();
    let mut variants = vec![document.tree.get(part).unwrap().material.unwrap()];
    for _ in 0..2 {
        bus.apply(&mut document, Command::UnlinkMaterial { node: part }).unwrap();
        variants.push(document.tree.get(part).unwrap().material.unwrap());
    }
    bus.apply(&mut document, Command::SetMultiMaterials { multi_materials: vec![MultiMaterial { id: 1, name: "Colour".into(), variants, active: 0, parts: vec![part] }] }).unwrap();
    let choice = |label: &str, target| Choice { label: label.into(), target, swatch: None };
    bus.apply(&mut document, Command::SetConfigurator {
        configurator: Box::new(Configurator {
            title: "Orb".into(),
            options: vec![ConfigOption {
                id: 1,
                name: "Colour".into(),
                drives: Drives::Material { multi_material: 1 },
                // A comma in a label: the farm sends choices by number.
                choices: vec![choice("Red", 0), choice("Blue, deep", 1), choice("Green", 2)],
                style: Style::Swatches,
            }],
            current: vec![0],
            ..Default::default()
        }),
    })
    .unwrap();
    let bounds = tequila_scene::snapshot::visible_bounds(&document);
    let centre = bounds.center();
    let eye = centre + tequila_core::math::vec3(0.6, 0.35, 1.0).normalize() * bounds.diagonal().length() * 1.6;
    let camera = document.cameras.active_mut();
    camera.camera = tequila_render_core::camera::Camera::look_at(eye, centre, tequila_core::math::Vec3::Y);
    camera.target = centre;
    let path = dir.join("orb.tequila");
    tequila_format::save(&path, &document, &Default::default()).expect("saves");
    path
}

/// A render node, and the address it said it listens on.
struct Node {
    child: Child,
    address: String,
}

impl Drop for Node {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn node(name: &str) -> Node {
    let mut child = Process::new(env!("CARGO_BIN_EXE_tequila"))
        .args(["--worker", "--listen", "127.0.0.1:0", "--name", name, "--token", TOKEN])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("a render node starts");
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).expect("the node says where it listens");
    // Read on: a node writes its progress there, and a pipe nobody reads
    // fills up, or once dropped, fails the writes.
    std::thread::spawn(move || {
        let mut sink = String::new();
        while reader.read_line(&mut sink).unwrap_or(0) > 0 {
            sink.clear();
        }
    });
    let address = line.trim().rsplit(' ').next().unwrap().to_string();
    assert!(address.starts_with("127.0.0.1:"), "{line}");
    Node { child, address }
}

#[test]
fn configurations_are_rendered_on_the_farm() {
    let dir = scratch("configurations");
    let project = project(&dir);
    let (a, b) = (node("alpha"), node("beta"));
    let output = dir.join("out").join("orb {configuration}.png");
    let run = Process::new(env!("CARGO_BIN_EXE_tequila"))
        .arg("-i")
        .arg(&project)
        .args(["--farm", &format!("{},{}", a.address, b.address), "--token", TOKEN, "--all-configurations", "-w", "48", "-h", "32", "-s", "4", "-o"])
        .arg(&output)
        .output()
        .expect("the dispatcher runs");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(run.status.success(), "{stdout}\n{}", String::from_utf8_lossy(&run.stderr));
    for name in ["Red", "Blue, deep", "Green"] {
        let file = dir.join("out").join(format!("orb {name}.png"));
        let bytes = std::fs::read(&file).unwrap_or_else(|_| panic!("{} was not written\n{stdout}", file.display()));
        assert!(bytes.starts_with(b"\x89PNG"), "{} is not a PNG", file.display());
    }
    // Both nodes took work.
    assert!(stdout.contains("(alpha,") && stdout.contains("(beta,"), "{stdout}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_node_with_another_token_renders_nothing_and_says_why() {
    let dir = scratch("token");
    let project = project(&dir);
    let stranger = node("stranger");
    let mut child = Process::new(env!("CARGO_BIN_EXE_tequila"))
        .arg("-i")
        .arg(&project)
        .args(["--farm", &stranger.address, "--token", "not the farm's token", "-w", "48", "-h", "32", "-s", "4", "-o"])
        .arg(dir.join("never.png"))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // It waits for a node that will take the job; the progress says why
    // none does.
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut seen = String::new();
    for _ in 0..10 {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        seen.push_str(&line);
        if line.contains("token does not match") {
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    assert!(seen.contains("token does not match"), "{seen}");
    assert!(!dir.join("never.png").exists());
    std::fs::remove_dir_all(&dir).ok();
}
