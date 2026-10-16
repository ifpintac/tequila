//! The geometry store, and instancing by content hash.
//!
//! `docs/03-SCENE-AND-IO.md` §2.2 is emphatic about why this matters:
//!
//! > A CAD assembly with 4,000 identical M3 screws must load as one mesh and
//! > 4,000 transforms; the difference is 3 seconds versus 3 minutes and 200 MB
//! > versus 8 GB.
//!
//! And §6.4 explains why detection cannot be left to the exporter: "many CAD
//! exporters write repeated geometry as distinct meshes and rely on the
//! consumer to detect it".

use crate::tree::MeshKey;
use tequila_core::math::{vec3, vec4, Aabb, Onb, Vec2, Vec3, Vec4};
use std::collections::BTreeMap;

/// A face group with its own material slot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SubMesh {
    pub first_index: u32,
    pub index_count: u32,
    pub slot: u32,
}

/// An indexed triangle mesh.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub positions: Vec<Vec3>,
    /// Empty means "no authored normals". §6.1 point 3 is explicit that these
    /// must never be overwritten when present: "CAD tessellation carries
    /// correct normals and regenerating them makes cylinders faceted".
    pub normals: Vec<Vec3>,
    /// Multiple UV sets, in channel order.
    pub uv_sets: Vec<Vec<Vec2>>,
    /// Per-vertex colours, linear RGBA (glTF's COLOR_0, a PLY's red, green
    /// and blue). Empty when the file carried none.
    pub colors: Vec<Vec4>,
    /// Tangent frames, `xyz` the tangent and `w` the bitangent sign. Empty
    /// when the mesh has no UVs, since a tangent frame is defined by the UV
    /// parameterization and means nothing without one.
    pub tangents: Vec<Vec4>,
    pub indices: Vec<u32>,
    pub submeshes: Vec<SubMesh>,
    /// The model's own edges, as segment end points in pairs, in the mesh's
    /// space: a CAD part's B-rep edges, where one face meets another, which
    /// the Solid view draws as lines. Empty for a mesh whose file had none,
    /// whose edges are found from its creases instead.
    pub edge_lines: Vec<Vec3>,
    bounds: Aabb,
}

impl Mesh {
    pub fn new(positions: Vec<Vec3>, indices: Vec<u32>) -> Self {
        let mut mesh = Mesh {
            positions,
            indices,
            ..Default::default()
        };
        mesh.recompute_bounds();
        mesh
    }

    pub fn with_normals(mut self, normals: Vec<Vec3>) -> Self {
        self.normals = normals;
        self
    }

    pub fn with_uvs(mut self, uvs: Vec<Vec2>) -> Self {
        self.uv_sets = vec![uvs];
        self
    }

    pub fn recompute_bounds(&mut self) {
        let mut bounds = Aabb::EMPTY;
        for p in &self.positions {
            bounds.extend_point(*p);
        }
        self.bounds = bounds;
    }

    #[inline]
    pub fn bounds(&self) -> Aabb {
        self.bounds
    }

    #[inline]
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    #[inline]
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    #[inline]
    pub fn triangle(&self, i: usize) -> [u32; 3] {
        [
            self.indices[i * 3],
            self.indices[i * 3 + 1],
            self.indices[i * 3 + 2],
        ]
    }

    /// Geometric normal of a triangle, scaled by twice its area.
    pub fn geometric_normal_scaled(&self, i: usize) -> Vec3 {
        let [a, b, c] = self.triangle(i);
        let (p0, p1, p2) = (
            self.positions[a as usize],
            self.positions[b as usize],
            self.positions[c as usize],
        );
        (p1 - p0).cross(p2 - p0)
    }

    /// Content hash for instancing detection (§6.4).
    ///
    /// **Topology only** — vertex count, index buffer, and which attribute
    /// channels exist. Deliberately not the positions.
    ///
    /// Hashing positions cannot work in `f32`. Two copies of a part placed at
    /// different points in an assembly do not have bit-identical
    /// centroid-relative coordinates: subtracting a centroid of magnitude
    /// 5,000 leaves about a thousandth of a unit of rounding error, so the two
    /// copies hash differently and the duplicate is missed — the exact case
    /// this exists to catch. Quantizing first only moves the problem to the
    /// bucket boundaries.
    ///
    /// The index buffer is a strong discriminator for CAD parts and is exactly
    /// stable, so it makes a good bucket key. §6.4 already requires confirming
    /// a hash match by comparing full buffers, so the hash only has to be
    /// cheap and never to *miss* a duplicate; letting it group a few extra
    /// candidates costs one comparison each.
    pub fn content_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.positions.len().hash(&mut hasher);
        self.indices.hash(&mut hasher);
        self.normals.len().hash(&mut hasher);
        self.tangents.len().hash(&mut hasher);
        self.uv_sets.len().hash(&mut hasher);
        for set in &self.uv_sets {
            set.len().hash(&mut hasher);
        }
        hasher.finish()
    }

    /// Largest absolute coordinate, which is what sets the `f32` precision
    /// available in this mesh's own numbers.
    fn coordinate_magnitude(&self) -> f32 {
        self.positions
            .iter()
            .map(|p| p.x.abs().max(p.y.abs()).max(p.z.abs()))
            .fold(0.0f32, f32::max)
    }

    /// Whether two meshes are the same shape up to a translation.
    ///
    /// §6.4: "Guard against false positives by comparing full buffers on hash
    /// match." A shared bucket is only a candidate; this is what decides, and
    /// two different parts silently rendering as one would be a wrong image.
    ///
    /// The tolerance accounts for both the shape's own size and how far it
    /// sits from the origin. A part 1.4 units across but positioned 5,000
    /// units out carries about a thousandth of a unit of rounding in its
    /// coordinates, so a tolerance derived from the shape alone would reject
    /// genuine duplicates.
    pub fn is_same_shape_as(&self, other: &Mesh) -> bool {
        // The same shape painted differently is two meshes, not one mesh
        // placed twice.
        if self.positions.len() != other.positions.len()
            || self.indices != other.indices
            || self.normals.len() != other.normals.len()
            || self.colors != other.colors
            || self.submeshes != other.submeshes
        {
            return false;
        }
        // Nor is the same shape mapped differently: keycaps sharing one
        // shape take each its own legend from an atlas, and placing the
        // first one's UVs on every key prints the same letter on all.
        if self.uv_sets.len() != other.uv_sets.len()
            || self.uv_sets.iter().zip(&other.uv_sets).any(|(a, b)| {
                a.len() != b.len() || a.iter().zip(b).any(|(p, q)| (p.x - q.x).abs() > 1e-6 || (p.y - q.y).abs() > 1e-6)
            })
        {
            return false;
        }
        let centroid = |m: &Mesh| -> Vec3 {
            if m.positions.is_empty() {
                return Vec3::ZERO;
            }
            // Summed in f64: adding thousands of large coordinates in f32
            // loses the low bits that distinguish the shapes.
            let (mut x, mut y, mut z) = (0.0f64, 0.0f64, 0.0f64);
            for p in &m.positions {
                x += p.x as f64;
                y += p.y as f64;
                z += p.z as f64;
            }
            let n = m.positions.len() as f64;
            vec3((x / n) as f32, (y / n) as f32, (z / n) as f32)
        };
        let (ca, cb) = (centroid(self), centroid(other));

        let shape_scale = self
            .bounds
            .diagonal()
            .length()
            .max(other.bounds.diagonal().length())
            .max(1e-6);
        let position_scale = self
            .coordinate_magnitude()
            .max(other.coordinate_magnitude());
        // Roughly a hundred ulps of the larger coordinate, or a hundred-
        // thousandth of the shape, whichever is looser.
        let tolerance = (shape_scale * 1e-5).max(position_scale * 1e-5).max(1e-6);

        for (a, b) in self.positions.iter().zip(&other.positions) {
            if ((*a - ca) - (*b - cb)).length() > tolerance {
                return false;
            }
        }
        for (a, b) in self.normals.iter().zip(&other.normals) {
            if (*a - *b).length() > 1e-4 {
                return false;
            }
        }
        true
    }

    /// Remove degenerate triangles and report non-finite positions (§6.1
    /// point 5).
    ///
    /// Returns the number of triangles dropped. Non-finite positions are an
    /// error rather than a repair: a NaN vertex poisons the BVH build and
    /// every ray that touches it, and silently substituting a value would hide
    /// a broken export.
    pub fn validate_and_clean(&mut self) -> Result<usize, String> {
        for (i, p) in self.positions.iter().enumerate() {
            if !p.is_finite() {
                return Err(format!("vertex {i} is not finite: {p:?}"));
            }
        }
        let vertex_count = self.positions.len() as u32;
        for (i, &index) in self.indices.iter().enumerate() {
            if index >= vertex_count {
                return Err(format!(
                    "index {i} refers to vertex {index}, but the mesh has {vertex_count}"
                ));
            }
        }
        if self.indices.len() % 3 != 0 {
            return Err(format!(
                "index count {} is not a multiple of three",
                self.indices.len()
            ));
        }

        let before = self.triangle_count();
        let mut kept = Vec::with_capacity(self.indices.len());
        for t in 0..before {
            let [a, b, c] = self.triangle(t);
            // Degenerate by topology or by area. Zero-area triangles have no
            // usable normal and give the BVH builder nothing to bound.
            if a == b || b == c || a == c {
                continue;
            }
            if self.geometric_normal_scaled(t).length() <= f32::MIN_POSITIVE {
                continue;
            }
            kept.extend_from_slice(&[a, b, c]);
        }
        let dropped = before - kept.len() / 3;
        self.indices = kept;
        Ok(dropped)
    }

    /// Merge vertices that are identical in every channel (§6.1 point 2).
    ///
    /// Returns the number of vertices removed. CAD tessellation is routinely
    /// fully unwelded — every triangle carries its own three vertices — and
    /// without this the mesh has three times the vertices it needs, no shared
    /// normals to smooth across, and a BVH built over triple the primitives.
    ///
    /// Identical means *bitwise* identical, not within a tolerance. Welding by
    /// proximity would collapse deliberately coincident surfaces (a seam, a
    /// zero-thickness flange) and is a different operation with a different
    /// failure mode; this one can only ever be a lossless compaction. Signed
    /// zero is normalized first, because exporters emit both and `-0.0 == 0.0`
    /// while their bits differ.
    pub fn join_identical_vertices(&mut self) -> usize {
        use std::collections::HashMap;

        let before = self.positions.len();
        if before == 0 {
            return 0;
        }

        // A vertex's full signature: position, normal if present, and every UV
        // channel. Tangents are deliberately excluded — they are derived from
        // the others, and are regenerated after welding anyway.
        let mut key = Vec::with_capacity(3 + 3 + 2 * self.uv_sets.len());
        let mut seen: HashMap<Vec<u32>, u32> = HashMap::with_capacity(before);
        let mut remap = vec![0u32; before];
        let mut positions = Vec::with_capacity(before);
        let mut normals = Vec::with_capacity(self.normals.len());
        let mut uv_sets: Vec<Vec<Vec2>> = self.uv_sets.iter().map(|_| Vec::new()).collect();
        let mut colors = Vec::with_capacity(self.colors.len());

        for (v, slot) in remap.iter_mut().enumerate() {
            key.clear();
            let p = self.positions[v];
            key.extend_from_slice(&[bits(p.x), bits(p.y), bits(p.z)]);
            if let Some(n) = self.normals.get(v) {
                key.extend_from_slice(&[bits(n.x), bits(n.y), bits(n.z)]);
            }
            for set in &self.uv_sets {
                if let Some(uv) = set.get(v) {
                    key.extend_from_slice(&[bits(uv.x), bits(uv.y)]);
                }
            }
            // Two vertices of different colours are two vertices.
            if let Some(c) = self.colors.get(v) {
                key.extend_from_slice(&[bits(c.x), bits(c.y), bits(c.z), bits(c.w)]);
            }

            match seen.get(&key) {
                Some(&existing) => *slot = existing,
                None => {
                    let new_index = positions.len() as u32;
                    seen.insert(key.clone(), new_index);
                    *slot = new_index;
                    positions.push(p);
                    if let Some(n) = self.normals.get(v) {
                        normals.push(*n);
                    }
                    for (channel, set) in self.uv_sets.iter().enumerate() {
                        if let Some(uv) = set.get(v) {
                            uv_sets[channel].push(*uv);
                        }
                    }
                    if let Some(c) = self.colors.get(v) {
                        colors.push(*c);
                    }
                }
            }
        }

        for index in &mut self.indices {
            *index = remap[*index as usize];
        }
        self.positions = positions;
        self.normals = normals;
        self.uv_sets = uv_sets;
        if colors.len() == self.positions.len() {
            self.colors = colors;
        } else {
            self.colors.clear();
        }
        // Derived from the old vertex order, so no longer meaningful.
        self.tangents.clear();
        before - self.positions.len()
    }

    /// Generate smooth vertex normals with a smoothing angle (§6.1 point 3).
    ///
    /// Only ever called when normals are **absent**. §6.1 is explicit that
    /// authored normals must never be overwritten: "CAD tessellation carries
    /// correct normals and regenerating them makes cylinders faceted".
    ///
    /// A corner averages only those incident faces within `smoothing_angle` of
    /// its own face normal, and vertices whose corners disagree are split. The
    /// angle is what separates a cylinder from a cube: averaging across every
    /// incident face unconditionally rounds off the cube's edges, and averaging
    /// across none facets the cylinder. Area weighting stops a cluster of tiny
    /// triangles from outvoting a large one, which is exactly the configuration
    /// a tessellator produces along a fillet.
    ///
    /// Vertex count can grow, so this must run before anything that records
    /// vertex indices.
    pub fn generate_normals(&mut self, smoothing_angle: f32) {
        let triangles = self.triangle_count();
        if triangles == 0 {
            self.normals = vec![Vec3::Y; self.positions.len()];
            return;
        }
        let cos_limit = smoothing_angle.clamp(0.0, std::f32::consts::PI).cos();

        // Area-scaled, so the accumulation is area-weighted for free; the unit
        // normal is only needed for the angle test.
        let scaled: Vec<Vec3> = (0..triangles)
            .map(|t| self.geometric_normal_scaled(t))
            .collect();
        let unit: Vec<Vec3> = scaled
            .iter()
            .map(|n| n.try_normalize().unwrap_or(Vec3::Y))
            .collect();

        let mut incident: Vec<Vec<u32>> = vec![Vec::new(); self.positions.len()];
        for t in 0..triangles {
            for &v in &self.triangle(t) {
                incident[v as usize].push(t as u32);
            }
        }

        // One normal per *corner*, then corners that agree collapse back into
        // a shared vertex.
        let mut corner_normals = vec![Vec3::Y; self.indices.len()];
        for t in 0..triangles {
            for (k, &v) in self.triangle(t).iter().enumerate() {
                let mut accumulated = Vec3::ZERO;
                for &other in &incident[v as usize] {
                    if unit[other as usize].dot(unit[t]) >= cos_limit {
                        accumulated += scaled[other as usize];
                    }
                }
                corner_normals[t * 3 + k] = accumulated.try_normalize().unwrap_or(unit[t]);
            }
        }

        // Rebuild, splitting a vertex once per distinct normal. Compared by
        // dot product rather than by bits: two corners of the same smoothing
        // group accumulate the same faces in the same order and so agree
        // exactly, but the comparison should not depend on that.
        let mut variants: Vec<Vec<(Vec3, u32)>> = vec![Vec::new(); self.positions.len()];
        let mut positions = Vec::with_capacity(self.positions.len());
        let mut normals: Vec<Vec3> = Vec::with_capacity(self.positions.len());
        let mut uv_sets: Vec<Vec<Vec2>> = self.uv_sets.iter().map(|_| Vec::new()).collect();
        let mut colors = Vec::with_capacity(self.colors.len());
        let mut indices = Vec::with_capacity(self.indices.len());

        for (&index, &normal) in self.indices.iter().zip(&corner_normals) {
            let original = index as usize;
            let existing = variants[original]
                .iter()
                .find(|(n, _)| n.dot(normal) > 0.999_9)
                .map(|(_, index)| *index);
            let index = match existing {
                Some(index) => index,
                None => {
                    let index = positions.len() as u32;
                    positions.push(self.positions[original]);
                    normals.push(normal);
                    for (channel, set) in self.uv_sets.iter().enumerate() {
                        if let Some(uv) = set.get(original) {
                            uv_sets[channel].push(*uv);
                        }
                    }
                    if let Some(c) = self.colors.get(original) {
                        colors.push(*c);
                    }
                    variants[original].push((normal, index));
                    index
                }
            };
            indices.push(index);
        }

        self.positions = positions;
        self.normals = normals;
        self.uv_sets = uv_sets;
        if colors.len() == self.positions.len() {
            self.colors = colors;
        } else {
            self.colors.clear();
        }
        self.indices = indices;
        self.tangents.clear();
        self.recompute_bounds();
    }

    /// Generate per-vertex tangent frames from UV channel 0 (§6.1 point 4),
    /// by MikkTSpace ([`tangent_frames`]).
    ///
    /// Returns false, leaving the mesh untouched, when there are no UVs: a
    /// tangent frame is defined by the UV parameterization, and inventing one
    /// without it would put an arbitrary rotation into every normal-mapped
    /// surface.
    pub fn generate_tangents(&mut self) -> bool {
        let Some(uvs) = self.uv_sets.first() else {
            return false;
        };
        match tangent_frames(&self.positions, &self.normals, uvs, &self.indices) {
            Some(tangents) => {
                self.tangents = tangents;
                true
            }
            None => false,
        }
    }
}

/// Per-vertex tangent frames from positions, normals, UV channel 0 and the
/// triangle list, as [`Mesh::generate_tangents`] stores them: `xyz` along u,
/// `w` the handedness in glTF's convention. `None` when the arrays do not
/// line up.
///
/// MikkTSpace (§6.1 point 4): the tangent space Blender, Substance and every
/// other baker bake normal maps in, so a baked map shows no seam where its
/// UV islands meet or mirror. The accumulation it replaced agreed on smooth
/// surfaces and parted from it exactly there. It is still the fallback, for
/// a mesh MikkTSpace will not take.
///
/// A function of the arrays rather than of a mesh so the renderer can call
/// it on the mesh it builds — tangents saved in a project by an earlier
/// version used the opposite handedness, and regenerating is what makes an
/// old file render right.
pub fn tangent_frames(
    positions: &[Vec3],
    normals: &[Vec3],
    uvs: &[tequila_core::math::Vec2],
    indices: &[u32],
) -> Option<Vec<Vec4>> {
    if uvs.len() != positions.len() || normals.len() != positions.len() {
        return None;
    }
    mikk_frames(positions, normals, uvs, indices).or_else(|| accumulated_frames(positions, normals, uvs, indices))
}

/// MikkTSpace's frames, or `None` where it declines the mesh or gives back
/// something that is not a frame.
fn mikk_frames(positions: &[Vec3], normals: &[Vec3], uvs: &[tequila_core::math::Vec2], indices: &[u32]) -> Option<Vec<Vec4>> {
    struct Geometry<'a> {
        positions: &'a [Vec3],
        normals: &'a [Vec3],
        uvs: &'a [tequila_core::math::Vec2],
        indices: &'a [u32],
        tangents: Vec<Option<Vec4>>,
    }
    impl Geometry<'_> {
        fn vertex(&self, face: usize, vert: usize) -> usize {
            self.indices[face * 3 + vert] as usize
        }
    }
    impl mikktspace::Geometry for Geometry<'_> {
        fn num_faces(&self) -> usize {
            self.indices.len() / 3
        }
        fn num_vertices_of_face(&self, _face: usize) -> usize {
            3
        }
        fn position(&self, face: usize, vert: usize) -> [f32; 3] {
            let p = self.positions[self.vertex(face, vert)];
            [p.x, p.y, p.z]
        }
        fn normal(&self, face: usize, vert: usize) -> [f32; 3] {
            let n = self.normals[self.vertex(face, vert)];
            [n.x, n.y, n.z]
        }
        // UVs are stored as glTF stores them, v = 0 at the top row; the
        // bakers' MikkTSpace has v = 0 at the bottom. Turned up that way,
        // its bitangent is up the image - which is what `w * cross(n, t)`
        // means in glTF - and its tangents are the bakers' own.
        fn tex_coord(&self, face: usize, vert: usize) -> [f32; 2] {
            let uv = self.uvs[self.vertex(face, vert)];
            [uv.x, 1.0 - uv.y]
        }
        fn set_tangent_encoded(&mut self, tangent: [f32; 4], face: usize, vert: usize) {
            let v = self.vertex(face, vert);
            self.tangents[v] = Some(vec4(tangent[0], tangent[1], tangent[2], if tangent[3] < 0.0 { -1.0 } else { 1.0 }));
        }
    }
    if indices.len() < 3 || indices.iter().any(|&i| i as usize >= positions.len()) {
        return None;
    }
    let mut geometry = Geometry { positions, normals, uvs, indices, tangents: vec![None; positions.len()] };
    if !mikktspace::generate_tangents(&mut geometry) {
        return None;
    }
    let frames: Vec<Vec4> = geometry
        .tangents
        .iter()
        .enumerate()
        .map(|(v, t)| match t {
            Some(t) if t.x.is_finite() && t.y.is_finite() && t.z.is_finite() && (t.x * t.x + t.y * t.y + t.z * t.z) > 1e-12 => *t,
            // A vertex no triangle uses, or one MikkTSpace could give no
            // direction: any frame about its normal.
            _ => {
                let o = Onb::from_normal(normals[v]);
                vec4(o.t.x, o.t.y, o.t.z, 1.0)
            }
        })
        .collect();
    Some(frames)
}

/// The per-triangle accumulation, with Gram-Schmidt: MikkTSpace's fallback.
fn accumulated_frames(positions: &[Vec3], normals: &[Vec3], uvs: &[tequila_core::math::Vec2], indices: &[u32]) -> Option<Vec<Vec4>> {

    let mut tangent = vec![Vec3::ZERO; positions.len()];
    let mut bitangent = vec![Vec3::ZERO; positions.len()];

    for triangle in indices.chunks_exact(3) {
        let [a, b, c] = [triangle[0] as usize, triangle[1] as usize, triangle[2] as usize];
        if a >= positions.len() || b >= positions.len() || c >= positions.len() {
            continue;
        }
        let (p0, p1, p2) = (positions[a], positions[b], positions[c]);
        let (w0, w1, w2) = (uvs[a], uvs[b], uvs[c]);

        let (e1, e2) = (p1 - p0, p2 - p0);
        let (du1, dv1) = (w1.x - w0.x, w1.y - w0.y);
        let (du2, dv2) = (w2.x - w0.x, w2.y - w0.y);

        // A degenerate UV triangle carries no direction to contribute; the
        // vertex falls back to a normal-derived frame below.
        let determinant = du1 * dv2 - du2 * dv1;
        if determinant.abs() <= f32::MIN_POSITIVE {
            continue;
        }
        let r = 1.0 / determinant;
        let t_dir = (e1 * dv2 - e2 * dv1) * r;
        let b_dir = (e2 * du1 - e1 * du2) * r;
        for &v in &[a, b, c] {
            tangent[v] += t_dir;
            bitangent[v] += b_dir;
        }
    }

    let frames = (0..positions.len())
        .map(|v| {
            let n = normals[v];
            // Gram-Schmidt: the tangent must be perpendicular to the
            // shading normal, which the accumulation does not guarantee.
            let t = (tangent[v] - n * n.dot(tangent[v]))
                .try_normalize()
                .unwrap_or_else(|| Onb::from_normal(n).t);
            // Handedness, so mirrored UV islands do not invert the frame —
            // in glTF's convention: `w * cross(n, t)` is the bitangent a
            // normal map's green channel means, which is *up the image*.
            // UVs are stored as glTF stores them, v = 0 at the top row,
            // so up is towards decreasing v: the opposite of `dP/dv`.
            let sign = if n.cross(t).dot(bitangent[v]) < 0.0 {
                1.0
            } else {
                -1.0
            };
            vec4(t.x, t.y, t.z, sign)
        })
        .collect();
    Some(frames)
}

/// Mean curvature at each vertex, in inverse units of the positions:
/// positive where the surface is convex (an edge, the outside of a fillet),
/// negative where it is concave (a crease, the inside of a groove), zero where
/// it is flat. About `1 / r` on a sphere of radius `r`.
///
/// §6.2's cheap and robust estimate: how fast the normal turns along each
/// edge, `(n_b - n_a) . (p_b - p_a) / |p_b - p_a|^2`, averaged over the edges
/// at a vertex. Vertices at the same position are welded first, with their
/// normals averaged, so a hard edge — split into two vertices with two
/// normals, which is how every CAD tessellation and most game meshes store
/// one — reads as the sharp convex edge it is rather than as two flat faces
/// that happen to touch. `normals` may be empty; face normals stand in.
pub fn vertex_curvature(positions: &[Vec3], normals: &[Vec3], indices: &[u32]) -> Vec<f32> {
    use std::collections::HashMap;

    let count = positions.len();
    if count == 0 {
        return Vec::new();
    }
    // Weld by exact position.
    let mut group_of = vec![0usize; count];
    let mut groups: HashMap<[u32; 3], usize> = HashMap::new();
    let mut centres: Vec<Vec3> = Vec::new();
    for (i, p) in positions.iter().enumerate() {
        let key = [bits(p.x), bits(p.y), bits(p.z)];
        let next = centres.len();
        let g = *groups.entry(key).or_insert(next);
        if g == next {
            centres.push(*p);
        }
        group_of[i] = g;
    }
    // A normal per welded point: the vertex normals averaged, or the face
    // normals when the mesh has none.
    let mut normal = vec![Vec3::ZERO; centres.len()];
    if normals.len() == count {
        for (i, n) in normals.iter().enumerate() {
            normal[group_of[i]] += *n;
        }
    } else {
        for triangle in indices.chunks_exact(3) {
            let [a, b, c] = [triangle[0] as usize, triangle[1] as usize, triangle[2] as usize];
            if a >= count || b >= count || c >= count {
                continue;
            }
            let face = (positions[b] - positions[a]).cross(positions[c] - positions[a]);
            for v in [a, b, c] {
                normal[group_of[v]] += face;
            }
        }
    }
    let normal: Vec<Vec3> = normal.into_iter().map(|n| n.try_normalize().unwrap_or(Vec3::ZERO)).collect();

    let mut sum = vec![0.0f32; centres.len()];
    let mut edges = vec![0u32; centres.len()];
    for triangle in indices.chunks_exact(3) {
        for (x, y) in [(0, 1), (1, 2), (2, 0)] {
            let (a, b) = (triangle[x] as usize, triangle[y] as usize);
            if a >= count || b >= count {
                continue;
            }
            let (ga, gb) = (group_of[a], group_of[b]);
            if ga == gb {
                continue;
            }
            let d = centres[gb] - centres[ga];
            let length2 = d.dot(d);
            if length2 <= 0.0 {
                continue;
            }
            let k = (normal[gb] - normal[ga]).dot(d) / length2;
            // Each interior edge is seen from both of its triangles, which
            // weights every edge alike; boundary edges once, which is fine.
            sum[ga] += k;
            sum[gb] += k;
            edges[ga] += 1;
            edges[gb] += 1;
        }
    }
    (0..count)
        .map(|i| {
            let g = group_of[i];
            if edges[g] == 0 {
                0.0
            } else {
                sum[g] / edges[g] as f32
            }
        })
        .collect()
}

/// `f32` bits with signed zero normalized, for exact-equality bucketing.
///
/// `-0.0 == 0.0` but their bit patterns differ, and exporters emit both, so
/// hashing the raw bits would treat two identical vertices as distinct.
#[inline]
fn bits(x: f32) -> u32 {
    if x == 0.0 {
        0
    } else {
        x.to_bits()
    }
}

/// Deduplicated mesh storage.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GeometryStore {
    meshes: BTreeMap<MeshKey, Mesh>,
    next_key: u64,
}

impl GeometryStore {
    pub fn new() -> Self {
        GeometryStore {
            meshes: BTreeMap::new(),
            next_key: 1,
        }
    }

    pub fn len(&self) -> usize {
        self.meshes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.meshes.is_empty()
    }

    pub fn get(&self, key: MeshKey) -> Option<&Mesh> {
        self.meshes.get(&key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (MeshKey, &Mesh)> {
        self.meshes.iter().map(|(k, m)| (*k, m))
    }

    pub fn keys(&self) -> impl Iterator<Item = MeshKey> + '_ {
        self.meshes.keys().copied()
    }

    pub fn next_key(&self) -> u64 {
        self.next_key
    }

    pub fn set_next_key(&mut self, next: u64) {
        self.next_key = self.next_key.max(next);
    }

    /// Store a mesh without deduplicating. Used by the loader, which is
    /// restoring keys that already exist in the file.
    pub fn insert_with_key(&mut self, key: MeshKey, mesh: Mesh) {
        self.next_key = self.next_key.max(key.0 + 1);
        self.meshes.insert(key, mesh);
    }

    /// Store a mesh, returning an existing key if an identical shape is
    /// already present.
    ///
    /// This is where the 4,000-screw assembly collapses to one mesh. The hash
    /// narrows the candidates; a full buffer comparison then confirms, because
    /// two distinct parts rendering as one would be a silently wrong image.
    pub fn insert_deduplicated(&mut self, mesh: Mesh) -> (MeshKey, bool) {
        let hash = mesh.content_hash();
        for (&key, existing) in &self.meshes {
            if existing.content_hash() == hash && existing.is_same_shape_as(&mesh) {
                return (key, true);
            }
        }
        let key = MeshKey(self.next_key);
        self.next_key += 1;
        self.meshes.insert(key, mesh);
        (key, false)
    }

    /// Drop meshes no longer referenced by any node.
    pub fn retain_keys(&mut self, keep: &dyn Fn(MeshKey) -> bool) -> usize {
        let before = self.meshes.len();
        self.meshes.retain(|&key, _| keep(key));
        before - self.meshes.len()
    }

    pub fn total_triangles(&self) -> usize {
        self.meshes.values().map(|m| m.triangle_count()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::{vec2, vec3, vec4};

    fn triangle(offset: Vec3) -> Mesh {
        Mesh::new(
            vec![
                offset,
                offset + vec3(1.0, 0.0, 0.0),
                offset + vec3(0.0, 1.0, 0.0),
            ],
            vec![0, 1, 2],
        )
    }

    /// The headline case of §2.2: the same part placed many times collapses to
    /// one mesh.
    #[test]
    fn identical_shapes_at_different_places_share_one_mesh() {
        let mut store = GeometryStore::new();
        let (first, reused) = store.insert_deduplicated(triangle(Vec3::ZERO));
        assert!(!reused);
        for i in 1..500 {
            let (key, reused) =
                store.insert_deduplicated(triangle(vec3(i as f32 * 10.0, 0.0, 0.0)));
            assert!(reused, "copy {i} was not recognized");
            assert_eq!(key, first);
        }
        assert_eq!(store.len(), 1, "4000 screws must not become 4000 meshes");
    }

    #[test]
    fn different_shapes_stay_distinct() {
        let mut store = GeometryStore::new();
        let (a, _) = store.insert_deduplicated(triangle(Vec3::ZERO));
        let bigger = Mesh::new(
            vec![Vec3::ZERO, vec3(2.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)],
            vec![0, 1, 2],
        );
        let (b, reused) = store.insert_deduplicated(bigger);
        assert!(!reused);
        assert_ne!(a, b);
        assert_eq!(store.len(), 2);
    }

    /// Same vertices, different winding: a different shape, and treating it as
    /// a duplicate would flip the part's normals.
    #[test]
    fn winding_differences_are_not_duplicates() {
        let mut store = GeometryStore::new();
        store.insert_deduplicated(triangle(Vec3::ZERO));
        let flipped = Mesh::new(
            vec![Vec3::ZERO, vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)],
            vec![0, 2, 1],
        );
        let (_, reused) = store.insert_deduplicated(flipped);
        assert!(!reused, "reversed winding must not deduplicate");
    }

    /// Differing authored normals mean differing shading, even on identical
    /// positions — a faceted and a smoothed copy are not interchangeable.
    #[test]
    fn differing_normals_are_not_duplicates() {
        let mut store = GeometryStore::new();
        store.insert_deduplicated(triangle(Vec3::ZERO).with_normals(vec![Vec3::Z; 3]));
        let (_, reused) =
            store.insert_deduplicated(triangle(Vec3::ZERO).with_normals(vec![Vec3::Y; 3]));
        assert!(!reused);
    }

    /// Keycaps of one shape with their legends at different places in an
    /// atlas are different meshes: the UVs are part of what is placed.
    #[test]
    fn differing_uvs_are_not_duplicates() {
        let uvs = |u: f32| vec![vec![vec2(u, 0.0), vec2(u + 0.1, 0.0), vec2(u, 0.1)]];
        let mut a = triangle(Vec3::ZERO);
        a.uv_sets = uvs(0.0);
        let mut b = triangle(vec3(2.0, 0.0, 0.0));
        b.uv_sets = uvs(0.5);
        assert!(!a.is_same_shape_as(&b));
        let mut c = triangle(vec3(4.0, 0.0, 0.0));
        c.uv_sets = uvs(0.0);
        assert!(a.is_same_shape_as(&c), "the same mapping is still the same mesh");
    }

    /// The hash groups candidates; `is_same_shape_as` decides. Translation
    /// must not separate two copies at either stage.
    #[test]
    fn translation_alone_is_not_a_difference() {
        let a = triangle(Vec3::ZERO);
        for offset in [
            vec3(1000.0, -50.0, 7.5),
            vec3(50_000.0, 0.0, 0.0),
            vec3(-0.001, 0.002, 0.0),
        ] {
            let b = triangle(offset);
            assert_eq!(a.content_hash(), b.content_hash(), "offset {offset:?}");
            assert!(a.is_same_shape_as(&b), "offset {offset:?}");
        }

        // Rotation is a genuine difference, and the comparison must catch it
        // even though the topology — and so the hash — is identical.
        let rotated = Mesh::new(
            vec![Vec3::ZERO, vec3(0.0, 1.0, 0.0), vec3(-1.0, 0.0, 0.0)],
            vec![0, 1, 2],
        );
        assert_eq!(a.content_hash(), rotated.content_hash());
        assert!(!a.is_same_shape_as(&rotated));
    }

    /// Signed zero and plain zero are equal but have different bit patterns,
    /// and exporters emit both. Neither stage may treat them as different.
    #[test]
    fn signed_zero_is_not_a_difference() {
        let a = Mesh::new(
            vec![
                vec3(0.0, 0.0, 0.0),
                vec3(1.0, 0.0, 0.0),
                vec3(0.0, 1.0, 0.0),
            ],
            vec![0, 1, 2],
        );
        let b = Mesh::new(
            vec![
                vec3(-0.0, -0.0, 0.0),
                vec3(1.0, -0.0, 0.0),
                vec3(0.0, 1.0, -0.0),
            ],
            vec![0, 1, 2],
        );
        assert_eq!(a.content_hash(), b.content_hash());
        assert!(a.is_same_shape_as(&b));
    }

    /// An unwelded cube: 12 triangles, 36 vertices, every one its own. This is
    /// what CAD tessellation and STL actually deliver.
    fn unwelded_cube() -> Mesh {
        let c = [
            vec3(-1.0, -1.0, -1.0),
            vec3(1.0, -1.0, -1.0),
            vec3(1.0, 1.0, -1.0),
            vec3(-1.0, 1.0, -1.0),
            vec3(-1.0, -1.0, 1.0),
            vec3(1.0, -1.0, 1.0),
            vec3(1.0, 1.0, 1.0),
            vec3(-1.0, 1.0, 1.0),
        ];
        // Counter-clockwise seen from outside, so the geometric normals face
        // outward — the winding convention the renderer relies on.
        let faces = [
            [0usize, 3, 2, 1], // -Z
            [4, 5, 6, 7],      // +Z
            [0, 1, 5, 4],      // -Y
            [3, 7, 6, 2],      // +Y
            [0, 4, 7, 3],      // -X
            [1, 2, 6, 5],      // +X
        ];
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        for face in faces {
            for [a, b, cc] in [[0usize, 1, 2], [0, 2, 3]] {
                for k in [a, b, cc] {
                    indices.push(positions.len() as u32);
                    positions.push(c[face[k]]);
                }
            }
        }
        Mesh::new(positions, indices)
    }

    #[test]
    fn joining_identical_vertices_welds_a_cad_style_mesh() {
        let mut mesh = unwelded_cube();
        assert_eq!(mesh.vertex_count(), 36);

        let removed = mesh.join_identical_vertices();

        assert_eq!(removed, 28);
        assert_eq!(mesh.vertex_count(), 8, "a cube has eight distinct corners");
        assert_eq!(mesh.triangle_count(), 12, "no triangle may be lost");
        mesh.validate_and_clean()
            .expect("welding kept the mesh valid");
    }

    /// Welding must not merge vertices that differ in a channel other than
    /// position — two sides of a UV seam sit at the same point and must stay
    /// separate, or the texture wraps across the seam.
    #[test]
    fn joining_keeps_vertices_that_differ_in_uv() {
        let positions = vec![
            Vec3::ZERO,
            vec3(1.0, 0.0, 0.0),
            vec3(0.0, 1.0, 0.0),
            Vec3::ZERO,
        ];
        let uvs = vec![
            vec2(0.0, 0.0),
            vec2(1.0, 0.0),
            vec2(0.0, 1.0),
            vec2(1.0, 1.0), // same position, far side of the seam
        ];
        let mut mesh = Mesh::new(positions, vec![0, 1, 2, 3, 1, 2]).with_uvs(uvs);

        assert_eq!(mesh.join_identical_vertices(), 0);
        assert_eq!(mesh.vertex_count(), 4);
    }

    /// The whole point of the smoothing angle. A cube's corners must keep one
    /// normal per face; averaging across the 90° edges rounds it off.
    #[test]
    fn the_smoothing_angle_keeps_a_cube_faceted() {
        let mut mesh = unwelded_cube();
        mesh.join_identical_vertices();
        mesh.generate_normals(30f32.to_radians());

        // Each corner is split three ways, once per face meeting there.
        assert_eq!(mesh.vertex_count(), 24, "8 corners x 3 faces");

        // And every normal is an axis direction, not a corner-ward average.
        for n in &mesh.normals {
            let largest = n.x.abs().max(n.y.abs()).max(n.z.abs());
            assert!(
                (largest - 1.0).abs() < 1e-4,
                "normal {n:?} is an average across the edge, not a face normal"
            );
        }
    }

    /// And the converse: a smooth fan must stay smooth, or every tessellated
    /// cylinder comes out faceted.
    #[test]
    fn the_smoothing_angle_keeps_a_shallow_fan_smooth() {
        // A 32-sided cylinder wall: adjacent facets meet at about 11°, well
        // inside the 30° default.
        let sides = 32;
        let mut positions = Vec::new();
        for i in 0..sides {
            let a = i as f32 / sides as f32 * std::f32::consts::TAU;
            positions.push(vec3(a.cos(), -1.0, a.sin()));
            positions.push(vec3(a.cos(), 1.0, a.sin()));
        }
        let mut indices = Vec::new();
        for i in 0..sides {
            let (b, n) = (i * 2, ((i + 1) % sides) * 2);
            // Counter-clockwise seen from outside, so the geometric normals
            // point away from the axis.
            indices.extend_from_slice(&[b as u32, (n + 1) as u32, n as u32]);
            indices.extend_from_slice(&[b as u32, (b + 1) as u32, (n + 1) as u32]);
        }
        let mut mesh = Mesh::new(positions, indices);
        let before = mesh.vertex_count();
        mesh.generate_normals(30f32.to_radians());

        assert_eq!(
            mesh.vertex_count(),
            before,
            "the wall was split, which is what makes a cylinder look faceted"
        );
        for (v, n) in mesh.normals.iter().enumerate() {
            let p = mesh.positions[v];
            let radial = vec3(p.x, 0.0, p.z).try_normalize().unwrap();
            assert!(
                n.dot(radial) > 0.99,
                "normal {n:?} does not point radially outward"
            );
        }
    }

    /// A vertex's tangent is MikkTSpace's: the directions of the faces
    /// around it averaged by the angle each makes there, not by how
    /// stretched each face's UVs are. Two faces meet at a vertex, each with
    /// a right angle there: one runs along +x with its UVs squeezed a
    /// thousandfold, the other diagonally at 45 degrees. MikkTSpace splits
    /// the difference at 22.5; the accumulation this replaced followed the
    /// squeezed face almost entirely - the seam a baked normal map shows.
    #[test]
    fn tangents_are_mikktspace() {
        use tequila_core::math::vec2;
        let positions = vec![vec3(0.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(-1.0, 0.0, 0.0)];
        let normals = vec![Vec3::Z; 4];
        let uvs = vec![vec2(0.0, 0.0), vec2(0.001, 0.0), vec2(0.0, 1.0), vec2(-1.0, 1.0)];
        let indices = vec![0, 1, 2, 0, 2, 3];
        let frames = tangent_frames(&positions, &normals, &uvs, &indices).expect("frames");
        let t = frames[0];
        let degrees = t.y.atan2(t.x).to_degrees();
        assert!(
            (degrees - 22.5).abs() < 4.0,
            "the shared vertex's tangent should lie between the faces' at 22.5 degrees; it is at {degrees:.1}"
        );
    }

    #[test]
    fn tangents_need_uvs_and_follow_the_u_direction() {
        // A quad in the XY plane with U along +X.
        let positions = vec![
            Vec3::ZERO,
            vec3(1.0, 0.0, 0.0),
            vec3(1.0, 1.0, 0.0),
            vec3(0.0, 1.0, 0.0),
        ];
        let mut mesh = Mesh::new(positions, vec![0, 1, 2, 0, 2, 3]);
        mesh.generate_normals(30f32.to_radians());

        assert!(!mesh.generate_tangents(), "no UVs means no tangent frame");
        assert!(mesh.tangents.is_empty());

        mesh.uv_sets = vec![vec![
            vec2(0.0, 0.0),
            vec2(1.0, 0.0),
            vec2(1.0, 1.0),
            vec2(0.0, 1.0),
        ]];
        assert!(mesh.generate_tangents());
        assert_eq!(mesh.tangents.len(), mesh.vertex_count());
        for t in &mesh.tangents {
            assert!(
                (t.x - 1.0).abs() < 1e-4 && t.y.abs() < 1e-4,
                "tangent {t:?} does not follow +U"
            );
            // v rises with +Y here, so up the image — decreasing v — is -Y,
            // and `w * cross(+Z, +X) = w * +Y` must point there.
            assert_eq!(t.w, -1.0, "the bitangent must point up the image");
        }

        // Laid out as glTF does, v = 0 at the top: up the image is +Y.
        mesh.uv_sets = vec![vec![
            vec2(0.0, 1.0),
            vec2(1.0, 1.0),
            vec2(1.0, 0.0),
            vec2(0.0, 0.0),
        ]];
        assert!(mesh.generate_tangents());
        assert!(mesh.tangents.iter().all(|t| t.w == 1.0));
    }

    /// Welding invalidates the tangent frames, which are indexed by the old
    /// vertex order. Leaving stale ones behind would silently mis-orient every
    /// normal-mapped surface.
    #[test]
    fn welding_and_renormalizing_drop_stale_tangents() {
        let mut mesh = unwelded_cube();
        mesh.uv_sets = vec![vec![vec2(0.0, 0.0); mesh.vertex_count()]];
        mesh.generate_normals(30f32.to_radians());
        mesh.tangents = vec![vec4(1.0, 0.0, 0.0, 1.0); mesh.vertex_count()];

        mesh.join_identical_vertices();
        assert!(mesh.tangents.is_empty());

        mesh.tangents = vec![vec4(1.0, 0.0, 0.0, 1.0); mesh.vertex_count()];
        mesh.generate_normals(30f32.to_radians());
        assert!(mesh.tangents.is_empty());
    }

    /// A UV sphere of radius `r`: `1 / r` everywhere, positive.
    fn sphere(r: f32) -> (Vec<Vec3>, Vec<Vec3>, Vec<u32>) {
        let (rings, segments) = (24u32, 48u32);
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        for i in 0..=rings {
            let theta = std::f32::consts::PI * i as f32 / rings as f32;
            for j in 0..segments {
                let phi = std::f32::consts::TAU * j as f32 / segments as f32;
                let n = vec3(theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin());
                positions.push(n * r);
                normals.push(n);
            }
        }
        let mut indices = Vec::new();
        for i in 0..rings {
            for j in 0..segments {
                let a = i * segments + j;
                let b = i * segments + (j + 1) % segments;
                let (c, d) = (a + segments, b + segments);
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
        (positions, normals, indices)
    }

    /// The estimate reads the sphere's curvature, at any size, with the
    /// sign that means convex.
    #[test]
    fn a_sphere_curves_by_one_over_its_radius() {
        for r in [0.01f32, 1.0, 25.0] {
            let (positions, normals, indices) = sphere(r);
            let k = vertex_curvature(&positions, &normals, &indices);
            // Away from the poles, where the lattice degenerates.
            let middle: Vec<f32> = k[5 * 48..20 * 48].to_vec();
            for value in middle {
                assert!((value * r - 1.0).abs() < 0.05, "radius {r}: curvature {value}, expected {}", 1.0 / r);
            }
        }
        // Inside out — normals pointing in — the same sphere is concave.
        let (positions, normals, indices) = sphere(1.0);
        let inward: Vec<Vec3> = normals.iter().map(|n| -*n).collect();
        let k = vertex_curvature(&positions, &inward, &indices);
        assert!(k[10 * 48] < -0.9, "an inward-facing sphere read {}", k[10 * 48]);
    }

    /// Flat is zero, and a hard edge stored as split vertices — a cube with
    /// a normal per face — is convex, because the split vertices are welded.
    #[test]
    fn flat_is_zero_and_hard_edges_are_convex() {
        let plane = vec![vec3(0.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(1.0, 0.0, 1.0), vec3(0.0, 0.0, 1.0)];
        let up = vec![Vec3::Y; 4];
        let k = vertex_curvature(&plane, &up, &[0, 2, 1, 0, 3, 2]);
        assert!(k.iter().all(|v| v.abs() < 1e-6), "a plane curves: {k:?}");

        let mut cube = unwelded_cube();
        cube.generate_normals(30f32.to_radians());
        let k = vertex_curvature(&cube.positions, &cube.normals, &cube.indices);
        assert!(k.iter().all(|v| *v > 0.3), "a cube's corners should read convex: {k:?}");
    }

    #[test]
    fn the_hash_is_stable_across_calls() {
        let mesh = triangle(vec3(3.0, 4.0, 5.0));
        assert_eq!(mesh.content_hash(), mesh.content_hash());
    }

    #[test]
    fn degenerate_triangles_are_dropped() {
        let mut mesh = Mesh::new(
            vec![Vec3::ZERO, vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)],
            vec![
                0, 1, 2, // good
                0, 0, 1, // repeated vertex
                0, 1, 1, // repeated vertex
            ],
        );
        let dropped = mesh.validate_and_clean().unwrap();
        assert_eq!(dropped, 2);
        assert_eq!(mesh.triangle_count(), 1);
    }

    #[test]
    fn zero_area_triangles_are_dropped() {
        let mut mesh = Mesh::new(
            vec![Vec3::ZERO, vec3(1.0, 0.0, 0.0), vec3(2.0, 0.0, 0.0)],
            vec![0, 1, 2],
        );
        assert_eq!(mesh.validate_and_clean().unwrap(), 1);
        assert_eq!(mesh.triangle_count(), 0);
    }

    /// A NaN vertex is reported, not repaired: it poisons the BVH and every
    /// ray that touches it, and substituting a value would hide a broken
    /// export (§6.1 point 5).
    #[test]
    fn non_finite_positions_are_reported() {
        let mut mesh = Mesh::new(
            vec![Vec3::ZERO, vec3(f32::NAN, 0.0, 0.0), vec3(0.0, 1.0, 0.0)],
            vec![0, 1, 2],
        );
        let error = mesh.validate_and_clean().unwrap_err();
        assert!(error.contains("not finite"), "{error}");
    }

    #[test]
    fn out_of_range_indices_are_reported() {
        let mut mesh = Mesh::new(vec![Vec3::ZERO, Vec3::X, Vec3::Y], vec![0, 1, 7]);
        assert!(mesh.validate_and_clean().unwrap_err().contains("vertex 7"));

        let mut ragged = Mesh::new(vec![Vec3::ZERO, Vec3::X, Vec3::Y], vec![0, 1]);
        assert!(ragged
            .validate_and_clean()
            .unwrap_err()
            .contains("multiple of three"));
    }

    #[test]
    fn generated_normals_are_unit_length_and_area_weighted() {
        let mut mesh = Mesh::new(
            vec![Vec3::ZERO, vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)],
            vec![0, 1, 2],
        );
        mesh.generate_normals(30f32.to_radians());
        assert_eq!(mesh.normals.len(), 3);
        for n in &mesh.normals {
            assert!((n.length() - 1.0).abs() < 1e-5);
            assert!((*n - Vec3::Z).length() < 1e-5);
        }
    }

    #[test]
    fn bounds_cover_every_vertex() {
        let mesh = triangle(vec3(2.0, 3.0, 4.0));
        assert_eq!(mesh.bounds().min, vec3(2.0, 3.0, 4.0));
        assert_eq!(mesh.bounds().max, vec3(3.0, 4.0, 4.0));
    }

    #[test]
    fn unreferenced_meshes_can_be_collected() {
        let mut store = GeometryStore::new();
        let (a, _) = store.insert_deduplicated(triangle(Vec3::ZERO));
        let (b, _) = store.insert_deduplicated(Mesh::new(
            vec![Vec3::ZERO, vec3(5.0, 0.0, 0.0), vec3(0.0, 5.0, 0.0)],
            vec![0, 1, 2],
        ));
        assert_eq!(store.len(), 2);
        let dropped = store.retain_keys(&|key| key == a);
        assert_eq!(dropped, 1);
        assert!(store.get(a).is_some() && store.get(b).is_none());
    }

    /// Keys must never be reused, for the same reason node IDs must not: a
    /// stale reference would silently re-point at unrelated geometry.
    #[test]
    fn mesh_keys_are_not_reused_after_collection() {
        let mut store = GeometryStore::new();
        let (a, _) = store.insert_deduplicated(triangle(Vec3::ZERO));
        store.retain_keys(&|_| false);
        let (b, _) = store.insert_deduplicated(triangle(Vec3::ZERO));
        assert_ne!(a, b);
    }

    #[test]
    fn iteration_is_deterministic() {
        let build = || {
            let mut store = GeometryStore::new();
            for i in 0..32 {
                store.insert_deduplicated(triangle(vec3(i as f32, i as f32 * 2.0, 0.0)));
            }
            store
        };
        let a = build();
        let b = build();
        assert_eq!(a.keys().collect::<Vec<_>>(), b.keys().collect::<Vec<_>>());
        assert_eq!(a, b);
    }
}
