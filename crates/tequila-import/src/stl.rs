//! STL, binary and ASCII.
//!
//! STL is the simplest thing the importer will ever read and the most hostile
//! to the renderer: a flat soup of independent triangles with no vertex
//! sharing, no UVs, no names, no units, and a per-facet normal that exporters
//! routinely fill with zeros. Everything useful about the result comes from
//! the post-process pipeline (§6.1) rather than from the file.
//!
//! Including its normals. A facet normal is the flat face's, so taken as
//! the vertices' normals it shaded every curved STL in facets, and kept the
//! welder from joining any two of them. It decides only which way a
//! triangle faces; the pipeline welds and smooths up to the crease angle,
//! as for any mesh that arrives without normals.

use crate::raw::{RawMesh, RawNode, RawScene};
use crate::ImportError;
use tequila_core::math::{vec3, Vec3};

/// Read an STL file, choosing the binary or ASCII reader by inspection.
pub fn read(bytes: &[u8], name: &str) -> Result<RawScene, ImportError> {
    // An ASCII file saved by a Windows editor can start with a byte-order
    // mark, which hid its "solid" and had it read as binary.
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let mesh = if looks_binary(bytes) {
        read_binary(bytes)?
    } else {
        read_ascii(bytes)?
    };

    Ok(RawScene {
        // STL has no materials, so it has no textures either.
        textures: Vec::new(),
        nodes: vec![RawNode::part(name, 0)],
        roots: vec![0],
        meshes: vec![mesh],
        materials: Vec::new(),
        source_unit: None,
        tessellation: None,
        warnings: Vec::new(),
    })
}

/// Decide between the two encodings.
///
/// The `"solid"` prefix is *not* a reliable test — plenty of binary writers put
/// a product name starting with "solid" in the 80-byte header, and files that
/// fail this way are common enough that treating the prefix as authoritative
/// is a known bug in other readers. The triangle count is checked instead: a
/// binary STL is exactly `84 + 50n` bytes, which an ASCII file matches only by
/// coincidence, and the prefix is used only to break that tie.
fn looks_binary(bytes: &[u8]) -> bool {
    if bytes.len() < 84 {
        return false;
    }
    let count = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
    let expected = count.checked_mul(50).and_then(|n| n.checked_add(84));
    match expected {
        Some(expected) if expected == bytes.len() => true,
        // The size does not match, so trust the prefix - unless what
        // follows it is not text, as a binary file padded at its end is not.
        _ => !bytes.starts_with(b"solid") || std::str::from_utf8(bytes).is_err(),
    }
}

/// A facet's corners, turned to face the way its normal says where the two
/// disagree. A blank or broken normal leaves them as written.
fn facing(facet: Vec3, [a, b, c]: [Vec3; 3]) -> [Vec3; 3] {
    let winding = (b - a).cross(c - a);
    if facet.is_finite() && facet.length() > 1e-6 && winding.dot(facet) < 0.0 {
        [a, c, b]
    } else {
        [a, b, c]
    }
}

fn read_binary(bytes: &[u8]) -> Result<RawMesh, ImportError> {
    let mut count = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
    // A writer that streams its triangles can leave the count at zero; the
    // file's length then says how many there are.
    if count == 0 {
        count = (bytes.len() - 84) / 50;
    }
    if count == 0 {
        return Err(ImportError::Malformed("the file contains no triangles".into()));
    }
    let needed = count
        .checked_mul(50)
        .and_then(|n| n.checked_add(84))
        .ok_or_else(|| ImportError::Malformed("the triangle count overflows".into()))?;
    if bytes.len() < needed {
        return Err(ImportError::Malformed(format!(
            "the header declares {count} triangles, which needs {needed} bytes, but the file is {}",
            bytes.len()
        )));
    }

    let mut mesh = RawMesh::default();
    mesh.positions.reserve(count * 3);
    mesh.indices.reserve(count * 3);

    for t in 0..count {
        let at = 84 + t * 50;
        let facet = read_vec3(&bytes[at..]);
        let vertices = [
            read_vec3(&bytes[at + 12..]),
            read_vec3(&bytes[at + 24..]),
            read_vec3(&bytes[at + 36..]),
        ];
        for v in facing(facet, vertices) {
            mesh.indices.push(mesh.positions.len() as u32);
            mesh.positions.push(v);
        }
    }
    Ok(mesh)
}

fn read_vec3(bytes: &[u8]) -> Vec3 {
    let f =
        |at: usize| f32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    vec3(f(0), f(4), f(8))
}

fn read_ascii(bytes: &[u8]) -> Result<RawMesh, ImportError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| ImportError::Malformed("the file is neither binary STL nor UTF-8".into()))?;

    let mut mesh = RawMesh::default();
    let mut facet = Vec3::ZERO;
    let mut corners: Vec<Vec3> = Vec::with_capacity(3);

    for (line_number, line) in text.lines().enumerate() {
        let mut words = line.split_whitespace();
        match words.next() {
            Some("facet") => {
                // `facet normal nx ny nz`
                let values: Vec<&str> = words.collect();
                facet = match values.as_slice() {
                    ["normal", x, y, z] => vec3(
                        parse(x, line_number)?,
                        parse(y, line_number)?,
                        parse(z, line_number)?,
                    ),
                    _ => Vec3::ZERO,
                };
                corners.clear();
            }
            Some("vertex") => {
                let values: Vec<&str> = words.collect();
                let [x, y, z] = values.as_slice() else {
                    return Err(ImportError::Malformed(format!(
                        "line {}: a vertex needs three coordinates",
                        line_number + 1
                    )));
                };
                corners.push(vec3(
                    parse(x, line_number)?,
                    parse(y, line_number)?,
                    parse(z, line_number)?,
                ));
            }
            // At `endloop` or `endfacet`, whichever comes: a writer that
            // leaves one out still closes its facets with the other.
            Some("endloop" | "endfacet") if !corners.is_empty() => {
                let [a, b, c] = corners[..] else {
                    return Err(ImportError::Malformed(format!(
                        "line {}: a facet has {} vertices, not three",
                        line_number + 1,
                        corners.len()
                    )));
                };
                for v in facing(facet, [a, b, c]) {
                    mesh.indices.push(mesh.positions.len() as u32);
                    mesh.positions.push(v);
                }
                corners.clear();
            }
            _ => {}
        }
    }

    if mesh.positions.is_empty() {
        return Err(ImportError::Malformed(
            "the file contains no triangles".into(),
        ));
    }
    Ok(mesh)
}

fn parse(word: &str, line_number: usize) -> Result<f32, ImportError> {
    word.parse::<f32>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| {
            ImportError::Malformed(format!(
                "line {}: `{word}` is not a finite number",
                line_number + 1
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binary_stl(triangles: &[([f32; 3], [[f32; 3]; 3])]) -> Vec<u8> {
        let mut bytes = vec![0u8; 80];
        bytes.extend_from_slice(&(triangles.len() as u32).to_le_bytes());
        for (normal, vertices) in triangles {
            for v in normal {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            for vertex in vertices {
                for v in vertex {
                    bytes.extend_from_slice(&v.to_le_bytes());
                }
            }
            bytes.extend_from_slice(&0u16.to_le_bytes());
        }
        bytes
    }

    const TRIANGLE: ([f32; 3], [[f32; 3]; 3]) = (
        [0.0, 0.0, 1.0],
        [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
    );

    #[test]
    fn reads_a_binary_triangle() {
        let scene = read(&binary_stl(&[TRIANGLE]), "part").unwrap();
        let mesh = &scene.meshes[0];
        assert_eq!(mesh.positions.len(), 3);
        assert_eq!(mesh.indices, vec![0, 1, 2]);
        assert!(mesh.normals.is_empty(), "facet normals are the pipeline's to smooth, not vertex normals");
        assert_eq!(scene.nodes[0].name, "part");
    }

    #[test]
    fn reads_an_ascii_triangle() {
        let text = b"solid part
  facet normal 0 0 1
    outer loop
      vertex 0 0 0
      vertex 1 0 0
      vertex 0 1 0
    endloop
  endfacet
endsolid part
";
        let scene = read(text, "part").unwrap();
        assert_eq!(scene.meshes[0].positions.len(), 3);
        assert!(scene.meshes[0].normals.is_empty());
    }

    /// A facet wound against its normal is turned to face the way the
    /// normal says, which is all a facet normal decides now.
    #[test]
    fn a_facet_faces_the_way_its_normal_says() {
        let backwards = ([0.0, 0.0, -1.0], TRIANGLE.1);
        let scene = read(&binary_stl(&[TRIANGLE, backwards]), "part").unwrap();
        let p = &scene.meshes[0].positions;
        let face = |t: usize| (p[3 * t + 1] - p[3 * t]).cross(p[3 * t + 2] - p[3 * t]);
        assert!(face(0).z > 0.0 && face(1).z < 0.0, "{:?} {:?}", face(0), face(1));
    }

    /// A curved STL comes out smooth: its facets' shared corners are joined
    /// and given normals between theirs, not each facet's flat one.
    #[test]
    fn a_curved_stl_is_smoothed_across_its_facets() {
        // A coarse cylinder wall, twenty-four facets round.
        let mut triangles = Vec::new();
        for i in 0..24 {
            // The last facet ends on the first's corner exactly, as a
            // writer's shared corners are.
            let (a0, a1) = (i as f32 / 24.0 * std::f32::consts::TAU, ((i + 1) % 24) as f32 / 24.0 * std::f32::consts::TAU);
            let (p0, p1) = ([a0.cos(), 0.0, a0.sin()], [a1.cos(), 0.0, a1.sin()]);
            let (q0, q1) = ([p0[0], 1.0, p0[2]], [p1[0], 1.0, p1[2]]);
            let mid = (i as f32 + 0.5) / 24.0 * std::f32::consts::TAU;
            let out = [mid.cos(), 0.0, mid.sin()];
            triangles.push((out, [p0, q0, p1]));
            triangles.push((out, [p1, q0, q1]));
        }
        let scene = read(&binary_stl(&triangles), "tube").unwrap();
        let document = tequila_scene::SceneDocument::new();
        let (command, _) = crate::build(&document, None, scene, &crate::ImportOptions::default(), None).unwrap();
        let tequila_scene::Command::Import { payload } = command else { panic!("an import") };
        let mesh = &payload.meshes[0].1;
        assert!(mesh.positions.len() <= 48 + 2, "the facets' corners were not joined: {} vertices", mesh.positions.len());
        // A normal at a corner points along its radius, between the two
        // facets', not along either.
        for (p, n) in mesh.positions.iter().zip(&mesh.normals) {
            let radial = vec3(p.x, 0.0, p.z).normalize();
            assert!(n.dot(radial) > 0.995, "a faceted normal {n:?} at {p:?}");
        }
    }

    /// A binary file whose writer left the count at zero still reads, by
    /// its length; and an ASCII one saved with a byte-order mark is ASCII.
    #[test]
    fn a_zero_count_and_a_byte_order_mark_are_read() {
        let mut bytes = binary_stl(&[TRIANGLE, TRIANGLE]);
        bytes[80..84].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(read(&bytes, "part").unwrap().meshes[0].positions.len(), 6);

        let mut text = b"\xEF\xBB\xBF".to_vec();
        text.extend_from_slice(b"solid p\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid p\n");
        assert_eq!(read(&text, "part").unwrap().meshes[0].positions.len(), 3);
    }

    /// The classic STL bug: a binary file whose 80-byte header happens to start
    /// with "solid". Reading it as ASCII yields an empty mesh or an error.
    #[test]
    fn a_binary_file_whose_header_says_solid_is_still_binary() {
        let mut bytes = binary_stl(&[TRIANGLE]);
        bytes[..14].copy_from_slice(b"solid Exported");

        let scene = read(&bytes, "part").unwrap();
        assert_eq!(scene.meshes[0].positions.len(), 3);
    }

    /// Writers routinely leave the facet normal at zero. Such a facet keeps
    /// the winding it was written with, and nothing of it is imported as a
    /// normal.
    #[test]
    fn blank_facet_normals_are_dropped_rather_than_imported() {
        let blank = ([0.0, 0.0, 0.0], TRIANGLE.1);
        let scene = read(&binary_stl(&[TRIANGLE, blank]), "part").unwrap();
        assert!(scene.meshes[0].normals.is_empty());
        assert_eq!(scene.meshes[0].positions[3..], scene.meshes[0].positions[..3], "a blank normal turned its facet");
    }

    #[test]
    fn a_truncated_file_is_an_error_not_a_panic() {
        let mut bytes = binary_stl(&[TRIANGLE]);
        bytes.truncate(bytes.len() - 10);
        assert!(read(&bytes, "part").is_err());

        // And a header claiming a preposterous count must not try to allocate
        // it.
        let mut bytes = vec![0u8; 84];
        bytes[80..84].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(read(&bytes, "part").is_err());
    }

    #[test]
    fn malformed_ascii_is_an_error() {
        for bad in [
            "solid\n facet normal 0 0 1\n vertex 0 0\n endfacet\n",
            "solid\n facet normal 0 0 1\n vertex a b c\n endfacet\n",
            "solid\nendsolid\n",
        ] {
            assert!(read(bad.as_bytes(), "part").is_err(), "{bad:?}");
        }
    }
}
