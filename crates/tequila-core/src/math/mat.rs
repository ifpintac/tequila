//! 4x4 matrices and rigid/affine transforms.

use super::vec::{vec3, vec4, Vec3, Vec4};
use std::ops::Mul;

/// Row-major 4x4 matrix. Row-major is chosen because it makes the common
/// `point * matrix` inner loop a sequence of dot products against contiguous
/// rows, and because it reads the same way it is written in the source.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Mat4 {
    pub m: [[f32; 4]; 4],
}

impl Default for Mat4 {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Mat4 {
    pub const IDENTITY: Self = Mat4 {
        m: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };

    #[inline]
    pub const fn from_rows(r0: Vec4, r1: Vec4, r2: Vec4, r3: Vec4) -> Self {
        Mat4 {
            m: [
                [r0.x, r0.y, r0.z, r0.w],
                [r1.x, r1.y, r1.z, r1.w],
                [r2.x, r2.y, r2.z, r2.w],
                [r3.x, r3.y, r3.z, r3.w],
            ],
        }
    }

    pub fn translation(t: Vec3) -> Self {
        let mut r = Self::IDENTITY;
        r.m[0][3] = t.x;
        r.m[1][3] = t.y;
        r.m[2][3] = t.z;
        r
    }

    /// A matrix whose upper 3x3 columns are the given vectors.
    ///
    /// With an orthonormal basis this is the rotation that takes world axes to
    /// that basis, and its transpose is the inverse — which is what lets a
    /// local-space scale be written as `B * S * B^T`.
    pub fn from_columns(x: Vec3, y: Vec3, z: Vec3) -> Self {
        let mut r = Self::IDENTITY;
        for (column, v) in [x, y, z].into_iter().enumerate() {
            r.m[0][column] = v.x;
            r.m[1][column] = v.y;
            r.m[2][column] = v.z;
        }
        r
    }

    pub fn scale(s: Vec3) -> Self {
        let mut r = Self::IDENTITY;
        r.m[0][0] = s.x;
        r.m[1][1] = s.y;
        r.m[2][2] = s.z;
        r
    }

    /// Rotation of `angle` radians about a unit `axis` (right-handed).
    pub fn rotation(axis: Vec3, angle: f32) -> Self {
        let a = axis.normalize();
        let (s, c) = angle.sin_cos();
        let ic = 1.0 - c;
        let mut r = Self::IDENTITY;
        r.m[0][0] = a.x * a.x * ic + c;
        r.m[0][1] = a.x * a.y * ic - a.z * s;
        r.m[0][2] = a.x * a.z * ic + a.y * s;
        r.m[1][0] = a.y * a.x * ic + a.z * s;
        r.m[1][1] = a.y * a.y * ic + c;
        r.m[1][2] = a.y * a.z * ic - a.x * s;
        r.m[2][0] = a.z * a.x * ic - a.y * s;
        r.m[2][1] = a.z * a.y * ic + a.x * s;
        r.m[2][2] = a.z * a.z * ic + c;
        r
    }

    /// A camera-to-world matrix placing the eye at `eye`, looking towards
    /// `target`, with `up` defining the roll. Columns are the camera basis
    /// with `-Z` forward, matching the OpenGL/glTF convention that the
    /// importers and the camera model both assume.
    pub fn look_at(eye: Vec3, target: Vec3, up: Vec3) -> Self {
        let forward = (target - eye).try_normalize().unwrap_or(-Vec3::Z);
        // Guard the degenerate case where the view direction is parallel to
        // `up`, which happens whenever the user orbits through the pole.
        let right = match forward.cross(up).try_normalize() {
            Some(r) => r,
            None => forward.cross(Vec3::X).try_normalize().unwrap_or(Vec3::X),
        };
        let true_up = right.cross(forward);
        Mat4::from_rows(
            vec4(right.x, true_up.x, -forward.x, eye.x),
            vec4(right.y, true_up.y, -forward.y, eye.y),
            vec4(right.z, true_up.z, -forward.z, eye.z),
            vec4(0.0, 0.0, 0.0, 1.0),
        )
    }

    pub fn transpose(&self) -> Self {
        let mut r = Self::IDENTITY;
        for i in 0..4 {
            for j in 0..4 {
                r.m[i][j] = self.m[j][i];
            }
        }
        r
    }

    /// Full 4x4 inverse by Gauss-Jordan elimination with partial pivoting.
    /// Returns `None` for a singular matrix; callers treat that as a malformed
    /// transform in the scene rather than panicking mid-render.
    // Gauss-Jordan elimination is written with explicit indices throughout.
    // Row and column arithmetic is what the algorithm *is*, and rewriting the
    // pivot and elimination loops as iterator chains obscures which index is
    // the pivot and which is being eliminated.
    #[allow(clippy::needless_range_loop)]
    pub fn inverse(&self) -> Option<Self> {
        // Augment [A | I] in f64: transform inverses feed ray transforms, and
        // f32 elimination on a large-scale CAD transform loses too much.
        let mut a = [[0.0f64; 8]; 4];
        for i in 0..4 {
            for j in 0..4 {
                a[i][j] = self.m[i][j] as f64;
            }
            a[i][4 + i] = 1.0;
        }
        for col in 0..4 {
            let mut pivot = col;
            for row in (col + 1)..4 {
                if a[row][col].abs() > a[pivot][col].abs() {
                    pivot = row;
                }
            }
            if a[pivot][col].abs() < 1e-12 {
                return None;
            }
            a.swap(col, pivot);
            let inv_p = 1.0 / a[col][col];
            for j in col..8 {
                a[col][j] *= inv_p;
            }
            for row in 0..4 {
                if row != col && a[row][col] != 0.0 {
                    let f = a[row][col];
                    for j in col..8 {
                        a[row][j] -= f * a[col][j];
                    }
                }
            }
        }
        let mut r = Self::IDENTITY;
        for i in 0..4 {
            for j in 0..4 {
                r.m[i][j] = a[i][4 + j] as f32;
            }
        }
        Some(r)
    }

    /// Transform a point (w = 1), applying the perspective divide if present.
    #[inline]
    pub fn transform_point(&self, p: Vec3) -> Vec3 {
        let m = &self.m;
        let x = m[0][0] * p.x + m[0][1] * p.y + m[0][2] * p.z + m[0][3];
        let y = m[1][0] * p.x + m[1][1] * p.y + m[1][2] * p.z + m[1][3];
        let z = m[2][0] * p.x + m[2][1] * p.y + m[2][2] * p.z + m[2][3];
        let w = m[3][0] * p.x + m[3][1] * p.y + m[3][2] * p.z + m[3][3];
        if w == 1.0 {
            vec3(x, y, z)
        } else {
            vec3(x, y, z) / w
        }
    }

    /// Transform a direction (w = 0). Does not renormalize: scale in the
    /// transform is meaningful to the caller (it is how ray differentials and
    /// instance scaling survive).
    #[inline]
    pub fn transform_vector(&self, v: Vec3) -> Vec3 {
        let m = &self.m;
        vec3(
            m[0][0] * v.x + m[0][1] * v.y + m[0][2] * v.z,
            m[1][0] * v.x + m[1][1] * v.y + m[1][2] * v.z,
            m[2][0] * v.x + m[2][1] * v.y + m[2][2] * v.z,
        )
    }

    /// Transform a normal. Normals transform by the inverse transpose, so this
    /// takes the *inverse* of the point transform and applies its transpose.
    /// Passing the forward matrix here is the classic bug that makes normals
    /// wrong under non-uniform scale.
    #[inline]
    pub fn transform_normal_with_inverse(inverse: &Mat4, n: Vec3) -> Vec3 {
        let m = &inverse.m;
        vec3(
            m[0][0] * n.x + m[1][0] * n.y + m[2][0] * n.z,
            m[0][1] * n.x + m[1][1] * n.y + m[2][1] * n.z,
            m[0][2] * n.x + m[1][2] * n.y + m[2][2] * n.z,
        )
    }

    /// Row-major storage flattened for FFI to APIs that want 12 or 16 floats.
    #[inline]
    pub fn to_cols_array_3x4(&self) -> [f32; 12] {
        // Column-major 3x4, the layout Embree's `RTC_FORMAT_FLOAT3X4_COLUMN_MAJOR`
        // wants: three basis columns followed by the translation column.
        [
            self.m[0][0],
            self.m[1][0],
            self.m[2][0],
            self.m[0][1],
            self.m[1][1],
            self.m[2][1],
            self.m[0][2],
            self.m[1][2],
            self.m[2][2],
            self.m[0][3],
            self.m[1][3],
            self.m[2][3],
        ]
    }
}

impl Mul for Mat4 {
    type Output = Mat4;
    fn mul(self, o: Mat4) -> Mat4 {
        let mut r = Mat4 { m: [[0.0; 4]; 4] };
        for i in 0..4 {
            for j in 0..4 {
                r.m[i][j] = self.m[i][0] * o.m[0][j]
                    + self.m[i][1] * o.m[1][j]
                    + self.m[i][2] * o.m[2][j]
                    + self.m[i][3] * o.m[3][j];
            }
        }
        r
    }
}

impl Mul<Vec4> for Mat4 {
    type Output = Vec4;
    #[inline]
    fn mul(self, v: Vec4) -> Vec4 {
        let m = &self.m;
        vec4(
            m[0][0] * v.x + m[0][1] * v.y + m[0][2] * v.z + m[0][3] * v.w,
            m[1][0] * v.x + m[1][1] * v.y + m[1][2] * v.z + m[1][3] * v.w,
            m[2][0] * v.x + m[2][1] * v.y + m[2][2] * v.z + m[2][3] * v.w,
            m[3][0] * v.x + m[3][1] * v.y + m[3][2] * v.z + m[3][3] * v.w,
        )
    }
}

/// A matrix paired with its cached inverse.
///
/// Every transform in the scene needs its inverse for ray transformation and
/// its inverse-transpose for normals, and computing them per-ray is both slow
/// and a source of drift. Store them together and keep them consistent by
/// construction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub matrix: Mat4,
    pub inverse: Mat4,
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    pub const IDENTITY: Self = Transform {
        matrix: Mat4::IDENTITY,
        inverse: Mat4::IDENTITY,
    };

    pub fn from_matrix(matrix: Mat4) -> Option<Self> {
        matrix
            .inverse()
            .map(|inverse| Transform { matrix, inverse })
    }

    pub fn translation(t: Vec3) -> Self {
        Transform {
            matrix: Mat4::translation(t),
            inverse: Mat4::translation(-t),
        }
    }

    pub fn rotation(axis: Vec3, angle: f32) -> Self {
        let matrix = Mat4::rotation(axis, angle);
        Transform {
            inverse: matrix.transpose(), // rotations are orthonormal
            matrix,
        }
    }

    pub fn inverted(&self) -> Self {
        Transform {
            matrix: self.inverse,
            inverse: self.matrix,
        }
    }

    #[inline]
    pub fn point(&self, p: Vec3) -> Vec3 {
        self.matrix.transform_point(p)
    }

    #[inline]
    pub fn vector(&self, v: Vec3) -> Vec3 {
        self.matrix.transform_vector(v)
    }

    #[inline]
    pub fn normal(&self, n: Vec3) -> Vec3 {
        Mat4::transform_normal_with_inverse(&self.inverse, n)
    }

    #[inline]
    pub fn inv_point(&self, p: Vec3) -> Vec3 {
        self.inverse.transform_point(p)
    }

    #[inline]
    pub fn inv_vector(&self, v: Vec3) -> Vec3 {
        self.inverse.transform_vector(v)
    }

    /// Carry a normal the other way, from world space back into object space.
    ///
    /// The inverse of this transform is the world-to-object map, and a normal
    /// transforms by the inverse *of that* transposed — which is this
    /// transform's own forward matrix. Reaching for `self.inverse` here because
    /// the operation is named "inv" is the trap; it gives normals that are not
    /// perpendicular to the surface under any non-uniform scale.
    #[inline]
    pub fn inv_normal(&self, n: Vec3) -> Vec3 {
        Mat4::transform_normal_with_inverse(&self.matrix, n)
    }

    /// How much a length shrinks passing from world space into object space.
    ///
    /// A texture footprint is a distance on the surface, and a distance does
    /// not survive a change of space unchanged: on a part scaled up ten times,
    /// a millimetre of world surface is a tenth of a millimetre of the object
    /// it was modelled as. Carrying the world figure across picks a MIP level
    /// ten times too coarse and blurs every texture on that part.
    ///
    /// One number for three axes is exact under a uniform scale and a
    /// compromise otherwise. The average is the compromise chosen here: the
    /// largest axis would over-blur everything to protect the worst case, and
    /// the smallest would alias.
    #[inline]
    pub fn inv_length_scale(&self) -> f32 {
        let s = (self.inv_vector(Vec3::X).length()
            + self.inv_vector(Vec3::Y).length()
            + self.inv_vector(Vec3::Z).length())
            / 3.0;
        if s.is_finite() && s > 0.0 {
            s
        } else {
            1.0
        }
    }
}

impl Mul for Transform {
    type Output = Transform;
    fn mul(self, o: Transform) -> Transform {
        Transform {
            matrix: self.matrix * o.matrix,
            inverse: o.inverse * self.inverse,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_round_trips() {
        let m = Mat4::translation(vec3(3.0, -2.0, 7.0))
            * Mat4::rotation(vec3(0.3, 1.0, -0.2), 0.9)
            * Mat4::scale(vec3(2.0, 0.5, 1.5));
        let inv = m.inverse().unwrap();
        let p = vec3(1.5, -0.25, 4.0);
        let round = inv.transform_point(m.transform_point(p));
        assert!((round - p).length() < 1e-4, "round trip drifted: {round:?}");
    }

    #[test]
    fn normals_survive_non_uniform_scale() {
        // Under a 4x squash in Y, the normal of a 45-degree XY plane must tilt
        // the opposite way from the tangent. Using the forward matrix instead
        // of the inverse transpose gets this backwards.
        let t = Transform::from_matrix(Mat4::scale(vec3(1.0, 0.25, 1.0))).unwrap();
        let n = t.normal(vec3(1.0, 1.0, 0.0).normalize()).normalize();
        let tangent = t.vector(vec3(-1.0, 1.0, 0.0)).normalize();
        assert!(
            n.dot(tangent).abs() < 1e-5,
            "normal not perpendicular to the transformed tangent"
        );
        assert!(n.y > n.x, "squashing Y should steepen the normal in Y");
    }

    /// The way back has the same trap as the way out, and the obvious spelling
    /// is the wrong one: the inverse-transpose of the world-to-object map is
    /// the object-to-world *forward* matrix, not the inverse.
    #[test]
    fn inv_normal_stays_perpendicular_under_non_uniform_scale() {
        let t = Transform::from_matrix(
            Mat4::translation(vec3(2.0, -1.0, 0.5))
                * Mat4::rotation(vec3(0.2, 1.0, -0.4), 0.7)
                * Mat4::scale(vec3(3.0, 0.4, 1.7)),
        )
        .unwrap();

        // A surface in object space, given by two tangents and their normal.
        let (a, b) = (vec3(1.0, 0.3, -0.2), vec3(-0.4, 0.9, 0.6));
        let object_normal = a.cross(b).normalize();

        // Out to the world and back again.
        let world_normal = t.normal(object_normal).normalize();
        let back = t.inv_normal(world_normal).normalize();
        assert!(
            (back - object_normal).length() < 1e-4,
            "the normal came back as {back:?} rather than {object_normal:?}"
        );

        // ...and the shortcut it would be natural to reach for instead — the
        // same inverse that carries a point back — does not do this. A normal
        // is not a vector under scale, and `inv_vector` here lands off the
        // perpendicular, which is the entire reason the two are separate
        // functions rather than one.
        let naive = t.inv_vector(world_normal).normalize();
        assert!(
            naive.dot(a).abs() > 0.01 || naive.dot(b).abs() > 0.01,
            "inv_vector happened to stay perpendicular here, so this transform \
             does not exercise the difference the test exists to show"
        );
    }

    /// A footprint is a distance, and distances change with the space they are
    /// measured in.
    #[test]
    fn inv_length_scale_undoes_a_uniform_scale() {
        let ten = Transform::from_matrix(Mat4::scale(Vec3::splat(10.0))).unwrap();
        assert!(
            (ten.inv_length_scale() - 0.1).abs() < 1e-5,
            "a part scaled up ten times gave {}",
            ten.inv_length_scale()
        );
        // A rigid transform does not change lengths at all.
        let rigid = Transform::from_matrix(
            Mat4::translation(vec3(4.0, 5.0, 6.0)) * Mat4::rotation(Vec3::Y, 1.1),
        )
        .unwrap();
        assert!((rigid.inv_length_scale() - 1.0).abs() < 1e-5);
        // A degenerate transform must not hand back a zero or a NaN footprint,
        // which would select a MIP level from a division by zero.
        let flat = Transform {
            matrix: Mat4::IDENTITY,
            inverse: Mat4::scale(Vec3::ZERO),
        };
        assert_eq!(flat.inv_length_scale(), 1.0);
    }

    #[test]
    fn singular_matrix_reports_none() {
        assert!(Mat4::scale(vec3(1.0, 0.0, 1.0)).inverse().is_none());
    }

    #[test]
    fn look_at_survives_a_pole_crossing() {
        // Looking straight down with a +Y up vector is the degenerate case the
        // orbit camera hits at the top of its arc.
        let m = Mat4::look_at(vec3(0.0, 5.0, 0.0), Vec3::ZERO, Vec3::Y);
        let forward = -m.transform_vector(Vec3::Z);
        assert!(forward.is_finite());
        assert!((forward.normalize() - vec3(0.0, -1.0, 0.0)).length() < 1e-5);
    }
}
