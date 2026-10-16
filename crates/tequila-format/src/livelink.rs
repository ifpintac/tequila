//! LiveLink messages (`docs/livelink-protocol.md`), read from JSON into the
//! scene's terms (`tequila_scene::livelink`).
//!
//! Buffers arrive as base64 of little-endian numbers - positions, normals
//! and UVs as `f32`, indices as `u32` - because that is what a client's
//! array library writes in one call, and JSON numbers for a million
//! vertices are megabytes of text to parse. Everything is already in
//! Tequila's axes: Y up, metres, UV `v` running down the image.

use crate::json::Value;
use tequila_core::math::{vec2, vec3, Mat4, Transform, Vec2, Vec3};
use tequila_scene::geometry::Mesh;
use tequila_scene::livelink::{LiveMaterial, LiveObject, LiveSync};
use std::collections::BTreeMap;

/// Standard base64, padding optional.
pub fn decode_base64(text: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0;
    for c in text.bytes() {
        let value = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return Err(format!("{:?} is not base64", c as char)),
        };
        buffer = buffer << 6 | value as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Ok(out)
}

fn floats(value: Option<&Value>, what: &str) -> Result<Vec<f32>, String> {
    let Some(text) = value.and_then(|v| v.as_str()) else { return Ok(Vec::new()) };
    let bytes = decode_base64(text)?;
    if bytes.len() % 4 != 0 {
        return Err(format!("{what} is not whole 32-bit numbers"));
    }
    Ok(bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect())
}

fn mesh(value: &Value) -> Result<Mesh, String> {
    let positions = floats(value.get("positions"), "positions")?;
    let normals = floats(value.get("normals"), "normals")?;
    let uvs = floats(value.get("uvs"), "uvs")?;
    let indices: Vec<u32> = match value.get("indices").and_then(|v| v.as_str()) {
        Some(text) => decode_base64(text)?.chunks_exact(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect(),
        None => (0..(positions.len() / 3) as u32).collect(),
    };
    let count = positions.len() / 3;
    if positions.len() % 3 != 0 || indices.len() % 3 != 0 {
        return Err("a mesh needs whole positions and whole triangles".into());
    }
    if indices.iter().any(|&i| i as usize >= count) {
        return Err("a triangle points past the last vertex".into());
    }
    let to3 = |v: &[f32]| -> Vec<Vec3> { v.chunks_exact(3).map(|c| vec3(c[0], c[1], c[2])).collect() };
    let mut out = Mesh::new(to3(&positions), indices);
    if normals.len() == positions.len() {
        out = out.with_normals(to3(&normals));
    }
    if uvs.len() == count * 2 {
        out = out.with_uvs(uvs.chunks_exact(2).map(|c| vec2(c[0], c[1])).collect::<Vec<Vec2>>());
    }
    // A client sends one vertex per triangle corner, which carries split
    // normals and seams exactly; shared corners are shared again here.
    out.join_identical_vertices();
    Ok(out)
}

/// Sixteen numbers, column-major as glTF writes them.
fn transform(value: Option<&Value>) -> Result<Transform, String> {
    let Some(values) = value.and_then(|v| v.as_f32_array()) else { return Ok(Transform::IDENTITY) };
    if values.len() != 16 {
        return Err("a matrix is sixteen numbers".into());
    }
    let mut m = Mat4::IDENTITY;
    for c in 0..4 {
        for r in 0..4 {
            m.m[r][c] = values[c * 4 + r];
        }
    }
    Transform::from_matrix(m).ok_or_else(|| "a matrix that flattens the object cannot be inverted".into())
}

fn material(value: &Value) -> LiveMaterial {
    let d = LiveMaterial::default();
    let number = |key: &str, fallback: f32| value.get(key).and_then(|v| v.as_f32()).unwrap_or(fallback);
    let colour = |key: &str, fallback: [f32; 3]| {
        value.get(key).and_then(|v| v.as_f32_array()).filter(|c| c.len() >= 3).map(|c| [c[0], c[1], c[2]]).unwrap_or(fallback)
    };
    LiveMaterial {
        name: value.get("name").and_then(|v| v.as_str()).unwrap_or("Material").to_string(),
        base_color: colour("base_color", d.base_color),
        metallic: number("metallic", d.metallic),
        roughness: number("roughness", d.roughness),
        ior: number("ior", d.ior),
        transmission: number("transmission", d.transmission),
        alpha: number("alpha", d.alpha),
        emission: colour("emission", d.emission),
        emission_strength: number("emission_strength", d.emission_strength),
        coat: number("coat", d.coat),
        coat_roughness: number("coat_roughness", d.coat_roughness),
        sheen: number("sheen", d.sheen),
        sheen_tint: colour("sheen_tint", d.sheen_tint),
    }
}

/// A `sync` message.
pub fn read_sync(value: &Value) -> Result<LiveSync, String> {
    let source = value.get("source").and_then(|v| v.as_str()).ok_or("a sync names its source")?.to_string();
    let label = value.get("label").and_then(|v| v.as_str()).unwrap_or(&source).to_string();
    let mut upserts = Vec::new();
    for entry in value.get("upserts").map(|v| v.as_array()).unwrap_or(&[]) {
        let key = entry.get("key").and_then(|v| v.as_str()).ok_or("an object has no key")?.to_string();
        let metadata: BTreeMap<String, String> = match entry.get("metadata") {
            Some(Value::Object(map)) => map.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect(),
            _ => BTreeMap::new(),
        };
        upserts.push(LiveObject {
            name: entry.get("name").and_then(|v| v.as_str()).unwrap_or(&key).to_string(),
            parent: entry.get("parent").and_then(|v| v.as_str()).map(str::to_string),
            transform: transform(entry.get("matrix")).map_err(|e| format!("{key}: {e}"))?,
            mesh: entry.get("mesh").map(mesh).transpose().map_err(|e| format!("{key}: {e}"))?,
            material: entry.get("material").map(material),
            metadata,
            key,
        });
    }
    let deletes = value
        .get("deletes")
        .map(|v| v.as_array().iter().filter_map(|k| k.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let complete = value.get("complete").and_then(|v| v.as_bool()).unwrap_or(false);
    Ok(LiveSync { source, label, upserts, deletes, complete })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(bytes: &[u8]) -> String {
        crate::web::base64(bytes)
    }

    #[test]
    fn base64_reads_what_it_wrote() {
        for text in [&b""[..], b"f", b"fo", b"foo", b"foobar", &[0u8, 255, 128, 7]] {
            assert_eq!(decode_base64(&b64(text)).unwrap(), text);
        }
        assert!(decode_base64("a*b").is_err());
    }

    /// A sync as a client writes it: one triangle, its corners' normals and
    /// UVs, a placement, a material.
    #[test]
    fn a_sync_message_reads() {
        let f = |v: &[f32]| b64(&v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>());
        let text = format!(
            r#"{{"source":"blender:a.blend","label":"Blender","upserts":[{{"key":"Tri","name":"Tri",
            "matrix":[1,0,0,0, 0,1,0,0, 0,0,1,0, 2,3,4,1],
            "mesh":{{"positions":"{}","normals":"{}","uvs":"{}"}},
            "material":{{"name":"Red","base_color":[0.8,0.1,0.1],"roughness":0.3}},
            "metadata":{{"Blender object":"Tri"}}}}],"deletes":["Old"]}}"#,
            f(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]),
            f(&[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0]),
            f(&[0.0, 1.0, 1.0, 1.0, 0.0, 0.0]),
        );
        let sync = read_sync(&crate::json::parse(text.as_bytes()).unwrap()).expect("reads");
        assert_eq!(sync.deletes, ["Old"]);
        let object = &sync.upserts[0];
        let mesh = object.mesh.as_ref().unwrap();
        assert_eq!((mesh.positions.len(), mesh.indices.len()), (3, 3));
        assert_eq!(mesh.uv_sets[0][1], vec2(1.0, 1.0));
        assert_eq!(object.transform.point(Vec3::ZERO), vec3(2.0, 3.0, 4.0), "the matrix is column-major");
        assert_eq!(object.material.as_ref().unwrap().roughness, 0.3);
        assert_eq!(object.metadata.get("Blender object").map(String::as_str), Some("Tri"));
    }
}
