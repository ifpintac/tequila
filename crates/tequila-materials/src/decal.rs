//! Printed image layers, ordered from bottom to top over a surface material.
use crate::graph::TextureId;
use tequila_texture::projection::Mapping;

#[derive(Clone, Debug, PartialEq)]
pub struct Decal {
    pub name: String,
    pub texture: TextureId,
    pub mapping: Mapping,
    pub opacity: f32,
    pub roughness: f32,
    /// Printed in foil: the artwork's colour reflects as metal does, as a
    /// hot-stamped logo or a foil seal (§1.11's per-label finish).
    pub metallic: bool,
    pub enabled: bool,
}

impl Decal {
    pub fn new(name: impl Into<String>, texture: TextureId) -> Self {
        Self {
            name: name.into(),
            texture,
            mapping: Mapping::default(),
            opacity: 1.0,
            roughness: 0.35,
            metallic: false,
            enabled: true,
        }
    }

    pub fn is_valid(&self) -> bool {
        self.opacity.is_finite()
            && (0.0..=1.0).contains(&self.opacity)
            && self.roughness.is_finite()
            && (0.0..=1.0).contains(&self.roughness)
            && self.mapping.scale.x.is_finite()
            && self.mapping.scale.x > 0.0
            && self.mapping.scale.y.is_finite()
            && self.mapping.scale.y > 0.0
            && self.mapping.scale.z.is_finite()
            && self.mapping.scale.z > 0.0
            && self.mapping.offset.x.is_finite()
            && self.mapping.offset.y.is_finite()
            && self.mapping.rotation.is_finite()
    }
}
