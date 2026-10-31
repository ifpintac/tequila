//! Lighting presets and the integrator configuration they expand to.
//!
//! `docs/01-RENDERER-CORE.md` §11 puts it plainly: the Lighting tab presets
//! are a UI over the configuration below, and **that table is the
//! implementation**. This module transcribes it, so the presets cannot drift
//! away from the specification without a test failing.

/// Everything the integrator's control flow is parameterized by.
#[derive(Clone, Debug, PartialEq)]
pub struct IntegratorConfig {
    pub max_bounces: u32,
    /// Indirect illumination, as the Lighting tab presents it.
    ///
    /// The integrator does not branch on this. In a path tracer "no indirect
    /// light" *is* a bounce limit of 1: the camera ray scatters once, and
    /// anything it finds after that is by definition indirect. The flag exists
    /// so the Custom panel can present the concept the way users think about
    /// it, and the presets keep the two consistent — asserted by
    /// [`tests::global_illumination_agrees_with_the_bounce_limit`].
    pub global_illumination: bool,
    /// Whether next-event estimation runs at all.
    pub direct_lighting: bool,
    pub shadows: bool,
    /// Whether emissive geometry lights the scene.
    pub light_source_materials: bool,
    /// Whether the ground plane bounces light back into the scene.
    pub ground_illumination: bool,
    /// The supplementary caustics pass (§6.5,
    /// [`crate::integrator::caustics`]): photons traced from the lights
    /// through glass, liquids, gems and mirrors onto what is around them,
    /// read by the camera path at the first surface it sees that is not
    /// mirror-smooth. Off, shadow rays and diffuse bounces pass clear glass
    /// instead ([`Self::light_through_glass`]), as KeyShot's renderer does
    /// with its caustics off, glossy bounces see it as glass, and light
    /// focused by anything else is found only by chance. How the pass finds
    /// that light is [`Self::caustics_method`].
    pub caustics: bool,
    /// How the caustics pass finds focused light, when it is on: photons
    /// from every light, or the paths from each lamp through smooth glass
    /// solved for at every surface ([`CausticsMethod`]).
    pub caustics_method: CausticsMethod,
    /// Portal sampling and increased NEE weighting for enclosed scenes.
    pub interior_mode: bool,
    /// Bounce at which the roughness floor starts being applied (§3.5).
    pub regularization_start_bounce: u32,
    /// The floor itself: `alpha = max(alpha, this)`.
    pub regularization_roughness: f32,
    /// Per-pixel relative error below which a pixel retires (§4.4).
    pub adaptive_sampling_threshold: f32,
    /// Tracked separately from `max_bounces`. §11: "glass and gems need many
    /// transmission events but few diffuse ones, and conflating them either
    /// makes glass black or makes everything slow".
    pub max_transmission_bounces: u32,
    pub russian_roulette_start_bounce: u32,
    /// How wide one pixel is, as an angle in radians: what a texture lookup's
    /// footprint grows by per unit of distance.
    ///
    /// Set by the render for its camera and resolution, never saved. A fixed
    /// guess of a thousandth suits a 660-pixel viewport at 50 mm and is three
    /// times too wide at 1920, where it picked MIP levels a step and a half
    /// too soft and fine print went mushy in the final image.
    pub pixel_spread: f32,
    /// Set on the path a shadow catcher traces for its own reflection, so
    /// that a catcher met along it is the floor it stands for rather than a
    /// view of the backdrop with yet another reflection in it: one bounce,
    /// not a recursion. Floor and part could otherwise reflect each other
    /// until the stack ran out.
    pub in_floor_reflection: bool,
    /// Render only the floor's reflection: a camera ray meeting a shadow
    /// catcher returns what the floor reflects and that share as coverage,
    /// and every other pixel nothing. Laid over an image Cycles rendered
    /// without one, it gives Cycles the same floor reflection as Tequila's
    /// renderer, which Cycles cannot make itself (its catcher reflects only
    /// as a ratio of the backdrop, so on a black one nothing shows). Never
    /// saved.
    pub floor_reflection_only: bool,
    /// Set on the path traced for what plain glass the camera sees reflects,
    /// split off the camera's path there (`integrator::trace`): it splits
    /// no glass of its own, and the sky it reaches is a reflection's, not
    /// the backdrop the camera sees. Never saved.
    pub reflected: bool,
    /// What Cycles is told beyond the above (Blender's Light Paths): `None`
    /// throughout is what it was always told.
    pub cycles: CyclesTransport,
    /// How a pixel's samples are spread, from the render settings for a
    /// final render; one pixel's box otherwise. Set per render, never saved
    /// here (it is the render's setting, `RenderSettings::pixel_filter`).
    pub pixel_filter: PixelFilter,
    /// Whether paths carry wavelengths rather than RGB (§7.4,
    /// [`crate::spectral`]). From the render settings, never saved here
    /// (`RenderSettings::spectral`).
    pub spectral: SpectralMode,
    /// Whether a spectral path carries its hero wavelength's three
    /// companions, as Wilkie et al. have it, or the hero alone from the
    /// start. Only ever off to measure what the companions buy; never saved.
    pub spectral_companions: bool,
}

/// Whether a render traces wavelengths or RGB (`docs/01` §7.4).
///
/// RGB cannot split light by wavelength, and a spectral path, which can,
/// costs more per vertex: each colour it meets is read as a spectrum at its
/// four wavelengths. So by default a scene is rendered spectrally only when
/// something in it disperses light.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SpectralMode {
    /// Spectral when the scene has a dispersive material, RGB otherwise.
    #[default]
    Auto,
    On,
    Off,
}

impl SpectralMode {
    pub const ALL: [SpectralMode; 3] = [SpectralMode::Auto, SpectralMode::On, SpectralMode::Off];

    /// The name in the file format.
    pub const fn key(self) -> &'static str {
        match self {
            SpectralMode::Auto => "auto",
            SpectralMode::On => "on",
            SpectralMode::Off => "off",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            SpectralMode::Auto => "Auto",
            SpectralMode::On => "On",
            SpectralMode::Off => "Off",
        }
    }

    pub fn from_key(key: &str) -> Option<SpectralMode> {
        SpectralMode::ALL.into_iter().find(|m| m.key() == key)
    }

    /// Whether `scene` is rendered spectrally.
    pub fn resolve(self, scene: &crate::scene::SceneSnapshot) -> bool {
        match self {
            SpectralMode::Auto => scene.has_dispersion(),
            SpectralMode::On => true,
            SpectralMode::Off => false,
        }
    }
}

/// How the caustics pass ([`IntegratorConfig::caustics`]) finds the light
/// that glass, liquids, gems and mirrors focus onto what is around them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CausticsMethod {
    /// Photons traced from every light, the environment's included,
    /// through glass and off mirrors ([`crate::integrator::caustics`]):
    /// soft at first and sharper the longer a render runs.
    #[default]
    Photons,
    /// Manifold next event estimation ([`crate::integrator::manifold`]):
    /// at each surface, the path from a lamp through smooth glass to it is
    /// solved for, so a caustic is sharp from the first samples where the
    /// path is found - but only the path the straight line to the lamp
    /// leads to, so light that crosses the glass elsewhere is missed, and
    /// at equal time it is noisier than the photons (`docs/01` §6.5).
    /// Lamps through refracting glass only: the environment and glowing
    /// parts reach through glass by bounces, and mirrors focus nothing here.
    Manifold,
}

impl CausticsMethod {
    pub const ALL: [CausticsMethod; 2] = [CausticsMethod::Photons, CausticsMethod::Manifold];

    /// The name in the file format.
    pub const fn key(self) -> &'static str {
        match self {
            CausticsMethod::Photons => "photons",
            CausticsMethod::Manifold => "manifold",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            CausticsMethod::Photons => "Photons",
            CausticsMethod::Manifold => "Manifold",
        }
    }

    pub fn from_key(key: &str) -> Option<CausticsMethod> {
        CausticsMethod::ALL.into_iter().find(|m| m.key() == key)
    }
}

/// The clear glass ([`crate::scene::Pane`]) a shadow ray passes on its way
/// to a light, which follows from how caustics are found
/// ([`IntegratorConfig::shadow_glass`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShadowGlass {
    /// None: glass stops it as anything solid does, and the light glass
    /// lets by is the photons'.
    Solid,
    /// All of it, straight through, tinted and less what it reflects.
    Clear,
    /// Glass that lets light by without bending it - a thin wall, glass at
    /// the index of air - and nothing that bends it, the light through
    /// which manifold next event estimation finds along the bent path.
    Unbent,
}

/// [`IntegratorConfig::pixel_spread`] when nothing better is known.
pub const DEFAULT_PIXEL_SPREAD: f32 = 1e-3;

/// Path limits and look settings Cycles distinguishes and Tequila's renderer
/// does not, for a studio that has to match Blender's exactly (Blender's
/// Light Paths panel). Each `None` sends Cycles what it was always sent:
/// diffuse and glossy bounces both `max_bounces` (at least 4), transparent
/// 8, Filter Glossy 1, Cycles' own volume bounces and caustics.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct CyclesTransport {
    /// Diffuse bounces. Set, or [`glossy_bounces`](Self::glossy_bounces)
    /// set, `max_bounces` is the total for all kinds together, as Blender's
    /// Total.
    pub diffuse_bounces: Option<u32>,
    pub glossy_bounces: Option<u32>,
    /// Passes through transparent surfaces and cutouts.
    pub transparent_bounces: Option<u32>,
    pub volume_bounces: Option<u32>,
    /// Blurs glossy reflections after diffuse bounces to cut noise; 0 is
    /// none, Blender's 0.5 a little.
    pub filter_glossy: Option<f32>,
    pub caustics_reflective: Option<bool>,
    pub caustics_refractive: Option<bool>,
}

impl CyclesTransport {
    /// Nothing set: what Cycles has always been sent.
    pub const NONE: CyclesTransport = CyclesTransport {
        diffuse_bounces: None,
        glossy_bounces: None,
        transparent_bounces: None,
        volume_bounces: None,
        filter_glossy: None,
        caustics_reflective: None,
        caustics_refractive: None,
    };
}

/// The shape a pixel's samples are spread over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFilterKind {
    /// Even over the pixel: sharp, a little aliased. Cycles' own default.
    Box,
    Gaussian,
    /// Blender's default: soft, without ringing.
    BlackmanHarris,
}

impl PixelFilterKind {
    pub const ALL: [PixelFilterKind; 3] = [PixelFilterKind::Box, PixelFilterKind::Gaussian, PixelFilterKind::BlackmanHarris];

    /// The name in the file format, and Cycles' own for its Film's filter.
    pub const fn key(self) -> &'static str {
        match self {
            PixelFilterKind::Box => "box",
            PixelFilterKind::Gaussian => "gaussian",
            PixelFilterKind::BlackmanHarris => "blackman_harris",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            PixelFilterKind::Box => "Box",
            PixelFilterKind::Gaussian => "Gaussian",
            PixelFilterKind::BlackmanHarris => "Blackman-Harris",
        }
    }

    pub fn from_key(key: &str) -> Option<PixelFilterKind> {
        PixelFilterKind::ALL.into_iter().find(|k| k.key() == key)
    }
}

/// A pixel filter: its shape and its width in pixels, as Cycles' Film takes
/// them (a Gaussian's width is a third of its support, a Blackman-Harris's
/// half of it, as there).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PixelFilter {
    pub kind: PixelFilterKind,
    pub width: f32,
}

impl PixelFilter {
    /// One pixel's box: what Tequila's renderer always used, and Cycles'
    /// default.
    pub const BOX: PixelFilter = PixelFilter { kind: PixelFilterKind::Box, width: 1.0 };

    /// What spreads a sample by this filter: filter importance sampling, so
    /// each sample still lands in one pixel and accumulation stays a mean.
    pub fn sampler(self) -> FilterSampler {
        let width = if self.width.is_finite() && self.width > 0.0 { self.width } else { 1.0 };
        let (half, f): (f32, Box<dyn Fn(f32) -> f32>) = match self.kind {
            PixelFilterKind::Box => return FilterSampler { width, table: None },
            PixelFilterKind::Gaussian => {
                let support = width * 3.0;
                (support * 0.5, Box::new(move |v: f32| (-2.0 * (v * 6.0 / support).powi(2)).exp()))
            }
            PixelFilterKind::BlackmanHarris => {
                let support = width * 2.0;
                (
                    support * 0.5,
                    Box::new(move |v: f32| {
                        let v = std::f32::consts::TAU * (v / support + 0.5);
                        0.35875 - 0.48829 * v.cos() + 0.14128 * (2.0 * v).cos() - 0.01168 * (3.0 * v).cos()
                    }),
                )
            }
        };
        // The inverse of the filter's distribution over one half, by a table
        // fine enough that the steps are far below a sample's spread.
        const STEPS: usize = 1024;
        let mut cdf = Vec::with_capacity(STEPS + 1);
        let mut total = 0.0f64;
        cdf.push(0.0f64);
        for i in 0..STEPS {
            let x = (i as f32 + 0.5) / STEPS as f32 * half;
            total += f64::from(f(x).max(0.0));
            cdf.push(total);
        }
        let mut table = Vec::with_capacity(STEPS + 1);
        let mut j = 0usize;
        for i in 0..=STEPS {
            let target = total * i as f64 / STEPS as f64;
            while j < STEPS && cdf[j + 1] < target {
                j += 1;
            }
            let span = (cdf[j + 1] - cdf[j]).max(1e-12);
            let within = ((target - cdf[j]) / span).clamp(0.0, 1.0);
            table.push((j as f64 + within) as f32 / STEPS as f32 * half);
        }
        FilterSampler { width, table: Some(table) }
    }
}

#[cfg(test)]
mod filter_tests {
    use super::*;
    use tequila_core::math::vec2;

    /// One pixel's box places a sample exactly where the jitter always did,
    /// so a render that asks for no filter is the same picture; the others
    /// spread samples symmetrically about the centre, within their support,
    /// Blackman-Harris at 1.5 px over three pixels and more of them near the
    /// middle than a box puts there.
    #[test]
    fn a_pixel_filter_places_samples_by_its_shape() {
        let boxed = PixelFilter::BOX.sampler();
        for u in [0.0, 0.13, 0.5, 0.999] {
            assert_eq!(boxed.place(vec2(u, 1.0 - u)), vec2(u, 1.0 - u));
        }
        let bh = PixelFilter { kind: PixelFilterKind::BlackmanHarris, width: 1.5 }.sampler();
        let n = 4000;
        let (mut sum, mut near, mut widest) = (0.0f64, 0, 0.0f32);
        for i in 0..n {
            let u = (i as f32 + 0.5) / n as f32;
            let p = bh.place(vec2(u, 0.5)).x - 0.5;
            sum += f64::from(p);
            widest = widest.max(p.abs());
            if p.abs() < 0.25 {
                near += 1;
            }
        }
        assert!((sum / f64::from(n)).abs() < 1e-3, "spread about the centre: mean {}", sum / f64::from(n));
        assert!(widest <= 1.5 + 1e-4 && widest > 1.0, "within its support of three pixels: {widest}");
        assert!(near as f32 / n as f32 > 0.3, "more near the middle than a box puts there: {near} of {n}");
        assert_eq!(bh.place(vec2(0.5, 0.5)), vec2(0.5, 0.5), "the middle of the range is the pixel's centre");
        let g = PixelFilter { kind: PixelFilterKind::Gaussian, width: 1.5 }.sampler();
        let edge = g.place(vec2(0.0, 1.0));
        assert!(edge.x < 0.5 - 1.0 && edge.y > 0.5 + 1.0, "a Gaussian reaches out to its support: {edge:?}");
    }
}

/// A [`PixelFilter`] ready to place samples (see [`PixelFilter::sampler`]).
#[derive(Clone, Debug, PartialEq)]
pub struct FilterSampler {
    width: f32,
    /// The inverse distribution over one half, `None` for a box.
    table: Option<Vec<f32>>,
}

impl FilterSampler {
    /// Where in (or around) the pixel a sample goes, for `jitter` uniform in
    /// [0, 1)²: the same point a box of one pixel always gave, and otherwise
    /// a point distributed as the filter is, about the pixel's centre.
    pub fn place(&self, jitter: tequila_core::math::Vec2) -> tequila_core::math::Vec2 {
        let along = |u: f32| match &self.table {
            None if self.width == 1.0 => u,
            None => 0.5 + (u - 0.5) * self.width,
            Some(table) => {
                let signed = 2.0 * u - 1.0;
                let at = signed.abs() * (table.len() - 1) as f32;
                let i = (at as usize).min(table.len() - 2);
                let t = at - i as f32;
                let offset = table[i] + (table[i + 1] - table[i]) * t;
                0.5 + offset.copysign(signed)
            }
        };
        tequila_core::math::vec2(along(jitter.x), along(jitter.y))
    }
}

impl Default for IntegratorConfig {
    fn default() -> Self {
        Self::product()
    }
}

impl IntegratorConfig {
    /// Whether shadow rays pass clear glass ([`crate::scene::Pane`]), so
    /// what is behind a pane or inside a bottle is lit through it. Not with
    /// the caustics pass, whose photons already carry the light glass lets
    /// by: a shadow ray that passed it too would count that light twice.
    /// With manifold caustics, only glass that does not bend light
    /// ([`Self::shadow_glass`]).
    ///
    /// The bounces off a surface that took such a light sample pass the
    /// glass too, and are weighed against the sample as in the open - the
    /// diffuse ones, and the glossy ones off a rough reflection. A bounce
    /// off a sharper reflection (the coat's, the interface's, a metal's)
    /// sees glass that bends light as glass, as the camera does, so a
    /// bottle mirrored in a glossy floor is mirrored as glass, with what it
    /// bends and the lamps it shows; the light through such glass is shared
    /// between the sample and the bounces so that each direction's is
    /// counted once (`integrator::left_to_bounces`).
    #[inline]
    pub fn light_through_glass(&self) -> bool {
        self.shadow_glass() != ShadowGlass::Solid
    }

    /// Which clear glass shadow rays pass. All of it with the caustics pass
    /// off, as KeyShot's renderer does; none with photons, which carry that
    /// light; and with manifold caustics only what does not bend light. Glass
    /// that bends it then stops every shadow ray, the environment's and a
    /// glowing part's too: a lamp's light through it is the manifold
    /// sample's, and theirs is found by bounces through the glass, which
    /// meet them where the bent path really leads. Let by straight as well,
    /// the light under a glass ball would be counted twice, once spread
    /// over its shadow and once focused.
    ///
    /// With the caustics pass off, the diffuse bounces off a surface that
    /// sampled the lights through glass pass it as its shadow rays did, and
    /// its glossy bounces pass only glass that does not bend light
    /// (`Unbent`): glass that bends it they see as glass, and meet what is
    /// past it along the bent path (`integrator::sees_glass`).
    #[inline]
    pub fn shadow_glass(&self) -> ShadowGlass {
        match (self.caustics, self.caustics_method) {
            (false, _) => ShadowGlass::Clear,
            (true, CausticsMethod::Photons) => ShadowGlass::Solid,
            (true, CausticsMethod::Manifold) => ShadowGlass::Unbent,
        }
    }

    /// Whether this render traces caustic photons.
    #[inline]
    pub fn photon_caustics(&self) -> bool {
        self.caustics && self.caustics_method == CausticsMethod::Photons
    }

    /// Whether lamps' light through smooth glass is found by manifold next
    /// event estimation ([`crate::integrator::manifold`]).
    #[inline]
    pub fn manifold_caustics(&self) -> bool {
        self.caustics && self.caustics_method == CausticsMethod::Manifold
    }

    /// Values shared by every preset. §3.4 fixes Russian roulette at bounce 3:
    /// "earlier termination adds variance at little speed gain in typical
    /// product scenes". §3.5 fixes the roughness floor at 0.09.
    const fn base() -> Self {
        IntegratorConfig {
            max_bounces: 6,
            global_illumination: true,
            direct_lighting: true,
            shadows: true,
            light_source_materials: true,
            ground_illumination: false,
            caustics: false,
            caustics_method: CausticsMethod::Photons,
            interior_mode: false,
            regularization_start_bounce: 2,
            regularization_roughness: 0.09,
            adaptive_sampling_threshold: 0.01,
            max_transmission_bounces: 8,
            russian_roulette_start_bounce: 3,
            pixel_spread: DEFAULT_PIXEL_SPREAD,
            in_floor_reflection: false,
            floor_reflection_only: false,
            reflected: false,
            cycles: CyclesTransport::NONE,
            pixel_filter: PixelFilter::BOX,
            spectral: SpectralMode::Auto,
            spectral_companions: true,
        }
    }

    /// Scene setup and fast navigation: no light-source materials, no
    /// shadows, minimal bounces.
    pub const fn performance() -> Self {
        IntegratorConfig {
            max_bounces: 1,
            global_illumination: false,
            shadows: false,
            light_source_materials: false,
            regularization_start_bounce: 1,
            adaptive_sampling_threshold: 0.05,
            max_transmission_bounces: 0,
            ..Self::base()
        }
    }

    /// Simple direct lighting with shadows, for models lit by the environment
    /// alone.
    pub const fn basic() -> Self {
        IntegratorConfig {
            max_bounces: 2,
            global_illumination: false,
            regularization_start_bounce: 1,
            adaptive_sampling_threshold: 0.02,
            max_transmission_bounces: 2,
            ..Self::base()
        }
    }

    /// The default: direct and indirect lighting with shadows, for product
    /// shots with transparent materials and local lights.
    pub const fn product() -> Self {
        Self::base()
    }

    /// Optimized for enclosed scenes: more bounces, ground illumination, and
    /// the portal-sampling strategy §5.2 calls for.
    pub const fn interior() -> Self {
        IntegratorConfig {
            max_bounces: 8,
            ground_illumination: true,
            interior_mode: true,
            ..Self::base()
        }
    }

    /// Interior plus ground illumination and the longest transmission chains
    /// — the preset that exists precisely because gems need them.
    ///
    /// It also turns on [`IntegratorConfig::caustics`], so the light a gem
    /// focuses is traced from the lights rather than found by chance.
    pub const fn jewelry() -> Self {
        IntegratorConfig {
            max_bounces: 12,
            ground_illumination: true,
            caustics: true,
            interior_mode: true,
            regularization_start_bounce: 3,
            adaptive_sampling_threshold: 0.005,
            max_transmission_bounces: 16,
            ..Self::base()
        }
    }
}

/// The Lighting tab's state: a preset, or a hand-tuned configuration.
///
/// `TEQUILA_SPEC.md` §1.6 asks for "the five presets + Custom", and Custom is
/// modelled as the *absence* of a preset rather than as a sixth one. That is
/// what makes the two facts a user cares about representable at the same time:
/// which preset is selected, and — once they touch a control — that the scene
/// no longer matches any of them. It also makes "customize" mean the obvious
/// thing, since the custom configuration starts as a copy of whatever preset
/// was selected instead of as some unrelated default.
#[derive(Clone, Debug, PartialEq)]
pub struct LightingSettings {
    /// `None` is Custom.
    pub preset: Option<LightingPreset>,
    /// The configuration Custom uses. Kept even while a preset is selected, so
    /// that switching to a preset and back does not discard a tuned setup.
    pub custom: IntegratorConfig,
}

impl Default for LightingSettings {
    fn default() -> Self {
        LightingSettings {
            preset: Some(LightingPreset::Product),
            custom: IntegratorConfig::product(),
        }
    }
}

impl LightingSettings {
    pub fn from_preset(preset: LightingPreset) -> LightingSettings {
        LightingSettings {
            preset: Some(preset),
            custom: preset.config(),
        }
    }

    /// What the integrator should run with.
    pub fn config(&self) -> IntegratorConfig {
        match self.preset {
            Some(preset) => preset.config(),
            None => self.custom.clone(),
        }
    }

    pub fn label(&self) -> &'static str {
        match self.preset {
            Some(preset) => preset.label(),
            None => "Custom",
        }
    }

    /// Leave the preset behind, keeping the configuration it produced.
    ///
    /// Called when a control on the Custom panel is touched: the values the
    /// user sees carry over, so the first edit changes exactly the one thing
    /// they edited rather than also snapping every other control to a default.
    pub fn customize(&mut self) {
        if let Some(preset) = self.preset.take() {
            self.custom = preset.config();
        }
    }

    /// Select a preset, leaving the custom configuration untouched.
    pub fn select(&mut self, preset: LightingPreset) {
        self.preset = Some(preset);
    }
}

/// The named presets of `TEQUILA_SPEC.md` §1.6.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LightingPreset {
    Performance,
    Basic,
    Product,
    Interior,
    Jewelry,
}

impl LightingPreset {
    pub fn config(self) -> IntegratorConfig {
        match self {
            LightingPreset::Performance => IntegratorConfig::performance(),
            LightingPreset::Basic => IntegratorConfig::basic(),
            LightingPreset::Product => IntegratorConfig::product(),
            LightingPreset::Interior => IntegratorConfig::interior(),
            LightingPreset::Jewelry => IntegratorConfig::jewelry(),
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            LightingPreset::Performance => "Performance",
            LightingPreset::Basic => "Basic",
            LightingPreset::Product => "Product",
            LightingPreset::Interior => "Interior",
            LightingPreset::Jewelry => "Jewelry",
        }
    }

    /// One line of guidance for the Lighting tab. Independently written, per
    /// `TEQUILA_SPEC.md` §0.2 rule 2.
    pub const fn description(self) -> &'static str {
        match self {
            LightingPreset::Performance => {
                "Fastest. Drops shadows and indirect light while you set the scene up."
            }
            LightingPreset::Basic => {
                "Direct light and shadows only. Good for models lit by the environment."
            }
            LightingPreset::Product => "Direct and indirect light. The default for product shots.",
            LightingPreset::Interior => {
                "Tuned for enclosed spaces lit through openings. Slower, much cleaner."
            }
            LightingPreset::Jewelry => {
                "Deep transmission chains and traced caustics, for gems and cut glass. Slowest."
            }
        }
    }

    pub const ALL: [LightingPreset; 5] = [
        LightingPreset::Performance,
        LightingPreset::Basic,
        LightingPreset::Product,
        LightingPreset::Interior,
        LightingPreset::Jewelry,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The preset table of `docs/01` §11 transcribed literally, so that a
    /// drift between the specification and the code is a test failure rather
    /// than a slow divergence nobody notices.
    #[test]
    fn presets_match_the_specification_table() {
        /// One row of the specification's table. A named struct rather
        /// than an eleven-wide tuple: at that width, a transposed pair of
        /// booleans is invisible on inspection, and this test exists
        /// precisely to catch transposed values.
        struct Row {
            preset: LightingPreset,
            bounces: u32,
            global_illumination: bool,
            shadows: bool,
            light_materials: bool,
            ground_illumination: bool,
            caustics: bool,
            interior: bool,
            regularization: u32,
            adaptive: f32,
            transmission: u32,
        }

        let table = [
            Row {
                preset: LightingPreset::Performance,
                bounces: 1,
                global_illumination: false,
                shadows: false,
                light_materials: false,
                ground_illumination: false,
                caustics: false,
                interior: false,
                regularization: 1,
                adaptive: 0.05,
                transmission: 0,
            },
            Row {
                preset: LightingPreset::Basic,
                bounces: 2,
                global_illumination: false,
                shadows: true,
                light_materials: true,
                ground_illumination: false,
                caustics: false,
                interior: false,
                regularization: 1,
                adaptive: 0.02,
                transmission: 2,
            },
            Row {
                preset: LightingPreset::Product,
                bounces: 6,
                global_illumination: true,
                shadows: true,
                light_materials: true,
                ground_illumination: false,
                caustics: false,
                interior: false,
                regularization: 2,
                adaptive: 0.01,
                transmission: 8,
            },
            Row {
                preset: LightingPreset::Interior,
                bounces: 8,
                global_illumination: true,
                shadows: true,
                light_materials: true,
                ground_illumination: true,
                caustics: false,
                interior: true,
                regularization: 2,
                adaptive: 0.01,
                transmission: 8,
            },
            Row {
                preset: LightingPreset::Jewelry,
                bounces: 12,
                global_illumination: true,
                shadows: true,
                light_materials: true,
                ground_illumination: true,
                caustics: true,
                interior: true,
                regularization: 3,
                adaptive: 0.005,
                transmission: 16,
            },
        ];

        for r in table {
            let c = r.preset.config();
            let name = r.preset.label();
            assert_eq!(c.max_bounces, r.bounces, "{name}: max bounces");
            assert_eq!(
                c.global_illumination, r.global_illumination,
                "{name}: global illumination"
            );
            assert_eq!(c.shadows, r.shadows, "{name}: shadows");
            assert_eq!(
                c.light_source_materials, r.light_materials,
                "{name}: light materials"
            );
            assert_eq!(
                c.ground_illumination, r.ground_illumination,
                "{name}: ground illumination"
            );
            assert_eq!(c.caustics, r.caustics, "{name}: caustics");
            assert_eq!(c.interior_mode, r.interior, "{name}: interior mode");
            assert_eq!(
                c.regularization_start_bounce, r.regularization,
                "{name}: regularization"
            );
            assert_eq!(
                c.adaptive_sampling_threshold, r.adaptive,
                "{name}: adaptive threshold"
            );
            assert_eq!(
                c.max_transmission_bounces, r.transmission,
                "{name}: transmission bounces"
            );
        }
    }

    /// Transmission depth is tracked separately from the general bounce limit.
    /// §11 explains why: conflating them "either makes glass black or makes
    /// everything slow", and the Jewelry preset exists for that distinction.
    #[test]
    fn transmission_depth_is_independent_of_bounce_depth() {
        let jewelry = IntegratorConfig::jewelry();
        assert!(
            jewelry.max_transmission_bounces > jewelry.max_bounces,
            "gems need more transmission events than diffuse bounces"
        );
        let product = IntegratorConfig::product();
        assert!(product.max_transmission_bounces > product.max_bounces / 2);
    }

    /// Quality must increase monotonically across the preset ladder — that is
    /// the whole promise the names make to the user.
    #[test]
    fn presets_are_ordered_by_quality() {
        let ladder = [
            LightingPreset::Performance,
            LightingPreset::Basic,
            LightingPreset::Product,
            LightingPreset::Interior,
            LightingPreset::Jewelry,
        ];
        for pair in ladder.windows(2) {
            let (a, b) = (pair[0].config(), pair[1].config());
            assert!(
                b.max_bounces >= a.max_bounces,
                "{} has fewer bounces than {}",
                pair[1].label(),
                pair[0].label()
            );
            assert!(
                b.adaptive_sampling_threshold <= a.adaptive_sampling_threshold,
                "{} has a looser convergence threshold than {}",
                pair[1].label(),
                pair[0].label()
            );
            assert!(b.max_transmission_bounces >= a.max_transmission_bounces);
        }
    }

    /// The `global_illumination` flag and the bounce limit must agree: a
    /// preset claiming indirect light while allowing only one bounce would be
    /// lying to the user, and one claiming none while allowing several would
    /// be slower than advertised.
    #[test]
    fn global_illumination_agrees_with_the_bounce_limit() {
        for p in LightingPreset::ALL {
            let c = p.config();
            assert_eq!(
                c.global_illumination,
                c.max_bounces > 2,
                "{}: global_illumination = {} with {} bounces",
                p.label(),
                c.global_illumination,
                c.max_bounces
            );
        }
    }

    #[test]
    fn every_preset_has_a_label_and_a_description() {
        for p in LightingPreset::ALL {
            assert!(!p.label().is_empty());
            assert!(
                p.description().len() > 20,
                "{}: thin description",
                p.label()
            );
        }
    }

    /// Russian roulette must never start before at least one indirect bounce
    /// has been taken (§3.4), or interiors go noisy for no benefit.
    #[test]
    fn russian_roulette_never_starts_too_early() {
        for p in LightingPreset::ALL {
            assert!(
                p.config().russian_roulette_start_bounce >= 2,
                "{} starts Russian roulette at bounce {}",
                p.label(),
                p.config().russian_roulette_start_bounce
            );
        }
    }
}
