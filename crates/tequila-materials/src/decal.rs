//! Printed image layers, ordered from bottom to top over a surface material.
use crate::graph::TextureId;
use tequila_texture::projection::{Mapping, Projection};

/// Which faces a planar label prints on. A planar projection goes straight
/// through a part, so by itself a label on the front panel also prints,
/// mirrored, on the back one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DecalSide {
    /// Every face it passes through, as labels always have.
    #[default]
    Both,
    /// Only faces looking along the projection's axis.
    Positive,
    /// Only faces looking against it.
    Negative,
}

/// How far from its axis a face may turn and still take a one-sided label:
/// 60 degrees. A face near edge-on would take the artwork smeared into
/// streaks, and a CAD face meant to be square to the label is rarely quite.
pub const ONE_SIDED_COSINE: f32 = 0.5;

impl DecalSide {
    pub const ALL: [DecalSide; 3] = [DecalSide::Both, DecalSide::Positive, DecalSide::Negative];

    pub fn key(self) -> &'static str {
        match self {
            DecalSide::Both => "both",
            DecalSide::Positive => "positive",
            DecalSide::Negative => "negative",
        }
    }

    pub fn from_key(key: &str) -> Option<DecalSide> {
        DecalSide::ALL.into_iter().find(|s| s.key() == key)
    }
}

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
    /// Which faces it prints on, for a planar label.
    pub side: DecalSide,
    /// The ink: the artwork's colour multiplied by it, so white artwork
    /// prints in any colour. White leaves the artwork as it is.
    pub ink: tequila_core::spectrum::Spectrum,
}

impl Decal {
    pub fn new(name: impl Into<String>, texture: TextureId) -> Self {
        Self {
            name: name.into(),
            texture,
            // Artwork turns as a whole: a wide logo turned is a wide logo.
            mapping: Mapping { rigid: true, ..Mapping::default() },
            opacity: 1.0,
            roughness: 0.35,
            metallic: false,
            enabled: true,
            side: DecalSide::Both,
            ink: tequila_core::spectrum::Spectrum::ONE,
        }
    }

    /// Whether it prints on a face whose normal, in the part's own space, is
    /// `normal`: always, unless a planar label keeps to one side.
    pub fn prints_on(&self, normal: tequila_core::math::Vec3) -> bool {
        if self.mapping.projection != Projection::Planar {
            return true;
        }
        let along = self.mapping.axis.of(normal);
        match self.side {
            DecalSide::Both => true,
            DecalSide::Positive => along > ONE_SIDED_COSINE,
            DecalSide::Negative => along < -ONE_SIDED_COSINE,
        }
    }

    pub fn is_valid(&self) -> bool {
        [self.ink.r, self.ink.g, self.ink.b].iter().all(|c| c.is_finite() && *c >= 0.0)
            && self.opacity.is_finite()
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

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec3;
    use tequila_texture::projection::Axis;

    /// A flat label kept to one side prints on faces looking along its axis
    /// that way only; on both sides, or projected any other way, anywhere.
    #[test]
    fn a_label_kept_to_one_side_prints_on_that_side() {
        let mut label = Decal::new("Print", TextureId(1));
        label.mapping.projection = Projection::Planar;
        label.mapping.axis = Axis::Z;
        let (front, back, edge) = (vec3(0.2, 0.3, 0.9), vec3(0.2, 0.3, -0.9), vec3(1.0, 0.0, 0.0));
        // A face drafted a couple of degrees off square to the label, and one
        // turned 70 degrees from it.
        let (drafted, steep) = (vec3(0.0, -0.999, 0.035), vec3(0.94, 0.0, 0.342));
        assert!(label.prints_on(front) && label.prints_on(back) && label.prints_on(edge));
        label.side = DecalSide::Positive;
        assert!(label.prints_on(front) && !label.prints_on(back) && !label.prints_on(edge));
        assert!(!label.prints_on(drafted) && !label.prints_on(steep), "near edge-on faces would smear it");
        label.side = DecalSide::Negative;
        assert!(!label.prints_on(front) && label.prints_on(back));
        label.mapping.projection = Projection::Cylindrical;
        assert!(label.prints_on(front), "a wrapped label has no one side");
    }
}
