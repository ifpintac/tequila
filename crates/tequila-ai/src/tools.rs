//! What the assistant may do, and how each tool becomes a scripting request.
//!
//! The same surface the MCP bridge offers, less quitting, opening another
//! project and raw requests. The assistant works on the open scene. What
//! leaves it - a file brought in, the project saved somewhere new, a render
//! written to a file, a preference, the library - is a tool like the rest,
//! and the application asks the person before it happens
//! (`tequila_scripting::approval`): the person decides what comes into the
//! scene and where it goes.

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
 {"name":"tequila_guide","description":"How to do a job in Tequila: one page of the guide, short enough to read whole, with tested examples. READ THE PAGE for an area before working in it for the first time. Topics: materials, recipes (a material of your own), graph (the material graph and its nodes), textures (picture maps and packs), labels (logos and decals), uv, scene (parts, groups, moving, shapes, units, cutaways), lighting (environment, lights, floor), camera, variants (colourways, Snapshots, configurator, studio presets), rendering, image (the look after the render), animation (turntable, spin), fixing (a model that came in wrong), files (import, update, save, export), plugins, app (panels and preferences), scripting, shots (whole shots step by step). With no topic, the list of pages.",
  "schema":{"type":"object","properties":{"topic":{"type":"string","enum":["start","materials","recipes","graph","textures","labels","uv","scene","lighting","camera","variants","rendering","image","animation","fixing","files","plugins","app","scripting","shots"]}}}},
 {"name":"tequila_read","description":"Read one part of the project, small enough to read: what is render (the render settings), image_style, color_management, environment (with its light pins and the floor), lighting, cameras, units, multi_materials (colourways), model_sets, studios (Snapshots), configurator, animation, product_turn, material_templates, measurements or textures - each as the command that sets it takes it back. index takes one entry of a list. node reads one node of the scene by its id: its transform, and a light's or a cutaway's settings. With nothing, what there is to read. Use it before any command that takes a whole block (image.set_style, camera.set, studio.set, scene.set_light): read, change what was asked, send all of it back.",
  "schema":{"type":"object","properties":{"what":{"type":"string","enum":["render","image_style","color_management","environment","lighting","cameras","units","multi_materials","model_sets","studios","configurator","animation","product_turn","material_templates","measurements","textures"]},"index":{"type":"integer"},"node":{"type":"integer"}}}},
 {"name":"tequila_scene","description":"Counts, units, overall bounds and the file of the open scene. A good first call.",
  "schema":{"type":"object","properties":{}}},
 {"name":"tequila_nodes","description":"Parts and groups: id, name, material and world size and centre. Filter by a word in the name or material name to keep the answer short.",
  "schema":{"type":"object","properties":{"filter":{"type":"string"},"parts_only":{"type":"boolean"}}}},
 {"name":"tequila_materials","description":"Every material: id, name, type, how many parts use it, and its texture maps. Use tequila_material for one material's parameters.",
  "schema":{"type":"object","properties":{}}},
 {"name":"tequila_material","description":"One material in full: every parameter value and texture.",
  "schema":{"type":"object","properties":{"id":{"type":"integer"}},"required":["id"]}},
 {"name":"tequila_api","description":"The command catalog, every material parameter (key, kind, range), every material type and every pattern, all at once: tens of thousands of characters. The materials page of tequila_guide has the parameters that matter and tequila_command one command; ask this only for a parameter or a pattern neither names.",
  "schema":{"type":"object","properties":{}}},
 {"name":"tequila_query","description":"A read-only query: selection, camera, commands, ui, jobs or history.",
  "schema":{"type":"object","properties":{"what":{"type":"string","enum":["selection","camera","commands","ui","jobs","history"]}},"required":["what"]}},
 {"name":"tequila_library","description":"Search the material library: several hundred finished presets. LOOK HERE FIRST for any material, before making one. With no arguments it lists the categories. search takes words that must all match the name, category, type, tags or description (\"black leather\", \"brushed aluminium\"); category narrows to one. Answers with name, category, type and a line about each: the name is what tequila_library_apply takes.",
  "schema":{"type":"object","properties":{"search":{"type":"string"},"category":{"type":"string"},"limit":{"type":"integer"}}}},
 {"name":"tequila_library_apply","description":"Put a preset from the material library on parts, by its exact name from tequila_library. unlink=true gives the parts a copy of their own. Undoable.",
  "schema":{"type":"object","properties":{"name":{"type":"string"},"nodes":{"type":"array","items":{"type":"integer"}},"unlink":{"type":"boolean"}},"required":["name","nodes"]}},
 {"name":"tequila_command","description":"How to write one command for tequila_apply: its exact fields and a working example. With no command, the name of every command. Ask before using a command for the first time, rather than guessing its fields: a field that is not in the example is not read.",
  "schema":{"type":"object","properties":{"command":{"type":"string"}}}},
 {"name":"tequila_add_shape","description":"Add a simple shape as a new part: plane, cube, cylinder, sphere, disc, backdrop or wall, beside what is in the scene and on the floor. Not for the floor or the background, which the scene already has: those are settings of the environment (tequila_guide lighting). Sizes are metres: width (a cylinder's, sphere's and disc's diameter), depth, height, radius (half a round shape's width; a backdrop's curve). Undoable. Answers with the new part's id.",
  "schema":{"type":"object","properties":{"shape":{"type":"string","enum":["plane","cube","cylinder","sphere","disc","backdrop","wall"]},"width":{"type":"number"},"depth":{"type":"number"},"height":{"type":"number"},"radius":{"type":"number"}},"required":["shape"]}},
 {"name":"tequila_apply","description":"Apply one command, as the guide's page for the job shows it (tequila_guide first), e.g. {\"command\":\"material.set_type\",\"material\":2,\"type\":\"velvet\"} or {\"command\":\"material.set_parameter\",\"material\":2,\"parameter\":\"base_color\",\"value\":[0.1,0.1,0.12]}. Materials can be shared: use material.unlink on a part before changing its material alone. Colours: send a hex sRGB string (\"value\":\"#c0392b\") and it is converted for you; an array is linear RGB 0-1 in the rendering space, ACEScg, used as given.",
  "schema":{"type":"object","properties":{"command":{"type":"object","description":"A command: {\"command\":\"<name>\", ...fields}"}},"required":["command"]}},
 {"name":"tequila_batch","description":"Several operations as ONE undo step; if one fails, none of them stay. Each is an object with an \"op\": apply (with \"command\"), select (\"nodes\"), frame (\"all\"), look, ui. Prefer this for more than one edit.",
  "schema":{"type":"object","properties":{"ops":{"type":"array","items":{"type":"object"}}},"required":["ops"]}},
 {"name":"tequila_undo","description":"Take back the last step: one edit or one whole batch.",
  "schema":{"type":"object","properties":{}}},
 {"name":"tequila_redo","description":"Put back the step tequila_undo last took away.",
  "schema":{"type":"object","properties":{}}},
 {"name":"tequila_select","description":"Select these node ids (replaces the selection).",
  "schema":{"type":"object","properties":{"nodes":{"type":"array","items":{"type":"integer"}}},"required":["nodes"]}},
 {"name":"tequila_frame","description":"Point the camera at everything (all=true, the default) or at the selection.",
  "schema":{"type":"object","properties":{"all":{"type":"boolean"}}}},
 {"name":"tequila_look","description":"Place the camera: a standard view; or yaw/pitch in degrees and distance around a target 'at'; or an exact eye 'from' looking at 'at'. Optional focal_length in mm. Undoable.",
  "schema":{"type":"object","properties":{"view":{"type":"string","enum":["front","back","left","right","top","bottom","iso"]},"at":{"type":"array","items":{"type":"number"}},"from":{"type":"array","items":{"type":"number"}},"yaw":{"type":"number"},"pitch":{"type":"number"},"distance":{"type":"number"},"focal_length":{"type":"number"}}}},
 {"name":"tequila_screenshot","description":"Look at the scene as it is now. Returns an image. viewport=true (default) shows only the 3D view. Switch the viewport to rendered first (tequila_ui) to judge materials and lighting.",
  "schema":{"type":"object","properties":{"viewport":{"type":"boolean"}}}},
 {"name":"tequila_ui","description":"Switch the viewport between 'rendered' (ray traced, shows real materials and light), 'material' and 'solid' (fast previews) and 'wireframe', and show the person a panel or a window: an Inspector tab, the Render window, the library, the Material Graph on a material (graph: its id), the UV editor on the selected part, the composition guides, the render's frame in the viewport.",
  "schema":{"type":"object","properties":{"viewport":{"type":"string","enum":["rendered","material","solid","wireframe"]},"render_window":{"type":"boolean"},"tab":{"type":"string","enum":["scene","material","environment","lighting","camera","image"]},"inspector":{"type":"boolean"},"library":{"type":"boolean"},"library_section":{"type":"string","enum":["materials","studios"]},"graph":{"type":"integer"},"uv_editor":{"type":"boolean"},"guides":{"type":"boolean"},"frame":{"type":"boolean"},"edges":{"type":"boolean"},"statistics":{"type":"boolean"},"full_viewport":{"type":"boolean"}}}},
 {"name":"tequila_queue","description":"Start a final render to a file, in the background, with the document's render settings. folder and filename choose where; left out, it goes to the project's render folder. camera is a saved camera's number (from 0); engine is tequila, cycles-cpu or cycles-gpu; studio renders one Snapshot by its name, studios=true every Snapshot, configurations=true every valid combination of the configurator, frames=true every frame of the turntable (first and last narrow it; a frame already rendered is left out). Only when asked for a render. The person is asked first. tequila_query what=jobs shows progress and where the file went.",
  "schema":{"type":"object","properties":{"name":{"type":"string"},"width":{"type":"integer"},"height":{"type":"integer"},"samples":{"type":"integer"},"denoise":{"type":"boolean"},"folder":{"type":"string"},"filename":{"type":"string"},"camera":{"type":"integer"},"engine":{"type":"string","enum":["tequila","cycles-cpu","cycles-gpu"]},"studio":{"type":"string"},"studios":{"type":"boolean"},"configurations":{"type":"boolean"},"frames":{"type":"boolean"},"first":{"type":"integer"},"last":{"type":"integer"}}}},
 {"name":"tequila_stop","description":"Stop the render that is running in the background: it writes what it has, and the jobs waiting behind it start. job stops one job by its id (tequila_query what=jobs): the one rendering is stopped, one still waiting is taken out of the queue. all=true stops the one rendering and takes out every job waiting: how to end a spin, a turntable or a batch of views early.",
  "schema":{"type":"object","properties":{"job":{"type":"integer"},"all":{"type":"boolean"}}}},
 {"name":"tequila_save","description":"Save the project: to its own file, or with path to a new place (the person is asked first). Only when asked to save.",
  "schema":{"type":"object","properties":{"path":{"type":"string"}}}},
 {"name":"tequila_import","description":"Bring a model file into the scene: glTF, OBJ, STEP and the others Tequila reads. A path that is not absolute is in the project's folder. up says which way up the file was drawn when it comes in lying down; lighting says what to do with a Blender file's own lights. The person is asked first.",
  "schema":{"type":"object","properties":{"path":{"type":"string"},"up":{"type":"string","enum":["y","z"]},"lighting":{"type":"string","enum":["blender","tequila"]}},"required":["path"]}},
 {"name":"tequila_preferences","description":"Tequila's own settings, a page at a time. With nothing, the pages there are. page is general, interface, viewport, navigation, keyboard (the shortcuts), import, folders or updates: each of its settings with its key, value, default and what it takes. search finds settings by words. set is an object of settings by name; reset is \"all\", a page or a setting; both answer with what changed. Changing one asks the person first. Only when asked to.",
  "schema":{"type":"object","properties":{"page":{"type":"string","enum":["general","interface","viewport","navigation","keyboard","import","folders","updates"]},"search":{"type":"string"},"set":{"type":"object"},"reset":{"type":"string"}}}}
]"##;

/// The MCP bridge's tools, whose material ones the assistant offers as they
/// are: one description of how to make a material, wherever the AI is.
const MCP_TOOLS: &str = include_str!("../../tequila-cli/src/mcp_tools.json");

/// Every tool, with its schema.
pub fn all() -> Vec<Tool> {
    let parsed = json::parse(TOOLS.as_bytes()).expect("the tool list is valid JSON");
    let bridge = json::parse(MCP_TOOLS.as_bytes()).expect("the bridge's tool list is valid JSON");
    // The bridge's material, graph, UV, model and plugin tools, as they
    // are: one description of each for every AI.
    let shared = |name: &str| {
        ["tequila_material_", "tequila_plugin_", "tequila_studio_", "tequila_graph_", "tequila_uv_"].iter().any(|p| name.starts_with(p)) || ["tequila_improve", "tequila_model_units"].contains(&name)
    };
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
    /// Text as it is written: a page of the guide.
    Text,
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
        "tequila_redo" => raw(request(vec![("op", text("redo"))])),
        "tequila_stop" => raw(with("stop", pick(arguments, &["job", "all"]))),
        // A page comes back as it is written, not as a JSON string.
        "tequila_guide" => Ok(Plan::Request { json: with("guide", pick(arguments, &["topic"])), shape: Shape::Text }),
        "tequila_read" => raw(with("read", pick(arguments, &["what", "node", "index"]))),
        "tequila_select" => raw(with("select", pick(arguments, &["nodes"]))),
        "tequila_frame" => raw(with("frame", pick(arguments, &["all"]))),
        "tequila_look" => raw(with("look", pick(arguments, &["view", "at", "from", "yaw", "pitch", "distance", "focal_length"]))),
        "tequila_screenshot" => Ok(Plan::Screenshot {
            viewport: arguments.get("viewport").and_then(|v| v.as_bool()).unwrap_or(true),
        }),
        "tequila_ui" => raw(with(
            "ui",
            pick(arguments, &["viewport", "render_window", "tab", "inspector", "library", "library_section", "graph", "uv_editor", "guides", "frame", "edges", "statistics", "full_viewport"]),
        )),
        "tequila_queue" => raw(with(
            "queue",
            pick(arguments, &["name", "width", "height", "samples", "denoise", "folder", "filename", "camera", "engine", "studio", "studios", "configurations", "frames", "first", "last"]),
        )),
        "tequila_save" => raw(with("save", pick(arguments, &["path"]))),
        "tequila_import" => {
            arguments.get("path").and_then(|v| v.as_str()).filter(|p| !p.trim().is_empty()).ok_or("say which file: its path")?;
            raw(with("import", pick(arguments, &["path", "up", "lighting"])))
        }
        // Briefly: every setting at once is nineteen thousand characters.
        "tequila_preferences" => raw(with("preferences", [("brief".to_string(), Value::Bool(true))].into_iter().chain(pick(arguments, &["set", "reset", "page", "search"])).collect())),
        "tequila_library" => raw(with("library", [("action".to_string(), text("search"))].into_iter().chain(pick(arguments, &["search", "category", "limit"])).collect())),
        "tequila_command" => raw(with("help", pick(arguments, &["command"]))),
        "tequila_add_shape" => {
            arguments.get("shape").and_then(|v| v.as_str()).ok_or("say which shape: plane, cube, cylinder, sphere, disc, backdrop or wall")?;
            raw(with("add_shape", pick(arguments, &["shape", "width", "diameter", "depth", "height", "radius"])))
        }
        "tequila_library_apply" => {
            arguments.get("name").and_then(|v| v.as_str()).ok_or("say which preset: its name, from tequila_library")?;
            raw(with("library", [("action".to_string(), text("apply"))].into_iter().chain(pick(arguments, &["name", "nodes", "unlink"])).collect()))
        }
        // A part at a time: the whole guide is thirty thousand characters.
        "tequila_material_guide" => {
            let topic = arguments.get("topic").and_then(|v| v.as_str()).map(str::trim).filter(|t| !t.is_empty()).unwrap_or("contents");
            raw(with("material", vec![("action".into(), text("guide")), ("topic".into(), text(topic))]))
        }
        "tequila_graph_nodes" => raw(with("graph", [("action".to_string(), text("catalogue"))].into_iter().chain(pick(arguments, &["search", "family", "name", "page"])).collect())),
        "tequila_graph_read" => {
            arguments.get("material").and_then(|v| v.as_f64()).ok_or("say which material: its id")?;
            raw(with("graph", [("action".to_string(), text("read"))].into_iter().chain(pick(arguments, &["material", "node", "page"])).collect()))
        }
        "tequila_graph_edit" => {
            arguments.get("material").and_then(|v| v.as_f64()).ok_or("say which material: its id")?;
            let action = arguments.get("action").and_then(|v| v.as_str()).unwrap_or("");
            if !["add", "set", "connect", "disconnect", "remove"].contains(&action) {
                return Err("say what to do, as action: add, set, connect, disconnect or remove".into());
            }
            raw(with("graph", pick(arguments, &["action", "material", "node", "settings", "fields", "inputs", "label", "muted", "from", "output", "to", "input"])))
        }
        "tequila_uv_islands" | "tequila_uv_cut" | "tequila_uv_stitch" => {
            let action = name.trim_start_matches("tequila_uv_");
            arguments.get("node").and_then(|v| v.as_f64()).ok_or("say which part: its id, as node")?;
            let taken: &[&str] = match action {
                "islands" => &["node", "set", "page"],
                "cut" => &["node", "set", "path", "sharper_than", "islands", "pack", "reflatten", "across"],
                _ => &["node", "set", "islands"],
            };
            raw(with("uv", [("action".to_string(), text(action))].into_iter().chain(pick(arguments, taken)).collect()))
        }
        "tequila_improve" => {
            arguments.get("node").and_then(|v| v.as_f64()).ok_or("say which node: its id")?;
            raw(with("improve", pick(arguments, &["node", "subdivide", "shade", "crease"])))
        }
        "tequila_model_units" => {
            arguments.get("node").and_then(|v| v.as_f64()).ok_or("say which node: its id")?;
            raw(with("model_units", pick(arguments, &["node", "unit", "read_as"])))
        }
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
        Shape::Text => match answer {
            Value::String(text) => return text.clone(),
            other => other.clone(),
        },
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

/// The guide's core: the Tequila skill's first page, which every assistant
/// is given whole (`tequila_scripting::guide::core` is the same text; a
/// test in the application, which has both, holds the two together).
const SKILL: &str = include_str!("../../../skills/tequila/SKILL.md");

/// What is said before the core to an assistant that Tequila runs itself,
/// through a provider's API or a model on this computer: where it is, who
/// it is working for, and what the application asks the person first.
const WHERE: &str = "You are the assistant built into Tequila, a product-rendering application, and you work on the scene that is open in it right now, using your tools. \
The person is a designer, not a programmer: answer in plain words, briefly, and do what they ask in the scene rather than describing how.

- Before anything that leaves the scene - importing a file, saving to a new place, rendering to a file, a preference, saving to the library, writing or running a plugin - Tequila asks the person itself. Call the tool and wait for its answer rather than asking first; if they said no, do not try another way. Only render, save, import or change a preference when asked to.
- A name written as @Name in a message is a part or a material of the scene, and its id follows in brackets.
- You have no tequila_request and no tequila_render tool. Where the guide sends a request through tequila_request (exporting, updating a model from its file, a texture pack, a UV layout picture), tell the person it is theirs to do in Tequila and where it is in the menus; for a quick look, switch the viewport to rendered and take a tequila_screenshot. render.set_settings is not yours either: tequila_queue takes the size, samples and engine.
- tequila_query offers selection, camera, commands, ui, jobs and history; read anything else of the project with tequila_read.
- Begin every request by calling tequila_guide with the page for the job. Only then look at the scene and change it.

";

/// What the assistant is told about the application and how to work in it:
/// where it is, then the guide's core, as every assistant has it.
pub fn system() -> &'static str {
    static SYSTEM: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SYSTEM.get_or_init(|| format!("{WHERE}{}", core()))
}

/// The skill's first page without the front matter a skill folder starts
/// with.
pub fn core() -> &'static str {
    let Some(rest) = SKILL.strip_prefix("---") else { return SKILL };
    match rest.split_once("\n---") {
        Some((_, body)) => body.trim_start(),
        None => SKILL,
    }
}

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
                "tequila_import" => args(r#"{"path":"model.glb"}"#),
                "tequila_library_apply" => args(r#"{"name":"Brushed Aluminium","nodes":[1]}"#),
                "tequila_add_shape" => args(r#"{"shape":"cylinder","width":0.3,"height":0.15}"#),
                "tequila_graph_read" => args(r#"{"material":2}"#),
                "tequila_graph_edit" => args(r#"{"material":2,"action":"add","node":"bump"}"#),
                "tequila_uv_islands" | "tequila_uv_cut" | "tequila_improve" | "tequila_model_units" => args(r#"{"node":3}"#),
                "tequila_uv_stitch" => args(r#"{"node":3,"islands":[0,1]}"#),
                _ => args("{}"),
            };
            plan(&tool.name, &arguments).unwrap_or_else(|e| panic!("{}: {e}", tool.name));
            assert_eq!(tool.schema.get("type").and_then(|t| t.as_str()), Some("object"), "{}", tool.name);
        }
    }

    /// The tools for the jobs the window alone could do become their
    /// requests, with nothing passed that the tool does not take; and the
    /// two long answers are asked for a part at a time, whatever the model
    /// leaves out.
    #[test]
    fn the_graph_uv_and_model_tools_become_requests() {
        let sent = |name: &str, arguments: &str| match plan(name, &args(arguments)).unwrap_or_else(|e| panic!("{name}: {e}")) {
            Plan::Request { json, .. } => json::parse(json.as_bytes()).unwrap(),
            other => panic!("{name}: {other:?}"),
        };
        let text = |value: &Value, key: &str| value.get(key).and_then(|v| v.as_str()).map(str::to_string);
        assert_eq!(text(&sent("tequila_graph_nodes", r#"{"search":"voronoi","path":"/etc"}"#), "action").as_deref(), Some("catalogue"));
        assert!(sent("tequila_graph_nodes", r#"{"search":"voronoi","path":"/etc"}"#).get("path").is_none());
        let wire = sent("tequila_graph_edit", r#"{"material":2,"action":"connect","from":3,"output":"Distance","input":"base_color"}"#);
        assert_eq!((text(&wire, "op").as_deref(), wire.get("from").and_then(|v| v.as_f64())), (Some("graph"), Some(3.0)));
        assert!(plan("tequila_graph_edit", &args(r#"{"material":2,"action":"catalogue"}"#)).is_err());
        assert!(plan("tequila_graph_edit", &args(r#"{"action":"add","node":"bump"}"#)).is_err(), "a material is named");
        assert_eq!(text(&sent("tequila_uv_cut", r#"{"node":3,"sharper_than":40}"#), "action").as_deref(), Some("cut"));
        assert_eq!(text(&sent("tequila_uv_stitch", r#"{"node":3,"islands":[0,1]}"#), "action").as_deref(), Some("stitch"));
        assert_eq!(text(&sent("tequila_improve", r#"{"node":3,"shade":30}"#), "op").as_deref(), Some("improve"));
        assert_eq!(text(&sent("tequila_model_units", r#"{"node":3,"unit":"mm"}"#), "unit").as_deref(), Some("mm"));
        assert_eq!(sent("tequila_queue", r#"{"frames":true,"last":11}"#).get("frames").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(sent("tequila_stop", r#"{"all":true}"#).get("all").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(text(&sent("tequila_material_guide", "{}"), "topic").as_deref(), Some("contents"));
        assert_eq!(text(&sent("tequila_material_guide", r#"{"topic":"leather"}"#), "topic").as_deref(), Some("leather"));
        assert_eq!(sent("tequila_preferences", "{}").get("brief").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(text(&sent("tequila_preferences", r#"{"page":"viewport"}"#), "page").as_deref(), Some("viewport"));
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

    /// Unlisted arguments are dropped, not forwarded: a tool passes on what
    /// it says it takes and nothing else.
    #[test]
    fn only_listed_arguments_pass_through() {
        match plan("tequila_queue", &args(r#"{"samples":64,"folder":"renders","token":"x","network":true}"#)).unwrap() {
            Plan::Request { json, .. } => {
                assert!(json.contains("samples") && json.contains("renders"));
                assert!(!json.contains("token") && !json.contains("network"), "{json}");
            }
            other => panic!("{other:?}"),
        }
        match plan("tequila_import", &args(r#"{"path":"model.glb","parent":3}"#)).unwrap() {
            Plan::Request { json, .. } => assert!(json.contains("model.glb") && !json.contains("parent"), "{json}"),
            other => panic!("{other:?}"),
        }
    }

    /// What leaves the scene becomes the request the application asks the
    /// person about: a save somewhere new, an import, a preference, a
    /// preset from the library.
    #[test]
    fn what_leaves_the_scene_is_a_request_the_application_can_ask_about() {
        let request = |tool: &str, arguments: &str| match plan(tool, &args(arguments)).unwrap() {
            Plan::Request { json, .. } => args(&json),
            other => panic!("{other:?}"),
        };
        let save = request("tequila_save", r#"{"path":"copy.tequila"}"#);
        assert_eq!((save.get("op").and_then(Value::as_str), save.get("path").and_then(Value::as_str)), (Some("save"), Some("copy.tequila")));
        assert!(request("tequila_save", "{}").get("path").is_none(), "its own file, when no path is given");
        let import = request("tequila_import", r#"{"path":"model.glb","up":"z"}"#);
        assert_eq!((import.get("op").and_then(Value::as_str), import.get("up").and_then(Value::as_str)), (Some("import"), Some("z")));
        assert!(plan("tequila_import", &args("{}")).is_err());
        let set = request("tequila_preferences", r#"{"set":{"viewport.gizmo_size":150}}"#);
        assert_eq!(set.get("op").and_then(Value::as_str), Some("preferences"));
        assert!(set.get("set").is_some());
        let preset = request("tequila_library_apply", r#"{"name":"Brushed Aluminium","nodes":[4],"unlink":true}"#);
        assert_eq!((preset.get("op").and_then(Value::as_str), preset.get("action").and_then(Value::as_str)), (Some("library"), Some("apply")));
        // The library is searched, not handed over whole; a command is
        // looked up by its name; a shape is added by its own request.
        let search = request("tequila_library", r#"{"search":"black leather","limit":5,"path":"/elsewhere"}"#);
        assert_eq!((search.get("op").and_then(Value::as_str), search.get("action").and_then(Value::as_str), search.get("search").and_then(Value::as_str)), (Some("library"), Some("search"), Some("black leather")));
        assert!(search.get("path").is_none());
        assert_eq!(request("tequila_command", r#"{"command":"scene.set_visibility"}"#).get("op").and_then(Value::as_str), Some("help"));
        let shape = request("tequila_add_shape", r#"{"shape":"cylinder","width":0.3,"height":0.15}"#);
        assert_eq!((shape.get("op").and_then(Value::as_str), shape.get("width").and_then(Value::as_f64)), (Some("add_shape"), Some(0.3)));
        assert!(plan("tequila_add_shape", &args("{}")).is_err());
        assert!(system().contains("ibrary first") && system().contains("tequila_command") && system().contains("tequila_guide"));
    }

    /// The assistant starts with the guide's core, not with a text of its
    /// own, and reads the rest a page at a time; what the core sends it to
    /// by a tool's name, it has - or has been told it has not.
    #[test]
    fn the_assistant_is_given_the_guides_core_and_reads_the_pages() {
        let told = system();
        assert!(told.starts_with("You are the assistant built into Tequila"));
        assert!(told.ends_with(core()) && !core().starts_with("---"));
        assert!(told.len() < 10_000, "{} characters before the first message", told.len());
        let offered: Vec<String> = all().into_iter().map(|tool| tool.name).collect();
        let mut named: Vec<&str> = core().split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).filter(|word| word.starts_with("tequila_")).collect();
        named.sort_unstable();
        named.dedup();
        for tool in named {
            assert!(offered.iter().any(|name| name == tool) || WHERE.contains(tool), "the core names {tool}, which this assistant has not and is not told so");
        }
        match plan("tequila_guide", &args(r#"{"topic":"uv","path":"/elsewhere"}"#)).unwrap() {
            Plan::Request { json, shape } => {
                assert_eq!(shape, Shape::Text);
                let request = args(&json);
                assert_eq!((request.get("op").and_then(Value::as_str), request.get("topic").and_then(Value::as_str)), (Some("guide"), Some("uv")));
                assert!(request.get("path").is_none());
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(describe(&Shape::Text, &Value::String("# UV\n\nA page.".into())), "# UV\n\nA page.");
        match plan("tequila_read", &args(r#"{"what":"cameras","index":1}"#)).unwrap() {
            Plan::Request { json, .. } => assert!(json.contains("\"read\"") && json.contains("cameras")),
            other => panic!("{other:?}"),
        }
        match plan("tequila_queue", &args(r#"{"studios":true,"camera":1}"#)).unwrap() {
            Plan::Request { json, .. } => assert!(json.contains("studios") && json.contains("camera")),
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
