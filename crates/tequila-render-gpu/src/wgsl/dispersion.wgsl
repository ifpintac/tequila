// Dispersion: tequila_materials::dispersion - glass with an Abbe number
// bends each wavelength by its own index (the two-term Cauchy law fitted to
// the d, F and C lines), and an RGB path meeting it is given one
// wavelength, drawn evenly over 380-730 nm, carried from there on as the
// colour it registers as (the CIE functions by Wyman, Sloan and Shirley's
// fit, into linear sRGB, clipped, scaled so the wavelengths average to
// white, into the rendering space).
//#include nodes

const DISPERSION_SHORTEST: f32 = 380.0;
const DISPERSION_LONGEST: f32 = 730.0;
const DISPERSION_D_LINE: f32 = 587.6;
const DISPERSION_F_LINE: f32 = 486.1;
const DISPERSION_C_LINE: f32 = 656.3;
// dispersion.rs's normalization(): each channel's scale, so the
// wavelengths average to white (tests/dispersion.rs works it out as the
// processor does and holds this line to it).
const DISPERSION_SCALE: vec3<f32> = vec3<f32>(1.9865750074386597, 3.0333974361419678, 3.202308177947998);

// ior_at: the index at `wavelength` nm of glass whose index is `ior` at the
// d line and whose Abbe number is `abbe`; no dispersion at an Abbe number
// of 0.
fn dispersion_ior_at(ior: f32, abbe: f32, wavelength: f32) -> f32 {
    if (abbe <= 0.0) {
        return ior;
    }
    let spread = cy_div(1.0, cy_exact(DISPERSION_F_LINE * DISPERSION_F_LINE)) - cy_div(1.0, cy_exact(DISPERSION_C_LINE * DISPERSION_C_LINE));
    let b = cy_div(ior - 1.0, cy_exact(abbe * spread));
    let a = ior - cy_div(b, cy_exact(DISPERSION_D_LINE * DISPERSION_D_LINE));
    return max(a + cy_div(b, cy_exact(wavelength * wavelength)), 1.0);
}

// cie_xyz's lobe: a Gaussian of widths `lo` below `mu` and `hi` above.
fn dispersion_lobe(x: f32, mu: f32, lo: f32, hi: f32) -> f32 {
    let t = cy_div(x - mu, select(hi, lo, x < mu));
    return exp(cy_exact(cy_exact(-0.5 * t) * t));
}

// cie_xyz: the CIE 1931 functions at `l` nm.
fn dispersion_cie_xyz(l: f32) -> vec3<f32> {
    let x = cy_exact(1.056 * dispersion_lobe(l, 599.8, 37.9, 31.0)) + cy_exact(0.362 * dispersion_lobe(l, 442.0, 16.0, 26.7)) - cy_exact(0.065 * dispersion_lobe(l, 501.1, 20.4, 26.2));
    let y = cy_exact(0.821 * dispersion_lobe(l, 568.8, 46.9, 40.5)) + cy_exact(0.286 * dispersion_lobe(l, 530.9, 16.3, 31.1));
    let z = cy_exact(1.217 * dispersion_lobe(l, 437.0, 11.8, 36.0)) + cy_exact(0.681 * dispersion_lobe(l, 459.0, 26.0, 13.8));
    return vec3<f32>(x, y, z);
}

// weight: the colour a path carrying `wavelength` stands for, a throughput
// weight that averages to white over the range.
fn dispersion_weight(wavelength: f32) -> vec3<f32> {
    let c = dispersion_cie_xyz(wavelength);
    let raw = max(vec3<f32>(
        cy_exact(3.2406 * c.x) - cy_exact(1.5372 * c.y) - cy_exact(0.4986 * c.z),
        cy_exact(-0.9689 * c.x) + cy_exact(1.8758 * c.y) + cy_exact(0.0415 * c.z),
        cy_exact(0.0557 * c.x) - cy_exact(0.2040 * c.y) + cy_exact(1.0570 * c.z),
    ), vec3<f32>(0.0));
    let v = cy_exact3(raw * DISPERSION_SCALE);
    // Spectrum::from_linear_srgb.
    return vec3<f32>(
        cy_exact(0.6130974 * v.x) + cy_exact(0.3395234 * v.y) + cy_exact(0.0473792 * v.z),
        cy_exact(0.0701936 * v.x) + cy_exact(0.9163535 * v.y) + cy_exact(0.0134529 * v.z),
        cy_exact(0.0206158 * v.x) + cy_exact(0.1095696 * v.y) + cy_exact(0.8698146 * v.z),
    );
}

// sample: a wavelength evenly over the range from `u` in [0, 1).
fn dispersion_sample(u: f32) -> f32 {
    return DISPERSION_SHORTEST + cy_exact((DISPERSION_LONGEST - DISPERSION_SHORTEST) * clamp(u, 0.0, 1.0));
}
