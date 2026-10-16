//! Progressive scheduling, the dirty-state taxonomy, and the render
//! orchestrator.
//!
//! `docs/01-RENDERER-CORE.md` §9 opens by saying this section "is the
//! difference between 'a path tracer' and 'KeyShot'. Implement it exactly."
//!
//! Two ideas carry that weight:
//!
//! - **The UI thread never waits on a render.** It draws the most recent
//!   completed image; if none is ready it redraws the previous one. Frame
//!   pacing is decoupled from sample production entirely (§9.1).
//! - **Not every edit invalidates every sample.** The dirty-state taxonomy
//!   (§9.3) classifies each mutation so that a material tweak resets only the
//!   pixels showing that material, and an exposure change resets nothing at
//!   all. §9.3 calls this "worth more perceived performance than a 2x raw
//!   speedup".

use crate::framebuffer::Framebuffer;

/// How much accumulated work a scene change invalidates.
///
/// The variants are ordered by severity so that combining two changes is a
/// `max`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Dirty {
    /// Nothing to redo. Selection highlights and overlays land here.
    #[default]
    None,
    /// Re-apply the display transform to the existing buffer. No re-render at
    /// all, so it is instant regardless of scene complexity — the row of §9.3
    /// that makes dragging an exposure slider feel free.
    Display,
    /// Reset a rectangle of pixels; everything outside keeps its samples.
    Region,
    /// Reset the whole accumulation buffer.
    Full,
}

impl Dirty {
    #[inline]
    pub fn merge(self, other: Dirty) -> Dirty {
        self.max(other)
    }
}

/// The kinds of scene mutation §9.3 tabulates, and what each one costs.
///
/// Written as an enum rather than left to callers so that the table lives in
/// one place and can be tested against the specification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneChange {
    ImageStyle,
    Exposure,
    CameraTransform,
    CameraLens,
    Resolution,
    EnvironmentRotation,
    EnvironmentBrightness,
    MaterialParameter,
    MaterialAssignment,
    LabelEdited,
    LightParameter,
    LightAddedOrRemoved,
    InstanceTransform,
    GeometryEdit,
    VisibilityToggle,
    SelectionChange,
    ModelSetOrStudioSwitch,
}

impl SceneChange {
    /// The dirty classification of §9.3, transcribed.
    pub const fn dirty(self) -> Dirty {
        match self {
            // Kept: re-tonemapping the existing buffer is enough.
            SceneChange::ImageStyle | SceneChange::Exposure => Dirty::Display,
            // Kept: an overlay, not part of the image.
            SceneChange::SelectionChange => Dirty::None,
            // Kept outside the affected instances' screen bounds.
            SceneChange::MaterialParameter
            | SceneChange::MaterialAssignment
            | SceneChange::LabelEdited
            | SceneChange::LightParameter => Dirty::Region,
            // Everything else discards accumulation.
            SceneChange::CameraTransform
            | SceneChange::CameraLens
            | SceneChange::Resolution
            | SceneChange::EnvironmentRotation
            | SceneChange::EnvironmentBrightness
            | SceneChange::LightAddedOrRemoved
            | SceneChange::InstanceTransform
            | SceneChange::GeometryEdit
            | SceneChange::VisibilityToggle
            | SceneChange::ModelSetOrStudioSwitch => Dirty::Full,
        }
    }

    /// Whether the change requires an acceleration structure rebuild, which
    /// §9.4 requires to happen asynchronously while the previous structure
    /// keeps being used.
    pub const fn requires_rebuild(self) -> bool {
        matches!(
            self,
            SceneChange::InstanceTransform
                | SceneChange::GeometryEdit
                | SceneChange::VisibilityToggle
                | SceneChange::LightAddedOrRemoved
                | SceneChange::ModelSetOrStudioSwitch
        )
    }
}

/// Which regime the renderer is in (§9.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Regime {
    /// The user is manipulating the view. Reduced resolution, one sample per
    /// pixel, denoise every frame. §9.2 targets 30 fps minimum.
    Interactive,
    /// Input has been idle long enough to start refining.
    Refining,
}

/// Milliseconds of idle input after which refinement begins (§9.2).
pub const INTERACTIVE_IDLE_MS: u64 = 150;

/// The resolution ladder the renderer climbs after input stops (§9.2):
/// quarter, half, then full linear resolution.
pub const RESOLUTION_LADDER: [u32; 3] = [4, 2, 1];

/// Progressive resolution and denoise scheduling.
#[derive(Clone, Debug)]
pub struct ProgressiveSchedule {
    pub regime: Regime,
    /// Linear resolution divisor: 4 means quarter width and height.
    pub resolution_divisor: u32,
    /// Milliseconds since the last user input.
    pub idle_ms: u64,
}

impl Default for ProgressiveSchedule {
    fn default() -> Self {
        ProgressiveSchedule {
            regime: Regime::Interactive,
            resolution_divisor: RESOLUTION_LADDER[0],
            idle_ms: 0,
        }
    }
}

impl ProgressiveSchedule {
    /// Advance the schedule given how long input has been idle.
    ///
    /// The ladder steps up over roughly the first 300 ms after input stops,
    /// restarting accumulation at each step. §9.2 notes that is cheap: only a
    /// few samples are lost, and the payoff is that the image is sharp almost
    /// immediately after the user lets go.
    pub fn update(&mut self, idle_ms: u64) {
        self.idle_ms = idle_ms;
        if idle_ms < INTERACTIVE_IDLE_MS {
            self.regime = Regime::Interactive;
            self.resolution_divisor = RESOLUTION_LADDER[0];
            return;
        }
        self.regime = Regime::Refining;
        let step_ms = 150;
        let steps = ((idle_ms - INTERACTIVE_IDLE_MS) / step_ms) as usize;
        self.resolution_divisor = RESOLUTION_LADDER[steps.min(RESOLUTION_LADDER.len() - 1)];
    }

    /// Note that the user has just interacted, dropping straight back to the
    /// interactive regime.
    pub fn interrupt(&mut self) {
        self.regime = Regime::Interactive;
        self.resolution_divisor = RESOLUTION_LADDER[0];
        self.idle_ms = 0;
    }

    #[inline]
    pub fn is_interactive(&self) -> bool {
        self.regime == Regime::Interactive
    }

    /// Whether to run the denoiser at this sample count.
    ///
    /// §9.2: every frame while interacting; then on a decaying cadence — every
    /// frame for the first 8 samples, then every 2, 4, 8, 16... The cost of
    /// denoising must not come to dominate sample production.
    pub fn should_denoise(&self, samples: u32) -> bool {
        self.should_denoise_every(samples, 1)
    }

    /// The same, with a cadence the user asked for.
    ///
    /// A cadence of one means §9.2's automatic policy, which is the right
    /// answer almost always: dense early, thinning out as the image settles.
    /// A larger number replaces it with a fixed interval, and exists because
    /// the automatic policy is tuned for a viewport-sized image — at 4K the
    /// denoiser costs more than the samples it is smoothing, and a user
    /// watching a big render wants it out of the way.
    ///
    /// Neither applies while the camera is moving. A frame the user is looking
    /// at *during* a move is almost entirely noise, and skipping the denoiser
    /// there would make navigation look broken rather than fast.
    pub fn should_denoise_every(&self, samples: u32, cadence: u32) -> bool {
        if self.regime == Regime::Interactive {
            return true;
        }
        if samples == 0 {
            return false;
        }
        let cadence = cadence.max(1);
        if cadence > 1 {
            return samples % cadence == 0;
        }
        if samples <= 8 {
            return true;
        }
        // Denoise when the sample count crosses a power of two.
        samples.is_power_of_two()
    }

    /// Whether adaptive sampling is active yet. §9.2 enables it after 16 spp,
    /// once the per-pixel variance estimate means something.
    pub fn adaptive_sampling_active(&self, samples: u32) -> bool {
        self.regime == Regime::Refining && samples >= 16
    }

    /// Render resolution for a given display resolution.
    pub fn render_resolution(&self, width: usize, height: usize) -> (usize, usize) {
        let d = self.resolution_divisor.max(1) as usize;
        ((width / d).max(1), (height / d).max(1))
    }
}

/// A rectangular unit of work.
///
/// 16x16, per §10.2. Small enough to balance across threads on a scene with
/// wildly varying per-pixel cost, large enough that the per-tile overhead and
/// the loss of ray coherence stay negligible.
pub const TILE_SIZE: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile {
    pub x0: usize,
    pub y0: usize,
    pub x1: usize,
    pub y1: usize,
}

impl Tile {
    #[inline]
    pub fn width(&self) -> usize {
        self.x1 - self.x0
    }

    #[inline]
    pub fn height(&self) -> usize {
        self.y1 - self.y0
    }

    #[inline]
    pub fn pixel_count(&self) -> usize {
        self.width() * self.height()
    }
}

/// Split an image into tiles.
///
/// Region rendering (§9.5) is expressed as a filter over this queue rather
/// than as a separate code path, exactly as that section requires — so a
/// region render is the same renderer, just fed fewer tiles.
pub fn tiles(width: usize, height: usize, region: Option<Tile>) -> Vec<Tile> {
    let mut out = Vec::new();
    let bound = region.unwrap_or(Tile {
        x0: 0,
        y0: 0,
        x1: width,
        y1: height,
    });
    let x0 = bound.x0.min(width);
    let y0 = bound.y0.min(height);
    let x1 = bound.x1.min(width);
    let y1 = bound.y1.min(height);

    let mut y = y0;
    while y < y1 {
        let mut x = x0;
        while x < x1 {
            out.push(Tile {
                x0: x,
                y0: y,
                x1: (x + TILE_SIZE).min(x1),
                y1: (y + TILE_SIZE).min(y1),
            });
            x += TILE_SIZE;
        }
        y += TILE_SIZE;
    }
    out
}

/// Statistics for the real-time view HUD (`TEQUILA_SPEC.md` §1.2).
#[derive(Clone, Copy, Debug, Default)]
pub struct RenderStats {
    pub samples: u32,
    pub converged_fraction: f32,
    pub triangles: usize,
    pub instances: usize,
    pub elapsed_seconds: f64,
    pub samples_per_second: f64,
    pub resolution: (usize, usize),
    pub interactive: bool,
}

impl RenderStats {
    pub fn from_framebuffer(fb: &Framebuffer) -> Self {
        RenderStats {
            samples: fb.min_samples(),
            converged_fraction: fb.converged_fraction(),
            resolution: (fb.width, fb.height),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The §9.3 table, transcribed. A change to the renderer that silently
    /// promoted a Region restart to a Full one would cost most of the app's
    /// perceived speed, and nothing else would catch it.
    #[test]
    fn dirty_classification_matches_the_specification_table() {
        use SceneChange::*;
        let expected = [
            (ImageStyle, Dirty::Display),
            (Exposure, Dirty::Display),
            (SelectionChange, Dirty::None),
            (MaterialParameter, Dirty::Region),
            (MaterialAssignment, Dirty::Region),
            (LabelEdited, Dirty::Region),
            (LightParameter, Dirty::Region),
            (CameraTransform, Dirty::Full),
            (CameraLens, Dirty::Full),
            (Resolution, Dirty::Full),
            (EnvironmentRotation, Dirty::Full),
            (EnvironmentBrightness, Dirty::Full),
            (LightAddedOrRemoved, Dirty::Full),
            (InstanceTransform, Dirty::Full),
            (GeometryEdit, Dirty::Full),
            (VisibilityToggle, Dirty::Full),
            (ModelSetOrStudioSwitch, Dirty::Full),
        ];
        for (change, dirty) in expected {
            assert_eq!(change.dirty(), dirty, "{change:?}");
        }
    }

    /// The headline interaction of the whole application — dragging a material
    /// onto a part — must not throw the image away.
    #[test]
    fn material_edits_keep_samples_outside_their_region() {
        assert_eq!(SceneChange::MaterialAssignment.dirty(), Dirty::Region);
        assert_eq!(SceneChange::MaterialParameter.dirty(), Dirty::Region);
        assert!(!SceneChange::MaterialAssignment.requires_rebuild());
    }

    /// Exposure and image style must cost nothing at all: §9.3 marks them
    /// "no re-render at all. Instant."
    #[test]
    fn look_development_never_re_renders() {
        for change in [SceneChange::Exposure, SceneChange::ImageStyle] {
            assert_eq!(change.dirty(), Dirty::Display);
            assert!(!change.requires_rebuild());
        }
    }

    #[test]
    fn merging_dirty_states_takes_the_worst() {
        assert_eq!(Dirty::None.merge(Dirty::Region), Dirty::Region);
        assert_eq!(Dirty::Full.merge(Dirty::Display), Dirty::Full);
        assert_eq!(Dirty::Display.merge(Dirty::Display), Dirty::Display);
        assert_eq!(Dirty::None.merge(Dirty::None), Dirty::None);
    }

    #[test]
    fn the_resolution_ladder_climbs_after_input_stops() {
        let mut s = ProgressiveSchedule::default();
        s.update(0);
        assert!(s.is_interactive());
        assert_eq!(s.resolution_divisor, 4);

        s.update(INTERACTIVE_IDLE_MS);
        assert_eq!(s.regime, Regime::Refining);
        assert_eq!(s.resolution_divisor, 4);

        s.update(INTERACTIVE_IDLE_MS + 150);
        assert_eq!(s.resolution_divisor, 2);
        s.update(INTERACTIVE_IDLE_MS + 300);
        assert_eq!(s.resolution_divisor, 1);
        // And it stays at full resolution rather than overshooting.
        s.update(60_000);
        assert_eq!(s.resolution_divisor, 1);
    }

    #[test]
    fn interacting_drops_straight_back_to_the_fast_path() {
        let mut s = ProgressiveSchedule::default();
        s.update(10_000);
        assert_eq!(s.resolution_divisor, 1);
        s.interrupt();
        assert!(s.is_interactive());
        assert_eq!(s.resolution_divisor, 4);
    }

    /// Quarter linear resolution is one sixteenth of the pixels — the point of
    /// the interactive regime.
    #[test]
    fn interactive_resolution_is_a_sixteenth_of_the_pixels() {
        let s = ProgressiveSchedule::default();
        let (w, h) = s.render_resolution(1920, 1080);
        assert_eq!((w, h), (480, 270));
        assert_eq!(1920 * 1080 / (w * h), 16);
    }

    #[test]
    fn render_resolution_never_collapses_to_zero() {
        let s = ProgressiveSchedule::default();
        assert_eq!(s.render_resolution(2, 2), (1, 1));
        assert_eq!(s.render_resolution(1, 1), (1, 1));
    }

    /// Denoising must be constant while interacting and then decay, so its
    /// cost never comes to dominate sample production (§9.2).
    #[test]
    fn denoise_cadence_decays_while_refining() {
        let mut s = ProgressiveSchedule::default();
        s.update(0);
        for n in [1u32, 2, 37, 1000] {
            assert!(s.should_denoise(n), "interactive must denoise every frame");
        }

        s.update(10_000);
        for n in 1..=8u32 {
            assert!(s.should_denoise(n), "first 8 samples denoise every frame");
        }
        assert!(s.should_denoise(16));
        assert!(s.should_denoise(1024));
        assert!(!s.should_denoise(9));
        assert!(!s.should_denoise(1000));

        // The cadence must actually thin out.
        let denoises = (1..=1024u32).filter(|&n| s.should_denoise(n)).count();
        assert!(denoises < 20, "denoised {denoises} times in 1024 samples");
    }

    #[test]
    fn adaptive_sampling_waits_for_the_warmup_and_for_refinement() {
        let mut s = ProgressiveSchedule::default();
        s.update(0);
        assert!(!s.adaptive_sampling_active(64), "never while interacting");
        s.update(10_000);
        assert!(!s.adaptive_sampling_active(15));
        assert!(s.adaptive_sampling_active(16));
    }

    #[test]
    fn tiles_cover_the_image_exactly_once() {
        for (w, h) in [(64usize, 64usize), (100, 37), (1, 1), (17, 33)] {
            let ts = tiles(w, h, None);
            let covered: usize = ts.iter().map(|t| t.pixel_count()).sum();
            assert_eq!(covered, w * h, "{w}x{h} coverage");
            // And no tile may exceed the tile size or leave the image.
            for t in &ts {
                assert!(t.width() <= TILE_SIZE && t.height() <= TILE_SIZE);
                assert!(t.x1 <= w && t.y1 <= h);
                assert!(t.x0 < t.x1 && t.y0 < t.y1);
            }
        }
    }

    /// Region rendering is a filter over the tile queue, not a separate code
    /// path (§9.5).
    #[test]
    fn region_rendering_is_a_tile_filter() {
        let region = Tile {
            x0: 20,
            y0: 10,
            x1: 50,
            y1: 40,
        };
        let ts = tiles(128, 128, Some(region));
        assert!(!ts.is_empty());
        for t in &ts {
            assert!(t.x0 >= region.x0 && t.x1 <= region.x1, "{t:?}");
            assert!(t.y0 >= region.y0 && t.y1 <= region.y1, "{t:?}");
        }
        let covered: usize = ts.iter().map(|t| t.pixel_count()).sum();
        assert_eq!(covered, 30 * 30);
    }

    #[test]
    fn a_region_outside_the_image_produces_no_work() {
        let ts = tiles(
            64,
            64,
            Some(Tile {
                x0: 100,
                y0: 100,
                x1: 200,
                y1: 200,
            }),
        );
        assert!(ts.is_empty());
    }

    /// A cadence the user asked for replaces §9.2's automatic one — and neither
    /// applies while the camera is moving, because a frame during a move is
    /// almost all noise and skipping the denoiser there would make navigation
    /// look broken rather than fast.
    #[test]
    fn a_denoise_cadence_replaces_the_automatic_one() {
        let mut schedule = ProgressiveSchedule::default();
        schedule.update(60_000);

        // Automatic: dense early, then on the powers of two.
        for samples in [1, 2, 8] {
            assert!(schedule.should_denoise_every(samples, 1), "{samples}");
        }
        assert!(!schedule.should_denoise_every(9, 1));
        assert!(schedule.should_denoise_every(16, 1));

        // A fixed interval instead.
        for samples in [4, 8, 12, 400] {
            assert!(schedule.should_denoise_every(samples, 4), "{samples}");
        }
        for samples in [1, 2, 3, 5, 9] {
            assert!(!schedule.should_denoise_every(samples, 4), "{samples}");
        }
        // Zero is not a cadence; it means the same as one rather than dividing
        // by nothing.
        assert_eq!(
            schedule.should_denoise_every(16, 0),
            schedule.should_denoise_every(16, 1)
        );
    }

    /// While the camera moves, every frame is denoised whatever the cadence
    /// says.
    #[test]
    fn navigation_always_denoises() {
        let mut schedule = ProgressiveSchedule::default();
        schedule.interrupt();
        for samples in [1, 3, 7, 100] {
            assert!(schedule.should_denoise_every(samples, 32), "{samples}");
        }
    }
}
