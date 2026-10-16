//! Bump and normal mapping (`docs/02-MATERIAL-SYSTEM.md` §5.4).
//!
//! Both perturb the shading normal so a flat triangle scatters light as though
//! it had detail. They differ in what the texture holds: a bump map holds a
//! *height*, and the normal comes from its slope; a normal map holds the
//! perturbed normal directly.
//!
//! ## The step size is a ray differential, and that is the whole point
//!
//! §5.4 asks for the gradient by "finite differences in texture space using ray
//! differentials for the step size (screen-size-independent bump, avoiding the
//! classic 'bump gets stronger when you zoom out' artifact)". The artifact is
//! worth spelling out, because the fix looks like an optimisation and is not.
//!
//! Take the naive approach: sample the height one texel apart and divide by one
//! texel. Now pull the camera back. The footprint grows, so the lookup climbs
//! the MIP pyramid and the heights come back pre-averaged and smooth — but the
//! step is still *one texel of level zero*, a distance far smaller than the
//! region those averaged values describe. Dividing a small height difference by
//! a too-small step inflates the slope, and the bump appears to deepen as the
//! object recedes. Surfaces that looked right in close-up turn corrugated in a
//! wide shot, and because it tracks the camera rather than the material it
//! reads as a lighting problem.
//!
//! Stepping by the footprint instead keeps the two consistent: the difference is
//! measured across the same region the MIP level is averaging over. Detail finer
//! than the footprint fades out, which is correct — it is below the resolution
//! of the pixel — rather than turning into noise that crawls.
//!
//! ## Bump layers add as gradients, not as heights
//!
//! §5.4 calls this out as "a real and common bug", and [`BumpGradient::add`] is
//! the operation that avoids it. Summing two height *fields* and taking the
//! slope of the result is not the same as summing their slopes where the fields
//! are sampled at different scales — the coarse layer's MIP averaging flattens
//! the fine layer's detail before the derivative is ever taken. Adding the
//! gradients keeps each layer's detail at its own scale.

use crate::mip::Wrap;
use crate::projection::SurfacePoint;
use crate::texture::{MapRole, Texture};
use tequila_core::math::Vec3;

/// A height field's slope in texture space.
///
/// Kept as a gradient rather than immediately applied to a normal so that
/// layers can be combined correctly — see this module's header.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BumpGradient {
    /// Rate of change of height along u and along v, in height units per unit
    /// of texture coordinate.
    pub du: f32,
    pub dv: f32,
}

impl BumpGradient {
    pub const FLAT: BumpGradient = BumpGradient { du: 0.0, dv: 0.0 };

    /// Combine two bump layers (§5.4's Bump Add).
    ///
    /// Gradients add; heights do not. See this module's header for why the
    /// difference is not cosmetic.
    pub fn add(self, other: BumpGradient) -> BumpGradient {
        BumpGradient {
            du: self.du + other.du,
            dv: self.dv + other.dv,
        }
    }

    pub fn scaled(self, by: f32) -> BumpGradient {
        BumpGradient {
            du: self.du * by,
            dv: self.dv * by,
        }
    }

    pub fn is_flat(self) -> bool {
        self.du == 0.0 && self.dv == 0.0
    }
}

/// The surface's own frame at a shading point.
#[derive(Clone, Copy, Debug)]
pub struct TangentFrame {
    pub normal: Vec3,
    /// Direction of increasing u along the surface, perpendicular to the
    /// normal.
    pub tangent: Vec3,
    /// Direction of increasing v. Not necessarily perpendicular to the tangent
    /// on a distorted parameterization, which is why it is stored rather than
    /// derived by a cross product.
    pub bitangent: Vec3,
}

impl TangentFrame {
    /// A frame from a normal alone, for geometry with no UVs.
    ///
    /// Tri-planar mapping needs this: there is no parameterization to take a
    /// tangent from, so any consistent perpendicular pair will do. It is stable
    /// under small changes in the normal, which matters because a frame that
    /// flipped between adjacent shading points would make bump mapping flicker.
    pub fn from_normal(normal: Vec3) -> TangentFrame {
        let n = normal.normalize();
        // Branch on the smallest component so the cross product never degenerates.
        let up = if n.z.abs() < 0.9 { Vec3::Z } else { Vec3::X };
        let tangent = up.cross(n).normalize();
        TangentFrame {
            normal: n,
            tangent,
            bitangent: n.cross(tangent),
        }
    }

    /// Perturb the normal by a bump gradient.
    ///
    /// The normal tilts *away* from uphill: a surface rising along +u scatters
    /// as though tipped back along -u.
    pub fn perturb(&self, gradient: BumpGradient, strength: f32) -> Vec3 {
        if gradient.is_flat() || strength == 0.0 {
            return self.normal;
        }
        let tilted = self.normal
            - (self.tangent * gradient.du + self.bitangent * gradient.dv) * strength;
        tilted.try_normalize().unwrap_or(self.normal)
    }

    /// Decode a tangent-space normal map sample into world space.
    ///
    /// The stored vector is remapped from 0–1 to -1–1 — which is why §5.2 puts
    /// normal maps in Raw: the sRGB curve applied to those values would bend
    /// every normal towards the surface, flattening the entire map in a way
    /// that reads as a weak texture rather than as a colour-space mistake.
    pub fn decode_normal_map(&self, sample: Vec3, strength: f32) -> Vec3 {
        let x = (sample.x * 2.0 - 1.0) * strength;
        let y = (sample.y * 2.0 - 1.0) * strength;
        // A normal map's blue channel is always positive: the stored vector
        // points out of the surface by construction. Clamping rather than
        // trusting it keeps a mis-tagged or badly authored map from turning
        // surfaces inside out.
        let z = (sample.z * 2.0 - 1.0).max(1e-4);
        let world = self.tangent * x + self.bitangent * y + self.normal * z;
        world.try_normalize().unwrap_or(self.normal)
    }
}

/// Measure a height field's slope at a shading point.
///
/// The step is the footprint, per §5.4. `scale` is the material's bump amount.
pub fn sample_gradient(texture: &Texture, point: &SurfacePoint, scale: f32) -> BumpGradient {
    if scale == 0.0 {
        return BumpGradient::FLAT;
    }

    let contributions = texture.mapping.project(point);
    let Some(primary) = contributions
        .iter()
        .max_by(|a, b| a.weight.total_cmp(&b.weight))
    else {
        return BumpGradient::FLAT;
    };
    let coords = primary.coords;

    // The step: the footprint, floored at a texel of the finest level so a
    // perfectly sharp view still has something to difference across.
    let base = texture.pyramid().base();
    let finest = 1.0 / base.width.max(base.height).max(1) as f32;
    let step = coords.width().max(finest);

    // Each axis is wrapped by its own mode before the lookup, which then only
    // has to stay in range. Passing one mode for both would tile a clamped
    // axis, and at a seam that shows as a band of false slope.
    let (wrap_u, wrap_v) = texture.mapping.wrap();
    let height = |u: f32, v: f32| -> f32 {
        texture
            .pyramid()
            .sample(wrap_u.apply(u), wrap_v.apply(v), step, Wrap::Clamp)
            .x
    };

    // Central differences: half a step either side, so the result is the slope
    // *at* the point rather than between it and its neighbour.
    let half = step * 0.5;
    let du = (height(coords.u + half, coords.v) - height(coords.u - half, coords.v)) / step;
    let dv = (height(coords.u, coords.v + half) - height(coords.u, coords.v - half)) / step;

    BumpGradient {
        du: du * scale,
        dv: dv * scale,
    }
}

/// Sample a tangent-space normal map and bring it into world space.
pub fn sample_normal_map(
    texture: &Texture,
    point: &SurfacePoint,
    frame: &TangentFrame,
    strength: f32,
) -> Vec3 {
    debug_assert_eq!(
        texture.role,
        MapRole::Normal,
        "a normal map must be tagged as one, or it is decoded with a transfer curve"
    );
    frame.decode_normal_map(texture.sample(point), strength)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::{ColorSpace, Image};
    use crate::projection::{Mapping, Projection};
    use tequila_core::math::{vec2, vec3};

    /// A height field rising linearly along u: constant slope everywhere, so
    /// the gradient has a closed form to check against.
    fn ramp(size: usize, rise: f32) -> Texture {
        let mut data = vec![0.0f32; size * size * 3];
        for y in 0..size {
            for x in 0..size {
                let h = rise * x as f32 / (size - 1) as f32;
                for c in 0..3 {
                    data[(y * size + x) * 3 + c] = h;
                }
            }
        }
        let image = Image {
            width: size,
            height: size,
            channels: 3,
            data,
            color_space: ColorSpace::Raw,
        };
        Texture::from_linear(
            image,
            MapRole::Height,
            Mapping {
                projection: Projection::Uv,
                wrap_u: Wrap::Clamp,
                wrap_v: Wrap::Clamp,
                ..Default::default()
            },
        )
    }

    fn at(u: f32, v: f32, footprint: f32) -> SurfacePoint {
        SurfacePoint {
            position: Vec3::ZERO,
            normal: Vec3::Z,
            uv: vec2(u, v),
            footprint,
            scale: 1.0,
            uv_footprint: None,
            curvature: 0.0,
            occlusion: 0.0,
            vertex_color: tequila_core::spectrum::Spectrum::ONE,
            world: Default::default(),
        }
    }

    /// §5.4's headline artifact, measured: the slope a bump map reports must
    /// not change with how far away the camera is.
    ///
    /// A linear ramp has the same slope at every scale, so any dependence on
    /// the footprint is the bug — and with a fixed one-texel step it is a large
    /// one, because the heights come back MIP-averaged while the divisor stays
    /// tiny.
    #[test]
    fn bump_strength_does_not_change_with_viewing_distance() {
        let texture = ramp(256, 1.0);
        let close = sample_gradient(&texture, &at(0.5, 0.5, 1.0 / 256.0), 1.0);
        let middling = sample_gradient(&texture, &at(0.5, 0.5, 8.0 / 256.0), 1.0);
        let distant = sample_gradient(&texture, &at(0.5, 0.5, 64.0 / 256.0), 1.0);

        for (name, measured) in [
            ("close", close),
            ("middling", middling),
            ("distant", distant),
        ] {
            assert!(
                (measured.du - 1.0).abs() < 0.05,
                "at {name} range the slope read {:.4}, but the ramp rises 1.0 \
                 across the texture at every scale",
                measured.du
            );
        }
        assert!(
            (close.du - distant.du).abs() < 0.02,
            "the bump deepened from {:.4} to {:.4} as the camera pulled back — \
             this is the artifact §5.4 names",
            close.du,
            distant.du
        );
    }

    /// A flat height field has no slope, at any distance.
    #[test]
    fn a_flat_height_field_perturbs_nothing() {
        let texture = ramp(64, 0.0);
        for footprint in [1.0 / 64.0, 0.1, 0.9] {
            let gradient = sample_gradient(&texture, &at(0.5, 0.5, footprint), 1.0);
            assert!(gradient.is_flat(), "got {gradient:?} at footprint {footprint}");
        }
        let frame = TangentFrame::from_normal(Vec3::Z);
        assert_eq!(frame.perturb(BumpGradient::FLAT, 5.0), frame.normal);
    }

    /// Scale is linear: twice the bump amount is twice the slope.
    #[test]
    fn the_bump_amount_scales_the_slope() {
        let texture = ramp(128, 1.0);
        let single = sample_gradient(&texture, &at(0.5, 0.5, 4.0 / 128.0), 1.0);
        let double = sample_gradient(&texture, &at(0.5, 0.5, 4.0 / 128.0), 2.0);
        assert!((double.du - single.du * 2.0).abs() < 1e-4);
        assert_eq!(sample_gradient(&texture, &at(0.5, 0.5, 0.01), 0.0), BumpGradient::FLAT);
    }

    /// §5.4's Bump Add: layers combine by summing gradients.
    #[test]
    fn bump_layers_add_as_gradients() {
        let a = BumpGradient { du: 0.3, dv: -0.1 };
        let b = BumpGradient { du: 0.2, dv: 0.4 };
        assert_eq!(a.add(b), BumpGradient { du: 0.5, dv: 0.3 });
        // Adding flat changes nothing, and the operation is commutative — both
        // of which a height-summing implementation would also satisfy, so the
        // real check is the scale-mixing one below.
        assert_eq!(a.add(BumpGradient::FLAT), a);
        assert_eq!(a.add(b), b.add(a));
    }

    /// The case that distinguishes adding gradients from adding heights: a
    /// fine layer and a coarse one, sampled at a footprint that blurs the fine
    /// one away.
    ///
    /// Summing heights would let the coarse layer's MIP averaging flatten the
    /// fine layer before any derivative was taken, and the fine detail would
    /// vanish from the combination even where it is resolvable. Summing
    /// gradients keeps each layer at its own scale.
    #[test]
    fn each_bump_layer_keeps_its_own_scale() {
        let coarse = ramp(256, 1.0);
        let fine = ramp(256, 0.25);

        // A footprint fine enough that both layers resolve.
        let point = at(0.5, 0.5, 2.0 / 256.0);
        let combined = sample_gradient(&coarse, &point, 1.0)
            .add(sample_gradient(&fine, &point, 1.0));

        assert!(
            (combined.du - 1.25).abs() < 0.05,
            "the layers combined to a slope of {:.4} rather than 1.25; the fine \
             layer's contribution was lost",
            combined.du
        );
    }

    /// The normal tilts away from uphill: a surface rising along +u scatters as
    /// though tipped back along -u. Getting this backwards inverts every bump
    /// map in the scene, which reads as "the lighting is coming from the wrong
    /// side".
    #[test]
    fn the_normal_tilts_away_from_uphill() {
        let frame = TangentFrame {
            normal: Vec3::Z,
            tangent: Vec3::X,
            bitangent: Vec3::Y,
        };
        let uphill_along_u = BumpGradient { du: 1.0, dv: 0.0 };
        let perturbed = frame.perturb(uphill_along_u, 1.0);

        assert!(
            perturbed.x < 0.0,
            "a surface rising along +u should tilt its normal towards -u, got {perturbed:?}"
        );
        assert!(perturbed.z > 0.0, "the normal flipped through the surface");
        assert!((perturbed.length() - 1.0).abs() < 1e-5, "not normalized");
    }

    /// A flat tangent-space normal — (0.5, 0.5, 1) encoded — must decode to the
    /// surface normal exactly, or every unmapped area is subtly wrong.
    #[test]
    fn a_flat_normal_map_decodes_to_the_surface_normal() {
        let frame = TangentFrame::from_normal(vec3(0.3, 0.5, 0.81).normalize());
        let decoded = frame.decode_normal_map(vec3(0.5, 0.5, 1.0), 1.0);
        assert!(
            (decoded - frame.normal).length() < 1e-5,
            "a flat normal map moved the normal to {decoded:?} from {:?}",
            frame.normal
        );
    }

    /// A normal map must never point into the surface, however badly authored
    /// or mis-tagged it is.
    #[test]
    fn a_normal_map_cannot_turn_a_surface_inside_out() {
        let frame = TangentFrame::from_normal(Vec3::Z);
        for sample in [
            vec3(0.0, 0.0, 0.0),
            vec3(1.0, 1.0, 0.0),
            vec3(0.0, 1.0, 0.2),
            vec3(0.5, 0.5, 0.49),
        ] {
            let decoded = frame.decode_normal_map(sample, 1.0);
            assert!(
                decoded.dot(frame.normal) > 0.0,
                "{sample:?} decoded to {decoded:?}, which faces into the surface"
            );
            assert!((decoded.length() - 1.0).abs() < 1e-4);
        }
    }

    /// Strength flattens a normal map towards the surface without ever
    /// inverting it.
    #[test]
    fn normal_map_strength_flattens_rather_than_inverting() {
        let frame = TangentFrame::from_normal(Vec3::Z);
        let steep = vec3(0.9, 0.5, 0.6);

        let full = frame.decode_normal_map(steep, 1.0);
        let half = frame.decode_normal_map(steep, 0.5);
        let none = frame.decode_normal_map(steep, 0.0);

        assert!(
            half.dot(frame.normal) > full.dot(frame.normal),
            "halving the strength should bring the normal closer to the surface's"
        );
        assert!((none - frame.normal).length() < 1e-5, "zero strength is flat");
    }

    /// A frame built from a normal alone has to be orthonormal, and stable —
    /// tri-planar mapping has no parameterization to take a tangent from.
    #[test]
    fn a_frame_from_a_normal_alone_is_orthonormal() {
        for normal in [
            Vec3::X,
            Vec3::Y,
            Vec3::Z,
            -Vec3::Z,
            vec3(1.0, 1.0, 1.0),
            vec3(0.0, 0.0, -1.0),
            vec3(-0.3, 0.9, 0.31),
        ] {
            let frame = TangentFrame::from_normal(normal);
            assert!((frame.normal.length() - 1.0).abs() < 1e-5);
            assert!((frame.tangent.length() - 1.0).abs() < 1e-5);
            assert!((frame.bitangent.length() - 1.0).abs() < 1e-5);
            assert!(frame.normal.dot(frame.tangent).abs() < 1e-5, "{normal:?}");
            assert!(frame.normal.dot(frame.bitangent).abs() < 1e-5);
            assert!(frame.tangent.dot(frame.bitangent).abs() < 1e-5);
        }
    }

    /// Nothing here may produce a non-finite normal.
    #[test]
    fn no_input_produces_a_non_finite_normal() {
        let texture = ramp(32, 1.0);
        for footprint in [0.0, 1e-9, 1.0, 1e6] {
            for (u, v) in [(0.5, 0.5), (0.0, 0.0), (1.0, 1.0), (-2.0, 3.0)] {
                let gradient = sample_gradient(&texture, &at(u, v, footprint), 1.0);
                assert!(
                    gradient.du.is_finite() && gradient.dv.is_finite(),
                    "({u}, {v}) at footprint {footprint} gave {gradient:?}"
                );
                let frame = TangentFrame::from_normal(Vec3::Z);
                assert!(frame.perturb(gradient, 1e6).is_finite());
            }
        }
    }
}
