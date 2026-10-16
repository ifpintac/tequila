//! Typed, stable identifiers.
//!
//! `docs/03-SCENE-AND-IO.md` §2 requires IDs that survive save/load and
//! re-import. They are typed rather than bare `u32` so that a material ID
//! cannot be passed where an instance ID belongs — a mistake that produces a
//! plausible-looking wrong image rather than a compile error.

macro_rules! define_id {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(transparent)]
        pub struct $name(pub u32);

        impl $name {
            /// Sentinel for "no such object". Chosen as `u32::MAX` so that a
            /// zero-initialized buffer reads as ID 0, not as invalid.
            pub const INVALID: Self = $name(u32::MAX);

            #[inline]
            pub const fn index(self) -> usize {
                self.0 as usize
            }

            #[inline]
            pub const fn is_valid(self) -> bool {
                self.0 != u32::MAX
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                if self.is_valid() {
                    write!(f, "{}#{}", stringify!($name), self.0)
                } else {
                    write!(f, "{}#invalid", stringify!($name))
                }
            }
        }
    };
}

define_id!(
    /// A node in the scene tree: a part, subassembly, or group.
    NodeId
);
define_id!(
    /// A unique mesh in the geometry store. Several instances may share one.
    MeshId
);
define_id!(
    /// A renderable instance: one mesh plus one transform plus one material.
    InstanceId
);
define_id!(
    /// A material *instance*. Shared by every part the user dropped it onto —
    /// the linking model of `docs/04-UX-SPEC.md` §3.
    MaterialId
);
define_id!(
    /// A light in the scene, including emissive geometry registered as one.
    LightId
);
define_id!(
    /// A light layer, for per-group AOVs and post-render relighting (§5.5).
    LightLayerId
);
define_id!(
    /// A texture in the texture cache.
    TextureId
);
define_id!(
    /// A camera.
    CameraId
);

/// Deterministic pseudo-random colour for an ID, used by the Clown/ID AOV
/// (`docs/01` §12) and by selection highlighting.
///
/// Must be a pure function of the ID so that the same object gets the same
/// colour across renders and across machines — a compositor relies on it.
pub fn id_to_debug_color(id: u32) -> [f32; 3] {
    let h = crate::rng::hash_u32(id.wrapping_add(1));
    [
        ((h >> 16) & 0xff) as f32 / 255.0,
        ((h >> 8) & 0xff) as f32 / 255.0,
        (h & 0xff) as f32 / 255.0,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_ids_are_detected() {
        assert!(!NodeId::INVALID.is_valid());
        assert!(NodeId(0).is_valid());
    }

    #[test]
    fn debug_colors_are_stable_and_distinct() {
        assert_eq!(id_to_debug_color(7), id_to_debug_color(7));
        assert_ne!(id_to_debug_color(7), id_to_debug_color(8));
    }
}
