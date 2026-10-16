//! A colour ramp: a scalar in, a colour out.
//!
//! This is the piece that makes a procedural worth attaching to a *colour*.
//! Every pattern in [`procedural`] produces one number, which says everything
//! a roughness or a bump needs and nothing a colour does. Wood is not a grey
//! field; it is a grey field read through two browns. Marble is veins read
//! through a ramp, granite is cells read through a palette, and each of them is
//! the same noise with a different ramp in front of it.
//!
//! So the ramp is not a convenience node. It is the reason the procedurals in
//! this crate can only drive scalars today, and adding it is what lets them
//! drive anything.
//!
//! [`procedural`]: crate::procedural

use tequila_core::spectrum::{spectrum, Spectrum};

/// How a ramp goes from one stop to the next, as Blender's Color Ramp offers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Interpolation {
    /// Straight from one colour to the next.
    #[default]
    Linear,
    /// Easing out of one stop and into the next: smoothstep between them.
    Ease,
    /// Each stop's colour held until the next: bands.
    Constant,
}

impl Interpolation {
    pub const ALL: [Interpolation; 3] = [Interpolation::Linear, Interpolation::Ease, Interpolation::Constant];

    /// Its stable name, in files.
    pub fn key(self) -> &'static str {
        match self {
            Interpolation::Linear => "linear",
            Interpolation::Ease => "ease",
            Interpolation::Constant => "constant",
        }
    }

    pub fn from_key(key: &str) -> Option<Interpolation> {
        Interpolation::ALL.into_iter().find(|i| i.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            Interpolation::Linear => "Linear",
            Interpolation::Ease => "Ease",
            Interpolation::Constant => "Constant",
        }
    }
}

/// One colour, at one position along the ramp.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stop {
    /// Where it sits, 0 to 1.
    pub at: f32,
    pub color: Spectrum,
}

/// A colour ramp.
///
/// Held as a list rather than as a fixed pair because two stops is the common
/// case and not the only one: a wood grain wants earlywood, latewood and the
/// dark line between them, and forcing that into two stops is what makes
/// procedural wood look like plastic.
#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    /// Sorted by position. Kept sorted by [`Gradient::new`] rather than at
    /// evaluation time, because evaluation runs per shading sample and sorting
    /// there would be a sort per ray.
    stops: Vec<Stop>,
    interpolation: Interpolation,
}

impl Default for Gradient {
    fn default() -> Self {
        Gradient::two(Spectrum::splat(0.05), Spectrum::splat(0.8))
    }
}

impl Gradient {
    /// From stops in any order. Empty yields mid-grey, since a ramp with
    /// nothing in it still has to answer.
    pub fn new(mut stops: Vec<Stop>) -> Gradient {
        stops.retain(|s| s.at.is_finite());
        for stop in &mut stops {
            stop.at = stop.at.clamp(0.0, 1.0);
        }
        stops.sort_by(|a, b| a.at.total_cmp(&b.at));
        Gradient { stops, interpolation: Interpolation::Linear }
    }

    /// The same ramp, going between its stops `interpolation`'s way.
    pub fn with_interpolation(mut self, interpolation: Interpolation) -> Gradient {
        self.interpolation = interpolation;
        self
    }

    pub fn interpolation(&self) -> Interpolation {
        self.interpolation
    }

    /// The same ramp with other stops, its interpolation kept.
    pub fn with_stops(&self, stops: Vec<Stop>) -> Gradient {
        Gradient::new(stops).with_interpolation(self.interpolation)
    }

    pub fn two(low: Spectrum, high: Spectrum) -> Gradient {
        Gradient {
            stops: vec![
                Stop {
                    at: 0.0,
                    color: low,
                },
                Stop {
                    at: 1.0,
                    color: high,
                },
            ],
            interpolation: Interpolation::Linear,
        }
    }

    pub fn stops(&self) -> &[Stop] {
        &self.stops
    }

    /// Add a stop, keeping the order.
    pub fn insert(&mut self, stop: Stop) {
        self.stops.push(Stop {
            at: stop.at.clamp(0.0, 1.0),
            ..stop
        });
        self.stops.sort_by(|a, b| a.at.total_cmp(&b.at));
    }

    /// Remove one by index, refusing to leave fewer than two.
    ///
    /// A ramp of one colour is a constant, and a ramp of none has nothing to
    /// answer with — neither is a state the interface should be able to reach
    /// by clicking a remove button one time too many.
    pub fn remove(&mut self, index: usize) -> bool {
        if self.stops.len() <= 2 || index >= self.stops.len() {
            return false;
        }
        self.stops.remove(index);
        true
    }

    /// The colour at `t`, clamped outside `[0, 1]`.
    pub fn evaluate(&self, t: f32) -> Spectrum {
        let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
        match self.stops.len() {
            0 => Spectrum::splat(0.5),
            1 => self.stops[0].color,
            _ => {
                // Held flat outside the first and last stops rather than
                // extrapolated: extrapolating a colour ramp runs the channels
                // out of range and produces colours nobody put in it.
                if t <= self.stops[0].at {
                    return self.stops[0].color;
                }
                let last = self.stops.len() - 1;
                if t >= self.stops[last].at {
                    return self.stops[last].color;
                }
                let upper = self
                    .stops
                    .iter()
                    .position(|s| s.at >= t)
                    .unwrap_or(last)
                    .max(1);
                let (a, b) = (&self.stops[upper - 1], &self.stops[upper]);
                let span = b.at - a.at;
                if span <= 0.0 {
                    // Two stops at the same position are a hard edge, which is
                    // a legitimate thing to want — the line between earlywood
                    // and latewood is one.
                    return b.color;
                }
                let f = (t - a.at) / span;
                match self.interpolation {
                    Interpolation::Linear => a.color.lerp(b.color, f),
                    Interpolation::Ease => a.color.lerp(b.color, f * f * (3.0 - 2.0 * f)),
                    Interpolation::Constant if t >= b.at => b.color,
                    Interpolation::Constant => a.color,
                }
            }
        }
    }

    /// The ramps worth offering by name.
    pub fn presets() -> [(&'static str, Gradient); 4] {
        [
            ("Greyscale", Gradient::default()),
            (
                "Wood",
                Gradient::new(vec![
                    Stop {
                        at: 0.0,
                        color: spectrum(0.16, 0.085, 0.04),
                    },
                    Stop {
                        at: 0.45,
                        color: spectrum(0.33, 0.19, 0.095),
                    },
                    // A hard step: the boundary between the dense latewood of
                    // one year and the open earlywood of the next is abrupt,
                    // and a smooth ramp there is what makes procedural wood
                    // read as plastic.
                    Stop {
                        at: 0.46,
                        color: spectrum(0.11, 0.055, 0.025),
                    },
                    Stop {
                        at: 1.0,
                        color: spectrum(0.38, 0.23, 0.12),
                    },
                ]),
            ),
            (
                "Marble",
                Gradient::new(vec![
                    Stop {
                        at: 0.0,
                        color: spectrum(0.72, 0.71, 0.68),
                    },
                    Stop {
                        at: 0.75,
                        color: spectrum(0.55, 0.55, 0.54),
                    },
                    Stop {
                        at: 1.0,
                        color: spectrum(0.13, 0.14, 0.16),
                    },
                ]),
            ),
            (
                "Rust",
                Gradient::new(vec![
                    Stop {
                        at: 0.0,
                        color: spectrum(0.20, 0.075, 0.025),
                    },
                    Stop {
                        at: 0.55,
                        color: spectrum(0.42, 0.17, 0.05),
                    },
                    Stop {
                        at: 1.0,
                        color: spectrum(0.58, 0.36, 0.16),
                    },
                ]),
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Blender's Color Ramp modes: ease is smoothstep between two stops,
    /// constant holds each stop's colour up to the next.
    #[test]
    fn interpolation_eases_and_holds() {
        let ramp = Gradient::two(Spectrum::ZERO, Spectrum::ONE);
        let at = |i: Interpolation, t: f32| ramp.clone().with_interpolation(i).evaluate(t).r;
        assert!((at(Interpolation::Linear, 0.25) - 0.25).abs() < 1e-6);
        assert!((at(Interpolation::Ease, 0.25) - 0.15625).abs() < 1e-6);
        assert!((at(Interpolation::Ease, 0.5) - 0.5).abs() < 1e-6);
        assert_eq!(at(Interpolation::Constant, 0.99), 0.0);
        assert_eq!(at(Interpolation::Constant, 1.0), 1.0);
        // Editing the stops keeps the mode.
        let eased = ramp.clone().with_interpolation(Interpolation::Ease);
        assert_eq!(eased.with_stops(ramp.stops().to_vec()).interpolation(), Interpolation::Ease);
    }

    #[test]
    fn a_ramp_hits_its_stops_exactly() {
        let g = Gradient::two(spectrum(1.0, 0.0, 0.0), spectrum(0.0, 0.0, 1.0));
        assert_eq!(g.evaluate(0.0), spectrum(1.0, 0.0, 0.0));
        assert_eq!(g.evaluate(1.0), spectrum(0.0, 0.0, 1.0));
        let middle = g.evaluate(0.5);
        assert!((middle.r - 0.5).abs() < 1e-5 && (middle.b - 0.5).abs() < 1e-5);
    }

    /// Held flat outside the ends rather than extrapolated. Extrapolating runs
    /// the channels out of range and produces colours nobody put in the ramp —
    /// and since the input is a procedural that is already clamped, the only
    /// way to reach outside is a bug, where a sensible answer beats a lurid
    /// one.
    #[test]
    fn it_holds_flat_outside_its_range() {
        let g = Gradient::new(vec![
            Stop {
                at: 0.25,
                color: spectrum(0.2, 0.2, 0.2),
            },
            Stop {
                at: 0.75,
                color: spectrum(0.8, 0.8, 0.8),
            },
        ]);
        assert_eq!(g.evaluate(0.0), spectrum(0.2, 0.2, 0.2));
        assert_eq!(g.evaluate(-5.0), spectrum(0.2, 0.2, 0.2));
        assert_eq!(g.evaluate(1.0), spectrum(0.8, 0.8, 0.8));
        assert_eq!(g.evaluate(f32::NAN), spectrum(0.2, 0.2, 0.2));
    }

    /// Stops given in any order sort themselves, once, at construction — not
    /// at evaluation, which runs per shading sample and would be a sort per
    /// ray.
    #[test]
    fn stops_are_sorted_on_construction() {
        let g = Gradient::new(vec![
            Stop {
                at: 0.9,
                color: spectrum(0.0, 0.0, 0.0),
            },
            Stop {
                at: 0.1,
                color: spectrum(1.0, 1.0, 1.0),
            },
        ]);
        assert!(g.stops()[0].at < g.stops()[1].at);
        assert_eq!(g.evaluate(0.0), spectrum(1.0, 1.0, 1.0));
    }

    /// Monotone between two stops, in every channel. A ramp that overshot —
    /// as a spline would — would put colours in a material that are not in its
    /// own gradient, which is the one thing a colour ramp must not do.
    #[test]
    fn it_never_leaves_the_colours_it_was_given() {
        let g = Gradient::new(vec![
            Stop {
                at: 0.0,
                color: spectrum(0.9, 0.1, 0.3),
            },
            Stop {
                at: 0.3,
                color: spectrum(0.1, 0.8, 0.2),
            },
            Stop {
                at: 1.0,
                color: spectrum(0.5, 0.2, 0.95),
            },
        ]);
        let (mut low, mut high) = (f32::MAX, f32::MIN);
        for i in 0..=1000 {
            let c = g.evaluate(i as f32 / 1000.0);
            for channel in [c.r, c.g, c.b] {
                low = low.min(channel);
                high = high.max(channel);
            }
        }
        assert!(low >= 0.1 - 1e-5, "a channel dipped to {low}");
        assert!(high <= 0.95 + 1e-5, "a channel reached {high}");
    }

    /// Two stops at one position are a hard edge, which is a thing to want:
    /// the line between one year's latewood and the next year's earlywood is
    /// abrupt, and smoothing it is what makes procedural wood read as plastic.
    #[test]
    fn coincident_stops_make_a_hard_edge() {
        let g = Gradient::new(vec![
            Stop {
                at: 0.0,
                color: spectrum(0.0, 0.0, 0.0),
            },
            Stop {
                at: 0.5,
                color: spectrum(1.0, 1.0, 1.0),
            },
            Stop {
                at: 0.5,
                color: spectrum(0.0, 0.0, 0.0),
            },
            Stop {
                at: 1.0,
                color: spectrum(1.0, 1.0, 1.0),
            },
        ]);
        let before = g.evaluate(0.499);
        let after = g.evaluate(0.501);
        assert!(
            (before.r - after.r).abs() > 0.9,
            "the edge softened to {} against {}",
            before.r,
            after.r
        );
    }

    /// A ramp cannot be clicked down to one colour. One stop is a constant and
    /// none has nothing to answer with; neither should be reachable by pressing
    /// remove once too often.
    #[test]
    fn a_ramp_keeps_at_least_two_stops() {
        let mut g = Gradient::two(Spectrum::ZERO, Spectrum::ONE);
        assert!(!g.remove(0));
        assert_eq!(g.stops().len(), 2);

        g.insert(Stop {
            at: 0.5,
            color: spectrum(0.5, 0.0, 0.0),
        });
        assert_eq!(g.stops().len(), 3);
        assert!(g.remove(1));
        assert_eq!(g.stops().len(), 2);
        assert!(!g.remove(0));
    }

    /// Every preset is a usable ramp, not a placeholder.
    #[test]
    fn the_presets_are_real_ramps() {
        for (name, gradient) in Gradient::presets() {
            assert!(gradient.stops().len() >= 2, "{name} has too few stops");
            let first = gradient.evaluate(0.0);
            let last = gradient.evaluate(1.0);
            assert!(
                (first.luminance() - last.luminance()).abs() > 0.02,
                "{name} runs from {} to {}, which is no ramp at all",
                first.luminance(),
                last.luminance()
            );
            for i in 0..=100 {
                let c = gradient.evaluate(i as f32 / 100.0);
                for channel in [c.r, c.g, c.b] {
                    assert!(
                        (0.0..=1.0).contains(&channel),
                        "{name} produced {channel}"
                    );
                }
            }
        }
    }
}
