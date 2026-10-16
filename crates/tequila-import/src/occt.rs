//! STEP read through OpenCASCADE, by `tequila-occt` (`native/occt`).
//!
//! truck, Tequila's pure-Rust reader, gets some CAD packages' surfaces
//! wrong: a 57 MB AP242 assembly written in metres lost a sixth of its
//! main part's faces and came out with splinters, after it had first hung.
//! OpenCASCADE, the CAD kernel the roadmap planned for this (M8), read the
//! same file in 33 s with every face. It runs as a program of its own,
//! started here, so a file it cannot handle costs that process rather than
//! Tequila's window, and its answer is the bodies, products and colours
//! [`crate::step`] lays out exactly as it lays out truck's.
//!
//! The helper is found beside Tequila (`occt/tequila-occt.exe`), where
//! `TEQUILA_OCCT` says, or in a development build's `native/occt/build`.
//! Without it, STEP is read by truck as before.

use crate::raw::RawMesh;
use tequila_core::math::vec3;
use tequila_format::json::{self, Value};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long a read may take before the helper is stopped and truck tries.
const LONGEST: Duration = Duration::from_secs(30 * 60);

/// The helper program, if there is one.
pub(crate) fn helper() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("TEQUILA_OCCT").map(PathBuf::from) {
        return path.is_file().then_some(path);
    }
    let name = if cfg!(windows) { "tequila-occt.exe" } else { "tequila-occt" };
    let beside = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join("occt").join(name)));
    let built = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../native/occt/build/occt").join(name);
    [beside, Some(built)].into_iter().flatten().find(|p| p.is_file())
}

/// What the helper read: the file's length unit in millimetres, each body
/// with its meshes by colour, and the products placing them.
pub(crate) struct Read {
    pub unit_mm: f64,
    pub bodies: Vec<Body>,
    pub products: Vec<Product>,
}

pub(crate) struct Body {
    pub name: String,
    pub faces: usize,
    pub lost: usize,
    /// sRGB-encoded colour, as the file wrote it, and the mesh in it.
    pub groups: Vec<(Option<[f32; 3]>, RawMesh)>,
}

pub(crate) struct Product {
    pub name: String,
    /// Row-major, in the file's unit, relative to the parent product.
    pub matrix: [f32; 16],
    pub bodies: Vec<usize>,
    pub children: Vec<Product>,
}

/// How far the helper has got: turning the file into shapes (percent), then
/// dividing their faces (done of total).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Reading(usize),
    Dividing(usize, usize),
}

impl Step {
    /// A line of the helper's standard output, if it is a progress line.
    fn parse(line: &str) -> Option<Step> {
        let mut words = line.split_whitespace();
        let mut number = || words.next()?.parse::<usize>().ok();
        match line.split_whitespace().next()? {
            "reading" => {
                number();
                Some(Step::Reading(number()?.min(100)))
            }
            "progress" => {
                number();
                Some(Step::Dividing(number()?, number()?))
            }
            _ => None,
        }
    }
}

/// Read `bytes` (a STEP file) with the helper at `chord` (a fraction of
/// each part's size) and `angle` (radians), reporting its steps as it goes.
pub(crate) fn read(helper: &Path, bytes: &[u8], chord: f64, angle: f64, progress: &dyn Fn(Step)) -> Result<Read, String> {
    static COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let folder = std::env::temp_dir().join(format!(
        "tequila-occt-{}-{}",
        std::process::id(),
        COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let result = run(helper, &folder, bytes, chord, angle, progress);
    let _ = std::fs::remove_dir_all(&folder);
    result
}

fn run(helper: &Path, folder: &Path, bytes: &[u8], chord: f64, angle: f64, progress: &dyn Fn(Step)) -> Result<Read, String> {
    let input = folder.join("model.step");
    let output = folder.join("model.fsocc");
    std::fs::write(&input, bytes).map_err(|e| format!("{}: {e}", input.display()))?;
    let mut command = Command::new(helper);
    command
        .arg(&input)
        .arg(&output)
        .arg(chord.to_string())
        .arg(angle.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        // A console program started from the window would open a console.
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().map_err(|e| format!("{} would not start: {e}", helper.display()))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (sender, steps) = std::sync::mpsc::channel::<Step>();
    let reader = std::thread::spawn(move || {
        for line in stdout.into_iter().flat_map(|out| BufReader::new(out).lines()).map_while(Result::ok) {
            if let Some(step) = Step::parse(&line) {
                let _ = sender.send(step);
            }
        }
    });
    let complaint = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(mut err) = stderr {
            let _ = std::io::Read::read_to_string(&mut err, &mut text);
        }
        text
    });
    let started = Instant::now();
    let status = loop {
        while let Ok(step) = steps.try_recv() {
            progress(step);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() > LONGEST => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("OpenCASCADE was still reading after {} minutes", LONGEST.as_secs() / 60));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(e.to_string()),
        }
    };
    let _ = reader.join();
    while let Ok(step) = steps.try_recv() {
        progress(step);
    }
    let complaint = complaint.join().unwrap_or_default();
    if !status.success() {
        let why = complaint.lines().find(|l| !l.trim().is_empty()).unwrap_or("it stopped").trim().to_string();
        return Err(format!("OpenCASCADE could not read the file: {why}"));
    }
    let bytes = std::fs::read(&output).map_err(|e| format!("OpenCASCADE's answer could not be read: {e}"))?;
    parse(&bytes)
}

/// The helper's answer: a header, its JSON, and the meshes' arrays.
pub(crate) fn parse(bytes: &[u8]) -> Result<Read, String> {
    let malformed = || "OpenCASCADE's answer is not one Tequila recognises".to_string();
    if bytes.len() < 16 || &bytes[..8] != b"FSOCC001" {
        return Err(malformed());
    }
    let length = u64::from_le_bytes(bytes[8..16].try_into().unwrap()) as usize;
    let header = bytes.get(16..16 + length).ok_or_else(malformed)?;
    let root = json::parse(header).map_err(|e| format!("OpenCASCADE's answer: {e}"))?;
    let mut data = &bytes[16 + length..];
    let mut take = |count: usize| -> Result<&[u8], String> {
        if data.len() < count * 4 {
            return Err(malformed());
        }
        let (taken, rest) = data.split_at(count * 4);
        data = rest;
        Ok(taken)
    };
    let floats = |raw: &[u8]| -> Vec<f32> { raw.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect() };
    let count = |v: &Value, key: &str| v.get(key).and_then(Value::as_f64).unwrap_or(0.0) as usize;

    let mut meshes = Vec::new();
    for mesh in root.get("meshes").map(Value::as_array).unwrap_or(&[]) {
        let (vertices, triangles, edges) = (count(mesh, "vertices"), count(mesh, "triangles"), count(mesh, "edge_points"));
        let positions = floats(take(vertices * 3)?);
        let normals = floats(take(vertices * 3)?);
        let indices: Vec<u32> = take(triangles * 3)?.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
        let edge_points = floats(take(edges * 3)?);
        if indices.iter().any(|&i| i as usize >= vertices) {
            return Err(malformed());
        }
        let three = |v: &[f32]| v.chunks_exact(3).map(|c| vec3(c[0], c[1], c[2])).collect::<Vec<_>>();
        meshes.push(Some(RawMesh {
            positions: three(&positions),
            normals: three(&normals),
            indices,
            edge_lines: three(&edge_points),
            ..RawMesh::default()
        }));
    }

    let mut bodies = Vec::new();
    for body in root.get("bodies").map(Value::as_array).unwrap_or(&[]) {
        let mut groups = Vec::new();
        for group in body.get("groups").map(Value::as_array).unwrap_or(&[]) {
            let index = count(group, "mesh");
            let mesh = meshes.get_mut(index).and_then(Option::take).ok_or_else(malformed)?;
            let colour = group.get("colour").map(Value::as_array).filter(|c| c.len() == 3).map(|c| {
                let channel = |i: usize| c[i].as_f64().unwrap_or(0.0) as f32;
                [channel(0), channel(1), channel(2)]
            });
            groups.push((colour, mesh));
        }
        bodies.push(Body {
            name: body.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
            faces: count(body, "faces"),
            lost: count(body, "lost"),
            groups,
        });
    }

    fn product(value: &Value) -> Product {
        let mut matrix = [0.0f32; 16];
        for (slot, v) in matrix.iter_mut().zip(value.get("matrix").map(Value::as_array).unwrap_or(&[])) {
            *slot = v.as_f64().unwrap_or(0.0) as f32;
        }
        Product {
            name: value.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
            matrix,
            bodies: value.get("bodies").map(Value::as_array).unwrap_or(&[]).iter().filter_map(Value::as_f64).map(|b| b as usize).collect(),
            children: value.get("children").map(Value::as_array).unwrap_or(&[]).iter().map(product).collect(),
        }
    }
    let products: Vec<Product> = root.get("products").map(Value::as_array).unwrap_or(&[]).iter().map(product).collect();
    fn valid(p: &Product, bodies: usize) -> bool {
        p.bodies.iter().all(|&b| b < bodies) && p.children.iter().all(|c| valid(c, bodies))
    }
    if !products.iter().all(|p| valid(p, bodies.len())) {
        return Err(malformed());
    }
    Ok(Read { unit_mm: root.get("unit_mm").and_then(Value::as_f64).unwrap_or(1.0), bodies, products })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An answer as the helper writes it: one triangle in one blue body,
    /// placed by one product.
    fn answer(index: u32, products: &str) -> Vec<u8> {
        let header = format!(
            r#"{{"unit":"mm","unit_mm":1,"bodies":[{{"name":"Pin","faces":2,"lost":1,"groups":[{{"mesh":0,"colour":[0,0,1]}}]}}],"products":{products},"meshes":[{{"vertices":3,"triangles":1,"edge_points":2}}]}}"#
        );
        let mut bytes = b"FSOCC001".to_vec();
        bytes.extend((header.len() as u64).to_le_bytes());
        bytes.extend(header.as_bytes());
        let floats = [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0];
        bytes.extend(floats.iter().flat_map(|f| f.to_le_bytes()));
        bytes.extend([0u32, 1, index].iter().flat_map(|i| i.to_le_bytes()));
        bytes.extend([0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0].iter().flat_map(|f| f.to_le_bytes()));
        bytes
    }

    const ONE: &str = r#"[{"name":"Fixture","matrix":[1,0,0,5,0,1,0,0,0,0,1,0,0,0,0,1],"bodies":[0],"children":[]}]"#;

    #[test]
    fn an_answer_reads_back_as_written() {
        let read = parse(&answer(2, ONE)).expect("a well-formed answer reads");
        assert_eq!(read.unit_mm, 1.0);
        let body = &read.bodies[0];
        assert_eq!((body.name.as_str(), body.faces, body.lost), ("Pin", 2, 1));
        let (colour, mesh) = &body.groups[0];
        assert_eq!(*colour, Some([0.0, 0.0, 1.0]));
        assert_eq!(mesh.indices, [0, 1, 2]);
        assert_eq!(mesh.positions[1], vec3(1.0, 0.0, 0.0));
        assert_eq!(mesh.normals[2], vec3(0.0, 0.0, 1.0));
        assert_eq!(mesh.edge_lines.len(), 2);
        assert_eq!((read.products[0].name.as_str(), read.products[0].matrix[3]), ("Fixture", 5.0));
    }

    /// A damaged answer is refused rather than read into a mesh that points
    /// past its vertices or a product that places a body that is not there.
    #[test]
    fn progress_lines_are_read_and_the_rest_ignored() {
        assert_eq!(Step::parse("reading 45"), Some(Step::Reading(45)));
        assert_eq!(Step::parse("reading 140"), Some(Step::Reading(100)));
        assert_eq!(Step::parse("progress 12 300"), Some(Step::Dividing(12, 300)));
        assert_eq!(Step::parse("progress 12"), None);
        assert_eq!(Step::parse("Warning: unit"), None);
        assert_eq!(Step::parse(""), None);
    }

    #[test]
    fn a_damaged_answer_is_refused() {
        assert!(parse(&answer(3, ONE)).is_err(), "a triangle past the vertices");
        let elsewhere = ONE.replace(r#""bodies":[0]"#, r#""bodies":[1]"#);
        assert!(parse(&answer(2, &elsewhere)).is_err(), "a product placing a missing body");
        let whole = answer(2, ONE);
        assert!(parse(&whole[..whole.len() - 4]).is_err(), "a short answer");
        assert!(parse(b"FSOCC000").is_err(), "another format");
    }
}
