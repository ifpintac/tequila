//! The headless renderer honours the lighting it is given.
//!
//! Two claims, and the second is a bug this test exists because of. The
//! renderer built its environment with `studio_environment()` unconditionally,
//! so a project set up with a sky, an HDRI or a stack of light pins rendered
//! from the command line came out lit by the built-in studio instead. Nothing
//! reported it. The image was simply wrong, in a way you would only notice by
//! rendering the same scene in the application and comparing.
//!
//! Running the binary rather than calling a function on purpose: the defect was
//! in the glue between the document and the renderer, which is the one part a
//! unit test of either side cannot reach.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join("tequila-headless-lighting");
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// Render the bracket fixture, with whatever extra arguments, and read back the
/// mean brightness of the image.
fn render(name: &str, extra: &[&str]) -> (f64, Vec<u8>) {
    let output = scratch().join(format!("{name}.png"));
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/bracket.obj")
        .canonicalize()
        .expect("the bracket fixture is in the repository");

    let mut command = Command::new(env!("CARGO_BIN_EXE_tequila"));
    command
        .arg("-i")
        .arg(&fixture)
        .arg("-o")
        .arg(&output)
        // Small and few: this is about which lighting was used, not about
        // convergence.
        .args(["-w", "96", "-h", "64", "-s", "16", "--no-denoise"])
        .args(extra);
    let status = command.status().expect("the renderer runs");
    assert!(status.success(), "{name}: the renderer failed");

    let bytes = std::fs::read(&output).expect("an image was written");
    let decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
    let mut reader = decoder.read_info().expect("a readable PNG");
    let mut pixels = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels).expect("pixels");
    let pixels = pixels[..info.buffer_size()].to_vec();

    let total: u64 = pixels.iter().map(|&b| b as u64).sum();
    (total as f64 / pixels.len() as f64, pixels)
}

/// `--sky` and `--pins` change the picture.
///
/// The weakest possible claim, and worth making: before this the flags did not
/// exist, and the lighting they select was reachable only from the interface.
#[test]
fn the_lighting_flags_reach_the_image() {
    let (plain, plain_pixels) = render("plain", &[]);
    let (sky, sky_pixels) = render("sky", &["--sky", "golden hour"]);
    let (pins, pin_pixels) = render("pins", &["--pins", "three-point"]);

    assert_ne!(
        plain_pixels, sky_pixels,
        "a sky produced a byte-identical image to no sky"
    );
    assert_ne!(
        plain_pixels, pin_pixels,
        "a studio setup produced a byte-identical image to none"
    );
    // Differing by more than noise, stated as well as the inequality above
    // because two images can differ by noise alone. Not "brighter": a low
    // evening sun is dimmer than a well-lit studio, and this held only while
    // the studio was too dark.
    assert!(
        (sky - plain).abs() > plain * 0.02,
        "a golden-hour sky gave mean {sky} against {plain} for no sky"
    );
    assert!(
        pins > plain * 1.02,
        "three softboxes gave mean {pins} against {plain} for none"
    );
}

/// A name the renderer does not know is an error, not a silent fallback.
///
/// A misspelled preset that quietly rendered the default would be the worst
/// kind of wrong: the command succeeds, the file appears, and the lighting is
/// not what was asked for.
#[test]
fn an_unknown_lighting_name_is_refused() {
    for flag in ["--sky", "--pins"] {
        let status = Command::new(env!("CARGO_BIN_EXE_tequila"))
            .args([flag, "not-a-real-setup"])
            .status()
            .expect("the renderer runs");
        assert!(
            !status.success(),
            "{flag} accepted a name it does not know"
        );
    }
}

/// A project's own environment reaches a headless render.
///
/// This is the defect the module header describes, and the flags above do not
/// cover it: they set the lighting from the command line, while this asks
/// whether the lighting *already in the file* is used. It was not — the
/// renderer built `studio_environment()` and never looked at the document — so
/// a scene lit in the application rendered differently from the command line,
/// silently.
#[test]
fn a_projects_own_lighting_is_used() {
    use tequila_scene::document::SceneDocument;

    fn project_with(name: &str, sky: Option<tequila_render_core::sky::Sky>) -> PathBuf {
        let mut document = SceneDocument::new();
        // A part to light, imported the same way the CLI would.
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/bracket.obj")
            .canonicalize()
            .expect("the bracket fixture is in the repository");
        let (command, _) =
            tequila_import::import(&document, None, &fixture, &Default::default())
                .expect("the fixture imports");
        let mut bus = tequila_scene::CommandBus::new();
        bus.apply(&mut document, command).expect("the import applies");

        document.environment.sky = sky;
        let path = scratch().join(format!("{name}.tequila"));
        tequila_format::save(&path, &document, &Default::default()).expect("saves");
        path
    }

    let plain = project_with("plain-project", None);
    let sunny = project_with(
        "sunny-project",
        tequila_render_core::sky::time_of_day("golden hour"),
    );

    let brightness = |project: &Path, name: &str| {
        let output = scratch().join(format!("{name}.png"));
        let status = std::process::Command::new(env!("CARGO_BIN_EXE_tequila"))
            .arg("-i")
            .arg(project)
            .arg("-o")
            .arg(&output)
            .args(["-w", "96", "-h", "64", "-s", "16", "--no-denoise"])
            .status()
            .expect("the renderer runs");
        assert!(status.success(), "{name}: the renderer failed");
        let bytes = std::fs::read(&output).expect("an image");
        let decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
        let mut reader = decoder.read_info().expect("a readable PNG");
        let mut pixels = vec![0u8; reader.output_buffer_size()];
        let info = reader.next_frame(&mut pixels).expect("pixels");
        let pixels = &pixels[..info.buffer_size()];
        pixels.iter().map(|&b| b as u64).sum::<u64>() as f64 / pixels.len() as f64
    };

    let without = brightness(&plain, "project-plain");
    let with = brightness(&sunny, "project-sunny");
    assert!(
        (with - without).abs() > without * 0.02,
        "a project saved with a golden-hour sky rendered at mean {with}, and \
         one without at {without}. The renderer is not reading the project's \
         environment."
    );
}
