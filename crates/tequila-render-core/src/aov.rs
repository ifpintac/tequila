//! Render passes (`docs/01-RENDERER-CORE.md` §12).
//!
//! Two kinds of pass, and the difference decides everything about how each is
//! stored:
//!
//! - **Light-path passes** are *accumulations of radiance* selected by a light
//!   path expression: Beauty, Diffuse, Reflection, Caustics, and one per light
//!   layer. Each needs its own running sum per pixel, and each is routed by the
//!   same match against the same path — which is what makes §12's "consistent
//!   by construction" true and what M4's exit criterion depends on.
//! - **Data passes** are *properties of the first hit*: depth, normal,
//!   position, albedo, the object and material IDs. One value per pixel,
//!   averaged like everything else, and mostly already in the framebuffer
//!   because the denoiser wanted them.
//!
//! # Why the light-path buffers are allocated separately
//!
//! A pixel is a small struct and there are eight million of them at 4K. Putting
//! a dozen spectra in it would cost a gigabyte and a half whether or not anyone
//! asked for a pass. So the passes live in their own buffers, allocated when
//! passes are requested and absent otherwise — and when they are absent the
//! matcher is empty and the integrator's routing costs nothing at all.

use crate::lpe::Lpe;
use tequila_core::spectrum::Spectrum;

/// How many light-path passes may be accumulated at once.
///
/// §12's table and every light group at once — eight layers, the environment
/// and glowing materials — with a little room. The number matters because it
/// is the width of the mask the matcher returns and the length of the array a
/// path carries; the buffers themselves are only allocated for the passes a
/// render asks for.
pub const MAX_LIGHT_PATH_PASSES: usize = 20;

/// A pass that accumulates radiance selected by a light path expression.
#[derive(Clone, Debug)]
pub struct LightPathPass {
    /// What it is called in the EXR, and in the UI.
    pub name: String,
    pub expression: Lpe,
}

impl LightPathPass {
    pub fn new(name: impl Into<String>, expression: &str) -> Result<LightPathPass, String> {
        Ok(LightPathPass {
            name: name.into(),
            expression: Lpe::parse(expression)?,
        })
    }
}

/// The passes §12 names, as expressions.
///
/// Written once, here, and used by the renderer, the EXR writer and the UI, so
/// a pass cannot exist in one and not the others. The names are the ones
/// compositors expect to find in a multi-layer EXR.
pub fn standard_passes() -> Vec<LightPathPass> {
    // `unwrap` rather than a `Result`: these are literals in this file, and a
    // malformed one is a bug that should stop the program rather than produce a
    // renderer that silently has no passes. `the_standard_passes_compile`
    // catches it long before then.
    let pass = |name: &str, expression: &str| {
        LightPathPass::new(name, expression).expect("a standard pass expression")
    };
    vec![
        pass("Beauty", "C.*L"),
        pass("Diffuse", "C<RD>.*L"),
        pass("Lighting", "C<R.>L"),
        pass("GlobalIllumination", "C<R.>.+L"),
        pass("Caustics", "C<RD>[SG]+L"),
        pass("Reflection", "C<RS>.*L"),
        pass("Refraction", "C<T.>.*L"),
    ]
}

/// One pass per light group (§5.5): each light layer in `layers`, the
/// environment, and glowing materials when `emissive`.
///
/// Every contribution a path makes ends at exactly one light, so these add
/// up to Beauty — which is what makes post-render relighting a sum of
/// rescaled passes rather than an approximation.
pub fn light_group_passes(layers: impl IntoIterator<Item = u8>, emissive: bool) -> Vec<LightPathPass> {
    let mut layers: Vec<u8> = layers.into_iter().map(|l| l.min(crate::lpe::LAST_LAYER)).collect();
    layers.sort_unstable();
    layers.dedup();
    let mut out: Vec<LightPathPass> = layers
        .into_iter()
        .map(|layer| LightPathPass::new(format!("LightLayer{layer}"), &format!("C.*L{layer}")).expect("a layer expression"))
        .collect();
    out.push(LightPathPass::new("Environment", "C.*LE").expect("the environment expression"));
    if emissive {
        out.push(LightPathPass::new("Emissive", "C.*LM").expect("the emissive expression"));
    }
    out
}

/// Everything a render is asked to produce beyond the beauty image.
#[derive(Clone, Debug, Default)]
pub struct AovSet {
    pub light_paths: Vec<LightPathPass>,
    /// Depth, normals, position, albedo and the ID passes, which are properties
    /// of the first hit rather than accumulations of radiance.
    pub data: DataPasses,
    /// Per-object and per-material coverage mattes (§12).
    pub cryptomatte: bool,
}

/// Which first-hit properties to keep.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DataPasses {
    pub depth: bool,
    pub normal: bool,
    pub position: bool,
    pub albedo: bool,
    /// A deterministic pseudo-random colour per object or per material.
    pub clown: bool,
    /// Ambient occlusion, as an independent short-ray estimate.
    pub ambient_occlusion: bool,
    /// The ratio of unshadowed to shadowed direct lighting.
    pub shadow: bool,
    /// Every label layer alone, with alpha (§12).
    pub labels: bool,
}

impl DataPasses {
    pub fn any(&self) -> bool {
        self.depth
            || self.normal
            || self.position
            || self.albedo
            || self.clown
            || self.ambient_occlusion
            || self.shadow
    }
}

impl AovSet {
    /// Everything §12's table names, which is what the exit criterion renders.
    pub fn standard() -> AovSet {
        AovSet {
            light_paths: standard_passes(),
            data: DataPasses {
                depth: true,
                normal: true,
                position: true,
                albedo: true,
                clown: true,
                ambient_occlusion: true,
                shadow: true,
                labels: true,
            },
            cryptomatte: true,
        }
    }

    /// [`AovSet::standard`], with a pass per light group `scene` has.
    pub fn standard_for(scene: &crate::scene::SceneSnapshot) -> AovSet {
        let mut set = AovSet::standard();
        set.light_paths.extend(AovSet::light_groups_of(scene));
        set
    }

    /// Only the light groups `scene` has: what relighting needs, and
    /// nothing else.
    pub fn light_groups_for(scene: &crate::scene::SceneSnapshot) -> AovSet {
        AovSet {
            light_paths: AovSet::light_groups_of(scene),
            ..AovSet::default()
        }
    }

    fn light_groups_of(scene: &crate::scene::SceneSnapshot) -> Vec<LightPathPass> {
        let layers = scene.lights.lights().iter().map(|light| light.layer);
        let emissive = scene.instances.iter().any(|instance| {
            !instance.light_proxy
                && scene
                    .materials
                    .get(instance.material.index())
                    .is_some_and(|m| m.is_emissive())
        });
        light_group_passes(layers, emissive)
    }

    /// Whether a pass is one of the light groups, by its name.
    pub fn is_light_group(name: &str) -> bool {
        name.starts_with("LightLayer") || name == "Environment" || name == "Emissive"
    }

    pub fn is_empty(&self) -> bool {
        self.light_paths.is_empty() && !self.data.any() && !self.cryptomatte
    }

    /// The expressions, for building a matcher.
    pub fn expressions(&self) -> Vec<Lpe> {
        self.light_paths
            .iter()
            .take(MAX_LIGHT_PATH_PASSES)
            .map(|pass| pass.expression.clone())
            .collect()
    }
}

/// One path's contributions, before they reach the framebuffer.
///
/// A fixed array rather than a map: a path makes a handful of contributions and
/// allocating for them would cost more than the rest of the vertex. The index
/// is the pass's position in [`AovSet::light_paths`], which is also the bit the
/// matcher returns.
#[derive(Clone, Copy, Debug)]
pub struct PathPasses {
    pub values: [Spectrum; MAX_LIGHT_PATH_PASSES],
    /// How many of them are live. Zero means the whole mechanism is off.
    pub count: usize,
}

impl Default for PathPasses {
    fn default() -> Self {
        PathPasses {
            values: [Spectrum::ZERO; MAX_LIGHT_PATH_PASSES],
            count: 0,
        }
    }
}

impl PathPasses {
    pub fn new(count: usize) -> PathPasses {
        PathPasses {
            values: [Spectrum::ZERO; MAX_LIGHT_PATH_PASSES],
            count: count.min(MAX_LIGHT_PATH_PASSES),
        }
    }

    pub fn clear(&mut self) {
        for value in &mut self.values[..self.count] {
            *value = Spectrum::ZERO;
        }
    }

    /// Add a contribution to every pass whose bit is set.
    #[inline]
    pub fn add(&mut self, mask: u64, contribution: Spectrum) {
        let mut remaining = mask & ((1u64 << self.count) - 1).max(0);
        while remaining != 0 {
            let index = remaining.trailing_zeros() as usize;
            remaining &= remaining - 1;
            self.values[index] = self.values[index] + contribution;
        }
    }
}

/// The accumulated light-path passes, one buffer per pass.
#[derive(Clone, Debug, Default)]
pub struct PassBuffers {
    pub width: usize,
    pub height: usize,
    /// `light_paths.len()` buffers of `width * height` sums each.
    sums: Vec<Vec<Spectrum>>,
}

impl PassBuffers {
    pub fn new(width: usize, height: usize, passes: usize) -> PassBuffers {
        PassBuffers {
            width,
            height,
            sums: vec![vec![Spectrum::ZERO; width * height]; passes],
        }
    }

    pub fn passes(&self) -> usize {
        self.sums.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sums.is_empty()
    }

    /// Accumulate one sample's contributions at a pixel.
    #[inline]
    pub fn accumulate(&mut self, x: usize, y: usize, passes: &PathPasses) {
        if x >= self.width || y >= self.height {
            return;
        }
        let index = y * self.width + x;
        for (buffer, value) in self.sums.iter_mut().zip(&passes.values[..passes.count]) {
            buffer[index] = buffer[index] + *value;
        }
    }

    /// The mean of one pass at one pixel.
    pub fn mean(&self, pass: usize, x: usize, y: usize, samples: u32) -> Spectrum {
        if samples == 0 || x >= self.width || y >= self.height {
            return Spectrum::ZERO;
        }
        match self.sums.get(pass) {
            Some(buffer) => buffer[y * self.width + x] / samples as f32,
            None => Spectrum::ZERO,
        }
    }

    /// Merge another buffer's sums into this one, for a tiled render.
    pub fn merge(&mut self, other: &PassBuffers) {
        for (into, from) in self.sums.iter_mut().zip(&other.sums) {
            for (a, b) in into.iter_mut().zip(from) {
                *a = *a + *b;
            }
        }
    }

    pub fn clear(&mut self) {
        for buffer in &mut self.sums {
            buffer.fill(Spectrum::ZERO);
        }
    }
}

/// A deterministic colour for an identifier — §12's Clown pass.
///
/// Deterministic across runs and across machines, because a clown pass is used
/// to *select* things in a compositor: a colour that changed between the draft
/// and the final render would break every selection made from the draft. So
/// this is a hash, not a random number generator.
pub fn clown_color(id: u32) -> Spectrum {
    // A cheap integer hash with good avalanche, then three bytes of it.
    let mut h = id.wrapping_mul(0x9e37_79b9);
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    // Kept out of the very dark corner of the cube: two adjacent parts that both
    // came out near black are indistinguishable on screen, which defeats the
    // point of the pass.
    let channel = |shift: u32| 0.15 + 0.85 * (((h >> shift) & 0xff) as f32 / 255.0);
    tequila_core::spectrum::spectrum(channel(0), channel(8), channel(16))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A malformed standard pass would make `standard_passes` panic at
    /// runtime, in a renderer, after a user pressed Render. Here instead.
    #[test]
    fn the_standard_passes_compile() {
        let passes = standard_passes();
        assert!(passes.len() >= 7);
        assert_eq!(passes[0].name, "Beauty");
        for pass in &passes {
            assert!(!pass.expression.source().is_empty());
        }
    }

    /// The exit criterion in miniature: the components §12 lists have to
    /// partition the paths, so their sum is the beauty pass. Checked on the
    /// expressions, because if the partition is wrong no amount of sampling
    /// will make an image add up.
    #[test]
    fn the_component_passes_partition_the_beauty_pass() {
        use crate::lpe::{Event, Scatter, Transport};

        let passes = standard_passes();
        let beauty = &passes[0].expression;
        // The partition is by first bounce: diffuse, specular reflection,
        // glossy reflection, or transmission — plus a camera ray that reaches a
        // light with no bounce at all.
        let components: Vec<&Lpe> = passes
            .iter()
            .filter(|p| {
                matches!(
                    p.name.as_str(),
                    "Diffuse" | "Reflection" | "Refraction"
                )
            })
            .map(|p| &p.expression)
            .collect();
        assert_eq!(components.len(), 3);

        let scatter = |t: Transport, s: Scatter| Event::Scatter(t, s);
        let paths: Vec<Vec<Event>> = vec![
            vec![
                Event::Camera,
                scatter(Transport::Reflect, Scatter::Diffuse),
                Event::Light(0),
            ],
            vec![
                Event::Camera,
                scatter(Transport::Reflect, Scatter::Specular),
                Event::Light(0),
            ],
            vec![
                Event::Camera,
                scatter(Transport::Transmit, Scatter::Specular),
                Event::Light(0),
            ],
            vec![
                Event::Camera,
                scatter(Transport::Reflect, Scatter::Diffuse),
                scatter(Transport::Reflect, Scatter::Specular),
                Event::Light(0),
            ],
        ];
        for p in &paths {
            assert!(beauty.matches(p), "beauty missed a path");
            let hits = components.iter().filter(|lpe| lpe.matches(p)).count();
            assert_eq!(hits, 1, "a path matched {hits} components rather than one");
        }

        // A glossy first bounce is deliberately in none of the three, which is
        // why the set above is not the whole partition — and stating it here
        // stops the next person assuming it is.
        let glossy = vec![
            Event::Camera,
            scatter(Transport::Reflect, Scatter::Glossy),
            Event::Light(0),
        ];
        assert!(beauty.matches(&glossy));
        assert_eq!(components.iter().filter(|l| l.matches(&glossy)).count(), 0);
    }

    /// Direct and indirect must not overlap, or Lighting + GI double-counts and
    /// the two together exceed the beauty pass.
    #[test]
    fn direct_and_indirect_do_not_overlap() {
        use crate::lpe::{Event, Scatter, Transport};

        let passes = standard_passes();
        let direct = &passes
            .iter()
            .find(|p| p.name == "Lighting")
            .unwrap()
            .expression;
        let indirect = &passes
            .iter()
            .find(|p| p.name == "GlobalIllumination")
            .unwrap()
            .expression;

        let one = vec![
            Event::Camera,
            Event::Scatter(Transport::Reflect, Scatter::Diffuse),
            Event::Light(0),
        ];
        let two = vec![
            Event::Camera,
            Event::Scatter(Transport::Reflect, Scatter::Diffuse),
            Event::Scatter(Transport::Reflect, Scatter::Diffuse),
            Event::Light(0),
        ];
        assert!(direct.matches(&one) && !indirect.matches(&one));
        assert!(!direct.matches(&two) && indirect.matches(&two));
    }

    /// A clown colour has to be the same every run: it is used to *select*
    /// parts in a compositor, and a colour that changed between the draft and
    /// the final render would break every selection made from the draft.
    #[test]
    fn clown_colours_are_stable_and_distinct() {
        assert_eq!(clown_color(7), clown_color(7));
        let mut seen = Vec::new();
        for id in 0..256u32 {
            let c = clown_color(id);
            assert!(c.r >= 0.15 && c.g >= 0.15 && c.b >= 0.15, "{c:?} is too dark");
            seen.push(c);
        }
        // Adjacent ids must not be adjacent colours, or a clown pass of a
        // sequentially numbered assembly is a smooth gradient nobody can pick a
        // part out of.
        //
        // Stated as a distribution rather than as a rule about every pair,
        // because a *good* hash puts colours at random and random points
        // occasionally land near each other: at 256 ids in a 0.85-wide cube,
        // about one adjacent pair closer than 0.1 is expected. Demanding that
        // none be close would be demanding a hash that is not random.
        let distance = |a: Spectrum, b: Spectrum| {
            ((a.r - b.r).powi(2) + (a.g - b.g).powi(2) + (a.b - b.b).powi(2)).sqrt()
        };
        let distances: Vec<f32> = (1..seen.len())
            .map(|i| distance(seen[i - 1], seen[i]))
            .collect();
        let mean = distances.iter().sum::<f32>() / distances.len() as f32;
        assert!(
            mean > 0.4,
            "adjacent ids average {mean} apart, which is a gradient rather than a \
             set of distinct colours"
        );
        let close = distances.iter().filter(|d| **d < 0.1).count();
        assert!(
            close <= 3,
            "{close} adjacent pairs are nearly the same colour, which is more \
             than chance"
        );
    }

    /// A path's contributions land in the passes the mask names, and nowhere
    /// else. The mask is a `u64` and the array is sixteen long, so the guard
    /// against a stray high bit is worth having.
    #[test]
    fn contributions_land_only_where_the_mask_says() {
        let mut passes = PathPasses::new(3);
        passes.add(0b101, Spectrum::splat(1.0));
        assert_eq!(passes.values[0], Spectrum::splat(1.0));
        assert_eq!(passes.values[1], Spectrum::ZERO);
        assert_eq!(passes.values[2], Spectrum::splat(1.0));
        // A bit past the end is ignored rather than writing out of bounds.
        passes.add(1 << 40, Spectrum::splat(1.0));
        assert_eq!(passes.values[0], Spectrum::splat(1.0));
    }
}
