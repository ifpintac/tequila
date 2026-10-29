//! A product turning over frames: a turntable, or a keyed spin.
//!
//! The product's turn ([`ProductTurn`](crate::studio::ProductTurn)) keyed
//! at frames and interpolated between them. A Studio's view is already a
//! turn of the product; an animation is the same turn over time, so a
//! frame is rendered as a view is - the product turned
//! ([`crate::studio::turn_product`]), nothing else moved.

/// How a turn goes from one key to the next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Interpolation {
    /// At an even rate: a turntable.
    #[default]
    Linear,
    /// Starting and stopping gently.
    Ease,
}

impl Interpolation {
    pub fn key(&self) -> &'static str {
        match self {
            Interpolation::Linear => "linear",
            Interpolation::Ease => "ease",
        }
    }

    pub fn from_key(key: &str) -> Option<Interpolation> {
        match key {
            "linear" => Some(Interpolation::Linear),
            "ease" => Some(Interpolation::Ease),
            _ => None,
        }
    }
}

/// The product's turn at one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TurnKey {
    pub frame: u32,
    /// Radians, as [`ProductTurn::yaw`](crate::studio::ProductTurn::yaw).
    pub product_yaw: f32,
}

/// Frames of the product turning.
#[derive(Clone, Debug, PartialEq)]
pub struct Animation {
    /// How many frames, numbered from 0.
    pub frames: u32,
    /// Frames a second, for whoever puts the frames together.
    pub fps: f32,
    /// The turn at some frames. Between two it is interpolated; before the
    /// first and after the last it holds.
    pub keys: Vec<TurnKey>,
    pub interpolation: Interpolation,
}

impl Animation {
    /// A turntable: `frames` frames turning the product through `degrees`,
    /// the frame after the last where the first began - so 36 frames of
    /// 360 degrees are ten apart, and none is the same as another.
    pub fn turntable(frames: u32, fps: f32, degrees: f32) -> Animation {
        let frames = frames.max(1);
        Animation {
            frames,
            fps,
            keys: vec![
                TurnKey { frame: 0, product_yaw: 0.0 },
                TurnKey { frame: frames, product_yaw: degrees.to_radians() },
            ],
            interpolation: Interpolation::Linear,
        }
    }

    /// The product's turn at `frame`. `None` with no keys.
    pub fn yaw_at(&self, frame: u32) -> Option<f32> {
        let mut keys = self.keys.clone();
        keys.sort_by_key(|k| k.frame);
        let first = *keys.first()?;
        if frame <= first.frame {
            return Some(first.product_yaw);
        }
        for pair in keys.windows(2) {
            let (from, to) = (pair[0], pair[1]);
            if frame <= to.frame {
                let span = to.frame.saturating_sub(from.frame).max(1) as f32;
                let mut t = (frame - from.frame) as f32 / span;
                if self.interpolation == Interpolation::Ease {
                    t = t * t * (3.0 - 2.0 * t);
                }
                return Some(from.product_yaw + (to.product_yaw - from.product_yaw) * t);
            }
        }
        keys.last().map(|k| k.product_yaw)
    }

    /// The frames `first` to `last`, as far as there are any.
    pub fn range(&self, first: u32, last: u32) -> std::ops::RangeInclusive<u32> {
        let end = self.frames.saturating_sub(1);
        first.min(end)..=last.min(end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Frame N of a turntable all the way round in 36 frames has the
    /// product turned N times ten degrees, and the last is a step short of
    /// the first.
    #[test]
    fn a_turntable_turns_ten_degrees_a_frame() {
        let turntable = Animation::turntable(36, 24.0, 360.0);
        for frame in 0..36 {
            let yaw = turntable.yaw_at(frame).unwrap().to_degrees();
            assert!((yaw - frame as f32 * 10.0).abs() < 1e-3, "frame {frame} at {yaw} degrees");
        }
        assert_eq!(turntable.range(0, 100), 0..=35);
    }

    /// Showing a frame turns the product by its angle, and nothing else;
    /// one undo turns it back.
    #[test]
    fn a_frame_turns_the_product_by_its_angle() {
        let mut document = crate::demo::demo_document(crate::demo::DemoMaterial::Plastic, 0.2);
        let turntable = Animation::turntable(36, 24.0, 360.0);
        let mut bus = crate::CommandBus::new();
        bus.apply(&mut document, crate::Command::TurnProduct { yaw: turntable.yaw_at(9).unwrap() }).unwrap();
        assert!((document.product_turn.yaw.to_degrees() - 90.0).abs() < 1e-3, "{}", document.product_turn.yaw.to_degrees());
        bus.undo(&mut document).unwrap();
        assert_eq!(document.product_turn, crate::studio::ProductTurn::default());
    }

    /// Eased, a turn starts and stops gently and passes the middle at the
    /// middle; keys hold before the first and after the last.
    #[test]
    fn an_eased_turn_starts_and_stops_gently() {
        let spin = Animation {
            frames: 30,
            fps: 30.0,
            keys: vec![TurnKey { frame: 20, product_yaw: 1.0 }, TurnKey { frame: 10, product_yaw: 0.0 }],
            interpolation: Interpolation::Ease,
        };
        assert_eq!(spin.yaw_at(0), Some(0.0));
        assert_eq!(spin.yaw_at(29), Some(1.0));
        assert!((spin.yaw_at(15).unwrap() - 0.5).abs() < 1e-6);
        assert!(spin.yaw_at(11).unwrap() < 0.1 / 3.0, "an eased turn leapt off its key");
        assert_eq!(Animation { keys: Vec::new(), ..spin }.yaw_at(3), None);
    }
}
