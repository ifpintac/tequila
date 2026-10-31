//! Guides on a part's UV layout: straight lines across it, each at one place
//! along `u` or along `v`, that the UV editor's moves, handles and measure
//! snap to.
//!
//! Kept on the part (`SceneNode::uv_guides`) rather than in the editor, so a
//! guide laid down to line labels up is still there when the project is
//! opened again, and goes with the part when it is deleted and undone. They
//! are positions on the layout, in its own units (0 to 1 across the unit
//! square), so they mean the same whichever unit the rulers show.

/// Which of the layout's axes a guide marks a place along.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GuideAxis {
    /// A place along `u`: a line running down the layout.
    U,
    /// A place along `v`: a line running across it.
    V,
}

impl GuideAxis {
    /// Its name in the project file and the journal.
    pub fn key(self) -> &'static str {
        match self {
            GuideAxis::U => "u",
            GuideAxis::V => "v",
        }
    }

    pub fn from_key(key: &str) -> Option<GuideAxis> {
        match key {
            "u" => Some(GuideAxis::U),
            "v" => Some(GuideAxis::V),
            _ => None,
        }
    }
}

/// One guide.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvGuide {
    pub axis: GuideAxis,
    /// Where along its axis, in layout units.
    pub at: f32,
}

/// How far from the unit square a guide may be: well past anything a layout
/// holds, and short of where a single-precision position stops being one.
pub const GUIDE_REACH: f32 = 1000.0;

impl UvGuide {
    pub fn new(axis: GuideAxis, at: f32) -> UvGuide {
        UvGuide { axis, at }
    }

    pub fn is_valid(&self) -> bool {
        self.at.is_finite() && self.at.abs() <= GUIDE_REACH
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_axis_reads_back_from_its_key() {
        for axis in [GuideAxis::U, GuideAxis::V] {
            assert_eq!(GuideAxis::from_key(axis.key()), Some(axis));
        }
        assert_eq!(GuideAxis::from_key("w"), None);
        assert!(UvGuide::new(GuideAxis::U, 0.5).is_valid());
        assert!(!UvGuide::new(GuideAxis::V, f32::NAN).is_valid());
        assert!(!UvGuide::new(GuideAxis::V, 2.0 * GUIDE_REACH).is_valid());
    }
}
