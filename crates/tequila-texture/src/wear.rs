//! Wear (`docs/02-MATERIAL-SYSTEM.md` §6.2): scratches, dust, smudges and
//! fingerprints, scuffs and worn edges - the marks of use, as patterns that
//! need no UVs.
//!
//! A real product is never as clean as a render of one. Anything that has
//! been handled carries scratches, a shelf leaves dust on whatever faces up,
//! a phone is never without fingerprints, a black plastic case goes grey and
//! dull where it rubs, and paint wears through first on the edges. Each of
//! these is a pattern here, read in the part's own space like every other
//! Tequila pattern, so it stays put as the part moves and needs no UVs: a
//! scratch is laid along the surface wherever the surface goes, a fingerprint
//! is pressed flat onto it, and dust settles by which way the surface faces.
//!
//! # One formula, two renderers
//!
//! Each pattern is written once, over [`Ops`]: the arithmetic of Cycles' own
//! shader nodes - Math, Vector Math, Map Range, White Noise, Noise and
//! Voronoi - which [`Eval`] runs here, for Tequila's renderer, and which the
//! Cycles translation builds as those nodes. The random numbers are Cycles'
//! own hashes ([`crate::cycles::hash`]), keyed by whole cell numbers so the
//! keys are the same to the bit in both, and the noise and the cells are
//! Cycles' own ([`crate::cycles`]), so a scratch lands on the same pixels in
//! both renderers rather than merely looking alike.
//!
//! # Far away
//!
//! A scratch a tenth of a millimetre wide is under a pixel a metre off, and
//! a pattern that draws it there anyway shimmers: one sample lands on it and
//! the next misses. These patterns read how wide the ray's footprint is
//! ([`WearPoint::footprint`]) and widen a line to it, dimmer by as much, so
//! it keeps its share of the surface; further off still, where a pixel holds
//! many scratches, they fade into the share of the surface they cover on
//! average ([`Scratches::mean`]). Whatever reads the pattern - a roughness,
//! a colour - then reads its average, which is what the eye sees of a
//! scratched part across a room: a slightly duller surface, not glitter.

use crate::cycles::noise::NoiseSettings;
use crate::cycles::voronoi::VoronoiSettings;
use tequila_core::math::{vec3, Vec3};

/// What a wear pattern reads of the shading point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WearPoint {
    /// Where, in the part's own space, in scene units: what every Tequila
    /// pattern reads.
    pub position: Vec3,
    /// The shading normal in the same space.
    pub normal: Vec3,
    /// How far the surface faces up in the world: 1 facing straight up, 0
    /// sideways, -1 facing the floor. Dust settles by it.
    pub up: f32,
    /// The radius of the ray's footprint here, in scene units: how much of
    /// the surface one pixel covers. Zero is a point, and draws every line
    /// however fine.
    pub footprint: f32,
    /// Mean curvature, in inverse scene units: positive on a convex edge,
    /// zero on a flat face (`SurfacePoint::curvature`).
    pub curvature: f32,
}

impl WearPoint {
    /// A point on a flat face looking straight up, from nowhere in
    /// particular: what a pattern reads without a renderer around it, as the
    /// flat patterns read the x-z plane.
    pub fn flat(position: Vec3) -> WearPoint {
        WearPoint { position, normal: Vec3::Y, up: 1.0, footprint: 0.0, curvature: 0.0 }
    }
}

/// The arithmetic the patterns are written in: Cycles' shader nodes, as
/// numbers ([`Eval`]) or as the nodes themselves (the Cycles translation).
/// Every operation means what its Cycles node means, down to dividing by
/// zero giving zero.
pub trait Ops {
    /// A number.
    type S: Copy;
    /// A vector (or a colour, read as one).
    type V: Copy;

    fn num(&mut self, x: f32) -> Self::S;
    fn vector(&mut self, v: Vec3) -> Self::V;

    /// The shading point's [`WearPoint::position`].
    fn position(&mut self) -> Self::V;
    /// [`WearPoint::normal`], as it arrives.
    fn normal(&mut self) -> Self::V;
    /// [`WearPoint::up`].
    fn up(&mut self) -> Self::S;
    /// [`WearPoint::footprint`].
    fn footprint(&mut self) -> Self::S;
    /// How convex the surface is for an edge rounded `radius` scene units
    /// tight: the curvature times the radius, kept to 0 to 1 (the Curvature
    /// node's Convex mask).
    fn convexity(&mut self, radius: f32) -> Self::S;

    fn add(&mut self, a: Self::S, b: Self::S) -> Self::S;
    fn sub(&mut self, a: Self::S, b: Self::S) -> Self::S;
    fn mul(&mut self, a: Self::S, b: Self::S) -> Self::S;
    /// Zero where `b` is.
    fn div(&mut self, a: Self::S, b: Self::S) -> Self::S;
    /// `a * b + c`.
    fn madd(&mut self, a: Self::S, b: Self::S, c: Self::S) -> Self::S;
    fn min(&mut self, a: Self::S, b: Self::S) -> Self::S;
    fn abs(&mut self, a: Self::S) -> Self::S;
    /// Zero for anything below zero.
    fn sqrt(&mut self, a: Self::S) -> Self::S;
    /// `1 / sqrt(a)`, zero for anything not above zero.
    fn inv_sqrt(&mut self, a: Self::S) -> Self::S;
    fn sin(&mut self, a: Self::S) -> Self::S;
    fn cos(&mut self, a: Self::S) -> Self::S;
    /// 1 where `a < b`, else 0.
    fn less(&mut self, a: Self::S, b: Self::S) -> Self::S;
    /// Map Range, smooth step: `x` from `from` to `to` (`from` below `to`)
    /// eased onto `low` to `high`, held at the ends.
    fn smooth(&mut self, x: Self::S, from: Self::S, to: Self::S, low: Self::S, high: Self::S) -> Self::S;

    fn vadd(&mut self, a: Self::V, b: Self::V) -> Self::V;
    fn vsub(&mut self, a: Self::V, b: Self::V) -> Self::V;
    /// `a * b + c`, component by component.
    fn vmadd(&mut self, a: Self::V, b: Self::V, c: Self::V) -> Self::V;
    fn vscale(&mut self, v: Self::V, s: Self::S) -> Self::V;
    fn vfloor(&mut self, v: Self::V) -> Self::V;
    fn dot(&mut self, a: Self::V, b: Self::V) -> Self::S;
    fn cross(&mut self, a: Self::V, b: Self::V) -> Self::V;
    fn length(&mut self, v: Self::V) -> Self::S;
    /// Unit length; a zero vector stays zero.
    fn normalize(&mut self, v: Self::V) -> Self::V;
    /// One component: 0 x, 1 y, 2 z.
    fn component(&mut self, v: Self::V, axis: usize) -> Self::S;
    fn combine(&mut self, x: Self::S, y: Self::S, z: Self::S) -> Self::V;

    /// Three random numbers in 0 to 1 for the point `key`: White Noise's
    /// Color, in three dimensions.
    fn white(&mut self, key: Self::V) -> Self::V;
    /// Cycles' Noise texture's Fac at `p` (already scaled): fBM, normalized,
    /// `detail` octaves past the first, lacunarity 2.
    fn noise(&mut self, p: Self::V, detail: f32, roughness: f32) -> Self::S;
    /// Cycles' Voronoi texture at `p` (already scaled), F1, Euclidean:
    /// the distance to the nearest cell's point, that cell's colour, and
    /// where the point is.
    fn voronoi(&mut self, p: Self::V) -> (Self::S, Self::V, Self::V);

    /// Whether what follows `weight` may be left out, because it is nothing
    /// here and everything after is multiplied by it. Numbers can tell; nodes
    /// are built whatever, and say no.
    fn nothing(&mut self, _weight: Self::S) -> bool {
        false
    }
}

/// Map Range's smooth step with fixed ends, which is most of them.
fn smooth<O: Ops>(o: &mut O, x: O::S, from: f32, to: f32, low: f32, high: f32) -> O::S {
    let (from, to, low, high) = (o.num(from), o.num(to), o.num(low), o.num(high));
    o.smooth(x, from, to, low, high)
}

/// `a + (b - a) * t`.
fn mix<O: Ops>(o: &mut O, a: O::S, b: O::S, t: O::S) -> O::S {
    let gap = o.sub(b, a);
    o.madd(gap, t, a)
}

/// `a * k`.
fn times<O: Ops>(o: &mut O, a: O::S, k: f32) -> O::S {
    let k = o.num(k);
    o.mul(a, k)
}

/// `a * k + c`.
fn affine<O: Ops>(o: &mut O, a: O::S, k: f32, c: f32) -> O::S {
    let (k, c) = (o.num(k), o.num(c));
    o.madd(a, k, c)
}

/// `v` with the part along the unit normal `n` taken out: laid flat on the
/// surface.
fn flatten<O: Ops>(o: &mut O, v: O::V, n: O::V) -> (O::V, O::S) {
    let along = o.dot(v, n);
    let up = o.vscale(n, along);
    (o.vsub(v, up), along)
}

// ---- Keys and offsets ------------------------------------------------------

/// A whole number of cells from the seed, below 2048, for keying random
/// numbers: the same in every renderer, since it is computed here.
fn whole(seed: u32, role: u32, axis: u32) -> f32 {
    let mut h = seed.wrapping_mul(0x9e37_79b9) ^ role.wrapping_mul(0x85eb_ca6b) ^ axis.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    (h % 2048) as f32
}

/// What a cell's number is moved by before it is hashed, for one use of
/// the random numbers: whole cells by the seed, and a fraction of its own,
/// so the key is exact (a whole number plus a power of two's fraction) and
/// no two uses ever hash the same key.
pub fn key(seed: u32, role: u32, fraction: f32) -> Vec3 {
    vec3(whole(seed, role, 0) + fraction, whole(seed, role, 1) + fraction, whole(seed, role, 2) + fraction)
}

/// Where a seed moves a pattern's noise to, up to a hundred cells off.
pub fn offset(seed: u32, role: u32) -> Vec3 {
    let at = |axis| whole(seed, role ^ 0x5bd1_e995, axis) * (100.0 / 2048.0);
    vec3(at(0), at(1), at(2))
}

// ---- Scratches -------------------------------------------------------------

/// Which way scratches run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ScratchDirection {
    /// Every way: handling, packing, keys in a pocket.
    #[default]
    Random,
    /// Along the part's X, give or take the spread: a slide, a drawer, a
    /// belt sander.
    AlongX,
    AlongY,
    AlongZ,
    /// In arcs about the places a polishing pad went round: the swirl marks
    /// on polished metal and car paint, which ring a highlight in circles.
    Swirl,
}

impl ScratchDirection {
    pub const ALL: [ScratchDirection; 5] =
        [ScratchDirection::Random, ScratchDirection::AlongX, ScratchDirection::AlongY, ScratchDirection::AlongZ, ScratchDirection::Swirl];

    pub fn key(self) -> &'static str {
        match self {
            ScratchDirection::Random => "random",
            ScratchDirection::AlongX => "x",
            ScratchDirection::AlongY => "y",
            ScratchDirection::AlongZ => "z",
            ScratchDirection::Swirl => "swirl",
        }
    }

    pub fn from_key(key: &str) -> Option<ScratchDirection> {
        ScratchDirection::ALL.into_iter().find(|d| d.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            ScratchDirection::Random => "Any way",
            ScratchDirection::AlongX => "Along X",
            ScratchDirection::AlongY => "Along Y",
            ScratchDirection::AlongZ => "Along Z",
            ScratchDirection::Swirl => "Swirls",
        }
    }

    fn axis(self) -> Option<Vec3> {
        match self {
            ScratchDirection::AlongX => Some(Vec3::X),
            ScratchDirection::AlongY => Some(Vec3::Y),
            ScratchDirection::AlongZ => Some(Vec3::Z),
            ScratchDirection::Random | ScratchDirection::Swirl => None,
        }
    }
}

/// Scratches: thin grooves laid along the surface, each tapering to its
/// ends, bending and wandering a little, some deeper than others, with
/// finer hairlines among them. 0 on untouched surface, up to 1 in the
/// deepest scratch: a mask for roughness or colour, and a depth for a Bump
/// with a negative depth (a groove goes in).
///
/// Laid in the part's own space without UVs: space is cut into cells, each
/// may hold one scratch near its middle, and a scratch is drawn on the
/// surface where the surface passes close to that middle, along its own
/// direction flattened onto the surface there. On a flat face that is a
/// scratch exactly as drawn; round a curve it follows the curve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scratches {
    /// Cells per scene unit: the scale. A scratch is at most four fifths of
    /// a cell long.
    pub frequency: f32,
    /// The share of cells that hold a scratch, 0 to 1.
    pub density: f32,
    /// How long the scratches are, 0 to 1 of the longest a cell holds.
    pub length: f32,
    /// How much shorter some are than others, 0 all alike to 1 from nothing
    /// to full length.
    pub length_variation: f32,
    /// Half the width of a scratch at its widest, as a share of a cell.
    pub width: f32,
    /// How much a scratch narrows towards its ends, 0 even to 1 to a point.
    pub taper: f32,
    /// How much of each end fades out, 0 cut off square to 1 fading from
    /// the middle.
    pub fade: f32,
    /// How much a scratch curves, 0 straight to 1 a hook.
    pub bend: f32,
    /// How irregular a scratch is, 0 drawn with a ruler to 1 by hand: its
    /// line wanders and its width comes and goes along it.
    pub wobble: f32,
    pub direction: ScratchDirection,
    /// How far scratches turn from their direction, 0 all parallel to 1
    /// barely held to it. Along an axis, from the axis; in swirls, from the
    /// circle.
    pub spread: f32,
    /// How much shallower some scratches are than others, 0 to 1.
    pub depth_variation: f32,
    /// How many layers: the scratches, then up to two of ever finer,
    /// fainter hairlines among them.
    pub layers: u32,
}

impl Default for Scratches {
    fn default() -> Self {
        Scratches {
            // Cells 25 mm across in a scene in metres: scratches up to
            // 20 mm long and 0.2 mm wide, on a part the size of a hand.
            frequency: 40.0,
            density: 0.5,
            length: 0.8,
            length_variation: 0.6,
            width: 0.004,
            taper: 0.6,
            fade: 0.3,
            bend: 0.2,
            wobble: 0.2,
            direction: ScratchDirection::Random,
            spread: 0.3,
            depth_variation: 0.6,
            layers: 2,
        }
    }
}

/// One layer of [`Scratches`], as it is drawn.
#[derive(Clone, Copy, Debug)]
struct ScratchLayer {
    frequency: f32,
    width: f32,
    strength: f32,
    role: u32,
}

/// How much finer, narrower (as a share of its own cell) and fainter each
/// layer is than the one before.
const LAYER_SCALE: [f32; 3] = [1.0, 2.5, 6.25];
const LAYER_WIDTH: [f32; 3] = [1.0, 0.75, 0.55];
const LAYER_STRENGTH: [f32; 3] = [1.0, 0.45, 0.22];

/// A scratch's middle may be this far off the surface, in cells, and
/// still be drawn in full; it fades out by `ACCEPT_TO`.
const ACCEPT_FROM: f32 = 0.15;
const ACCEPT_TO: f32 = 0.25;
/// The longest a scratch is, from its middle, in cells.
const HALF_LENGTH: f32 = 0.4;
/// A line under a pixel is widened to this much of the footprint.
const WIDEN: f32 = 0.7;
/// Between these footprints, in cells, a layer fades into its average.
const FAR_FROM: f32 = 0.08;
const FAR_TO: f32 = 0.3;

impl Scratches {
    fn layer(&self, index: usize) -> ScratchLayer {
        ScratchLayer {
            frequency: self.frequency.max(1e-4) * LAYER_SCALE[index],
            width: self.width.max(1e-5) * LAYER_WIDTH[index],
            strength: LAYER_STRENGTH[index],
            role: 0x5c7a_0000 + index as u32 * 16,
        }
    }

    fn layer_count(&self) -> usize {
        self.layers.clamp(1, 3) as usize
    }

    fn half_length(&self) -> f32 {
        HALF_LENGTH * self.length.clamp(0.05, 1.0)
    }

    /// The share of the surface one layer covers, on average, as its
    /// strength: what it fades into far off.
    fn layer_mean(&self, layer: &ScratchLayer) -> f32 {
        let density = self.density.clamp(0.0, 1.0);
        let dv = self.depth_variation.clamp(0.0, 1.0);
        // Middles within the shell about the surface, per cell of area: the
        // fade from `ACCEPT_FROM` to `ACCEPT_TO` either side, integrated.
        let shell = ACCEPT_FROM + ACCEPT_TO;
        let half = self.half_length() * (1.0 - self.length_variation.clamp(0.0, 1.0) * 0.5);
        // Along: the fade at the ends and the taper; across: the groove's
        // profile integrates to its half-width.
        let along = 2.0 * half * (1.0 - 0.5 * self.fade.clamp(0.02, 1.0)) * (1.0 - self.taper.clamp(0.0, 0.95) / 3.0);
        let width = layer.width * (1.0 - dv * 0.25);
        let depth = 1.0 - dv * 0.5;
        (density * shell * along * width * depth * layer.strength).clamp(0.0, 1.0)
    }

    /// The share of the surface the scratches cover on average, weighted by
    /// depth: what the pattern reads far off, where a pixel holds many.
    pub fn mean(&self) -> f32 {
        (0..self.layer_count()).map(|i| self.layer_mean(&self.layer(i))).sum::<f32>().min(1.0)
    }
}

/// [`Scratches`] at the point `o` reads.
pub fn scratches<O: Ops>(o: &mut O, s: &Scratches, seed: u32) -> O::S {
    scratches_in(o, s, seed, 2)
}

/// [`scratches`], looking in `span` cells along each axis about the point
/// rather than the two that can reach it: what checks the two are enough.
fn scratches_in<O: Ops>(o: &mut O, s: &Scratches, seed: u32, span: u32) -> O::S {
    let raw_normal = o.normal();
    let n = o.normalize(raw_normal);
    let p = o.position();
    let footprint = o.footprint();
    let density = s.density.clamp(0.0, 1.0);
    let half_max = s.half_length();
    let length_variation = s.length_variation.clamp(0.0, 1.0);
    let taper = s.taper.clamp(0.0, 0.95);
    let fade = s.fade.clamp(0.02, 1.0);
    let bend = s.bend.clamp(0.0, 1.0) * 1.6;
    let wobble = s.wobble.clamp(0.0, 1.0);
    let spread = s.spread.clamp(0.0, 1.0);
    let depth_variation = s.depth_variation.clamp(0.0, 1.0);
    let mut total = o.num(0.0);
    if density <= 0.0 {
        return total;
    }
    for index in 0..s.layer_count() {
        let layer = s.layer(index);
        let f = o.num(layer.frequency);
        let at = o.vscale(p, f);
        let half_cell = o.vector(Vec3::splat(-0.5));
        let shifted = o.vadd(at, half_cell);
        let base = o.vfloor(shifted);
        let first = -((span / 2) as f32 - 1.0);
        // The footprint in cells, and what a line is widened by for it.
        let reach = o.mul(footprint, f);
        let widen = times(o, reach, WIDEN);
        let tiny = o.num(1e-12);
        let widen = o.madd(widen, widen, tiny);
        let mut sum = o.num(0.0);
        // A scratch reaches at most half a cell from its middle, and a
        // middle lies anywhere in its cell, so the two cells nearest along
        // each axis hold every scratch that can reach the point.
        for corner in 0..span * span * span {
            let (i, j, k) = (corner % span, corner / span % span, corner / (span * span));
            let step = o.vector(vec3(i as f32, j as f32, k as f32) + Vec3::splat(first));
            let cell = o.vadd(base, step);
            let k1 = o.vector(key(seed, layer.role, 0.5));
            let k1 = o.vadd(cell, k1);
            let jitter = o.white(k1);
            let middle = o.vadd(cell, jitter);
            // Where its wobble starts, from the same numbers.
            let phase_weights = o.vector(vec3(37.1, 59.3, 83.7));
            let phase = o.dot(jitter, phase_weights);
            let q = o.vsub(at, middle);
            let distance = o.length(q);
            // Nothing past half a cell, softly, so a scratch never meets
            // the edge of the cells that are looked in.
            let held = smooth(o, distance, 0.45, 0.5, 1.0, 0.0);
            if o.nothing(held) {
                continue;
            }
            let k3 = o.vector(key(seed, layer.role + 1, 0.75));
            let k3 = o.vadd(cell, k3);
            let chance = o.white(k3);
            let presence = o.component(chance, 0);
            let lengths = o.component(chance, 1);
            let bends = o.component(chance, 2);
            let threshold = o.num(density);
            let present = o.less(presence, threshold);
            if o.nothing(present) {
                continue;
            }
            // Drawn where the surface passes near the middle.
            let off = o.dot(q, n);
            let off_size = o.abs(off);
            let near = smooth(o, off_size, ACCEPT_FROM, ACCEPT_TO, 1.0, 0.0);
            if o.nothing(near) {
                continue;
            }

            // Where the point is along the scratch (`a`) and across it
            // (`b`), in the surface, and how sharply it curves (towards
            // +b). Worked out as numbers, not as the scratch's directions:
            // `a` is `q` along the direction flattened onto the surface,
            // `b` along the normal crossed with it.
            let k2 = o.vector(key(seed, layer.role + 2, 0.25));
            let k2 = o.vadd(cell, k2);
            let turn = o.white(k2);
            let h = affine(o, lengths, -length_variation * half_max, half_max);
            let (a, b, curve) = match s.direction {
                ScratchDirection::Swirl => {
                    // Arcs about the place a pad went round: one per block
                    // of three cells each way, somewhere in it.
                    let third = o.num(1.0 / 3.0);
                    let block = o.vscale(cell, third);
                    let block = o.vfloor(block);
                    let kb = o.vector(key(seed, layer.role + 3, 0.125));
                    let kb = o.vadd(block, kb);
                    let pad = o.white(kb);
                    let pad = o.vadd(block, pad);
                    let three = o.num(3.0);
                    let pad = o.vscale(pad, three);
                    let from_pad = o.vsub(middle, pad);
                    // Round the pad: along is n x from_pad, its length the
                    // radius; inward, n x along, points at the pad.
                    let up = o.dot(n, from_pad);
                    let square = o.dot(from_pad, from_pad);
                    let up2 = o.mul(up, up);
                    let flat2 = o.sub(square, up2);
                    let radius = o.sqrt(flat2);
                    let twist = o.cross(from_pad, q);
                    let round = o.dot(n, twist);
                    let straight = o.dot(q, from_pad);
                    let lifted = o.mul(off, up);
                    let inward = o.sub(lifted, straight);
                    let a0 = o.div(round, radius);
                    let b0 = o.div(inward, radius);
                    // Turned off the circle by up to half the spread of a
                    // half turn either way.
                    let wander = o.component(turn, 0);
                    let angle = affine(o, wander, spread * std::f32::consts::PI, -0.5 * spread * std::f32::consts::PI);
                    let (c, si) = (o.cos(angle), o.sin(angle));
                    let bs = o.mul(b0, si);
                    let a = o.madd(a0, c, bs);
                    let as_ = o.mul(a0, si);
                    let bc = o.mul(b0, c);
                    let b = o.sub(bc, as_);
                    // Following the circle, but never tighter than a hook.
                    let one = o.num(1.0);
                    let follow = o.div(one, radius);
                    let two = o.num(2.0);
                    let tightest = o.div(two, h);
                    let curve = o.min(follow, tightest);
                    (a, b, curve)
                }
                direction => {
                    let d = match direction.axis() {
                        // The axis, pushed off it by the spread; a little
                        // even at none, so a face square to the axis still
                        // gets scratches, every way.
                        Some(axis) => {
                            let k = spread * 1.5 + 0.02;
                            let (two, from) = (o.vector(Vec3::splat(2.0 * k)), o.vector(axis - Vec3::splat(k)));
                            o.vmadd(turn, two, from)
                        }
                        None => {
                            let (two, from) = (o.vector(Vec3::splat(2.0)), o.vector(Vec3::splat(-1.0)));
                            o.vmadd(turn, two, from)
                        }
                    };
                    let dn = o.dot(d, n);
                    let dd = o.dot(d, d);
                    let qd = o.dot(q, d);
                    let twist = o.cross(d, q);
                    let across = o.dot(n, twist);
                    let dn2 = o.mul(dn, dn);
                    let flat2 = o.sub(dd, dn2);
                    let length = o.sqrt(flat2);
                    let lifted = o.mul(off, dn);
                    let along = o.sub(qd, lifted);
                    let a = o.div(along, length);
                    let b = o.div(across, length);
                    let curl = affine(o, bends, 2.0 * bend, -bend);
                    let curve = o.div(curl, h);
                    (a, b, curve)
                }
            };

            let t = o.div(a, h);
            // Bent: a parabola through the middle, its slope at `a` the
            // curve times `a`.
            let slope = o.mul(curve, a);
            let sag = o.mul(slope, a);
            let half = o.num(-0.5);
            let b = o.madd(sag, half, b);
            // Wobble: the line wanders, and its width comes and goes.
            let wave = {
                let k = o.num(std::f32::consts::TAU * 0.9 / half_max);
                let x = o.madd(a, k, phase);
                o.sin(x)
            };
            let b = {
                let amount = o.num(wobble * 0.05 * half_max);
                o.madd(wave, amount, b)
            };
            let swell = {
                let k = o.num(std::f32::consts::TAU * 2.3 / half_max);
                let x = o.madd(a, k, phase);
                let s = o.sin(x);
                affine(o, s, wobble * 0.35, 1.0)
            };
            // Narrowing to the ends, and deeper scratches wider.
            let tt = o.mul(t, t);
            let narrowing = affine(o, tt, -taper, 1.0);
            let strength = o.div(presence, threshold);
            let depth = affine(o, strength, -depth_variation, 1.0);
            let wider = affine(o, depth, 0.5 * layer.width, 0.5 * layer.width);
            let w = o.mul(narrowing, swell);
            let w = o.mul(w, wider);
            // Widened to the footprint, dimmed by as much.
            let w2 = o.madd(w, w, widen);
            let widened = o.sqrt(w2);
            let dim = o.div(w, widened);
            // How far across, square to the bent line, in widths.
            let b = o.abs(b);
            let one = o.num(1.0);
            let lean = o.madd(slope, slope, one);
            let lean = o.inv_sqrt(lean);
            let square = o.mul(b, lean);
            let across = o.div(square, widened);
            let groove = smooth(o, across, 0.0, 1.0, 1.0, 0.0);
            let reach_t = o.abs(t);
            let ends = smooth(o, reach_t, 1.0 - fade, 1.0, 1.0, 0.0);
            let mut value = o.mul(groove, ends);
            for weight in [held, near, present, dim, depth] {
                value = o.mul(value, weight);
            }
            sum = o.add(sum, value);
        }
        let one = o.num(1.0);
        let sum = o.min(sum, one);
        let sum = times(o, sum, layer.strength);
        // Far off, a layer is the share of the surface it covers.
        let far = smooth(o, reach, FAR_FROM, FAR_TO, 0.0, 1.0);
        let mean = o.num(s.layer_mean(&layer));
        let seen = mix(o, sum, mean, far);
        total = o.add(total, seen);
    }
    let one = o.num(1.0);
    o.min(total, one)
}

// ---- Dust ------------------------------------------------------------------

/// Dust: a soft, uneven film with fine specks in it, settling on what faces
/// up. 0 clean to 1 thickly dusted: a mask to lighten the colour and
/// roughen the gloss by.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dust {
    /// The film's patches per scene unit.
    pub frequency: f32,
    /// How much film, 0 none to 1 a coat.
    pub amount: f32,
    /// How patchy it is, 0 even to 1 in drifts with clean ground between.
    pub patchiness: f32,
    /// How many specks, 0 to 1.
    pub specks: f32,
    /// Specks per scene unit, about: higher is finer.
    pub speck_frequency: f32,
    /// How much it keeps to what faces up, 0 everywhere alike to 1 on top
    /// faces only.
    pub up_bias: f32,
}

impl Default for Dust {
    fn default() -> Self {
        Dust { frequency: 25.0, amount: 0.5, patchiness: 0.5, specks: 0.5, speck_frequency: 900.0, up_bias: 0.7 }
    }
}

/// A speck's radius, in cells, between these.
const SPECK_SMALL: f32 = 0.08;
const SPECK_LARGE: f32 = 0.3;

impl Dust {
    fn speck_mean(&self) -> f32 {
        // A sphere per cell, cut by the surface, covers as much of it as the
        // sphere does of space: 4/3 pi r^3, its soft edge inside 0.8 r, at
        // the speck's mean strength.
        let (a, b) = (SPECK_SMALL, SPECK_LARGE);
        let cube = (b.powi(4) - a.powi(4)) / (4.0 * (b - a));
        self.specks.clamp(0.0, 1.0) * 4.0 / 3.0 * std::f32::consts::PI * cube * 0.8f32.powi(3) * 0.775
    }

    fn film_mean(&self) -> f32 {
        let amount = self.amount.clamp(0.0, 1.0);
        amount * (0.4 + 0.6 * amount)
    }

    /// On average, on a face looking up.
    pub fn mean(&self) -> f32 {
        (self.film_mean() + self.speck_mean()).min(1.0)
    }
}

pub fn dust<O: Ops>(o: &mut O, d: &Dust, seed: u32) -> O::S {
    let p = o.position();
    let footprint = o.footprint();
    let amount = d.amount.clamp(0.0, 1.0);
    // The film: noise over the surface, kept where it is above a line the
    // amount lowers, softly or in drifts.
    let f = o.num(d.frequency.max(1e-4));
    let at = o.vscale(p, f);
    let moved = o.vector(offset(seed, 1));
    let at = o.vadd(at, moved);
    let cloud = o.noise(at, 4.0, 0.55);
    let soft = 0.05 + 0.45 * (1.0 - d.patchiness.clamp(0.0, 1.0));
    let line = 1.0 - amount;
    let film = smooth(o, cloud, line - soft, line + soft, 0.0, amount * (0.4 + 0.6 * amount));
    // Specks: a few of the cells of a fine lattice hold a grain, of its own
    // size and greyness.
    let fs = o.num(d.speck_frequency.max(1e-4));
    let grains = o.vscale(p, fs);
    let moved = o.vector(offset(seed, 2));
    let grains = o.vadd(grains, moved);
    let (distance, colour, _) = o.voronoi(grains);
    let which = o.component(colour, 0);
    let size = o.component(colour, 1);
    let grey = o.component(colour, 2);
    let many = o.num(d.specks.clamp(0.0, 1.0));
    let present = o.less(which, many);
    let radius = affine(o, size, SPECK_LARGE - SPECK_SMALL, SPECK_SMALL);
    let inner = times(o, radius, 0.5);
    let (low, high) = (o.num(1.0), o.num(0.0));
    let speck = o.smooth(distance, inner, radius, low, high);
    let speck = o.mul(speck, present);
    let strength = affine(o, grey, 0.45, 0.55);
    let speck = o.mul(speck, strength);
    // Under a pixel, a speck is the share of the surface they cover.
    let reach = o.mul(footprint, fs);
    let far = smooth(o, reach, 0.1, 0.4, 0.0, 1.0);
    let mean = o.num(d.speck_mean());
    let speck = mix(o, speck, mean, far);
    let both = o.add(film, speck);
    let one = o.num(1.0);
    let both = o.min(both, one);
    // Settling: all of it on top, less on the sides, least underneath.
    let up = o.up();
    let facing = smooth(o, up, -0.2, 0.9, 0.0, 1.0);
    let bias = d.up_bias.clamp(0.0, 1.0);
    let settled = affine(o, facing, bias, 1.0 - bias);
    o.mul(both, settled)
}

// ---- Fingerprints ----------------------------------------------------------

/// Fingerprints and smudges: the oily marks of hands on a glossy surface -
/// prints with their ridges, and wiped smears - for screens, glass, gloss
/// plastics and polished metal. 0 clean to 1 oily: a mask for roughness,
/// the haze a print leaves on a reflection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fingerprints {
    /// Cells per scene unit; a print is about half a cell long.
    pub frequency: f32,
    /// The share of cells with a print, 0 to 1.
    pub prints: f32,
    /// How much wiped smear lies between them, 0 to 1.
    pub smears: f32,
    /// How clearly a print's ridges show, 0 a plain oily patch to 1 crisp.
    pub ridges: f32,
    /// How soft a print's edge is, 0 to 1.
    pub softness: f32,
}

impl Default for Fingerprints {
    fn default() -> Self {
        // Cells 40 mm across: prints about 18 mm by 13, ridges half a
        // millimetre apart.
        Fingerprints { frequency: 25.0, prints: 0.5, smears: 0.3, ridges: 0.7, softness: 0.4 }
    }
}

/// A print's half-length, in cells, and how much narrower than long it is.
const PRINT_LENGTH: f32 = 0.22;
const PRINT_ASPECT: f32 = 0.72;
/// Ridges across a print's half-length.
const PRINT_RIDGES: f32 = 18.0;

impl Fingerprints {
    fn smear_mean(&self) -> f32 {
        let s = self.smears.clamp(0.0, 1.0);
        s * s * 0.35
    }

    fn print_mean(&self) -> f32 {
        // Prints within the shell about the surface, each an ellipse.
        let half = PRINT_LENGTH * 1.05;
        let area = std::f32::consts::PI * half * half * PRINT_ASPECT;
        self.prints.clamp(0.0, 1.0) * 0.48 * area * 0.8 * (1.0 - 0.5 * self.ridges.clamp(0.0, 1.0)) * 0.8
    }

    /// On average.
    pub fn mean(&self) -> f32 {
        (self.print_mean() + self.smear_mean()).min(1.0)
    }
}

pub fn fingerprints<O: Ops>(o: &mut O, s: &Fingerprints, seed: u32) -> O::S {
    let raw_normal = o.normal();
    let n = o.normalize(raw_normal);
    let p = o.position();
    let footprint = o.footprint();
    let f = o.num(s.frequency.max(1e-4));
    let at = o.vscale(p, f);
    let moved = o.vector(offset(seed, 3));
    let at = o.vadd(at, moved);

    // The nearest print's middle, pressed flat onto the surface here.
    let (_, colour, middle) = o.voronoi(at);
    let q = o.vsub(at, middle);
    let (flat, off) = flatten(o, q, n);
    let off = o.abs(off);
    let near = smooth(o, off, 0.18, 0.3, 1.0, 0.0);
    let k = o.vector(key(seed, 0x51d6, 0.5));
    let k = o.vadd(middle, k);
    let turn = o.white(k);
    let (two, from) = (o.vector(Vec3::splat(2.0)), o.vector(Vec3::splat(-1.0)));
    let d = o.vmadd(turn, two, from);
    let (d, _) = flatten(o, d, n);
    let along = o.normalize(d);
    let across = o.cross(n, along);
    let which = o.component(colour, 0);
    let size = o.component(colour, 1);
    let press = o.component(colour, 2);
    let half = affine(o, size, 0.5 * PRINT_LENGTH, 0.8 * PRINT_LENGTH);
    let a = o.dot(flat, along);
    let b = o.dot(flat, across);
    let x = o.div(a, half);
    let narrow = times(o, half, PRINT_ASPECT);
    let y = o.div(b, narrow);
    let xx = o.mul(x, x);
    let r2 = o.madd(y, y, xx);
    let r = o.sqrt(r2);
    // A ragged edge, as a pressed finger leaves.
    let fine = o.num(7.0);
    let rough_at = o.vscale(at, fine);
    let edge = o.noise(rough_at, 2.0, 0.5);
    let r_edge = {
        let e = affine(o, edge, 0.35, -0.175);
        o.add(r, e)
    };
    let soft = 0.1 + 0.5 * s.softness.clamp(0.0, 1.0);
    let mask = smooth(o, r_edge, 1.0 - soft, 1.0, 1.0, 0.0);
    // Ridges: rings about the print's middle, bent into loops and whorls by
    // a slow noise and leaning along it.
    let slow = o.num(3.0);
    let wander_at = o.vscale(at, slow);
    let moved = o.vector(offset(seed, 4));
    let wander_at = o.vadd(wander_at, moved);
    let wander = o.noise(wander_at, 2.0, 0.5);
    let rings = times(o, r, PRINT_RIDGES);
    let lean = times(o, x, 2.5);
    let rings = o.add(rings, lean);
    let bent = affine(o, wander, 6.0, -3.0);
    let rings = o.add(rings, bent);
    let turns = times(o, rings, std::f32::consts::TAU);
    let ridge = o.cos(turns);
    let ridge = affine(o, ridge, 0.5, 0.5);
    // Ridges under a pixel are their average.
    let reach = o.mul(footprint, f);
    let spacing = PRINT_LENGTH / PRINT_RIDGES;
    let far = smooth(o, reach, 0.3 * spacing, spacing, 0.0, 1.0);
    let middle_grey = o.num(0.5);
    let ridge = mix(o, ridge, middle_grey, far);
    let ridges = s.ridges.clamp(0.0, 1.0);
    let ridged = affine(o, ridge, ridges, 1.0 - ridges);
    let prints = o.num(s.prints.clamp(0.0, 1.0));
    let present = o.less(which, prints);
    let strength = affine(o, press, 0.4, 0.6);
    let mut print = o.mul(mask, ridged);
    for weight in [near, present, strength] {
        print = o.mul(print, weight);
    }

    // Smears: an oily film wiped about, in slow patches streaked finer.
    let smears = s.smears.clamp(0.0, 1.0);
    let slow = o.num(0.6);
    let patch_at = o.vscale(at, slow);
    let moved = o.vector(offset(seed, 5));
    let patch_at = o.vadd(patch_at, moved);
    let patches = o.noise(patch_at, 3.0, 0.5);
    let film = smooth(o, patches, 0.62 - 0.25 * smears, 0.8 - 0.25 * smears, 0.0, smears);
    let streak_scale = o.num(9.0);
    let streak_at = o.vscale(at, streak_scale);
    let moved = o.vector(offset(seed, 6));
    let streak_at = o.vadd(streak_at, moved);
    let streaks = o.noise(streak_at, 3.0, 0.6);
    let streaked = affine(o, streaks, 0.7, 0.65);
    let film = o.mul(film, streaked);

    let both = o.add(print, film);
    let one = o.num(1.0);
    o.min(both, one)
}

// ---- Scuffs ----------------------------------------------------------------

/// Scuffs: soft worn patches where something rubbed, full of fine abrasion
/// lines running the way it rubbed. 0 untouched to 1 worn: a mask to dull
/// the gloss and lighten the colour by - a black plastic gone grey where a
/// bag rubbed it, a shoe's toe.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scuffs {
    /// Cells per scene unit; a scuff is up to about half a cell long.
    pub frequency: f32,
    /// The share of cells with a scuff, 0 to 1.
    pub coverage: f32,
    /// How drawn out a scuff is, 0 round to 1 a long streak.
    pub stretch: f32,
    /// How much its fine abrasion lines show, 0 a plain patch to 1 all lines.
    pub streaks: f32,
    /// How soft its edge is, 0 to 1.
    pub softness: f32,
}

impl Default for Scuffs {
    fn default() -> Self {
        Scuffs { frequency: 12.0, coverage: 0.4, stretch: 0.6, streaks: 0.6, softness: 0.5 }
    }
}

/// A scuff's half-length, in cells, at its smallest and its largest.
const SCUFF_SMALL: f32 = 0.18;
const SCUFF_LARGE: f32 = 0.36;
/// Abrasion lines across a cell, and along it.
const SCUFF_LINES: f32 = 150.0;
const SCUFF_ALONG: f32 = 4.0;

impl Scuffs {
    /// On average.
    pub fn mean(&self) -> f32 {
        let half = (SCUFF_SMALL + SCUFF_LARGE) * 0.5;
        let narrow = 1.0 - 0.75 * self.stretch.clamp(0.0, 1.0);
        let area = std::f32::consts::PI * half * half * narrow;
        let streaks = self.streaks.clamp(0.0, 1.0);
        (self.coverage.clamp(0.0, 1.0) * 0.52 * area * 0.75 * (1.0 - streaks * 0.6) * 0.77).min(1.0)
    }
}

pub fn scuffs<O: Ops>(o: &mut O, s: &Scuffs, seed: u32) -> O::S {
    let raw_normal = o.normal();
    let n = o.normalize(raw_normal);
    let p = o.position();
    let footprint = o.footprint();
    let f = o.num(s.frequency.max(1e-4));
    let at = o.vscale(p, f);
    let moved = o.vector(offset(seed, 7));
    let at = o.vadd(at, moved);

    let (_, colour, middle) = o.voronoi(at);
    let q = o.vsub(at, middle);
    let (flat, off) = flatten(o, q, n);
    let off = o.abs(off);
    let near = smooth(o, off, 0.2, 0.32, 1.0, 0.0);
    let k = o.vector(key(seed, 0x5c0f, 0.5));
    let k = o.vadd(middle, k);
    let turn = o.white(k);
    let (two, from) = (o.vector(Vec3::splat(2.0)), o.vector(Vec3::splat(-1.0)));
    let d = o.vmadd(turn, two, from);
    let (d, _) = flatten(o, d, n);
    let along = o.normalize(d);
    let across = o.cross(n, along);
    let which = o.component(colour, 0);
    let size = o.component(colour, 1);
    let rub = o.component(colour, 2);
    let half = affine(o, size, SCUFF_LARGE - SCUFF_SMALL, SCUFF_SMALL);
    let narrow = times(o, half, 1.0 - 0.75 * s.stretch.clamp(0.0, 1.0));
    let a = o.dot(flat, along);
    let b = o.dot(flat, across);
    let x = o.div(a, half);
    let y = o.div(b, narrow);
    let xx = o.mul(x, x);
    let r2 = o.madd(y, y, xx);
    let r = o.sqrt(r2);
    // A broken edge: worn further in some places than others.
    let fine = o.num(4.0);
    let rough_at = o.vscale(at, fine);
    let moved = o.vector(offset(seed, 8));
    let rough_at = o.vadd(rough_at, moved);
    let broken = o.noise(rough_at, 3.0, 0.55);
    let r = {
        let e = affine(o, broken, 0.5, -0.25);
        o.add(r, e)
    };
    let soft = 0.15 + 0.6 * s.softness.clamp(0.0, 1.0);
    let mask = smooth(o, r, 1.0 - soft, 1.0, 1.0, 0.0);
    // The abrasion lines: noise fine across the rub and drawn out along it.
    let line_a = times(o, a, SCUFF_ALONG);
    let line_b = times(o, b, SCUFF_LINES);
    let line_c = times(o, rub, 31.7);
    let lines_at = o.combine(line_a, line_b, line_c);
    let lines = o.noise(lines_at, 2.0, 0.5);
    let lines = smooth(o, lines, 0.42, 0.7, 0.0, 1.0);
    // Under a pixel, lines are their average.
    let reach = o.mul(footprint, f);
    let far = smooth(o, reach, 0.3 / SCUFF_LINES, 1.0 / SCUFF_LINES, 0.0, 1.0);
    let average = o.num(0.4);
    let lines = mix(o, lines, average, far);
    let streaks = s.streaks.clamp(0.0, 1.0);
    let inner = affine(o, lines, streaks, 1.0 - streaks);
    let coverage = o.num(s.coverage.clamp(0.0, 1.0));
    let present = o.less(which, coverage);
    let strength = affine(o, rub, 0.45, 0.55);
    let mut scuff = o.mul(mask, inner);
    for weight in [near, present, strength] {
        scuff = o.mul(scuff, weight);
    }
    scuff
}

// ---- Edge wear -------------------------------------------------------------

/// Edge wear: where a part's edges have been rubbed through - paint worn to
/// the metal, a moulding's edges polished pale - ragged where it stops.
/// Read from how sharply the surface bends (the Curvature node's own
/// measure, `docs/02` §6.2), broken up by noise. 0 untouched to 1 worn
/// through.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeWear {
    /// In scene units: an edge rounded this tightly or tighter can wear
    /// through; gentler ones less, flat faces never.
    pub radius: f32,
    /// How far in from the edge it wears, 0 to 1.
    pub amount: f32,
    /// How ragged its border is, 0 smooth to 1 in chips.
    pub breakup: f32,
    /// The raggedness's features per scene unit.
    pub frequency: f32,
    /// How soft its border is, 0 to 1.
    pub softness: f32,
}

impl Default for EdgeWear {
    fn default() -> Self {
        EdgeWear { radius: 0.002, amount: 0.5, breakup: 0.6, frequency: 300.0, softness: 0.2 }
    }
}

impl EdgeWear {
    /// On average: flat faces never wear, and most of a part is flat.
    pub fn mean(&self) -> f32 {
        0.0
    }
}

pub fn edge_wear<O: Ops>(o: &mut O, e: &EdgeWear, seed: u32) -> O::S {
    let convex = o.convexity(e.radius.max(0.0));
    let p = o.position();
    let f = o.num(e.frequency.max(1e-4));
    let at = o.vscale(p, f);
    let moved = o.vector(offset(seed, 9));
    let at = o.vadd(at, moved);
    let n = o.noise(at, 4.0, 0.6);
    let breakup = e.breakup.clamp(0.0, 1.0) * 1.2;
    let ragged = affine(o, n, breakup, -0.5 * breakup);
    let v = o.add(convex, ragged);
    let line = 1.0 - e.amount.clamp(0.0, 1.0);
    let soft = 0.02 + 0.25 * e.softness.clamp(0.0, 1.0);
    let worn = smooth(o, v, line - soft, line + soft, 0.0, 1.0);
    // Never on a flat face, whatever the noise says.
    let edge = smooth(o, convex, 0.0, 0.15, 0.0, 1.0);
    o.mul(worn, edge)
}

// ---- Numbers ---------------------------------------------------------------

/// The patterns as numbers, at one point: what Tequila's renderer reads.
#[derive(Clone, Copy, Debug)]
pub struct Eval {
    pub point: WearPoint,
}

impl Eval {
    pub fn new(point: WearPoint) -> Eval {
        Eval { point }
    }
}

fn a3(v: Vec3) -> [f32; 3] {
    [v.x, v.y, v.z]
}

impl Ops for Eval {
    type S = f32;
    type V = Vec3;

    fn num(&mut self, x: f32) -> f32 {
        x
    }
    fn vector(&mut self, v: Vec3) -> Vec3 {
        v
    }
    fn position(&mut self) -> Vec3 {
        self.point.position
    }
    fn normal(&mut self) -> Vec3 {
        self.point.normal
    }
    fn up(&mut self) -> f32 {
        self.point.up
    }
    fn footprint(&mut self) -> f32 {
        self.point.footprint
    }
    fn convexity(&mut self, radius: f32) -> f32 {
        (self.point.curvature * radius).clamp(0.0, 1.0)
    }
    fn add(&mut self, a: f32, b: f32) -> f32 {
        a + b
    }
    fn sub(&mut self, a: f32, b: f32) -> f32 {
        a - b
    }
    fn mul(&mut self, a: f32, b: f32) -> f32 {
        a * b
    }
    fn div(&mut self, a: f32, b: f32) -> f32 {
        if b != 0.0 { a / b } else { 0.0 }
    }
    fn madd(&mut self, a: f32, b: f32, c: f32) -> f32 {
        a * b + c
    }
    fn min(&mut self, a: f32, b: f32) -> f32 {
        a.min(b)
    }
    fn abs(&mut self, a: f32) -> f32 {
        a.abs()
    }
    fn sqrt(&mut self, a: f32) -> f32 {
        a.max(0.0).sqrt()
    }
    fn inv_sqrt(&mut self, a: f32) -> f32 {
        if a > 0.0 { 1.0 / a.sqrt() } else { 0.0 }
    }
    fn sin(&mut self, a: f32) -> f32 {
        a.sin()
    }
    fn cos(&mut self, a: f32) -> f32 {
        a.cos()
    }
    fn less(&mut self, a: f32, b: f32) -> f32 {
        (a < b) as i32 as f32
    }
    fn smooth(&mut self, x: f32, from: f32, to: f32, low: f32, high: f32) -> f32 {
        crate::cycles::ops::map_range(crate::cycles::ops::MapRangeKind::Smoothstep, x, from, to, low, high, 4.0, true)
    }
    fn vadd(&mut self, a: Vec3, b: Vec3) -> Vec3 {
        a + b
    }
    fn vsub(&mut self, a: Vec3, b: Vec3) -> Vec3 {
        a - b
    }
    fn vmadd(&mut self, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
        vec3(a.x * b.x + c.x, a.y * b.y + c.y, a.z * b.z + c.z)
    }
    fn vscale(&mut self, v: Vec3, s: f32) -> Vec3 {
        v * s
    }
    fn vfloor(&mut self, v: Vec3) -> Vec3 {
        vec3(v.x.floor(), v.y.floor(), v.z.floor())
    }
    fn dot(&mut self, a: Vec3, b: Vec3) -> f32 {
        a.x * b.x + a.y * b.y + a.z * b.z
    }
    fn cross(&mut self, a: Vec3, b: Vec3) -> Vec3 {
        // In single precision, as Cycles' Vector Math has it.
        vec3(a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x)
    }
    fn length(&mut self, v: Vec3) -> f32 {
        (v.x * v.x + v.y * v.y + v.z * v.z).sqrt()
    }
    fn normalize(&mut self, v: Vec3) -> Vec3 {
        let l = self.length(v);
        if l != 0.0 { v * (1.0 / l) } else { v }
    }
    fn component(&mut self, v: Vec3, axis: usize) -> f32 {
        match axis {
            0 => v.x,
            1 => v.y,
            _ => v.z,
        }
    }
    fn combine(&mut self, x: f32, y: f32, z: f32) -> Vec3 {
        vec3(x, y, z)
    }
    fn white(&mut self, key: Vec3) -> Vec3 {
        let [x, y, z] = crate::cycles::hash::hash_float3_to_float3(a3(key));
        vec3(x, y, z)
    }
    fn noise(&mut self, p: Vec3, detail: f32, roughness: f32) -> f32 {
        NoiseSettings { detail, roughness, ..NoiseSettings::default() }.value(a3(p))
    }
    fn voronoi(&mut self, p: Vec3) -> (f32, Vec3, Vec3) {
        let out = VoronoiSettings { scale: 1.0, ..VoronoiSettings::default() }.texture(3, a3(p), 0.0);
        let [x, y, z, _] = out.position;
        (out.distance, vec3(out.color[0], out.color[1], out.color[2]), vec3(x, y, z))
    }
    fn nothing(&mut self, weight: f32) -> bool {
        weight == 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(point: WearPoint) -> Eval {
        Eval::new(point)
    }

    fn scratch(s: &Scratches, seed: u32, point: WearPoint) -> f32 {
        scratches(&mut at(point), s, seed)
    }

    /// A plane of points facing up, `count` square, `step` apart.
    fn plane(count: usize, step: f32) -> impl Iterator<Item = WearPoint> {
        (0..count * count).map(move |i| WearPoint::flat(vec3((i % count) as f32 * step + 0.013, 0.021, (i / count) as f32 * step + 0.017)))
    }

    /// The same point and seed give the same value to the bit, every time:
    /// a renderer averages many samples of a pixel, and a pattern that
    /// changed between them would add noise of its own.
    #[test]
    fn every_wear_pattern_is_the_same_at_the_same_point_and_seed() {
        for seed in [0, 1, 77] {
            for point in plane(30, 0.0013) {
                let s = Scratches::default();
                assert_eq!(scratch(&s, seed, point).to_bits(), scratch(&s, seed, point).to_bits());
                let d = Dust::default();
                assert_eq!(dust(&mut at(point), &d, seed).to_bits(), dust(&mut at(point), &d, seed).to_bits());
                let m = Fingerprints::default();
                assert_eq!(fingerprints(&mut at(point), &m, seed).to_bits(), fingerprints(&mut at(point), &m, seed).to_bits());
                let c = Scuffs::default();
                assert_eq!(scuffs(&mut at(point), &c, seed).to_bits(), scuffs(&mut at(point), &c, seed).to_bits());
            }
        }
    }

    /// And a different seed is a different scratching.
    #[test]
    fn the_seed_moves_the_scratches() {
        let s = Scratches { density: 1.0, width: 0.02, ..Scratches::default() };
        let a: Vec<f32> = plane(150, 0.001).map(|p| scratch(&s, 1, p)).collect();
        let b: Vec<f32> = plane(150, 0.001).map(|p| scratch(&s, 2, p)).collect();
        let differ = a.iter().zip(&b).filter(|(x, y)| (*x - *y).abs() > 0.05).count();
        let marked = a.iter().filter(|v| **v > 0.05).count();
        assert!(marked > 50, "only {marked} points scratched");
        assert!(differ > marked / 2, "seed 2 changed {differ} of {marked} scratched points");
    }

    /// Scratches are marks on an intact surface: most of it untouched, a
    /// little scratched, and nothing outside 0 to 1.
    #[test]
    fn scratches_stay_in_range_and_leave_most_of_the_surface() {
        let s = Scratches { width: 0.01, ..Scratches::default() };
        let values: Vec<f32> = plane(300, 0.001).map(|p| scratch(&s, 3, p)).collect();
        assert!(values.iter().all(|v| (0.0..=1.0).contains(v)), "out of range");
        let clean = values.iter().filter(|v| **v == 0.0).count() as f32 / values.len() as f32;
        let deep = values.iter().filter(|v| **v > 0.5).count();
        assert!(clean > 0.7, "only {:.0}% untouched", clean * 100.0);
        assert!(deep > 20, "only {deep} points deep in a scratch");
    }

    /// A scratch stays within its own bounds: nothing is drawn further from
    /// a cell's middle than half a cell, which is what lets the two nearest
    /// cells along each axis be all that are looked in. Looking in four
    /// along each instead finds nothing they missed, at the most bent and
    /// wandering, on flat faces and tilted ones.
    #[test]
    fn scratches_stay_within_the_cells_looked_in() {
        let s = Scratches { frequency: 10.0, density: 1.0, length: 1.0, length_variation: 0.0, bend: 1.0, wobble: 1.0, width: 0.02, layers: 1, ..Scratches::default() };
        let mut drawn = 0;
        for direction in ScratchDirection::ALL {
            let s = Scratches { direction, ..s };
            for (i, point) in plane(90, 0.0021).enumerate() {
                let normal = if i % 2 == 0 { Vec3::Y } else { vec3(0.3, 0.8, -0.52).normalize() };
                let point = WearPoint { normal, ..point };
                let two = scratches_in(&mut at(point), &s, 7, 2);
                let four = scratches_in(&mut at(point), &s, 7, 4);
                assert!((two - four).abs() < 1e-5, "{direction:?} at {:?}: {two} looking in two cells each way, {four} in four", point.position);
                drawn += (two > 0.0) as usize;
            }
        }
        assert!(drawn > 100, "only {drawn} points drawn");
    }

    /// Density, length and width each do what they say, monotonically.
    #[test]
    fn more_density_length_and_width_mean_more_scratch() {
        let covered = |s: Scratches| plane(200, 0.001).map(|p| scratch(&s, 5, p)).sum::<f32>();
        let base = Scratches { layers: 1, width: 0.02, ..Scratches::default() };
        assert!(covered(Scratches { density: 0.8, ..base }) > covered(Scratches { density: 0.2, ..base }) * 1.5);
        assert!(covered(Scratches { length: 1.0, ..base }) > covered(Scratches { length: 0.4, ..base }) * 1.3);
        assert!(covered(Scratches { width: 0.04, ..base }) > covered(Scratches { width: 0.01, ..base }) * 1.5);
        assert!(covered(Scratches { layers: 3, ..base }) > covered(base));
    }

    /// Along an axis, scratches run along it: walking along X crosses far
    /// fewer of them than walking along Z.
    #[test]
    fn scratches_along_x_run_along_x() {
        let s = Scratches { direction: ScratchDirection::AlongX, spread: 0.0, density: 1.0, layers: 1, width: 0.006, bend: 0.0, wobble: 0.0, ..Scratches::default() };
        let walk = |dx: f32, dz: f32| {
            let mut changes = 0.0;
            for line in 0..20 {
                let mut previous = 0.0;
                for i in 0..2000 {
                    let p = vec3(0.01 + line as f32 * 0.031 + i as f32 * dx, 0.0, 0.02 + line as f32 * 0.027 + i as f32 * dz);
                    let v = scratch(&s, 9, WearPoint::flat(p));
                    changes += (v - previous).abs();
                    previous = v;
                }
            }
            changes
        };
        let (along, across) = (walk(0.0002, 0.0), walk(0.0, 0.0002));
        assert!(across > along * 3.0, "along X {along}, across {across}");
    }

    /// A scratch is laid along the surface whichever way the surface faces:
    /// a wall (facing X) is scratched as much as a floor.
    #[test]
    fn scratches_need_no_uvs_and_follow_any_face() {
        let s = Scratches { layers: 1, width: 0.02, ..Scratches::default() };
        let floor: f32 = plane(150, 0.002).map(|p| scratch(&s, 4, p)).sum();
        let wall: f32 = plane(150, 0.002)
            .map(|p| scratch(&s, 4, WearPoint { position: vec3(0.031, p.position.x, p.position.z), normal: Vec3::X, ..p }))
            .sum();
        assert!(floor > 0.0 && wall > floor * 0.5 && wall < floor * 2.0, "floor {floor}, wall {wall}");
    }

    /// Far off, scratches fade into their average rather than shimmering:
    /// with a footprint of a cell the pattern is nearly flat, and at about
    /// the share of the surface it covers close up.
    #[test]
    fn far_off_scratches_are_their_average() {
        let s = Scratches { density: 1.0, width: 0.02, layers: 1, ..Scratches::default() };
        let near: Vec<f32> = plane(400, 0.00125).map(|p| scratch(&s, 6, p)).collect();
        let far: Vec<f32> = plane(400, 0.00125).map(|p| scratch(&s, 6, WearPoint { footprint: 1.0 / s.frequency, ..p })).collect();
        let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
        let spread = |v: &[f32]| {
            let m = mean(v);
            (v.iter().map(|x| (x - m).powi(2)).sum::<f32>() / v.len() as f32).sqrt()
        };
        assert!(spread(&far) < spread(&near) * 0.1, "far off the pattern still spreads {} against {}", spread(&far), spread(&near));
        let (m_near, m_far) = (mean(&near), mean(&far));
        assert!((m_far / m_near - 1.0).abs() < 0.3, "close up the scratches cover {m_near}, far off {m_far}");
        assert!((s.mean() / m_near - 1.0).abs() < 0.3, "the average {} against {m_near} measured", s.mean());
    }

    /// Swirl marks draw, and stay in range.
    #[test]
    fn swirls_draw_in_range() {
        let s = Scratches { direction: ScratchDirection::Swirl, spread: 0.0, density: 1.0, layers: 1, width: 0.02, ..Scratches::default() };
        let values: Vec<f32> = plane(150, 0.002).map(|p| scratch(&s, 8, p)).collect();
        assert!(values.iter().any(|v| *v > 0.3), "no swirl showed");
        assert!(values.iter().all(|v| (0.0..=1.0).contains(v)));
    }

    /// Dust settles on what faces up: a top face holds more than a side,
    /// and a side more than an underside.
    #[test]
    fn dust_settles_on_what_faces_up() {
        let d = Dust::default();
        let on = |up: f32| plane(80, 0.001).map(|p| dust(&mut at(WearPoint { up, ..p }), &d, 2)).sum::<f32>();
        let (top, side, under) = (on(1.0), on(0.0), on(-1.0));
        assert!(top > side * 1.5 && side >= under, "top {top}, side {side}, under {under}");
        let even = Dust { up_bias: 0.0, ..d };
        let flat = |up: f32| plane(40, 0.001).map(|p| dust(&mut at(WearPoint { up, ..p }), &even, 2)).sum::<f32>();
        assert_eq!(flat(1.0), flat(-1.0), "with no bias dust is the same everywhere");
    }

    /// More amount is more dust, and none is clean.
    #[test]
    fn dust_amount_is_monotone() {
        let on = |amount: f32, specks: f32| plane(60, 0.001).map(|p| dust(&mut at(p), &Dust { amount, specks, ..Dust::default() }, 4)).sum::<f32>();
        assert_eq!(on(0.0, 0.0), 0.0);
        assert!(on(0.8, 0.5) > on(0.3, 0.5));
        assert!(on(0.0, 1.0) > on(0.0, 0.2), "more specks were not more dust");
    }

    /// Fingerprints have ridges: inside a print the value rises and falls
    /// every half millimetre or so, and without ridges it does not.
    #[test]
    fn fingerprints_have_ridges() {
        let ridged = Fingerprints { prints: 1.0, smears: 0.0, ridges: 1.0, ..Fingerprints::default() };
        let plain = Fingerprints { ridges: 0.0, ..ridged };
        let walk = |s: &Fingerprints| {
            let mut changes = 0.0;
            let mut previous = 0.0;
            for i in 0..4000 {
                let v = fingerprints(&mut at(WearPoint::flat(vec3(0.0005 + i as f32 * 0.00005, 0.003, 0.011))), s, 1);
                changes += (v - previous).abs();
                previous = v;
            }
            changes
        };
        let (with, without) = (walk(&ridged), walk(&plain));
        assert!(with > without * 3.0, "ridges {with}, plain {without}");
    }

    /// Scuffs are patches: most of the surface untouched, some worn, all in
    /// range.
    #[test]
    fn scuffs_are_patches() {
        let s = Scuffs { coverage: 1.0, ..Scuffs::default() };
        let values: Vec<f32> = plane(150, 0.004).map(|p| scuffs(&mut at(p), &s, 3)).collect();
        assert!(values.iter().all(|v| (0.0..=1.0).contains(v)));
        let worn = values.iter().filter(|v| **v > 0.2).count();
        let clean = values.iter().filter(|v| **v == 0.0).count();
        assert!(worn > 100 && clean > values.len() / 2, "{worn} worn, {clean} clean of {}", values.len());
    }

    /// Edges wear, flat faces never do.
    #[test]
    fn only_edges_wear() {
        let e = EdgeWear::default();
        let flat: f32 = plane(60, 0.0005).map(|p| edge_wear(&mut at(p), &e, 1)).sum();
        assert_eq!(flat, 0.0, "a flat face wore");
        let edge: f32 = plane(60, 0.0005).map(|p| edge_wear(&mut at(WearPoint { curvature: 1.0 / e.radius, ..p }), &e, 1)).sum();
        assert!(edge > 3600.0 * 0.3, "a sharp edge wore only {edge}");
    }
}
