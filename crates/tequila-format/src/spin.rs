//! Spin viewers (`docs/03` §8, KeyShotXR's kind): a grid of renders around
//! the product - columns all the way round, rows tilting over it - and a
//! small viewer that turns it under a finger or a mouse.
//!
//! The frames are ordinary renders from generated cameras, queued like any
//! other; this module says where the cameras go and packages the folder:
//! `index.html` and `spin.js` (no library), `manifest.json` describing the
//! grid for anyone embedding it, `manifest.js` holding the same for the page
//! opened from the disk (which may not fetch), and once every frame is in,
//! `sprites.jpg` - every frame small on one sheet, so the viewer turns at
//! once while the full frames load.

use crate::json::Value;
use crate::json_write::Object;
use tequila_core::math::{vec3, Mat4, Vec3};
use tequila_render_core::camera::Camera;
use std::path::Path;

const VIEWER: &str = include_str!("../web/spin.js");
const PAGE: &str = include_str!("../web/spin.html");

/// The width of a frame on the sprite sheet.
pub const SPRITE_WIDTH: usize = 192;

#[derive(Clone, Debug, PartialEq)]
pub struct SpinOptions {
    /// Frames around: 36 is a frame every ten degrees.
    pub columns: usize,
    /// Rows of tilt, centred on the camera's own height.
    pub rows: usize,
    /// Degrees between rows.
    pub tilt: f32,
    /// The product turns clockwise, seen from above, as the frames advance.
    pub clockwise: bool,
    /// `jpg` or `png`; PNG frames can keep the background transparent.
    pub extension: String,
}

impl Default for SpinOptions {
    fn default() -> Self {
        SpinOptions { columns: 36, rows: 1, tilt: 15.0, clockwise: true, extension: "jpg".into() }
    }
}

/// One frame of the grid.
#[derive(Clone, Debug)]
pub struct Frame {
    pub column: usize,
    pub row: usize,
    pub camera: Camera,
    /// File name without extension, under `frames/`.
    pub name: String,
}

/// The cameras of the grid: `base` orbited about `target`, keeping its
/// distance, lens and settings. Row-major, the first row the highest.
pub fn frames(base: &Camera, target: Vec3, options: &SpinOptions) -> Vec<Frame> {
    let columns = options.columns.max(1);
    let rows = options.rows.max(1);
    let eye = base.position();
    let offset = eye - target;
    let distance = offset.length().max(1e-4);
    let yaw0 = offset.x.atan2(offset.z);
    let pitch0 = (offset.y / distance).clamp(-1.0, 1.0).asin();
    let sign = if options.clockwise { -1.0 } else { 1.0 };
    let mut out = Vec::with_capacity(columns * rows);
    for row in 0..rows {
        let lift = (rows as f32 - 1.0) * 0.5 - row as f32;
        let pitch = (pitch0 + (lift * options.tilt).to_radians()).clamp(-85f32.to_radians(), 85f32.to_radians());
        for column in 0..columns {
            let yaw = yaw0 + sign * column as f32 * std::f32::consts::TAU / columns as f32;
            let eye = target + vec3(pitch.cos() * yaw.sin(), pitch.sin(), pitch.cos() * yaw.cos()) * distance;
            let mut camera = base.clone();
            camera.camera_to_world = Mat4::look_at(eye, target, Vec3::Y);
            camera.focus_distance = distance;
            out.push(Frame { column, row, camera, name: format!("frame_r{row:02}_c{column:03}") });
        }
    }
    out
}

/// The grid's description, as the viewer reads it.
pub fn manifest(title: &str, options: &SpinOptions, width: usize, height: usize, sprites: Option<(usize, usize)>) -> Value {
    let mut frames = Vec::new();
    for row in 0..options.rows.max(1) {
        for column in 0..options.columns.max(1) {
            frames.push(Value::from(format!("frames/frame_r{row:02}_c{column:03}.{}", options.extension)));
        }
    }
    let mut out = Object::new();
    out.set("title", title)
        .set("columns", options.columns.max(1))
        .set("rows", options.rows.max(1))
        .set("width", width)
        .set("height", height)
        .set("frames", frames)
        .set("start_row", (options.rows.max(1) - 1) / 2);
    if let Some((w, h)) = sprites {
        let mut s = Object::new();
        s.set("src", "sprites.jpg").set("width", w).set("height", h);
        out.set("sprites", s.build());
    }
    out.build()
}

/// Write the viewer, the page and the manifests into `folder`. Frames go
/// in `folder/frames`, rendered separately.
pub fn write_package(folder: &Path, title: &str, options: &SpinOptions, width: usize, height: usize) -> std::io::Result<()> {
    std::fs::create_dir_all(folder.join("frames"))?;
    let page = PAGE.replace("{{title}}", &title.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;"));
    std::fs::write(folder.join("index.html"), page)?;
    std::fs::write(folder.join("spin.js"), VIEWER)?;
    write_manifests(folder, &manifest(title, options, width, height, None))?;
    std::fs::write(
        folder.join("EMBED.txt"),
        "Tequila spin viewer\n\
         \n\
         Open index.html to try it. To put it on a page, either frame the whole folder:\n\
         \n\
         \x20   <iframe src=\"spin/index.html\" width=\"800\" height=\"600\" style=\"border:0\" allowfullscreen></iframe>\n\
         \n\
         or use the element, with the folder on the same site:\n\
         \n\
         \x20   <script src=\"spin/spin.js\"></script>\n\
         \x20   <tequila-spin src=\"spin/manifest.json\" style=\"width:800px;height:600px\"></tequila-spin>\n\
         \n\
         Drag to turn, drag up and down to tilt when there are rows, scroll or pinch to zoom, double-click to reset.\n",
    )?;
    Ok(())
}

fn write_manifests(folder: &Path, manifest: &Value) -> std::io::Result<()> {
    let text = crate::json_write::to_string(manifest);
    std::fs::write(folder.join("manifest.json"), &text)?;
    std::fs::write(folder.join("manifest.js"), format!("window.TEQUILA_SPIN = {text};\n"))
}

/// Once every frame is rendered: put them small on one sheet and tell the
/// manifests. `false` while frames are still missing.
pub fn finish(folder: &Path, title: &str, options: &SpinOptions, width: usize, height: usize) -> Result<bool, String> {
    let columns = options.columns.max(1);
    let rows = options.rows.max(1);
    let (w, h) = (SPRITE_WIDTH, ((SPRITE_WIDTH as f32 * height as f32 / width.max(1) as f32).round() as usize).max(1));
    let mut sheet = vec![0u8; columns * w * rows * h * 3];
    let stride = columns * w * 3;
    for row in 0..rows {
        for column in 0..columns {
            let path = folder.join(format!("frames/frame_r{row:02}_c{column:03}.{}", options.extension));
            if !path.exists() {
                return Ok(false);
            }
            let image = tequila_texture::image::Image::load(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            // A box filter down to the sprite, then sRGB.
            for y in 0..h {
                for x in 0..w {
                    let (x0, x1) = (x * image.width / w, ((x + 1) * image.width / w).max(x * image.width / w + 1));
                    let (y0, y1) = (y * image.height / h, ((y + 1) * image.height / h).max(y * image.height / h + 1));
                    let mut sum = [0.0f32; 3];
                    let mut n = 0.0f32;
                    for sy in y0..y1.min(image.height) {
                        for sx in x0..x1.min(image.width) {
                            let i = (sy * image.width + sx) * image.channels;
                            let alpha = if image.channels == 4 { image.data[i + 3] } else { 1.0 };
                            for k in 0..3 {
                                // Transparent frames go on white.
                                sum[k] += image.data[i + k.min(image.channels - 1)] * alpha + (1.0 - alpha);
                            }
                            n += 1.0;
                        }
                    }
                    let at = (row * h + y) * stride + (column * w + x) * 3;
                    for k in 0..3 {
                        let v = (sum[k] / n.max(1.0)).clamp(0.0, 1.0);
                        let e = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
                        sheet[at + k] = (e * 255.0 + 0.5) as u8;
                    }
                }
            }
        }
    }
    jpeg_encoder::Encoder::new_file(folder.join("sprites.jpg"), 82)
        .map_err(|e| e.to_string())?
        .encode(&sheet, (columns * w) as u16, (rows * h) as u16, jpeg_encoder::ColorType::Rgb)
        .map_err(|e| e.to_string())?;
    write_manifests(folder, &manifest(title, options, width, height, Some((w, h)))).map_err(|e| e.to_string())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The grid goes all the way round at the camera's distance, each row
    /// tilted by the step, and always looks at the target.
    #[test]
    fn the_cameras_orbit_the_target() {
        let target = vec3(0.0, 0.1, 0.0);
        let base = Camera::look_at(vec3(0.0, 0.1, 1.0), target, Vec3::Y);
        let options = SpinOptions { columns: 8, rows: 3, tilt: 20.0, ..Default::default() };
        let grid = frames(&base, target, &options);
        assert_eq!(grid.len(), 24);
        for f in &grid {
            let eye = f.camera.position();
            assert!(((eye - target).length() - 1.0).abs() < 1e-4);
            assert!((f.camera.forward() - (target - eye).normalize()).length() < 1e-4, "looks at the target");
        }
        // The middle row is the camera's own height, the first above it.
        assert!((grid[8].camera.position().y - 0.1).abs() < 1e-4);
        assert!(grid[0].camera.position().y > 0.1 + 0.3);
        // A quarter of the way round is a quarter turn.
        let quarter = grid[8 + 2].camera.position() - target;
        assert!(quarter.z.abs() < 1e-4 && (quarter.x.abs() - 1.0).abs() < 1e-4, "{quarter:?}");
        assert_eq!(grid[8 + 2].name, "frame_r01_c002");
    }

    #[test]
    fn the_manifest_lists_every_frame_row_by_row() {
        let options = SpinOptions { columns: 4, rows: 2, extension: "png".into(), ..Default::default() };
        let m = manifest("Kettle", &options, 800, 600, Some((192, 144)));
        let frames = m.get("frames").unwrap().as_array();
        assert_eq!(frames.len(), 8);
        assert_eq!(frames[5].as_str(), Some("frames/frame_r01_c001.png"));
        assert_eq!(m.get("start_row").and_then(|v| v.as_index()), Some(0));
        assert_eq!(m.get("sprites").and_then(|s| s.get("src")).and_then(|v| v.as_str()), Some("sprites.jpg"));
    }
}
