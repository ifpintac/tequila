//! Denoising via Intel Open Image Denoise.
//!
//! `docs/01-RENDERER-CORE.md` §13 lists details it calls non-negotiable, and
//! this module implements each of them:
//!
//! - **Always pass albedo and normal guides.** "Denoising the beauty pass
//!   alone destroys texture detail; with guides it preserves it."
//! - **Prefilter the guides** when the sample count is low, or noise in the
//!   guides propagates into the result.
//! - **HDR mode on scene-linear data.** Never denoise after tone mapping.
//! - **Expose a blend control**, because at high sample counts many people
//!   prefer a little residual grain to the plastic look of full denoising.
//! - **Never write the result back into the accumulation buffer.** Denoising
//!   is applied for display and for output only; accumulating denoised results
//!   would poison convergence.
//!
//! The last one is structural here: [`Denoiser::denoise`] takes the buffers by
//! reference and returns a fresh image, so there is no path by which a
//! denoised value can re-enter accumulation.

#![warn(missing_debug_implementations)]

mod gpu;
mod oidn_sys;

pub use gpu::GpuDenoiser;

/// Either denoiser, for a caller that should not care which device it runs
/// on: the CPU one for frames made on the CPU, the GPU one for frames that
/// came from the graphics card.
pub trait Denoise: Send + Sync {
    fn denoise(
        &self,
        width: usize,
        height: usize,
        color: &[f32],
        albedo: &[f32],
        normal: &[f32],
        settings: DenoiseSettings,
    ) -> Result<DenoisedImage, String>;
}

impl Denoise for Denoiser {
    fn denoise(&self, width: usize, height: usize, color: &[f32], albedo: &[f32], normal: &[f32], settings: DenoiseSettings) -> Result<DenoisedImage, String> {
        Denoiser::denoise(self, width, height, color, albedo, normal, settings)
    }
}

impl Denoise for GpuDenoiser {
    fn denoise(&self, width: usize, height: usize, color: &[f32], albedo: &[f32], normal: &[f32], settings: DenoiseSettings) -> Result<DenoisedImage, String> {
        GpuDenoiser::denoise(self, width, height, color, albedo, normal, settings)
    }
}

use oidn_sys as sys;
use std::ffi::{c_char, CStr, CString};
use std::sync::{Arc, Mutex};

/// How aggressively to denoise.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Quality {
    /// For the interactive viewport, where a frame is due every 16 ms.
    Fast,
    #[default]
    Balanced,
    /// For final frames, where the extra cost does not matter.
    High,
}

impl Quality {
    fn raw(self) -> i32 {
        match self {
            Quality::Fast => sys::OIDN_QUALITY_FAST,
            Quality::Balanced => sys::OIDN_QUALITY_BALANCED,
            Quality::High => sys::OIDN_QUALITY_HIGH,
        }
    }
}

/// Denoiser settings.
#[derive(Clone, Copy, Debug)]
pub struct DenoiseSettings {
    pub quality: Quality,
    /// Blend between the raw image (0) and the fully denoised one (1).
    pub strength: f32,
    /// Run the guide buffers through their own denoiser first. Worth it at
    /// low sample counts, wasted effort at high ones.
    pub prefilter_guides: bool,
}

impl Default for DenoiseSettings {
    fn default() -> Self {
        DenoiseSettings {
            quality: Quality::Balanced,
            strength: 1.0,
            prefilter_guides: true,
        }
    }
}

/// A denoised image: `width * height * 3` scene-linear floats.
#[derive(Clone, Debug)]
pub struct DenoisedImage {
    pub width: usize,
    pub height: usize,
    pub data: Vec<f32>,
}

#[derive(Debug)]
struct RawDevice(sys::OIDNDevice);

// OIDN devices are documented as thread-safe for filter creation and
// execution; the `Mutex` below serializes filter *reuse*, not the device.
unsafe impl Send for RawDevice {}
unsafe impl Sync for RawDevice {}

impl Drop for RawDevice {
    fn drop(&mut self) {
        // SAFETY: released exactly once, from the single owner.
        unsafe { sys::oidnReleaseDevice(self.0) }
    }
}

/// An OIDN device plus its filters.
#[derive(Debug)]
pub struct Denoiser {
    device: Arc<RawDevice>,
    /// Serializes filter execution. OIDN filters hold per-filter scratch
    /// state and must not be executed concurrently with themselves.
    filters: Mutex<Filters>,
}

#[derive(Debug)]
struct Filters {
    beauty: sys::OIDNFilter,
    albedo: sys::OIDNFilter,
    normal: sys::OIDNFilter,
    /// An alpha channel's own filter: no guides, low dynamic range.
    alpha: sys::OIDNFilter,
    /// Dimensions the filters were last configured for, so that a resize
    /// reconfigures rather than silently reading the wrong extents.
    configured: Option<(usize, usize)>,
}

unsafe impl Send for Filters {}

impl Drop for Filters {
    fn drop(&mut self) {
        // SAFETY: each handle came from `oidnNewFilter` and is released once.
        unsafe {
            sys::oidnReleaseFilter(self.beauty);
            sys::oidnReleaseFilter(self.albedo);
            sys::oidnReleaseFilter(self.normal);
            sys::oidnReleaseFilter(self.alpha);
        }
    }
}

static SHARED: std::sync::OnceLock<Result<Arc<Denoiser>, String>> = std::sync::OnceLock::new();

impl Denoiser {
    /// The process-wide denoiser.
    ///
    /// Creating an OIDN device loads and initializes a neural-network weight
    /// set, which costs enough that doing it per frame would show up in the
    /// interactivity budget.
    pub fn shared() -> Result<Arc<Denoiser>, String> {
        SHARED.get_or_init(Denoiser::create).clone()
    }

    fn create() -> Result<Arc<Denoiser>, String> {
        // The CPU device, explicitly.
        //
        // `OIDN_DEVICE_TYPE_DEFAULT` picks the fastest device present, which on
        // a machine with a discrete GPU is CUDA or HIP. Those cannot read the
        // plain host pointers `oidnSetSharedFilterImage` is given — they need
        // buffers allocated through OIDN's own device allocator, and every
        // frame would then pay two PCIe copies. Since the M0 renderer produces
        // its framebuffer on the CPU, keeping the denoiser there too is both
        // simpler and faster.
        //
        // The GPU device becomes worthwhile when the *renderer* moves to the
        // GPU at M9 (`docs/01` §10.3) and the framebuffer already lives in
        // device memory. That change belongs there, with the buffer plumbing.
        // SAFETY: an enumerator OIDN defines; the call cannot fail unsafely.
        let device = unsafe { sys::oidnNewDevice(sys::OIDN_DEVICE_TYPE_CPU) };
        if device.is_null() {
            return Err("OIDN device creation failed; is OpenImageDenoise.dll on the path?".into());
        }
        // SAFETY: `device` is non-null and freshly created.
        unsafe { sys::oidnCommitDevice(device) };
        let device = Arc::new(RawDevice(device));
        check_error(device.0)?;

        let rt = CString::new("RT").unwrap();
        // SAFETY: "RT" is OIDN's ray-tracing filter, present on every build.
        let filters = unsafe {
            Filters {
                beauty: sys::oidnNewFilter(device.0, rt.as_ptr()),
                albedo: sys::oidnNewFilter(device.0, rt.as_ptr()),
                normal: sys::oidnNewFilter(device.0, rt.as_ptr()),
                alpha: sys::oidnNewFilter(device.0, rt.as_ptr()),
                configured: None,
            }
        };
        if filters.beauty.is_null() || filters.alpha.is_null() {
            return Err("OIDN filter creation failed".into());
        }
        check_error(device.0)?;

        Ok(Arc::new(Denoiser {
            device,
            filters: Mutex::new(filters),
        }))
    }

    /// Denoise one frame.
    ///
    /// `color`, `albedo` and `normal` are each `width * height * 3` floats.
    /// `color` must be **scene-linear**, before any display transform.
    pub fn denoise(
        &self,
        width: usize,
        height: usize,
        color: &[f32],
        albedo: &[f32],
        normal: &[f32],
        settings: DenoiseSettings,
    ) -> Result<DenoisedImage, String> {
        let expected = width * height * 3;
        if color.len() != expected || albedo.len() != expected || normal.len() != expected {
            return Err(format!(
                "denoiser buffer size mismatch: expected {expected} floats, \
                 got color {}, albedo {}, normal {}",
                color.len(),
                albedo.len(),
                normal.len()
            ));
        }
        if width == 0 || height == 0 {
            return Ok(DenoisedImage {
                width,
                height,
                data: Vec::new(),
            });
        }

        let mut guard = self.filters.lock().map_err(|_| "denoiser mutex poisoned")?;
        let filters = &mut *guard;

        // OIDN reads and writes through the pointers it is given, so the
        // inputs are copied into buffers this call owns. Handing it `&[f32]`
        // directly would require casting away shared-ness.
        let mut color_buf = color.to_vec();
        let mut albedo_buf = albedo.to_vec();
        let mut normal_buf = normal.to_vec();
        let mut output = vec![0.0f32; expected];

        // SAFETY: every buffer below lives until the end of this function, and
        // every filter is executed before any of them is dropped. The mutex
        // guarantees no other thread is using these filters concurrently.
        unsafe {
            if settings.prefilter_guides {
                // §13: noise in the guides propagates into the result, so at
                // low sample counts the guides get denoised first.
                run_guide_filter(filters.albedo, "albedo", width, height, &mut albedo_buf)?;
                run_guide_filter(filters.normal, "normal", width, height, &mut normal_buf)?;
            }

            let beauty = filters.beauty;
            set_image(beauty, "color", width, height, color_buf.as_mut_ptr());
            set_image(beauty, "albedo", width, height, albedo_buf.as_mut_ptr());
            set_image(beauty, "normal", width, height, normal_buf.as_mut_ptr());
            set_image(beauty, "output", width, height, output.as_mut_ptr());

            let hdr = CString::new("hdr").unwrap();
            // HDR mode: the input is scene-linear radiance with no upper
            // bound, not a display-referred image.
            sys::oidnSetFilterBool(beauty, hdr.as_ptr(), true);

            let clean_aux = CString::new("cleanAux").unwrap();
            // Tell OIDN the guides are already filtered, so it does not filter
            // them a second time.
            sys::oidnSetFilterBool(beauty, clean_aux.as_ptr(), settings.prefilter_guides);

            let quality = CString::new("quality").unwrap();
            sys::oidnSetFilterInt(beauty, quality.as_ptr(), settings.quality.raw());

            sys::oidnCommitFilter(beauty);
            sys::oidnExecuteFilter(beauty);
        }
        check_error(self.device.0)?;
        filters.configured = Some((width, height));
        drop(guard);

        // Blend, per §13's "denoise strength" control.
        let strength = settings.strength.clamp(0.0, 1.0);
        if strength < 1.0 {
            for (out, raw) in output.iter_mut().zip(color) {
                *out = *raw + (*out - *raw) * strength;
            }
        }

        Ok(DenoisedImage {
            width,
            height,
            data: output,
        })
    }

    /// Denoise an alpha channel: how much of a transparent floor a shadow
    /// or a reflection covers, which denoising the colour leaves as noisy as
    /// it was rendered - grain in a cut-out's shadow, specks where a blurred
    /// reflection fades.
    ///
    /// Filtered as a grey picture of its own, without guides: the guides
    /// describe the surface a pixel shows, and a floor's coverage changes
    /// where nothing about the floor does. Where the input is wholly opaque
    /// it stays so - the inside of the product is exactly as rendered - and
    /// a clear pixel stays clear unless something partly covered is beside
    /// it: the filter bled a hard edge's corner a pixel out, which would
    /// ring the product's outline in faint grey. What it leaves below half
    /// a step of an 8-bit file is clear too.
    pub fn denoise_alpha(&self, width: usize, height: usize, alpha: &[f32]) -> Result<Vec<f32>, String> {
        if alpha.len() != width * height {
            return Err(format!("alpha size mismatch: expected {} values, got {}", width * height, alpha.len()));
        }
        // Nothing between clear and opaque: nothing to filter.
        if width == 0 || height == 0 || !alpha.iter().any(|a| *a > 0.0 && *a < 1.0) {
            return Ok(alpha.to_vec());
        }
        let mut grey: Vec<f32> = alpha.iter().flat_map(|a| [a.clamp(0.0, 1.0); 3]).collect();
        let mut output = vec![0.0f32; grey.len()];
        let guard = self.filters.lock().map_err(|_| "denoiser mutex poisoned")?;
        // SAFETY: both buffers live until the end of this function and the
        // filter runs before either is dropped; the mutex keeps the filter
        // to this call.
        unsafe {
            let filter = guard.alpha;
            set_image(filter, "color", width, height, grey.as_mut_ptr());
            set_image(filter, "output", width, height, output.as_mut_ptr());
            // Coverage is 0 to 1, not radiance.
            let hdr = CString::new("hdr").unwrap();
            sys::oidnSetFilterBool(filter, hdr.as_ptr(), false);
            let quality = CString::new("quality").unwrap();
            sys::oidnSetFilterInt(filter, quality.as_ptr(), Quality::High.raw());
            sys::oidnCommitFilter(filter);
            sys::oidnExecuteFilter(filter);
        }
        check_error(self.device.0)?;
        drop(guard);
        let partial = |x: usize, y: usize| {
            let a = alpha[y * width + x];
            a > 0.0 && a < 1.0
        };
        let beside_partial = |i: usize| {
            let (x, y) = (i % width, i / width);
            (y.saturating_sub(1)..(y + 2).min(height)).any(|ny| (x.saturating_sub(1)..(x + 2).min(width)).any(|nx| partial(nx, ny)))
        };
        Ok(alpha
            .iter()
            .zip(output.chunks_exact(3))
            .enumerate()
            .map(|(i, (&raw, grey))| {
                if raw >= 1.0 {
                    return 1.0;
                }
                if raw <= 0.0 && !beside_partial(i) {
                    return 0.0;
                }
                let smooth = ((grey[0] + grey[1] + grey[2]) / 3.0).clamp(0.0, 1.0);
                if smooth < 0.5 / 255.0 {
                    0.0
                } else {
                    smooth
                }
            })
            .collect())
    }
}

/// Denoise a guide buffer in place.
///
/// Bound as what it is, `"albedo"` or `"normal"`, which is how OIDN is told
/// to clean a guide on its own. Bound as `"color"`, as it was, it is read
/// as a low-dynamic-range picture: every negative normal component clamped
/// to zero, and the main filter then told the ruined guides were clean, so
/// edges between surfaces facing away from +X, +Y or +Z blurred.
///
/// # Safety
/// `data` must be `width * height * 3` floats and must outlive the call.
unsafe fn run_guide_filter(
    filter: sys::OIDNFilter,
    guide: &str,
    width: usize,
    height: usize,
    data: &mut [f32],
) -> Result<(), String> {
    set_image(filter, guide, width, height, data.as_mut_ptr());
    set_image(filter, "output", width, height, data.as_mut_ptr());
    sys::oidnCommitFilter(filter);
    sys::oidnExecuteFilter(filter);
    Ok(())
}

/// # Safety
/// `ptr` must point to `width * height * 3` floats that outlive the filter's
/// execution.
unsafe fn set_image(
    filter: sys::OIDNFilter,
    name: &str,
    width: usize,
    height: usize,
    ptr: *mut f32,
) {
    let name = CString::new(name).expect("filter image names contain no interior nul");
    sys::oidnSetSharedFilterImage(
        filter,
        name.as_ptr(),
        ptr.cast(),
        sys::OIDN_FORMAT_FLOAT3,
        width,
        height,
        0,
        // Zero means "compute the strides for me", which is right for tightly
        // packed RGB.
        0,
        0,
    );
}

fn check_error(device: sys::OIDNDevice) -> Result<(), String> {
    let mut message: *const c_char = std::ptr::null();
    // SAFETY: `device` is a valid handle; OIDN writes a borrowed static-ish
    // string pointer into `message` that stays valid until the next call.
    let code = unsafe { sys::oidnGetDeviceError(device, &mut message) };
    if code == sys::OIDN_ERROR_NONE {
        return Ok(());
    }
    let detail = if message.is_null() {
        String::new()
    } else {
        // SAFETY: non-null and nul-terminated per the OIDN contract.
        unsafe { CStr::from_ptr(message) }
            .to_string_lossy()
            .into_owned()
    };
    Err(format!("OIDN error {code}: {detail}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A noisy image of a smooth gradient, plus matching clean guides.
    fn noisy_gradient(width: usize, height: usize) -> (Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>) {
        let mut rng = tequila_core::rng::Pcg32::new(1, 1);
        let mut noisy = Vec::with_capacity(width * height * 3);
        let mut clean = Vec::with_capacity(width * height * 3);
        let mut albedo = Vec::with_capacity(width * height * 3);
        let mut normal = Vec::with_capacity(width * height * 3);
        for y in 0..height {
            for x in 0..width {
                let v = 0.2 + 0.6 * (x as f32 / width as f32);
                // Heavy, zero-mean multiplicative noise, as a 1-spp render has.
                let n = v * (0.2 + 1.6 * rng.next_f32());
                noisy.extend_from_slice(&[n, n, n]);
                clean.extend_from_slice(&[v, v, v]);
                albedo.extend_from_slice(&[v, v, v]);
                normal.extend_from_slice(&[0.0, 0.0, 1.0]);
                let _ = y;
            }
        }
        (noisy, clean, albedo, normal)
    }

    fn rms(a: &[f32], b: &[f32]) -> f32 {
        let sum: f64 = a.iter().zip(b).map(|(x, y)| ((x - y) as f64).powi(2)).sum();
        (sum / a.len() as f64).sqrt() as f32
    }

    /// On a machine with a CUDA card, the GPU denoiser cleans an image as
    /// well as the CPU one, and to nearly the same picture. Skipped without
    /// one.
    #[test]
    fn the_gpu_denoiser_matches_the_cpu_one() {
        let gpu = match GpuDenoiser::shared() {
            Ok(gpu) => gpu,
            Err(why) => {
                eprintln!("skipped: {why}");
                return;
            }
        };
        let cpu = Denoiser::shared().unwrap();
        let (width, height) = (128, 96);
        let (noisy, clean, albedo, normal) = noisy_gradient(width, height);
        for prefilter_guides in [false, true] {
            let settings = DenoiseSettings { prefilter_guides, ..DenoiseSettings::default() };
            let on_cpu = cpu.denoise(width, height, &noisy, &albedo, &normal, settings).unwrap();
            // Twice: the second reuses the buffers the first bound.
            for _ in 0..2 {
                let on_gpu = gpu.denoise(width, height, &noisy, &albedo, &normal, settings).unwrap();
                let before = rms(&noisy, &clean);
                let after = rms(&on_gpu.data, &clean);
                assert!(after < before * 0.5, "the GPU denoiser barely helped: RMS {before} -> {after}");
                let apart = rms(&on_gpu.data, &on_cpu.data);
                assert!(apart < 0.02, "the GPU and CPU denoisers disagree by {apart}");
            }
        }
    }

    #[test]
    fn the_device_comes_up() {
        Denoiser::shared().expect("OIDN device");
    }

    /// The point of the module: the denoised image must be measurably closer
    /// to ground truth than the noisy input.
    /// A transparent render's alpha: a shadow fading across a floor, rendered
    /// with grain, beside an opaque product and clear sky. Denoised, the
    /// shadow is closer to the truth, the product exactly opaque and the
    /// sky exactly clear.
    /// A normal guide facing -X comes out of prefiltering still facing -X.
    #[test]
    fn prefiltering_keeps_normals_that_face_away() {
        let denoiser = Denoiser::shared().unwrap();
        let (width, height) = (32, 32);
        let mut normal: Vec<f32> = (0..width * height).flat_map(|_| [-1.0, 0.0, 0.0]).collect();
        let filters = denoiser.filters.lock().unwrap();
        // SAFETY: `normal` holds width * height * 3 floats and outlives the call.
        unsafe { run_guide_filter(filters.normal, "normal", width, height, &mut normal).unwrap() };
        let mean_x = normal.chunks(3).map(|n| n[0]).sum::<f32>() / (width * height) as f32;
        assert!(mean_x < -0.9, "the guide's -X became {mean_x}");
    }

    #[test]
    fn an_alpha_channel_is_denoised_and_keeps_what_is_solid_and_clear() {
        let denoiser = Denoiser::shared().unwrap();
        let (width, height) = (128, 96);
        let mut seed = 0x2545_f491_u32;
        let mut noise = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as f32 / u32::MAX as f32 - 0.5
        };
        let (mut clean, mut noisy) = (Vec::new(), Vec::new());
        for y in 0..height {
            for x in 0..width {
                let truth = if (40..80).contains(&x) && (10..50).contains(&y) {
                    1.0
                } else if y >= 50 {
                    0.6 * (1.0 - (x as f32 / width as f32))
                } else {
                    0.0
                };
                clean.push(truth);
                noisy.push(if truth > 0.0 && truth < 1.0 { (truth + 0.3 * noise()).clamp(0.0, 1.0) } else { truth });
            }
        }
        let smooth = denoiser.denoise_alpha(width, height, &noisy).unwrap();
        let shadow = |image: &[f32]| -> f32 {
            let errors: Vec<f32> = (50 * width..height * width).map(|i| (image[i] - clean[i]).powi(2)).collect();
            (errors.iter().sum::<f32>() / errors.len() as f32).sqrt()
        };
        assert!(shadow(&smooth) < shadow(&noisy) * 0.5, "the shadow's grain stayed: {} -> {}", shadow(&noisy), shadow(&smooth));
        assert!((10..50).all(|y| (40..80).all(|x| smooth[y * width + x] == 1.0)), "the product is no longer opaque");
        assert!((0..10).all(|y| (0..width).all(|x| smooth[y * width + x] == 0.0)), "the clear sky went hazy");
    }

    #[test]
    fn denoising_reduces_error_against_ground_truth() {
        let denoiser = Denoiser::shared().unwrap();
        let (width, height) = (128, 128);
        let (noisy, clean, albedo, normal) = noisy_gradient(width, height);

        let result = denoiser
            .denoise(
                width,
                height,
                &noisy,
                &albedo,
                &normal,
                DenoiseSettings::default(),
            )
            .unwrap();

        let before = rms(&noisy, &clean);
        let after = rms(&result.data, &clean);
        assert!(
            after < before * 0.5,
            "denoising barely helped: RMS {before} -> {after}"
        );
    }

    /// Strength must blend continuously between the raw and denoised images,
    /// so a user who wants some grain back can have it (§13).
    #[test]
    fn strength_blends_between_raw_and_denoised() {
        let denoiser = Denoiser::shared().unwrap();
        let (width, height) = (64, 64);
        let (noisy, clean, albedo, normal) = noisy_gradient(width, height);

        let at = |strength: f32| {
            denoiser
                .denoise(
                    width,
                    height,
                    &noisy,
                    &albedo,
                    &normal,
                    DenoiseSettings {
                        strength,
                        ..Default::default()
                    },
                )
                .unwrap()
                .data
        };

        let none = at(0.0);
        let half = at(0.5);
        let full = at(1.0);

        // Zero strength must return the input untouched.
        assert!(rms(&none, &noisy) < 1e-6, "strength 0 altered the image");
        // And error must fall monotonically as strength rises.
        let (e_none, e_half, e_full) = (rms(&none, &clean), rms(&half, &clean), rms(&full, &clean));
        assert!(e_half < e_none, "{e_none} -> {e_half}");
        assert!(e_full < e_half, "{e_half} -> {e_full}");
    }

    #[test]
    fn buffer_size_mismatches_are_reported_not_ignored() {
        let denoiser = Denoiser::shared().unwrap();
        let err = denoiser
            .denoise(4, 4, &[0.0; 48], &[0.0; 12], &[0.0; 48], Default::default())
            .unwrap_err();
        assert!(err.contains("size mismatch"), "{err}");
    }

    #[test]
    fn an_empty_image_is_handled_gracefully() {
        let denoiser = Denoiser::shared().unwrap();
        let out = denoiser
            .denoise(0, 0, &[], &[], &[], Default::default())
            .unwrap();
        assert!(out.data.is_empty());
    }

    /// The denoiser is shared across the render orchestrator and the output
    /// writer, so concurrent calls must be safe.
    #[test]
    fn concurrent_denoising_is_safe() {
        let denoiser = Denoiser::shared().unwrap();
        let (width, height) = (48, 48);
        let (noisy, _clean, albedo, normal) = noisy_gradient(width, height);

        let handles: Vec<_> = (0..4)
            .map(|_| {
                let d = denoiser.clone();
                let (n, a, nm) = (noisy.clone(), albedo.clone(), normal.clone());
                std::thread::spawn(move || {
                    for _ in 0..2 {
                        d.denoise(width, height, &n, &a, &nm, Default::default())
                            .unwrap();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
    }

    /// Different resolutions must both work through the same reused filters —
    /// the viewport changes size, and a stale extent would read out of bounds.
    #[test]
    fn filters_are_reconfigured_when_the_resolution_changes() {
        let denoiser = Denoiser::shared().unwrap();
        for (w, h) in [(32usize, 32usize), (64, 48), (32, 32), (96, 32)] {
            let (noisy, _, albedo, normal) = noisy_gradient(w, h);
            let out = denoiser
                .denoise(w, h, &noisy, &albedo, &normal, Default::default())
                .unwrap();
            assert_eq!(out.data.len(), w * h * 3);
            assert!(
                out.data.iter().all(|v| v.is_finite()),
                "{w}x{h} produced NaNs"
            );
        }
    }
}
