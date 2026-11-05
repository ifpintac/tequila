//! Rhino 7's SubD objects: the control cage read from the file, and the
//! smooth surface it stands for made by Catmull-Clark subdivision.
//!
//! A file keeps a SubD as its cage - level zero's vertices, edges and
//! faces, each vertex and edge tagged smooth or crease - and no mesh of its
//! surface (a level's "render mesh" flag is read and its chunk passed
//! over; no file seen sets it). So the surface is made here: the cage
//! divided a few times by Catmull-Clark's rules, creases kept by the
//! crease rules, and every vertex then moved to where the limit surface
//! has it. Rhino's own evaluation differs in detail (its edge weights near
//! a crease vertex, its semi-sharp creases), not in shape.
//!
//! The layout follows `ON_SubD::Read`: a flag, then an anonymous chunk
//! (`ON_SubDimple`) of levels, each an anonymous chunk listing its
//! vertices, edges and faces by archive id. Rhino 7 and 8 end each
//! component with a run of "additions" (a size byte and that many bytes,
//! 254 for a chunk, 255 to end), which is how later fields are skipped.

use super::archive::{uuid, Object, Reader, Uuid, R};
use super::geometry::MeshData;
use std::collections::HashMap;

pub(super) const ON_SUBD: Uuid = uuid("F09BA4D9-455B-42C3-BA3B-E6CCACEF853B");

type P3 = [f64; 3];

const SMOOTH: u8 = 1;
const CREASE: u8 = 2;
const CORNER: u8 = 3;

/// A SubD's level zero.
#[derive(Clone, Debug, Default)]
pub(super) struct Cage {
    pub points: Vec<P3>,
    /// 0 unset, 1 smooth, 2 crease, 3 corner, 4 dart.
    pub vertex_tags: Vec<u8>,
    /// Each edge's two vertices and its tag: 1 smooth, 2 crease.
    pub edges: Vec<([usize; 2], u8)>,
    /// Each face's vertices, in order round it.
    pub faces: Vec<Vec<usize>>,
}

/// One component reference: an archive id and a direction bit.
fn pointer(r: &mut Reader<'_>) -> R<(u32, bool)> {
    let id = r.u32()?;
    let flags = r.u8()?;
    Ok((id, flags & 1 != 0))
}

/// What one addition marker says.
enum Addition {
    Absent,
    Present,
    End,
}

/// The next addition, which is the one `expected` bytes long when present.
fn addition(r: &mut Reader<'_>, expected: u8) -> R<Addition> {
    loop {
        match r.u8()? {
            0 => return Ok(Addition::Absent),
            v if v == expected => return Ok(Addition::Present),
            254 => {
                r.chunk()?;
            }
            255 => return Ok(Addition::End),
            v => return Err(format!("a SubD addition of {v} bytes where {expected} were expected")),
        }
    }
}

/// The rest of a component's additions, to its end marker.
fn finish(r: &mut Reader<'_>) -> R<()> {
    loop {
        match r.u8()? {
            255 => return Ok(()),
            254 => {
                r.chunk()?;
            }
            0 => {}
            n => r.skip(n as usize)?,
        }
    }
}

/// The start every component shares: its archive id, an id, its level and,
/// before Rhino 7, two optional saved points; after, its additions.
fn base(r: &mut Reader<'_>, expected: u32) -> R<()> {
    let id = r.u32()?;
    if id != expected {
        return Err(format!("SubD component {id} where {expected} was expected"));
    }
    r.u32()?;
    r.u16()?;
    if r.archive < 70 {
        for _ in 0..2 {
            match r.u8()? {
                0 => {}
                4 => r.skip(24)?,
                v => return Err(format!("a SubD saved point of size {v}")),
            }
        }
        return Ok(());
    }
    match addition(r, 24)? {
        Addition::End => return Ok(()),
        Addition::Present => r.skip(24)?,
        Addition::Absent => {}
    }
    match addition(r, 4)? {
        Addition::End => return Ok(()),
        Addition::Present => r.skip(4)?,
        Addition::Absent => {}
    }
    match addition(r, 5)? {
        Addition::End => return Ok(()),
        Addition::Present => r.skip(5)?,
        Addition::Absent => {}
    }
    finish(r)
}

fn end(r: &mut Reader<'_>) -> R<()> {
    if r.archive < 70 {
        r.u8()?;
        Ok(())
    } else {
        finish(r)
    }
}

/// A SubD object's cage; `None` for an empty one.
pub(super) fn subd(object: &Object<'_>) -> R<Option<Cage>> {
    let mut r = object.data;
    if r.u8()? == 0 {
        return Ok(None);
    }
    let (mut s, major, _) = r.anonymous("a SubD")?;
    if major != 1 {
        return Err(format!("a SubD of version {major}"));
    }
    let levels = s.u32()?;
    if levels == 0 {
        return Ok(None);
    }
    s.skip(12)?;
    s.skip(48)?;
    let (mut l, major, minor) = s.anonymous("a SubD level")?;
    if major != 1 || minor < 1 {
        return Err(format!("a SubD level of version {major}.{minor}"));
    }
    if l.u16()? != 0 {
        return Err("a SubD whose first level is not level zero".into());
    }
    l.skip(3)?;
    l.skip(48)?;
    let p = [l.u32()?, l.u32()?, l.u32()?, l.u32()?];
    if !(p[0] <= p[1] && p[1] <= p[2] && p[2] <= p[3]) || (p[3] - p[0]) as usize > l.remaining() / 10 {
        return Err("a SubD level whose component counts do not hold together".into());
    }
    let mut cage = Cage::default();
    let mut vertex_at = HashMap::new();
    for id in p[0]..p[1] {
        base(&mut l, id)?;
        let tag = l.u8()?;
        let point = l.point()?;
        let edges = l.u16()? as usize;
        let faces = l.u16()? as usize;
        if l.u8()? != 0 {
            // Saved limit points: each twelve doubles and a pointer.
            let n = l.u32()? as usize;
            l.skip(n.checked_mul(96 + 5).ok_or("too many saved limit points")?)?;
        }
        let listed = l.u16()? as usize;
        l.skip(listed * 5)?;
        let listed_faces = l.u16()? as usize;
        l.skip(listed_faces * 5)?;
        if listed != edges || listed_faces != faces {
            return Err("a SubD vertex whose counts disagree".into());
        }
        end(&mut l)?;
        vertex_at.insert(id, cage.points.len());
        cage.points.push(point);
        cage.vertex_tags.push(tag);
    }
    let mut edge_at = HashMap::new();
    for id in p[1]..p[2] {
        base(&mut l, id)?;
        let tag = l.u8()?;
        let faces = l.u16()? as usize;
        l.skip(16)?;
        l.f64()?;
        if l.u16()? != 2 {
            return Err("a SubD edge without two ends".into());
        }
        let (a, b) = (pointer(&mut l)?.0, pointer(&mut l)?.0);
        let listed = l.u16()? as usize;
        if listed != faces {
            return Err("a SubD edge whose counts disagree".into());
        }
        l.skip(listed * 5)?;
        if l.archive < 70 {
            l.u8()?;
        } else {
            let mut done = false;
            if l.archive >= 80 {
                match l.u8()? {
                    8 => l.skip(8)?,
                    255 => done = true,
                    v => return Err(format!("a SubD edge's end sharpness of {v} bytes")),
                }
            }
            if !done {
                finish(&mut l)?;
            }
        }
        let ends = [*vertex_at.get(&a).ok_or("a SubD edge on a vertex the cage lacks")?, *vertex_at.get(&b).ok_or("a SubD edge on a vertex the cage lacks")?];
        edge_at.insert(id, cage.edges.len());
        cage.edges.push((ends, tag));
    }
    for id in p[2]..p[3] {
        base(&mut l, id)?;
        l.skip(8)?;
        let n = l.u16()? as usize;
        if l.u16()? as usize != n {
            return Err("a SubD face whose counts disagree".into());
        }
        let mut ring = Vec::with_capacity(n);
        for _ in 0..n {
            let (e, reversed) = pointer(&mut l)?;
            let (ends, _) = cage.edges[*edge_at.get(&e).ok_or("a SubD face on an edge the cage lacks")?];
            ring.push(if reversed { ends[1] } else { ends[0] });
        }
        face_end(&mut l, n)?;
        if ring.len() >= 3 {
            cage.faces.push(ring);
        }
    }
    Ok(Some(cage))
}

/// A face's trailing fields: before Rhino 7 a zero; after, additions for
/// its packing rectangle, material channel, colour, pack id and texture
/// points, then the rest.
fn face_end(r: &mut Reader<'_>, edges: usize) -> R<()> {
    if r.archive < 70 {
        r.u8()?;
        return Ok(());
    }
    match addition(r, 34)? {
        Addition::End => return Ok(()),
        Addition::Present => r.skip(34)?,
        Addition::Absent => {}
    }
    for _ in 0..3 {
        match addition(r, 4)? {
            Addition::End => return Ok(()),
            Addition::Present => r.skip(4)?,
            Addition::Absent => {}
        }
    }
    match addition(r, 4)? {
        Addition::End => return Ok(()),
        Addition::Present => {
            // Texture points, ten corners to a run.
            r.skip(4)?;
            for _ in 0..edges / 10 {
                r.u8()?;
                r.skip(240)?;
            }
            if edges % 10 > 0 {
                r.u8()?;
                r.skip(edges % 10 * 24)?;
            }
        }
        Addition::Absent => {}
    }
    finish(r)
}

/// A mesh of the cage's smooth surface: `levels` rounds of Catmull-Clark,
/// then each vertex moved to its limit position.
pub(super) fn surface(cage: &Cage, levels: usize) -> MeshData {
    let mut points = cage.points.clone();
    let mut faces = cage.faces.clone();
    let mut creases: HashMap<(usize, usize), bool> = HashMap::new();
    for (ends, tag) in &cage.edges {
        creases.insert(key(ends[0], ends[1]), *tag == CREASE);
    }
    let mut tags = cage.vertex_tags.clone();
    for _ in 0..levels {
        (points, faces, creases, tags) = divide(&points, &faces, &creases, &tags);
    }
    let points = limit(&points, &faces, &creases, &tags);

    let mut out = MeshData { positions: points, ..Default::default() };
    for f in &faces {
        if f.len() == 4 {
            let p = |i: usize| out.positions[f[i]];
            let d = |a: P3, b: P3| (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f64>();
            if d(p(0), p(2)) <= d(p(1), p(3)) {
                out.triangles.push([f[0] as u32, f[1] as u32, f[2] as u32]);
                out.triangles.push([f[0] as u32, f[2] as u32, f[3] as u32]);
            } else {
                out.triangles.push([f[0] as u32, f[1] as u32, f[3] as u32]);
                out.triangles.push([f[1] as u32, f[2] as u32, f[3] as u32]);
            }
        } else {
            for i in 1..f.len() - 1 {
                out.triangles.push([f[0] as u32, f[i] as u32, f[i + 1] as u32]);
            }
        }
    }
    out
}

fn key(a: usize, b: usize) -> (usize, usize) {
    (a.min(b), a.max(b))
}

fn average(points: impl Iterator<Item = P3>) -> P3 {
    let (mut s, mut n) = ([0.0; 3], 0.0);
    for p in points {
        for k in 0..3 {
            s[k] += p[k];
        }
        n += 1.0;
    }
    if n == 0.0 { s } else { [s[0] / n, s[1] / n, s[2] / n] }
}

/// What each vertex is, from its tag and the creases at it: a boundary
/// edge is always a crease, two creases make a crease vertex, three or more
/// a corner.
fn kinds(points: usize, faces: &[Vec<usize>], creases: &HashMap<(usize, usize), bool>, tags: &[u8]) -> (Vec<u8>, HashMap<(usize, usize), Vec<usize>>) {
    let mut edge_faces: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for (i, f) in faces.iter().enumerate() {
        for j in 0..f.len() {
            edge_faces.entry(key(f[j], f[(j + 1) % f.len()])).or_default().push(i);
        }
    }
    let mut crease_count = vec![0usize; points];
    for (e, fs) in &edge_faces {
        if fs.len() != 2 || creases.get(e).copied().unwrap_or(false) {
            crease_count[e.0] += 1;
            crease_count[e.1] += 1;
        }
    }
    let kinds = (0..points)
        .map(|v| match (tags.get(v).copied().unwrap_or(0), crease_count[v]) {
            (CORNER, _) => CORNER,
            (_, 0 | 1) => SMOOTH,
            (_, 2) => CREASE,
            _ => CORNER,
        })
        .collect();
    (kinds, edge_faces)
}

fn is_crease(e: (usize, usize), edge_faces: &HashMap<(usize, usize), Vec<usize>>, creases: &HashMap<(usize, usize), bool>) -> bool {
    edge_faces.get(&e).is_none_or(|f| f.len() != 2) || creases.get(&e).copied().unwrap_or(false)
}

type Level = (Vec<P3>, Vec<Vec<usize>>, HashMap<(usize, usize), bool>, Vec<u8>);

/// One round of Catmull-Clark: a point for each face, each edge and each
/// vertex, and four-sided faces between them.
fn divide(points: &[P3], faces: &[Vec<usize>], creases: &HashMap<(usize, usize), bool>, tags: &[u8]) -> Level {
    let (kind, edge_faces) = kinds(points.len(), faces, creases, tags);
    let face_points: Vec<P3> = faces.iter().map(|f| average(f.iter().map(|&v| points[v]))).collect();
    let mut edges: Vec<(usize, usize)> = edge_faces.keys().copied().collect();
    edges.sort_unstable();
    let edge_index: HashMap<(usize, usize), usize> = edges.iter().enumerate().map(|(i, e)| (*e, i)).collect();
    let edge_points: Vec<P3> = edges
        .iter()
        .map(|&e| {
            let mid = average([points[e.0], points[e.1]].into_iter());
            if is_crease(e, &edge_faces, creases) {
                return mid;
            }
            let f = &edge_faces[&e];
            average([points[e.0], points[e.1], face_points[f[0]], face_points[f[1]]].into_iter())
        })
        .collect();
    // Each vertex's neighbours across its edges, and its faces.
    let mut around: Vec<Vec<(usize, usize)>> = vec![Vec::new(); points.len()];
    for &e in &edges {
        around[e.0].push(e);
        around[e.1].push(e);
    }
    let mut vertex_faces: Vec<Vec<usize>> = vec![Vec::new(); points.len()];
    for (i, f) in faces.iter().enumerate() {
        for &v in f {
            vertex_faces[v].push(i);
        }
    }
    let vertex_points: Vec<P3> = (0..points.len())
        .map(|v| {
            let p = points[v];
            let other = |e: (usize, usize)| if e.0 == v { e.1 } else { e.0 };
            match kind[v] {
                CORNER => p,
                CREASE => {
                    let ends: Vec<P3> = around[v].iter().filter(|&&e| is_crease(e, &edge_faces, creases)).map(|&e| points[other(e)]).collect();
                    if ends.len() != 2 {
                        return p;
                    }
                    let mut q = [0.0; 3];
                    for k in 0..3 {
                        q[k] = (6.0 * p[k] + ends[0][k] + ends[1][k]) / 8.0;
                    }
                    q
                }
                _ => {
                    let n = around[v].len() as f64;
                    if n < 3.0 || vertex_faces[v].len() != around[v].len() {
                        return p;
                    }
                    let q = average(vertex_faces[v].iter().map(|&f| face_points[f]));
                    let r = average(around[v].iter().map(|&e| average([points[e.0], points[e.1]].into_iter())));
                    let mut out = [0.0; 3];
                    for k in 0..3 {
                        out[k] = (q[k] + 2.0 * r[k] + (n - 3.0) * p[k]) / n;
                    }
                    out
                }
            }
        })
        .collect();

    // New numbering: vertices, then edges, then faces.
    let (nv, ne) = (points.len(), edges.len());
    let mut next_points = vertex_points;
    next_points.extend(edge_points);
    next_points.extend(face_points);
    let mut next_tags: Vec<u8> = (0..nv).map(|v| if kind[v] == CORNER { CORNER } else { tags.get(v).copied().unwrap_or(0) }).collect();
    next_tags.extend(edges.iter().map(|&e| if is_crease(e, &edge_faces, creases) { CREASE } else { SMOOTH }));
    next_tags.extend(std::iter::repeat_n(SMOOTH, faces.len()));
    let mut next_creases = HashMap::new();
    let mut next_faces = Vec::with_capacity(faces.len() * 4);
    for (i, f) in faces.iter().enumerate() {
        let centre = nv + ne + i;
        for j in 0..f.len() {
            let (prev, here, next) = (f[(j + f.len() - 1) % f.len()], f[j], f[(j + 1) % f.len()]);
            let a = nv + edge_index[&key(here, next)];
            let b = nv + edge_index[&key(prev, here)];
            next_faces.push(vec![here, a, centre, b]);
        }
    }
    for &e in &edges {
        let crease = is_crease(e, &edge_faces, creases);
        let m = nv + edge_index[&e];
        next_creases.insert(key(e.0, m), crease);
        next_creases.insert(key(e.1, m), crease);
    }
    (next_points, next_faces, next_creases, next_tags)
}

/// Each vertex moved to the limit surface: the standard masks for a smooth
/// vertex of a four-sided mesh and for a crease, a corner where it is.
fn limit(points: &[P3], faces: &[Vec<usize>], creases: &HashMap<(usize, usize), bool>, tags: &[u8]) -> Vec<P3> {
    let (kind, edge_faces) = kinds(points.len(), faces, creases, tags);
    let mut neighbours: Vec<Vec<usize>> = vec![Vec::new(); points.len()];
    let mut diagonals: Vec<Vec<usize>> = vec![Vec::new(); points.len()];
    for f in faces {
        if f.len() != 4 {
            continue;
        }
        for j in 0..4 {
            diagonals[f[j]].push(f[(j + 2) % 4]);
        }
    }
    for e in edge_faces.keys() {
        neighbours[e.0].push(e.1);
        neighbours[e.1].push(e.0);
    }
    (0..points.len())
        .map(|v| {
            let p = points[v];
            match kind[v] {
                CORNER => p,
                CREASE => {
                    let ends: Vec<P3> = neighbours[v].iter().filter(|&&u| is_crease(key(u, v), &edge_faces, creases)).map(|&u| points[u]).collect();
                    if ends.len() != 2 {
                        return p;
                    }
                    let mut q = [0.0; 3];
                    for k in 0..3 {
                        q[k] = (4.0 * p[k] + ends[0][k] + ends[1][k]) / 6.0;
                    }
                    q
                }
                _ => {
                    let n = neighbours[v].len();
                    if n < 3 || diagonals[v].len() != n {
                        return p;
                    }
                    let nf = n as f64;
                    let mut q = [0.0; 3];
                    for k in 0..3 {
                        let e: f64 = neighbours[v].iter().map(|&u| points[u][k]).sum();
                        let d: f64 = diagonals[v].iter().map(|&u| points[u][k]).sum();
                        q[k] = (nf * nf * p[k] + 4.0 * e + d) / (nf * (nf + 5.0));
                    }
                    q
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube() -> Cage {
        let points = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 1.0], [1.0, 1.0, 1.0], [0.0, 1.0, 1.0]];
        let faces = vec![vec![0, 3, 2, 1], vec![4, 5, 6, 7], vec![0, 1, 5, 4], vec![1, 2, 6, 5], vec![2, 3, 7, 6], vec![3, 0, 4, 7]];
        let mut edges = Vec::new();
        for f in &faces {
            for j in 0..4 {
                let e = key(f[j], f[(j + 1) % 4]);
                if !edges.iter().any(|(x, _): &([usize; 2], u8)| key(x[0], x[1]) == e) {
                    edges.push(([e.0, e.1], SMOOTH));
                }
            }
        }
        Cage { points, vertex_tags: vec![SMOOTH; 8], edges, faces }
    }

    /// A smooth cube's cage becomes a rounded blob inside it, symmetric,
    /// closed, every face divided into sixty-four.
    #[test]
    fn a_smooth_cube_rounds_off_inside_its_cage() {
        let mesh = surface(&cube(), 3);
        assert_eq!(mesh.triangles.len(), 6 * 64 * 2);
        let centre = [0.5; 3];
        let distances: Vec<f64> = mesh.positions.iter().map(|p| (0..3).map(|k| (p[k] - centre[k]).powi(2)).sum::<f64>().sqrt()).collect();
        let (lo, hi) = distances.iter().fold((f64::MAX, 0.0f64), |(a, b), d| (a.min(*d), b.max(*d)));
        assert!(hi < 0.5 * 3f64.sqrt() && lo > 0.25, "between {lo} and {hi}");
        assert!(mesh.positions.iter().all(|p| p.iter().all(|c| *c > 0.0 && *c < 1.0)), "everything stays inside the cage");
    }

    /// With every edge a crease the cage is its own surface: flat faces,
    /// sharp corners.
    #[test]
    fn creased_edges_keep_the_cage_square() {
        let mut cage = cube();
        for e in &mut cage.edges {
            e.1 = CREASE;
        }
        let mesh = surface(&cage, 2);
        assert!(mesh.positions.iter().all(|p| p.iter().any(|c| c.abs() < 1e-9 || (c - 1.0).abs() < 1e-9)), "every point on the cube's faces");
        assert!(mesh.positions.iter().any(|p| p.iter().all(|c| c.abs() < 1e-9)), "the corner stays");
    }
}
