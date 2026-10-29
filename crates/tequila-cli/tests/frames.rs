//! `--frames` renders a project's turntable a numbered image a frame, and
//! leaves out a frame already rendered, so a run stopped halfway is
//! finished by running it again.
//!
//! The binary itself, as `headless_lighting.rs` runs it: the numbering and
//! the skipping are in the glue between the flags and the files.

use std::path::PathBuf;
use std::process::Command;

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tequila-frames-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// A demo project turning all the way round in four frames.
fn turntable(dir: &std::path::Path) -> PathBuf {
    let mut document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.3);
    document.animation = Some(tequila_scene::animation::Animation::turntable(4, 24.0, 360.0));
    let path = dir.join("turntable.tequila");
    tequila_format::save(&path, &document, &Default::default()).expect("the project saves");
    path
}

fn run(project: &std::path::Path, output: &std::path::Path, frames: &str) -> String {
    let result = Command::new(env!("CARGO_BIN_EXE_tequila"))
        .arg("-i")
        .arg(project)
        .arg("-o")
        .arg(output)
        .args(["-w", "32", "-h", "24", "-s", "2", "--no-denoise", "--frames", frames])
        .output()
        .expect("the renderer runs");
    assert!(result.status.success(), "the renderer failed: {}", String::from_utf8_lossy(&result.stderr));
    String::from_utf8_lossy(&result.stdout).into_owned()
}

#[test]
fn frames_are_numbered_and_a_rendered_one_is_skipped() {
    let dir = scratch();
    let project = turntable(&dir);
    let output = dir.join("turn.png");
    run(&project, &output, "0-3");
    let frame = |n: u32| dir.join(format!("turn_{n:04}.png"));
    for n in 0..4 {
        assert!(frame(n).is_file(), "frame {n} was not written as {}", frame(n).display());
    }
    assert!(!frame(4).exists(), "a frame past the turntable's last");

    // One taken away, and the same run again: only it is rendered.
    std::fs::remove_file(frame(2)).unwrap();
    let said = run(&project, &output, "0-3");
    assert!(frame(2).is_file(), "the missing frame was not rendered again");
    assert_eq!(said.matches("is already there").count(), 3, "{said}");
    let _ = std::fs::remove_dir_all(&dir);
}
