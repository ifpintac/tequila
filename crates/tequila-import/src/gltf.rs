//! glTF 2.0, both the `.gltf` (JSON plus external buffers) and `.glb`
//! (single-file, binary-chunked) encodings.
//!
//! glTF is the format M1's exit criterion is written against, and the one that
//! actually carries what a scene needs: a node hierarchy with transforms,
//! per-primitive materials, and PBR parameters that map onto OpenPBR directly
//! rather than by approximation (§6.1 point 8).
//!
//! Two conventions worth stating, because both are silent failures if missed:
//!
//! - glTF is **metres by specification** (§3.4 wants the unit honoured where
//!   the format states one), and Y-up, right-handed — the same as this
//!   renderer, so no axis conversion is applied.
//! - A node's transform is either a `matrix` **or** a TRS triple, never both,
//!   and the TRS order is `T * R * S`.

use crate::raw::{RawMesh, RawNode, RawScene};
use crate::ImportError;
use tequila_core::math::{vec2, vec3, Mat4, Transform, Vec3};
use tequila_core::spectrum::spectrum;
use tequila_core::units::LengthUnit;
use tequila_format::json;
use tequila_materials::fresnel::metals;
use tequila_materials::material_type::MaterialType;
use tequila_render_core::material::Material;
use tequila_scene::material::MaterialDefinition;

/// Component types, from the glTF specification's accessor table.
const BYTE: usize = 5120;
const UNSIGNED_BYTE: usize = 5121;
const SHORT: usize = 5122;
const UNSIGNED_SHORT: usize = 5123;
const UNSIGNED_INT: usize = 5125;
const FLOAT: usize = 5126;

/// Triangles. Other primitive modes (points, lines, strips, fans) are skipped
/// with a warning rather than silently dropped.
const MODE_TRIANGLES: usize = 4;

/// Read a `.gltf` or `.glb`. `resolve` supplies the contents of any external
/// buffer URI; returning `None` for one is an error, since geometry cannot be
/// reconstructed without it.
pub fn read(
    bytes: &[u8],
    name: &str,
    resolve: &dyn Fn(&str) -> Option<Vec<u8>>,
) -> Result<RawScene, ImportError> {
    let (document, binary_chunk) = split_container(bytes)?;
    let root = json::parse(&document).map_err(|e| ImportError::Malformed(e.to_string()))?;

    let version = root
        .get("asset")
        .and_then(|a| a.get("version"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !version.starts_with('2') {
        return Err(ImportError::Unsupported(format!(
            "glTF version `{version}`; this reader implements 2.x"
        )));
    }

    let mut warnings = Vec::new();
    report_unsupported_extensions(&root, &mut warnings);
    let buffers = load_buffers(&root, binary_chunk, resolve, &mut warnings)?;
    let mut textures = Vec::new();
    let materials = read_materials(&root, &buffers, &mut textures, &mut warnings);
    let meshes = read_meshes(&root, &buffers, &mut warnings)?;
    let (nodes, roots) = read_nodes(&root, &meshes, name, &mut warnings)?;

    let scene = RawScene {
        textures,
        nodes,
        roots,
        meshes: meshes.into_iter().flat_map(|m| m.primitives).collect(),
        materials,
        // §3.4: honour the unit where the format states one. glTF states metres.
        source_unit: Some(LengthUnit::Metre),
        tessellation: None,
        warnings,
    };
    scene.validate().map_err(ImportError::Malformed)?;
    Ok(scene)
}

/// Split a GLB into its JSON and binary chunks, or pass a plain `.gltf`
/// through unchanged.
fn split_container(bytes: &[u8]) -> Result<(Vec<u8>, Option<&[u8]>), ImportError> {
    if !bytes.starts_with(b"glTF") {
        return Ok((bytes.to_vec(), None));
    }
    if bytes.len() < 12 {
        return Err(ImportError::Malformed("the GLB header is truncated".into()));
    }
    let declared = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    if declared > bytes.len() {
        return Err(ImportError::Malformed(format!(
            "the GLB header declares {declared} bytes but the file is {}",
            bytes.len()
        )));
    }

    let mut document = None;
    let mut binary = None;
    let mut at = 12;
    while at + 8 <= declared {
        let length =
            u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]) as usize;
        let kind = &bytes[at + 4..at + 8];
        let start = at + 8;
        let end = start
            .checked_add(length)
            .filter(|end| *end <= declared)
            .ok_or_else(|| {
                ImportError::Malformed(format!("a GLB chunk at byte {at} runs past the file"))
            })?;
        match kind {
            b"JSON" => document = Some(bytes[start..end].to_vec()),
            b"BIN\0" => binary = Some(&bytes[start..end]),
            // Unknown chunk types are to be ignored, by specification.
            _ => {}
        }
        // Chunks are 4-byte aligned.
        at = end + (4 - end % 4) % 4;
    }

    let document =
        document.ok_or_else(|| ImportError::Malformed("the GLB contains no JSON chunk".into()))?;
    Ok((document, binary))
}

/// Extensions the file uses that this reader does not implement.
///
/// Two lists in the file, and they call for different words. `extensionsUsed`
/// is everything the asset touches; `extensionsRequired` is the subset the
/// specification says it "cannot be properly loaded" without. Ignoring either
/// gives a render that does not match the file, so both are reported — a
/// required one more firmly, because the file itself says so.
///
/// Loading anyway is right in both cases. The geometry is nearly always fine,
/// and a model on screen can be judged where a refusal cannot. Loading anyway
/// *in silence* is the thing worth avoiding: an unread extension is a material
/// that comes in looking like plastic with nothing to say why.
///
/// Listed against what is implemented rather than against a catalogue of what
/// exists, so an extension added to the reader stops being reported without
/// anyone having to remember a second list.
fn report_unsupported_extensions(root: &json::Value, warnings: &mut Vec<String>) {
    const IMPLEMENTED: &[&str] = &[
        "KHR_texture_transform",
        "KHR_materials_emissive_strength",
        "KHR_materials_ior",
        "KHR_materials_transmission",
        "KHR_materials_volume",
        "KHR_materials_clearcoat",
        "KHR_materials_specular",
        "KHR_materials_sheen",
        "KHR_materials_anisotropy",
        "KHR_materials_iridescence",
    ];

    let names = |key: &str| -> Vec<String> {
        root.get(key)
            .map(|v| v.as_array())
            .unwrap_or(&[])
            .iter()
            .filter_map(|v| v.as_str())
            .filter(|name| !IMPLEMENTED.contains(name))
            .map(|name| name.to_string())
            .collect()
    };

    let required = names("extensionsRequired");
    if !required.is_empty() {
        warnings.push(format!(
            "this model requires {}, which this reader does not implement; it \
             will import, but not as the file describes it",
            required.join(", ")
        ));
    }
    // Only the ones not already named above, so a required extension is
    // reported once rather than twice in different words.
    let merely_used: Vec<String> = names("extensionsUsed")
        .into_iter()
        .filter(|name| !required.contains(name))
        .collect();
    if !merely_used.is_empty() {
        warnings.push(format!(
            "this model uses {}, which this reader does not implement; those \
             materials will differ from the file",
            merely_used.join(", ")
        ));
    }
}

fn load_buffers(
    root: &json::Value,
    binary_chunk: Option<&[u8]>,
    resolve: &dyn Fn(&str) -> Option<Vec<u8>>,
    warnings: &mut Vec<String>,
) -> Result<Vec<Vec<u8>>, ImportError> {
    let mut out = Vec::new();
    for (i, buffer) in root
        .get("buffers")
        .map(|b| b.as_array())
        .unwrap_or(&[])
        .iter()
        .enumerate()
    {
        let declared = buffer
            .get("byteLength")
            .and_then(|v| v.as_index())
            .unwrap_or(0);
        let from_binary_chunk = buffer.get("uri").is_none();
        let data = match buffer.get("uri").and_then(|v| v.as_str()) {
            // No URI means the GLB's own binary chunk, which only buffer 0 may
            // claim.
            None => binary_chunk.map(|c| c.to_vec()).ok_or_else(|| {
                ImportError::Malformed(format!(
                    "buffer {i} has no URI, but the file has no binary chunk"
                ))
            })?,
            Some(uri) if uri.starts_with("data:") => decode_data_uri(uri).ok_or_else(|| {
                ImportError::Malformed(format!("buffer {i} has a malformed data URI"))
            })?,
            Some(uri) => resolve(&percent_decode(uri)).ok_or_else(|| {
                ImportError::Missing(format!(
                    "buffer {i} refers to `{uri}`, which could not be read"
                ))
            })?,
        };
        if declared > data.len() {
            return Err(ImportError::Malformed(format!(
                "buffer {i} declares {declared} bytes but only {} are available",
                data.len()
            )));
        }
        // A GLB's binary chunk is padded to a four-byte boundary, and the
        // specification says so: its length "MAY be up to 3 bytes bigger than
        // JSON-defined buffer.byteLength". Those bytes are padding, not data,
        // and complaining about them means every GLB whose buffer is not
        // already a multiple of four arrives with a warning it has done
        // nothing to deserve. Anything beyond three bytes is still worth
        // saying, and so is any excess at all in a buffer that came from a
        // file or a data URI, where no padding rule applies.
        let slack = if from_binary_chunk { 3 } else { 0 };
        if declared != 0 && declared + slack < data.len() {
            warnings.push(format!(
                "buffer {i} is longer than its declared {declared} bytes; the extra is ignored"
            ));
        }
        out.push(data);
    }
    Ok(out)
}

/// `data:application/octet-stream;base64,...`
fn decode_data_uri(uri: &str) -> Option<Vec<u8>> {
    let (header, payload) = uri.split_once(',')?;
    if !header.contains(";base64") {
        // Only base64 payloads occur in practice, and a plain-text one would
        // need percent-decoding into arbitrary bytes.
        return None;
    }
    base64_decode(payload)
}

fn base64_decode(text: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lookup = [0xffu8; 256];
    for (i, &c) in TABLE.iter().enumerate() {
        lookup[c as usize] = i as u8;
    }

    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut accumulator = 0u32;
    let mut bits = 0u32;
    for byte in text.bytes() {
        if byte == b'=' || byte.is_ascii_whitespace() {
            continue;
        }
        let value = lookup[byte as usize];
        if value == 0xff {
            return None;
        }
        accumulator = (accumulator << 6) | value as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((accumulator >> bits) as u8);
        }
    }
    Some(out)
}

/// URIs are percent-encoded, and part names with spaces are common enough that
/// skipping this loses real files.
pub(crate) fn percent_decode(uri: &str) -> String {
    let bytes = uri.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| (b as char).to_digit(16);
            if let (Some(high), Some(low)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((high * 16 + low) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// --- materials -----------------------------------------------------------

/// Map glTF's PBR metallic-roughness onto the OpenPBR model.
///
/// §6.1 point 8 asks for these to be mapped exactly rather than approximated,
/// and the scalar parameters are: `baseColorFactor`, `roughnessFactor` and
/// `emissiveFactor` transfer unchanged.
///
/// glTF's metallic-roughness parameters map one-to-one onto §2.2, so nothing
/// here is approximated. M1 thresholded `metallicFactor` at 0.5 and warned
/// about the ambiguous middle, because the material model then had four fixed
/// kinds and no way to say "60% metallic"; `base_metalness` is a real
/// parameter now and carries the value through unchanged.
/// An image a material asked for, and what it is for.
///
/// Resolved to a path on disk rather than decoded here: `TextureSource` holds
/// a path, the renderer's cache decodes it when something asks to shade, and
/// an importer that decoded eagerly would stall on a scene full of 4K maps
/// that the user may never render.
pub struct ImportedTexture {
    pub source: tequila_scene::textures::TextureSource,
}

/// The URI of the image at this index, if it is one this importer can use.
///
/// Only external files. A `data:` URI is base64 inside the JSON and a GLB
/// stores images in its binary chunk; both are real and neither is what a
/// downloaded model directory looks like, so they are skipped with a warning
/// rather than half-supported.
fn image_source(
    root: &json::Value,
    buffers: &[Vec<u8>],
    image_index: usize,
    role: tequila_texture::texture::MapRole,
    warnings: &mut Vec<String>,
) -> Option<crate::raw::RawTexture> {
    let image = root
        .get("images")
        .map(|v| v.as_array())
        .unwrap_or(&[])
        .get(image_index)?;

    // A file beside the model: the common case, and the only one that was
    // handled. Nothing is decoded here — the name is passed on and `import`
    // resolves it, because reading a model touches no filesystem.
    if let Some(uri) = image.get("uri").and_then(|v| v.as_str()) {
        if !uri.starts_with("data:") {
            return Some(crate::raw::RawTexture {
                uri: uri.to_string(),
                role,
                embedded: None,
            });
        }
    }

    // Otherwise the bytes are in the file. Two ways in: a `data:` URI, or a
    // view into the binary chunk, which is what a `.glb` uses.
    let bytes = match image.get("uri").and_then(|v| v.as_str()) {
        Some(uri) => match decode_data_uri(uri) {
            Some(bytes) => bytes,
            None => {
                warnings.push(format!(
                    "image {image_index} has a data URI this reader cannot \
                     decode; that map will not appear"
                ));
                return None;
            }
        },
        None => {
            let Some(view) = image.get("bufferView").and_then(|v| v.as_index()) else {
                warnings.push(format!(
                    "image {image_index} has neither a URI nor a bufferView; \
                     that map will not appear"
                ));
                return None;
            };
            match buffer_view_bytes(root, buffers, view) {
                Some(bytes) => bytes.to_vec(),
                None => {
                    warnings.push(format!(
                        "image {image_index} points at bufferView {view}, which \
                         is not in the file; that map will not appear"
                    ));
                    return None;
                }
            }
        }
    };

    let Some(extension) = image_extension(image.get("mimeType").and_then(|v| v.as_str()), &bytes)
    else {
        warnings.push(format!(
            "image {image_index} is embedded in a format this reader does not \
             recognise; that map will not appear"
        ));
        return None;
    };
    Some(crate::raw::RawTexture {
        uri: format!("{}.{extension}", embedded_name(image, image_index)),
        role,
        embedded: Some(std::sync::Arc::new(bytes)),
    })
}

/// A bufferView's bytes.
///
/// No stride: an image occupies its view whole, and the specification forbids
/// `byteStride` on a view an image uses.
fn buffer_view_bytes<'a>(
    root: &json::Value,
    buffers: &'a [Vec<u8>],
    view_index: usize,
) -> Option<&'a [u8]> {
    let view = root.get("bufferViews").and_then(|v| v.index(view_index))?;
    let buffer = buffers.get(view.get("buffer").and_then(|v| v.as_index()).unwrap_or(0))?;
    let offset = view.get("byteOffset").and_then(|v| v.as_index()).unwrap_or(0);
    let length = view.get("byteLength").and_then(|v| v.as_index())?;
    buffer.get(offset..offset.checked_add(length)?)
}

/// The extension to save an embedded image under.
///
/// `mimeType` is required by the specification for a `bufferView` image and
/// optional on a data URI, and exporters do leave it off, so the bytes get the
/// final say. Extension rather than format because the decoder dispatches on
/// extension: writing a JPEG out as `.png` would make it unreadable by a path
/// that has the decoder for it right there.
fn image_extension(mime: Option<&str>, bytes: &[u8]) -> Option<&'static str> {
    match mime {
        Some("image/png") => return Some("png"),
        Some("image/jpeg") => return Some("jpg"),
        _ => {}
    }
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else {
        None
    }
}

/// A filename for an image that never had one.
///
/// The glTF's own `name` where there is one, so that extracted files are
/// recognisable rather than a row of numbers. Reduced to characters that are
/// safe in a filename on every platform: the name is arbitrary text from a
/// file that may not be friendly, and it is about to become a path.
fn embedded_name(image: &json::Value, index: usize) -> String {
    let cleaned: String = image
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    let cleaned = cleaned.trim_matches('_');
    if cleaned.is_empty() {
        format!("image_{index}")
    } else {
        format!("{cleaned}_{index}")
    }
}

/// A texture reference: which image, and how its coordinates are transformed.
///
/// `KHR_texture_transform` is the extension an exporter writes whenever it
/// atlases or tiles, which is often. Ignoring it does not fail — it renders
/// the whole atlas onto every part, or tiles once where the model asked for
/// eight — so it belongs with the other silent wrongness this importer has
/// been shedding.
fn texture_reference(
    root: &json::Value,
    reference: Option<&json::Value>,
    warnings: &mut Vec<String>,
) -> Option<(usize, tequila_texture::projection::Mapping)> {
    let reference = reference?;
    let image = texture_image(root, Some(reference))?;
    Some((image, texture_transform(reference, warnings)))
}

/// `KHR_texture_transform`, as a [`Mapping`](tequila_texture::projection::Mapping).
///
/// The extension composes its matrix as `translation * rotation * scale`, and
/// `Mapping` applies scale, then rotation, then offset — the same order, so
/// the three numbers transfer directly rather than needing to be recomposed.
///
/// The rotation sign follows from reading the specification's matrices as
/// column-major, which its flat arrays are: the first column `(cos, sin, 0)`
/// and the second `(-sin, cos, 0)` give the same matrix `Mapping` applies, so
/// the angle carries over unchanged but for radians becoming turns. That
/// reading is the one assumption here, and the test named after it is what
/// would catch it being wrong.
fn texture_transform(
    reference: &json::Value,
    warnings: &mut Vec<String>,
) -> tequila_texture::projection::Mapping {
    use tequila_core::math::{vec2, vec3};
    let mut mapping = tequila_texture::projection::Mapping::default();

    // Only UV set 0 reaches the shading, so a reference that asks for another
    // one would be textured from coordinates it did not choose.
    if reference.get("texCoord").and_then(|v| v.as_f32()).unwrap_or(0.0) != 0.0 {
        warnings.push(
            "a material asks for a second UV set, which this renderer does \
             not carry yet; the first one was used"
                .into(),
        );
    }

    let Some(transform) = reference
        .get("extensions")
        .and_then(|e| e.get("KHR_texture_transform"))
    else {
        return mapping;
    };

    if let Some(scale) = transform.get("scale").and_then(|v| v.as_f32_array()) {
        if scale.len() >= 2 {
            mapping.scale = vec3(scale[0], scale[1], 1.0);
        }
    }
    if let Some(offset) = transform.get("offset").and_then(|v| v.as_f32_array()) {
        if offset.len() >= 2 {
            mapping.offset = vec2(offset[0], offset[1]);
        }
    }
    if let Some(rotation) = transform.get("rotation").and_then(|v| v.as_f32()) {
        mapping.rotation = rotation / std::f32::consts::TAU;
    }
    if transform
        .get("texCoord")
        .and_then(|v| v.as_f32())
        .unwrap_or(0.0)
        != 0.0
    {
        warnings.push(
            "a texture transform redirects to a second UV set, which this \
             renderer does not carry yet; the first one was used"
                .into(),
        );
    }
    mapping
}

/// The image index a texture reference resolves to.
fn texture_image(root: &json::Value, reference: Option<&json::Value>) -> Option<usize> {
    let index = reference?.get("index")?.as_f32()? as usize;
    let texture = root
        .get("textures")
        .map(|v| v.as_array())
        .unwrap_or(&[])
        .get(index)?;
    Some(texture.get("source")?.as_f32()? as usize)
}

fn read_materials(
    root: &json::Value,
    buffers: &[Vec<u8>],
    textures: &mut Vec<crate::raw::RawTexture>,
    warnings: &mut Vec<String>,
) -> Vec<MaterialDefinition> {
    root.get("materials")
        .map(|m| m.as_array())
        .unwrap_or(&[])
        .iter()
        .enumerate()
        .map(|(i, source)| {
            let name = source
                .get("name")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("Material {}", i + 1));

            let pbr = source.get("pbrMetallicRoughness");
            let base = pbr
                .and_then(|p| p.get("baseColorFactor"))
                .and_then(|v| v.as_f32_array())
                .filter(|v| v.len() >= 3)
                .map(|v| linear_srgb(v[0], v[1], v[2]))
                .unwrap_or(spectrum(1.0, 1.0, 1.0));
            let metallic = pbr
                .and_then(|p| p.get("metallicFactor"))
                .and_then(|v| v.as_f32())
                .unwrap_or(1.0);
            let roughness = pbr
                .and_then(|p| p.get("roughnessFactor"))
                .and_then(|v| v.as_f32())
                .unwrap_or(1.0)
                .clamp(0.0, 1.0);
            // §alphaMode. `OPAQUE` — the default — says to ignore alpha
            // altogether, including the factor's, so a model that happens to
            // carry a transparent base colour still renders solid.
            let transparent = source
                .get("alphaMode")
                .and_then(|v| v.as_str())
                .is_some_and(|mode| mode == "MASK" || mode == "BLEND");
            let base_alpha = if transparent {
                pbr.and_then(|p| p.get("baseColorFactor"))
                    .and_then(|v| v.as_f32_array())
                    .filter(|v| v.len() >= 4)
                    .map(|v| v[3].clamp(0.0, 1.0))
                    .unwrap_or(1.0)
            } else {
                1.0
            };
            // A threshold this renderer does not apply, because it reads
            // alpha as coverage instead — see `attach_alpha`. Only a
            // non-default one is worth saying, since exporters write the
            // default out constantly.
            if transparent {
                if let Some(cutoff) = source.get("alphaCutoff").and_then(|v| v.as_f32()) {
                    if (cutoff - 0.5).abs() > 1e-6 {
                        warnings.push(format!(
                            "a material sets alphaCutoff to {cutoff}; this renderer \
                             reads alpha as coverage rather than testing it \
                             against a threshold, so the edge will be softer \
                             than the file asks"
                        ));
                    }
                }
            }
            let emission = source
                .get("emissiveFactor")
                .and_then(|v| v.as_f32_array())
                .filter(|v| v.len() >= 3)
                .map(|v| linear_srgb(v[0], v[1], v[2]))
                .unwrap_or(tequila_core::spectrum::Spectrum::ZERO);

            // glTF's metallic-roughness model is a strict subset of §2.2,
            // so this is a direct mapping rather than an approximation. It
            // used to round `metallicFactor` to metal-or-plastic and warn
            // about anything in between; there is nothing to round now,
            // because `base_metalness` interpolates exactly as glTF says it
            // should.
            let material = Material {
                base_color: base,
                base_metalness: metallic,
                specular_roughness: roughness,
                // glTF's baseColor is the conductor tint on the metallic side
                // and the albedo on the dielectric side — which is what
                // `base_color` means in both halves of §2.2's base.
                metal_ior: metals::ALUMINIUM,
                emission_color: emission,
                emission_luminance: if emission.is_black() { 0.0 } else { 1.0 },
                geometry_opacity: base_alpha,
                ..Material::plastic(base, roughness)
            };
            let mut material = material;
            read_material_extensions(source, &mut material, warnings);
            // glTF tints the light a transmissive surface passes by its base
            // colour, as Blender's principled glass does; §2.2 tints it by
            // the transmission colour, which on a surface with no absorbing
            // volume is exactly that tint. Left white, a dirty bulb's glass
            // came in spotless: its grime is base colour.
            if tints_transmission(&material) {
                material.transmission_color = material.base_color;
            }

            // The catalog entry is chosen for the panel the user will land in,
            // not for how it shades: the parameters above already say that.
            let material_type = if metallic >= 0.5 {
                MaterialType::Metal
            } else {
                MaterialType::Plastic
            };
            let mut definition = MaterialDefinition::new(name, material_type, material);
            attach_maps(
                root, buffers, source, transparent, &mut definition, textures, warnings,
            );
            definition
        })
        .collect()
}

/// Whether a material's base colour is also what its transmitted light is
/// tinted by: transmissive, with no absorbing volume already saying what
/// colour the light becomes.
fn tints_transmission(material: &Material) -> bool {
    material.transmission_weight > 0.0 && material.transmission_depth <= 0.0
}

/// The `KHR_materials_*` extensions whose parameters §2.2 already has.
///
/// glTF's core material is metallic-roughness and nothing else; everything a
/// product render usually wants — glass, a clear coat, a measured IOR, a
/// luminance for an emitter — arrives as an extension. Ignoring them is not a
/// failure with a message, it is a model that comes in looking like plastic.
///
/// Only the ones that map onto a parameter this renderer implements are read.
/// An extension whose parameter is stored and unread would be a worse outcome
/// than not reading it, because the file would look honoured and the render
/// would not change; `Param::unimplemented` is the list of those, and
/// `report_unsupported_extensions` is what says so.
fn read_material_extensions(
    source: &json::Value,
    material: &mut Material,
    warnings: &mut Vec<String>,
) {
    let Some(extensions) = source.get("extensions") else {
        return;
    };
    let number = |ext: &json::Value, key: &str| ext.get(key).and_then(|v| v.as_f32());
    let colour = |ext: &json::Value, key: &str| {
        ext.get(key)
            .and_then(|v| v.as_f32_array())
            .filter(|v| v.len() >= 3)
            .map(|v| linear_srgb(v[0], v[1], v[2]))
    };

    // Emission in physical units. The factor alone is a 0..1 colour with no
    // scale, and this is the extension that gives it one.
    if let Some(ext) = extensions.get("KHR_materials_emissive_strength") {
        if let Some(strength) = number(ext, "emissiveStrength") {
            if material.emission_luminance > 0.0 {
                material.emission_luminance = strength.max(0.0);
            }
        }
    }

    if let Some(ext) = extensions.get("KHR_materials_ior") {
        if let Some(ior) = number(ext, "ior") {
            material.specular_ior = ior.clamp(1.0, 3.0);
        }
    }

    if let Some(ext) = extensions.get("KHR_materials_transmission") {
        material.transmission_weight = number(ext, "transmissionFactor")
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);
        // Without a volume the specification's transmission is thin-walled:
        // a shell light passes through, not a block it refracts in — a bulb,
        // a window. Read as solid, a bulb's single-surface glass bent every
        // ray as a lens would and turned the filament behind it into a
        // smear. `KHR_materials_volume`, below, says otherwise where there
        // is one.
        if extensions.get("KHR_materials_volume").is_none() {
            material.geometry_thin_walled = true;
        }
        if ext.get("transmissionTexture").is_some() {
            warnings.push(
                "a material varies its transmission with a texture, which this \
                 importer does not wire yet; the single factor was used"
                    .into(),
            );
        }
    }

    // The other half of glass. `transmission` says light passes through;
    // this says what happens to it on the way, and §2.2 spells the two
    // parameters the same way glTF does — a colour reached at a distance.
    if let Some(ext) = extensions.get("KHR_materials_volume") {
        // `thicknessFactor` of zero is the specification's way of saying
        // thin-walled, which is exactly §2.2's distinction between a pane
        // and a block.
        let thickness = number(ext, "thicknessFactor").unwrap_or(0.0);
        material.geometry_thin_walled = thickness <= 0.0;

        if let Some(tint) = colour(ext, "attenuationColor") {
            material.transmission_color = tint;
        }
        // The default is infinity — no absorption however far light
        // travels — and §2.2 writes that as a depth of zero. A thin wall
        // has no interior to absorb in either way.
        let distance = number(ext, "attenuationDistance").unwrap_or(f32::INFINITY);
        material.transmission_depth = if thickness > 0.0 && distance.is_finite() {
            distance.max(0.0)
        } else {
            0.0
        };
    }

    if let Some(ext) = extensions.get("KHR_materials_clearcoat") {
        material.coat_weight = number(ext, "clearcoatFactor").unwrap_or(0.0).clamp(0.0, 1.0);
        material.coat_roughness = number(ext, "clearcoatRoughnessFactor")
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);
    }

    if let Some(ext) = extensions.get("KHR_materials_specular") {
        if let Some(weight) = number(ext, "specularFactor") {
            material.specular_weight = weight.clamp(0.0, 1.0);
        }
        if let Some(tint) = colour(ext, "specularColorFactor") {
            material.specular_color = tint;
        }
    }

    // glTF folds sheen's strength into its colour; §2.2 keeps the two apart,
    // so the brightest channel becomes the weight and the colour is normalized
    // against it. A black sheen colour means no sheen either way.
    if let Some(ext) = extensions.get("KHR_materials_sheen") {
        // The brightest channel as the file states it, so a factor of 0.4
        // is a weight of 0.4 whichever space the tint is then held in.
        if let Some(tint) = colour(ext, "sheenColorFactor") {
            let stated = tint.to_linear_srgb();
            let peak = stated.x.max(stated.y).max(stated.z);
            if peak > 1e-4 {
                material.fuzz_weight = peak.clamp(0.0, 1.0);
                material.fuzz_color = tint * (1.0 / peak);
            }
        }
        if let Some(roughness) = number(ext, "sheenRoughnessFactor") {
            material.fuzz_roughness = roughness.clamp(0.0, 1.0);
        }
    }

    // §2.2's thin film, which glTF calls iridescence. The two mean the same
    // thing by strength and by film index; they differ over thickness, which
    // glTF gives as a *range* plus a texture that picks a point in it, where
    // §2.2 has one number. With no texture the specification says the
    // thickness is the maximum, so that is what arrives.
    if let Some(ext) = extensions.get("KHR_materials_iridescence") {
        material.thin_film_weight = number(ext, "iridescenceFactor")
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);
        // glTF's default film index is 1.3, not the 1.5 that most of its
        // other indices default to.
        material.thin_film_ior = number(ext, "iridescenceIor")
            .unwrap_or(1.3)
            .clamp(1.0, 3.0);
        material.thin_film_thickness = number(ext, "iridescenceThicknessMaximum")
            .unwrap_or(400.0)
            .clamp(0.0, 2000.0);

        if ext.get("iridescenceTexture").is_some() {
            warnings.push(
                "a material varies its iridescence strength with a texture, which this importer does not wire yet; the single factor was used"
                    .into(),
            );
        }
        // Worth its own warning rather than sharing one: a thickness map is
        // how an exporter paints the *pattern* of an oil film or a tempered
        // finish, so a model that has one is a model whose iridescence is
        // meant to vary across the surface. Ignoring it leaves the film
        // uniform, which looks like a different material rather than like a
        // detail gone missing.
        if ext.get("iridescenceThicknessTexture").is_some() {
            warnings.push(format!(
                "a material varies its film thickness with a texture, which this importer does not wire yet; the maximum of {:.0} nm was used everywhere",
                material.thin_film_thickness
            ));
        }
    }

    if let Some(ext) = extensions.get("KHR_materials_anisotropy") {
        if let Some(strength) = number(ext, "anisotropyStrength") {
            material.specular_roughness_anisotropy = strength.clamp(0.0, 1.0);
        }
        // Radians from the tangent towards the bitangent, which is the way
        // the renderer turns the mesh's tangent too; ours runs 0 to 1 over the
        // half turn after which the lobe repeats.
        if let Some(radians) = number(ext, "anisotropyRotation") {
            material.specular_rotation = (radians / std::f32::consts::PI).rem_euclid(1.0);
        }
    }
}

/// A colour a glTF file states: linear, with sRGB's primaries, as its
/// textures and vertex colours are. Held in the rendering space.
fn linear_srgb(r: f32, g: f32, b: f32) -> tequila_core::spectrum::Spectrum {
    tequila_core::spectrum::Spectrum::from_linear_srgb(tequila_core::math::vec3(r, g, b))
}

/// Wire a glTF material's images into its graph.
///
/// Base colour, normal, the packed metallic-roughness map, and occlusion.
///
/// glTF puts occlusion in red, roughness in green and metalness in blue, and a
/// channel node takes each out separately. Fed whole to any one of them the
/// image would arrive as the luminance of all three, which is none of the
/// three things it packs.
///
/// `occlusionTexture` and `metallicRoughnessTexture` are usually the same file
/// — the ORM packing every exporter emits — and a glTF says so by pointing two
/// texture references at one image. The textures are deduplicated by file and
/// role, so that file is still decoded once and sampled three times.
#[allow(clippy::too_many_arguments)]
fn attach_maps(
    root: &json::Value,
    buffers: &[Vec<u8>],
    source: &json::Value,
    // Whether `alphaMode` asked for the base colour's alpha to be honoured.
    transparent: bool,
    definition: &mut MaterialDefinition,
    textures: &mut Vec<crate::raw::RawTexture>,
    warnings: &mut Vec<String>,
) {
    use tequila_materials::graph::{Channel, Value};
    use tequila_materials::material_type::Param;
    use tequila_texture::texture::MapRole;

    let pbr = source.get("pbrMetallicRoughness");

    if let Some((image, mapping)) =
        texture_reference(root, pbr.and_then(|p| p.get("baseColorTexture")), warnings)
    {
        let tint = definition.material().base_color;
        attach_image(
            root,
            buffers,
            image,
            mapping,
            Param::BaseColor,
            MapRole::Color,
            Value::Color(tint),
            definition,
            textures,
            warnings,
        );
        // And what the glass tints the light it passes, where the base
        // colour is that tint (see `tints_transmission`).
        if tints_transmission(definition.material()) {
            attach_image(
                root,
                buffers,
                image,
                mapping,
                Param::TransmissionColor,
                MapRole::Color,
                Value::Color(tint),
                definition,
                textures,
                warnings,
            );
        }
        if transparent {
            attach_alpha(
                root,
                buffers,
                image,
                mapping,
                definition.material().geometry_opacity,
                definition,
                textures,
                warnings,
            );
        }
    }
    // What glows, and in what colour: a filament's hot spot, a screen, an
    // indicator. Left out, the emission factor and strength lit the whole
    // surface — a bulb's entire screw base shone white.
    if let Some((image, mapping)) = texture_reference(root, source.get("emissiveTexture"), warnings) {
        let tint = definition.material().emission_color;
        attach_image(
            root,
            buffers,
            image,
            mapping,
            Param::EmissionColor,
            MapRole::Color,
            Value::Color(tint),
            definition,
            textures,
            warnings,
        );
    }
    if let Some((image, mapping)) = texture_reference(root, source.get("normalTexture"), warnings)
    {
        attach_image(
            root,
            buffers,
            image,
            mapping,
            Param::Bump,
            MapRole::Normal,
            Value::Float(0.0),
            definition,
            textures,
            warnings,
        );
    }
    if let Some((image, mapping)) = texture_reference(
        root,
        pbr.and_then(|p| p.get("metallicRoughnessTexture")),
        warnings,
    ) {
        // Data, not colour: these are numbers the shading model reads, not
        // something anyone looks at, so decoding them through an sRGB curve
        // would bend every roughness in the model.
        // glTF multiplies the map by the factor rather than replacing it, and
        // both factors default to 1 when absent. Passing the factors in keeps
        // a model that sets `metallicFactor: 0.5` half as metallic as its map
        // says, instead of exactly as metallic.
        let roughness_factor = definition.material().specular_roughness;
        let metallic_factor = definition.material().base_metalness;
        attach_channel(
            root,
            buffers,
            image,
            mapping,
            Channel::Green,
            Param::SpecularRoughness,
            roughness_factor,
            definition,
            textures,
            warnings,
        );
        attach_channel(
            root,
            buffers,
            image,
            mapping,
            Channel::Blue,
            Param::BaseMetalness,
            metallic_factor,
            definition,
            textures,
            warnings,
        );
    }
    if let Some(reference) = source.get("occlusionTexture") {
        if let Some((image, mapping)) = texture_reference(root, Some(reference), warnings) {
            attach_occlusion(
                root,
                buffers,
                image,
                mapping,
                reference
                    .get("strength")
                    .and_then(|v| v.as_f32())
                    .unwrap_or(1.0),
                definition,
                textures,
                warnings,
            );
        }
    }
}

/// A whole image driving one parameter.
#[allow(clippy::too_many_arguments)]
fn attach_image(
    root: &json::Value,
    buffers: &[Vec<u8>],
    image: usize,
    mapping: tequila_texture::projection::Mapping,
    param: tequila_materials::material_type::Param,
    role: tequila_texture::texture::MapRole,
    fallback: tequila_materials::graph::Value,
    definition: &mut MaterialDefinition,
    textures: &mut Vec<crate::raw::RawTexture>,
    warnings: &mut Vec<String>,
) {
    use tequila_materials::graph::{Node, NodeKind, Socket, TextureId};

    let Some(index) = texture_slot(root, buffers, image, role, textures, warnings) else {
        return;
    };
    let graph = &mut definition.graph;
    let coords = graph.add_node(Node::new(NodeKind::Mapping2d { mapping }));
    let node = graph.add_node(Node::new(NodeKind::ImageTexture {
        texture: TextureId(index as u32),
        fallback,
    }));
    let output = graph.output();
    graph.connect(coords, node, Socket::Input(0));
    graph.connect(node, output, Socket::Param(param));
}

/// The base colour image's own alpha, driving opacity.
///
/// Where an alpha-tested model keeps its holes: a leaf, a grille, a decal on a
/// transparent sheet. The mask is the fourth channel of the colour picture
/// rather than a file of its own, and it shares that picture's mapping exactly
/// — anything else slides the holes off the leaf.
///
/// # `MASK` is read as coverage, not as a hard test
///
/// glTF asks for `alphaMode: MASK` to compare alpha against `alphaCutoff` and
/// snap to fully present or fully gone. That rule exists for renderers that
/// cannot do partial coverage at all; this one can, because a cutout here is
/// resolved stochastically and averages over samples. Taking the filtered
/// alpha as coverage keeps the antialiased edge the texture's author drew,
/// where snapping would throw it away and hand back the stair-stepped edge
/// alpha testing is known for. `BLEND` reads the same way, and for the same
/// reason.
///
/// The cost of that choice is `alphaCutoff`: a model that sets one other than
/// the default is asking for a threshold this does not apply, so it says so.
#[allow(clippy::too_many_arguments)]
fn attach_alpha(
    root: &json::Value,
    buffers: &[Vec<u8>],
    image: usize,
    mapping: tequila_texture::projection::Mapping,
    factor: f32,
    definition: &mut MaterialDefinition,
    textures: &mut Vec<crate::raw::RawTexture>,
    warnings: &mut Vec<String>,
) {
    use tequila_materials::graph::{Node, NodeKind, Socket, TextureId, Value};
    use tequila_materials::material_type::Param;

    // The colour role, because it is the same file as the base colour and a
    // second entry would decode the picture twice. `sample_alpha` takes the
    // channel the sRGB decode already left alone.
    let Some(index) = texture_slot(
        root,
        buffers,
        image,
        tequila_texture::texture::MapRole::Color,
        textures,
        warnings,
    ) else {
        return;
    };

    let graph = &mut definition.graph;
    let coords = graph.add_node(Node::new(NodeKind::Mapping2d { mapping }));
    let alpha = graph.add_node(Node::new(NodeKind::ImageAlpha {
        texture: TextureId(index as u32),
        fallback: Value::Float(factor),
    }));
    let output = graph.output();
    graph.connect(coords, alpha, Socket::Input(0));

    if (factor - 1.0).abs() < 1e-6 {
        graph.connect(alpha, output, Socket::Param(Param::Opacity));
        return;
    }
    // glTF multiplies the factor's alpha by the texture's.
    let scale = graph.add_node(Node::new(NodeKind::Constant(Value::Float(factor))));
    let product = graph.add_node(Node::new(NodeKind::Multiply));
    graph.connect(alpha, product, Socket::Input(0));
    graph.connect(scale, product, Socket::Input(1));
    graph.connect(product, output, Socket::Param(Param::Opacity));
}

/// glTF's occlusion map: red channel, scaled by `strength`.
///
/// `strength` is not a multiply. The specification defines it as
/// `lerp(1, occlusion, strength)`, so that turning it down fades towards
/// *unoccluded* rather than towards black — which a plain multiply would get
/// backwards, darkening the model the further the strength is lowered. A remap
/// of 0..1 onto (1 − strength)..1 is that lerp exactly.
fn attach_occlusion(
    root: &json::Value,
    buffers: &[Vec<u8>],
    image: usize,
    mapping: tequila_texture::projection::Mapping,
    strength: f32,
    definition: &mut MaterialDefinition,
    textures: &mut Vec<crate::raw::RawTexture>,
    warnings: &mut Vec<String>,
) {
    use tequila_core::math::vec2;
    use tequila_materials::graph::{Channel, Node, NodeKind, Socket, TextureId, Value};
    use tequila_materials::material_type::Param;

    let strength = strength.clamp(0.0, 1.0);
    let Some(index) = texture_slot(
        root,
        buffers,
        image,
        tequila_texture::texture::MapRole::Data,
        textures,
        warnings,
    ) else {
        return;
    };

    // Unoccluded is the fallback, so a map that fails to load leaves the
    // surface lit rather than in shadow.
    let graph = &mut definition.graph;
    let coords = graph.add_node(Node::new(NodeKind::Mapping2d { mapping }));
    let texture = graph.add_node(Node::new(NodeKind::ImageTexture {
        texture: TextureId(index as u32),
        fallback: Value::Float(1.0),
    }));
    let pick = graph.add_node(Node::new(NodeKind::Channel {
        channel: Channel::Red,
    }));
    let output = graph.output();
    graph.connect(coords, texture, Socket::Input(0));
    graph.connect(texture, pick, Socket::Input(0));

    if (strength - 1.0).abs() < 1e-6 {
        graph.connect(pick, output, Socket::Param(Param::Occlusion));
        return;
    }
    let faded = graph.add_node(Node::new(NodeKind::Remap {
        from: vec2(0.0, 1.0),
        to: vec2(1.0 - strength, 1.0),
    }));
    graph.connect(pick, faded, Socket::Input(0));
    graph.connect(faded, output, Socket::Param(Param::Occlusion));
}

/// The texture table slot for one image in one role.
///
/// One entry per distinct image and role, so an ORM image referenced by both
/// `occlusionTexture` and `metallicRoughnessTexture` is decoded once and
/// sampled three times. The same file in two *roles* still gets two entries,
/// because the two decode differently (`docs/file-format.md` §3.1).
fn texture_slot(
    root: &json::Value,
    buffers: &[Vec<u8>],
    image: usize,
    role: tequila_texture::texture::MapRole,
    textures: &mut Vec<crate::raw::RawTexture>,
    warnings: &mut Vec<String>,
) -> Option<usize> {
    let wanted = image_source(root, buffers, image, role, warnings)?;
    Some(
        textures
            .iter()
            .position(|existing| existing.same_image(&wanted))
            .unwrap_or_else(|| {
                textures.push(wanted);
                textures.len() - 1
            }),
    )
}

/// One channel of an image, driving one parameter.
#[allow(clippy::too_many_arguments)]
fn attach_channel(
    root: &json::Value,
    buffers: &[Vec<u8>],
    image: usize,
    mapping: tequila_texture::projection::Mapping,
    channel: tequila_materials::graph::Channel,
    param: tequila_materials::material_type::Param,
    // What glTF says to multiply the channel by.
    factor: f32,
    definition: &mut MaterialDefinition,
    textures: &mut Vec<crate::raw::RawTexture>,
    warnings: &mut Vec<String>,
) {
    use tequila_materials::graph::{Node, NodeKind, Socket, TextureId, Value};

    let Some(index) = texture_slot(
        root,
        buffers,
        image,
        tequila_texture::texture::MapRole::Data,
        textures,
        warnings,
    ) else {
        return;
    };

    // The factor is the fallback, so a map that fails to load leaves the
    // material exactly as the file described it without one.
    let graph = &mut definition.graph;
    let coords = graph.add_node(Node::new(NodeKind::Mapping2d { mapping }));
    let texture = graph.add_node(Node::new(NodeKind::ImageTexture {
        texture: TextureId(index as u32),
        fallback: Value::Float(factor),
    }));
    let pick = graph.add_node(Node::new(NodeKind::Channel { channel }));
    let output = graph.output();
    graph.connect(coords, texture, Socket::Input(0));
    graph.connect(texture, pick, Socket::Input(0));

    // A factor of 1 is every model that leaves it out, so the multiply is
    // skipped there rather than leaving a node that does nothing in every
    // graph anyone opens.
    if (factor - 1.0).abs() < 1e-6 {
        graph.connect(pick, output, Socket::Param(param));
        return;
    }
    let scale = graph.add_node(Node::new(NodeKind::Constant(Value::Float(factor))));
    let product = graph.add_node(Node::new(NodeKind::Multiply));
    graph.connect(pick, product, Socket::Input(0));
    graph.connect(scale, product, Socket::Input(1));
    graph.connect(product, output, Socket::Param(param));
}

// --- meshes --------------------------------------------------------------

/// A glTF mesh, which is a list of primitives each with its own material.
struct GltfMesh {
    /// Index of each primitive in the flattened `RawScene::meshes`.
    first: usize,
    primitives: Vec<RawMesh>,
}

fn read_meshes(
    root: &json::Value,
    buffers: &[Vec<u8>],
    warnings: &mut Vec<String>,
) -> Result<Vec<GltfMesh>, ImportError> {
    let mut out = Vec::new();
    let mut next = 0;

    for (i, mesh) in root
        .get("meshes")
        .map(|m| m.as_array())
        .unwrap_or(&[])
        .iter()
        .enumerate()
    {
        let name = mesh
            .get("name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("Mesh {}", i + 1));

        let mut primitives = Vec::new();
        for (p, primitive) in mesh
            .get("primitives")
            .map(|p| p.as_array())
            .unwrap_or(&[])
            .iter()
            .enumerate()
        {
            let mode = primitive
                .get("mode")
                .and_then(|v| v.as_index())
                .unwrap_or(MODE_TRIANGLES);
            if mode != MODE_TRIANGLES {
                warnings.push(format!(
                    "`{name}` primitive {p} uses mode {mode}; only triangles are rendered"
                ));
                continue;
            }

            let attributes = primitive.get("attributes");
            let Some(position_accessor) = attributes
                .and_then(|a| a.get("POSITION"))
                .and_then(|v| v.as_index())
            else {
                warnings.push(format!("`{name}` primitive {p} has no POSITION attribute"));
                continue;
            };

            let positions = read_vec3_accessor(root, buffers, position_accessor)?;
            let normals = attributes
                .and_then(|a| a.get("NORMAL"))
                .and_then(|v| v.as_index())
                .map(|a| read_vec3_accessor(root, buffers, a))
                .transpose()?
                .filter(|n| n.len() == positions.len())
                .unwrap_or_default();
            let uvs = attributes
                .and_then(|a| a.get("TEXCOORD_0"))
                .and_then(|v| v.as_index())
                .map(|a| read_vec2_accessor(root, buffers, a))
                .transpose()?
                .filter(|u| u.len() == positions.len())
                .unwrap_or_default();

            // COLOR_0: linear by the specification, three or four components,
            // floats or normalised integers.
            let colors = attributes
                .and_then(|a| a.get("COLOR_0"))
                .and_then(|v| v.as_index())
                .map(|a| read_color_accessor(root, buffers, a))
                .transpose()?
                .filter(|c| c.len() == positions.len())
                .unwrap_or_default();

            let indices = match primitive.get("indices").and_then(|v| v.as_index()) {
                Some(accessor) => read_index_accessor(root, buffers, accessor)?,
                // Non-indexed: the vertices are the triangles, in order.
                None => (0..positions.len() as u32).collect(),
            };
            for &index in &indices {
                if index as usize >= positions.len() {
                    return Err(ImportError::Malformed(format!(
                        "`{name}` primitive {p} indexes vertex {index} of {}",
                        positions.len()
                    )));
                }
            }

            primitives.push(RawMesh {
                positions,
                normals,
                uvs,
                colors,
                indices,
                edge_lines: Vec::new(),
                material: primitive.get("material").and_then(|v| v.as_index()),
                name: if mesh
                    .get("primitives")
                    .map(|p| p.as_array().len())
                    .unwrap_or(1)
                    > 1
                {
                    // Each primitive becomes its own node, since a node carries
                    // one material.
                    format!("{name} [{}]", p + 1)
                } else {
                    name.clone()
                },
            });
        }

        out.push(GltfMesh {
            first: next,
            primitives,
        });
        next = out
            .last()
            .map(|m| m.first + m.primitives.len())
            .unwrap_or(0);
    }
    Ok(out)
}

// --- accessors -----------------------------------------------------------

/// A decoded view of one accessor: where its elements live and how far apart.
struct AccessorView<'a> {
    data: &'a [u8],
    offset: usize,
    stride: usize,
    count: usize,
    component: usize,
    components_per_element: usize,
    normalized: bool,
}

fn accessor_view<'a>(
    root: &json::Value,
    buffers: &'a [Vec<u8>],
    index: usize,
) -> Result<AccessorView<'a>, ImportError> {
    let accessor = root
        .get("accessors")
        .and_then(|a| a.index(index))
        .ok_or_else(|| ImportError::Malformed(format!("accessor {index} does not exist")))?;

    let count = accessor
        .get("count")
        .and_then(|v| v.as_index())
        .unwrap_or(0);
    let component = accessor
        .get("componentType")
        .and_then(|v| v.as_index())
        .ok_or_else(|| ImportError::Malformed(format!("accessor {index} has no componentType")))?;
    let kind = accessor
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ImportError::Malformed(format!("accessor {index} has no type")))?;
    let components_per_element = match kind {
        "SCALAR" => 1,
        "VEC2" => 2,
        "VEC3" => 3,
        "VEC4" => 4,
        "MAT2" => 4,
        "MAT3" => 9,
        "MAT4" => 16,
        other => {
            return Err(ImportError::Malformed(format!(
                "accessor {index} has unknown type `{other}`"
            )))
        }
    };
    let component_size = match component {
        BYTE | UNSIGNED_BYTE => 1,
        SHORT | UNSIGNED_SHORT => 2,
        UNSIGNED_INT | FLOAT => 4,
        other => {
            return Err(ImportError::Malformed(format!(
                "accessor {index} has unknown componentType {other}"
            )))
        }
    };
    let element_size = component_size * components_per_element;
    let accessor_offset = accessor
        .get("byteOffset")
        .and_then(|v| v.as_index())
        .unwrap_or(0);

    // A sparse or view-less accessor reads as zeros, by specification.
    let Some(view_index) = accessor.get("bufferView").and_then(|v| v.as_index()) else {
        return Err(ImportError::Unsupported(format!(
            "accessor {index} has no bufferView; sparse accessors are not implemented"
        )));
    };
    let view = root
        .get("bufferViews")
        .and_then(|v| v.index(view_index))
        .ok_or_else(|| ImportError::Malformed(format!("bufferView {view_index} does not exist")))?;
    let buffer_index = view.get("buffer").and_then(|v| v.as_index()).unwrap_or(0);
    let data = buffers
        .get(buffer_index)
        .ok_or_else(|| ImportError::Malformed(format!("buffer {buffer_index} does not exist")))?;
    let view_offset = view
        .get("byteOffset")
        .and_then(|v| v.as_index())
        .unwrap_or(0);
    let view_length = view
        .get("byteLength")
        .and_then(|v| v.as_index())
        .unwrap_or(0);
    // Zero stride means tightly packed, which is not the same as zero.
    let stride = view
        .get("byteStride")
        .and_then(|v| v.as_index())
        .filter(|s| *s > 0)
        .unwrap_or(element_size);

    // The last element's *end*, which is what has to fit — with a stride, the
    // gap after the final element need not be present. Every step checked:
    // a count that overflows read as needing nothing, and the reader then
    // reserved room for 2^57 elements and took the application down.
    let past_the_end = || ImportError::Malformed(format!("accessor {index} reads past the end of buffer {buffer_index}"));
    let needed = match count {
        0 => Some(0),
        n => (n - 1).checked_mul(stride).and_then(|n| n.checked_add(element_size)),
    };
    let offset = view_offset.checked_add(accessor_offset).ok_or_else(past_the_end)?;
    let end = needed.and_then(|needed| offset.checked_add(needed)).ok_or_else(past_the_end)?;
    let view_end = view_offset.checked_add(view_length).ok_or_else(past_the_end)?;
    if end > data.len() || view_end > data.len() {
        return Err(past_the_end());
    }

    Ok(AccessorView {
        data,
        offset,
        stride,
        count,
        component,
        components_per_element,
        normalized: accessor
            .get("normalized")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    })
}

impl AccessorView<'_> {
    /// Element `e` of an integer accessor, exactly. Through `f32`, an index
    /// above 2^24 rounds to its neighbour, and a 20-million-vertex scan
    /// grows spikes where triangles reach the wrong vertices.
    fn integer(&self, e: usize) -> Option<u32> {
        let at = self.offset + e * self.stride;
        let d = self.data;
        match self.component {
            UNSIGNED_BYTE => Some(d[at] as u32),
            UNSIGNED_SHORT => Some(u16::from_le_bytes([d[at], d[at + 1]]) as u32),
            UNSIGNED_INT => Some(u32::from_le_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]])),
            _ => None,
        }
    }

    /// Component `c` of element `e`, as an `f32`.
    fn component(&self, e: usize, c: usize) -> f32 {
        let at = self.offset + e * self.stride + c * self.component_size();
        let d = self.data;
        match self.component {
            FLOAT => f32::from_le_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]]),
            UNSIGNED_BYTE => self.denormalize(d[at] as f32, 255.0),
            BYTE => self.denormalize(d[at] as i8 as f32, 127.0),
            UNSIGNED_SHORT => {
                self.denormalize(u16::from_le_bytes([d[at], d[at + 1]]) as f32, 65535.0)
            }
            SHORT => self.denormalize(i16::from_le_bytes([d[at], d[at + 1]]) as f32, 32767.0),
            UNSIGNED_INT => u32::from_le_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]]) as f32,
            _ => 0.0,
        }
    }

    /// glTF's `normalized` flag means integers encode `[0,1]` or `[-1,1]`.
    /// Ignoring it turns a quantized UV of 32767 into a texture coordinate of
    /// 32767 rather than 1.0.
    fn denormalize(&self, value: f32, scale: f32) -> f32 {
        if self.normalized {
            (value / scale).max(-1.0)
        } else {
            value
        }
    }

    fn component_size(&self) -> usize {
        match self.component {
            BYTE | UNSIGNED_BYTE => 1,
            SHORT | UNSIGNED_SHORT => 2,
            _ => 4,
        }
    }
}

fn read_vec3_accessor(
    root: &json::Value,
    buffers: &[Vec<u8>],
    index: usize,
) -> Result<Vec<Vec3>, ImportError> {
    let view = accessor_view(root, buffers, index)?;
    if view.components_per_element < 3 {
        return Err(ImportError::Malformed(format!(
            "accessor {index} has {} components where 3 were expected",
            view.components_per_element
        )));
    }
    (0..view.count)
        .map(|e| {
            let v = vec3(
                view.component(e, 0),
                view.component(e, 1),
                view.component(e, 2),
            );
            // §6.1 point 5: report non-finite positions rather than importing
            // them. One NaN vertex poisons the BVH and every ray near it.
            if v.is_finite() {
                Ok(v)
            } else {
                Err(ImportError::Malformed(format!(
                    "accessor {index} element {e} is not finite: {v:?}"
                )))
            }
        })
        .collect()
}

fn read_vec2_accessor(
    root: &json::Value,
    buffers: &[Vec<u8>],
    index: usize,
) -> Result<Vec<tequila_core::math::Vec2>, ImportError> {
    let view = accessor_view(root, buffers, index)?;
    if view.components_per_element < 2 {
        return Err(ImportError::Malformed(format!(
            "accessor {index} has {} components where 2 were expected",
            view.components_per_element
        )));
    }
    Ok((0..view.count)
        .map(|e| {
            let uv = vec2(view.component(e, 0), view.component(e, 1));
            // A non-finite UV is survivable — it costs a texture lookup, not
            // the BVH — so it is clamped rather than refused.
            if uv.x.is_finite() && uv.y.is_finite() {
                uv
            } else {
                vec2(0.0, 0.0)
            }
        })
        .collect())
}

fn read_color_accessor(
    root: &json::Value,
    buffers: &[Vec<u8>],
    index: usize,
) -> Result<Vec<tequila_core::math::Vec4>, ImportError> {
    let view = accessor_view(root, buffers, index)?;
    if view.components_per_element < 3 {
        return Err(ImportError::Malformed(format!(
            "colour accessor {index} has {} components where 3 or 4 were expected",
            view.components_per_element
        )));
    }
    let alpha = view.components_per_element >= 4;
    Ok((0..view.count)
        .map(|e| {
            let c = |k: usize| {
                let v = view.component(e, k);
                if v.is_finite() { v.max(0.0) } else { 0.0 }
            };
            tequila_core::math::vec4(c(0), c(1), c(2), if alpha { c(3).min(1.0) } else { 1.0 })
        })
        .collect())
}

fn read_index_accessor(
    root: &json::Value,
    buffers: &[Vec<u8>],
    index: usize,
) -> Result<Vec<u32>, ImportError> {
    let view = accessor_view(root, buffers, index)?;
    if view.components_per_element != 1 {
        return Err(ImportError::Malformed(format!(
            "index accessor {index} is not SCALAR"
        )));
    }
    if view.count % 3 != 0 {
        return Err(ImportError::Malformed(format!(
            "index accessor {index} has {} indices, not a multiple of three",
            view.count
        )));
    }
    (0..view.count)
        .map(|e| view.integer(e))
        .collect::<Option<Vec<u32>>>()
        .ok_or_else(|| ImportError::Malformed(format!("index accessor {index} is not unsigned integers")))
}

// --- nodes ---------------------------------------------------------------

fn read_nodes(
    root: &json::Value,
    meshes: &[GltfMesh],
    file_name: &str,
    warnings: &mut Vec<String>,
) -> Result<(Vec<RawNode>, Vec<usize>), ImportError> {
    let source_nodes = root.get("nodes").map(|n| n.as_array()).unwrap_or(&[]);

    // Which nodes the chosen scene starts from. Falling back to "every node
    // that nobody names as a child" keeps files without a `scenes` array
    // working.
    let scene_index = root.get("scene").and_then(|v| v.as_index()).unwrap_or(0);
    let declared_roots: Vec<usize> = root
        .get("scenes")
        .and_then(|s| s.index(scene_index))
        .and_then(|s| s.get("nodes"))
        .map(|n| n.as_array().iter().filter_map(|v| v.as_index()).collect())
        .unwrap_or_else(|| {
            let mut is_child = vec![false; source_nodes.len()];
            for node in source_nodes {
                for child in node.get("children").map(|c| c.as_array()).unwrap_or(&[]) {
                    if let Some(i) = child.as_index() {
                        if i < is_child.len() {
                            is_child[i] = true;
                        }
                    }
                }
            }
            (0..source_nodes.len()).filter(|i| !is_child[*i]).collect()
        });

    let mut out: Vec<RawNode> = Vec::new();
    let mut roots = Vec::new();

    // Depth-first, parent before child, which is what `RawScene` requires and
    // what `Command::Import` then relies on.
    struct Frame {
        source: usize,
        parent: Option<usize>,
        depth: usize,
    }
    let mut stack: Vec<Frame> = declared_roots
        .iter()
        .rev()
        .map(|&source| Frame {
            source,
            parent: None,
            depth: 0,
        })
        .collect();
    // glTF permits a cycle in `children`; walking one would not terminate.
    let mut visited = vec![false; source_nodes.len()];

    while let Some(frame) = stack.pop() {
        let Some(source) = source_nodes.get(frame.source) else {
            warnings.push(format!("node {} does not exist", frame.source));
            continue;
        };
        if frame.depth > 256 {
            warnings.push("the node hierarchy is deeper than 256 levels; it was cut off".into());
            continue;
        }
        if visited[frame.source] {
            warnings.push(format!(
                "node {} appears more than once in the hierarchy; the repeat was skipped",
                frame.source
            ));
            continue;
        }
        visited[frame.source] = true;

        let name = source
            .get("name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("Node {}", frame.source + 1));
        let transform = read_transform(source, &name, warnings);

        let mesh = source.get("mesh").and_then(|v| v.as_index());
        let primitives = mesh
            .and_then(|m| meshes.get(m))
            .map(|m| m.primitives.len())
            .unwrap_or(0);

        let index = out.len();
        let mut node = RawNode {
            name: name.clone(),
            parent: frame.parent,
            transform,
            // A node with exactly one primitive *is* that part; with several it
            // becomes a group, because a node carries one material.
            mesh: match (mesh, primitives) {
                (Some(m), 1) => Some(meshes[m].first),
                _ => None,
            },
            metadata: Default::default(),
        };
        // §6.5: "Retain all imported attributes per node." glTF puts them in
        // `extras`; nested objects and arrays have nowhere to go in a
        // string-to-string map and are skipped rather than stringified.
        if let Some(json::Value::Object(map)) = source.get("extras") {
            for (key, value) in map {
                let text = match value {
                    json::Value::String(s) => s.clone(),
                    json::Value::Number(n) => n.to_string(),
                    json::Value::Bool(b) => b.to_string(),
                    _ => continue,
                };
                node.metadata.insert(key.clone(), text);
            }
        }
        if frame.parent.is_none() {
            roots.push(index);
        }
        out.push(node);

        // Split a multi-primitive mesh into one child per primitive.
        if let (Some(m), 2..) = (mesh, primitives) {
            for p in 0..primitives {
                let mut child =
                    RawNode::part(meshes[m].primitives[p].name.clone(), meshes[m].first + p);
                child.parent = Some(index);
                out.push(child);
            }
        }

        for child in source
            .get("children")
            .map(|c| c.as_array())
            .unwrap_or(&[])
            .iter()
            .rev()
        {
            if let Some(child) = child.as_index() {
                stack.push(Frame {
                    source: child,
                    parent: Some(index),
                    depth: frame.depth + 1,
                });
            }
        }
    }

    if out.is_empty() {
        return Err(ImportError::Malformed("the file contains no nodes".into()));
    }
    // A file with several roots gets a group, so one import is one thing in the
    // Scene tab and can be moved or deleted as a unit.
    if roots.len() > 1 {
        for node in out.iter_mut() {
            node.parent = Some(node.parent.map(|p| p + 1).unwrap_or(0));
        }
        out.insert(0, RawNode::group(file_name));
        roots = vec![0];
    }
    Ok((out, roots))
}

/// A node's local transform: either `matrix`, or a TRS triple composed as
/// `T * R * S`. Never both, by specification.
fn read_transform(source: &json::Value, name: &str, warnings: &mut Vec<String>) -> Transform {
    if let Some(values) = source.get("matrix").and_then(|v| v.as_f32_array()) {
        if values.len() == 16 {
            // glTF matrices are column-major; `Mat4` is row-major.
            let mut m = Mat4::IDENTITY;
            for row in 0..4 {
                for column in 0..4 {
                    m.m[row][column] = values[column * 4 + row];
                }
            }
            return match Transform::from_matrix(m) {
                Some(t) => t,
                None => {
                    // A singular matrix — a zero scale on some axis — cannot be
                    // inverted, and the renderer needs the inverse for normals.
                    warnings.push(format!(
                        "`{name}` has a non-invertible transform; it was replaced with the identity"
                    ));
                    Transform::IDENTITY
                }
            };
        }
        warnings.push(format!(
            "`{name}` has a matrix with {} values",
            values.len()
        ));
    }

    let translation = source
        .get("translation")
        .and_then(|v| v.as_f32_array())
        .filter(|v| v.len() == 3)
        .map(|v| vec3(v[0], v[1], v[2]))
        .unwrap_or(Vec3::ZERO);
    let scale = source
        .get("scale")
        .and_then(|v| v.as_f32_array())
        .filter(|v| v.len() == 3)
        .map(|v| vec3(v[0], v[1], v[2]))
        .unwrap_or(vec3(1.0, 1.0, 1.0));
    let rotation = source
        .get("rotation")
        .and_then(|v| v.as_f32_array())
        .filter(|v| v.len() == 4)
        .map(|v| [v[0], v[1], v[2], v[3]])
        .unwrap_or([0.0, 0.0, 0.0, 1.0]);

    let matrix = Mat4::translation(translation) * quaternion_matrix(rotation) * Mat4::scale(scale);
    Transform::from_matrix(matrix).unwrap_or_else(|| {
        warnings.push(format!(
            "`{name}` has a zero scale on some axis; its transform was replaced with the identity"
        ));
        Transform::IDENTITY
    })
}

/// glTF stores rotations as `[x, y, z, w]` quaternions.
fn quaternion_matrix([x, y, z, w]: [f32; 4]) -> Mat4 {
    // Normalize: exporters emit quaternions a few ulps off unit length, and an
    // unnormalized one scales the node.
    let length = (x * x + y * y + z * z + w * w).sqrt();
    let (x, y, z, w) = if length > 0.0 {
        (x / length, y / length, z / length, w / length)
    } else {
        (0.0, 0.0, 0.0, 1.0)
    };
    let mut m = Mat4::IDENTITY;
    m.m[0][0] = 1.0 - 2.0 * (y * y + z * z);
    m.m[0][1] = 2.0 * (x * y - z * w);
    m.m[0][2] = 2.0 * (x * z + y * w);
    m.m[1][0] = 2.0 * (x * y + z * w);
    m.m[1][1] = 1.0 - 2.0 * (x * x + z * z);
    m.m[1][2] = 2.0 * (y * z - x * w);
    m.m[2][0] = 2.0 * (x * z - y * w);
    m.m[2][1] = 2.0 * (y * z + x * w);
    m.m[2][2] = 1.0 - 2.0 * (x * x + y * y);
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 32-bit indices arrive exactly, past where an `f32` stops counting
    /// every integer.
    #[test]
    fn a_large_index_is_read_exactly() {
        let big = (1u32 << 24) + 1;
        let data: Vec<u8> = [big, u32::MAX].iter().flat_map(|i| i.to_le_bytes()).collect();
        let view = AccessorView { data: &data, offset: 0, stride: 4, count: 2, component: UNSIGNED_INT, components_per_element: 1, normalized: false };
        assert_eq!(view.integer(0), Some(big));
        assert_eq!(view.integer(1), Some(u32::MAX));
        assert_ne!(view.component(0, 0) as u32, big, "the f32 path this replaces rounds it");
    }
}
