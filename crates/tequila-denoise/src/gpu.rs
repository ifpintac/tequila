//! Denoising on the graphics card, for a renderer whose frames come from it.
//!
//! The CPU denoiser hands OIDN plain host pointers, which a GPU device cannot
//! read; this one keeps OIDN buffers on the card instead, allocated and bound
//! to the filters once per image size. A frame then costs three uploads, the
//! filter and one download — on an RTX 2060 a few tens of milliseconds for a
//! viewport-sized image, where the CPU took well over a second.

use crate::oidn_sys as sys;
use crate::{check_error, DenoiseSettings, DenoisedImage, Quality, RawDevice};
use std::ffi::CString;
use std::sync::{Arc, Mutex, OnceLock};

/// The most working memory, in megabytes, each filter is asked to keep on a
/// Mac: each of the two that filter the guides, and the one that denoises
/// the picture. Over it OIDN denoises in tiles, which overlap enough that
/// the picture comes out the same to the bit.
///
/// OIDN's Metal device keeps several times what it counts: it makes an
/// MPSGraph for each layer of the network, and each keeps buffers of its
/// own the size of the image, beside the memory the filters share. Without
/// a limit the Rendered view's denoiser grew with the viewport, in the
/// computer's own memory, a Mac's graphics card having none of its own:
/// measured on an M3 Max, 3.0 GB at 1528x858 (a 1440x900 window), 7.9 GB
/// at 2560x1440 and 9.9 GB at 3840x2160, held as long as the view showed.
/// With these limits (and the guides at Balanced, see [`guide_quality`]),
/// 1.85, 1.9 and 2.4 GB. The guides are filtered on fewer frames than the
/// picture - by default those of fewer than 32 samples, as the view settles
/// - so they are held tighter. The frames that filter them take about as
/// long as they did (123 ms instead of 132 at 1528x858, 386 instead of 364
/// at 2560x1440, 1.2 s instead of 1.6 at 3840x2160); the picture's alone
/// take as long as before at 1528x858, an eighth longer at 2560x1440 and a
/// third at 3840x2160, where OIDN cuts it into tiles.
///
/// Not on CUDA or HIP: there the memory is the card's own, and what OIDN
/// counts is what it takes.
const GUIDE_MEMORY_MB: i32 = 192;
/// See [`GUIDE_MEMORY_MB`].
const BEAUTY_MEMORY_MB: i32 = 256;

/// The quality the guides are filtered at: the picture's, as Cycles filters
/// them, but no lower than Balanced, OIDN having no faster filter for
/// guides. Left at OIDN's default, High, under the viewport's Fast and
/// Balanced pictures, they took 0.65 GB more and most of the time of the
/// frames that filter them (132 ms where Balanced guides take 72, 1528x858
/// on an M3 Max), for no more detail: a texture of one-pixel checks and
/// lines with grainy guides kept the same detail and error either way,
/// within 3%.
fn guide_quality(quality: Quality) -> i32 {
    match quality {
        Quality::High => sys::OIDN_QUALITY_HIGH,
        Quality::Fast | Quality::Balanced => sys::OIDN_QUALITY_BALANCED,
    }
}

/// Filters committed to one image size and quality, with the buffers
/// bound to them.
///
/// Committing is the dear part: on a Mac OIDN builds a graph of the network
/// for the size and quality, and an M3 Max took 110 to 330 ms over it for a
/// 1448x814 view, against 72 to 84 ms to denoise a frame once committed. A
/// set is kept for each of the last few sizes ([`kept`]), so a view that
/// goes back and forth between them - Tequila's renderer's quarter-size
/// frames while the camera moves, and its full-size ones once still - does
/// not commit at every move.
struct Bound {
    width: usize,
    height: usize,
    quality: i32,
    beauty: sys::OIDNFilter,
    albedo_filter: sys::OIDNFilter,
    normal_filter: sys::OIDNFilter,
    color: sys::OIDNBuffer,
    albedo: sys::OIDNBuffer,
    normal: sys::OIDNBuffer,
    output: sys::OIDNBuffer,
}

impl Drop for Bound {
    fn drop(&mut self) {
        // SAFETY: each came from `oidnNewFilter` or `oidnNewBuffer` and is
        // released once.
        unsafe {
            sys::oidnReleaseFilter(self.beauty);
            sys::oidnReleaseFilter(self.albedo_filter);
            sys::oidnReleaseFilter(self.normal_filter);
            sys::oidnReleaseBuffer(self.color);
            sys::oidnReleaseBuffer(self.albedo);
            sys::oidnReleaseBuffer(self.normal);
            sys::oidnReleaseBuffer(self.output);
        }
    }
}

/// Which of `sets` - the newest last - stay committed: the newest, and the
/// others, newest first, while all kept come to no more than half as many
/// pixels again as the largest of them. A view's quarter- and half-size
/// sets stay beside its full-size one (5/16 more), at a small share of the
/// memory; the set of a size the window has left, about as large as the
/// one it has now, goes. A size seen at another quality is that set made
/// again: one quality to a size is what the viewport asks for.
fn kept(sets: &mut Vec<Bound>) {
    let keep = keeps(&sets.iter().map(|b| (b.width, b.height)).collect::<Vec<_>>());
    let mut flags = keep.into_iter();
    sets.retain(|_| flags.next().unwrap_or(false));
}

/// [`kept`]'s choice, for sets of these sizes, the newest last.
fn keeps(sizes: &[(usize, usize)]) -> Vec<bool> {
    let mut keep = vec![false; sizes.len()];
    let mut chosen: Vec<(usize, usize)> = Vec::new();
    for (i, &(w, h)) in sizes.iter().enumerate().rev() {
        let largest = chosen.iter().map(|(w, h)| w * h).max().unwrap_or(0).max(w * h);
        let total: usize = chosen.iter().map(|(w, h)| w * h).sum::<usize>() + w * h;
        if chosen.is_empty() || (!chosen.contains(&(w, h)) && total * 2 <= largest * 3) {
            keep[i] = true;
            chosen.push((w, h));
        }
    }
    keep
}

#[cfg(test)]
mod tests {
    use super::keeps;

    /// A view's smaller sizes stay beside its full one; a size the window
    /// has left, about as large as the new one, goes; a size seen again at
    /// another quality is made again rather than kept twice.
    #[test]
    fn the_sizes_kept_are_the_views_own() {
        let (full, half, quarter) = ((1448, 814), (724, 407), (362, 204));
        assert_eq!(keeps(&[full, quarter, half]), vec![true, true, true], "the ladder stays");
        assert_eq!(keeps(&[quarter, half, full]), vec![true, true, true]);
        assert_eq!(keeps(&[full, (1300, 800)]), vec![false, true], "a resized window's old size goes");
        assert_eq!(keeps(&[full, full]), vec![false, true], "one set a size");
        assert_eq!(keeps(&[(64, 64)]), vec![true]);
        assert!(keeps(&[]).is_empty());
    }
}

struct State {
    /// The committed sets, the one used last at the end.
    bound: Vec<Bound>,
}

// The handles are only touched under the mutex.
unsafe impl Send for State {}

/// An OIDN device on the graphics card, with its filters.
pub struct GpuDenoiser {
    device: Arc<RawDevice>,
    state: Mutex<State>,
    /// The kind of device it is on: "Metal", "CUDA" or "HIP".
    kind: &'static str,
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
        let Some((device, kind)) = kinds.iter().map(|(kind, name)| (unsafe { sys::oidnNewDevice(*kind) }, *name)).find(|(d, _)| !d.is_null()) else {
            let names: Vec<&str> = kinds.iter().map(|(_, name)| *name).collect();
            return Err(format!("no {} denoising device", names.join(" or ")));
        };
        // SAFETY: non-null and freshly created.
        unsafe { sys::oidnCommitDevice(device) };
        let device = Arc::new(RawDevice(device));
        check_error(device.0)?;
        Ok(Arc::new(GpuDenoiser { device, state: Mutex::new(State { bound: Vec::new() }), kind }))
    }

    /// The kind of device it denoises on: "Metal", "CUDA" or "HIP".
    pub fn kind(&self) -> &'static str {
        self.kind
    }

    /// Let go of what the sizes seen last hold on the graphics card: the
    /// buffers bound to them, and the working memory OIDN's filters keep for
    /// them, about 2 GB on a Mac however large the viewport (see
    /// [`GUIDE_MEMORY_MB`]). Metal frees it within a second or two of this;
    /// about a tenth of a gigabyte stays, the device's and Metal's own
    /// (test `the_viewport_denoiser_holds_little_and_gives_it_back`). The
    /// next frame makes and commits filters at its own size, as the first did.
    pub fn release(&self) {
        let Ok(mut state) = self.state.lock() else { return };
        state.bound.clear();
    }

    /// Whether a frame is being denoised now, on another thread: a caller
    /// that would rather not wait goes to the processor's denoiser.
    pub fn is_busy(&self) -> bool {
        matches!(self.state.try_lock(), Err(std::sync::TryLockError::WouldBlock))
    }

    /// How many sizes have filters committed now.
    pub fn committed(&self) -> usize {
        self.state.lock().map_or(0, |state| state.bound.len())
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
        // Prefiltering or not changes only which filters run, so the
        // viewport's frames, filtered below four samples and not above, keep
        // their buffers.
        let found = state.bound.iter().position(|b| b.width == width && b.height == height && b.quality == quality);
        // SAFETY: buffers are created on this device and bound before the
        // filters run; every call below is serialized by the mutex.
        unsafe {
            match found {
                Some(at) => {
                    let set = state.bound.remove(at);
                    state.bound.push(set);
                }
                None => {
                    let device = self.device.0;
                    let rt = CString::new("RT").unwrap();
                    let bound = Bound {
                        width,
                        height,
                        quality,
                        beauty: sys::oidnNewFilter(device, rt.as_ptr()),
                        albedo_filter: sys::oidnNewFilter(device, rt.as_ptr()),
                        normal_filter: sys::oidnNewFilter(device, rt.as_ptr()),
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
                    let set_int = |filter, name: &str, value: i32| {
                        let name = CString::new(name).unwrap();
                        sys::oidnSetFilterInt(filter, name.as_ptr(), value);
                    };
                    let limits = (self.kind == "Metal").then_some((GUIDE_MEMORY_MB, BEAUTY_MEMORY_MB));
                    // The guides are filtered in place, so the beauty filter reads
                    // them clean.
                    // Bound as what they are; see `run_guide_filter`.
                    for (filter, buffer, guide) in [(bound.albedo_filter, bound.albedo, "albedo"), (bound.normal_filter, bound.normal, "normal")] {
                        image(filter, guide, buffer);
                        image(filter, "output", buffer);
                        set_int(filter, "quality", guide_quality(settings.quality));
                        if let Some((guides, _)) = limits {
                            set_int(filter, "maxMemoryMB", guides);
                        }
                        sys::oidnCommitFilter(filter);
                    }
                    image(bound.beauty, "color", bound.color);
                    image(bound.beauty, "albedo", bound.albedo);
                    image(bound.beauty, "normal", bound.normal);
                    image(bound.beauty, "output", bound.output);
                    let set_bool = |name: &str, value: bool| {
                        let name = CString::new(name).unwrap();
                        sys::oidnSetFilterBool(bound.beauty, name.as_ptr(), value);
                    };
                    set_bool("hdr", true);
                    // Clean either way; see `DenoiseSettings::prefilter_guides`.
                    set_bool("cleanAux", true);
                    set_int(bound.beauty, "quality", quality);
                    if let Some((_, beauty)) = limits {
                        set_int(bound.beauty, "maxMemoryMB", beauty);
                    }
                    sys::oidnCommitFilter(bound.beauty);
                    check_error(device)?;
                    state.bound.push(bound);
                    kept(&mut state.bound);
                }
            }
            let bound = state.bound.last().expect("bound above");
            sys::oidnWriteBuffer(bound.color, 0, bytes, color.as_ptr().cast());
            sys::oidnWriteBuffer(bound.albedo, 0, bytes, albedo.as_ptr().cast());
            sys::oidnWriteBuffer(bound.normal, 0, bytes, normal.as_ptr().cast());
            if settings.prefilter_guides {
                sys::oidnExecuteFilter(bound.albedo_filter);
                sys::oidnExecuteFilter(bound.normal_filter);
            }
            sys::oidnExecuteFilter(bound.beauty);
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
