//! What the assistant may do, and how each tool becomes a scripting request.
//!
//! The same surface the MCP bridge offers, less the parts that reach outside
//! the scene: no importing arbitrary files, no saving to arbitrary paths, no
//! quitting, and no raw requests. The assistant works on the open scene; the
//! person decides what comes into it and where it goes.

use tequila_format::json::{self, Value};
use tequila_format::json_write::{self, Object};

#[derive(Clone, Debug)]
pub struct Tool {
    pub name: String,
    pub description: String,
    /// JSON Schema for the arguments.
    pub schema: Value,
}

const TOOLS: &str = r##"[
 {"name":"tequila_scene","description":"Counts, units, overall bounds and the file of the open scene. A good first call.",
  "schema":{"type":"object","properties":{}}},
 {"name":"tequila_nodes","description":"Parts and groups: id, name, material and world size and centre. Filter by a word in the name or material name to keep the answer short.",
  "schema":{"type":"object","properties":{"filter":{"type":"string"},"parts_only":{"type":"boolean"}}}},
 {"name":"tequila_materials","description":"Every material: id, name, type, how many parts use it, and its texture maps. Use tequila_material for one material's parameters.",
  "schema":{"type":"object","properties":{}}},
 {"name":"tequila_material","description":"One material in full: every parameter value and texture.",
  "schema":{"type":"object","properties":{"id":{"type":"integer"}},"required":["id"]}},
 {"name":"tequila_api","description":"The command catalog, every material parameter (key, kind, range) and every material type. Call once before editing materials, to use exact names.",
  "schema":{"type":"object","properties":{}}},
 {"name":"tequila_query","description":"A read-only query: selection, camera, commands, ui, jobs or history.",
  "schema":{"type":"object","properties":{"what":{"type":"string","enum":["selection","camera","commands","ui","jobs","history"]}},"required":["what"]}},
 {"name":"tequila_apply","description":"Apply one command, e.g. {\"command\":\"material.set_type\",\"material\":2,\"type\":\"velvet\"} or {\"command\":\"material.set_parameter\",\"material\":2,\"parameter\":\"base_color\",\"value\":[0.1,0.1,0.12]}. Materials can be shared: use material.unlink on a part before changing its material alone. Colours: send a hex sRGB string (\"value\":\"#c0392b\") and it is converted for you; an array is linear RGB 0-1 in the rendering space, ACEScg, used as given.",
  "schema":{"type":"object","properties":{"command":{"type":"object","description":"A command: {\"command\":\"<name>\", ...fields}"}},"required":["command"]}},
 {"name":"tequila_batch","description":"Several operations as ONE undo step; if one fails, none of them stay. Each is an object with an \"op\": apply (with \"command\"), select (\"nodes\"), frame (\"all\"), look, ui. Prefer this for more than one edit.",
  "schema":{"type":"object","properties":{"ops":{"type":"array","items":{"type":"object"}}},"required":["ops"]}},
 {"name":"tequila_undo","description":"Take back the last step: one edit or one whole batch.",
  "schema":{"type":"object","properties":{}}},
 {"name":"tequila_select","description":"Select these node ids (replaces the selection).",
  "schema":{"type":"object","properties":{"nodes":{"type":"array","items":{"type":"integer"}}},"required":["nodes"]}},
 {"name":"tequila_frame","description":"Point the camera at everything (all=true, the default) or at the selection.",
  "schema":{"type":"object","properties":{"all":{"type":"boolean"}}}},
 {"name":"tequila_look","description":"Place the camera: a standard view; or yaw/pitch in degrees and distance around a target 'at'; or an exact eye 'from' looking at 'at'. Optional focal_length in mm. Undoable.",
  "schema":{"type":"object","properties":{"view":{"type":"string","enum":["front","back","left","right","top","bottom","iso"]},"at":{"type":"array","items":{"type":"number"}},"from":{"type":"array","items":{"type":"number"}},"yaw":{"type":"number"},"pitch":{"type":"number"},"distance":{"type":"number"},"focal_length":{"type":"number"}}}},
 {"name":"tequila_screenshot","description":"Look at the scene as it is now. Returns an image. viewport=true (default) shows only the 3D view. Switch the viewport to rendered first (tequila_ui) to judge materials and lighting.",
  "schema":{"type":"object","properties":{"viewport":{"type":"boolean"}}}},
 {"name":"tequila_ui","description":"Switch the viewport between 'rendered' (ray traced, shows real materials and light) and 'solid' (fast preview), and open panels.",
  "schema":{"type":"object","properties":{"viewport":{"type":"string","enum":["rendered","solid"]},"render_window":{"type":"boolean"},"tab":{"type":"string","enum":["scene","material","environment","lighting","camera","image"]}}}},
 {"name":"tequila_queue","description":"Start a final render in the background with the document's render settings. Only when asked for a render.",
  "schema":{"type":"object","properties":{"name":{"type":"string"},"width":{"type":"integer"},"height":{"type":"integer"},"samples":{"type":"integer"},"denoise":{"type":"boolean"}}}},
 {"name":"tequila_save","description":"Save the project to its own file. Only when asked to save.",
  "schema":{"type":"object","properties":{}}}
]"##;

/// The MCP bridge's tools, whose material ones the assistant offers as they
/// are: one description of how to make a material, wherever the AI is.
const MCP_TOOLS: &str = include_str!("../../tequila-cli/src/mcp_tools.json");

/// Every tool, with its schema.
pub fn all() -> Vec<Tool> {
    let parsed = json::parse(TOOLS.as_bytes()).expect("the tool list is valid JSON");
    let bridge = json::parse(MCP_TOOLS.as_bytes()).expect("the bridge's tool list is valid JSON");
    // The bridge's material and plugin tools, as they are: one description
    // of each for every AI.
    let shared = |name: &str| ["tequila_material_", "tequila_plugin_", "tequila_studio_"].iter().any(|p| name.starts_with(p));
    let materials = bridge.as_array().iter().filter(|t| t.get("name").and_then(|n| n.as_str()).is_some_and(shared)).map(|tool| {
        let mut entry = tool.clone();
        if let Value::Object(fields) = &mut entry {
            if let Some(schema) = fields.remove("inputSchema") {
                fields.insert("schema".into(), schema);
            }
        }
        entry
    });
    parsed
        .as_array()
        .iter()
        .cloned()
        .chain(materials)
        .map(|tool| {
            let mut schema = tool.get("schema").cloned().expect("every tool has a schema");
            // Every schema is an object that allows nothing unlisted, and
            // says so explicitly, which some OpenAI-compatible servers need.
            if let Value::Object(fields) = &mut schema {
                fields.entry("required".into()).or_insert_with(|| Value::Array(Vec::new()));
            }
            Tool {
                name: tool.get("name").and_then(|v| v.as_str()).unwrap().to_string(),
                description: tool.get("description").and_then(|v| v.as_str()).unwrap().to_string(),
                schema,
            }
        })
        .collect()
}

/// How the answer to a request should be shaped for the model.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// As the application answered.
    Raw,
    /// A node list, narrowed and slimmed. See [`slim_nodes`].
    Nodes { filter: Option<String>, parts_only: bool },
    /// The material list without every parameter of every material.
    Materials,
    /// One material out of the list.
    Material(u64),
}

/// What carrying out a tool call means.
#[derive(Clone, Debug, PartialEq)]
pub enum Plan {
    /// Send this scripting request, and shape its answer.
    Request { json: String, shape: Shape },
    /// Take a screenshot: deferred, because the window has to be drawn first.
    Screenshot { viewport: bool },
    /// A material request (`{"op":"material",...}`), answered with the
    /// material on the preview ball when `picture`: a preview always, a
    /// create or update unless asked not to.
    Material { json: String, picture: bool },
    /// Run a plugin's command (`json`), then wait - frames, not the window -
    /// for what it logs: `finished` and the command, or a traceback.
    PluginRun { json: String, plugin: String, command: String },
}

fn request(pairs: Vec<(&str, Value)>) -> String {
    let mut out = Object::new();
    for (key, value) in pairs {
        out.set(key, value);
    }
    json_write::to_string(&out.build())
}

fn text(value: &str) -> Value {
    Value::String(value.into())
}

/// The operations a batch from the assistant may contain.
const BATCH_OPS: [&str; 5] = ["apply", "select", "frame", "look", "ui"];

/// Only the listed keys, so a tool cannot be used to pass anything else.
fn pick(arguments: &Value, keys: &[&str]) -> Vec<(String, Value)> {
    keys.iter()
        .filter_map(|key| arguments.get(key).map(|v| (key.to_string(), v.clone())))
        .collect()
}

fn with(op: &str, fields: Vec<(String, Value)>) -> String {
    let mut out = Object::new();
    out.set("op", op);
    for (key, value) in fields {
        out.set(&key, value);
    }
    json_write::to_string(&out.build())
}

pub fn plan(name: &str, arguments: &Value) -> Result<Plan, String> {
    let raw = |json: String| Ok(Plan::Request { json, shape: Shape::Raw });
    let query = |what: &str| request(vec![("op", text("query")), ("what", text(what))]);
    match name {
        "tequila_scene" => raw(query("scene")),
        "tequila_nodes" => Ok(Plan::Request {
            json: query("nodes"),
            shape: Shape::Nodes {
                filter: arguments.get("filter").and_then(|v| v.as_str()).map(str::to_string).filter(|s| !s.trim().is_empty()),
                parts_only: arguments.get("parts_only").and_then(|v| v.as_bool()).unwrap_or(false),
            },
        }),
        "tequila_materials" => Ok(Plan::Request { json: query("materials"), shape: Shape::Materials }),
        "tequila_material" => {
            let id = arguments.get("id").and_then(|v| v.as_f64()).ok_or("give the material's id")?;
            Ok(Plan::Request { json: query("materials"), shape: Shape::Material(id as u64) })
        }
        "tequila_api" => raw(query("api")),
        "tequila_query" => {
            let what = arguments.get("what").and_then(|v| v.as_str()).ok_or("say what to query")?;
            if !["selection", "camera", "commands", "ui", "jobs", "history"].contains(&what) {
                return Err(format!("`{what}` is not one of the queries this tool offers"));
            }
            raw(query(what))
        }
        "tequila_apply" => {
            let command = arguments.get("command").ok_or("give a command")?;
            check_scene_command(command)?;
            if !matches!(command, Value::Object(_)) {
                return Err("the command must be an object: {\"command\":\"<name>\", ...}".into());
            }
            raw(request(vec![("op", text("apply")), ("command", command.clone())]))
        }
        "tequila_batch" => {
            let Some(Value::Array(ops)) = arguments.get("ops") else {
                return Err("give ops: an array of operations".into());
            };
            for (i, op) in ops.iter().enumerate() {
                if let Some(command) = op.get("command") { check_scene_command(command)?; }
                let kind = op.get("op").and_then(|v| v.as_str()).unwrap_or("(none)");
                if !BATCH_OPS.contains(&kind) {
                    return Err(format!("operation {} is `{kind}`; a batch here may hold {}", i + 1, BATCH_OPS.join(", ")));
                }
            }
            raw(request(vec![("op", text("batch")), ("ops", Value::Array(ops.clone()))]))
        }
        "tequila_undo" => raw(request(vec![("op", text("undo"))])),
        "tequila_select" => raw(with("select", pick(arguments, &["nodes"]))),
        "tequila_frame" => raw(with("frame", pick(arguments, &["all"]))),
        "tequila_look" => raw(with("look", pick(arguments, &["view", "at", "from", "yaw", "pitch", "distance", "focal_length"]))),
        "tequila_screenshot" => Ok(Plan::Screenshot {
            viewport: arguments.get("viewport").and_then(|v| v.as_bool()).unwrap_or(true),
        }),
        "tequila_ui" => raw(with("ui", pick(arguments, &["viewport", "render_window", "tab"]))),
        "tequila_queue" => raw(with("queue", pick(arguments, &["name", "width", "height", "samples", "denoise"]))),
        "tequila_save" => raw(request(vec![("op", text("save"))])),
        "tequila_material_guide" => raw(with("material", vec![("action".into(), text("guide"))])),
        "tequila_material_recipe" => raw(with("material", [("action".to_string(), text("recipe"))].into_iter().chain(pick(arguments, &["material"])).collect())),
        "tequila_material_save" => raw(with("material", [("action".to_string(), text("save"))].into_iter().chain(pick(arguments, &["material", "name", "category", "tags"])).collect())),
        "tequila_material_preview" => Ok(Plan::Material {
            json: with("material", [("action".to_string(), text("preview"))].into_iter().chain(pick(arguments, &["recipe", "material", "radius", "shape"])).collect()),
            picture: true,
        }),
        "tequila_material_create" | "tequila_material_update" => {
            let action = if name == "tequila_material_create" { "create" } else { "update" };
            Ok(Plan::Material {
                json: with("material", [("action".to_string(), text(action))].into_iter().chain(pick(arguments, &["recipe", "nodes", "material"])).collect()),
                picture: arguments.get("preview").and_then(|v| v.as_bool()).unwrap_or(true),
            })
        }
        "tequila_studio_presets" => raw(with("studio_preset", vec![("action".into(), text("list"))])),
        "tequila_studio_preset_apply" => {
            arguments.get("preset").and_then(|v| v.as_str()).ok_or("say which studio preset: its id, as preset")?;
            raw(with(
                "studio_preset",
                [("action".to_string(), text("apply"))]
                    .into_iter()
                    .chain(pick(arguments, &["preset", "front", "scale", "keep_lights", "keep_cameras", "keep_placement", "dress"]))
                    .collect(),
            ))
        }
        "tequila_studio_preset_save" => {
            arguments.get("name").and_then(|v| v.as_str()).ok_or("give the preset a name")?;
            raw(with(
                "studio_preset",
                [("action".to_string(), text("save"))].into_iter().chain(pick(arguments, &["name", "family", "size", "description", "materials"])).collect(),
            ))
        }
        "tequila_studio_render_all" => raw(with("studio_preset", vec![("action".into(), text("render_all"))])),
        "tequila_plugin_guide" => raw(with("plugin", vec![("action".into(), text("guide"))])),
        "tequila_plugin_list" => raw(with("plugin", vec![("action".into(), text("list"))])),
        "tequila_plugin_write" | "tequila_plugin_enable" | "tequila_plugin_log" => {
            let action = name.trim_start_matches("tequila_plugin_");
            let id = arguments.get("id").cloned().ok_or("say which plugin: its id")?;
            let mut fields = vec![("action".to_string(), text(action)), ("plugin".to_string(), id)];
            fields.extend(pick(arguments, &["files", "lines"]));
            // Code this assistant changes runs only once the person says so.
            if action == "write" {
                fields.push(("ask".into(), Value::Bool(true)));
            }
            raw(with("plugin", fields))
        }
        "tequila_plugin_run" => {
            let plugin = arguments.get("id").and_then(|v| v.as_str()).ok_or("say which plugin: its id")?.to_string();
            let command = arguments.get("command").and_then(|v| v.as_str()).ok_or("say which command")?.to_string();
            let mut fields = vec![("action".to_string(), text("run")), ("plugin".to_string(), text(&plugin)), ("command".to_string(), text(&command))];
            fields.extend(pick(arguments, &["values"]));
            Ok(Plan::PluginRun { json: with("plugin", fields), plugin, command })
        }
        other => Err(format!("there is no tool called `{other}`")),
    }
}

/// Keep local output destinations outside the assistant's editing surface.
fn check_scene_command(command: &Value) -> Result<(), String> {
    if command.get("command").and_then(|v| v.as_str()) == Some("render.set_settings") {
        return Err("Render output settings are controlled locally; use tequila_queue for resolution, samples and denoising".into());
    }
    Ok(())
}

/// An application answer, as text for the model.
pub fn describe(shape: &Shape, answer: &Value) -> String {
    let shaped = match shape {
        Shape::Raw => answer.clone(),
        Shape::Nodes { filter, parts_only } => slim_nodes(answer, filter.as_deref(), *parts_only),
        Shape::Materials => slim_materials(answer),
        Shape::Material(id) => answer
            .get("materials")
            .and_then(|list| list.as_array().iter().find(|m| m.get("id").and_then(|i| i.as_f64()) == Some(*id as f64)).cloned())
            .unwrap_or_else(|| Value::String(format!("there is no material {id}"))),
    };
    compact(&shaped)
}

/// JSON on as few lines as it will go: the model reads it, not a person.
fn compact(value: &Value) -> String {
    json_write::to_string(value).split('\n').map(str::trim).collect::<Vec<_>>().join(" ")
}

fn round(v: f64) -> Value {
    Value::Number((v * 10_000.0).round() / 10_000.0)
}

/// A node list without the corners of every bounding box — size and centre
/// say where a part is in half the words — and narrowed if asked.
pub fn slim_nodes(answer: &Value, filter: Option<&str>, parts_only: bool) -> Value {
    let needle = filter.map(str::to_lowercase);
    let nodes: Vec<Value> = answer
        .get("nodes")
        .map(|n| n.as_array())
        .unwrap_or(&[])
        .iter()
        .filter(|n| !parts_only || n.get("kind").and_then(|k| k.as_str()) == Some("part"))
        .filter(|n| match &needle {
            None => true,
            Some(needle) => ["name", "material_name"].iter().any(|key| {
                n.get(key).and_then(|v| v.as_str()).is_some_and(|s| s.to_lowercase().contains(needle.as_str()))
            }),
        })
        .map(|n| {
            let mut out = Object::new();
            for key in ["id", "name", "kind", "material", "material_name", "visible"] {
                if let Some(value) = n.get(key) {
                    out.set(key, value.clone());
                }
            }
            if let Some(bounds) = n.get("bounds") {
                let corner = |key| bounds.get(key).map(|v| v.as_array().iter().filter_map(|x| x.as_f64()).collect::<Vec<_>>()).unwrap_or_default();
                let (min, max) = (corner("min"), corner("max"));
                if min.len() == 3 && max.len() == 3 {
                    out.set("size", Value::Array((0..3).map(|i| round(max[i] - min[i])).collect()));
                    out.set("center", Value::Array((0..3).map(|i| round((max[i] + min[i]) / 2.0)).collect()));
                }
            }
            out.build()
        })
        .collect();
    let mut out = Object::new();
    out.set("count", nodes.len() as f64).set("nodes", Value::Array(nodes));
    out.build()
}

fn slim_materials(answer: &Value) -> Value {
    let list: Vec<Value> = answer
        .get("materials")
        .map(|m| m.as_array())
        .unwrap_or(&[])
        .iter()
        .map(|m| {
            let mut out = Object::new();
            for key in ["id", "name", "type", "used_by"] {
                if let Some(value) = m.get(key) {
                    out.set(key, value.clone());
                }
            }
            let maps: Vec<Value> = m
                .get("textures")
                .map(|t| t.as_array())
                .unwrap_or(&[])
                .iter()
                .filter_map(|t| t.get("role").cloned())
                .collect();
            out.set("maps", Value::Array(maps));
            out.build()
        })
        .collect();
    let mut out = Object::new();
    out.set("materials", Value::Array(list));
    out.build()
}

/// What the assistant is told about the application and how to work in it.
pub const SYSTEM: &str = "You are the assistant built into Tequila, a product-rendering application, and you work on the scene that is open in it right now, using your tools. \
The person asks in plain language for scenes, materials, textures, lighting and camera work; you carry it out.

How to work:
- Look before you change: tequila_scene, then tequila_nodes (filter by a word) to find parts by name, material or size. Never guess ids.
- Call tequila_api once before editing materials, and use its exact parameter keys and material type ids. Its command_examples has an example of every command: copy one, then change the ids, keys and values (the ones in it are placeholders).
- For patterns (noise, granite, leather, weave, perforated mesh and the rest), copy an entry from tequila_api's procedurals list into material.set_procedural with a param and a seed, and change its fields. On a colour parameter a ramp is added between them.
- Materials can be shared by many parts. To change one part without the others, apply material.unlink to that part first, then read its new material id from tequila_nodes.
- Put several edits in one tequila_batch: it is one undo step for the person, and if anything fails none of it stays.
- Check your work: switch the viewport to rendered (tequila_ui viewport=rendered) and take a tequila_screenshot. Judge the picture, and fix what is wrong.
- Units are metres; +Y is up. Give colours as hex sRGB strings (\"#c0392b\"): Tequila converts them into ACEScg, the rendering space. An array is taken as linear ACEScg as it is. Sizes are real: glass reaches its colour over its transmission depth in metres and pattern frequencies are per metre, so check a part's size in tequila_nodes before tuning either.
- A liquid in a glass container is modelled a fraction of a millimetre into the glass wall; the Liquid type's higher priority makes it the medium there.
- For a material of the person's own (\"make it look like terrazzo\"), read tequila_material_guide once, try recipes with tequila_material_preview, then tequila_material_create on the parts and refine with tequila_material_update after looking at it. Prefer a library preset when one fits.
- To add to Tequila itself - a panel, a button, something done on every import - write a plugin: tequila_plugin_guide once, then tequila_plugin_write. The person turns it on in Tequila (tequila_plugin_enable asks them, and so does every change you make to one that is on); then tequila_plugin_run tries it and shows its log. Only when asked for one.
- For a whole studio at once - lights, cameras and views, floor and look for a kind of product - list tequila_studio_presets and apply the suggested size with tequila_studio_preset_apply (one undo step). tequila_studio_render_all renders every view, only when asked for renders.
- Only render (tequila_queue) or save (tequila_save) when asked to.
- Everything you do is undoable and shown to the person as you do it. When you finish, say briefly what you changed.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_paths_cannot_be_redirected_through_apply_or_batch() {
        let command = r#"{"command":"render.set_settings","render":{"output_directory":"C:/outside","filename_template":"redirected"}}"#;
        for (tool, input) in [
            ("tequila_apply", format!(r#"{{"command":{command}}}"#)),
            ("tequila_batch", format!(r#"{{"ops":[{{"op":"apply","command":{command}}}]}}"#)),
        ] {
            assert!(plan(tool, &args(&input)).unwrap_err().contains("controlled locally"));
        }
    }

    fn args(text: &str) -> Value {
        json::parse(text.as_bytes()).unwrap()
    }

    #[test]
    fn every_tool_has_a_plan() {
        for tool in all() {
            let arguments = match tool.name.as_str() {
                "tequila_material" => args(r#"{"id":2}"#),
                "tequila_query" => args(r#"{"what":"camera"}"#),
                "tequila_apply" => args(r#"{"command":{"command":"scene.rename","node":1,"name":"x"}}"#),
                "tequila_batch" => args(r#"{"ops":[{"op":"select","nodes":[1]}]}"#),
                "tequila_select" => args(r#"{"nodes":[1]}"#),
                "tequila_material_create" => args(r#"{"recipe":{"type":"plastic"}}"#),
                "tequila_material_update" => args(r#"{"material":2,"recipe":{"type":"plastic"}}"#),
                "tequila_material_recipe" | "tequila_material_save" => args(r#"{"material":2}"#),
                "tequila_plugin_write" => args(r#"{"id":"my.x","files":{"main.py":""}}"#),
                "tequila_plugin_enable" | "tequila_plugin_log" => args(r#"{"id":"my.x"}"#),
                "tequila_plugin_run" => args(r#"{"id":"my.x","command":"go"}"#),
                "tequila_studio_preset_apply" => args(r#"{"preset":"studio"}"#),
                "tequila_studio_preset_save" => args(r#"{"name":"Mine"}"#),
                _ => args("{}"),
            };
            plan(&tool.name, &arguments).unwrap_or_else(|e| panic!("{}: {e}", tool.name));
            assert_eq!(tool.schema.get("type").and_then(|t| t.as_str()), Some("object"), "{}", tool.name);
        }
    }

    /// The assistant's plugin requests read as the application reads them,
    /// name the plugin as `plugin`, and a write always asks the person
    /// again: code the assistant changes runs only once they say so.
    #[test]
    fn the_assistants_plugin_requests_ask_the_person() {
        let Ok(Plan::Request { json, .. }) = plan("tequila_plugin_write", &args(r#"{"id":"my.x","files":{"main.py":"print(1)"}}"#)) else { panic!("not a request") };
        let write = args(&json);
        assert_eq!(write.get("op").and_then(Value::as_str), Some("plugin"));
        assert_eq!(write.get("plugin").and_then(Value::as_str), Some("my.x"));
        assert!(write.get("id").is_none(), "`id` is the request's own number");
        assert_eq!(write.get("ask").and_then(Value::as_bool), Some(true), "a write by the assistant must ask the person again");
        let Ok(Plan::PluginRun { json, plugin, command }) = plan("tequila_plugin_run", &args(r#"{"id":"my.x","command":"go","values":{"n":1}}"#)) else { panic!("not a run") };
        assert_eq!((plugin.as_str(), command.as_str()), ("my.x", "go"));
        assert_eq!(args(&json).get("values").and_then(|v| v.get("n")).and_then(Value::as_f64), Some(1.0));
    }

    /// The assistant cannot reach outside the scene through a batch.
    #[test]
    fn a_batch_cannot_import_save_or_quit() {
        for op in ["import", "save", "quit", "render", "screenshot"] {
            let error = plan("tequila_batch", &args(&format!(r#"{{"ops":[{{"op":"{op}","path":"C:/x"}}]}}"#))).unwrap_err();
            assert!(error.contains(op), "{op}: {error}");
        }
    }

    /// Unlisted arguments are dropped, not forwarded: queue cannot be given a
    /// folder to write to.
    #[test]
    fn only_listed_arguments_pass_through() {
        match plan("tequila_queue", &args(r#"{"samples":64,"folder":"C:/Windows"}"#)).unwrap() {
            Plan::Request { json, .. } => {
                assert!(json.contains("samples"));
                assert!(!json.contains("folder"), "{json}");
            }
            other => panic!("{other:?}"),
        }
        match plan("tequila_save", &args(r#"{"path":"C:/elsewhere.tequila"}"#)).unwrap() {
            Plan::Request { json, .. } => assert!(!json.contains("elsewhere"), "{json}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn nodes_are_narrowed_and_slimmed() {
        let answer = args(r#"{"nodes":[
            {"id":1,"name":"Quest","kind":"group"},
            {"id":360,"name":"part 12","kind":"part","material":2,"material_name":"Strap Felt","bounds":{"min":[0,0,0],"max":[0.1,0.2,0.3],"size":[0.1,0.2,0.3]}}]}"#);
        let text = describe(&Shape::Nodes { filter: Some("strap".into()), parts_only: false }, &answer);
        let value = json::parse(text.as_bytes()).unwrap();
        assert_eq!(value.get("count").and_then(|c| c.as_f64()), Some(1.0));
        let node = &value.get("nodes").unwrap().as_array()[0];
        assert!(node.get("bounds").is_none());
        assert_eq!(node.get("center").unwrap().as_array()[2].as_f64(), Some(0.15));
    }

    #[test]
    fn one_material_is_picked_out() {
        let answer = args(r#"{"materials":[{"id":1,"name":"A"},{"id":2,"name":"B","definition":{"base_color":[1,0,0]}}]}"#);
        let text = describe(&Shape::Material(2), &answer);
        assert!(text.contains("\"B\"") && text.contains("base_color") && !text.contains("\"A\""), "{text}");
        assert!(describe(&Shape::Material(9), &answer).contains("no material 9"));
    }
}
