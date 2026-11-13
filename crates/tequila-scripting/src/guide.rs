//! The guide: how to do a job in Tequila, a page at a time.
//!
//! An assistant that is handed everything reads none of it: an answer of
//! tens of thousands of characters is skipped, and the next thing it does
//! is guess. So what there is to know about working in Tequila is written
//! in layers. The core ([`core`]) is short enough to be given to every
//! assistant before it starts: how to look, change, check a picture and
//! undo, and which page to open for which job. Each page ([`PAGES`]) is one
//! area of the application, taught in the order a person does the work,
//! and short enough to be read whole.
//!
//! The pages are the Tequila skill's own files (`skills/tequila`), built
//! into the binary, so the Claude app reading the skill's folder, an AI app
//! asking the bridge, and Tequila's own Assistant all read the same words:
//! `tests/skill_examples.rs` reads every example in them with the parsers
//! Tequila itself uses, and holds each to its size.

use tequila_format::json::Value;
use tequila_format::json_write::Object;

/// One page of the guide.
pub struct Page {
    /// What it is asked for by, and its file's name in the skill.
    pub topic: &'static str,
    /// What it covers, in a line, for the list of pages.
    pub about: &'static str,
    pub text: &'static str,
}

/// The topic the core itself is asked for by.
pub const START: &str = "start";

/// The longest the core may be, in characters: it is given to every
/// assistant with every conversation.
pub const CORE_LIMIT: usize = 8_000;

/// The longest a page may be, in characters: longer, and it is not read.
pub const PAGE_LIMIT: usize = 12_000;

const SKILL: &str = include_str!("../../../skills/tequila/SKILL.md");

macro_rules! page {
    ($topic:literal, $about:literal) => {
        Page { topic: $topic, about: $about, text: include_str!(concat!("../../../skills/tequila/reference/", $topic, ".md")) }
    };
}

/// Every page, in the order the core lists them.
pub const PAGES: &[Page] = &[
    page!("materials", "Materials on parts: the library, changing a preset, types, parameters, colours, shared materials"),
    page!("recipes", "A material of your own from a recipe: patterns, layers and masks, when the library has nothing close"),
    page!("graph", "The material graph: reading it, driving one parameter with a pattern, a ramp or a picture, changing nodes"),
    page!("textures", "Picture maps on a material, texture packs, how a picture is laid on a surface, Substance files"),
    page!("labels", "Labels and decals: a logo or print put on a material, placed, sized and finished"),
    page!("uv", "UV layouts: unwrapping a part, cutting, packing, straightening, UV sets, the layout as a picture"),
    page!("scene", "Parts and groups: naming, hiding, moving, shapes, patterns of copies, units, cutaways, measurements"),
    page!("lighting", "Light: lighting presets, the environment, an HDRI, a sky, light pins, lights, the studio kit, the floor"),
    page!("camera", "The view and saved cameras: standard views, lens, depth of field, composition guides"),
    page!("variants", "Colourways, model sets, Snapshots (studios) and the configurator; studio presets; material templates"),
    page!("rendering", "Rendering: a quick look, a final render, the queue, size, samples, format, transparency, layers, passes"),
    page!("image", "The look after the render: exposure, contrast, white balance, tone curve, bloom, vignette, colour management"),
    page!("animation", "A turntable and a 360-degree spin viewer"),
    page!("fixing", "Fixing a model: wrong size, one lump, faceted curves, lying on its side, missing parts, no UVs"),
    page!("files", "Files: importing a model, updating it from its file, saving, exporting to glTF, USD, AR, the web"),
    page!("plugins", "Adding to Tequila itself: a plugin with a panel, a button, a batch job or an automation"),
    page!("app", "The window and Tequila's own settings: panels, viewport modes, the UV and graph editors, preferences"),
    page!("scripting", "Raw requests and the Python client, for what no tool covers and for work with no window"),
    page!("shots", "Whole shots, step by step: clean studio, premium black, white e-commerce, cut-out, jewellery"),
];

/// The core: the skill's own first page, without the front matter a skill
/// folder starts with.
pub fn core() -> &'static str {
    let Some(rest) = SKILL.strip_prefix("---") else { return SKILL };
    match rest.split_once("\n---") {
        Some((_, body)) => body.trim_start(),
        None => SKILL,
    }
}

/// Other names a page is asked for by. An assistant asks for `nodes`,
/// `render` or `decals` as readily as for the page's own name, and a
/// refusal costs it a step it may not take.
const ALSO: &[(&str, &str)] = &[
    ("material", "materials"), ("library", "materials"), ("colour", "materials"), ("color", "materials"),
    ("recipe", "recipes"), ("procedural", "recipes"), ("patterns", "recipes"),
    ("nodes", "graph"), ("node", "graph"), ("material_graph", "graph"), ("node_graph", "graph"), ("shader", "graph"),
    ("texture", "textures"), ("maps", "textures"), ("substance", "textures"), ("texture_pack", "textures"),
    ("label", "labels"), ("decal", "labels"), ("decals", "labels"), ("logo", "labels"),
    ("uvs", "uv"), ("unwrap", "uv"), ("uv_editing", "uv"), ("uv_layout", "uv"),
    ("parts", "scene"), ("scene_setup", "scene"), ("transform", "scene"), ("transforms", "scene"), ("cutaway", "scene"), ("measurements", "scene"), ("units", "scene"),
    ("light", "lighting"), ("lights", "lighting"), ("environment", "lighting"), ("floor", "lighting"), ("ground", "lighting"), ("hdri", "lighting"),
    ("cameras", "camera"), ("composition", "camera"), ("view", "camera"), ("depth_of_field", "camera"),
    ("colourways", "variants"), ("colorways", "variants"), ("configurator", "variants"), ("configurators", "variants"), ("studios", "variants"), ("studio", "variants"), ("snapshots", "variants"),
    ("render", "rendering"), ("renders", "rendering"), ("queue", "rendering"), ("passes", "rendering"), ("output", "rendering"),
    ("look", "image"), ("image_style", "image"), ("image_look", "image"), ("post", "image"), ("tone", "image"),
    ("turntable", "animation"), ("turntables", "animation"), ("spin", "animation"),
    ("fix", "fixing"), ("repair", "fixing"), ("fixing_a_model", "fixing"), ("normals", "fixing"),
    ("file", "files"), ("import", "files"), ("export", "files"), ("import_export", "files"), ("save", "files"),
    ("plugin", "plugins"),
    ("preferences", "app"), ("settings", "app"), ("ui", "app"), ("window", "app"), ("interface", "app"),
    ("python", "scripting"), ("script", "scripting"), ("scripts", "scripting"), ("requests", "scripting"),
    ("shot", "shots"),
    ("core", START), ("index", START), ("help", START), ("guide", START), ("pages", START),
];

/// The page a topic names: its own name, however it is written - `UV`,
/// `reference/uv.md`, `uv editing` - or one of the other names it goes by.
pub fn find(topic: &str) -> Option<&'static str> {
    let asked = topic.trim().to_lowercase();
    let asked = asked.rsplit(['/', '\\']).next().unwrap_or_default();
    let asked = asked.strip_suffix(".md").unwrap_or(asked).replace([' ', '-'], "_");
    if asked == START || asked == "skill" {
        return Some(START);
    }
    if let Some(page) = PAGES.iter().find(|page| page.topic == asked) {
        return Some(page.topic);
    }
    ALSO.iter().find(|(other, _)| *other == asked).map(|(_, topic)| *topic)
}

/// The list of pages: what `guide` answers with when no topic is given.
pub fn list() -> Value {
    let entry = |topic: &str, about: &str| {
        let mut out = Object::new();
        out.set("topic", topic).set("about", about);
        out.build()
    };
    let mut pages = vec![entry(START, "How to work in Tequila: look, change, check the picture, undo. Read it first, if you have not been given it")];
    pages.extend(PAGES.iter().map(|page| entry(page.topic, page.about)));
    let mut out = Object::new();
    out.set("pages", Value::Array(pages));
    out.set("how", "Ask again with `topic` for one page. Read the page for an area before you work in it for the first time.");
    out.build()
}

/// One page as text, or the list of pages when no topic is given.
pub fn page(topic: Option<&str>) -> Result<Value, String> {
    let Some(asked) = topic.map(str::trim).filter(|topic| !topic.is_empty()) else {
        return Ok(list());
    };
    match find(asked) {
        Some(START) => Ok(Value::String(core().to_string())),
        Some(topic) => Ok(Value::String(PAGES.iter().find(|page| page.topic == topic).expect("found by its name").text.to_string())),
        None => Err(format!(
            "there is no page `{asked}`; the pages are {START}, {}",
            PAGES.iter().map(|page| page.topic).collect::<Vec<_>>().join(", ")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_is_found_by_its_name_however_written() {
        assert_eq!(find("uv"), Some("uv"));
        assert_eq!(find(" UV "), Some("uv"));
        assert_eq!(find("reference/uv.md"), Some("uv"));
        assert_eq!(find("UV editing"), Some("uv"));
        assert_eq!(find("nodes"), Some("graph"));
        assert_eq!(find("render"), Some("rendering"));
        assert_eq!(find("decals"), Some("labels"));
        assert_eq!(find("SKILL.md"), Some(START));
        assert_eq!(find("teapots"), None);
        // No other name hides a page, and each leads to one.
        for (other, topic) in ALSO {
            assert!(PAGES.iter().all(|page| page.topic != *other), "`{other}` is a page's own name");
            assert!(*topic == START || PAGES.iter().any(|page| page.topic == *topic), "`{other}` leads to `{topic}`, which is no page");
        }
    }

    #[test]
    fn a_topic_answers_with_its_page_and_none_with_the_list() {
        let Value::String(uv) = page(Some("uv")).unwrap() else { panic!("a page is text") };
        assert!(uv.starts_with("# "), "{}", &uv[..40]);
        let Value::String(start) = page(Some("start")).unwrap() else { panic!("the core is text") };
        assert!(!start.starts_with("---") && start.contains("tequila_guide"), "the core, without the skill's front matter");
        let list = page(None).unwrap();
        let listed: Vec<&str> = list.get("pages").unwrap().as_array().iter().filter_map(|p| p.get("topic").and_then(Value::as_str)).collect();
        assert_eq!(listed.len(), PAGES.len() + 1);
        assert_eq!(listed[0], START);
        assert_eq!(page(Some("  ")).unwrap(), list);
        let refused = page(Some("teapots")).unwrap_err();
        assert!(refused.contains("materials") && refused.contains("scripting"), "{refused}");
    }
}
