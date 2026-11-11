//! Adobe Substance materials in the application: a file opened as a
//! material, its maps kept rendered, and its settings in the Material tab.
//!
//! One route for every way in — the File menu, the Library panel, the
//! Material tab, a file dropped on the window and the scripting channel —
//! as texture packs have (`texture_pack.rs`), and the maps are wired by the
//! same code a pack's are (`tequila_import::texture_set::material`).
//!
//! A Substance material is an ordinary material whose graph reads a
//! texture per output, plus a link (`SubstanceLink`) saying which file and
//! graph it came from and the values the person gave its inputs. Tequila
//! reads the file's settings itself; the maps are rendered by Adobe's
//! `sbsrender` (`tequila_import::substance::bridge`), in the background,
//! a moment after the settings stop changing, into a cache folder named for
//! the settings. When a render finishes the material's textures are pointed
//! at it. A saved project keeps the maps (`docs/file-format.md` §4.2), so it
//! opens and renders the same on a computer without Adobe's tools; there the
//! Material tab still shows the settings, and says what to install to change
//! them.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use tequila_import::substance::{self as sbs, bridge, FileKind, Graph, InputKind, SubstanceFile, Value};
use tequila_import::texture_set::Slot;
use tequila_materials::graph::TextureId;
use tequila_scene::material::SubstanceLink;
use tequila_scene::tree::{MaterialKey, NodeId};
use tequila_scene::Command;

use crate::project::Project;

/// The output size a new Substance material is rendered at, as a power of
/// two (10 is 1024 pixels), unless its file asks for more: Substance
/// Source's materials default to 256 pixels, made for a game's texture
/// budget rather than a product shot.
pub const DEFAULT_OUTPUT_SIZE: f64 = 10.0;

/// How long the settings have to stay still before their maps are rendered:
/// a slider's drag renders once, where it ends.
pub const SETTLE: Duration = Duration::from_millis(400);

/// How a material's maps stand.
#[derive(Clone, Debug, PartialEq)]
pub enum State {
    /// The maps on the material are the render of its settings.
    Rendered,
    /// The settings changed a moment ago; the render starts when they stop.
    Waiting,
    Rendering,
    /// The last render of these settings failed, and why.
    Failed(String),
    /// Adobe's tools were not found, so the maps cannot be made.
    NoTools,
    /// The file is not where the material says it is.
    Missing,
    /// The file is there and could not be read.
    Unreadable(String),
    /// Its maps are another material's — it is an unlinked copy — and its
    /// settings now differ: it needs maps of its own before they can follow.
    Shared(String),
}

/// Give `material`, a Substance material sharing its textures with another
/// (an unlinked copy), textures of its own, showing what they show now, so
/// its maps can follow its own settings. One undo step.
pub fn own_maps(project: &mut Project, material: MaterialKey) -> Result<(), String> {
    let definition = project.document().materials.get(material).ok_or("no such material")?.clone();
    let mut link = definition.substance.as_deref().ok_or("not a Substance material")?.clone();
    let old: Vec<TextureId> = link.maps.values().copied().collect();
    let fresh = tequila_scene::command::reserve_texture_ids(project.document(), old.len());
    let renumber: HashMap<TextureId, TextureId> = old.iter().copied().zip(fresh.iter().copied()).collect();
    let textures: Vec<(TextureId, tequila_scene::textures::TextureSource)> = old
        .iter()
        .filter_map(|id| Some((renumber[id], project.document().textures.get(*id)?.clone())))
        .collect();
    let mut graph = definition.graph.clone();
    let nodes: Vec<_> = graph.nodes().map(|(id, _)| id).collect();
    for id in nodes {
        if let Some(tequila_materials::graph::NodeKind::ImageTexture { texture, .. }) = graph.node_mut(id).map(|n| &mut n.kind) {
            if let Some(new) = renumber.get(texture) {
                *texture = *new;
            }
        }
    }
    for texture in link.maps.values_mut() {
        if let Some(new) = renumber.get(texture) {
            *texture = *new;
        }
    }
    project.begin_group();
    let applied = project
        .apply(Command::Import {
            payload: Box::new(tequila_scene::command::ImportPayload { meshes: Vec::new(), textures, materials: Vec::new(), nodes: Vec::new(), roots: Vec::new() }),
        })
        .and_then(|_| project.apply(Command::SetGraph { material, graph: Box::new(graph) }))
        .and_then(|_| project.apply(Command::SetSubstance { material, substance: Some(Box::new(link)) }));
    match applied {
        Ok(()) => {
            project.end_group();
            project.end_interaction();
            Ok(())
        }
        Err(error) => {
            project.abandon_group();
            Err(error.to_string())
        }
    }
}

/// What the person chose, kept beside their other settings: where Adobe's
/// tools are, when not in a default folder.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preferences {
    pub tools: Option<PathBuf>,
}

impl Preferences {
    fn path() -> PathBuf {
        tequila_format::library::settings_directory().join("substance.txt")
    }

    fn from_text(text: &str) -> Preferences {
        let mut out = Preferences::default();
        for line in text.lines() {
            if let Some(path) = line.strip_prefix("tools=") {
                let path = path.trim();
                if !path.is_empty() {
                    out.tools = Some(PathBuf::from(path));
                }
            }
        }
        out
    }

    fn to_text(&self) -> String {
        match &self.tools {
            Some(path) => format!("tools={}\n", path.display()),
            None => String::new(),
        }
    }

    pub fn load() -> Preferences {
        std::fs::read_to_string(Preferences::path()).map(|t| Preferences::from_text(&t)).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Preferences::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, self.to_text()).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// A file as last read: its size and time, so a change on disk reads it
/// again, its content's hash, and what it said.
struct Described {
    stamp: (u64, Option<SystemTime>),
    hash: String,
    file: Result<Arc<SubstanceFile>, String>,
}

/// What a finished render sends back.
type Finished = (MaterialKey, String, Result<(), String>, Duration);

/// Every Substance material's maps, kept rendered.
pub struct Renders {
    pub preferences: Preferences,
    pub tools: bridge::Tools,
    /// What runs a tool: the program itself, or the tests' stand-in.
    pub runner: bridge::Runner,
    /// Where renders are filed, `<folder>/<settings key>/`.
    pub cache: PathBuf,
    /// How long the settings have to stay still before a render.
    pub settle: Duration,
    described: HashMap<PathBuf, Described>,
    /// Renders running on their own threads, by material: the settings each
    /// is of.
    jobs: HashMap<MaterialKey, String>,
    /// Settings that changed and when, waiting to settle.
    pending: HashMap<MaterialKey, (String, Instant)>,
    /// Renders that failed, by the settings they were of.
    failed: HashMap<MaterialKey, (String, String)>,
    states: HashMap<MaterialKey, State>,
    finished: (Sender<Finished>, Receiver<Finished>),
    thumbnails: HashMap<(PathBuf, String), Option<egui::TextureHandle>>,
    /// A file picked in the panel for one of the image inputs, a folder for
    /// the tools, or a file that moved, waiting for the dialog to be shown
    /// after the panel has drawn.
    asked: Option<Ask>,
    /// Look for the tools again on the next poll.
    look_again: bool,
}

#[derive(Clone, Debug)]
enum Ask {
    Image { material: MaterialKey, input: String },
    Tools,
    Relink(MaterialKey),
    OwnMaps(MaterialKey),
}

impl Default for Renders {
    fn default() -> Renders {
        Renders {
            preferences: Preferences::default(),
            tools: bridge::Tools::default(),
            runner: bridge::run_program,
            cache: tequila_format::substance_cache_directory(),
            settle: SETTLE,
            described: HashMap::new(),
            jobs: HashMap::new(),
            pending: HashMap::new(),
            failed: HashMap::new(),
            states: HashMap::new(),
            finished: channel(),
            thumbnails: HashMap::new(),
            asked: None,
            look_again: false,
        }
    }
}

impl Renders {
    /// The person's own: their preferences, and the tools wherever they are
    /// (`TEQUILA_SUBSTANCE_TOOLS`, then the preference, then the default
    /// folders and the PATH).
    pub fn for_this_computer() -> Renders {
        let mut renders = Renders { preferences: Preferences::load(), ..Renders::default() };
        renders.find_tools();
        renders
    }

    /// Look for Adobe's tools again.
    pub fn find_tools(&mut self) {
        let configured = std::env::var_os("TEQUILA_SUBSTANCE_TOOLS")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| self.preferences.tools.clone());
        self.tools = bridge::find(configured.as_deref());
        // A failure for want of the tools is not a failure once they are here.
        self.failed.clear();
    }

    /// Use the tools in `folder` (a folder, or `sbsrender` itself) from now
    /// on, and remember it.
    pub fn set_tools_folder(&mut self, folder: Option<PathBuf>) -> Result<(), String> {
        self.preferences.tools = folder;
        let saved = if cfg!(test) { Ok(()) } else { self.preferences.save() };
        self.find_tools();
        saved
    }

    /// What `path` says about itself, read once and again when it changes.
    pub fn describe(&mut self, path: &Path) -> Result<Arc<SubstanceFile>, String> {
        self.describe_with_hash(path).map(|(file, _)| file)
    }

    fn describe_with_hash(&mut self, path: &Path) -> Result<(Arc<SubstanceFile>, String), String> {
        let metadata = std::fs::metadata(path).map_err(|_| format!("{} is not there", path.display()))?;
        let stamp = (metadata.len(), metadata.modified().ok());
        if let Some(known) = self.described.get(path).filter(|d| d.stamp == stamp) {
            return known.file.clone().map(|f| (f, known.hash.clone()));
        }
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let hash = sbs::content_hash(&bytes);
        let file = match FileKind::of(path) {
            Some(FileKind::Sbsar) => sbs::read_sbsar(&bytes),
            Some(FileKind::Sbs) => sbs::read_sbs(&String::from_utf8_lossy(&bytes)),
            None => Err(tequila_import::ImportError::UnknownFormat(path.display().to_string())),
        }
        .map(Arc::new)
        .map_err(|e| e.to_string());
        self.described.insert(path.to_path_buf(), Described { stamp, hash: hash.clone(), file: file.clone() });
        file.map(|f| (f, hash))
    }

    /// The hash of `path`'s content as last read, if it has been.
    fn hash_of(&self, path: &Path) -> Option<&str> {
        self.described.get(path).map(|d| d.hash.as_str())
    }

    /// How `material`'s maps stand, as the last poll found.
    pub fn state(&self, material: MaterialKey) -> Option<&State> {
        self.states.get(&material)
    }

    /// The folder a render of `link`'s settings goes in.
    pub fn folder(&self, link: &SubstanceLink) -> PathBuf {
        self.cache.join(link.settings_key())
    }

    /// Whether a render is running or waiting to.
    pub fn busy(&self) -> bool {
        !self.jobs.is_empty() || !self.pending.is_empty()
    }

    /// Bring every Substance material's maps up to its settings: point its
    /// textures at a finished render of them, or start one once they have
    /// settled. Returns what is worth telling the person: renders that
    /// finished or failed.
    pub fn poll(&mut self, project: &mut Project) -> Vec<String> {
        let mut said = Vec::new();
        while let Ok((material, settings, result, took)) = self.finished.1.try_recv() {
            self.jobs.remove(&material);
            let name = project.document().materials.get(material).map(|d| d.name.clone()).unwrap_or_default();
            match result {
                Ok(()) => said.push(format!("Rendered the maps of `{name}` with Adobe's sbsrender in {:.1} s", took.as_secs_f32())),
                Err(error) => {
                    said.push(format!("The maps of `{name}` could not be rendered: {error}"));
                    self.failed.insert(material, (settings, error));
                }
            }
        }
        if self.look_again {
            self.look_again = false;
            self.find_tools();
        }
        let links: Vec<(MaterialKey, SubstanceLink)> = project
            .document()
            .materials
            .iter()
            .filter_map(|(key, d)| d.substance.as_ref().map(|l| (key, (**l).clone())))
            .collect();
        self.states.retain(|key, _| links.iter().any(|(k, _)| k == key));
        self.pending.retain(|key, _| links.iter().any(|(k, _)| k == key));
        let mut repoint = Vec::new();
        // A texture belongs to the first material that claims it. An
        // unlinked copy of a Substance material shares its original's
        // textures; while their settings agree that is harmless, and once
        // they differ the copy must not repoint the original's maps — the
        // two would point them back and forth for ever — until it is given
        // maps of its own ([`own_maps`]).
        let mut owners: HashMap<TextureId, (MaterialKey, String)> = HashMap::new();
        for (material, link) in links {
            let settings = link.settings_key();
            let shared = link.maps.values().find_map(|t| owners.get(t).filter(|(_, s)| *s != settings).map(|(owner, _)| *owner));
            let state = match shared {
                Some(owner) => State::Shared(project.document().materials.get(owner).map(|d| d.name.clone()).unwrap_or_default()),
                None => self.bring_up(material, &link, project, &mut repoint),
            };
            for texture in link.maps.values() {
                owners.entry(*texture).or_insert((material, settings.clone()));
            }
            // Only settings waiting to settle are pending; a material that
            // cannot render, or is rendering, is not waiting for anything.
            if state != State::Waiting {
                self.pending.remove(&material);
            }
            self.states.insert(material, state);
        }
        project.repoint_textures(&repoint);
        said
    }

    fn bring_up(&mut self, material: MaterialKey, link: &SubstanceLink, project: &Project, repoint: &mut Vec<(TextureId, PathBuf)>) -> State {
        let file = match self.describe(&link.file) {
            Ok(file) => file,
            Err(_) if !link.file.exists() => return State::Missing,
            Err(error) => return State::Unreadable(error),
        };
        let Some(graph) = file.graph(Some(&link.graph)) else {
            return State::Failed(format!("{} no longer has the graph {}", link.file.display(), link.graph));
        };
        let settings = link.settings_key();
        let folder = self.cache.join(&settings);
        if bridge::is_rendered(&folder) {
            for (texture, path) in map_files(link, graph, &folder) {
                let current = project.document().textures.get(texture).map(|t| t.path.clone());
                if path.is_file() && current.as_deref() != Some(path.as_path()) {
                    repoint.push((texture, path));
                }
            }
            self.pending.remove(&material);
            return State::Rendered;
        }
        if let Some((failed, error)) = self.failed.get(&material) {
            if *failed == settings {
                return State::Failed(error.clone());
            }
        }
        if self.jobs.contains_key(&material) {
            return State::Rendering;
        }
        if !self.tools.can_render_kind(file.kind) {
            return State::NoTools;
        }
        let now = Instant::now();
        let since = match self.pending.get(&material) {
            Some((pending, since)) if *pending == settings => *since,
            _ => {
                self.pending.insert(material, (settings.clone(), now));
                now
            }
        };
        if now.duration_since(since) < self.settle {
            return State::Waiting;
        }
        self.pending.remove(&material);
        self.start(material, link, graph.clone(), folder);
        State::Rendering
    }

    fn start(&mut self, material: MaterialKey, link: &SubstanceLink, graph: Graph, folder: PathBuf) {
        let settings = link.settings_key();
        let tools = self.tools.clone();
        let runner = self.runner;
        let file = link.file.clone();
        let values = link.values.clone();
        let images = link.images.clone();
        let cooked = self.cache.join("cooked").join(if link.file_hash.is_empty() { "file" } else { &link.file_hash });
        let sender = self.finished.0.clone();
        let started = Instant::now();
        self.jobs.insert(material, settings.clone());
        std::thread::spawn(move || {
            let result = bridge::render(&tools, runner, &file, &graph, &values, &images, &folder, &cooked).map(|_| ());
            let _ = sender.send((material, settings, result, started.elapsed()));
        });
    }

    /// Wait, at most `limit`, for every render running or waiting to finish,
    /// polling as the window would: for scripts and tests.
    pub fn wait(&mut self, project: &mut Project, limit: Duration) -> Vec<String> {
        let started = Instant::now();
        let mut said = self.poll(project);
        while self.busy() && started.elapsed() < limit {
            std::thread::sleep(Duration::from_millis(20));
            said.extend(self.poll(project));
        }
        said
    }

    /// Forget a failure, so the same settings are tried again.
    pub fn retry(&mut self, material: MaterialKey) {
        self.failed.remove(&material);
    }
}

/// Each map texture of `link`, and the file in `folder` it reads: the
/// rendered map, or for a normal map drawn in DirectX's convention, the
/// OpenGL copy made beside it.
pub fn map_files(link: &SubstanceLink, graph: &Graph, folder: &Path) -> Vec<(TextureId, PathBuf)> {
    let directx = graph.normal_is_directx(&link.values);
    link.maps
        .iter()
        .filter_map(|(output, texture)| {
            let output = graph.outputs.iter().find(|o| o.identifier == *output)?;
            Some((*texture, map_file(folder, output, directx)))
        })
        .collect()
}

fn map_file(folder: &Path, output: &sbs::Output, directx: bool) -> PathBuf {
    if directx && matches!(output.slot(true), Ok(Slot::Normal { .. })) {
        sbs::converted_file(folder, &output.identifier)
    } else {
        sbs::rendered_file(folder, &output.identifier)
    }
}

/// What opening a Substance file made.
#[derive(Debug)]
pub struct Opened {
    /// One material per graph, in the file's order.
    pub materials: Vec<(MaterialKey, String)>,
    /// The parts the first went on.
    pub assigned: Vec<NodeId>,
    pub file: PathBuf,
    /// The roles of the first material's maps.
    pub maps: Vec<&'static str>,
    /// Outputs not used, and why.
    pub unused: Vec<(String, String)>,
    /// Whether Adobe's tools are here to render the maps.
    pub can_render: bool,
}

/// The values a new material starts with: the file's defaults, but rendered
/// at [`DEFAULT_OUTPUT_SIZE`] at least.
pub fn starting_values(graph: &Graph) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    if let Some(size) = graph.input("$outputsize") {
        let default = size.default.numbers();
        if default.iter().copied().fold(0.0, f64::max) < DEFAULT_OUTPUT_SIZE {
            values.insert("$outputsize".to_string(), sbs::format_numbers(&[DEFAULT_OUTPUT_SIZE, DEFAULT_OUTPUT_SIZE], true));
        }
    }
    values
}

/// Open the Substance file at `source` as materials — one per graph, or
/// only `graph` when named — and put the first on `targets`. One undo step.
pub fn open(project: &mut Project, renders: &mut Renders, source: &Path, targets: Vec<NodeId>, graph: Option<&str>) -> Result<Opened, String> {
    if FileKind::of(source).is_none() {
        return Err(format!("{} is not a Substance material (.sbsar or .sbs)", source.display()));
    }
    for &node in &targets {
        match project.document().tree.get(node) {
            None => return Err(format!("there is no node {}", node.0)),
            Some(n) if project.document().tree.is_effectively_locked(node) => return Err(format!("{} is locked", n.name)),
            Some(_) => {}
        }
    }
    let (file, hash) = renders.describe_with_hash(source)?;
    let graphs: Vec<&Graph> = match graph {
        Some(url) => vec![file.graph(Some(url)).ok_or_else(|| {
            let names: Vec<&str> = file.graphs.iter().map(|g| g.url.as_str()).collect();
            format!("{} has no graph `{url}`; it has {}", source.display(), names.join(", "))
        })?],
        None => file.visible_graphs(),
    };
    if graphs.is_empty() {
        return Err(format!("{} holds no graphs", source.display()));
    }
    let options = crate::texture_pack::set_options(project.document(), &targets);
    let document = project.document();
    let (_, keys, _) = tequila_scene::command::reserve_keys(document, 0, graphs.len(), 0);
    let mut next_texture = document.textures.next_id();
    let mut materials = Vec::new();
    let mut textures = Vec::new();
    let mut first_maps = Vec::new();
    let mut first_unused = Vec::new();
    for (index, (graph, key)) in graphs.iter().zip(keys).enumerate() {
        let values = starting_values(graph);
        let mut link = SubstanceLink {
            file: source.to_path_buf(),
            file_hash: hash.clone(),
            graph: graph.url.clone(),
            values,
            images: BTreeMap::new(),
            maps: BTreeMap::new(),
        };
        let folder = renders.folder(&link);
        let directx = graph.normal_is_directx(&link.values);
        let (set, unused) = sbs::texture_set(graph, directx, |o| map_file(&folder, o, directx));
        let count = tequila_import::texture_set::texture_count(&set);
        let ids: Vec<TextureId> = (0..count as u32).map(|i| TextureId(next_texture + i)).collect();
        next_texture += count as u32;
        let (mut definition, used) = tequila_import::texture_set::material(&set, &ids, options);
        for (id, source) in &used {
            if let Some(output) = graph.outputs.iter().find(|o| map_file(&folder, o, directx) == source.path) {
                link.maps.insert(output.identifier.clone(), *id);
            }
        }
        if index == 0 {
            first_maps = set.maps.iter().map(|m| m.slot.label()).collect();
            first_unused = unused;
        }
        definition.substance = Some(Box::new(link));
        textures.extend(used);
        materials.push((key, definition));
    }
    let names: Vec<(MaterialKey, String)> = materials.iter().map(|(k, d)| (*k, d.name.clone())).collect();
    let command = Command::Import {
        payload: Box::new(tequila_scene::command::ImportPayload { meshes: Vec::new(), textures, materials, nodes: Vec::new(), roots: Vec::new() }),
    };
    project.begin_group();
    let applied = project.apply(command).map_err(|e| e.to_string()).and_then(|_| match names.first() {
        Some(&(key, _)) if !targets.is_empty() => {
            project.apply(Command::AssignMaterial { nodes: targets.clone(), material: key }).map(|_| targets).map_err(|e| e.to_string())
        }
        _ => Ok(Vec::new()),
    });
    let assigned = match applied {
        Ok(assigned) => {
            project.end_group();
            assigned
        }
        Err(error) => {
            project.abandon_group();
            return Err(error);
        }
    };
    project.end_interaction();
    // At once: a render already in the cache goes straight on.
    renders.poll(project);
    Ok(Opened { materials: names, assigned, file: source.to_path_buf(), maps: first_maps, unused: first_unused, can_render: renders.tools.can_render_kind(file.kind) })
}

/// The status line for what was opened.
pub fn status(opened: &Opened) -> String {
    let Some((_, name)) = opened.materials.first() else {
        return "The file held no material".into();
    };
    let file = opened.file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let parts = opened.assigned.len();
    let mut line = if parts == 0 {
        format!("Added `{name}` from {file} to the project. Select a part, then pick it from the material name at the top of the Material tab")
    } else {
        format!("`{name}` from {file} on {parts} part{}", if parts == 1 { "" } else { "s" })
    };
    if opened.can_render {
        line.push_str(&format!(". Its maps ({}) are being rendered by Adobe's sbsrender; its settings are in the Material tab", opened.maps.join(", ")));
    } else {
        line.push_str(". Its settings are in the Material tab, but its maps need Adobe's sbsrender, which was not found: install Substance 3D Designer or the Substance 3D Automation Toolkit");
    }
    if !opened.unused.is_empty() {
        let names: Vec<&str> = opened.unused.iter().map(|(id, _)| id.as_str()).collect();
        line.push_str(&format!(". Not used: {}", names.join(", ")));
    }
    if opened.materials.len() > 1 {
        let others: Vec<&str> = opened.materials[1..].iter().map(|(_, n)| n.as_str()).collect();
        line.push_str(&format!(". Also added: {}", others.join(", ")));
    }
    line
}

/// Ask for a Substance file.
pub fn pick() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Open a Substance material")
        .add_filter("Substance material", &["sbsar", "sbs"])
        .pick_file()
}

// --- the Material tab's panel -------------------------------------------------

/// What the panel asks for once it has drawn.
#[derive(Default)]
pub struct Asked {
    pub command: Option<Command>,
    /// Whether the edit is finished (a drag released, a choice made), so the
    /// next starts its own undo step.
    pub commit: bool,
}

/// The link with `input` set to `value`, or back to the default when
/// `value` is the default.
fn with_value(link: &SubstanceLink, graph: &Graph, input: &str, value: &Value) -> SubstanceLink {
    let mut link = link.clone();
    if let Some(definition) = graph.input(input) {
        let text = value.to_text(definition.value_type);
        if text == definition.default.to_text(definition.value_type) && input != "$outputsize" {
            link.values.remove(input);
        } else {
            link.values.insert(input.to_string(), text);
        }
    }
    link
}

fn set(material: MaterialKey, link: SubstanceLink) -> Command {
    Command::SetSubstance { material, substance: Some(Box::new(link)) }
}

/// The Substance section of the Material tab, for `material`, which is a
/// Substance material. Returns the edit to apply.
pub fn panel(ui: &mut egui::Ui, project: &Project, renders: &mut Renders, material: MaterialKey) -> Asked {
    let mut asked = Asked::default();
    let Some(link) = project.document().materials.get(material).and_then(|d| d.substance.as_deref()).cloned() else {
        return asked;
    };
    // Its title is the section's, on the Material page.
    let described = renders.describe(&link.file);
    let graph = described.as_ref().ok().and_then(|f| f.graph(Some(&link.graph)).cloned());
    let file_name = link.file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();

    // Who made it and what it is, beside its own picture.
    ui.horizontal(|ui| {
        if let Some(graph) = &graph {
            if let Some(texture) = renders.thumbnail(ui.ctx(), &link.file, graph) {
                ui.add(egui::Image::from_texture(egui::load::SizedTexture::from_handle(&texture)).fit_to_exact_size(egui::vec2(64.0, 64.0)).rounding(6.0));
            }
        }
        ui.vertical(|ui| {
            let title = graph.as_ref().map(|g| g.name()).unwrap_or_else(|| link.graph.clone());
            ui.label(egui::RichText::new(title).font(crate::theme::strong(13.0)));
            let mut about = vec![file_name.clone()];
            if let Some(graph) = &graph {
                if !graph.author.is_empty() {
                    about.push(format!("by {}", graph.author));
                }
                if !graph.category.is_empty() {
                    about.push(graph.category.replace('/', " › "));
                }
            }
            ui.label(egui::RichText::new(about.join("  ·  ")).small().color(crate::theme::dim())).on_hover_text(link.file.display().to_string());
            if let Some(graph) = graph.as_ref().filter(|g| !g.description.is_empty()) {
                ui.label(egui::RichText::new(&graph.description).small().color(crate::theme::dim()));
            }
        });
    });
    ui.add_space(4.0);
    state_line(ui, renders, material, &link, project, &mut asked);

    let Some(graph) = graph else {
        if let Err(error) = &described {
            if link.file.exists() {
                ui.label(egui::RichText::new(format!("Its settings could not be read: {error}")).small().color(crate::theme::hue(crate::theme::Hue::Amber)));
            }
        }
        return asked;
    };

    // Presets, the output size and the seed, before the graph's own inputs.
    egui::Grid::new(("substance-system", material.0)).num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        if !graph.presets.is_empty() {
            ui.label("Preset");
            egui::ComboBox::from_id_salt(("substance-preset", material.0)).selected_text("Choose…").width(170.0).show_ui(ui, |ui| {
                for preset in &graph.presets {
                    if ui.selectable_label(false, &preset.label).clicked() {
                        let mut next = link.clone();
                        for (id, value) in preset.values_for(&graph) {
                            next.values.insert(id, value);
                        }
                        asked.command = Some(set(material, next));
                        asked.commit = true;
                    }
                }
            });
            ui.end_row();
        }
        if let Some(size) = graph.input("$outputsize") {
            ui.label("Output size").on_hover_text("How many pixels across the maps are rendered. Larger is sharper up close and slower to render.");
            let current = size.value_in(&link.values).first().unwrap_or(DEFAULT_OUTPUT_SIZE).round() as i64;
            let label = |power: i64| format!("{0} × {0}", 1i64 << power.clamp(0, 16));
            egui::ComboBox::from_id_salt(("substance-size", material.0)).selected_text(label(current)).width(170.0).show_ui(ui, |ui| {
                for power in 8..=12i64 {
                    if ui.selectable_label(power == current, label(power)).clicked() && power != current {
                        asked.command = Some(set(material, with_value(&link, &graph, "$outputsize", &Value::Numbers(vec![power as f64, power as f64]))));
                        asked.commit = true;
                    }
                }
            });
            ui.end_row();
        }
        if let Some(seed) = graph.input("$randomseed") {
            ui.label("Random seed").on_hover_text("Another seed gives another variation of the same material.");
            let mut value = seed.value_in(&link.values).first().unwrap_or(0.0).round() as i64;
            let response = ui.add(egui::DragValue::new(&mut value).range(0..=1_000_000).speed(0.2));
            if response.changed() {
                asked.command = Some(set(material, with_value(&link, &graph, "$randomseed", &Value::Numbers(vec![value as f64]))));
            }
            if response.drag_stopped() || response.lost_focus() {
                asked.commit = true;
            }
            ui.end_row();
        }
    });

    for (group, inputs) in graph.groups() {
        let title = if group.is_empty() { "Settings".to_string() } else { group.replace('/', " › ") };
        egui::CollapsingHeader::new(title).id_salt(("substance-group", material.0, group.clone())).default_open(true).show(ui, |ui| {
            egui::Grid::new(("substance-inputs", material.0, group.clone())).num_columns(3).spacing([8.0, 4.0]).show(ui, |ui| {
                for input in inputs {
                    input_row(ui, renders, material, &link, &graph, input, &mut asked);
                    ui.end_row();
                }
            });
        });
    }

    // What became of each output.
    let directx = graph.normal_is_directx(&link.values);
    let (set_of_maps, unused) = sbs::texture_set(&graph, directx, |o| PathBuf::from(&o.identifier));
    let mut used: Vec<String> = set_of_maps.maps.iter().map(|m| m.slot.label().to_string()).collect();
    if directx && set_of_maps.map(Slot::Normal { directx: true }).is_some() {
        used.push("the DirectX normal map is turned into an OpenGL one".into());
    }
    ui.label(egui::RichText::new(format!("Maps: {}", used.join(", "))).small().color(crate::theme::dim()));
    if !unused.is_empty() {
        let names: Vec<String> = unused.iter().map(|(id, why)| format!("{id} ({why})")).collect();
        ui.label(egui::RichText::new(format!("Not used: {}", names.join("; "))).small().color(crate::theme::dim()));
    }
    ui.horizontal(|ui| {
        if ui.small_button("Reset all").on_hover_text("Every setting back to the file's own, at the output size Tequila starts with").clicked() {
            let mut next = link.clone();
            next.values = starting_values(&graph);
            next.images.clear();
            asked.command = Some(set(material, next));
            asked.commit = true;
        }
        if ui.small_button("Read the file again").on_hover_text("After changing it in Substance Designer: its settings, and maps rendered from the new version").clicked() {
            if let Some(hash) = renders.hash_of(&link.file).map(str::to_string) {
                if hash != link.file_hash {
                    let mut next = link.clone();
                    next.file_hash = hash;
                    asked.command = Some(set(material, next));
                    asked.commit = true;
                }
            }
        }
        if ui
            .small_button("Keep the maps, let go of the file")
            .on_hover_text("Make it an ordinary material with the maps it has now: its settings go, and nothing renders again")
            .clicked()
        {
            asked.command = Some(Command::SetSubstance { material, substance: None });
            asked.commit = true;
        }
    });
    asked
}

/// The line that says how the maps stand, with what to do about it.
fn state_line(ui: &mut egui::Ui, renders: &mut Renders, material: MaterialKey, link: &SubstanceLink, project: &Project, asked: &mut Asked) {
    let has_maps = link.maps.values().any(|t| project.document().textures.get(*t).is_some_and(|s| s.path.is_file()));
    let amber = crate::theme::hue(crate::theme::Hue::Amber);
    let changed_on_disk = renders.hash_of(&link.file).is_some_and(|h| !link.file_hash.is_empty() && h != link.file_hash);
    match renders.state(material).cloned() {
        Some(State::Rendered) => {
            let size = link.values.get("$outputsize").and_then(|v| Value::parse(v).first().copied());
            let text = match size {
                Some(power) => format!("Maps rendered by Adobe's sbsrender at {0} × {0}", 1i64 << (power.round() as i64).clamp(0, 16)),
                None => "Maps rendered by Adobe's sbsrender".to_string(),
            };
            ui.label(egui::RichText::new(text).small().color(crate::theme::hue(crate::theme::Hue::Green)));
        }
        Some(State::Waiting) | Some(State::Rendering) => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(egui::RichText::new("Rendering the maps with Adobe's sbsrender…").small());
            });
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        Some(State::Failed(error)) => {
            ui.label(egui::RichText::new(format!("The maps could not be rendered: {error}")).small().color(amber));
            if ui.small_button("Try again").clicked() {
                renders.retry(material);
            }
        }
        Some(State::NoTools) => {
            crate::theme::card(ui, |ui| {
                ui.label(egui::RichText::new("Install Adobe's tools to render this material").font(crate::theme::strong(12.5)).color(amber));
                ui.label(egui::RichText::new(bridge::INSTALL_HINT).small());
                if has_maps {
                    ui.label(egui::RichText::new("Showing the maps saved with the project; a changed setting shows once its maps can be rendered.").small().color(crate::theme::dim()));
                } else {
                    ui.label(egui::RichText::new("Until then the material shows its plain colours.").small().color(crate::theme::dim()));
                }
                ui.horizontal(|ui| {
                    if ui.button("Set the folder of Adobe's tools…").clicked() {
                        renders.asked = Some(Ask::Tools);
                    }
                    if ui.button("Look again").clicked() {
                        renders.look_again = true;
                    }
                });
            });
        }
        Some(State::Missing) => {
            ui.label(
                egui::RichText::new(format!(
                    "{} is not there any more.{}",
                    link.file.display(),
                    if has_maps { " Showing the maps it last rendered." } else { "" }
                ))
                .small()
                .color(amber),
            );
            if ui.small_button("Find it…").clicked() {
                renders.asked = Some(Ask::Relink(material));
            }
        }
        Some(State::Shared(owner)) => {
            ui.label(
                egui::RichText::new(format!(
                    "This copy shares its maps with `{owner}`, so they show that material's settings, not these."
                ))
                .small()
                .color(amber),
            );
            if ui.small_button("Give it maps of its own").on_hover_text("New textures for this material, so its maps follow its own settings").clicked() {
                renders.asked = Some(Ask::OwnMaps(material));
            }
        }
        Some(State::Unreadable(error)) => {
            ui.label(egui::RichText::new(format!("The file could not be read: {error}")).small().color(amber));
        }
        None => {}
    }
    if changed_on_disk {
        ui.label(egui::RichText::new("The file has changed since it was read: Read the file again to use the new version.").small().color(amber));
    }
    let _ = asked;
}

fn input_row(ui: &mut egui::Ui, renders: &mut Renders, material: MaterialKey, link: &SubstanceLink, graph: &Graph, input: &sbs::Input, asked: &mut Asked) {
    let label = ui.label(input.name());
    let mut hover = Vec::new();
    if !input.description.is_empty() {
        hover.push(input.description.clone());
    }
    hover.push(format!("`{}`", input.identifier));
    if !input.visible_if.is_empty() {
        hover.push(format!("Shown in Substance when {}", input.visible_if));
    }
    label.on_hover_text(hover.join("\n"));
    let value = input.value_in(&link.values);
    let numbers = value.numbers().to_vec();
    let mut changed: Option<Value> = None;
    let mut done = false;
    let range = |i: usize| -> std::ops::RangeInclusive<f64> {
        let at = |v: &Option<Vec<f64>>, fallback: f64| v.as_ref().and_then(|v| v.get(i).or(v.first())).copied().unwrap_or(fallback);
        let (low, high) = (at(&input.min, 0.0), at(&input.max, 1.0));
        if high > low { low..=high } else { 0.0..=1.0 }
    };
    ui.horizontal(|ui| match input.kind() {
        InputKind::Float(1) | InputKind::Integer(1) => {
            let integer = matches!(input.kind(), InputKind::Integer(_));
            let mut v = numbers.first().copied().unwrap_or(0.0);
            let mut slider = egui::Slider::new(&mut v, range(0)).clamping(if input.clamp { egui::SliderClamping::Always } else { egui::SliderClamping::Never });
            if integer {
                slider = slider.integer();
            } else if let Some(step) = input.step {
                slider = slider.step_by(step);
            }
            let response = ui.add(slider);
            if response.changed() {
                changed = Some(Value::Numbers(vec![v]));
            }
            done = response.drag_stopped() || response.lost_focus();
        }
        InputKind::Float(n) | InputKind::Integer(n) => {
            let integer = matches!(input.kind(), InputKind::Integer(_));
            let mut values: Vec<f64> = (0..n as usize).map(|i| numbers.get(i).copied().unwrap_or(0.0)).collect();
            let mut any = false;
            for (i, v) in values.iter_mut().enumerate() {
                if let Some(name) = input.labels.get(i).filter(|l| !l.is_empty()) {
                    ui.label(egui::RichText::new(name).small().color(crate::theme::dim()));
                }
                let mut drag = egui::DragValue::new(v).speed(if integer { 0.2 } else { 0.01 });
                if input.clamp {
                    drag = drag.range(range(i));
                }
                if integer {
                    drag = drag.max_decimals(0);
                } else {
                    drag = drag.max_decimals(3);
                }
                let response = ui.add(drag);
                any |= response.changed();
                done |= response.drag_stopped() || response.lost_focus();
            }
            if any {
                changed = Some(Value::Numbers(values));
            }
        }
        InputKind::Toggle => {
            let mut on = numbers.first().copied().unwrap_or(0.0) >= 0.5;
            let text = input.labels.get(usize::from(on)).filter(|l| !l.is_empty()).cloned().unwrap_or_default();
            if ui.checkbox(&mut on, text).changed() {
                changed = Some(Value::Numbers(vec![if on { 1.0 } else { 0.0 }]));
                done = true;
            }
        }
        InputKind::Choice => {
            let current = numbers.first().copied().unwrap_or(0.0).round() as i64;
            let shown = input.choices.iter().find(|(v, _)| *v == current).map(|(_, t)| t.clone()).unwrap_or_else(|| current.to_string());
            egui::ComboBox::from_id_salt(("substance-choice", material.0, input.identifier.clone())).selected_text(shown).width(150.0).show_ui(ui, |ui| {
                for (v, text) in &input.choices {
                    if ui.selectable_label(*v == current, text).clicked() && *v != current {
                        changed = Some(Value::Numbers(vec![*v as f64]));
                        done = true;
                    }
                }
            });
        }
        InputKind::Color { alpha } => {
            // Substance's colours are sRGB values, as its colour pickers
            // show them.
            let byte = |i: usize, fallback: f64| (numbers.get(i).copied().unwrap_or(fallback).clamp(0.0, 1.0) * 255.0).round() as u8;
            let response = if alpha {
                let mut colour = egui::Color32::from_rgba_unmultiplied(byte(0, 0.5), byte(1, 0.5), byte(2, 0.5), byte(3, 1.0));
                let r = egui::color_picker::color_edit_button_srgba(ui, &mut colour, egui::color_picker::Alpha::OnlyBlend);
                if r.changed() {
                    let [red, green, blue, a] = colour.to_srgba_unmultiplied();
                    changed = Some(Value::Numbers([red, green, blue, a].iter().map(|c| *c as f64 / 255.0).collect()));
                }
                r
            } else {
                let mut colour = [byte(0, 0.5), byte(1, 0.5), byte(2, 0.5)];
                let r = egui::color_picker::color_edit_button_srgb(ui, &mut colour);
                if r.changed() {
                    changed = Some(Value::Numbers(colour.iter().map(|c| *c as f64 / 255.0).collect()));
                }
                r
            };
            // The picker is a popup; closing it ends the interaction.
            done = response.clicked_elsewhere() || response.lost_focus();
        }
        InputKind::Text => {
            let mut text = match &value {
                Value::Text(t) => t.clone(),
                Value::Numbers(_) => String::new(),
            };
            let response = ui.add(egui::TextEdit::singleline(&mut text).desired_width(150.0));
            if response.lost_focus() && Value::Text(text.clone()) != value {
                changed = Some(Value::Text(text));
                done = true;
            }
        }
        InputKind::Image => {
            let current = link.images.get(&input.identifier);
            let shown = current.and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "None".into());
            if ui.button(shown).on_hover_text("A picture the material takes in").clicked() {
                renders.asked = Some(Ask::Image { material, input: input.identifier.clone() });
            }
            if current.is_some() && ui.small_button("×").on_hover_text("Take the picture out").clicked() {
                let mut next = link.clone();
                next.images.remove(&input.identifier);
                asked.command = Some(set(material, next));
                asked.commit = true;
            }
        }
        InputKind::Font => {
            ui.label(egui::RichText::new("a font (not editable here)").small().color(crate::theme::dim()));
        }
    });
    // Back to the file's value.
    if link.values.contains_key(&input.identifier) || link.images.contains_key(&input.identifier) {
        if ui.small_button("↺").on_hover_text("Back to the file's own value").clicked() {
            let mut next = link.clone();
            next.values.remove(&input.identifier);
            next.images.remove(&input.identifier);
            asked.command = Some(set(material, next));
            asked.commit = true;
        }
    } else {
        ui.label("");
    }
    // Only a value that reads differently is a change: a slider's snapping
    // to its step can nudge 0.7 to 0.7000000000000001 on its own.
    if let Some(value) = changed {
        let next = with_value(link, graph, &input.identifier, &value);
        if next != *link {
            asked.command = Some(set(material, next));
        }
    }
    if done {
        asked.commit = true;
    }
}

impl Renders {
    /// The graph's own picture as a texture for the panel.
    fn thumbnail(&mut self, ctx: &egui::Context, file: &Path, graph: &Graph) -> Option<egui::TextureHandle> {
        let key = (file.to_path_buf(), graph.url.clone());
        if let Some(known) = self.thumbnails.get(&key) {
            return known.clone();
        }
        let texture = graph.thumbnail.as_deref().and_then(decode_png).map(|image| ctx.load_texture(format!("substance-{}-{}", file.display(), graph.url), image, egui::TextureOptions::LINEAR));
        self.thumbnails.insert(key, texture.clone());
        texture
    }

    /// Show the dialog the panel asked for, after it has drawn, and say what
    /// to apply.
    pub fn answer(&mut self, project: &mut Project) -> Option<Command> {
        let ask = self.asked.take()?;
        match ask {
            Ask::Tools => {
                if let Some(folder) = rfd::FileDialog::new().set_title("The folder that holds Adobe's sbsrender").pick_folder() {
                    let _ = self.set_tools_folder(Some(folder));
                }
                None
            }
            Ask::Image { material, input } => {
                let path = rfd::FileDialog::new()
                    .set_title(format!("Picture for {input}"))
                    .add_filter("Images", &["png", "jpg", "jpeg", "tif", "tiff", "tga", "bmp", "exr", "hdr", "psd", "webp"])
                    .pick_file()?;
                let mut link = project.document().materials.get(material)?.substance.as_deref()?.clone();
                link.images.insert(input, path);
                Some(set(material, link))
            }
            Ask::OwnMaps(material) => {
                if let Err(error) = own_maps(project, material) {
                    project.status = error;
                }
                None
            }
            Ask::Relink(material) => {
                let path = pick()?;
                let mut link = project.document().materials.get(material)?.substance.as_deref()?.clone();
                link.file = path.clone();
                if let Ok((_, hash)) = self.describe_with_hash(&path) {
                    // The same file found elsewhere keeps its renders; another
                    // version of it renders afresh.
                    link.file_hash = hash;
                }
                Some(set(material, link))
            }
        }
    }
}

fn decode_png(bytes: &[u8]) -> Option<egui::ColorImage> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).ok()?;
    let (width, height) = (info.width as usize, info.height as usize);
    let data = &buffer[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data.chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => data.chunks(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    (rgba.len() == width * height * 4).then(|| egui::ColorImage::from_rgba_unmultiplied([width, height], &rgba))
}

#[cfg(test)]
mod tests;
