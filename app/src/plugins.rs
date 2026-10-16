//! Plugins: folders that add to Tequila, for people who want to improve it.
//!
//! A plugin is a folder with a `plugin.json` in it. It can bring
//!
//! - **materials**: a folder of presets or `.fsmat` packages, shown in the
//!   library under the plugin's name, read-only - content alone, no code;
//! - **commands**: entries in the Plugins menu;
//! - **panels**: small windows of fields - numbers, text, toggles, choices,
//!   colours, buttons - declared in the manifest and drawn by Tequila;
//! - **events**: the selection changing, the project being saved, a render
//!   finishing.
//!
//! Code is Python, run as a process of its own by `python/tequila_plugin.py`,
//! the host: it loads the plugin's `main` file, connects back to this window
//! through the scripting channel - the same API every script uses, so a
//! plugin can do what a script can and nothing a script cannot - and takes
//! commands and events from Tequila as JSON lines on its stdin, answering
//! on its stdout. A plugin that crashes takes its own process down, not the
//! application; what it does to the scene is in the Scripting tab's log,
//! with undo, like anything else.
//!
//! Nothing runs that the person did not turn on: plugins start disabled,
//! and turning on one that runs code asks first.

use tequila_format::json::{self, Value};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};

/// The plugin API this build speaks: a manifest asking for a newer one is
/// refused with a reason rather than half-working.
pub const API: u32 = 1;

/// What a plugin is, from its `plugin.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    /// Unique: letters, digits, `.`, `-` and `_`, like `com.example.colourways`.
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    /// The plugin API it was written for.
    pub api: u32,
    /// Its Python file, relative to the folder, if it has code.
    pub main: Option<String>,
    /// Its folder of materials, relative to the folder, if it brings any.
    pub materials: Option<String>,
    pub commands: Vec<CommandSpec>,
    pub panels: Vec<PanelSpec>,
    /// Which of [`EVENTS`] it wants told about.
    pub events: Vec<String>,
}

/// The events a plugin can ask for.
pub const EVENTS: &[&str] = &["selection", "saved", "render_finished", "imported", "opened", "panel_opened"];

/// An entry in the Plugins menu: runs the command, or opens a panel.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandSpec {
    pub id: String,
    pub title: String,
    pub description: String,
    /// Open this panel rather than running anything.
    pub panel: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PanelSpec {
    pub id: String,
    pub title: String,
    pub fields: Vec<Field>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub id: String,
    pub label: String,
    pub kind: FieldKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FieldKind {
    Number { min: f64, max: f64, step: f64, default: f64 },
    /// Text, on one line or - with `lines` over 1 - in a box that tall.
    Text { default: String, lines: usize },
    Toggle { default: bool },
    Choice { options: Vec<String>, default: String },
    /// sRGB, 0 to 1, as a swatch shows it.
    Colour { default: [f32; 3] },
    /// Runs `command` with the panel's values.
    Button { command: String },
    /// Text the plugin can change as it works.
    Label { text: String },
    /// A file or folder on this computer, typed or chosen with a Browse
    /// button: a file to open (`save` false) or write (`save` true), of
    /// `extensions` when given, or a `folder`.
    Path { folder: bool, save: bool, extensions: Vec<String>, default: String },
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(|v| v.as_str()).map(str::to_string)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 80 && !id.starts_with('.') && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// Read a plugin's manifest, or say what is wrong with it.
pub fn parse_manifest(source: &str) -> Result<Manifest, String> {
    let value = json::parse(source.as_bytes()).map_err(|e| format!("plugin.json is not JSON: {e}"))?;
    let id = text(&value, "id").ok_or("plugin.json has no `id`")?;
    if !valid_id(&id) {
        return Err(format!("`{id}` is not a plugin id: letters, digits, `.`, `-` and `_` only"));
    }
    let name = text(&value, "name").ok_or("plugin.json has no `name`")?;
    let api = value.get("tequila").or_else(|| value.get("freeshot")).and_then(|v| v.as_f32()).map(|v| v as u32).unwrap_or(1);
    if api > API {
        return Err(format!("`{name}` is for a newer Tequila (plugin API {api}; this one speaks {API})"));
    }
    let relative = |key: &str| -> Result<Option<String>, String> {
        match text(&value, key) {
            None => Ok(None),
            Some(path) => {
                let normal = path.replace('\\', "/");
                if Path::new(&path).is_absolute() || normal.starts_with('/') || normal.split('/').any(|p| p == ".." || p.contains(':')) {
                    Err(format!("`{key}` must be a path inside the plugin's folder, not {path}"))
                } else {
                    Ok(Some(normal))
                }
            }
        }
    };
    let main = relative("main")?;
    let materials = relative("materials")?;

    let mut panels = Vec::new();
    for panel in value.get("panels").map(|v| v.as_array().to_vec()).unwrap_or_default() {
        let id = text(&panel, "id").ok_or("a panel has no `id`")?;
        let title = text(&panel, "title").unwrap_or_else(|| id.clone());
        let mut fields = Vec::new();
        for field in panel.get("fields").map(|v| v.as_array().to_vec()).unwrap_or_default() {
            let fid = text(&field, "id").ok_or_else(|| format!("a field of panel `{id}` has no `id`"))?;
            let label = text(&field, "label").unwrap_or_else(|| fid.clone());
            let kind = match text(&field, "type").as_deref() {
                Some("number") => {
                    let number = |key: &str, or: f64| field.get(key).and_then(|v| v.as_f32()).map(f64::from).unwrap_or(or);
                    let (min, max) = (number("min", 0.0), number("max", 1.0));
                    FieldKind::Number { min, max, step: number("step", 0.0), default: number("default", min).clamp(min.min(max), max.max(min)) }
                }
                Some("text") => FieldKind::Text {
                    default: text(&field, "default").unwrap_or_default(),
                    lines: field.get("lines").and_then(|v| v.as_f64()).map_or(1, |n| n.clamp(1.0, 20.0) as usize),
                },
                Some("toggle") => FieldKind::Toggle { default: field.get("default").and_then(|v| v.as_bool()).unwrap_or(false) },
                Some("choice") => {
                    let options: Vec<String> = field.get("options").map(|v| v.as_array().iter().filter_map(|o| o.as_str().map(str::to_string)).collect()).unwrap_or_default();
                    if options.is_empty() {
                        return Err(format!("choice `{fid}` has no options"));
                    }
                    let default = text(&field, "default").filter(|d| options.contains(d)).unwrap_or_else(|| options[0].clone());
                    FieldKind::Choice { options, default }
                }
                Some("colour") | Some("color") => {
                    let rgb: Vec<f32> = field.get("default").map(|v| v.as_array().iter().filter_map(|c| c.as_f32()).collect()).unwrap_or_default();
                    FieldKind::Colour { default: if rgb.len() == 3 { [rgb[0], rgb[1], rgb[2]] } else { [0.8, 0.8, 0.8] } }
                }
                Some("button") => FieldKind::Button { command: text(&field, "command").ok_or_else(|| format!("button `{fid}` runs no `command`"))? },
                Some("label") => FieldKind::Label { text: text(&field, "text").unwrap_or_default() },
                Some(kind @ ("file" | "folder")) => FieldKind::Path {
                    folder: kind == "folder",
                    save: field.get("save").and_then(|v| v.as_bool()).unwrap_or(false),
                    extensions: field
                        .get("extensions")
                        .map(|v| v.as_array().iter().filter_map(|e| e.as_str().map(|e| e.trim_start_matches('.').to_string())).collect())
                        .unwrap_or_default(),
                    default: text(&field, "default").unwrap_or_default(),
                },
                other => return Err(format!(
                    "field `{fid}` has a type this does not know: {other:?}; the types are number, text, toggle, choice, colour, file, folder, label and button"
                )),
            };
            fields.push(Field { id: fid, label, kind });
        }
        panels.push(PanelSpec { id, title, fields });
    }

    let mut commands = Vec::new();
    for command in value.get("commands").map(|v| v.as_array().to_vec()).unwrap_or_default() {
        let id = text(&command, "id").ok_or("a command has no `id`")?;
        let panel = text(&command, "panel");
        if let Some(panel) = &panel {
            if !panels.iter().any(|p| &p.id == panel) {
                return Err(format!("command `{id}` opens panel `{panel}`, which the plugin does not declare"));
            }
        }
        commands.push(CommandSpec {
            title: text(&command, "title").unwrap_or_else(|| id.clone()),
            description: text(&command, "description").unwrap_or_default(),
            id,
            panel,
        });
    }
    let events: Vec<String> = value.get("events").map(|v| v.as_array().iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default();
    if let Some(unknown) = events.iter().find(|e| !EVENTS.contains(&e.as_str())) {
        return Err(format!("`{unknown}` is not an event; the events are {}", EVENTS.join(", ")));
    }
    let runs_code = commands.iter().any(|c| c.panel.is_none())
        || panels.iter().any(|p| p.fields.iter().any(|f| matches!(f.kind, FieldKind::Button { .. })))
        || !events.is_empty();
    if runs_code && main.is_none() {
        return Err("it has commands, buttons or events but no `main` Python file to run them".into());
    }
    Ok(Manifest {
        id,
        name,
        version: text(&value, "version").unwrap_or_else(|| "0".into()),
        author: text(&value, "author").unwrap_or_default(),
        description: text(&value, "description").unwrap_or_default(),
        api,
        main,
        materials,
        commands,
        panels,
        events,
    })
}

/// Where a plugin came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Shipped with Tequila, as an example. Read-only.
    Bundled,
    /// Installed by the person, in their plugins folder.
    User,
}

#[derive(Clone, Debug)]
pub struct Plugin {
    pub manifest: Manifest,
    pub folder: PathBuf,
    pub origin: Origin,
}

impl Plugin {
    pub fn runs_code(&self) -> bool {
        self.manifest.main.is_some()
    }

    /// Its materials folder, if it has one and it is there.
    pub fn materials_folder(&self) -> Option<PathBuf> {
        self.manifest.materials.as_ref().map(|m| self.folder.join(m)).filter(|p| p.is_dir())
    }
}

/// The user's plugins folder, beside their library.
pub fn user_directory() -> PathBuf {
    tequila_format::library::user_directory().with_file_name("plugins")
}

/// Where the plugin `id` keeps its settings: `plugin-data/<id>` beside the
/// plugins folder. Its code makes it when it first writes there.
pub fn data_directory(id: &str) -> PathBuf {
    user_directory().with_file_name("plugin-data").join(id)
}

/// The plugins shipped with Tequila: `plugins` beside the executable once
/// packaged, `plugins/examples` in the checkout it was built from.
pub fn bundled_directory() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        let mut candidate = exe;
        for _ in 0..6 {
            if !candidate.pop() {
                break;
            }
            for path in [candidate.join("plugins"), candidate.join("Resources").join("plugins"), candidate.join("plugins").join("examples")] {
                if path.join(".bundled").is_file() {
                    return Some(path);
                }
            }
        }
    }
    let built_from = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("plugins").join("examples");
    built_from.is_dir().then_some(built_from)
}

/// Every plugin in `folders`, each a folder of plugin folders; problems with
/// single plugins are reported rather than hiding the rest. A plugin id
/// found twice keeps the user's copy.
pub fn discover(folders: &[(PathBuf, Origin)]) -> (Vec<Plugin>, Vec<String>) {
    let mut found: Vec<Plugin> = Vec::new();
    let mut problems = Vec::new();
    for (folder, origin) in folders {
        let Ok(listing) = std::fs::read_dir(folder) else { continue };
        let mut dirs: Vec<PathBuf> = listing.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.join("plugin.json").is_file()).collect();
        dirs.sort();
        for dir in dirs {
            match std::fs::read_to_string(dir.join("plugin.json")).map_err(|e| e.to_string()).and_then(|t| parse_manifest(&t)) {
                Ok(manifest) => {
                    if let Some(existing) = found.iter_mut().find(|p| p.manifest.id == manifest.id) {
                        if *origin == Origin::User {
                            *existing = Plugin { manifest, folder: dir, origin: *origin };
                        }
                    } else {
                        found.push(Plugin { manifest, folder: dir, origin: *origin });
                    }
                }
                Err(error) => problems.push(format!("{}: {error}", dir.display())),
            }
        }
    }
    found.sort_by(|a, b| a.manifest.name.to_lowercase().cmp(&b.manifest.name.to_lowercase()));
    (found, problems)
}

/// Install a plugin from a ZIP into `into`: its `plugin.json` at the top or
/// in one folder, every member kept inside the new folder. A plugin already
/// there is replaced only with `replace`. The folder it went to.
pub fn install_zip(zip: &Path, into: &Path, replace: bool) -> Result<PathBuf, String> {
    let bytes = std::fs::read(zip).map_err(|e| format!("{}: {e}", zip.display()))?;
    let archive = tequila_format::zip::ZipArchive::read(&bytes).map_err(|e| format!("{} is not a plugin ZIP: {e:?}", zip.display()))?;
    let manifest_name = archive
        .names()
        .iter()
        .filter(|n| n.ends_with("plugin.json") && n.matches('/').count() <= 1)
        .min_by_key(|n| n.len())
        .cloned()
        .ok_or_else(|| format!("{} holds no plugin.json", zip.display()))?;
    let prefix = manifest_name.trim_end_matches("plugin.json").to_string();
    let manifest = parse_manifest(&String::from_utf8_lossy(archive.get(&manifest_name).unwrap_or_default()))?;
    let target = into.join(&manifest.id);
    if target.exists() {
        if !replace {
            return Err(format!("`{}` is already installed", manifest.name));
        }
        std::fs::remove_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
    }
    for name in archive.names() {
        let Some(inside) = name.strip_prefix(&prefix) else { continue };
        if inside.is_empty() || name.ends_with('/') {
            continue;
        }
        // The reader refuses `..` and absolute names; a drive letter too.
        if inside.split('/').any(|p| p == ".." || p.contains(':')) {
            return Err(format!("{} names a file outside itself: {name}", zip.display()));
        }
        let path = target.join(inside);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&path, archive.get(name).unwrap_or_default()).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(target)
}

/// A new plugin to start from, in `into`: a manifest with a command, a
/// panel and an event, and the Python that answers them - working as it
/// stands, so the first thing a person sees is it doing something.
pub fn create_from_template(into: &Path, name: &str) -> Result<PathBuf, String> {
    let stem = tequila_format::library::file_stem(name).replace('_', "-");
    let id = format!("my.{stem}");
    let folder = into.join(&id);
    if folder.exists() {
        return Err(format!("{} already exists", folder.display()));
    }
    std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let manifest = TEMPLATE_MANIFEST.replace("{id}", &id).replace("{name}", &name.replace('"', "'"));
    std::fs::write(folder.join("plugin.json"), manifest).map_err(|e| e.to_string())?;
    std::fs::write(folder.join("main.py"), TEMPLATE_MAIN.replace("{name}", name)).map_err(|e| e.to_string())?;
    Ok(folder)
}

const TEMPLATE_MANIFEST: &str = r#"{
  "id": "{id}",
  "name": "{name}",
  "version": "0.1.0",
  "author": "",
  "description": "A plugin of my own.",
  "tequila": 1,
  "main": "main.py",
  "commands": [
    { "id": "count", "title": "Count the selected parts" },
    { "id": "open", "title": "{name}…", "panel": "main" }
  ],
  "panels": [
    {
      "id": "main",
      "title": "{name}",
      "fields": [
        { "id": "prefix", "type": "text", "label": "Rename to", "default": "Part" },
        { "id": "rename", "type": "button", "label": "Rename the selected parts", "command": "rename" },
        { "id": "selected", "type": "label", "text": "Select some parts." }
      ]
    }
  ],
  "events": ["selection"]
}
"#;

const TEMPLATE_MAIN: &str = r#""""{name}: a Tequila plugin.

`fs` is a connected tequila.Client - the whole scripting API
(docs/scripting-api.md). Change this file, then click Restart on the plugin
in the Plugins window to load it. print() goes to the plugin's log.
"""
import tequila_plugin as plugin


@plugin.command('count')
def count(fs, values):
    selected = fs.query('selection')['nodes']
    plugin.status(f'{len(selected)} part(s) selected')


@plugin.command('rename')
def rename(fs, values):
    prefix = values.get('prefix') or 'Part'
    selected = fs.query('selection')['nodes']
    with fs.batch() as batch:  # one undo step for all of them
        for number, node in enumerate(selected, start=1):
            batch.commands.scene.rename(node=node, name=f'{prefix} {number}')
    plugin.status(f'Renamed {len(selected)} part(s)')


@plugin.on('selection')
def selection(fs, data):
    count = len(data.get('nodes', []))
    plugin.set_panel('main', selected=f'{count} part(s) selected.' if count else 'Select some parts.')
"#;

/// The files a plugin may be written as: code, its manifest, presets and
/// notes. Text only, so what is written can be read before it is run.
const WRITABLE: [&str; 8] = ["py", "json", "md", "txt", "csv", "toml", "ini", "cfg"];
/// The most a written file may hold.
const WRITABLE_SIZE: usize = 1 << 20;

/// Write `files` (relative path, text) into the plugin `id`'s `folder` -
/// a new plugin, or new versions of an existing one's files - for a script
/// or an AI app making a plugin. Every path stays inside the plugin's
/// folder, every file is text of a known kind, and the manifest, written or
/// already there, must read and name `id`: nothing is written otherwise.
pub fn write_files(folder: &Path, id: &str, files: &[(String, String)]) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err(format!("`{id}` is not a plugin id: letters, digits, `.`, `-` and `_`, like `my.colourways`"));
    }
    if files.is_empty() {
        return Err("no files to write".into());
    }
    let folder = folder.to_path_buf();
    let mut checked = Vec::with_capacity(files.len());
    for (path, text) in files {
        let normal = path.replace('\\', "/");
        let parts: Vec<&str> = normal.split('/').filter(|p| !p.is_empty()).collect();
        if parts.is_empty() || Path::new(path).is_absolute() || normal.starts_with('/') || parts.iter().any(|p| *p == ".." || *p == "." || p.contains(':')) {
            return Err(format!("`{path}` is not a path inside the plugin's folder"));
        }
        let extension = parts.last().and_then(|name| name.rsplit_once('.')).map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        if !WRITABLE.contains(&extension.as_str()) {
            return Err(format!("`{path}`: a plugin's files are written as text: {}", WRITABLE.join(", ")));
        }
        if text.len() > WRITABLE_SIZE {
            return Err(format!("`{path}` is over a megabyte"));
        }
        checked.push((parts.join("/"), text));
    }
    let manifest = match checked.iter().find(|(p, _)| p == "plugin.json") {
        Some((_, text)) => (*text).clone(),
        None => std::fs::read_to_string(folder.join("plugin.json")).map_err(|_| "a new plugin needs its plugin.json".to_string())?,
    };
    let parsed = parse_manifest(&manifest).map_err(|e| format!("plugin.json: {e}"))?;
    if parsed.id != id {
        return Err(format!("plugin.json names the plugin `{}`, not `{id}`", parsed.id));
    }
    if let Some(main) = &parsed.main {
        let present = checked.iter().any(|(p, _)| p == main) || folder.join(main).is_file();
        if !present {
            return Err(format!("plugin.json runs `{main}`, which is not written and not there"));
        }
    }
    for (path, text) in checked {
        let target = folder.join(&path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&target, text).map_err(|e| format!("{}: {e}", target.display()))?;
    }
    Ok(folder)
}

/// How to write a plugin: the reference (`docs/plugins.md`), what this
/// build accepts, the template, and an example - for an AI app making one,
/// in one answer.
pub fn guide() -> Value {
    use tequila_format::json_write::Object;
    let mut example = Object::new();
    example
        .set("plugin.json", TEMPLATE_MANIFEST.replace("{id}", "my.example").replace("{name}", "Example"))
        .set("main.py", TEMPLATE_MAIN.replace("{name}", "Example"));
    let mut out = Object::new();
    out.set("reference", include_str!("../../docs/plugins.md"))
        .set("api", API as f64)
        .set("events", Value::Array(EVENTS.iter().map(|e| Value::String((*e).into())).collect()))
        .set("field_types", Value::Array(["number", "text", "toggle", "choice", "colour", "file", "folder", "label", "button"].iter().map(|t| Value::String((*t).into())).collect()))
        .set("writable_files", Value::Array(WRITABLE.iter().map(|e| Value::String(format!(".{e}"))).collect()))
        .set("template", example.build())
        .set(
            "workflow",
            "Write plugin.json and main.py (tequila_plugin_write, or the plugin operation's write with `plugin` naming it; its id is the folder, e.g. my.batch-renamer). Ask the person to turn it \
             on with plugin enable - they confirm in Tequila, as it runs Python as them. Try a command with plugin run, read \
             plugin log for its prints and tracebacks, fix and write again: a running plugin restarts with the new code.",
        );
    out.build()
}

/// Remove a user plugin's folder.
pub fn remove(plugin: &Plugin) -> Result<(), String> {
    if plugin.origin != Origin::User {
        return Err(format!("`{}` ships with Tequila and cannot be removed", plugin.manifest.name));
    }
    std::fs::remove_dir_all(&plugin.folder).map_err(|e| format!("{}: {e}", plugin.folder.display()))
}

/// Which plugins are on, kept beside the plugins folder.
pub fn settings_path() -> PathBuf {
    user_directory().with_file_name("plugins.json")
}

pub fn read_enabled(path: &Path) -> BTreeSet<String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| json::parse(t.as_bytes()).ok())
        .map(|v| v.get("enabled").map(|e| e.as_array().iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default())
        .unwrap_or_default()
}

pub fn write_enabled(path: &Path, enabled: &BTreeSet<String>) -> Result<(), String> {
    let mut object = tequila_format::json_write::Object::new();
    object.set("enabled", enabled.iter().map(|s| Value::String(s.clone())).collect::<Vec<Value>>());
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(path, tequila_format::json_write::to_string(&object.build())).map_err(|e| format!("{}: {e}", path.display()))
}

/// How to run Python: `TEQUILA_PYTHON` if set, else the first of the usual
/// names that answers `--version` with Python 3. Asked once.
pub fn python() -> Option<Vec<String>> {
    static FOUND: std::sync::OnceLock<Option<Vec<String>>> = std::sync::OnceLock::new();
    FOUND
        .get_or_init(|| {
            let mut candidates: Vec<Vec<String>> = Vec::new();
            if let Some(explicit) = std::env::var_os("TEQUILA_PYTHON") {
                candidates.push(vec![explicit.to_string_lossy().into_owned()]);
            }
            if cfg!(windows) {
                candidates.push(vec!["py".into(), "-3".into()]);
            }
            candidates.push(vec!["python3".into()]);
            candidates.push(vec!["python".into()]);
            candidates.into_iter().find(|c| {
                let mut command = std::process::Command::new(&c[0]);
                command.args(&c[1..]).arg("--version");
                quiet(&mut command);
                command.output().is_ok_and(|o| o.status.success() && String::from_utf8_lossy(&[o.stdout, o.stderr].concat()).contains("Python 3"))
            })
        })
        .clone()
}

/// No console window for a child process on Windows: a plugin is not a
/// window of its own.
fn quiet(command: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

/// The host script and the folder of the `tequila` module it imports.
pub fn host_script() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        let mut candidate = exe;
        for _ in 0..6 {
            if !candidate.pop() {
                break;
            }
            for path in [candidate.join("python").join("tequila_plugin.py"), candidate.join("Resources").join("python").join("tequila_plugin.py")] {
                if path.is_file() {
                    return Some(path);
                }
            }
        }
    }
    let built_from = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("python").join("tequila_plugin.py");
    built_from.is_file().then_some(built_from)
}

/// What a running plugin said.
#[derive(Clone, Debug, PartialEq)]
pub enum Said {
    /// Up, with the commands and events its code registered.
    Ready,
    /// A message for the person: a toast.
    Status(String),
    /// Field values for one of its panels.
    Panel { panel: String, values: Vec<(String, Value)> },
    /// A line for its log: print(), a warning.
    Log(String),
    /// A command or an event handler failed: the traceback.
    Error(String),
    /// A command (named) or an event handler finished.
    Done(Option<String>),
    /// The process ended, with its exit code if it had one.
    Exited(Option<i32>),
}

fn read_said(line: &str) -> Said {
    let Ok(value) = json::parse(line.as_bytes()) else {
        return Said::Log(line.to_string());
    };
    match value.get("type").and_then(|t| t.as_str()) {
        Some("ready") => Said::Ready,
        Some("status") => Said::Status(text(&value, "text").unwrap_or_default()),
        Some("error") => Said::Error(text(&value, "text").unwrap_or_default()),
        Some("log") => Said::Log(text(&value, "text").unwrap_or_default()),
        Some("done") => Said::Done(text(&value, "command")),
        Some("panel") => Said::Panel {
            panel: text(&value, "panel").unwrap_or_default(),
            values: match value.get("values") {
                Some(Value::Object(fields)) => fields.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                _ => Vec::new(),
            },
        },
        _ => Said::Log(line.to_string()),
    }
}

/// A plugin's code, running.
pub struct Host {
    child: std::process::Child,
    /// Lines for the plugin, written to its input by a thread of their own.
    /// Written from the window's thread, a plugin busy in a handler - or
    /// waiting on a request only the window's thread could answer - let its
    /// input pipe fill, and the window stopped with it.
    to_plugin: Option<std::sync::mpsc::SyncSender<String>>,
    pub said: Receiver<Said>,
}

impl Host {
    /// Start `plugin`'s code, connected to the channel recorded at
    /// `discovery` - this window's, whose process is `pid` - or to nothing,
    /// for a test, when `discovery` is `None`.
    pub fn start(plugin: &Plugin, discovery: Option<(&Path, u32)>) -> Result<Host, String> {
        let python = python().ok_or("Python 3 is needed to run plugins: install it from python.org, or set TEQUILA_PYTHON")?;
        let host = host_script().ok_or("the plugin host (python/tequila_plugin.py) is missing from this installation")?;
        let mut command = std::process::Command::new(&python[0]);
        command
            .args(&python[1..])
            .arg("-u")
            .arg(&host)
            .arg(&plugin.folder)
            .current_dir(&plugin.folder)
            .env("PYTHONPATH", host.parent().unwrap_or(Path::new(".")))
            .env("PYTHONIOENCODING", "utf-8")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        // Where it may keep settings and caches: the person's, beside their
        // plugins, so they survive the plugin being updated or reinstalled.
        command.env("TEQUILA_PLUGIN_DATA", data_directory(&plugin.manifest.id));
        match discovery {
            Some((path, pid)) => {
                command.env("TEQUILA_DISCOVERY", path).env("TEQUILA_PID", pid.to_string());
            }
            None => {
                command.env_remove("TEQUILA_DISCOVERY");
            }
        }
        quiet(&mut command);
        let mut child = command.spawn().map_err(|e| format!("Python would not start: {e}"))?;
        let (sender, said): (Sender<Said>, Receiver<Said>) = std::sync::mpsc::channel();
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let stderr = child.stderr.take().ok_or("no stderr")?;
        let out = sender.clone();
        std::thread::Builder::new()
            .name(format!("plugin-{}", plugin.manifest.id))
            .spawn(move || {
                for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
                    if out.send(read_said(&line)).is_err() {
                        break;
                    }
                }
                let _ = out.send(Said::Exited(None));
            })
            .map_err(|e| e.to_string())?;
        std::thread::Builder::new()
            .name(format!("plugin-{}-log", plugin.manifest.id))
            .spawn(move || {
                for line in std::io::BufReader::new(stderr).lines().map_while(Result::ok) {
                    if sender.send(Said::Log(line)).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        let mut stdin = child.stdin.take().ok_or("no stdin")?;
        // Bounded: a plugin that has stopped reading is not queued for
        // without end; what it would not take is refused, and said.
        let (to_plugin, lines) = std::sync::mpsc::sync_channel::<String>(1024);
        std::thread::Builder::new()
            .name(format!("plugin-{}-input", plugin.manifest.id))
            .spawn(move || {
                for line in lines {
                    if stdin.write_all(line.as_bytes()).and_then(|_| stdin.flush()).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Host { child, to_plugin: Some(to_plugin), said })
    }

    fn send(&mut self, object: tequila_format::json_write::Object) -> Result<(), String> {
        let to_plugin = self.to_plugin.as_ref().ok_or("the plugin has stopped")?;
        let mut line = tequila_format::json_write::to_string(&object.build()).replace('\n', " ");
        line.push('\n');
        to_plugin.try_send(line).map_err(|e| match e {
            std::sync::mpsc::TrySendError::Full(_) => "the plugin is not keeping up, so this was not sent to it".to_string(),
            std::sync::mpsc::TrySendError::Disconnected(_) => "the plugin has stopped".to_string(),
        })
    }

    /// Run one of its commands, with a panel's values.
    pub fn command(&mut self, command: &str, values: &[(String, Value)]) -> Result<(), String> {
        let mut fields = tequila_format::json_write::Object::new();
        for (key, value) in values {
            fields.set(key, value.clone());
        }
        let mut object = tequila_format::json_write::Object::new();
        object.set("type", "command").set("command", command).set("values", fields.build());
        self.send(object)
    }

    /// Tell it about an event it asked for.
    pub fn event(&mut self, event: &str, data: Value) -> Result<(), String> {
        let mut object = tequila_format::json_write::Object::new();
        object.set("type", "event").set("event", event).set("data", data);
        self.send(object)
    }

    /// Whether the process is still going.
    pub fn running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Host {
    /// Ask it to stop, give it a moment, then end it - by its own process,
    /// never by name.
    fn drop(&mut self) {
        let mut object = tequila_format::json_write::Object::new();
        object.set("type", "shutdown");
        let _ = self.send(object);
        // Its input closes once the writer has sent what is queued.
        self.to_plugin = None;
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(800);
        while std::time::Instant::now() < deadline {
            if !matches!(self.child.try_wait(), Ok(None)) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// An open panel: which plugin's, and its fields' values as they stand.
pub struct OpenPanel {
    pub plugin: String,
    pub panel: String,
    pub values: Vec<(String, Value)>,
}

/// Every plugin, which are on, the running ones, their logs and open panels.
pub struct Plugins {
    pub plugins: Vec<Plugin>,
    pub problems: Vec<String>,
    pub enabled: BTreeSet<String>,
    pub hosts: HashMap<String, Host>,
    pub logs: HashMap<String, VecDeque<String>>,
    /// How many lines each plugin has logged, ever: where a reader that
    /// came back is up to, though the log keeps only the last lines.
    pub logged: HashMap<String, u64>,
    pub panels: Vec<OpenPanel>,
    /// Why a plugin that should be running is not.
    pub failures: HashMap<String, String>,
    settings: PathBuf,
    folders: Vec<(PathBuf, Origin)>,
}

/// How many lines of a plugin's log are kept.
const LOG_LINES: usize = 400;

impl Plugins {
    /// The user's plugins and the bundled ones, as this machine has them.
    pub fn load() -> Plugins {
        let mut folders = Vec::new();
        if let Some(bundled) = bundled_directory() {
            folders.push((bundled, Origin::Bundled));
        }
        folders.push((user_directory(), Origin::User));
        Plugins::load_from(folders, settings_path())
    }

    pub fn load_from(folders: Vec<(PathBuf, Origin)>, settings: PathBuf) -> Plugins {
        let (plugins, problems) = discover(&folders);
        Plugins {
            enabled: read_enabled(&settings),
            plugins,
            problems,
            hosts: HashMap::new(),
            logs: HashMap::new(),
            logged: HashMap::new(),
            panels: Vec::new(),
            failures: HashMap::new(),
            settings,
            folders,
        }
    }

    /// Read the folders again, keeping what is running that is still there.
    pub fn reload(&mut self) {
        let (plugins, problems) = discover(&self.folders);
        self.plugins = plugins;
        self.problems = problems;
        let present: BTreeSet<String> = self.plugins.iter().map(|p| p.manifest.id.clone()).collect();
        self.hosts.retain(|id, _| present.contains(id));
        self.panels.retain(|p| present.contains(&p.plugin));
    }

    pub fn get(&self, id: &str) -> Option<&Plugin> {
        self.plugins.iter().find(|p| p.manifest.id == id)
    }

    pub fn is_enabled(&self, id: &str) -> bool {
        self.enabled.contains(id)
    }

    /// Turn a plugin on or off, remembering it. Its code starts when the
    /// application next calls [`Plugins::start_enabled`].
    pub fn set_enabled(&mut self, id: &str, on: bool) -> Result<(), String> {
        if on {
            self.enabled.insert(id.to_string());
        } else {
            self.enabled.remove(id);
            self.hosts.remove(id);
            self.panels.retain(|p| p.plugin != id);
            self.failures.remove(id);
        }
        write_enabled(&self.settings, &self.enabled)
    }

    /// Start every enabled plugin with code that is not running, connected
    /// to this window's channel.
    pub fn start_enabled(&mut self, discovery: Option<(&Path, u32)>) {
        let wanted: Vec<Plugin> = self
            .plugins
            .iter()
            .filter(|p| self.enabled.contains(&p.manifest.id) && p.runs_code() && !self.hosts.contains_key(&p.manifest.id) && !self.failures.contains_key(&p.manifest.id))
            .cloned()
            .collect();
        for plugin in wanted {
            match Host::start(&plugin, discovery) {
                Ok(host) => {
                    self.hosts.insert(plugin.manifest.id.clone(), host);
                }
                Err(error) => {
                    self.log(&plugin.manifest.id, &error);
                    self.failures.insert(plugin.manifest.id.clone(), error);
                }
            }
        }
    }

    /// Stop a plugin's code and start it again: how its author reloads it.
    pub fn restart(&mut self, id: &str) {
        self.hosts.remove(id);
        self.failures.remove(id);
    }

    pub fn log(&mut self, id: &str, line: &str) {
        let log = self.logs.entry(id.to_string()).or_default();
        let logged = self.logged.entry(id.to_string()).or_default();
        for part in line.lines() {
            log.push_back(part.to_string());
            *logged += 1;
        }
        while log.len() > LOG_LINES {
            log.pop_front();
        }
    }

    /// Run a command of an enabled plugin: open its panel, or hand it to
    /// its code with `values`.
    pub fn run(&mut self, id: &str, command: &str, values: &[(String, Value)]) -> Result<(), String> {
        let plugin = self.get(id).ok_or_else(|| format!("no plugin `{id}`"))?.clone();
        if let Some(panel) = plugin.manifest.commands.iter().find(|c| c.id == command).and_then(|c| c.panel.clone()) {
            self.open_panel(id, &panel);
            return Ok(());
        }
        let host = self.hosts.get_mut(id).ok_or_else(|| self.failures.get(id).cloned().unwrap_or_else(|| format!("`{}` is not running", plugin.manifest.name)))?;
        host.command(command, values)
    }

    /// Open one of a plugin's panels, its fields at their defaults.
    pub fn open_panel(&mut self, id: &str, panel: &str) {
        if self.panels.iter().any(|p| p.plugin == id && p.panel == panel) {
            return;
        }
        let Some(spec) = self.get(id).and_then(|p| p.manifest.panels.iter().find(|s| s.id == panel)).cloned() else {
            return;
        };
        let values = spec
            .fields
            .iter()
            .filter_map(|f| {
                let value = match &f.kind {
                    FieldKind::Number { default, .. } => Value::Number(*default),
                    FieldKind::Text { default, .. } => Value::String(default.clone()),
                    FieldKind::Toggle { default } => Value::Bool(*default),
                    FieldKind::Choice { default, .. } => Value::String(default.clone()),
                    FieldKind::Colour { default } => Value::Array(default.iter().map(|c| Value::Number(*c as f64)).collect()),
                    FieldKind::Label { text } => Value::String(text.clone()),
                    FieldKind::Path { default, .. } => Value::String(default.clone()),
                    FieldKind::Button { .. } => return None,
                };
                Some((f.id.clone(), value))
            })
            .collect();
        self.panels.push(OpenPanel { plugin: id.to_string(), panel: panel.to_string(), values });
        // A plugin that asked is told, so it can fill the panel in - with
        // what it remembered, or what the scene holds now.
        let listening = self.get(id).is_some_and(|p| p.manifest.events.iter().any(|e| e == "panel_opened"));
        if let (true, Some(host)) = (listening, self.hosts.get_mut(id)) {
            let mut data = tequila_format::json_write::Object::new();
            data.set("panel", panel);
            if let Err(error) = host.event("panel_opened", data.build()) {
                self.log(id, &error);
            }
        }
    }

    /// Tell the running plugins that asked about `event`.
    pub fn broadcast(&mut self, event: &str, data: &Value) {
        let listening: Vec<String> = self
            .plugins
            .iter()
            .filter(|p| p.manifest.events.iter().any(|e| e == event))
            .map(|p| p.manifest.id.clone())
            .collect();
        for id in listening {
            if let Some(host) = self.hosts.get_mut(&id) {
                if let Err(error) = host.event(event, data.clone()) {
                    self.log(&id, &error);
                }
            }
        }
    }

    /// What the running plugins said since last asked: their messages for
    /// the person, as `(plugin name, message, is an error)`. Logs and panel
    /// values are taken in here.
    pub fn poll(&mut self) -> Vec<(String, String, bool)> {
        let mut messages = Vec::new();
        let ids: Vec<String> = self.hosts.keys().cloned().collect();
        for id in ids {
            let name = self.get(&id).map(|p| p.manifest.name.clone()).unwrap_or_else(|| id.clone());
            let mut said = Vec::new();
            if let Some(host) = self.hosts.get(&id) {
                while let Ok(one) = host.said.try_recv() {
                    said.push(one);
                }
            }
            for one in said {
                match one {
                    Said::Ready => self.log(&id, "running"),
                    Said::Status(text) => {
                        self.log(&id, &text);
                        messages.push((name.clone(), text, false));
                    }
                    Said::Error(text) => {
                        self.log(&id, &text);
                        let first = text.lines().last().unwrap_or("failed").to_string();
                        messages.push((name.clone(), first, true));
                    }
                    Said::Log(text) => self.log(&id, &text),
                    Said::Done(Some(command)) => self.log(&id, &format!("finished `{command}`")),
                    Said::Done(None) => {}
                    Said::Panel { panel, values } => {
                        if let Some(open) = self.panels.iter_mut().find(|p| p.plugin == id && p.panel == panel) {
                            for (key, value) in values {
                                match open.values.iter_mut().find(|(k, _)| *k == key) {
                                    Some(slot) => slot.1 = value,
                                    None => open.values.push((key, value)),
                                }
                            }
                        }
                    }
                    Said::Exited(code) => {
                        let reason = match code {
                            Some(code) => format!("stopped (exit {code})"),
                            None => "stopped".to_string(),
                        };
                        self.log(&id, &reason);
                    }
                }
            }
            let stopped = self.hosts.get_mut(&id).is_some_and(|h| !h.running());
            if stopped {
                self.hosts.remove(&id);
                let reason = "its code stopped; see its log".to_string();
                self.failures.insert(id.clone(), reason.clone());
                messages.push((name, reason, true));
            }
        }
        messages
    }

    /// The materials folders of the enabled plugins, with their names.
    pub fn material_folders(&self) -> Vec<(PathBuf, String)> {
        self.plugins
            .iter()
            .filter(|p| self.enabled.contains(&p.manifest.id))
            .filter_map(|p| p.materials_folder().map(|f| (f, p.manifest.name.clone())))
            .collect()
    }

    /// Where the user's plugins go.
    pub fn user_folder(&self) -> PathBuf {
        self.folders.iter().find(|(_, o)| *o == Origin::User).map(|(f, _)| f.clone()).unwrap_or_else(user_directory)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tequila-plugins-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The template is a valid plugin as written: a command, a panel with
    /// a button and a label, an event, and code to run them.
    #[test]
    fn the_template_is_a_plugin() {
        let into = scratch("template");
        let folder = create_from_template(&into, "Tidy Names").expect("writes");
        let manifest = parse_manifest(&std::fs::read_to_string(folder.join("plugin.json")).unwrap()).expect("parses");
        assert_eq!(manifest.id, "my.tidy-names");
        assert_eq!(manifest.name, "Tidy Names");
        assert_eq!(manifest.main.as_deref(), Some("main.py"));
        assert!(manifest.commands.iter().any(|c| c.panel.as_deref() == Some("main")));
        assert_eq!(manifest.events, vec!["selection".to_string()]);
        assert!(create_from_template(&into, "Tidy Names").is_err(), "a second made over the first");
        let (found, problems) = discover(&[(into.clone(), Origin::User)]);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(found.len(), 1);
        let _ = std::fs::remove_dir_all(&into);
    }

    /// The plugins shipped as examples are found and read, and the material
    /// pack's presets load.
    #[test]
    fn the_examples_are_plugins() {
        let bundled = bundled_directory().expect("the examples ship");
        let (found, problems) = discover(&[(bundled, Origin::Bundled)]);
        assert!(problems.is_empty(), "{problems:?}");
        let ids: Vec<&str> = found.iter().map(|p| p.manifest.id.as_str()).collect();
        for id in ["tequila.colourways", "tequila.anodised-extras", "tequila.vibes", "tequila.auto-materials", "tequila.render-every-camera"] {
            assert!(ids.contains(&id), "{id} is missing: {ids:?}");
        }
        let vibes = found.iter().find(|p| p.manifest.id == "tequila.vibes").unwrap();
        assert!(vibes.runs_code());
        let panel = vibes.manifest.panels.iter().find(|p| p.id == "vibes").expect("the Vibes panel");
        assert!(panel.fields.iter().any(|f| matches!(&f.kind, FieldKind::Choice { options, .. } if options.len() == 8)), "the eight vibes are not a choice");
        let pack = found.iter().find(|p| p.manifest.id == "tequila.anodised-extras").unwrap();
        assert!(!pack.runs_code(), "a material pack asks to run code");
        let (presets, problems) = tequila_format::library::read_directory_from(&pack.materials_folder().expect("its materials"), tequila_format::library::Source::Plugin);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(presets.len(), 4);
        assert!(presets.iter().all(|p| !p.is_editable()));
    }

    /// A manifest that would not work is refused with the reason, before
    /// anything runs: a bad id, code with nothing to run it, a panel that
    /// is not there, an event that does not exist, a path out of the
    /// folder, a newer API.
    #[test]
    fn a_broken_manifest_says_why() {
        for (manifest, reason) in [
            (r#"{"name":"X"}"#, "no `id`"),
            (r#"{"id":"a b","name":"X"}"#, "not a plugin id"),
            (r#"{"id":"x","name":"X","commands":[{"id":"go"}]}"#, "no `main`"),
            (r#"{"id":"x","name":"X","main":"m.py","commands":[{"id":"go","panel":"p"}]}"#, "does not declare"),
            (r#"{"id":"x","name":"X","main":"m.py","events":["explode"]}"#, "not an event"),
            (r#"{"id":"x","name":"X","main":"../m.py"}"#, "inside the plugin's folder"),
            (r#"{"id":"x","name":"X","tequila":9}"#, "newer Tequila"),
            (r#"{"id":"x","name":"X","main":"m.py","panels":[{"id":"p","fields":[{"id":"f","type":"dial"}]}]}"#, "type this does not know"),
        ] {
            let error = parse_manifest(manifest).expect_err(manifest);
            assert!(error.contains(reason), "{manifest}: {error}");
        }
        let content = parse_manifest(r#"{"id":"pack.metals","name":"Metals","materials":"materials"}"#).expect("a content pack needs no code");
        assert!(content.main.is_none());
    }

    /// A plugin shared as a ZIP installs into its own folder, from the top
    /// of the archive or one folder down; it is not installed over itself
    /// unless asked; and nothing in it lands outside its folder.
    #[test]
    fn a_plugin_installs_from_a_zip() {
        let root = scratch("zip");
        let into = root.join("plugins");
        let mut zip = tequila_format::zip::ZipWriter::new();
        zip.add("shared/plugin.json", br#"{"id":"com.example.shared","name":"Shared","main":"main.py","commands":[{"id":"go"}]}"#.to_vec());
        zip.add("shared/main.py", b"import tequila_plugin\n".to_vec());
        zip.add("shared/materials/blue.json", br#"{"name":"Blue","type":"plastic","category":"Plastic"}"#.to_vec());
        let file = root.join("shared.zip");
        std::fs::write(&file, zip.finish()).unwrap();
        let folder = install_zip(&file, &into, false).expect("installs");
        assert_eq!(folder, into.join("com.example.shared"));
        assert!(folder.join("main.py").is_file());
        assert!(folder.join("materials").join("blue.json").is_file());
        assert!(install_zip(&file, &into, false).is_err(), "installed over itself");
        assert!(install_zip(&file, &into, true).is_ok());
        let mut hostile = tequila_format::zip::ZipWriter::new();
        hostile.add("plugin.json", br#"{"id":"evil","name":"Evil"}"#.to_vec());
        hostile.add("C:/escaped.txt", b"x".to_vec());
        let bad = root.join("evil.zip");
        std::fs::write(&bad, hostile.finish()).unwrap();
        assert!(install_zip(&bad, &into, false).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// What is on is remembered; a plugin with materials offers them only
    /// while it is on; a user's copy of a bundled plugin wins.
    #[test]
    fn enabled_plugins_are_remembered_and_bring_their_materials() {
        let root = scratch("enabled");
        let (bundled, user) = (root.join("bundled"), root.join("user"));
        for (folder, name) in [(&bundled, "Bundled Pack"), (&user, "My Pack")] {
            let dir = folder.join("pack");
            std::fs::create_dir_all(dir.join("materials")).unwrap();
            std::fs::write(dir.join("plugin.json"), format!(r#"{{"id":"pack","name":"{name}","materials":"materials"}}"#)).unwrap();
        }
        let settings = root.join("plugins.json");
        let folders = vec![(bundled.clone(), Origin::Bundled), (user.clone(), Origin::User)];
        let mut plugins = Plugins::load_from(folders.clone(), settings.clone());
        assert_eq!(plugins.plugins.len(), 1);
        assert_eq!(plugins.plugins[0].manifest.name, "My Pack", "the bundled copy won");
        assert!(plugins.material_folders().is_empty(), "a plugin that is off brought materials");
        plugins.set_enabled("pack", true).unwrap();
        assert_eq!(plugins.material_folders().len(), 1);
        let again = Plugins::load_from(folders, settings);
        assert!(again.is_enabled("pack"), "the setting was not kept");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A plugin's code runs in its own Python process: a command reaches it
    /// with the panel's values, its status comes back, print() goes to the
    /// log rather than into the conversation, an error is reported and the
    /// plugin keeps running, and dropping it ends the process. Skipped
    /// where there is no Python.
    #[test]
    fn a_plugins_code_runs_and_answers() {
        if python().is_none() {
            eprintln!("skipped: no Python 3");
            return;
        }
        let root = scratch("host");
        let dir = root.join("echo");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("plugin.json"),
            r#"{"id":"echo","name":"Echo","main":"main.py","commands":[{"id":"say"},{"id":"fail"}],"events":["selection"]}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("main.py"),
            "import tequila_plugin as plugin\n\n@plugin.command('say')\ndef say(fs, values):\n    print('printed, not said')\n    plugin.status('heard ' + values['word'] + (' alone' if fs is None else ''))\n\n@plugin.command('fail')\ndef fail(fs, values):\n    raise ValueError('on purpose')\n\n@plugin.on('selection')\ndef selection(fs, data):\n    plugin.status('selected ' + str(len(data['nodes'])))\n",
        )
        .unwrap();
        let (found, problems) = discover(&[(root.clone(), Origin::User)]);
        assert!(problems.is_empty(), "{problems:?}");
        let mut host = Host::start(&found[0], None).expect("starts");
        let wait_for = |host: &Host, want: &dyn Fn(&Said) -> bool| -> Vec<Said> {
            let mut seen = Vec::new();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while std::time::Instant::now() < deadline {
                if let Ok(said) = host.said.recv_timeout(std::time::Duration::from_millis(100)) {
                    let found = want(&said);
                    seen.push(said);
                    if found {
                        return seen;
                    }
                }
            }
            panic!("timed out; heard {seen:?}");
        };
        wait_for(&host, &|s| *s == Said::Ready);
        host.command("say", &[("word".into(), Value::String("hello".into()))]).unwrap();
        let heard = wait_for(&host, &|s| *s == Said::Status("heard hello alone".into()));
        assert!(heard.contains(&Said::Log("printed, not said".into())), "{heard:?}");
        host.command("fail", &[]).unwrap();
        let heard = wait_for(&host, &|s| matches!(s, Said::Error(_)));
        assert!(heard.iter().any(|s| matches!(s, Said::Error(t) if t.contains("on purpose"))), "{heard:?}");
        let mut nodes = tequila_format::json_write::Object::new();
        nodes.set("nodes", Value::Array(vec![Value::Number(3.0), Value::Number(4.0)]));
        host.event("selection", nodes.build()).unwrap();
        wait_for(&host, &|s| *s == Said::Status("selected 2".into()));
        assert!(host.running(), "an error stopped the plugin");
        drop(host);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// What a script or an AI app writes stays inside the plugin's folder,
    /// is text, and is a plugin: refused whole otherwise, nothing written.
    #[test]
    fn a_plugin_is_written_only_as_a_plugin_inside_its_folder() {
        let root = scratch("write");
        let folder = root.join("my.renamer");
        let manifest = r#"{"id":"my.renamer","name":"Renamer","main":"main.py","commands":[{"id":"go"}]}"#.to_string();
        let file = |path: &str, text: &str| (path.to_string(), text.to_string());
        for (files, why) in [
            (vec![file("plugin.json", &manifest), file("main.py", ""), file("../escape.py", "")], "a path out of the folder"),
            (vec![file("plugin.json", &manifest), file("main.py", ""), file("lib/../../escape.py", "")], "a path out through a subfolder"),
            (vec![file("plugin.json", &manifest), file("main.py", ""), file("C:/escape.py", "")], "a drive path"),
            (vec![file("plugin.json", &manifest), file("main.py", ""), file("/escape.py", "")], "an absolute path"),
            (vec![file("plugin.json", &manifest), file("main.py", ""), file("tool.exe", "")], "a program"),
            (vec![file("plugin.json", &manifest.replace("my.renamer", "my.other")), file("main.py", "")], "a manifest naming another plugin"),
            (vec![file("plugin.json", &manifest)], "a manifest running a file that is not there"),
            (vec![file("main.py", "")], "a new plugin with no manifest"),
            (vec![file("plugin.json", "{ not json"), file("main.py", "")], "a manifest that does not read"),
            (vec![file("plugin.json", &manifest), file("main.py", &"#".repeat(WRITABLE_SIZE + 1))], "a file over a megabyte"),
        ] {
            assert!(write_files(&folder, "my.renamer", &files).is_err(), "wrote {why}");
            assert!(!folder.exists() && !root.join("escape.py").exists(), "{why} left files behind");
        }
        assert!(write_files(&root.join(".."), "..", &[file("plugin.json", "{}")]).is_err(), "an id that is a parent folder");

        let written = write_files(&folder, "my.renamer", &[file("plugin.json", &manifest), file("main.py", "print(1)\n"), file("notes\\README.md", "hi")]).expect("a plugin");
        assert_eq!(written, folder);
        assert_eq!(std::fs::read_to_string(folder.join("notes").join("README.md")).unwrap(), "hi");
        // A later write of one file keeps the manifest it has.
        write_files(&folder, "my.renamer", &[file("main.py", "print(2)\n")]).expect("one file changed");
        assert_eq!(std::fs::read_to_string(folder.join("main.py")).unwrap(), "print(2)\n");
        let (found, problems) = discover(&[(root.clone(), Origin::User)]);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(found[0].manifest.id, "my.renamer");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A file or folder field reads its kind, what it opens or saves, and
    /// its extensions with or without the dot; its panel starts at the
    /// default path.
    #[test]
    fn file_and_folder_fields_read() {
        let manifest = parse_manifest(
            r#"{"id":"p","name":"P","panels":[{"id":"main","fields":[
                {"id":"source","type":"file","label":"CSV","extensions":[".csv","TXT"]},
                {"id":"out","type":"file","save":true,"default":"C:/out.png"},
                {"id":"into","type":"folder"}]}]}"#,
        )
        .expect("parses");
        let kinds: Vec<&FieldKind> = manifest.panels[0].fields.iter().map(|f| &f.kind).collect();
        assert!(matches!(kinds[0], FieldKind::Path { folder: false, save: false, extensions, .. } if *extensions == vec!["csv".to_string(), "TXT".to_string()]));
        assert!(matches!(kinds[1], FieldKind::Path { folder: false, save: true, default, .. } if default == "C:/out.png"));
        assert!(matches!(kinds[2], FieldKind::Path { folder: true, .. }));
        let error = parse_manifest(r#"{"id":"p","name":"P","panels":[{"id":"main","fields":[{"id":"x","type":"slider"}]}]}"#).unwrap_err();
        assert!(error.contains("file, folder"), "{error}");

        let root = scratch("paths");
        std::fs::create_dir_all(root.join("p")).unwrap();
        std::fs::write(root.join("p").join("plugin.json"), r#"{"id":"p","name":"P","panels":[{"id":"main","fields":[{"id":"out","type":"file","default":"C:/out.png"}]}]}"#).unwrap();
        let mut plugins = Plugins::load_from(vec![(root.clone(), Origin::User)], root.join("enabled.json"));
        plugins.open_panel("p", "main");
        assert_eq!(plugins.panels[0].values, vec![("out".to_string(), Value::String("C:/out.png".into()))]);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The guide is this build's: its events, its field types, and a
    /// template that is a plugin.
    #[test]
    fn the_guide_describes_this_build() {
        let guide = guide();
        assert!(guide.get("reference").and_then(Value::as_str).is_some_and(|r| r.contains("plugin.json")));
        let events: Vec<&str> = guide.get("events").unwrap().as_array().iter().filter_map(Value::as_str).collect();
        assert_eq!(events, EVENTS);
        let template = guide.get("template").unwrap();
        let manifest = parse_manifest(template.get("plugin.json").and_then(Value::as_str).unwrap()).expect("the template parses");
        assert_eq!(manifest.id, "my.example");
        assert!(template.get("main.py").and_then(Value::as_str).unwrap().contains("@plugin.command"));
    }

    /// A finished command is named, so its log says which one finished.
    #[test]
    fn a_finished_command_is_named() {
        assert_eq!(read_said(r#"{"type":"done","command":"go"}"#), Said::Done(Some("go".into())));
        assert_eq!(read_said(r#"{"type":"done"}"#), Said::Done(None));
    }
}
