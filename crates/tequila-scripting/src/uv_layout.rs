//! `uv_layout`: a part's UV layout written as a picture, for a script -
//! an SVG to draw artwork over, or a PNG (`tequila_scene::uv::export`).
//!
//! An operation rather than a command: it writes a file and changes
//! nothing in the document, so there is nothing to undo or journal.
//!
//! ```json
//! {"op":"uv_layout","node":12,"path":"C:/work/bottle-uv.png","set":0,"size":2048,"fill":"islands"}
//! ```
//!
//! `fill` is `islands` (each a colour of its own), `none`, or a heatmap of
//! how much the layout stretches each triangle: `area` (blue coarser, red
//! finer than the rest) or `angle` (yellow to red). `edges`, `seams` and
//! `note` (the square's size in millimetres, where the layout is as dense
//! all over) are on unless turned off. `line` draws the lines in a colour,
//! `"#rrggbb"`, and `transparent` leaves out the white square under the
//! layout. The format is the path's extension unless `format` says.
//!
//! Both the application and a headless session answer it, as an operation of
//! their own beside the protocol's (`crate::server`).

use tequila_format::json::Value;
use tequila_format::json_write::Object;
use tequila_scene::document::SceneDocument;
use tequila_scene::tree::{NodeId, NodeKind};
use tequila_scene::uv::{layout_png, layout_svg, Fill, LayoutPicture, Measure, UvLayout};

/// The operation's name, as a request's `op`.
pub const OPERATION: &str = "uv_layout";

/// Write the layout a request asks for, and say what was written.
pub fn answer(document: &SceneDocument, request: &Value) -> Result<Value, String> {
    let node = request
        .get("node")
        .and_then(|v| v.as_index())
        .map(|v| NodeId(v as u64))
        .ok_or("`uv_layout` needs the part's `node`")?;
    let path = request.get("path").and_then(|v| v.as_str()).ok_or("`uv_layout` needs a `path` to write, .svg or .png")?;
    let set = request.get("set").and_then(|v| v.as_index()).unwrap_or(0);
    let size = request.get("size").and_then(|v| v.as_index()).unwrap_or(2048).clamp(16, tequila_scene::uv::export::MOST_PIXELS as usize) as u32;
    let fill = match request.get("fill").and_then(|v| v.as_str()).unwrap_or("islands") {
        "islands" => Fill::Islands,
        "none" => Fill::None,
        "area" => Fill::Distortion(Measure::Area),
        "angle" => Fill::Distortion(Measure::Angle),
        other => return Err(format!("`{other}` is not a fill: islands, none, area or angle")),
    };
    let flag = |key: &str| request.get(key).and_then(|v| v.as_bool()).unwrap_or(true);
    let line = match request.get("line").and_then(|v| v.as_str()) {
        Some(text) => Some(colour(text).ok_or_else(|| format!("`{text}` is not a colour: give one as \"#rrggbb\""))?),
        None => None,
    };
    let transparent = request.get("transparent").and_then(|v| v.as_bool()).unwrap_or(false);
    let picture = LayoutPicture { size, fill, edges: flag("edges"), seams: flag("seams"), note: flag("note"), line, transparent };
    let format = match request.get("format").and_then(|v| v.as_str()) {
        Some(format) => format.to_ascii_lowercase(),
        None => std::path::Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("png").to_ascii_lowercase(),
    };
    let target = document.tree.get(node).ok_or_else(|| format!("there is no node {}", node.0))?;
    let NodeKind::Part { mesh } = target.kind else {
        return Err(format!("`{}` is not a part, and has no UV layout", target.name));
    };
    let mesh = document.geometry.get(mesh).ok_or("the part has no mesh")?;
    let layout = UvLayout::new(mesh, set).ok_or_else(|| format!("`{}` has no UV set {set}", target.name))?;
    let bytes = match format.as_str() {
        "svg" => layout_svg(mesh, set, &picture).map_err(|e| e.to_string())?.into_bytes(),
        "png" => layout_png(mesh, set, &picture)?,
        other => return Err(format!("`{other}` is not a layout format: svg or png")),
    };
    std::fs::write(path, &bytes).map_err(|e| format!("Could not write {path}: {e}"))?;
    let mut out = Object::new();
    out.set("path", path)
        .set("format", format)
        .set("bytes", bytes.len())
        .set("size", size as usize)
        .set("set", set)
        .set("islands", layout.islands.len())
        .set("seams", layout.edges.iter().filter(|e| e.kind == tequila_scene::uv::EdgeKind::Seam).count() / 2)
        .set_some("units_per_uv", layout.units_per_uv)
        .set_some("note", tequila_scene::uv::export::size_note(layout.units_per_uv, size));
    Ok(out.build())
}

/// A colour written `#rrggbb`, the hash optional.
pub fn colour(text: &str) -> Option<[u8; 3]> {
    let digits = text.trim().trim_start_matches('#');
    if digits.len() != 6 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_scene::command::{reserve_keys, ImportPayload};
    use tequila_scene::{Command, CommandBus, SceneNode};

    fn a_part() -> (SceneDocument, NodeId) {
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();
        let (meshes, _, ids) = reserve_keys(&document, 1, 0, 1);
        let positions = vec![
            tequila_core::math::vec3(0.0, 0.0, 0.0),
            tequila_core::math::vec3(0.1, 0.0, 0.0),
            tequila_core::math::vec3(0.1, 0.05, 0.0),
            tequila_core::math::vec3(0.0, 0.05, 0.0),
        ];
        let mesh = tequila_scene::Mesh::new(positions, vec![0, 1, 2, 0, 2, 3]);
        let part = SceneNode::new(ids[0], "Label panel", NodeKind::Part { mesh: meshes[0] });
        let payload = ImportPayload { meshes: vec![(meshes[0], mesh)], textures: Vec::new(), materials: Vec::new(), nodes: vec![part], roots: vec![ids[0]] };
        bus.apply(&mut document, Command::Import { payload: Box::new(payload) }).unwrap();
        bus.apply(&mut document, Command::UnwrapUvs { node: ids[0], options: Default::default() }).unwrap();
        (document, ids[0])
    }

    #[test]
    fn a_layout_is_written_as_svg_and_png() {
        let (document, node) = a_part();
        let folder = std::env::temp_dir().join(format!("tequila-uv-layout-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        for (name, fill) in [("layout.svg", "islands"), ("layout.png", "angle")] {
            let path = folder.join(name);
            let request = tequila_format::json::parse(
                format!(r#"{{"op":"uv_layout","node":{},"path":{:?},"size":256,"fill":"{fill}"}}"#, node.0, path.display().to_string()).as_bytes(),
            )
            .unwrap();
            let reply = answer(&document, &request).unwrap();
            assert_eq!(reply.get("islands").and_then(|v| v.as_index()), Some(1));
            let note = reply.get("note").and_then(|v| v.as_str()).expect("a note: an unwrap is even");
            assert!(note.contains("mm on the part"), "{note}");
            // The panel's 100 mm side across the square, less its margin.
            let metres = reply.get("units_per_uv").and_then(|v| v.as_f64()).unwrap();
            assert!((metres - 0.1).abs() < 0.002, "{metres}");
            let written = std::fs::read(&path).unwrap();
            assert_eq!(written.len(), reply.get("bytes").and_then(|v| v.as_index()).unwrap());
        }
        let bad = tequila_format::json::parse(format!(r#"{{"op":"uv_layout","node":{},"path":"x.png","set":4}}"#, node.0).as_bytes()).unwrap();
        assert!(answer(&document, &bad).unwrap_err().contains("no UV set 4"));
        let _ = std::fs::remove_dir_all(&folder);
    }
}
