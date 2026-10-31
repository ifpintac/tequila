//! Turning a point on a surface into texture coordinates
//! (`docs/02-MATERIAL-SYSTEM.md` §5.3).
//!
//! Most renderers assume a mesh arrives with UV coordinates. CAD geometry does
//! not: a STEP file describes solids, and tessellating one gives triangles with
//! positions and normals and nothing else. §5.3 calls tri-planar "the workhorse
//! for CAD geometry, which usually has no UVs", and for this product that is the
//! common case rather than the fallback.
//!
//! Each mode here maps a point — and, where it matters, a normal — to a `(u, v)`
//! pair, together with the footprint that [`MipPyramid::sample`] needs. The
//! footprint is not an afterthought: a projection that returns coordinates
//! without them forces the caller to guess a MIP level, and guessing gives back
//! the aliasing the pyramid existed to remove.
//!
//! ## The cylindrical seam
//!
//! §5.3 asks for "derivative-aware" seam handling on the cylindrical
//! projection, and it is worth spelling out what goes wrong without it.
//! Cylindrical `u` is an angle divided by a full turn, so it runs 0 → 1 and
//! then jumps back to 0. Two neighbouring points either side of that jump have
//! `u` values of about 0.999 and 0.001. A naive derivative reads that as a
//! footprint spanning 99.8% of the texture, selects the very top of the
//! pyramid, and paints a blurred stripe down the seam of every cylinder in the
//! scene. Measuring the distance the short way around instead — which is what
//! [`wrapped_delta`] does — gives the true footprint of 0.002.

use crate::mip::Wrap;
use tequila_core::math::{Vec2, Vec3};

/// A principal axis, for the nodes that have a direction.
///
/// Three fixed axes rather than an arbitrary vector. Marble veins run along the
/// slab and wood grain runs along the plank, and in both cases the part was
/// modelled with that direction on an axis — an arbitrary vector would be three
/// numbers to get right where one choice of three is always correct.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Axis {
    X,
    #[default]
    Y,
    Z,
}

impl Axis {
    pub const ALL: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];

    pub fn label(self) -> &'static str {
        match self {
            Axis::X => "X",
            Axis::Y => "Y",
            Axis::Z => "Z",
        }
    }

    /// The component along this axis.
    #[inline]
    pub fn of(self, p: Vec3) -> f32 {
        match self {
            Axis::X => p.x,
            Axis::Y => p.y,
            Axis::Z => p.z,
        }
    }

    /// The two components across it, which is the plane the rings live in.
    #[inline]
    pub fn across(self, p: Vec3) -> (f32, f32) {
        match self {
            Axis::X => (p.y, p.z),
            Axis::Y => (p.x, p.z),
            Axis::Z => (p.x, p.y),
        }
    }

    /// The inverse of [`Axis::across`] and [`Axis::of`] together: the point
    /// with these two components across the axis and this one along it.
    ///
    /// Needed to go backwards through a projection — to ask "where on the part
    /// is the corner of this artwork", which is what draws a placement frame in
    /// the viewport. The component order has to match `across` exactly or the
    /// frame is drawn somewhere other than where the label is.
    #[inline]
    pub fn compose(self, a: f32, b: f32, along: f32) -> Vec3 {
        match self {
            Axis::X => Vec3 {
                x: along,
                y: a,
                z: b,
            },
            Axis::Y => Vec3 {
                x: a,
                y: along,
                z: b,
            },
            Axis::Z => Vec3 {
                x: a,
                y: b,
                z: along,
            },
        }
    }
}

/// How a surface point becomes a texture coordinate (§5.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Projection {
    /// The mesh's own UV set. The node selects which set.
    #[default]
    Uv,
    /// Flat, looking down [`Mapping::axis`].
    Planar,
    /// Wrapped around [`Mapping::axis`].
    Cylindrical,
    /// Equirectangular, from the mapping frame's origin.
    Spherical,
    /// Three planar projections blended by the normal. The CAD workhorse.
    TriPlanar,
}

/// The shared per-mapping controls of §5.3.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mapping {
    pub projection: Projection,
    /// Which axis the projection is built about: [`Projection::Planar`] looks
    /// down it, [`Projection::Cylindrical`] wraps around it. Ignored by the
    /// other modes, which have no direction to choose.
    ///
    /// Defaults to Z, which is what a planar projection has always done here.
    /// A cylinder is more often wrapped about Y, so a mapping that wants the
    /// usual upright cylinder has to say so — rather than have one field mean
    /// a different axis depending on the mode beside it.
    pub axis: Axis,
    /// Which of the mesh's UV sets [`Projection::Uv`] reads.
    pub uv_set: usize,
    /// Texture repeats across the projected extent.
    pub scale: Vec3,
    pub offset: Vec2,
    /// Rotation within the texture plane, in turns.
    pub rotation: f32,
    /// The point [`Mapping::rotation`] turns about, in scaled coordinates —
    /// after [`Mapping::scale`], before [`Mapping::offset`].
    ///
    /// Zero, the default, turns about the projection's own origin. That is
    /// fine for a texture that tiles forever and wrong for a single piece of
    /// artwork: a label placed on the side of a box and then rotated would
    /// swing away across the surface instead of turning where it sits, which
    /// reads as the rotation control being broken. Placing the pivot at the
    /// artwork's centre makes it turn in place.
    pub pivot: Vec2,
    pub wrap_u: Wrap,
    pub wrap_v: Wrap,
    pub flip_u: bool,
    pub flip_v: bool,
    /// Blend exponent for [`Projection::TriPlanar`]. §5.3 suggests 4–8: higher
    /// is a tighter blend with less cross-fading on a flat face, lower is a
    /// softer transition across an edge.
    pub blend_sharpness: f32,
    /// Interpret [`Mapping::scale`] as a size in scene units rather than as a
    /// number of repeats.
    ///
    /// §5.3 calls this "essential when scene units matter, and something users
    /// of CAD-derived scenes expect": a brushed-metal texture specified as 50 mm
    /// across should stay 50 mm across whether it lands on a bracket or on a
    /// door panel, and under a repeat count it would not.
    pub real_world_units: bool,
    /// Turn the texture as a whole: [`Mapping::rotation`] turns it before
    /// [`Mapping::scale`] stretches it, so stretched artwork - a label wider
    /// than it is tall - turns without shearing.
    ///
    /// Off, the order glTF's texture transform and Blender's point mapping
    /// use, and so what an imported or exported texture means: stretched,
    /// then turned, which keeps the stretch along the coordinates' own axes -
    /// a wide label turned a little comes out squashed. Labels are rigid.
    pub rigid: bool,
}

impl Default for Mapping {
    fn default() -> Self {
        Mapping {
            projection: Projection::Uv,
            axis: Axis::Z,
            uv_set: 0,
            scale: Vec3::ONE,
            offset: Vec2::ZERO,
            rotation: 0.0,
            pivot: Vec2::ZERO,
            wrap_u: Wrap::Repeat,
            wrap_v: Wrap::Repeat,
            flip_u: false,
            flip_v: false,
            blend_sharpness: 6.0,
            real_world_units: false,
            rigid: false,
        }
    }
}

/// What a projection produced: coordinates, and how big the shading point's
/// footprint is in them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextureCoords {
    pub u: f32,
    pub v: f32,
    /// Footprint along each axis, in texture coordinates.
    ///
    /// These are kept separately rather than collapsed, because for an angular
    /// projection they genuinely differ: cylindrical `u` compresses with
    /// distance from the axis while `v` does not, so a point far out has a
    /// footprint many times wider in one axis than the other.
    ///
    /// A trilinear lookup can only take one number and has to use
    /// [`TextureCoords::width`], the larger of the two — which over-blurs the
    /// narrow axis. That is the right way to be wrong (blur beats aliasing),
    /// and it is exactly the case anisotropic filtering exists to fix, so the
    /// information is preserved here rather than thrown away at the projection.
    pub width_u: f32,
    pub width_v: f32,
}

impl TextureCoords {
    /// The isotropic footprint a trilinear lookup should use: the larger axis,
    /// so detail is lost rather than aliased.
    #[inline]
    pub fn width(&self) -> f32 {
        self.width_u.max(self.width_v)
    }

    /// How far from isotropic the footprint is. 1 is square; larger means one
    /// axis is being over-blurred by [`TextureCoords::width`].
    #[inline]
    pub fn anisotropy(&self) -> f32 {
        let (lo, hi) = if self.width_u < self.width_v {
            (self.width_u, self.width_v)
        } else {
            (self.width_v, self.width_u)
        };
        if lo <= 1e-12 {
            1.0
        } else {
            hi / lo
        }
    }
}

/// One contribution to a blended lookup.
///
/// [`Projection::TriPlanar`] returns three of these; every other mode returns
/// one with a weight of 1. Blending happens at the *sample* rather than at the
/// coordinate, because averaging three sets of coordinates and then sampling
/// once would fetch from a position on the texture that none of the three
/// projections asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contribution {
    pub coords: TextureCoords,
    pub weight: f32,
}

/// Everything a projection needs about the shading point.
#[derive(Clone, Copy, Debug)]
pub struct SurfacePoint {
    /// Position in the mapping frame's space.
    ///
    /// The renderer supplies this in the part's **object space**, which is
    /// what §6 and §5.3 mean by patterns belonging to the part: a projection
    /// fed world-space positions paints a pattern onto the room and lets the
    /// part slide through it. `SceneSnapshot::shading_point` is where the
    /// conversion happens.
    pub position: Vec3,
    /// Shading normal in the same space, normalized.
    pub normal: Vec3,
    /// The mesh's own coordinates, for [`Projection::Uv`].
    pub uv: Vec2,
    /// Radius of the ray's footprint on the surface, in scene units. The
    /// integrator's ray differentials supply this.
    pub footprint: f32,
    /// Scene units per unit of `position`: the part's own scale.
    ///
    /// Patterns multiply by it, so their frequencies are per scene unit as
    /// documented. Object space alone is the mesh's own units, and an
    /// imported model scaled down fifty times (centimetres under a 1/100
    /// node) shaded every pattern fifty times finer than asked.
    pub scale: f32,
    /// The footprint in UV units, along u and along v, when the caller knows
    /// how the surface is parameterized.
    ///
    /// What a UV lookup needs to pick its MIP level. `footprint` is a length
    /// in the mesh's own units, and treating it as a width in UV was right
    /// only by coincidence: on a model imported at a fiftieth of its units'
    /// size the lookup averaged fifty times too many texels, and logos,
    /// print and fabric disappeared from the render. `None` falls back to
    /// that old reading, for callers with no mesh to measure.
    pub uv_footprint: Option<[f32; 2]>,
    /// Mean curvature of the surface here, in inverse scene units: positive
    /// on convex edges, negative in creases, zero on flat faces. What the
    /// Curvature node reads; zero where nothing measured it.
    pub curvature: f32,
    /// How much of the hemisphere above the surface here is blocked by the
    /// part itself within the Occlusion node's radius, 0 open to 1 closed
    /// (`docs/02` §6.2). Zero where nothing measured it.
    pub occlusion: f32,
    /// The mesh's vertex colour here, in the rendering space; white where
    /// the mesh has none.
    pub vertex_color: tequila_core::spectrum::Spectrum,
    /// The world around the point, for the material nodes that read it.
    pub world: PointWorld,
}

/// What a shading point knows of the world around it, for the Blender
/// nodes that read it (`tequila_materials::nodes::PointInput`): Geometry,
/// Layer Weight, Fresnel, Object Info, Texture Coordinate's Generated.
/// Zeros where nothing filled it in, as in a flat preview swatch.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PointWorld {
    /// World space.
    pub position: Vec3,
    /// The shading normal, in world space.
    pub normal: Vec3,
    /// The geometric normal, in world space.
    pub true_normal: Vec3,
    /// Towards the viewer, unit length, in world space.
    pub incoming: Vec3,
    /// Seen from behind its geometric normal.
    pub backfacing: bool,
    /// Object space normalized to the mesh's bounding box, 0 to 1.
    pub generated: Vec3,
    /// The part's origin, in world space.
    pub object_location: Vec3,
    /// 0 to 1, the same over a part, different between parts.
    pub object_random: f32,
    /// The kind of ray that came to the point, for the Light Path node.
    pub ray: RayKind,
}

/// The kind of ray a shading point was reached by: the camera's, a shadow
/// ray's, or a bounce of one kind (`tequila_materials::nodes::PointInput`'s
/// ray outputs, Cycles' Light Path node's).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RayKind {
    /// What the camera sees, directly.
    #[default]
    Camera,
    /// A shadow ray, testing what stands between a point and a light.
    Shadow,
    /// After a diffuse bounce.
    Diffuse,
    /// After a glossy bounce: a reflection.
    Glossy,
    /// After passing through glass.
    Transmission,
}

impl Mapping {
    /// Project a shading point, returning up to three weighted contributions.
    /// The projected coordinates, before scale, rotation and offset.
    ///
    /// `None` for [`Projection::TriPlanar`], which has three frames rather than
    /// one and so has no single place to put a piece of artwork.
    ///
    /// Exists so that placing a label — working out the offset that puts the
    /// artwork's centre on a point the user clicked — reads the same formula
    /// the renderer reads. Written out a second time in the placement code, the
    /// two would agree until one of them changed, and the symptom would be a
    /// label landing next to the cursor rather than under it.
    pub fn raw_coords(&self, point: &SurfacePoint) -> Option<(f32, f32)> {
        self.frame_coords(&self.in_frame_units(point))
    }

    /// [`Mapping::raw_coords`] for a point already in the frame's units.
    fn frame_coords(&self, point: &SurfacePoint) -> Option<(f32, f32)> {
        Some(match self.projection {
            Projection::Uv => (point.uv.x, point.uv.y),
            Projection::Planar => self.axis.across(point.position),
            Projection::Cylindrical => {
                let (a, b) = self.axis.across(point.position);
                (
                    b.atan2(a) / std::f32::consts::TAU + 0.5,
                    self.axis.of(point.position),
                )
            }
            Projection::Spherical => {
                let radius = point.position.length();
                // The projection is undefined at its own centre — every
                // direction is equally the answer — and `normalize` there
                // returns NaN, which would propagate into the texture lookup
                // and out into the image as a black or white pixel. Any finite
                // coordinate is better than that.
                if radius < 1e-6 {
                    (0.0, 0.5)
                } else {
                    let direction = point.position / radius;
                    (
                        direction.z.atan2(direction.x) / std::f32::consts::TAU + 0.5,
                        direction.y.clamp(-1.0, 1.0).acos() / std::f32::consts::PI,
                    )
                }
            }
            Projection::TriPlanar => return None,
        })
    }

    /// Project a shading point, returning up to three weighted contributions.
    pub fn project(&self, point: &SurfacePoint) -> Vec<Contribution> {
        let point = &self.in_frame_units(point);
        if self.projection == Projection::TriPlanar {
            return self.tri_planar(point);
        }
        let (u, v) = self
            .frame_coords(point)
            .expect("every mode but tri-planar has one frame");

        // The footprint is what differs between the modes, not the
        // coordinates: an angular projection compresses `u` with distance from
        // its axis while `v` is a straight length along it.
        let coords = match self.projection {
            Projection::Uv => match point.uv_footprint {
                Some([width_u, width_v]) => self.finish_anisotropic(u, v, width_u, width_v),
                None => self.finish(u, v, point.footprint),
            },
            Projection::Planar => self.finish(u, v, point.footprint),
            Projection::Cylindrical => {
                let (a, b) = self.axis.across(point.position);
                let radius = (a * a + b * b).sqrt().max(1e-6);
                let angular = point.footprint / (radius * std::f32::consts::TAU);
                self.finish_anisotropic(u, v, angular, point.footprint)
            }
            Projection::Spherical => {
                let radius = point.position.length();
                if radius < 1e-6 {
                    self.finish(u, v, point.footprint)
                } else {
                    let angular = point.footprint / (radius * std::f32::consts::TAU);
                    self.finish_anisotropic(u, v, angular, angular * 2.0)
                }
            }
            Projection::TriPlanar => unreachable!("returned above"),
        };
        vec![Contribution {
            coords,
            weight: 1.0,
        }]
    }

    /// The point as a positional projection with real-world units measures
    /// it: in scene units, the part's own scale applied.
    ///
    /// A real-world size is a size in the scene — a 5 cm weave, a 20 cm
    /// tile — and object space is only the mesh's own units. A model whose
    /// mesh is in centimetres under a 1/100 node (the Quest headset from
    /// Sketchfab, and most glTF exports from tools that work in centimetres)
    /// read every real-world size a hundred times too small: a weave of a
    /// millimetre came out a hundredth of one, averaged to flat grey. The
    /// unmapped pattern path already multiplied by the scale; this is the
    /// mapped one catching up. UV and repeat-count mappings are unchanged —
    /// neither is a length in the scene.
    fn in_frame_units(&self, point: &SurfacePoint) -> SurfacePoint {
        if !self.real_world_units || self.projection == Projection::Uv || point.scale == 1.0 {
            return *point;
        }
        SurfacePoint {
            position: point.position * point.scale,
            footprint: point.footprint * point.scale,
            ..*point
        }
    }

    /// Three planar projections, blended by how much the normal faces each
    /// axis (§5.3).
    fn tri_planar(&self, point: &SurfacePoint) -> Vec<Contribution> {
        let n = Vec3 {
            x: point.normal.x.abs(),
            y: point.normal.y.abs(),
            z: point.normal.z.abs(),
        };
        let k = self.blend_sharpness.max(1.0);
        let mut weights = [n.x.powf(k), n.y.powf(k), n.z.powf(k)];
        let total: f32 = weights.iter().sum();
        // A normal exactly on a diagonal, or a degenerate one, still has to
        // produce something rather than dividing by zero.
        if total <= 1e-12 {
            weights = [1.0 / 3.0; 3];
        } else {
            for w in &mut weights {
                *w /= total;
            }
        }

        let p = point.position;
        // Each plane is projected along the axis its normal faces, and the two
        // remaining axes become u and v. The ordering matters: chosen so that
        // the texture is not mirrored on opposite faces of a box, which reads
        // as a reversed logo on the far side.
        let planes = [
            (p.z, p.y, weights[0]), // facing X
            (p.x, p.z, weights[1]), // facing Y
            (p.x, p.y, weights[2]), // facing Z
        ];

        planes
            .into_iter()
            .filter(|(_, _, w)| *w > 1e-4)
            .map(|(u, v, weight)| Contribution {
                coords: self.finish(u, v, point.footprint),
                weight,
            })
            .collect()
    }

    /// Apply the shared controls and work out the footprint.
    fn finish(&self, u: f32, v: f32, footprint: f32) -> TextureCoords {
        self.finish_anisotropic(u, v, footprint, footprint)
    }

    /// As [`Mapping::finish`], for projections whose footprint differs between
    /// the two axes — anything angular, where `u` compresses with radius.
    fn finish_anisotropic(
        &self,
        u: f32,
        v: f32,
        footprint_u: f32,
        footprint_v: f32,
    ) -> TextureCoords {
        // Real-world units invert the meaning of `scale`: a scale of 50 means
        // "the texture spans 50 units" rather than "50 repeats across the
        // projected extent".
        let (su, sv) = if self.real_world_units {
            (
                1.0 / self.scale.x.abs().max(1e-6),
                1.0 / self.scale.y.abs().max(1e-6),
            )
        } else {
            (self.scale.x, self.scale.y)
        };

        let (mut u, mut v) = (u * su, v * sv);

        if self.rotation != 0.0 {
            (u, v) = self.spin(u, v, self.rotation, su, sv);
        }

        u += self.offset.x;
        v += self.offset.y;

        if self.flip_u {
            u = -u;
        }
        if self.flip_v {
            v = -v;
        }

        // The footprint scales with the coordinates. Rotation does not change
        // its size, so the two axes are simply scaled and carried through.
        TextureCoords {
            u,
            v,
            width_u: footprint_u * su.abs(),
            width_v: footprint_v * sv.abs(),
        }
    }

    pub fn wrap(&self) -> (Wrap, Wrap) {
        (self.wrap_u, self.wrap_v)
    }

    /// The scale as repeats per projected unit, whichever way it is stated.
    pub fn repeats(&self) -> (f32, f32) {
        if self.real_world_units {
            (1.0 / self.scale.x.abs().max(1e-6), 1.0 / self.scale.y.abs().max(1e-6))
        } else {
            (self.scale.x, self.scale.y)
        }
    }

    /// A turn of `turns` about the pivot, on coordinates already scaled by
    /// (`su`, `sv`): as it is, or - rigid - with the scale taken out for the
    /// turn and put back after, so the texture turns as a whole.
    fn spin(&self, u: f32, v: f32, turns: f32, su: f32, sv: f32) -> (f32, f32) {
        let (sin, cos) = (turns * std::f32::consts::TAU).sin_cos();
        // About the pivot, which is the origin unless someone placed it.
        let (pu, pv) = (u - self.pivot.x, v - self.pivot.y);
        if self.rigid {
            let safe = |s: f32| if s.abs() < 1e-12 { 1e-12f32.copysign(s) } else { s };
            let (du, dv) = (pu / safe(su), pv / safe(sv));
            ((du * cos - dv * sin) * su + self.pivot.x, (du * sin + dv * cos) * sv + self.pivot.y)
        } else {
            (pu * cos - pv * sin + self.pivot.x, pu * sin + pv * cos + self.pivot.y)
        }
    }

    /// Where a projected (u, v) lands on the texture: its scale, turn, offset
    /// and flips.
    pub fn apply_to(&self, u: f32, v: f32) -> (f32, f32) {
        let coords = self.finish(u, v, 0.0);
        (coords.u, coords.v)
    }

    /// The projected (u, v) that lands on (`u`, `v`) of the texture:
    /// [`Mapping::apply_to`] undone, the forward order reversed. Getting the
    /// order wrong is not visible in the arithmetic: it shows as a frame that
    /// is right until the label is turned, and then drifts.
    pub fn unapply(&self, u: f32, v: f32) -> (f32, f32) {
        let (mut u, mut v) = (if self.flip_u { -u } else { u }, if self.flip_v { -v } else { v });
        u -= self.offset.x;
        v -= self.offset.y;
        let (su, sv) = self.repeats();
        if self.rotation != 0.0 {
            (u, v) = self.spin(u, v, -self.rotation, su, sv);
        }
        let safe = |s: f32| if s.abs() < 1e-9 { 1e-9 } else { s };
        (u / safe(su), v / safe(sv))
    }

    /// The artwork mirrored where it sits: across its width (`horizontal`)
    /// or its height.
    ///
    /// A flip on its own negates the finished coordinate, and a label is the
    /// one piece of artwork between 0 and 1, so flipping alone would move it a
    /// whole artwork-width away, usually off the part. Moving the offset by
    /// one as well keeps that square where it was, and the picture turns over
    /// inside it.
    pub fn mirrored(&self, horizontal: bool) -> Mapping {
        let mut out = *self;
        let (flip, offset) = if horizontal {
            (&mut out.flip_u, &mut out.offset.x)
        } else {
            (&mut out.flip_v, &mut out.offset.y)
        };
        *offset += if *flip { 1.0 } else { -1.0 };
        *flip = !*flip;
        out
    }
}

/// The distance between two coordinates on a repeating axis, measured the short
/// way around.
///
/// This is §5.3's "derivative-aware" seam handling. Two points either side of a
/// cylindrical seam have `u` values near 1 and near 0; subtracting them gives
/// almost a whole turn, which selects the coarsest MIP level and draws a blurred
/// stripe down the seam. The short way round gives the true separation.
#[inline]
pub fn wrapped_delta(a: f32, b: f32) -> f32 {
    let direct = (a - b).abs();
    direct.min(1.0 - direct)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::{vec2, vec3};

    fn point(position: Vec3, normal: Vec3) -> SurfacePoint {
        SurfacePoint {
            position,
            normal: normal.normalize(),
            uv: vec2(0.25, 0.75),
            footprint: 0.01,
            scale: 1.0,
            uv_footprint: None,
            curvature: 0.0,
            occlusion: 0.0,
            vertex_color: tequila_core::spectrum::Spectrum::ONE,
            world: Default::default(),
        }
    }

    fn only(contributions: &[Contribution]) -> TextureCoords {
        assert_eq!(contributions.len(), 1, "expected a single contribution");
        contributions[0].coords
    }

    /// Artwork twice as wide as it is tall, turned an eighth: rigid, its
    /// corners on the layout are still a rectangle twice as wide as tall;
    /// as glTF turns a texture, they are not. Undone, every point comes back.
    #[test]
    fn a_rigid_turn_keeps_stretched_artwork_rectangular() {
        let base = Mapping { scale: vec3(2.0, 4.0, 1.0), rotation: 0.125, pivot: vec2(0.6, 1.2), offset: vec2(0.1, -0.3), ..Mapping::default() };
        let corners = |m: &Mapping| [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].map(|(u, v)| m.unapply(u, v));
        let edge = |a: (f32, f32), b: (f32, f32)| ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        let right_angle = |c: [(f32, f32); 4]| {
            let (e1, e2) = ((c[1].0 - c[0].0, c[1].1 - c[0].1), (c[3].0 - c[0].0, c[3].1 - c[0].1));
            (e1.0 * e2.0 + e1.1 * e2.1).abs() < 1e-5
        };
        let rigid = Mapping { rigid: true, ..base };
        let c = corners(&rigid);
        assert!(right_angle(c), "a rectangle still: {c:?}");
        assert!((edge(c[0], c[1]) / edge(c[0], c[3]) - 2.0).abs() < 1e-4, "twice as wide as tall: {c:?}");
        assert!(!right_angle(corners(&base)), "glTF's order shears it, as glTF does");
        for m in [base, rigid] {
            for (u, v) in [(0.1, 0.2), (0.9, -0.4), (2.5, 1.5)] {
                let (a, b) = m.apply_to(u, v);
                let (x, y) = m.unapply(a, b);
                assert!((x - u).abs() < 1e-4 && (y - v).abs() < 1e-4, "rigid {}: ({u},{v}) came back as ({x},{y})", m.rigid);
            }
        }
        // Scaled evenly, the two orders are the same thing.
        let even = Mapping { scale: vec3(3.0, 3.0, 1.0), ..base };
        let (a, b) = (even.apply_to(0.3, 0.7), Mapping { rigid: true, ..even }.apply_to(0.3, 0.7));
        assert!((a.0 - b.0).abs() < 1e-5 && (a.1 - b.1).abs() < 1e-5);
    }

    /// Mirroring a label turns the picture over where it sits: every point
    /// that showed the artwork still does, at the mirrored spot in it, and a
    /// second mirror puts it back exactly.
    #[test]
    fn a_mirrored_label_stays_where_it_was() {
        let label = Mapping {
            projection: Projection::Planar,
            real_world_units: true,
            scale: vec3(0.2, 0.1, 1.0),
            offset: vec2(0.5 - 1.3 / 0.2, 0.5 - 0.4 / 0.1),
            pivot: vec2(1.3 / 0.2, 0.4 / 0.1),
            rotation: 0.07,
            wrap_u: Wrap::Clamp,
            wrap_v: Wrap::Clamp,
            ..Default::default()
        };
        for horizontal in [true, false] {
            let mirrored = label.mirrored(horizontal);
            for (x, y) in [(1.3, 0.4), (1.36, 0.43), (1.25, 0.37), (1.4, 0.5), (2.0, 0.4)] {
                let at = point(vec3(x, y, 0.0), Vec3::Z);
                let (a, b) = (only(&label.project(&at)), only(&mirrored.project(&at)));
                let (want_u, want_v) = if horizontal { (1.0 - a.u, a.v) } else { (a.u, 1.0 - a.v) };
                assert!(
                    (b.u - want_u).abs() < 1e-4 && (b.v - want_v).abs() < 1e-4,
                    "at ({x}, {y}) the label read ({}, {}); mirrored it read ({}, {}), not ({want_u}, {want_v})",
                    a.u,
                    a.v,
                    b.u,
                    b.v
                );
            }
            assert_eq!(mirrored.mirrored(horizontal), label, "mirroring twice is not where it started");
        }
    }

    #[test]
    fn uv_passes_the_meshs_own_coordinates_through() {
        let mapping = Mapping::default();
        let coords = only(&mapping.project(&point(vec3(9.0, 9.0, 9.0), Vec3::Y)));
        assert!((coords.u - 0.25).abs() < 1e-6);
        assert!((coords.v - 0.75).abs() < 1e-6);
    }

    /// The seam is the whole reason §5.3 asks for derivative-aware handling.
    #[test]
    fn the_cylindrical_seam_does_not_blow_up_the_footprint() {
        // Two points a hair either side of the wrap, at radius 1.
        let epsilon = 0.001;
        let before = wrapped_delta(1.0 - epsilon, 0.0 + epsilon);
        assert!(
            before < 0.01,
            "the seam measured {before} across; a naive subtraction would give \
             nearly a whole turn and blur a stripe down every cylinder"
        );
        // Away from the seam it is an ordinary distance.
        assert!((wrapped_delta(0.30, 0.35) - 0.05).abs() < 1e-6);
        assert!((wrapped_delta(0.5, 0.5) - 0.0).abs() < 1e-6);
    }

    /// Cylindrical `u` must run once around, and its footprint must shrink as
    /// the point moves away from the axis — the same arc length subtends less
    /// angle further out.
    #[test]
    fn cylindrical_wraps_once_and_narrows_with_radius() {
        let mapping = Mapping {
            projection: Projection::Cylindrical,
            axis: Axis::Y,
            ..Default::default()
        };

        // Which axis it wraps about is now a choice, so it has to be stated
        // and checked. Every point below sits on the X axis, which lies in the
        // XY and XZ planes both — so the assertions that follow would hold for
        // a cylinder about Y or about Z, and this is the one that separates
        // them: `v` runs along the axis of the wrap.
        let up = only(&mapping.project(&point(vec3(1.0, 0.75, 0.0), Vec3::X)));
        assert!(
            (up.v - 0.75).abs() < 1e-5,
            "v read {} at 0.75 up the Y axis; the cylinder is not wrapped \
             about Y",
            up.v
        );

        let front = only(&mapping.project(&point(vec3(1.0, 0.0, 0.0), Vec3::X)));
        let back = only(&mapping.project(&point(vec3(-1.0, 0.0, 0.0), -Vec3::X)));
        assert!((front.u - 0.5).abs() < 1e-5, "front sat at {}", front.u);
        assert!(
            back.u < 1e-5 || back.u > 1.0 - 1e-5,
            "the seam should be behind, but it sat at {}",
            back.u
        );

        // The angular footprint narrows with radius: the same arc length
        // subtends a tenth of the angle ten times further out. It is `width_u`
        // that carries this — `width_v` is a straight distance along the axis
        // and does not change, which is precisely why the two are kept apart.
        let near = only(&mapping.project(&point(vec3(1.0, 0.0, 0.0), Vec3::X)));
        let far = only(&mapping.project(&point(vec3(10.0, 0.0, 0.0), Vec3::X)));
        assert!(
            (far.width_u - near.width_u * 0.1).abs() < 1e-6,
            "ten times further out gave a u footprint of {} against {}",
            far.width_u,
            near.width_u
        );
        assert!(
            (far.width_v - near.width_v).abs() < 1e-9,
            "the axial footprint should not change with radius"
        );

        // ...and a trilinear lookup, which gets one number, has to take the
        // wider axis and over-blur the narrow one.
        assert!(far.width() >= far.width_u);
        assert!(
            far.anisotropy() > 5.0,
            "a cylinder seen from ten units out is {}:1 anisotropic; \
             trilinear filtering will visibly over-blur it",
            far.anisotropy()
        );
    }

    /// A planar projection looks down its axis: the two axes across it become
    /// u and v, and moving along it changes nothing.
    ///
    /// This is what lets a label land on the face of a part that was chosen
    /// rather than on whichever face happens to point down Z.
    #[test]
    fn planar_projects_down_the_axis_it_was_given() {
        let position = vec3(0.3, 0.5, 0.7);
        for axis in Axis::ALL {
            let mapping = Mapping {
                projection: Projection::Planar,
                axis,
                ..Default::default()
            };
            let flat = only(&mapping.project(&point(position, Vec3::Y)));
            let (expect_u, expect_v) = match axis {
                Axis::X => (0.5, 0.7),
                Axis::Y => (0.3, 0.7),
                Axis::Z => (0.3, 0.5),
            };
            assert!(
                (flat.u - expect_u).abs() < 1e-6 && (flat.v - expect_v).abs() < 1e-6,
                "{axis:?}: projected to ({}, {}) rather than ({expect_u}, \
                 {expect_v})",
                flat.u,
                flat.v
            );

            // Depth down the axis must not move the artwork, or a label would
            // slide as the surface it sits on curves away.
            let deeper = only(&mapping.project(&point(
                position
                    + match axis {
                        Axis::X => vec3(5.0, 0.0, 0.0),
                        Axis::Y => vec3(0.0, 5.0, 0.0),
                        Axis::Z => vec3(0.0, 0.0, 5.0),
                    },
                Vec3::Y,
            )));
            assert!(
                (deeper.u - flat.u).abs() < 1e-6 && (deeper.v - flat.v).abs() < 1e-6,
                "{axis:?}: moving five units down the projection axis moved \
                 the coordinates"
            );
        }
    }

    /// A rotation turns the artwork about its pivot, and with the pivot at the
    /// artwork's centre that means it turns in place.
    ///
    /// Without this, placing a label on the side of a box and reaching for the
    /// rotation control swings it away across the surface and off the part,
    /// which reads as the control being broken rather than as a choice about
    /// where the origin is.
    #[test]
    fn rotation_turns_about_the_pivot() {
        // Artwork one unit across, centred on the object's origin: the centre
        // of the part lands at coordinate 0.5, so that is the pivot.
        let centred = |rotation: f32| Mapping {
            projection: Projection::Planar,
            real_world_units: true,
            scale: vec3(1.0, 1.0, 1.0),
            offset: vec2(0.5, 0.5),
            pivot: vec2(0.0, 0.0),
            rotation,
            ..Default::default()
        };

        // The centre of the artwork is the one point a rotation in place must
        // leave alone. Here the pivot is at the origin and the part's centre
        // is too, so it already holds...
        let still = only(&centred(0.0).project(&point(Vec3::ZERO, Vec3::Z)));
        let turned = only(&centred(0.125).project(&point(Vec3::ZERO, Vec3::Z)));
        assert!((still.u - turned.u).abs() < 1e-6 && (still.v - turned.v).abs() < 1e-6);

        // ...and here is the case that matters: a part modelled away from the
        // origin, as an imported part almost always is. Without a pivot the
        // artwork swings around the origin; with the pivot on the part it
        // turns where it sits.
        let centre = vec3(4.0, 3.0, 0.0);
        let swinging = Mapping {
            offset: vec2(0.5 - 4.0, 0.5 - 3.0),
            ..centred(0.25)
        };
        let swung = only(&swinging.project(&point(centre, Vec3::Z)));
        assert!(
            (swung.u - 0.5).abs() > 0.5,
            "the artwork's centre stayed near 0.5 ({}) without a pivot; this \
             test cannot show what the pivot fixes",
            swung.u
        );

        let pinned = Mapping {
            pivot: vec2(4.0, 3.0),
            ..swinging
        };
        let held = only(&pinned.project(&point(centre, Vec3::Z)));
        assert!(
            (held.u - 0.5).abs() < 1e-5 && (held.v - 0.5).abs() < 1e-5,
            "the artwork's centre moved to ({}, {}) when it was rotated about \
             its own pivot",
            held.u,
            held.v
        );
    }

    /// `compose` has to be the exact inverse of `across` and `of`, or a frame
    /// drawn from it appears somewhere other than where the label is.
    #[test]
    fn compose_inverts_across_and_of() {
        let p = vec3(1.5, -2.25, 0.75);
        for axis in Axis::ALL {
            let (a, b) = axis.across(p);
            let back = axis.compose(a, b, axis.of(p));
            assert_eq!(back, p, "{axis:?} did not round trip");
        }
    }

    /// Spherical is equirectangular: poles at v = 0 and 1, equator at 0.5.
    #[test]
    fn spherical_is_equirectangular() {
        let mapping = Mapping {
            projection: Projection::Spherical,
            ..Default::default()
        };
        let top = only(&mapping.project(&point(vec3(0.0, 1.0, 0.0), Vec3::Y)));
        let bottom = only(&mapping.project(&point(vec3(0.0, -1.0, 0.0), -Vec3::Y)));
        let equator = only(&mapping.project(&point(vec3(1.0, 0.0, 0.0), Vec3::X)));

        assert!(top.v.abs() < 1e-5, "the pole sat at v = {}", top.v);
        assert!((bottom.v - 1.0).abs() < 1e-5);
        assert!((equator.v - 0.5).abs() < 1e-5);
    }

    /// A face pointing straight down an axis must take its texture entirely
    /// from that axis's plane — no cross-fading on a flat surface, which is
    /// what a low blend exponent would give.
    #[test]
    fn tri_planar_does_not_cross_fade_on_a_flat_face() {
        let mapping = Mapping {
            projection: Projection::TriPlanar,
            ..Default::default()
        };
        for normal in [Vec3::X, Vec3::Y, Vec3::Z, -Vec3::X, -Vec3::Y, -Vec3::Z] {
            let contributions = mapping.project(&point(vec3(0.3, 0.4, 0.5), normal));
            let dominant = contributions
                .iter()
                .map(|c| c.weight)
                .fold(0.0f32, f32::max);
            assert!(
                dominant > 0.999,
                "an axis-aligned face blended {} of another plane in",
                1.0 - dominant
            );
        }
    }

    /// On a 45-degree edge the two planes must share evenly, and the weights
    /// must always sum to one — otherwise the surface brightens or darkens
    /// where projections meet.
    #[test]
    fn tri_planar_weights_always_sum_to_one() {
        let mapping = Mapping {
            projection: Projection::TriPlanar,
            ..Default::default()
        };
        let normals = [
            vec3(1.0, 1.0, 0.0),
            vec3(1.0, 1.0, 1.0),
            vec3(0.3, -0.9, 0.31),
            vec3(-0.7, 0.1, 0.7),
            Vec3::X,
        ];
        for normal in normals {
            let contributions = mapping.project(&point(vec3(0.3, 0.4, 0.5), normal));
            let total: f32 = contributions.iter().map(|c| c.weight).sum();
            assert!(
                (total - 1.0).abs() < 1e-4,
                "weights for {normal:?} summed to {total}, which would change the \
                 surface's brightness where planes meet"
            );
        }

        // The 45-degree case shares evenly between exactly two planes.
        let contributions = mapping.project(&point(vec3(0.3, 0.4, 0.5), vec3(1.0, 1.0, 0.0)));
        assert_eq!(contributions.len(), 2);
        for c in &contributions {
            assert!((c.weight - 0.5).abs() < 1e-4, "got {}", c.weight);
        }
    }

    /// A degenerate normal must not divide by zero.
    #[test]
    fn tri_planar_survives_a_degenerate_normal() {
        let mapping = Mapping {
            projection: Projection::TriPlanar,
            ..Default::default()
        };
        let mut p = point(vec3(1.0, 2.0, 3.0), Vec3::Y);
        p.normal = Vec3::ZERO;
        let contributions = mapping.project(&p);
        let total: f32 = contributions.iter().map(|c| c.weight).sum();
        assert!((total - 1.0).abs() < 1e-4, "degenerate normal gave {total}");
        assert!(contributions.iter().all(|c| c.coords.u.is_finite()));
    }

    /// Scaling the texture must scale the footprint with it, or a tiled
    /// texture aliases exactly where a tiled texture most obviously would.
    #[test]
    fn scaling_the_texture_scales_the_footprint() {
        let plain = Mapping {
            projection: Projection::Planar,
            ..Default::default()
        };
        let tiled = Mapping {
            projection: Projection::Planar,
            scale: vec3(16.0, 16.0, 1.0),
            ..Default::default()
        };
        let p = point(vec3(0.25, 0.5, 0.0), Vec3::Z);

        let a = only(&plain.project(&p));
        let b = only(&tiled.project(&p));
        assert!(
            (b.width() - a.width() * 16.0).abs() < 1e-5,
            "16 repeats gave footprint {} against {}",
            b.width(),
            a.width()
        );
        assert!((b.u - a.u * 16.0).abs() < 1e-5);
    }

    /// §5.3's real-world-units toggle: a size in scene units, not a repeat
    /// count. A 0.05-unit texture must tile twenty times per unit.
    #[test]
    fn real_world_units_are_a_size_not_a_repeat_count() {
        let mapping = Mapping {
            projection: Projection::Planar,
            real_world_units: true,
            scale: vec3(0.05, 0.05, 1.0),
            ..Default::default()
        };
        let at_one = only(&mapping.project(&point(vec3(1.0, 0.0, 0.0), Vec3::Z)));
        assert!(
            (at_one.u - 20.0).abs() < 1e-4,
            "a 0.05-unit texture reached u = {} at one unit across, not 20",
            at_one.u
        );

        // ...and the same texture on a part ten times bigger keeps its size.
        let at_ten = only(&mapping.project(&point(vec3(10.0, 0.0, 0.0), Vec3::Z)));
        assert!((at_ten.u - 200.0).abs() < 1e-3);
    }

    /// A real-world size is a size in the scene, however the part's mesh is
    /// scaled. A part modelled in centimetres under a 1/100 node has a point
    /// one metre across at position 100 in its own units; a 5 cm texture
    /// must reach u = 20 there, exactly as on a part modelled in metres.
    /// Read in the mesh's own units it reached 2000, a hundred times finer.
    #[test]
    fn real_world_units_measure_in_scene_units_on_a_scaled_part() {
        for projection in [Projection::Planar, Projection::TriPlanar] {
            let mapping = Mapping {
                projection,
                real_world_units: true,
                scale: vec3(0.05, 0.05, 1.0),
                ..Default::default()
            };
            let metres = mapping.project(&point(vec3(1.0, 0.3, 0.0), Vec3::Z));
            let centimetres =
                mapping.project(&SurfacePoint { scale: 0.01, ..point(vec3(100.0, 30.0, 0.0), Vec3::Z) });
            assert_eq!(metres.len(), centimetres.len());
            for (m, c) in metres.iter().zip(&centimetres) {
                assert!(
                    (m.coords.u - c.coords.u).abs() < 1e-3 && (m.coords.v - c.coords.v).abs() < 1e-3,
                    "{projection:?}: ({}, {}) on the part in metres, ({}, {}) on the part in centimetres",
                    m.coords.u, m.coords.v, c.coords.u, c.coords.v
                );
            }
            assert_eq!(mapping.raw_coords(&SurfacePoint { scale: 0.01, ..point(vec3(100.0, 30.0, 0.0), Vec3::Z) })
                .map(|(u, _)| (u * 100.0).round()), mapping.raw_coords(&point(vec3(1.0, 0.3, 0.0), Vec3::Z)).map(|(u, _)| (u * 100.0).round()));
        }
        // Repeat counts are not lengths, and stay in the mesh's own units.
        let repeats = Mapping { projection: Projection::Planar, scale: vec3(2.0, 2.0, 1.0), ..Default::default() };
        let scaled = only(&repeats.project(&SurfacePoint { scale: 0.01, ..point(vec3(100.0, 0.0, 0.0), Vec3::Z) }));
        assert!((scaled.u - 200.0).abs() < 1e-3, "a repeat-count mapping moved to {}", scaled.u);
    }

    #[test]
    fn rotation_offset_and_flips_apply() {
        let quarter = Mapping {
            projection: Projection::Planar,
            rotation: 0.25,
            ..Default::default()
        };
        // A quarter turn takes (1, 0) to (0, 1).
        let rotated = only(&quarter.project(&point(vec3(1.0, 0.0, 0.0), Vec3::Z)));
        assert!(rotated.u.abs() < 1e-5, "u = {}", rotated.u);
        assert!((rotated.v - 1.0).abs() < 1e-5, "v = {}", rotated.v);

        let shifted = Mapping {
            projection: Projection::Planar,
            offset: vec2(0.5, -0.25),
            ..Default::default()
        };
        let moved = only(&shifted.project(&point(vec3(0.1, 0.2, 0.0), Vec3::Z)));
        assert!((moved.u - 0.6).abs() < 1e-5);
        assert!((moved.v - (-0.05)).abs() < 1e-5);

        let flipped = Mapping {
            projection: Projection::Planar,
            flip_u: true,
            ..Default::default()
        };
        let mirrored = only(&flipped.project(&point(vec3(0.3, 0.4, 0.0), Vec3::Z)));
        assert!((mirrored.u + 0.3).abs() < 1e-5);
        assert!((mirrored.v - 0.4).abs() < 1e-5);
    }

    /// Nothing may produce a non-finite coordinate, whatever it is handed.
    #[test]
    fn no_projection_produces_a_non_finite_coordinate() {
        let awkward = [
            vec3(0.0, 0.0, 0.0),
            vec3(1e-9, 0.0, 0.0),
            vec3(1e9, -1e9, 1e9),
            vec3(0.0, 1.0, 0.0),
        ];
        for projection in [
            Projection::Uv,
            Projection::Planar,
            Projection::Cylindrical,
            Projection::Spherical,
            Projection::TriPlanar,
        ] {
            let mapping = Mapping {
                projection,
                ..Default::default()
            };
            for position in awkward {
                for normal in [Vec3::Y, vec3(1.0, 1.0, 1.0), Vec3::ZERO] {
                    let mut p = point(position, normal);
                    p.normal = if normal == Vec3::ZERO {
                        Vec3::ZERO
                    } else {
                        normal.normalize()
                    };
                    for c in mapping.project(&p) {
                        assert!(
                            c.coords.u.is_finite()
                                && c.coords.v.is_finite()
                                && c.coords.width().is_finite()
                                && c.weight.is_finite(),
                            "{projection:?} at {position:?} with normal {normal:?} \
                             gave {:?}",
                            c.coords
                        );
                    }
                }
            }
        }
    }
}
