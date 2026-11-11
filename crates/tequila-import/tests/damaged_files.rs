//! A damaged file is an error, never a crash.
//!
//! People open files that were cut short by a download, a full disk or a
//! sync that stopped half way, and files that are simply something else
//! with the wrong ending. Every reader should say the file could not be
//! read; none may panic and take the application, and the person's
//! unsaved work, down with it. Each sample here is read whole, empty, cut
//! short at several points, and with bytes scrambled at others, and every
//! one of those must come back as `Ok` or `Err`.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

/// A few hundred bytes of each text format, written here so the test needs
/// no files for them.
fn written_samples() -> Vec<(&'static str, Vec<u8>)> {
    let obj = b"# a cube's corner\nmtllib none.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nv 0 0 1\nvt 0 0\nvt 1 0\nvt 0 1\nvn 0 0 1\nusemtl red\nf 1/1/1 2/2/1 3/3/1\nf 1 3 4\nf 1 2 4\ng side\nf 2 3 4\n".to_vec();
    let stl_ascii = b"solid corner\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nfacet normal 1 1 1\nouter loop\nvertex 1 0 0\nvertex 0 1 0\nvertex 0 0 1\nendloop\nendfacet\nendsolid corner\n".to_vec();
    let mut stl_binary = vec![0u8; 80];
    stl_binary.extend_from_slice(&2u32.to_le_bytes());
    for triangle in [[[0.0f32, 0.0, 1.0], [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], [[1.0, 1.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]] {
        for v in triangle {
            for c in v {
                stl_binary.extend_from_slice(&c.to_le_bytes());
            }
        }
        stl_binary.extend_from_slice(&0u16.to_le_bytes());
    }
    let ply = b"ply\nformat ascii 1.0\nelement vertex 4\nproperty float x\nproperty float y\nproperty float z\nelement face 2\nproperty list uchar int vertex_indices\nend_header\n0 0 0\n1 0 0\n0 1 0\n0 0 1\n3 0 1 2\n3 1 2 3\n".to_vec();
    let usda = b"#usda 1.0\n(\n    upAxis = \"Y\"\n)\ndef Mesh \"Corner\"\n{\n    int[] faceVertexCounts = [3, 3]\n    int[] faceVertexIndices = [0, 1, 2, 1, 2, 3]\n    point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0), (0, 0, 1)]\n}\n".to_vec();
    vec![("obj", obj), ("stl", stl_ascii), ("stl", stl_binary), ("ply", ply), ("usda", usda)]
}

/// The samples on disk, by the extension they are read as.
fn file_samples() -> Vec<(&'static str, Vec<u8>)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let samples = root.join("../../assets/samples");
    let data = root.join("tests/data");
    let mut out = Vec::new();
    for (extension, path) in [
        ("gltf", samples.join("glasses.gltf")),
        ("fbx", data.join("primitives.fbx")),
        ("fbx", data.join("primitives_ascii.fbx")),
        ("3dm", data.join("rhino3dm/mesh.3dm")),
    ] {
        if let Ok(bytes) = std::fs::read(&path) {
            out.push((extension, bytes));
        }
    }
    out
}

/// The file whole, empty, cut short at nine points, and with a run of bytes
/// scrambled at nine others - the same ones every time.
fn damaged(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut out = vec![("whole".to_string(), bytes.to_vec()), ("empty".to_string(), Vec::new())];
    let n = bytes.len();
    for k in 1..10 {
        let at = n * k / 10;
        out.push((format!("cut at {at} of {n}"), bytes[..at].to_vec()));
    }
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..9 {
        let mut copy = bytes.to_vec();
        if n == 0 {
            break;
        }
        let at = (next() as usize) % n;
        for i in at..(at + 16).min(n) {
            copy[i] = next() as u8;
        }
        out.push((format!("scrambled at {at} of {n}"), copy));
    }
    out
}

fn panic_text(payload: Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic with no message".into())
}

#[test]
fn damaged_models_are_errors_not_crashes() {
    let mut crashed = Vec::new();
    let mut tried = 0;
    for (extension, bytes) in written_samples().into_iter().chain(file_samples()) {
        for (how, variant) in damaged(&bytes) {
            tried += 1;
            let none = |_: &str| -> Option<Vec<u8>> { None };
            let result = catch_unwind(AssertUnwindSafe(|| tequila_import::read_format(&variant, extension, "damaged", &none).map(|_| ())));
            if let Err(payload) = result {
                crashed.push(format!(".{extension}, {how}: {}", panic_text(payload)));
            }
        }
    }
    assert!(tried > 100, "only {tried} files tried");
    assert!(crashed.is_empty(), "{} of {tried} damaged files crashed the reader:\n{}", crashed.len(), crashed.join("\n"));
}

#[test]
fn damaged_projects_are_errors_not_crashes() {
    let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/samples");
    let mut crashed = Vec::new();
    let mut tried = 0;
    for name in ["Perfume bottle.tequila", "Headphones.tequila"] {
        let Ok(bytes) = std::fs::read(samples.join(name)) else { continue };
        for (how, variant) in damaged(&bytes) {
            tried += 1;
            let result = catch_unwind(AssertUnwindSafe(|| tequila_format::read(&variant).map(|_| ())));
            if let Err(payload) = result {
                crashed.push(format!("{name}, {how}: {}", panic_text(payload)));
            }
        }
        // A file cut short fails at its table of contents, before anything
        // in it is read: so each member is damaged too, inside a whole
        // archive, which is what reaches the readers of the scene, the
        // materials and the meshes.
        let Ok(archive) = tequila_format::zip::ZipArchive::read(&bytes) else { continue };
        let names: Vec<String> = archive.names().to_vec();
        // One of each kind of member: the manifest, the scene, a material,
        // a mesh, and whatever else there is.
        let mut kinds = std::collections::BTreeSet::new();
        for member in names.iter().filter(|m| kinds.insert(m.chars().map(|c| if c.is_ascii_digit() { '#' } else { c }).collect::<String>())) {
            let original = archive.get(member).expect("listed").to_vec();
            for (how, damaged_member) in damaged(&original).into_iter().skip(1) {
                let mut writer = tequila_format::zip::ZipWriter::new();
                for other in &names {
                    let content = if other == member { damaged_member.clone() } else { archive.get(other).expect("listed").to_vec() };
                    writer.add(other.clone(), content);
                }
                let repacked = writer.finish();
                tried += 1;
                let result = catch_unwind(AssertUnwindSafe(|| tequila_format::read(&repacked).map(|_| ())));
                if let Err(payload) = result {
                    crashed.push(format!("{name}, {member} {how}: {}", panic_text(payload)));
                }
            }
        }
    }
    assert!(tried > 0, "no sample projects");
    assert!(crashed.is_empty(), "{} of {tried} damaged projects crashed the reader:\n{}", crashed.len(), crashed.join("\n"));
}

/// What people share besides models and projects: a material file and a
/// studio preset, both JSON, and a STEP model, which goes to the CAD
/// reader in its own process.
#[test]
fn damaged_presets_and_cad_are_errors_not_crashes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut crashed = Vec::new();
    let mut tried = 0;

    let material = std::fs::read(root.join("../../assets/library/standard/abs_black.json")).expect("a library preset");
    for (how, variant) in damaged(&material) {
        tried += 1;
        let text = String::from_utf8_lossy(&variant).into_owned();
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| tequila_format::library::read_entry(&text, "damaged").map(|_| ()))) {
            crashed.push(format!("material, {how}: {}", panic_text(payload)));
        }
    }

    let studio = std::fs::read(root.join("../../assets/studios/tequila-soft-studio/tequila-soft-studio.studio.json")).expect("a studio preset");
    for (how, variant) in damaged(&studio) {
        tried += 1;
        let result = catch_unwind(AssertUnwindSafe(|| {
            tequila_format::json::parse(&variant).ok().map(|value| tequila_format::studio_preset::from_inline(&value).map(|_| ()))
        }));
        if let Err(payload) = result {
            crashed.push(format!("studio, {how}: {}", panic_text(payload)));
        }
    }

    let step = std::fs::read(root.join("tests/data/two_bodies.step")).expect("a STEP file");
    for (how, variant) in damaged(&step) {
        tried += 1;
        let none = |_: &str| -> Option<Vec<u8>> { None };
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| tequila_import::read_format(&variant, "step", "damaged", &none).map(|_| ()))) {
            crashed.push(format!(".step, {how}: {}", panic_text(payload)));
        }
    }

    assert!(crashed.is_empty(), "{} of {tried} damaged files crashed their reader:\n{}", crashed.len(), crashed.join("\n"));
}
