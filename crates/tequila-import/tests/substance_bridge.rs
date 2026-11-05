//! The Substance bridge against a separate program that takes the same
//! arguments as Adobe's `sbsrender` and `sbscooker`.
//!
//! The program is this test itself: started with the tools' arguments it
//! acts as them (`tequila_import::substance::fake`); started by `cargo test`
//! it runs the tests, which start it again as the tools. So the bridge is
//! exercised end to end — the process, its arguments, its exit code and
//! what it says on failure, the files it leaves — on a computer with none
//! of Adobe's software. What it does not prove is that the real tools
//! behave the same; nothing here could.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tequila_import::substance::{self, bridge, fixture};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("render") || args.iter().any(|a| a == "--inputs") {
        std::process::exit(substance::fake::main(&args));
    }
    let tests: [(&str, fn()); 3] = [
        ("an_sbsar_renders_through_a_separate_program", an_sbsar_renders_through_a_separate_program),
        ("an_sbs_is_cooked_then_rendered_by_separate_programs", an_sbs_is_cooked_then_rendered_by_separate_programs),
        ("a_failing_tool_says_what_it_said", a_failing_tool_says_what_it_said),
    ];
    let filters: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    let mut failed = 0;
    for (name, test) in tests {
        if !filters.is_empty() && !filters.iter().any(|f| name.contains(f.as_str())) {
            continue;
        }
        match std::panic::catch_unwind(test) {
            Ok(()) => println!("test {name} ... ok"),
            Err(_) => {
                println!("test {name} ... FAILED");
                failed += 1;
            }
        }
    }
    if failed > 0 {
        println!("test result: FAILED. {failed} failed");
        std::process::exit(1);
    }
    println!("test result: ok.");
}

fn scratch(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("tequila-sbs-bridge-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

/// This program, as both of Adobe's tools.
fn tools() -> bridge::Tools {
    let me = std::env::current_exe().expect("the test's own path");
    bridge::Tools { sbsrender: Some(me.clone()), sbscooker: Some(me) }
}

fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

fn an_sbsar_renders_through_a_separate_program() {
    let folder = scratch("sbsar");
    let file = folder.join("bricks.sbsar");
    std::fs::write(&file, fixture::bricks_sbsar()).unwrap();
    let graph = fixture::bricks();
    let out = folder.join("render");
    let rendered = bridge::render(
        &tools(),
        bridge::run_program,
        &file,
        &graph,
        &values(&[("$outputsize", "6,6"), ("brick_color", "0.2,0.4,0.8,1")]),
        &BTreeMap::new(),
        &out,
        &folder.join("cooked"),
    )
    .expect("renders");
    assert_eq!(rendered.maps.len(), graph.outputs.len());
    assert!(bridge::is_rendered(&out));
    // The file DirectX's normal map is read from, beside the one drawn.
    assert!(substance::converted_file(&out, "normal").is_file());
    let colour = tequila_texture::image::Image::load_raw(substance::rendered_file(&out, "basecolor")).unwrap();
    assert_eq!((colour.width, colour.height), (64, 64));
    // Blue bricks: the value reached the program.
    let (mut red, mut blue) = (0.0, 0.0);
    for pixel in colour.data.chunks(colour.channels) {
        red += pixel[0];
        blue += pixel[2];
    }
    assert!(blue > red, "the brick colour did not reach the renderer");
    let _ = std::fs::remove_dir_all(&folder);
}

fn an_sbs_is_cooked_then_rendered_by_separate_programs() {
    let folder = scratch("sbs");
    let file = folder.join("bricks.sbs");
    std::fs::write(&file, fixture::bricks_sbs()).unwrap();
    let graph = substance::read(&file).unwrap().graphs.remove(0);
    let out = folder.join("render");
    let rendered = bridge::render(&tools(), bridge::run_program, &file, &graph, &values(&[("$outputsize", "5,5")]), &BTreeMap::new(), &out, &folder.join("cooked"))
        .expect("cooks and renders");
    assert_eq!(rendered.sbsar, folder.join("cooked").join("bricks.sbsar"));
    assert!(rendered.sbsar.is_file());
    assert!(substance::rendered_file(&out, "roughness").is_file());
    let _ = std::fs::remove_dir_all(&folder);
}

fn a_failing_tool_says_what_it_said() {
    let me = std::env::current_exe().unwrap();
    let error = bridge::run(&me, &["render".into(), "--input".into(), "C:/nowhere/missing.sbsar".into()], std::time::Duration::from_secs(60)).unwrap_err();
    assert!(error.contains("exit code 1") && error.contains("missing.sbsar"), "{error}");
    let folder = scratch("fail");
    let file = folder.join("bricks.sbsar");
    std::fs::write(&file, fixture::bricks_sbsar()).unwrap();
    // A file the tool cannot write to: the bridge says so and marks nothing
    // finished.
    let blocked = folder.join("blocked");
    std::fs::write(&blocked, b"a file where the folder should be").unwrap();
    let result = bridge::render(&tools(), bridge::run_program, &file, &fixture::bricks(), &BTreeMap::new(), &BTreeMap::new(), &blocked, &folder.join("cooked"));
    assert!(result.is_err());
    assert!(!bridge::is_rendered(Path::new(&blocked)));
    let _ = std::fs::remove_dir_all(&folder);
}
