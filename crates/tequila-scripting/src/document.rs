//! The answers to the scripting channel's queries that need only the
//! document: its parts with their world bounds, its materials, its size,
//! and the description of the API itself.
//!
//! Shared by the application and the headless server (`tequila --serve`),
//! so a script reads the same shapes from either, and a client written
//! against one works against the other.

use tequila_core::math::{Aabb, Vec3};
use tequila_format::json::Value;
use tequila_format::json_write::Object;
use tequila_scene::document::SceneDocument;
use tequila_scene::tree::{NodeId, NodeKind};

/// The protocol revision, so a client can refuse an end it does not
/// understand rather than misreading it.
pub const PROTOCOL: f64 = 1.0;

/// What the application answers.
pub const APP_OPERATIONS: &[&str] = &["ping", "query", "apply", "batch", "undo", "redo", "select", "import", "reimport", "texture_set", "save", "frame", "look", "screenshot", "render", "queue", "export", "livelink", "library", "material", "plugin", "stop", "ui", "new", "open", "quit"];
pub const APP_QUERIES: &[&str] = &["api", "context", "scene", "nodes", "materials", "selection", "camera", "commands", "ui", "jobs", "history", "library"];
pub const APP_LIMITS: &str = "No embedded Python interpreter. Screenshot completion is deferred: poll for the file. Paths refer to this computer. quit refuses unsaved work unless given save:true or discard:true.";

/// What a headless session (`tequila --serve`) answers: what needs only
/// the document.
pub const HEADLESS_OPERATIONS: &[&str] = &["ping", "query", "apply", "batch", "undo", "redo", "select", "import", "reimport", "save", "frame", "look", "render", "export", "library", "material", "new", "open", "quit"];
pub const HEADLESS_QUERIES: &[&str] = &["api", "context", "scene", "nodes", "materials", "selection", "camera", "commands", "history", "library"];
pub const HEADLESS_LIMITS: &str = "Headless: there is no window, so no screenshot, ui, texture_set, livelink or background queue; render blocks until the picture is written. Paths refer to this computer. quit refuses unsaved work unless given save:true or discard:true.";

/// What a client needs to know to use the channel: the protocol, the
/// operations and queries this end answers, how to batch, and an example of
/// every command. The application and the headless server each say which
/// operations they take, and what they cannot do.
pub fn api(operations: &[&str], queries: &[&str], limits: &str) -> Value {
    use tequila_materials::material_type::{MaterialType, Param};
    let strings = |items: &[&str]| Value::Array(items.iter().map(|s| Value::String((*s).into())).collect());
    let mut out = Object::new();
    out.set("protocol", PROTOCOL)
        .set("examples", tequila_format::json::parse(include_bytes!("examples.json")).expect("tested API examples"))
        .set("transport", "Authenticated loopback TCP; one JSON request/reply per line")
        .set("queries", strings(queries))
        .set("operations", strings(operations))
        .set("coordinates", "Right-handed, Y up; positions and every other length are in metres; document.units is only how the app shows them. Colors are scene-linear RGB.")
        .set("workflow", "Query context; resolve IDs; inspect shared materials; apply edits; frame; screenshot or render; query context to verify.")
        .set("batches", "Send many operations as {\"op\":\"batch\",\"ops\":[...]}: they are one undo step, and if one fails the whole batch is taken back (\"atomic\":false keeps what worked). Undo, redo and quit cannot be inside a batch.")
        .set("camera", "look: {\"view\":\"front|back|left|right|top|bottom|iso\"} or {\"at\":[x,y,z],\"yaw\":deg,\"pitch\":deg,\"distance\":d} or {\"from\":[x,y,z],\"at\":[x,y,z]}, plus optional focal_length in mm. Undoable.")
        .set("rendering", "render blocks the UI until done and uses the current view; queue renders in the background with the document's render settings (override width, height, samples, folder, filename, denoise, camera) and reports progress through the jobs query; stop ends the running job, which still writes its image.")
        .set("seeing", "screenshot with \"viewport\":true captures just the picture. ui opens and closes panels and switches the viewport between rendered and solid; query ui to see what is open.")
        .set("limits", limits);
    let parameters = Param::ALL.iter().map(|p| {
        let mut entry = Object::new();
        entry.set("key", p.key()).set("label", p.label()).set("kind", format!("{:?}", p.ui()));
        if let Some(range) = p.range() { entry.set("min", range.low).set("max", range.high); }
        entry.build()
    }).collect();
    out.set("material_parameters", Value::Array(parameters));
    out.set("material_types", Value::Array(MaterialType::ALL.iter().map(|kind| {
        let mut entry = Object::new();
        entry.set("id", kind.id()).set("label", kind.label());
        entry.set("parameters", Value::Array(kind.exposed().iter().map(|p| Value::String(p.key().into())).collect()));
        entry.build()
    }).collect()));
    out.set("commands", Value::Array(tequila_scene::Command::CATALOG.iter().map(|name| Value::String((*name).into())).collect()));
    // An example of every command, exactly as apply takes it: the journal's
    // own round-trip samples, so each is known to be accepted. Ids and keys
    // in them are placeholders — read the real ones from the scene first.
    let mut examples = Object::new();
    for (name, example) in tequila_format::examples::by_name() {
        examples.set(name, example);
    }
    out.set("command_examples", examples.build());
    // Every pattern, at its defaults, in exactly the shape material.set_procedural
    // takes — so asking for granite is a copy and an edit, not a guess.
    out.set("procedurals", Value::Array(tequila_texture::procedural::Procedural::catalog().iter().map(tequila_format::procedural_to_json).collect()));
    out.build()
}

pub fn vector(v: Vec3) -> Value {
    Value::Array(vec![
        Value::Number(v.x as f64),
        Value::Number(v.y as f64),
        Value::Number(v.z as f64),
    ])
}

/// A box as its corners and size, or `null` for one that holds nothing - an
/// empty scene, a part without triangles. Its corners are infinite, which
/// JSON cannot carry: written as numbers they made the whole reply
/// unreadable, so a script, a plugin or an AI app asking about a new, empty
/// project got a broken connection instead of an answer.
pub fn bounds_value(bounds: Aabb) -> Value {
    let finite = |v: Vec3| v.x.is_finite() && v.y.is_finite() && v.z.is_finite();
    if bounds.is_empty() || !finite(bounds.min) || !finite(bounds.max) {
        return Value::Null;
    }
    let mut out = Object::new();
    out.set("min", vector(bounds.min))
        .set("max", vector(bounds.max))
        .set("size", vector(bounds.max - bounds.min));
    out.build()
}

/// Where a part is in the world, which is how a script names the one it means.
///
/// The eight corners are transformed and re-bounded rather than the two
/// extremes, because a rotated box's extremes are not the transforms of the
/// original extremes — taking the shortcut gives a box that is too small, and
/// too small in a way that depends on the rotation, which is the worst kind of
/// wrong for something a script is using to pick parts by size.
pub fn world_bounds(document: &SceneDocument, id: NodeId, local: Aabb) -> Aabb {
    let transform = document.tree.world_transform(id);
    let (lo, hi) = (local.min, local.max);
    let mut out = Aabb::EMPTY;
    for i in 0..8 {
        let corner = Vec3 {
            x: if i & 1 == 0 { lo.x } else { hi.x },
            y: if i & 2 == 0 { lo.y } else { hi.y },
            z: if i & 4 == 0 { lo.z } else { hi.z },
        };
        out.extend_point(transform.point(corner));
    }
    out
}

pub fn nodes(document: &SceneDocument) -> Value {
    let mut list = Vec::new();
    for node in document.tree.iter() {
        let mut entry = Object::new();
        entry
            .set("id", node.id.0 as f64)
            .set("name", node.name.clone())
            .set("visible", node.visible);
        entry.set_some("parent", node.parent.map(|p| p.0 as f64));

        match node.kind {
            NodeKind::Part { mesh } => {
                entry.set("kind", "part");
                if let Some(source) = document.geometry.get(mesh) {
                    entry
                        .set("triangles", (source.indices.len() / 3) as f64)
                        .set("bounds", bounds_value(world_bounds(document, node.id, source.bounds())));
                }
            }
            NodeKind::Group => {
                entry.set("kind", "group");
            }
            NodeKind::Light { .. } => {
                entry.set("kind", "light");
            }
            NodeKind::Cutaway { .. } => {
                entry.set("kind", "cutaway");
            }
        }

        if let Some(key) = node.material {
            entry.set("material", key.0 as f64);
            if let Some(definition) = document.materials.get(key) {
                entry.set("material_name", definition.name.clone());
            }
        }
        // CAD attributes and notes (docs/03 §6.5), so a script can find
        // parts by them: [n for n in nodes if n["metadata"].get("Material") == "ABS"].
        let mut fields = Object::new();
        for (key, value) in &node.metadata {
            fields.set(key, value.clone());
        }
        entry.set("metadata", fields.build());
        list.push(entry.build());
    }
    let mut out = Object::new();
    out.set("nodes", Value::Array(list));
    out.build()
}

pub fn materials(document: &SceneDocument) -> Value {
    let mut list = Vec::new();
    for (key, definition) in document.materials.iter() {
        let used = document
            .tree
            .iter()
            .filter(|n| n.material == Some(key))
            .count();
        let mut entry = Object::new();
        entry
            .set("id", key.0 as f64)
            .set("name", definition.name.clone())
            .set("definition", tequila_format::material_to_json(key, definition))
            .set("type", definition.material_type.label())
            .set("used_by", used as f64);

        // Which files this material reads, by the role each one plays. A
        // script checking that an import wired its maps has no other way to
        // see them.
        let used_here: std::collections::BTreeSet<_> = definition
            .graph
            .nodes()
            .filter_map(|(_, node)| match node.kind {
                tequila_materials::graph::NodeKind::ImageTexture { texture, .. } => Some(texture),
                tequila_materials::graph::NodeKind::ImageAlpha { texture, .. } => Some(texture),
                _ => None,
            })
            .collect();
        let mut maps = Vec::new();
        for (id, source) in document.textures.iter() {
            if used_here.contains(&id) {
                let mut map = Object::new();
                map.set("id", id.0 as f64)
                    .set("role", format!("{:?}", source.role))
                    .set("path", source.path.display().to_string());
                maps.push(map.build());
            }
        }
        entry.set("textures", Value::Array(maps));
        list.push(entry.build());
    }
    let mut out = Object::new();
    out.set("materials", Value::Array(list));
    out.build()
}

/// The `library` query: every preset, the shipped ones and the user's own,
/// with what a script needs to pick one - its name, category, tags, type -
/// and where the user's own are kept.
pub fn library(user: &std::path::Path) -> Value {
    use tequila_format::library::{read_libraries, standard_directory, Source};
    let (entries, problems) = read_libraries(&standard_directory(), user);
    let presets = entries
        .iter()
        .map(|entry| {
            let mut out = Object::new();
            out.set("name", entry.name.clone())
                .set("category", entry.category.clone())
                .set("tags", Value::Array(entry.tags.iter().map(|t| Value::String(t.clone())).collect()))
                .set("type", entry.material_type.label())
                .set("source", match entry.source {
                    Source::User => "user",
                    Source::Plugin => "plugin",
                    Source::Standard => "standard",
                })
                .set("file", entry.path.clone());
            out.build()
        })
        .collect();
    let mut out = Object::new();
    out.set("presets", Value::Array(presets))
        .set("user_folder", user.display().to_string())
        .set("problems", Value::Array(problems.into_iter().map(Value::String).collect()));
    out.build()
}

/// What a `library` request comes to.
pub enum LibraryOutcome {
    /// Apply these to the document, in order, as one undo step, then answer
    /// with the value.
    Commands(Vec<tequila_scene::Command>, Value),
    /// Done - the user's library changed, or a file was written - and the
    /// value says what.
    Done(Value),
}

/// A texture pack made into a library entry: `(path, size, name, category,
/// tags)`. Passed in, because reading a pack is the importer's and this
/// crate does not depend on it.
pub type PackReader<'a> = &'a dyn Fn(&std::path::Path, f32, String, String, Vec<String>) -> Result<tequila_format::library::LibraryEntry, String>;

/// Carry out a `library` request (see `LibraryRequest` for the actions),
/// for the application and a headless session alike: `apply` comes back as
/// commands for the caller's bus, the rest act on the user's library at
/// `user` (`library::user_directory()`, but for a test).
pub fn library_request(
    request: &crate::protocol::LibraryRequest,
    document: &SceneDocument,
    selection: &[NodeId],
    user: &std::path::Path,
    pack: PackReader,
) -> Result<LibraryOutcome, String> {
    use tequila_format::library::{self, LibraryEntry, Source};
    let user = user.to_path_buf();
    let (entries, _) = library::read_libraries(&library::standard_directory(), &user);
    // A name in both libraries means the user's own: theirs is the one
    // they meant to change.
    let find = |name: &str, own: bool| -> Result<&LibraryEntry, String> {
        entries
            .iter()
            .filter(|e| e.name.eq_ignore_ascii_case(name) && (!own || e.source == Source::User))
            .max_by_key(|e| e.source == Source::User)
            .ok_or_else(|| {
                if own && entries.iter().any(|e| e.name.eq_ignore_ascii_case(name)) {
                    format!("`{name}` is part of the shipped library, which is read-only")
                } else {
                    format!("there is no preset called `{name}`; `query library` lists them")
                }
            })
    };
    let name = request.name.clone().unwrap_or_default();
    let done = |key: &str, name: &str, file: Option<&std::path::Path>| {
        let mut out = Object::new();
        out.set(key, name.to_string());
        if let Some(file) = file {
            out.set("file", file.display().to_string());
        }
        LibraryOutcome::Done(out.build())
    };
    match request.action.as_str() {
        "apply" => {
            let entry = find(&name, false)?;
            let nodes: Vec<NodeId> = if request.nodes.is_empty() {
                selection.to_vec()
            } else {
                request.nodes.iter().map(|&n| NodeId(n)).collect()
            };
            if nodes.is_empty() {
                return Err("nothing to put it on: name `nodes`, or select some parts".into());
            }
            if let Some(missing) = nodes.iter().find(|n| document.tree.get(**n).is_none()) {
                return Err(format!("no such part: {}", missing.0));
            }
            // A group stands for the parts in it, which is what is counted.
            let parts = tequila_scene::apply::parts_of(document, nodes.iter().copied());
            if parts.is_empty() {
                return Err("there are no parts in what was named: lights and empty groups take no material".into());
            }
            let commands = library::apply_commands(entry, document, &parts, request.unlink);
            let mut out = Object::new();
            out.set("applied", entry.name.clone()).set("parts", parts.len() as f64);
            Ok(LibraryOutcome::Commands(commands, out.build()))
        }
        "save" => {
            let key = tequila_scene::tree::MaterialKey(request.material.unwrap_or_default());
            let definition = document.materials.get(key).ok_or_else(|| format!("no material {}", key.0))?;
            let name = request.name.clone().unwrap_or_else(|| definition.name.clone());
            let category = request.category.clone().unwrap_or_else(|| "My materials".into());
            let mut entry = LibraryEntry::from_graph_with_pictures(name.clone(), category, definition.material_type, &definition.graph, &document.textures, Source::User);
            entry.tags = request.tags.clone();
            let file = library::save_entry(&user, &entry).map_err(|e| e.to_string())?;
            Ok(done("saved", &name, Some(&file)))
        }
        "import" => {
            let path = std::path::PathBuf::from(request.path.clone().unwrap_or_default());
            if !path.exists() {
                return Err(format!("{} does not exist", path.display()));
            }
            // What a KeyShot package could not bring across.
            let mut notes: Vec<String> = Vec::new();
            let file = if tequila_import::keyshot::is_kmp(&path) {
                let (file, kept) = tequila_import::keyshot::import_to_library(&path, &user, |entry| {
                    if let Some(name) = &request.name {
                        entry.name = name.clone();
                    }
                    if let Some(category) = &request.category {
                        entry.category = category.clone();
                    }
                    if !request.tags.is_empty() {
                        entry.tags = request.tags.clone();
                    }
                })
                .map_err(|e| e.to_string())?;
                notes = kept;
                file
            } else if library::is_material_file(&path) {
                library::import_material_file(&path, &user, |entry| {
                    if let Some(name) = &request.name {
                        entry.name = name.clone();
                    }
                    if let Some(category) = &request.category {
                        entry.category = category.clone();
                    }
                    if !request.tags.is_empty() {
                        entry.tags = request.tags.clone();
                    }
                })
                .map_err(|e| e.to_string())?
            } else {
                let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Texture pack".into());
                let entry = pack(
                    &path,
                    request.size.unwrap_or(0.3),
                    request.name.clone().unwrap_or(stem),
                    request.category.clone().unwrap_or_else(|| "Textured".into()),
                    request.tags.clone(),
                )?;
                library::save_entry(&user, &entry).map_err(|e| e.to_string())?
            };
            let (now, _) = library::read_directory_from(&user, Source::User);
            let imported = now.iter().find(|e| std::path::Path::new(&e.path) == file).map(|e| e.name.clone()).unwrap_or_default();
            let mut out = Object::new();
            out.set("imported", imported).set("file", file.display().to_string());
            if !notes.is_empty() {
                out.set("notes", Value::Array(notes.into_iter().map(Value::String).collect()));
            }
            Ok(LibraryOutcome::Done(out.build()))
        }
        "export" => {
            let entry = find(&name, false)?;
            let to = std::path::PathBuf::from(request.path.clone().unwrap_or_default());
            library::export_package(entry, &to).map_err(|e| e.to_string())?;
            Ok(done("exported", &entry.name, Some(&to)))
        }
        "delete" => {
            let entry = find(&name, true)?;
            library::delete_entry(entry).map_err(|e| e.to_string())?;
            Ok(done("deleted", &entry.name, None))
        }
        other => Err(format!("`{other}` is not a library action")),
    }
}

/// How big the scene is: parts, nodes, materials, textures, its bounds, and
/// where it is saved.
pub fn scene(document: &SceneDocument, path: Option<&std::path::Path>) -> Value {
    let parts = document.tree.iter().filter(|n| matches!(n.kind, NodeKind::Part { .. })).count();
    let mut out = Object::new();
    out.set("parts", parts as f64)
        .set("nodes", document.tree.iter().count() as f64)
        .set("materials", document.materials.iter().count() as f64)
        .set("textures", document.textures.len() as f64)
        .set("bounds", bounds_value(tequila_scene::snapshot::visible_bounds(document)));
    if let Some(path) = path {
        out.set("path", path.display().to_string());
    }
    out.build()
}

/// The command catalog, straight from the build, so this cannot describe a
/// build it is not part of.
pub fn commands() -> Value {
    let names: Vec<Value> = tequila_scene::command::Command::CATALOG.iter().map(|name| Value::String((*name).to_string())).collect();
    let mut out = Object::new();
    out.set("commands", Value::Array(names)).set("note", "send one as {\"op\":\"apply\",\"command\":{\"command\":<name>, ...}}");
    out.build()
}

/// Where a camera is, in the terms `look` takes.
pub fn camera(orbit: &tequila_scene::orbit::OrbitCamera, name: &str) -> Value {
    let mut out = Object::new();
    out.set("name", name)
        .set("position", vector(orbit.camera.position()))
        .set("target", vector(orbit.target))
        .set("yaw", orbit.yaw.to_degrees())
        .set("pitch", orbit.pitch.to_degrees())
        .set("distance", orbit.distance);
    match orbit.camera.projection {
        tequila_render_core::camera::Projection::Perspective { focal_length_mm } | tequila_render_core::camera::Projection::Shift { focal_length_mm, .. } => {
            out.set("focal_length", focal_length_mm);
        }
        _ => {}
    }
    out.build()
}

#[cfg(test)]
mod library_tests {
    use super::*;
    use crate::protocol::LibraryRequest;

    fn no_packs(_: &std::path::Path, _: f32, _: String, _: String, _: Vec<String>) -> Result<tequila_format::library::LibraryEntry, String> {
        Err("no packs here".into())
    }

    fn request(action: &str) -> LibraryRequest {
        LibraryRequest { action: action.into(), ..LibraryRequest::default() }
    }

    /// What a script does with the library, end to end on a library of its
    /// own: a shipped preset put on a part as commands for the bus, a
    /// document material saved, exported, imported back and deleted - and
    /// the shipped presets left alone.
    #[test]
    fn a_script_uses_the_library_as_the_browser_does() {
        let user = std::env::temp_dir().join(format!("tequila-scripting-library-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&user);
        let mut document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.2);
        let part = document.tree.iter().find(|n| n.mesh().is_some()).map(|n| n.id).expect("a part");

        let listed = library(&user);
        let presets = listed.get("presets").map(|p| p.as_array().len()).unwrap_or(0);
        assert!(presets > 200, "the shipped library was not listed: {presets}");

        let mut apply = request("apply");
        apply.name = Some("aluminium spun".into());
        assert!(library_request(&apply, &document, &[], &user, &no_packs).is_err(), "applied with nothing to apply to");
        let Ok(LibraryOutcome::Commands(commands, _)) = library_request(&apply, &document, &[part], &user, &no_packs) else {
            panic!("apply did not come back as commands");
        };
        let mut bus = tequila_scene::command::CommandBus::new();
        for command in commands {
            bus.apply(&mut document, command).expect("applies");
        }
        let key = document.tree.get(part).and_then(|n| n.material).expect("assigned");
        assert_eq!(document.materials.get(key).unwrap().name, "Aluminium Spun");

        let mut save = request("save");
        save.material = Some(key.0);
        save.name = Some("Spun Housing".into());
        save.category = Some("Housings".into());
        assert!(matches!(library_request(&save, &document, &[], &user, &no_packs), Ok(LibraryOutcome::Done(_))));
        let mut export = request("export");
        export.name = Some("Spun Housing".into());
        let package = user.with_extension("fsmat");
        export.path = Some(package.display().to_string());
        assert!(library_request(&export, &document, &[], &user, &no_packs).is_ok());
        let mut import = request("import");
        import.path = Some(package.display().to_string());
        import.name = Some("Spun Housing Imported".into());
        assert!(library_request(&import, &document, &[], &user, &no_packs).is_ok());
        let (mine, _) = tequila_format::library::read_directory_from(&user, tequila_format::library::Source::User);
        let names: Vec<&str> = mine.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"Spun Housing") && names.contains(&"Spun Housing Imported"), "{names:?}");

        let mut delete = request("delete");
        delete.name = Some("Spun Housing Imported".into());
        assert!(library_request(&delete, &document, &[], &user, &no_packs).is_ok());
        delete.name = Some("ABS White".into());
        let refused = library_request(&delete, &document, &[], &user, &no_packs).err().unwrap_or_default();
        assert!(refused.contains("read-only"), "{refused}");
        delete.name = Some("No Such Thing".into());
        assert!(library_request(&delete, &document, &[], &user, &no_packs).is_err());
        let _ = std::fs::remove_dir_all(&user);
        let _ = std::fs::remove_file(&package);
    }
}

#[cfg(test)]
mod bounds_tests {
    use super::*;

    /// A new, empty project answers with readable JSON: its bounds are
    /// `null`, not infinite corners. Before, the reply to `scene` (and to
    /// `context`, which carries it) did not parse, and the connection ended.
    #[test]
    fn an_empty_scene_answers_with_readable_json() {
        let document = SceneDocument::new();
        let reply = crate::protocol::encode_reply(1, &crate::protocol::Reply::Value(scene(&document, None)));
        let parsed = tequila_format::json::parse(reply.as_bytes()).unwrap_or_else(|e| panic!("{e}: {reply}"));
        let bounds = parsed.get("result").and_then(|r| r.get("bounds")).expect("bounds are reported");
        assert!(matches!(bounds, Value::Null), "{bounds:?}");

        let demo = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.2);
        let size = scene(&demo, None).get("bounds").and_then(|b| b.get("size")).map(|s| s.as_array().len());
        assert_eq!(size, Some(3), "a scene with parts lost its bounds");
    }
}
