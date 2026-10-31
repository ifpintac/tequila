//! Denoising on the graphics card, for a renderer whose frames come from it.
//!
//! The CPU denoiser hands OIDN plain host pointers, which a GPU device cannot
//! read; this one keeps OIDN buffers on the card instead, allocated and bound
//! to the filters once per image size. A frame then costs three uploads, the
//! filter and one download — on an RTX 2060 a few tens of milliseconds for a
//! viewport-sized image, where the CPU took well over a second.

use crate::oidn_sys as sys;
use crate::{check_error, DenoiseSettings, DenoisedImage, RawDevice};
use std::ffi::CString;
use std::sync::{Arc, Mutex, OnceLock};

/// Buffers and filters bound to one image size and configuration.
struct Bound {
    width: usize,
    height: usize,
    quality: i32,
    prefilter: bool,
    color: sys::OIDNBuffer,
    albedo: sys::OIDNBuffer,
    normal: sys::OIDNBuffer,
    output: sys::OIDNBuffer,
}

impl Drop for Bound {
    fn drop(&mut self) {
        // SAFETY: each came from `oidnNewBuffer` and is released once.
        unsafe {
            sys::oidnReleaseBuffer(self.color);
            sys::oidnReleaseBuffer(self.albedo);
            sys::oidnReleaseBuffer(self.normal);
            sys::oidnReleaseBuffer(self.output);
        }
    }
}

struct State {
    beauty: sys::OIDNFilter,
    albedo: sys::OIDNFilter,
    normal: sys::OIDNFilter,
    bound: Option<Bound>,
}

impl Drop for State {
    fn drop(&mut self) {
        self.bound = None;
        // SAFETY: each handle came from `oidnNewFilter` and is released once.
        unsafe {
            sys::oidnReleaseFilter(self.beauty);
            sys::oidnReleaseFilter(self.albedo);
            sys::oidnReleaseFilter(self.normal);
        }
    }
}

// The handles are only touched under the mutex.
unsafe impl Send for State {}

/// An OIDN device on the graphics card, with its filters.
pub struct GpuDenoiser {
    device: Arc<RawDevice>,
    state: Mutex<State>,
}

// SAFETY: the device is used through the filters, which the mutex serializes.
unsafe impl Sync for GpuDenoiser {}
unsafe impl Send for GpuDenoiser {}

impl std::fmt::Debug for GpuDenoiser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuDenoiser").finish_non_exhaustive()
    }
}

impl GpuDenoiser {
    /// The shared one, made on first use, or why there is none (no CUDA card,
    /// or on a Mac no Metal device, or OIDN built without it).
    pub fn shared() -> Result<Arc<GpuDenoiser>, String> {
        static SHARED: OnceLock<Result<Arc<GpuDenoiser>, String>> = OnceLock::new();
        SHARED.get_or_init(GpuDenoiser::create).clone()
    }

    fn create() -> Result<Arc<GpuDenoiser>, String> {
        // The graphics card's own kind: Metal on a Mac (with
        // libOpenImageDenoise_device_metal beside the library), CUDA on an
        // NVIDIA card elsewhere, HIP on an AMD one.
        let kinds: &[(i32, &str)] = if cfg!(target_os = "macos") {
            &[(sys::OIDN_DEVICE_TYPE_METAL, "Metal")]
        } else {
            &[(sys::OIDN_DEVICE_TYPE_CUDA, "CUDA"), (sys::OIDN_DEVICE_TYPE_HIP, "HIP")]
        };
        // SAFETY: enumerators OIDN defines; the call cannot fail unsafely.
        let Some(device) = kinds.iter().map(|(kind, _)| unsafe { sys::oidnNewDevice(*kind) }).find(|d| !d.is_null()) else {
            let names: Vec<&str> = kinds.iter().map(|(_, name)| *name).collect();
            return Err(format!("no {} denoising device", names.join(" or ")));
        };
        // SAFETY: non-null and freshly created.
        unsafe { sys::oidnCommitDevice(device) };
        let device = Arc::new(RawDevice(device));
        check_error(device.0)?;
        let rt = CString::new("RT").unwrap();
        // SAFETY: "RT" is OIDN's ray-tracing filter, present on every build.
        let state = unsafe {
            State {
                beauty: sys::oidnNewFilter(device.0, rt.as_ptr()),
                albedo: sys::oidnNewFilter(device.0, rt.as_ptr()),
                normal: sys::oidnNewFilter(device.0, rt.as_ptr()),
                bound: None,
            }
        };
        check_error(device.0)?;
        Ok(Arc::new(GpuDenoiser { device, state: Mutex::new(state) }))
    }

    /// Denoise one frame. The same contract as
    /// [`Denoiser::denoise`](crate::Denoiser::denoise).
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
            return Err("denoiser buffer size mismatch".into());
        }
        if width == 0 || height == 0 {
            return Ok(DenoisedImage { width, height, data: Vec::new() });
        }
        let bytes = expected * std::mem::size_of::<f32>();
        let quality = settings.quality.raw();
        let mut state = self.state.lock().map_err(|_| "denoiser mutex poisoned")?;
        let rebind = state.bound.as_ref().is_none_or(|b| {
            b.width != width || b.height != height || b.quality != quality || b.prefilter != settings.prefilter_guides
        });
        // SAFETY: buffers are created on this device and bound before the
        // filters run; every call below is serialized by the mutex.
        unsafe {
            if rebind {
                state.bound = None;
                let device = self.device.0;
                let bound = Bound {
                    width,
                    height,
                    quality,
                    prefilter: settings.prefilter_guides,
                    color: sys::oidnNewBuffer(device, bytes),
                    albedo: sys::oidnNewBuffer(device, bytes),
                    normal: sys::oidnNewBuffer(device, bytes),
                    output: sys::oidnNewBuffer(device, bytes),
                };
                check_error(device)?;
                let image = |filter, name: &str, buffer| {
                    let name = CString::new(name).unwrap();
                    sys::oidnSetFilterImage(filter, name.as_ptr(), buffer, sys::OIDN_FORMAT_FLOAT3, width, height, 0, 0, 0);
                };
                // The guides are filtered in place, so the beauty filter reads
                // them clean.
                // Bound as what they are; see `run_guide_filter`.
                for (filter, buffer, guide) in [(state.albedo, bound.albedo, "albedo"), (state.normal, bound.normal, "normal")] {
                    image(filter, guide, buffer);
                    image(filter, "output", buffer);
                    sys::oidnCommitFilter(filter);
                }
                image(state.beauty, "color", bound.color);
                image(state.beauty, "albedo", bound.albedo);
                image(state.beauty, "normal", bound.normal);
                image(state.beauty, "output", bound.output);
                let set_bool = |name: &str, value: bool| {
                    let name = CString::new(name).unwrap();
                    sys::oidnSetFilterBool(state.beauty, name.as_ptr(), value);
                };
                set_bool("hdr", true);
                set_bool("cleanAux", settings.prefilter_guides);
                let q = CString::new("quality").unwrap();
                sys::oidnSetFilterInt(state.beauty, q.as_ptr(), quality);
                sys::oidnCommitFilter(state.beauty);
                check_error(device)?;
                state.bound = Some(bound);
            }
            let bound = state.bound.as_ref().expect("bound above");
            sys::oidnWriteBuffer(bound.color, 0, bytes, color.as_ptr().cast());
            sys::oidnWriteBuffer(bound.albedo, 0, bytes, albedo.as_ptr().cast());
            sys::oidnWriteBuffer(bound.normal, 0, bytes, normal.as_ptr().cast());
            if settings.prefilter_guides {
                sys::oidnExecuteFilter(state.albedo);
                sys::oidnExecuteFilter(state.normal);
            }
            sys::oidnExecuteFilter(state.beauty);
            let mut output = vec![0.0f32; expected];
            sys::oidnReadBuffer(bound.output, 0, bytes, output.as_mut_ptr().cast());
            check_error(self.device.0)?;
            drop(state);

            let strength = settings.strength.clamp(0.0, 1.0);
            if strength < 1.0 {
                for (out, raw) in output.iter_mut().zip(color) {
                    *out = *raw + (*out - *raw) * strength;
                }
            }
            Ok(DenoisedImage { width, height, data: output })
        }
    }
}
