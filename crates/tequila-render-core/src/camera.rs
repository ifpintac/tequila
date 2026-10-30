//! The camera model of `docs/01-RENDERER-CORE.md` §8.
//!
//! Parameterized photographically, not in renderer units. Users of this
//! application think in millimetres and f-stops, and §8.1 is explicit that the
//! coupling `f_number -> aperture_radius -> blur` must be physical: "users of
//! this application think in photographic terms and will notice if f/2.8 does
//! not behave like f/2.8".

use tequila_core::math::{vec2, vec3, Mat4, Ray, Vec2, Vec3};
use tequila_core::sampler::Sampler;
use tequila_core::sampling::{sample_polygon_aperture, sample_uniform_disk_concentric};

/// Full-frame sensor width, the reference every focal length is quoted
/// against.
pub const FULL_FRAME_SENSOR_WIDTH_MM: f32 = 36.0;

/// Projection model. `docs/01` §8.2 and `TEQUILA_SPEC.md` §1.7.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Projection {
    /// Thin-lens perspective. The default.
    Perspective { focal_length_mm: f32 },
    /// Parallel projection; `scale` is the vertical extent in scene units.
    Orthographic { scale: f32 },
    /// Perspective with a film-plane offset, in fractions of the sensor size.
    /// Keeps verticals parallel, which architectural and packaging work needs.
    Shift {
        focal_length_mm: f32,
        shift_x: f32,
        shift_y: f32,
    },
    /// Equirectangular 360 projection, for environment capture and VR.
    Equirectangular,
}

impl Default for Projection {
    fn default() -> Self {
        Projection::Perspective {
            focal_length_mm: 50.0,
        }
    }
}

/// Aperture shape. Real lenses have polygonal irises, and the resulting bokeh
/// shape is a look choice product photographers make deliberately.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Aperture {
    #[default]
    Circular,
    /// 3 or more straight blades.
    Bladed { blades: u32, rotation: f32 },
}

/// A camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// Camera-to-world transform: `-Z` forward, `+Y` up, `+X` right.
    pub camera_to_world: Mat4,
    pub projection: Projection,
    pub sensor_width_mm: f32,
    /// f-number. `f32::INFINITY` means a pinhole: everything in focus.
    pub f_number: f32,
    pub focus_distance: f32,
    pub aperture: Aperture,
    /// Shutter interval in frame-relative time, for motion blur (§8.4).
    pub shutter_open: f32,
    pub shutter_close: f32,
    /// ISO and shutter speed, feeding the physical exposure of §8.3.
    pub iso: f32,
    pub shutter_speed_seconds: f32,
    /// When set, the focus distance is re-derived from the camera-target
    /// distance whenever the camera moves — KeyShot's "Keep Focus" (§8.1).
    pub keep_focus: bool,
    /// Expose by ISO, shutter and f-number as a camera would
    /// ([`Camera::exposure_scale`]), on top of the image's own exposure. Off,
    /// they change nothing, which is how every camera before them renders.
    pub physical_exposure: bool,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            camera_to_world: Mat4::IDENTITY,
            projection: Projection::default(),
            sensor_width_mm: FULL_FRAME_SENSOR_WIDTH_MM,
            f_number: f32::INFINITY,
            focus_distance: 1.0,
            aperture: Aperture::Circular,
            shutter_open: 0.0,
            shutter_close: 0.0,
            iso: 100.0,
            shutter_speed_seconds: 1.0 / 100.0,
            keep_focus: false,
            physical_exposure: false,
        }
    }
}

impl Camera {
    pub fn look_at(eye: Vec3, target: Vec3, up: Vec3) -> Self {
        Camera {
            camera_to_world: Mat4::look_at(eye, target, up),
            focus_distance: (target - eye).length().max(1e-4),
            ..Default::default()
        }
    }

    #[inline]
    pub fn position(&self) -> Vec3 {
        self.camera_to_world.transform_point(Vec3::ZERO)
    }

    #[inline]
    pub fn forward(&self) -> Vec3 {
        -self.camera_to_world.transform_vector(Vec3::Z)
    }

    /// Sensor height derived from the width and the image aspect ratio. The
    /// sensor is what physically sets the field of view, so a resolution
    /// change must not silently change the framing.
    #[inline]
    pub fn sensor_height_mm(&self, aspect: f32) -> f32 {
        self.sensor_width_mm / aspect.max(1e-6)
    }

    /// Vertical field of view in radians.
    pub fn vertical_fov(&self, aspect: f32) -> f32 {
        match self.projection {
            Projection::Perspective { focal_length_mm }
            | Projection::Shift {
                focal_length_mm, ..
            } => 2.0 * (self.sensor_height_mm(aspect) / (2.0 * focal_length_mm)).atan(),
            Projection::Orthographic { .. } => 0.0,
            Projection::Equirectangular => std::f32::consts::PI,
        }
    }

    /// Set the focal length from a desired vertical field of view. The Camera
    /// tab exposes both and keeps them inversely coupled (`TEQUILA_SPEC.md`
    /// §1.7).
    pub fn set_vertical_fov(&mut self, fov: f32, aspect: f32) {
        let focal = self.sensor_height_mm(aspect) / (2.0 * (fov * 0.5).tan());
        match &mut self.projection {
            Projection::Perspective { focal_length_mm }
            | Projection::Shift {
                focal_length_mm, ..
            } => *focal_length_mm = focal.clamp(1.0, 2000.0),
            _ => {}
        }
    }

    /// Aperture radius in scene units, from the physical relationship
    /// `radius = focal_length / (2 * N)`.
    ///
    /// The focal length is converted from millimetres via the *sensor* scale
    /// rather than being treated as a scene length: what governs the blur
    /// circle is the ratio of aperture to sensor, and the sensor's size in the
    /// scene is set by the focus distance and field of view.
    pub fn aperture_radius(&self, aspect: f32) -> f32 {
        if !self.f_number.is_finite() || self.f_number <= 0.0 {
            return 0.0;
        }
        match self.projection {
            Projection::Perspective { focal_length_mm }
            | Projection::Shift {
                focal_length_mm, ..
            } => {
                let radius_mm = focal_length_mm / (2.0 * self.f_number);
                // Scene units per millimetre of sensor, at the focus plane.
                let sensor_to_scene = self.sensor_height_mm(aspect).recip()
                    * 2.0
                    * self.focus_distance
                    * (self.vertical_fov(aspect) * 0.5).tan();
                radius_mm * sensor_to_scene
            }
            _ => 0.0,
        }
    }

    /// Exposure scale applied to radiance, from `EV = log2(N^2 / t) -
    /// log2(ISO / 100)` (§8.3).
    ///
    /// Deriving it this way rather than from a raw multiplier is what makes
    /// depth of field and brightness respond together the way a real camera
    /// does: stopping down darkens the image exactly as much as it deepens
    /// the focus.
    pub fn exposure_scale(&self) -> f32 {
        if !self.f_number.is_finite() {
            // A pinhole has no aperture to speak of; expose from ISO and
            // shutter alone so that switching DOF off does not change
            // brightness.
            return self.iso / 100.0;
        }
        let ev = (self.f_number * self.f_number / self.shutter_speed_seconds).log2()
            - (self.iso / 100.0).log2();
        // The 1/16 calibration constant is the standard "K = 12.5" reflected
        // -light meter convention rearranged for scene-linear radiance.
        f32::exp2(-ev) * 16.0
    }

    /// The stops this camera adds to the image's exposure: its
    /// [`Camera::exposure_scale`] when it exposes physically, none otherwise.
    pub fn exposure_stops(&self) -> f32 {
        if self.physical_exposure {
            self.exposure_scale().log2()
        } else {
            0.0
        }
    }

    /// A deterministic ray for picking and for the gizmo.
    ///
    /// The same construction as [`Self::generate_ray`], but through the centre
    /// of the aperture rather than a sampled point on it. Picking has to be
    /// repeatable: with a shallow depth of field, two clicks in the same place
    /// with jittered lens samples would select two different parts.
    pub fn pick_ray(&self, film: Vec2, aspect: f32) -> Ray {
        let mut sampler = Sampler::new(0, 0, 0, 0);
        // A pinhole copy: the lens radius is what the jitter would act on, so
        // removing it removes the jitter without duplicating the ray maths.
        let pinhole = Camera {
            f_number: f32::INFINITY,
            ..*self
        };
        pinhole.generate_ray(film, aspect, &mut sampler)
    }

    /// Where a world-space point lands on the film, in the same `(0,0)`
    /// top-left to `(1,1)` bottom-right coordinates [`Self::generate_ray`]
    /// takes. `None` when the point is behind the camera.
    ///
    /// The inverse of the pinhole ray construction, and only defined for the
    /// projections that have one: an equirectangular camera sees in every
    /// direction, so "behind" is not a thing and the gizmo does not run there.
    pub fn project(&self, world: Vec3, aspect: f32) -> Option<Vec2> {
        let local = self.camera_to_world.inverse()?.transform_point(world);
        match self.projection {
            Projection::Perspective { .. } | Projection::Shift { .. } => {
                if local.z >= -1e-6 {
                    return None;
                }
                let (shift_x, shift_y) = match self.projection {
                    Projection::Shift {
                        shift_x, shift_y, ..
                    } => (shift_x, shift_y),
                    _ => (0.0, 0.0),
                };
                let half_height = (self.vertical_fov(aspect) * 0.5).tan();
                let half_width = half_height * aspect;
                let sx = local.x / (-local.z * half_width) - 2.0 * shift_x;
                let sy = local.y / (-local.z * half_height) - 2.0 * shift_y;
                Some(vec2((sx + 1.0) * 0.5, (1.0 - sy) * 0.5))
            }
            Projection::Orthographic { scale } => {
                let half_height = scale * 0.5;
                let half_width = half_height * aspect;
                let sx = local.x / half_width;
                let sy = local.y / half_height;
                Some(vec2((sx + 1.0) * 0.5, (1.0 - sy) * 0.5))
            }
            Projection::Equirectangular => None,
        }
    }

    /// Generate a primary ray for a film point.
    ///
    /// `film` is in normalized device coordinates: `(0,0)` is the top-left
    /// corner of the image, `(1,1)` the bottom-right. The pixel filter has
    /// already been applied by the caller (§4.5 filter importance sampling),
    /// so this maps a point, not a pixel.
    pub fn generate_ray(&self, film: Vec2, aspect: f32, sampler: &mut Sampler) -> Ray {
        // NDC to a symmetric [-1, 1] frame with +Y up.
        let sx = 2.0 * film.x - 1.0;
        let sy = 1.0 - 2.0 * film.y;

        let (origin, direction) = match self.projection {
            Projection::Perspective { .. } | Projection::Shift { .. } => {
                let (shift_x, shift_y) = match self.projection {
                    Projection::Shift {
                        shift_x, shift_y, ..
                    } => (shift_x, shift_y),
                    _ => (0.0, 0.0),
                };
                let half_height = (self.vertical_fov(aspect) * 0.5).tan();
                let half_width = half_height * aspect;
                let dir = vec3(
                    (sx + 2.0 * shift_x) * half_width,
                    (sy + 2.0 * shift_y) * half_height,
                    -1.0,
                );

                let radius = self.aperture_radius(aspect);
                if radius <= 0.0 {
                    (Vec3::ZERO, dir.normalize())
                } else {
                    // Thin lens: the ray must still pass through the point
                    // where the pinhole ray meets the focus plane, so that
                    // objects at the focus distance stay sharp regardless of
                    // where on the lens the ray started.
                    let lens_uv = sampler.lens();
                    let lens = self.sample_aperture(lens_uv) * radius;
                    let t_focus = self.focus_distance / -dir.z;
                    let focal_point = dir * t_focus;
                    let origin = vec3(lens.x, lens.y, 0.0);
                    (origin, (focal_point - origin).normalize())
                }
            }
            Projection::Orthographic { scale } => {
                let half_height = scale * 0.5;
                let half_width = half_height * aspect;
                (
                    vec3(sx * half_width, sy * half_height, 0.0),
                    vec3(0.0, 0.0, -1.0),
                )
            }
            Projection::Equirectangular => {
                let phi = sx * std::f32::consts::PI;
                // `film.y = 0` is the top of the image, which is the zenith:
                // theta measures down from +Y, so it maps straight through.
                let theta = film.y * std::f32::consts::PI;
                let sin_theta = theta.sin();
                (
                    Vec3::ZERO,
                    vec3(sin_theta * phi.sin(), theta.cos(), -sin_theta * phi.cos()),
                )
            }
        };

        let time = if self.shutter_close > self.shutter_open {
            self.shutter_open + sampler.time() * (self.shutter_close - self.shutter_open)
        } else {
            self.shutter_open
        };

        Ray {
            origin: self.camera_to_world.transform_point(origin),
            direction: self.camera_to_world.transform_vector(direction).normalize(),
            t_min: 0.0,
            t_max: f32::INFINITY,
            time,
        }
    }

    fn sample_aperture(&self, u: Vec2) -> Vec2 {
        match self.aperture {
            Aperture::Circular => sample_uniform_disk_concentric(u),
            Aperture::Bladed { blades, rotation } => {
                sample_polygon_aperture(u, blades.max(3), rotation)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec2;

    fn sampler() -> Sampler {
        Sampler::new(0, 0, 0, 1)
    }

    #[test]
    fn fifty_millimetres_on_full_frame_gives_the_expected_field_of_view() {
        let cam = Camera {
            projection: Projection::Perspective {
                focal_length_mm: 50.0,
            },
            ..Default::default()
        };
        // Horizontal FOV on a 36mm sensor at 50mm is ~39.6 degrees.
        let aspect = 3.0 / 2.0;
        let vfov = cam.vertical_fov(aspect);
        let hfov = 2.0 * ((vfov * 0.5).tan() * aspect).atan();
        assert!(
            (hfov.to_degrees() - 39.6).abs() < 0.5,
            "hfov = {}",
            hfov.to_degrees()
        );
    }

    #[test]
    fn focal_length_and_fov_are_inverses() {
        let mut cam = Camera::default();
        let aspect = 16.0 / 9.0;
        for target in [20.0f32, 45.0, 90.0] {
            cam.set_vertical_fov(target.to_radians(), aspect);
            let back = cam.vertical_fov(aspect).to_degrees();
            assert!((back - target).abs() < 0.01, "{target} -> {back}");
        }
    }

    #[test]
    fn centre_of_frame_looks_straight_ahead() {
        let cam = Camera::look_at(vec3(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
        let ray = cam.generate_ray(vec2(0.5, 0.5), 1.0, &mut sampler());
        assert!((ray.origin - vec3(0.0, 0.0, 5.0)).length() < 1e-5);
        assert!(
            (ray.direction - vec3(0.0, 0.0, -1.0)).length() < 1e-5,
            "{:?}",
            ray.direction
        );
    }

    /// Screen orientation: `film.y = 0` is the top of the image, so it must
    /// map to a ray pointing *up*. Getting this backwards flips every render.
    #[test]
    fn film_coordinates_are_top_left_origin() {
        let cam = Camera::look_at(vec3(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
        let top = cam.generate_ray(vec2(0.5, 0.0), 1.0, &mut sampler());
        let bottom = cam.generate_ray(vec2(0.5, 1.0), 1.0, &mut sampler());
        assert!(top.direction.y > 0.0, "top of frame points down");
        assert!(bottom.direction.y < 0.0, "bottom of frame points up");

        let left = cam.generate_ray(vec2(0.0, 0.5), 1.0, &mut sampler());
        let right = cam.generate_ray(vec2(1.0, 0.5), 1.0, &mut sampler());
        assert!(left.direction.x < 0.0 && right.direction.x > 0.0);
    }

    /// Every ray from a defocused lens must still pass through the same point
    /// on the focus plane. This is the whole content of the thin-lens model,
    /// and it is what keeps the focus plane sharp.
    #[test]
    fn defocused_rays_converge_on_the_focus_plane() {
        let mut cam = Camera::look_at(vec3(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
        cam.f_number = 1.4;
        cam.focus_distance = 5.0;

        let film = vec2(0.3, 0.7);
        let mut points = Vec::new();
        for i in 0..64 {
            let mut s = Sampler::new(i, i * 7, i, 3);
            let ray = cam.generate_ray(film, 1.5, &mut s);
            // Intersect the plane z = 0, which is the focus plane here.
            let t = (0.0 - ray.origin.z) / ray.direction.z;
            points.push(ray.at(t));
        }
        let first = points[0];
        for p in &points {
            assert!(
                (*p - first).length() < 1e-3,
                "focus plane point drifted: {p:?} vs {first:?}"
            );
        }
        // And the lens origins must actually differ, or the test proves nothing.
        let mut s0 = Sampler::new(0, 0, 0, 3);
        let mut s1 = Sampler::new(5, 9, 11, 3);
        let a = cam.generate_ray(film, 1.5, &mut s0).origin;
        let b = cam.generate_ray(film, 1.5, &mut s1).origin;
        assert!((a - b).length() > 1e-4, "aperture was not sampled");
    }

    #[test]
    fn a_pinhole_has_no_aperture() {
        let cam = Camera::look_at(vec3(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
        assert_eq!(cam.aperture_radius(1.5), 0.0);
        let mut s = Sampler::new(3, 4, 5, 6);
        let ray = cam.generate_ray(vec2(0.2, 0.8), 1.5, &mut s);
        assert!((ray.origin - cam.position()).length() < 1e-6);
    }

    /// Stopping down must widen the depth of field: the aperture radius has to
    /// fall as `1/N`.
    #[test]
    fn aperture_radius_tracks_the_f_number() {
        let mut cam = Camera::look_at(vec3(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
        cam.f_number = 2.0;
        let wide = cam.aperture_radius(1.5);
        cam.f_number = 4.0;
        let narrow = cam.aperture_radius(1.5);
        assert!((wide / narrow - 2.0).abs() < 1e-3, "{wide} vs {narrow}");
        assert!(wide > 0.0);
    }

    /// Stopping down two stops must darken by exactly two stops.
    #[test]
    fn exposure_follows_the_f_number_by_stops() {
        let mut cam = Camera {
            f_number: 2.0,
            ..Default::default()
        };
        let bright = cam.exposure_scale();
        cam.f_number = 4.0;
        let dark = cam.exposure_scale();
        assert!((bright / dark - 4.0).abs() < 1e-3, "{bright} vs {dark}");

        // And doubling ISO must brighten by one stop.
        cam.iso = 200.0;
        assert!((cam.exposure_scale() / dark - 2.0).abs() < 1e-3);
    }

    #[test]
    fn orthographic_rays_are_parallel() {
        let cam = Camera {
            camera_to_world: Mat4::look_at(vec3(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y),
            projection: Projection::Orthographic { scale: 2.0 },
            ..Default::default()
        };
        let a = cam.generate_ray(vec2(0.1, 0.2), 1.5, &mut sampler());
        let b = cam.generate_ray(vec2(0.9, 0.8), 1.5, &mut sampler());
        assert!((a.direction - b.direction).length() < 1e-6);
        assert!(
            (a.origin - b.origin).length() > 0.1,
            "no film offset applied"
        );
    }

    /// A shift lens keeps verticals parallel by offsetting the film, not by
    /// tilting the camera: the ray directions must skew while the camera's own
    /// orientation is untouched.
    #[test]
    fn shift_offsets_the_frame_without_rotating_the_camera() {
        let base = Camera {
            camera_to_world: Mat4::look_at(vec3(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y),
            projection: Projection::Perspective {
                focal_length_mm: 50.0,
            },
            ..Default::default()
        };
        let shifted = Camera {
            projection: Projection::Shift {
                focal_length_mm: 50.0,
                shift_x: 0.0,
                shift_y: 0.25,
            },
            ..base
        };
        let a = base.generate_ray(vec2(0.5, 0.5), 1.5, &mut sampler());
        let b = shifted.generate_ray(vec2(0.5, 0.5), 1.5, &mut sampler());
        assert!(b.direction.y > a.direction.y + 0.05, "no shift applied");
        assert!((a.origin - b.origin).length() < 1e-6, "camera moved");
    }

    /// The equirectangular projection must cover the whole sphere, which is
    /// what makes it usable for environment capture.
    #[test]
    fn equirectangular_covers_the_sphere() {
        let cam = Camera {
            projection: Projection::Equirectangular,
            ..Default::default()
        };
        let up = cam.generate_ray(vec2(0.5, 0.0), 2.0, &mut sampler());
        let down = cam.generate_ray(vec2(0.5, 1.0), 2.0, &mut sampler());
        assert!(up.direction.y > 0.99, "{:?}", up.direction);
        assert!(down.direction.y < -0.99, "{:?}", down.direction);
        let centre = cam.generate_ray(vec2(0.5, 0.5), 2.0, &mut sampler());
        assert!(centre.direction.z < -0.99, "{:?}", centre.direction);
    }

    #[test]
    fn motion_blur_samples_the_shutter_interval() {
        let mut cam = Camera {
            shutter_open: 0.0,
            shutter_close: 1.0,
            ..Default::default()
        };
        let mut times = Vec::new();
        for i in 0..32 {
            let mut s = Sampler::new(1, 1, i, 9);
            times.push(cam.generate_ray(vec2(0.5, 0.5), 1.0, &mut s).time);
        }
        assert!(times.iter().all(|&t| (0.0..1.0).contains(&t)));
        let spread = times.iter().cloned().fold(0.0f32, f32::max)
            - times.iter().cloned().fold(1.0f32, f32::min);
        assert!(spread > 0.5, "shutter barely sampled: spread {spread}");

        // With the shutter closed, every ray is at the same instant.
        cam.shutter_close = 0.0;
        let mut s = Sampler::new(1, 1, 7, 9);
        assert_eq!(cam.generate_ray(vec2(0.5, 0.5), 1.0, &mut s).time, 0.0);
    }

    #[test]
    fn generated_rays_are_always_unit_length() {
        let mut cam = Camera::look_at(vec3(1.0, 2.0, 3.0), Vec3::ZERO, Vec3::Y);
        cam.f_number = 1.8;
        for i in 0..200 {
            let mut s = Sampler::new(i % 13, i % 7, i, 2);
            let film = vec2((i % 17) as f32 / 17.0, (i % 11) as f32 / 11.0);
            let ray = cam.generate_ray(film, 1.7, &mut s);
            assert!(
                (ray.direction.length() - 1.0).abs() < 1e-5,
                "ray {i} length {}",
                ray.direction.length()
            );
        }
    }
}
