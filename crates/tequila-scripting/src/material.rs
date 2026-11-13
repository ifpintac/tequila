//! Materials made from a recipe: how a script, a plugin or an AI app makes
//! a material of its own, in one request rather than a node graph built a
//! wire at a time.
//!
//! A recipe is a library preset as JSON (`tequila_format::library::
//! read_entry`): a type, the parameters it changes, and `patterns` -
//! procedurals driving parameters through colour ramps and ranges, laid over
//! one another with masks. `guide` describes it all, with tested examples to
//! start from; `create` makes one and puts it on parts; `update` replaces a
//! material's look in place, for trying again; `recipe` reads any material
//! back as one; `preview` renders one on the library's ball without touching
//! the scene; `save` keeps one in the person's library; `tint` makes one the
//! same material in another colour, wherever its colour is kept.

use crate::document::LibraryOutcome;
use crate::protocol::{LibraryRequest, MaterialRequest};
use tequila_format::json::{self, Value};
use tequila_format::json_write::{self, Object};
use tequila_format::library::{self, LibraryEntry, Source};
use tequila_materials::graph::MaterialGraph;
use tequila_scene::tree::{MaterialKey, NodeId};
use tequila_scene::{Command, SceneDocument};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

/// The actions, for the protocol and its documentation.
pub const MATERIAL_ACTIONS: &[&str] = &["guide", "create", "update", "recipe", "preview", "save", "tint"];

/// The guide's hand-written half: how to write a recipe, what each pattern
/// looks like, and examples. Each example is checked to read and compile.
const GUIDE: &str = include_str!("material_guide.json");

/// Carry out a `material` request for the application or a headless session:
/// `create` and `update` come back as commands for the caller's bus, the
/// rest as answers.
pub fn material_request(
    request: &MaterialRequest,
    document: &SceneDocument,
    selection: &[NodeId],
    user: &Path,
) -> Result<LibraryOutcome, String> {
    match request.action.as_str() {
        "guide" => Ok(LibraryOutcome::Done(guide())),
        "create" => create(request, document, selection),
        "update" => update(request, document),
        "recipe" => {
            let key = material_of(request, document)?;
            let definition = document.materials.get(key).expect("checked");
            let entry = LibraryEntry::from_graph(definition.name.clone(), "My materials".into(), definition.material_type, &definition.graph, Source::User);
            let recipe = json::parse(library::write_entry(&entry).as_bytes()).map_err(|e| e.to_string())?;
            let mut out = Object::new();
            out.set("material", key.0 as f64).set("recipe", recipe);
            Ok(LibraryOutcome::Done(out.build()))
        }
        "preview" => preview(request, document).map(LibraryOutcome::Done),
        "tint" => tint(request, document),
        "save" => {
            let key = material_of(request, document)?;
            let save = LibraryRequest {
                action: "save".into(),
                name: request.name.clone(),
                material: Some(key.0),
                category: request.category.clone(),
                tags: request.tags.clone(),
                ..Default::default()
            };
            crate::document::library_request(&save, document, selection, user, &|_, _, _, _, _| Err("not a pack".into()))
        }
        other => Err(format!("`{other}` is not a material action; the actions are {}", MATERIAL_ACTIONS.join(", "))),
    }
}

/// A recipe read as a library entry: its errors say which pattern or field
/// is wrong, which is what lets a model fix one on its next try.
pub fn entry_of(recipe: &Value) -> Result<LibraryEntry, String> {
    let name = recipe.get("name").and_then(Value::as_str).unwrap_or("Recipe").to_string();
    if !matches!(recipe, Value::Object(_)) {
        return Err("a recipe is a JSON object: {\"name\":..., \"type\":..., \"patterns\":[...]}".into());
    }
    library::read_entry(&json_write::to_string(recipe), &name).map_err(|e| e.to_string())
}

fn material_of(request: &MaterialRequest, document: &SceneDocument) -> Result<MaterialKey, String> {
    let key = MaterialKey(request.material.ok_or_else(|| format!("material {} needs `material`, a material id", request.action))?);
    document.materials.get(key).map(|_| key).ok_or_else(|| format!("there is no material {}; `query materials` lists them", key.0))
}

fn recipe_of(request: &MaterialRequest) -> Result<LibraryEntry, String> {
    entry_of(request.recipe.as_ref().ok_or_else(|| format!("material {} needs a `recipe`; action `guide` shows how to write one", request.action))?)
}

/// `name`, or `name 2`, `name 3`... - whichever no material has, so a second
/// try is told from the first in the material list.
fn unused_name(document: &SceneDocument, name: &str) -> String {
    let taken = |candidate: &str| document.materials.iter().any(|(_, d)| d.name == candidate);
    if !taken(name) {
        return name.to_string();
    }
    (2..).map(|n| format!("{name} {n}")).find(|c| !taken(c)).expect("an unbounded range has a free name")
}

fn create(request: &MaterialRequest, document: &SceneDocument, selection: &[NodeId]) -> Result<LibraryOutcome, String> {
    let entry = recipe_of(request)?;
    let nodes: Vec<NodeId> = if request.nodes.is_empty() { selection.to_vec() } else { request.nodes.iter().map(|&n| NodeId(n)).collect() };
    if let Some(missing) = nodes.iter().find(|n| document.tree.get(**n).is_none()) {
        return Err(format!("no such part: {}", missing.0));
    }
    let key = MaterialKey(document.materials.next_key());
    let (mut definition, textures) = entry.for_document(document);
    definition.name = unused_name(document, &entry.name);
    let name = definition.name.clone();
    let mut commands = vec![Command::Import {
        payload: Box::new(tequila_scene::command::ImportPayload {
            meshes: Vec::new(),
            textures,
            materials: vec![(key, definition)],
            nodes: Vec::new(),
            roots: Vec::new(),
        }),
    }];
    if !nodes.is_empty() {
        commands.push(Command::AssignMaterial { nodes: nodes.clone(), material: key });
    }
    let mut out = Object::new();
    out.set("material", key.0 as f64)
        .set("name", name)
        .set("type", entry.material_type.id())
        .set("parts", nodes.len() as f64)
        .set("nodes", entry.graph.len() as f64);
    if nodes.is_empty() {
        out.set("note", "made, on no part: name `nodes`, or select parts, or assign it with material.assign");
    }
    Ok(LibraryOutcome::Commands(commands, out.build()))
}

fn update(request: &MaterialRequest, document: &SceneDocument) -> Result<LibraryOutcome, String> {
    let key = material_of(request, document)?;
    let entry = recipe_of(request)?;
    let current = document.materials.get(key).expect("checked");
    let (definition, textures) = entry.for_document(document);
    let mut commands = Vec::new();
    if !textures.is_empty() {
        commands.push(Command::Import {
            payload: Box::new(tequila_scene::command::ImportPayload {
                meshes: Vec::new(),
                textures,
                materials: Vec::new(),
                nodes: Vec::new(),
                roots: Vec::new(),
            }),
        });
    }
    if current.material_type != entry.material_type {
        commands.push(Command::SetMaterialType { material: key, material_type: entry.material_type });
    }
    commands.push(Command::SetGraph { material: key, graph: Box::new(definition.graph) });
    // The recipe's name when it gives one, as a new version of the same idea
    // usually does not need a new name.
    if request.recipe.as_ref().and_then(|r| r.get("name")).is_some() && entry.name != current.name {
        commands.push(Command::RenameMaterial { material: key, name: unused_name(document, &entry.name) });
    }
    let mut out = Object::new();
    out.set("material", key.0 as f64)
        .set("type", entry.material_type.id())
        .set("nodes", entry.graph.len() as f64)
        .set("parts", document.tree.iter().filter(|n| n.material == Some(key)).count() as f64);
    Ok(LibraryOutcome::Commands(commands, out.build()))
}

/// The material `material` made the same material in `color`
/// (`tequila_materials::tint`): one command, so one undo step, and an answer
/// that says where the colour was found. A material shared between parts
/// changes on all of them, as any edit to it does, and the answer says how
/// many.
fn tint(request: &MaterialRequest, document: &SceneDocument) -> Result<LibraryOutcome, String> {
    let key = material_of(request, document)?;
    let colour = request.color.ok_or("material tint needs `color`: a hex sRGB string like \"#1f4d2b\"")?;
    let definition = document.materials.get(key).expect("checked");
    let mut graph = definition.graph.clone();
    let done = tequila_materials::tint::tint(&mut graph, definition.material_type, colour).map_err(|why| format!("`{}` {why}", definition.name))?;
    let parts = document.tree.iter().filter(|n| n.material == Some(key)).count();
    let mut out = Object::new();
    out.set("material", key.0 as f64).set("name", definition.name.clone()).set("parts", parts as f64);
    let mut notes = tint_answer(&mut out, &done);
    if parts > 1 {
        notes.push(format!("{parts} parts share this material and all of them changed; to change one alone, undo, material.unlink that part and tint its copy"));
    }
    if !notes.is_empty() {
        out.set("notes", Value::Array(notes.into_iter().map(Value::String).collect()));
    }
    Ok(LibraryOutcome::Commands(vec![Command::SetGraph { material: key, graph: Box::new(graph) }], out.build()))
}

/// What a tint did, written into an answer: the colour it came out and the
/// colour it was, as hex sRGB, and each parameter changed with how many
/// colours and where they were. Returns what the caller should also be
/// told, in words.
pub(crate) fn tint_answer(out: &mut Object, done: &tequila_materials::tint::Tint) -> Vec<String> {
    out.set("color", tequila_format::colour_hex(done.now)).set("was", tequila_format::colour_hex(done.was));
    let changed: Vec<Value> = done
        .changed
        .iter()
        .map(|change| {
            let mut entry = Object::new();
            entry
                .set("parameter", change.param.key())
                .set("colors", change.colors as f64)
                .set("in", if change.pattern { "pattern" } else { "value" });
            // Colours that are none of the main one's family - white
            // threads, a rust - and stay as they are.
            if change.kept > 0 {
                entry.set("kept", change.kept as f64);
            }
            entry.build()
        })
        .collect();
    out.set("changed", Value::Array(changed));
    let mut notes = Vec::new();
    if done.limited {
        notes.push(format!(
            "a metal reflects no more than the metal under the tint does, and the colour asked for is brighter than that: it came out {}, as near as it gets. For a brighter one set metal_ior to Aluminium or Silver and tint again",
            tequila_format::colour_hex(done.now)
        ));
    }
    if done.thinned {
        notes.push(format!(
            "a clear material is seen through its whole thickness, so it was kept as clear as it was: the colour asked for, thinned to {} at its transmission_depth. For a deeper colour shorten transmission_depth, or set transmission_color itself",
            tequila_format::colour_hex(done.now)
        ));
    }
    if done.pictures {
        notes.push("part of its colour comes from a picture (a texture map), which was left as it is".into());
    }
    if done.computed {
        notes.push("part of its colour is worked out by nodes that hold no colour, which were left as they are".into());
    }
    notes
}

/// The library's ball, lit as its tiles are; built once.
fn preview_environment() -> Arc<tequila_render_core::environment::Environment> {
    static ENVIRONMENT: OnceLock<Arc<tequila_render_core::environment::Environment>> = OnceLock::new();
    ENVIRONMENT.get_or_init(|| Arc::new(tequila_scene::demo::preview_environment())).clone()
}

/// Render a recipe, or a material of the document, on the preview ball and
/// write it to `path` (a temporary file when absent). The scene is not
/// touched: this is for trying ideas.
fn preview(request: &MaterialRequest, document: &SceneDocument) -> Result<Value, String> {
    use tequila_render_cpu::{render_thumbnail_textured, Device, PreviewSettings, PreviewShape};
    let (graph, pictures, name): (MaterialGraph, Vec<_>, String) = match (&request.recipe, request.material) {
        (Some(_), _) => {
            let entry = recipe_of(request)?;
            (entry.graph, entry.textures, entry.name)
        }
        (None, Some(_)) => {
            let key = material_of(request, document)?;
            let definition = document.materials.get(key).expect("checked");
            let wanted = definition.graph.compile().map(|p| p.requirements.textures).unwrap_or_default();
            let pictures = wanted.into_iter().filter_map(|id| document.textures.get(id).map(|s| (id, s.clone()))).collect();
            (definition.graph.clone(), pictures, definition.name.clone())
        }
        (None, None) => return Err("material preview needs a `recipe` or a `material`".into()),
    };
    let mut textures = tequila_render_core::shade::TextureTable::new();
    for (id, source) in &pictures {
        if let Some(texture) = tequila_scene::snapshot::load_texture(source) {
            textures.insert(*id, Arc::new(texture));
        }
    }
    let size = request.size.unwrap_or(320).clamp(64, 1024);
    let settings = PreviewSettings {
        size,
        samples: 64,
        shape: if request.shape.as_deref() == Some("slab") { PreviewShape::Slab } else { PreviewShape::Sphere },
        // A real-sized ball: the library's 5 cm one, or the size asked for,
        // since a pattern's scale only reads against an object's.
        radius: request.radius.filter(|r| r.is_finite() && *r > 0.0).unwrap_or(tequila_render_cpu::preview::PREVIEW_RADIUS),
        uv_unit: None,
    };
    let device = Device::shared()?;
    let thumbnail = render_thumbnail_textured(&device, &graph, Arc::new(textures), preview_environment(), settings)?;
    let (width, height) = (thumbnail.width, thumbnail.height);
    let denoised = tequila_denoise::Denoiser::shared().ok().and_then(|denoiser| {
        denoiser
            .denoise(
                width,
                height,
                &thumbnail.pixels,
                &thumbnail.albedo,
                &thumbnail.normal,
                tequila_denoise::DenoiseSettings { quality: tequila_denoise::Quality::High, strength: 1.0, prefilter_guides: true },
            )
            .ok()
    });
    let color: Vec<tequila_core::spectrum::Spectrum> = match &denoised {
        Some(image) => image.data.chunks_exact(3).map(|c| tequila_core::spectrum::Spectrum::from_rendering_rgb(tequila_core::math::vec3(c[0], c[1], c[2]))).collect(),
        None => (0..width * height).map(|i| thumbnail.pixel(i % width, i / width)).collect(),
    };
    let path = match &request.path {
        Some(path) => PathBuf::from(path),
        None => {
            static COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            std::env::temp_dir().join(format!(
                "tequila-material-{}-{}.png",
                std::process::id(),
                COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ))
        }
    };
    let output = tequila_format::output::RenderOutput {
        width,
        height,
        color,
        alpha: vec![1.0; width * height],
        layers: Vec::new(),
        style: tequila_color::ImageStyle::default(),
        display: tequila_color::DisplayPipeline::default(),
        alpha_mode: tequila_format::output::AlphaMode::None,
    };
    output.write(&path, tequila_format::output::ImageFormat::Png(tequila_format::psd::Depth::Eight)).map_err(|e| e.to_string())?;
    let mut out = Object::new();
    out.set("path", path.display().to_string())
        .set("name", name)
        .set("width", width as f64)
        .set("height", height as f64)
        .set("ball_radius_m", settings.radius as f64);
    if !pictures.is_empty() && thumbnail.mean_luminance() <= 0.0 {
        out.set("note", "the preview came out black");
    }
    Ok(out.build())
}

/// The whole guide: the hand-written half, with the patterns' defaults and
/// the material types as this build has them.
pub fn guide() -> Value {
    use tequila_materials::material_type::MaterialType;
    let mut guide = json::parse(GUIDE.as_bytes()).expect("the material guide is valid JSON");
    let defaults: Vec<Value> = tequila_texture::procedural::Procedural::catalog().iter().map(tequila_format::procedural_to_json).collect();
    if let Value::Object(fields) = &mut guide {
        if let Some(Value::Array(patterns)) = fields.get_mut("patterns") {
            for pattern in patterns.iter_mut() {
                let kind = pattern.get("kind").and_then(Value::as_str).unwrap_or("").to_string();
                if let (Value::Object(entry), Some(default)) = (pattern, defaults.iter().find(|d| d.get("kind").and_then(Value::as_str) == Some(&kind))) {
                    entry.insert("defaults".into(), default.clone());
                }
            }
        }
        let types = MaterialType::ALL
            .iter()
            // Materials a part can wear: not the lights, the cutaway or the
            // ground, which are types for other things.
            .filter(|kind| !kind.is_light() && !matches!(kind, MaterialType::Cutaway | MaterialType::Ground))
            .map(|kind| {
                let mut entry = Object::new();
                entry.set("id", kind.id()).set("label", kind.label());
                entry.set("parameters", Value::Array(kind.exposed().iter().map(|p| Value::String(p.key().into())).collect()));
                entry.build()
            })
            .collect();
        fields.insert("types".into(), Value::Array(types));
    }
    guide
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every example in the guide reads and compiles, and every pattern the
    /// catalogue has is described there, and nothing else.
    #[test]
    fn the_guides_examples_compile_and_its_patterns_are_the_catalogue() {
        let guide = guide();
        let examples = guide.get("examples").map(Value::as_array).unwrap_or(&[]);
        assert!(examples.len() >= 10, "{} examples", examples.len());
        for example in examples {
            let recipe = example.get("recipe").expect("an example has a recipe");
            let entry = entry_of(recipe).unwrap_or_else(|e| panic!("{e}"));
            entry.graph.compile().unwrap_or_else(|e| panic!("{}: {e:?}", entry.name));
        }
        let described: Vec<&str> = guide.get("patterns").map(Value::as_array).unwrap_or(&[]).iter().filter_map(|p| p.get("kind").and_then(Value::as_str)).collect();
        let catalogue: Vec<&str> = tequila_texture::procedural::Procedural::catalog().iter().map(|p| p.key()).collect();
        for kind in &catalogue {
            assert!(described.contains(kind), "the guide does not describe `{kind}`");
        }
        for kind in &described {
            assert!(catalogue.contains(kind), "the guide describes `{kind}`, which is no pattern");
        }
        for pattern in guide.get("patterns").map(Value::as_array).unwrap_or(&[]) {
            assert!(pattern.get("defaults").is_some(), "{pattern:?} has no defaults");
        }
    }

    /// Every example on the ball, for a person to look at before it ships:
    /// `TEQUILA_GUIDE_PREVIEWS=<folder> cargo test -p tequila-scripting
    /// render_the_guides_examples -- --ignored`.
    #[test]
    #[ignore = "renders the guide's examples for a person to look at"]
    fn render_the_guides_examples() {
        let folder = PathBuf::from(std::env::var("TEQUILA_GUIDE_PREVIEWS").expect("TEQUILA_GUIDE_PREVIEWS names a folder"));
        std::fs::create_dir_all(&folder).unwrap();
        let document = SceneDocument::new();
        let only = std::env::var("TEQUILA_GUIDE_ONLY").ok();
        // Candidates from a file of the guide's shape, while tuning them.
        let source = match std::env::var("TEQUILA_GUIDE_RECIPES") {
            Ok(file) => json::parse(&std::fs::read(file).expect("the recipes file reads")).expect("the recipes file is JSON"),
            Err(_) => guide(),
        };
        for example in source.get("examples").map(Value::as_array).unwrap_or(&[]) {
            let recipe = example.get("recipe").unwrap().clone();
            let name = recipe.get("name").and_then(Value::as_str).unwrap().to_string();
            if only.as_deref().is_some_and(|o| !name.to_lowercase().contains(&o.to_lowercase())) {
                continue;
            }
            let request = MaterialRequest {
                action: "preview".into(),
                recipe: Some(recipe),
                size: Some(256),
                path: Some(folder.join(format!("{}.png", name.replace(' ', "_"))).display().to_string()),
                ..Default::default()
            };
            material_request(&request, &document, &[], &folder).unwrap_or_else(|e| panic!("{name}: {e}"));
            println!("rendered {name}");
        }
    }

    fn colour(hex: &str) -> tequila_core::spectrum::Spectrum {
        tequila_format::hex_colour(hex).unwrap()
    }

    /// A colour's hue in degrees, and whether it has one to speak of: red
    /// near 0, green near 120, blue near 240.
    fn hue(c: tequila_core::spectrum::Spectrum) -> (f32, bool) {
        let v = c.to_linear_srgb();
        let (max, min) = (v.x.max(v.y).max(v.z), v.x.min(v.y).min(v.z));
        let spread = max - min;
        if spread <= max * 0.08 {
            return (0.0, false);
        }
        let hue = if max == v.x {
            ((v.y - v.z) / spread).rem_euclid(6.0)
        } else if max == v.y {
            (v.z - v.x) / spread + 2.0
        } else {
            (v.x - v.y) / spread + 4.0
        };
        (hue * 60.0, true)
    }

    /// The colour a material shows through its pattern: its albedo on the
    /// library's ball, averaged over the middle of the picture.
    fn seen(graph: &MaterialGraph) -> tequila_core::spectrum::Spectrum {
        use tequila_render_cpu::{render_thumbnail_textured, Device, PreviewSettings, PreviewShape};
        let settings = PreviewSettings { size: 48, samples: 4, shape: PreviewShape::Sphere, radius: tequila_render_cpu::preview::PREVIEW_RADIUS, uv_unit: None };
        let table = Arc::new(tequila_render_core::shade::TextureTable::new());
        let picture = render_thumbnail_textured(&Device::shared().unwrap(), graph, table, preview_environment(), settings).unwrap();
        let (mut sum, mut count) = ([0.0f32; 3], 0.0);
        for y in picture.height * 3 / 8..picture.height * 5 / 8 {
            for x in picture.width * 3 / 8..picture.width * 5 / 8 {
                let at = (y * picture.width + x) * 3;
                for (channel, total) in sum.iter_mut().enumerate() {
                    *total += picture.albedo[at + channel];
                }
                count += 1.0;
            }
        }
        tequila_core::spectrum::spectrum(sum[0] / count, sum[1] / count, sum[2] / count)
    }

    /// A shipped preset put on the demo's part, as `library apply` does it.
    fn with_preset(name: &str, color: Option<&str>) -> (SceneDocument, tequila_scene::CommandBus, MaterialKey, Value) {
        let mut document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.2);
        let mut bus = tequila_scene::CommandBus::new();
        let part = document.tree.iter().find(|n| n.mesh().is_some()).map(|n| n.id).expect("a part");
        let apply = LibraryRequest { action: "apply".into(), name: Some(name.into()), nodes: vec![part.0], color: color.map(colour), ..Default::default() };
        let user = std::env::temp_dir().join(format!("tequila-tint-{}", std::process::id()));
        let outcome = crate::document::library_request(&apply, &document, &[], &user, &|_, _, _, _, _| Err("not a pack".into())).unwrap_or_else(|e| panic!("{name}: {e}"));
        let LibraryOutcome::Commands(commands, answer) = outcome else { panic!("apply is commands") };
        for command in commands {
            bus.apply(&mut document, command).unwrap();
        }
        let key = document.tree.get(part).unwrap().material.expect("the part has the preset");
        assert_eq!(answer.get("material").and_then(Value::as_f64), Some(key.0 as f64), "apply says which material the parts now have");
        (document, bus, key, answer)
    }

    /// `material tint`, carried out on the bus as the application does.
    fn tinted(document: &mut SceneDocument, bus: &mut tequila_scene::CommandBus, key: MaterialKey, hex: &str) -> Value {
        let request = MaterialRequest { action: "tint".into(), material: Some(key.0), color: Some(colour(hex)), ..Default::default() };
        let LibraryOutcome::Commands(commands, answer) = material_request(&request, document, &[], Path::new("unused")).unwrap_or_else(|e| panic!("{e}")) else { panic!("tint is commands") };
        assert_eq!(commands.len(), 1, "one command, so one step to undo");
        for command in commands {
            bus.apply(document, command).unwrap();
        }
        answer
    }

    fn changed(answer: &Value) -> Vec<(String, usize, String)> {
        answer
            .get("changed")
            .map(Value::as_array)
            .unwrap_or(&[])
            .iter()
            .map(|c| {
                (
                    c.get("parameter").and_then(Value::as_str).unwrap().to_string(),
                    c.get("colors").and_then(Value::as_index).unwrap(),
                    c.get("in").and_then(Value::as_str).unwrap().to_string(),
                )
            })
            .collect()
    }

    /// A plain preset: its colour is `base_color` and nothing else, and
    /// that is what changes. One undo puts the red back.
    #[test]
    fn a_plain_preset_is_tinted_in_its_value() {
        let (mut document, mut bus, key, _) = with_preset("ABS Red Gloss", None);
        let before = document.materials.get(key).unwrap().graph.clone();
        assert!(!before.is_connected(tequila_materials::material_type::Param::BaseColor), "ABS Red Gloss is no longer a plain preset: pick another");
        let answer = tinted(&mut document, &mut bus, key, "#1f4d2b");
        let after = document.materials.get(key).unwrap();
        let green = colour("#1f4d2b");
        let base = after.material().base_color;
        assert!((base.r - green.r).abs() < 1e-5 && (base.g - green.g).abs() < 1e-5 && (base.b - green.b).abs() < 1e-5, "{base:?}");
        assert_eq!(changed(&answer), vec![("base_color".to_string(), 1, "value".to_string())]);
        assert_eq!(answer.get("color").and_then(Value::as_str), Some("#1f4d2b"));
        assert!(matches!(hue(colour(answer.get("was").and_then(Value::as_str).unwrap())), (h, true) if !(20.0..340.0).contains(&h)), "it was red: {answer:?}");
        assert_eq!(answer.get("name").and_then(Value::as_str), Some("ABS Red Gloss"));
        // Everything but the colour is as it was.
        assert_eq!(after.material().specular_roughness, before.material().specular_roughness);
        assert_eq!(after.graph.topology_hash(), before.topology_hash());

        bus.undo(&mut document).unwrap();
        assert_eq!(document.materials.get(key).unwrap().graph.content_hash(), before.content_hash(), "one undo takes the tint back");
    }

    /// A leather: its tan is inside a node that multiplies a grain by it,
    /// which `base_color` cannot reach. Tinted, the ball itself is green,
    /// grain and all.
    #[test]
    fn a_patterned_leather_is_tinted_in_its_pattern() {
        let (mut document, mut bus, key, _) = with_preset("Leather Tan", None);
        let before = document.materials.get(key).unwrap().graph.clone();
        assert!(before.is_connected(tequila_materials::material_type::Param::BaseColor), "Leather Tan no longer keeps its colour in a pattern: pick another");
        let tan = seen(&before);
        assert!(matches!(hue(tan), (h, true) if h > 15.0 && h < 50.0), "the leather starts tan: {tan:?}");

        let answer = tinted(&mut document, &mut bus, key, "#1f4d2b");
        let after = document.materials.get(key).unwrap().graph.clone();
        assert_eq!(changed(&answer), vec![("base_color".to_string(), 1, "pattern".to_string())]);
        let green = seen(&after);
        assert!(matches!(hue(green), (h, true) if h > 115.0 && h < 165.0), "the leather is not green: {green:?}");
        assert!(green.luminance() < tan.luminance() * 0.6, "a dark green is darker than the tan: {green:?} against {tan:?}");
        // The colour itself sits where the tan sat, in the node the grain
        // is multiplied by.
        let driver = after.driver_of(tequila_materials::material_type::Param::BaseColor).unwrap();
        let held = |graph: &MaterialGraph| graph.node(driver).unwrap().literals.values().find_map(|v| if let tequila_materials::graph::Value::Color(c) = v { Some(*c) } else { None }).unwrap();
        assert_eq!(held(&before), before.material().base_color);
        assert_eq!(held(&after), colour("#1f4d2b"));
        // The same leather: its grain, bump and sheen untouched.
        assert_eq!(after.topology_hash(), before.topology_hash());
        assert_eq!(after.material().specular_roughness, before.material().specular_roughness);
        after.compile().expect("the tinted leather compiles");

        bus.undo(&mut document).unwrap();
        assert_eq!(document.materials.get(key).unwrap().graph.content_hash(), before.content_hash());
    }

    /// A cloth woven of three blues: all three turn, the pale thread still
    /// the palest and the dark still the darkest, so the weave still reads.
    #[test]
    fn a_cloth_keeps_its_weave_in_the_new_colour() {
        use tequila_materials::graph::{NodeKind, Value as Literal};
        let colours = |graph: &MaterialGraph| -> Vec<tequila_core::spectrum::Spectrum> {
            let mut all = Vec::new();
            for (_, node) in graph.nodes() {
                if matches!(node.kind, NodeKind::Surface(_)) {
                    continue;
                }
                all.extend(node.literals.values().filter_map(|v| if let Literal::Color(c) = v { Some(*c) } else { None }));
                if let NodeKind::Gradient { gradient } = &node.kind {
                    all.extend(gradient.stops().iter().map(|s| s.color));
                }
            }
            all
        };
        let (mut document, mut bus, key, _) = with_preset("Denim Indigo", None);
        let before = colours(&document.materials.get(key).unwrap().graph);
        assert!(before.len() >= 3, "Denim Indigo no longer mixes several colours: pick another cloth");
        let answer = tinted(&mut document, &mut bus, key, "#8c2f2a");
        let after = colours(&document.materials.get(key).unwrap().graph);
        // Its sheen is a paler blue in a parameter of its own, and follows.
        assert_eq!(changed(&answer), vec![("base_color".to_string(), before.len(), "pattern".to_string()), ("fuzz_color".to_string(), 1, "value".to_string())]);
        assert_eq!(after.len(), before.len());
        for (was, now) in before.iter().zip(&after) {
            assert!(matches!(hue(*was), (h, true) if h > 200.0 && h < 260.0), "a thread that was not blue: {was:?}");
            assert!(matches!(hue(*now), (h, true) if !(25.0..335.0).contains(&h)), "a thread that is not red: {now:?}");
        }
        // The order from dark to pale is the weave: kept.
        let order = |all: &[tequila_core::spectrum::Spectrum]| {
            let mut order: Vec<usize> = (0..all.len()).collect();
            order.sort_by(|a, b| all[*a].luminance().total_cmp(&all[*b].luminance()));
            order
        };
        assert_eq!(order(&before), order(&after));
        // And its contrast, palest against darkest. What is kept is how
        // light each looks, which the tint's own tests measure; by the
        // light it reflects a blue is darker than a red that looks as
        // light, so here it is only held to stay a weave of the same depth.
        let contrast = |all: &[tequila_core::spectrum::Spectrum]| {
            let (low, high) = all.iter().fold((f32::MAX, 0.0f32), |(low, high), c| (low.min(c.luminance()), high.max(c.luminance())));
            high / low
        };
        assert!(contrast(&after) > 3.0 && contrast(&after) / contrast(&before) > 0.6 && contrast(&after) / contrast(&before) < 1.6, "{} became {}", contrast(&before), contrast(&after));
        let red = seen(&document.materials.get(key).unwrap().graph);
        assert!(matches!(hue(red), (h, true) if !(25.0..335.0).contains(&h)), "the denim is not red: {red:?}");
    }

    /// `library apply` with a colour: the preset arrives tinted, as a
    /// material named for the colour, and asking again reuses it.
    #[test]
    fn a_preset_is_applied_already_tinted() {
        let (document, _, key, answer) = with_preset("Leather Tan", Some("#1f4d2b"));
        let material = document.materials.get(key).unwrap();
        assert_eq!(material.name, "Leather Tan #1f4d2b");
        assert_eq!(answer.get("applied").and_then(Value::as_str), Some("Leather Tan"));
        assert_eq!(answer.get("material_name").and_then(Value::as_str), Some("Leather Tan #1f4d2b"));
        assert_eq!(changed(&answer), vec![("base_color".to_string(), 1, "pattern".to_string())]);
        let green = seen(&material.graph);
        assert!(matches!(hue(green), (h, true) if h > 115.0 && h < 165.0), "{green:?}");

        let part = document.tree.iter().find(|n| n.mesh().is_some()).map(|n| n.id).unwrap();
        let again = LibraryRequest { action: "apply".into(), name: Some("Leather Tan".into()), nodes: vec![part.0], color: Some(colour("#1f4d2b")), ..Default::default() };
        let LibraryOutcome::Commands(commands, _) = crate::document::library_request(&again, &document, &[], Path::new("unused"), &|_, _, _, _, _| Err("not a pack".into())).unwrap() else { panic!() };
        assert!(matches!(commands.as_slice(), [Command::AssignMaterial { material, .. }] if *material == key), "the same tint twice is one material");
        // The library's own tan is another material, by another name.
        let plain = LibraryRequest { color: None, ..again };
        let LibraryOutcome::Commands(commands, _) = crate::document::library_request(&plain, &document, &[], Path::new("unused"), &|_, _, _, _, _| Err("not a pack".into())).unwrap() else { panic!() };
        assert_eq!(commands.len(), 2);
    }

    /// A material with no colour of its own is refused in words that name
    /// it and say why, and nothing is changed.
    #[test]
    fn a_material_with_no_colour_is_refused_plainly() {
        let mut document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.2);
        let mut bus = tequila_scene::CommandBus::new();
        let film = json::parse(br#"{"name": "Soap Film", "type": "thin_film"}"#).unwrap();
        let create = MaterialRequest { action: "create".into(), recipe: Some(film), ..Default::default() };
        let LibraryOutcome::Commands(commands, answer) = material_request(&create, &document, &[], Path::new("unused")).unwrap() else { panic!() };
        for command in commands {
            bus.apply(&mut document, command).unwrap();
        }
        let key = answer.get("material").and_then(Value::as_index).unwrap() as u64;
        let tint = MaterialRequest { action: "tint".into(), material: Some(key), color: Some(colour("#1f4d2b")), ..Default::default() };
        let refused = material_request(&tint, &document, &[], Path::new("unused")).err().expect("a thin film has no colour");
        assert!(refused.starts_with("`Soap Film` has no colour to change"), "{refused}");

        let nobody = MaterialRequest { material: Some(9999), ..tint.clone() };
        assert!(material_request(&nobody, &document, &[], Path::new("unused")).err().unwrap().contains("there is no material 9999"));
        let no_colour = MaterialRequest { color: None, ..tint };
        assert!(material_request(&no_colour, &document, &[], Path::new("unused")).err().unwrap().contains("needs `color`"));
    }

    /// Before and after, on the ball, for a person to look at:
    /// `TEQUILA_TINT_SWATCHES=<folder> cargo test -p tequila-scripting
    /// render_tint_swatches -- --ignored`. Each line of `swatches.txt` in
    /// the folder is `preset name | #colour | ball radius in metres`.
    #[test]
    #[ignore = "renders tinted presets for a person to look at"]
    fn render_tint_swatches() {
        let folder = PathBuf::from(std::env::var("TEQUILA_TINT_SWATCHES").expect("TEQUILA_TINT_SWATCHES names a folder"));
        let list = std::fs::read_to_string(folder.join("swatches.txt")).expect("swatches.txt lists what to render");
        let (entries, _) = library::read_libraries(&library::standard_directory(), &folder.join("no-library"));
        let document = SceneDocument::new();
        for (index, line) in list.lines().filter(|l| !l.trim().is_empty()).enumerate() {
            let fields: Vec<&str> = line.split('|').map(str::trim).collect();
            let entry = entries.iter().find(|e| e.name.eq_ignore_ascii_case(fields[0])).unwrap_or_else(|| panic!("no preset `{}`", fields[0]));
            let radius = fields.get(2).and_then(|r| r.parse::<f32>().ok());
            let render = |entry: &LibraryEntry, file: String| {
                let request = MaterialRequest {
                    action: "preview".into(),
                    recipe: Some(json::parse(library::write_entry(entry).as_bytes()).unwrap()),
                    size: Some(360),
                    radius,
                    path: Some(folder.join(file).display().to_string()),
                    ..Default::default()
                };
                material_request(&request, &document, &[], &folder).unwrap_or_else(|e| panic!("{}: {e}", entry.name));
            };
            render(entry, format!("{index:02}-before.png"));
            match entry.tinted(colour(fields[1])) {
                Ok((tinted, done)) => {
                    render(&tinted, format!("{index:02}-after.png"));
                    println!("{index:02} | {} | {} | {:?}", entry.name, fields[1], done.changed);
                }
                Err(why) => println!("{index:02} | {} | refused: it {why}", entry.name),
            }
        }
    }

    /// Made, put on a part, updated, read back and previewed: the round a
    /// model makes with a material of its own.
    #[test]
    fn a_recipe_is_made_updated_read_back_and_previewed() {
        let mut document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.2);
        let mut bus = tequila_scene::CommandBus::new();
        let part = document.tree.iter().find(|n| n.mesh().is_some()).map(|n| n.id).expect("a part");
        let recipe = json::parse(br#"{"name": "Speckle", "type": "plastic", "specular_roughness": 0.4,
            "patterns": [{"param": "base_color", "procedural": {"kind": "spots", "coverage": 0.4}, "size": 0.004,
                          "colors": [{"at": 0, "color": [0.8, 0.8, 0.78]}, {"at": 1, "color": [0.05, 0.05, 0.05]}]}]}"#).unwrap();
        let request = |action: &str, recipe: Option<Value>, material: Option<u64>| MaterialRequest {
            action: action.into(),
            recipe,
            material,
            nodes: vec![part.0],
            ..Default::default()
        };
        let user = std::env::temp_dir().join(format!("tequila-material-op-{}", std::process::id()));

        let LibraryOutcome::Commands(commands, answer) = material_request(&request("create", Some(recipe.clone()), None), &document, &[], &user).unwrap() else { panic!("create is commands") };
        for command in commands {
            bus.apply(&mut document, command).unwrap();
        }
        let key = MaterialKey(answer.get("material").and_then(Value::as_f64).unwrap() as u64);
        assert_eq!(document.tree.get(part).unwrap().material, Some(key));
        assert_eq!(document.materials.get(key).unwrap().name, "Speckle");

        // Again: a second material, told apart by its name.
        let LibraryOutcome::Commands(again, _) = material_request(&request("create", Some(recipe), None), &document, &[], &user).unwrap() else { panic!() };
        for command in again {
            bus.apply(&mut document, command).unwrap();
        }
        assert!(document.materials.iter().any(|(_, d)| d.name == "Speckle 2"));

        let metal = json::parse(br#"{"type": "metal", "metal_preset": "Copper", "patterns": [{"param": "specular_roughness", "procedural": {"kind": "noise"}, "size": 0.01, "range": [0.2, 0.4]}]}"#).unwrap();
        let LibraryOutcome::Commands(changes, _) = material_request(&request("update", Some(metal), Some(key.0)), &document, &[], &user).unwrap() else { panic!() };
        for command in changes {
            bus.apply(&mut document, command).unwrap();
        }
        let updated = document.materials.get(key).unwrap();
        assert_eq!(updated.material_type, tequila_materials::material_type::MaterialType::Metal);
        assert_eq!(updated.name, "Speckle", "a recipe without a name keeps the material's");
        assert!(updated.graph.is_connected(tequila_materials::material_type::Param::SpecularRoughness));

        let LibraryOutcome::Done(read) = material_request(&request("recipe", None, Some(key.0)), &document, &[], &user).unwrap() else { panic!() };
        let back = entry_of(read.get("recipe").unwrap()).expect("a material read back is a recipe that reads");
        assert_eq!(back.graph.content_hash(), updated.graph.content_hash());

        let path = std::env::temp_dir().join(format!("tequila-material-preview-{}.png", std::process::id()));
        let mut preview = request("preview", None, Some(key.0));
        preview.path = Some(path.display().to_string());
        preview.size = Some(96);
        let LibraryOutcome::Done(shown) = material_request(&preview, &document, &[], &user).unwrap() else { panic!() };
        assert_eq!(shown.get("path").and_then(Value::as_str), Some(path.display().to_string().as_str()));
        assert!(std::fs::metadata(&path).map(|m| m.len() > 1000).unwrap_or(false), "no picture written");
        let _ = std::fs::remove_file(&path);

        let bad = json::parse(br#"{"type": "plastic", "patterns": [{"param": "base_colour", "procedural": {"kind": "noise"}}]}"#).unwrap();
        let error = material_request(&request("create", Some(bad), None), &document, &[], &user).err().expect("a misspelt parameter is refused");
        assert!(error.contains("base_colour"), "{error}");
    }
}
