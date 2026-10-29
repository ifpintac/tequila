//! The `studio_preset` operation's answers that need only a document
//! (`docs/studio-presets.md`): the list with each family's suggested size,
//! one preset described, and a request's options read. The application and
//! the headless server answer from these, so a script reads the same from
//! either.

use tequila_format::json::Value;
use tequila_format::json_write::Object;
use tequila_format::studio_preset::{self as presets, PresetFile, PresetSource, SaveAs};
use tequila_scene::preset::{ApplyOptions, FramingScale, Front};

/// One preset, as a script sees it.
pub fn describe(file: &PresetFile, suggested: bool) -> Value {
    let p = &file.preset;
    let mut out = Object::new();
    out.set("id", p.id.clone())
        .set("name", p.name.clone())
        .set("family", p.family.clone())
        .set("size", p.size.clone())
        .set("description", p.description.clone())
        .set("source", if file.source == PresetSource::Builtin { "builtin" } else { "user" })
        .set("path", file.path.display().to_string())
        .set_some("thumbnail", file.thumbnail.as_ref().map(|t| t.display().to_string()))
        .set("views", Value::Array(p.studios.iter().map(|s| Value::String(s.name.clone())).collect()))
        .set("lights", p.lights.len() as f64)
        .set("materials", Value::Array(p.materials.iter().map(|m| Value::String(m.definition.name.clone())).collect()))
        .set("reference_radius_m", p.framing.reference_radius)
        .set("suggested", suggested);
    out.build()
}

/// The ids of the presets framed nearest a product of `radius`, one per
/// family.
pub fn suggestions(all: &[PresetFile], radius: Option<f32>) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    let Some(radius) = radius else { return out };
    let families: std::collections::BTreeSet<&str> = all.iter().map(|p| p.preset.family.as_str()).collect();
    for family in families {
        if let Some(best) = tequila_scene::preset::nearest_size(all.iter().filter(|p| p.preset.family == family).map(|p| &p.preset), radius) {
            out.insert(best.id.clone());
        }
    }
    out
}

/// Every preset installed, each family's size nearest a product of
/// `radius` marked.
pub fn list(radius: Option<f32>) -> Value {
    let (all, problems) = presets::list();
    let suggested = suggestions(&all, radius);
    let mut out = Object::new();
    out.set("presets", Value::Array(all.iter().map(|p| describe(p, suggested.contains(&p.preset.id))).collect()))
        .set("problems", Value::Array(problems.into_iter().map(Value::String).collect()))
        .set_some("product_radius_m", radius);
    out.build()
}

/// How an `apply` asks for the preset to fit.
pub fn options(request: &Value) -> Result<ApplyOptions, String> {
    let text = |key: &str| request.get(key).and_then(|v| v.as_str());
    let flag = |key: &str| request.get(key).and_then(|v| v.as_bool()).unwrap_or(false);
    let front = match text("front") {
        None => Front::PlusZ,
        Some(f) => Front::from_key(f).ok_or_else(|| format!("`{f}` is no front: +z, -z, +x or -x"))?,
    };
    let scale = match text("scale") {
        None => None,
        Some(s) => Some(FramingScale::from_key(s).ok_or_else(|| format!("`{s}` is no framing scale: cameras, cameras_and_lights or none"))?),
    };
    Ok(ApplyOptions {
        front,
        scale,
        keep_lights: flag("keep_lights"),
        keep_cameras: flag("keep_cameras"),
        keep_placement: flag("keep_placement"),
    })
}

/// What a `save` asks the studio to be saved as.
pub fn save_as(request: &Value) -> SaveAs {
    let text = |key: &str| request.get(key).and_then(|v| v.as_str()).unwrap_or_default().to_string();
    SaveAs {
        name: text("name"),
        family: text("family"),
        size: text("size"),
        description: text("description"),
        materials: request.get("materials").and_then(|v| v.as_bool()).unwrap_or(false),
    }
}

/// The installed preset a request names: its id, as `preset` - `id` is
/// the request's own on the wire.
pub fn named(request: &Value, action: &str) -> Result<PresetFile, String> {
    let id = request
        .get("preset")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("`{action}` needs the installed preset's id, as \"preset\""))?;
    presets::find(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Value {
        tequila_format::json::parse(text.as_bytes()).unwrap()
    }

    #[test]
    fn options_are_read_with_their_defaults_and_refused_when_wrong() {
        let read = options(&parse(r#"{"front":"-x","scale":"none","keep_lights":true}"#)).unwrap();
        assert_eq!(read.front, Front::MinusX);
        assert_eq!(read.scale, Some(FramingScale::None));
        assert!(read.keep_lights && !read.keep_cameras && !read.keep_placement);
        assert_eq!(options(&parse("{}")).unwrap(), ApplyOptions::default());
        assert!(options(&parse(r#"{"front":"up"}"#)).unwrap_err().contains("no front"));
        assert!(options(&parse(r#"{"scale":"all"}"#)).unwrap_err().contains("no framing scale"));
    }
}
