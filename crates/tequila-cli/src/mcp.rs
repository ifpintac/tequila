//! `tequila --mcp`: Tequila's tools for an AI app, over the Model Context
//! Protocol.
//!
//! An AI app on this computer - the Claude app, Cursor, VS Code with GitHub
//! Copilot, Codex, Gemini CLI, LM Studio and others - starts this program and talks JSON-RPC to it over standard input and
//! output; it turns tool calls into requests on the scripting channel of the
//! Tequila that is open, and hands screenshots and renders back as images,
//! so the AI can see its work. Everything it does shows in Tequila's
//! console, in amber, and every edit is one undo step.
//!
//! The same bridge as `python/tequila_mcp.py`, built in, so it needs no
//! Python: on a work PC that has none, the Claude app could not reach
//! Tequila at all, and drove the screen with its mouse instead. The tools
//! are that bridge's (`mcp_tools.json`, written from its `TOOLS`).
//!
//! `--restricted` is for anything reached from outside this computer: no raw
//! requests, no importing files, no saving or rendering to a path the caller
//! chooses.

use tequila_format::json::{self, Value};
use tequila_format::json_write::{self, Object};
use tequila_scripting::client::{one_line, Connection as Channel};
use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const TOOLS: &str = include_str!("mcp_tools.json");
const PROTOCOLS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const INSTRUCTIONS: &str = "Tequila is a product-rendering application open on this computer, and these tools are \
the way to work in it: use them, not screenshots of the desktop, mouse clicks or any computer-use control - they \
are exact, fast and undoable. Start with tequila_scene, then tequila_nodes to find parts by name, material or size. \
Materials may be shared between parts: unlink a part (material.unlink) before changing it alone. Use tequila_batch \
for several edits: they become one undo step, and if one fails none of them stay. Look at your work with \
tequila_screenshot (viewport=true for just the picture). For a material of your own, read tequila_material_guide, \
try recipes with tequila_material_preview, then tequila_material_create and refine with tequila_material_update. \
To extend Tequila itself - a panel, a batch job, an automation - write a plugin: tequila_plugin_guide, then \
tequila_plugin_write, tequila_plugin_enable (the person confirms), tequila_plugin_run and tequila_plugin_log. \
Every command is undoable, and the person watching sees each request in Tequila's console.";

/// Tools that reach outside the open scene: gone when restricted.
const UNRESTRICTED_ONLY: [&str; 10] = [
    "tequila_request",
    "tequila_import",
    "tequila_material_save",
    "tequila_studio_preset_save",
    "tequila_plugin_guide",
    "tequila_plugin_list",
    "tequila_plugin_write",
    "tequila_plugin_enable",
    "tequila_plugin_run",
    "tequila_plugin_log",
];
/// What applying or saving a studio preset reads.
const STUDIO_PRESET_FIELDS: [&str; 12] =
    ["preset", "front", "scale", "keep_lights", "keep_cameras", "keep_placement", "dress", "name", "family", "size", "description", "materials"];
/// What a batch may hold when restricted: nothing that writes a file.
const RESTRICTED_BATCH: [&str; 5] = ["apply", "frame", "look", "select", "ui"];

/// Serve until standard input closes. `discovery` names the channel file to
/// connect through; by default the one every Tequila writes.
pub fn run(discovery: Option<&str>, restricted: bool) -> Result<(), String> {
    let mut bridge = Bridge::new(discovery.map(PathBuf::from), restricted);
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let response = match json::parse(line.as_bytes()) {
            Ok(message) => handle(&mut bridge, &message),
            Err(_) => Some(failure(Value::Null, -32700, "Parse error")),
        };
        if let Some(response) = response {
            writeln!(stdout, "{}", one_line(&response)).map_err(|e| e.to_string())?;
            stdout.flush().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// The answer to one JSON-RPC message; `None` for a notification.
pub fn handle(bridge: &mut Bridge, message: &Value) -> Option<Value> {
    let id = message.get("id")?.clone();
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let empty = Value::Object(BTreeMap::new());
    let params = message.get("params").unwrap_or(&empty);
    let reply = |result: Value| {
        let mut out = Object::new();
        out.set("jsonrpc", "2.0").set("id", id.clone()).set("result", result);
        out.build()
    };
    Some(match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
            let version = PROTOCOLS.iter().find(|v| **v == asked).copied().unwrap_or(PROTOCOLS[0]);
            let mut info = Object::new();
            info.set("name", "tequila").set("version", env!("CARGO_PKG_VERSION"));
            let mut capabilities = Object::new();
            capabilities.set("tools", Value::Object(BTreeMap::new()));
            let mut out = Object::new();
            out.set("protocolVersion", version)
                .set("capabilities", capabilities.build())
                .set("serverInfo", info.build())
                .set("instructions", INSTRUCTIONS);
            reply(out.build())
        }
        "ping" => reply(Value::Object(BTreeMap::new())),
        "tools/list" => {
            let mut out = Object::new();
            out.set("tools", Value::Array(bridge.tools()));
            reply(out.build())
        }
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            if !bridge.tools().iter().any(|t| t.get("name").and_then(Value::as_str) == Some(name)) {
                return Some(failure(id, -32602, &format!("Unknown tool: {name}")));
            }
            let arguments = params.get("arguments").cloned().unwrap_or(Value::Object(BTreeMap::new()));
            let (content, is_error) = match bridge.call(name, &arguments) {
                Ok(content) => (content, false),
                // A refusal from the application is a result for the AI to
                // read and act on, not a protocol failure.
                Err(problem) => (text(&problem), true),
            };
            let mut out = Object::new();
            out.set("content", Value::Array(content)).set("isError", is_error);
            reply(out.build())
        }
        other => failure(id, -32601, &format!("Method not found: {other}")),
    })
}

fn failure(id: Value, code: i64, message: &str) -> Value {
    let mut error = Object::new();
    error.set("code", code as f64).set("message", message);
    let mut out = Object::new();
    out.set("jsonrpc", "2.0").set("id", id).set("error", error.build());
    out.build()
}

/// Tool calls in, channel requests out: one connection, made again when the
/// application restarts.
pub struct Bridge {
    discovery: PathBuf,
    restricted: bool,
    channel: Option<Channel>,
}

impl Bridge {
    pub fn new(discovery: Option<PathBuf>, restricted: bool) -> Bridge {
        let discovery = discovery.unwrap_or_else(tequila_scripting::server::discovery_path);
        Bridge { discovery, restricted, channel: None }
    }

    pub fn tools(&self) -> Vec<Value> {
        let all = json::parse(TOOLS.as_bytes()).expect("the shipped tool list is JSON");
        all.as_array()
            .iter()
            .filter(|tool| !self.restricted || !UNRESTRICTED_ONLY.contains(&tool.get("name").and_then(Value::as_str).unwrap_or("")))
            .map(|tool| {
                let name = tool.get("name").and_then(Value::as_str).unwrap_or("");
                if self.restricted && matches!(name, "tequila_save" | "tequila_queue") {
                    without_fields(tool, &["path", "folder", "filename"])
                } else {
                    tool.clone()
                }
            })
            .collect()
    }

    fn request(&mut self, op: &str, fields: Vec<(&str, Value)>) -> Result<Value, String> {
        if self.channel.is_none() {
            self.channel = Some(
                Channel::connect(&self.discovery)
                    .map_err(|e| format!("Tequila does not seem to be open (or its channel is off): {e}"))?,
            );
        }
        let channel = self.channel.as_mut().expect("connected above");
        let result = channel.request(op, fields);
        // A broken connection is made again next time; a refusal from the
        // application leaves it open. Nothing is retried either way.
        if result.as_ref().is_err_and(|e| e.starts_with("Connection ended")) {
            self.channel = None;
        }
        result
    }

    pub fn call(&mut self, name: &str, arguments: &Value) -> Result<Vec<Value>, String> {
        let get = |key: &str| arguments.get(key).cloned();
        if self.restricted {
            self.check_restricted(name, arguments)?;
        }
        match name {
            "tequila_scene" => self.request("query", vec![("what", "scene".into())]).map(|v| text_value(&v)),
            "tequila_nodes" => {
                let nodes = self.request("query", vec![("what", "nodes".into())])?;
                Ok(text_value(&slim_nodes(&nodes, arguments)))
            }
            "tequila_materials" => self.request("query", vec![("what", "materials".into())]).map(|v| text_value(&v)),
            "tequila_query" => {
                let what = get("what").ok_or("tequila_query needs `what`")?;
                self.request("query", vec![("what", what)]).map(|v| text_value(&v))
            }
            "tequila_apply" => {
                let command = get("command").ok_or("tequila_apply needs `command`")?;
                let applied = command.get("command").cloned().unwrap_or(Value::Null);
                self.request("apply", vec![("command", command)])?;
                let mut out = Object::new();
                out.set("ok", true).set("applied", applied);
                Ok(text_value(&out.build()))
            }
            "tequila_material_guide" => self.request("material", vec![("action", "guide".into())]).map(|v| text_value(&v)),
            "tequila_material_preview" => {
                let mut fields = vec![("action", Value::from("preview"))];
                for key in ["recipe", "material", "radius", "shape"] {
                    if let Some(value) = get(key) {
                        fields.push((key, value));
                    }
                }
                self.material_with_picture(fields, true)
            }
            "tequila_material_create" | "tequila_material_update" => {
                let action = if name == "tequila_material_create" { "create" } else { "update" };
                let mut fields = vec![("action", Value::from(action))];
                for key in ["recipe", "nodes", "material"] {
                    if let Some(value) = get(key) {
                        fields.push((key, value));
                    }
                }
                let picture = get("preview").and_then(|v| v.as_bool()).unwrap_or(true);
                self.material_with_picture(fields, picture)
            }
            "tequila_material_recipe" => {
                let material = get("material").ok_or("tequila_material_recipe needs `material`")?;
                self.request("material", vec![("action", "recipe".into()), ("material", material)]).map(|v| text_value(&v))
            }
            "tequila_material_save" => {
                let mut fields = vec![("action", Value::from("save"))];
                for key in ["material", "name", "category", "tags"] {
                    if let Some(value) = get(key) {
                        fields.push((key, value));
                    }
                }
                self.request("material", fields).map(|v| text_value(&v))
            }
            "tequila_plugin_guide" => self.request("plugin", vec![("action", "guide".into())]).map(|v| text_value(&v)),
            "tequila_plugin_list" => self.request("plugin", vec![("action", "list".into())]).map(|v| text_value(&v)),
            "tequila_plugin_write" | "tequila_plugin_enable" | "tequila_plugin_log" => {
                let action = name.trim_start_matches("tequila_plugin_");
                let mut fields = vec![("action", Value::from(action))];
                // The tool's `id` is the request's `plugin`: a request's own
                // `id` is its number.
                if let Some(id) = get("id") {
                    fields.push(("plugin", id));
                }
                for key in ["files", "lines"] {
                    if let Some(value) = get(key) {
                        fields.push((key, value));
                    }
                }
                self.request("plugin", fields).map(|v| text_value(&v))
            }
            "tequila_plugin_run" => {
                let id = get("id").ok_or("tequila_plugin_run needs `id`")?;
                let command = get("command").ok_or("tequila_plugin_run needs `command`")?;
                let wait = get("wait").and_then(|v| v.as_f64()).unwrap_or(10.0).clamp(0.0, 120.0);
                self.plugin_run(id, command, get("values"), Duration::from_secs_f64(wait)).map(|v| text_value(&v))
            }
            "tequila_batch" => {
                let ops = get("ops").ok_or("tequila_batch needs `ops`")?;
                let atomic = get("atomic").unwrap_or(Value::Bool(true));
                self.request("batch", vec![("ops", ops), ("atomic", atomic)]).map(|v| text_value(&v))
            }
            "tequila_undo" => self.request("undo", vec![]).map(|v| text_value(&v)),
            "tequila_redo" => self.request("redo", vec![]).map(|v| text_value(&v)),
            "tequila_select" => {
                let nodes = get("nodes").ok_or("tequila_select needs `nodes`")?;
                self.request("select", vec![("nodes", nodes.clone())])?;
                let mut out = Object::new();
                out.set("ok", true).set("selected", nodes);
                Ok(text_value(&out.build()))
            }
            "tequila_frame" => {
                let all = get("all").unwrap_or(Value::Bool(true));
                self.request("frame", vec![("all", all)]).map(|v| text_value(&v))
            }
            "tequila_look" => self.request("look", fields_of(arguments)).map(|v| text_value(&v)),
            "tequila_screenshot" => {
                let viewport = get("viewport").and_then(|v| v.as_bool()).unwrap_or(true);
                let path = temporary("tequila-mcp", "png");
                self.request("screenshot", vec![("path", path.to_string_lossy().into_owned().into()), ("viewport", viewport.into())])?;
                wait_for_png(&path, Duration::from_secs(30))?;
                image(&path, if viewport { "Tequila viewport" } else { "Tequila window" })
            }
            "tequila_render" => {
                let path = temporary("tequila-mcp-render", "png");
                let number = |key: &str, default: f64| Value::Number(get(key).and_then(|v| v.as_f64()).unwrap_or(default));
                let result = self.request(
                    "render",
                    vec![
                        ("path", path.to_string_lossy().into_owned().into()),
                        ("width", number("width", 640.0)),
                        ("height", number("height", 400.0)),
                        ("samples", number("samples", 32.0)),
                    ],
                )?;
                image(&path, &one_line(&result))
            }
            "tequila_queue" => self.request("queue", fields_of(arguments)).map(|v| text_value(&v)),
            "tequila_jobs" => self.request("query", vec![("what", "jobs".into())]).map(|v| text_value(&v)),
            "tequila_studio_presets" => self.request("studio_preset", vec![("action", "list".into())]).map(|v| text_value(&v)),
            "tequila_studio_preset_apply" | "tequila_studio_preset_save" => {
                let action = if name == "tequila_studio_preset_apply" { "apply" } else { "save" };
                let mut fields = vec![("action", Value::from(action))];
                for key in STUDIO_PRESET_FIELDS {
                    if let Some(value) = get(key) {
                        fields.push((key, value));
                    }
                }
                self.request("studio_preset", fields).map(|v| text_value(&v))
            }
            "tequila_studio_render_all" => self.request("studio_preset", vec![("action", "render_all".into())]).map(|v| text_value(&v)),
            "tequila_stop" => self.request("stop", vec![]).map(|v| text_value(&v)),
            "tequila_ui" => self.request("ui", fields_of(arguments)).map(|v| text_value(&v)),
            "tequila_import" => {
                let path = get("path").and_then(|v| v.as_str().map(absolute)).ok_or("tequila_import needs `path`")?;
                self.request("import", vec![("path", path.into())]).map(|v| text_value(&v))
            }
            "tequila_save" => {
                let fields = match get("path").and_then(|v| v.as_str().map(absolute)) {
                    Some(path) => vec![("path", path.into())],
                    None => vec![],
                };
                self.request("save", fields).map(|v| text_value(&v))
            }
            "tequila_request" => {
                let request = get("request").ok_or("tequila_request needs `request`")?;
                let op = request.get("op").and_then(Value::as_str).ok_or("the request needs an \"op\"")?.to_string();
                let fields = fields_of(&request).into_iter().filter(|(k, _)| !matches!(*k, "op" | "id" | "token")).collect();
                self.request(&op, fields).map(|v| text_value(&v))
            }
            other => Err(format!("Unknown tool: {other}")),
        }
    }

    /// Run a plugin's command and wait - up to `wait` - for it to finish,
    /// answering with what it logged meanwhile: its prints, `finished`, or
    /// its traceback, so the AI sees the outcome in one call.
    fn plugin_run(&mut self, id: Value, command: Value, values: Option<Value>, wait: Duration) -> Result<Value, String> {
        let total = |log: &Value| log.get("total").and_then(Value::as_f64).unwrap_or(0.0) as u64;
        let before = total(&self.request("plugin", vec![("action", "log".into()), ("plugin", id.clone()), ("lines", 1.0.into())])?);
        let mut fields = vec![("action", Value::from("run")), ("plugin", id.clone()), ("command", command.clone())];
        if let Some(values) = values {
            fields.push(("values", values));
        }
        let sent = self.request("plugin", fields)?;
        if sent.get("opened_panel").is_some() {
            return Ok(sent);
        }
        let finished = format!("finished `{}`", command.as_str().unwrap_or_default());
        let deadline = Instant::now() + wait;
        loop {
            let log = self.request("plugin", vec![("action", "log".into()), ("plugin", id.clone()), ("lines", 400.0.into())])?;
            let lines = log.get("lines").map(Value::as_array).unwrap_or(&[]);
            let new = (total(&log).saturating_sub(before) as usize).min(lines.len());
            let said: Vec<Value> = lines[lines.len() - new..].to_vec();
            let done = said.iter().any(|l| l.as_str() == Some(finished.as_str()));
            let stopped = !log.get("running").and_then(Value::as_bool).unwrap_or(false);
            if done || stopped || Instant::now() >= deadline {
                let mut out = Object::new();
                out.set("finished", done).set("log", Value::Array(said));
                if !done {
                    out.set("note", if stopped { "the plugin is not running; see its failure in tequila_plugin_list" } else { "still running; tequila_plugin_log shows more later" });
                }
                return Ok(out.build());
            }
            std::thread::sleep(Duration::from_millis(150));
        }
    }

    /// A material request, answered with its reply and - for a preview, or a
    /// create or update with `picture` - the material on the preview ball, so
    /// the AI sees what it made in the same answer.
    fn material_with_picture(&mut self, mut fields: Vec<(&str, Value)>, picture: bool) -> Result<Vec<Value>, String> {
        let action = fields.first().and_then(|(_, v)| v.as_str()).unwrap_or("").to_string();
        if action == "preview" {
            let path = temporary("tequila-mcp-material", "png");
            fields.push(("path", path.to_string_lossy().into_owned().into()));
            let shown = self.request("material", fields)?;
            return image(&path, &one_line(&shown));
        }
        let made = self.request("material", fields)?;
        let mut out = text_value(&made);
        if picture {
            if let Some(material) = made.get("material").cloned() {
                let path = temporary("tequila-mcp-material", "png");
                let shown = self.request("material", vec![("action", "preview".into()), ("material", material), ("path", path.to_string_lossy().into_owned().into())]);
                if shown.is_ok() {
                    out.extend(image(&path, "the material on a 5 cm ball")?);
                }
            }
        }
        Ok(out)
    }

    fn check_restricted(&self, name: &str, arguments: &Value) -> Result<(), String> {
        // Not listed is not enough: a client may call a tool by name.
        if UNRESTRICTED_ONLY.contains(&name) {
            return Err(format!("{name} is not available remotely"));
        }
        let check_command = |command: Option<&Value>| match command.and_then(|c| c.get("command")).and_then(Value::as_str) {
            Some("render.set_settings") => Err(
                "Render output settings are controlled locally; use tequila_queue for resolution, samples and denoising".to_string(),
            ),
            _ => Ok(()),
        };
        if name == "tequila_apply" {
            check_command(arguments.get("command"))?;
        }
        if matches!(name, "tequila_save" | "tequila_queue") {
            for field in ["path", "folder", "filename"] {
                if arguments.get(field).is_some() {
                    return Err(format!("{field} is not available remotely; the project and render settings decide where files go"));
                }
            }
        }
        if name == "tequila_batch" {
            for (i, op) in arguments.get("ops").map(Value::as_array).unwrap_or(&[]).iter().enumerate() {
                let kind = op.get("op").and_then(Value::as_str).unwrap_or("");
                if kind == "apply" {
                    check_command(op.get("command"))?;
                }
                if !RESTRICTED_BATCH.contains(&kind) {
                    return Err(format!(
                        "operation {} ({kind}) is not available remotely; a batch may hold {}",
                        i + 1,
                        RESTRICTED_BATCH.join(", ")
                    ));
                }
            }
        }
        Ok(())
    }
}

/// The answer's parts and groups, slimmed: id, name, kind, material and
/// visibility, with each one's size and centre rather than its corners, and
/// filtered as asked, so a model split into hundreds of parts reads.
fn slim_nodes(answer: &Value, arguments: &Value) -> Value {
    let parts_only = arguments.get("parts_only").and_then(Value::as_bool).unwrap_or(false);
    let word = arguments.get("filter").and_then(Value::as_str).map(str::to_lowercase);
    let mut slim = Vec::new();
    for node in answer.get("nodes").map(Value::as_array).unwrap_or(&[]) {
        let field = |key: &str| node.get(key).and_then(Value::as_str).unwrap_or("").to_lowercase();
        if parts_only && field("kind") != "part" {
            continue;
        }
        if let Some(word) = &word {
            if !field("name").contains(word) && !field("material_name").contains(word) {
                continue;
            }
        }
        let mut entry = BTreeMap::new();
        for key in ["id", "name", "kind", "material", "material_name", "visible"] {
            if let Some(value) = node.get(key) {
                entry.insert(key.to_string(), value.clone());
            }
        }
        if let Some(bounds) = node.get("bounds") {
            let round = |v: f64| Value::Number((v * 1e4).round() / 1e4);
            let list = |key: &str| bounds.get(key).map(Value::as_array).unwrap_or(&[]).iter().filter_map(Value::as_f64).collect::<Vec<_>>();
            entry.insert("size".into(), Value::Array(list("size").into_iter().map(round).collect()));
            let centre = list("min").into_iter().zip(list("max")).map(|(a, b)| round((a + b) / 2.0)).collect();
            entry.insert("center".into(), Value::Array(centre));
        }
        slim.push(Value::Object(entry));
    }
    let mut out = Object::new();
    out.set("count", slim.len()).set("nodes", Value::Array(slim));
    out.build()
}

fn fields_of(value: &Value) -> Vec<(&str, Value)> {
    match value {
        Value::Object(map) => map.iter().map(|(k, v)| (k.as_str(), v.clone())).collect(),
        _ => Vec::new(),
    }
}

fn without_fields(tool: &Value, fields: &[&str]) -> Value {
    let mut tool = tool.clone();
    if let Value::Object(map) = &mut tool {
        if let Some(Value::Object(schema)) = map.get_mut("inputSchema") {
            if let Some(Value::Object(properties)) = schema.get_mut("properties") {
                for field in fields {
                    properties.remove(*field);
                }
            }
        }
    }
    tool
}

fn text(message: &str) -> Vec<Value> {
    let mut out = Object::new();
    out.set("type", "text").set("text", message);
    vec![out.build()]
}

fn text_value(value: &Value) -> Vec<Value> {
    match value {
        Value::String(s) => text(s),
        other => text(json_write::to_string(other).trim_end()),
    }
}

/// A PNG as an image for the AI, with a caption; the file is removed.
fn image(path: &Path, caption: &str) -> Result<Vec<Value>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let _ = std::fs::remove_file(path);
    let mut picture = Object::new();
    picture.set("type", "image").set("data", base64(&bytes)).set("mimeType", "image/png");
    let mut out = vec![picture.build()];
    out.extend(text(caption));
    Ok(out)
}

/// Wait until a PNG the application is writing has its end chunk.
fn wait_for_png(path: &Path, timeout: Duration) -> Result<(), String> {
    const END: [u8; 12] = [0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82];
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(bytes) = std::fs::read(path) {
            if bytes.ends_with(&END) {
                return Ok(());
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err("the screenshot has not completed; look in Tequila's console for why".into())
}

fn temporary(stem: &str, extension: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    std::env::temp_dir().join(format!("{stem}-{}-{nanos}.{extension}", std::process::id()))
}

fn absolute(path: &str) -> String {
    let path = Path::new(path);
    let full = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(path) };
    full.to_string_lossy().into_owned()
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Python bridge offers the same tools as this one: an AI app set up
    /// with either sees the same Tequila.
    #[test]
    fn the_python_bridge_offers_the_same_tools() {
        let python = include_str!("../../../python/tequila_mcp.py");
        let mut theirs: Vec<&str> = python
            .split("tool('")
            .skip(1)
            .filter_map(|rest| rest.split('\'').next())
            .filter(|name| name.starts_with("tequila_"))
            .collect();
        let tools = json::parse(TOOLS.as_bytes()).unwrap();
        let mut ours: Vec<&str> = tools.as_array().iter().filter_map(|t| t.get("name").and_then(Value::as_str)).collect();
        theirs.sort_unstable();
        ours.sort_unstable();
        assert_eq!(ours, theirs);
    }

    fn call(bridge: &mut Bridge, message: &str) -> Value {
        handle(bridge, &json::parse(message.as_bytes()).unwrap()).expect("an answer")
    }

    /// The handshake names the protocol asked for, offers tools and tells
    /// the AI to use them rather than driving the screen.
    #[test]
    fn it_introduces_itself_and_lists_the_tools() {
        let mut bridge = Bridge::new(Some(std::env::temp_dir().join("tequila-mcp-nowhere.json")), false);
        let hello = call(&mut bridge, r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#);
        let result = hello.get("result").unwrap();
        assert_eq!(result.get("protocolVersion").and_then(Value::as_str), Some("2025-03-26"));
        assert!(result.get("instructions").and_then(Value::as_str).unwrap().contains("not screenshots of the desktop"));
        let list = call(&mut bridge, r#"{"jsonrpc":"2.0","id":"a","method":"tools/list"}"#);
        let tools = list.get("result").unwrap().get("tools").unwrap().as_array();
        assert_eq!(tools.len(), 36);
        assert!(tools.iter().any(|t| t.get("name").and_then(Value::as_str) == Some("tequila_material_create")), "the material tools are listed");
        assert_eq!(list.get("id"), Some(&Value::String("a".into())), "the id comes back as it was sent");
        // A notification gets no answer; an unknown method an error.
        assert!(handle(&mut bridge, &json::parse(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).unwrap()).is_none());
        assert!(call(&mut bridge, r#"{"jsonrpc":"2.0","id":2,"method":"nope"}"#).get("error").is_some());
        // Restricted, the tools that reach outside the scene are gone.
        let restricted = Bridge::new(None, true).tools();
        assert_eq!(restricted.len(), 26);
        assert!(!restricted.iter().any(|t| t.get("name").and_then(Value::as_str) == Some("tequila_import")));
        assert!(!restricted.iter().any(|t| t.get("name").and_then(Value::as_str).is_some_and(|n| n.starts_with("tequila_plugin"))));
    }

    /// Restricted, a tool that is not listed cannot be called by its name
    /// either: the protocol does not know it, and the bridge itself refuses
    /// it too, before anything reaches Tequila.
    #[test]
    fn restricted_hidden_tools_cannot_be_called_by_name() {
        let mut bridge = Bridge::new(Some(std::env::temp_dir().join("tequila-mcp-nowhere.json")), true);
        for (name, arguments) in [
            ("tequila_request", r#"{"request":{"op":"quit","discard":true}}"#),
            ("tequila_import", r#"{"path":"C:/x.glb"}"#),
            ("tequila_plugin_write", r#"{"id":"x","files":{"main.py":""}}"#),
        ] {
            let message = format!(r#"{{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{{"name":"{name}","arguments":{arguments}}}}}"#);
            let answer = call(&mut bridge, &message);
            assert_eq!(answer.get("error").and_then(|e| e.get("code")).and_then(Value::as_f64), Some(-32602.0), "{name}");
            let refused = bridge.call(name, &json::parse(arguments.as_bytes()).unwrap()).unwrap_err();
            assert!(refused.contains("not available remotely"), "{name}: {refused}");
        }
    }

    /// With no Tequila open, a tool call says so as a result the AI can
    /// read, not as a protocol error.
    #[test]
    fn with_tequila_closed_a_call_says_so() {
        let mut bridge = Bridge::new(Some(std::env::temp_dir().join("tequila-mcp-nowhere.json")), false);
        let answer = call(&mut bridge, r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"tequila_scene"}}"#);
        let result = answer.get("result").unwrap();
        assert_eq!(result.get("isError").and_then(Value::as_bool), Some(true));
        let said = result.get("content").unwrap().as_array()[0].get("text").and_then(Value::as_str).unwrap();
        assert!(said.contains("does not seem to be open"), "{said}");
    }

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn a_message_is_one_line() {
        let mut nested = Object::new();
        nested.set("a", Value::Array(vec![Value::String("x\ny".into()), Value::Bool(true)]));
        let line = one_line(&nested.build());
        assert!(!line.contains('\n'), "{line}");
        assert_eq!(json::parse(line.as_bytes()).unwrap().get("a").unwrap().as_array()[0].as_str(), Some("x\ny"));
    }
}
