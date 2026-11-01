//! The UV editor's operations on the parts of the sample scenes: real CAD
//! tessellation, with its slivers, fillets and shared meshes, rather than
//! the tidy meshes the unit tests make.
//!
//! Each operation is checked as the unit tests check it - nothing folds,
//! the area and the seams are what they should be - and, with
//! `TEQUILA_UV_PICTURES` naming a folder, drawn there before and after as
//! PNGs to look at.

use std::path::{Path, PathBuf};
use tequila_core::math::{vec2, Vec2};
use tequila_scene::geometry::Mesh;
use tequila_scene::tree::NodeKind;
use tequila_scene::uv::*;

fn sample(name: &str) -> tequila_scene::SceneDocument {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/samples").join(name);
    tequila_format::load(&path).unwrap_or_else(|e| panic!("{name}: {e}")).document
}

/// The sample's parts by mesh, largest first: each mesh once, with the
/// name of a part that wears it.
fn parts(document: &tequila_scene::SceneDocument) -> Vec<(String, Mesh)> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out: Vec<(String, Mesh)> = Vec::new();
    for node in document.tree.iter() {
        let NodeKind::Part { mesh } = node.kind else { continue };
        if !seen.insert(mesh) {
            continue;
        }
        if let Some(m) = document.geometry.get(mesh) {
            if m.triangle_count() > 0 {
                out.push((node.name.clone(), m.clone()));
            }
        }
    }
    out.sort_by(|a, b| b.1.triangle_count().cmp(&a.1.triangle_count()).then(a.0.cmp(&b.0)));
    out
}

fn folder() -> Option<PathBuf> {
    let folder = PathBuf::from(std::env::var_os("TEQUILA_UV_PICTURES")?);
    std::fs::create_dir_all(&folder).ok()?;
    Some(folder)
}

fn picture(mesh: &Mesh, name: &str, fill: Fill) {
    let Some(folder) = folder() else { return };
    let png = layout_png(mesh, 0, &LayoutPicture { size: 1024, fill, ..Default::default() }).expect("a picture");
    std::fs::write(folder.join(format!("{name}.png")), png).unwrap();
}

fn no_folds(mesh: &Mesh, what: &str) {
    let layout = UvLayout::new(mesh, 0).expect("a layout");
    let folded = layout.folded_triangles();
    assert!(folded.is_empty(), "{what}: {} of {} triangles folded", folded.len(), mesh.triangle_count());
}

fn slug(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect::<String>().trim_matches('-').to_string()
}

/// How much of the square the row packer the unwrap used before covered
/// with these islands: each turned along its principal axis, wider than
/// tall, put in rows as wide as the square the islands would make.
fn row_coverage(layout: &UvLayout, margin: f32) -> f32 {
    let mut boxes: Vec<Vec2> = Vec::new();
    for island in &layout.islands {
        let points: Vec<Vec2> = island.uv_vertices.iter().map(|&v| layout.uv_vertices[v as usize].uv).collect();
        let n = points.len() as f32;
        let mean = points.iter().fold(Vec2::ZERO, |a, p| a + *p) / n;
        let (mut xx, mut xy, mut yy) = (0.0f32, 0.0f32, 0.0f32);
        for p in &points {
            let d = *p - mean;
            xx += d.x * d.x;
            xy += d.x * d.y;
            yy += d.y * d.y;
        }
        let angle = 0.5 * (2.0 * xy).atan2(xx - yy);
        let (c, s) = (angle.cos(), angle.sin());
        let (mut low, mut high) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
        for p in &points {
            let q = vec2(p.x * c + p.y * s, -p.x * s + p.y * c);
            low = low.min(q);
            high = high.max(q);
        }
        let size = high - low;
        boxes.push(if size.y > size.x { vec2(size.y, size.x) } else { size });
    }
    let area: f32 = boxes.iter().map(|b| b.x * b.y).sum();
    let widest = boxes.iter().map(|b| b.x).fold(0.0f32, f32::max);
    let row_width = (area.sqrt() * 1.15).max(widest);
    boxes.sort_by(|a, b| b.y.total_cmp(&a.y));
    let gap = margin * row_width;
    let (mut x, mut y, mut row_height, mut width) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for b in &boxes {
        if x > 0.0 && x + b.x > row_width {
            x = 0.0;
            y += row_height + gap;
            row_height = 0.0;
        }
        width = width.max(x + b.x);
        x += b.x + gap;
        row_height = row_height.max(b.y);
    }
    let side = width.max(y + row_height) * (1.0 + 2.0 * margin);
    let flat: f32 = layout.islands.iter().map(|i| i.uv_area).sum();
    flat / (side * side)
}

/// Half of an island's triangles, grown out across its edges from its first
/// one, nearest first: one connected patch, as a person picks faces.
fn half_of(mesh: &Mesh, layout: &UvLayout, island: u32) -> Vec<u32> {
    let triangles = &layout.islands[island as usize].triangles;
    let mut by_edge: std::collections::HashMap<(u32, u32), Vec<u32>> = std::collections::HashMap::new();
    for &t in triangles {
        let corners = mesh.triangle(t as usize).map(|v| layout.weld[v as usize]);
        for k in 0..3 {
            let (a, b) = (corners[k], corners[(k + 1) % 3]);
            by_edge.entry((a.min(b), a.max(b))).or_default().push(t);
        }
    }
    let mut picked = std::collections::BTreeSet::new();
    let mut front = std::collections::VecDeque::from([triangles[0]]);
    picked.insert(triangles[0]);
    while let Some(t) = front.pop_front() {
        if picked.len() * 2 >= triangles.len() {
            break;
        }
        let corners = mesh.triangle(t as usize).map(|v| layout.weld[v as usize]);
        for k in 0..3 {
            let (a, b) = (corners[k], corners[(k + 1) % 3]);
            for &n in &by_edge[&(a.min(b), a.max(b))] {
                if picked.len() * 2 < triangles.len() && picked.insert(n) {
                    front.push_back(n);
                }
            }
        }
    }
    picked.into_iter().collect()
}

/// Every operation on one part, with its pictures.
fn exercise(name: &str, mesh: &Mesh) {
    let tag = slug(name);
    let had = UvLayout::new(mesh, 0);
    if had.is_some() {
        picture(mesh, &format!("{tag}-0-before"), Fill::Islands);
    }

    // Unwrap.
    let (unwrapped, report) = unwrap(mesh, &UnwrapOptions::default());
    no_folds(&unwrapped, "unwrap");
    let layout = UvLayout::new(&unwrapped, 0).unwrap();
    assert!(layout.islands.iter().all(|i| !i.mirrored), "{name}: an unwrap reads the right way");
    let rows = row_coverage(&layout, UnwrapOptions::default().margin);
    eprintln!(
        "{name}: {} triangles, {} islands, coverage {:.3} packed (rows would cover {:.3}), {} mm a unit",
        mesh.triangle_count(),
        layout.islands.len(),
        report.coverage,
        rows,
        layout.units_per_uv.map(|u| format!("{:.1}", u * 1000.0)).unwrap_or_else(|| "-".into())
    );
    assert!(report.coverage >= rows * 0.98, "{name}: the packer covers less than rows did: {} < {rows}", report.coverage);
    assert!(layout.units_per_uv.is_some(), "{name}: an unwrap is as dense all over");
    picture(&unwrapped, &format!("{tag}-1-unwrap"), Fill::Islands);
    picture(&unwrapped, &format!("{tag}-1-unwrap-area"), Fill::Distortion(Measure::Area));
    picture(&unwrapped, &format!("{tag}-1-unwrap-angle"), Fill::Distortion(Measure::Angle));
    if let Some(folder) = folder() {
        std::fs::write(folder.join(format!("{tag}-1-unwrap.svg")), layout_svg(&unwrapped, 0, &LayoutPicture::default()).unwrap()).unwrap();
    }

    // The largest island, by area: what the island edits work on.
    let largest = (0..layout.islands.len() as u32).max_by(|&a, &b| layout.islands[a as usize].uv_area.total_cmp(&layout.islands[b as usize].uv_area)).unwrap();
    let island = &layout.islands[largest as usize];

    // Cut it in two along the border of faces picked as a person picks them:
    // a patch grown out from its first triangle to half of it. Then stitch
    // the halves back.
    let left = half_of(&unwrapped, &layout, largest);
    let border = layout.border_of_faces(&left);
    if !left.is_empty() && left.len() < island.triangles.len() {
        let parted = cut(&unwrapped, 0, &border, &CutOptions::default()).unwrap();
        no_folds(&parted, "cut");
        let after = UvLayout::new(&parted, 0).unwrap();
        assert!(after.islands.len() > layout.islands.len(), "{name}: the cut parts the island");
        let before_area: f32 = layout.islands.iter().map(|i| i.uv_area).sum();
        let after_area: f32 = after.islands.iter().map(|i| i.uv_area).sum();
        assert!((after_area - before_area).abs() / before_area < 0.02, "{name}: the cut keeps the area: {before_area} {after_area}");
        picture(&parted, &format!("{tag}-2-cut"), Fill::Islands);
        let halves = after.islands_of_faces(&island.triangles);
        let seams = after.shared_seams(&halves);
        match stitch(&parted, 0, &seams) {
            Ok(whole) => {
                no_folds(&whole, "stitch the cut");
                let rejoined = UvLayout::new(&whole, 0).unwrap();
                assert_eq!(rejoined.islands_of_faces(&island.triangles).len(), 1, "{name}: stitched back into one");
                picture(&whole, &format!("{tag}-3-stitched-back"), Fill::Islands);
            }
            // Halves laid out apart that would fold where they meet are
            // refused rather than folded.
            Err(UvError::WouldFold) => eprintln!("{name}: the halves would fold stitched back"),
            Err(other) => panic!("{name}: {other}"),
        }
    }

    // Stitch the two islands that share the most seam.
    let mut shared: std::collections::BTreeMap<(u32, u32), usize> = std::collections::BTreeMap::new();
    for edge in layout.edges.iter().filter(|e| e.kind == EdgeKind::Seam) {
        let other = layout.edges[edge.twin.unwrap() as usize].island;
        if edge.island < other {
            *shared.entry((edge.island, other)).or_default() += 1;
        }
    }
    if let Some((&(a, b), _)) = shared.iter().max_by_key(|(pair, n)| (**n, std::cmp::Reverse(**pair))) {
        let seams = layout.shared_seams(&[a, b]);
        match stitch(&unwrapped, 0, &seams) {
            Ok(joined) => {
                no_folds(&joined, "stitch");
                picture(&joined, &format!("{tag}-4-stitch"), Fill::Islands);
            }
            // Two pieces that fold where they meet are refused, not folded.
            Err(UvError::WouldFold) => eprintln!("{name}: islands {a} and {b} would fold stitched"),
            Err(other) => panic!("{name}: {other}"),
        }
    }

    // Pack: islands scaled at random, then packed back, evened.
    let mut jumbled = unwrapped.clone();
    for i in 0..layout.islands.len().min(12) as u32 {
        let k = 0.6 + (i % 5) as f32 * 0.25;
        jumbled = transform(&jumbled, 0, &UvSelection::Islands(vec![i]), &UvAffine::uniform(k).then(&UvAffine::translation(vec2(0.03 * i as f32, 0.0))), Pivot::Islands).unwrap();
    }
    picture(&jumbled, &format!("{tag}-5-pack-before"), Fill::Islands);
    let (packed, pack_report) = pack(&jumbled, 0, None, &PackOptions { normalize: true, ..Default::default() }).unwrap();
    no_folds(&packed, "pack");
    eprintln!("{name}: packed again, coverage {:.3}", pack_report.coverage);
    assert!(UvLayout::new(&packed, 0).unwrap().units_per_uv.is_some(), "{name}: normalized");
    picture(&packed, &format!("{tag}-5-pack-after"), Fill::Islands);

    // Straighten the largest island, turned first.
    let turned = transform(&unwrapped, 0, &UvSelection::Islands(vec![largest]), &UvAffine::rotation(0.45), Pivot::Islands).unwrap();
    picture(&turned, &format!("{tag}-6-straighten-before"), Fill::Islands);
    let straight = straighten(&turned, 0, &[largest], StraightenBy::Axis, Alignment::Nearest).unwrap();
    no_folds(&straight, "straighten");
    // Square by its axis where it has a clear one; a round island has
    // near-equal tightest boxes, and any of them is straight.
    let straightened = &UvLayout::new(&straight, 0).unwrap().islands[largest as usize];
    if straightened.elongation >= 1.5 {
        let angle = straightened.axis;
        let square = (angle.abs()).min((angle.abs() - std::f32::consts::FRAC_PI_2).abs());
        assert!(square < 1e-3, "{name}: straightened to {angle}");
    }
    picture(&straight, &format!("{tag}-6-straighten-after"), Fill::Islands);

    // Even size: some islands scaled up and down, then evened.
    let mut uneven = unwrapped.clone();
    for i in 0..layout.islands.len().min(6) as u32 {
        uneven = transform(&uneven, 0, &UvSelection::Islands(vec![i]), &UvAffine::uniform(0.5 + i as f32 * 0.3), Pivot::Islands).unwrap();
    }
    picture(&uneven, &format!("{tag}-7-even-before"), Fill::Distortion(Measure::Area));
    let all: Vec<u32> = (0..layout.islands.len() as u32).collect();
    let even = even_density(&uneven, 0, &all, DensityTarget::Average).unwrap();
    no_folds(&even, "even density");
    assert!(UvLayout::new(&even, 0).unwrap().units_per_uv.is_some(), "{name}: one density");
    picture(&even, &format!("{tag}-7-even-after"), Fill::Distortion(Measure::Area));

    // Re-unwrap with pins: the largest island squashed, then laid out
    // again with its two farthest-apart vertices pinned.
    let squashed = transform(&unwrapped, 0, &UvSelection::Islands(vec![largest]), &UvAffine::scale(vec2(1.0, 0.45)), Pivot::Islands).unwrap();
    let squashed_layout = UvLayout::new(&squashed, 0).unwrap();
    let by_u = |a: &&u32, b: &&u32| squashed_layout.uv_vertices[**a as usize].uv.x.total_cmp(&squashed_layout.uv_vertices[**b as usize].uv.x);
    let island = &squashed_layout.islands[largest as usize];
    let pins: Vec<u32> = [island.uv_vertices.iter().min_by(by_u).unwrap(), island.uv_vertices.iter().max_by(by_u).unwrap()]
        .iter()
        .map(|&&v| squashed_layout.uv_vertices[v as usize].vertex)
        .collect();
    picture(&squashed, &format!("{tag}-8-pins-before"), Fill::Distortion(Measure::Angle));
    match reflatten(&squashed, 0, &[largest], &pins, None) {
        Ok(relaxed) => {
            no_folds(&relaxed, "reflatten with pins");
            for &p in &pins {
                assert_eq!(relaxed.uv_sets[0][p as usize], squashed.uv_sets[0][p as usize], "{name}: pin {p} moved");
            }
            picture(&relaxed, &format!("{tag}-8-pins-after"), Fill::Distortion(Measure::Angle));
        }
        Err(UvError::CannotFlatten(_)) => eprintln!("{name}: the pinned island would fold laid out again"),
        Err(other) => panic!("{name}: {other}"),
    }

    // Keeping the cuts: the unwrap's own islands laid out again whole.
    let (kept, kept_report) = unwrap(&unwrapped, &UnwrapOptions { respect_cuts: true, ..Default::default() });
    no_folds(&kept, "unwrap keeping cuts");
    assert!(kept_report.charts >= layout.islands.len(), "{name}: no cut is lost");
}

#[test]
fn the_headphones_parts_lay_out_and_edit_cleanly() {
    let document = sample("Headphones.tequila");
    let parts = parts(&document);
    for (name, mesh) in parts.iter().take(3) {
        exercise(name, mesh);
    }
}

#[test]
fn the_perfume_bottle_parts_lay_out_and_edit_cleanly() {
    let document = sample("Perfume bottle.tequila");
    let parts = parts(&document);
    for (name, mesh) in parts.iter().take(3) {
        exercise(name, mesh);
    }
}

/// A part of a sample by name, laid out: its own layout, or an unwrap where
/// it has none.
fn laid_out(document: &tequila_scene::SceneDocument, name: &str) -> Mesh {
    let node = document.tree.iter().find(|n| n.name == name).unwrap_or_else(|| panic!("no part {name}"));
    let NodeKind::Part { mesh } = node.kind else { panic!("{name} is not a part") };
    let mesh = document.geometry.get(mesh).unwrap();
    if mesh.uv_sets.is_empty() { unwrap(mesh, &UnwrapOptions::default()).0 } else { mesh.clone() }
}

/// The arranging tools on real parts: repeated pieces found and laid over
/// each other exactly, islands sorted, set out as on the part, spread,
/// sized alike and squared, and none of it folds.
#[test]
fn the_headphones_repeated_pieces_stack_and_arrange_cleanly() {
    let document = sample("Headphones.tequila");
    for name in ["Knuckle L", "Shell R", "Pad", "Marks L"] {
        let mesh = laid_out(&document, name);
        let tag = format!("arrange-{}", slug(name));
        let layout = UvLayout::new(&mesh, 0).unwrap();
        let all: Vec<u32> = (0..layout.islands.len() as u32).collect();
        let shapes: Vec<(u32, similar::IslandShape)> = all.iter().map(|&i| (i, similar::IslandShape::of(&layout, &mesh, i))).collect();
        let classes = similar::classes(&shapes, 0.05, true);
        let repeated = classes.iter().max_by_key(|c| c.len()).unwrap();
        assert!(repeated.len() >= 2, "{name}: a repeated piece");
        // Stacked: every copy laid over the first, corner for corner.
        let first = &shapes[repeated[0] as usize].1;
        let (mut vertices, mut places) = (Vec::new(), Vec::new());
        for &other in &repeated[1..] {
            let theirs = &shapes[other as usize].1;
            let m = similar::correspond(first, theirs, true).expect("the same piece");
            let (v, p) = similar::laid_over(first, theirs, &m, Vec2::ZERO);
            vertices.extend(v);
            places.extend(p);
        }
        picture(&mesh, &format!("{tag}-1-stack-before"), Fill::Islands);
        let stacked = set_positions_from_lists(&mesh, 0, &vertices, &places).unwrap_or_else(|e| panic!("{name}: {e}"));
        picture(&stacked, &format!("{tag}-1-stack-after"), Fill::Islands);
        let after = UvLayout::new(&stacked, 0).unwrap();
        for &other in &repeated[1..] {
            let (a, b) = (&after.islands[repeated[0] as usize], &after.islands[other as usize]);
            assert!((a.min - b.min).length() < 1e-5 && (a.max - b.max).length() < 1e-5, "{name}: island {other} lies over the first");
        }
        // Sorted along the part, then as on the part: nothing folds, and
        // the islands keep their shapes.
        let options = sort::SortOptions { by: sort::SortBy::Axis(0), wrap: sort::Wrap::Width, spacing: 0.01, ..Default::default() };
        let plan = sort::sort(&layout, &mesh, &all, &options, &sort::PartAxes::default());
        let sorted = place_islands(&mesh, 0, &plan.islands, &plan.maps).unwrap();
        no_folds(&sorted, "sort");
        picture(&sorted, &format!("{tag}-2-sorted"), Fill::Islands);
        let density = arrange::linear_density(&layout, &all).unwrap();
        let plan = sort::as_on_part(&layout, &mesh, &all, sort::View::Auto, &sort::PartAxes::default(), density, 0.005);
        let seen = place_islands(&mesh, 0, &plan.islands, &plan.maps).unwrap();
        no_folds(&seen, "as on the part");
        picture(&seen, &format!("{tag}-3-as-on-the-part"), Fill::Islands);
        let seen_layout = UvLayout::new(&seen, 0).unwrap();
        for (a, b) in layout.islands.iter().zip(&seen_layout.islands) {
            assert!((a.uv_area - b.uv_area).abs() <= a.uv_area * 1e-3 + 1e-9, "{name}: moved, not scaled");
        }
        // Spread, sized alike, fitted, shuffled, squared.
        for plan in [
            arrange::distribute(&layout, &all, true, arrange::Spread::Centres),
            arrange::match_size(&layout, &all, arrange::SizeTarget::Average, true, true).unwrap(),
            arrange::fit(&layout, &all, UvRect::UNIT, 0.01, false).unwrap(),
            arrange::shuffle(&layout, &all, 3),
        ] {
            if !plan.is_empty() {
                no_folds(&place_islands(&mesh, 0, &plan.islands, &plan.maps).unwrap(), "arranged");
            }
        }
        let edges: Vec<u32> = (0..layout.edges.len() as u32).collect();
        let squared = vertices::square_edges(&layout, &mesh, &edges, 15f32.to_radians());
        if !squared.is_empty() {
            if let Ok(out) = set_positions_from_lists(&mesh, 0, &squared.vertices, &squared.uv) {
                no_folds(&out, "squared");
            }
        }
        let coverage = density::coverage(&layout, 1024);
        assert!(coverage.covered > 0.05 && coverage.covered <= 1.0 && coverage.overlap <= coverage.covered, "{name}: {coverage:?}");
    }
    // A shape copied from one part's piece pastes on the other side's.
    let (left, right) = (laid_out(&document, "Name L"), laid_out(&document, "Name R"));
    let (ll, rl) = (UvLayout::new(&left, 0).unwrap(), UvLayout::new(&right, 0).unwrap());
    let copied = similar::IslandShape::of(&ll, &left, 0);
    let target = (0..rl.islands.len() as u32).find_map(|i| similar::correspond(&copied, &similar::IslandShape::of(&rl, &right, i), true).map(|m| (i, m)));
    let (island, m) = target.expect("Name R has Name L's first piece");
    let theirs = similar::IslandShape::of(&rl, &right, island);
    let (vertices, places) = similar::laid_over(&copied, &theirs, &m, theirs.centre() - copied.centre());
    no_folds(&set_positions_from_lists(&right, 0, &vertices, &places).unwrap(), "pasted");
}
