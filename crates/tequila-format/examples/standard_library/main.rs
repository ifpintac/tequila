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
mod fabric;
mod fdm;
mod finish;
mod granite;
mod marble;
mod metal;
mod metal_finish;
mod metal_pattern;
mod natural;
mod paint;
mod plastic;
mod plastic_texture;
mod recipe;
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
}

pub fn preset(name: &'static str, category: &'static str, tags: &'static [&'static str], ty: MaterialType, recipe: Recipe) -> Preset {
    Preset { name, category, tags, ty, recipe }
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

fn main() {
    let presets: Vec<Preset> = [plastic::all(), plastic_texture::all(), metal::all(), metal_finish::all(), metal_pattern::all(), paint::all(), natural::all(), marble::all(), granite::all(), concrete::all(), ceramic::all(), composite::all(), wood::all(), fabric::all(), clear::all(), fdm::all()].into_iter().flatten().collect();
    let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/library/standard");
    let mut names = std::collections::BTreeSet::new();
    let mut written = Vec::new();
    for p in presets {
        assert!(names.insert(p.name), "{} twice", p.name);
        p.recipe.graph.compile().unwrap_or_else(|e| panic!("{} does not compile: {e:?}", p.name));
        let material = p.recipe.graph.material().clone();
        assert_eq!(material.clone().sanitized(), material, "{} holds a value out of range", p.name);
        let entry = LibraryEntry {
            name: p.name.to_string(),
            category: p.category.to_string(),
            tags: p.tags.iter().map(|t| t.to_string()).collect(),
            material_type: p.ty,
            material,
            graph: p.recipe.graph,
            textures: Vec::new(),
            source: Source::Standard,
            path: String::new(),
        };
        let file = format!("{}.json", library::file_stem(p.name));
        std::fs::write(folder.join(&file), library::write_entry(&entry)).expect("writable");
        written.push(file);
    }
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
