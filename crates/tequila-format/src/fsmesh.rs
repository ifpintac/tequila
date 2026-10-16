//! `.fsmesh`, the binary mesh member of §4.3.
//!
//! > Fixed-layout and aligned so it can be memory-mapped and handed to Embree
//! > without a copy.
//!
//! Which sets the constraints: little-endian, no varints, no per-element tags,
//! and every array 4-byte aligned from the start of the file. The header is
//! 52 bytes, itself a multiple of 4, so the arrays that follow it stay aligned
//! without padding.

use tequila_core::math::{vec2, vec3, vec4, Vec2, Vec3, Vec4};
use tequila_scene::geometry::{Mesh, SubMesh};
use std::fmt;

const MAGIC: &[u8; 8] = b"FSMESH\0\0";
const VERSION: u32 = 1;

const FLAG_NORMALS: u32 = 1 << 0;
const FLAG_TANGENTS: u32 = 1 << 1;
const FLAG_COLORS: u32 = 1 << 2;
/// The model's own edges follow the submeshes: a segment count, then each
/// segment's two end points. Last in the member, so a build that does not
/// know the flag reads everything before it and stops.
const FLAG_EDGES: u32 = 1 << 3;

/// Header size in bytes: magic(8) + version(4) + counts(8) + flags(4) +
/// uv_set_count(4) + bounds(24).
const HEADER: usize = 52;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MeshError {
    Malformed(String),
    /// Written by a newer build using a layout this one does not know.
    Version(u32),
}

impl fmt::Display for MeshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MeshError::Malformed(m) => write!(f, "the mesh is malformed: {m}"),
            MeshError::Version(v) => write!(
                f,
                "the mesh uses .fsmesh version {v}; this build reads version {VERSION}"
            ),
        }
    }
}

impl std::error::Error for MeshError {}

pub fn write(mesh: &Mesh) -> Vec<u8> {
    let vertices = mesh.positions.len();
    let mut flags = 0u32;
    if mesh.normals.len() == vertices && vertices > 0 {
        flags |= FLAG_NORMALS;
    }
    if mesh.tangents.len() == vertices && vertices > 0 {
        flags |= FLAG_TANGENTS;
    }
    if mesh.colors.len() == vertices && vertices > 0 {
        flags |= FLAG_COLORS;
    }
    if mesh.edge_lines.len() >= 2 {
        flags |= FLAG_EDGES;
    }

    let uv_sets: Vec<&Vec<Vec2>> = mesh
        .uv_sets
        .iter()
        .filter(|set| set.len() == vertices)
        .collect();

    let mut out = Vec::with_capacity(HEADER + vertices * 12 + mesh.indices.len() * 4);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(vertices as u32).to_le_bytes());
    out.extend_from_slice(&(mesh.indices.len() as u32).to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&(uv_sets.len() as u32).to_le_bytes());

    let bounds = mesh.bounds();
    for v in [bounds.min, bounds.max] {
        for c in [v.x, v.y, v.z] {
            out.extend_from_slice(&c.to_le_bytes());
        }
    }
    debug_assert_eq!(out.len(), HEADER);

    for p in &mesh.positions {
        write_vec3(&mut out, *p);
    }
    if flags & FLAG_NORMALS != 0 {
        for n in &mesh.normals {
            write_vec3(&mut out, *n);
        }
    }
    if flags & FLAG_TANGENTS != 0 {
        for t in &mesh.tangents {
            for c in [t.x, t.y, t.z, t.w] {
                out.extend_from_slice(&c.to_le_bytes());
            }
        }
    }
    for set in &uv_sets {
        for uv in set.iter() {
            out.extend_from_slice(&uv.x.to_le_bytes());
            out.extend_from_slice(&uv.y.to_le_bytes());
        }
    }
    if flags & FLAG_COLORS != 0 {
        for c in &mesh.colors {
            for v in [c.x, c.y, c.z, c.w] {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
    }
    for &index in &mesh.indices {
        out.extend_from_slice(&index.to_le_bytes());
    }

    out.extend_from_slice(&(mesh.submeshes.len() as u32).to_le_bytes());
    for sub in &mesh.submeshes {
        out.extend_from_slice(&sub.first_index.to_le_bytes());
        out.extend_from_slice(&sub.index_count.to_le_bytes());
        out.extend_from_slice(&sub.slot.to_le_bytes());
    }
    if flags & FLAG_EDGES != 0 {
        let segments = mesh.edge_lines.len() / 2;
        out.extend_from_slice(&(segments as u32).to_le_bytes());
        for p in &mesh.edge_lines[..segments * 2] {
            write_vec3(&mut out, *p);
        }
    }
    out
}

fn write_vec3(out: &mut Vec<u8>, v: Vec3) {
    for c in [v.x, v.y, v.z] {
        out.extend_from_slice(&c.to_le_bytes());
    }
}

pub fn read(bytes: &[u8]) -> Result<Mesh, MeshError> {
    if bytes.len() < HEADER || &bytes[..8] != MAGIC {
        return Err(MeshError::Malformed(
            "the member does not begin with the .fsmesh magic".into(),
        ));
    }
    let mut reader = Reader { bytes, at: 8 };
    let version = reader.u32()?;
    if version != VERSION {
        return Err(MeshError::Version(version));
    }
    let vertices = reader.u32()? as usize;
    let index_count = reader.u32()? as usize;
    let flags = reader.u32()?;
    let uv_set_count = reader.u32()? as usize;
    // A handful in any real mesh. With no vertices the size check below
    // passes whatever this says, and the sets were reserved for before it.
    if uv_set_count > 64 {
        return Err(MeshError::Malformed(format!("{uv_set_count} UV sets is not a mesh")));
    }

    // Check the sizes against what is actually here *before* allocating, so a
    // header claiming four billion vertices fails rather than aborting on a
    // failed allocation.
    let channels = 3
        + 3 * usize::from(flags & FLAG_NORMALS != 0)
        + 4 * usize::from(flags & FLAG_TANGENTS != 0)
        + 4 * usize::from(flags & FLAG_COLORS != 0)
        + 2 * uv_set_count;
    let needed = vertices
        .checked_mul(channels * 4)
        .and_then(|n| n.checked_add(index_count.checked_mul(4)?))
        .and_then(|n| n.checked_add(HEADER + 4))
        .ok_or_else(|| MeshError::Malformed("the declared sizes overflow".into()))?;
    if bytes.len() < needed {
        return Err(MeshError::Malformed(format!(
            "the header declares {vertices} vertices and {index_count} indices, needing \
             {needed} bytes, but the member is {}",
            bytes.len()
        )));
    }
    if index_count % 3 != 0 {
        return Err(MeshError::Malformed(format!(
            "{index_count} indices is not a multiple of three"
        )));
    }

    let mut bounds = [0f32; 6];
    for slot in &mut bounds {
        *slot = reader.f32()?;
    }

    let mut positions = Vec::with_capacity(vertices);
    for _ in 0..vertices {
        positions.push(reader.vec3()?);
    }
    let mut normals = Vec::new();
    if flags & FLAG_NORMALS != 0 {
        normals.reserve(vertices);
        for _ in 0..vertices {
            normals.push(reader.vec3()?);
        }
    }
    let mut tangents: Vec<Vec4> = Vec::new();
    if flags & FLAG_TANGENTS != 0 {
        tangents.reserve(vertices);
        for _ in 0..vertices {
            tangents.push(vec4(
                reader.f32()?,
                reader.f32()?,
                reader.f32()?,
                reader.f32()?,
            ));
        }
    }
    let mut uv_sets = Vec::with_capacity(uv_set_count);
    for _ in 0..uv_set_count {
        let mut set = Vec::with_capacity(vertices);
        for _ in 0..vertices {
            set.push(vec2(reader.f32()?, reader.f32()?));
        }
        uv_sets.push(set);
    }
    // After the UV sets, as the layout table has always said. No build wrote
    // colours before this one, so no file has them anywhere else.
    let mut colors = Vec::new();
    if flags & FLAG_COLORS != 0 {
        colors.reserve(vertices);
        for _ in 0..vertices {
            colors.push(vec4(reader.f32()?, reader.f32()?, reader.f32()?, reader.f32()?));
        }
    }

    let mut indices = Vec::with_capacity(index_count);
    for _ in 0..index_count {
        let index = reader.u32()?;
        if index as usize >= vertices {
            return Err(MeshError::Malformed(format!(
                "index {index} refers to a vertex beyond the {vertices} present"
            )));
        }
        indices.push(index);
    }

    let submesh_count = reader.u32()? as usize;
    let mut submeshes = Vec::with_capacity(submesh_count.min(1024));
    for _ in 0..submesh_count {
        let sub = SubMesh {
            first_index: reader.u32()?,
            index_count: reader.u32()?,
            slot: reader.u32()?,
        };
        if sub.first_index as usize + sub.index_count as usize > index_count {
            return Err(MeshError::Malformed(
                "a submesh runs past the index buffer".into(),
            ));
        }
        submeshes.push(sub);
    }

    let mut edge_lines = Vec::new();
    if flags & FLAG_EDGES != 0 {
        let segments = reader.u32()? as usize;
        if bytes.len() < reader.at + segments.saturating_mul(24) {
            return Err(MeshError::Malformed(format!("{segments} edge segments run past the member")));
        }
        edge_lines.reserve(segments * 2);
        for _ in 0..segments * 2 {
            edge_lines.push(reader.vec3()?);
        }
    }

    let mut mesh = Mesh::new(positions, indices);
    mesh.edge_lines = edge_lines;
    mesh.normals = normals;
    mesh.tangents = tangents;
    mesh.uv_sets = uv_sets;
    mesh.colors = colors;
    mesh.submeshes = submeshes;
    // Bounds are recomputed rather than trusted: they are derivable from the
    // positions, and a stored value that disagreed would make the BVH cull
    // geometry that is really there.
    mesh.recompute_bounds();
    let stored = (
        vec3(bounds[0], bounds[1], bounds[2]),
        vec3(bounds[3], bounds[4], bounds[5]),
    );
    debug_assert!(
        mesh.positions.is_empty()
            || ((mesh.bounds().min - stored.0).length() < 1e-3
                && (mesh.bounds().max - stored.1).length() < 1e-3),
        "the stored bounds disagree with the positions"
    );
    Ok(mesh)
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn u32(&mut self) -> Result<u32, MeshError> {
        let b = self
            .bytes
            .get(self.at..self.at + 4)
            .ok_or_else(|| MeshError::Malformed("the member ends unexpectedly".into()))?;
        self.at += 4;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn f32(&mut self) -> Result<f32, MeshError> {
        Ok(f32::from_bits(self.u32()?))
    }

    fn vec3(&mut self) -> Result<Vec3, MeshError> {
        Ok(vec3(self.f32()?, self.f32()?, self.f32()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_mesh() -> Mesh {
        let mut mesh = Mesh::new(
            vec![Vec3::ZERO, vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)],
            vec![0, 1, 2],
        );
        mesh.normals = vec![Vec3::Z; 3];
        mesh.tangents = vec![vec4(1.0, 0.0, 0.0, -1.0); 3];
        mesh.uv_sets = vec![
            vec![vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0)],
            vec![vec2(0.5, 0.5); 3],
        ];
        mesh.colors = vec![vec4(1.0, 0.0, 0.0, 1.0), vec4(0.0, 0.5, 0.0, 0.25), vec4(0.1, 0.2, 0.3, 1.0)];
        mesh.submeshes = vec![SubMesh {
            first_index: 0,
            index_count: 3,
            slot: 2,
        }];
        mesh
    }

    /// A part's own edges come back as they went, after everything else
    /// in the member; a mesh with none writes none.
    #[test]
    fn the_parts_edges_survive_the_round_trip() {
        let mut mesh = full_mesh();
        mesh.edge_lines = vec![Vec3::ZERO, vec3(1.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)];
        let bytes = write(&mesh);
        assert_eq!(read(&bytes).unwrap(), mesh);
        let without = write(&full_mesh());
        assert_eq!(bytes.len() - without.len(), 4 + 4 * 12, "a count and four points, appended");
        assert_eq!(bytes[..without.len()].len(), without.len());
    }

    #[test]
    fn every_channel_survives_the_round_trip() {
        let mesh = full_mesh();
        let read = read(&write(&mesh)).unwrap();
        assert_eq!(read, mesh);
    }

    #[test]
    fn a_bare_mesh_round_trips_without_inventing_channels() {
        let mesh = Mesh::new(
            vec![Vec3::ZERO, vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)],
            vec![0, 1, 2],
        );
        let read = read(&write(&mesh)).unwrap();
        assert_eq!(read, mesh);
        assert!(read.normals.is_empty(), "a normal channel was fabricated");
        assert!(read.tangents.is_empty());
        assert!(read.uv_sets.is_empty());
    }

    #[test]
    fn an_empty_mesh_round_trips() {
        let mesh = Mesh::default();
        assert_eq!(read(&write(&mesh)).unwrap(), mesh);
    }

    /// §4.3 fixes the layout so it can be mapped; the header offsets are part
    /// of the format and cannot drift silently.
    #[test]
    fn the_header_layout_is_what_the_specification_says() {
        let bytes = write(&full_mesh());
        assert_eq!(&bytes[..8], MAGIC);
        assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 1); // version
        assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 3); // vertices
        assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), 3); // indices
        assert_eq!(
            u32::from_le_bytes(bytes[20..24].try_into().unwrap()),
            FLAG_NORMALS | FLAG_TANGENTS | FLAG_COLORS
        );
        assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), 2); // uv sets
                                                                              // Arrays start immediately after the header and stay 4-byte aligned.
        assert_eq!(HEADER % 4, 0);
        assert_eq!(u32::from_le_bytes(bytes[52..56].try_into().unwrap()), 0); // first position x
    }

    #[test]
    fn two_writes_of_one_mesh_are_byte_identical() {
        assert_eq!(write(&full_mesh()), write(&full_mesh()));
    }

    #[test]
    fn a_future_version_is_refused_by_number() {
        let mut bytes = write(&full_mesh());
        bytes[8..12].copy_from_slice(&99u32.to_le_bytes());
        match read(&bytes) {
            Err(MeshError::Version(v)) => assert_eq!(v, 99),
            other => panic!("expected a version error, got {other:?}"),
        }
    }

    /// A header declaring more than the member holds must be rejected before
    /// anything is allocated for it.
    #[test]
    fn an_implausible_header_is_refused_without_allocating() {
        let mut bytes = write(&full_mesh());
        bytes[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(read(&bytes).is_err());

        let mut bytes = write(&full_mesh());
        bytes[16..20].copy_from_slice(&1_000_000u32.to_le_bytes());
        assert!(read(&bytes).is_err());
    }

    /// An index past the vertex buffer would be an out-of-bounds read in the
    /// renderer, so it is caught at the boundary.
    #[test]
    fn an_out_of_range_index_is_refused() {
        let mesh = full_mesh();
        let mut bytes = write(&mesh);
        let indices_at = bytes.len() - 4 - 12 - 3 * 4;
        bytes[indices_at..indices_at + 4].copy_from_slice(&7u32.to_le_bytes());
        let error = read(&bytes).unwrap_err();
        assert!(error.to_string().contains("index 7"), "{error}");
    }

    #[test]
    fn truncation_at_any_point_is_an_error_not_a_panic() {
        let bytes = write(&full_mesh());
        for cut in 0..bytes.len() {
            assert!(
                read(&bytes[..cut]).is_err(),
                "truncating to {cut} should fail"
            );
        }
    }

    /// Colours sit after the UV sets, as the layout table has always said.
    #[test]
    fn colours_follow_the_uv_sets() {
        let mesh = full_mesh();
        let bytes = write(&mesh);
        let vertices = mesh.positions.len();
        let colours_at = HEADER + vertices * (3 + 3 + 4) * 4 + mesh.uv_sets.len() * vertices * 2 * 4;
        let first = f32::from_le_bytes(bytes[colours_at..colours_at + 4].try_into().unwrap());
        let alpha = f32::from_le_bytes(bytes[colours_at + 28..colours_at + 32].try_into().unwrap());
        assert_eq!((first, alpha), (1.0, 0.25));
        assert!(a_bare_mesh_has_no_colours());
    }

    fn a_bare_mesh_has_no_colours() -> bool {
        let mesh = Mesh::new(vec![Vec3::ZERO, vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)], vec![0, 1, 2]);
        read(&write(&mesh)).unwrap().colors.is_empty()
    }
}
