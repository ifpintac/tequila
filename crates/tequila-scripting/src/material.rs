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
//! the scene; `save` keeps one in the person's library.

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
pub const MATERIAL_ACTIONS: &[&str] = &["guide", "create", "update", "recipe", "preview", "save"];

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
    output.write(&path, tequila_format::output::ImageFormat::Png).map_err(|e| e.to_string())?;
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
