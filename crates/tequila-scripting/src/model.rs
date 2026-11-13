//! `improve` and `model_units`: two things the window does to a whole
//! model that no command carried - a mesh made finer or shaded again, and
//! a model's size put right when its file named no unit.
//!
//! Both come to a command the window's own tools make
//! (`tequila_scene::refine`, `tequila_scene::model_size`), one step to
//! undo. Asked with nothing to do, each says what there is to choose from:
//! the triangles a subdivision would make, the size the model would be in
//! each unit.

use crate::document::LibraryOutcome;
use tequila_core::units::LengthUnit;
use tequila_format::json::Value;
use tequila_format::json_write::Object;
use tequila_scene::document::SceneDocument;
use tequila_scene::tree::{NodeId, NodeKind};
use tequila_scene::{model_size, refine};

/// The units a model's numbers may be in, smallest first, each with the
/// names a request may call it.
const UNITS: [(LengthUnit, &[&str]); 5] = [
    (LengthUnit::Millimetre, &["mm", "millimetre", "millimetres", "millimeter", "millimeters"]),
    (LengthUnit::Centimetre, &["cm", "centimetre", "centimetres", "centimeter", "centimeters"]),
    (LengthUnit::Inch, &["in", "inch", "inches"]),
    (LengthUnit::Foot, &["ft", "foot", "feet"]),
    (LengthUnit::Metre, &["m", "metre", "metres", "meter", "meters"]),
];

fn unit_named(request: &Value, key: &str) -> Result<Option<LengthUnit>, String> {
    match request.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(named) => {
            let name = named.as_str().map(|n| n.trim().to_ascii_lowercase()).unwrap_or_default();
            UNITS.iter().find(|(_, names)| names.contains(&name.as_str())).map(|(unit, _)| Some(*unit)).ok_or_else(|| format!("`{key}` is a unit: mm, cm, in, ft or m"))
        }
    }
}

fn node_of(document: &SceneDocument, request: &Value) -> Result<NodeId, String> {
    let node = request.get("node").and_then(Value::as_index).map(|id| NodeId(id as u64)).ok_or("say which node: its id, as `node` (tequila_nodes lists them)")?;
    document.tree.get(node).map(|_| node).ok_or_else(|| format!("there is no node {}", node.0))
}

fn triangles_of(document: &SceneDocument, parts: &[NodeId]) -> usize {
    parts
        .iter()
        .filter_map(|&id| match document.tree.get(id)?.kind {
            NodeKind::Part { mesh } => document.geometry.get(mesh).map(|m| m.triangle_count()),
            _ => None,
        })
        .sum()
}

/// `improve`: the parts at and under `node` made finer (`subdivide`, 1 to
/// 3 times: every triangle becomes four, the new points following the
/// curve its shading describes) or shaded again (`shade`: smooth across
/// edges gentler than that many degrees). With neither, what there is.
pub fn improve(request: &Value, document: &SceneDocument) -> Result<LibraryOutcome, String> {
    let node = node_of(document, request)?;
    let name = document.tree.get(node).map(|n| n.name.clone()).unwrap_or_default();
    let parts = refine::parts_under(document, &[node]);
    if parts.is_empty() {
        return Err(format!("`{name}` has no parts under it to improve"));
    }
    let before = triangles_of(document, &parts);
    let crease = match request.get("crease") {
        None | Some(Value::Null) => 30.0,
        Some(angle) => angle.as_f32().filter(|a| (5.0..=90.0).contains(a)).ok_or("`crease` is an angle in degrees, 5 to 90: edges sharper than it stay sharp")?,
    };
    let mut out = Object::new();
    out.set("node", node.0 as f64).set("name", name.clone()).set("parts", parts.len()).set("triangles_before", before);
    let (command, after) = match (request.get("subdivide"), request.get("shade")) {
        (Some(_), Some(_)) => return Err("one thing at a time: `subdivide` or `shade`".into()),
        (Some(levels), None) => {
            let levels = levels.as_index().filter(|l| (1..=3).contains(l)).ok_or("`subdivide` is how many times, 1 to 3: each makes four triangles of one")? as u32;
            let after = refine::subdivided_count(before, levels);
            if after > refine::MOST_TRIANGLES {
                return Err(format!("that would make {after} triangles of {before}, more than the viewport can turn ({}); subdivide fewer times, or fewer parts", refine::MOST_TRIANGLES));
            }
            out.set("did", format!("subdivided {levels} time(s)"));
            (refine::replace_meshes(document, &parts, |mesh| refine::subdivide(mesh, levels, crease)), after)
        }
        (None, Some(angle)) => {
            let degrees = match angle {
                Value::Bool(true) => crease,
                other => other.as_f32().filter(|a| (5.0..=90.0).contains(a)).ok_or("`shade` is an angle in degrees, 5 to 90: edges sharper than it stay sharp")?,
            };
            out.set("did", format!("shaded again, smooth across edges gentler than {degrees} degrees"));
            (refine::replace_meshes(document, &parts, |mesh| refine::reshade(mesh, degrees)), before)
        }
        (None, None) => {
            let mut choices = Object::new();
            for levels in 1..=3u32 {
                let after = refine::subdivided_count(before, levels);
                if after <= refine::MOST_TRIANGLES {
                    choices.set(&levels.to_string(), after);
                }
            }
            out.set("subdivide_would_make", choices.build());
            out.set("note", "`subdivide` 1 to 3 makes the mesh finer, for a model that shows its facets on curves; `shade` (degrees) works its shading out again, for an STL or OBJ that looks faceted all over. A CAD model (STEP, IGES) is better divided again from its exact surfaces: reimport with a finer tessellation (page fixing).");
            return Ok(LibraryOutcome::Done(out.build()));
        }
    };
    let command = command.ok_or("the parts have no meshes to improve")?;
    out.set("triangles_after", after);
    Ok(LibraryOutcome::Commands(vec![command], out.build()))
}

/// `model_units`: the model `node` belongs to, its numbers read as `unit`
/// instead of the metres they came in as (or `read_as`, when it has been
/// read as something else since) - scaled about the middle of its base,
/// so it stands where it stood, as Units of this model does. With no
/// unit, its size now and what it would be in each.
pub fn units(request: &Value, document: &SceneDocument) -> Result<LibraryOutcome, String> {
    let node = node_of(document, request)?;
    let roots = tequila_scene::reimport::import_roots(document, node);
    let name = roots.first().and_then(|&root| document.tree.get(root)).map(|n| n.name.clone()).unwrap_or_else(|| "The model".into());
    let bounds = model_size::bounds_of(document, &roots).ok_or_else(|| format!("`{name}` has no geometry to measure"))?;
    let size = bounds.max - bounds.min;
    let read_as = unit_named(request, "read_as")?.unwrap_or(LengthUnit::Metre);
    let mut out = Object::new();
    out.set("model", name.clone()).set("nodes", Value::Array(roots.iter().map(|id| Value::Number(id.0 as f64)).collect())).set("size", model_size::real_size(size));
    let Some(unit) = unit_named(request, "unit")? else {
        let mut choices = Object::new();
        for (unit, names) in UNITS {
            choices.set(names[0], model_size::real_size(size * (unit.to_metres() / read_as.to_metres()) as f32));
        }
        out.set("if_its_numbers_are", choices.build());
        out.set("note", "The size it is now, and the size it would be were its numbers in each unit: send `unit` with the one that reads as the real thing. Nothing was changed.");
        return Ok(LibraryOutcome::Done(out.build()));
    };
    let ratio = unit.to_metres() / read_as.to_metres();
    if (ratio - 1.0).abs() < 1e-9 {
        return Err(format!("`{name}` is read that way already; it is {} - give `read_as` if its numbers are read as something other than metres now", model_size::real_size(size)));
    }
    out.set("was", model_size::real_size(size)).set("size", model_size::real_size(size * ratio as f32)).set("scaled_by", ratio);
    out.set("note", "Only this model's size changed, about the middle of its base. Frame it again (tequila_frame); lights and cameras set up round the old size stay where they were.");
    Ok(LibraryOutcome::Commands(vec![model_size::rescale(document, &roots, model_size::base_of(&bounds), ratio as f32)], out.build()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_format::json;
    use tequila_scene::CommandBus;

    fn ask(text: &str) -> Value {
        json::parse(text.as_bytes()).expect("the test's request is JSON")
    }

    fn a_model() -> (SceneDocument, CommandBus, NodeId) {
        let document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.2);
        let part = document.tree.iter().find(|n| n.mesh().is_some()).map(|n| n.id).expect("a part");
        (document, CommandBus::new(), part)
    }

    fn carry(document: &mut SceneDocument, bus: &mut CommandBus, outcome: Result<LibraryOutcome, String>) -> Result<Value, String> {
        match outcome? {
            LibraryOutcome::Commands(commands, answer) => {
                for command in commands {
                    bus.apply(document, command).map_err(|e| e.to_string())?;
                }
                Ok(answer)
            }
            LibraryOutcome::Done(answer) => Ok(answer),
        }
    }

    #[test]
    fn a_part_is_subdivided_and_shaded_again_a_step_each() {
        let (mut document, mut bus, part) = a_model();
        let outcome = improve(&ask(&format!(r#"{{"node":{}}}"#, part.0)), &document);
        let asked = carry(&mut document, &mut bus, outcome).unwrap();
        let before = asked.get("triangles_before").and_then(Value::as_index).unwrap();
        assert_eq!(asked.get("subdivide_would_make").and_then(|c| c.get("1")).and_then(Value::as_index), Some(before * 4));

        let outcome = improve(&ask(&format!(r#"{{"node":{},"subdivide":1}}"#, part.0)), &document);
        let done = carry(&mut document, &mut bus, outcome).unwrap();
        assert_eq!(done.get("triangles_after").and_then(Value::as_index), Some(before * 4));
        assert_eq!(triangles_of(&document, &[part]), before * 4);
        bus.undo(&mut document).unwrap();
        assert_eq!(triangles_of(&document, &[part]), before);

        let outcome = improve(&ask(&format!(r#"{{"node":{},"shade":40}}"#, part.0)), &document);
        let shaded = carry(&mut document, &mut bus, outcome).unwrap();
        assert!(shaded.get("did").and_then(Value::as_str).unwrap().contains("40 degrees"));

        assert!(improve(&ask(&format!(r#"{{"node":{},"subdivide":9}}"#, part.0)), &document).err().unwrap().contains("1 to 3"));
        assert!(improve(&ask(&format!(r#"{{"node":{},"subdivide":1,"shade":30}}"#, part.0)), &document).err().unwrap().contains("one thing at a time"));
        assert!(improve(&ask(r#"{"node":99999}"#), &document).err().unwrap().contains("no node"));
    }

    #[test]
    fn a_models_numbers_are_read_in_another_unit_about_its_base() {
        let (mut document, mut bus, part) = a_model();
        let roots = tequila_scene::reimport::import_roots(&document, part);
        let before = model_size::bounds_of(&document, &roots).unwrap();
        let outcome = units(&ask(&format!(r#"{{"node":{}}}"#, part.0)), &document);
        let asked = carry(&mut document, &mut bus, outcome).unwrap();
        assert!(asked.get("if_its_numbers_are").and_then(|c| c.get("mm")).is_some());

        let outcome = units(&ask(&format!(r#"{{"node":{},"unit":"cm"}}"#, part.0)), &document);
        let done = carry(&mut document, &mut bus, outcome).unwrap();
        assert!((done.get("scaled_by").and_then(Value::as_f64).unwrap() - 0.01).abs() < 1e-9);
        let after = model_size::bounds_of(&document, &roots).unwrap();
        let (was, now) = (before.max - before.min, after.max - after.min);
        assert!((now.x - was.x * 0.01).abs() < 1e-5 && (now.y - was.y * 0.01).abs() < 1e-5, "{was:?} to {now:?}");
        // It stands where it stood: the middle of its base has not moved.
        assert!((model_size::base_of(&after) - model_size::base_of(&before)).length() < 1e-5);
        bus.undo(&mut document).unwrap();
        assert!(((model_size::bounds_of(&document, &roots).unwrap().max - before.max).length()) < 1e-6);

        assert!(units(&ask(&format!(r#"{{"node":{},"unit":"m"}}"#, part.0)), &document).err().unwrap().contains("already"));
        assert!(units(&ask(&format!(r#"{{"node":{},"unit":"furlong"}}"#, part.0)), &document).err().unwrap().contains("mm, cm, in, ft or m"));
        // Read as millimetres when it should have been inches: 25.4 times.
        let LibraryOutcome::Commands(_, inches) = units(&ask(&format!(r#"{{"node":{},"unit":"in","read_as":"mm"}}"#, part.0)), &document).unwrap() else { panic!() };
        assert!((inches.get("scaled_by").and_then(Value::as_f64).unwrap() - 25.4).abs() < 1e-6);
    }
}
