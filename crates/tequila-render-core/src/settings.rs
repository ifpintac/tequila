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
    /// mirror-smooth. Off, such light is found only by chance.
    pub caustics: bool,
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
}

/// [`IntegratorConfig::pixel_spread`] when nothing better is known.
pub const DEFAULT_PIXEL_SPREAD: f32 = 1e-3;

impl Default for IntegratorConfig {
    fn default() -> Self {
        Self::product()
    }
}

impl IntegratorConfig {
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
            interior_mode: false,
            regularization_start_bounce: 2,
            regularization_roughness: 0.09,
            adaptive_sampling_threshold: 0.01,
            max_transmission_bounces: 8,
            russian_roulette_start_bounce: 3,
            pixel_spread: DEFAULT_PIXEL_SPREAD,
            in_floor_reflection: false,
            floor_reflection_only: false,
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
