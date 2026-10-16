//! A STEP file still reads when OpenCASCADE's helper fails: truck reads it
//! instead and the scene says so. A test binary of its own, because the
//! helper is chosen by an environment variable the whole process shares.

use tequila_import::read_format;

const TWO_BODIES: &[u8] = include_bytes!("data/two_bodies.step");

/// The helper here is this test program, which runs, writes no answer and
/// exits: as a helper missing a library or stopped part way would.
#[test]
fn a_helper_that_gives_no_answer_falls_back_to_truck() {
    let this = std::env::current_exe().expect("the test knows where it is");
    std::env::set_var("TEQUILA_OCCT", &this);
    let scene = read_format(TWO_BODIES, "step", "file", &|_| None).expect("truck reads the fixture");
    scene.validate().expect("the reader's output holds together");
    assert_eq!(scene.meshes.iter().filter(|m| !m.indices.is_empty()).count(), 3, "both bodies, one in two colours");
    assert!(
        scene.warnings.first().is_some_and(|w| w.contains("OpenCASCADE") && w.contains("Tequila's own reader")),
        "{:?}",
        scene.warnings
    );
}
