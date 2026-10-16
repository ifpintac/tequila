//! Cycles' converter and colour nodes: Math, Vector Math, Mix (every blend
//! mode), Map Range, Clamp, Mapping, Hue/Saturation/Value, Bright/Contrast,
//! Gamma, Invert, and Separate/Combine Color (`kernel/svm/math_util.h`,
//! `color_util.h`, `map_range.h`, `mapping_util.h`, `hsv.h`, `util/color.h`).

pub type V3 = [f32; 3];

#[inline]
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
#[inline]
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
#[inline]
fn mul(a: V3, b: V3) -> V3 {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}
#[inline]
fn scale(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
#[inline]
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
#[inline]
fn len(a: V3) -> f32 {
    dot(a, a).sqrt()
}
#[inline]
fn interp(a: V3, b: V3, t: f32) -> V3 {
    add(a, scale(sub(b, a), t))
}
#[inline]
fn map3(a: V3, f: impl Fn(f32) -> f32) -> V3 {
    [f(a[0]), f(a[1]), f(a[2])]
}
#[inline]
fn zip3(a: V3, b: V3, f: impl Fn(f32, f32) -> f32) -> V3 {
    [f(a[0], b[0]), f(a[1], b[1]), f(a[2], b[2])]
}

pub fn safe_divide(a: f32, b: f32) -> f32 {
    if b != 0.0 { a / b } else { 0.0 }
}

fn compatible_powf(x: f32, y: f32) -> f32 {
    if y == 0.0 {
        1.0
    } else if x == 0.0 {
        0.0
    } else {
        x.powf(y)
    }
}

fn safe_powf(a: f32, b: f32) -> f32 {
    if a < 0.0 && b != (b as i32) as f32 { 0.0 } else { compatible_powf(a, b) }
}

fn safe_logf(a: f32, b: f32) -> f32 {
    if a <= 0.0 || b <= 0.0 { 0.0 } else { safe_divide(a.ln(), b.ln()) }
}

fn compatible_signf(f: f32) -> f32 {
    if f == 0.0 {
        0.0
    } else if f < 0.0 {
        -1.0
    } else {
        1.0
    }
}

fn fractf(x: f32) -> f32 {
    x - x.floor()
}

fn wrapf(value: f32, max: f32, min: f32) -> f32 {
    let range = max - min;
    if range != 0.0 { value - range * ((value - min) / range).floor() } else { min }
}

fn pingpongf(a: f32, b: f32) -> f32 {
    if b != 0.0 { (fractf((a - b) / (b * 2.0)) * b * 2.0 - b).abs() } else { 0.0 }
}

fn smoothminf(a: f32, b: f32, k: f32) -> f32 {
    if k != 0.0 {
        let h = (k - (a - b).abs()).max(0.0) / k;
        a.min(b) - h * h * h * k * (1.0 / 6.0)
    } else {
        a.min(b)
    }
}

fn safe_floored_fmod(a: f32, b: f32) -> f32 {
    if b != 0.0 { a - (a / b).floor() * b } else { 0.0 }
}

pub fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    if x < edge0 {
        0.0
    } else if x >= edge1 {
        1.0
    } else {
        let t = (x - edge0) / (edge1 - edge0);
        (3.0 - 2.0 * t) * (t * t)
    }
}

fn smootherstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let x = safe_divide(x - edge0, edge1 - edge0).clamp(0.0, 1.0);
    x * x * x * (x * (x * 6.0 - 15.0) + 10.0)
}

// ---- Math --------------------------------------------------------------

macro_rules! ops {
    ($name:ident { $($variant:ident => $cycles:literal, $label:literal, $inputs:literal;)* }) => {
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
        pub enum $name {
            #[default]
            $($variant,)*
        }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant,)*];

            /// Cycles' name for it (the node's `math_type`).
            pub fn cycles(self) -> &'static str {
                match self { $($name::$variant => $cycles,)* }
            }

            pub fn label(self) -> &'static str {
                match self { $($name::$variant => $label,)* }
            }

            /// How many of the node's inputs it reads.
            pub fn inputs(self) -> u8 {
                match self { $($name::$variant => $inputs,)* }
            }

            pub fn from_cycles(key: &str) -> Option<$name> {
                Self::ALL.iter().copied().find(|op| op.cycles() == key)
            }
        }
    };
}

ops!(MathOp {
    Add => "add", "Add", 2;
    Subtract => "subtract", "Subtract", 2;
    Multiply => "multiply", "Multiply", 2;
    Divide => "divide", "Divide", 2;
    MultiplyAdd => "multiply_add", "Multiply add", 3;
    Power => "power", "Power", 2;
    Logarithm => "logarithm", "Logarithm", 2;
    Sqrt => "sqrt", "Square root", 1;
    InvSqrt => "inversesqrt", "Inverse square root", 1;
    Absolute => "absolute", "Absolute", 1;
    Exponent => "exponent", "Exponent", 1;
    Minimum => "minimum", "Minimum", 2;
    Maximum => "maximum", "Maximum", 2;
    LessThan => "less_than", "Less than", 2;
    GreaterThan => "greater_than", "Greater than", 2;
    Sign => "sign", "Sign", 1;
    Compare => "compare", "Compare", 3;
    SmoothMin => "smoothmin", "Smooth minimum", 3;
    SmoothMax => "smoothmax", "Smooth maximum", 3;
    Round => "round", "Round", 1;
    Floor => "floor", "Floor", 1;
    Ceil => "ceil", "Ceiling", 1;
    Trunc => "trunc", "Truncate", 1;
    Fraction => "fraction", "Fraction", 1;
    Modulo => "modulo", "Truncated modulo", 2;
    FlooredModulo => "floored_modulo", "Floored modulo", 2;
    Wrap => "wrap", "Wrap", 3;
    Snap => "snap", "Snap", 2;
    PingPong => "pingpong", "Ping-pong", 2;
    Sine => "sine", "Sine", 1;
    Cosine => "cosine", "Cosine", 1;
    Tangent => "tangent", "Tangent", 1;
    Arcsine => "arcsine", "Arcsine", 1;
    Arccosine => "arccosine", "Arccosine", 1;
    Arctangent => "arctangent", "Arctangent", 1;
    Arctan2 => "arctan2", "Arctan2", 2;
    Sinh => "sinh", "Hyperbolic sine", 1;
    Cosh => "cosh", "Hyperbolic cosine", 1;
    Tanh => "tanh", "Hyperbolic tangent", 1;
    Radians => "radians", "To radians", 1;
    Degrees => "degrees", "To degrees", 1;
});

impl MathOp {
    pub fn apply(self, a: f32, b: f32, c: f32) -> f32 {
        use std::f32::consts::PI;
        match self {
            MathOp::Add => a + b,
            MathOp::Subtract => a - b,
            MathOp::Multiply => a * b,
            MathOp::Divide => safe_divide(a, b),
            MathOp::Power => safe_powf(a, b),
            MathOp::Logarithm => safe_logf(a, b),
            MathOp::Sqrt => a.max(0.0).sqrt(),
            MathOp::InvSqrt => {
                if a > 0.0 {
                    1.0 / a.sqrt()
                } else {
                    0.0
                }
            }
            MathOp::Absolute => a.abs(),
            MathOp::Radians => a * (PI / 180.0),
            MathOp::Degrees => a * (180.0 / PI),
            MathOp::Minimum => a.min(b),
            MathOp::Maximum => a.max(b),
            MathOp::LessThan => (a < b) as i32 as f32,
            MathOp::GreaterThan => (a > b) as i32 as f32,
            MathOp::Round => (a + 0.5).floor(),
            MathOp::Floor => a.floor(),
            MathOp::Ceil => a.ceil(),
            MathOp::Fraction => a - a.floor(),
            MathOp::Modulo => {
                if b != 0.0 {
                    a % b
                } else {
                    0.0
                }
            }
            MathOp::FlooredModulo => safe_floored_fmod(a, b),
            MathOp::Trunc => {
                if a >= 0.0 {
                    a.floor()
                } else {
                    a.ceil()
                }
            }
            MathOp::Snap => safe_divide(a, b).floor() * b,
            MathOp::Wrap => wrapf(a, b, c),
            MathOp::PingPong => pingpongf(a, b),
            MathOp::Sine => a.sin(),
            MathOp::Cosine => a.cos(),
            MathOp::Tangent => a.tan(),
            MathOp::Sinh => a.sinh(),
            MathOp::Cosh => a.cosh(),
            MathOp::Tanh => a.tanh(),
            MathOp::Arcsine => a.clamp(-1.0, 1.0).asin(),
            MathOp::Arccosine => a.clamp(-1.0, 1.0).acos(),
            MathOp::Arctangent => a.atan(),
            MathOp::Arctan2 => {
                if a == 0.0 && b == 0.0 {
                    0.0
                } else {
                    a.atan2(b)
                }
            }
            MathOp::Sign => compatible_signf(a),
            MathOp::Exponent => a.exp(),
            MathOp::Compare => {
                if a == b || (a - b).abs() <= c.max(f32::EPSILON) {
                    1.0
                } else {
                    0.0
                }
            }
            MathOp::MultiplyAdd => a * b + c,
            MathOp::SmoothMin => smoothminf(a, b, c),
            MathOp::SmoothMax => -smoothminf(-a, -b, c),
        }
    }
}

ops!(VectorMathOp {
    Add => "add", "Add", 2;
    Subtract => "subtract", "Subtract", 2;
    Multiply => "multiply", "Multiply", 2;
    Divide => "divide", "Divide", 2;
    MultiplyAdd => "multiply_add", "Multiply add", 3;
    CrossProduct => "cross_product", "Cross product", 2;
    Project => "project", "Project", 2;
    Reflect => "reflect", "Reflect", 2;
    Refract => "refract", "Refract", 2;
    Faceforward => "faceforward", "Faceforward", 3;
    DotProduct => "dot_product", "Dot product", 2;
    Distance => "distance", "Distance", 2;
    Length => "length", "Length", 1;
    Scale => "scale", "Scale", 1;
    Normalize => "normalize", "Normalize", 1;
    Absolute => "absolute", "Absolute", 1;
    Power => "power", "Power", 2;
    Sign => "sign", "Sign", 1;
    Minimum => "minimum", "Minimum", 2;
    Maximum => "maximum", "Maximum", 2;
    Floor => "floor", "Floor", 1;
    Ceil => "ceil", "Ceiling", 1;
    Fraction => "fraction", "Fraction", 1;
    Modulo => "modulo", "Modulo", 2;
    Wrap => "wrap", "Wrap", 3;
    Snap => "snap", "Snap", 2;
    Sine => "sine", "Sine", 1;
    Cosine => "cosine", "Cosine", 1;
    Tangent => "tangent", "Tangent", 1;
});

impl VectorMathOp {
    /// Whether the result is the Value output rather than the Vector.
    pub fn gives_value(self) -> bool {
        matches!(self, VectorMathOp::DotProduct | VectorMathOp::Distance | VectorMathOp::Length)
    }

    /// The Vector and Value outputs; `param` is the Scale (or IOR) input.
    pub fn apply(self, a: V3, b: V3, c: V3, param: f32) -> (V3, f32) {
        let safe_normalize = |v: V3| {
            let t = len(v);
            if t != 0.0 { scale(v, 1.0 / t) } else { v }
        };
        let zero = [0.0; 3];
        match self {
            VectorMathOp::Add => (add(a, b), 0.0),
            VectorMathOp::Subtract => (sub(a, b), 0.0),
            VectorMathOp::Multiply => (mul(a, b), 0.0),
            VectorMathOp::Divide => (zip3(a, b, safe_divide), 0.0),
            VectorMathOp::CrossProduct => ([a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]], 0.0),
            VectorMathOp::Project => {
                let l2 = dot(b, b);
                (if l2 != 0.0 { scale(b, dot(a, b) / l2) } else { zero }, 0.0)
            }
            VectorMathOp::Reflect => {
                let n = safe_normalize(b);
                (sub(a, scale(n, 2.0 * dot(a, n))), 0.0)
            }
            VectorMathOp::Refract => {
                let n = safe_normalize(b);
                let eta = param;
                let k = 1.0 - eta * eta * (1.0 - dot(n, a) * dot(n, a));
                (if k < 0.0 { zero } else { sub(scale(a, eta), scale(n, eta * dot(n, a) + k.sqrt())) }, 0.0)
            }
            VectorMathOp::Faceforward => (if dot(c, b) < 0.0 { a } else { scale(a, -1.0) }, 0.0),
            VectorMathOp::MultiplyAdd => (add(mul(a, b), c), 0.0),
            VectorMathOp::DotProduct => (zero, dot(a, b)),
            VectorMathOp::Distance => (zero, len(sub(a, b))),
            VectorMathOp::Length => (zero, len(a)),
            VectorMathOp::Scale => (scale(a, param), 0.0),
            VectorMathOp::Normalize => (safe_normalize(a), 0.0),
            VectorMathOp::Snap => (zip3(a, b, |x, y| safe_divide(x, y).floor() * y), 0.0),
            VectorMathOp::Floor => (map3(a, f32::floor), 0.0),
            VectorMathOp::Ceil => (map3(a, f32::ceil), 0.0),
            VectorMathOp::Modulo => (zip3(a, b, |x, y| if y == 0.0 { 0.0 } else { x % y }), 0.0),
            VectorMathOp::Wrap => (
                [
                    safe_floored_fmod(a[0] - c[0], b[0] - c[0]) + c[0],
                    safe_floored_fmod(a[1] - c[1], b[1] - c[1]) + c[1],
                    safe_floored_fmod(a[2] - c[2], b[2] - c[2]) + c[2],
                ],
                0.0,
            ),
            VectorMathOp::Fraction => (map3(a, |x| x - x.floor()), 0.0),
            VectorMathOp::Absolute => (map3(a, f32::abs), 0.0),
            VectorMathOp::Power => (zip3(a, b, safe_powf), 0.0),
            VectorMathOp::Sign => (map3(a, compatible_signf), 0.0),
            VectorMathOp::Minimum => (zip3(a, b, f32::min), 0.0),
            VectorMathOp::Maximum => (zip3(a, b, f32::max), 0.0),
            VectorMathOp::Sine => (map3(a, f32::sin), 0.0),
            VectorMathOp::Cosine => (map3(a, f32::cos), 0.0),
            VectorMathOp::Tangent => (map3(a, f32::tan), 0.0),
        }
    }
}

// ---- Mix ---------------------------------------------------------------

ops!(BlendMode {
    Mix => "mix", "Mix", 2;
    Darken => "darken", "Darken", 2;
    Multiply => "multiply", "Multiply", 2;
    Burn => "burn", "Color burn", 2;
    Lighten => "lighten", "Lighten", 2;
    Screen => "screen", "Screen", 2;
    Dodge => "dodge", "Color dodge", 2;
    Add => "add", "Add", 2;
    Overlay => "overlay", "Overlay", 2;
    SoftLight => "soft_light", "Soft light", 2;
    LinearLight => "linear_light", "Linear light", 2;
    Difference => "difference", "Difference", 2;
    Exclusion => "exclusion", "Exclusion", 2;
    Subtract => "subtract", "Subtract", 2;
    Divide => "divide", "Divide", 2;
    Hue => "hue", "Hue", 2;
    Saturation => "saturation", "Saturation", 2;
    Color => "color", "Color", 2;
    Value => "value", "Value", 2;
});

impl BlendMode {
    /// Cycles' blend of `c2` over `c1` by `t` (`svm_mix`).
    pub fn blend(self, t: f32, c1: V3, c2: V3) -> V3 {
        let tm = 1.0 - t;
        match self {
            BlendMode::Mix => interp(c1, c2, t),
            BlendMode::Add => interp(c1, add(c1, c2), t),
            BlendMode::Multiply => interp(c1, mul(c1, c2), t),
            BlendMode::Screen => {
                let one = [1.0; 3];
                sub(one, mul(add([tm; 3], scale(sub(one, c2), t)), sub(one, c1)))
            }
            BlendMode::Overlay => std::array::from_fn(|i| {
                let o = c1[i];
                if o < 0.5 { o * (tm + 2.0 * t * c2[i]) } else { 1.0 - (tm + 2.0 * t * (1.0 - c2[i])) * (1.0 - o) }
            }),
            BlendMode::Subtract => interp(c1, sub(c1, c2), t),
            BlendMode::Divide => std::array::from_fn(|i| if c2[i] != 0.0 { tm * c1[i] + t * c1[i] / c2[i] } else { c1[i] }),
            BlendMode::Difference => interp(c1, map3(sub(c1, c2), f32::abs), t),
            BlendMode::Exclusion => {
                let target = std::array::from_fn(|i| c1[i] + c2[i] - 2.0 * c1[i] * c2[i]);
                map3(interp(c1, target, t), |v| v.max(0.0))
            }
            BlendMode::Darken => interp(c1, zip3(c1, c2, f32::min), t),
            BlendMode::Lighten => interp(c1, zip3(c1, c2, f32::max), t),
            BlendMode::Dodge => std::array::from_fn(|i| {
                let o = c1[i];
                if o == 0.0 {
                    return o;
                }
                let tmp = 1.0 - t * c2[i];
                if tmp <= 0.0 {
                    1.0
                } else {
                    (o / tmp).min(1.0)
                }
            }),
            BlendMode::Burn => std::array::from_fn(|i| {
                let tmp = tm + t * c2[i];
                if tmp <= 0.0 {
                    0.0
                } else {
                    (1.0 - (1.0 - c1[i]) / tmp).clamp(0.0, 1.0)
                }
            }),
            BlendMode::Hue => {
                let hsv2 = rgb_to_hsv(c2);
                if hsv2[1] != 0.0 {
                    let mut hsv = rgb_to_hsv(c1);
                    hsv[0] = hsv2[0];
                    interp(c1, hsv_to_rgb(hsv), t)
                } else {
                    c1
                }
            }
            BlendMode::Saturation => {
                let mut hsv = rgb_to_hsv(c1);
                if hsv[1] != 0.0 {
                    let hsv2 = rgb_to_hsv(c2);
                    hsv[1] = tm * hsv[1] + t * hsv2[1];
                    hsv_to_rgb(hsv)
                } else {
                    c1
                }
            }
            BlendMode::Value => {
                let mut hsv = rgb_to_hsv(c1);
                let hsv2 = rgb_to_hsv(c2);
                hsv[2] = tm * hsv[2] + t * hsv2[2];
                hsv_to_rgb(hsv)
            }
            BlendMode::Color => {
                let hsv2 = rgb_to_hsv(c2);
                if hsv2[1] != 0.0 {
                    let mut hsv = rgb_to_hsv(c1);
                    hsv[0] = hsv2[0];
                    hsv[1] = hsv2[1];
                    interp(c1, hsv_to_rgb(hsv), t)
                } else {
                    c1
                }
            }
            BlendMode::SoftLight => {
                let one = [1.0; 3];
                let scr = sub(one, mul(sub(one, c2), sub(one, c1)));
                add(scale(c1, tm), scale(add(mul(mul(sub(one, c1), c2), c1), mul(c1, scr)), t))
            }
            BlendMode::LinearLight => add(c1, scale(add(scale(c2, 2.0), [-1.0; 3]), t)),
        }
    }
}

// ---- Colour spaces -----------------------------------------------------

pub fn rgb_to_hsv(rgb: V3) -> V3 {
    let cmax = rgb[0].max(rgb[1].max(rgb[2]));
    let cmin = rgb[0].min(rgb[1].min(rgb[2]));
    let cdelta = cmax - cmin;
    let v = cmax;
    let s = if cmax != 0.0 { cdelta / cmax } else { 0.0 };
    let mut h;
    if s != 0.0 {
        let c = map3(sub([cmax; 3], rgb), |x| x / cdelta);
        h = if rgb[0] == cmax {
            c[2] - c[1]
        } else if rgb[1] == cmax {
            2.0 + c[0] - c[2]
        } else {
            4.0 + c[1] - c[0]
        };
        h /= 6.0;
        if h < 0.0 {
            h += 1.0;
        }
    } else {
        h = 0.0;
    }
    [h, s, v]
}

pub fn hsv_to_rgb(hsv: V3) -> V3 {
    let [mut h, s, v] = hsv;
    if s == 0.0 {
        return [v, v, v];
    }
    if h == 1.0 {
        h = 0.0;
    }
    h *= 6.0;
    let i = h.floor();
    let f = h - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    if i == 0.0 {
        [v, t, p]
    } else if i == 1.0 {
        [q, v, p]
    } else if i == 2.0 {
        [p, v, t]
    } else if i == 3.0 {
        [p, q, v]
    } else if i == 4.0 {
        [t, p, v]
    } else {
        [v, p, q]
    }
}

pub fn rgb_to_hsl(rgb: V3) -> V3 {
    let cmax = rgb[0].max(rgb[1].max(rgb[2]));
    let cmin = rgb[0].min(rgb[1].min(rgb[2]));
    let l = 1.0f32.min((cmax + cmin) / 2.0);
    let (mut h, s);
    if cmax == cmin {
        h = 0.0;
        s = 0.0;
    } else {
        let cdelta = cmax - cmin;
        s = if l > 0.5 { cdelta / (2.0 - cmax - cmin) } else { cdelta / (cmax + cmin) };
        h = if cmax == rgb[0] {
            (rgb[1] - rgb[2]) / cdelta + if rgb[1] < rgb[2] { 6.0 } else { 0.0 }
        } else if cmax == rgb[1] {
            (rgb[2] - rgb[0]) / cdelta + 2.0
        } else {
            (rgb[0] - rgb[1]) / cdelta + 4.0
        };
    }
    h /= 6.0;
    [h, s, l]
}

pub fn hsl_to_rgb(hsl: V3) -> V3 {
    let [h, s, l] = hsl;
    let nr = ((h * 6.0 - 3.0).abs() - 1.0).clamp(0.0, 1.0);
    let ng = (2.0 - (h * 6.0 - 2.0).abs()).clamp(0.0, 1.0);
    let nb = (2.0 - (h * 6.0 - 4.0).abs()).clamp(0.0, 1.0);
    let chroma = (1.0 - (2.0 * l - 1.0).abs()) * s;
    [(nr - 0.5) * chroma + l, (ng - 0.5) * chroma + l, (nb - 0.5) * chroma + l]
}

/// How Separate and Combine Color read a colour's three numbers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ColorModel {
    #[default]
    Rgb,
    Hsv,
    Hsl,
}

impl ColorModel {
    pub const ALL: [ColorModel; 3] = [ColorModel::Rgb, ColorModel::Hsv, ColorModel::Hsl];

    pub fn cycles(self) -> &'static str {
        match self {
            ColorModel::Rgb => "rgb",
            ColorModel::Hsv => "hsv",
            ColorModel::Hsl => "hsl",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ColorModel::Rgb => "RGB",
            ColorModel::Hsv => "HSV",
            ColorModel::Hsl => "HSL",
        }
    }

    pub fn channel_names(self) -> [&'static str; 3] {
        match self {
            ColorModel::Rgb => ["Red", "Green", "Blue"],
            ColorModel::Hsv => ["Hue", "Saturation", "Value"],
            ColorModel::Hsl => ["Hue", "Saturation", "Lightness"],
        }
    }

    pub fn separate(self, rgb: V3) -> V3 {
        match self {
            ColorModel::Rgb => rgb,
            ColorModel::Hsv => rgb_to_hsv(rgb),
            ColorModel::Hsl => rgb_to_hsl(rgb),
        }
    }

    pub fn combine(self, c: V3) -> V3 {
        match self {
            ColorModel::Rgb => c,
            ColorModel::Hsv => hsv_to_rgb(c),
            ColorModel::Hsl => hsl_to_rgb(c),
        }
    }
}

/// The Hue/Saturation/Value node.
pub fn hue_saturation_value(color: V3, hue: f32, saturation: f32, value: f32, fac: f32) -> V3 {
    let mut hsv = rgb_to_hsv(color);
    hsv[0] = fractf(hsv[0] + hue + 0.5);
    hsv[1] = (hsv[1] * saturation).clamp(0.0, 1.0);
    hsv[2] *= value;
    let out = hsv_to_rgb(hsv);
    std::array::from_fn(|i| (fac * out[i] + (1.0 - fac) * color[i]).max(0.0))
}

/// The Bright/Contrast node.
pub fn bright_contrast(color: V3, brightness: f32, contrast: f32) -> V3 {
    let a = 1.0 + contrast;
    let b = brightness - contrast * 0.5;
    map3(color, |c| (a * c + b).max(0.0))
}

/// The Gamma node.
pub fn gamma(color: V3, gamma: f32) -> V3 {
    if gamma == 0.0 {
        return [1.0; 3];
    }
    map3(color, |c| if c > 0.0 { c.powf(gamma) } else { c })
}

/// The Invert node.
pub fn invert(color: V3, fac: f32) -> V3 {
    map3(color, |c| fac * (1.0 - c) + (1.0 - fac) * c)
}

// ---- Map Range, Clamp --------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MapRangeKind {
    #[default]
    Linear,
    Stepped,
    Smoothstep,
    Smootherstep,
}

impl MapRangeKind {
    pub const ALL: [MapRangeKind; 4] = [MapRangeKind::Linear, MapRangeKind::Stepped, MapRangeKind::Smoothstep, MapRangeKind::Smootherstep];

    pub fn cycles(self) -> &'static str {
        match self {
            MapRangeKind::Linear => "linear",
            MapRangeKind::Stepped => "stepped",
            MapRangeKind::Smoothstep => "smoothstep",
            MapRangeKind::Smootherstep => "smootherstep",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            MapRangeKind::Linear => "Linear",
            MapRangeKind::Stepped => "Stepped",
            MapRangeKind::Smoothstep => "Smooth step",
            MapRangeKind::Smootherstep => "Smoother step",
        }
    }
}

/// The Map Range node. Clamping, where asked, holds the result between
/// the two "to" ends, as Cycles' added clamp does for the linear and
/// stepped kinds.
#[allow(clippy::too_many_arguments)]
pub fn map_range(kind: MapRangeKind, value: f32, from_min: f32, from_max: f32, to_min: f32, to_max: f32, steps: f32, clamp: bool) -> f32 {
    if from_max == from_min {
        return 0.0;
    }
    let factor = match kind {
        MapRangeKind::Linear => (value - from_min) / (from_max - from_min),
        MapRangeKind::Stepped => {
            let f = (value - from_min) / (from_max - from_min);
            if steps > 0.0 { (f * (steps + 1.0)).floor() / steps } else { 0.0 }
        }
        MapRangeKind::Smoothstep => {
            if from_min > from_max {
                1.0 - smoothstep(from_max, from_min, value)
            } else {
                smoothstep(from_min, from_max, value)
            }
        }
        MapRangeKind::Smootherstep => {
            if from_min > from_max {
                1.0 - smootherstep(from_max, from_min, value)
            } else {
                smootherstep(from_min, from_max, value)
            }
        }
    };
    let result = to_min + factor * (to_max - to_min);
    if clamp && matches!(kind, MapRangeKind::Linear | MapRangeKind::Stepped) {
        result.clamp(to_min.min(to_max), to_min.max(to_max))
    } else {
        result
    }
}

/// The Clamp node: between `min` and `max`, or with Range, between
/// whichever of them is lower and higher.
pub fn clamp(value: f32, min: f32, max: f32, range: bool) -> f32 {
    if range && min > max { value.clamp(max, min) } else { value.max(min).min(max) }
}

// ---- Mapping -----------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MappingKind {
    #[default]
    Point,
    Texture,
    Vector,
    Normal,
}

impl MappingKind {
    pub const ALL: [MappingKind; 4] = [MappingKind::Point, MappingKind::Texture, MappingKind::Vector, MappingKind::Normal];

    pub fn cycles(self) -> &'static str {
        match self {
            MappingKind::Point => "point",
            MappingKind::Texture => "texture",
            MappingKind::Vector => "vector",
            MappingKind::Normal => "normal",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            MappingKind::Point => "Point",
            MappingKind::Texture => "Texture",
            MappingKind::Vector => "Vector",
            MappingKind::Normal => "Normal",
        }
    }
}

/// Rows of the rotation Cycles builds from Euler angles in radians.
fn euler_rows(e: V3) -> [V3; 3] {
    let (cx, cy, cz) = (e[0].cos(), e[1].cos(), e[2].cos());
    let (sx, sy, sz) = (e[0].sin(), e[1].sin(), e[2].sin());
    [
        [cy * cz, sy * sx * cz - cx * sz, sy * cx * cz + sx * sz],
        [cy * sz, sy * sx * sz + cx * cz, sy * cx * sz - sx * cz],
        [-sy, cy * sx, cy * cx],
    ]
}

/// The Mapping node.
pub fn mapping(kind: MappingKind, vector: V3, location: V3, rotation: V3, scale_by: V3) -> V3 {
    let r = euler_rows(rotation);
    let apply = |v: V3| [dot(r[0], v), dot(r[1], v), dot(r[2], v)];
    let apply_t = |v: V3| {
        [r[0][0] * v[0] + r[1][0] * v[1] + r[2][0] * v[2], r[0][1] * v[0] + r[1][1] * v[1] + r[2][1] * v[2], r[0][2] * v[0] + r[1][2] * v[1] + r[2][2] * v[2]]
    };
    match kind {
        MappingKind::Point => add(apply(mul(vector, scale_by)), location),
        MappingKind::Texture => zip3(apply_t(sub(vector, location)), scale_by, safe_divide),
        MappingKind::Vector => apply(mul(vector, scale_by)),
        MappingKind::Normal => {
            let v = apply(zip3(vector, scale_by, safe_divide));
            let t = len(v);
            if t != 0.0 { scale(v, 1.0 / t) } else { v }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_and_hsl_round_trip() {
        for c in [[0.2, 0.5, 0.9], [0.9, 0.1, 0.1], [0.3, 0.3, 0.3], [0.0, 1.0, 0.5]] {
            let h = hsv_to_rgb(rgb_to_hsv(c));
            let l = hsl_to_rgb(rgb_to_hsl(c));
            for i in 0..3 {
                assert!((h[i] - c[i]).abs() < 1e-5, "{c:?} via HSV gave {h:?}");
                assert!((l[i] - c[i]).abs() < 1e-5, "{c:?} via HSL gave {l:?}");
            }
        }
    }

    #[test]
    fn mapping_texture_undoes_point() {
        let (loc, rot, s) = ([0.3, -1.0, 2.0], [0.4, 0.1, -0.7], [2.0, 0.5, 1.5]);
        let v = [0.7, 0.2, -0.4];
        let there = mapping(MappingKind::Point, v, loc, rot, s);
        let back = mapping(MappingKind::Texture, there, loc, rot, s);
        for i in 0..3 {
            assert!((back[i] - v[i]).abs() < 1e-4, "{back:?} is not {v:?}");
        }
    }

    #[test]
    fn math_ops_match_their_names() {
        assert_eq!(MathOp::Add.apply(2.0, 3.0, 0.0), 5.0);
        assert_eq!(MathOp::Divide.apply(1.0, 0.0, 0.0), 0.0, "safe");
        assert_eq!(MathOp::PingPong.apply(2.5, 1.0, 0.0), 0.5);
        assert_eq!(MathOp::from_cycles("floored_modulo"), Some(MathOp::FlooredModulo));
    }
}
