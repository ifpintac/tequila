//! The C interface of `tequila_cycles.dll`, loaded at runtime.
//!
//! The declarations mirror `native/cycles/tequila_cycles.h`. They are
//! resolved once, on first use, so a missing DLL is an error a caller can
//! report ("Cycles is not installed") rather than a process that will not
//! start.

use std::ffi::{c_char, c_int, c_uint, c_void, CStr};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub type Session = c_void;
pub type Node = *mut c_void;
pub type Progress = Option<unsafe extern "C" fn(user: *mut c_void, fraction: f32)>;

pub const VISIBLE_CAMERA: c_uint = 1;
pub const VISIBLE_DIFFUSE: c_uint = 2;
pub const VISIBLE_GLOSSY: c_uint = 4;
pub const VISIBLE_TRANSMISSION: c_uint = 8;
pub const VISIBLE_SHADOW: c_uint = 16;
/// Volume scattering: what a light needs to light smoke, milk or a frosted
/// medium from within.
pub const VISIBLE_SCATTER: c_uint = 32;
pub const VISIBLE_ALL: c_uint = 63;

macro_rules! api {
    (
        $($name:ident: fn($($arg:ty),*) $(-> $ret:ty)?;)*
        @optional { $($opt:ident: fn($($oarg:ty),*) $(-> $oret:ty)?;)* }
    ) => {
        /// Every entry point, resolved; the optional ones are those a library
        /// built before them lacks, and Tequila manages without.
        #[allow(non_snake_case)]
        pub struct Api {
            _library: libloading::Library,
            /// Where the library was loaded from: its own libraries and the
            /// GPU kernels (`lib/`) sit beside it.
            pub folder: PathBuf,
            $(pub $name: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)*
            $(pub $opt: Option<unsafe extern "C" fn($($oarg),*) $(-> $oret)?>,)*
        }

        impl Api {
            fn load(path: &Path) -> Result<Api, String> {
                let library = open(path)?;
                unsafe {
                    $(
                        let $name = *library
                            .get::<unsafe extern "C" fn($($arg),*) $(-> $ret)?>(
                                concat!(stringify!($name), "\0").as_bytes(),
                            )
                            .map_err(|e| format!("{}: {e}", stringify!($name)))?;
                    )*
                    $(
                        let $opt = library
                            .get::<unsafe extern "C" fn($($oarg),*) $(-> $oret)?>(
                                concat!(stringify!($opt), "\0").as_bytes(),
                            )
                            .ok()
                            .map(|symbol| *symbol);
                    )*
                    let folder = path.parent().map(Path::to_path_buf).unwrap_or_default();
                    Ok(Api { _library: library, folder, $($name,)* $($opt,)* })
                }
            }
        }
    };
}

api! {
    fsc_last_error: fn() -> *const c_char;
    fsc_init: fn(*const c_char);
    fsc_devices: fn() -> *const c_char;
    fsc_create: fn(*const c_char, c_int, *const c_char) -> *mut Session;
    fsc_destroy: fn(*mut Session);
    fsc_scene_node: fn(*mut Session, *const c_char) -> Node;
    fsc_set_float: fn(Node, *const c_char, f32) -> c_int;
    fsc_set_float3: fn(Node, *const c_char, f32, f32, f32) -> c_int;
    fsc_set_int: fn(Node, *const c_char, c_int) -> c_int;
    fsc_set_bool: fn(Node, *const c_char, c_int) -> c_int;
    fsc_set_string: fn(Node, *const c_char, *const c_char) -> c_int;
    fsc_set_transform: fn(Node, *const c_char, *const f32) -> c_int;
    fsc_shader_new: fn(*mut Session, *const c_char) -> Node;
    fsc_shader_node: fn(*mut Session, Node, *const c_char) -> Node;
    fsc_connect: fn(*mut Session, Node, Node, *const c_char, Node, *const c_char) -> c_int;
    fsc_shader_commit: fn(*mut Session, Node) -> c_int;
    fsc_set_world: fn(*mut Session, Node);
    fsc_image: fn(*mut Session, *const c_char, c_int, c_int, c_int, *const f32) -> c_int;
    fsc_set_image: fn(*mut Session, Node, c_int) -> c_int;
    fsc_image_bytes: fn(*mut Session, *const c_char, c_int, c_int, c_int, *const u8, c_int) -> c_int;
    fsc_forget_image: fn(*mut Session, c_int);
    fsc_mesh: fn(
        *mut Session,
        c_int,
        *const f32,
        *const f32,
        c_int,
        *const c_int,
        *const f32,
        *const c_int,
        c_int,
        *const Node
    ) -> Node;
    fsc_object: fn(*mut Session, Node, *const f32, c_uint, c_int) -> Node;
    fsc_light: fn(*mut Session, *const c_char, Node, *const f32, c_uint) -> Node;
    fsc_camera: fn(*mut Session, *const f32, f32, c_int, c_int);
    fsc_render: fn(*mut Session, c_int, c_int, c_int, *mut f32, Progress, *mut c_void) -> c_int;
    fsc_create_live: fn(*const c_char, c_int, *const c_char) -> *mut Session;
    fsc_edit_begin: fn(*mut Session);
    fsc_edit_end: fn(*mut Session, c_int, c_int, c_int);
    fsc_clear_scene: fn(*mut Session);
    fsc_live_frame: fn(*mut Session, *mut f32, *mut f32, *mut f32, *mut f32, c_int, *mut c_int, *mut c_int, *mut c_int, *mut u64) -> c_int;
    fsc_set_paused: fn(*mut Session, c_int);
    fsc_status: fn(*mut Session) -> *const c_char;
    fsc_cancel: fn(*mut Session);
    fsc_set_time_limit: fn(*mut Session, f64);
    fsc_set_shadow_tint: fn(*mut Session, f32, f32, f32);
    fsc_rendered_samples: fn(*mut Session) -> c_int;
    fsc_clear: fn(*mut Session);
    @optional {
        fsc_mesh_colors: fn(*mut Session, Node, c_int, *const f32) -> c_int;
        fsc_shader_reopen: fn(*mut Session, Node) -> c_int;
        fsc_light_linking: fn(*mut Session, Node, u64, c_uint) -> c_int;
        fsc_shadow_linking: fn(*mut Session, Node, u64, c_uint) -> c_int;
        fsc_camera_viewplane: fn(*mut Session, f32, f32, f32, f32);
        fsc_add_pass: fn(*mut Session, *const c_char, *const c_char, c_int, *const c_char) -> c_int;
        fsc_cryptomatte: fn(*mut Session, c_int) -> c_int;
        fsc_render_pass: fn(*mut Session, *const c_char, *mut f32, c_int) -> c_int;
        fsc_object_of: fn(*mut Session, Node) -> Node;
        fsc_set_name: fn(Node, *const c_char) -> c_int;
        fsc_mesh_attribute: fn(*mut Session, Node, *const c_char, c_int, *const f32) -> c_int;
    }
}

#[cfg(windows)]
fn open(path: &Path) -> Result<libloading::Library, String> {
    // With the altered search path, the DLL's own folder is searched for the
    // libraries it depends on (OpenImageIO and the rest), which is where they
    // are shipped.
    const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 0x0000_0008;
    unsafe {
        libloading::os::windows::Library::load_with_flags(path, LOAD_WITH_ALTERED_SEARCH_PATH)
            .map(Into::into)
            .map_err(|e| format!("could not load {}: {e}", path.display()))
    }
}

#[cfg(not(windows))]
fn open(path: &Path) -> Result<libloading::Library, String> {
    unsafe {
        libloading::Library::new(path).map_err(|e| format!("could not load {}: {e}", path.display()))
    }
}

const LIBRARY: &str = if cfg!(windows) {
    "tequila_cycles.dll"
} else if cfg!(target_os = "macos") {
    "libtequila_cycles.dylib"
} else {
    "libtequila_cycles.so"
};

/// Where to look: `TEQUILA_CYCLES` (a file or a folder), then a `cycles`
/// folder beside the executable, then beside the executable itself - and in
/// a Mac app bundle, whose executable is in `Contents/MacOS`, the bundle's
/// `Contents/Frameworks/cycles`.
fn candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(path) = std::env::var_os("TEQUILA_CYCLES") {
        let path = PathBuf::from(path);
        if path.is_dir() {
            out.push(path.join(LIBRARY));
        } else {
            out.push(path);
        }
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        out.extend(beside(&dir, cfg!(target_os = "macos")));
    }
    out
}

/// The places beside an executable in `dir`.
fn beside(dir: &Path, macos: bool) -> Vec<PathBuf> {
    let mut out = vec![dir.join("cycles").join(LIBRARY), dir.join(LIBRARY)];
    if macos && dir.file_name().is_some_and(|n| n == "MacOS") {
        if let Some(contents) = dir.parent() {
            out.push(contents.join("Frameworks").join("cycles").join(LIBRARY));
        }
    }
    out
}

#[cfg(test)]
mod location_tests {
    use super::*;

    #[test]
    fn a_mac_bundle_is_looked_in() {
        let found = beside(Path::new("/Applications/Tequila.app/Contents/MacOS"), true);
        assert!(found.iter().any(|p| p.ends_with(Path::new("Contents/Frameworks/cycles").join(LIBRARY))), "{found:?}");
        // Elsewhere, and outside a bundle, only beside the executable.
        assert_eq!(beside(Path::new("C:/Program Files/Tequila"), false).len(), 2);
        assert_eq!(beside(Path::new("/usr/local/bin"), true).len(), 2);
    }
}

/// The loaded library, or why it could not be loaded.
pub fn api() -> Result<&'static Api, String> {
    static API: OnceLock<Result<Api, String>> = OnceLock::new();
    API.get_or_init(|| {
        let tried = candidates();
        let Some(path) = tried.iter().find(|p| p.exists()) else {
            return Err(format!(
                "Cycles is not installed: no {LIBRARY} in {}",
                tried.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
            ));
        };
        Api::load(path)
    })
    .as_ref()
    .map_err(Clone::clone)
}

/// The library's last error message.
pub fn last_error(api: &Api) -> String {
    unsafe {
        let message = (api.fsc_last_error)();
        if message.is_null() {
            String::new()
        } else {
            CStr::from_ptr(message).to_string_lossy().into_owned()
        }
    }
}
