//! §6.2's procedural texture nodes, built on the primitives in [`noise`].
//!
//! Four to begin with, chosen because they are what a product render actually
//! needs. Almost nothing manufactured is uniform: a moulded housing has a fine
//! noise in its roughness, a machined face has a brush direction, and anything
//! that has been handled has scratches. A material that is one roughness
//! everywhere reads as computer graphics from across the room, and no amount
//! of lighting fixes it.
//!
//! They evaluate in **object space** (§6), so a procedural does not swim across
//! a part as the part moves, and they are **seeded**, so two copies of the same
//! bracket are not the same bracket twice.
//!
//! # What these output, and what they do not
//!
//! A scalar in `[0, 1]`. That is the honest scope for a first set: scratches,
//! brushing and fine noise earn their keep on *roughness* and *bump*, where a
//! single number is the whole answer. Granite and marble need a colour ramp to
//! mean anything, and half a colour ramp is worse than none — so those wait for
//! the gradient node rather than shipping as grey mush.
//!
//! [`noise`]: crate::noise

use crate::noise::{self, Basis, Fractal, Metric};
use tequila_core::math::{vec3, Vec3};

// `Axis` lives with the projections rather than here: a label projected onto
// the front of a box and marble banded across it are choosing the same thing,
// and two enums spelled the same way would end up serialized under two
// vocabularies. Re-exported so `procedural::Axis` keeps resolving.
pub use crate::projection::Axis;

/// What a [`Procedural::Cellular`] reads out of a Worley lookup.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CellOutput {
    /// Distance to the nearest feature point: rounded blobs.
    #[default]
    Distance,
    /// `F2 - F1`, which goes to zero exactly on a cell boundary. The crack
    /// network in leather, the joins in granite, the grout between tiles.
    Edges,
    /// A flat random value per cell, for per-cell variation.
    Id,
}

/// How the threads of a [`Procedural::Weave`] cross.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WeavePattern {
    /// Over one, under one. Poplin, canvas, most shirting.
    #[default]
    Plain,
    /// Over two, under one, stepping one thread per row: the diagonal of denim
    /// and gabardine.
    Twill,
    /// Long floats, over four and under one, stepped so the crossings scatter:
    /// the smooth face of satin and sateen.
    Satin,
}

/// The hole in a [`Procedural::Mesh`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HoleShape {
    /// Round holes on a square grid.
    #[default]
    Round,
    /// Square holes on a square grid: wire mesh, grilles.
    Square,
    /// Hexagonal holes on a staggered grid: speaker grilles, honeycomb.
    Hex,
}

/// A procedural pattern, evaluated at a point in object space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Procedural {
    /// §6.2's Noise (Fractal). The general-purpose one.
    Noise {
        fractal: Fractal,
        /// Features per scene unit. Higher is finer.
        frequency: f32,
    },
    /// §6.2's Cellular.
    Cellular {
        metric: Metric,
        output: CellOutput,
        frequency: f32,
    },
    /// §6.2's Scratches. The single most useful of the four on a product:
    /// almost everything that has been assembled, packed or handled carries
    /// them, and they are what stops a polished surface looking injection-
    /// moulded from a render farm.
    Scratches {
        frequency: f32,
        /// Half-width of a scratch, as a fraction of the cell.
        width: f32,
        /// Fraction of cells that carry a scratch at all, 0 to 1. Scratches
        /// are sparse; a scratch in every cell is a texture, not damage.
        density: f32,
        /// How far a scratch may turn from the reference direction, in turns.
        /// Zero is perfectly parallel — a brushed finish; a quarter turn is
        /// random, which is wear.
        spread: f32,
    },
    /// §6.2's Marble. `sin(p·axis·frequency + turbulence(p·frequency/3)·distortion)`,
    /// the classic vein formula — a stack of parallel bands, bent by
    /// turbulence — with the turbulence scaled along with the bands, so it
    /// works on a part measured in metres and not only on one a unit across.
    /// The bending is the whole thing — without it the node draws stripes.
    Marble {
        axis: Axis,
        frequency: f32,
        /// How far the turbulence bends the bands. Zero is stripes; large is
        /// unrecognisable as marble.
        distortion: f32,
        fractal: Fractal,
    },
    /// §6.2's Wood. Radial growth rings about a grain axis, bent by noise, plus
    /// an axial grain stretched along the direction the plank was cut.
    Wood {
        axis: Axis,
        /// Rings per scene unit across the grain.
        frequency: f32,
        /// How far the rings wander. Real growth rings are never concentric.
        distortion: f32,
        /// Strength of the fine streaking along the grain, which is what stops
        /// the rings reading as printed-on circles.
        grain: f32,
        fractal: Fractal,
    },
    /// §6.2's Wood (Advanced): Wood's rings and grain, with what makes a
    /// board read as a board rather than a pattern — ring widths that vary
    /// year to year, a light earlywood and a dark latewood in every ring, ray
    /// flecks radiating from the pith, knots the rings swell round, and
    /// planks side by side each cut from a different part of the tree.
    WoodAdvanced {
        axis: Axis,
        frequency: f32,
        distortion: f32,
        grain: f32,
        fractal: Fractal,
        /// How much ring widths vary from one year to the next, 0 to 1.
        ring_variation: f32,
        /// The share of each ring that is dark latewood, 0 to 1.
        latewood: f32,
        /// How sharply earlywood and latewood differ, 0 to 1.
        contrast: f32,
        /// How many ray flecks, 0 to 1. Quarter-sawn oak is full of them.
        flecks: f32,
        /// How often a knot appears along the grain, 0 to 1.
        knots: f32,
        /// Plank width across the grain, in scene units. Zero is one piece.
        board_width: f32,
    },
    /// §6.3's Brushed. One-dimensional noise stretched along a direction.
    Brushed {
        frequency: f32,
        /// How far the noise is stretched along the brush direction. Higher is
        /// a longer, finer grain.
        stretch: f32,
    },
    /// §6.2's Noise (Texture): one octave, with contrast and bias. Where the
    /// fractal is for surfaces, this is for masks — a soft blotch to break up
    /// a colour, pushed hard or soft with contrast.
    NoiseTexture {
        basis: Basis,
        frequency: f32,
        /// Around the middle: 1 leaves it, above 1 hardens it toward a mask.
        contrast: f32,
        /// Added after the contrast, so it moves the result up or down without
        /// changing its shape.
        bias: f32,
    },
    /// §6.1's ridged multifractal as a node: sharp crests with the detail
    /// collected on them. Rock, eroded stone, hammered metal.
    Ridged { fractal: Fractal, frequency: f32 },
    /// §6.2's Granite: cells at two scales with a fine grain over them and
    /// scattered flecks at both ends of the range. Read through a ramp, which
    /// supplies the palette.
    Granite {
        frequency: f32,
        /// How many cells carry a fleck, 0 to 1.
        speckle: f32,
        /// How much of the fine grain shows over the cells, 0 to 1.
        fine: f32,
    },
    /// §6.2's Leather: a pebble of grain per cell, a crack network along the
    /// cell boundaries, and a fine wrinkle over both. A height, for bump.
    Leather {
        frequency: f32,
        /// How wide the cracks are, as a fraction of a cell. Zero is none.
        crack_width: f32,
        /// How much of the fine wrinkle shows, 0 to 1.
        wrinkle: f32,
    },
    /// §6.2's Spots: one round spot per cell. 1 inside a spot, 0 outside.
    Spots {
        frequency: f32,
        /// How large the spots are, 0 to 1. Zero is none.
        coverage: f32,
        /// How much the spots differ in size, 0 to 1.
        variance: f32,
        /// How soft a spot's edge is, 0 to 1.
        softness: f32,
    },
    /// §6.2's Camouflage: layers of thresholded noise at different scales,
    /// stacked in order. Outputs `k / layers` for the topmost layer `k`, so a
    /// ramp with that many colour steps paints it.
    Camouflage {
        frequency: f32,
        /// Two to five.
        layers: u32,
    },
    /// §6.2's Contour: lines at even steps of the distance along an axis, bent
    /// by noise. 1 on a line, 0 between. Topographic maps, technical looks.
    Contour {
        axis: Axis,
        /// Lines per scene unit.
        frequency: f32,
        /// Line width as a fraction of the spacing.
        width: f32,
        softness: f32,
        /// How far noise bends the lines, in spacings. Zero is straight.
        distortion: f32,
    },
    /// §6.2's Brushed (Radial): brushing in circles about an axis — the finish
    /// on a machined knob or a watch crown.
    BrushedRadial {
        axis: Axis,
        frequency: f32,
        /// How far the grain is stretched around the circle.
        stretch: f32,
    },
    /// §6.3's Weave: warp and weft threads crossing over and under. A height:
    /// the thread on top at a crossing is highest, gaps between threads are 0.
    Weave {
        pattern: WeavePattern,
        /// Threads per scene unit.
        frequency: f32,
        /// How much of the pitch a thread fills, 0 to 1. Below 1 leaves gaps.
        width: f32,
        /// Per-thread variation in height, 0 to 1. Real yarn is never even.
        variance: f32,
    },
    /// §6.3's Mesh: a perforated sheet. 1 on the material, 0 in a hole — an
    /// opacity, for cut-outs.
    Mesh {
        shape: HoleShape,
        /// Holes per scene unit.
        frequency: f32,
        /// Hole size as a fraction of the pitch.
        size: f32,
        softness: f32,
    },
}

impl Default for Procedural {
    fn default() -> Self {
        Procedural::Noise {
            fractal: Fractal::default(),
            frequency: 4.0,
        }
    }
}

impl Procedural {
    /// The name shown in the interface.
    pub fn label(&self) -> &'static str {
        match self {
            Procedural::Noise { .. } => "Noise",
            Procedural::Cellular { .. } => "Cellular",
            Procedural::Scratches { .. } => "Scratches",
            Procedural::Marble { .. } => "Marble",
            Procedural::Wood { .. } => "Wood",
            Procedural::WoodAdvanced { .. } => "Wood (Advanced)",
            Procedural::Brushed { .. } => "Brushed",
            Procedural::NoiseTexture { .. } => "Noise (Texture)",
            Procedural::Ridged { .. } => "Ridged noise",
            Procedural::Granite { .. } => "Granite",
            Procedural::Leather { .. } => "Leather",
            Procedural::Spots { .. } => "Spots",
            Procedural::Camouflage { .. } => "Camouflage",
            Procedural::Contour { .. } => "Contour",
            Procedural::BrushedRadial { .. } => "Brushed (Radial)",
            Procedural::Weave { .. } => "Weave",
            Procedural::Mesh { .. } => "Perforated mesh",
        }
    }

    /// A short key for the file format.
    pub fn key(&self) -> &'static str {
        match self {
            Procedural::Noise { .. } => "noise",
            Procedural::Cellular { .. } => "cellular",
            Procedural::Scratches { .. } => "scratches",
            Procedural::Marble { .. } => "marble",
            Procedural::Wood { .. } => "wood",
            Procedural::WoodAdvanced { .. } => "wood_advanced",
            Procedural::Brushed { .. } => "brushed",
            Procedural::NoiseTexture { .. } => "noise_texture",
            Procedural::Ridged { .. } => "ridged",
            Procedural::Granite { .. } => "granite",
            Procedural::Leather { .. } => "leather",
            Procedural::Spots { .. } => "spots",
            Procedural::Camouflage { .. } => "camouflage",
            Procedural::Contour { .. } => "contour",
            Procedural::BrushedRadial { .. } => "brushed_radial",
            Procedural::Weave { .. } => "weave",
            Procedural::Mesh { .. } => "mesh",
        }
    }

    /// Whether this is a flat pattern (§6.3) rather than a solid one (§6.2).
    ///
    /// A flat pattern lies in a plane: in object space the x–z plane, the way
    /// scratches always have, and under a mapping the mapping's (u, v).
    pub fn is_planar(&self) -> bool {
        matches!(
            self,
            Procedural::Scratches { .. } | Procedural::Weave { .. } | Procedural::Mesh { .. }
        )
    }


    /// One of each, at usable defaults, for the menu.
    pub fn catalog() -> [Procedural; 17] {
        [
            Procedural::Noise {
                fractal: Fractal::default(),
                frequency: 4.0,
            },
            Procedural::Cellular {
                metric: Metric::Euclidean,
                output: CellOutput::Distance,
                frequency: 6.0,
            },
            Procedural::Scratches {
                // A scratch here is about eight thousandths of a scene unit
                // across — a tenth of a millimetre on a hundred-millimetre
                // part, which is what a real scratch is.
                //
                // The first defaults were frequency 30 and width 0.02, making
                // a scratch two thirds of a *thousandth* of a unit wide. That
                // is also physically plausible and it is useless: it is well
                // under a pixel at any resolution anyone renders at, so it
                // aliases into a faint sparkle or vanishes. A default has to
                // be visible before it can be tuned.
                frequency: 8.0,
                width: 0.06,
                density: 0.3,
                spread: 0.05,
            },
            Procedural::Marble {
                axis: Axis::Y,
                frequency: 3.0,
                distortion: 2.5,
                fractal: Fractal {
                    octaves: 5,
                    ..Fractal::default()
                },
            },
            Procedural::Wood {
                axis: Axis::Y,
                frequency: 8.0,
                distortion: 0.35,
                grain: 0.35,
                fractal: Fractal {
                    octaves: 3,
                    ..Fractal::default()
                },
            },
            Procedural::WoodAdvanced {
                axis: Axis::Y,
                frequency: 8.0,
                distortion: 0.3,
                grain: 0.25,
                fractal: Fractal {
                    octaves: 3,
                    ..Fractal::default()
                },
                ring_variation: 0.5,
                latewood: 0.3,
                contrast: 0.8,
                flecks: 0.3,
                knots: 0.25,
                board_width: 0.0,
            },
            Procedural::Brushed {
                frequency: 60.0,
                stretch: 40.0,
            },
            Procedural::NoiseTexture {
                basis: Basis::Perlin,
                frequency: 8.0,
                contrast: 1.0,
                bias: 0.0,
            },
            Procedural::Ridged {
                fractal: Fractal {
                    octaves: 5,
                    ..Fractal::default()
                },
                frequency: 3.0,
            },
            Procedural::Granite {
                frequency: 10.0,
                speckle: 0.15,
                fine: 0.3,
            },
            Procedural::Leather {
                frequency: 25.0,
                crack_width: 0.12,
                wrinkle: 0.25,
            },
            Procedural::Spots {
                frequency: 8.0,
                coverage: 0.5,
                variance: 0.3,
                softness: 0.1,
            },
            Procedural::Camouflage {
                frequency: 3.0,
                layers: 4,
            },
            Procedural::Contour {
                axis: Axis::Y,
                frequency: 10.0,
                width: 0.1,
                softness: 0.05,
                distortion: 0.8,
            },
            Procedural::BrushedRadial {
                axis: Axis::Y,
                frequency: 200.0,
                stretch: 60.0,
            },
            Procedural::Weave {
                pattern: WeavePattern::Plain,
                frequency: 40.0,
                width: 0.85,
                variance: 0.2,
            },
            Procedural::Mesh {
                shape: HoleShape::Round,
                frequency: 20.0,
                size: 0.6,
                softness: 0.05,
            },
        ]
    }

    /// Evaluate at mapped coordinates, in `[0, 1]`: what a Mapping node wired
    /// into the pattern's Coordinates hands it. A flat pattern lies on (u, v);
    /// a solid one is sampled on the plane z = 0 of its space.
    pub fn evaluate_uv(&self, u: f32, v: f32, seed: u32) -> f32 {
        if self.is_planar() {
            self.evaluate_planar(u, v, seed)
        } else {
            self.evaluate(vec3(u, v, 0.0), seed)
        }
    }

    /// A flat pattern at a point of its plane.
    fn evaluate_planar(&self, a: f32, b: f32, seed: u32) -> f32 {
        match *self {
            Procedural::Scratches {
                frequency,
                width,
                density,
                spread,
            } => scratches(a, b, seed, frequency, width, density, spread),
            Procedural::Weave {
                pattern,
                frequency,
                width,
                variance,
            } => {
                let (x, y) = phase(a * frequency.max(1e-4), b * frequency.max(1e-4), seed);
                weave(x, y, seed, pattern, width, variance)
            }
            Procedural::Mesh {
                shape,
                frequency,
                size,
                softness,
            } => {
                let (x, y) = phase(a * frequency.max(1e-4), b * frequency.max(1e-4), seed);
                mesh(x, y, shape, size, softness)
            }
            // Not flat: sampled on the plane, as a mapping would.
            solid => solid.evaluate(vec3(a, b, 0.0), seed),
        }
    }

    /// Evaluate at an object-space point, in `[0, 1]`.
    pub fn evaluate(&self, position: Vec3, seed: u32) -> f32 {
        if self.is_planar() {
            // Flattened onto the x–z plane. A scratch or a thread is a surface
            // feature, and a 3D field would put it *through* the part rather
            // than across it.
            return self.evaluate_planar(position.x, position.z, seed);
        }
        match *self {
            Procedural::Noise { fractal, frequency } => {
                // fBm runs [-1, 1]; a texture parameter wants [0, 1].
                noise::fbm(position * frequency.max(1e-4), seed, fractal) * 0.5 + 0.5
            }

            Procedural::Cellular {
                metric,
                output,
                frequency,
            } => {
                let cell = noise::worley(position * frequency.max(1e-4), seed, metric);
                match output {
                    // Clamped rather than normalized: the maximum possible F1
                    // is a diagonal of the neighbourhood, which almost never
                    // occurs, so normalizing by it would leave the useful range
                    // squeezed into the bottom third.
                    CellOutput::Distance => cell.f1.clamp(0.0, 1.0),
                    CellOutput::Edges => (cell.f2 - cell.f1).clamp(0.0, 1.0),
                    CellOutput::Id => cell.id,
                }
            }

            Procedural::Marble {
                axis,
                frequency,
                distortion,
                fractal,
            } => {
                // Bands along the axis, displaced by turbulence. Turbulence
                // rather than plain fBm because its creases — the zero
                // crossings of |noise| — are what give marble its veins rather
                // than a soft wobble.
                // The turbulence at the pattern's own scale, so veins bend the
                // same whatever size the bands are: at the default frequency
                // of 3 this is exactly the classic unit-scale formula.
                let banded = axis.of(position) * frequency
                    + noise::turbulence(position * (frequency / 3.0), seed, fractal) * distortion;
                banded.sin() * 0.5 + 0.5
            }

            Procedural::Wood {
                axis,
                frequency,
                distortion,
                grain,
                fractal,
            } => {
                let (u, v) = axis.across(position);
                let radius = (u * u + v * v).sqrt();
                // Rings, wandering. Perfectly concentric rings read as a
                // target painted on the surface; real growth is never centred.
                // The wander at the rings' own scale and in rings, so it looks
                // the same on a part of any size; at the default frequency of
                // 8 this is exactly the unit-scale formula it replaces.
                let rings = radius * frequency
                    + noise::fbm(position * (frequency / 8.0), seed, fractal) * distortion * 8.0;
                let ring = rings.fract().abs();

                // And the axial streaking, stretched along the grain so it
                // reads as fibre rather than as blotches. Without it the rings
                // look printed on.
                let along = axis.of(position);
                let streak = noise::perlin(
                    vec3(u * frequency * 3.0, along * frequency * 0.08, v * frequency * 3.0),
                    seed ^ 0x51a4_0f37,
                ) * 0.5
                    + 0.5;
                (ring * (1.0 - grain) + streak * grain).clamp(0.0, 1.0)
            }

            Procedural::WoodAdvanced {
                axis,
                frequency,
                distortion,
                grain,
                fractal,
                ring_variation,
                latewood,
                contrast,
                flecks,
                knots,
                board_width,
            } => {
                let frequency = frequency.max(1e-3);
                let (mut u, mut v) = axis.across(position);
                let along = axis.of(position);

                // Planks side by side across the grain, each cut from its own
                // log: the pith somewhere else, the grain turned a little.
                let mut board_seed = seed;
                if board_width > 1e-4 {
                    let board = (u / board_width).floor() as i32;
                    let key = seed ^ 0x2b0a_7d11;
                    let centre = (board as f32 + 0.5) * board_width;
                    u = u - centre + (hash01(board, 0, key) - 0.5) * board_width * 3.0;
                    v += (hash01(board, 1, key) - 0.5) * board_width * 4.0;
                    let (s, c) = ((hash01(board, 2, key) - 0.5) * 0.6).sin_cos();
                    (u, v) = (u * c - v * s, u * s + v * c);
                    board_seed = seed.wrapping_add((board as u32).wrapping_mul(7919));
                }
                let mut radius = (u * u + v * v).sqrt();

                // Knots: branches leaving the pith at a random angle, one
                // chance per stretch of grain. The rings swell round each and
                // its core is dark.
                let mut knot_core = 0.0f32;
                if knots > 0.0 {
                    let key = board_seed ^ 0x6f1d_93a5;
                    let cell_length = 6.0 / frequency;
                    let cell = (along / cell_length).floor() as i32;
                    for k in cell - 1..=cell + 1 {
                        if hash01(k, 3, key) >= knots {
                            continue;
                        }
                        let angle = hash01(k, 4, key) * std::f32::consts::TAU;
                        let height = (k as f32 + hash01(k, 5, key)) * cell_length;
                        let (du, dv) = (angle.cos(), angle.sin());
                        if u * du + v * dv <= 0.0 {
                            continue;
                        }
                        let side = -u * dv + v * du;
                        let d = (side * side + (along - height) * (along - height)).sqrt();
                        let size = 0.6 / frequency * (0.6 + hash01(k, 6, key));
                        radius += (-(d * d) / (size * size)).exp() * size * 2.0;
                        knot_core = knot_core.max((1.0 - d / (size * 0.6)).clamp(0.0, 1.0));
                    }
                }

                // Years of growth, wandering, and not all the same width.
                let years = radius * frequency
                    + noise::fbm(position * (frequency / 8.0), seed, fractal) * distortion * 8.0;
                let uneven = noise::perlin(vec3(years * 0.15, 0.0, 0.0), board_seed ^ 0x1234_5678) * ring_variation;
                let phase = (years + uneven * 2.0).fract().abs();

                // Each ring light and wide, then dark and narrow.
                let late = latewood.clamp(0.02, 0.98);
                let t = ((phase - (1.0 - late)) / late).clamp(0.0, 1.0);
                let profile = t * t * (3.0 - 2.0 * t);
                let ring = phase + (profile - phase) * contrast.clamp(0.0, 1.0);

                // Ray flecks: short thin streaks radiating from the pith.
                let fleck = if flecks > 0.0 {
                    let angle = v.atan2(u);
                    let n = noise::perlin(
                        vec3(angle * 80.0, radius * frequency * 0.6, along * frequency * 0.8),
                        board_seed ^ 0x77aa_1c3d,
                    ) * 0.5
                        + 0.5;
                    let edge = 1.0 - flecks.clamp(0.0, 1.0) * 0.35;
                    let x = ((n - edge) / (1.0 - edge).max(1e-4)).clamp(0.0, 1.0);
                    x * x * (3.0 - 2.0 * x)
                } else {
                    0.0
                };

                let streak = noise::perlin(
                    vec3(u * frequency * 3.0, along * frequency * 0.08, v * frequency * 3.0),
                    seed ^ 0x51a4_0f37,
                ) * 0.5
                    + 0.5;
                let wood = ring * (1.0 - grain) + streak * grain;
                (wood * (1.0 - 0.5 * fleck) + knot_core * (1.0 - wood)).clamp(0.0, 1.0)
            }

            Procedural::Brushed { frequency, stretch } => {
                // Squashing one axis is the whole trick: noise sampled on a
                // grid that is fine across the grain and coarse along it comes
                // out as streaks rather than as blobs.
                let stretched = vec3(
                    position.x * frequency.max(1e-4),
                    position.y * frequency.max(1e-4) / stretch.max(1e-3),
                    position.z * frequency.max(1e-4),
                );
                noise::perlin(stretched, seed) * 0.5 + 0.5
            }

            Procedural::NoiseTexture {
                basis,
                frequency,
                contrast,
                bias,
            } => {
                let single = Fractal {
                    octaves: 1,
                    basis,
                    ..Fractal::default()
                };
                let n = noise::fbm(position * frequency.max(1e-4), seed, single) * 0.5 + 0.5;
                ((n - 0.5) * contrast.max(0.0) + 0.5 + bias).clamp(0.0, 1.0)
            }

            Procedural::Ridged { fractal, frequency } => {
                noise::ridged(position * frequency.max(1e-4), seed, fractal).clamp(0.0, 1.0)
            }

            Procedural::Granite {
                frequency,
                speckle,
                fine,
            } => {
                let q = position * frequency.max(1e-4);
                // Crystals at two sizes, so it is not one grid of cells.
                let large = noise::worley(q, seed, Metric::Euclidean).id;
                let small = noise::worley(q * 2.7, seed ^ 0x3c6e_f372, Metric::Euclidean).id;
                let grain = noise::fbm(
                    q * 9.0,
                    seed ^ 0x2545_f491,
                    Fractal {
                        octaves: 2,
                        ..Fractal::default()
                    },
                ) * 0.5
                    + 0.5;
                let fine = fine.clamp(0.0, 1.0);
                let mut v = (large * 0.55 + small * 0.45) * (1.0 - fine) + grain * fine;
                // Flecks: the brightest and darkest ends of the ramp, in small
                // round grains, which is what makes it read as stone.
                let fleck = noise::worley(q * 5.3, seed ^ 0x9e37_79b9, Metric::Euclidean);
                let speckle = speckle.clamp(0.0, 1.0);
                if fleck.f1 < 0.3 {
                    if fleck.id < speckle * 0.5 {
                        v = 1.0;
                    } else if fleck.id > 1.0 - speckle * 0.5 {
                        v = 0.0;
                    }
                }
                v.clamp(0.0, 1.0)
            }

            Procedural::Leather {
                frequency,
                crack_width,
                wrinkle,
            } => {
                let q = position * frequency.max(1e-4);
                let cell = noise::worley(q, seed, Metric::Euclidean);
                // A dome per cell: the pebble.
                let pebble = 1.0 - smoothstep(0.0, 1.0, cell.f1);
                // Zero on a boundary, rising to one a crack's width away.
                let crack = if crack_width > 0.0 {
                    smoothstep(0.0, crack_width, cell.f2 - cell.f1)
                } else {
                    1.0
                };
                let fine = noise::fbm(
                    q * 6.0,
                    seed ^ 0x6a09_e667,
                    Fractal {
                        octaves: 3,
                        ..Fractal::default()
                    },
                ) * 0.5
                    + 0.5;
                let wrinkle = wrinkle.clamp(0.0, 1.0);
                ((pebble * (1.0 - wrinkle) + fine * wrinkle) * crack).clamp(0.0, 1.0)
            }

            Procedural::Spots {
                frequency,
                coverage,
                variance,
                softness,
            } => {
                let cell = noise::worley(position * frequency.max(1e-4), seed, Metric::Euclidean);
                let radius = coverage.clamp(0.0, 1.0) * 0.8 * (1.0 - variance.clamp(0.0, 1.0) * cell.id);
                if radius <= 0.0 {
                    return 0.0;
                }
                let soft = softness.clamp(0.0, 1.0) * radius * 0.5 + 1e-4;
                1.0 - smoothstep(radius - soft, radius + soft, cell.f1)
            }

            Procedural::Camouflage { frequency, layers } => {
                let layers = layers.clamp(2, 5);
                let mut top = 0;
                for layer in 0..layers {
                    // Each layer finer than the one below, and offset so they
                    // do not share blobs.
                    let scale = frequency.max(1e-4) * (1.0 + layer as f32 * 0.7);
                    let n = noise::fbm(
                        position * scale + vec3(layer as f32 * 17.3, 0.0, layer as f32 * 9.1),
                        seed.wrapping_add(layer * 0x2f6b),
                        Fractal {
                            octaves: 3,
                            ..Fractal::default()
                        },
                    );
                    if n > 0.05 {
                        top = layer + 1;
                    }
                }
                top as f32 / layers as f32
            }

            Procedural::Contour {
                axis,
                frequency,
                width,
                softness,
                distortion,
            } => {
                let bend = if distortion != 0.0 {
                    noise::fbm(
                        position * frequency.max(1e-4) * 0.3,
                        seed,
                        Fractal {
                            octaves: 3,
                            ..Fractal::default()
                        },
                    ) * distortion
                } else {
                    0.0
                };
                let field = axis.of(position) * frequency + bend;
                // Distance to the nearest line, in spacings: 0 on it, 0.5
                // midway between two.
                let d = (field - field.round()).abs();
                let half = width.clamp(0.0, 1.0) * 0.5;
                let soft = softness.clamp(0.0, 1.0) * 0.5 + 1e-4;
                1.0 - smoothstep(half, half + soft, d)
            }

            Procedural::BrushedRadial {
                axis,
                frequency,
                stretch,
            } => {
                let (u, v) = axis.across(position);
                let radius = (u * u + v * v).sqrt();
                let (cu, cv) = if radius > 1e-6 { (u / radius, v / radius) } else { (0.0, 0.0) };
                // Around the circle the noise is read on a circle of its own,
                // so there is no seam where the angle wraps; its radius sets
                // how fast the grain changes along the brushing.
                let around = radius * frequency.max(1e-4) / stretch.max(1e-3);
                noise::perlin(vec3(radius * frequency.max(1e-4), cu * around, cv * around), seed) * 0.5 + 0.5
            }

            // Flat patterns are answered above, before this match.
            Procedural::Scratches { .. } | Procedural::Weave { .. } | Procedural::Mesh { .. } => {
                unreachable!("planar procedurals are evaluated on their plane")
            }
        }
    }
}

#[inline]
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    if edge1 <= edge0 {
        return if x < edge0 { 0.0 } else { 1.0 };
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Where a regular pattern starts, from the seed: two parts cut from the
/// same sheet of fabric or perforated metal are cut from different places in
/// it. Seed 0 leaves the pattern on the origin, for a sheet lined up with the
/// part's edges. `x` and `y` are in pattern units — threads or holes.
/// A flat pattern's point moved by its seed, so two seeds lay two different
/// stretches of the same weave or mesh.
pub fn phase(x: f32, y: f32, seed: u32) -> (f32, f32) {
    if seed == 0 {
        return (x, y);
    }
    // Whole units and a fraction, so the offset moves the pattern rather than
    // only relabelling threads.
    (
        x + hash01(0x5eed, 0x0a11, seed) * 64.0,
        y + hash01(0x0a11, 0x5eed, seed) * 64.0,
    )
}

/// A thread's cross-section: 1 along its centre line, falling to 0 at its
/// edge, 0 beyond. `offset` is the distance from the centre line and `half`
/// the thread's half-width, both in pitches.
#[inline]
fn thread(offset: f32, half: f32) -> f32 {
    if half <= 0.0 || offset >= half {
        return 0.0;
    }
    let t = offset / half;
    (1.0 - t * t).sqrt()
}

/// The over/under weave at `(x, y)`, in threads.
///
/// Warp threads run along `y`, one per unit of `x`; weft threads run along
/// `x`. At each crossing the pattern says which is on top, and the thread on
/// top humps over the crossing — it is higher at its middle, where it passes
/// over, than at the ends, where it dives under the next.
fn weave(x: f32, y: f32, seed: u32, pattern: WeavePattern, width: f32, variance: f32) -> f32 {
    use std::f32::consts::PI;

    let (i, j) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let half = width.clamp(0.0, 1.0) * 0.5;

    let warp_on_top = match pattern {
        WeavePattern::Plain => (i + j).rem_euclid(2) == 0,
        WeavePattern::Twill => (i - j).rem_euclid(3) != 0,
        WeavePattern::Satin => (j - 2 * i).rem_euclid(5) != 0,
    };
    let warp = thread((fx - 0.5).abs(), half);
    let weft = thread((fy - 0.5).abs(), half);
    // Per-thread variation, by the thread's index, so a thread keeps its
    // height all the way along.
    let variance = variance.clamp(0.0, 1.0);
    let warp_scale = 1.0 - variance * hash01(i, 0x7a3d, seed);
    let weft_scale = 1.0 - variance * hash01(0x1f5b, j, seed ^ 0x4b3f_27a1);

    let (top, top_arch, top_scale, under, under_scale) = if warp_on_top {
        (warp, (PI * fy).sin(), warp_scale, weft, weft_scale)
    } else {
        (weft, (PI * fx).sin(), weft_scale, warp, warp_scale)
    };
    let height = if top > 0.0 {
        0.5 + 0.5 * top * (0.6 + 0.4 * top_arch) * top_scale
    } else {
        0.5 * under * under_scale
    };
    height.clamp(0.0, 1.0)
}

/// A perforated sheet at `(x, y)`, in holes: 1 on the material, 0 in a hole.
fn mesh(x: f32, y: f32, shape: HoleShape, size: f32, softness: f32) -> f32 {
    let radius = size.clamp(0.0, 1.0) * 0.5;
    let distance = match shape {
        HoleShape::Round | HoleShape::Square => {
            let (dx, dy) = ((x - x.floor()) - 0.5, (y - y.floor()) - 0.5);
            if shape == HoleShape::Round {
                (dx * dx + dy * dy).sqrt()
            } else {
                dx.abs().max(dy.abs())
            }
        }
        HoleShape::Hex => {
            // A triangular lattice of centres, one unit apart: the nearer of
            // two offset rectangular grids.
            let h = 3.0f32.sqrt();
            let local = |x: f32, y: f32| ((x - x.div_euclid(1.0)) - 0.5, (y - (y / h).floor() * h) - h * 0.5);
            let (ax, ay) = local(x, y);
            let (bx, by) = local(x - 0.5, y - h * 0.5);
            let (dx, dy) = if ax * ax + ay * ay < bx * bx + by * by { (ax, ay) } else { (bx, by) };
            // Hexagonal distance, pointy side up.
            (dx.abs() * 0.866_025 + dy.abs() * 0.5).max(dy.abs())
        }
    };
    let soft = softness.clamp(0.0, 1.0) * 0.25 + 1e-4;
    smoothstep(radius - soft, radius + soft, distance)
}

/// An anisotropic line field: one candidate scratch per cell of a 2D lattice,
/// at a random angle within `spread`, with the distance to the nearest line
/// thresholded by `width`.
///
/// Returns 1 away from any scratch and falls towards 0 inside one, so it reads
/// directly as a roughness or bump multiplier: a scratch is a *local*
/// disturbance in an otherwise intact surface, which is the way round that
/// needs no inversion at the call site.
fn scratches(
    a: f32,
    b: f32,
    seed: u32,
    frequency: f32,
    width: f32,
    density: f32,
    spread: f32,
) -> f32 {
    use std::f32::consts::TAU;

    let p = vec3(a, b, 0.0) * frequency.max(1e-4);
    let base = vec3(p.x.floor(), p.y.floor(), 0.0);

    let width = width.max(1e-4);
    let mut nearest = f32::MAX;

    // The 3x3 neighbourhood: a scratch is up to half a cell long, so it cannot
    // reach further than one cell away.
    for dy in -1..=1 {
        for dx in -1..=1 {
            let cell = vec3(base.x + dx as f32, base.y + dy as f32, 0.0);
            let (cx, cy) = (cell.x as i32, cell.y as i32);

            // Sparse: most cells carry nothing. Compared against a hash of the
            // cell rather than counted, so density is a probability and stays
            // stable as the pattern moves.
            let present = hash01(cx, cy, seed ^ 0x51ed_2701);
            if present > density.clamp(0.0, 1.0) {
                continue;
            }

            let centre = cell
                + vec3(
                    hash01(cx, cy, seed),
                    hash01(cx, cy, seed ^ 0x2f39_11ab),
                    0.0,
                );
            // Within `spread` of the reference direction, so a brushed finish
            // (spread 0) and random wear (spread 0.25) are the same node.
            let angle = (hash01(cx, cy, seed ^ 0x7c93_1055) - 0.5) * spread.clamp(0.0, 0.5) * TAU;
            let direction = vec3(angle.cos(), angle.sin(), 0.0);
            let half_length = 0.2 + hash01(cx, cy, seed ^ 0x11a7_3c4d) * 0.3;

            // Distance from the point to the segment.
            let offset = p - centre;
            let along = offset.dot(direction).clamp(-half_length, half_length);
            let closest = centre + direction * along;
            nearest = nearest.min((p - closest).length());
        }
    }

    if nearest == f32::MAX {
        return 1.0;
    }
    // A soft shoulder rather than a hard threshold: a scratch with a step edge
    // aliases badly under a bump map, which is exactly where these are used.
    (nearest / width).clamp(0.0, 1.0)
}

#[inline]
fn hash01(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd873_2f1b);
    h ^= seed.wrapping_mul(0x9e37_79b9);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A step small enough to resolve the node's finest feature.
    ///
    /// This is not a detail, and getting it wrong twice is what makes it worth
    /// writing down. A fixed step of 0.021 world units against scratches at
    /// frequency 30 samples every 0.63 of a cell, while a scratch is 0.02 of a
    /// cell wide: the grid steps clean over them. Six in ten thousand samples
    /// landed on a scratch, the field read as a constant, and the procedural
    /// was fine the whole time.
    ///
    /// A sampling grid coarser than the thing it is sampling does not measure
    /// a weak signal. It measures nothing, and reports a confident zero.
    fn step_for(p: Procedural) -> f32 {
        match p {
            // Half the width of a scratch, so several samples land across one.
            Procedural::Scratches {
                frequency, width, ..
            } => width / frequency * 0.5,
            Procedural::Noise { frequency, .. }
            | Procedural::Cellular { frequency, .. }
            | Procedural::Brushed { frequency, .. } => 0.08 / frequency,
            // Marble's bands and wood's rings are the finest features in
            // those two, and both are one period per unit of `frequency`.
            Procedural::Marble { frequency, .. }
            | Procedural::Wood { frequency, .. }
            | Procedural::WoodAdvanced { frequency, .. } => {
                0.08 / frequency
            }
            Procedural::NoiseTexture { frequency, .. }
            | Procedural::Ridged { frequency, .. }
            | Procedural::Granite { frequency, .. }
            | Procedural::Leather { frequency, .. }
            | Procedural::Spots { frequency, .. }
            | Procedural::Camouflage { frequency, .. }
            | Procedural::Contour { frequency, .. }
            | Procedural::BrushedRadial { frequency, .. }
            | Procedural::Weave { frequency, .. }
            | Procedural::Mesh { frequency, .. } => 0.08 / frequency,
        }
    }

    /// A plane of sample points, spaced to resolve the node.
    ///
    /// A plane rather than a cube: `Scratches` is a field in x and z with y
    /// unused, so a cube spends most of its samples re-measuring the same
    /// line, and a plane of the same size covers far more of the pattern.
    fn samples(p: Procedural, seed: u32, count: i32) -> Vec<f32> {
        let step = step_for(p);
        let mut out = Vec::with_capacity((count * count) as usize);
        for i in 0..count {
            for k in 0..count {
                out.push(p.evaluate(vec3(i as f32 * step, 0.037, k as f32 * step), seed));
            }
        }
        out
    }

    /// Everything a parameter can be driven by must land in `[0, 1]`. A
    /// procedural that overshot would push a roughness past one or a weight
    /// past full somewhere far from where anyone was looking.
    #[test]
    fn every_procedural_stays_in_range() {
        for procedural in Procedural::catalog() {
            for v in samples(procedural, 5, 220) {
                assert!(
                    (0.0..=1.0).contains(&v),
                    "{} produced {v}",
                    procedural.label()
                );
            }
        }
    }

    /// And each must actually vary. A node that returns a constant is a
    /// control that does nothing, and it would look exactly like a material
    /// with no texture on it — which is the hardest kind of broken to notice.
    #[test]
    fn every_procedural_varies() {
        for procedural in Procedural::catalog() {
            let values = samples(procedural, 5, 220);
            let low = values.iter().copied().fold(f32::MAX, f32::min);
            let high = values.iter().copied().fold(f32::MIN, f32::max);
            assert!(
                high - low > 0.1,
                "{} spans only {low} to {high}",
                procedural.label()
            );
        }
    }

    /// §6: seedable, so duplicated parts differ.
    ///
    /// Measured as the mean change against the field's own spread, not as a
    /// count of pixels that moved. Counting was the first attempt and it is
    /// wrong for a sparse node: scratches cover a couple of percent of a
    /// surface by design, so reseeding can only ever change a couple of
    /// percent of it, and a threshold written for a dense field reads that
    /// correct behaviour as a failure. Against the field's own spread the
    /// question becomes the one that matters — did the pattern move, relative
    /// to how much pattern there is — and it is the same question for all four
    /// nodes.
    #[test]
    fn the_seed_changes_every_procedural() {
        for procedural in Procedural::catalog() {
            let a = samples(procedural, 1, 220);
            let b = samples(procedural, 2, 220);

            let mean = a.iter().sum::<f32>() / a.len() as f32;
            let spread =
                (a.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / a.len() as f32).sqrt();
            let change =
                a.iter().zip(&b).map(|(x, y)| (x - y).abs()).sum::<f32>() / a.len() as f32;

            assert!(spread > 1e-3, "{} has no structure at all", procedural.label());
            assert!(
                change > spread * 0.2,
                "{} changed by {change} against a spread of {spread}; the seed \
                 barely moves it",
                procedural.label()
            );
        }
    }

    /// Deterministic, for the same reason the primitives are: a renderer
    /// averages hundreds of samples of one pixel, and a procedural that
    /// differed between them would be adding its own noise to the transport's.
    #[test]
    fn every_procedural_is_deterministic() {
        for procedural in Procedural::catalog() {
            assert_eq!(samples(procedural, 9, 60), samples(procedural, 9, 60));
        }
    }

    /// Scratches are *sparse*. The whole point is a mostly intact surface with
    /// occasional damage — a scratch in every cell is a texture, and it is the
    /// difference between a part that has been handled and a part that has been
    /// sandblasted.
    #[test]
    fn scratches_leave_most_of_the_surface_alone() {
        let sparse = Procedural::Scratches {
            frequency: 8.0,
            width: 0.06,
            density: 0.3,
            spread: 0.05,
        };
        let values = samples(sparse, 3, 220);
        let untouched = values.iter().filter(|v| **v > 0.99).count() as f32 / values.len() as f32;
        assert!(
            untouched > 0.5,
            "only {:.0}% of the surface is unscratched",
            untouched * 100.0
        );
        // And some of it genuinely is scratched, or the node does nothing.
        let scratched = values.iter().filter(|v| **v < 0.5).count();
        assert!(scratched > 0, "nothing was scratched at all");
    }

    /// Density does what it says, monotonically. A control that is not
    /// monotone in its parameter is a control nobody can aim.
    #[test]
    fn more_density_means_more_scratches() {
        let at = |density: f32| {
            let p = Procedural::Scratches {
                frequency: 8.0,
                width: 0.06,
                density,
                spread: 0.05,
            };
            samples(p, 3, 220).iter().filter(|v| **v < 0.5).count()
        };
        let (light, heavy) = (at(0.1), at(0.8));
        assert!(
            heavy > light,
            "density 0.8 scratched {heavy} against 0.1's {light}"
        );
    }

    /// Brushing is *directional*: the field varies quickly across the grain and
    /// slowly along it. Without that it is just noise, and the anisotropic
    /// highlight it exists to feed has nothing to line up with.
    #[test]
    fn brushing_runs_along_its_grain() {
        let brushed = Procedural::Brushed {
            frequency: 60.0,
            stretch: 40.0,
        };
        let step = 0.004;
        let change = |axis: Vec3| {
            let mut total = 0.0;
            for i in 0..200 {
                let p = vec3(0.13, 0.21, 0.07) + axis * (i as f32 * step);
                let q = p + axis * step;
                total += (brushed.evaluate(q, 4) - brushed.evaluate(p, 4)).abs();
            }
            total
        };
        // The stretch is applied to y, so y is the grain direction.
        let along = change(vec3(0.0, 1.0, 0.0));
        let across = change(vec3(1.0, 0.0, 0.0));
        assert!(
            across > along * 4.0,
            "across the grain changes {across} and along it {along}; that is not a brush"
        );
    }

    /// A point at `(u, v)` in the plane across `axis`, `along` units up it.
    fn point_about(axis: Axis, u: f32, v: f32, along: f32) -> Vec3 {
        match axis {
            Axis::X => vec3(along, u, v),
            Axis::Y => vec3(u, along, v),
            Axis::Z => vec3(u, v, along),
        }
    }

    /// The total absolute change walking `steps` of `step` from `from`.
    fn travel(p: Procedural, from: Vec3, direction: Vec3, step: f32, steps: i32) -> f32 {
        let mut total = 0.0;
        let mut previous = p.evaluate(from, 3);
        for i in 1..steps {
            let value = p.evaluate(from + direction * (i as f32 * step), 3);
            total += (value - previous).abs();
            previous = value;
        }
        total
    }

    /// Marble is *bands*, and bands have a direction: the field varies along
    /// the axis and not across it.
    ///
    /// Tested with the distortion at zero, which reduces the node to
    /// `sin(axis . frequency)` — a closed form with an exactly known answer.
    /// That is what makes this analytic rather than a vibe check: with the
    /// turbulence off there is nothing left to be uncertain about, and an axis
    /// read in place of another fails immediately. An axis mix-up is the most
    /// likely mistake in the node and the hardest to see in a render, where
    /// bands in the wrong direction still look like bands.
    #[test]
    fn marble_bands_run_across_its_axis() {
        for axis in Axis::ALL {
            let marble = Procedural::Marble {
                axis,
                frequency: 4.0,
                distortion: 0.0,
                fractal: Fractal::default(),
            };
            let start = vec3(0.11, 0.13, 0.17);
            let along = travel(marble, start, point_about(axis, 0.0, 0.0, 1.0), 0.01, 200);
            // Across the axis an undistorted marble is exactly constant: the
            // formula does not read those components at all.
            let across = travel(marble, start, point_about(axis, 1.0, 0.0, 0.0), 0.01, 200);

            assert!(
                along > 1.0,
                "{axis:?}: the field barely moves along its own axis ({along})"
            );
            assert!(
                across < 1e-5,
                "{axis:?}: an undistorted marble varies across its axis by \
                 {across}; the axis is being read wrong"
            );
        }
    }

    /// And the distortion is what turns bands into veins. At zero the field is
    /// straight stripes; with it up, the same walk across the axis varies —
    /// which is the turbulence bending the bands, and the whole difference
    /// between marble and a barcode.
    #[test]
    fn marble_distortion_bends_the_bands() {
        let across = |distortion: f32| {
            travel(
                Procedural::Marble {
                    axis: Axis::Y,
                    frequency: 4.0,
                    distortion,
                    fractal: Fractal::default(),
                },
                vec3(0.0, 0.21, 0.0),
                vec3(1.0, 0.0, 0.0),
                0.01,
                200,
            )
        };
        assert!(across(0.0) < 1e-5, "straight bands already wander");
        let bent = across(3.0);
        assert!(bent > 0.5, "turbulence at 3.0 bent the bands by only {bent}");
    }

    /// Veins are veins at any size. Frequencies are per metre, so a marble on
    /// a real part has bands centimetres apart — and the turbulence that
    /// bends them was sampled at the raw position, making features a metre
    /// across that never changed over a part the size of a hand: the veins
    /// came out as plain straight stripes. Scaled up thirty times and walked
    /// a thirtieth as far, a marble has to bend as much as before.
    #[test]
    fn marble_veins_bend_at_the_scale_of_a_real_part() {
        let across = |frequency: f32, step: f32| {
            travel(
                Procedural::Marble { axis: Axis::Y, frequency, distortion: 3.0, fractal: Fractal::default() },
                vec3(0.0, 0.21, 0.0) * (4.0 / frequency),
                vec3(1.0, 0.0, 0.0),
                step,
                200,
            )
        };
        let unit = across(4.0, 0.01);
        let real = across(120.0, 0.01 / 30.0);
        assert!(real > unit * 0.5, "bands bent by {unit} at unit scale and {real} at a real part's");
    }

    /// Wood is *rings*, and rings are concentric: every point at one distance
    /// from the grain axis gets the same value, at any angle and anywhere
    /// along the axis — a ring is a cylinder, not a circle.
    ///
    /// Distortion and grain at zero reduce the node to `frac(radius .
    /// frequency)`, which is again a closed form. Rings built about the wrong
    /// axis fail this at the first angle.
    /// Growth rings wander at the scale of the rings, whatever that is.
    /// The wander was noise sampled at the raw position — features a metre
    /// across — with an amplitude that grew with the ring count, so on a part
    /// a few centimetres across the rings did not wander at all: they were
    /// shifted wholesale, perfectly round, and a turned piece of oak read as
    /// a target. Frequencies are per metre, so the same wood at thirty times
    /// the frequency is the same wood thirty times smaller, point for point.
    #[test]
    fn wood_is_the_same_wood_at_any_scale() {
        for advanced in [false, true] {
            let wood = |frequency: f32| {
                if advanced {
                    Procedural::WoodAdvanced {
                        axis: Axis::Y, frequency, distortion: 0.35, grain: 0.0, fractal: Fractal::default(),
                        ring_variation: 0.5, latewood: 0.3, contrast: 1.0, flecks: 0.0, knots: 0.0, board_width: 0.0,
                    }
                } else {
                    Procedural::Wood { axis: Axis::Y, frequency, distortion: 0.35, grain: 0.0, fractal: Fractal::default() }
                }
            };
            let mut worst = 0.0f32;
            for i in 0..200 {
                let t = i as f32 * 0.37;
                let p = vec3(0.3 + t.sin() * 0.9, t * 0.05, 0.2 + t.cos() * 0.7);
                let big = wood(8.0).evaluate(p, 5);
                let small = wood(240.0).evaluate(p * (1.0 / 30.0), 5);
                worst = worst.max((big - small).abs());
            }
            assert!(
                worst < 1e-2,
                "{}: the same wood thirty times smaller differs by up to {worst}",
                if advanced { "advanced wood" } else { "wood" }
            );
        }
    }

    #[test]
    fn wood_rings_are_concentric_about_its_axis() {
        use std::f32::consts::TAU;

        for axis in Axis::ALL {
            let wood = Procedural::Wood {
                axis,
                frequency: 6.0,
                distortion: 0.0,
                grain: 0.0,
                fractal: Fractal::default(),
            };
            let radius = 0.37;
            let reference = wood.evaluate(point_about(axis, radius, 0.0, 0.0), 2);

            for i in 0..64 {
                let angle = i as f32 / 64.0 * TAU;
                let along = (i as f32 - 32.0) * 0.05;
                let value = wood.evaluate(
                    point_about(axis, radius * angle.cos(), radius * angle.sin(), along),
                    2,
                );
                assert!(
                    (value - reference).abs() < 1e-4,
                    "{axis:?}: radius {radius} reads {value} at angle {angle} against \
                     {reference} at zero; the rings are not concentric"
                );
            }

            // Half a ring further out is a different place in the cycle, or
            // there are no rings.
            let nearer = wood.evaluate(point_about(axis, radius + 0.5 / 6.0, 0.0, 0.0), 2);
            assert!(
                (nearer - reference).abs() > 0.2,
                "{axis:?}: half a ring out reads {nearer} against {reference}"
            );
        }
    }

    /// The grain control mixes in the axial streaking, and at zero contributes
    /// nothing — so "no grain" is no grain rather than a faint always-on wash.
    /// With rings alone, travelling along the axis cannot change anything,
    /// because every term reads only the radius.
    #[test]
    fn wood_grain_is_off_at_zero_and_on_above_it() {
        let wood = |grain: f32| Procedural::Wood {
            axis: Axis::Y,
            frequency: 6.0,
            distortion: 0.0,
            grain,
            fractal: Fractal::default(),
        };
        let along = |grain: f32| {
            travel(
                wood(grain),
                vec3(0.3, 0.0, 0.0),
                vec3(0.0, 1.0, 0.0),
                0.02,
                100,
            )
        };
        let off = along(0.0);
        assert!(
            off < 1e-4,
            "with grain at zero the field still varies along the axis by {off}"
        );
        let on = along(0.6);
        assert!(on > 0.05, "grain at 0.6 varies by only {on}");
    }

    /// The cellular outputs are three different pictures, and the edge mask in
    /// particular must reach zero — it is what draws a crack network, and a
    /// version that never touched zero would draw a soft haze instead.
    #[test]
    fn the_cellular_outputs_differ_and_the_edges_reach_zero() {
        let of = |output: CellOutput| {
            samples(
                Procedural::Cellular {
                    metric: Metric::Euclidean,
                    output,
                    frequency: 6.0,
                },
                8,
                160,
            )
        };
        let distance = of(CellOutput::Distance);
        let edges = of(CellOutput::Edges);
        let ids = of(CellOutput::Id);

        let differ = |a: &[f32], b: &[f32]| {
            a.iter().zip(b).filter(|(x, y)| (*x - *y).abs() > 1e-3).count()
        };
        assert!(differ(&distance, &edges) > distance.len() / 2);
        assert!(differ(&distance, &ids) > distance.len() / 2);

        let smallest = edges.iter().copied().fold(f32::MAX, f32::min);
        assert!(
            smallest < 0.02,
            "the closest the edge mask came to a boundary was {smallest}"
        );
    }
    fn mean(values: &[f32]) -> f32 {
        values.iter().sum::<f32>() / values.len() as f32
    }

    fn deviation(values: &[f32]) -> f32 {
        let m = mean(values);
        (values.iter().map(|v| (v - m).powi(2)).sum::<f32>() / values.len() as f32).sqrt()
    }

    /// Contrast widens the spread about the middle, and bias moves it.
    #[test]
    fn noise_texture_contrast_and_bias_do_what_they_say() {
        let at = |contrast: f32, bias: f32| {
            samples(
                Procedural::NoiseTexture {
                    basis: Basis::Perlin,
                    frequency: 8.0,
                    contrast,
                    bias,
                },
                4,
                120,
            )
        };
        let plain = at(1.0, 0.0);
        let hard = at(3.0, 0.0);
        assert!(
            deviation(&hard) > deviation(&plain) * 1.5,
            "contrast 3 spreads {} against {}",
            deviation(&hard),
            deviation(&plain)
        );
        let raised = at(1.0, 0.2);
        assert!(mean(&raised) > mean(&plain) + 0.15, "bias 0.2 moved the mean by {}", mean(&raised) - mean(&plain));
    }

    /// Granite's flecks sit at the two ends of the ramp, and the speckle
    /// control is what puts them there.
    #[test]
    fn granite_speckle_adds_flecks_at_both_ends() {
        let extremes = |speckle: f32| {
            let values = samples(
                Procedural::Granite {
                    frequency: 10.0,
                    speckle,
                    fine: 0.3,
                },
                6,
                200,
            );
            let bright = values.iter().filter(|v| **v > 0.995).count();
            let dark = values.iter().filter(|v| **v < 0.005).count();
            (bright, dark, values.len())
        };
        let (bright, dark, total) = extremes(0.0);
        assert!(bright + dark < total / 1000, "no speckle still left {bright} bright and {dark} dark");
        let (bright, dark, total) = extremes(0.4);
        assert!(bright > total / 200 && dark > total / 200, "speckle 0.4 made {bright} bright and {dark} dark flecks");
    }

    /// Leather's cracks run to zero along the cell boundaries, and a crack
    /// width of zero turns them off.
    #[test]
    fn leather_cracks_follow_the_cells_and_can_be_turned_off() {
        let low = |crack_width: f32| {
            samples(
                Procedural::Leather {
                    frequency: 25.0,
                    crack_width,
                    wrinkle: 0.25,
                },
                2,
                200,
            )
            .iter()
            .filter(|v| **v < 0.03)
            .count()
        };
        let (cracked, smooth) = (low(0.15), low(0.0));
        assert!(cracked > smooth * 3 + 20, "cracks at 0.15 left {cracked} low samples against {smooth} without");
    }

    /// More coverage is more spot, and none is none.
    #[test]
    fn spot_coverage_is_monotone_and_zero_is_empty() {
        let at = |coverage: f32| {
            mean(&samples(
                Procedural::Spots {
                    frequency: 8.0,
                    coverage,
                    variance: 0.3,
                    softness: 0.1,
                },
                5,
                150,
            ))
        };
        assert_eq!(at(0.0), 0.0);
        let (small, large) = (at(0.3), at(0.7));
        assert!(large > small + 0.1, "coverage 0.7 covers {large} against 0.3's {small}");
    }

    /// Camouflage is a stack of flat colours: only whole layer steps, and
    /// more than two of them showing.
    #[test]
    fn camouflage_is_flat_layers() {
        let layers = 4;
        let values = samples(Procedural::Camouflage { frequency: 3.0, layers }, 7, 200);
        let mut seen = std::collections::BTreeSet::new();
        for v in &values {
            let step = v * layers as f32;
            assert!((step - step.round()).abs() < 1e-5, "{v} is not a whole layer");
            seen.insert(step.round() as u32);
        }
        assert!(seen.len() >= 3, "only layers {seen:?} showed");
    }

    /// Undistorted, contour lines sit exactly at whole steps along the axis,
    /// with nothing midway, and do not change across it.
    #[test]
    fn contour_lines_sit_at_even_steps() {
        let contour = Procedural::Contour {
            axis: Axis::Y,
            frequency: 10.0,
            width: 0.1,
            softness: 0.05,
            distortion: 0.0,
        };
        for k in 1..6 {
            let on = contour.evaluate(vec3(0.3, k as f32 / 10.0, 0.2), 1);
            let between = contour.evaluate(vec3(0.3, (k as f32 + 0.5) / 10.0, 0.2), 1);
            assert!(on > 0.99 && between < 0.01, "line {k}: {on} on it, {between} between");
            let across = contour.evaluate(vec3(0.9, k as f32 / 10.0, -0.4), 1);
            assert!((across - on).abs() < 1e-5, "the line moved across the axis");
        }
    }

    /// Radial brushing goes round: it changes fast across the radius and
    /// slowly around the circle, and it has no seam where the angle wraps.
    #[test]
    fn radial_brushing_runs_in_circles() {
        use std::f32::consts::TAU;
        let brushed = Procedural::BrushedRadial {
            axis: Axis::Y,
            frequency: 200.0,
            stretch: 60.0,
        };
        let radius = 0.3;
        let at = |r: f32, angle: f32| brushed.evaluate(vec3(r * angle.cos(), 0.0, r * angle.sin()), 4);
        let step = 0.0005;
        let mut across = 0.0;
        let mut around = 0.0;
        for i in 0..200 {
            let r = radius + i as f32 * step;
            across += (at(r + step, 1.0) - at(r, 1.0)).abs();
            // The same arc length, around the circle.
            let angle = 1.0 + i as f32 * step / radius;
            around += (at(radius, angle + step / radius) - at(radius, angle)).abs();
        }
        assert!(across > around * 4.0, "across {across}, around {around}: that is not brushing in circles");
        let seam = (at(radius, TAU - 1e-4) - at(radius, 1e-4)).abs();
        assert!(seam < 0.02, "the grain jumps by {seam} where the angle wraps");
    }

    /// A weave is flat: the height off the plane does not matter, and a
    /// mapping's (u, v) lands on the same plane as object space's (x, z).
    #[test]
    fn a_weave_is_flat_and_maps_onto_uv() {
        let weave = Procedural::Weave {
            pattern: WeavePattern::Twill,
            frequency: 40.0,
            width: 0.85,
            variance: 0.2,
        };
        for i in 0..50 {
            let (u, v) = (i as f32 * 0.0137, i as f32 * 0.0091);
            let flat = weave.evaluate(vec3(u, 0.0, v), 3);
            assert_eq!(flat, weave.evaluate(vec3(u, 5.0, v), 3), "the weave varies off its plane");
            assert_eq!(flat, weave.evaluate_uv(u, v, 3), "(u, v) and (x, z) disagree");
        }
    }

    /// The weave patterns differ, and threads narrower than the pitch leave
    /// gaps.
    #[test]
    fn weave_patterns_differ_and_narrow_threads_leave_gaps() {
        let of = |pattern: WeavePattern, width: f32| {
            samples(
                Procedural::Weave {
                    pattern,
                    frequency: 40.0,
                    width,
                    variance: 0.0,
                },
                0,
                150,
            )
        };
        let plain = of(WeavePattern::Plain, 0.85);
        for other in [WeavePattern::Twill, WeavePattern::Satin] {
            let changed = plain.iter().zip(of(other, 0.85)).filter(|(a, b)| (**a - *b).abs() > 1e-3).count();
            assert!(changed > plain.len() / 10, "{other:?} matches plain in all but {changed} samples");
        }
        // Gaps as area: full-width threads still meet at a point where four
        // threads' edges touch, which a sample can land on exactly.
        let gaps = |values: &[f32]| values.iter().filter(|v| **v == 0.0).count() as f32 / values.len() as f32;
        assert!(gaps(&plain) > 0.01, "threads 0.85 of the pitch wide left {} gap", gaps(&plain));
        let full = gaps(&of(WeavePattern::Plain, 1.0));
        assert!(full < 0.005, "full-width threads left {full} gap");
    }

    /// A perforated sheet has a hole at each lattice centre and material at
    /// the corners, and bigger holes leave less material.
    #[test]
    fn mesh_holes_sit_on_the_lattice_and_grow_with_size() {
        let mesh = |shape: HoleShape, size: f32| Procedural::Mesh {
            shape,
            frequency: 20.0,
            size,
            softness: 0.05,
        };
        for shape in [HoleShape::Round, HoleShape::Square] {
            let sheet = mesh(shape, 0.6);
            assert!(sheet.evaluate(vec3(0.5 / 20.0, 0.0, 0.5 / 20.0), 0) < 0.01, "{shape:?}: no hole at the centre");
            assert!(sheet.evaluate(vec3(0.0, 0.0, 0.0), 0) > 0.99, "{shape:?}: a hole at the corner");
        }
        for shape in [HoleShape::Round, HoleShape::Square, HoleShape::Hex] {
            let (small, large) = (mean(&samples(mesh(shape, 0.3), 0, 150)), mean(&samples(mesh(shape, 0.8), 0, 150)));
            assert!(large < small - 0.1, "{shape:?}: size 0.8 leaves {large} of the sheet against 0.3's {small}");
        }
    }

    /// Under a mapping a flat pattern reads (u, v). Reading it as the solid
    /// position (u, v, 0) put v on the axis a flat pattern ignores, and a
    /// mapped scratch field varied along u only.
    #[test]
    fn a_mapped_flat_pattern_varies_along_v() {
        let scratches = Procedural::catalog()[2];
        assert!(scratches.is_planar());
        let along_v = (0..400)
            .map(|i| scratches.evaluate_uv(0.31, i as f32 * 0.003, 5))
            .fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)));
        assert!(along_v.1 - along_v.0 > 0.5, "along v the scratches span only {along_v:?}");
    }

    fn advanced_wood(board_width: f32, knots: f32, ring_variation: f32) -> Procedural {
        Procedural::WoodAdvanced {
            axis: Axis::Y,
            frequency: 8.0,
            distortion: 0.3,
            grain: 0.2,
            fractal: Fractal {
                octaves: 3,
                ..Fractal::default()
            },
            ring_variation,
            latewood: 0.3,
            contrast: 0.8,
            flecks: 0.3,
            knots,
            board_width,
        }
    }

    fn sample_line(p: &Procedural, from: Vec3, to: Vec3, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let t = i as f32 / (n - 1) as f32;
                p.evaluate(from + (to - from) * t, 7)
            })
            .collect()
    }

    /// Every value is between 0 and 1 and the same each time it is asked.
    #[test]
    fn advanced_wood_is_bounded_and_repeatable() {
        let p = advanced_wood(0.3, 0.5, 0.6);
        for v in sample_line(&p, vec3(-1.0, -2.0, -0.7), vec3(1.3, 2.0, 0.9), 400) {
            assert!((0.0..=1.0).contains(&v), "{v}");
        }
        assert_eq!(p.evaluate(vec3(0.3, 0.2, 0.1), 7), p.evaluate(vec3(0.3, 0.2, 0.1), 7));
    }

    /// Each of the advanced features changes the wood: rings of uneven width,
    /// knots, and boards that each come from a different log — across a
    /// board's edge the pattern jumps, where within a board it does not.
    #[test]
    fn advanced_wood_features_each_change_the_board() {
        let line = |p: &Procedural| sample_line(p, vec3(0.05, 0.0, 0.1), vec3(0.9, 1.5, 0.4), 300);
        let differs = |a: &[f32], b: &[f32]| a.iter().zip(b).filter(|(x, y)| (*x - *y).abs() > 1e-3).count();
        let plain = line(&advanced_wood(0.0, 0.0, 0.0));
        assert!(differs(&plain, &line(&advanced_wood(0.0, 0.0, 0.9))) > 30, "ring variation changed nothing");
        assert!(differs(&plain, &line(&advanced_wood(0.0, 1.0, 0.0))) > 5, "knots changed nothing");

        // Boards 0.25 wide: compare the jump across an edge with steps inside.
        let boards = advanced_wood(0.25, 0.0, 0.0);
        let at = |x: f32| boards.evaluate(vec3(x, 0.4, 0.13), 7);
        let mut across = 0.0;
        let mut within = 0.0;
        for edge in 1..8 {
            let x = edge as f32 * 0.25;
            across += (at(x + 0.001) - at(x - 0.001)).abs();
            within += (at(x - 0.1 + 0.001) - at(x - 0.1 - 0.001)).abs();
        }
        assert!(across > within * 3.0, "boards do not change at their edges: {across} across, {within} within");
    }
}
