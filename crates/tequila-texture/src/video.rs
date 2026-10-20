//! A frame of a video, for a label: a product's screen showing a moment of
//! its interface footage, a box printed with a frame of the advert.
//!
//! Decoded by the system's own decoders - Media Foundation on Windows,
//! AVFoundation on a Mac - so Tequila carries no video codec of its own and
//! reads what the system plays: H.264 and HEVC in MP4 and MOV everywhere,
//! and on Windows WMV and AVI as well. Elsewhere a video is refused with a
//! message saying so. A label is a still picture, so a video gives up one
//! frame, which the application keeps as a PNG beside the project.

use std::path::Path;

/// The extensions read as video.
pub const EXTENSIONS: &[&str] = &["mp4", "mov", "m4v", "avi", "wmv", "mkv", "webm"];

/// Whether `path` names a video, by its extension.
pub fn is_video(path: impl AsRef<Path>) -> bool {
    path.as_ref()
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// One frame: RGBA, 8 bits a channel, display-encoded (sRGB), rows top to
/// bottom.
#[derive(Clone, Debug)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
    /// Where in the video it is, in seconds.
    pub seconds: f64,
}

/// The frame showing at `seconds` into the video at `path`: the last one
/// to start at or before it.
pub fn frame(path: impl AsRef<Path>, seconds: f64) -> Result<Frame, String> {
    let path = path.as_ref();
    if !path.is_file() {
        return Err(format!("{} is not a file", path.display()));
    }
    system::frame(path, seconds.max(0.0))
}

/// How long the video at `path` runs, in seconds.
pub fn duration(path: impl AsRef<Path>) -> Result<f64, String> {
    let path = path.as_ref();
    if !path.is_file() {
        return Err(format!("{} is not a file", path.display()));
    }
    system::duration(path)
}

/// A still of the video at `path` at `seconds`, kept as a PNG in `folder`
/// and named after the video and the time; where it was kept. What a label
/// made from a video points at: a picture like any other label's, which
/// every renderer reads and a saved project keeps.
pub fn still(path: impl AsRef<Path>, seconds: f64, folder: impl AsRef<Path>) -> Result<std::path::PathBuf, String> {
    let path = path.as_ref();
    let frame = frame(path, seconds)?;
    let folder = folder.as_ref();
    std::fs::create_dir_all(folder).map_err(|e| format!("could not make {}: {e}", folder.display()))?;
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "video".into());
    let kept = folder.join(format!("{stem} at {:.2} s.png", frame.seconds));
    save_png(&frame, &kept)?;
    Ok(kept)
}

/// `frame` as an 8-bit RGBA PNG at `path`.
pub fn save_png(frame: &Frame, path: &Path) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("could not write {}: {e}", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), frame.width as u32, frame.height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    encoder
        .write_header()
        .and_then(|mut writer| writer.write_image_data(&frame.rgba))
        .map_err(|e| format!("could not write {}: {e}", path.display()))
}

#[cfg(windows)]
mod system {
    use super::Frame;
    use std::path::Path;
    use windows::core::{GUID, HSTRING, PROPVARIANT};
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

    /// Media Foundation, started for as long as it lives on this thread.
    struct Started {
        com: bool,
    }

    impl Started {
        fn new() -> Result<Started, String> {
            // SAFETY: plain initialisation calls, balanced in `drop`; COM
            // already started in another mode on this thread is no error
            // here, and then is not ours to end.
            unsafe {
                let com = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
                if let Err(e) = MFStartup(MF_VERSION, MFSTARTUP_LITE) {
                    if com {
                        CoUninitialize();
                    }
                    return Err(format!("Windows' video decoding would not start: {e}"));
                }
                Ok(Started { com })
            }
        }
    }

    impl Drop for Started {
        fn drop(&mut self) {
            // SAFETY: balances `new`.
            unsafe {
                let _ = MFShutdown();
                if self.com {
                    CoUninitialize();
                }
            }
        }
    }

    fn reader(path: &Path) -> Result<IMFSourceReader, String> {
        // SAFETY: Media Foundation's own calls, on objects it made.
        unsafe {
            let mut attributes: Option<IMFAttributes> = None;
            MFCreateAttributes(&mut attributes, 1).map_err(|e| e.to_string())?;
            let attributes = attributes.ok_or("Media Foundation made no attributes")?;
            // Its own colour conversion, to the RGB asked for below.
            attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1).map_err(|e| e.to_string())?;
            MFCreateSourceReaderFromURL(&HSTRING::from(path.as_os_str()), &attributes)
                .map_err(|e| format!("Windows cannot read this video ({})", e.message()))
        }
    }

    pub fn duration(path: &Path) -> Result<f64, String> {
        let _started = Started::new()?;
        let reader = reader(path)?;
        // SAFETY: as in `reader`.
        let value = unsafe { reader.GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION) }
            .map_err(|e| e.to_string())?;
        let hundreds = u64::try_from(&value).map_err(|e| e.to_string())?;
        Ok(hundreds as f64 / 1e7)
    }

    pub fn frame(path: &Path, seconds: f64) -> Result<Frame, String> {
        let _started = Started::new()?;
        let reader = reader(path)?;
        let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        // SAFETY: as in `reader`; the buffer is read only while locked, and
        // within the length it reports.
        unsafe {
            let wanted = MFCreateMediaType().map_err(|e| e.to_string())?;
            wanted.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(|e| e.to_string())?;
            wanted.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32).map_err(|e| e.to_string())?;
            reader
                .SetCurrentMediaType(stream, None, &wanted)
                .map_err(|e| format!("Windows cannot decode this video's pictures ({})", e.message()))?;
            let actual = reader.GetCurrentMediaType(stream).map_err(|e| e.to_string())?;
            let size = actual.GetUINT64(&MF_MT_FRAME_SIZE).map_err(|e| e.to_string())?;
            let (width, height) = ((size >> 32) as usize, (size & 0xffff_ffff) as usize);
            if width == 0 || height == 0 {
                return Err("the video has no picture".into());
            }
            let stride = actual.GetUINT32(&MF_MT_DEFAULT_STRIDE).map(|s| s as i32).unwrap_or(width as i32 * 4);

            // To the key frame before it, then forward to the frame showing.
            let target = (seconds * 1e7) as i64;
            let position = PROPVARIANT::from(target);
            reader.SetCurrentPosition(&GUID::zeroed(), &position).map_err(|e| e.to_string())?;
            let mut showing: Option<(IMFSample, i64)> = None;
            loop {
                let (mut flags, mut timestamp) = (0u32, 0i64);
                let mut sample: Option<IMFSample> = None;
                reader
                    .ReadSample(stream, 0, None, Some(&mut flags), Some(&mut timestamp), Some(&mut sample))
                    .map_err(|e| e.to_string())?;
                let ended = flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0;
                if let Some(sample) = sample {
                    // Past it: the one before is the one showing, unless
                    // there was none before.
                    if timestamp > target && showing.is_some() {
                        break;
                    }
                    showing = Some((sample, timestamp));
                    if timestamp >= target {
                        break;
                    }
                }
                if ended {
                    break;
                }
            }
            let (sample, timestamp) = showing.ok_or("the video has no picture at that time")?;
            let buffer = sample.ConvertToContiguousBuffer().map_err(|e| e.to_string())?;
            let mut data = std::ptr::null_mut();
            let mut length = 0u32;
            buffer.Lock(&mut data, None, Some(&mut length)).map_err(|e| e.to_string())?;
            let bytes = std::slice::from_raw_parts(data, length as usize);
            let row = stride.unsigned_abs() as usize;
            let mut rgba = Vec::with_capacity(width * height * 4);
            let fits = row >= width * 4 && bytes.len() >= row * height;
            if fits {
                for y in 0..height {
                    // A negative stride is a picture stored bottom up.
                    let source = if stride < 0 { height - 1 - y } else { y };
                    for pixel in bytes[source * row..source * row + width * 4].chunks_exact(4) {
                        // BGRX.
                        rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
                    }
                }
            }
            let _ = buffer.Unlock();
            if !fits {
                return Err("the decoded picture is smaller than the video says".into());
            }
            Ok(Frame { width, height, rgba, seconds: timestamp as f64 / 1e7 })
        }
    }
}

#[cfg(target_os = "macos")]
mod system {
    use super::Frame;
    use objc2::encode::{Encode, Encoding, RefEncode};
    use objc2::runtime::{AnyObject, Bool, MessageReceiver};
    use objc2::{class, msg_send, sel};
    use std::ffi::{c_void, CStr, CString};
    use std::path::Path;

    /// CoreMedia's time: a value over a timescale.
    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    struct CMTime {
        value: i64,
        timescale: i32,
        flags: u32,
        epoch: i64,
    }

    // SAFETY: CMTime's layout and its encoding, `{?=qiIq}`.
    unsafe impl Encode for CMTime {
        const ENCODING: Encoding = Encoding::Struct("?", &[i64::ENCODING, i32::ENCODING, u32::ENCODING, i64::ENCODING]);
    }
    unsafe impl RefEncode for CMTime {
        const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
    }

    /// A Core Graphics image, only ever behind a pointer.
    #[repr(C)]
    struct CGImage {
        _opaque: [u8; 0],
    }
    // SAFETY: CGImageRef's encoding, `^{CGImage=}`.
    unsafe impl RefEncode for CGImage {
        const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGImage", &[]));
    }

    #[repr(C)]
    struct CGRect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    }

    #[link(name = "AVFoundation", kind = "framework")]
    extern "C" {}

    #[link(name = "CoreMedia", kind = "framework")]
    extern "C" {
        fn CMTimeMakeWithSeconds(seconds: f64, timescale: i32) -> CMTime;
        fn CMTimeGetSeconds(time: CMTime) -> f64;
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        static kCGColorSpaceSRGB: *const c_void;
        fn CGColorSpaceCreateWithName(name: *const c_void) -> *mut c_void;
        fn CGColorSpaceRelease(space: *mut c_void);
        fn CGBitmapContextCreate(
            data: *mut c_void,
            width: usize,
            height: usize,
            bits: usize,
            row: usize,
            space: *mut c_void,
            info: u32,
        ) -> *mut c_void;
        fn CGContextDrawImage(context: *mut c_void, rect: CGRect, image: *mut CGImage);
        fn CGContextRelease(context: *mut c_void);
        fn CGImageGetWidth(image: *mut CGImage) -> usize;
        fn CGImageGetHeight(image: *mut CGImage) -> usize;
        fn CGImageRelease(image: *mut CGImage);
    }

    /// An asset for the file at `path`.
    ///
    /// # Safety
    /// Inside an autorelease pool, on any thread.
    unsafe fn asset(path: &Path) -> Result<*mut AnyObject, String> {
        let text = CString::new(path.to_string_lossy().as_bytes()).map_err(|_| "the path has a NUL in it")?;
        let string: *mut AnyObject = msg_send![class!(NSString), stringWithUTF8String: text.as_ptr()];
        let url: *mut AnyObject = msg_send![class!(NSURL), fileURLWithPath: string];
        let nothing = std::ptr::null_mut::<AnyObject>();
        let asset: *mut AnyObject = msg_send![class!(AVURLAsset), URLAssetWithURL: url, options: nothing];
        if asset.is_null() {
            return Err("macOS cannot read this video".into());
        }
        Ok(asset)
    }

    /// An NSError's description.
    unsafe fn described(error: *mut AnyObject) -> String {
        if error.is_null() {
            return "macOS cannot decode this video".into();
        }
        let text: *mut AnyObject = msg_send![error, localizedDescription];
        let utf8: *const std::ffi::c_char = if text.is_null() { std::ptr::null() } else { msg_send![text, UTF8String] };
        if utf8.is_null() {
            "macOS cannot decode this video".into()
        } else {
            CStr::from_ptr(utf8).to_string_lossy().into_owned()
        }
    }

    pub fn duration(path: &Path) -> Result<f64, String> {
        objc2::rc::autoreleasepool(|_| unsafe {
            let asset = asset(path)?;
            let time: CMTime = msg_send![asset, duration];
            let seconds = CMTimeGetSeconds(time);
            if seconds.is_finite() {
                Ok(seconds)
            } else {
                Err("the video says no length".into())
            }
        })
    }

    pub fn frame(path: &Path, seconds: f64) -> Result<Frame, String> {
        // SAFETY: AVFoundation's and Core Graphics' calls on objects they
        // made, each checked for null; the image and the context are
        // released before returning.
        objc2::rc::autoreleasepool(|_| unsafe {
            let asset = asset(path)?;
            let generator: *mut AnyObject = msg_send![class!(AVAssetImageGenerator), assetImageGeneratorWithAsset: asset];
            if generator.is_null() {
                return Err("macOS cannot take pictures from this video".into());
            }
            // Upright as it plays, and the frame at that time exactly rather
            // than the nearest key frame.
            let _: () = msg_send![generator, setAppliesPreferredTrackTransform: Bool::YES];
            let exact = CMTime { value: 0, timescale: 1, flags: 1, epoch: 0 };
            let _: () = msg_send![generator, setRequestedTimeToleranceBefore: exact];
            let _: () = msg_send![generator, setRequestedTimeToleranceAfter: exact];
            let time = CMTimeMakeWithSeconds(seconds, 600);
            let mut actual = CMTime { value: 0, timescale: 1, flags: 0, epoch: 0 };
            let mut error: *mut AnyObject = std::ptr::null_mut();
            // A `copy` method: sent plainly, as it returns no object.
            let image: *mut CGImage = generator.send_message(
                sel!(copyCGImageAtTime:actualTime:error:),
                (time, &mut actual as *mut CMTime, &mut error as *mut *mut AnyObject),
            );
            if image.is_null() {
                return Err(described(error));
            }
            let (width, height) = (CGImageGetWidth(image), CGImageGetHeight(image));
            let mut rgba = vec![0u8; width * height * 4];
            let space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
            // RGBA, premultiplied, in that byte order.
            const PREMULTIPLIED_LAST: u32 = 1;
            const BYTE_ORDER_32_BIG: u32 = 4 << 12;
            let context = CGBitmapContextCreate(
                rgba.as_mut_ptr().cast(),
                width,
                height,
                8,
                width * 4,
                space,
                PREMULTIPLIED_LAST | BYTE_ORDER_32_BIG,
            );
            if !space.is_null() {
                CGColorSpaceRelease(space);
            }
            if context.is_null() {
                CGImageRelease(image);
                return Err("macOS could not hand over the picture".into());
            }
            CGContextDrawImage(context, CGRect { x: 0.0, y: 0.0, width: width as f64, height: height as f64 }, image);
            CGContextRelease(context);
            CGImageRelease(image);
            // A video is opaque; straighten whatever is not.
            for pixel in rgba.chunks_exact_mut(4) {
                let a = pixel[3];
                if a > 0 && a < 255 {
                    for c in &mut pixel[..3] {
                        *c = ((*c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8;
                    }
                }
            }
            let shown = CMTimeGetSeconds(actual);
            Ok(Frame { width, height, rgba, seconds: if shown.is_finite() { shown } else { seconds } })
        })
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod system {
    use super::Frame;
    use std::path::Path;

    pub fn duration(_path: &Path) -> Result<f64, String> {
        Err("video labels need Windows or macOS".into())
    }

    pub fn frame(_path: &Path, _seconds: f64) -> Result<Frame, String> {
        Err("video labels need Windows or macOS".into())
    }
}
