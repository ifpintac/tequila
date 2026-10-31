//! The units the editor reads in: millimetres, centimetres or inches on the
//! part, from how large the part is on its layout, or the layout's own units
//! (0 to 1 across its square) where the layout has no one size to read by.

/// A unit to read the layout in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Unit {
    #[default]
    Millimetres,
    Centimetres,
    Inches,
    /// The layout's own: 1 across its square.
    Layout,
}

impl Unit {
    pub const ALL: [Unit; 4] = [Unit::Millimetres, Unit::Centimetres, Unit::Inches, Unit::Layout];

    /// Its short name, after a number.
    pub fn suffix(self) -> &'static str {
        match self {
            Unit::Millimetres => "mm",
            Unit::Centimetres => "cm",
            Unit::Inches => "in",
            Unit::Layout => "UV",
        }
    }

    /// Its name in the editor's settings file.
    pub fn key(self) -> &'static str {
        match self {
            Unit::Millimetres => "mm",
            Unit::Centimetres => "cm",
            Unit::Inches => "in",
            Unit::Layout => "uv",
        }
    }

    pub fn from_key(key: &str) -> Option<Unit> {
        Unit::ALL.into_iter().find(|u| u.key() == key)
    }

    /// How many of it make a metre on the part; none for the layout's own.
    fn per_metre(self) -> Option<f32> {
        match self {
            Unit::Millimetres => Some(1000.0),
            Unit::Centimetres => Some(100.0),
            Unit::Inches => Some(1.0 / 0.0254),
            Unit::Layout => None,
        }
    }

    /// What a typed length in a field of this unit is in, so `2in` goes
    /// into a field in millimetres.
    pub fn numeric(self) -> Option<crate::numeric::Unit> {
        match self {
            Unit::Millimetres => Some(crate::numeric::Unit::Millimetres),
            Unit::Centimetres => Some(crate::numeric::Unit::Centimetres),
            Unit::Inches => Some(crate::numeric::Unit::Inches),
            Unit::Layout => None,
        }
    }
}

/// A unit and how many of it are in one unit of the layout: what every
/// number the editor shows is read through.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scale {
    pub unit: Unit,
    pub per_uv: f32,
}

impl Scale {
    /// The unit asked for, on a layout of `density` metres of the part per
    /// layout unit; the layout's own units where it has no one density.
    pub fn of(unit: Unit, density: Option<f32>) -> Scale {
        match (unit.per_metre(), density.filter(|d| d.is_finite() && *d > 0.0)) {
            (Some(per_metre), Some(density)) => Scale { unit, per_uv: per_metre * density },
            _ => Scale { unit: Unit::Layout, per_uv: 1.0 },
        }
    }

    pub fn shown(self, uv: f32) -> f32 {
        uv * self.per_uv
    }

    pub fn layout(self, shown: f32) -> f32 {
        shown / self.per_uv
    }

    /// How many decimals a length in it is shown to: tenths of a
    /// millimetre, hundredths of a centimetre or an inch, thousandths of
    /// the layout.
    pub fn decimals(self) -> usize {
        match self.unit {
            Unit::Millimetres => 1,
            Unit::Centimetres | Unit::Inches => 2,
            Unit::Layout => 3,
        }
    }

    /// A length, with its unit.
    pub fn format(self, uv: f32) -> String {
        format!("{:.*} {}", self.decimals(), self.shown(uv), self.unit.suffix())
    }

    /// The step a nudge moves by, in layout units: a millimetre, a tenth of
    /// a centimetre, a sixty-fourth of an inch, a thousandth of the layout.
    pub fn nudge(self) -> f32 {
        let shown = match self.unit {
            Unit::Millimetres => 1.0,
            Unit::Centimetres => 0.1,
            Unit::Inches => 1.0 / 64.0,
            Unit::Layout => 0.001,
        };
        self.layout(shown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_read_in_the_part_s_units_where_the_layout_allows() {
        // A part 0.2 m across its layout's square.
        let mm = Scale::of(Unit::Millimetres, Some(0.2));
        assert!((mm.shown(0.5) - 100.0).abs() < 1e-3);
        assert!((mm.layout(50.0) - 0.25).abs() < 1e-6);
        assert_eq!(mm.format(0.5), "100.0 mm");
        let inches = Scale::of(Unit::Inches, Some(0.254));
        assert!((inches.shown(1.0) - 10.0).abs() < 1e-3);
        // No one size: the layout's own units, whatever was asked for.
        let fallback = Scale::of(Unit::Centimetres, None);
        assert_eq!(fallback, Scale { unit: Unit::Layout, per_uv: 1.0 });
        assert_eq!(fallback.format(0.25), "0.250 UV");
        for unit in Unit::ALL {
            assert_eq!(Unit::from_key(unit.key()), Some(unit));
        }
    }
}
