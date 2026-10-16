//! §6.1's noise primitives — the building blocks every procedural sits on.
//!
//! Four properties are load-bearing, and each is a test below rather than an
//! intention:
//!
//! **Deterministic.** The same point and seed give the same value, on any
//! thread, in any order. A renderer that samples a pixel a hundred times must
//! get the same surface each time, or the "noise" it averages is the
//! procedural rather than the light transport — and `docs/05` §3.6 requires the
//! whole render to be reproducible.
//!
//! **Seedable.** §6 opens with "all are seedable so duplicated parts differ",
//! which is what stops a row of forty identical brackets reading as one object
//! smeared across the frame.
//!
//! **Bounded.** Gradient noise is stated in `[-1, 1]` and everything built on
//! it assumes that. A primitive that overshot would push a roughness past one
//! or a colour past white somewhere far from where anyone was looking.
//!
//! **Continuous.** No seams at integer coordinates. A lattice noise whose
//! interpolant is merely linear has a discontinuous derivative at every cell
//! boundary, which is invisible in the value and glaring the moment the field
//! drives a bump map — the surface creases along a grid. That is why the
//! quintic interpolant the spec names is not a detail.
//!
//! Evaluated in object space by default (§6), so a procedural does not swim
//! across a part as the part moves.

use tequila_core::math::{vec3, Vec3};

/// A 32-bit hash of three lattice coordinates and a seed.
///
/// This is where determinism comes from: no state, no tables to initialize, no
/// dependence on evaluation order. A permutation table would be the classical
/// choice and would need building, seeding and sharing across threads.
#[inline]
fn hash(x: i32, y: i32, z: i32, seed: u32) -> u32 {
    // Three rounds of an integer mixer. The constants are the usual
    // odd-multiplier-and-xorshift pattern; what matters is that neighbouring
    // lattice points produce unrelated outputs, which a weaker mix does not.
    let mut h = (x as u32).wrapping_mul(0x8da6_b343);
    h ^= (y as u32).wrapping_mul(0xd8163_841u32.wrapping_add(0));
    h = h.wrapping_add((z as u32).wrapping_mul(0xcb1a_b31f));
    h ^= seed.wrapping_mul(0x9e37_79b9);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    h
}

/// A uniform value in `[0, 1)` for a lattice point.
#[inline]
fn hash01(x: i32, y: i32, z: i32, seed: u32) -> f32 {
    // 24 bits, which is every value a float can hold in the unit interval
    // without gaps. Using all 32 would quantize unevenly.
    (hash(x, y, z, seed) >> 8) as f32 / (1u32 << 24) as f32
}

/// One of twelve gradient directions, the classical Perlin set.
///
/// The edges of a cube rather than a sphere: they are cheap to select, evenly
/// spread, and — the reason Perlin changed to them — they avoid the clumping
/// near the axes that a naive random direction produces.
#[inline]
fn gradient(x: i32, y: i32, z: i32, seed: u32) -> Vec3 {
    const DIRECTIONS: [Vec3; 12] = [
        vec3(1.0, 1.0, 0.0),
        vec3(-1.0, 1.0, 0.0),
        vec3(1.0, -1.0, 0.0),
        vec3(-1.0, -1.0, 0.0),
        vec3(1.0, 0.0, 1.0),
        vec3(-1.0, 0.0, 1.0),
        vec3(1.0, 0.0, -1.0),
        vec3(-1.0, 0.0, -1.0),
        vec3(0.0, 1.0, 1.0),
        vec3(0.0, -1.0, 1.0),
        vec3(0.0, 1.0, -1.0),
        vec3(0.0, -1.0, -1.0),
    ];
    DIRECTIONS[(hash(x, y, z, seed) % 12) as usize]
}

/// The quintic interpolant, `6t⁵ - 15t⁴ + 10t³`.
///
/// Not smoothstep. Both are zero at 0 and one at 1 with zero first derivative
/// at both ends, but smoothstep's *second* derivative is not zero there — and
/// the second derivative is what a bump map differentiates. With smoothstep a
/// gradient noise creases faintly along every lattice plane; with this it does
/// not. It is the whole reason §6.1 names the interpolant.
#[inline]
fn quintic(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// 3D gradient (Perlin) noise, in `[-1, 1]`.
pub fn perlin(p: Vec3, seed: u32) -> f32 {
    let (xi, yi, zi) = (
        p.x.floor() as i32,
        p.y.floor() as i32,
        p.z.floor() as i32,
    );
    let f = vec3(p.x - p.x.floor(), p.y - p.y.floor(), p.z - p.z.floor());
    let (u, v, w) = (quintic(f.x), quintic(f.y), quintic(f.z));

    // The dot of each corner's gradient with the offset to the sample point.
    let corner = |dx: i32, dy: i32, dz: i32| -> f32 {
        let g = gradient(xi + dx, yi + dy, zi + dz, seed);
        let d = vec3(f.x - dx as f32, f.y - dy as f32, f.z - dz as f32);
        g.dot(d)
    };

    let x00 = lerp(corner(0, 0, 0), corner(1, 0, 0), u);
    let x10 = lerp(corner(0, 1, 0), corner(1, 1, 0), u);
    let x01 = lerp(corner(0, 0, 1), corner(1, 0, 1), u);
    let x11 = lerp(corner(0, 1, 1), corner(1, 1, 1), u);
    let y0 = lerp(x00, x10, v);
    let y1 = lerp(x01, x11, v);
    // The twelve-direction gradients give a range of about ±0.75 in 3D; the
    // scale brings it to the ±1 everything downstream assumes.
    (lerp(y0, y1, w) * 1.3333).clamp(-1.0, 1.0)
}

/// Single-octave value noise in `[-1, 1]`.
///
/// Cheaper than gradient noise and visibly blockier — the spec's "Noise
/// (Texture)" node wants it, and it is the right basis when the result is
/// going through a strong contrast curve anyway.
pub fn value(p: Vec3, seed: u32) -> f32 {
    let (xi, yi, zi) = (
        p.x.floor() as i32,
        p.y.floor() as i32,
        p.z.floor() as i32,
    );
    let (u, v, w) = (
        quintic(p.x - p.x.floor()),
        quintic(p.y - p.y.floor()),
        quintic(p.z - p.z.floor()),
    );
    let corner = |dx: i32, dy: i32, dz: i32| hash01(xi + dx, yi + dy, zi + dz, seed) * 2.0 - 1.0;

    let x00 = lerp(corner(0, 0, 0), corner(1, 0, 0), u);
    let x10 = lerp(corner(0, 1, 0), corner(1, 1, 0), u);
    let x01 = lerp(corner(0, 0, 1), corner(1, 0, 1), u);
    let x11 = lerp(corner(0, 1, 1), corner(1, 1, 1), u);
    lerp(lerp(x00, x10, v), lerp(x01, x11, v), w)
}

/// How a cellular lookup measures distance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Metric {
    #[default]
    Euclidean,
    Manhattan,
    Chebyshev,
}

impl Metric {
    #[inline]
    fn of(self, d: Vec3) -> f32 {
        match self {
            Metric::Euclidean => d.length(),
            Metric::Manhattan => d.x.abs() + d.y.abs() + d.z.abs(),
            Metric::Chebyshev => d.x.abs().max(d.y.abs()).max(d.z.abs()),
        }
    }
}

/// What a Worley lookup found.
#[derive(Clone, Copy, Debug)]
pub struct Cell {
    /// Distance to the nearest feature point.
    pub f1: f32,
    /// Distance to the second nearest. `f2 - f1` is the edge mask: it goes to
    /// zero exactly on the boundary between two cells, which is what draws the
    /// cracks in leather and the joins in granite.
    pub f2: f32,
    /// A stable random value in `[0, 1)` for the nearest cell, for per-cell
    /// colour. Derived from the cell's lattice coordinates rather than from a
    /// counter, so it does not change with evaluation order.
    pub id: f32,
}

/// Worley / cellular noise.
///
/// One feature point per lattice cell, searched over the 3×3×3 neighbourhood.
/// One point per cell rather than several is what keeps the search to
/// twenty-seven cells: with more, a nearer point could hide two cells away and
/// the mask would tear.
pub fn worley(p: Vec3, seed: u32, metric: Metric) -> Cell {
    let base = vec3(p.x.floor(), p.y.floor(), p.z.floor());
    let mut f1 = f32::MAX;
    let mut f2 = f32::MAX;
    let mut id = 0.0;

    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let cell = vec3(base.x + dx as f32, base.y + dy as f32, base.z + dz as f32);
                let (cx, cy, cz) = (cell.x as i32, cell.y as i32, cell.z as i32);
                // The feature point's position within its cell, from the same
                // hash the id comes from.
                let point = cell
                    + vec3(
                        hash01(cx, cy, cz, seed),
                        hash01(cx, cy, cz, seed ^ 0x5bf0_3635),
                        hash01(cx, cy, cz, seed ^ 0x1b87_3593),
                    );
                let d = metric.of(point - p);
                if d < f1 {
                    f2 = f1;
                    f1 = d;
                    id = hash01(cx, cy, cz, seed ^ 0x68bc_21eb);
                } else if d < f2 {
                    f2 = d;
                }
            }
        }
    }
    Cell { f1, f2, id }
}

/// The basis a fractal sum is built from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Basis {
    #[default]
    Perlin,
    Value,
}

impl Basis {
    #[inline]
    fn at(self, p: Vec3, seed: u32) -> f32 {
        match self {
            Basis::Perlin => perlin(p, seed),
            Basis::Value => value(p, seed),
        }
    }
}

/// A fractal sum's shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fractal {
    pub octaves: u32,
    /// Frequency multiplier per octave. Two is the classical choice; something
    /// slightly off two avoids the octaves lining up on the lattice.
    pub lacunarity: f32,
    /// Amplitude multiplier per octave.
    pub gain: f32,
    pub basis: Basis,
}

impl Default for Fractal {
    fn default() -> Self {
        Fractal {
            octaves: 4,
            lacunarity: 2.0,
            gain: 0.5,
            basis: Basis::Perlin,
        }
    }
}

/// Fractional Brownian motion: `Σ gainⁱ · noise(lacunarityⁱ · p)`, normalized
/// to `[-1, 1]`.
///
/// Normalized by the sum of the amplitudes rather than left raw, so that
/// changing the octave count changes the *detail* and not the contrast. Raw,
/// adding a fifth octave would darken everything downstream slightly, and
/// nobody would connect the two.
pub fn fbm(p: Vec3, seed: u32, shape: Fractal) -> f32 {
    let mut sum = 0.0;
    let mut amplitude = 1.0;
    let mut total = 0.0;
    let mut point = p;
    for octave in 0..shape.octaves.max(1) {
        sum += shape.basis.at(point, seed.wrapping_add(octave * 0x9e37)) * amplitude;
        total += amplitude;
        amplitude *= shape.gain;
        point = point * shape.lacunarity;
    }
    if total > 0.0 {
        (sum / total).clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

/// Turbulence: fBm of `|noise|`, in `[0, 1]`.
///
/// The absolute value is what creates the creases — the field has a minimum of
/// zero along every zero crossing of the underlying noise, and those lines are
/// the veins in marble and the wisps in smoke.
pub fn turbulence(p: Vec3, seed: u32, shape: Fractal) -> f32 {
    let mut sum = 0.0;
    let mut amplitude = 1.0;
    let mut total = 0.0;
    let mut point = p;
    for octave in 0..shape.octaves.max(1) {
        sum += shape
            .basis
            .at(point, seed.wrapping_add(octave * 0x9e37))
            .abs()
            * amplitude;
        total += amplitude;
        amplitude *= shape.gain;
        point = point * shape.lacunarity;
    }
    if total > 0.0 {
        (sum / total).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Ridged multifractal: `Σ (1 - |noise|)²` weighted per octave, in `[0, 1]`.
///
/// The square sharpens the ridges, and weighting each octave by the previous
/// one's value is what makes the detail collect *on* the ridges rather than
/// spreading evenly — which is what distinguishes a mountain range from noise
/// with peaks in it.
pub fn ridged(p: Vec3, seed: u32, shape: Fractal) -> f32 {
    let mut sum = 0.0;
    let mut amplitude = 1.0;
    let mut total = 0.0;
    let mut weight = 1.0f32;
    let mut point = p;
    for octave in 0..shape.octaves.max(1) {
        let n = shape.basis.at(point, seed.wrapping_add(octave * 0x9e37));
        let ridge = (1.0 - n.abs()).powi(2) * weight;
        weight = (ridge * 2.0).clamp(0.0, 1.0);
        sum += ridge * amplitude;
        total += amplitude;
        amplitude *= shape.gain;
        point = point * shape.lacunarity;
    }
    if total > 0.0 {
        (sum / total).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(step: f32, count: i32) -> impl Iterator<Item = Vec3> {
        (0..count).flat_map(move |i| {
            (0..count).flat_map(move |j| {
                (0..count).map(move |k| {
                    vec3(i as f32 * step, j as f32 * step, k as f32 * step) - vec3(3.0, 3.0, 3.0)
                })
            })
        })
    }

    /// The same point gives the same value, always. A renderer samples a pixel
    /// hundreds of times and averages; a procedural that varied between those
    /// samples would be adding its own noise to the light transport's, and
    /// `docs/05` §3.6's reproducible render would be impossible.
    #[test]
    fn every_primitive_is_deterministic() {
        for p in grid(0.37, 6) {
            assert_eq!(perlin(p, 7), perlin(p, 7));
            assert_eq!(value(p, 7), value(p, 7));
            assert_eq!(fbm(p, 7, Fractal::default()), fbm(p, 7, Fractal::default()));
            let a = worley(p, 7, Metric::Euclidean);
            let b = worley(p, 7, Metric::Euclidean);
            assert_eq!((a.f1, a.f2, a.id), (b.f1, b.f2, b.id));
        }
    }

    /// §6: "all are seedable so duplicated parts differ". Without it a row of
    /// forty identical brackets reads as one object smeared across the frame.
    #[test]
    fn the_seed_changes_the_field() {
        let differences = grid(0.31, 6)
            .filter(|p| (perlin(*p, 1) - perlin(*p, 2)).abs() > 1e-4)
            .count();
        assert!(
            differences > 100,
            "only {differences} points differ between two seeds"
        );
    }

    /// Bounded, because everything downstream assumes it: a primitive that
    /// overshot would push a roughness past one or a colour past white
    /// somewhere nobody was looking.
    #[test]
    fn every_primitive_stays_in_range() {
        for p in grid(0.13, 12) {
            let n = perlin(p, 3);
            assert!((-1.0..=1.0).contains(&n), "perlin gave {n}");
            let v = value(p, 3);
            assert!((-1.0..=1.0).contains(&v), "value gave {v}");
            let f = fbm(p, 3, Fractal::default());
            assert!((-1.0..=1.0).contains(&f), "fbm gave {f}");
            let t = turbulence(p, 3, Fractal::default());
            assert!((0.0..=1.0).contains(&t), "turbulence gave {t}");
            let r = ridged(p, 3, Fractal::default());
            assert!((0.0..=1.0).contains(&r), "ridged gave {r}");
        }
    }

    /// Gradient noise is zero at every lattice point — that is what makes it
    /// *gradient* noise, and it is the cheapest check that the corner
    /// gradients are being dotted with the right offsets.
    #[test]
    fn gradient_noise_vanishes_on_the_lattice() {
        for i in -3..3 {
            for j in -3..3 {
                let p = vec3(i as f32, j as f32, 1.0);
                assert!(perlin(p, 11).abs() < 1e-5, "perlin({p:?}) = {}", perlin(p, 11));
            }
        }
    }

    /// Continuous across cell boundaries — in the *derivative*, not just the
    /// value.
    ///
    /// This is the property the quintic interpolant exists for, and it is
    /// easy to test vacuously. A linear interpolant is perfectly continuous in
    /// value and still creases along every lattice plane the moment the field
    /// drives a bump map, because its derivative jumps. The first version of
    /// this test compared slopes with a generous tolerance and passed happily
    /// with `quintic` replaced by `t`.
    ///
    /// So what is measured is the second difference — curvature — at points
    /// straddling a lattice plane against points in the middle of a cell. A
    /// derivative discontinuity is an unbounded spike in the second
    /// difference, so the boundary points stand out by orders of magnitude if
    /// the interpolant is wrong, and by nothing at all if it is right.
    #[test]
    fn the_field_has_no_seam_at_a_cell_boundary() {
        let h = 1e-3;
        let curvature = |x: f32| {
            let f = |x: f32| perlin(vec3(x, 0.31, 0.77), 5);
            (f(x + h) - 2.0 * f(x) + f(x - h)).abs()
        };

        // Straddling the lattice planes at every integer.
        let boundary = (-4..=4)
            .map(|i| curvature(i as f32))
            .fold(0.0f32, f32::max);
        // And well inside the cells, for a scale to judge it against. The
        // fractional part is kept away from zero on purpose: an evenly spaced
        // sweep lands on the integers too, and then the "interior" maximum is
        // the boundary spike and the comparison is against itself. That is
        // exactly how the previous version of this test passed with a linear
        // interpolant.
        let interior = (-4..4)
            .flat_map(|k| (0..10).map(move |j| k as f32 + 0.15 + j as f32 * 0.07))
            .map(curvature)
            .fold(0.0f32, f32::max);
        assert!(
            boundary <= interior * 3.0 + 1e-9,
            "curvature reaches {boundary} on a lattice plane against {interior}              inside a cell; the interpolant leaves a crease"
        );
    }

    /// A fractal sum's contrast must not depend on how many octaves it has.
    /// Unnormalized, adding detail would also darken the result, and nobody
    /// would connect the two.
    #[test]
    fn octaves_add_detail_without_changing_contrast() {
        let spread = |octaves: u32| {
            let values: Vec<f32> = grid(0.21, 10)
                .map(|p| {
                    fbm(
                        p,
                        4,
                        Fractal {
                            octaves,
                            ..Fractal::default()
                        },
                    )
                })
                .collect();
            let mean = values.iter().sum::<f32>() / values.len() as f32;
            (values.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / values.len() as f32).sqrt()
        };
        let (one, six) = (spread(1), spread(6));
        assert!(
            (one - six).abs() < one * 0.6,
            "one octave spreads {one} and six spread {six}"
        );
    }

    /// The second-nearest feature point is never nearer than the nearest, and
    /// `f2 - f1` reaches zero on a cell boundary — which is the edge mask that
    /// draws the cracks in leather and the joins in granite.
    #[test]
    fn worley_orders_its_distances_and_finds_edges() {
        let mut smallest_edge = f32::MAX;
        for p in grid(0.05, 24) {
            let cell = worley(p, 9, Metric::Euclidean);
            assert!(
                cell.f2 >= cell.f1 - 1e-6,
                "f2 {} is nearer than f1 {}",
                cell.f2,
                cell.f1
            );
            assert!((0.0..1.0).contains(&cell.id));
            smallest_edge = smallest_edge.min(cell.f2 - cell.f1);
        }
        assert!(
            smallest_edge < 0.02,
            "the closest approach to a cell boundary was {smallest_edge}; there is no edge mask"
        );
    }

    /// All three metrics work and genuinely differ. A metric selector where two
    /// options produce the same field is a control that lies.
    #[test]
    fn the_distance_metrics_differ() {
        let sample = |metric: Metric| {
            grid(0.29, 6)
                .map(|p| worley(p, 2, metric).f1)
                .sum::<f32>()
        };
        let (e, m, c) = (
            sample(Metric::Euclidean),
            sample(Metric::Manhattan),
            sample(Metric::Chebyshev),
        );
        assert!((e - m).abs() > 1.0 && (e - c).abs() > 1.0 && (m - c).abs() > 1.0);
        // Chebyshev is the smallest distance for any offset and Manhattan the
        // largest, so the totals have to order that way.
        assert!(c < e && e < m, "metrics ordered {c}, {e}, {m}");
    }
}
