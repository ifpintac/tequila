//! What a client may ask a running Tequila for, and what it gets back.
//!
//! One JSON object per line, in both directions. That is the same shape the
//! journal already writes (`docs/03` §4.4), and it is not a coincidence: an
//! `apply` request carries a *journal line* as its payload, so
//! [`tequila_format::journal::decode`] parses it unchanged and the
//! `every_command_round_trips_through_the_journal` test in that crate is
//! already the coverage guarantee for this API. A command that can be
//! journalled can be scripted, by construction, and a new command cannot be
//! added to the catalog without both.
//!
//! # Why a request carries an id
//!
//! Replies are matched to requests by `id` rather than by order, because the
//! interesting operations are the slow ones — a render is seconds, a query is
//! microseconds — and a client that has to wait for the render before it can
//! ask anything else is a client that cannot poll progress.

use tequila_format::json::{self, Value};
use tequila_format::json_write::{self, Object};
use tequila_scene::command::Command;

/// What a client is asking for.
#[derive(Debug)]
pub enum Operation {
    /// Liveness, and which protocol the other end speaks.
    Ping,
    /// Apply one command to the document, through the bus, exactly as a click
    /// would. Undoable, journalled, and subject to the same validation.
    Apply(Box<Command>),
    Undo,
    Redo,
    /// Read the document. The half of the API that commands cannot provide,
    /// because a command is a write.
    Query(Query),
    /// Replace the selection. Selection is view state rather than document
    /// state — it is not in the command catalog and is not undoable — so it
    /// gets its own operation rather than a fabricated command.
    Select(Vec<u64>),
    /// Load a model into the open document.
    ///
    /// A path rather than a payload. `scene.import` exists as a command, but
    /// it carries the whole mesh inlined, which is the right shape for a
    /// journal and an absurd one for a request — so this names the file and
    /// lets the application read it, then applies the very same command.
    Import { path: String, parent: Option<u64> },
    /// Make materials from a PBR texture pack — a folder, a ZIP, or one of
    /// its maps — and put the first on `nodes` (the selection when empty).
    TextureSet { path: String, nodes: Vec<u64> },
    /// Update an imported model from its file, or from `path`.
    ///
    /// With `tessellation` (`draft`, `medium`, `high`, `ultra`) and no path,
    /// a CAD model is divided again from the source kept with the project.
    Reimport { node: Option<u64>, path: Option<String>, tessellation: Option<String> },
    /// Write the project to disk.
    ///
    /// Scripted work that cannot be saved is scripted work waiting to be
    /// lost, which is not a hypothetical: a session's worth of it went that
    /// way for want of this.
    Save { path: Option<String> },
    /// Point the camera at something, so the next picture contains it.
    ///
    /// Vision, not decoration: a script that can change a part but never look
    /// at it is working blind, and framing is how you get the part you just
    /// changed into the frame.
    Frame { all: bool },
    /// Capture the application window as it stands, the way a person looking
    /// at the screen would see it.
    ///
    /// `window` names one of the application's other windows — `render`,
    /// `graph`, `shortcuts`, `connect` — to capture that instead: they are
    /// windows of their own, outside the main one.
    Screenshot { path: String, viewport: bool, window: Option<String> },
    /// Render the document offline, at a quality the viewport does not have
    /// to sustain interactively.
    Render {
        path: String,
        width: usize,
        height: usize,
        samples: u32,
        /// Draw the model's edges instead, as the document's Wireframe
        /// settings say: no samples, a second or two.
        wireframe: bool,
    },
    /// Several operations, in order, as **one undo step**.
    ///
    /// What a script is: one intention in many commands. Applied one request
    /// at a time, thirty material edits were thirty Ctrl+Z presses for the
    /// person who then wanted them gone. Stops at the first failure; when
    /// `atomic` (the default) it also takes back everything the batch had
    /// applied, so a half-finished plan never lands in the scene.
    Batch { operations: Vec<Operation>, atomic: bool },
    /// Close the application, cleanly.
    ///
    /// Refused while there are unsaved changes unless the request says which
    /// way to settle them — `save` or `discard` — because a script that closes
    /// the window on someone's work is the one thing it must never do by
    /// default. A clean exit also clears the crash journal, which killing the
    /// process does not, and which is why a killed session greets the next
    /// launch with an offer to recover work that was never lost.
    Quit { save: bool, discard: bool },
    /// Start a new project - the starter scene, as a launch has it - or
    /// open the project at `path`. Unsaved work settles as for `quit`: the
    /// request is refused unless it says `save` or `discard`.
    Project { action: ProjectAction, save: bool, discard: bool },
    /// Open, close and switch the application's panels, and set the viewport
    /// mode — so a script can put on screen what it wants a screenshot of.
    Ui(UiChange),
    /// Write the scene out for other software: `glb` a glTF binary of what
    /// is shown, `web` the configurator as a static web folder - its live
    /// 3D view, and with `photos` a queued render of every combination.
    ///
    /// `spin` renders a spin viewer into a new folder under `path`: `columns`
    /// frames round (36), `rows` of tilt (1) `tilt` degrees apart (15),
    /// `transparent` cut out on PNG.
    Export {
        format: String,
        path: String,
        live: bool,
        photos: bool,
        columns: Option<usize>,
        rows: Option<usize>,
        tilt: Option<f32>,
        transparent: bool,
    },
    /// Add a render to the background queue, where the Render window's own
    /// renders go, and return at once.
    ///
    /// The other way to render. `render` blocks the whole application until
    /// it finishes; this does not, reports progress through `jobs`, and uses
    /// the document's render settings, so it writes what the Render window
    /// would have written.
    Queue(QueueRender),
    /// Stop the running background render. It still writes what it has.
    Stop,
    /// Put the camera somewhere precise.
    ///
    /// `frame` gets the model into view; this is for the shot itself: a
    /// standard view, an orbit angle, or an exact eye and target. Undoable,
    /// like moving the camera by hand.
    Look(Look),
    /// A LiveLink message from a modelling application
    /// (`docs/livelink-protocol.md`): `hello`, `sync` or `camera`, read by
    /// the application, which owns the scene the sync applies to.
    LiveLink(Value),
    /// `studio_preset`: Studio presets (`docs/studio-presets.md`) - `list`
    /// them, `apply` an installed one (its id as `preset`), `save` the current studio as one,
    /// `install` or `export` a package, `delete` one, or `render_all`
    /// views. Carried as sent; the application reads its fields.
    StudioPreset(Value),
    /// `substance`: Adobe Substance materials (`docs/02` §5.6) - `info` on a
    /// file, `open` one as materials, `set` a material's values, `status`
    /// of its maps, `wait` for them, `tools` to say where Adobe's are.
    /// Carried as sent; the application reads its fields.
    Substance(Value),
    /// The material library: putting presets on parts, and adding to the
    /// user's own - what a script or a plugin does with materials beyond
    /// the document. See [`LibraryRequest`].
    Library(LibraryRequest),
    /// A material of the caller's own, from a recipe. See [`MaterialRequest`].
    Material(MaterialRequest),
    /// Making and driving plugins, for a script or an AI app writing one:
    /// the Tequila window's, as plugins run beside it. See [`PluginRequest`].
    Plugin(PluginRequest),
}

/// What a [`PluginRequest`] can do.
pub const PLUGIN_ACTIONS: [&str; 7] = ["guide", "list", "write", "log", "run", "enable", "restart"];

/// A plugin, made or driven by a script or an AI app. `action` says what:
///
/// - `guide`: how to write one - the reference, the manifest's field and
///   event names, a template, and the way to work.
/// - `list`: every plugin, with whether it is on, running, or failed, its
///   commands, panels and events, and its folder.
/// - `write`: `files` (path in the plugin's folder, text) into the user's
///   plugin `plugin`, new or changed. A running plugin restarts with them.
/// - `log`: the last `lines` (40) of the plugin's log: its prints, status
///   messages and tracebacks.
/// - `run`: the plugin's `command`, with `values` as a panel would send.
/// - `enable`: ask the person to turn the plugin on. One with code runs
///   Python as them, so Tequila asks them, and only they can say yes; a
///   material pack is turned on at once.
/// - `restart`: stop the plugin's code and start it again.
///
/// The plugin is named by `plugin`, its id: `id` is every request's own.
/// A write with `ask` turns off a plugin with code that was on, and asks the
/// person to turn it on again - so code changed by the built-in assistant
/// runs only once they say so.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PluginRequest {
    pub action: String,
    pub plugin: Option<String>,
    pub ask: bool,
    pub files: Vec<(String, String)>,
    pub command: Option<String>,
    pub values: Vec<(String, Value)>,
    pub lines: Option<usize>,
}

/// A material made from a recipe (`crate::material`). `action` says what:
///
/// - `guide`: how to write a recipe - its fields, every pattern with what
///   it looks like and its settings, the material types, and examples.
/// - `create`: a new material from `recipe`, put on `nodes` (the selection
///   when empty; on nothing when both are). Answers with its id.
/// - `update`: the material `material` remade from `recipe`, where it is.
/// - `recipe`: the material `material` as a recipe, to change and update.
/// - `preview`: `recipe`, or the material `material`, rendered on a ball
///   `radius` metres across (5 cm by default; `shape` "slab" for a slab),
///   `size` pixels square, to `path` or a temporary file. The scene is not
///   touched.
/// - `save`: the material `material` into the user's library, as `name`
///   under `category`, with `tags`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct MaterialRequest {
    pub action: String,
    pub recipe: Option<Value>,
    pub material: Option<u64>,
    pub nodes: Vec<u64>,
    pub name: Option<String>,
    pub category: Option<String>,
    pub tags: Vec<String>,
    pub size: Option<usize>,
    pub shape: Option<String>,
    pub radius: Option<f32>,
    pub path: Option<String>,
}

/// What a `new` or `open` request does: see [`Operation::Project`].
#[derive(Clone, Debug, PartialEq)]
pub enum ProjectAction {
    New,
    Open(String),
}

/// What to do with the material library. `action` says which, and the
/// other fields are what it needs:
///
/// - `apply`: the preset `name` onto `nodes` (the selection when empty), a
///   copy of its own with `unlink`. Undoable, like dropping it by hand.
/// - `save`: the document's material `material` into the user's library,
///   as `name` (its own when absent) under `category`, with `tags`.
/// - `import`: the file at `path` into the user's library - a material
///   package (`.fsmat`) or a preset's `.json`, or a texture pack (a folder,
///   a ZIP, or one of its maps), one repeat `size` metres across - as `name`
///   and under `category` where given.
/// - `export`: the preset `name` as a material package at `path`.
/// - `delete`: the user's own preset `name`. The shipped ones never.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct LibraryRequest {
    pub action: String,
    pub name: Option<String>,
    pub nodes: Vec<u64>,
    pub unlink: bool,
    pub material: Option<u64>,
    pub category: Option<String>,
    pub tags: Vec<String>,
    pub path: Option<String>,
    pub size: Option<f32>,
}

/// The `studio_preset` operation's actions.
pub const STUDIO_PRESET_ACTIONS: &[&str] = &["list", "apply", "save", "delete", "install", "export", "render_all"];

/// What the `substance` operation does.
pub const SUBSTANCE_ACTIONS: &[&str] = &["info", "open", "set", "status", "wait", "tools"];

/// A `studio_preset.apply` naming an installed preset by `id` rather than
/// carrying it: the preset read in, so the command is whole - what the
/// journal records, and what replays.
/// `{"reports": [[1, 0, 0, ...], ...], "seconds": 1}`: each report a list
/// of bytes, 0 to 255.
fn spacemouse_input(value: &Value) -> Result<SpaceMouseInput, ProtocolError> {
    let bad = || ProtocolError("`spacemouse` is {\"reports\": [[id, byte, ...], ...], \"seconds\": n}, each byte 0 to 255".into());
    let mut reports = Vec::new();
    for report in value.get("reports").map(|r| r.as_array()).unwrap_or(&[]) {
        let mut bytes = Vec::new();
        for byte in report.as_array() {
            let b = byte.as_index().filter(|b| *b < 256).ok_or_else(bad)?;
            bytes.push(b as u8);
        }
        if bytes.is_empty() || bytes.len() > 64 {
            return Err(bad());
        }
        reports.push(bytes);
    }
    let seconds = match value.get("seconds") {
        None => 0.0,
        Some(v) => v.as_f32().filter(|s| s.is_finite() && *s >= 0.0).ok_or_else(bad)?,
    };
    Ok(SpaceMouseInput { reports, seconds })
}

fn with_preset(command: Value) -> Result<Value, ProtocolError> {
    if command.get("command").and_then(|c| c.as_str()) != Some("studio_preset.apply") {
        return Ok(command);
    }
    // The preset itself, or an installed one's id in its place.
    let id = match command.get("preset") {
        Some(Value::String(id)) => id.clone(),
        Some(_) => return Ok(command),
        None => match command.get("id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => return Err(ProtocolError("studio_preset.apply needs an installed preset's id as \"preset\", or the preset itself".into())),
        },
    };
    let (presets, _) = tequila_format::studio_preset::list();
    let Some(found) = presets.iter().find(|p| p.preset.id == id) else {
        let ids: Vec<&str> = presets.iter().map(|p| p.preset.id.as_str()).collect();
        return Err(ProtocolError(format!(
            "no studio preset `{id}` is installed; there are {}",
            if ids.is_empty() { "none".to_string() } else { ids.join(", ") }
        )));
    };
    let Value::Object(mut map) = command else { return Err(ProtocolError("a command is an object".into())) };
    map.remove("id");
    map.insert("preset".into(), tequila_format::studio_preset::to_inline(&found.preset));
    Ok(Value::Object(map))
}

/// The library's actions, for the protocol and its documentation.
pub const LIBRARY_ACTIONS: &[&str] = &["apply", "save", "import", "export", "delete"];

/// Changes to the application's panels. Absent fields are left alone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UiChange {
    pub render_window: Option<bool>,
    pub console: Option<bool>,
    pub inspector: Option<bool>,
    /// An Inspector tab to switch to, by name: scene, material, environment,
    /// lighting, camera, image, scripting. Showing a tab shows the Inspector.
    pub tab: Option<String>,
    pub library: Option<bool>,
    /// The Library's section: `materials` or `snapshots`. Showing one
    /// shows the Library.
    pub library_section: Option<String>,
    /// `wireframe`, `solid`, `material` or `rendered`.
    pub viewport: Option<String>,
    /// Hide everything but the viewport.
    pub full_viewport: Option<bool>,
    /// Presentation mode: the configurator over the view, full screen.
    pub present: Option<bool>,
    /// Draw the parts' own edges in Solid and Material Preview.
    pub edges: Option<bool>,
    /// Show the viewport at the render's shape, the rest left empty, so
    /// what is in the view is what the render will hold.
    pub frame: Option<bool>,
    /// The shortcuts window.
    pub help: Option<bool>,
    /// `light`, `dark` or `auto`.
    pub mode: Option<String>,
    /// A theme by name: graphite, ocean, orchid, ember, meadow, rose.
    pub theme: Option<String>,
    /// The appearance picker, open.
    pub appearance: Option<bool>,
    /// Help > About Tequila, open.
    pub about: Option<bool>,
    /// Help > Test this computer, open and running: opening it starts the
    /// checks, unless they are running already.
    pub self_test: Option<bool>,
    /// A menu of the menu bar, open: `file` or `view`. What the documentation shows
    /// a menu with.
    pub menu: Option<String>,
    /// The start screen, open.
    pub start_screen: Option<bool>,
    /// Help > Check for updates, open. Opening it asks GitHub for the
    /// latest release.
    pub updates: Option<bool>,
    /// The Assistant's Connect an AI window, open: AI apps and providers.
    pub connect: Option<bool>,
    /// The UV editor, open on the selected part.
    pub uv_editor: Option<bool>,
    /// The UV editor driven from a script (`uv`): on a part or a label, its
    /// tool and zoom, what is selected, and a drag held for a screenshot.
    pub uv: Option<UvEditorChange>,
    /// The Material Graph editor: `Some(Some(material))` opens it on that
    /// material, `Some(None)` (from `false`) closes it.
    pub graph: Option<Option<u64>>,
    /// A Studio held in the Studios tab as though being dragged, for a
    /// screenshot: the Studio, and the slot it would land in (0 above the
    /// first). `Some(None)` (from `false`) lets it go; nothing moves.
    pub studio_drag: Option<Option<[usize; 2]>>,
    /// The Preferences window (the 3D mouse's settings), open.
    pub preferences: Option<bool>,
    /// A 3D mouse's input, made up: reports as a device would send them.
    pub spacemouse: Option<SpaceMouseInput>,
    /// P's Screenshot, as the key takes it: the viewport's picture saved
    /// into the screenshot folder, with its notice.
    pub save_screenshot: Option<bool>,
    /// Where Screenshots go, as View › Screenshot folder… chooses it; kept
    /// in the application's settings.
    pub screenshot_folder: Option<String>,
    /// The window's inner size, in points.
    pub window_size: Option<[f32; 2]>,
}

/// Synthetic 3D mouse input, for scripts and tests without a device: HID
/// input reports, each its report id and then its bytes (report 1, six
/// signed 16-bit little-endian numbers for translation or twelve for all
/// six axes; report 2, rotation; report 3, a button mask), read as a
/// device's are. The motion they end on is held for `seconds`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpaceMouseInput {
    pub reports: Vec<Vec<u8>>,
    pub seconds: f32,
}

/// The UV editor, as a script asks for it. Absent fields are left alone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UvEditorChange {
    /// Open on this part.
    pub part: Option<u64>,
    /// Open on this label: its material, and its place in the material's
    /// list. A part with no layout is unwrapped, as the Labels tab's button
    /// does.
    pub label: Option<(u64, usize)>,
    /// `select`, `move`, `rotate`, `scale` or `measure`.
    pub tool: Option<String>,
    /// How far in: 1 is the layout's square filling the view.
    pub zoom: Option<f32>,
    /// Everything framed (`"zoom": "fit"`).
    pub fit: bool,
    /// These labels selected, by their place in the material's list.
    pub select: Option<Vec<usize>>,
    /// A texture on the layout selected, by the parameter it drives.
    pub texture: Option<String>,
    /// An island selected, by its number from 0, as a click picks it.
    pub island: Option<usize>,
    /// The shortcuts popover.
    pub help: Option<bool>,
    /// Where the pointer is shown, on the layout; `Some(None)` gives it back.
    pub pointer: Option<Option<[f32; 2]>>,
    /// A drag with the editor's tool held from the first point of the layout
    /// to the second; `Some(None)` lets it go without finishing it.
    pub drag: Option<Option<[[f32; 2]; 2]>>,
    /// Finish the held drag, as letting go of the button does.
    pub release: bool,
    /// The selected labels aligned as the Align panel does: `left`,
    /// `centre`, `right`, `top`, `middle` or `bottom`.
    pub align: Option<String>,
    /// A click at a point of the layout, as a person's: it picks a label, a
    /// guide or an island.
    pub click: Option<[f32; 2]>,
    /// The view framed on a box of the layout, from one corner to the other.
    pub frame: Option<[[f32; 2]; 2]>,
    /// What a click picks: `labels`, `islands`, `edges` or `vertices`.
    pub mode: Option<String>,
    /// These islands selected, by number, in the Islands mode.
    pub islands: Option<Vec<usize>>,
    /// These edges selected, each by the mesh vertices at its ends, in the
    /// Edges mode.
    pub edges: Option<Vec<[usize; 2]>>,
    /// These vertices selected, by mesh vertex, in the Vertices mode.
    pub vertices: Option<Vec<usize>>,
    /// Everything of the mode's kind selected, as Ctrl+A does.
    pub select_all: bool,
    /// The selected edges cut, as the Cut button does.
    pub cut: bool,
    /// The selected seams, or the seams between the selected islands,
    /// stitched.
    pub stitch: bool,
    /// The selected vertices pinned (`true`) or unpinned.
    pub pin: Option<bool>,
    /// The selected islands laid out flat again with their pins; `repack`
    /// packs the layout after.
    pub reflatten: bool,
    pub repack: Option<bool>,
    /// The selected islands, or all of them, packed as the Pack popover
    /// says, with any of its settings changed first.
    pub pack: Option<UvPack>,
    /// The Pack popover, open.
    pub pack_options: Option<bool>,
    /// The selected islands, or all, turned square; scaled to one density;
    /// laid out flat again keeping their cuts.
    pub straighten: bool,
    pub even: bool,
    pub unwrap: bool,
    /// The selection moved, turned, scaled, flipped or turned a quarter, as
    /// the Transform panel does it.
    pub transform: Option<UvTransform>,
    /// The stretch heatmap: `area`, `angle` or `off`.
    pub heatmap: Option<String>,
    /// The UV set shown.
    pub set: Option<usize>,
    /// `add`, `copy`, `delete`, `up` or `down`: what the set picker does to
    /// the set shown.
    pub set_action: Option<String>,
    /// The set picker's list, open.
    pub set_menu: Option<bool>,
    /// The layout written as a picture, as the Export dialog does.
    pub export: Option<UvExport>,
    /// The Export dialog, open.
    pub export_dialog: Option<bool>,
    /// These faces of the part selected in the viewport, as picked there at
    /// Face level.
    pub faces: Option<Vec<usize>>,
    /// The islands of the faces selected in the viewport selected here.
    pub from_faces: bool,
    /// The faces selected in the viewport cut out of the layout as an
    /// island of their own.
    pub cut_faces: bool,
    /// A face of the part held under the viewport's pointer, as if a person
    /// pointed at it there; `Some(None)` gives the pointer back.
    pub hover_face: Option<Option<usize>>,
    /// The selected labels put on the island last clicked: `fit`, `centre`,
    /// `along` or `square`, as the Islands panel's buttons do.
    pub place: Option<String>,
    /// The arranging tools, the Select menu and what the editor shows: keys
    /// of this same object, read apart (`uv_arrange`).
    pub arrange: UvArrange,
    /// The shape tools' keys as given, in the order of [`SHAPE_KEYS`]: the
    /// UV editor reads each - its options, and the tool it runs - as the
    /// Unwrap, Shape and Symmetry panels do.
    pub shape: Vec<(String, Value)>,
}

mod uv_arrange;
pub use uv_arrange::{UvArrange, UvCursor};

/// The UV editor's shape tools, as `ui.uv` names them.
pub const SHAPE_KEYS: &[&str] = &[
    "shape_panel", "unwrap_options", "shape_options", "straight", "grid", "rectangle", "project", "relax", "circle", "grow", "weld",
    "match_seams", "cut_angle", "symmetrize", "snap_mirror", "select_side", "select_mirror",
];

/// A pack's settings, as the Pack popover has them. Absent fields keep the
/// popover's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UvPack {
    /// The gap between islands, in `unit`.
    pub margin: Option<f32>,
    /// `mm`, or `px` of a texture `texture` pixels a side.
    pub unit: Option<String>,
    pub texture: Option<u32>,
    pub rotate: Option<bool>,
    /// `square`, the layout's square, or `guides`, the box between them.
    pub into: Option<String>,
}

/// A move, turn, scale or flip of the selection, about the Transform
/// panel's chosen point.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UvTransform {
    /// Across and down, in the layout's units.
    pub offset: Option<[f32; 2]>,
    /// Degrees, anticlockwise as the panel counts.
    pub turn: Option<f32>,
    /// Across and down; one number for both.
    pub scale: Option<[f32; 2]>,
    /// `horizontal` or `vertical`.
    pub flip: Option<String>,
    /// Quarter turns, clockwise as the editor shows them; negative the
    /// other way.
    pub quarter: Option<i32>,
}

/// The layout written as a picture, as the Export dialog writes it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UvExport {
    pub path: String,
    pub size: Option<u32>,
    /// `islands`, `none`, `area` or `angle`.
    pub fill: Option<String>,
    /// `"#rrggbb"`.
    pub line: Option<String>,
    pub transparent: Option<bool>,
}

fn point(value: &Value) -> Option<[f32; 2]> {
    match value.as_array() {
        [u, v] => Some([u.as_f64()? as f32, v.as_f64()? as f32]).filter(|p| p.iter().all(|c| c.is_finite())),
        _ => None,
    }
}

fn uv_editor_change(value: &Value) -> Result<UvEditorChange, ProtocolError> {
    if !matches!(value, Value::Object(_)) {
        return Err(ProtocolError("`uv` is an object, such as {\"tool\": \"measure\"}".into()));
    }
    let (zoom, fit) = match value.get("zoom") {
        None | Some(Value::Null) => (None, false),
        Some(Value::String(s)) if s == "fit" => (None, true),
        Some(v) => (Some(v.as_f64().filter(|z| z.is_finite() && *z > 0.0).ok_or_else(|| ProtocolError("`zoom` is a number above 0, or \"fit\"".into()))? as f32), false),
    };
    let label = optional(value, "label", "a material and a label's place, [material, label]", |v| match v.as_array() {
        [m, l] => Some((m.as_index()? as u64, l.as_index()?)),
        _ => None,
    })?;
    let select = optional(value, "select", "a list of labels' places", |v| v.as_array().iter().map(Value::as_index).collect::<Option<Vec<_>>>())?;
    let pointer = match value.get("pointer") {
        None => None,
        Some(Value::Null) | Some(Value::Bool(false)) => Some(None),
        Some(v) => Some(Some(point(v).ok_or_else(|| ProtocolError("`pointer` is a point of the layout, [u, v], or false".into()))?)),
    };
    let drag = match value.get("drag") {
        None => None,
        Some(Value::Null) | Some(Value::Bool(false)) => Some(None),
        Some(v) => {
            let ends = match v.as_array() {
                [a, b] => point(a).zip(point(b)).map(|(a, b)| [a, b]),
                _ => None,
            };
            Some(Some(ends.ok_or_else(|| ProtocolError("`drag` is two points of the layout, [[u, v], [u, v]], or false".into()))?))
        }
    };
    Ok(UvEditorChange {
        part: opt_index(value, "part")?.map(|n| n as u64),
        label,
        tool: opt_string(value, "tool")?,
        zoom,
        fit,
        select,
        texture: opt_string(value, "texture")?,
        island: opt_index(value, "island")?,
        help: opt_bool(value, "help")?,
        pointer,
        drag,
        release: opt_bool(value, "release")?.unwrap_or(false),
        align: opt_string(value, "align")?,
        click: optional(value, "click", "a point of the layout, [u, v]", point)?,
        frame: optional(value, "frame", "two points of the layout, [[u, v], [u, v]]", |v| match v.as_array() {
            [a, b] => Some([point(a)?, point(b)?]),
            _ => None,
        })?,
        mode: opt_string(value, "mode")?,
        islands: indices(value, "islands")?,
        edges: optional(value, "edges", "a list of edges, each the mesh vertices at its ends, [[a, b], ...]", |v| {
            v.as_array()
                .iter()
                .map(|e| match e.as_array() {
                    [a, b] => Some([a.as_index()?, b.as_index()?]),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()
        })?,
        vertices: indices(value, "vertices")?,
        select_all: opt_bool(value, "select_all")?.unwrap_or(false),
        cut: opt_bool(value, "cut")?.unwrap_or(false),
        stitch: opt_bool(value, "stitch")?.unwrap_or(false),
        pin: opt_bool(value, "pin")?,
        reflatten: opt_bool(value, "reflatten")?.unwrap_or(false),
        repack: opt_bool(value, "repack")?,
        pack: match value.get("pack") {
            None | Some(Value::Null) | Some(Value::Bool(false)) => None,
            Some(Value::Bool(true)) => Some(UvPack::default()),
            Some(object @ Value::Object(_)) => Some(UvPack {
                margin: opt_number(object, "margin")?,
                unit: opt_string(object, "unit")?,
                texture: opt_index(object, "texture")?.map(|n| n as u32),
                rotate: opt_bool(object, "rotate")?,
                into: opt_string(object, "into")?,
            }),
            Some(_) => return Err(ProtocolError("`pack` is true, or the popover's settings: {\"margin\": 2, \"unit\": \"mm\", \"rotate\": true, \"into\": \"guides\"}".into())),
        },
        pack_options: opt_bool(value, "pack_options")?,
        straighten: opt_bool(value, "straighten")?.unwrap_or(false),
        even: opt_bool(value, "even")?.unwrap_or(false),
        unwrap: opt_bool(value, "unwrap")?.unwrap_or(false),
        transform: match value.get("transform") {
            None | Some(Value::Null) => None,
            Some(object @ Value::Object(_)) => Some(UvTransform {
                offset: optional(object, "offset", "a move of the layout, [du, dv]", point)?,
                turn: opt_number(object, "turn")?,
                scale: match object.get("scale") {
                    None | Some(Value::Null) => None,
                    Some(Value::Number(k)) if k.is_finite() && *k > 0.0 => Some([*k as f32; 2]),
                    Some(v) => Some(point(v).filter(|p| p.iter().all(|k| *k > 0.0)).ok_or_else(|| ProtocolError("`scale` is a number above 0, or [across, down]".into()))?),
                },
                flip: opt_string(object, "flip")?,
                quarter: optional(object, "quarter", "a whole number of quarter turns", |v| v.as_f64().filter(|n| n.fract() == 0.0 && n.abs() <= 8.0).map(|n| n as i32))?,
            }),
            Some(_) => return Err(ProtocolError("`transform` is an object, such as {\"offset\": [0.1, 0], \"turn\": 15}".into())),
        },
        heatmap: opt_string(value, "heatmap")?,
        set: opt_index(value, "set")?,
        set_action: opt_string(value, "set_action")?,
        set_menu: opt_bool(value, "set_menu")?,
        export: match value.get("export") {
            None | Some(Value::Null) => None,
            Some(object @ Value::Object(_)) => Some(UvExport {
                path: opt_string(object, "path")?.ok_or_else(|| ProtocolError("`export` needs a `path`, .svg or .png".into()))?,
                size: opt_index(object, "size")?.map(|n| n as u32),
                fill: opt_string(object, "fill")?,
                line: opt_string(object, "line")?,
                transparent: opt_bool(object, "transparent")?,
            }),
            Some(_) => return Err(ProtocolError("`export` is an object, such as {\"path\": \"C:/work/layout.png\", \"size\": 2048}".into())),
        },
        export_dialog: opt_bool(value, "export_dialog")?,
        faces: indices(value, "faces")?,
        from_faces: opt_bool(value, "from_faces")?.unwrap_or(false),
        cut_faces: opt_bool(value, "cut_faces")?.unwrap_or(false),
        hover_face: match value.get("hover_face") {
            None => None,
            Some(Value::Null) | Some(Value::Bool(false)) => Some(None),
            Some(v) => Some(Some(v.as_index().ok_or_else(|| ProtocolError("`hover_face` is a face of the part, by number, or false".into()))?)),
        },
        place: opt_string(value, "place")?,
        arrange: uv_arrange::uv_arrange(value)?,
        shape: SHAPE_KEYS.iter().filter_map(|&key| value.get(key).map(|v| (key.to_string(), v.clone()))).collect(),
    })
}

/// A list of whole numbers.
fn indices(value: &Value, key: &str) -> Result<Option<Vec<usize>>, ProtocolError> {
    optional(value, key, "a list of whole numbers", |v| v.as_array().iter().map(Value::as_index).collect::<Option<Vec<_>>>())
}

/// A background render. Absent fields come from the document's settings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QueueRender {
    pub name: Option<String>,
    pub width: Option<usize>,
    pub height: Option<usize>,
    pub samples: Option<u32>,
    pub folder: Option<String>,
    pub filename: Option<String>,
    pub denoise: Option<bool>,
    /// Which saved camera, by index. The active one when absent.
    pub camera: Option<usize>,
    /// The engine, by its project-file name: `tequila`, `cycles-cpu` or
    /// `cycles-gpu`. The document's when absent.
    pub engine: Option<String>,
    /// One render for every valid combination of the configurator, each
    /// from its own camera, instead of one of the scene as it is.
    pub configurations: Option<bool>,
    /// Sent to the render nodes (the Render window's Network rendering)
    /// rather than rendered here.
    pub network: Option<bool>,
    /// A Studio - a view - rendered as it says, by its name or its index:
    /// applied for the render and taken back, so the scene is as it was.
    pub studio: Option<String>,
    /// Every Studio, one render each, named for its view.
    pub studios: Option<bool>,
}

/// Where to put the camera. Every field is optional and starts from where
/// the camera already is: a view name, then orbit angles around `at`, or an
/// exact `from`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Look {
    /// front, back, left, right, top, bottom or iso.
    pub view: Option<String>,
    pub from: Option<[f32; 3]>,
    pub at: Option<[f32; 3]>,
    /// Degrees around +Y, 0 looking from the front.
    pub yaw: Option<f32>,
    /// Degrees above the horizon.
    pub pitch: Option<f32>,
    pub distance: Option<f32>,
    pub focal_length: Option<f32>,
}

/// The read side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Query {
    /// API discovery, conventions and parameter catalogs.
    Api,
    /// Document state plus semantic summaries, captured in one UI frame.
    Context,
    /// Counts, units and overall bounds: enough to know whether the thing a
    /// script expected is even loaded.
    Scene,
    /// Every part, with the facts a script needs to *find* one: its name, its
    /// material, and where it is in the world.
    ///
    /// The bounds are the point. A split model arrives as `part 1 … part 355`
    /// with no semantics, and the only way to say "the small dark one at the
    /// front" is to have somewhere to say it from.
    Nodes,
    Materials,
    Selection,
    Camera,
    /// Every command this build accepts.
    ///
    /// So a client can find out what it may ask for without reading the
    /// source of the application it is talking to.
    Commands,
    /// Which panels are open and what the viewport is showing.
    Ui,
    /// The background render queue: every job, its state and progress, and
    /// where its file went.
    Jobs,
    /// What undo and redo would do, and whether there is unsaved work.
    History,
    /// The material library: every preset, shipped and the user's own, by
    /// name, category, tags and type.
    Library,
}

#[derive(Debug)]
pub struct Request {
    pub id: u64,
    pub token: String,
    pub operation: Operation,
    /// Who sent it, for the person watching.
    ///
    /// Filled in by the server from the connection rather than taken from the
    /// request, because a client that could name itself would be a client that
    /// could name itself something else. Empty for a request built inside the
    /// application, which the console labels for itself.
    pub peer: String,
}

/// Why a request could not be understood.
///
/// Separate from a request that was understood and *failed*: the first is the
/// client's bug and the second is the scene's answer, and a client debugging
/// itself needs to tell them apart.
#[derive(Debug, PartialEq, Eq)]
pub struct ProtocolError(pub String);

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn field<'a>(value: &'a Value, key: &str) -> Result<&'a Value, ProtocolError> {
    value
        .get(key)
        .ok_or_else(|| ProtocolError(format!("the request has no `{key}`")))
}

fn string(value: &Value, key: &str) -> Result<String, ProtocolError> {
    field(value, key)?
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| ProtocolError(format!("`{key}` is not a string")))
}

fn index(value: &Value, key: &str, fallback: usize) -> usize {
    value
        .get(key)
        .and_then(|v| v.as_index())
        .unwrap_or(fallback)
}

/// An optional field that, when present, must be the right shape. Silently
/// ignoring a `"width":"1920"` would render at the default size and leave the
/// client wondering why.
fn optional<T>(
    value: &Value,
    key: &str,
    kind: &str,
    read: impl Fn(&Value) -> Option<T>,
) -> Result<Option<T>, ProtocolError> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(field) => read(field)
            .map(Some)
            .ok_or_else(|| ProtocolError(format!("`{key}` must be {kind}"))),
    }
}

fn opt_bool(value: &Value, key: &str) -> Result<Option<bool>, ProtocolError> {
    optional(value, key, "true or false", Value::as_bool)
}

fn opt_string(value: &Value, key: &str) -> Result<Option<String>, ProtocolError> {
    optional(value, key, "a string", |v| v.as_str().map(str::to_string))
}

fn opt_index(value: &Value, key: &str) -> Result<Option<usize>, ProtocolError> {
    optional(value, key, "a whole number", Value::as_index)
}

fn opt_number(value: &Value, key: &str) -> Result<Option<f32>, ProtocolError> {
    optional(value, key, "a number", |v| v.as_f64().filter(|n| n.is_finite()).map(|n| n as f32))
}

fn opt_vector(value: &Value, key: &str) -> Result<Option<[f32; 3]>, ProtocolError> {
    optional(value, key, "three numbers, [x, y, z]", |v| match v.as_array() {
        [x, y, z] => Some([x.as_f64()? as f32, y.as_f64()? as f32, z.as_f64()? as f32]),
        _ => None,
    })
}

/// The `id` of a request that could not be parsed, when it has one — so
/// the refusal answers that request, not id 0. A client matches replies to
/// requests by id, and one that gets an answer to a request it never sent
/// has to assume the connection is broken.
pub fn request_id(line: &str) -> u64 {
    json::parse(line.as_bytes())
        .ok()
        .and_then(|value| value.get("id").and_then(|v| v.as_index()))
        .unwrap_or(0) as u64
}

pub fn parse_request(line: &str) -> Result<Request, ProtocolError> {
    let value = json::parse(line.as_bytes())
        .map_err(|e| ProtocolError(format!("the request is not JSON: {e}")))?;

    let id = value.get("id").and_then(|v| v.as_index()).unwrap_or(0) as u64;
    let token = value
        .get("token")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let operation = parse_operation(&value)?;

    Ok(Request {
        id,
        token,
        operation,
        peer: String::new(),
    })
}

/// How deep batches may nest. Deep enough for any real script; shallow
/// enough that a malicious or runaway request cannot exhaust the stack.
const BATCH_DEPTH: usize = 8;

/// One operation, from an object with an `op` — a whole request, or one
/// entry of a batch.
pub fn parse_operation(value: &Value) -> Result<Operation, ProtocolError> {
    parse_operation_at(value, 0)
}

/// `material.add_decal` naming a video takes a still of it, at `time`
/// seconds (the first frame without one), kept among Tequila's settings
/// under "Video frames"; the command then names that picture, so the
/// journal and an undo replay a picture rather than decode the video again.
fn video_label(command: &Value) -> Result<Value, ProtocolError> {
    let mut command = command.clone();
    let is_label = command.get("command").and_then(Value::as_str) == Some("material.add_decal");
    let video = command.get("path").and_then(Value::as_str).filter(|p| tequila_texture::video::is_video(p)).map(str::to_string);
    if let (true, Some(video), Value::Object(fields)) = (is_label, video, &mut command) {
        let seconds = fields.remove("time").and_then(|t| t.as_f64()).unwrap_or(0.0);
        let folder = tequila_format::library::settings_directory().join("Video frames");
        let still = tequila_texture::video::still(&video, seconds, folder).map_err(|e| ProtocolError(format!("the video's frame could not be taken: {e}")))?;
        fields.insert("path".into(), Value::String(still.display().to_string()));
    }
    Ok(command)
}

fn parse_operation_at(value: &Value, depth: usize) -> Result<Operation, ProtocolError> {
    let op = string(value, "op")?;

    let operation = match op.as_str() {
        "ping" => Operation::Ping,
        "undo" => Operation::Undo,
        "redo" => Operation::Redo,
        "apply" => {
            // The payload is a journal line. Re-serializing it rather than
            // reaching into the parsed value keeps exactly one decoder for
            // commands, which is what makes the journal's exhaustive
            // round-trip test cover this path too.
            let command = with_preset(video_label(field(value, "command")?)?)?;
            let text = json_write::to_string(&command);
            let command = tequila_format::journal::decode(&text)
                .map_err(|e| ProtocolError(format!("the command could not be read: {e}")))?;
            Operation::Apply(Box::new(command))
        }
        "query" => {
            let what = string(value, "what")?;
            let query = match what.as_str() {
                "api" => Query::Api,
                "context" => Query::Context,
                "scene" => Query::Scene,
                "nodes" => Query::Nodes,
                "materials" => Query::Materials,
                "selection" => Query::Selection,
                "camera" => Query::Camera,
                "commands" => Query::Commands,
                "ui" => Query::Ui,
                "jobs" => Query::Jobs,
                "history" => Query::History,
                "library" => Query::Library,
                other => {
                    return Err(ProtocolError(format!(
                        "`{other}` is not something this understands; try \
                         context, scene, nodes, materials, selection, camera, \
                         commands, ui, jobs, history, library or api"
                    )))
                }
            };
            Operation::Query(query)
        }
        "select" => {
            let Value::Array(items) = field(value, "nodes")? else { return Err(ProtocolError("`nodes` must be an array of integer IDs".into())); };
            let nodes = items
                .iter()
                .map(|v| v.as_index().map(|i| i as u64).ok_or_else(|| ProtocolError("Each node ID must be a nonnegative integer".into())))
                .collect::<Result<Vec<_>, _>>()?;
            Operation::Select(nodes)
        }
        "import" => Operation::Import {
            path: string(value, "path")?,
            parent: value.get("parent").and_then(|v| v.as_index()).map(|i| i as u64),
        },
        "reimport" => Operation::Reimport {
            node: value.get("node").and_then(|v| v.as_index()).map(|i| i as u64),
            path: value.get("path").and_then(|v| v.as_str()).map(str::to_string),
            tessellation: value.get("tessellation").and_then(|v| v.as_str()).map(str::to_string),
        },
        "texture_set" => Operation::TextureSet {
            path: string(value, "path")?,
            nodes: match value.get("nodes") {
                None => Vec::new(),
                Some(Value::Array(items)) => items
                    .iter()
                    .map(|v| v.as_index().map(|i| i as u64).ok_or_else(|| ProtocolError("Each node ID must be a nonnegative integer".into())))
                    .collect::<Result<Vec<_>, _>>()?,
                Some(_) => return Err(ProtocolError("`nodes` must be an array of integer IDs".into())),
            },
        },
        "livelink" => Operation::LiveLink(value.clone()),
        "studio_preset" => {
            let action = string(value, "action")?.to_ascii_lowercase();
            if !STUDIO_PRESET_ACTIONS.contains(&action.as_str()) {
                return Err(ProtocolError(format!(
                    "`{action}` is not a studio preset action; the actions are {}",
                    STUDIO_PRESET_ACTIONS.join(", ")
                )));
            }
            Operation::StudioPreset(value.clone())
        }
        "substance" => {
            let action = string(value, "action")?.to_ascii_lowercase();
            if !SUBSTANCE_ACTIONS.contains(&action.as_str()) {
                return Err(ProtocolError(format!(
                    "`{action}` is not a substance action; the actions are {}",
                    SUBSTANCE_ACTIONS.join(", ")
                )));
            }
            Operation::Substance(value.clone())
        }
        "library" => {
            let action = string(value, "action")?.to_ascii_lowercase();
            if !LIBRARY_ACTIONS.contains(&action.as_str()) {
                return Err(ProtocolError(format!("`{action}` is not a library action; the actions are {}", LIBRARY_ACTIONS.join(", "))));
            }
            let nodes = match value.get("nodes") {
                None => Vec::new(),
                Some(Value::Array(items)) => items
                    .iter()
                    .map(|v| v.as_index().map(|i| i as u64).ok_or_else(|| ProtocolError("Each node ID must be a nonnegative integer".into())))
                    .collect::<Result<Vec<_>, _>>()?,
                Some(_) => return Err(ProtocolError("`nodes` must be an array of integer IDs".into())),
            };
            let tags = match value.get("tags") {
                None => Vec::new(),
                Some(Value::Array(items)) => items
                    .iter()
                    .map(|v| v.as_str().map(str::to_string).ok_or_else(|| ProtocolError("Each tag must be a string".into())))
                    .collect::<Result<Vec<_>, _>>()?,
                Some(_) => return Err(ProtocolError("`tags` must be an array of strings".into())),
            };
            let request = LibraryRequest {
                action,
                name: opt_string(value, "name")?,
                nodes,
                unlink: opt_bool(value, "unlink")?.unwrap_or(false),
                material: value.get("material").and_then(|v| v.as_index()).map(|i| i as u64),
                category: opt_string(value, "category")?,
                tags,
                path: opt_string(value, "path")?,
                size: opt_number(value, "size")?,
            };
            let needs = |field: &str, present: bool| {
                if present {
                    Ok(())
                } else {
                    Err(ProtocolError(format!("library {} needs `{field}`", request.action)))
                }
            };
            match request.action.as_str() {
                "apply" | "delete" => needs("name", request.name.is_some())?,
                "save" => needs("material", request.material.is_some())?,
                "import" => needs("path", request.path.is_some())?,
                "export" => {
                    needs("name", request.name.is_some())?;
                    needs("path", request.path.is_some())?;
                }
                _ => {}
            }
            Operation::Library(request)
        }
        "material" => {
            let action = string(value, "action")?.to_ascii_lowercase();
            if !crate::material::MATERIAL_ACTIONS.contains(&action.as_str()) {
                return Err(ProtocolError(format!("`{action}` is not a material action; the actions are {}", crate::material::MATERIAL_ACTIONS.join(", "))));
            }
            let ids = |key: &str| -> Result<Vec<u64>, ProtocolError> {
                match value.get(key) {
                    None => Ok(Vec::new()),
                    Some(Value::Array(items)) => items
                        .iter()
                        .map(|v| v.as_index().map(|i| i as u64).ok_or_else(|| ProtocolError(format!("each of `{key}` must be a nonnegative integer id"))))
                        .collect(),
                    Some(_) => Err(ProtocolError(format!("`{key}` must be an array of integer ids"))),
                }
            };
            let tags = match value.get("tags") {
                None => Vec::new(),
                Some(Value::Array(items)) => items.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
                Some(_) => return Err(ProtocolError("`tags` must be an array of strings".into())),
            };
            let request = MaterialRequest {
                action,
                recipe: value.get("recipe").cloned(),
                material: value.get("material").and_then(|v| v.as_index()).map(|i| i as u64),
                nodes: ids("nodes")?,
                name: opt_string(value, "name")?,
                category: opt_string(value, "category")?,
                tags,
                size: opt_index(value, "size")?,
                shape: opt_string(value, "shape")?,
                radius: opt_number(value, "radius")?,
                path: opt_string(value, "path")?,
            };
            let needs = |field: &str, present: bool| {
                if present {
                    Ok(())
                } else {
                    Err(ProtocolError(format!("material {} needs `{field}`", request.action)))
                }
            };
            match request.action.as_str() {
                "create" => needs("recipe", request.recipe.is_some())?,
                "update" => {
                    needs("material", request.material.is_some())?;
                    needs("recipe", request.recipe.is_some())?;
                }
                "recipe" | "save" => needs("material", request.material.is_some())?,
                "preview" => needs("recipe or material", request.recipe.is_some() || request.material.is_some())?,
                _ => {}
            }
            Operation::Material(request)
        }
        "plugin" => {
            let action = string(value, "action")?.to_ascii_lowercase();
            if !PLUGIN_ACTIONS.contains(&action.as_str()) {
                return Err(ProtocolError(format!("`{action}` is not a plugin action; the actions are {}", PLUGIN_ACTIONS.join(", "))));
            }
            let files = match value.get("files") {
                None => Vec::new(),
                Some(Value::Object(map)) => map
                    .iter()
                    .map(|(path, text)| match text {
                        Value::String(text) => Ok((path.clone(), text.clone())),
                        _ => Err(ProtocolError(format!("`files.{path}` must be the file's text"))),
                    })
                    .collect::<Result<_, _>>()?,
                Some(_) => return Err(ProtocolError("`files` must be an object of path: text".into())),
            };
            let values = match value.get("values") {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Object(map)) => map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                Some(_) => return Err(ProtocolError("`values` must be an object of field: value".into())),
            };
            let request = PluginRequest {
                action,
                plugin: opt_string(value, "plugin")?,
                ask: opt_bool(value, "ask")?.unwrap_or(false),
                files,
                command: opt_string(value, "command")?,
                values,
                lines: opt_index(value, "lines")?,
            };
            let needs = |field: &str, present: bool| {
                if present {
                    Ok(())
                } else {
                    Err(ProtocolError(format!("plugin {} needs `{field}`", request.action)))
                }
            };
            match request.action.as_str() {
                "guide" | "list" => {}
                "write" => {
                    needs("plugin", request.plugin.is_some())?;
                    needs("files", !request.files.is_empty())?;
                }
                "run" => {
                    needs("plugin", request.plugin.is_some())?;
                    needs("command", request.command.is_some())?;
                }
                _ => needs("plugin", request.plugin.is_some())?,
            }
            Operation::Plugin(request)
        }
        "export" => Operation::Export {
            format: value.get("format").and_then(|v| v.as_str()).unwrap_or("glb").to_ascii_lowercase(),
            path: string(value, "path")?,
            live: opt_bool(value, "live")?.unwrap_or(true),
            photos: opt_bool(value, "photos")?.unwrap_or(false),
            columns: opt_index(value, "columns")?,
            rows: opt_index(value, "rows")?,
            tilt: value.get("tilt").and_then(|v| v.as_f32()),
            transparent: opt_bool(value, "transparent")?.unwrap_or(false),
        },
        "save" => Operation::Save {
            path: value.get("path").and_then(|v| v.as_str()).map(str::to_string),
        },
        "frame" => Operation::Frame {
            // Framing everything is the safe default: a script that has not
            // selected anything wants to see the model, not an empty box.
            all: value.get("all").and_then(|v| v.as_bool()).unwrap_or(true),
        },
        "screenshot" => Operation::Screenshot {
            path: string(value, "path")?,
            // Just the picture, without the panels around it, when a script
            // wants to look at the render rather than at the application.
            viewport: opt_bool(value, "viewport")?.unwrap_or(false),
            window: opt_string(value, "window")?,
        },
        "render" => Operation::Render {
            path: string(value, "path")?,
            width: index(value, "width", 960),
            height: index(value, "height", 600),
            // Saturating: a count past u32 wrapped round to a small one.
            samples: index(value, "samples", 64).min(u32::MAX as usize) as u32,
            wireframe: opt_bool(value, "wireframe")?.unwrap_or(false),
        },
        "batch" => {
            if depth >= BATCH_DEPTH {
                return Err(ProtocolError(format!("batches nest at most {BATCH_DEPTH} deep")));
            }
            let Value::Array(items) = field(value, "ops")? else {
                return Err(ProtocolError("`ops` must be an array of operations".into()));
            };
            let operations = items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    parse_operation_at(item, depth + 1)
                        .map_err(|e| ProtocolError(format!("operation {} of the batch: {e}", i + 1)))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Operation::Batch {
                operations,
                atomic: opt_bool(value, "atomic")?.unwrap_or(true),
            }
        }
        "quit" => Operation::Quit {
            save: opt_bool(value, "save")?.unwrap_or(false),
            discard: opt_bool(value, "discard")?.unwrap_or(false),
        },
        "new" | "open" => Operation::Project {
            action: if op == "new" { ProjectAction::New } else { ProjectAction::Open(string(value, "path")?) },
            save: opt_bool(value, "save")?.unwrap_or(false),
            discard: opt_bool(value, "discard")?.unwrap_or(false),
        },
        "ui" => Operation::Ui(UiChange {
            render_window: opt_bool(value, "render_window")?,
            console: opt_bool(value, "console")?,
            inspector: opt_bool(value, "inspector")?,
            tab: opt_string(value, "tab")?,
            library: opt_bool(value, "library")?,
            library_section: opt_string(value, "library_section")?,
            viewport: opt_string(value, "viewport")?,
            full_viewport: opt_bool(value, "full_viewport")?,
            present: opt_bool(value, "present")?,
            edges: opt_bool(value, "edges")?,
            frame: opt_bool(value, "frame")?,
            help: opt_bool(value, "help")?,
            mode: opt_string(value, "mode")?,
            theme: opt_string(value, "theme")?,
            appearance: opt_bool(value, "appearance")?,
            about: opt_bool(value, "about")?,
            menu: opt_string(value, "menu")?,
            start_screen: opt_bool(value, "start_screen")?,
            updates: opt_bool(value, "updates")?,
            self_test: opt_bool(value, "self_test")?,
            connect: opt_bool(value, "connect")?,
            uv_editor: opt_bool(value, "uv_editor")?,
            uv: value.get("uv").map(uv_editor_change).transpose()?,
            graph: match value.get("graph") {
                None | Some(Value::Null) => None,
                Some(Value::Bool(false)) => Some(None),
                Some(v) => Some(Some(v.as_index().ok_or_else(|| {
                    ProtocolError("`graph` must be a material number, or false to close the editor".into())
                })? as u64)),
            },
            studio_drag: match value.get("studio_drag") {
                None | Some(Value::Null) => None,
                Some(Value::Bool(false)) => Some(None),
                Some(v) => match v.as_array() {
                    [from, slot] => match (from.as_index(), slot.as_index()) {
                        (Some(from), Some(slot)) => Some(Some([from, slot])),
                        _ => return Err(ProtocolError("`studio_drag` is [studio, slot], two numbers, or false".into())),
                    },
                    _ => return Err(ProtocolError("`studio_drag` is [studio, slot], two numbers, or false".into())),
                },
            },
            preferences: opt_bool(value, "preferences")?,
            spacemouse: value.get("spacemouse").map(spacemouse_input).transpose()?,
            save_screenshot: opt_bool(value, "save_screenshot")?,
            screenshot_folder: opt_string(value, "screenshot_folder")?,
            window_size: optional(value, "window_size", "[width, height] in points", |v| match v.as_array() {
                [w, h] => Some([w.as_f32()?, h.as_f32()?]).filter(|[w, h]| w.is_finite() && h.is_finite() && *w >= 1.0 && *h >= 1.0),
                _ => None,
            })?,
        }),
        "queue" => Operation::Queue(QueueRender {
            name: opt_string(value, "name")?,
            width: opt_index(value, "width")?,
            height: opt_index(value, "height")?,
            samples: opt_index(value, "samples")?.map(|n| n as u32),
            folder: opt_string(value, "folder")?,
            filename: opt_string(value, "filename")?,
            denoise: opt_bool(value, "denoise")?,
            camera: opt_index(value, "camera")?,
            engine: opt_string(value, "engine")?,
            configurations: opt_bool(value, "configurations")?,
            network: opt_bool(value, "network")?,
            // A name, or an index given as a number.
            studio: match value.get("studio") {
                Some(v) if v.as_index().is_some() => v.as_index().map(|i| i.to_string()),
                _ => opt_string(value, "studio")?,
            },
            studios: opt_bool(value, "studios")?,
        }),
        "stop" => Operation::Stop,
        "look" => Operation::Look(Look {
            view: opt_string(value, "view")?,
            from: opt_vector(value, "from")?,
            at: opt_vector(value, "at")?,
            yaw: opt_number(value, "yaw")?,
            pitch: opt_number(value, "pitch")?,
            distance: opt_number(value, "distance")?,
            focal_length: opt_number(value, "focal_length")?,
        }),
        other => {
            return Err(ProtocolError(format!(
                "`{other}` is not an operation this understands"
            )))
        }
    };

    Ok(operation)
}

impl Operation {
    /// One short line naming what this would do, for a log a person reads.
    ///
    /// Commands give their own catalog name, which is the same string the
    /// undo history and the journal use — so an entry here can be matched
    /// against both without a translation table.
    pub fn summary(&self) -> String {
        match self {
            Operation::Ping => "ping".into(),
            Operation::Apply(command) => command.name().to_string(),
            Operation::Undo => "undo".into(),
            Operation::Redo => "redo".into(),
            Operation::Query(query) => format!("query {}", query.label()),
            Operation::Select(nodes) => format!("select {} part(s)", nodes.len()),
            Operation::Import { path, .. } => format!("import {path}"),
            Operation::TextureSet { path, .. } => format!("texture set {path}"),
            Operation::Reimport { path, .. } => match path {
                Some(path) => format!("update model from {path}"),
                None => "reload model from its file".to_string(),
            },
            Operation::Save { path } => match path {
                Some(path) => format!("save {path}"),
                None => "save".into(),
            },
            Operation::Frame { all } => {
                if *all {
                    "frame everything".into()
                } else {
                    "frame selection".into()
                }
            }
            Operation::Screenshot { path, .. } => format!("screenshot {path}"),
            Operation::Render { width, height, wireframe: true, .. } => format!("wireframe {width}x{height}"),
            Operation::Render {
                width,
                height,
                samples,
                ..
            } => format!("render {width}x{height} at {samples} spp"),
            Operation::Batch { operations, .. } => format!("batch of {}", operations.len()),
            Operation::Quit { save, discard } => match (save, discard) {
                (true, _) => "quit, saving".into(),
                (_, true) => "quit, discarding changes".into(),
                _ => "quit".into(),
            },
            Operation::Project { action: ProjectAction::New, .. } => "new project".into(),
            Operation::Project { action: ProjectAction::Open(path), .. } => format!("open {path}"),
            Operation::Ui(_) => "change panels".into(),
            Operation::LiveLink(message) => format!("LiveLink {}", message.get("message").and_then(|m| m.as_str()).unwrap_or("message")),
            Operation::StudioPreset(request) => match request.get("preset").and_then(|i| i.as_str()) {
                Some(id) => format!("studio preset {} {id}", request.get("action").and_then(|a| a.as_str()).unwrap_or("")),
                None => format!("studio preset {}", request.get("action").and_then(|a| a.as_str()).unwrap_or("")),
            },
            Operation::Substance(request) => match request.get("path").and_then(|p| p.as_str()) {
                Some(path) => format!("substance {} {path}", request.get("action").and_then(|a| a.as_str()).unwrap_or("")),
                None => format!("substance {}", request.get("action").and_then(|a| a.as_str()).unwrap_or("")),
            },
            Operation::Export { format, path, .. } => format!("export {format} to {path}"),
            Operation::Queue(job) => match &job.name {
                Some(name) => format!("queue render {name}"),
                None => "queue render".into(),
            },
            Operation::Stop => "stop render".into(),
            Operation::Library(request) => match &request.name {
                Some(name) => format!("library {} {name}", request.action),
                None => format!("library {}", request.action),
            },
            Operation::Material(request) => {
                let name = request.recipe.as_ref().and_then(|r| r.get("name")).and_then(Value::as_str);
                match (name, request.material) {
                    (Some(name), _) => format!("material {} {name}", request.action),
                    (None, Some(id)) => format!("material {} {id}", request.action),
                    (None, None) => format!("material {}", request.action),
                }
            }
            Operation::Look(look) => match &look.view {
                Some(view) => format!("look {view}"),
                None => "move camera".into(),
            },
            Operation::Plugin(request) => match (&request.plugin, &request.command) {
                (Some(id), Some(command)) => format!("plugin {} {id} {command}", request.action),
                (Some(id), None) => format!("plugin {} {id}", request.action),
                _ => format!("plugin {}", request.action),
            },
        }
    }
}

impl Query {
    pub fn label(self) -> &'static str {
        match self {
            Query::Api => "api",
            Query::Context => "context",
            Query::Scene => "scene",
            Query::Nodes => "nodes",
            Query::Materials => "materials",
            Query::Selection => "selection",
            Query::Camera => "camera",
            Query::Commands => "commands",
            Query::Ui => "ui",
            Query::Jobs => "jobs",
            Query::History => "history",
            Query::Library => "library",
        }
    }
}

/// What came back.
#[derive(Debug)]
pub enum Reply {
    /// Nothing to say beyond "it worked".
    Done,
    /// An answer, already shaped as JSON by whoever knew the document.
    Value(Value),
    /// Understood, attempted, and did not work.
    Failed(String),
}

pub fn encode_reply(id: u64, reply: &Reply) -> String {
    let mut out = Object::new();
    out.set("id", id as f64);
    match reply {
        Reply::Done => {
            out.set("ok", true);
        }
        Reply::Value(value) => {
            out.set("ok", true).set("result", value.clone());
        }
        Reply::Failed(message) => {
            out.set("ok", false).set("error", message.clone());
        }
    }
    one_line(&out.build())
}

/// The shared writer indents and wraps, and this protocol is line-oriented,
/// so the two have to be reconciled somewhere.
///
/// Flattened the same way [`tequila_format::journal`] flattens a record, and
/// for a related reason: there it is so a truncated line cannot be mistaken
/// for a split one, here it is so a reader can call `read_line` and get a
/// whole answer. Safe because the writer escapes newlines inside strings, so
/// the only newlines left are the ones it added itself.
fn one_line(value: &Value) -> String {
    json_write::to_string(value)
        .split('\n')
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shipped_example_is_a_valid_request() {
        let examples = json::parse(include_bytes!("examples.json")).unwrap();
        for example in examples.as_array() {
            let request = json_write::to_string(example.get("request").unwrap());
            parse_request(&request).unwrap_or_else(|e| panic!("{request}: {e}"));
        }
    }

    /// The UV editor's shape tool keys come through as given, in the order
    /// the editor reads them, for it to check and carry out.
    #[test]
    fn the_uv_shape_keys_are_carried_in_their_order() {
        let request = parse_request(r#"{"op":"ui","uv":{"weld":{"within":2},"straight":true,"islands":[1],"shape_panel":"shape"}}"#).unwrap();
        let Operation::Ui(UiChange { uv: Some(uv), .. }) = request.operation else { panic!("a ui change") };
        let keys: Vec<&str> = uv.shape.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["shape_panel", "straight", "weld"]);
        assert_eq!(uv.shape[2].1.get("within").and_then(|v| v.as_f64()), Some(2.0));
        assert_eq!(uv.islands, Some(vec![1]));
    }

    #[test]
    fn malformed_selection_is_rejected_instead_of_silently_cleared() {
        for nodes in ["null", "\"1\"", "[1,\"2\"]", "[-1]", "[1.5]"] {
            assert!(parse_request(&format!("{{\"op\":\"select\",\"nodes\":{nodes}}}")).is_err());
        }
    }

    #[test]
    fn a_ping_parses() {
        let request = parse_request(r#"{"id":7,"token":"abc","op":"ping"}"#).expect("parses");
        assert_eq!(request.id, 7);
        assert_eq!(request.token, "abc");
        assert!(matches!(request.operation, Operation::Ping));
    }

    /// The load-bearing claim of this module: an `apply` payload is a journal
    /// line, so anything the journal can write, a script can send.
    #[test]
    fn an_apply_carries_a_journal_line() {
        let command = Command::AddGroup {
            parent: None,
            name: "Rig".into(),
        };
        let line = tequila_format::journal::encode(&command);
        let request = format!(r#"{{"id":1,"token":"t","op":"apply","command":{line}}}"#);

        let parsed = parse_request(&request).expect("parses");
        match parsed.operation {
            Operation::Apply(decoded) => assert_eq!(*decoded, command),
            other => panic!("expected an apply, got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_operation_says_so_rather_than_guessing() {
        let error = parse_request(r#"{"id":1,"op":"destroy"}"#).expect_err("refuses");
        assert!(error.0.contains("destroy"), "{error}");
    }

    #[test]
    fn a_bad_query_names_the_ones_that_exist() {
        let error = parse_request(r#"{"id":1,"op":"query","what":"everything"}"#)
            .expect_err("refuses");
        assert!(error.0.contains("nodes"), "{error}");
        assert!(error.0.contains("materials"), "{error}");
    }

    #[test]
    fn a_render_has_defaults_but_takes_what_it_is_given() {
        let request =
            parse_request(r#"{"id":1,"op":"render","path":"out.png","samples":256}"#)
                .expect("parses");
        match request.operation {
            Operation::Render {
                path,
                width,
                height,
                samples,
                wireframe,
            } => {
                assert!(!wireframe, "a photo unless asked");
                assert_eq!(path, "out.png");
                assert_eq!(samples, 256);
                // Unstated size falls back rather than failing: a script that
                // wants "a picture" should not have to care.
                assert_eq!((width, height), (960, 600));
            }
            other => panic!("expected a render, got {other:?}"),
        }
    }

    fn field_of(line: &str, key: &str) -> Value {
        json::parse(line.as_bytes())
            .expect("a reply is JSON")
            .get(key)
            .cloned()
            .unwrap_or(Value::Null)
    }

    #[test]
    fn a_reimport_names_its_model_and_optionally_a_file() {
        let request = parse_request(r#"{"op":"reimport","node":12,"path":"C:/cad/bracket_v2.step"}"#).expect("parses");
        assert!(matches!(request.operation, Operation::Reimport { node: Some(12), ref path, .. } if path.as_deref() == Some("C:/cad/bracket_v2.step")));
        let bare = parse_request(r#"{"op":"reimport"}"#).expect("parses");
        assert!(matches!(bare.operation, Operation::Reimport { node: None, path: None, tessellation: None }));
    }

    #[test]
    fn a_texture_set_names_its_pack_and_optionally_its_parts() {
        let request = parse_request(r#"{"op":"texture_set","path":"C:/packs/Wood049.zip","nodes":[4,7]}"#).expect("parses");
        assert!(matches!(request.operation, Operation::TextureSet { ref path, ref nodes } if path == "C:/packs/Wood049.zip" && nodes == &[4, 7]));
        let bare = parse_request(r#"{"op":"texture_set","path":"C:/packs/wood"}"#).expect("parses");
        assert!(matches!(bare.operation, Operation::TextureSet { ref nodes, .. } if nodes.is_empty()));
        assert!(parse_request(r#"{"op":"texture_set","path":"x","nodes":"all"}"#).is_err());
    }

    /// A library request says what to do and what with, and is refused
    /// when it leaves out what its action needs - before anything is done.
    #[test]
    fn a_library_request_carries_what_its_action_needs() {
        let apply = parse_request(r#"{"op":"library","action":"apply","name":"Aluminium Spun","nodes":[3],"unlink":true}"#).expect("parses");
        let Operation::Library(request) = apply.operation else { panic!("not a library request") };
        assert_eq!(request.action, "apply");
        assert_eq!(request.name.as_deref(), Some("Aluminium Spun"));
        assert_eq!(request.nodes, vec![3]);
        assert!(request.unlink);
        let save = parse_request(r#"{"op":"library","action":"save","material":2,"category":"Mine","tags":["satin","blue"]}"#).expect("parses");
        let Operation::Library(request) = save.operation else { panic!("not a library request") };
        assert_eq!(request.material, Some(2));
        assert_eq!(request.tags, vec!["satin".to_string(), "blue".to_string()]);
        let import = parse_request(r#"{"op":"library","action":"import","path":"C:/packs/Wood049.zip","size":0.25}"#).expect("parses");
        let Operation::Library(request) = import.operation else { panic!("not a library request") };
        assert_eq!(request.size, Some(0.25));
        for refused in [
            r#"{"op":"library","action":"melt","name":"x"}"#,
            r#"{"op":"library","action":"apply"}"#,
            r#"{"op":"library","action":"save"}"#,
            r#"{"op":"library","action":"import"}"#,
            r#"{"op":"library","action":"export","name":"x"}"#,
            r#"{"op":"library","action":"apply","name":"x","tags":"one"}"#,
        ] {
            assert!(parse_request(refused).is_err(), "{refused} was accepted");
        }
        assert!(matches!(parse_request(r#"{"op":"query","what":"library"}"#).unwrap().operation, Operation::Query(Query::Library)));
    }

    /// A plugin request carries files as path: text and a command's values
    /// as a panel's, and is refused without what its action needs.
    #[test]
    fn a_plugin_request_carries_what_its_action_needs() {
        // As every client sends it: the request's own `id` and `token` beside
        // the plugin's name. Named `id` as well, the two collided and every
        // real request was refused.
        let write = parse_request(r#"{"id":7,"token":"t","op":"plugin","action":"write","plugin":"my.x","files":{"plugin.json":"{}","main.py":"print(1)"}}"#).expect("parses");
        assert_eq!(write.id, 7);
        let Operation::Plugin(request) = write.operation else { panic!("not a plugin request") };
        assert_eq!(request.plugin.as_deref(), Some("my.x"));
        assert_eq!(request.files, vec![("main.py".to_string(), "print(1)".to_string()), ("plugin.json".to_string(), "{}".to_string())]);
        let run = parse_request(r#"{"id":8,"op":"plugin","action":"run","plugin":"my.x","command":"go","values":{"n":2}}"#).expect("parses");
        let Operation::Plugin(request) = run.operation else { panic!("not a plugin request") };
        assert_eq!(request.values, vec![("n".to_string(), Value::Number(2.0))]);
        assert!(parse_request(r#"{"id":9,"op":"plugin","action":"list"}"#).is_ok());
        for refused in [
            r#"{"op":"plugin","action":"install","plugin":"x"}"#,
            r#"{"op":"plugin","action":"write","plugin":"x"}"#,
            r#"{"op":"plugin","action":"write","files":{"main.py":""}}"#,
            r#"{"op":"plugin","action":"write","plugin":"x","files":{"main.py":3}}"#,
            r#"{"op":"plugin","action":"run","plugin":"x"}"#,
            r#"{"op":"plugin","action":"enable"}"#,
            r#"{"op":"plugin","action":"run","plugin":"x","command":"go","values":[1]}"#,
        ] {
            assert!(parse_request(refused).is_err(), "{refused} was accepted");
        }
    }

    /// `new` and `open` settle unsaved work as `quit` does, and `open`
    /// needs its path.
    #[test]
    fn new_and_open_read_their_fields() {
        let new = parse_request(r#"{"id":1,"op":"new","discard":true}"#).expect("parses");
        assert!(matches!(new.operation, Operation::Project { action: ProjectAction::New, save: false, discard: true }));
        let open = parse_request(r#"{"id":2,"op":"open","path":"C:/work/kettle.tequila","save":true}"#).expect("parses");
        assert!(matches!(open.operation, Operation::Project { action: ProjectAction::Open(ref path), save: true, discard: false } if path == "C:/work/kettle.tequila"));
        assert!(parse_request(r#"{"id":3,"op":"open"}"#).is_err(), "an open with nothing to open");
        assert!(matches!(parse_request(r#"{"id":4,"op":"ui","about":true}"#).unwrap().operation, Operation::Ui(UiChange { about: Some(true), .. })));
    }

    /// The Library's section, P's Screenshot and its folder, and the
    /// window's size, as a script asks for them; a size that is not two
    /// numbers of a point or more is refused.
    #[test]
    fn ui_takes_the_library_section_a_screenshot_and_a_window_size() {
        let request = parse_request(r#"{"id":8,"op":"ui","library_section":"snapshots","save_screenshot":true,"screenshot_folder":"C:/shots","window_size":[1200,800]}"#).unwrap();
        let Operation::Ui(change) = request.operation else { panic!("a ui change") };
        assert_eq!(change.library_section.as_deref(), Some("snapshots"));
        assert_eq!(change.save_screenshot, Some(true));
        assert_eq!(change.screenshot_folder.as_deref(), Some("C:/shots"));
        assert_eq!(change.window_size, Some([1200.0, 800.0]));
        assert!(parse_request(r#"{"id":9,"op":"ui","window_size":[0,800]}"#).is_err());
        assert!(parse_request(r#"{"id":10,"op":"ui","window_size":"big"}"#).is_err());
    }

    /// A 3D mouse's input, made up: its reports as bytes, and how long the
    /// motion is held; a byte past 255 or an empty report is refused.
    #[test]
    fn synthetic_3d_mouse_input_parses() {
        let request = parse_request(r#"{"id":5,"op":"ui","preferences":true,"spacemouse":{"reports":[[1,0,0,0,0,0,0,0,0,0,0,94,1],[3,2]],"seconds":0.5}}"#).unwrap();
        let Operation::Ui(UiChange { preferences: Some(true), spacemouse: Some(input), .. }) = request.operation else { panic!("a ui change") };
        assert_eq!(input.reports, vec![vec![1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 94, 1], vec![3, 2]]);
        assert_eq!(input.seconds, 0.5);
        assert!(parse_request(r#"{"id":6,"op":"ui","spacemouse":{"reports":[[1,300]]}}"#).is_err());
        assert!(parse_request(r#"{"id":7,"op":"ui","spacemouse":{"reports":[[]]}}"#).is_err());
    }

    #[test]
    fn the_operations_that_take_a_path_read_it() {
        let import = parse_request(r#"{"op":"import","path":"C:/models/part.gltf"}"#)
            .expect("parses");
        assert!(matches!(
            import.operation,
            Operation::Import { ref path, parent: None } if path == "C:/models/part.gltf"
        ));

        // Save without a path means "where it came from", which is what a
        // script editing an open project almost always wants.
        let save = parse_request(r#"{"op":"save"}"#).expect("parses");
        assert!(matches!(save.operation, Operation::Save { path: None }));
    }

    /// Framing defaults to everything, because a script that has not selected
    /// anything wants to see the model rather than an empty box.
    #[test]
    fn framing_defaults_to_the_whole_scene() {
        let all = parse_request(r#"{"op":"frame"}"#).expect("parses");
        assert!(matches!(all.operation, Operation::Frame { all: true }));
        let selection = parse_request(r#"{"op":"frame","all":false}"#).expect("parses");
        assert!(matches!(selection.operation, Operation::Frame { all: false }));
    }

    #[test]
    fn replies_distinguish_a_failure_from_an_answer() {
        let done = encode_reply(3, &Reply::Done);
        assert_eq!(field_of(&done, "ok"), Value::Bool(true));

        let failed = encode_reply(3, &Reply::Failed("no such part".into()));
        assert_eq!(field_of(&failed, "ok"), Value::Bool(false));
        assert_eq!(
            field_of(&failed, "error"),
            Value::String("no such part".into())
        );
    }

    /// The property the whole transport rests on: one reply is one line.
    ///
    /// The shared JSON writer indents and wraps by default, so this is not
    /// free — and a reply that spanned two lines would leave every client
    /// blocked on a `read_line` that had already returned half an answer.
    #[test]
    fn a_reply_is_always_exactly_one_line() {
        let replies = [
            Reply::Done,
            Reply::Failed("something
with a newline in it".into()),
            Reply::Value(Value::Array(vec![
                Value::String("a".into()),
                Value::Number(2.0),
            ])),
        ];
        for reply in &replies {
            let line = encode_reply(1, reply);
            assert!(!line.contains('\n'), "a reply spanned lines: {line:?}");
            // And it still parses, which is what says the flattening did not
            // corrupt an escaped newline inside a string.
            json::parse(line.as_bytes()).expect("still JSON");
        }
    }
}

#[cfg(test)]
mod control_tests {
    use super::*;

    fn operation(line: &str) -> Operation {
        parse_request(line).unwrap_or_else(|e| panic!("{line}: {e}")).operation
    }

    #[test]
    fn a_batch_holds_operations_and_is_atomic_by_default() {
        let parsed = operation(
            r#"{"op":"batch","ops":[{"op":"select","nodes":[3]},{"op":"frame","all":false},{"op":"undo"}]}"#,
        );
        match parsed {
            Operation::Batch { operations, atomic } => {
                assert!(atomic, "a failed batch is taken back unless asked otherwise");
                assert_eq!(operations.len(), 3);
                assert!(matches!(operations[1], Operation::Frame { all: false }));
            }
            other => panic!("{other:?}"),
        }
        match operation(r#"{"op":"batch","atomic":false,"ops":[]}"#) {
            Operation::Batch { atomic, .. } => assert!(!atomic),
            other => panic!("{other:?}"),
        }
    }

    /// A bad entry is reported by its position, so a fifty-line batch does
    /// not have to be bisected by hand.
    #[test]
    fn a_bad_batch_entry_is_named_by_position() {
        let error = parse_request(r#"{"op":"batch","ops":[{"op":"ping"},{"op":"explode"}]}"#)
            .expect_err("refuses");
        assert!(error.0.contains("operation 2"), "{error}");
        assert!(error.0.contains("explode"), "{error}");
    }

    #[test]
    fn batches_cannot_nest_without_limit() {
        let mut line = r#"{"op":"ping"}"#.to_string();
        for _ in 0..(BATCH_DEPTH + 2) {
            line = format!(r#"{{"op":"batch","ops":[{line}]}}"#);
        }
        assert!(parse_request(&line).expect_err("refuses").0.contains("nest"));
    }

    /// A field of the wrong shape is an error, not a silent default: a
    /// `"width":"1920"` rendered at the default size is a bug nobody can see.
    #[test]
    fn a_mistyped_field_is_refused() {
        for line in [
            r#"{"op":"queue","width":"1920"}"#,
            r#"{"op":"ui","console":"yes"}"#,
            r#"{"op":"look","at":[0,1]}"#,
            r#"{"op":"quit","save":1}"#,
        ] {
            assert!(parse_request(line).is_err(), "{line} was accepted");
        }
    }

    /// A queued job can name its engine, and leaves the document's alone
    /// when it does not.
    #[test]
    fn a_queued_job_can_choose_its_engine() {
        match operation(r#"{"op":"queue","engine":"cycles-gpu"}"#) {
            Operation::Queue(job) => assert_eq!(job.engine.as_deref(), Some("cycles-gpu")),
            other => panic!("{other:?}"),
        }
        match operation(r#"{"op":"queue"}"#) {
            Operation::Queue(job) => assert_eq!(job.engine, None),
            other => panic!("{other:?}"),
        }
        assert!(parse_request(r#"{"op":"queue","engine":3}"#).is_err());
    }

    #[test]
    fn quit_saves_nothing_and_discards_nothing_unless_told() {
        match operation(r#"{"op":"quit"}"#) {
            Operation::Quit { save, discard } => assert!(!save && !discard),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn look_takes_a_view_angles_or_points() {
        match operation(r#"{"op":"look","from":[1,2,3],"at":[0,0.5,0],"focal_length":85}"#) {
            Operation::Look(look) => {
                assert_eq!(look.from, Some([1.0, 2.0, 3.0]));
                assert_eq!(look.at, Some([0.0, 0.5, 0.0]));
                assert_eq!(look.focal_length, Some(85.0));
                assert_eq!(look.view, None);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_new_queries_parse() {
        for (what, query) in [("ui", Query::Ui), ("jobs", Query::Jobs), ("history", Query::History)] {
            match operation(&format!(r#"{{"op":"query","what":"{what}"}}"#)) {
                Operation::Query(parsed) => assert_eq!(parsed, query),
                other => panic!("{other:?}"),
            }
        }
    }
}
