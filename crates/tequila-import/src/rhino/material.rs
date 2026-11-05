//! A 3dm file's materials: Rhino's basic material (`ON_Material`) and, from
//! Rhino 7, the physically based settings it carries as user data, made
//! OpenPBR materials.
//!
//! Rhino 6 and later keep their render materials (RDK content) as XML of
//! their own, and also write each as an `ON_Material` whose basic fields
//! and PBR user data simulate it; the simulation is what is read here.
//!
//! The mapping, which `docs/03` §6.1 point 8 asks to be documented:
//!
//! * PBR: base colour, metallic, roughness, clear coat and its roughness,
//!   sheen, subsurface with its colour and radius, anisotropy and its
//!   rotation, emission, opacity (transmission, with its IOR and
//!   roughness) and alpha map across directly; Rhino's `specular` (0.5 for
//!   an IOR of 1.5) sets the IOR of an opaque material by Schlick's
//!   F0 = 0.08 × specular. Colours are taken as display (sRGB-encoded)
//!   values, as Rhino's colour pickers show them.
//! * Basic: the diffuse colour is the base colour; the shine (0 to 255, a
//!   Phong exponent) is a roughness by §6.1's `sqrt(2 / (exponent + 2))`;
//!   transparency is transmission at the material's IOR (1.5 where it says
//!   1, which would make glass vanish) tinted by the transparent colour;
//!   reflectivity is a metal of the reflection colour when Fresnel
//!   reflections are off and a clear coat when they are on, either as
//!   glossy as the reflection glossiness says; emission is an emission
//!   colour.
//! * Maps, either way, by the texture's type: colour (bitmap), bump (a
//!   height map, or a normal map when its file is named like one),
//!   transparency (opacity), and the PBR metallic, roughness, clear coat,
//!   emission and alpha maps. A map's own repeat and offset are not read.

use super::archive::{uuid, Object, Reader, Uuid, R, TCODE_ANONYMOUS_CHUNK, TCODE_MODEL_ATTRIBUTES_CHUNK};
use crate::raw::RawTexture;
use tequila_core::math::vec3;
use tequila_core::spectrum::Spectrum;
use tequila_materials::graph::Value;
use tequila_materials::material_type::{MaterialType, Param};
use tequila_render_core::material::Material;
use tequila_scene::material::MaterialDefinition;
use tequila_texture::texture::MapRole;

pub(super) const ON_MATERIAL: Uuid = uuid("60B5DBBC-E660-11d3-BFE4-0010830122F0");
const ON_TEXTURE: Uuid = uuid("D6FF106D-329B-4F29-97E2-FD282A618020");
const ON_PBR_MATERIAL_USERDATA: Uuid = uuid("5694E1AC-40E6-44F4-9CA9-3B6D0E8C4440");

/// `ON_Texture::TYPE`s read.
const TEXTURE_COLOUR: u32 = 1;
const TEXTURE_BUMP: u32 = 2;
const TEXTURE_TRANSPARENCY: u32 = 3;
const TEXTURE_METALLIC: u32 = 13;
const TEXTURE_ROUGHNESS: u32 = 16;
const TEXTURE_CLEARCOAT: u32 = 21;
const TEXTURE_CLEARCOAT_ROUGHNESS: u32 = 22;
const TEXTURE_EMISSION: u32 = 25;
const TEXTURE_ALPHA: u32 = 30;

#[derive(Clone, Debug, Default)]
pub(super) struct RhinoMaterial {
    pub name: String,
    pub diffuse: [u8; 4],
    pub emission: [u8; 4],
    pub reflection: [u8; 4],
    pub transparent: [u8; 4],
    pub ior: f64,
    pub reflectivity: f64,
    pub shine: f64,
    pub transparency: f64,
    /// Fresnel reflections, and the reflection and refraction glossiness.
    pub fresnel: Option<(bool, f64, f64)>,
    pub textures: Vec<Texture>,
    pub pbr: Option<Pbr>,
}

#[derive(Clone, Debug)]
pub(super) struct Texture {
    pub kind: u32,
    pub path: String,
}

/// `ON_PhysicallyBasedMaterial`'s settings, as its user data stores them.
#[derive(Clone, Debug, Default)]
pub(super) struct Pbr {
    pub base: [f32; 4],
    pub subsurface: f64,
    pub subsurface_colour: [f32; 4],
    pub subsurface_radius: f64,
    pub metallic: f64,
    pub specular: f64,
    pub roughness: f64,
    pub anisotropic: f64,
    pub anisotropic_rotation: f64,
    pub sheen: f64,
    pub clearcoat: f64,
    pub clearcoat_roughness: f64,
    pub opacity_ior: f64,
    pub opacity: f64,
    pub opacity_roughness: f64,
    pub emission: [f32; 4],
    pub alpha: Option<f64>,
}

/// The parts of `ON_ModelComponent` a table record starts with from
/// Rhino 6: its index, id and name. Two encodings: a status byte before
/// each field, or a bit set saying which follow.
pub(super) fn component(r: &mut Reader<'_>) -> R<(Option<i32>, Uuid, String)> {
    let at = r.pos;
    let chunk = r.chunk()?;
    if chunk.tcode != TCODE_MODEL_ATTRIBUTES_CHUNK && chunk.tcode != TCODE_ANONYMOUS_CHUNK {
        return Err(format!("expected a component's attributes at byte {at}"));
    }
    let mut b = chunk.body;
    b.i32()?;
    b.i32()?;
    let (mut index, mut id, mut name) = (None, super::archive::NIL, String::new());
    if chunk.tcode == TCODE_ANONYMOUS_CHUNK {
        let bits = b.u32()?;
        if bits & 1 != 0 {
            id = b.uuid()?;
        }
        if bits & 2 != 0 {
            b.skip(16)?;
        }
        if bits & 4 != 0 {
            index = Some(b.i32()?);
        }
        if bits & 8 != 0 {
            name = b.string()?;
        }
        return Ok((index, id, name));
    }
    if b.u8()? == 1 {
        b.skip(12)?;
    }
    if b.u8()? == 1 {
        id = b.uuid()?;
    }
    if b.u8()? == 1 {
        b.skip(4)?;
    }
    if b.u8()? == 1 {
        index = Some(b.i32()?);
    }
    if b.u8()? == 1 {
        name = b.string()?;
    }
    Ok((index, id, name))
}

/// `ON_Material::Read`. Rhino 6 on: an anonymous chunk holding the
/// component's attributes, then the fields. Before: a version byte, then
/// an anonymous chunk holding the id, index and name, then the same fields.
pub(super) fn read(object: &Object<'_>) -> R<RhinoMaterial> {
    let mut data = object.data;
    let modern = data.remaining() > 0 && { data }.u8()? == 0;
    let (mut r, minor) = if modern {
        let (mut r, major, _) = data.anonymous("a material")?;
        if major != 1 {
            return Err(format!("a material of version {major}"));
        }
        let (_, _, name) = component(&mut r)?;
        (r, (name, i32::MAX))
    } else {
        let (major, _) = data.version()?;
        if major != 2 {
            return Err(format!("a material of version {major}"));
        }
        let (mut r, major, minor) = data.anonymous("a material")?;
        if major != 1 {
            return Err(format!("a material of version {major}.{minor}"));
        }
        r.uuid()?;
        r.i32()?;
        let name = r.string()?;
        (r, (name, minor))
    };
    let (name, minor) = minor;
    let mut m = RhinoMaterial { name, ..Default::default() };
    r.uuid()?;
    let colour = |r: &mut Reader<'_>| -> R<[u8; 4]> { Ok([r.u8()?, r.u8()?, r.u8()?, r.u8()?]) };
    colour(&mut r)?;
    m.diffuse = colour(&mut r)?;
    m.emission = colour(&mut r)?;
    colour(&mut r)?;
    m.reflection = colour(&mut r)?;
    m.transparent = colour(&mut r)?;
    m.ior = r.finite("an index of refraction")?;
    m.reflectivity = r.finite("a reflectivity")?;
    m.shine = r.finite("a shine")?;
    m.transparency = r.finite("a transparency")?;
    m.textures = textures(&mut r).unwrap_or_default();
    // What follows matters less; a problem in it keeps what is read.
    let _ = (|| -> R<()> {
        if !modern && minor >= 1 {
            r.string()?;
        }
        if minor >= 2 {
            let n = r.count(20)?;
            r.skip(n * 20)?;
        }
        if minor >= 3 {
            r.skip(2)?;
        }
        if minor >= 4 {
            let fresnel = r.bool()?;
            let (reflection, refraction) = (r.f64()?, r.f64()?);
            r.f64()?;
            m.fresnel = Some((fresnel, reflection, refraction));
        }
        Ok(())
    })();
    if let Some(u) = object.userdata.iter().find(|u| u.class == ON_PBR_MATERIAL_USERDATA) {
        m.pbr = pbr(u.payload).ok();
    }
    Ok(m)
}

/// The texture array: an anonymous chunk of `ON_Texture` objects. Each
/// object's data: an anonymous chunk with its id, mapping channel, the
/// file's full path as it was, whether it is on, and its type; the rest is
/// how it is blended and placed.
fn textures(r: &mut Reader<'_>) -> R<Vec<Texture>> {
    let (mut a, major, _) = r.anonymous("a material's textures")?;
    if major != 1 {
        return Ok(Vec::new());
    }
    let n = a.count(4)?;
    let mut out = Vec::new();
    for _ in 0..n {
        let object = a.object()?;
        if object.class != ON_TEXTURE {
            continue;
        }
        let mut data = object.data;
        let (mut t, major, _) = data.anonymous("a texture")?;
        if major != 1 {
            continue;
        }
        t.uuid()?;
        t.u32()?;
        let path = t.string()?;
        let enabled = t.bool()?;
        let kind = t.u32()?;
        if enabled && !path.trim().is_empty() {
            out.push(Texture { kind, path });
        }
    }
    Ok(out)
}

/// The PBR user data: an anonymous chunk, version 1.1 or 1.2, of colours
/// as four floats and the rest as doubles.
fn pbr(mut payload: Reader<'_>) -> R<Pbr> {
    let (mut r, major, minor) = payload.anonymous("physically based settings")?;
    if major != 1 {
        return Err(format!("physically based settings of version {major}.{minor}"));
    }
    let colour = |r: &mut Reader<'_>| -> R<[f32; 4]> { Ok([r.f32()?, r.f32()?, r.f32()?, r.f32()?]) };
    let mut p = Pbr { base: colour(&mut r)?, ..Default::default() };
    r.i32()?;
    p.subsurface = r.finite("a subsurface")?;
    p.subsurface_colour = colour(&mut r)?;
    p.subsurface_radius = r.finite("a subsurface radius")?;
    p.metallic = r.finite("a metallic")?;
    p.specular = r.finite("a specular")?;
    r.f64()?;
    p.roughness = r.finite("a roughness")?;
    p.anisotropic = r.finite("an anisotropy")?;
    p.anisotropic_rotation = r.finite("an anisotropy rotation")?;
    p.sheen = r.finite("a sheen")?;
    r.f64()?;
    p.clearcoat = r.finite("a clear coat")?;
    p.clearcoat_roughness = r.finite("a clear coat roughness")?;
    p.opacity_ior = r.finite("an opacity IOR")?;
    p.opacity = r.finite("an opacity")?;
    p.opacity_roughness = r.finite("an opacity roughness")?;
    p.emission = colour(&mut r)?;
    if minor >= 2 {
        p.alpha = Some(r.finite("an alpha")?);
    }
    Ok(p)
}

fn display(c: [u8; 4]) -> Spectrum {
    Spectrum::from_srgb_encoded(vec3(c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0))
}

fn display_f(c: [f32; 4]) -> Spectrum {
    Spectrum::from_srgb_encoded(vec3(c[0].clamp(0.0, 1.0), c[1].clamp(0.0, 1.0), c[2].clamp(0.0, 1.0)))
}

/// A plastic of a display colour: what an object with no render material
/// wears, as Rhino shows it.
pub(super) fn colour_material(name: &str, c: [u8; 4]) -> MaterialDefinition {
    MaterialDefinition::new(name, MaterialType::Plastic, Material::plastic(display(c), 0.35))
}

/// A Rhino material as an OpenPBR one, its maps added to `textures`.
pub(super) fn definition(m: &RhinoMaterial, textures: &mut Vec<RawTexture>) -> MaterialDefinition {
    let name = if m.name.trim().is_empty() { "Rhino material" } else { m.name.trim() };
    let (material, kind) = match &m.pbr {
        Some(p) => {
            let base = display_f(p.base);
            let transmission = (1.0 - p.opacity).clamp(0.0, 1.0) as f32;
            // Schlick: F0 = ((n - 1) / (n + 1))^2, and Rhino's specular 0.5
            // is F0 0.04, an IOR of 1.5.
            let f0 = (0.08 * p.specular).clamp(0.0, 0.9).sqrt();
            let ior = if transmission > 0.0 { p.opacity_ior } else { (1.0 + f0) / (1.0 - f0) };
            let emission = display_f(p.emission);
            let material = Material {
                base_metalness: p.metallic.clamp(0.0, 1.0) as f32,
                specular_ior: ior.clamp(1.0, 3.0) as f32,
                specular_roughness_anisotropy: p.anisotropic.clamp(0.0, 1.0) as f32,
                specular_rotation: p.anisotropic_rotation.rem_euclid(1.0) as f32,
                transmission_weight: transmission,
                transmission_color: if transmission > 0.0 { base } else { Spectrum::ONE },
                subsurface_weight: p.subsurface.clamp(0.0, 1.0) as f32,
                subsurface_color: display_f(p.subsurface_colour),
                subsurface_radius: Spectrum::splat(p.subsurface_radius.max(0.0) as f32),
                coat_weight: p.clearcoat.clamp(0.0, 1.0) as f32,
                coat_roughness: p.clearcoat_roughness.clamp(0.0, 1.0) as f32,
                fuzz_weight: p.sheen.clamp(0.0, 1.0) as f32,
                emission_color: emission,
                emission_luminance: if emission.is_black() { 0.0 } else { 1.0 },
                geometry_opacity: p.alpha.unwrap_or(1.0).clamp(0.0, 1.0) as f32,
                geometry_thin_walled: false,
                ..Material::plastic(base, p.roughness.clamp(0.0, 1.0) as f32)
            };
            let kind = if transmission >= 0.5 {
                MaterialType::GlassSolid
            } else if p.metallic >= 0.5 {
                MaterialType::Metal
            } else {
                MaterialType::Plastic
            };
            (material, kind)
        }
        None => {
            let base = display(m.diffuse);
            let roughness = (2.0 / (m.shine.clamp(0.0, 255.0) + 2.0)).sqrt() as f32;
            let transparency = m.transparency.clamp(0.0, 1.0) as f32;
            let ior = if m.ior > 1.01 { m.ior.min(3.0) } else { 1.5 } as f32;
            let emission = display(m.emission);
            let (fresnel, gloss, refraction_gloss) = m.fresnel.unwrap_or((true, 1.0, 1.0));
            let reflectivity = m.reflectivity.clamp(0.0, 1.0) as f32;
            let mut material = Material {
                specular_ior: ior,
                emission_color: emission,
                emission_luminance: if emission.is_black() { 0.0 } else { 1.0 },
                geometry_thin_walled: false,
                ..Material::plastic(base, roughness)
            };
            let mut kind = MaterialType::Plastic;
            if transparency > 0.0 {
                material.transmission_weight = transparency;
                material.transmission_color = display(m.transparent);
                material.specular_roughness = (1.0 - refraction_gloss.clamp(0.0, 1.0) as f32).min(roughness);
                if transparency >= 0.5 {
                    kind = MaterialType::GlassSolid;
                }
            } else if reflectivity > 0.0 && !fresnel {
                material.base_metalness = reflectivity;
                material.base_color = display(m.reflection);
                material.specular_roughness = 1.0 - gloss.clamp(0.0, 1.0) as f32;
                if reflectivity >= 0.5 {
                    kind = MaterialType::Metal;
                }
            } else if reflectivity > 0.0 {
                material.coat_weight = reflectivity;
                material.coat_roughness = 1.0 - gloss.clamp(0.0, 1.0) as f32;
            }
            (material, kind)
        }
    };
    let mut definition = MaterialDefinition::new(name, kind, material);
    for t in &m.textures {
        let looks_normal = {
            let file = t.path.rsplit(['/', '\\']).next().unwrap_or(&t.path).to_lowercase();
            file.contains("normal") || file.contains("_nrm") || file.contains("_nor.") || file.contains("_n.")
        };
        let m = definition.material().clone();
        let (role, param, fallback) = match t.kind {
            TEXTURE_COLOUR => (MapRole::Color, Param::BaseColor, Value::Color(m.base_color)),
            TEXTURE_BUMP if looks_normal => (MapRole::Normal, Param::Bump, Value::Float(0.0)),
            TEXTURE_BUMP => (MapRole::Height, Param::Bump, Value::Float(0.0)),
            TEXTURE_TRANSPARENCY | TEXTURE_ALPHA => (MapRole::Data, Param::Opacity, Value::Float(m.geometry_opacity)),
            TEXTURE_METALLIC => (MapRole::Data, Param::BaseMetalness, Value::Float(m.base_metalness)),
            TEXTURE_ROUGHNESS => (MapRole::Data, Param::SpecularRoughness, Value::Float(m.specular_roughness)),
            TEXTURE_CLEARCOAT => (MapRole::Data, Param::CoatWeight, Value::Float(m.coat_weight)),
            TEXTURE_CLEARCOAT_ROUGHNESS => (MapRole::Data, Param::CoatRoughness, Value::Float(m.coat_roughness)),
            TEXTURE_EMISSION => (MapRole::Color, Param::EmissionColor, Value::Color(m.emission_color)),
            _ => continue,
        };
        crate::obj::attach_map(&mut definition, textures, &t.path, role, param, fallback);
    }
    definition
}
