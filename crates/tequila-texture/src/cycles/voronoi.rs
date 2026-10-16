//! Cycles' Voronoi Texture (`kernel/svm/voronoi.h`): F1, F2, smooth F1,
//! distance to edge and n-sphere radius, in one to four dimensions, with
//! the fractal octaves Blender 4 added.

use super::hash::*;

/// Which feature of the cells the node reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VoronoiFeature {
    #[default]
    F1,
    F2,
    SmoothF1,
    DistanceToEdge,
    NSphereRadius,
}

impl VoronoiFeature {
    pub const ALL: [VoronoiFeature; 5] =
        [VoronoiFeature::F1, VoronoiFeature::F2, VoronoiFeature::SmoothF1, VoronoiFeature::DistanceToEdge, VoronoiFeature::NSphereRadius];

    pub fn cycles(self) -> &'static str {
        match self {
            VoronoiFeature::F1 => "f1",
            VoronoiFeature::F2 => "f2",
            VoronoiFeature::SmoothF1 => "smooth_f1",
            VoronoiFeature::DistanceToEdge => "distance_to_edge",
            VoronoiFeature::NSphereRadius => "n_sphere_radius",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            VoronoiFeature::F1 => "F1",
            VoronoiFeature::F2 => "F2",
            VoronoiFeature::SmoothF1 => "Smooth F1",
            VoronoiFeature::DistanceToEdge => "Distance to edge",
            VoronoiFeature::NSphereRadius => "N-sphere radius",
        }
    }
}

/// How distance is measured between a point and a cell's feature point.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VoronoiMetric {
    #[default]
    Euclidean,
    Manhattan,
    Chebychev,
    Minkowski,
}

impl VoronoiMetric {
    pub const ALL: [VoronoiMetric; 4] = [VoronoiMetric::Euclidean, VoronoiMetric::Manhattan, VoronoiMetric::Chebychev, VoronoiMetric::Minkowski];

    pub fn cycles(self) -> &'static str {
        match self {
            VoronoiMetric::Euclidean => "euclidean",
            VoronoiMetric::Manhattan => "manhattan",
            VoronoiMetric::Chebychev => "chebychev",
            VoronoiMetric::Minkowski => "minkowski",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            VoronoiMetric::Euclidean => "Euclidean",
            VoronoiMetric::Manhattan => "Manhattan",
            VoronoiMetric::Chebychev => "Chebychev",
            VoronoiMetric::Minkowski => "Minkowski",
        }
    }
}

/// The node's settings. Scale is applied by the caller, as the node does
/// before anything else; it is here only for the Position output.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoronoiSettings {
    pub feature: VoronoiFeature,
    pub metric: VoronoiMetric,
    pub scale: f32,
    pub detail: f32,
    pub roughness: f32,
    pub lacunarity: f32,
    pub smoothness: f32,
    pub exponent: f32,
    pub randomness: f32,
    pub normalize: bool,
}

impl Default for VoronoiSettings {
    /// Blender's defaults for a new Voronoi Texture.
    fn default() -> Self {
        VoronoiSettings {
            feature: VoronoiFeature::F1,
            metric: VoronoiMetric::Euclidean,
            scale: 5.0,
            detail: 0.0,
            roughness: 0.5,
            lacunarity: 2.0,
            smoothness: 1.0,
            exponent: 0.5,
            randomness: 1.0,
            normalize: false,
        }
    }
}

/// What the node gives: Distance, Color, Position (and W in its last
/// component), Radius.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VoronoiOutput {
    pub distance: f32,
    pub color: [f32; 3],
    pub position: [f32; 4],
    pub radius: f32,
}

/// Settings after the node's clamps, with the distance normalization needs.
#[derive(Clone, Copy)]
struct Params {
    feature: VoronoiFeature,
    metric: VoronoiMetric,
    scale: f32,
    detail: f32,
    roughness: f32,
    lacunarity: f32,
    smoothness: f32,
    exponent: f32,
    randomness: f32,
    max_distance: f32,
    normalize: bool,
}

#[inline]
fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + t * (b - a)
}

#[inline]
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    if x < edge0 {
        0.0
    } else if x >= edge1 {
        1.0
    } else {
        let t = (x - edge0) / (edge1 - edge0);
        (3.0 - 2.0 * t) * (t * t)
    }
}

fn distance<const N: usize>(a: [f32; N], b: [f32; N], p: &Params) -> f32 {
    match p.metric {
        VoronoiMetric::Euclidean => (0..N).map(|i| (a[i] - b[i]) * (a[i] - b[i])).sum::<f32>().sqrt(),
        VoronoiMetric::Manhattan => (0..N).map(|i| (a[i] - b[i]).abs()).sum(),
        VoronoiMetric::Chebychev => (0..N).map(|i| (a[i] - b[i]).abs()).fold(f32::MIN, f32::max),
        VoronoiMetric::Minkowski => (0..N).map(|i| (a[i] - b[i]).abs().powf(p.exponent)).sum::<f32>().powf(1.0 / p.exponent),
    }
}

/// A cheaper measure that orders points as `distance` does.
fn distance_bound<const N: usize>(a: [f32; N], b: [f32; N], p: &Params) -> f32 {
    match p.metric {
        VoronoiMetric::Euclidean => (0..N).map(|i| (a[i] - b[i]) * (a[i] - b[i])).sum(),
        VoronoiMetric::Manhattan => (0..N).map(|i| (a[i] - b[i]).abs()).sum(),
        VoronoiMetric::Chebychev => (0..N).map(|i| (a[i] - b[i]).abs()).fold(f32::MIN, f32::max),
        VoronoiMetric::Minkowski => (0..N).map(|i| (a[i] - b[i]).abs().powf(p.exponent)).sum(),
    }
}

fn jitter<const N: usize>(cell: [i32; N]) -> [f32; N] {
    let mut out = [0.0; N];
    match N {
        2 => out[..2].copy_from_slice(&hash_int2_to_float2([cell[0], cell[1]])),
        3 => out[..3].copy_from_slice(&hash_int3_to_float3([cell[0], cell[1], cell[2]])),
        _ => out[..4].copy_from_slice(&hash_int4_to_float4([cell[0], cell[1], cell[2], cell[3]])),
    }
    out
}

fn cell_color<const N: usize>(cell: [i32; N]) -> [f32; 3] {
    match N {
        2 => hash_int2_to_float3([cell[0], cell[1]]),
        3 => hash_int3_to_float3([cell[0], cell[1], cell[2]]),
        _ => hash_int4_to_float3([cell[0], cell[1], cell[2], cell[3]]),
    }
}

fn position4<const N: usize>(p: [f32; N]) -> [f32; 4] {
    let mut out = [0.0; 4];
    out[..N].copy_from_slice(&p);
    out
}

/// The cells within `reach` of the origin, the last axis outermost, as
/// Cycles' nested loops visit them: smooth F1 sums in that order.
fn offsets<const N: usize>(reach: i32) -> impl Iterator<Item = [i32; N]> {
    let width = 2 * reach + 1;
    let count = (width as usize).pow(N as u32);
    (0..count).map(move |mut index| {
        let mut out = [0; N];
        for slot in out.iter_mut() {
            *slot = (index % width as usize) as i32 - reach;
            index /= width as usize;
        }
        out
    })
}

fn add_i<const N: usize>(a: [i32; N], b: [i32; N]) -> [i32; N] {
    std::array::from_fn(|i| a[i].wrapping_add(b[i]))
}

fn point<const N: usize>(cell: [i32; N], offset: [i32; N], randomness: f32) -> [f32; N] {
    let j = jitter(add_i(cell, offset));
    std::array::from_fn(|i| offset[i] as f32 + j[i] * randomness)
}

fn split<const N: usize>(coord: [f32; N]) -> ([f32; N], [f32; N], [i32; N]) {
    let floor: [f32; N] = std::array::from_fn(|i| coord[i].floor());
    let local = std::array::from_fn(|i| coord[i] - floor[i]);
    let cell = std::array::from_fn(|i| floor[i] as i32);
    (floor, local, cell)
}

fn f1<const N: usize>(p: &Params, coord: [f32; N]) -> VoronoiOutput {
    let (floor, local, cell) = split(coord);
    let (mut min, mut target_offset, mut target) = (f32::MAX, [0; N], [0.0; N]);
    for offset in offsets::<N>(1) {
        let point = point(cell, offset, p.randomness);
        let d = distance_bound(point, local, p);
        if d < min {
            target_offset = offset;
            min = d;
            target = point;
        }
    }
    VoronoiOutput {
        distance: distance(target, local, p),
        color: cell_color(add_i(cell, target_offset)),
        position: position4(std::array::from_fn::<f32, N, _>(|i| target[i] + floor[i])),
        radius: 0.0,
    }
}

fn smooth_f1<const N: usize>(p: &Params, coord: [f32; N]) -> VoronoiOutput {
    let (floor, local, cell) = split(coord);
    let (mut smooth_distance, mut smooth_color, mut smooth_position) = (0.0f32, [0.0f32; 3], [0.0f32; N]);
    let mut h = -1.0f32;
    for offset in offsets::<N>(2) {
        let point = point(cell, offset, p.randomness);
        let d = distance(point, local, p);
        h = if h == -1.0 { 1.0 } else { smoothstep(0.0, 1.0, 0.5 + 0.5 * (smooth_distance - d) / p.smoothness) };
        let mut correction = p.smoothness * h * (1.0 - h);
        smooth_distance = mix(smooth_distance, d, h) - correction;
        correction /= 1.0 + 3.0 * p.smoothness;
        let color = cell_color(add_i(cell, offset));
        smooth_color = std::array::from_fn(|i| mix(smooth_color[i], color[i], h) - correction);
        smooth_position = std::array::from_fn(|i| mix(smooth_position[i], point[i], h) - correction);
    }
    VoronoiOutput {
        distance: smooth_distance,
        color: smooth_color,
        position: position4(std::array::from_fn::<f32, N, _>(|i| floor[i] + smooth_position[i])),
        radius: 0.0,
    }
}

fn f2<const N: usize>(p: &Params, coord: [f32; N]) -> VoronoiOutput {
    let (floor, local, cell) = split(coord);
    let (mut d1, mut d2) = (f32::MAX, f32::MAX);
    let (mut off1, mut off2) = ([0; N], [0; N]);
    let (mut pos1, mut pos2) = ([0.0; N], [0.0; N]);
    for offset in offsets::<N>(1) {
        let point = point(cell, offset, p.randomness);
        let d = distance(point, local, p);
        if d < d1 {
            d2 = d1;
            d1 = d;
            off2 = off1;
            off1 = offset;
            pos2 = pos1;
            pos1 = point;
        } else if d < d2 {
            d2 = d;
            off2 = offset;
            pos2 = point;
        }
    }
    let _ = (off1, pos1);
    VoronoiOutput {
        distance: d2,
        color: cell_color(add_i(cell, off2)),
        position: position4(std::array::from_fn::<f32, N, _>(|i| pos2[i] + floor[i])),
        radius: 0.0,
    }
}

fn dot<const N: usize>(a: [f32; N], b: [f32; N]) -> f32 {
    (0..N).map(|i| a[i] * b[i]).sum()
}

fn distance_to_edge<const N: usize>(p: &Params, coord: [f32; N]) -> f32 {
    let (_, local, cell) = split(coord);
    let to_point = |offset: [i32; N]| -> [f32; N] {
        let pt = point(cell, offset, p.randomness);
        std::array::from_fn(|i| pt[i] - local[i])
    };
    let (mut to_closest, mut min) = ([0.0; N], f32::MAX);
    for offset in offsets::<N>(1) {
        let v = to_point(offset);
        let d = dot(v, v);
        if d < min {
            min = d;
            to_closest = v;
        }
    }
    min = f32::MAX;
    for offset in offsets::<N>(1) {
        let v = to_point(offset);
        let perpendicular: [f32; N] = std::array::from_fn(|i| v[i] - to_closest[i]);
        let len2 = dot(perpendicular, perpendicular);
        if len2 > 0.0001 {
            let len = len2.sqrt();
            let mid: [f32; N] = std::array::from_fn(|i| (to_closest[i] + v[i]) / 2.0);
            let unit: [f32; N] = std::array::from_fn(|i| perpendicular[i] / len);
            min = min.min(dot(mid, unit));
        }
    }
    min
}

fn n_sphere_radius<const N: usize>(p: &Params, coord: [f32; N]) -> f32 {
    let (_, local, cell) = split(coord);
    let len2 = |a: [f32; N], b: [f32; N]| (0..N).map(|i| (a[i] - b[i]) * (a[i] - b[i])).sum::<f32>();
    let (mut closest, mut closest_offset, mut min) = ([0.0; N], [0; N], f32::MAX);
    for offset in offsets::<N>(1) {
        let pt = point(cell, offset, p.randomness);
        let d = len2(pt, local);
        if d < min {
            min = d;
            closest = pt;
            closest_offset = offset;
        }
    }
    min = f32::MAX;
    let mut nearest = [0.0; N];
    for offset in offsets::<N>(1) {
        if offset.iter().all(|&o| o == 0) {
            continue;
        }
        let shifted = add_i(offset, closest_offset);
        let pt = point(cell, shifted, p.randomness);
        let d = len2(closest, pt);
        if d < min {
            min = d;
            nearest = pt;
        }
    }
    len2(nearest, closest).sqrt() / 2.0
}

// ---- One dimension, on float cells, as Cycles has it -------------------

fn f1_1d(p: &Params, coord: f32) -> VoronoiOutput {
    let cell = coord.floor();
    let local = coord - cell;
    let (mut min, mut target_offset, mut target) = (f32::MAX, 0.0f32, 0.0f32);
    for i in -1..=1 {
        let offset = i as f32;
        let pt = offset + hash_float_to_float(cell + offset) * p.randomness;
        let d = (pt - local).abs();
        if d < min {
            target_offset = offset;
            min = d;
            target = pt;
        }
    }
    VoronoiOutput { distance: min, color: hash_float_to_float3(cell + target_offset), position: [0.0, 0.0, 0.0, target + cell], radius: 0.0 }
}

fn smooth_f1_1d(p: &Params, coord: f32) -> VoronoiOutput {
    let cell = coord.floor();
    let local = coord - cell;
    let (mut smooth_distance, mut smooth_position, mut smooth_color) = (0.0f32, 0.0f32, [0.0f32; 3]);
    let mut h = -1.0f32;
    for i in -2..=2 {
        let offset = i as f32;
        let pt = offset + hash_float_to_float(cell + offset) * p.randomness;
        let d = (pt - local).abs();
        h = if h == -1.0 { 1.0 } else { smoothstep(0.0, 1.0, 0.5 + 0.5 * (smooth_distance - d) / p.smoothness) };
        let mut correction = p.smoothness * h * (1.0 - h);
        smooth_distance = mix(smooth_distance, d, h) - correction;
        correction /= 1.0 + 3.0 * p.smoothness;
        let color = hash_float_to_float3(cell + offset);
        smooth_color = std::array::from_fn(|k| mix(smooth_color[k], color[k], h) - correction);
        smooth_position = mix(smooth_position, pt, h) - correction;
    }
    VoronoiOutput { distance: smooth_distance, color: smooth_color, position: [0.0, 0.0, 0.0, cell + smooth_position], radius: 0.0 }
}

fn f2_1d(p: &Params, coord: f32) -> VoronoiOutput {
    let cell = coord.floor();
    let local = coord - cell;
    let (mut d1, mut d2) = (f32::MAX, f32::MAX);
    let (mut off1, mut off2, mut pos1, mut pos2) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for i in -1..=1 {
        let offset = i as f32;
        let pt = offset + hash_float_to_float(cell + offset) * p.randomness;
        let d = (pt - local).abs();
        if d < d1 {
            d2 = d1;
            d1 = d;
            off2 = off1;
            off1 = offset;
            pos2 = pos1;
            pos1 = pt;
        } else if d < d2 {
            d2 = d;
            off2 = offset;
            pos2 = pt;
        }
    }
    let _ = (off1, pos1);
    VoronoiOutput { distance: d2, color: hash_float_to_float3(cell + off2), position: [0.0, 0.0, 0.0, pos2 + cell], radius: 0.0 }
}

fn distance_to_edge_1d(p: &Params, coord: f32) -> f32 {
    let cell = coord.floor();
    let local = coord - cell;
    let mid = hash_float_to_float(cell) * p.randomness;
    let left = -1.0 + hash_float_to_float(cell - 1.0) * p.randomness;
    let right = 1.0 + hash_float_to_float(cell + 1.0) * p.randomness;
    ((mid + left) / 2.0 - local).abs().min(((mid + right) / 2.0 - local).abs())
}

fn n_sphere_radius_1d(p: &Params, coord: f32) -> f32 {
    let cell = coord.floor();
    let local = coord - cell;
    let (mut closest, mut closest_offset, mut min) = (0.0f32, 0.0f32, f32::MAX);
    for i in -1..=1 {
        let offset = i as f32;
        let pt = offset + hash_float_to_float(cell + offset) * p.randomness;
        let d = (pt - local).abs();
        if d < min {
            min = d;
            closest = pt;
            closest_offset = offset;
        }
    }
    min = f32::MAX;
    let mut nearest = 0.0f32;
    for i in -1..=1 {
        if i == 0 {
            continue;
        }
        let offset = i as f32 + closest_offset;
        let pt = offset + hash_float_to_float(cell + offset) * p.randomness;
        let d = (closest - pt).abs();
        if d < min {
            min = d;
            nearest = pt;
        }
    }
    (nearest - closest).abs() / 2.0
}

// ---- The fractal layer -------------------------------------------------

/// Octaves of F1, F2 or smooth F1, summed as fBM Noise sums them, some of
/// the additions lerps.
fn fractal(p: &Params, octave_at: impl Fn(f32) -> VoronoiOutput) -> VoronoiOutput {
    let (mut amplitude, mut max_amplitude, mut scale) = (1.0f32, 0.0f32, 1.0f32);
    let mut output = VoronoiOutput::default();
    let zero_input = p.detail == 0.0 || p.roughness == 0.0;
    let mut i = 0;
    while (i as f32) <= p.detail.ceil() {
        let octave = octave_at(scale);
        if zero_input {
            max_amplitude = 1.0;
            output = octave;
            break;
        }
        if (i as f32) <= p.detail {
            max_amplitude += amplitude;
            output.distance += octave.distance * amplitude;
            for k in 0..3 {
                output.color[k] += octave.color[k] * amplitude;
            }
            for k in 0..4 {
                output.position[k] = mix(output.position[k], octave.position[k] / scale, amplitude);
            }
            scale *= p.lacunarity;
            amplitude *= p.roughness;
        } else {
            let remainder = p.detail - p.detail.floor();
            if remainder != 0.0 {
                max_amplitude = mix(max_amplitude, max_amplitude + amplitude, remainder);
                output.distance = mix(output.distance, output.distance + octave.distance * amplitude, remainder);
                for k in 0..3 {
                    output.color[k] = mix(output.color[k], output.color[k] + octave.color[k] * amplitude, remainder);
                }
                for k in 0..4 {
                    output.position[k] = mix(output.position[k], mix(output.position[k], octave.position[k] / scale, amplitude), remainder);
                }
            }
        }
        i += 1;
    }
    if p.normalize {
        output.distance /= max_amplitude * p.max_distance;
        for k in 0..3 {
            output.color[k] /= max_amplitude;
        }
    }
    for k in 0..4 {
        output.position[k] = if p.scale != 0.0 { output.position[k] / p.scale } else { 0.0 };
    }
    output
}

fn fractal_distance_to_edge(p: &Params, edge_at: impl Fn(f32) -> f32) -> f32 {
    let (mut amplitude, mut max_amplitude, mut scale, mut distance) = (1.0f32, p.max_distance, 1.0f32, 8.0f32);
    let zero_input = p.detail == 0.0 || p.roughness == 0.0;
    let mut i = 0;
    while (i as f32) <= p.detail.ceil() {
        let octave = edge_at(scale);
        if zero_input {
            distance = octave;
            break;
        }
        if (i as f32) <= p.detail {
            max_amplitude = mix(max_amplitude, p.max_distance / scale, amplitude);
            distance = mix(distance, distance.min(octave / scale), amplitude);
            scale *= p.lacunarity;
            amplitude *= p.roughness;
        } else {
            let remainder = p.detail - p.detail.floor();
            if remainder != 0.0 {
                let lerp_amplitude = mix(max_amplitude, p.max_distance / scale, amplitude);
                max_amplitude = mix(max_amplitude, lerp_amplitude, remainder);
                let lerp_distance = mix(distance, distance.min(octave / scale), amplitude);
                distance = mix(distance, distance.min(lerp_distance), remainder);
            }
        }
        i += 1;
    }
    if p.normalize {
        distance /= max_amplitude;
    }
    distance
}

impl VoronoiSettings {
    fn params(&self) -> Params {
        Params {
            feature: self.feature,
            metric: self.metric,
            scale: self.scale,
            detail: self.detail.clamp(0.0, 15.0),
            roughness: self.roughness.clamp(0.0, 1.0),
            lacunarity: self.lacunarity,
            smoothness: (self.smoothness / 2.0).clamp(0.0, 0.5),
            exponent: self.exponent,
            randomness: self.randomness.clamp(0.0, 1.0),
            max_distance: 0.0,
            normalize: self.normalize,
        }
    }

    /// The node at `coord` (already multiplied by Scale, as `w` is), in
    /// `dims` dimensions: 1 reads `w`, 2 the first two of `coord`, 3 all of
    /// it, 4 `coord` and `w`.
    pub fn texture(&self, dims: u8, coord: [f32; 3], w: f32) -> VoronoiOutput {
        let mut p = self.params();
        let reach = 0.5 + 0.5 * p.randomness;
        let c2 = [coord[0], coord[1]];
        let c4 = [coord[0], coord[1], coord[2], w];
        match p.feature {
            VoronoiFeature::DistanceToEdge => {
                p.max_distance = reach;
                let distance = match dims {
                    1 => fractal_distance_to_edge(&p, |s| distance_to_edge_1d(&p, w * s)),
                    2 => fractal_distance_to_edge(&p, |s| distance_to_edge(&p, c2.map(|v| v * s))),
                    3 => fractal_distance_to_edge(&p, |s| distance_to_edge(&p, coord.map(|v| v * s))),
                    _ => fractal_distance_to_edge(&p, |s| distance_to_edge(&p, c4.map(|v| v * s))),
                };
                VoronoiOutput { distance, ..Default::default() }
            }
            VoronoiFeature::NSphereRadius => {
                let radius = match dims {
                    1 => n_sphere_radius_1d(&p, w),
                    2 => n_sphere_radius(&p, c2),
                    3 => n_sphere_radius(&p, coord),
                    _ => n_sphere_radius(&p, c4),
                };
                VoronoiOutput { radius, ..Default::default() }
            }
            feature => {
                let doubled = if feature == VoronoiFeature::F2 { 2.0 } else { 1.0 };
                let smooth = feature == VoronoiFeature::SmoothF1 && p.smoothness != 0.0;
                p.max_distance = match dims {
                    1 => reach,
                    2 => distance([0.0; 2], [reach; 2], &p),
                    3 => distance([0.0; 3], [reach; 3], &p),
                    _ => distance([0.0; 4], [reach; 4], &p),
                } * doubled;
                fn pick<const N: usize>(p: &Params, smooth: bool, c: [f32; N]) -> VoronoiOutput {
                    match p.feature {
                        VoronoiFeature::F2 => f2(p, c),
                        _ if smooth => smooth_f1(p, c),
                        _ => f1(p, c),
                    }
                }
                let one = |c: f32| match p.feature {
                    VoronoiFeature::F2 => f2_1d(&p, c),
                    _ if smooth => smooth_f1_1d(&p, c),
                    _ => f1_1d(&p, c),
                };
                match dims {
                    1 => fractal(&p, |s| one(w * s)),
                    2 => fractal(&p, |s| pick(&p, smooth, c2.map(|v| v * s))),
                    3 => fractal(&p, |s| pick(&p, smooth, coord.map(|v| v * s))),
                    _ => fractal(&p, |s| pick(&p, smooth, c4.map(|v| v * s))),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F1's distance is never more than F2's, and both are zero only on a
    /// feature point.
    #[test]
    fn f1_is_nearer_than_f2() {
        let mut s = VoronoiSettings::default();
        for i in 0..500 {
            let c = [i as f32 * 0.173, i as f32 * 0.091 - 3.0, i as f32 * 0.047];
            s.feature = VoronoiFeature::F1;
            let a = s.texture(3, c, 0.0).distance;
            s.feature = VoronoiFeature::F2;
            let b = s.texture(3, c, 0.0).distance;
            assert!(a <= b + 1e-6, "{a} > {b} at {c:?}");
        }
    }

    /// The cells visited run the last axis outermost, as Cycles' loops do.
    #[test]
    fn offsets_run_the_last_axis_outermost() {
        let first: Vec<[i32; 2]> = offsets::<2>(1).take(4).collect();
        assert_eq!(first, vec![[-1, -1], [0, -1], [1, -1], [-1, 0]]);
    }
}
