//! Studio preset files (`docs/studio-presets.md`; the schema is
//! `docs/schemas/studio-preset.schema.json`).
//!
//! A preset is a folder: `<id>.studio.json`, its HDRI and other pictures
//! under `textures/`, its thumbnail, and its material presets under
//! `materials/`. Every path in the preset is relative to its file, and none
//! may leave its folder, as a material package's may not. Shared as one
//! `.tqstudio` file: the folder, zipped.
//!
//! Written with Tequila's own readers and writers for each part -
//! environment, lighting, render settings, image style, lights, cameras and
//! Studios - so a preset holds exactly what a project would.

use crate::json::Value;
use crate::json_write::Object;
use crate::library::{self, LibraryEntry, Source};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tequila_core::math::Mat4;
use tequila_scene::document::EnvironmentSettings;
use tequila_scene::preset::{Framing, FramingScale, MaterialRule, PresetLight, PresetMaterial, StudioPreset};

/// What the file says it is.
pub const FORMAT: &str = "tequila.studio-preset";
/// The newest version this reads; a newer one is refused with a reason.
pub const VERSION: u32 = 1;
/// A preset file's name ends so.
pub const EXTENSION: &str = ".studio.json";
/// A preset package: the folder, zipped.
pub const PACKAGE_EXTENSION: &str = "tqstudio";

/// Where a preset came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresetSource {
    /// Shipped with Tequila, read-only.
    Builtin,
    /// The person's own: saved or installed.
    User,
}

/// A preset as read from its folder.
#[derive(Clone, Debug, PartialEq)]
pub struct PresetFile {
    pub preset: StudioPreset,
    /// Its `.studio.json`.
    pub path: PathBuf,
    pub thumbnail: Option<PathBuf>,
    pub source: PresetSource,
    /// Keys this build does not know, kept as written: a preset from a newer
    /// Tequila keeps them when it is exported again.
    pub unknown: BTreeMap<String, Value>,
}

impl PresetFile {
    /// The folder it lives in.
    pub fn folder(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new("."))
    }
}

/// The folder a person's presets live in: `<settings>/studios`, one folder
/// each.
pub fn user_directory() -> PathBuf {
    if let Some(dir) = std::env::var_os("TEQUILA_STUDIO_PRESETS") {
        return PathBuf::from(dir);
    }
    library::settings_directory().join("studios")
}

/// Where the shipped presets are: `assets/studios`, found as the material
/// library is.
pub fn builtin_directory() -> PathBuf {
    let library = library::standard_directory();
    library
        .parent()
        .and_then(|l| l.parent())
        .map(|assets| assets.join("studios"))
        .unwrap_or_else(|| PathBuf::from("assets/studios"))
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// How the materials and pictures are written.
enum Layout<'a> {
    /// In a folder: pictures and materials as paths relative to it.
    Folder { materials: &'a [String] },
    /// In one value - a journal line - with absolute paths and the materials
    /// inline.
    Inline,
}

fn to_json(preset: &StudioPreset, layout: Layout, thumbnail: Option<&str>, unknown: &BTreeMap<String, Value>) -> Value {
    let mut root = Object::new();
    for (key, value) in unknown {
        root.set(key, value.clone());
    }
    root.set("format", FORMAT)
        .set("version", f64::from(VERSION))
        .set("id", preset.id.clone())
        .set("name", preset.name.clone())
        .set("family", preset.family.clone())
        .set("size", preset.size.clone())
        .set("description", preset.description.clone())
        .set("tags", preset.tags.iter().cloned().map(Value::from).collect::<Vec<_>>())
        .set("preset_version", preset.version.clone())
        .set_some("thumbnail", thumbnail.map(str::to_string))
        .set("environment", crate::environment_to_json(&preset.environment))
        .set("lighting", crate::lighting_to_json(&preset.lighting))
        .set("render", crate::render_settings_to_json(&preset.render))
        .set("image_style", crate::image_style_to_json(&preset.image_style))
        .set(
            "lights",
            preset
                .lights
                .iter()
                .map(|light| {
                    let mut entry = Object::new();
                    entry
                        .set("name", light.name.clone())
                        .set("light", crate::light_to_json(&light.settings))
                        .set("transform", light.transform.m.iter().flatten().map(|v| Value::from(*v)).collect::<Vec<_>>());
                    entry.build()
                })
                .collect::<Vec<_>>(),
        )
        .set("cameras", preset.cameras.iter().map(crate::camera_to_json).collect::<Vec<_>>())
        .set("studios", preset.studios.iter().map(crate::studio_to_json).collect::<Vec<_>>())
        .set("framing", {
            let mut framing = Object::new();
            framing
                .set("reference_radius_m", preset.framing.reference_radius)
                .set("base_offset_m", preset.framing.base_offset)
                .set("scale", preset.framing.scale.key());
            framing.build()
        })
        .set(
            "materials",
            match layout {
                Layout::Folder { materials } => materials.iter().cloned().map(Value::from).collect::<Vec<_>>(),
                Layout::Inline => preset
                    .materials
                    .iter()
                    .filter_map(|m| crate::json::parse(library::write_entry(&entry_of(m)).as_bytes()).ok())
                    .collect::<Vec<_>>(),
            },
        )
        .set(
            "material_rules",
            preset
                .material_rules
                .iter()
                .map(|rule| {
                    let mut entry = Object::new();
                    entry.set("pattern", rule.pattern.clone()).set("material", rule.material.clone());
                    entry.build()
                })
                .collect::<Vec<_>>(),
        )
        .set("notes", preset.notes.clone());
    root.build()
}

/// A preset in one value, its paths absolute and its materials inline: what
/// a journal line or a script carries.
pub fn to_inline(preset: &StudioPreset) -> Value {
    to_json(preset, Layout::Inline, None, &BTreeMap::new())
}

/// A library entry for one of a preset's materials.
fn entry_of(material: &PresetMaterial) -> LibraryEntry {
    LibraryEntry {
        name: material.definition.name.clone(),
        category: material.category.clone(),
        tags: material.tags.clone(),
        material_type: material.definition.material_type,
        material: material.definition.material().clone(),
        graph: material.definition.graph.clone(),
        textures: material.textures.clone(),
        source: Source::User,
        path: String::new(),
    }
}

/// One of a preset's materials, from a library entry.
pub fn material_of(entry: &LibraryEntry) -> PresetMaterial {
    PresetMaterial { definition: entry.definition(), textures: entry.textures.clone(), category: entry.category.clone(), tags: entry.tags.clone() }
}

/// A file name made of `name`: what a preset's id is when none is given.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "studio".into() } else { out }
}

/// Every picture path the environment names: its image, backplate and
/// image pins'.
fn pictures(environment: &mut EnvironmentSettings, mut each: impl FnMut(&mut String)) {
    if let Some(image) = &mut environment.image {
        each(image);
    }
    if let Some(plate) = &mut environment.backplate {
        each(plate);
    }
    for pin in &mut environment.pins {
        if let tequila_render_core::pins::PinKind::Image { path } = &mut pin.kind {
            each(path);
        }
    }
}

/// Write `preset` as a folder at `folder`: its pictures copied into
/// `textures/`, its materials into `materials/`, the thumbnail beside it,
/// and `<id>.studio.json`. Returns the file.
pub fn write_folder(preset: &StudioPreset, folder: &Path, thumbnail: Option<&[u8]>) -> Result<PathBuf, String> {
    write_folder_keeping(preset, folder, thumbnail, &BTreeMap::new())
}

fn write_folder_keeping(preset: &StudioPreset, folder: &Path, thumbnail: Option<&[u8]>, unknown: &BTreeMap<String, Value>) -> Result<PathBuf, String> {
    let failed = |what: &Path, e: std::io::Error| format!("{}: {e}", what.display());
    std::fs::create_dir_all(folder).map_err(|e| failed(folder, e))?;
    let mut written = preset.clone();
    // The pictures, copied in and named by where they now are.
    let textures = folder.join("textures");
    let mut used = std::collections::BTreeSet::new();
    let mut problem = None;
    pictures(&mut written.environment, |path| {
        let source = PathBuf::from(&*path);
        if problem.is_some() || inside(folder, &source) {
            if let Ok(relative) = source.strip_prefix(folder) {
                *path = relative.to_string_lossy().replace('\\', "/");
            }
            return;
        }
        let name = source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "picture".into());
        let name = if used.insert(name.to_lowercase()) { name } else { format!("{}_{name}", used.len()) };
        if let Err(e) = std::fs::create_dir_all(&textures).and_then(|_| std::fs::copy(&source, textures.join(&name))) {
            problem = Some(failed(&source, e));
            return;
        }
        *path = format!("textures/{name}");
    });
    if let Some(problem) = problem {
        return Err(problem);
    }
    // The materials, as library presets of their own.
    let materials_dir = folder.join("materials");
    let mut files = Vec::new();
    if !preset.materials.is_empty() {
        let _ = std::fs::remove_dir_all(&materials_dir);
    }
    for material in &preset.materials {
        let saved = library::save_entry(&materials_dir, &entry_of(material)).map_err(|e| format!("{e}"))?;
        let relative = saved.strip_prefix(folder).unwrap_or(&saved).to_string_lossy().replace('\\', "/");
        files.push(relative);
    }
    let thumbnail_name = match thumbnail {
        Some(bytes) => {
            let file = folder.join("thumbnail.png");
            std::fs::write(&file, bytes).map_err(|e| failed(&file, e))?;
            Some("thumbnail.png")
        }
        None => folder.join("thumbnail.png").is_file().then_some("thumbnail.png"),
    };
    let file = folder.join(format!("{}{EXTENSION}", written.id));
    let text = crate::json_write::to_string(&to_json(&written, Layout::Folder { materials: &files }, thumbnail_name, unknown));
    std::fs::write(&file, text).map_err(|e| failed(&file, e))?;
    Ok(file)
}

fn inside(folder: &Path, path: &Path) -> bool {
    match (folder.canonicalize(), path.canonicalize()) {
        (Ok(folder), Ok(path)) => path.starts_with(folder),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// The keys this build reads.
const KNOWN: [&str; 21] = [
    "format",
    "version",
    "id",
    "name",
    "family",
    "size",
    "description",
    "tags",
    "preset_version",
    "thumbnail",
    "environment",
    "lighting",
    "render",
    "image_style",
    "lights",
    "cameras",
    "studios",
    "framing",
    "materials",
    "material_rules",
    "notes",
];

/// A path the preset names, inside its folder: relative, and never up out
/// of it or onto another drive.
fn within(folder: &Path, relative: &str, what: &str) -> Result<PathBuf, String> {
    let clean = relative.replace('\\', "/");
    if Path::new(&clean).is_absolute() || clean.starts_with('/') || clean.split('/').any(|part| part == ".." || part.contains(':')) {
        return Err(format!("{what} `{relative}` leaves the preset's folder: every path must be inside it"));
    }
    Ok(folder.join(clean))
}

/// Read a preset from its value. With a folder, its paths are inside it and
/// its materials are files there; without one (a journal line), its paths
/// are as written and its materials inline.
fn parse(root: &Value, folder: Option<&Path>) -> Result<(StudioPreset, Option<PathBuf>, BTreeMap<String, Value>), String> {
    let Value::Object(map) = root else { return Err("a studio preset is a JSON object".into()) };
    match root.get("format").and_then(|v| v.as_str()) {
        Some(FORMAT) => {}
        Some(other) => return Err(format!("this is a `{other}` file, not a Tequila studio preset (`{FORMAT}`)")),
        None => return Err(format!("this is not a Tequila studio preset: it has no \"format\": \"{FORMAT}\"")),
    }
    let version = root.get("version").and_then(|v| v.as_index()).ok_or("a studio preset needs a whole-number \"version\"")?;
    if version as u32 > VERSION {
        return Err(format!("this preset is version {version}, made by a newer Tequila; this one reads version {VERSION}"));
    }
    let text = |key: &str| root.get(key).and_then(|v| v.as_str()).map(str::to_string);
    let id = text("id").filter(|s| !s.trim().is_empty()).ok_or("a studio preset needs an \"id\"")?;
    let name = text("name").filter(|s| !s.trim().is_empty()).ok_or("a studio preset needs a \"name\"")?;
    if id.contains(['/', '\\', ':']) || id.contains("..") {
        return Err(format!("the id `{id}` cannot name a folder: use letters, digits and dashes"));
    }

    let mut environment = crate::read_environment(root);
    if let Some(folder) = folder {
        let mut problem = None;
        pictures(&mut environment, |path| match within(folder, path, "a picture") {
            Ok(full) => *path = full.to_string_lossy().into_owned(),
            Err(e) => problem = Some(e),
        });
        if let Some(problem) = problem {
            return Err(problem);
        }
    }

    let mut lights = Vec::new();
    for (i, entry) in root.get("lights").map(|v| v.as_array()).unwrap_or_default().iter().enumerate() {
        let values = entry.get("transform").and_then(|t| t.as_f32_array()).filter(|v| v.len() == 16);
        let Some(values) = values else { return Err(format!("`lights[{i}]` needs a \"transform\" of 16 numbers")) };
        let mut m = Mat4::IDENTITY;
        for (k, v) in values.iter().enumerate() {
            m.m[k / 4][k % 4] = *v;
        }
        lights.push(PresetLight {
            name: entry.get("name").and_then(|v| v.as_str()).unwrap_or("Light").to_string(),
            settings: entry.get("light").map(crate::read_light).unwrap_or_default(),
            transform: m,
        });
    }

    let framing = match root.get("framing") {
        Some(f) => Framing {
            reference_radius: f.get("reference_radius_m").and_then(|v| v.as_f32()).filter(|r| *r > 0.0).ok_or("`framing` needs a \"reference_radius_m\" above 0")?,
            base_offset: f.get("base_offset_m").and_then(|v| v.as_f32()).unwrap_or(0.0),
            scale: match f.get("scale").and_then(|v| v.as_str()) {
                None => FramingScale::Cameras,
                Some(s) => FramingScale::from_key(s).ok_or_else(|| format!("`framing.scale` is `{s}`: cameras, cameras_and_lights or none"))?,
            },
        },
        None => Framing::default(),
    };

    let mut materials = Vec::new();
    for (i, entry) in root.get("materials").map(|v| v.as_array()).unwrap_or_default().iter().enumerate() {
        let read = match (entry, folder) {
            (Value::String(relative), Some(folder)) => {
                let file = within(folder, relative, "a material")?;
                let text = std::fs::read_to_string(&file).map_err(|e| format!("its material {relative}: {e}"))?;
                let hint = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let mut entry = library::read_entry(&text, &hint).map_err(|e| format!("its material {relative}: {e}"))?;
                let beside = file.parent().unwrap_or(folder).to_path_buf();
                for (_, picture) in &mut entry.textures {
                    if picture.path.is_relative() {
                        let rel = picture.path.to_string_lossy().into_owned();
                        let full = beside.join(&rel);
                        if !inside(folder, &full) && full.exists() {
                            return Err(format!("its material {relative}: the picture `{rel}` leaves the preset's folder"));
                        }
                        picture.path = full;
                    }
                }
                entry
            }
            (Value::Object(_), _) => library::read_entry(&crate::json_write::to_string(entry), "material").map_err(|e| format!("`materials[{i}]`: {e}"))?,
            _ => return Err(format!("`materials[{i}]` is neither a material file's path nor a material")),
        };
        materials.push(material_of(&read));
    }

    let material_rules = root
        .get("material_rules")
        .map(|v| v.as_array())
        .unwrap_or_default()
        .iter()
        .filter_map(|r| Some(MaterialRule { pattern: r.get("pattern")?.as_str()?.to_string(), material: r.get("material")?.as_str()?.to_string() }))
        .collect();

    let thumbnail = match (text("thumbnail"), folder) {
        (Some(relative), Some(folder)) => Some(within(folder, &relative, "the thumbnail")?),
        (Some(path), None) => Some(PathBuf::from(path)),
        _ => None,
    };

    let preset = StudioPreset {
        id,
        name,
        family: text("family").unwrap_or_default(),
        size: text("size").unwrap_or_default(),
        description: text("description").unwrap_or_default(),
        tags: root.get("tags").map(|t| t.as_array().iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default(),
        version: text("preset_version").unwrap_or_default(),
        environment,
        lighting: crate::read_lighting(root),
        render: root.get("render").map(crate::read_render_settings_value).unwrap_or_default(),
        image_style: root.get("image_style").map(crate::read_image_style_value).unwrap_or_default(),
        lights,
        cameras: root.get("cameras").map(|v| v.as_array().iter().map(crate::read_camera).collect()).unwrap_or_default(),
        studios: root.get("studios").map(|v| v.as_array().iter().map(crate::read_studio).collect()).unwrap_or_default(),
        framing,
        materials,
        material_rules,
        notes: text("notes").unwrap_or_default(),
    };
    let unknown = map.iter().filter(|(k, _)| !KNOWN.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
    Ok((preset, thumbnail, unknown))
}

/// A preset from a value [`to_inline`] wrote.
pub fn from_inline(value: &Value) -> Result<StudioPreset, String> {
    parse(value, None).map(|(preset, _, _)| preset)
}

/// Read the preset at `path`, a `.studio.json` in its folder.
pub fn read(path: &Path, source: PresetSource) -> Result<PresetFile, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let value = crate::json::parse(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    let folder = path.parent().unwrap_or(Path::new("."));
    let (preset, thumbnail, unknown) = parse(&value, Some(folder)).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(PresetFile { preset, path: path.to_path_buf(), thumbnail: thumbnail.filter(|t| t.is_file()), source, unknown })
}

/// The preset file in `folder`: its one `*.studio.json`.
pub fn file_in(folder: &Path) -> Option<PathBuf> {
    std::fs::read_dir(folder)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.file_name().is_some_and(|n| n.to_string_lossy().to_ascii_lowercase().ends_with(EXTENSION)))
        .min()
}

/// Every preset, built-in first, then the person's, each family's sizes
/// together; problems with single presets are returned rather than failing
/// the lot.
pub fn list() -> (Vec<PresetFile>, Vec<String>) {
    list_in(&[(builtin_directory(), PresetSource::Builtin), (user_directory(), PresetSource::User)])
}

/// [`list`], over the folders given.
pub fn list_in(roots: &[(PathBuf, PresetSource)]) -> (Vec<PresetFile>, Vec<String>) {
    let mut found = Vec::new();
    let mut problems = Vec::new();
    for (root, source) in roots {
        let Ok(entries) = std::fs::read_dir(root) else { continue };
        let mut folders: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        folders.sort();
        for folder in folders {
            let Some(file) = file_in(&folder) else { continue };
            match read(&file, *source) {
                Ok(preset) => found.push(preset),
                Err(problem) => problems.push(problem),
            }
        }
    }
    found.sort_by(|a, b| (a.preset.family.as_str(), size_rank(&a.preset.size), a.preset.name.as_str()).cmp(&(b.preset.family.as_str(), size_rank(&b.preset.size), b.preset.name.as_str())));
    (found, problems)
}

/// Small before Medium before Large, the rest after in their own order.
fn size_rank(size: &str) -> (u8, String) {
    let rank = match size.to_ascii_lowercase().as_str() {
        "xs" | "extra small" => 0,
        "small" | "s" => 1,
        "medium" | "m" => 2,
        "large" | "l" => 3,
        "xl" | "extra large" => 4,
        _ => 5,
    };
    (rank, size.to_string())
}

// ---------------------------------------------------------------------------
// Sharing
// ---------------------------------------------------------------------------

/// Save `preset` as one of the person's own, replacing one of its id.
pub fn save(preset: &StudioPreset, thumbnail: Option<&[u8]>) -> Result<PresetFile, String> {
    let folder = user_directory().join(&preset.id);
    let _ = std::fs::remove_dir_all(&folder);
    let file = write_folder(preset, &folder, thumbnail)?;
    read(&file, PresetSource::User)
}

/// Write `preset` as a package at `to`: its folder, zipped.
pub fn export(preset: &PresetFile, to: &Path) -> Result<(), String> {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let scratch = std::env::temp_dir().join(format!("tequila-studio-export-{}-{stamp}", std::process::id()));
    let thumbnail = preset.thumbnail.as_ref().and_then(|t| std::fs::read(t).ok());
    let result = write_folder_keeping(&preset.preset, &scratch, thumbnail.as_deref(), &preset.unknown).and_then(|_| {
        let mut zip = crate::zip::ZipWriter::new();
        add_folder(&mut zip, &scratch, &scratch)?;
        std::fs::write(to, zip.finish()).map_err(|e| format!("{}: {e}", to.display()))
    });
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

fn add_folder(zip: &mut crate::zip::ZipWriter, root: &Path, folder: &Path) -> Result<(), String> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(folder).map_err(|e| format!("{}: {e}", folder.display()))?.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            add_folder(zip, root, &path)?;
        } else {
            let name = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
            zip.add(name, std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?);
        }
    }
    Ok(())
}

/// What installing did.
#[derive(Clone, Debug, PartialEq)]
pub struct Installed {
    pub preset: PresetFile,
    /// The materials added to the person's library.
    pub materials: Vec<String>,
}

/// Install the preset in `from` - a package, a preset's folder or its
/// `.studio.json` - as one of the person's own, and its materials into
/// their library (`library_dir`), replacing what an earlier install of it
/// put there.
pub fn install(from: &Path, library_dir: &Path) -> Result<Installed, String> {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let scratch = std::env::temp_dir().join(format!("tequila-studio-install-{}-{stamp}", std::process::id()));
    let file = if from.is_dir() {
        file_in(from).ok_or_else(|| format!("{} holds no {EXTENSION} file", from.display()))?
    } else if from.extension().is_some_and(|e| e.eq_ignore_ascii_case(PACKAGE_EXTENSION) || e.eq_ignore_ascii_case("zip")) {
        let bytes = std::fs::read(from).map_err(|e| format!("{}: {e}", from.display()))?;
        let archive = crate::zip::ZipArchive::read(&bytes).map_err(|e| format!("{}: {e:?}", from.display()))?;
        for name in archive.names() {
            let clean = name.replace('\\', "/");
            if clean.starts_with('/') || clean.split('/').any(|part| part == ".." || part.contains(':')) {
                return Err(format!("{}: `{name}` would be written outside the preset", from.display()));
            }
            if clean.ends_with('/') {
                continue;
            }
            let target = scratch.join(&clean);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            std::fs::write(&target, archive.get(name).unwrap_or_default()).map_err(|e| format!("{}: {e}", target.display()))?;
        }
        file_in(&scratch).ok_or_else(|| format!("{} holds no {EXTENSION} file", from.display()))?
    } else {
        from.to_path_buf()
    };
    let result = (|| {
        let read = read(&file, PresetSource::User)?;
        let thumbnail = read.thumbnail.as_ref().and_then(|t| std::fs::read(t).ok());
        let folder = user_directory().join(&read.preset.id);
        // Written from what was read rather than copied, so a preset whose
        // folder holds anything else installs only what it names.
        let staged = scratch.join("__installing");
        write_folder_keeping(&read.preset, &staged, thumbnail.as_deref(), &read.unknown)?;
        let _ = std::fs::remove_dir_all(&folder);
        if let Some(parent) = folder.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        copy_folder(&staged, &folder)?;
        let installed = self::read(&folder.join(format!("{}{EXTENSION}", read.preset.id)), PresetSource::User)?;
        // The materials, into the library, each in place of one of its name
        // that an earlier install put there.
        let (existing, _) = library::read_directory_from(library_dir, Source::User);
        let mut materials = Vec::new();
        for material in &read.preset.materials {
            for old in existing.iter().filter(|e| e.name == material.definition.name && !e.path.is_empty()) {
                let _ = library::delete_entry(old);
            }
            library::save_entry(library_dir, &entry_of(material)).map_err(|e| format!("{e}"))?;
            materials.push(material.definition.name.clone());
        }
        Ok(Installed { preset: installed, materials })
    })();
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

fn copy_folder(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    for entry in std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?.flatten() {
        let path = entry.path();
        let target = to.join(entry.file_name());
        if path.is_dir() {
            copy_folder(&path, &target)?;
        } else {
            std::fs::copy(&path, &target).map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    Ok(())
}

/// Remove one of the person's presets. A built-in one is not theirs to.
pub fn delete(preset: &PresetFile) -> Result<(), String> {
    if preset.source != PresetSource::User {
        return Err(format!("{} comes with Tequila and cannot be deleted", preset.preset.name));
    }
    std::fs::remove_dir_all(preset.folder()).map_err(|e| format!("{}: {e}", preset.folder().display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_scene::preset::capture;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tequila-preset-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample() -> StudioPreset {
        let document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.3);
        let mut preset = capture(&document, "sample-studio", "Sample Studio", "Samples", "Small");
        preset.material_rules = vec![MaterialRule { pattern: "Metal; Screws*".into(), material: "Steel".into() }];
        preset.notes = "made in a test".into();
        preset
    }

    /// A preset written to a folder reads back as it was, its unknown keys
    /// kept, and inline too.
    #[test]
    fn a_preset_survives_its_folder_and_a_journal_line() {
        let folder = scratch("round-trip");
        let preset = sample();
        let file = write_folder(&preset, &folder, Some(b"\x89PNG")).unwrap();
        assert!(file.to_string_lossy().ends_with("sample-studio.studio.json"));
        let back = read(&file, PresetSource::User).unwrap();
        assert_eq!(back.preset, preset);
        assert!(back.thumbnail.is_some());
        assert_eq!(from_inline(&to_inline(&preset)).unwrap(), preset);

        // A key from a newer Tequila stays when the preset goes round again.
        let mut value = crate::json::parse(&std::fs::read(&file).unwrap()).unwrap();
        if let Value::Object(map) = &mut value {
            map.insert("future".into(), Value::from("kept"));
        }
        std::fs::write(&file, crate::json_write::to_string(&value)).unwrap();
        let kept = read(&file, PresetSource::User).unwrap();
        assert_eq!(kept.unknown.get("future").and_then(|v| v.as_str()), Some("kept"));
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// What is not a preset, or one too new, or one whose paths leave its
    /// folder, is refused with a reason that says so.
    #[test]
    fn a_bad_preset_is_refused_with_a_reason() {
        let folder = scratch("refused");
        let good = to_inline(&sample());
        let refused = |edit: &dyn Fn(&mut BTreeMap<String, Value>)| {
            let mut value = good.clone();
            if let Value::Object(map) = &mut value {
                edit(map);
            }
            parse(&value, Some(&folder)).err().unwrap_or_default()
        };
        assert!(refused(&|m| { m.remove("format"); }).contains("not a Tequila studio preset"));
        assert!(refused(&|m| { m.insert("version".into(), Value::from(9.0)); }).contains("newer Tequila"));
        assert!(refused(&|m| { m.remove("id"); }).contains("\"id\""));
        assert!(refused(&|m| { m.insert("id".into(), Value::from("../escape")); }).contains("cannot name a folder"));
        assert!(refused(&|m| { m.insert("thumbnail".into(), Value::from("../../secret.png")); }).contains("leaves the preset"));
        assert!(refused(&|m| { m.insert("framing".into(), crate::json::parse(b"{\"reference_radius_m\": 0}").unwrap()); }).contains("reference_radius_m"));
        assert!(parse(&good, Some(&folder)).is_ok());
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Exported and installed elsewhere, a preset is the same preset, and
    /// its materials land in the library.
    #[test]
    fn a_preset_exports_and_installs() {
        let folder = scratch("share");
        let presets = folder.join("presets");
        let library_dir = folder.join("library");
        std::env::set_var("TEQUILA_STUDIO_PRESETS", &presets);
        let mut preset = sample();
        let entry = library::read_entry(r#"{"name": "Steel", "category": "Samples", "type": "metal"}"#, "steel").unwrap();
        preset.materials = vec![material_of(&entry)];
        let made = folder.join("made");
        let file = write_folder(&preset, &made, None).unwrap();
        let original = read(&file, PresetSource::User).unwrap();
        let package = folder.join("sample.tqstudio");
        export(&original, &package).unwrap();
        let installed = install(&package, &library_dir).unwrap();
        assert_eq!(installed.preset.preset, original.preset);
        assert_eq!(installed.materials, vec!["Steel".to_string()]);
        let (library, _) = library::read_directory_from(&library_dir, Source::User);
        assert!(library.iter().any(|e| e.name == "Steel" && e.category == "Samples"));
        // Installed again, it replaces itself rather than doubling up.
        install(&package, &library_dir).unwrap();
        let (library, _) = library::read_directory_from(&library_dir, Source::User);
        assert_eq!(library.iter().filter(|e| e.name == "Steel").count(), 1);
        let (listed, problems) = list_in(&[(presets.clone(), PresetSource::User)]);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(listed.len(), 1);
        delete(&listed[0]).unwrap();
        std::env::remove_var("TEQUILA_STUDIO_PRESETS");
        let _ = std::fs::remove_dir_all(&folder);
    }
}
