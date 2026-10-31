//! Wireframe renders: the scene's edges drawn rather than traced (the
//! scene crate's `RenderKind::Wireframe`, which gathers the lines).
//!
//! Each line is walked across the image half a pixel at a time, and at every
//! step a ray asks the scene whether anything lies between that point and the
//! camera. So the stretches in front and behind come out exact at any size of
//! image, where a depth buffer would have to guess about edges lying on the
//! very surfaces it holds. Each stretch is then drawn at the width asked for,
//! its coverage of a pixel worked out from its distance to the pixel's
//! centre, which makes the lines smooth with no samples to take.

use rayon::prelude::*;
use tequila_core::math::{Mat4, Ray, Vec3};
use tequila_render_core::camera::{Camera, Projection};
use tequila_render_core::intersect::Intersector;

/// Each pixel's coverage by the lines in front and by those behind, 0 to 1.
#[derive(Debug)]
pub struct Coverage {
    pub width: usize,
    pub height: usize,
    pub front: Vec<f32>,
    pub behind: Vec<f32>,
    /// How many stretches were drawn, for the report.
    pub strokes: usize,
}

/// A stretch of a line on the image, in pixels, and whether it is in front.
#[derive(Clone, Copy, Debug)]
struct Stroke {
    a: [f32; 2],
    b: [f32; 2],
    front: bool,
}

/// How far apart, on the image, a line is asked about: under a pixel, so a
/// line passing behind something changes from strong to faint where it does.
const STEP_PX: f32 = 0.5;
/// The square of pixels drawn together, each on a thread of its own.
const TILE: usize = 32;
/// The most steps one line takes, however long it is on the image.
const MOST_STEPS: usize = 1 << 16;

/// The camera's projection, set up once for every point.
struct Lens {
    to_camera: Mat4,
    orthographic: bool,
    half_width: f32,
    half_height: f32,
    shift: (f32, f32),
    size: (f32, f32),
    eye: Vec3,
    forward: Vec3,
    /// The nearest distance a line is drawn at.
    near: f32,
}

impl Lens {
    fn of(camera: &Camera, width: usize, height: usize, extent: f32) -> Result<Lens, String> {
        let aspect = width as f32 / height as f32;
        let to_camera = camera.camera_to_world.inverse().ok_or("the camera's placement cannot be inverted")?;
        let (orthographic, half_height, shift) = match camera.projection {
            Projection::Perspective { .. } => (false, (camera.vertical_fov(aspect) * 0.5).tan(), (0.0, 0.0)),
            Projection::Shift { shift_x, shift_y, .. } => (false, (camera.vertical_fov(aspect) * 0.5).tan(), (shift_x, shift_y)),
            Projection::Orthographic { scale } => (true, scale * 0.5, (0.0, 0.0)),
            Projection::Equirectangular => return Err("a 360° camera has no wireframe: pick a perspective or orthographic one".into()),
        };
        Ok(Lens {
            to_camera,
            orthographic,
            half_width: half_height * aspect,
            half_height,
            shift,
            size: (width as f32, height as f32),
            eye: camera.position(),
            forward: camera.forward(),
            near: (extent * 1e-5).max(1e-6),
        })
    }

    /// Distance in front of the camera, along its view.
    fn depth(&self, local: Vec3) -> f32 {
        -local.z
    }

    /// Where a point in the camera's space lands on the image, in pixels.
    fn raster(&self, local: Vec3) -> [f32; 2] {
        let (sx, sy) = if self.orthographic {
            (local.x / self.half_width, local.y / self.half_height)
        } else {
            let depth = self.depth(local);
            (
                local.x / (depth * self.half_width) - 2.0 * self.shift.0,
                local.y / (depth * self.half_height) - 2.0 * self.shift.1,
            )
        };
        [(sx + 1.0) * 0.5 * self.size.0, (1.0 - sy) * 0.5 * self.size.1]
    }

    /// Whether the camera sees a point, or something stands in the way.
    fn sees(&self, intersector: &dyn Intersector, point: Vec3) -> bool {
        let (origin, direction, distance) = if self.orthographic {
            let distance = (point - self.eye).dot(self.forward);
            (point - self.forward * distance, self.forward, distance)
        } else {
            let to = point - self.eye;
            let distance = to.length();
            if distance <= 0.0 {
                return true;
            }
            (self.eye, to / distance, distance)
        };
        // Short of the point by a hair: the point is on its own faces, and a
        // ray reaching it meets them there, which is no reason to hide it.
        let ray = Ray { origin, direction, t_min: 0.0, t_max: distance * (1.0 - 1e-4), time: 0.0 };
        intersector.intersect(&ray).is_none()
    }
}

/// Draw `lines`, world-space pairs of points, as the camera sees them on an
/// image of `width` by `height`, `line_width` pixels wide. With `behind`, the
/// stretches the scene hides are drawn too, into their own coverage, for
/// the caller to fade.
pub fn draw(
    intersector: &dyn Intersector,
    camera: &Camera,
    width: usize,
    height: usize,
    lines: &[[Vec3; 2]],
    line_width: f32,
    behind: bool,
) -> Result<Coverage, String> {
    if width == 0 || height == 0 {
        return Err("a wireframe needs an image at least a pixel across".into());
    }
    let extent = intersector.scene().bounds().diagonal().length().max(1e-6);
    let lens = Lens::of(camera, width, height, extent)?;
    let line_width = line_width.max(0.05);
    let margin = line_width * 0.5 + 2.0;

    let strokes: Vec<Stroke> = lines
        .par_iter()
        .flat_map_iter(|line| strokes_of(&lens, intersector, line, margin, behind))
        .collect();

    // Each stroke listed in the tiles it reaches, then each tile drawn on
    // its own, so no two threads write a pixel.
    let tiles_x = width.div_ceil(TILE);
    let tiles_y = height.div_ceil(TILE);
    let mut bins: Vec<Vec<u32>> = vec![Vec::new(); tiles_x * tiles_y];
    let reach = line_width.max(1.0) * 0.5 + 1.0;
    for (index, stroke) in strokes.iter().enumerate() {
        let low = |a: f32, b: f32| (a.min(b) - reach).floor().max(0.0) as usize;
        let high = |a: f32, b: f32, limit: usize| ((a.max(b) + reach).ceil().max(0.0) as usize).min(limit.saturating_sub(1));
        let (x0, x1) = (low(stroke.a[0], stroke.b[0]), high(stroke.a[0], stroke.b[0], width));
        let (y0, y1) = (low(stroke.a[1], stroke.b[1]), high(stroke.a[1], stroke.b[1], height));
        if x0 > x1 || y0 > y1 {
            continue;
        }
        for ty in y0 / TILE..=y1 / TILE {
            for tx in x0 / TILE..=x1 / TILE {
                bins[ty * tiles_x + tx].push(index as u32);
            }
        }
    }

    let drawn: Vec<(usize, Vec<f32>, Vec<f32>)> = bins
        .par_iter()
        .enumerate()
        .filter(|(_, bin)| !bin.is_empty())
        .map(|(tile, bin)| {
            let (tx, ty) = (tile % tiles_x, tile / tiles_x);
            let (x0, y0) = (tx * TILE, ty * TILE);
            let (w, h) = (TILE.min(width - x0), TILE.min(height - y0));
            let mut front = vec![0.0f32; w * h];
            let mut back = vec![0.0f32; w * h];
            for &index in bin {
                let stroke = strokes[index as usize];
                let target = if stroke.front { &mut front } else { &mut back };
                stamp(target, x0, y0, w, h, &stroke, line_width);
            }
            (tile, front, back)
        })
        .collect();

    let mut coverage = Coverage { width, height, front: vec![0.0; width * height], behind: vec![0.0; width * height], strokes: strokes.len() };
    for (tile, front, back) in drawn {
        let (x0, y0) = ((tile % tiles_x) * TILE, (tile / tiles_x) * TILE);
        let w = TILE.min(width - x0);
        for (row, (f, b)) in front.chunks_exact(w).zip(back.chunks_exact(w)).enumerate() {
            let at = (y0 + row) * width + x0;
            coverage.front[at..at + w].copy_from_slice(f);
            coverage.behind[at..at + w].copy_from_slice(b);
        }
    }
    Ok(coverage)
}

/// One line's stretches in front and behind, as the image shows them.
fn strokes_of(lens: &Lens, intersector: &dyn Intersector, line: &[Vec3; 2], margin: f32, behind: bool) -> Vec<Stroke> {
    let (mut a, mut b) = (line[0], line[1]);
    let (mut la, mut lb) = (lens.to_camera.transform_point(a), lens.to_camera.transform_point(b));
    // Only what is in front of the camera: a line reaching behind it is cut
    // where it passes the nearest distance drawn.
    let (da, db) = (lens.depth(la), lens.depth(lb));
    if da < lens.near && db < lens.near {
        return Vec::new();
    }
    if da < lens.near || db < lens.near {
        let t = (lens.near - da) / (db - da);
        let cut = a + (b - a) * t;
        let local = la + (lb - la) * t;
        if da < lens.near {
            (a, la) = (cut, local);
        } else {
            (b, lb) = (cut, local);
        }
    }
    let (ra, rb) = (lens.raster(la), lens.raster(lb));
    // Only the part on the image, and a little either side for the width.
    let Some((s0, s1)) = clip(ra, rb, [-margin, -margin], [lens.size.0 + margin, lens.size.1 + margin]) else {
        return Vec::new();
    };
    let on_image = ((rb[0] - ra[0]).powi(2) + (rb[1] - ra[1]).powi(2)).sqrt() * (s1 - s0);
    let steps = ((on_image / STEP_PX).ceil() as usize).clamp(1, MOST_STEPS);
    let (za, zb) = (lens.depth(la), lens.depth(lb));
    let at = |s: f32| {
        // Evenly along the image is not evenly along a line in perspective:
        // its inverse depth is what runs evenly.
        let t = if lens.orthographic { s } else { s * za / ((1.0 - s) * zb + s * za) };
        let point = a + (b - a) * t;
        let screen = [ra[0] + (rb[0] - ra[0]) * s, ra[1] + (rb[1] - ra[1]) * s];
        (screen, lens.sees(intersector, point))
    };

    let mut strokes = Vec::new();
    let (mut start, mut seen) = at(s0);
    let mut previous = start;
    for step in 1..=steps {
        let (screen, now) = at(s0 + (s1 - s0) * step as f32 / steps as f32);
        if now != seen {
            let middle = [(previous[0] + screen[0]) * 0.5, (previous[1] + screen[1]) * 0.5];
            if seen || behind {
                strokes.push(Stroke { a: start, b: middle, front: seen });
            }
            start = middle;
            seen = now;
        }
        previous = screen;
    }
    if seen || behind {
        strokes.push(Stroke { a: start, b: previous, front: seen });
    }
    strokes
}

/// The stretch of the segment from `a` to `b` inside the box from `low` to
/// `high`, as its parameters from 0 at `a` to 1 at `b` (Liang and Barsky).
fn clip(a: [f32; 2], b: [f32; 2], low: [f32; 2], high: [f32; 2]) -> Option<(f32, f32)> {
    let (mut s0, mut s1) = (0.0f32, 1.0f32);
    for axis in 0..2 {
        let d = b[axis] - a[axis];
        for (p, q) in [(-d, a[axis] - low[axis]), (d, high[axis] - a[axis])] {
            if p == 0.0 {
                if q < 0.0 {
                    return None;
                }
            } else {
                let r = q / p;
                if p < 0.0 {
                    s0 = s0.max(r);
                } else {
                    s1 = s1.min(r);
                }
            }
        }
    }
    (s0 <= s1).then_some((s0, s1))
}

/// Draw one stroke into a tile's coverage, keeping each pixel's strongest:
/// lines meeting at a corner do not darken it twice.
fn stamp(target: &mut [f32], x0: usize, y0: usize, w: usize, h: usize, stroke: &Stroke, line_width: f32) {
    // A line under a pixel wide is drawn a pixel wide and that much fainter,
    // which is how it would look sampled.
    let drawn = line_width.max(1.0);
    let thin = line_width.min(1.0);
    let half = drawn * 0.5;
    let reach = half + 1.0;
    let (ax, ay) = (stroke.a[0], stroke.a[1]);
    let (dx, dy) = (stroke.b[0] - ax, stroke.b[1] - ay);
    let length2 = dx * dx + dy * dy;
    let left = ((ax.min(stroke.b[0]) - reach).floor() as isize - x0 as isize).max(0) as usize;
    let right = ((ax.max(stroke.b[0]) + reach).ceil() as isize - x0 as isize).clamp(0, w as isize - 1) as usize;
    let top = ((ay.min(stroke.b[1]) - reach).floor() as isize - y0 as isize).max(0) as usize;
    let bottom = ((ay.max(stroke.b[1]) + reach).ceil() as isize - y0 as isize).clamp(0, h as isize - 1) as usize;
    if left > right || top > bottom {
        return;
    }
    for y in top..=bottom {
        let cy = (y0 + y) as f32 + 0.5;
        for x in left..=right {
            let cx = (x0 + x) as f32 + 0.5;
            let along = if length2 > 0.0 { (((cx - ax) * dx + (cy - ay) * dy) / length2).clamp(0.0, 1.0) } else { 0.0 };
            let (px, py) = (ax + dx * along - cx, ay + dy * along - cy);
            let distance = (px * px + py * py).sqrt();
            let cover = (half + 0.5 - distance).clamp(0.0, 1.0) * thin;
            let pixel = &mut target[y * w + x];
            if cover > *pixel {
                *pixel = cover;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_segment_is_clipped_to_the_box() {
        assert_eq!(clip([-10.0, 5.0], [10.0, 5.0], [0.0, 0.0], [10.0, 10.0]), Some((0.5, 1.0)));
        assert_eq!(clip([-10.0, 50.0], [10.0, 50.0], [0.0, 0.0], [10.0, 10.0]), None);
        let (s0, s1) = clip([-5.0, -5.0], [15.0, 15.0], [0.0, 0.0], [10.0, 10.0]).unwrap();
        assert!((s0 - 0.25).abs() < 1e-6 && (s1 - 0.75).abs() < 1e-6);
    }

    #[test]
    fn a_line_is_as_wide_as_asked_and_smooth_at_its_sides() {
        // A horizontal stroke through the middle of a tile, three pixels wide:
        // full in the three rows it covers, nothing beyond, and a corner met
        // twice no stronger than once.
        let (w, h) = (32, 32);
        let mut tile = vec![0.0; w * h];
        let stroke = Stroke { a: [2.0, 16.5], b: [30.0, 16.5], front: true };
        stamp(&mut tile, 0, 0, w, h, &stroke, 3.0);
        stamp(&mut tile, 0, 0, w, h, &Stroke { a: [16.5, 16.5], b: [16.5, 30.0], front: true }, 3.0);
        let column: Vec<f32> = (10..23).map(|y| tile[y * w + 10]).collect();
        let full = column.iter().filter(|c| **c > 0.99).count();
        assert_eq!(full, 3, "{column:?}");
        assert!(column.iter().all(|c| *c <= 1.0));
        assert_eq!(tile[5 * w + 10], 0.0);
        let thin = {
            let mut tile = vec![0.0; w * h];
            stamp(&mut tile, 0, 0, w, h, &Stroke { a: [2.0, 16.5], b: [30.0, 16.5], front: true }, 0.5);
            tile[16 * w + 10]
        };
        assert!((thin - 0.5).abs() < 1e-6, "a half-pixel line through a pixel's centre covers half of it: {thin}");
    }
}
