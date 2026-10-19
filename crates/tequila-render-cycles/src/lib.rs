//! Rendering through Cycles.
//!
//! Cycles is Blender's path tracer, and an Apache-2.0 project of its own. It
//! is reached through `tequila_cycles.dll`, a thin C interface over its C++
//! API (`native/cycles`), which is loaded at runtime: without it Tequila
//! builds and runs exactly as before, and [`available`] says why Cycles is not
//! there.
//!
//! The division of labour: Cycles traces paths. Tequila decides what the
//! scene is ([`translate`] turns a render-core
//! [`SceneSnapshot`](tequila_render_core::scene::SceneSnapshot) into Cycles
//! nodes) and what the result looks like (the pixels come back scene-linear
//! and go through Tequila's own develop step, as its renderer's do).

pub mod ffi;
pub mod translate;

use std::ffi::{c_int, c_void, CString};
use std::sync::atomic::{AtomicBool, Ordering};

/// A Cycles node, as a session handed it out. Opaque, so only a session
/// makes one: the methods that take one pass it to C, and taking a bare
/// pointer from safe code would let any pointer through.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Node(ffi::Node);

/// Whether Cycles can be used here, and if not, why.
pub fn available() -> Result<(), String> {
    ffi::api().map(|_| ())
}

/// The devices Cycles can render on, as (type, description, id).
pub fn devices() -> Result<Vec<(String, String, String)>, String> {
    let api = ffi::api()?;
    // The kernel folder first: listed before it is set, the devices leave
    // out the graphics card, and leave it out for the rest of the process.
    let kernels = c(&kernel_folder(api, None).to_string_lossy());
    unsafe { (api.fsc_init)(kernels.as_ptr()) };
    let text = unsafe { std::ffi::CStr::from_ptr((api.fsc_devices)()) }
        .to_string_lossy()
        .into_owned();
    Ok(text
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            Some((parts.next()?.to_string(), parts.next()?.to_string(), parts.next()?.to_string()))
        })
        .collect())
}

/// The folder holding Cycles' `lib/` of GPU kernels: `explicit`, else
/// `TEQUILA_CYCLES_KERNELS`, else the library's own folder, which is where
/// a packaged build keeps them - or, in a Mac app bundle, the kernel
/// sources' folder in its Resources.
fn kernel_folder(api: &ffi::Api, explicit: Option<&std::path::Path>) -> std::path::PathBuf {
    explicit
        .map(std::path::Path::to_path_buf)
        .or_else(|| std::env::var_os("TEQUILA_CYCLES_KERNELS").map(Into::into))
        .or_else(|| bundle_kernels(&api.folder))
        .unwrap_or_else(|| api.folder.clone())
}

/// In a Mac app bundle the library is in `Contents/Frameworks/cycles`, where
/// only code may go, and the kernel sources Metal compiles from at the first
/// GPU render in `Contents/Resources/cycles/source`: Cycles is pointed at
/// the folder holding them.
fn bundle_kernels(library_folder: &std::path::Path) -> Option<std::path::PathBuf> {
    let frameworks = library_folder.parent()?;
    let contents = frameworks.parent()?;
    if frameworks.file_name()? != "Frameworks" || contents.file_name()? != "Contents" {
        return None;
    }
    let resources = contents.join("Resources").join("cycles");
    resources.join("source").is_dir().then_some(resources)
}

fn c(text: &str) -> CString {
    CString::new(text.replace('\0', " ")).expect("NULs replaced")
}

/// A Cycles session: one scene on one device.
pub struct Session {
    api: &'static ffi::Api,
    raw: *mut ffi::Session,
}

// The library serializes scene edits under Cycles' own scene mutex, and a
// session is only ever driven from one thread at a time by `&mut self`.
unsafe impl Send for Session {}

/// Where a live frame is copied: colour (RGBA), the denoising guides (RGB
/// each) and the pass id of the object each pixel shows, reused from frame
/// to frame.
#[derive(Default)]
pub struct FrameBuffers {
    pub rgba: Vec<f32>,
    pub albedo: Vec<f32>,
    pub normal: Vec<f32>,
    pub object: Vec<f32>,
}

/// A live session's frame: its size and how many samples it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub samples: u32,
}

/// A shader being built. Nodes are added to it until [`Session::commit`].
#[derive(Clone, Copy, Debug)]
pub struct Shader(pub(crate) Node);

impl Session {
    /// A session on `device`: "CPU", "OPTIX", "CUDA", or an id from
    /// [`devices`]. `kernels` is the folder holding Cycles' `lib/` of GPU
    /// kernels: by default `TEQUILA_CYCLES_KERNELS`, else the library's own
    /// folder, which is where a packaged build keeps them.
    pub fn new(device: &str, threads: usize, kernels: Option<&std::path::Path>) -> Result<Session, String> {
        Session::create(device, threads, kernels, false)
    }

    /// A live session, for a viewport: it renders continuously, refining one
    /// image, and restarts it at every [`Session::end_edit`].
    pub fn live(device: &str, threads: usize, kernels: Option<&std::path::Path>) -> Result<Session, String> {
        Session::create(device, threads, kernels, true)
    }

    fn create(device: &str, threads: usize, kernels: Option<&std::path::Path>, live: bool) -> Result<Session, String> {
        match Session::create_on(device, threads, kernels, live) {
            // OptiX needs RTX hardware and a recent driver; CUDA runs on any
            // NVIDIA card Cycles supports, a little slower.
            Err(_) if device.eq_ignore_ascii_case("OPTIX") => Session::create_on("CUDA", threads, kernels, live),
            result => result,
        }
    }

    fn create_on(device: &str, threads: usize, kernels: Option<&std::path::Path>, live: bool) -> Result<Session, String> {
        let api = ffi::api()?;
        let kernels = Some(c(&kernel_folder(api, kernels).to_string_lossy()));
        let create = if live { api.fsc_create_live } else { api.fsc_create };
        let raw = unsafe {
            create(
                c(device).as_ptr(),
                threads as c_int,
                kernels.as_ref().map_or(std::ptr::null(), |k| k.as_ptr()),
            )
        };
        if raw.is_null() {
            return Err(ffi::last_error(api));
        }
        Ok(Session { api, raw })
    }

    fn check(&self, ok: c_int) -> Result<(), String> {
        if ok != 0 {
            Ok(())
        } else {
            Err(ffi::last_error(self.api))
        }
    }

    fn node_or_error(&self, node: ffi::Node) -> Result<Node, String> {
        if node.is_null() {
            Err(ffi::last_error(self.api))
        } else {
            Ok(Node(node))
        }
    }

    /// "integrator", "film", "background" or "camera".
    pub fn scene_node(&mut self, which: &str) -> Result<Node, String> {
        let node = unsafe { (self.api.fsc_scene_node)(self.raw, c(which).as_ptr()) };
        self.node_or_error(node)
    }

    pub fn set_float(&mut self, node: Node, socket: &str, value: f32) -> Result<(), String> {
        self.check(unsafe { (self.api.fsc_set_float)(node.0, c(socket).as_ptr(), value) })
    }

    pub fn set_float3(&mut self, node: Node, socket: &str, value: [f32; 3]) -> Result<(), String> {
        self.check(unsafe {
            (self.api.fsc_set_float3)(node.0, c(socket).as_ptr(), value[0], value[1], value[2])
        })
    }

    pub fn set_int(&mut self, node: Node, socket: &str, value: i32) -> Result<(), String> {
        self.check(unsafe { (self.api.fsc_set_int)(node.0, c(socket).as_ptr(), value) })
    }

    pub fn set_bool(&mut self, node: Node, socket: &str, value: bool) -> Result<(), String> {
        self.check(unsafe { (self.api.fsc_set_bool)(node.0, c(socket).as_ptr(), value as c_int) })
    }

    /// Strings, and enums by their option name.
    pub fn set_string(&mut self, node: Node, socket: &str, value: &str) -> Result<(), String> {
        self.check(unsafe { (self.api.fsc_set_string)(node.0, c(socket).as_ptr(), c(value).as_ptr()) })
    }

    /// A 3x4 row-major transform.
    pub fn set_transform(&mut self, node: Node, socket: &str, matrix: &[f32; 12]) -> Result<(), String> {
        self.check(unsafe { (self.api.fsc_set_transform)(node.0, c(socket).as_ptr(), matrix.as_ptr()) })
    }

    pub fn shader(&mut self, name: &str) -> Result<Shader, String> {
        let node = unsafe { (self.api.fsc_shader_new)(self.raw, c(name).as_ptr()) };
        self.node_or_error(node).map(Shader)
    }

    /// A node of Cycles type `kind` ("principled_bsdf", "image_texture", ...)
    /// in `shader`. "output" is the shader's output.
    pub fn node(&mut self, shader: Shader, kind: &str) -> Result<Node, String> {
        let node = unsafe { (self.api.fsc_shader_node)(self.raw, shader.0.0, c(kind).as_ptr()) };
        self.node_or_error(node)
    }

    pub fn connect(&mut self, shader: Shader, from: Node, output: &str, to: Node, input: &str) -> Result<(), String> {
        self.check(unsafe {
            (self.api.fsc_connect)(self.raw, shader.0.0, from.0, c(output).as_ptr(), to.0, c(input).as_ptr())
        })
    }

    pub fn commit(&mut self, shader: Shader) -> Result<Node, String> {
        self.check(unsafe { (self.api.fsc_shader_commit)(self.raw, shader.0.0) })?;
        Ok(shader.0)
    }

    pub fn set_world(&mut self, shader: Node) {
        unsafe { (self.api.fsc_set_world)(self.raw, shader.0) }
    }

    /// Linear float pixels, rows top to bottom.
    pub fn image(&mut self, name: &str, width: usize, height: usize, channels: usize, pixels: &[f32]) -> Result<i32, String> {
        assert!(pixels.len() >= width * height * channels);
        let id = unsafe {
            (self.api.fsc_image)(
                self.raw,
                c(name).as_ptr(),
                width as c_int,
                height as c_int,
                channels as c_int,
                pixels.as_ptr(),
            )
        };
        if id < 0 {
            Err(ffi::last_error(self.api))
        } else {
            Ok(id)
        }
    }

    /// The same from 8-bit pixels: `srgb` for colour with the sRGB curve on
    /// it, which Cycles decodes as it samples.
    pub fn image_bytes(
        &mut self,
        name: &str,
        width: usize,
        height: usize,
        channels: usize,
        pixels: &[u8],
        srgb: bool,
    ) -> Result<i32, String> {
        assert!(pixels.len() >= width * height * channels);
        let id = unsafe {
            (self.api.fsc_image_bytes)(
                self.raw,
                c(name).as_ptr(),
                width as c_int,
                height as c_int,
                channels as c_int,
                pixels.as_ptr(),
                srgb as c_int,
            )
        };
        if id < 0 {
            Err(ffi::last_error(self.api))
        } else {
            Ok(id)
        }
    }

    /// Let go of an image's pixels on this side.
    pub fn forget_image(&mut self, image: i32) {
        unsafe { (self.api.fsc_forget_image)(self.raw, image) }
    }

    pub fn set_image(&mut self, node: Node, image: i32) -> Result<(), String> {
        self.check(unsafe { (self.api.fsc_set_image)(self.raw, node.0, image) })
    }

    /// A triangle mesh. `uvs` are per triangle corner.
    pub fn mesh(
        &mut self,
        positions: &[[f32; 3]],
        normals: Option<&[[f32; 3]]>,
        triangles: &[[i32; 3]],
        uvs: Option<&[[f32; 2]]>,
        shaders: &[Node],
    ) -> Result<Node, String> {
        if let Some(n) = normals {
            assert_eq!(n.len(), positions.len());
        }
        if let Some(uv) = uvs {
            assert_eq!(uv.len(), triangles.len() * 3);
        }
        let node = unsafe {
            (self.api.fsc_mesh)(
                self.raw,
                positions.len() as c_int,
                positions.as_ptr().cast(),
                normals.map_or(std::ptr::null(), |n| n.as_ptr().cast()),
                triangles.len() as c_int,
                triangles.as_ptr().cast(),
                uvs.map_or(std::ptr::null(), |u| u.as_ptr().cast()),
                std::ptr::null(),
                shaders.len() as c_int,
                // `Node` is transparent over the pointer.
                shaders.as_ptr().cast(),
            )
        };
        self.node_or_error(node)
    }

    /// Whether this library takes vertex colours (`mesh_colors`); one built
    /// before them does not.
    pub fn has_vertex_colours(&self) -> bool {
        self.api.fsc_mesh_colors.is_some()
    }

    /// Colours painted on `mesh`'s vertices, RGBA in the rendering space, one
    /// per vertex: the "tq_color" attribute.
    pub fn mesh_colors(&mut self, mesh: Node, rgba: &[[f32; 4]]) -> Result<(), String> {
        let Some(send) = self.api.fsc_mesh_colors else {
            return Err("this Cycles library predates vertex colours".into());
        };
        self.check(unsafe { send(self.raw, mesh.0, rgba.len() as c_int, rgba.as_ptr().cast()) })
    }

    pub fn object(&mut self, mesh: Node, matrix: &[f32; 12], visibility: u32, shadow_catcher: bool) -> Result<Node, String> {
        let node = unsafe { (self.api.fsc_object)(self.raw, mesh.0, matrix.as_ptr(), visibility, shadow_catcher as c_int) };
        self.node_or_error(node)
    }

    /// A light of Cycles type `kind` ("point", "area", "background", ...)
    /// emitting through `shader`, seen by the rays in `visibility`
    /// ([`ffi::VISIBLE_ALL`] and friends).
    pub fn light(&mut self, kind: &str, shader: Node, matrix: &[f32; 12], visibility: u32) -> Result<Node, String> {
        let node = unsafe { (self.api.fsc_light)(self.raw, c(kind).as_ptr(), shader.0, matrix.as_ptr(), visibility) };
        self.node_or_error(node)
    }

    /// `matrix` is camera to world, looking along +Z with +Y up; `fov` is the
    /// full angle across the image's shorter side.
    pub fn camera(&mut self, matrix: &[f32; 12], fov: f32, width: usize, height: usize) {
        unsafe { (self.api.fsc_camera)(self.raw, matrix.as_ptr(), fov, width as c_int, height as c_int) }
    }

    /// The colour of a shadow catcher's shadow, in the working space: the
    /// images handed back mix towards it by how much the catcher is shadowed,
    /// where Cycles' own shadow is a darkening to black.
    pub fn set_shadow_tint(&mut self, rgb: [f32; 3]) {
        unsafe { (self.api.fsc_set_shadow_tint)(self.raw, rgb[0], rgb[1], rgb[2]) }
    }

    /// Stop the next renders after `seconds`, or never.
    pub fn set_time_limit(&mut self, seconds: Option<f64>) {
        unsafe { (self.api.fsc_set_time_limit)(self.raw, seconds.unwrap_or(0.0)) }
    }

    /// Render and wait. Scene-linear RGBA in Cycles' working space (linear
    /// Rec.709), rows top to bottom, colour premultiplied by alpha.
    /// `progress` hears the fraction done and the samples a pixel has had.
    pub fn render(&mut self, width: usize, height: usize, samples: u32, progress: impl FnMut(f32, u32)) -> Result<Vec<f32>, String> {
        self.render_until(width, height, samples, progress, None)
    }

    /// The same, stopping early with what it has when `cancel` is set.
    pub fn render_until(
        &mut self,
        width: usize,
        height: usize,
        samples: u32,
        mut progress: impl FnMut(f32, u32),
        cancel: Option<&AtomicBool>,
    ) -> Result<Vec<f32>, String> {
        // Progress is reported from Cycles' own thread, which is also the
        // one place that can notice the flag while the render runs.
        let (api, raw) = (self.api, self.raw as usize);
        let mut progress = move |fraction: f32| {
            if cancel.is_some_and(|c| c.load(Ordering::Acquire)) {
                unsafe { (api.fsc_cancel)(raw as *mut ffi::Session) };
            }
            let done = unsafe { (api.fsc_rendered_samples)(raw as *mut ffi::Session) };
            progress(fraction, done.max(0) as u32);
        };
        unsafe extern "C" fn trampoline<F: FnMut(f32)>(user: *mut c_void, fraction: f32) {
            let f = &mut *(user as *mut F);
            f(fraction);
        }
        fn trampoline_for<F: FnMut(f32)>(_: &F) -> unsafe extern "C" fn(*mut c_void, f32) {
            trampoline::<F>
        }
        let callback = trampoline_for(&progress);
        let mut rgba = vec![0.0f32; width * height * 4];
        let user = &mut progress as *mut _ as *mut c_void;
        let ok = unsafe {
            (self.api.fsc_render)(
                self.raw,
                width as c_int,
                height as c_int,
                samples as c_int,
                rgba.as_mut_ptr(),
                Some(callback),
                user,
            )
        };
        self.check(ok)?;
        Ok(rgba)
    }

    /// Start changing a live session's scene. Everything else on the session
    /// may be called until [`Session::end_edit`].
    pub fn begin_edit(&mut self) {
        unsafe { (self.api.fsc_edit_begin)(self.raw) }
    }

    /// Finish the change and restart the image at this size and sample count.
    pub fn end_edit(&mut self, width: usize, height: usize, samples: u32) {
        unsafe { (self.api.fsc_edit_end)(self.raw, width as c_int, height as c_int, samples as c_int) }
    }

    /// Remove every object, mesh, light and shader, keeping the device and its
    /// loaded kernels: cheaper than a new session for a new scene.
    pub fn clear_scene(&mut self) {
        unsafe { (self.api.fsc_clear_scene)(self.raw) }
    }

    /// A live session's newest frame, if it is newer than `seen`: RGBA rows
    /// top to bottom into `frame.rgba`, at the size it was rendered, with its
    /// albedo and normal for a denoiser.
    pub fn latest_frame(&mut self, frame: &mut FrameBuffers, capacity: (usize, usize), seen: &mut u64) -> Option<Frame> {
        let pixels = capacity.0 * capacity.1;
        frame.rgba.resize(pixels * 4, 0.0);
        frame.albedo.resize(pixels * 3, 0.0);
        frame.normal.resize(pixels * 3, 0.0);
        frame.object.resize(pixels, 0.0);
        let (mut width, mut height, mut samples) = (0, 0, 0);
        let got = unsafe {
            (self.api.fsc_live_frame)(
                self.raw,
                frame.rgba.as_mut_ptr(),
                frame.albedo.as_mut_ptr(),
                frame.normal.as_mut_ptr(),
                frame.object.as_mut_ptr(),
                pixels as c_int,
                &mut width,
                &mut height,
                &mut samples,
                seen,
            )
        };
        (got != 0).then(|| {
            let n = width as usize * height as usize;
            frame.rgba.truncate(n * 4);
            frame.albedo.truncate(n * 3);
            frame.normal.truncate(n * 3);
            frame.object.truncate(n);
            Frame { width: width as usize, height: height as usize, samples: samples.max(0) as u32 }
        })
    }

    /// What the session says it is doing, or `Err` with why it stopped.
    pub fn status(&self) -> Result<String, String> {
        let text = unsafe { std::ffi::CStr::from_ptr((self.api.fsc_status)(self.raw)) }
            .to_string_lossy()
            .into_owned();
        match text.strip_prefix("error: ") {
            Some(error) => Err(error.to_string()),
            None => Ok(text),
        }
    }

    pub fn set_paused(&mut self, paused: bool) {
        unsafe { (self.api.fsc_set_paused)(self.raw, paused as c_int) }
    }

    /// Drop everything added, to build a different scene.
    pub fn clear(&mut self) {
        unsafe { (self.api.fsc_clear)(self.raw) }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe { (self.api.fsc_destroy)(self.raw) }
    }
}

#[cfg(test)]
mod bundle_tests {
    use super::*;

    #[test]
    fn a_mac_bundle_compiles_its_kernels_from_its_resources() {
        let root = std::env::temp_dir().join(format!("tequila-cycles-bundle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let contents = root.join("Tequila.app").join("Contents");
        let library = contents.join("Frameworks").join("cycles");
        std::fs::create_dir_all(&library).unwrap();
        // Without the sources, the library's own folder, as on Windows.
        assert_eq!(bundle_kernels(&library), None);
        std::fs::create_dir_all(contents.join("Resources").join("cycles").join("source").join("kernel")).unwrap();
        assert_eq!(bundle_kernels(&library), Some(contents.join("Resources").join("cycles")));
        // Not a bundle: a `cycles` folder beside an executable.
        std::fs::create_dir_all(root.join("Tequila").join("cycles")).unwrap();
        assert_eq!(bundle_kernels(&root.join("Tequila").join("cycles")), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
