//! A texture: pixels, a MIP pyramid, a projection, and a colour space
//! (`docs/02-MATERIAL-SYSTEM.md` §5.2).
//!
//! ## Colour space is decided by what the map is *for*
//!
//! §5.2: "the importer assigns colour spaces by **map role**, not by file
//! format, and defaults data maps to Raw." `docs/01` §7.2 calls mis-tagging a
//! data map as sRGB "a common and destructive error", and it is worth being
//! precise about why, because the two failures look nothing alike.
//!
//! A colour map holds an *appearance*. An 8-bit PNG of red paint stores values
//! that a monitor turns into the right red, which means they carry the sRGB
//! transfer curve and have to be linearised before a renderer can multiply them
//! by anything.
//!
//! A roughness map holds a *number*. 0.5 in the file means roughness 0.5. Run it
//! through the same sRGB curve and it becomes 0.21 — the surface turns glossy
//! everywhere the artist asked for satin, and no amount of adjusting the
//! roughness slider fixes it, because the map's midtones have been crushed
//! rather than scaled. It reads as "the roughness map isn't working", which
//! sends people looking in the wrong place.
//!
//! So the role decides, the file format does not, and the assignment is stored
//! on the texture so it survives a round trip through the file.
//!
//! ## Decode happens before the pyramid, not after
//!
//! This is the other half of the same idea and it is easy to get backwards.
//! Averaging four sRGB-encoded texels and *then* linearising is not the same as
//! linearising four texels and then averaging them — the curve is not linear, so
//! it does not commute with the mean. A black-and-white checkerboard averaged in
//! sRGB space and decoded afterwards comes out at 0.21 rather than 0.5: distant
//! surfaces darken as they recede up the pyramid. Decoding first is what
//! [`MipPyramid::build`]'s exact mean-preservation is worth anything for.

use crate::image::{ColorSpace, Image};
use crate::mip::{MipPyramid, Wrap};
use crate::projection::{Mapping, SurfacePoint};
use tequila_core::math::Vec3;
use tequila_core::spectrum::srgb_eotf;

/// What a texture is used for, which is what decides its colour space (§5.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapRole {
    /// An appearance: base colour, coat colour, emission colour. sRGB by
    /// default for 8-bit formats.
    Color,
    /// A number: roughness, metalness, opacity, weights, masks. Raw, always.
    Data,
    /// A tangent-space normal map. Raw — the vector is encoded in the values
    /// themselves and a transfer curve would bend it.
    Normal,
    /// A height field for bump or displacement. Raw.
    Height,
    /// A density grid for a Volume Map: an OpenVDB file, read by
    /// [`crate::volume`] rather than decoded as an image.
    Volume,
}

impl MapRole {
    /// The colour space this role implies for an 8-bit file.
    ///
    /// Floating-point formats (EXR, HDR) are already linear whatever the role,
    /// so this only applies where a transfer curve might be present.
    pub fn default_color_space(self) -> ColorSpace {
        match self {
            MapRole::Color => ColorSpace::Srgb,
            MapRole::Data | MapRole::Normal | MapRole::Height | MapRole::Volume => ColorSpace::Raw,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            MapRole::Color => "Color",
            MapRole::Data => "Data",
            MapRole::Normal => "Normal",
            MapRole::Height => "Height",
            MapRole::Volume => "Volume",
        }
    }
}

/// A loaded, filterable texture.
#[derive(Clone, Debug)]
pub struct Texture {
    decal_pyramid: std::sync::OnceLock<MipPyramid>,
    /// The alpha channel on its own, for a texture used as a cutout mask.
    ///
    /// A second pyramid rather than a fourth channel through the first: the
    /// filtering, wrapping, footprint and tri-planar blending are all in
    /// [`Texture::sample_pyramid`], and a scalar pyramid gets every one of
    /// them for free. `decal_pyramid` beside it is the same idea for the same
    /// reason, and both are built only if something asks.
    alpha_pyramid: std::sync::OnceLock<MipPyramid>,
    pyramid: MipPyramid,
    pub mapping: Mapping,
    /// The colour space the source file was interpreted in. Stored so it
    /// round-trips through the document (§5.2), and so the panel can show what
    /// was assumed rather than leaving the user to guess.
    pub source_color_space: ColorSpace,
    pub role: MapRole,
}

impl Texture {
    /// Build from an image whose values are already scene-linear.
    pub fn from_linear(image: Image, role: MapRole, mapping: Mapping) -> Texture {
        let source_color_space = image.color_space;
        Texture {
            decal_pyramid: std::sync::OnceLock::new(),
            alpha_pyramid: std::sync::OnceLock::new(),
            pyramid: MipPyramid::build(image),
            mapping,
            source_color_space,
            role,
        }
    }

    /// Build from 8-bit file bytes, decoding according to the role.
    ///
    /// The decode happens here, before the pyramid is built, for the reason in
    /// this module's header: the sRGB curve does not commute with averaging.
    pub fn from_bytes(
        width: usize,
        height: usize,
        channels: usize,
        bytes: &[u8],
        role: MapRole,
        mapping: Mapping,
    ) -> Texture {
        let space = role.default_color_space();
        let data: Vec<f32> = bytes
            .iter()
            .enumerate()
            .map(|(i, &b)| {
                let v = b as f32 / 255.0;
                // Alpha is never gamma-encoded, whatever the role.
                let is_alpha = channels == 4 && i % 4 == 3;
                if space == ColorSpace::Srgb && !is_alpha {
                    srgb_eotf(v)
                } else {
                    v
                }
            })
            .collect();

        let image = Image {
            width,
            height,
            channels,
            data,
            color_space: match space {
                ColorSpace::Srgb => ColorSpace::LinearSrgb,
                other => other,
            },
        };
        Texture {
            decal_pyramid: std::sync::OnceLock::new(),
            alpha_pyramid: std::sync::OnceLock::new(),
            pyramid: MipPyramid::build(image),
            mapping,
            source_color_space: space,
            role,
        }
    }

    /// A flat value, for a texture that has not finished loading (§5.1).
    ///
    /// Mid-grey for colour, 0.5 for data — §5.1's own choice, and the reason is
    /// that a placeholder of black or white reads as a *result* rather than as
    /// an absence, and people start debugging a material that is merely still
    /// loading.
    pub fn placeholder(role: MapRole, mapping: Mapping) -> Texture {
        let value = match role {
            MapRole::Color => 0.18,
            MapRole::Data | MapRole::Height => 0.5,
            MapRole::Volume => 1.0,
            // A flat tangent-space normal is (0, 0, 1), encoded.
            MapRole::Normal => 0.5,
        };
        let mut image = Image::constant(1, 1, Vec3::splat(value));
        if role == MapRole::Normal {
            image.data = vec![0.5, 0.5, 1.0];
        }
        image.color_space = ColorSpace::Raw;
        Texture {
            decal_pyramid: std::sync::OnceLock::new(),
            alpha_pyramid: std::sync::OnceLock::new(),
            pyramid: MipPyramid::build(image),
            mapping,
            source_color_space: ColorSpace::Raw,
            role,
        }
    }

    pub fn pyramid(&self) -> &MipPyramid {
        &self.pyramid
    }

    /// Build premultiplied MIPs only for textures used as decals. Alpha never
    /// goes through a color transfer function; invisible RGB cannot bleed.
    pub fn sample_decal(&self, mapping: &Mapping, point: &SurfacePoint) -> (Vec3, f32) {
        let pyramid = self.decal_pyramid.get_or_init(|| {
            let source = self.pyramid.base();
            let mut data = Vec::with_capacity(source.width * source.height * 4);
            for pixel in source.data.chunks_exact(source.channels) {
                let alpha = if source.channels == 4 {
                    pixel[3].clamp(0.0, 1.0)
                } else {
                    1.0
                };
                data.extend_from_slice(&[
                    pixel[0] * alpha,
                    pixel[1] * alpha,
                    pixel[2] * alpha,
                    alpha,
                ]);
            }
            MipPyramid::build(Image {
                width: source.width,
                height: source.height,
                channels: 4,
                data,
                color_space: source.color_space,
            })
        });
        let mut color = Vec3::ZERO;
        let mut alpha = 0.0;
        for contribution in mapping.project(point) {
            let uv = contribution.coords;
            let rgba = pyramid.sample_rgba_border(uv.u, uv.v, uv.width());
            color += tequila_core::math::vec3(rgba[0], rgba[1], rgba[2]) * contribution.weight;
            alpha += rgba[3] * contribution.weight;
        }
        (color, alpha.clamp(0.0, 1.0))
    }

    /// Look the texture up at a shading point, projecting and filtering.
    ///
    /// Tri-planar returns up to three contributions and they are blended
    /// *after* sampling: averaging the three coordinate sets and sampling once
    /// would fetch from a position none of the three projections asked for,
    /// which on a box corner is a completely unrelated part of the texture.
    pub fn sample(&self, point: &SurfacePoint) -> Vec3 {
        self.sample_with(&self.mapping, point)
    }

    /// Look the texture up through a mapping supplied by the caller.
    ///
    /// A material graph has its own Mapping 2D node, and that node's projection
    /// is the one that must apply — projecting again with the texture's own
    /// would discard the coordinates the graph just computed and read from
    /// somewhere else entirely. The texture's `mapping` field is the default
    /// for when nothing else supplies one, not an extra transform to compose.
    pub fn sample_with(&self, mapping: &Mapping, point: &SurfacePoint) -> Vec3 {
        self.sample_pyramid(&self.pyramid, mapping, point)
    }

    /// The alpha channel, filtered exactly as the colour is.
    ///
    /// This is what makes an alpha-tested surface possible: glTF's
    /// `alphaMode`, and any PNG whose transparency is the mask, keep the
    /// cutout in the colour image's fourth channel rather than in a file of
    /// its own. Sampled through the same projection and footprint as the
    /// colour, so a leaf's holes land exactly where its picture says.
    ///
    /// 1 where the image has no alpha at all, which is a fully present
    /// surface — the answer that changes nothing.
    pub fn sample_alpha(&self, mapping: &Mapping, point: &SurfacePoint) -> f32 {
        if self.pyramid.base().channels < 4 {
            return 1.0;
        }
        let pyramid = self.alpha_pyramid.get_or_init(|| {
            let source = self.pyramid.base();
            // Splatted across three channels rather than kept as one:
            // `Image::texel` reads three floats per texel whatever `channels`
            // says, so a single-channel image would read into its neighbour
            // and index past the end at the last one.
            let mut data = Vec::with_capacity(source.width * source.height * 3);
            for pixel in source.data.chunks_exact(source.channels) {
                data.extend_from_slice(&[pixel[3], pixel[3], pixel[3]]);
            }
            MipPyramid::build(Image {
                width: source.width,
                height: source.height,
                channels: 3,
                data,
                // Alpha is coverage, not colour, and was never given a
                // transfer curve to remove — `Image::from_srgb8` skips the
                // fourth channel for exactly this reason.
                color_space: ColorSpace::Raw,
            })
        });
        self.sample_pyramid(pyramid, mapping, point)
            .x
            .clamp(0.0, 1.0)
    }

    fn sample_pyramid(
        &self,
        pyramid: &MipPyramid,
        mapping: &Mapping,
        point: &SurfacePoint,
    ) -> Vec3 {
        let (wrap_u, wrap_v) = mapping.wrap();
        let contributions = mapping.project(point);

        let mut total = Vec3::ZERO;
        let mut weight = 0.0;
        for contribution in &contributions {
            let coords = contribution.coords;
            // A clamped axis leaves the surface untouched outside its extent
            // rather than smearing the edge texel across it — what a decal
            // needs.
            if !wrap_u.is_inside(coords.u) || !wrap_v.is_inside(coords.v) {
                continue;
            }
            let value = sample_axes(pyramid, coords.u, coords.v, coords.width(), wrap_u, wrap_v);
            total = total + value * contribution.weight;
            weight += contribution.weight;
        }

        if weight <= 1e-6 {
            return Vec3::ZERO;
        }
        // Renormalize: contributions falling outside a clamped extent were
        // skipped, and the rest must still average rather than darken.
        total / weight
    }

    /// A normal map read through a projection of its own, rather than
    /// through the mesh's UVs: the surface normal it gives, in the point's
    /// space, or `None` where nothing of the map lands.
    ///
    /// A normal map's texel is a direction in the frame the image was laid
    /// down in — across it and up it. Under a projection that frame is the
    /// projection's, not the mesh's: a tri-planar lookup lays the image down
    /// three times, along three axes. So each contribution is decoded in its
    /// own frame, and the three results blended as directions. Blending the
    /// texels first, as a colour lookup does, mixes three vectors that point
    /// along three different sets of axes; decoded in the mesh's UV frame
    /// after that, the surface's relief points nowhere in particular.
    ///
    /// Each frame is read from the projection itself — where the coordinates
    /// go as the point moves a little along each axis — so rotation, flips,
    /// scale and the mirrored far faces all come out right without being
    /// worked out again here.
    pub fn sample_normal_with(&self, mapping: &Mapping, point: &SurfacePoint) -> Option<Vec3> {
        let (wrap_u, wrap_v) = mapping.wrap();
        let contributions = mapping.project(point);
        let n = point.normal;
        let step = point.footprint.max((point.position.length() + 1.0) * 1e-4);
        let axes = [Vec3::X, Vec3::Y, Vec3::Z];
        let moved: Vec<Vec<crate::projection::Contribution>> = axes
            .iter()
            .map(|axis| mapping.project(&SurfacePoint { position: point.position + *axis * step, ..*point }))
            .collect();

        let mut total = Vec3::ZERO;
        let mut any = false;
        for (i, contribution) in contributions.iter().enumerate() {
            let coords = contribution.coords;
            if !wrap_u.is_inside(coords.u) || !wrap_v.is_inside(coords.v) {
                continue;
            }
            // The same normal, so the same contributions in the same order.
            let Some(shifted) = moved.iter().map(|m| m.get(i).map(|c| c.coords)).collect::<Option<Vec<_>>>() else {
                continue;
            };
            let gradient = |along: fn(&crate::projection::TextureCoords) -> f32| Vec3 {
                x: (along(&shifted[0]) - along(&coords)) / step,
                y: (along(&shifted[1]) - along(&coords)) / step,
                z: (along(&shifted[2]) - along(&coords)) / step,
            };
            let (across, down) = (gradient(|c| c.u), gradient(|c| c.v));
            let (Some(tangent), Some(down)) = (
                (across - n * n.dot(across)).try_normalize(),
                (down - n * n.dot(down)).try_normalize(),
            ) else {
                continue;
            };
            let texel = sample_axes(&self.pyramid, coords.u, coords.v, coords.width(), wrap_u, wrap_v);
            // Green is up the image, and v runs down it.
            let frame = crate::TangentFrame { normal: n, tangent, bitangent: -down };
            total = total + frame.decode_normal_map(texel, 1.0) * contribution.weight;
            any = true;
        }
        if any {
            total.try_normalize()
        } else {
            None
        }
    }

    /// The scalar a data map carries, taking the red channel.
    pub fn sample_scalar(&self, point: &SurfacePoint) -> f32 {
        self.sample(point).x
    }

}

fn sample_axes(
    pyramid: &MipPyramid,
    u: f32,
    v: f32,
    width: f32,
    wrap_u: Wrap,
    wrap_v: Wrap,
) -> Vec3 {
    // The pyramid takes one wrap mode; where the two axes differ, the
    // per-axis wrapping is applied first and the lookup told to clamp,
    // which is a no-op on an already-wrapped coordinate.
    if wrap_u == wrap_v {
        pyramid.sample(u, v, width, wrap_u)
    } else {
        pyramid.sample(wrap_u.apply(u), wrap_v.apply(v), width, Wrap::Clamp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::Projection;
    use tequila_core::math::{vec2, vec3};

    fn point(position: Vec3, normal: Vec3, uv: (f32, f32)) -> SurfacePoint {
        SurfacePoint {
            position,
            normal: normal.normalize(),
            uv: vec2(uv.0, uv.1),
            footprint: 0.0,
            scale: 1.0,
            uv_footprint: None,
            curvature: 0.0,
            occlusion: 0.0,
            vertex_color: tequila_core::spectrum::Spectrum::ONE,
            world: Default::default(),
        }
    }

    /// A texture's alpha comes out where its picture says, and filters.
    ///
    /// The channel an alpha-tested surface lives in. Two things have gone
    /// wrong here before they could ship: reading it from a single-channel
    /// pyramid, which `Image::texel` cannot address, and losing it to the sRGB
    /// curve, which alpha never carried.
    #[test]
    fn alpha_is_sampled_where_the_image_says_and_is_not_gamma_decoded() {
        // Left half opaque, right half a hole, in a colour that would be
        // obvious if the alpha were taken from it by mistake.
        let (w, h) = (4usize, 2usize);
        let mut bytes = Vec::with_capacity(w * h * 4);
        for _ in 0..h {
            for x in 0..w {
                let alpha = if x < w / 2 { 255 } else { 0 };
                bytes.extend_from_slice(&[10, 200, 60, alpha]);
            }
        }
        let texture = Texture::from_bytes(w, h, 4, &bytes, MapRole::Color, Mapping::default());

        let at = |u: f32| {
            texture.sample_alpha(&Mapping::default(), &point(Vec3::ZERO, Vec3::Y, (u, 0.5)))
        };
        assert!((at(0.15) - 1.0).abs() < 1e-3, "opaque side read {}", at(0.15));
        assert!((at(0.85) - 0.0).abs() < 1e-3, "clear side read {}", at(0.85));

        // Half coverage has to stay half. An sRGB decode would drag 0.5 down
        // to about 0.21, which is the mistake that makes every soft edge
        // shrink.
        let mut half = Vec::with_capacity(w * h * 4);
        for _ in 0..(w * h) {
            half.extend_from_slice(&[10, 200, 60, 128]);
        }
        let texture = Texture::from_bytes(w, h, 4, &half, MapRole::Color, Mapping::default());
        let mid = texture.sample_alpha(&Mapping::default(), &point(Vec3::ZERO, Vec3::Y, (0.5, 0.5)));
        assert!(
            (mid - 128.0 / 255.0).abs() < 1e-3,
            "half coverage read {mid}, which is what an sRGB decode would do to it"
        );
    }

    /// An image with no alpha at all is fully present, not fully absent.
    #[test]
    fn a_texture_without_alpha_samples_as_opaque() {
        let bytes = vec![200u8; 4 * 4 * 3];
        let texture = Texture::from_bytes(4, 4, 3, &bytes, MapRole::Color, Mapping::default());
        let a = texture.sample_alpha(&Mapping::default(), &point(Vec3::ZERO, Vec3::Y, (0.5, 0.5)));
        assert_eq!(a, 1.0, "a texture with no alpha channel must read as solid");
    }

    /// An 8-bit checkerboard of pure black and pure white, as bytes.
    fn checker_bytes(size: usize) -> Vec<u8> {
        let mut bytes = vec![0u8; size * size * 3];
        for y in 0..size {
            for x in 0..size {
                let on = (x + y) % 2 == 0;
                for c in 0..3 {
                    bytes[(y * size + x) * 3 + c] = if on { 255 } else { 0 };
                }
            }
        }
        bytes
    }

    /// §5.2, the destructive error: a roughness map must not be run through the
    /// sRGB curve.
    ///
    /// The number in the file *is* the roughness. Decoding it as a colour turns
    /// a requested 0.5 into 0.21, and the surface comes out glossy everywhere
    /// the artist asked for satin.
    #[test]
    fn a_data_map_is_not_decoded_as_a_colour() {
        let midtone = vec![128u8; 3];
        let mapping = Mapping::default();
        let data = Texture::from_bytes(1, 1, 3, &midtone, MapRole::Data, mapping);
        let colour = Texture::from_bytes(1, 1, 3, &midtone, MapRole::Color, mapping);

        let p = point(Vec3::ZERO, Vec3::Z, (0.5, 0.5));
        let as_data = data.sample_scalar(&p);
        let as_colour = colour.sample_scalar(&p);

        assert!(
            (as_data - 128.0 / 255.0).abs() < 1e-4,
            "a data map read 0.502 in the file and returned {as_data}"
        );
        assert!(
            (as_colour - srgb_eotf(128.0 / 255.0)).abs() < 1e-4,
            "a colour map should be linearised, got {as_colour}"
        );
        assert!(
            as_colour < as_data * 0.6,
            "the two decodes should differ sharply: {as_colour} against {as_data}"
        );
        assert_eq!(data.source_color_space, ColorSpace::Raw);
        assert_eq!(colour.source_color_space, ColorSpace::Srgb);
    }

    #[test]
    fn every_non_colour_role_defaults_to_raw() {
        assert_eq!(MapRole::Color.default_color_space(), ColorSpace::Srgb);
        for role in [MapRole::Data, MapRole::Normal, MapRole::Height] {
            assert_eq!(
                role.default_color_space(),
                ColorSpace::Raw,
                "{} must not carry a transfer curve",
                role.label()
            );
        }
    }

    /// Decoding must happen before the pyramid is built, because the sRGB
    /// curve does not commute with averaging.
    ///
    /// A black-and-white checkerboard averages to 0.5 in linear light. Average
    /// the encoded bytes first and decode afterwards and it comes out at 0.21 —
    /// so every surface darkens as it recedes into the distance, which reads as
    /// a lighting bug rather than a texturing one.
    #[test]
    fn mip_levels_are_averaged_in_linear_light() {
        let bytes = checker_bytes(64);
        let texture = Texture::from_bytes(64, 64, 3, &bytes, MapRole::Color, Mapping::default());

        let top = texture.pyramid().level(texture.pyramid().levels() - 1);
        let averaged = top.texel(0, 0).x;

        assert!(
            (averaged - 0.5).abs() < 1e-3,
            "a black-and-white checkerboard averaged to {averaged} rather than 0.5"
        );
        // The wrong answer, for contrast: averaging encoded values first.
        let wrong = srgb_eotf(0.5);
        assert!(
            (averaged - wrong).abs() > 0.25,
            "the result is indistinguishable from averaging in gamma space"
        );
    }

    /// Tri-planar blending happens after sampling, not before. On a corner the
    /// three projections read completely different parts of the texture, and
    /// averaging their coordinates would fetch from somewhere none of them
    /// asked for.
    #[test]
    fn tri_planar_blends_samples_rather_than_coordinates() {
        // A texture that varies sharply across u, so the difference shows.
        let size = 64;
        let mut data = vec![0.0f32; size * size * 3];
        for y in 0..size {
            for x in 0..size {
                for c in 0..3 {
                    data[(y * size + x) * 3 + c] = x as f32 / (size - 1) as f32;
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
        let mapping = Mapping {
            projection: Projection::TriPlanar,
            ..Default::default()
        };
        let texture = Texture::from_linear(image, MapRole::Data, mapping);

        // A 45-degree corner: two planes share evenly.
        let p = point(vec3(0.2, 0.7, 0.4), vec3(1.0, 1.0, 0.0), (0.0, 0.0));
        let blended = texture.sample(&p);

        // Sample each plane on its own and average by hand.
        let contributions = mapping.project(&p);
        assert_eq!(contributions.len(), 2);
        let expected: f32 = contributions
            .iter()
            .map(|c| {
                texture
                    .pyramid()
                    .sample(c.coords.u, c.coords.v, c.coords.width(), Wrap::Repeat)
                    .x
                    * c.weight
            })
            .sum();

        assert!(
            (blended.x - expected).abs() < 1e-5,
            "blended to {} rather than the weighted mean of the planes, {expected}",
            blended.x
        );
    }

    /// A clamped texture leaves the surface alone outside its own extent. This
    /// is what a decal needs: a logo on a panel must not smear its border
    /// colour across the rest of the part.
    #[test]
    fn a_clamped_texture_reports_nothing_outside_its_extent() {
        let mapping = Mapping {
            projection: Projection::Uv,
            wrap_u: Wrap::Clamp,
            wrap_v: Wrap::Clamp,
            ..Default::default()
        };
        let texture = Texture::from_linear(
            Image::constant(4, 4, vec3(1.0, 0.0, 0.0)),
            MapRole::Color,
            mapping,
        );

        let inside = texture.sample(&point(Vec3::ZERO, Vec3::Z, (0.5, 0.5)));
        assert!((inside.x - 1.0).abs() < 1e-5, "inside read {inside:?}");

        for outside in [(1.4, 0.5), (-0.2, 0.5), (0.5, 2.0)] {
            let value = texture.sample(&point(Vec3::ZERO, Vec3::Z, outside));
            assert_eq!(
                value,
                Vec3::ZERO,
                "at {outside:?} the decal painted {value:?} outside its own bounds"
            );
        }
    }

    /// A placeholder must be neutral and must not look like a result. §5.1
    /// picks mid-grey for colour and 0.5 for data.
    #[test]
    fn placeholders_are_neutral() {
        let mapping = Mapping::default();
        let p = point(Vec3::ZERO, Vec3::Z, (0.37, 0.62));

        let colour = Texture::placeholder(MapRole::Color, mapping);
        assert!((colour.sample(&p).x - 0.18).abs() < 1e-5);

        let data = Texture::placeholder(MapRole::Data, mapping);
        assert!((data.sample_scalar(&p) - 0.5).abs() < 1e-5);

        // A flat tangent-space normal points straight out: (0, 0, 1) encoded
        // as (0.5, 0.5, 1).
        let normal = Texture::placeholder(MapRole::Normal, mapping);
        let n = normal.sample(&p);
        assert!((n.x - 0.5).abs() < 1e-5 && (n.y - 0.5).abs() < 1e-5 && (n.z - 1.0).abs() < 1e-5);
    }

    /// Alpha is never gamma-encoded, even in a colour map.
    #[test]
    fn alpha_is_left_alone_in_a_colour_map() {
        let bytes = vec![128u8, 128, 128, 128];
        let texture = Texture::from_bytes(1, 1, 4, &bytes, MapRole::Color, Mapping::default());
        let alpha = texture.pyramid().base().data[3];
        assert!(
            (alpha - 128.0 / 255.0).abs() < 1e-4,
            "alpha was decoded as a colour: {alpha}"
        );
    }
}
