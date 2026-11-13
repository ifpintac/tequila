//! The answers to the scripting channel's queries that need only the
//! document: its parts with their world bounds, its materials, its size,
//! and the description of the API itself.
//!
//! Shared by the application and the headless server (`tequila --serve`),
//! so a script reads the same shapes from either, and a client written
//! against one works against the other.

use std::collections::BTreeMap;
use tequila_core::math::{Aabb, Vec3};
use tequila_format::json::Value;
use tequila_format::json_write::Object;
use tequila_scene::document::SceneDocument;
use tequila_scene::tree::{NodeId, NodeKind};

/// The protocol revision, so a client can refuse an end it does not
/// understand rather than misreading it.
pub const PROTOCOL: f64 = 1.0;

/// What the application answers.
pub const APP_OPERATIONS: &[&str] = &["ping", "query", "apply", "batch", "undo", "redo", "select", "import", "reimport", "texture_set", "save", "frame", "look", "screenshot", "render", "queue", "export", "livelink", "library", "material", "plugin", "studio_preset", "studio_kit", "substance", "stop", "ui", "preferences", "assistant", "new", "open", "quit", "uv_layout", "add_shape", "help", "guide", "read", "graph", "uv", "improve", "model_units"];
/// The application's own operations, beyond the protocol's
/// (`crate::server`): requests it answers whole, outside a batch.
pub const APP_OWN_OPERATIONS: &[&str] = &[crate::uv_layout::OPERATION];
pub const APP_QUERIES: &[&str] = &["api", "context", "scene", "nodes", "materials", "selection", "camera", "commands", "ui", "jobs", "history", "library"];
pub const APP_LIMITS: &str = "No embedded Python interpreter. Screenshot completion is deferred: poll for the file. Paths refer to this computer. quit refuses unsaved work unless given save:true or discard:true.";

/// What a headless session (`tequila --serve`) answers: what needs only
/// the document.
pub const HEADLESS_OPERATIONS: &[&str] = &["ping", "query", "apply", "batch", "undo", "redo", "select", "import", "reimport", "save", "frame", "look", "render", "export", "library", "material", "studio_preset", "studio_kit", "substance", "new", "open", "quit", "uv_layout", "help", "guide", "read", "graph", "uv", "improve", "model_units"];
/// The headless session's own operations, beyond the protocol's
/// (`crate::server`): requests it answers whole, outside a batch.
pub const HEADLESS_OWN_OPERATIONS: &[&str] = &[crate::uv_layout::OPERATION];
pub const HEADLESS_QUERIES: &[&str] = &["api", "context", "scene", "nodes", "materials", "selection", "camera", "commands", "history", "library"];
pub const HEADLESS_LIMITS: &str = "Headless: there is no window, so no screenshot, ui, texture_set, livelink or background queue; render and studio_preset render_all block until the pictures are written; substance answers info and tools only, since its maps are rendered by the application. Paths refer to this computer. quit refuses unsaved work unless given save:true or discard:true.";

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
                // Its mesh's key: parts with the same are copies of one shape
                // (an assembly's screws), which a script can treat as one.
                entry.set("kind", "part").set("mesh", mesh.0 as f64);
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
        // The CAD layers it is on, by the names scene.set_layer_visibility
        // takes: [n for n in nodes if "Construction" in n["layers"]].
        entry.set("layers", Value::Array(node.layers.iter().map(|l| Value::String(l.clone())).collect()));
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
/// with what a script needs to pick one - its name, category, tags, type
/// and what it is in a sentence - and where the user's own are kept.
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
                .set("description", entry.description.clone())
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

/// The library, searched, in a size an AI can read: the whole list is
/// hundreds of presets with their tags and files, a third of a million
/// characters, which an AI asked for a material would sooner not open than
/// read - and so made its own.
///
/// `all` is [`library`]'s answer. With neither `search` nor `category` the
/// answer is the categories and how many presets each holds, to choose from.
/// Otherwise it is the presets that have every word of `search` somewhere in
/// their name, category, type, tags or description, those with it in their
/// name first, at most `limit` of them, each with only what is needed to
/// pick one: its name (which is what applies it), category, type and a line
/// about it.
pub fn library_search(all: &Value, search: Option<&str>, category: Option<&str>, limit: usize) -> Value {
    let empty = Vec::new();
    let presets = match all.get("presets") {
        Some(Value::Array(presets)) => presets,
        _ => &empty,
    };
    let word = |preset: &Value, key: &str| preset.get(key).and_then(Value::as_str).unwrap_or_default().to_lowercase();
    let mut out = Object::new();
    out.set("total", presets.len());
    let wanted: Vec<String> = search.unwrap_or_default().to_lowercase().split_whitespace().map(str::to_string).collect();
    let category = category.map(|c| c.trim().to_lowercase()).filter(|c| !c.is_empty());
    if wanted.is_empty() && category.is_none() {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for preset in presets {
            *counts.entry(preset.get("category").and_then(Value::as_str).unwrap_or("Other").to_string()).or_default() += 1;
        }
        out.set("categories", Value::Object(counts.into_iter().map(|(name, count)| (name, Value::Number(count as f64))).collect()));
        out.set("how", "Give `search` (words such as \"black leather\" or \"brushed aluminium\") or a `category` to see presets; apply one by its name.");
        return out.build();
    }
    let mut found: Vec<(bool, &Value)> = presets
        .iter()
        .filter(|preset| category.as_ref().is_none_or(|c| word(preset, "category") == *c))
        .filter_map(|preset| {
            let name = word(preset, "name");
            let tags = preset.get("tags").map(Value::as_array).unwrap_or(&[]).iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ").to_lowercase();
            let rest = format!("{} {} {tags} {}", word(preset, "category"), word(preset, "type"), word(preset, "description"));
            wanted.iter().all(|w| name.contains(w.as_str()) || rest.contains(w.as_str())).then(|| (wanted.iter().all(|w| name.contains(w.as_str())), preset))
        })
        .collect();
    // In the name first; otherwise as the library orders them.
    found.sort_by_key(|(in_name, _)| !in_name);
    out.set("found", found.len());
    let shown: Vec<Value> = found
        .iter()
        .take(limit.max(1))
        .map(|(_, preset)| {
            let mut entry = Object::new();
            let about = preset.get("description").and_then(Value::as_str).unwrap_or_default();
            let about = match about.char_indices().nth(110) {
                Some((at, _)) => format!("{}\u{2026}", about[..at].trim_end()),
                None => about.to_string(),
            };
            entry
                .set("name", preset.get("name").and_then(Value::as_str).unwrap_or_default())
                .set("category", preset.get("category").and_then(Value::as_str).unwrap_or_default())
                .set("type", preset.get("type").and_then(Value::as_str).unwrap_or_default())
                .set("about", about);
            entry.build()
        })
        .collect();
    if found.len() > shown.len() {
        out.set("note", format!("{} more: add a word to narrow it, or raise `limit`.", found.len() - shown.len()));
    }
    if found.is_empty() {
        out.set("note", "Nothing matches every word. Try fewer or other words (a colour, a finish, what it is made of), or a `category`.");
    }
    out.set("presets", Value::Array(shown));
    out.build()
}

/// One command, for someone about to send it: an example exactly as `apply`
/// takes it, the fields in it, and what is worth knowing that the example
/// does not say. Without a name, every command's name. The whole catalog is
/// tens of thousands of characters; this is the page of it that is wanted.
pub fn help(command: Option<&str>) -> Result<Value, String> {
    let names = tequila_scene::Command::CATALOG;
    let mut out = Object::new();
    let Some(wanted) = command.map(str::trim).filter(|c| !c.is_empty()) else {
        out.set("commands", Value::Array(names.iter().map(|name| Value::String((*name).into())).collect()));
        out.set("how", "Ask again with `command` for one command's fields and an example.");
        return Ok(out.build());
    };
    let Some(name) = names.iter().find(|name| name.eq_ignore_ascii_case(wanted)) else {
        // The nearest, by the words the two share: `scene.set_visible`
        // finds `scene.set_visibility`, `scene.reshape` finds
        // `scene.set_shape`. A word counts by its first letters, so an
        // ending does not matter, and either way round, so a word run
        // together with another still counts. Only the best are named.
        let asked = wanted.to_lowercase();
        let stems: Vec<String> = asked.split(['.', '_', ' ']).filter(|w| w.len() > 2).map(|w| w.chars().take(5).collect()).collect();
        let score = |name: &str| stems.iter().filter(|stem| name.contains(stem.as_str())).count() + name.split(['.', '_']).filter(|word| word.len() > 3 && asked.contains(word)).count();
        let best = names.iter().map(|name| score(name)).max().unwrap_or(0);
        let near: Vec<&str> = if best == 0 { Vec::new() } else { names.iter().copied().filter(|name| score(name) == best).take(8).collect() };
        return Err(if near.is_empty() {
            format!("there is no command `{wanted}`; ask without a name for the list")
        } else {
            format!("there is no command `{wanted}`; did you mean one of: {}", near.join(", "))
        });
    };
    out.set("command", *name);
    if let Some((_, example)) = tequila_format::examples::by_name().into_iter().find(|(known, _)| known == name) {
        if let Value::Object(fields) = &example {
            out.set("fields", Value::Array(fields.keys().filter(|key| key.as_str() != "command").map(|key| Value::String(key.clone())).collect()));
        }
        out.set("example", example);
        out.set("note", "Send it with apply. The ids, keys and values in the example are placeholders: read the real ones from the scene first. A field not in the example is not read.");
    }
    // What the example cannot say.
    match *name {
        "environment.set_parameter" => {
            out.set("parameters", Value::Array(tequila_scene::command::EnvParam::ALL.iter().map(|p| Value::String(p.key().into())).collect()));
        }
        "scene.set_visibility" | "scene.set_locked" => {
            out.set("mind", "Each state is {\"node\": id, \"value\": true or false}. `value` is required: true shows (or locks), false hides (or unlocks).");
        }
        "scene.set_shape" => {
            out.set("mind", "Leave `mesh` out and Tequila gives the new shape a mesh of its own. To add a new shape rather than change one, use the add_shape operation.");
        }
        "material.set_parameter" => {
            out.set("mind", "The parameter keys and their ranges are in query api under material_parameters; a colour may be a hex sRGB string.");
        }
        _ => {}
    }
    Ok(out.build())
}

/// The parts of a project that are read whole: each small, and each what
/// one of the whole-block commands (`render.set_settings`,
/// `image.set_style`, `camera.set`, `studio.set`...) takes back changed.
/// With what they are, for the list.
pub const SECTIONS: &[(&str, &str)] = &[
    ("render", "the render settings: engine, size, samples, format, transparency, layers (render.set_settings)"),
    ("image_style", "the look after the render: exposure, contrast, white balance (image.set_style)"),
    ("color_management", "an OCIO display and view, or null for the built-in transforms (image.set_color_management)"),
    ("environment", "the environment, its light pins, the background and the floor (environment.set_parameter, environment.set_pins)"),
    ("lighting", "the lighting preset and its custom settings (lighting.set_preset, lighting.set_custom)"),
    ("cameras", "the saved cameras, each whole, and which is active (camera.set, camera.add, camera.select)"),
    ("units", "the unit lengths are shown in (scene.set_units)"),
    ("multi_materials", "colourways: parts with several material variants (material.set_multi_materials)"),
    ("model_sets", "which parts are shown together (scene.set_model_sets)"),
    ("studios", "Snapshots: saved set-ups of environment, camera and variants (studio.set, studio.apply)"),
    ("configurator", "the configurator's options, choices and rules (configurator.set)"),
    ("animation", "the turntable, when there is one (animation.set)"),
    ("product_turn", "how far a Snapshot or the turntable has turned the product"),
    ("material_templates", "rules that put materials on parts by name (material.set_templates)"),
    ("measurements", "the dimension lines (scene.set_measurements)"),
    ("textures", "every picture file the materials use, by its id"),
];

/// One part of the project, small enough to be read: a section of it as it
/// is saved, one entry of a section that is a list, or one node of the tree.
/// With nothing asked for, what there is and how long each is. The whole
/// project at once - `query context` - is tens of thousands of characters
/// on a small scene, which nobody reads; but most of what changes a
/// project's look takes its whole block back, so each block has to be
/// readable by itself.
pub fn read(document: &SceneDocument, what: Option<&str>, node: Option<u64>, index: Option<usize>) -> Result<Value, String> {
    let description = tequila_format::scene_description(document);
    let project = tequila_format::json::parse(description.as_bytes()).map_err(|_| "the project could not be read".to_string())?;
    if let Some(id) = node {
        let found = project.get("tree").map(Value::as_array).unwrap_or(&[]).iter().find(|entry| entry.get("id").and_then(Value::as_f64) == Some(id as f64));
        let Some(Value::Object(fields)) = found else {
            return Err(format!("there is no node {id}; the nodes query lists them"));
        };
        let mut fields = fields.clone();
        // A part's material by the number the commands take, beside the
        // name the file keeps it under.
        if let Some(key) = document.tree.get(tequila_scene::tree::NodeId(id)).and_then(|node| node.material) {
            fields.insert("material_id".into(), Value::Number(key.0 as f64));
        }
        fields.insert("note".into(), Value::String("`transform` is the node's 4 by 4 matrix, row by row, in metres, relative to its parent: the place is the 4th, 8th and 12th numbers. Send it back changed with scene.set_transform.".into()));
        return Ok(Value::Object(fields));
    }
    let Some(what) = what else {
        let mut sections = Object::new();
        for (name, about) in SECTIONS {
            let mut entry = Object::new();
            entry.set("about", *about);
            match project.get(name) {
                Some(Value::Array(list)) => entry.set("entries", list.len()),
                Some(Value::Null) | None => entry.set("there", false),
                Some(_) => entry.set("there", true),
            };
            sections.set(name, entry.build());
        }
        let mut out = Object::new();
        out.set("parts", sections.build());
        out.set("how", "Ask again with `what` for one of these, with `index` too for one entry of a list, or with `node` for one node of the scene (its transform, light or cutaway). Parts and materials are listed by the nodes and materials queries.");
        return Ok(out.build());
    };
    if what == "node" {
        return Err("give the node's id as `node`: {\"node\": 12}".into());
    }
    if matches!(what, "tree" | "nodes" | "materials") {
        return Err(format!("`{what}` is not read whole here: the nodes and materials queries list them, and `node` reads one node"));
    }
    let Some((name, _)) = SECTIONS.iter().find(|(name, _)| *name == what) else {
        return Err(format!("there is no part of the project called `{what}`; there are {}", SECTIONS.iter().map(|(name, _)| *name).collect::<Vec<_>>().join(", ")));
    };
    let mut out = Object::new();
    out.set("what", *name);
    match (project.get(name), index) {
        (None | Some(Value::Null), _) => {
            out.set("value", Value::Null).set("note", "The project has none yet.");
        }
        (Some(Value::Array(list)), Some(index)) => match list.get(index) {
            Some(entry) => {
                out.set("index", index).set("of", list.len()).set("value", entry.clone());
            }
            None => return Err(format!("`{name}` has {} entries, numbered from 0; there is no {index}", list.len())),
        },
        (Some(_), Some(_)) => return Err(format!("`{name}` is not a list, so it has no `index`")),
        (Some(value), None) => {
            out.set("value", value.clone());
        }
    }
    if *name == "cameras" {
        out.set("active_camera", project.get("active_camera").cloned().unwrap_or(Value::Null));
    }
    Ok(out.build())
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
    // Looking, which changes nothing: before anything is asked of the name.
    if request.action == "search" {
        let words = request.search.as_deref().or(request.name.as_deref());
        return Ok(LibraryOutcome::Done(library_search(&library(user), words, request.category.as_deref(), request.limit.unwrap_or(20))));
    }
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

/// How big the scene is: parts, nodes, materials, textures, its bounds, its
/// CAD layers, and where it is saved.
pub fn scene(document: &SceneDocument, path: Option<&std::path::Path>) -> Value {
    let parts = document.tree.iter().filter(|n| matches!(n.kind, NodeKind::Part { .. })).count();
    let mut out = Object::new();
    out.set("parts", parts as f64)
        .set("nodes", document.tree.iter().count() as f64)
        .set("materials", document.materials.iter().count() as f64)
        .set("textures", document.textures.len() as f64)
        .set("bounds", bounds_value(tequila_scene::snapshot::visible_bounds(document)))
        .set("layers", layers(document));
    if let Some(path) = path {
        out.set("path", path.display().to_string());
    }
    // Where to go from here, said where nearly every assistant starts: one
    // that has looked at the scene and not at the guide goes on to do the
    // job the way some other application would have it done.
    out.set(
        "guide",
        format!(
            "Before changing anything, read the guide's page for the job - the tequila_guide tool, or the guide operation, with its topic: {}.",
            crate::guide::PAGES.iter().map(|page| page.topic).collect::<Vec<_>>().join(", ")
        ),
    );
    out.build()
}

/// The CAD layers the scene's parts are on, in the order the tree meets
/// them: each one's name, its nodes, and whether they are shown - "all",
/// "none", or "some" where a part was shown or hidden by hand.
pub fn layers(document: &SceneDocument) -> Value {
    let list = tequila_scene::layers::layers(document)
        .into_iter()
        .map(|layer| {
            let mut entry = Object::new();
            entry
                .set("name", layer.name.clone())
                .set("nodes", Value::Array(layer.nodes.iter().map(|id| Value::Number(id.0 as f64)).collect()))
                .set("shown", layer.shown().label());
            entry.build()
        })
        .collect();
    Value::Array(list)
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

    fn names(answer: &Value) -> Vec<String> {
        answer.get("presets").map(Value::as_array).unwrap_or(&[]).iter().filter_map(|p| p.get("name").and_then(Value::as_str)).map(str::to_string).collect()
    }

    /// The library as an AI is shown it. The whole list is a third of a
    /// million characters, which an assistant asked for leather did not
    /// open, and made its own; a search answers in a page.
    #[test]
    fn the_library_is_searched_in_a_size_an_ai_reads() {
        let user = std::env::temp_dir().join(format!("tequila-scripting-search-{}", std::process::id()));
        let all = library(&user);
        let whole = tequila_format::json_write::to_string(&all).len();

        // Nothing asked: the categories, to choose from, and no presets.
        let categories = library_search(&all, None, None, 20);
        assert!(categories.get("categories").and_then(|c| c.get("Metal")).and_then(Value::as_f64).unwrap_or(0.0) > 20.0, "{categories:?}");
        assert!(categories.get("presets").is_none());

        // Words: every one has to be there, whatever their case, and a
        // preset with them in its name comes before one that only mentions
        // them.
        let leather = library_search(&all, Some("Black LEATHER"), None, 20);
        let found = names(&leather);
        assert!(!found.is_empty(), "no black leather in the library");
        assert!(found[0].to_lowercase().contains("leather"), "{found:?}");
        let page = tequila_format::json_write::to_string(&leather).len();
        assert!(page < 8_000 && page * 20 < whole, "a search answered {page} characters of {whole}");
        for preset in leather.get("presets").unwrap().as_array() {
            assert!(preset.get("category").is_some() && preset.get("type").is_some() && preset.get("about").is_some());
            assert!(preset.get("file").is_none() && preset.get("tags").is_none(), "more than is needed to pick one");
        }

        // No more than asked for, and it says there are more.
        let few = library_search(&all, Some("aluminium"), None, 3);
        assert_eq!(names(&few).len(), 3);
        assert!(few.get("found").and_then(Value::as_f64).unwrap() > 3.0);
        assert!(few.get("note").and_then(Value::as_str).unwrap().contains("more"));

        // One category.
        let wood = library_search(&all, None, Some("wood"), 500);
        let woods = wood.get("presets").unwrap().as_array();
        assert!(woods.len() > 5 && woods.iter().all(|p| p.get("category").and_then(Value::as_str) == Some("Wood")));
        assert!(wood.get("note").is_none());

        // Nothing: said, with what to try.
        let nothing = library_search(&all, Some("unobtainium zzz"), None, 20);
        assert_eq!(nothing.get("found").and_then(Value::as_f64), Some(0.0));
        assert!(nothing.get("note").and_then(Value::as_str).unwrap().contains("Nothing matches"));

        // As a request, where it needs neither a name nor a part.
        let document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.2);
        let mut search = request("search");
        search.search = Some("spun".into());
        let Ok(LibraryOutcome::Done(answer)) = library_request(&search, &document, &[], &user, &no_packs) else {
            panic!("a search did not answer");
        };
        assert!(names(&answer).iter().any(|name| name == "Aluminium Spun"), "{answer:?}");
        // The name it answers with is the one `apply` takes.
        let mut apply = request("apply");
        apply.name = Some(names(&answer)[0].clone());
        let part = document.tree.iter().find(|n| n.mesh().is_some()).map(|n| n.id).expect("a part");
        assert!(matches!(library_request(&apply, &document, &[part], &user, &no_packs), Ok(LibraryOutcome::Commands(..))));
    }

    /// A command looked up by its name: the fields, an example that is
    /// itself a command, and what the example cannot say. An assistant that
    /// guessed `visible` for a state's `value` hid the part it meant to
    /// show; one that looks finds the word.
    #[test]
    fn one_command_is_looked_up_by_its_name() {
        let list = help(None).unwrap();
        let commands = list.get("commands").unwrap().as_array();
        assert!(commands.len() > 50 && commands.iter().any(|c| c.as_str() == Some("scene.set_visibility")));

        let one = help(Some(" Scene.Set_Visibility ")).unwrap();
        assert_eq!(one.get("command").and_then(Value::as_str), Some("scene.set_visibility"));
        assert!(one.get("fields").unwrap().as_array().iter().any(|f| f.as_str() == Some("states")));
        assert!(one.get("mind").and_then(Value::as_str).unwrap().contains("`value` is required"));
        let example = tequila_format::json_write::to_string(one.get("example").unwrap());
        tequila_format::journal::decode(&example).unwrap_or_else(|e| panic!("{example}: {e:?}"));

        // A near miss is answered with the names it might have meant.
        let near = help(Some("scene.set_visible")).unwrap_err();
        assert!(near.contains("scene.set_visibility"), "{near}");
        assert!(help(Some("zz")).unwrap_err().contains("no command"));

        // The environment's settings by their names, which no example lists.
        let environment = help(Some("environment.set_parameter")).unwrap();
        assert!(environment.get("parameters").unwrap().as_array().iter().any(|p| p.as_str() == Some("ground_reflections")));

        // Every command has a page, short enough to read.
        for name in tequila_scene::Command::CATALOG {
            let page = help(Some(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(page.get("example").is_some(), "{name} has no example");
            let length = tequila_format::json_write::to_string(&page).len();
            assert!(length < 12_000, "{name}'s page is {length} characters");
        }
    }
}

#[cfg(test)]
mod bounds_tests {
    use super::*;

    /// A project is read a part at a time: what there is, one block as the
    /// command that sets it takes it, one camera of the list, one node with
    /// its place - and never the whole of it.
    #[test]
    fn a_project_is_read_a_part_at_a_time() {
        let document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.2);
        let parts = read(&document, None, None, None).unwrap();
        let listed = parts.get("parts").unwrap();
        for (name, _) in SECTIONS {
            assert!(listed.get(name).is_some(), "{name}");
        }
        assert_eq!(listed.get("render").and_then(|r| r.get("there")).and_then(Value::as_bool), Some(true));
        assert_eq!(listed.get("studios").and_then(|r| r.get("there")).and_then(Value::as_bool), Some(false), "none yet");
        assert!(tequila_format::json_write::to_string(&parts).len() < 4000, "the list itself is short");

        // The block is what the command takes back.
        let render = read(&document, Some("render"), None, None).unwrap();
        let mut command = Object::new();
        command.set("command", "render.set_settings").set("render", render.get("value").unwrap().clone());
        tequila_format::journal::decode(&tequila_format::json_write::to_string(&command.build())).expect("read, then sent back");

        let cameras = read(&document, Some("cameras"), None, None).unwrap();
        assert!(cameras.get("active_camera").is_some());
        let first = read(&document, Some("cameras"), None, Some(0)).unwrap();
        assert!(first.get("value").and_then(|camera| camera.get("focal_length_mm")).is_some());
        assert!(read(&document, Some("cameras"), None, Some(99)).unwrap_err().contains("numbered from 0"));
        assert!(read(&document, Some("render"), None, Some(0)).unwrap_err().contains("not a list"));
        assert_eq!(read(&document, Some("studios"), None, None).unwrap().get("value"), Some(&Value::Null));

        let part = document.tree.iter().find(|node| matches!(node.kind, NodeKind::Part { .. })).expect("the demo has a part");
        let node = read(&document, None, Some(part.id.0), None).unwrap();
        assert_eq!(node.get("transform").map(|t| t.as_array().len()), Some(16));
        assert_eq!(node.get("material_id").and_then(Value::as_f64), part.material.map(|key| key.0 as f64));
        assert!(read(&document, None, Some(9_999_999), None).unwrap_err().contains("no node"));

        assert!(read(&document, Some("tree"), None, None).unwrap_err().contains("nodes and materials queries"));
        assert!(read(&document, Some("teapots"), None, None).unwrap_err().contains("image_style"));
    }

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
        // And it says where to read before changing anything.
        let guide = scene(&demo, None).get("guide").and_then(|g| g.as_str().map(str::to_string)).expect("the scene points at the guide");
        assert!(guide.contains("tequila_guide") && crate::guide::PAGES.iter().all(|page| guide.contains(page.topic)), "{guide}");
        let size = scene(&demo, None).get("bounds").and_then(|b| b.get("size")).map(|s| s.as_array().len());
        assert_eq!(size, Some(3), "a scene with parts lost its bounds");
    }
}
