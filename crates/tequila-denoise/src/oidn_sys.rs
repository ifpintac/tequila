//! Minimal FFI bindings to the Intel Open Image Denoise 2.x C API.
//!
//! Hand-written for the same reason as the Embree bindings: the surface area
//! is a dozen functions, and generating them would add libclang to the build
//! for no benefit. Transcribed from `OpenImageDenoise/oidn.h` of OIDN 2.5.1.

#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_void};

pub type OIDNDevice = *mut c_void;
pub type OIDNFilter = *mut c_void;
pub type OIDNBuffer = *mut c_void;

/// Let OIDN pick the best available device. Not used yet — see the note in
/// `Denoiser::create` about why the CPU device is chosen explicitly until the
/// renderer itself moves to the GPU at M9.
#[allow(dead_code)]
pub const OIDN_DEVICE_TYPE_DEFAULT: i32 = 0;
/// The CPU device, the only one that can read plain host pointers.
pub const OIDN_DEVICE_TYPE_CPU: i32 = 1;
pub const OIDN_DEVICE_TYPE_CUDA: i32 = 3;
pub const OIDN_DEVICE_TYPE_HIP: i32 = 4;
pub const OIDN_DEVICE_TYPE_METAL: i32 = 5;

pub const OIDN_FORMAT_FLOAT3: i32 = 3;

pub const OIDN_ERROR_NONE: i32 = 0;

/// Quality presets. `Balanced` is the interactive default; `High` is for final
/// frames, where the extra cost is irrelevant.
#[allow(dead_code)]
pub const OIDN_QUALITY_DEFAULT: i32 = 0;
pub const OIDN_QUALITY_FAST: i32 = 4;
pub const OIDN_QUALITY_BALANCED: i32 = 5;
pub const OIDN_QUALITY_HIGH: i32 = 6;

#[link(name = "OpenImageDenoise")]
extern "C" {
    pub fn oidnNewDevice(device_type: i32) -> OIDNDevice;
    pub fn oidnCommitDevice(device: OIDNDevice);
    pub fn oidnReleaseDevice(device: OIDNDevice);
    pub fn oidnGetDeviceError(device: OIDNDevice, out_message: *mut *const c_char) -> i32;

    pub fn oidnNewBuffer(device: OIDNDevice, byte_size: usize) -> OIDNBuffer;
    pub fn oidnReleaseBuffer(buffer: OIDNBuffer);
    pub fn oidnReadBuffer(buffer: OIDNBuffer, byte_offset: usize, byte_size: usize, dst_host_ptr: *mut c_void);
    pub fn oidnWriteBuffer(buffer: OIDNBuffer, byte_offset: usize, byte_size: usize, src_host_ptr: *const c_void);
    pub fn oidnSetFilterImage(
        filter: OIDNFilter,
        name: *const c_char,
        buffer: OIDNBuffer,
        format: i32,
        width: usize,
        height: usize,
        byte_offset: usize,
        pixel_byte_stride: usize,
        row_byte_stride: usize,
    );

    pub fn oidnNewFilter(device: OIDNDevice, filter_type: *const c_char) -> OIDNFilter;
    pub fn oidnReleaseFilter(filter: OIDNFilter);
    pub fn oidnCommitFilter(filter: OIDNFilter);
    pub fn oidnExecuteFilter(filter: OIDNFilter);

    pub fn oidnSetSharedFilterImage(
        filter: OIDNFilter,
        name: *const c_char,
        dev_ptr: *mut c_void,
        format: i32,
        width: usize,
        height: usize,
        byte_offset: usize,
        pixel_byte_stride: usize,
        row_byte_stride: usize,
    );
    #[allow(dead_code)]
    pub fn oidnUnsetFilterImage(filter: OIDNFilter, name: *const c_char);
    pub fn oidnSetFilterBool(filter: OIDNFilter, name: *const c_char, value: bool);
    pub fn oidnSetFilterInt(filter: OIDNFilter, name: *const c_char, value: i32);
    #[allow(dead_code)]
    pub fn oidnSetFilterFloat(filter: OIDNFilter, name: *const c_char, value: f32);
}
