//! `uv`: a part's UV layout read island by island, and its seams cut and
//! stitched by where they are on the part.
//!
//! The `uv.*` commands name islands by number and edges by the mesh's own
//! vertex numbers, which is what the UV editor has when a person clicks
//! and nothing a script could find out: no request said which island is
//! where. This is the reading - `islands`: each island's number, its box
//! on the layout, how much of the part it is, how stretched, and where on
//! the part it lies - and the two edits that needed mesh vertices:
//!
//! - `cut`: along a `path` of points on the part, in the world's metres,
//!   joined the shortest way by the part's own edges, and carried on to
//!   the island's edge where it stops just short of it (`across`, unless
//!   told not to); or along every edge `sharper_than` an angle.
//! - `stitch`: the seams between the `islands` named.
//!
//! Each edit is the `uv.cut` or `uv.stitch` command it comes to, one step
//! to undo, and answers with how many islands there were and are: their
//! numbers change with a cut, so the layout is read again after one.

use crate::document::LibraryOutcome;
use tequila_core::math::{Aabb, Vec3};
use tequila_format::json::Value;
use tequila_format::json_write::Object;
use tequila_scene::document::SceneDocument;
use tequila_scene::geometry::Mesh;
use tequila_scene::tree::{NodeId, NodeKind};
use tequila_scene::uv::{self, EdgeKind, UvLayout};
use tequila_scene::Command;

/// What a `uv` request can do.
pub const ACTIONS: &[&str] = &["islands", "cut", "stitch"];

/// How many islands one answer lists.
pub const PAGE: usize = 25;

fn rounded(value: f32, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (value as f64 * scale).round() / scale
}

fn rounded_vector(v: Vec3, places: i32) -> Value {
    Value::Array(vec![Value::Number(rounded(v.x, places)), Value::Number(rounded(v.y, places)), Value::Number(rounded(v.z, places))])
}

/// The part a request names, its mesh and the UV set it means.
fn part<'d>(document: &'d SceneDocument, request: &Value) -> Result<(NodeId, &'d str, &'d Mesh, usize), String> {
    let node = request.get("node").and_then(Value::as_index).map(|id| NodeId(id as u64)).ok_or("say which part: its id, as `node` (tequila_nodes lists them)")?;
    let target = document.tree.get(node).ok_or_else(|| format!("there is no node {}", node.0))?;
    let NodeKind::Part { mesh } = target.kind else {
        return Err(format!("`{}` is not a part, and has no UV layout", target.name));
    };
    let mesh = document.geometry.get(mesh).ok_or("the part has no mesh")?;
    let set = request.get("set").and_then(Value::as_index).unwrap_or(0);
    Ok((node, &target.name, mesh, set))
}

fn layout_of(mesh: &Mesh, name: &str, set: usize) -> Result<UvLayout, String> {
    UvLayout::new(mesh, set).ok_or_else(|| match mesh.uv_sets.len() {
        0 => format!("`{name}` has no UV layout: unwrap it first (scene.unwrap_uvs)"),
        sets => format!("`{name}` has no UV set {set}; it has {sets}, numbered from 0"),
    })
}

/// The world's axis a direction mostly points along, when it does.
fn facing(direction: Vec3) -> &'static str {
    let (x, y, z) = (direction.x.abs(), direction.y.abs(), direction.z.abs());
    if x >= y && x >= z {
        if direction.x > 0.0 { "+x" } else { "-x" }
    } else if y >= z {
        if direction.y > 0.0 { "+y (up)" } else { "-y (down)" }
    } else if direction.z > 0.0 {
        "+z"
    } else {
        "-z"
    }
}

/// The layout, an island a line: largest on the part first.
fn islands(document: &SceneDocument, request: &Value) -> Result<Value, String> {
    let (node, name, mesh, set) = part(document, request)?;
    let layout = layout_of(mesh, name, set)?;
    let page = match request.get("page") {
        None | Some(Value::Null) => 1,
        Some(page) => page.as_index().filter(|page| *page >= 1).ok_or("`page` is a whole number, from 1")?,
    };
    let pages = layout.islands.len().div_ceil(PAGE).max(1);
    if page > pages {
        return Err(format!("this layout has {pages} page(s) of islands"));
    }
    let world = document.tree.world_transform(node);
    let stretch = uv::distortion(mesh, set).map_err(|e| e.to_string())?;
    let surface: f32 = layout.islands.iter().map(|i| i.surface_area).sum();
    let flat: f32 = layout.islands.iter().map(|i| i.uv_area).sum();
    let overall = if surface > 0.0 { flat / surface } else { 0.0 };
    // The world's square metres to one of the part's own.
    let scale = world.matrix.transform_vector(Vec3 { x: 1.0, y: 0.0, z: 0.0 }).length();
    let mut order: Vec<usize> = (0..layout.islands.len()).collect();
    order.sort_by(|a, b| layout.islands[*b].surface_area.total_cmp(&layout.islands[*a].surface_area).then(a.cmp(b)));
    let lines: Vec<Value> = order
        .iter()
        .skip((page - 1) * PAGE)
        .take(PAGE)
        .map(|&index| {
            let island = &layout.islands[index];
            let mut bounds = Aabb::EMPTY;
            let mut normal = Vec3::ZERO;
            let (mut weighted, mut weight) = (0.0f64, 0.0f64);
            for &triangle in &island.triangles {
                let corners = mesh.triangle(triangle as usize).map(|v| world.point(mesh.positions[v as usize]));
                for corner in corners {
                    bounds.extend_point(corner);
                }
                // Twice the triangle's area, along its normal.
                let face = (corners[1] - corners[0]).cross(corners[2] - corners[0]);
                normal = normal + face;
                let area = face.length() as f64;
                let angle = stretch[triangle as usize].angle;
                if angle.is_finite() {
                    weighted += angle as f64 * area;
                    weight += area;
                }
            }
            let area: f32 = island.triangles.iter().map(|&t| {
                let c = mesh.triangle(t as usize).map(|v| world.point(mesh.positions[v as usize]));
                (c[1] - c[0]).cross(c[2] - c[0]).length()
            }).sum();
            let mut line = Object::new();
            line.set("island", index)
                .set("box", Value::Array([island.min.x, island.min.y, island.max.x, island.max.y].iter().map(|v| Value::Number(rounded(*v, 3))).collect()))
                .set("of_part", rounded(if surface > 0.0 { island.surface_area / surface } else { 0.0 }, 3))
                .set("area_cm2", rounded(island.surface_area * scale * scale * 1e4, 2))
                .set("triangles", island.triangles.len())
                .set("stretch", if weight > 0.0 { rounded((weighted / weight) as f32, 2) } else { 1.0 })
                .set("density", rounded(if overall > 0.0 { island.density() / overall } else { 0.0 }, 2))
                .set("centre", rounded_vector(bounds.center(), 4))
                .set("size", rounded_vector(bounds.max - bounds.min, 4));
            // An island that faces one way says which; one that goes round
            // the part - a tube's wall - faces every way and none.
            let flatness = if area > 0.0 { normal.length() / area } else { 0.0 };
            line.set("faces", if flatness > 0.6 { facing(normal) } else { "round" });
            if island.mirrored {
                line.set("mirrored", true);
            }
            line.build()
        })
        .collect();
    let mut out = Object::new();
    out.set("node", node.0 as f64)
        .set("name", name)
        .set("set", set)
        .set("sets", mesh.uv_sets.len())
        .set("islands", layout.islands.len())
        .set("seams", layout.edges.iter().filter(|e| e.kind == EdgeKind::Seam).count() / 2)
        .set("covered", rounded(flat, 3))
        .set("page", page)
        .set("pages", pages)
        .set("list", Value::Array(lines));
    out.set(
        "note",
        "Largest first. `island` is the number the uv.* commands take. `box` is [u0, v0, u1, v1] on the layout's square; `centre` and `size` are where the island lies on the part, in the world's metres; `faces` the way it faces, or round when it wraps the part. `stretch` 1 is none (1.2 shows on a label); `density` 1 is as fine as the rest. Numbers change when the layout is cut or unwrapped: read again after.",
    );
    Ok(out.build())
}

fn island_numbers(request: &Value, layout: &UvLayout) -> Result<Vec<u32>, String> {
    let Some(listed) = request.get("islands") else { return Ok(Vec::new()) };
    let numbers: Option<Vec<u32>> = listed.as_array().iter().map(|v| v.as_index().map(|i| i as u32)).collect();
    let numbers = numbers.filter(|_| matches!(listed, Value::Array(_))).ok_or("`islands` is a list of island numbers")?;
    if let Some(missing) = numbers.iter().find(|i| **i as usize >= layout.islands.len()) {
        return Err(format!("there is no island {missing}; this layout has {}, numbered from 0", layout.islands.len()));
    }
    Ok(numbers)
}

/// Carry out a `uv` request (the module says what each action is).
pub fn request(request: &Value, document: &SceneDocument) -> Result<LibraryOutcome, String> {
    let action = request.get("action").and_then(Value::as_str).unwrap_or("islands").trim().to_ascii_lowercase();
    match action.as_str() {
        "islands" => islands(document, request).map(LibraryOutcome::Done),
        "cut" => {
            let (node, name, mesh, set) = part(document, request)?;
            let layout = layout_of(mesh, name, set)?;
            let chosen = island_numbers(request, &layout)?;
            let world = document.tree.world_transform(node);
            let mut out = Object::new();
            let edges = match (request.get("path"), request.get("sharper_than")) {
                (Some(path), None) => {
                    let points: Option<Vec<Vec3>> = path
                        .as_array()
                        .iter()
                        .map(|p| p.as_f32_array().filter(|p| p.len() == 3 && p.iter().all(|n| n.is_finite())).map(|p| world.inverse.transform_point(Vec3 { x: p[0], y: p[1], z: p[2] })))
                        .collect();
                    let points = points.filter(|p| p.len() >= 2).ok_or("`path` is two or more points on the part, each [x, y, z] in the world's metres")?;
                    let across = request.get("across").and_then(Value::as_bool).unwrap_or(true);
                    let found = uv::edges_along(mesh, set, &points, across).map_err(|e| e.to_string())?;
                    let scale = world.matrix.transform_vector(Vec3 { x: 1.0, y: 0.0, z: 0.0 }).length();
                    out.set("length_mm", rounded(found.length * scale * 1000.0, 1));
                    // A seam runs along the mesh's edges, from one of its
                    // points to another. Where the mesh has no point near
                    // the one asked for - the middle of a flat CAD face,
                    // which is two large triangles - the nearest edges are
                    // somewhere else, and a cut there is not the cut that
                    // was meant.
                    let bounds = mesh.bounds();
                    let across = (bounds.max - bounds.min).length();
                    let off = rounded(found.furthest * scale * 1000.0, 1);
                    if found.furthest > across * 0.05 {
                        return Err(format!(
                            "the part's mesh has no point within {off} mm of a point of the path, so there is no edge to cut along there: a seam follows the mesh's own edges, and a flat face of a CAD model is a few large triangles. Check the path against the part's centre and size; or cut along its edges (sharper_than); or give the mesh more edges first (tequila_improve subdivide) and cut again"
                        ));
                    }
                    if found.furthest > across * 0.02 {
                        out.set("warning", format!("A point of the path was {off} mm from the nearest point of the mesh: the cut starts there. Look at the layout."));
                    }
                    found.edges
                }
                (None, Some(angle)) => {
                    let degrees = angle.as_f32().filter(|a| (1.0..=179.0).contains(a)).ok_or("`sharper_than` is an angle in degrees, 1 to 179")?;
                    uv::angle_cut::sharp_edges(mesh, set, &chosen, degrees).map_err(|e| e.to_string())?
                }
                _ => return Err("say where to cut: a `path` of points on the part, or every edge `sharper_than` an angle".into()),
            };
            // A seam already is no cut to make: only edges inside an
            // island are, whichever side of one names it.
            let welded = |edge: [u32; 2]| {
                let (a, b) = (layout.weld[edge[0] as usize], layout.weld[edge[1] as usize]);
                (a.min(b), a.max(b))
            };
            let inside: std::collections::HashSet<(u32, u32)> = layout.edges.iter().filter(|e| e.kind == EdgeKind::Inner).map(|e| welded(e.vertices)).collect();
            let inner: Vec<[u32; 2]> = edges.into_iter().filter(|edge| inside.contains(&welded(*edge))).collect();
            if inner.is_empty() {
                return Err("there is nothing to cut there: every edge on the way is a seam already, or the edge of the part".into());
            }
            let reflatten = request.get("reflatten").and_then(Value::as_bool).unwrap_or(true);
            let after = uv::cut(mesh, set, &inner, &uv::CutOptions { reflatten, ..Default::default() }).map_err(|e| e.to_string())?;
            let islands_after = UvLayout::new(&after, set).map(|l| l.islands.len()).unwrap_or(layout.islands.len());
            let mut commands = vec![Command::CutUvs { node, set, edges: inner.clone(), reflatten }];
            let pack = request.get("pack").and_then(Value::as_bool).unwrap_or(false);
            if pack {
                commands.push(Command::PackUvs { node, set, islands: Vec::new(), options: uv::PackOptions::default() });
            }
            out.set("node", node.0 as f64)
                .set("set", set)
                .set("edges_cut", inner.len())
                .set("islands_before", layout.islands.len())
                .set("islands_after", islands_after)
                .set("packed", pack);
            out.set(
                "note",
                match islands_after == layout.islands.len() {
                    true => "The cut opened a seam inside an island without dividing it: to divide one, the path runs from one edge of the island to another (its centre, half its size either way). Read the islands again: the layout changed.",
                    false => "The islands are numbered anew: read them again before naming one.",
                },
            );
            Ok(LibraryOutcome::Commands(commands, out.build()))
        }
        "stitch" => {
            let (node, name, mesh, set) = part(document, request)?;
            let layout = layout_of(mesh, name, set)?;
            let chosen = island_numbers(request, &layout)?;
            if chosen.is_empty() {
                return Err("say which islands to join, as `islands`: two or more that meet along a seam, or one to shut a seam inside it".into());
            }
            let edges = layout.shared_seams(&chosen);
            if edges.is_empty() {
                return Err(format!("islands {chosen:?} share no seam: they do not touch on the part"));
            }
            let after = uv::stitch(mesh, set, &edges).map_err(|e| e.to_string())?;
            let islands_after = UvLayout::new(&after, set).map(|l| l.islands.len()).unwrap_or(layout.islands.len());
            let mut out = Object::new();
            out.set("node", node.0 as f64)
                .set("set", set)
                .set("seams_shut", edges.len())
                .set("islands_before", layout.islands.len())
                .set("islands_after", islands_after)
                .set("note", "The islands are numbered anew: read them again before naming one. Lay the joined island flat again with uv.reflatten if it shows stretch.");
            Ok(LibraryOutcome::Commands(vec![Command::StitchUvs { node, set, edges }], out.build()))
        }
        other => Err(format!("`{other}` is not something `uv` does; its actions are {}", ACTIONS.join(", "))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_format::json;
    use tequila_scene::command::{reserve_keys, ImportPayload};
    use tequila_scene::{CommandBus, SceneNode};

    fn ask(text: &str) -> Value {
        json::parse(text.as_bytes()).expect("the test's request is JSON")
    }

    /// A box 10 by 4 by 6 cm, standing on the floor a metre along x,
    /// unwrapped: six islands, one a face.
    fn a_box() -> (SceneDocument, CommandBus, NodeId) {
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();
        let (meshes, _, ids) = reserve_keys(&document, 1, 0, 1);
        let (w, h, d) = (0.10f32, 0.04f32, 0.06f32);
        let v = tequila_core::math::vec3;
        let faces: [[Vec3; 4]; 6] = [
            [v(0.0, 0.0, d), v(w, 0.0, d), v(w, h, d), v(0.0, h, d)],
            [v(w, 0.0, 0.0), v(0.0, 0.0, 0.0), v(0.0, h, 0.0), v(w, h, 0.0)],
            [v(0.0, 0.0, 0.0), v(0.0, 0.0, d), v(0.0, h, d), v(0.0, h, 0.0)],
            [v(w, 0.0, d), v(w, 0.0, 0.0), v(w, h, 0.0), v(w, h, d)],
            [v(0.0, h, d), v(w, h, d), v(w, h, 0.0), v(0.0, h, 0.0)],
            [v(0.0, 0.0, 0.0), v(w, 0.0, 0.0), v(w, 0.0, d), v(0.0, 0.0, d)],
        ];
        // Each face a grid of quads, so a path has edges to follow.
        let (mut positions, mut indices) = (Vec::new(), Vec::new());
        let n = 4;
        for quad in faces {
            let base = positions.len() as u32;
            for r in 0..=n {
                for c in 0..=n {
                    let (s, t) = (c as f32 / n as f32, r as f32 / n as f32);
                    positions.push(quad[0] + (quad[1] - quad[0]) * s + (quad[3] - quad[0]) * t);
                }
            }
            let at = |r: u32, c: u32| base + r * (n + 1) + c;
            for r in 0..n {
                for c in 0..n {
                    indices.extend_from_slice(&[at(r, c), at(r, c + 1), at(r + 1, c + 1), at(r, c), at(r + 1, c + 1), at(r + 1, c)]);
                }
            }
        }
        let mesh = Mesh::new(positions, indices);
        let mut node = SceneNode::new(ids[0], "Case", NodeKind::Part { mesh: meshes[0] });
        node.transform = tequila_core::math::Transform::from_matrix(tequila_core::math::Mat4::translation(v(1.0, 0.0, 0.0))).unwrap();
        let payload = ImportPayload { meshes: vec![(meshes[0], mesh)], textures: Vec::new(), materials: Vec::new(), nodes: vec![node], roots: vec![ids[0]] };
        bus.apply(&mut document, Command::Import { payload: Box::new(payload) }).unwrap();
        bus.apply(&mut document, Command::UnwrapUvs { node: ids[0], options: Default::default() }).unwrap();
        (document, bus, ids[0])
    }

    fn carry(document: &mut SceneDocument, bus: &mut CommandBus, text: &str) -> Result<Value, String> {
        match request(&ask(text), document)? {
            LibraryOutcome::Commands(commands, answer) => {
                bus.begin_group();
                for command in commands {
                    bus.apply(document, command).map_err(|e| e.to_string())?;
                }
                bus.end_group();
                Ok(answer)
            }
            LibraryOutcome::Done(answer) => Ok(answer),
        }
    }

    /// The reading says which island is which face: the top is the one
    /// facing up, at the box's height, the size of its top.
    #[test]
    fn the_islands_say_where_each_lies_on_the_part() {
        let (mut document, mut bus, node) = a_box();
        let read = carry(&mut document, &mut bus, &format!(r#"{{"op":"uv","node":{}}}"#, node.0)).unwrap();
        assert_eq!(read.get("islands").and_then(Value::as_index), Some(6));
        let list = read.get("list").map(Value::as_array).unwrap();
        assert_eq!(list.len(), 6);
        let top = list.iter().find(|i| i.get("faces").and_then(Value::as_str) == Some("+y (up)")).expect("one island faces up");
        let centre = top.get("centre").and_then(Value::as_f32_array).unwrap();
        assert!((centre[0] - 1.05).abs() < 1e-3 && (centre[1] - 0.04).abs() < 1e-3 && (centre[2] - 0.03).abs() < 1e-3, "{centre:?}");
        assert!((top.get("area_cm2").and_then(Value::as_f32).unwrap() - 60.0).abs() < 0.5);
        assert!((top.get("stretch").and_then(Value::as_f32).unwrap() - 1.0).abs() < 0.05, "a flat face lies flat");
        // Largest first, and every island's box inside the square.
        let areas: Vec<f32> = list.iter().map(|i| i.get("area_cm2").and_then(Value::as_f32).unwrap()).collect();
        assert!(areas.windows(2).all(|w| w[0] >= w[1]), "{areas:?}");
        for island in list {
            let b = island.get("box").and_then(Value::as_f32_array).unwrap();
            assert!(b[0] >= -0.001 && b[1] >= -0.001 && b[2] <= 1.001 && b[3] <= 1.001 && b[0] < b[2] && b[1] < b[3], "{b:?}");
        }
        assert!(json_len(&read) < 4_000, "{} characters for six islands", json_len(&read));
    }

    fn json_len(value: &Value) -> usize {
        tequila_format::json_write::to_string(value).len()
    }

    /// A path across the top, said in the world's metres, cuts the top in
    /// two; stitching the two halves makes it one again.
    #[test]
    fn a_path_on_the_part_cuts_an_island_and_a_stitch_joins_it() {
        let (mut document, mut bus, node) = a_box();
        let cut = carry(&mut document, &mut bus, &format!(r#"{{"op":"uv","action":"cut","node":{},"path":[[1.05,0.04,0.0],[1.05,0.04,0.06]]}}"#, node.0)).unwrap();
        assert_eq!(cut.get("islands_before").and_then(Value::as_index), Some(6));
        assert_eq!(cut.get("islands_after").and_then(Value::as_index), Some(7));
        assert_eq!(cut.get("edges_cut").and_then(Value::as_index), Some(4));
        assert!((cut.get("length_mm").and_then(Value::as_f32).unwrap() - 60.0).abs() < 0.5);
        let read = carry(&mut document, &mut bus, &format!(r#"{{"op":"uv","node":{}}}"#, node.0)).unwrap();
        let halves: Vec<usize> = read
            .get("list")
            .map(Value::as_array)
            .unwrap()
            .iter()
            .filter(|i| i.get("faces").and_then(Value::as_str) == Some("+y (up)"))
            .map(|i| i.get("island").and_then(Value::as_index).unwrap())
            .collect();
        assert_eq!(halves.len(), 2, "the top is two islands now");

        let joined = carry(&mut document, &mut bus, &format!(r#"{{"op":"uv","action":"stitch","node":{},"islands":[{},{}]}}"#, node.0, halves[0], halves[1])).unwrap();
        assert_eq!(joined.get("islands_after").and_then(Value::as_index), Some(6));
        // Each was one step: two undos, and the layout is as it was.
        bus.undo(&mut document).unwrap();
        bus.undo(&mut document).unwrap();
        let back = carry(&mut document, &mut bus, &format!(r#"{{"op":"uv","node":{}}}"#, node.0)).unwrap();
        assert_eq!(back.get("islands").and_then(Value::as_index), Some(6));
    }

    #[test]
    fn what_cannot_be_cut_is_said_in_words() {
        let (mut document, mut bus, node) = a_box();
        let refused = |document: &mut SceneDocument, bus: &mut CommandBus, tail: &str| carry(document, bus, &format!(r#"{{"op":"uv","node":{},{tail}}}"#, node.0)).unwrap_err();
        assert!(refused(&mut document, &mut bus, r#""action":"cut""#).contains("say where to cut"));
        assert!(refused(&mut document, &mut bus, r#""action":"cut","path":[[1,0,0]]"#).contains("two or more points"));
        // In the middle of a cell of the mesh, far from any of its points.
        assert!(refused(&mut document, &mut bus, r#""action":"cut","path":[[1.0125,0.04,0.0075],[1.0875,0.04,0.0075]]"#).contains("no edge to cut along there"));
        // Along a box's own edge, where the unwrap cut already.
        assert!(refused(&mut document, &mut bus, r#""action":"cut","path":[[1.0,0.04,0.0],[1.1,0.04,0.0]]"#).contains("seam already"));
        assert!(refused(&mut document, &mut bus, r#""action":"cut","sharper_than":30"#).contains("seam already"), "an unwrapped box has no sharp edge left inside an island");
        assert!(refused(&mut document, &mut bus, r#""action":"stitch","islands":[0,99]"#).contains("no island 99"));
        assert!(refused(&mut document, &mut bus, r#""action":"fold""#).contains("actions are"));
        assert!(refused(&mut document, &mut bus, r#""set":3"#).contains("no UV set 3"));
        assert!(carry(&mut document, &mut bus, r#"{"op":"uv","node":999}"#).unwrap_err().contains("no node 999"));
    }
}
