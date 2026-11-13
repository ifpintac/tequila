//! What the assistant may do, and how each tool becomes a scripting request.
//!
//! The same surface the MCP bridge offers, less quitting, opening another
//! project, a modelling application's link and the Assistant's own
//! controls. The assistant works on the open scene. What leaves it - a file
//! brought in or written, the project saved somewhere new, a render, a
//! preference, the library - is a tool like the rest, or a request sent
//! through `tequila_request` ([`REQUESTS`]), and the application asks the
//! person before it happens (`tequila_scripting::approval`): the person
//! decides what comes into the scene and where it goes. Where renders are
//! written is theirs alone: the render settings can be changed, but not
//! their folder or file name ([`keep_render_output`]).

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
 {"name":"tequila_library_apply","description":"Put a preset from the material library on parts, by its exact name from tequila_library. Answers with the material's id. color (hex sRGB, \"#1f4d2b\") applies it in another colour, pattern and finish kept, as a material of its own named for the colour. unlink=true gives the parts a copy of their own. To change a preset's colour once it is on a part use tequila_material_tint, not base_color: many presets keep their colour inside a pattern. Undoable.",
  "schema":{"type":"object","properties":{"name":{"type":"string"},"nodes":{"type":"array","items":{"type":"integer"}},"color":{"type":"string"},"unlink":{"type":"boolean"}},"required":["name","nodes"]}},
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
 {"name":"tequila_frame","description":"Point the camera at everything (all=true, the default) or at the selection. This moves the view the person is looking at: for when they ask for one, or to find something out of sight - not after a material edit, which leaves the camera where it was.",
  "schema":{"type":"object","properties":{"all":{"type":"boolean"}}}},
 {"name":"tequila_look","description":"Place the camera: a standard view; or yaw/pitch in degrees and distance around a target 'at'; or an exact eye 'from' looking at 'at'. Optional focal_length in mm. Undoable. The view is the person's too: read it first (tequila_query camera) if you mean to put it back.",
  "schema":{"type":"object","properties":{"view":{"type":"string","enum":["front","back","left","right","top","bottom","iso"]},"at":{"type":"array","items":{"type":"number"}},"from":{"type":"array","items":{"type":"number"}},"yaw":{"type":"number"},"pitch":{"type":"number"},"distance":{"type":"number"},"focal_length":{"type":"number"}}}},
 {"name":"tequila_screenshot","description":"Look at the scene as it is now. Returns an image. viewport=true (default) shows only the 3D view. Switch the viewport to rendered first (tequila_ui) to judge materials and lighting; settle is how many seconds the picture is given to finish (12). window looks at one of Tequila's other windows instead, which must be open: uv (the UV editor), graph (the Material Graph), render, preferences.",
  "schema":{"type":"object","properties":{"viewport":{"type":"boolean"},"settle":{"type":"number"},"window":{"type":"string","enum":["uv","graph","render","preferences"]}}}},
 {"name":"tequila_render","description":"A quick true render of the view, returned as an image; nothing is kept. Tequila is busy until it is done, so keep it small: up to about 800 pixels and 64 samples (640 by 400 at 32 when left out). wireframe=true draws the model's edges instead, in a second.",
  "schema":{"type":"object","properties":{"width":{"type":"integer"},"height":{"type":"integer"},"samples":{"type":"integer"},"wireframe":{"type":"boolean"}}}},
 {"name":"tequila_ui","description":"Switch the viewport between 'rendered' (ray traced, shows real materials and light), 'material' and 'solid' (fast previews) and 'wireframe', and show the person a panel or a window: an Inspector tab, the Render window, the library, the Material Graph on a material (graph: its id), the UV editor on the selected part, the composition guides, the render's frame in the viewport.",
  "schema":{"type":"object","properties":{"viewport":{"type":"string","enum":["rendered","material","solid","wireframe"]},"render_window":{"type":"boolean"},"tab":{"type":"string","enum":["scene","material","environment","lighting","camera","image"]},"inspector":{"type":"boolean"},"library":{"type":"boolean"},"library_section":{"type":"string","enum":["materials","studios"]},"graph":{"type":"integer"},"uv_editor":{"type":"boolean"},"guides":{"type":"boolean"},"frame":{"type":"boolean"},"edges":{"type":"boolean"},"statistics":{"type":"boolean"},"full_viewport":{"type":"boolean"}}}},
 {"name":"tequila_queue","description":"Start a final render to a file, in the background, with the document's render settings. folder and filename choose where; left out, it goes to the project's render folder. camera is a saved camera's number (from 0); engine is tequila, cycles-cpu or cycles-gpu; studio renders one Snapshot by its name, studios=true every Snapshot, configurations=true every valid combination of the configurator, frames=true every frame of the turntable (first and last narrow it; a frame already rendered is left out). Only when asked for a render. The person is asked first. tequila_query what=jobs shows progress and where the file went.",
  "schema":{"type":"object","properties":{"name":{"type":"string"},"width":{"type":"integer"},"height":{"type":"integer"},"samples":{"type":"integer"},"denoise":{"type":"boolean"},"folder":{"type":"string"},"filename":{"type":"string"},"camera":{"type":"integer"},"engine":{"type":"string","enum":["tequila","cycles-cpu","cycles-gpu"]},"studio":{"type":"string"},"studios":{"type":"boolean"},"configurations":{"type":"boolean"},"frames":{"type":"boolean"},"first":{"type":"integer"},"last":{"type":"integer"}}}},
 {"name":"tequila_jobs","description":"The renders started with tequila_queue: each one's state, its progress and, when done, the file it wrote.",
  "schema":{"type":"object","properties":{}}},
 {"name":"tequila_stop","description":"Stop the render that is running in the background: it writes what it has, and the jobs waiting behind it start. job stops one job by its id (tequila_query what=jobs): the one rendering is stopped, one still waiting is taken out of the queue. all=true stops the one rendering and takes out every job waiting: how to end a spin, a turntable or a batch of views early.",
  "schema":{"type":"object","properties":{"job":{"type":"integer"},"all":{"type":"boolean"}}}},
 {"name":"tequila_save","description":"Save the project: to its own file, or with path to a new place (the person is asked first). Only when asked to save.",
  "schema":{"type":"object","properties":{"path":{"type":"string"}}}},
 {"name":"tequila_import","description":"Bring a model file into the scene: glTF, OBJ, STEP and the others Tequila reads. A path that is not absolute is in the project's folder. up says which way up the file was drawn when it comes in lying down; lighting says what to do with a Blender file's own lights. The person is asked first.",
  "schema":{"type":"object","properties":{"path":{"type":"string"},"up":{"type":"string","enum":["y","z"]},"lighting":{"type":"string","enum":["blender","tequila"]}},"required":["path"]}},
 {"name":"tequila_preferences","description":"Tequila's own settings, a page at a time. With nothing, the pages there are. page is general, interface, viewport, navigation, keyboard (the shortcuts), import, folders or updates: each of its settings with its key, value, default and what it takes. search finds settings by words. set is an object of settings by name; reset is \"all\", a page or a setting; both answer with what changed. Changing one asks the person first. Only when asked to.",
  "schema":{"type":"object","properties":{"page":{"type":"string","enum":["general","interface","viewport","navigation","keyboard","import","folders","updates"]},"search":{"type":"string"},"set":{"type":"object"},"reset":{"type":"string"}}}},
 {"name":"tequila_request","description":"One request that has no tool of its own, written as the guide's page for the job shows it: {\"request\":{\"op\":\"export\",\"format\":\"glb\",\"path\":\"kettle.glb\"}}. The operations: export (glb, usd, usdz, web, spin), reimport (update a model from its file), texture_set (a folder or ZIP of picture maps as a material), substance, uv_layout (a part's UV layout as a picture), studio_kit, library (save, import, export, delete), studio_preset, queue (network rendering), plugin, screenshot and render (to a file the person named), and ui (the UV editor's and the graph editor's own controls). A path that is not absolute is in the project's folder. The person is asked before a file is read or written. Read the page first: a request is not guessed.",
  "schema":{"type":"object","properties":{"request":{"type":"object","description":"The request: {\"op\":\"<name>\", ...fields}"}},"required":["request"]}}
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
    /// Take a screenshot: deferred, because the window has to be drawn
    /// first. Of the main window, given `settle` seconds for its picture to
    /// finish, or of one of the others (`window`).
    Screenshot { viewport: bool, window: Option<String>, settle: f32 },
    /// A quick render of the view, answered with the picture; the
    /// application says where its scratch file goes.
    Render { width: usize, height: usize, samples: u32, wireframe: bool },
    /// An `apply` or a `batch` (`json`) that changes the render settings.
    /// Before it is sent, the application lays each block over the
    /// project's own with [`keep_render_output`], which refuses one that
    /// would move where renders are written.
    RenderSettings { json: String },
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

/// The operations `tequila_request` sends as they are written: what has no
/// tool of its own. A list of what may pass, not of what may not, so an
/// operation Tequila learns later is not the assistant's until it is put
/// here. Each is asked about by the application where it reads or writes a
/// file (`tequila_scripting::approval`).
pub const REQUESTS: [&str; 13] =
    ["export", "reimport", "texture_set", "substance", "uv_layout", "studio_kit", "library", "studio_preset", "queue", "plugin", "screenshot", "render", "ui"];

/// The operations that have a tool whose arguments are the request's own
/// fields: sent through `tequila_request`, they are that tool's call, held
/// to what the tool allows.
const REQUEST_TOOLS: [(&str, &str); 17] = [
    ("apply", "tequila_apply"),
    ("batch", "tequila_batch"),
    ("undo", "tequila_undo"),
    ("redo", "tequila_redo"),
    ("stop", "tequila_stop"),
    ("select", "tequila_select"),
    ("frame", "tequila_frame"),
    ("look", "tequila_look"),
    ("save", "tequila_save"),
    ("import", "tequila_import"),
    ("preferences", "tequila_preferences"),
    ("add_shape", "tequila_add_shape"),
    ("guide", "tequila_guide"),
    ("read", "tequila_read"),
    ("help", "tequila_command"),
    ("improve", "tequila_improve"),
    ("model_units", "tequila_model_units"),
];

/// What the person does themselves, and the assistant is told so.
const NOT_THE_ASSISTANTS: [(&str, &str); 5] = [
    ("quit", "Quitting Tequila is the person's to do"),
    ("new", "Starting a new project closes the open one, and is the person's to do: File > New"),
    ("open", "Opening another project closes the open one, and is the person's to do: File > Open"),
    ("assistant", "The Assistant cannot drive itself"),
    ("livelink", "A modelling application's link is that application's to send"),
];

/// The fields of a render settings block that say where renders are
/// written: the person's, and never the assistant's to change.
pub const RENDER_OUTPUT: [&str; 2] = ["output_directory", "filename_template"];

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
            if !matches!(command, Value::Object(_)) {
                return Err("the command must be an object: {\"command\":\"<name>\", ...}".into());
            }
            let json = request(vec![("op", text("apply")), ("command", command.clone())]);
            if sets_render(command) { Ok(Plan::RenderSettings { json }) } else { raw(json) }
        }
        "tequila_batch" => {
            let Some(Value::Array(ops)) = arguments.get("ops") else {
                return Err("give ops: an array of operations".into());
            };
            for (i, op) in ops.iter().enumerate() {
                let kind = op.get("op").and_then(|v| v.as_str()).unwrap_or("(none)");
                if !BATCH_OPS.contains(&kind) {
                    return Err(format!("operation {} is `{kind}`; a batch here may hold {}", i + 1, BATCH_OPS.join(", ")));
                }
            }
            let json = request(vec![("op", text("batch")), ("ops", Value::Array(ops.clone()))]);
            if ops.iter().any(|op| op.get("command").is_some_and(sets_render)) { Ok(Plan::RenderSettings { json }) } else { raw(json) }
        }
        "tequila_request" => {
            let request = arguments.get("request").filter(|r| matches!(r, Value::Object(_))).ok_or("give the request: an object with an \"op\"")?;
            let op = request.get("op").and_then(|v| v.as_str()).ok_or("the request needs an \"op\": {\"op\":\"<name>\", ...}")?;
            if let Some((_, why)) = NOT_THE_ASSISTANTS.iter().find(|(name, _)| *name == op) {
                return Err(format!("{why}. Tell them so; do not try it another way."));
            }
            // A tool's own job stays that tool's, with what it allows.
            if let Some((_, tool)) = REQUEST_TOOLS.iter().find(|(name, _)| *name == op) {
                return plan(tool, request);
            }
            if op == "query" {
                let what = request.get("what").and_then(|v| v.as_str()).unwrap_or_default();
                return match what {
                    "scene" | "nodes" | "materials" | "api" => plan(&format!("tequila_{what}"), request),
                    "library" => Err("the library is searched, not read whole: tequila_library".into()),
                    "context" => Err("the project is read a part at a time: tequila_read".into()),
                    _ => plan("tequila_query", request),
                };
            }
            if op == "material" {
                return Err("a material request has tools of its own: tequila_material_guide, tequila_material_preview, tequila_material_create, tequila_material_update, tequila_material_tint, tequila_material_recipe and tequila_material_save".into());
            }
            if op == "graph" {
                return Err("a graph request has tools of its own: tequila_graph_nodes (the catalogue), tequila_graph_read and tequila_graph_edit".into());
            }
            if op == "uv" {
                return Err("a UV request has tools of its own: tequila_uv_islands, tequila_uv_cut and tequila_uv_stitch".into());
            }
            if !REQUESTS.contains(&op) {
                return Err(format!("`{op}` is not a request this tool sends; it sends {}", REQUESTS.join(", ")));
            }
            // As written, less what only the connection itself may say: the
            // request's number, the token, and the mark of an agent.
            let mut fields: Vec<(String, Value)> = match request {
                Value::Object(fields) => fields.iter().filter(|(key, _)| !matches!(key.as_str(), "id" | "token" | "agent")).map(|(k, v)| (k.clone(), v.clone())).collect(),
                _ => Vec::new(),
            };
            if op == "plugin" {
                let action = request.get("action").and_then(|v| v.as_str()).unwrap_or_default();
                // Code this assistant changes runs only once the person
                // says so, whatever the request said of asking.
                fields.retain(|(key, _)| key != "ask");
                if action == "write" {
                    fields.push(("ask".into(), Value::Bool(true)));
                }
                if action == "run" {
                    let word = |key: &str| request.get(key).and_then(|v| v.as_str()).map(str::to_string);
                    let plugin = word("plugin").ok_or("say which plugin: its id, as plugin")?;
                    let command = word("command").ok_or("say which command")?;
                    return Ok(Plan::PluginRun { json: with_fields(fields), plugin, command });
                }
            }
            raw(with_fields(fields))
        }
        "tequila_undo" => raw(request(vec![("op", text("undo"))])),
        "tequila_redo" => raw(request(vec![("op", text("redo"))])),
        "tequila_stop" => raw(with("stop", pick(arguments, &["job", "all"]))),
        "tequila_jobs" => raw(query("jobs")),
        // A page comes back as it is written, not as a JSON string.
        "tequila_guide" => Ok(Plan::Request { json: with("guide", pick(arguments, &["topic"])), shape: Shape::Text }),
        "tequila_read" => raw(with("read", pick(arguments, &["what", "node", "index"]))),
        "tequila_select" => raw(with("select", pick(arguments, &["nodes"]))),
        "tequila_frame" => raw(with("frame", pick(arguments, &["all"]))),
        "tequila_look" => raw(with("look", pick(arguments, &["view", "at", "from", "yaw", "pitch", "distance", "focal_length"]))),
        "tequila_screenshot" => Ok(Plan::Screenshot {
            viewport: arguments.get("viewport").and_then(|v| v.as_bool()).unwrap_or(true),
            window: arguments.get("window").and_then(|v| v.as_str()).map(str::to_string),
            settle: arguments.get("settle").and_then(|v| v.as_f64()).unwrap_or(12.0).clamp(0.0, 60.0) as f32,
        }),
        "tequila_render" => {
            let count = |key: &str, fallback: f64, most: f64| arguments.get(key).and_then(|v| v.as_f64()).unwrap_or(fallback).clamp(1.0, most);
            Ok(Plan::Render {
                width: count("width", 640.0, 8192.0) as usize,
                height: count("height", 400.0, 8192.0) as usize,
                samples: count("samples", 32.0, 65536.0) as u32,
                wireframe: arguments.get("wireframe").and_then(|v| v.as_bool()).unwrap_or(false),
            })
        }
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
            raw(with("library", [("action".to_string(), text("apply"))].into_iter().chain(pick(arguments, &["name", "nodes", "unlink", "color"])).collect()))
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
        "tequila_material_tint" => {
            arguments.get("material").and_then(|v| v.as_f64()).ok_or("say which material: its id, as material")?;
            arguments.get("color").and_then(|v| v.as_str()).ok_or("say which colour: a hex sRGB string like \"#1f4d2b\", as color")?;
            Ok(Plan::Material {
                json: with("material", [("action".to_string(), text("tint"))].into_iter().chain(pick(arguments, &["material", "color"])).collect()),
                picture: arguments.get("preview").and_then(|v| v.as_bool()).unwrap_or(true),
            })
        }
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

/// A request from its fields, as they are.
fn with_fields(fields: Vec<(String, Value)>) -> String {
    let mut out = Object::new();
    for (key, value) in fields {
        out.set(&key, value);
    }
    json_write::to_string(&out.build())
}

/// Whether a command is the one that sets the render settings.
fn sets_render(command: &Value) -> bool {
    command.get("command").and_then(|v| v.as_str()) == Some("render.set_settings")
}

/// The render settings an assistant sent, laid over the project's own
/// (`current`, as `tequila_read` gives them): the block to set.
///
/// The settings are one block, sent whole, and among them is where renders
/// are written - the one thing in it that is the person's alone, since a
/// render written somewhere else is a file nobody was asked about. So a
/// block that names another folder or file name is refused, and says where
/// one render's place can be chosen with the person's yes. Everything else
/// is the assistant's to set; what it leaves out stays as it is, so a model
/// that sends only `format` and `alpha_mode` has not put the rest back to
/// what a new project has.
pub fn render_settings_kept(current: &Value, sent: &Value) -> Result<Value, String> {
    let (Value::Object(current), Value::Object(sent)) = (current, sent) else {
        return Err("render.set_settings takes the settings as `render`: an object, as tequila_read {\"what\":\"render\"} gives them".into());
    };
    let mut kept = current.clone();
    for (key, value) in sent {
        if RENDER_OUTPUT.contains(&key.as_str()) {
            if current.get(key) != Some(value) {
                return Err(format!(
                    "Where renders are written is the person's to choose: `{key}` stays as it is. Leave it as it was read, or out. \
                     For one render somewhere else, tequila_queue takes a folder and a filename, and the person is asked."
                ));
            }
            continue;
        }
        kept.insert(key.clone(), value.clone());
    }
    Ok(Value::Object(kept))
}

/// An `apply` or a `batch` that changes the render settings
/// ([`Plan::RenderSettings`]), with each block laid over the project's own
/// by [`render_settings_kept`]: the request to send. Two in one batch build
/// on each other.
pub fn keep_render_output(json: &str, current: &Value) -> Result<String, String> {
    let mut request = json::parse(json.as_bytes()).map_err(|_| "the request could not be read".to_string())?;
    let mut current = current.clone();
    let mut keep = |command: &mut Value| -> Result<(), String> {
        if !sets_render(command) {
            return Ok(());
        }
        let kept = render_settings_kept(&current, command.get("render").unwrap_or(&Value::Null))?;
        current = kept.clone();
        if let Value::Object(fields) = command {
            fields.insert("render".into(), kept);
        }
        Ok(())
    };
    if let Value::Object(fields) = &mut request {
        if let Some(command) = fields.get_mut("command") {
            keep(command)?;
        }
        if let Some(Value::Array(ops)) = fields.get_mut("ops") {
            for op in ops {
                if let Value::Object(op) = op {
                    if let Some(command) = op.get_mut("command") {
                        keep(command)?;
                    }
                }
            }
        }
    }
    Ok(json_write::to_string(&request))
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

- Before anything that leaves the scene - importing or exporting a file, saving to a new place, rendering to a file, a preference, the library, writing or running a plugin - Tequila asks the person itself. Call the tool and wait for its answer rather than asking first; if they said no, do not try another way. Only render, save, import, export or change a preference when asked to.
- A name written as @Name in a message is a part or a material of the scene, and its id follows in brackets.
- tequila_request sends the requests the guide's pages write for it: export, reimport, texture_set, substance, uv_layout, studio_kit, library, studio_preset, queue, plugin, screenshot, render and ui. Starting or opening another project and quitting are the person's to do, in the File menu: say so.
- render.set_settings changes the fields you send and leaves the rest. Where renders are written is the person's: leave output_directory and filename_template as they were read, or out.
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

    /// The render settings as a project has them, for the tests: the few
    /// fields that matter here.
    const RENDER: &str = r#"{"engine":"cycles-gpu","resolution":[1920,1080],"max_samples":256,"format":"jpeg","alpha_mode":"none","output_directory":"/work/renders","filename_template":"{project}_{camera}"}"#;

    /// The assistant sets the render settings - format, transparency,
    /// layers - but a block that would move where renders are written is
    /// refused, alone or in a batch, through whichever tool it is sent.
    #[test]
    fn output_paths_cannot_be_redirected_through_apply_or_batch() {
        let current = args(RENDER);
        let command = r#"{"command":"render.set_settings","render":{"format":"png","output_directory":"C:/outside","filename_template":"redirected"}}"#;
        for (tool, input) in [
            ("tequila_apply", format!(r#"{{"command":{command}}}"#)),
            ("tequila_batch", format!(r#"{{"ops":[{{"op":"select","nodes":[1]}},{{"op":"apply","command":{command}}}]}}"#)),
            ("tequila_request", format!(r#"{{"request":{{"op":"apply","command":{command}}}}}"#)),
            ("tequila_request", format!(r#"{{"request":{{"op":"batch","ops":[{{"op":"apply","command":{command}}}]}}}}"#)),
        ] {
            // Never a request to send as it is: one the application lays
            // over the project's own settings first.
            let Plan::RenderSettings { json } = plan(tool, &args(&input)).unwrap() else { panic!("{tool}: sent as written") };
            let refused = keep_render_output(&json, &current).unwrap_err();
            assert!(RENDER_OUTPUT.iter().any(|field| refused.contains(field)) && refused.contains("the person's"), "{tool}: {refused}");
        }
        for moved in [r#"{"filename_template":"elsewhere"}"#, r#"{"output_directory":null}"#, r#"{"output_directory":"/work/renders/../../other"}"#] {
            assert!(render_settings_kept(&current, &args(moved)).is_err(), "{moved}");
        }
        assert!(render_settings_kept(&current, &args("\"png\"")).is_err(), "a block is an object");
    }

    /// What the assistant sends is laid over what the project has: the
    /// fields it names change, the rest stay - the folder and the file
    /// name among them, sent back as they were read or left out.
    #[test]
    fn render_settings_change_only_what_was_sent() {
        let current = args(RENDER);
        let Plan::RenderSettings { json } = plan("tequila_apply", &args(r#"{"command":{"command":"render.set_settings","render":{"format":"png","alpha_mode":"straight"}}}"#)).unwrap() else { panic!() };
        let sent = args(&keep_render_output(&json, &current).unwrap());
        assert_eq!(sent.get("op").and_then(Value::as_str), Some("apply"));
        let block = sent.get("command").and_then(|c| c.get("render")).unwrap();
        let word = |key: &str| block.get(key).and_then(Value::as_str);
        assert_eq!((word("format"), word("alpha_mode")), (Some("png"), Some("straight")));
        assert_eq!((word("output_directory"), word("filename_template")), (Some("/work/renders"), Some("{project}_{camera}")));
        assert_eq!((word("engine"), block.get("max_samples").and_then(Value::as_f64)), (Some("cycles-gpu"), Some(256.0)), "what was not sent stays");

        // The whole block sent back, as the guide teaches, is the same thing.
        let mut whole = current.clone();
        if let Value::Object(fields) = &mut whole {
            fields.insert("alpha_mode".into(), Value::String("straight".into()));
        }
        let kept = render_settings_kept(&current, &whole).unwrap();
        assert_eq!(kept, whole);

        // Two in one batch build on each other; the other steps are left alone.
        let batch = r#"{"ops":[{"op":"apply","command":{"command":"render.set_settings","render":{"format":"png"}}},{"op":"apply","command":{"command":"scene.rename","node":1,"name":"x"}},{"op":"apply","command":{"command":"render.set_settings","render":{"max_samples":64}}}]}"#;
        let Plan::RenderSettings { json } = plan("tequila_batch", &args(batch)).unwrap() else { panic!() };
        let sent = args(&keep_render_output(&json, &current).unwrap());
        let ops = sent.get("ops").unwrap().as_array();
        let last = ops[2].get("command").and_then(|c| c.get("render")).unwrap();
        assert_eq!((last.get("format").and_then(Value::as_str), last.get("max_samples").and_then(Value::as_f64)), (Some("png"), Some(64.0)));
        assert_eq!(ops[1].get("command").and_then(|c| c.get("name")).and_then(Value::as_str), Some("x"));
        // A batch without one is a request like any other.
        assert!(matches!(plan("tequila_batch", &args(r#"{"ops":[{"op":"select","nodes":[1]}]}"#)).unwrap(), Plan::Request { .. }));
    }

    /// `tequila_request` sends what has no tool of its own, from a list;
    /// what closes the project or drives the Assistant is not on it, and
    /// what has a tool is that tool's call, with what the tool allows.
    #[test]
    fn a_raw_request_is_one_of_a_list() {
        let sent = |request: &str| match plan("tequila_request", &args(&format!(r#"{{"request":{request}}}"#))) {
            Ok(Plan::Request { json, .. }) => Ok(args(&json)),
            Ok(other) => panic!("{request}: {other:?}"),
            Err(error) => Err(error),
        };
        for request in [
            r#"{"op":"export","format":"glb","path":"kettle.glb"}"#,
            r#"{"op":"reimport","node":3}"#,
            r#"{"op":"texture_set","path":"packs/oak","nodes":[4]}"#,
            r#"{"op":"substance","action":"open","path":"wear.sbsar"}"#,
            r#"{"op":"uv_layout","node":12,"path":"shell-uv.svg","size":4096}"#,
            r#"{"op":"studio_kit","piece":"softbox"}"#,
            r#"{"op":"library","action":"import","path":"packs/oak.tqmat"}"#,
            r#"{"op":"studio_preset","action":"export","preset":"mine","path":"mine.tqstudio"}"#,
            r#"{"op":"queue","network":true}"#,
            r#"{"op":"screenshot","path":"look.png"}"#,
            r#"{"op":"render","path":"quick.png","width":800,"height":600}"#,
            r#"{"op":"ui","uv_editor":true,"uv_fit":true}"#,
            r#"{"op":"plugin","action":"restart","plugin":"my.x"}"#,
        ] {
            let request = args(request);
            assert_eq!(sent(&json_write::to_string(&request)).as_ref(), Ok(&request), "as written");
            assert!(REQUESTS.contains(&request.get("op").and_then(Value::as_str).unwrap()));
        }
        // The connection's own words are not the model's to write.
        let marked = sent(r#"{"op":"export","format":"glb","path":"a.glb","id":7,"token":"t","agent":"mark"}"#).unwrap();
        assert!(marked.get("id").is_none() && marked.get("token").is_none() && marked.get("agent").is_none());

        for (request, says) in [
            (r#"{"op":"quit","discard":true}"#, "the person's to do"),
            (r#"{"op":"new","discard":true}"#, "the person's to do"),
            (r#"{"op":"open","path":"other.tequila","discard":true}"#, "the person's to do"),
            (r#"{"op":"assistant","answer":{"question":1,"allow":true}}"#, "cannot drive itself"),
            (r#"{"op":"livelink","message":{}}"#, "that application's"),
            (r#"{"op":"ping"}"#, "not a request this tool sends"),
            (r#"{"op":"something_new","path":"/x"}"#, "not a request this tool sends"),
            (r#"{"op":"material","action":"create","recipe":{}}"#, "tequila_material_create"),
            (r#"{"op":"query","what":"context"}"#, "tequila_read"),
            (r#"{"op":"query","what":"library"}"#, "tequila_library"),
            (r#"{"op":"batch","ops":[{"op":"quit"}]}"#, "a batch here may hold"),
            (r#"{"op":"batch","ops":[{"op":"export","format":"glb","path":"a.glb"}]}"#, "a batch here may hold"),
            (r#"{"path":"a.glb"}"#, "needs an \"op\""),
        ] {
            let refused = sent(request).unwrap_err();
            assert!(refused.contains(says), "{request}: {refused}");
        }
        assert!(plan("tequila_request", &args(r#"{"request":"export"}"#)).is_err());
        for kept_out in ["quit", "new", "open", "assistant", "livelink"] {
            assert!(!REQUESTS.contains(&kept_out) && !REQUEST_TOOLS.iter().any(|(op, _)| *op == kept_out), "{kept_out}");
        }

        // A tool's job is the tool's call: only what it passes on, passes.
        let queue = sent(r#"{"op":"save","path":"copy.tequila","token":"x"}"#).unwrap();
        assert_eq!((queue.get("op").and_then(Value::as_str), queue.get("token")), (Some("save"), None));
        assert_eq!(sent(r#"{"op":"help","command":"scene.rename"}"#).unwrap().get("command").and_then(Value::as_str), Some("scene.rename"));
        assert!(matches!(plan("tequila_request", &args(r#"{"request":{"op":"query","what":"nodes","filter":"strap"}}"#)).unwrap(), Plan::Request { shape: Shape::Nodes { .. }, .. }));
        assert!(matches!(plan("tequila_request", &args(r#"{"request":{"op":"guide","topic":"uv"}}"#)).unwrap(), Plan::Request { shape: Shape::Text, .. }));

        // A plugin's code written through a request asks the person again,
        // whatever the request says of asking; a run waits for its log.
        for asking in ["", r#","ask":false"#] {
            let write = sent(&format!(r#"{{"op":"plugin","action":"write","plugin":"my.x","files":{{"main.py":"print(1)"}}{asking}}}"#)).unwrap();
            assert_eq!(write.get("ask").and_then(Value::as_bool), Some(true), "{asking}");
        }
        let run = plan("tequila_request", &args(r#"{"request":{"op":"plugin","action":"run","plugin":"my.x","command":"go"}}"#)).unwrap();
        assert!(matches!(run, Plan::PluginRun { ref plugin, ref command, .. } if plugin == "my.x" && command == "go"), "{run:?}");
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
                "tequila_material_tint" => args(r##"{"material":2,"color":"#1f4d2b"}"##),
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
                "tequila_request" => args(r#"{"request":{"op":"export","format":"glb","path":"kettle.glb"}}"#),
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
        // Written as a raw request, each goes to its own tool or is told which.
        assert_eq!(text(&sent("tequila_request", r#"{"request":{"op":"improve","node":3,"subdivide":1}}"#), "op").as_deref(), Some("improve"));
        assert_eq!(text(&sent("tequila_request", r#"{"request":{"op":"model_units","node":3}}"#), "op").as_deref(), Some("model_units"));
        let told = plan("tequila_request", &args(r#"{"request":{"op":"graph","action":"read","material":2}}"#)).unwrap_err();
        assert!(told.contains("tequila_graph_edit"), "{told}");
        let told = plan("tequila_request", &args(r#"{"request":{"op":"uv","action":"islands","node":3}}"#)).unwrap_err();
        assert!(told.contains("tequila_uv_cut"), "{told}");
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
        // In another colour: as the preset goes on, or on the material
        // afterwards - a request of its own, not base_color.
        let tinted = request("tequila_library_apply", r##"{"name":"Leather Tan","nodes":[4],"color":"#1f4d2b"}"##);
        assert_eq!(tinted.get("color").and_then(Value::as_str), Some("#1f4d2b"));
        let Plan::Material { json: tint, picture } = plan("tequila_material_tint", &args(r##"{"material":7,"color":"#1f4d2b","path":"/elsewhere"}"##)).unwrap() else {
            panic!("a tint is a material request");
        };
        let tint = args(&tint);
        assert_eq!(
            (tint.get("op").and_then(Value::as_str), tint.get("action").and_then(Value::as_str), tint.get("material").and_then(Value::as_f64), tint.get("color").and_then(Value::as_str)),
            (Some("material"), Some("tint"), Some(7.0), Some("#1f4d2b"))
        );
        assert!(picture && tint.get("path").is_none());
        assert!(plan("tequila_material_tint", &args(r#"{"material":7}"#)).is_err(), "no colour, no tint");
        assert!(system().contains("tequila_material_tint"));
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

    /// The pictures the assistant asks for: the main window's, another
    /// window's, and a quick render, small unless it says otherwise.
    #[test]
    fn pictures_are_planned_with_their_sizes_and_windows() {
        assert_eq!(plan("tequila_screenshot", &args("{}")).unwrap(), Plan::Screenshot { viewport: true, window: None, settle: 12.0 });
        assert_eq!(
            plan("tequila_screenshot", &args(r#"{"window":"uv","settle":400}"#)).unwrap(),
            Plan::Screenshot { viewport: true, window: Some("uv".into()), settle: 60.0 }
        );
        assert_eq!(plan("tequila_render", &args("{}")).unwrap(), Plan::Render { width: 640, height: 400, samples: 32, wireframe: false });
        assert_eq!(
            plan("tequila_render", &args(r#"{"width":800,"height":600,"samples":64,"wireframe":true,"path":"/elsewhere.png"}"#)).unwrap(),
            Plan::Render { width: 800, height: 600, samples: 64, wireframe: true }
        );
        assert!(matches!(plan("tequila_jobs", &args("{}")).unwrap(), Plan::Request { ref json, .. } if json.contains("jobs")));
    }

    /// The person turns and zooms the view while the assistant works. One
    /// that found it closer after its material changes framed the model
    /// again each time: it is told whose a moved view is, and the tool that
    /// frames says what it is not for.
    #[test]
    fn a_moved_view_is_the_persons() {
        assert!(system().contains("The view is the person's as well") && system().contains("do not frame it or move it back"));
        let frame = all().into_iter().find(|tool| tool.name == "tequila_frame").expect("the tool that frames");
        assert!(frame.description.contains("not after a material edit"), "{}", frame.description);
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
