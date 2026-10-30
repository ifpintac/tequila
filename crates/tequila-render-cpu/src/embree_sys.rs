//! Minimal FFI bindings to the Embree 4 C API.
//!
//! Hand-written rather than generated. `bindgen` would pull in libclang as a
//! build-time dependency for a surface area of roughly a dozen functions, and
//! the structs below are stable ABI that Embree has not changed across the 4.x
//! series. Transcribing them keeps the build to "unzip the SDK, set one
//! environment variable".
//!
//! Layouts are transcribed from `embree4/rtcore_ray.h`, `rtcore_common.h`,
//! `rtcore_geometry.h` and `rtcore_buffer.h` of Embree 4.4.1. The static
//! assertions at the bottom of this file check the sizes and offsets the
//! renderer depends on, so a future SDK that moved a field would fail the
//! build rather than silently return wrong hits.

#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_void};

pub type RTCDevice = *mut c_void;
pub type RTCScene = *mut c_void;
pub type RTCGeometry = *mut c_void;

pub const RTC_INVALID_GEOMETRY_ID: u32 = u32::MAX;

/// `RTC_MAX_INSTANCE_LEVEL_COUNT` from `rtcore_config.h`. Embree's default
/// build is single-level, and the shipped SDK confirms it.
pub const RTC_MAX_INSTANCE_LEVEL_COUNT: usize = 1;

// --- enums, as their underlying integer values ----------------------------

pub const RTC_GEOMETRY_TYPE_TRIANGLE: u32 = 0;
/// 121, not 120: 120 is `RTC_GEOMETRY_TYPE_USER`. Attaching an instanced
/// scene to a user geometry is accepted by the compiler and rejected by
/// Embree at commit time with a bare "invalid operation".
pub const RTC_GEOMETRY_TYPE_INSTANCE: u32 = 121;
/// Ray-facing ribbons along straight segments: the fast curves Embree
/// offers for hair, whose width faces every ray that meets them. Their
/// hits closer than twice the radius to a ray's origin are ignored, which is
/// what lets a ray leave a fibre without meeting the same fibre again.
pub const RTC_GEOMETRY_TYPE_FLAT_LINEAR_CURVE: u32 = 17;

pub const RTC_BUFFER_TYPE_INDEX: u32 = 0;
pub const RTC_BUFFER_TYPE_VERTEX: u32 = 1;

pub const RTC_FORMAT_UINT: u32 = 0x5001;
pub const RTC_FORMAT_UINT3: u32 = 0x5003;
pub const RTC_FORMAT_FLOAT3: u32 = 0x9003;
/// A curve's control point: position and radius.
pub const RTC_FORMAT_FLOAT4: u32 = 0x9004;
pub const RTC_FORMAT_FLOAT3X4_COLUMN_MAJOR: u32 = 0x9234;

// The full quality ladder is transcribed even though only HIGH is used yet.
// REFIT in particular is load-bearing later: `docs/01` §14 wants a refit
// rather than a rebuild for animation playback, which is roughly an order of
// magnitude cheaper. Keeping the constants together means that change is a
// one-line edit rather than another trip through the headers.
#[allow(dead_code)]
pub const RTC_BUILD_QUALITY_LOW: u32 = 0;
#[allow(dead_code)]
pub const RTC_BUILD_QUALITY_MEDIUM: u32 = 1;
pub const RTC_BUILD_QUALITY_HIGH: u32 = 2;
#[allow(dead_code)]
pub const RTC_BUILD_QUALITY_REFIT: u32 = 3;

pub const RTC_ERROR_NONE: u32 = 0;

/// `RTC_DEVICE_PROPERTY_RAY_MASK_SUPPORTED` from `rtcore_device.h`. Masking is
/// a compile-time option of the Embree build (`EMBREE_RAY_MASK`, on in the
/// shipped SDK's `embree-config.cmake`), and a device built without it does
/// not test geometry masks at all, so it is asked rather than assumed.
pub const RTC_DEVICE_PROPERTY_RAY_MASK_SUPPORTED: u32 = 64;

/// Single-ray query structure.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default)]
pub struct RTCRay {
    pub org_x: f32,
    pub org_y: f32,
    pub org_z: f32,
    pub tnear: f32,
    pub dir_x: f32,
    pub dir_y: f32,
    pub dir_z: f32,
    pub time: f32,
    pub tfar: f32,
    pub mask: u32,
    pub id: u32,
    pub flags: u32,
}

/// Single-ray hit structure.
///
/// `inst_prim_id` is present because the shipped SDK defines
/// `RTC_GEOMETRY_INSTANCE_ARRAY`. Omitting it would leave the struct short and
/// let Embree write past the end of it.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug)]
pub struct RTCHit {
    pub ng_x: f32,
    pub ng_y: f32,
    pub ng_z: f32,
    pub u: f32,
    pub v: f32,
    pub prim_id: u32,
    pub geom_id: u32,
    pub inst_id: [u32; RTC_MAX_INSTANCE_LEVEL_COUNT],
    pub inst_prim_id: [u32; RTC_MAX_INSTANCE_LEVEL_COUNT],
}

impl Default for RTCHit {
    fn default() -> Self {
        RTCHit {
            ng_x: 0.0,
            ng_y: 0.0,
            ng_z: 0.0,
            u: 0.0,
            v: 0.0,
            prim_id: RTC_INVALID_GEOMETRY_ID,
            geom_id: RTC_INVALID_GEOMETRY_ID,
            inst_id: [RTC_INVALID_GEOMETRY_ID; RTC_MAX_INSTANCE_LEVEL_COUNT],
            inst_prim_id: [RTC_INVALID_GEOMETRY_ID; RTC_MAX_INSTANCE_LEVEL_COUNT],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct RTCRayHit {
    pub ray: RTCRay,
    pub hit: RTCHit,
}

#[link(name = "embree4")]
extern "C" {
    pub fn rtcNewDevice(config: *const c_char) -> RTCDevice;
    pub fn rtcReleaseDevice(device: RTCDevice);
    pub fn rtcGetDeviceError(device: RTCDevice) -> u32;
    pub fn rtcGetDeviceProperty(device: RTCDevice, property: u32) -> isize;

    pub fn rtcNewScene(device: RTCDevice) -> RTCScene;
    pub fn rtcReleaseScene(scene: RTCScene);
    pub fn rtcSetSceneBuildQuality(scene: RTCScene, quality: u32);
    pub fn rtcCommitScene(scene: RTCScene);
    pub fn rtcAttachGeometry(scene: RTCScene, geometry: RTCGeometry) -> u32;

    pub fn rtcNewGeometry(device: RTCDevice, kind: u32) -> RTCGeometry;
    pub fn rtcReleaseGeometry(geometry: RTCGeometry);
    pub fn rtcCommitGeometry(geometry: RTCGeometry);

    /// Retrieve a geometry from the scene it was attached to.
    ///
    /// The alternative — keeping the handle from `rtcAttachGeometry` after
    /// releasing our own reference — works, because the scene holds one, but
    /// it leaves a raw pointer whose validity depends on a refcount nothing in
    /// the type system is tracking. Looking it up costs an array index.
    ///
    /// Not thread-safe against `rtcCommitScene`; only called from the single
    /// -threaded update path.
    pub fn rtcGetGeometry(scene: RTCScene, geom_id: u32) -> RTCGeometry;
    #[allow(dead_code)]
    pub fn rtcSetGeometryBuildQuality(geometry: RTCGeometry, quality: u32);
    pub fn rtcSetSharedGeometryBuffer(
        geometry: RTCGeometry,
        buffer_type: u32,
        slot: u32,
        format: u32,
        ptr: *const c_void,
        byte_offset: usize,
        byte_stride: usize,
        item_count: usize,
    );
    pub fn rtcSetGeometryInstancedScene(geometry: RTCGeometry, scene: RTCScene);
    pub fn rtcSetGeometryMask(geometry: RTCGeometry, mask: u32);
    pub fn rtcSetGeometryTransform(
        geometry: RTCGeometry,
        time_step: u32,
        format: u32,
        xfm: *const c_void,
    );

    pub fn rtcIntersect1(scene: RTCScene, rayhit: *mut RTCRayHit, args: *mut c_void);
    pub fn rtcOccluded1(scene: RTCScene, ray: *mut RTCRay, args: *mut c_void);
}

// Layout checks. If a future Embree moves a field, this fails to compile
// rather than returning quietly wrong intersections — which would look like a
// geometry bug and cost days to localize.
const _: () = {
    use std::mem::{align_of, size_of};

    assert!(size_of::<RTCRay>() == 48);
    assert!(align_of::<RTCRay>() == 16);
    // 7 floats + 2 uints of hit data, plus one instance level of each id
    // array, all 4 bytes wide, rounded up to the 16-byte alignment.
    assert!(size_of::<RTCHit>() == 48);
    assert!(align_of::<RTCHit>() == 16);
    assert!(size_of::<RTCRayHit>() == 96);

    // `tfar` must sit immediately after the direction and time, because
    // occlusion queries signal a hit by writing a negative value into it.
    assert!(std::mem::offset_of!(RTCRay, tfar) == 32);
    assert!(std::mem::offset_of!(RTCHit, prim_id) == 20);
    assert!(std::mem::offset_of!(RTCHit, geom_id) == 24);
    assert!(std::mem::offset_of!(RTCHit, inst_id) == 28);
};
