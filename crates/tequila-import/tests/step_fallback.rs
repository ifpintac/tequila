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

/// truck reads a file's layers and attributes as OpenCASCADE does (see
/// `layers_and_attributes_arrive_as_metadata` in `step.rs`).
#[test]
fn truck_reads_layers_and_attributes_too() {
    let this = std::env::current_exe().expect("the test knows where it is");
    std::env::set_var("TEQUILA_OCCT", &this);
    let scene = read_format(include_bytes!("data/layers.step"), "step", "file", &|_| None).expect("truck reads the fixture");
    let node = |name: &str| scene.nodes.iter().find(|n| n.name == name).unwrap_or_else(|| panic!("no node `{name}`"));
    let housing = node("Housing");
    assert_eq!(housing.metadata.get("Part number").map(String::as_str), Some("PN-1042"));
    assert_eq!(housing.metadata.get("Description").map(String::as_str), Some("Front housing"));
    assert_eq!(housing.metadata.get("Definition").map(String::as_str), Some("Production release"));
    assert_eq!(housing.metadata.get("Finish").map(String::as_str), Some("Mold-Tech 11010"));
    assert_eq!(node("Box").layers, ["Outer"]);
    assert_eq!(node("Box").metadata.get("Material").map(String::as_str), Some("ABS"));
    assert!(!node("Box").hidden);
    assert_eq!(node("Pin").layers, ["Construction"]);
    assert!(node("Pin").hidden);
}
