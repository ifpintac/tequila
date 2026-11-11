//! The Tequila Standard Library, written as code.
//!
//! Every preset in `assets/library/standard` is built here and written out
//! as JSON: `cargo run --release -p tequila-format --example
//! standard_library`. A preset is a material type's defaults, the physical
//! values that make it what it is (measured indices of refraction, the
//! absorption of a liquid, the scattering of a wax) and a node graph for
//! what makes it look real rather than computed: the orange peel under a
//! car's clear coat, the dimples of a texture-moulded part, the rings and
//! pores of wood, the way any handled surface is a little glossier in one
//! place than another. Lengths are real ones, in metres; patterns read the
//! part's own space in scene units, so a preset looks right on a part of
//! the size it is made for.
//!
//! The colours are original: picked by eye in sRGB, or kept from the
//! measured values the library already held. Nothing is taken from a
//! commercial swatch system (`assets/library/CREDITS.md`).

mod ceramic;
mod clear;
mod composite;
mod concrete;
mod equipment;
mod fabric;
mod fdm;
mod finish;
mod footwear;
mod fur;
mod granite;
mod leather;
mod marble;
mod metal;
mod metal_finish;
mod metal_pattern;
mod natural;
mod paint;
mod plastic;
mod plastic_texture;
mod product;
mod recipe;
mod suede;
mod wood;

use tequila_core::spectrum::Spectrum;
use tequila_format::library::{self, LibraryEntry, Source};
use tequila_materials::graph::{Node, NodeKind};
use tequila_materials::material_type::MaterialType;
use tequila_materials::openpbr::Material;
use tequila_texture::procedural::Procedural;
pub use recipe::{srgb, wear, Out, Recipe};

/// A colour in the rendering space, as the library has measured it.
pub fn rgb(r: f32, g: f32, b: f32) -> Spectrum {
    Spectrum { r, g, b }
}

pub struct Preset {
    name: &'static str,
    category: &'static str,
    tags: &'static [&'static str],
    ty: MaterialType,
    recipe: Recipe,
    /// For a preset read along a strip's UVs: how much of the strip one UV
    /// unit is, in metres, for its thumbnail (`LibraryEntry::preview_uv_unit`).
    preview_uv_unit: Option<f32>,
}

pub fn preset(name: &'static str, category: &'static str, tags: &'static [&'static str], ty: MaterialType, recipe: Recipe) -> Preset {
    Preset { name, category, tags, ty, recipe, preview_uv_unit: None }
}

impl Preset {
    /// Previewed as a strip `unit` metres to the UV unit.
    pub fn previewed_as_strip(self, unit: f32) -> Preset {
        Preset { preview_uv_unit: Some(unit), ..self }
    }
}

/// A type's defaults with some values changed.
pub fn base(ty: MaterialType, change: impl FnOnce(&mut Material)) -> Material {
    let mut m = ty.defaults();
    change(&mut m);
    m
}

/// A plain preset: physical values and no graph.
pub fn plain(name: &'static str, category: &'static str, tags: &'static [&'static str], ty: MaterialType, change: impl FnOnce(&mut Material)) -> Preset {
    preset(name, category, tags, ty, Recipe::new(base(ty, change)))
}

pub fn metal_ior(name: &str) -> tequila_materials::fresnel::ComplexIor {
    tequila_materials::fresnel::metals::ALL.iter().find(|(n, _)| *n == name).map(|(_, ior)| *ior).unwrap_or_else(|| panic!("no metal {name}"))
}

/// One of Tequila's own patterns as a node.
pub fn pattern(r: &mut Recipe, procedural: Procedural, seed: u32) -> Out {
    let id = r.graph.add_node(Node::new(NodeKind::Procedural { procedural: Box::new(procedural), seed }));
    Out(id, 0)
}

/// `a * weight + b`: two heights summed into one.
pub fn plus(r: &mut Recipe, a: Out, weight: f32, b: Out) -> Out {
    r.node("math").set("operation", "multiply_add").w("Value", a).f("Value 2", weight).w("Value 3", b).out("Value")
}

/// A noise's value, `scale` features a metre.
pub fn noise(r: &mut Recipe, scale: f32, detail: f32, roughness: f32) -> Out {
    r.node("noise_texture").f("Scale", scale).f("Detail", detail).f("Roughness", roughness).out("Fac")
}

/// The bumps of a moulded or blasted surface, `depth` metres high at `scale`.
pub fn micro(r: &mut Recipe, scale: f32, depth: f32) {
    let n = noise(r, scale, 2.0, 0.5);
    r.bump(n, depth);
}

/// Orange peel: the gentle waviness a sprayed or powder-coated finish
/// levels to, `scale` waves a metre and `depth` metres deep.
pub fn orange_peel(r: &mut Recipe, scale: f32, depth: f32) -> Out {
    let n = r.node("noise_texture").f("Scale", scale).f("Detail", 3.0).f("Roughness", 0.45).out("Fac");
    r.bump(n, depth);
    n
}

/// No made thing has a knife edge, and a rendering of one that has is
/// known for a rendering at once: a real part's edge is where it catches
/// a line of light. Where the recipe of a plastic, a metal, a paint or a wood says
/// nothing of its edges, they are rounded as its making leaves them, in
/// metres: a moulding's as its tool's, which is not cut sharper than a
/// third of a millimetre; a casting's or a forging's the same; a machined
/// or sheared metal's as deburring leaves it; a clear plastic's polished
/// arris; and a paint's, which pulls back from an edge and rounds it,
/// a powder's more than an enamel's.
fn edge_of(p: &Preset) -> f32 {
    use tequila_materials::material_type::Param;
    let name = p.name.to_lowercase();
    let says = |word: &str| name.contains(word) || p.tags.iter().any(|t| t.contains(word));
    // Nothing to round where the type has no such thing, or where the
    // recipe cuts holes: a hole's edge is drawn, not modelled.
    if !p.ty.exposed().contains(&Param::EdgeRadius) || p.recipe.graph.is_connected(Param::Opacity) {
        return 0.0;
    }
    match p.category {
        "Plastic" if p.ty == MaterialType::PlasticTransparent => 2e-4,
        "Plastic" if says("3d print") => 2e-4,
        "Plastic" if says("rubber") || says("foam") || says("silicone") || says("soft touch") => 4e-4,
        "Plastic" => 3.5e-4,
        "Paint" if says("powder") => 5e-4,
        "Paint" => 4e-4,
        "Metal" if says("cast") || says("hammered") || says("forged") || says("rust") || says("corten") || says("patina") || says("verdigris") => 4e-4,
        "Metal" => 1.5e-4,
        // A joiner breaks every edge with a pass of paper.
        "Wood" => 5e-4,
        _ => 0.0,
    }
}

/// What the library says of a preset beyond its recipe: the category it
/// is listed under, a sentence on what it is and what it is for, the other
/// words it is found by, and what it is used for (words from
/// `library::USES`). Kept in `catalogue.json` beside this file, one entry a
/// preset, so the words can be read and corrected together; it decides the
/// category, over the one the recipe was written with.
struct Listing {
    category: String,
    description: String,
    tags: Vec<String>,
    uses: Vec<String>,
}

fn catalogue() -> std::collections::BTreeMap<String, Listing> {
    let text = include_str!("catalogue.json");
    let value = tequila_format::json::parse(text.as_bytes()).expect("catalogue.json reads");
    let words = |v: &tequila_format::json::Value, key: &str| -> Vec<String> {
        v.get(key).map(|w| w.as_array()).unwrap_or(&[]).iter().filter_map(|w| w.as_str().map(str::to_string)).collect()
    };
    let mut listings = std::collections::BTreeMap::new();
    for item in value.as_array() {
        let name = item.get("name").and_then(|v| v.as_str()).expect("a listing names its preset").to_string();
        let listing = Listing {
            category: item.get("category").and_then(|v| v.as_str()).expect("a listing has a category").to_string(),
            description: item.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            tags: words(item, "tags"),
            uses: words(item, "uses"),
        };
        assert!(library::CATEGORIES.contains(&listing.category.as_str()), "{name}: `{}` is not one of the library's categories", listing.category);
        for u in &listing.uses {
            assert!(library::USES.contains(&u.as_str()), "{name}: `{u}` is not one of the uses");
        }
        assert!(listings.insert(name.clone(), listing).is_none(), "{name} is listed twice");
    }
    listings
}

/// The recipe's own words, then the listing's, then its uses, each once.
fn words_of(own: &[&str], listing: &Listing) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    for w in own.iter().map(|w| w.to_string()).chain(listing.tags.iter().cloned()).chain(listing.uses.iter().cloned()) {
        let w = w.trim().to_lowercase();
        if !w.is_empty() && !words.contains(&w) {
            words.push(w);
        }
    }
    words
}

fn main() {
    let mut listings = catalogue();
    let presets: Vec<Preset> = [plastic::all(), plastic_texture::all(), metal::all(), metal_finish::all(), metal_pattern::all(), paint::all(), natural::all(), marble::all(), granite::all(), concrete::all(), ceramic::all(), composite::all(), wood::all(), fabric::all(), suede::all(), fur::all(), footwear::all(), clear::all(), fdm::all(), product::all(), equipment::all(), leather::all()].into_iter().flatten().collect();
    let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/library/standard");
    let mut names = std::collections::BTreeSet::new();
    let mut written = Vec::new();
    for mut p in presets {
        assert!(names.insert(p.name), "{} twice", p.name);
        if p.recipe.graph.material().edge_radius == 0.0 {
            p.recipe.material_mut().edge_radius = edge_of(&p);
        }
        p.recipe.graph.compile().unwrap_or_else(|e| panic!("{} does not compile: {e:?}", p.name));
        let material = p.recipe.graph.material().clone();
        assert_eq!(material.clone().sanitized(), material, "{} holds a value out of range", p.name);
        let listing = listings.remove(p.name).unwrap_or_else(|| panic!("{} is not in catalogue.json", p.name));
        assert!(!listing.description.is_empty() && !listing.uses.is_empty(), "{}: its listing wants a description and a use", p.name);
        let entry = LibraryEntry {
            description: listing.description.clone(),
            name: p.name.to_string(),
            category: listing.category.clone(),
            tags: words_of(p.tags, &listing),
            material_type: p.ty,
            material,
            graph: p.recipe.graph,
            textures: Vec::new(),
            preview_uv_unit: p.preview_uv_unit,
            source: Source::Standard,
            path: String::new(),
        };
        let file = format!("{}.json", library::file_stem(p.name));
        std::fs::write(folder.join(&file), library::write_entry(&entry)).expect("writable");
        written.push(file);
    }
    assert!(listings.is_empty(), "catalogue.json lists presets that are not built: {:?}", listings.keys().collect::<Vec<_>>());
    // What is not built here any more goes.
    for old in std::fs::read_dir(&folder).expect("the library folder") {
        let path = old.expect("listable").path();
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if name.ends_with(".json") && !written.contains(&name) {
            std::fs::remove_file(&path).expect("removable");
            println!("removed {name}");
        }
    }
    println!("wrote {} presets", written.len());
}
