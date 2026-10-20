//! Building the renderer's environment from the document's description.
//!
//! The document holds a *description* — a path, a rotation, a contrast — and
//! the renderer needs a built object with a 2D CDF over it. Turning one into
//! the other is not free: `docs/01` §5.2 budgets around 10 ms for a 4K map,
//! and doing it per frame would put that on the interactive path.
//!
//! So this module exists to answer one question well: **what actually needs
//! rebuilding?** §9.3's table draws the line, and it falls between the two
//! kinds of control on the Environment tab:
//!
//! - Rotation, brightness, size, height and flatten change *where the light
//!   comes from* or *how bright it is*. None of them changes the image, so the
//!   CDF built from it is still correct and the built object is reused.
//! - Contrast and the image itself change the function being sampled. A CDF
//!   describing the old one would mean drawing from one distribution and
//!   evaluating another, which is the variance explosion §5.2 warns about, so
//!   these rebuild.
//!
//! Getting that split wrong is invisible either way: too much rebuilding is a
//! slider that stutters, too little is a render that is subtly noisier than it
//! should be.

use crate::document::{BackgroundMode, EnvironmentSettings};
use tequila_render_core::environment::{apply_contrast, Background, Dome, Environment};
use tequila_texture::Image;
use std::sync::Arc;

/// Keeps the built environment across edits, rebuilding only when the image it
/// was built from has changed.
#[derive(Debug, Default)]
pub struct EnvironmentCache {
    /// What the current CDF was built from: the image path (empty for the
    /// built-in studio setup), the contrast baked into it, and the pins
    /// painted over it.
    ///
    /// The pins belong in the key because they change the image, and the CDF
    /// is built from the image. Leaving them out would keep serving a CDF that
    /// does not know a light is there — the pin would appear in reflections
    /// and be invisible to the sampler, which is noise rather than lighting.
    /// What the *base* image was built from — everything except the pins.
    ///
    /// Held apart from the key below because the two cost wildly different
    /// amounts. A sky is fifty milliseconds to generate; painting pins over
    /// one that already exists is a fraction of that. Keyed together, every
    /// drag of a pin's brightness slider regenerated the sky — twice, once
    /// for the lighting image and once for the camera's.
    base_from: Option<(String, f32, Option<tequila_render_core::sky::Sky>)>,
    base: Option<Arc<Image>>,
    /// Whether the base is the built-in studio, whose lights are pins of
    /// its own and are added to the document's.
    base_is_studio: bool,
    /// The studio's surround with its own lights painted in, for lighting.
    /// Cached beside the base because those lights never change: painting
    /// them on every drag of one of the document's pins was most of what a
    /// drag cost.
    studio_lit: Option<Arc<Image>>,
    /// Bumped whenever a new base is installed, and part of the built key:
    /// an environment built over a base that has since been replaced is
    /// rebuilt, even though the settings did not change in between.
    base_version: u64,
    /// Build new bases on a background thread (§9.4), serving the last one
    /// until the new one arrives. Off by default, so a caller that asks for
    /// an environment gets that environment; the app turns it on.
    asynchronous: bool,
    /// The background build in flight, and the base it is building.
    pending: Option<Pending>,
    /// The pins and the base version the built environment was made from.
    /// The base's own settings are not part of it: while a new base is being
    /// built in the background, the old one is still what there is to build
    /// over, and rebuilding over it again would only redo the same work.
    built_from: Option<(Vec<tequila_render_core::pins::Pin>, u64)>,
    built: Option<Arc<Environment>>,
    /// The built environment turned, brightened and given its dome, and the
    /// rotation, brightness and dome it was given. Handed out again while
    /// they stay the same, so a snapshot of an edit that left the
    /// environment alone shares it with the one before - which is how
    /// Cycles' live view tells a dragged part or a resized label from new
    /// lighting, and moves or repaints rather than translating everything.
    finished: Option<((f32, f32, Dome), Arc<Environment>)>,
    /// The decoded backplate, and the path it came from.
    backplate: Option<(String, Option<Arc<Image>>)>,
    /// Image pins' pictures, decoded once each; `None` for one that would
    /// not load, so a missing file is not retried on every drag.
    pin_images: std::collections::HashMap<String, Option<Arc<Image>>>,
}

/// What a base is built from.
type BaseKey = (String, f32, Option<tequila_render_core::sky::Sky>);

/// A base being built on another thread.
#[derive(Debug)]
struct Pending {
    key: BaseKey,
    result: std::sync::mpsc::Receiver<(Image, bool)>,
}

/// Build a base: a sky, a file, or the built-in studio's surround, with its
/// contrast applied. `true` alongside it when it is the studio. A function
/// of its key and nothing else, so it can run on any thread.
fn build_base(key: &BaseKey) -> (Image, bool) {
    let (image, contrast, sky) = key;
    let mut studio = false;
    let raw = match sky {
        // 512 wide: the sun lands in one texel whatever the resolution, so
        // this trades the sharpness of its shadow against the cost of the
        // build rather than against its brightness.
        Some(sky) => sky.to_image(512, 256),
        None => match load_image((!image.is_empty()).then_some(image.as_str())) {
            Some(image) => image,
            None => {
                studio = true;
                crate::demo::studio_surround()
            }
        },
    };
    (apply_contrast(&raw, *contrast), studio)
}

impl EnvironmentCache {
    pub fn new() -> EnvironmentCache {
        EnvironmentCache::default()
    }

    /// Build new skies and loaded images in the background (`docs/01` §9.4),
    /// serving the environment as it was until they arrive. A sky takes
    /// tens of milliseconds to integrate and a 4K image longer to decode;
    /// done on the UI thread, every step of a sky slider is a stall.
    pub fn set_asynchronous(&mut self, asynchronous: bool) {
        self.asynchronous = asynchronous;
    }

    /// Whether a background build is still running: the caller should look
    /// again soon, and rebuild what it built from this cache when
    /// [`EnvironmentCache::poll`] says the new base has arrived.
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Collect a finished background build. `true` when a new base was
    /// installed, so what was built from the old one is out of date.
    pub fn poll(&mut self) -> bool {
        let Some(pending) = &self.pending else {
            return false;
        };
        match pending.result.try_recv() {
            Ok((image, studio)) => {
                let key = pending.key.clone();
                self.pending = None;
                self.install(key, image, studio);
                true
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.pending = None;
                false
            }
        }
    }

    fn install(&mut self, key: BaseKey, image: Image, studio: bool) {
        self.base = Some(Arc::new(image));
        self.base_from = Some(key);
        self.base_is_studio = studio;
        self.studio_lit = None;
        self.base_version += 1;
    }

    /// Forget everything, so the next call rebuilds.
    pub fn clear(&mut self) {
        self.base_from = None;
        self.base = None;
        self.studio_lit = None;
        self.pending = None;
        self.built_from = None;
        self.built = None;
        self.backplate = None;
        self.pin_images.clear();
    }

    /// The environment before pins, with its contrast applied.
    ///
    /// A sky if there is one, else a file, else the built-in studio — and the
    /// same answer for the lighting image and the camera's, which is the point
    /// of having it in one place.
    fn base(&mut self, settings: &EnvironmentSettings) -> Arc<Image> {
        let key: BaseKey = (
            settings.image.clone().unwrap_or_default(),
            settings.contrast,
            settings.sky,
        );
        self.poll();
        if self.base_from.as_ref() == Some(&key) {
            if let Some(base) = &self.base {
                return base.clone();
            }
        }
        // Something to show while the new one is built: start it, unless it
        // is already being built, and show the old one meanwhile. A build for
        // settings that have since moved on is abandoned — its result arrives
        // into a receiver nobody holds.
        if self.asynchronous && self.base.is_some() {
            if self.pending.as_ref().map(|p| &p.key) != Some(&key) {
                let (send, result) = std::sync::mpsc::channel();
                let job = key.clone();
                let spawned = std::thread::Builder::new()
                    .name("tequila-environment".into())
                    .spawn(move || {
                        let _ = send.send(build_base(&job));
                    });
                if spawned.is_ok() {
                    self.pending = Some(Pending { key, result });
                } else {
                    // No thread to be had: build it here after all.
                    let (image, studio) = build_base(&key);
                    self.install(key, image, studio);
                }
            }
            return self.base.clone().expect("checked above");
        }
        // Nothing yet, or asked to answer now: build it here.
        let (image, studio) = build_base(&key);
        self.pending = None;
        self.install(key, image, studio);
        self.base.clone().expect("just built")
    }

    /// The environment these settings describe.
    pub fn environment(&mut self, settings: &EnvironmentSettings) -> Arc<Environment> {
        // Asked first, so a background build that has finished is installed
        // before deciding whether anything needs rebuilding.
        let base = self.base(settings);
        let key = (settings.pins.clone(), self.base_version);
        let needs_build = self.built_from.as_ref() != Some(&key) || self.built.is_none();
        if needs_build {
            let images = self.pin_images(&settings.pins);
            // The built-in studio's own lights light and reflect but stay out
            // of the backdrop; they are painted into a lighting base of their
            // own, once, and the document's pins go over that.
            let lighting_base = if self.base_is_studio {
                self.studio_lit
                    .get_or_insert_with(|| {
                        Arc::new(tequila_render_core::pins::composite(&base, &crate::demo::studio_lights()))
                    })
                    .clone()
            } else {
                base.clone()
            };
            let pins = &settings.pins;
            // Painted on before the CDF is built, which is the order §1.5
            // requires — the sampler has to see the pins.
            let image = if pins.iter().any(|pin| pin.enabled && pin.in_lighting) {
                Arc::new(tequila_render_core::pins::composite_with(
                    &lighting_base,
                    pins,
                    &images,
                    |pin| pin.in_lighting,
                ))
            } else {
                lighting_base.clone()
            };
            let mut environment = Environment::new(image, 1.0);
            // A second image for the camera, built only when what it sees
            // differs from what lights: a pin kept out of the backdrop, or one
            // that is only backdrop. It carries no CDF of its own: nothing
            // samples it, it is only looked up along rays the camera can
            // already see.
            if self.base_is_studio
                || pins
                    .iter()
                    .any(|pin| pin.enabled && pin.in_lighting != pin.show_in_background)
            {
                environment.set_camera_image(Some(Arc::new(
                    tequila_render_core::pins::composite_with(&base, pins, &images, |pin| {
                        pin.show_in_background
                    }),
                )));
            }
            self.built = Some(Arc::new(environment));
            self.built_from = Some(key);
            self.finished = None;
        }

        // The cheap settings are applied to a *copy* of the built object. The
        // copy shares the image and clones the CDF vectors, which is a memcpy
        // rather than an integration — and it has to be a copy, because the
        // renderer holds the previous snapshot while this one is built.
        let dome = Dome {
            radius: (settings.size > 0.0).then_some(settings.size * 0.5),
            height: settings.height,
            flatten: settings.ground.flatten,
        };
        let finish = (settings.rotation, settings.brightness, dome);
        if let Some((key, finished)) = &self.finished {
            if *key == finish {
                return finished.clone();
            }
        }
        let mut environment = (**self.built.as_ref().expect("just built")).clone();
        environment.set_rotation_y(settings.rotation);
        environment.set_intensity(settings.brightness);
        environment.set_dome(dome);
        let environment = Arc::new(environment);
        self.finished = Some((finish, environment.clone()));
        environment
    }

    /// The pictures these pins show, decoding any not seen before.
    fn pin_images(
        &mut self,
        pins: &[tequila_render_core::pins::Pin],
    ) -> tequila_render_core::pins::PinImages {
        use tequila_render_core::pins::PinKind;
        let mut out = tequila_render_core::pins::PinImages::new();
        for pin in pins {
            let PinKind::Image { path } = &pin.kind else {
                continue;
            };
            let loaded = self
                .pin_images
                .entry(path.clone())
                .or_insert_with(|| load_image(Some(path.as_str())).map(Arc::new));
            if let Some(image) = loaded {
                out.insert(path.clone(), image.clone());
            }
        }
        out
    }

    /// Image pins whose picture would not load, so the panel can say so
    /// rather than leaving an empty patch of sky to be puzzled over.
    pub fn missing_pin_images(&self) -> Vec<String> {
        let mut missing: Vec<String> = self
            .pin_images
            .iter()
            .filter(|(_, image)| image.is_none())
            .map(|(path, _)| path.clone())
            .collect();
        missing.sort();
        missing
    }

    /// The composed environment as a plain equirectangular image, `width`
    /// texels around, for writing out as an HDRI (`docs/03` §5: "export to
    /// plain .hdr/.exr by rasterizing the composite at a chosen resolution").
    ///
    /// The lighting image — everything that lights the scene and shows in
    /// reflections — or, with `background`, what the camera sees behind the
    /// product. Contrast and brightness are baked in, because another
    /// application loading the file has neither; rotation is not, because an
    /// HDRI is turned where it is used.
    ///
    /// Built in the foreground at the resolution asked for: a sky is
    /// integrated afresh, and a file or the studio is resampled before the
    /// pins are painted, so the pins are as sharp as the output.
    pub fn rasterize(&mut self, settings: &EnvironmentSettings, width: usize, background: bool) -> Image {
        let width = width.clamp(16, 16384);
        let height = (width / 2).max(8);
        let (image, contrast, sky) = (
            settings.image.clone().unwrap_or_default(),
            settings.contrast,
            settings.sky,
        );
        let (base, studio) = match sky {
            Some(sky) => (apply_contrast(&sky.to_image(width, height), contrast), false),
            None => {
                let (source, studio) = build_base(&(image, contrast, None));
                (resample(&source, width, height), studio)
            }
        };
        let images = self.pin_images(&settings.pins);
        let mut pins = Vec::new();
        if studio && !background {
            pins.extend(crate::demo::studio_lights());
        }
        pins.extend(settings.pins.iter().cloned());
        let mut out = tequila_render_core::pins::composite_with(&base, &pins, &images, |pin| {
            if background {
                pin.show_in_background
            } else {
                pin.in_lighting
            }
        });
        if settings.brightness != 1.0 {
            for value in &mut out.data {
                *value *= settings.brightness;
            }
        }
        out
    }

    /// What the camera sees where the scene is empty.
    pub fn background(&mut self, settings: &EnvironmentSettings) -> Background {
        match settings.background {
            BackgroundMode::Environment => Background::Environment,
            BackgroundMode::Color => Background::Color(settings.background_color),
            BackgroundMode::Backplate => {
                let path = settings.backplate.clone().unwrap_or_default();
                if self.backplate.as_ref().map(|(p, _)| p.as_str()) != Some(path.as_str()) {
                    let loaded = load_image(Some(path.as_str())).map(Arc::new);
                    self.backplate = Some((path, loaded));
                }
                match self.backplate.as_ref().and_then(|(_, image)| image.clone()) {
                    Some(image) => Background::Backplate { image },
                    // A backplate that will not load falls back to showing the
                    // environment rather than to black. Black would look like
                    // a deliberate choice and send the user hunting through
                    // the lighting for a fault that is really a missing file.
                    None => Background::Environment,
                }
            }
        }
    }
}

/// Bilinear resampling of an equirectangular image to a new size, wrapping
/// round the horizon.
fn resample(image: &Image, width: usize, height: usize) -> Image {
    if image.width == width && image.height == height {
        return image.clone();
    }
    let mut data = Vec::with_capacity(width * height * 3);
    for y in 0..height {
        let v = (y as f32 + 0.5) / height as f32;
        for x in 0..width {
            let u = (x as f32 + 0.5) / width as f32;
            let c = image.sample_bilinear_wrap_u(u, v);
            data.extend_from_slice(&[c.x, c.y, c.z]);
        }
    }
    Image {
        width,
        height,
        channels: 3,
        data,
        color_space: image.color_space,
    }
}

/// An environment or backplate picture, in the rendering space.
///
/// HDRIs and photographs are linear sRGB once decoded — Rec.709 primaries —
/// and the renderer lights and displays in ACEScg. Used as they stood, a
/// coloured sky lit the product more saturated than it is, and a backplate
/// photograph came out oversaturated behind it. Converted here, once, where
/// the picture enters the scene: both views then read it as what it is.
fn load_image(path: Option<&str>) -> Option<Image> {
    let path = path?;
    if path.is_empty() {
        return None;
    }
    let mut image = Image::load(path).ok()?;
    to_rendering_space(&mut image);
    Some(image)
}

/// Rec.709 primaries to ACEScg, in place, leaving alpha and data alone.
fn to_rendering_space(image: &mut Image) {
    if image.color_space == tequila_texture::image::ColorSpace::Raw || image.channels < 3 {
        return;
    }
    for pixel in image.data.chunks_exact_mut(image.channels) {
        let v = tequila_core::spectrum::srgb_to_acescg(tequila_core::math::vec3(pixel[0], pixel[1], pixel[2]));
        pixel[0] = v.x;
        pixel[1] = v.y;
        pixel[2] = v.z;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An HDRI or backplate enters the scene in the rendering space: its
    /// sRGB primaries converted to ACEScg, as the sky's and every picked
    /// colour are. Left as they were, a coloured sky lit the product more
    /// saturated than it is.
    #[test]
    fn an_environment_picture_is_converted_to_the_rendering_space() {
        let path = std::env::temp_dir().join(format!("tequila-environment-colour-{}.png", std::process::id()));
        let file = std::fs::File::create(&path).unwrap();
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), 1, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&[200, 150, 90]).unwrap();
        let raw = Image::load(&path).unwrap().texel(0, 0);
        let loaded = load_image(path.to_str()).unwrap().texel(0, 0);
        let expected = tequila_core::spectrum::srgb_to_acescg(raw);
        assert!((loaded - expected).length() < 1e-5, "{loaded:?}, not {expected:?}");
        assert!((loaded - raw).length() > 1e-3, "left in sRGB primaries");
        let _ = std::fs::remove_file(&path);
    }

    /// The studio's lights light the product and stay out of the backdrop:
    /// a softbox standing behind a packshot is a mistake in the photograph.
    /// The first build baked them into the one image, and a strip light
    /// stood in the background of every render like a white pillar.
    #[test]
    fn the_studio_lights_but_is_not_seen_behind_the_product() {
        use tequila_core::math::vec3;
        let environment = EnvironmentCache::new().environment(&EnvironmentSettings::default());
        let mut lit_max: f32 = 0.0;
        let mut seen_max: f32 = 0.0;
        for y in 0..64 {
            for x in 0..128 {
                let d = tequila_render_core::pins::equirect_direction((x as f32 + 0.5) / 128.0, (y as f32 + 0.5) / 64.0);
                lit_max = lit_max.max(environment.radiance(d).luminance());
                seen_max = seen_max.max(environment.background_radiance_from(vec3(0.0, 0.0, 0.0), d).luminance());
            }
        }
        assert!(lit_max > 1.0, "the studio has no lights to reflect: brightest {lit_max}");
        // The surround tops out under a half at the ceiling; a light is
        // several times that, so one that showed would be the brightest thing
        // in the backdrop by far.
        assert!(seen_max < 0.5, "a light shows in the backdrop: brightest {seen_max}");
        assert!(lit_max > 4.0 * seen_max, "the lights are barely brighter than the walls: {lit_max} against {seen_max}");
    }

    /// A white product reads white from the front, on a light grey floor.
    ///
    /// The studio was first tuned to match the one it replaced, which lit
    /// from overhead: a white face turned to the camera came back at 0.19 —
    /// mid-grey once developed — on a floor at 0.045 that rendered almost
    /// black, and every white plastic product looked grey and gloomy. The
    /// Solid view, with its own light backdrop, looked nothing like it.
    #[test]
    fn a_white_product_reads_white_on_a_light_floor() {
        use tequila_core::math::vec3;
        let environment = EnvironmentCache::new().environment(&EnvironmentSettings::default());
        let (w, h) = (256, 128);
        let white_facing = |n: tequila_core::math::Vec3| {
            let mut sum = 0.0f32;
            for y in 0..h {
                for x in 0..w {
                    let (u, v) = ((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
                    let d = tequila_render_core::pins::equirect_direction(u, v);
                    let cos = d.dot(n);
                    if cos > 0.0 {
                        let dw = (std::f32::consts::TAU / w as f32) * (std::f32::consts::PI / h as f32) * (v * std::f32::consts::PI).sin();
                        sum += environment.radiance(d).luminance() * cos * dw;
                    }
                }
            }
            // Irradiance over pi: what a white matte surface sends back.
            sum / std::f32::consts::PI
        };
        let front = white_facing(vec3(0.0, 0.0, 1.0));
        assert!(front > 0.45, "a white face turned to the camera reads {front}");
        let floor = environment
            .background_radiance_from(tequila_core::math::Vec3::ZERO, vec3(0.0, -0.35, -1.0).normalize())
            .luminance();
        assert!(floor > 0.2, "the floor behind the product is {floor}, nearly black");
    }

    /// Every side of a product gets light. The first studio gave a face
    /// turned towards a typical camera a tenth of what a top face got, and
    /// side-lit parts rendered dim and flat.
    #[test]
    fn the_studio_lights_every_side() {
        use tequila_core::math::vec3;
        let environment = EnvironmentCache::new().environment(&EnvironmentSettings::default());
        let irradiance = |n: tequila_core::math::Vec3| {
            let (w, h) = (256, 128);
            let mut sum = 0.0f32;
            for y in 0..h {
                for x in 0..w {
                    let (u, v) = ((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
                    let d = tequila_render_core::pins::equirect_direction(u, v);
                    let cos = d.dot(n);
                    if cos > 0.0 {
                        let dw = (std::f32::consts::TAU / w as f32) * (std::f32::consts::PI / h as f32) * (v * std::f32::consts::PI).sin();
                        sum += environment.radiance(d).luminance() * cos * dw;
                    }
                }
            }
            sum
        };
        let top = irradiance(vec3(0.0, 1.0, 0.0));
        for side in [vec3(1.0, 0.0, 0.0), vec3(-1.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0), vec3(0.0, 0.0, -1.0)] {
            let lit = irradiance(side);
            assert!(lit > top * 0.2, "a side facing {side:?} gets {lit} against the top's {top}");
        }
    }

    /// Rotation is the control a user drags, and §5.2 is explicit that it must
    /// not rebuild anything. The cache is what guarantees that, so the
    /// guarantee is worth asserting rather than assuming.
    #[test]
    fn rotating_does_not_rebuild_the_distribution() {
        let mut cache = EnvironmentCache::new();
        let mut settings = EnvironmentSettings::default();
        let first = cache.environment(&settings);
        let built = Arc::as_ptr(cache.built.as_ref().unwrap());

        settings.rotation = 1.2;
        let second = cache.environment(&settings);
        assert!(
            std::ptr::eq(built, Arc::as_ptr(cache.built.as_ref().unwrap())),
            "rotation rebuilt the CDF"
        );
        // And it did change something, or the test proves only that nothing
        // happened at all. Swept, because a rotation about the vertical axis
        // leaves every direction's elevation alone: a single probe can easily
        // land on two points of the same brightness and prove nothing.
        let moved = (0..64).any(|i| {
            let phi = i as f32 / 64.0 * std::f32::consts::TAU;
            let direction =
                tequila_core::math::vec3(phi.cos(), 0.4, phi.sin()).normalize();
            first.radiance(direction) != second.radiance(direction)
        });
        assert!(moved, "rotation changed nothing");
    }

    /// Contrast changes the function being sampled, so it must rebuild. A
    /// cache that held on here would leave the sampler drawing from the old
    /// image while the integrator evaluated the new one.
    #[test]
    fn contrast_rebuilds_the_distribution() {
        let mut cache = EnvironmentCache::new();
        let mut settings = EnvironmentSettings::default();
        cache.environment(&settings);
        let built = Arc::as_ptr(cache.built.as_ref().unwrap());

        settings.contrast = 1.6;
        cache.environment(&settings);
        assert!(
            !std::ptr::eq(built, Arc::as_ptr(cache.built.as_ref().unwrap())),
            "contrast reused a CDF built from a different image"
        );
    }

    /// Brightness scales the result rather than the image, which is what keeps
    /// it off the rebuild path — and it still has to actually scale.
    #[test]
    fn brightness_scales_without_rebuilding() {
        let mut cache = EnvironmentCache::new();
        let mut settings = EnvironmentSettings::default();
        let plain = cache.environment(&settings);
        let built = Arc::as_ptr(cache.built.as_ref().unwrap());

        settings.brightness = 3.0;
        let brighter = cache.environment(&settings);
        assert!(std::ptr::eq(built, Arc::as_ptr(cache.built.as_ref().unwrap())));

        let direction = tequila_core::math::vec3(0.2, 0.9, 0.1).normalize();
        let a = plain.radiance(direction).luminance();
        let b = brighter.radiance(direction).luminance();
        assert!((b / a.max(1e-6) - 3.0).abs() < 1e-3, "{a} -> {b}");
    }

    /// A backplate that cannot be loaded shows the environment rather than a
    /// black frame, which would read as a lighting fault.
    #[test]
    fn a_missing_backplate_falls_back_to_the_environment() {
        let mut cache = EnvironmentCache::new();
        let settings = EnvironmentSettings {
            background: BackgroundMode::Backplate,
            backplate: Some("no/such/plate.jpg".into()),
            ..Default::default()
        };
        assert!(matches!(
            cache.background(&settings),
            Background::Environment
        ));
    }

    /// Flattening the ground is a property of the dome, not of the image, so
    /// it must not rebuild either — and it has to reach the dome.
    /// Moving a pin must not regenerate the sky.
    ///
    /// Asserted by identity rather than by a stopwatch: the base image is
    /// shared, so if the sky were rebuilt the `Arc` would be a different
    /// allocation. A timing test would say the same thing less reliably and
    /// fail on a loaded machine.
    ///
    /// Worth holding down because the two were keyed together at first, and a
    /// pin's brightness slider regenerated the sky twice per frame — once for
    /// the lighting image and once for the camera's.
    #[test]
    fn dragging_a_pin_does_not_rebuild_the_sky() {
        let mut cache = EnvironmentCache::new();
        let mut settings = EnvironmentSettings {
            sky: Some(tequila_render_core::sky::Sky::default()),
            pins: vec![tequila_render_core::pins::Pin {
                intensity: 5.0,
                ..Default::default()
            }],
            ..Default::default()
        };

        let _ = cache.environment(&settings);
        let first = cache.base.clone().expect("a base was built");

        settings.pins[0].intensity = 25.0;
        let _ = cache.environment(&settings);
        let after_pin = cache.base.clone().expect("still a base");
        assert!(
            Arc::ptr_eq(&first, &after_pin),
            "changing a pin threw away the sky and generated another"
        );

        // ...and the sky's own settings do rebuild it, or the cache would be
        // serving a stale sky instead.
        settings.sky = Some(tequila_render_core::sky::Sky {
            turbidity: 9.0,
            ..Default::default()
        });
        let _ = cache.environment(&settings);
        let after_sky = cache.base.clone().expect("still a base");
        assert!(
            !Arc::ptr_eq(&first, &after_sky),
            "changing the sky did not rebuild it"
        );
    }

    /// Wait for a background build, polling as the app does once a frame.
    fn settle(cache: &mut EnvironmentCache) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while cache.is_pending() {
            if cache.poll() {
                return true;
            }
            assert!(std::time::Instant::now() < deadline, "the background build never finished");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        false
    }

    /// With builds in the background, a sky change answers at once with the
    /// environment as it was, and the new one replaces it when it lands.
    ///
    /// The first build is still made on the spot — there is nothing older to
    /// show — and the rebuilt environment has to follow the new base even
    /// though the settings did not change between the two asks.
    #[test]
    fn a_new_sky_is_built_in_the_background() {
        let mut cache = EnvironmentCache::new();
        cache.set_asynchronous(true);
        let mut settings = EnvironmentSettings {
            sky: Some(tequila_render_core::sky::Sky::default()),
            ..Default::default()
        };

        let _ = cache.environment(&settings);
        assert!(!cache.is_pending(), "the first environment was not built on the spot");
        let first = cache.built.clone().expect("built");

        settings.sky = Some(tequila_render_core::sky::Sky {
            turbidity: 9.0,
            ..Default::default()
        });
        let _ = cache.environment(&settings);
        assert!(cache.is_pending(), "the new sky was built on the calling thread");
        let meanwhile = cache.built.clone().expect("built");
        assert!(
            Arc::ptr_eq(&first, &meanwhile),
            "while the new sky was being built, the old environment was not served"
        );

        assert!(settle(&mut cache), "the finished build was never installed");
        let rebuilt = cache.environment(&settings);
        let built = cache.built.clone().expect("built");
        assert!(!Arc::ptr_eq(&first, &built), "the new sky never reached the environment");

        // The same as building it in the foreground.
        let mut direct = EnvironmentCache::new();
        let expected = direct.environment(&settings);
        assert_eq!(
            rebuilt.radiance(tequila_core::math::Vec3::Y),
            expected.radiance(tequila_core::math::Vec3::Y),
            "the background build differs from the foreground one"
        );
    }

    /// Settings that move on while a build is running abandon it: only the
    /// latest settings are ever installed, however the builds finish.
    #[test]
    fn a_superseded_background_build_is_not_installed() {
        let mut cache = EnvironmentCache::new();
        cache.set_asynchronous(true);
        let mut settings = EnvironmentSettings {
            sky: Some(tequila_render_core::sky::Sky::default()),
            ..Default::default()
        };
        let _ = cache.environment(&settings);

        for turbidity in [3.0, 5.0, 7.0] {
            settings.sky = Some(tequila_render_core::sky::Sky {
                turbidity,
                ..Default::default()
            });
            let _ = cache.environment(&settings);
        }
        settle(&mut cache);
        let key = cache.base_from.clone().expect("a base");
        assert_eq!(key.2.map(|sky| sky.turbidity), Some(7.0), "a superseded sky was installed");
    }

    /// A backdrop gradient changes what the camera sees behind the product
    /// and nothing about how it is lit — `docs/03` §5's "BackgroundOnly",
    /// the separation that makes gradient pins useful.
    #[test]
    fn a_backdrop_gradient_is_seen_and_does_not_light() {
        use tequila_core::spectrum::Spectrum;
        let mut cache = EnvironmentCache::new();
        let mut settings = EnvironmentSettings {
            sky: Some(tequila_render_core::sky::Sky::default()),
            ..Default::default()
        };
        let plain = cache.environment(&settings);

        let top = Spectrum { r: 0.2, g: 0.3, b: 0.5 };
        settings.pins = vec![tequila_render_core::pins::Pin::backdrop_gradient(Spectrum::splat(0.02), top)];
        let backdrop = cache.environment(&settings);

        let up = tequila_core::math::Vec3::Y;
        assert_eq!(
            plain.radiance(up),
            backdrop.radiance(up),
            "the backdrop changed the light the scene is lit by"
        );
        let seen = backdrop.background_radiance_from(tequila_core::math::Vec3::ZERO, up);
        // The pin paints its colour into the environment as it is, and the
        // camera sees it as it is: one colour, not the colour converted twice.
        assert!(
            (seen - top).max_component().abs() < 0.02 && (top - seen).max_component() < 0.02,
            "the camera sees {seen:?} overhead, not the gradient's {top:?}"
        );
    }

    /// An image pin's picture is read from disk, once, and a picture that
    /// will not load is reported rather than painted as nothing in silence.
    #[test]
    fn an_image_pin_reads_its_picture_and_reports_a_missing_one() {
        use tequila_core::spectrum::Spectrum;
        let directory = std::env::temp_dir().join(format!("tequila-pin-image-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("window.hdr");
        let white = Image::constant(4, 4, tequila_core::math::vec3(3.0, 3.0, 3.0));
        white.save_radiance(&path).expect("writes");

        let mut cache = EnvironmentCache::new();
        let mut pin = tequila_render_core::pins::Pin::image(
            "Window",
            path.to_str().unwrap(),
            tequila_core::math::Vec3::Y,
            0.8,
        );
        pin.show_in_background = false;
        let settings = EnvironmentSettings {
            sky: Some(tequila_render_core::sky::Sky::default()),
            pins: vec![pin.clone()],
            ..Default::default()
        };
        let lit = cache.environment(&settings);
        let overhead = lit.radiance(tequila_core::math::Vec3::Y);
        assert!(
            (overhead - Spectrum::splat(3.0)).max_component().abs() < 0.05,
            "overhead is {overhead:?}, not the picture"
        );
        assert!(cache.missing_pin_images().is_empty());

        let mut gone = settings.clone();
        gone.pins[0].kind = tequila_render_core::pins::PinKind::Image {
            path: directory.join("nowhere.png").to_string_lossy().into_owned(),
        };
        let _ = cache.environment(&gone);
        assert_eq!(cache.missing_pin_images().len(), 1, "the missing picture was not reported");
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// The export is the composed environment at the size asked for, with
    /// brightness baked in, and backdrop-only pins kept to the background
    /// export.
    #[test]
    fn the_environment_rasterizes_for_export() {
        use tequila_core::spectrum::Spectrum;
        let mut cache = EnvironmentCache::new();
        let key = tequila_render_core::pins::Pin {
            direction: tequila_core::math::Vec3::Y,
            intensity: 20.0,
            softness: 0.0,
            angular_size: tequila_core::math::vec2(0.6, 0.6),
            ..Default::default()
        };
        let mut settings = EnvironmentSettings {
            sky: Some(tequila_render_core::sky::Sky::default()),
            pins: vec![
                key,
                tequila_render_core::pins::Pin::backdrop_gradient(Spectrum::splat(0.5), Spectrum::splat(0.5)),
            ],
            ..Default::default()
        };
        let lighting = cache.rasterize(&settings, 256, false);
        assert_eq!((lighting.width, lighting.height), (256, 128));
        // The top row is the zenith, where the key light is.
        let top = lighting.texel(10, 0);
        assert!(top.y > 19.0, "the key light is missing from the export: {top:?}");
        // No backdrop in the lighting export: the horizon is still sky.
        let horizon = lighting.texel(10, 64);
        assert!((horizon.y - 0.5).abs() > 1e-3, "the backdrop-only gradient leaked into the lighting");

        let background = cache.rasterize(&settings, 256, true);
        let horizon = background.texel(10, 64);
        assert!((horizon.y - 0.5).abs() < 1e-3, "the background export lacks the backdrop: {horizon:?}");

        settings.brightness = 2.0;
        let brighter = cache.rasterize(&settings, 256, false);
        assert!((brighter.texel(10, 0).y - 2.0 * top.y).abs() < 1e-3 * top.y, "brightness was not baked in");
    }

    #[test]
    fn flattening_the_ground_reaches_the_dome() {
        let mut cache = EnvironmentCache::new();
        let mut settings = EnvironmentSettings {
            size: 100.0,
            ..Default::default()
        };
        let plain = cache.environment(&settings);
        assert!(!plain.dome().flatten);
        assert_eq!(plain.dome().radius, Some(50.0));

        settings.ground.flatten = true;
        let flattened = cache.environment(&settings);
        assert!(flattened.dome().flatten);
    }
}
