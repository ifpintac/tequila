//! Blender's `.blend`, read by Blender.
//!
//! A `.blend` is Blender's memory written out, in Blender's own layout,
//! which changes from version to version, and much of what it holds is not
//! geometry yet but what makes it: modifiers, curves, text, geometry nodes,
//! instances. Only Blender turns those into what renders. So Tequila asks
//! it: the Blender installed on this computer runs in the background on the
//! file, with the file's own scripts kept off (`--disable-autoexec`), and
//! `tequila_export.py` writes down what the scene holds - `scene.json`, and
//! the meshes' arrays in `scene.bin` - which [`scene_from`] reads. Nothing is
//! saved back to the file.
//!
//! Tequila links nothing of Blender, which is GPL: the script is Tequila's
//! own, under Apache-2.0 like the rest, and Blender is a program Tequila
//! runs, as it runs OpenCASCADE's reader (`occt`).
//!
//! What comes over:
//!
//! - every mesh, curve, surface, metaball and text object of the scene the
//!   file was saved in, its modifiers applied, by its name, under its
//!   collection and its parent as Blender's outliner shows it; hidden where
//!   Blender's render leaves it out (switched off for rendering, or in an
//!   excluded or unrendered collection); an empty collection or empty is
//!   left out;
//! - instances - collection instances, particles, geometry nodes' - placed
//!   under what made them, sharing a mesh where Blender does;
//! - each material, its node tree translated node for node (`material`),
//!   and its pictures, packed ones written out beside the model;
//! - the scene's cameras, with its active one, and its render size and
//!   view transform for a scene opened from it.
//!
//! Not: lights (Tequila lights a model with its studios), the world, the
//! other scenes in the file, and anything Blender itself could not make
//! into a mesh. Each is named in the import's warnings.

mod material;

use crate::raw::{RawMesh, RawNode, RawScene, RawSetup, RawTexture};
use crate::ImportError;
use std::collections::{BTreeMap, HashMap};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tequila_core::math::{vec2, vec3, vec4, Mat4, Transform, Vec2, Vec3, Vec4};
use tequila_format::json::{self, Value as Json};
use tequila_render_core::camera::{Aperture, Camera, Projection};
use tequila_scene::lighting::SavedCamera;

/// The script Blender runs.
const SCRIPT: &str = include_str!("tequila_export.py");

/// The answer's format, as the script writes it.
const FORMAT: &str = "tequila-blend";
const VERSION: usize = 1;

/// How long Blender may take before it is stopped.
const LONGEST: Duration = Duration::from_secs(30 * 60);

/// What the import says when there is no Blender to ask.
pub const NO_BLENDER: &str = "opening a Blender file needs Blender 4.2 or later on this computer, which reads the file for Tequila; \
     it is free from blender.org";

/// Blender's Z up to Tequila's Y up.
const Z_UP_TO_Y_UP: Mat4 = Mat4 { m: [[1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, -1.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0]] };

/// Warnings, each said once, in the order they came.
#[derive(Default)]
pub(crate) struct Notes(Vec<String>);

impl Notes {
    pub(crate) fn add(&mut self, note: String) {
        if !self.0.contains(&note) {
            self.0.push(note);
        }
    }
}

/// The Blender this computer has: where `TEQUILA_BLENDER` says, else the
/// newest where Blender's installers put it, else one on the `PATH`.
pub fn blender() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("TEQUILA_BLENDER").map(PathBuf::from) {
        return path.is_file().then_some(path);
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        for folder in [Some(PathBuf::from("/Applications")), home.map(|h| h.join("Applications"))].into_iter().flatten() {
            // Blender.app, and side-by-side installs such as Blender 4.2.app.
            candidates.extend(newest_first(&folder, |n| n.starts_with("Blender") && n.ends_with(".app")).into_iter().map(|app| app.join("Contents/MacOS/Blender")));
        }
    }
    #[cfg(windows)]
    {
        for base in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"].into_iter().filter_map(std::env::var_os).map(PathBuf::from) {
            candidates.extend(newest_first(&base.join("Blender Foundation"), |n| n.starts_with("Blender")).into_iter().map(|dir| dir.join("blender.exe")));
            candidates.push(base.join("Steam").join("steamapps").join("common").join("Blender").join("blender.exe"));
        }
    }
    #[cfg(target_os = "linux")]
    {
        candidates.push(PathBuf::from("/snap/bin/blender"));
        candidates.push(PathBuf::from("/var/lib/flatpak/exports/bin/org.blender.Blender"));
    }
    let program = if cfg!(windows) { "blender.exe" } else { "blender" };
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|dir| dir.join(program)));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// The entries of `folder` whose names pass `wanted`, the highest version
/// first and an unnumbered one (the usual download) before any.
#[allow(dead_code)]
fn newest_first(folder: &Path, wanted: impl Fn(&str) -> bool) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else { return Vec::new() };
    let mut found: Vec<(Vec<u32>, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(&wanted))
        .map(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let version: Vec<u32> = name.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect();
            (if version.is_empty() { vec![u32::MAX] } else { version }, p)
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().map(|(_, p)| p).collect()
}

/// Read the `.blend` at `path`, by the Blender this computer has.
pub fn read_file(path: &Path, name: &str) -> Result<RawScene, ImportError> {
    let program = blender().ok_or_else(|| ImportError::Missing(NO_BLENDER.to_string()))?;
    read_with(&program, path, name)
}

/// Read a `.blend` from its bytes: written to a temporary file first, so
/// pictures it names beside where it was saved will not be found.
pub fn read(bytes: &[u8], name: &str) -> Result<RawScene, ImportError> {
    let program = blender().ok_or_else(|| ImportError::Missing(NO_BLENDER.to_string()))?;
    let folder = work_folder()?;
    let file = folder.join("model.blend");
    let result = std::fs::write(&file, bytes)
        .map_err(|e| ImportError::Io(format!("{}: {e}", file.display())))
        .and_then(|()| read_with(&program, &file, name));
    let _ = std::fs::remove_dir_all(&folder);
    result
}

/// Read the `.blend` at `path` with the Blender at `program`.
pub fn read_with(program: &Path, path: &Path, name: &str) -> Result<RawScene, ImportError> {
    let folder = work_folder()?;
    let result = run(program, path, &folder).and_then(|()| read_answer(&folder.join("answer"), name));
    let _ = std::fs::remove_dir_all(&folder);
    result
}

fn work_folder() -> Result<PathBuf, ImportError> {
    static COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let folder = std::env::temp_dir().join(format!("tequila-blend-{}-{}", std::process::id(), COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    std::fs::create_dir_all(folder.join("answer")).map_err(|e| ImportError::Io(format!("{}: {e}", folder.display())))?;
    Ok(folder)
}

/// Blender, in the background, writing its answer into `folder/answer`.
fn run(program: &Path, path: &Path, folder: &Path) -> Result<(), ImportError> {
    let script = folder.join("tequila_export.py");
    std::fs::write(&script, SCRIPT).map_err(|e| ImportError::Io(format!("{}: {e}", script.display())))?;
    let mut command = Command::new(program);
    command
        .arg("--background")
        // Blender as it comes: no add-ons or preferences of the person's
        // changing what is read.
        .arg("--factory-startup")
        // And nothing in the file runs: no scripts, no Python drivers.
        .arg("--disable-autoexec")
        .arg(path)
        .arg("--python-exit-code")
        .arg("1")
        .arg("--python")
        .arg(&script)
        .arg("--")
        .arg(folder.join("answer"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().map_err(|e| ImportError::Io(format!("Blender ({}) would not start: {e}", program.display())))?;
    let drain = |stream: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut text = String::new();
            if let Some(mut s) = stream {
                let mut bytes = Vec::new();
                let _ = s.read_to_end(&mut bytes);
                text = String::from_utf8_lossy(&bytes).into_owned();
            }
            text
        })
    };
    let out = drain(child.stdout.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>));
    let err = drain(child.stderr.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() > LONGEST => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ImportError::Io(format!("Blender was still reading the file after {} minutes", LONGEST.as_secs() / 60)));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(ImportError::Io(e.to_string())),
        }
    };
    let said = format!("{}\n{}", out.join().unwrap_or_default(), err.join().unwrap_or_default());
    if !status.success() || !folder.join("answer").join("scene.json").is_file() {
        return Err(ImportError::Malformed(format!("Blender could not read it: {}", complaint(&said))));
    }
    Ok(())
}

/// The line of what Blender printed that says what went wrong.
fn complaint(said: &str) -> String {
    let lines: Vec<&str> = said.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    // The script's own refusal, a Python error, or Blender's.
    let pick = lines
        .iter()
        .rev()
        .find(|l| l.contains("too old") || l.contains("could not open"))
        .or_else(|| lines.iter().rev().find(|l| l.starts_with("Error") || l.contains("Error:") || l.contains("Error ")))
        .or_else(|| lines.last());
    pick.map(|l| l.chars().take(300).collect()).unwrap_or_else(|| "it stopped without saying why".to_string())
}

/// Blender's answer, already written into `folder` by `tequila_export.py`:
/// what [`read_with`] reads once Blender has run, and what the tests read
/// without Blender.
pub fn read_answer(folder: &Path, name: &str) -> Result<RawScene, ImportError> {
    let json_path = folder.join("scene.json");
    let bytes = std::fs::read(&json_path).map_err(|e| ImportError::Io(format!("Blender's answer could not be read: {e}")))?;
    let answer = json::parse(&bytes).map_err(|e| ImportError::Malformed(format!("Blender's answer: {e}")))?;
    let bin = std::fs::read(folder.join("scene.bin")).unwrap_or_default();
    let packed = |file: &str| -> Option<Arc<Vec<u8>>> {
        // A plain name the script wrote, never a path out of the folder.
        if file.is_empty() || file.contains(['/', '\\']) || file.starts_with('.') {
            return None;
        }
        std::fs::read(folder.join(file)).ok().map(Arc::new)
    };
    scene_from(&answer, &bin, &packed, name)
}

/// One mesh of the answer, as its arrays.
struct Exported {
    name: String,
    positions: Vec<Vec3>,
    normals: Vec<Vec3>,
    uvs: Vec<Vec2>,
    colors: Vec<Vec4>,
    /// Each triangle's material slot.
    slots: Vec<i32>,
}

fn array<'b>(bin: &'b [u8], at: Option<&Json>, what: &str) -> Result<&'b [u8], ImportError> {
    let at = at.ok_or_else(|| ImportError::Malformed(format!("Blender's answer has no {what}")))?;
    let (Some(offset), Some(length)) = (at.index(0).and_then(Json::as_index), at.index(1).and_then(Json::as_index)) else {
        return Err(ImportError::Malformed(format!("Blender's answer names its {what} oddly")));
    };
    offset
        .checked_add(length)
        .filter(|&end| end <= bin.len() && length % 4 == 0)
        .map(|end| &bin[offset..end])
        .ok_or_else(|| ImportError::Malformed(format!("Blender's answer's {what} runs past its arrays")))
}

fn floats(bytes: &[u8]) -> impl Iterator<Item = f32> + '_ {
    bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
}

fn finite(v: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

fn exported(entry: &Json, bin: &[u8]) -> Result<Exported, ImportError> {
    let name = entry.get("name").and_then(Json::as_str).unwrap_or("Mesh").to_string();
    let triangles = entry.get("triangles").and_then(Json::as_index).unwrap_or(0);
    let corners = triangles.checked_mul(3).ok_or_else(|| ImportError::Malformed("a mesh too large to hold".into()))?;
    let vectors = |bytes: &[u8]| -> Vec<Vec3> {
        let f: Vec<f32> = floats(bytes).map(finite).collect();
        f.chunks_exact(3).map(|c| vec3(c[0], c[1], c[2])).collect()
    };
    let positions = vectors(array(bin, entry.get("positions"), "positions")?);
    let normals = vectors(array(bin, entry.get("normals"), "normals")?);
    let slots: Vec<i32> = array(bin, entry.get("slots"), "material slots")?.chunks_exact(4).map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    if positions.len() != corners || normals.len() != corners || slots.len() != triangles {
        return Err(ImportError::Malformed(format!("Blender's answer for the mesh {name} does not add up")));
    }
    // The UV map a render uses, which the script puts first. Blender's v
    // runs up the picture; Tequila's, as glTF's, down it.
    let uvs = match entry.get("uvs").and_then(|u| u.index(0)) {
        Some(at) => {
            let f: Vec<f32> = floats(array(bin, Some(at), "UVs")?).map(finite).collect();
            let uvs: Vec<Vec2> = f.chunks_exact(2).map(|c| vec2(c[0], 1.0 - c[1])).collect();
            if uvs.len() == corners {
                uvs
            } else {
                Vec::new()
            }
        }
        None => Vec::new(),
    };
    let colors = match entry.get("colors") {
        Some(at) => {
            let f: Vec<f32> = floats(array(bin, Some(at), "colours")?).map(finite).collect();
            let colors: Vec<Vec4> = f.chunks_exact(4).map(|c| vec4(c[0], c[1], c[2], c[3])).collect();
            if colors.len() == corners {
                colors
            } else {
                Vec::new()
            }
        }
        None => Vec::new(),
    };
    Ok(Exported { name, positions, normals, uvs, colors, slots })
}

/// The triangles of `mesh` in any of `wanted` of its `slots` (Blender
/// takes a slot past the last as the last), as a mesh of their own.
fn part_mesh(mesh: &Exported, wanted: &[usize], slots: usize, name: String, material: Option<usize>, colours: bool) -> RawMesh {
    let last = slots.max(1) - 1;
    let mut out = RawMesh { name, material, ..RawMesh::default() };
    for (t, &s) in mesh.slots.iter().enumerate() {
        if !wanted.contains(&(s.max(0) as usize).min(last)) {
            continue;
        }
        for c in 3 * t..3 * t + 3 {
            out.indices.push(out.positions.len() as u32);
            out.positions.push(mesh.positions[c]);
            out.normals.push(mesh.normals[c]);
            if !mesh.uvs.is_empty() {
                out.uvs.push(mesh.uvs[c]);
            }
            if colours && !mesh.colors.is_empty() {
                out.colors.push(mesh.colors[c]);
            }
        }
    }
    out
}

/// The slots `mesh`'s triangles use, of `slots`.
fn used_slots(mesh: &Exported, slots: usize) -> Vec<usize> {
    let last = slots.max(1) - 1;
    let mut used: Vec<usize> = mesh.slots.iter().map(|&s| (s.max(0) as usize).min(last)).collect();
    used.sort_unstable();
    used.dedup();
    used
}

fn matrix(entry: Option<&Json>) -> Option<Mat4> {
    let values = entry?.as_f32_array()?;
    if values.len() != 16 || values.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let mut m = Mat4::IDENTITY;
    for row in 0..4 {
        for column in 0..4 {
            m.m[row][column] = values[row * 4 + column];
        }
    }
    Some(m)
}

/// Blender's answer as a scene: `packed` gives back the pictures it wrote
/// out, by file name. Reads no file itself, so it can be tested on an
/// answer kept beside the tests.
pub(crate) fn scene_from(answer: &Json, bin: &[u8], packed: &dyn Fn(&str) -> Option<Arc<Vec<u8>>>, name: &str) -> Result<RawScene, ImportError> {
    if answer.get("format").and_then(Json::as_str) != Some(FORMAT) {
        return Err(ImportError::Malformed("Blender's answer is not one Tequila asked for".into()));
    }
    if answer.get("version").and_then(Json::as_index).map_or(true, |v| v > VERSION) {
        return Err(ImportError::Unsupported("Blender's answer is from a newer Tequila".into()));
    }
    let mut notes = Notes::default();
    for warning in answer.get("warnings").map(Json::as_array).unwrap_or(&[]) {
        if let Some(w) = warning.as_str() {
            notes.add(w.to_string());
        }
    }
    let unit_scale = answer.get("unit_scale").and_then(Json::as_f32).filter(|s| s.is_finite() && *s > 0.0).unwrap_or(1.0);
    let list = |key: &str| answer.get(key).map(Json::as_array).unwrap_or(&[]);

    let meshes: Vec<Exported> = list("meshes").iter().map(|m| exported(m, bin)).collect::<Result<_, _>>()?;
    let mut textures: Vec<RawTexture> = Vec::new();
    let cx = material::Context { trees: list("trees"), images: list("images"), packed, unit_scale };
    let materials: Vec<_> = list("materials").iter().map(|m| material::material(m, &cx, &mut textures, &mut notes)).collect();
    // Which materials read the mesh's own colours: only their meshes carry
    // them, so a colour attribute nothing uses tints nothing.
    let reads_colours: Vec<bool> = materials.iter().map(|m| m.graph.nodes().any(|(_, n)| matches!(n.kind, tequila_materials::graph::NodeKind::VertexColor))).collect();

    let entries = list("nodes");
    // What holds something: a part, or a group with a part in it somewhere.
    let mut content = vec![false; entries.len()];
    for i in (0..entries.len()).rev() {
        let mesh = entries[i].get("mesh").and_then(Json::as_index).filter(|&m| m < meshes.len());
        if mesh.is_some_and(|m| !meshes[m].slots.is_empty()) {
            content[i] = true;
        }
        if content[i] {
            if let Some(p) = entries[i].get("parent").and_then(Json::as_index).filter(|&p| p < i) {
                content[p] = true;
            }
        }
    }

    let mut scene = RawScene { textures: Vec::new(), materials, ..RawScene::default() };
    let mut made: HashMap<(usize, Vec<usize>, usize, Option<usize>), usize> = HashMap::new();
    let mut places: Vec<Option<usize>> = vec![None; entries.len()];
    for (i, entry) in entries.iter().enumerate() {
        if i > 0 && !content[i] {
            continue;
        }
        let parent = entry.get("parent").and_then(Json::as_index).filter(|&p| p < i).and_then(|p| places[p]);
        if i > 0 && parent.is_none() {
            continue;
        }
        let node_name = entry.get("name").and_then(Json::as_str).unwrap_or("Object").to_string();
        let transform = if i == 0 {
            Transform::from_matrix(Z_UP_TO_Y_UP).expect("a turn inverts")
        } else {
            match matrix(entry.get("matrix")) {
                None => Transform::IDENTITY,
                Some(m) => Transform::from_matrix(m).unwrap_or_else(|| {
                    notes.add(format!("{node_name} is scaled to nothing on some axis and was placed unscaled"));
                    Transform::IDENTITY
                }),
            }
        };
        let mut metadata = BTreeMap::new();
        if let Some(Json::Object(map)) = entry.get("metadata") {
            for (k, v) in map {
                if let Some(s) = v.as_str() {
                    metadata.insert(k.clone(), s.to_string());
                }
            }
        }
        let hidden = entry.get("hidden").and_then(Json::as_bool).unwrap_or(false);
        let mut node = RawNode { name: if i == 0 { name.to_string() } else { node_name.clone() }, parent, transform, mesh: None, metadata, layers: Vec::new(), hidden };
        let index = scene.nodes.len();
        let mesh = entry.get("mesh").and_then(Json::as_index).filter(|&m| m < meshes.len());
        let slot_materials: Vec<Option<usize>> = entry.get("materials").map(Json::as_array).unwrap_or(&[]).iter().map(|m| m.as_index().filter(|&m| m < scene.materials.len())).collect();
        let mut children = Vec::new();
        if let Some(m) = mesh {
            let slots = slot_materials.len();
            let material_of = |slot: usize| slot_materials.get(slot).copied().flatten();
            // The slots in use, gathered by material: two slots of one
            // material are one part.
            let mut by_material: Vec<(Option<usize>, Vec<usize>)> = Vec::new();
            for slot in used_slots(&meshes[m], slots) {
                let material = material_of(slot);
                match by_material.iter_mut().find(|(k, _)| *k == material) {
                    Some((_, list)) => list.push(slot),
                    None => by_material.push((material, vec![slot])),
                }
            }
            let mut place = |material: Option<usize>, wanted: &[usize], part_name: String| -> usize {
                let colours = material.is_some_and(|k| reads_colours[k]);
                *made.entry((m, wanted.to_vec(), slots, material)).or_insert_with(|| {
                    scene.meshes.push(part_mesh(&meshes[m], wanted, slots, part_name, material, colours));
                    scene.meshes.len() - 1
                })
            };
            if by_material.len() == 1 {
                let (material, wanted) = &by_material[0];
                node.mesh = Some(place(*material, wanted, meshes[m].name.clone()));
            } else {
                // A node carries one material: one part for each.
                for (material, wanted) in &by_material {
                    let label = material.map(|k| scene.materials[k].name.clone()).unwrap_or_else(|| format!("slot {}", wanted[0] + 1));
                    let mesh = place(*material, wanted, format!("{} [{label}]", meshes[m].name));
                    children.push((format!("{node_name} [{label}]"), mesh));
                }
            }
        }
        scene.nodes.push(node);
        places[i] = Some(index);
        for (child_name, mesh) in children {
            let mut child = RawNode::part(child_name, mesh);
            child.parent = Some(index);
            child.hidden = hidden;
            scene.nodes.push(child);
        }
    }
    if scene.nodes.len() <= 1 || scene.meshes.is_empty() {
        return Err(ImportError::Malformed("the file's scene has nothing Tequila can show: no meshes, curves or text".into()));
    }
    scene.roots = vec![0];
    scene.textures = textures;

    // Its unit: Blender's numbers times its unit scale are metres.
    scene.source_unit = Some(match unit_scale {
        s if (s - 1.0).abs() < 1e-6 => tequila_core::units::LengthUnit::Metre,
        s if (s - 0.01).abs() < 1e-8 => tequila_core::units::LengthUnit::Centimetre,
        s if (s - 0.001).abs() < 1e-9 => tequila_core::units::LengthUnit::Millimetre,
        s if (s - 0.0254).abs() < 1e-8 => tequila_core::units::LengthUnit::Inch,
        s if (s - 0.3048).abs() < 1e-7 => tequila_core::units::LengthUnit::Foot,
        s => {
            crate::rescale(&mut scene, s as f64);
            tequila_core::units::LengthUnit::Metre
        }
    });

    scene.setup = setup(answer, &scene, &mut notes);
    scene.warnings = notes.0;
    Ok(scene)
}

/// The scene's cameras, and its render size and look for a scene opened
/// from it.
fn setup(answer: &Json, scene: &RawScene, notes: &mut Notes) -> Option<Box<RawSetup>> {
    let cameras = answer.get("cameras").map(Json::as_array).unwrap_or(&[]);
    if cameras.is_empty() {
        return None;
    }
    let render = answer.get("render");
    let size = |key: &str| render.and_then(|r| r.get(key)).and_then(Json::as_index).filter(|&s| s > 0);
    let (width, height) = (size("width").unwrap_or(1920) as f32, size("height").unwrap_or(1080) as f32);
    let centre = centre_of(scene);
    let mut setup = RawSetup::default();
    for c in cameras {
        if let Some(camera) = camera(c, width / height, centre, notes) {
            setup.cameras.push(camera);
        }
    }
    let active = answer.get("camera").and_then(Json::as_str);
    setup.active_camera = active.and_then(|a| setup.cameras.iter().position(|c| c.name == a));
    let mut settings = tequila_scene::render::RenderSettings::default();
    if let (Some(w), Some(h)) = (size("width"), size("height")) {
        settings.width = w;
        settings.height = h;
    }
    setup.render = Some(settings);
    let mut style = tequila_color::ImageStyle::default();
    if let Some(view) = render.and_then(|r| r.get("view_transform")).and_then(Json::as_str) {
        use tequila_color::DisplayTransform as D;
        style.display = match view {
            "Standard" => D::Srgb,
            "Khronos PBR Neutral" => D::PbrNeutral,
            "AgX" => D::AgX,
            "Raw" => D::Raw,
            v if v.starts_with("ACES") => D::Aces,
            "Filmic" => {
                notes.add("Blender's Filmic view came over as AgX, its successor".into());
                D::AgX
            }
            other => {
                notes.add(format!("Blender's {other} view has no counterpart; Tequila's own was kept"));
                style.display
            }
        };
    }
    style.exposure = render.and_then(|r| r.get("exposure")).and_then(Json::as_f32).unwrap_or(0.0);
    if render.and_then(|r| r.get("look")).and_then(Json::as_str).is_some_and(|l| l != "None") {
        notes.add("Blender's look (in Color Management) has no counterpart and was left out".into());
    }
    setup.image_style = Some(style);
    Some(Box::new(setup))
}

/// The middle of the scene's parts, in Tequila's world: what a camera
/// without a focus distance orbits.
fn centre_of(scene: &RawScene) -> Option<Vec3> {
    let mut world: Vec<Option<Mat4>> = Vec::with_capacity(scene.nodes.len());
    let (mut low, mut high) = (Vec3::ONE * f32::INFINITY, Vec3::ONE * f32::NEG_INFINITY);
    for node in &scene.nodes {
        let m = match node.parent.and_then(|p| world.get(p).copied().flatten()) {
            Some(parent) => parent * node.transform.matrix,
            None => node.transform.matrix,
        };
        world.push(Some(m));
        if let Some(mesh) = node.mesh.and_then(|k| scene.meshes.get(k)) {
            if node.hidden {
                continue;
            }
            for p in mesh.positions.iter().step_by(7) {
                let q = m.transform_point(*p);
                low = low.min(q);
                high = high.max(q);
            }
        }
    }
    (low.x <= high.x).then(|| (low + high) * 0.5)
}

/// A Blender camera as Tequila's: the same place, lens and framing.
fn camera(c: &Json, aspect: f32, centre: Option<Vec3>, notes: &mut Notes) -> Option<SavedCamera> {
    let name = c.get("name").and_then(Json::as_str).unwrap_or("Camera").to_string();
    let m = Z_UP_TO_Y_UP * matrix(c.get("matrix"))?;
    let column = |i: usize| vec3(m.m[0][i], m.m[1][i], m.m[2][i]);
    // A camera looks down its -Z with +Y up, in Blender as in Tequila; its
    // own scale is not a lens.
    let back = column(2).try_normalize()?;
    let right = column(1).cross(back).try_normalize()?;
    let up = back.cross(right);
    let position = column(3);
    let mut to_world = Mat4::IDENTITY;
    for (i, axis) in [right, up, back, position].iter().enumerate() {
        to_world.m[0][i] = axis.x;
        to_world.m[1][i] = axis.y;
        to_world.m[2][i] = axis.z;
    }
    let number = |key: &str| c.get(key).and_then(Json::as_f32).filter(|v| v.is_finite());
    let forward = -back;
    let dof = c.get("dof").and_then(Json::as_bool).unwrap_or(false);
    let focus = number("focus").filter(|f| *f > 0.0);
    let distance = match (dof, focus, centre) {
        (true, Some(f), _) => f,
        (_, _, Some(centre)) => (centre - position).dot(forward).max(0.01),
        (_, Some(f), None) => f,
        _ => 1.0,
    };
    let mut camera = Camera { camera_to_world: to_world, focus_distance: distance, ..Camera::default() };
    camera.sensor_width_mm = number("sensor").filter(|s| *s > 0.0).unwrap_or(36.0);
    let lens = number("lens").filter(|l| *l > 0.0).unwrap_or(50.0);
    let shift = c.get("shift").and_then(Json::as_f32_array).unwrap_or_default();
    let (shift_x, shift_y) = (shift.first().copied().unwrap_or(0.0), shift.get(1).copied().unwrap_or(0.0));
    camera.projection = match c.get("type").and_then(Json::as_str) {
        Some("ORTHO") => {
            // Blender's scale is across the wider side; Tequila's up it.
            let scale = number("ortho_scale").unwrap_or(1.0);
            let across = c.get("ortho_across").and_then(Json::as_str) == Some("HORIZONTAL");
            Projection::Orthographic { scale: if across { scale / aspect } else { scale } }
        }
        Some("PANO") => {
            notes.add(format!("the camera {name} is a panorama; it came over as a lens of its focal length"));
            Projection::Perspective { focal_length_mm: lens }
        }
        // Blender shifts by the wider side; Tequila by each side's own.
        _ if shift_x != 0.0 || shift_y != 0.0 => Projection::Shift { focal_length_mm: lens, shift_x, shift_y: shift_y * aspect.max(1.0 / aspect) },
        _ => Projection::Perspective { focal_length_mm: lens },
    };
    if dof {
        camera.f_number = number("fstop").filter(|f| *f > 0.0).unwrap_or(f32::INFINITY);
        let blades = c.get("blades").and_then(Json::as_index).unwrap_or(0);
        if blades >= 3 {
            camera.aperture = Aperture::Bladed { blades: blades as u32, rotation: number("blade_rotation").unwrap_or(0.0) };
        }
    }
    Some(SavedCamera { name, camera, target: position + forward * distance })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_blender_is_found_first() {
        let folder = std::env::temp_dir().join(format!("tequila-blender-versions-{}", std::process::id()));
        for name in ["Blender 3.6.app", "Blender 4.2.app", "Blender.app", "Blender 4.10.app", "Notes.app"] {
            std::fs::create_dir_all(folder.join(name)).unwrap();
        }
        let found: Vec<String> = newest_first(&folder, |n| n.starts_with("Blender") && n.ends_with(".app")).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        let _ = std::fs::remove_dir_all(&folder);
        assert_eq!(found, ["Blender.app", "Blender 4.10.app", "Blender 4.2.app", "Blender 3.6.app"]);
    }

    #[test]
    fn what_blender_says_is_wrong_is_found_in_what_it_printed() {
        let said = "Blender 5.2.0\nRead blend: x\nError: Cannot read file \"x.blend\": File format is not supported\n\nBlender quit\n";
        assert!(complaint(said).starts_with("Error: Cannot read file"), "{}", complaint(said));
        assert!(complaint("Blender 3.6.0 is too old for Tequila to read its files: 4.2 or later is needed\n").contains("too old"));
        assert_eq!(complaint(""), "it stopped without saying why");
    }
}
