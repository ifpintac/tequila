//! Pictures on the card against `tequila-texture` (`docs/07` R3 tier A,
//! image textures): `textures.wgsl` reading the levels `textures.rs` packs,
//! against the processor's own lookups.
//!
//! - Every value of every encoding decoded (sRGB and plain bytes, sixteen
//!   bits, every finite half float, floats): to the bit.
//! - The level's logarithm against the correctly rounded one.
//! - `Texture::sample_with` through the mesh's UVs - random coordinates,
//!   footprints, wraps (the two axes alike and apart), scales, offsets,
//!   turns and flips - over pyramids of every encoding and channel count,
//!   non-power-of-two sizes, 1 x 1, a row and a column; `sample_at`,
//!   `sample_alpha`, `alpha_at`; and `colour_of`.
//! - `Mapping::project`'s coordinates and the lookups through each
//!   projection (planar, cylindrical, spherical, tri-planar), in real-world
//!   units and not, rigid and not; and `sample_normal_with`, a normal map
//!   laid down by each.
//!
//! The bound: 1e-6 absolute and 1e-5 relative, and at most one item in a
//! thousand outside it. The UV lookups are the processor's to the bit but
//! where the level's logarithm differs in its last bit (the processor's
//! log2f is not correctly rounded about once in a thousand): how many items
//! are the processor's bits is printed. The card's division, arctangent and
//! powf are correctly rounded, as the processor's are all but now and then;
//! spherical v goes through the card's own acos, an ulp or two from the
//! processor's, and its bound is printed with it.

use tequila_core::math::{vec2, vec3, Vec3};
use tequila_core::rng::Pcg32;
use tequila_core::spectrum::Spectrum;
use tequila_render_gpu::harness::Harness;
use tequila_render_gpu::program::{self, TEXTURE_ALPHA, TEXTURE_COLOUR, TEXTURE_NORMAL, TEXTURE_PROJECT};
use tequila_render_gpu::textures::CardPictures;
use tequila_texture::image::{ColorSpace, Image};
use tequila_texture::mip::{MipPyramid, Precision, Wrap};
use tequila_texture::projection::{Axis, Mapping, PointWorld, Projection, SurfacePoint};
use tequila_texture::texture::{MapRole, Texture};

// ---- Pictures made here ---------------------------------------------------

/// Bytes from a fixed sequence, detail at every scale.
fn noise_bytes(count: usize, seed: u32) -> Vec<u8> {
    let mut state = seed | 1;
    (0..count)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state >> 24) as u8
        })
        .collect()
}

/// A half float's value, worked out by hand.
fn half_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = ((h >> 10) & 0x1F) as i32;
    let mantissa = (h & 0x3FF) as f32;
    match exponent {
        0 => sign * mantissa * 2f32.powi(-24),
        31 => {
            if mantissa == 0.0 {
                sign * f32::INFINITY
            } else {
                f32::NAN
            }
        }
        e => sign * (1.0 + mantissa / 1024.0) * 2f32.powi(e - 15),
    }
}

fn floats(width: usize, height: usize, channels: usize, data: Vec<f32>, color_space: ColorSpace) -> Image {
    Image { width, height, channels, data, color_space }
}

/// A black-and-white checkerboard of `square`-texel squares, as bytes.
fn checker(size: usize, square: usize, channels: usize) -> Vec<u8> {
    (0..size * size).flat_map(|i| vec![if ((i % size) / square + (i / size) / square) % 2 == 0 { 255 } else { 0 }; channels]).collect()
}

/// The pictures every lookup test reads: every encoding a level is kept in,
/// one to four channels, sizes odd and even, square and not, down to 1 x 1.
fn test_textures() -> Vec<(&'static str, Texture)> {
    let uv = Mapping::default();
    let mut rng = Pcg32::new(41, 3);
    let mut v = Vec::new();
    v.push(("sRGB bytes, RGB 300 x 260", Texture::from_linear(Image::from_srgb8(300, 260, 3, &noise_bytes(300 * 260 * 3, 1)), MapRole::Color, uv)));
    v.push(("sRGB bytes with alpha, 37 x 23", Texture::from_linear(Image::from_srgb8(37, 23, 4, &noise_bytes(37 * 23 * 4, 2)), MapRole::Color, uv)));
    let grey: Vec<u8> = noise_bytes(512 * 384, 3).iter().flat_map(|&b| [b, b, b]).collect();
    v.push(("bytes, grey 512 x 384", Texture::from_linear(Image::from_raw8(512, 384, 3, &grey), MapRole::Data, uv)));
    let sixteen: Vec<f32> = (0..320 * 256).map(|_| (rng.next_u32() >> 16) as f32 / 65535.0).collect();
    v.push(("sixteen bits, grey 320 x 256", Texture::from_linear(floats(320, 256, 1, sixteen, ColorSpace::Raw), MapRole::Data, uv)));
    let halves: Vec<f32> = noise_bytes(300 * 300 * 3, 4).iter().enumerate().map(|(i, &b)| b as f32 * 0.0137 + (i % 7) as f32 * 0.1).collect();
    v.push(("half floats, RGB 300 x 300", Texture::from_linear(floats(300, 300, 3, halves, ColorSpace::LinearSrgb), MapRole::Color, uv)));
    let lossless: Vec<f32> = (0..64 * 48 * 4).map(|_| rng.next_f32() * 3.0 - 0.5).collect();
    v.push(("floats, RGBA 64 x 48", Texture::from_linear(floats(64, 48, 4, lossless, ColorSpace::LinearSrgb), MapRole::Color, uv)));
    v.push(("one texel", Texture::from_linear(Image::constant(1, 1, vec3(0.3, 0.6, 0.9)), MapRole::Color, uv)));
    let column: Vec<f32> = (0..61 * 3).map(|_| rng.next_f32()).collect();
    v.push(("a column, 1 x 61", Texture::from_linear(floats(1, 61, 3, column, ColorSpace::LinearSrgb), MapRole::Color, uv)));
    let row: Vec<f32> = (0..61).map(|_| rng.next_f32()).collect();
    v.push(("a row, grey 61 x 1", Texture::from_linear(floats(61, 1, 1, row, ColorSpace::Raw), MapRole::Data, uv)));
    let height: Vec<f32> = (0..300 * 260).map(|_| rng.next_f32() * 0.7 + 0.01).collect();
    v.push(("a height, floats 300 x 260", Texture::from_linear(floats(300, 260, 1, height, ColorSpace::Raw), MapRole::Height, uv)));
    let grey_alpha: Vec<u8> = noise_bytes(260 * 257, 5).iter().zip(noise_bytes(260 * 257, 6)).flat_map(|(&g, a)| [g, g, g, a]).collect();
    v.push(("bytes, grey with alpha 260 x 257", Texture::from_linear(Image::from_raw8(260, 257, 4, &grey_alpha), MapRole::Data, uv)));
    v.push(("sRGB checkerboard 256 x 256", Texture::from_linear(Image::from_srgb8(256, 256, 3, &checker(256, 4, 3)), MapRole::Color, uv)));
    v.push(("sRGB bytes, RGB 1024 x 8", Texture::from_linear(Image::from_srgb8(1024, 8, 3, &noise_bytes(1024 * 8 * 3, 7)), MapRole::Color, uv)));
    let rgba_halves: Vec<f32> = (0..280 * 270 * 4).map(|_| rng.next_f32() * 2.0).collect();
    v.push(("half floats, RGBA 280 x 270", Texture::from_linear(floats(280, 270, 4, rgba_halves, ColorSpace::LinearSrgb), MapRole::Color, uv)));
    v
}

/// The pictures packed for the card: each one's colour pyramid, and its
/// alpha's where it has one.
struct OnCard {
    textures: Vec<(&'static str, Texture)>,
    card: CardPictures,
    colour: Vec<u32>,
    alpha: Vec<Option<u32>>,
}

fn on_card() -> OnCard {
    let textures = test_textures();
    let mut card = CardPictures::default();
    let mut colour = Vec::new();
    let mut alpha = Vec::new();
    for (_, t) in &textures {
        colour.push(card.add(t.pyramid()));
        alpha.push(t.alpha_pyramid().map(|p| card.add(p)));
    }
    card.finish();
    OnCard { textures, card, colour, alpha }
}

// ---- Comparing ----------------------------------------------------------

#[derive(Clone, Copy)]
struct Bound {
    absolute: f32,
    relative: f32,
}

const TIGHT: Bound = Bound { absolute: 1e-6, relative: 1e-5 };

fn close(card: f32, processor: f32, bound: Bound) -> bool {
    if card.is_nan() && processor.is_nan() {
        return true;
    }
    if card.is_infinite() || processor.is_infinite() {
        return card == processor;
    }
    (card - processor).abs() <= bound.absolute + bound.relative * processor.abs()
}

/// Every item's words against the processor's: how many are its bits, and
/// how many are outside `bound` (each printed, up to a dozen).
fn tally(name: &str, card: &[u32], want: &[Vec<f32>], labels: &[String], bound: Bound) -> (usize, usize) {
    let words = want.first().map_or(0, |w| w.len());
    let mut exact = 0;
    let mut outside = Vec::new();
    for (i, w) in want.iter().enumerate() {
        let c: Vec<f32> = card[i * words..(i + 1) * words].iter().map(|b| f32::from_bits(*b)).collect();
        if c.iter().zip(w).all(|(a, b)| a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan()) || (*a == 0.0 && *b == 0.0)) {
            exact += 1;
        }
        if let Some(k) = (0..words).find(|&k| !close(c[k], w[k], bound)) {
            outside.push(format!("  {} word {k}: card {} processor {}", labels[i], c[k], w[k]));
        }
    }
    eprintln!(
        "{name}: {} items, {exact} ({:.3}%) the processor's bits, {} outside {:e} absolute / {:e} relative",
        want.len(),
        100.0 * exact as f64 / want.len().max(1) as f64,
        outside.len(),
        bound.absolute,
        bound.relative
    );
    for line in outside.iter().take(12) {
        eprintln!("{line}");
    }
    (exact, outside.len())
}

fn within_budget(name: &str, outside: usize, total: usize) {
    assert!(outside * 1000 <= total, "{name}: {outside} of {total} items outside the bound, more than one in a thousand");
}

// ---- The instruction ------------------------------------------------------

/// Words an item takes: the settings, the place, the mapping's three
/// constants, the point (position, normal, uv, footprint, scale, uv
/// footprint and whether there is one), and nought for `cy_zero`.
const IN: usize = 31;

const INSTRUCTION: &str = "
    cy_zero = inp(30u);
    let place = vec4<f32>(inf(1u), inf(2u), inf(3u), inf(4u));
    let a1 = vec4<f32>(inf(5u), inf(6u), inf(7u), inf(8u));
    let a2 = vec4<f32>(inf(9u), inf(10u), inf(11u), inf(12u));
    let a3 = vec4<f32>(inf(13u), inf(14u), inf(15u), inf(16u));
    var tp: TexPoint;
    tp.position = vec3<f32>(inf(17u), inf(18u), inf(19u));
    tp.normal = vec3<f32>(inf(20u), inf(21u), inf(22u));
    tp.uv = vec2<f32>(inf(23u), inf(24u));
    tp.footprint = inf(25u);
    tp.scale = inf(26u);
    tp.uv_footprint = vec2<f32>(inf(27u), inf(28u));
    tp.has_uv_footprint = inp(29u) != 0u;
    let r = tex_instruction(inp(0u), place, a1, a2, a3, tp);
    outf(0u, r.x); outf(1u, r.y); outf(2u, r.z); outf(3u, r.w);";

/// One instruction to run: its settings, the place (for `at`), the
/// mapping, the point; and what the processor makes of it.
struct Item {
    settings: u32,
    place: [f32; 2],
    mapping: Mapping,
    point: SurfacePoint,
    want: Vec<f32>,
    label: String,
}

fn inputs(items: &[Item]) -> Vec<u32> {
    let mut out = Vec::with_capacity(items.len() * IN);
    for item in items {
        let c = program::mapping_constants(&item.mapping);
        let p = &item.point;
        let (uvf, has) = match p.uv_footprint {
            Some(f) => (f, 1u32),
            None => ([0.0, 0.0], 0),
        };
        out.push(item.settings);
        let f: Vec<f32> = [item.place[0], item.place[1], 0.0, 0.0]
            .into_iter()
            .chain(c.iter().flatten().copied())
            .chain([p.position.x, p.position.y, p.position.z, p.normal.x, p.normal.y, p.normal.z, p.uv.x, p.uv.y, p.footprint, p.scale, uvf[0], uvf[1]])
            .collect();
        out.extend(f.iter().map(|v| v.to_bits()));
        out.push(has);
        out.push(0);
    }
    assert_eq!(out.len(), items.len() * IN);
    out
}

fn run(h: &Harness, card: &CardPictures, items: &[Item]) -> Vec<u32> {
    h.run_wgsl_with(&["textures"], INSTRUCTION, &inputs(items), IN, 4, &[(16, &card.texels), (17, &card.header)])
}

fn check(h: &Harness, card: &CardPictures, name: &str, items: &[Item], bound: Bound) -> (usize, usize) {
    let got = run(h, card, items);
    let want: Vec<Vec<f32>> = items.iter().map(|i| i.want.clone()).collect();
    let labels: Vec<String> = items.iter().map(|i| i.label.clone()).collect();
    tally(name, &got, &want, &labels, bound)
}

// ---- Random inputs --------------------------------------------------------

fn uniform(rng: &mut Pcg32, lo: f32, hi: f32) -> f32 {
    lo + (hi - lo) * rng.next_f32()
}

fn log_uniform(rng: &mut Pcg32, lo: f32, hi: f32) -> f32 {
    (lo.ln() + (hi.ln() - lo.ln()) * rng.next_f32()).exp()
}

fn unit(rng: &mut Pcg32) -> Vec3 {
    loop {
        let v = vec3(uniform(rng, -1.0, 1.0), uniform(rng, -1.0, 1.0), uniform(rng, -1.0, 1.0));
        let l = v.length();
        if l > 0.1 && l <= 1.0 {
            return v * (1.0 / l);
        }
    }
}

/// A coordinate across a picture `size` texels wide: anywhere in a few
/// tiles, now and then on an edge, a texel centre or a hair from a tile's.
fn coordinate(rng: &mut Pcg32, size: usize) -> f32 {
    let r = rng.next_f32();
    let n = size as f32;
    if r < 0.05 {
        [0.0, 1.0, -1e-9, 1.0 - 1e-7, 2.0, -1.0][(rng.next_u32() % 6) as usize]
    } else if r < 0.15 {
        ((rng.next_u32() % size as u32) as f32 + 0.5) / n
    } else if r < 0.2 {
        (rng.next_u32() % (size as u32 + 1)) as f32 / n
    } else {
        uniform(rng, -2.0, 3.0)
    }
}

fn wrap(rng: &mut Pcg32) -> Wrap {
    [Wrap::Repeat, Wrap::Clamp, Wrap::Mirror][(rng.next_u32() % 3) as usize]
}

/// A footprint: from far under a texel to wider than the picture.
fn footprint(rng: &mut Pcg32) -> f32 {
    let r = rng.next_f32();
    if r < 0.05 {
        0.0
    } else {
        log_uniform(rng, 1e-6, 3.0)
    }
}

fn point(position: Vec3, normal: Vec3, uv: (f32, f32), footprint: f32, scale: f32, uv_footprint: Option<[f32; 2]>) -> SurfacePoint {
    SurfacePoint {
        position,
        normal,
        uv: vec2(uv.0, uv.1),
        footprint,
        scale,
        uv_footprint,
        curvature: 0.0,
        occlusion: 0.0,
        vertex_color: Spectrum::ONE,
        world: PointWorld::default(),
    }
}

/// A mapping's shared controls, at random: plain half the time, else
/// scaled, offset, turned about a pivot, flipped.
fn controls(rng: &mut Pcg32, mut mapping: Mapping) -> Mapping {
    mapping.wrap_u = wrap(rng);
    mapping.wrap_v = if rng.next_f32() < 0.5 { mapping.wrap_u } else { wrap(rng) };
    if rng.next_f32() < 0.5 {
        mapping.scale = vec3(uniform(rng, 0.2, 6.0) * if rng.next_f32() < 0.1 { -1.0 } else { 1.0 }, uniform(rng, 0.2, 6.0), 1.0);
        mapping.offset = vec2(uniform(rng, -1.0, 1.0), uniform(rng, -1.0, 1.0));
        if rng.next_f32() < 0.6 {
            mapping.rotation = uniform(rng, -1.0, 1.0);
            mapping.pivot = vec2(uniform(rng, -1.0, 1.0), uniform(rng, -1.0, 1.0));
            mapping.rigid = rng.next_f32() < 0.5;
        }
        mapping.flip_u = rng.next_f32() < 0.2;
        mapping.flip_v = rng.next_f32() < 0.2;
    }
    mapping
}

fn vec4(v: Vec3) -> Vec<f32> {
    vec![v.x, v.y, v.z, 0.0]
}

// ---- Tests ----------------------------------------------------------------

/// Every value each encoding holds reads back on the card as `Level` reads
/// it: every sRGB byte through the curve's table, every byte, every sixteen
/// bits, every finite half float, and random floats.
#[test]
fn every_value_of_every_encoding_decodes_to_the_bit() {
    let Some(h) = Harness::for_test() else { return };
    let all_bytes: Vec<u8> = (0..=255u8).collect();
    let sixteen: Vec<f32> = (0..=65535u32).map(|v| v as f32 / 65535.0).collect();
    let finite: Vec<f32> = (0..=65535u16).map(half_to_f32).filter(|v| v.is_finite()).collect();
    let mut halves = finite.clone();
    halves.resize(256 * 256, 0.0);
    let mut rng = Pcg32::new(5, 5);
    let random: Vec<f32> = (0..64 * 64 * 3).map(|_| f32::from_bits(rng.next_u32() & 0xBFFF_FFFF)).collect();
    let pyramids = [
        ("sRGB bytes", MipPyramid::build(Image::from_srgb8(256, 1, 1, &all_bytes)), tequila_texture::Encoding::Srgb8),
        ("bytes", MipPyramid::build(Image::from_raw8(256, 1, 1, &all_bytes)), tequila_texture::Encoding::Unorm8),
        ("sixteen bits", MipPyramid::build(floats(256, 256, 1, sixteen, ColorSpace::Raw)), tequila_texture::Encoding::Unorm16),
        ("half floats", MipPyramid::build(floats(256, 256, 1, halves, ColorSpace::LinearSrgb)), tequila_texture::Encoding::Half),
        ("floats", MipPyramid::build_with(floats(64, 64, 3, random, ColorSpace::LinearSrgb), Precision::Lossless), tequila_texture::Encoding::Float),
        ("sRGB bytes RGBA", MipPyramid::build(Image::from_srgb8(300, 260, 4, &noise_bytes(300 * 260 * 4, 9))), tequila_texture::Encoding::Srgb8),
    ];
    let mut card = CardPictures::default();
    let mut inputs = Vec::new();
    let mut want = Vec::new();
    let mut labels = Vec::new();
    for (name, pyramid, encoding) in &pyramids {
        assert_eq!(pyramid.base().encoding(), *encoding, "{name}: the base is kept as {:?}", pyramid.base().encoding());
        let index = card.add(pyramid);
        for level in 0..pyramid.levels().min(2) {
            let l = pyramid.level(level);
            for y in 0..l.height {
                for x in 0..l.width {
                    inputs.extend_from_slice(&[index, level as u32, x as u32, y as u32]);
                    let t = l.texel(x, y);
                    want.push(vec![t.x, t.y, t.z]);
                    labels.push(format!("{name} level {level} ({x}, {y})"));
                }
            }
        }
    }
    card.finish();
    let body = "
    let record = TEX_PYRAMIDS + 4u * inp(0u);
    let at = texture_header[record] + 4u * inp(1u);
    let level = vec4<u32>(texture_header[at], texture_header[at + 1u], texture_header[at + 2u], texture_header[at + 3u]);
    let t = tex_texel(level, inp(2u), inp(3u));
    outf(0u, t.x); outf(1u, t.y); outf(2u, t.z);";
    let got = h.run_wgsl_with(&["textures"], body, &inputs, 4, 3, &[(16, &card.texels), (17, &card.header)]);
    let (exact, _) = tally("decode", &got, &want, &labels, Bound { absolute: 0.0, relative: 0.0 });
    assert_eq!(exact, want.len(), "every value is the processor's bits");
}

/// The level's logarithm is the correctly rounded log2 for every footprint
/// a lookup can have; the processor's log2f is that all but about once in a
/// thousand, which is printed.
#[test]
fn the_levels_logarithm_is_correctly_rounded() {
    let Some(h) = Harness::for_test() else { return };
    let mut rng = Pcg32::new(7, 7);
    let mut xs: Vec<f32> = (0..400_000).map(|_| log_uniform(&mut rng, 1.0, 70_000.0)).collect();
    for e in 0..17 {
        let p = 2f32.powi(e);
        xs.extend_from_slice(&[p, f32::from_bits(p.to_bits() + 1), f32::from_bits(p.to_bits().saturating_sub(1))]);
    }
    xs.extend((0..200_000u32).map(|i| f32::from_bits(1.0f32.to_bits() + i)));
    let inputs: Vec<u32> = xs.iter().flat_map(|x| [x.to_bits(), 0]).collect();
    let got = h.run_wgsl_with(&["textures"], "cy_zero = inp(1u); outf(0u, tex_log2(inf(0u)));", &inputs, 2, 1, &[(16, &[0u32]), (17, &tequila_render_gpu::textures::header_tables())]);
    let (mut wrong, mut unlike_processor) = (Vec::new(), 0usize);
    for (x, g) in xs.iter().zip(&got) {
        let g = f32::from_bits(*g);
        let correct = if *x < 1.0 { -1.0 } else { (*x as f64).log2() as f32 };
        if g.to_bits() != correct.to_bits() {
            wrong.push((*x, g, correct));
        }
        if *x >= 1.0 && g.to_bits() != x.log2().to_bits() {
            unlike_processor += 1;
        }
    }
    eprintln!(
        "log2: {} values, {} not correctly rounded, {} ({:.3}%) unlike the processor's log2f",
        xs.len(),
        wrong.len(),
        unlike_processor,
        100.0 * unlike_processor as f64 / xs.len() as f64
    );
    for (x, g, c) in wrong.iter().take(8) {
        eprintln!("  log2({x:e}): card {g:e}, correctly rounded {c:e}");
    }
    assert!(wrong.len() * 10_000 <= xs.len(), "{} of {} not correctly rounded", wrong.len(), xs.len());
}

/// `Texture::sample_with` through the mesh's UVs, over every picture.
#[test]
fn uv_lookups_are_the_processors() {
    let Some(h) = Harness::for_test() else { return };
    let pictures = on_card();
    let mut rng = Pcg32::new(11, 1);
    let mut items = Vec::new();
    for (t, (name, texture)) in pictures.textures.iter().enumerate() {
        let base = texture.pyramid().base();
        for _ in 0..12_000 {
            let mapping = controls(&mut rng, Mapping::default());
            let uv = (coordinate(&mut rng, base.width), coordinate(&mut rng, base.height));
            let uv_footprint = if rng.next_f32() < 0.8 {
                let a = footprint(&mut rng);
                Some([a, if rng.next_f32() < 0.5 { a } else { footprint(&mut rng) }])
            } else {
                None
            };
            let p = point(vec3(uniform(&mut rng, -1.0, 1.0), 0.0, 0.0), Vec3::Y, uv, footprint(&mut rng), 1.0, uv_footprint);
            let want = texture.sample_with(&mapping, &p);
            items.push(Item {
                settings: program::settings::texture(TEXTURE_COLOUR, &mapping, false, false, pictures.colour[t]),
                place: [0.0, 0.0],
                mapping,
                point: p,
                want: vec4(want),
                label: format!("{name} at {uv:?} {uv_footprint:?} {mapping:?}"),
            });
        }
    }
    let (_, outside) = check(&h, &pictures.card, "UV lookups", &items, TIGHT);
    within_budget("UV lookups", outside, items.len());
}

/// `sample_at` and `alpha_at`, at places a graph worked out; and
/// `sample_alpha` through the UVs.
#[test]
fn lookups_at_a_place_and_alpha_are_the_processors() {
    let Some(h) = Harness::for_test() else { return };
    let pictures = on_card();
    let mut rng = Pcg32::new(13, 1);
    let mut items = Vec::new();
    for (t, (name, texture)) in pictures.textures.iter().enumerate() {
        let base = texture.pyramid().base();
        for _ in 0..6_000 {
            let place = [coordinate(&mut rng, base.width) * 2.0 - 0.5, coordinate(&mut rng, base.height) * 2.0 - 0.5];
            let p = point(Vec3::ZERO, Vec3::Y, (0.0, 0.0), 0.0, 1.0, None);
            items.push(Item {
                settings: program::settings::texture(TEXTURE_COLOUR, &Mapping::default(), true, false, pictures.colour[t]),
                place,
                mapping: Mapping::default(),
                point: p,
                want: vec4(texture.sample_at(place[0], place[1])),
                label: format!("{name} sample_at {place:?}"),
            });
            let Some(alpha) = pictures.alpha[t] else { continue };
            let a = texture.alpha_at(place[0], place[1]);
            items.push(Item {
                settings: program::settings::texture(TEXTURE_ALPHA, &Mapping::default(), true, false, alpha),
                place,
                mapping: Mapping::default(),
                point: p,
                want: vec![a; 4],
                label: format!("{name} alpha_at {place:?}"),
            });
            let mapping = controls(&mut rng, Mapping::default());
            let uv = (coordinate(&mut rng, base.width), coordinate(&mut rng, base.height));
            let f = footprint(&mut rng);
            let p = point(Vec3::ZERO, Vec3::Y, uv, f, 1.0, Some([f, f * 0.5]));
            let a = texture.sample_alpha(&mapping, &p);
            items.push(Item {
                settings: program::settings::texture(TEXTURE_ALPHA, &mapping, false, false, alpha),
                place: [0.0, 0.0],
                mapping,
                point: p,
                want: vec![a; 4],
                label: format!("{name} sample_alpha at {uv:?} {mapping:?}"),
            });
        }
    }
    let (_, outside) = check(&h, &pictures.card, "lookups at a place, alpha", &items, TIGHT);
    within_budget("lookups at a place, alpha", outside, items.len());
}

/// A colour picture's lookup brought into the rendering space
/// (`colour_of`), as `Spectrum::from_linear_srgb` brings it.
#[test]
fn a_colour_lookup_is_brought_into_the_rendering_space() {
    let Some(h) = Harness::for_test() else { return };
    let pictures = on_card();
    let mut rng = Pcg32::new(17, 1);
    let mut items = Vec::new();
    for (t, (name, texture)) in pictures.textures.iter().enumerate() {
        let base = texture.pyramid().base();
        for _ in 0..2_000 {
            let mapping = controls(&mut rng, Mapping::default());
            let uv = (coordinate(&mut rng, base.width), coordinate(&mut rng, base.height));
            let f = footprint(&mut rng);
            let p = point(Vec3::ZERO, Vec3::Y, uv, f, 1.0, Some([f, f]));
            let s = Spectrum::from_linear_srgb(texture.sample_with(&mapping, &p));
            items.push(Item {
                settings: program::settings::texture(TEXTURE_COLOUR, &mapping, false, true, pictures.colour[t]),
                place: [0.0, 0.0],
                mapping,
                point: p,
                want: vec![s.r, s.g, s.b, 0.0],
                label: format!("{name} managed at {uv:?}"),
            });
        }
    }
    let (_, outside) = check(&h, &pictures.card, "colour_of", &items, TIGHT);
    within_budget("colour_of", outside, items.len());
}

/// A mapping of `projection` with random controls: real-world sizes or
/// repeats, an axis, a tri-planar blend.
fn projected_mapping(rng: &mut Pcg32, projection: Projection) -> Mapping {
    let mut m = controls(rng, Mapping { projection, ..Mapping::default() });
    m.axis = [Axis::X, Axis::Y, Axis::Z][(rng.next_u32() % 3) as usize];
    m.blend_sharpness = if rng.next_f32() < 0.5 { 6.0 } else { uniform(rng, 0.5, 9.0) };
    if rng.next_f32() < 0.4 {
        m.real_world_units = true;
        m.scale = vec3(log_uniform(rng, 0.05, 3.0), log_uniform(rng, 0.05, 3.0), 1.0);
    }
    m
}

fn projected_point(rng: &mut Pcg32) -> SurfacePoint {
    let reach = if rng.next_f32() < 0.5 { 1.0 } else { 20.0 };
    let position = vec3(uniform(rng, -reach, reach), uniform(rng, -reach, reach), uniform(rng, -reach, reach));
    let normal = if rng.next_f32() < 0.2 { [Vec3::X, Vec3::Y, Vec3::Z, -Vec3::X][(rng.next_u32() % 4) as usize] } else { unit(rng) };
    let scale = if rng.next_f32() < 0.5 { 1.0 } else { log_uniform(rng, 0.01, 100.0) };
    point(position, normal, (uniform(rng, -1.0, 2.0), uniform(rng, -1.0, 2.0)), log_uniform(rng, 1e-5, 0.5), scale, None)
}

const PROJECTIONS: [Projection; 5] = [Projection::Uv, Projection::Planar, Projection::Cylindrical, Projection::Spherical, Projection::TriPlanar];

/// `Mapping::project`'s first contribution, as `Instruction::Project`
/// gives it: (u, v, width).
#[test]
fn projected_coordinates_are_the_processors() {
    let Some(h) = Harness::for_test() else { return };
    let card = {
        let mut c = CardPictures::default();
        c.finish();
        c
    };
    for projection in PROJECTIONS {
        let mut rng = Pcg32::new(19, projection as u64);
        let items: Vec<Item> = (0..40_000)
            .map(|_| {
                let mapping = projected_mapping(&mut rng, projection);
                let p = projected_point(&mut rng);
                let first = mapping.project(&p)[0].coords;
                Item {
                    settings: program::settings::texture(TEXTURE_PROJECT, &mapping, false, false, 0),
                    place: [0.0, 0.0],
                    mapping,
                    point: p,
                    want: vec![first.u, first.v, first.width(), 0.0],
                    label: format!("{projection:?} at {:?} n {:?} scale {} {mapping:?}", p.position, p.normal, p.scale),
                }
            })
            .collect();
        // Spherical v goes through the card's own acos.
        let bound = match projection {
            Projection::Spherical => Bound { absolute: 2e-6, relative: 2e-5 },
            _ => TIGHT,
        };
        let name = format!("{projection:?} coordinates");
        let (_, outside) = check(&h, &card, &name, &items, bound);
        within_budget(&name, outside, items.len());
    }
}

/// `Texture::sample_with` through each projection.
#[test]
fn lookups_through_every_projection_are_the_processors() {
    let Some(h) = Harness::for_test() else { return };
    let pictures = on_card();
    for projection in PROJECTIONS {
        let mut rng = Pcg32::new(23, projection as u64);
        let mut items = Vec::new();
        for (t, (name, texture)) in pictures.textures.iter().enumerate() {
            for _ in 0..3_000 {
                let mapping = projected_mapping(&mut rng, projection);
                let mut p = projected_point(&mut rng);
                if rng.next_f32() < 0.5 {
                    let f = footprint(&mut rng);
                    p.uv_footprint = Some([f, f]);
                }
                items.push(Item {
                    settings: program::settings::texture(TEXTURE_COLOUR, &mapping, false, false, pictures.colour[t]),
                    place: [0.0, 0.0],
                    mapping,
                    point: p,
                    want: vec4(texture.sample_with(&mapping, &p)),
                    label: format!("{name} {projection:?} at {:?} n {:?} {mapping:?}", p.position, p.normal),
                });
            }
        }
        // An ulp of spherical v (the card's acos) is a part in ten million;
        // scaled to a few hundred repeats across a picture a few hundred
        // texels wide it moves a lookup a hundredth of a texel, which on
        // noise from black to white is a hundredth of its range.
        let bound = match projection {
            Projection::Spherical => Bound { absolute: 2e-3, relative: 2e-3 },
            _ => TIGHT,
        };
        let name = format!("{projection:?} lookups");
        let (_, outside) = check(&h, &pictures.card, &name, &items, bound);
        within_budget(&name, outside, items.len());
    }
}

/// A normal map laid down by a projection (`Texture::sample_normal_with`):
/// the normal it gives in the part's space, or none. Its frames are the
/// projection's coordinates differenced a step apart, so an ulp of an angle
/// is a part in a thousand of a cylindrical or spherical frame: those are
/// held to 1e-2, planar and tri-planar to 1e-4.
#[test]
fn projected_normal_maps_are_the_processors() {
    let Some(h) = Harness::for_test() else { return };
    let pictures = on_card();
    for projection in [Projection::Planar, Projection::TriPlanar, Projection::Cylindrical, Projection::Spherical] {
        let mut rng = Pcg32::new(29, projection as u64);
        let mut items = Vec::new();
        for (t, (name, texture)) in pictures.textures.iter().enumerate() {
            for _ in 0..2_000 {
                let mapping = projected_mapping(&mut rng, projection);
                let p = projected_point(&mut rng);
                let want = match texture.sample_normal_with(&mapping, &p) {
                    Some(n) => vec![n.x, n.y, n.z, 1.0],
                    None => vec![0.0; 4],
                };
                items.push(Item {
                    settings: program::settings::texture(TEXTURE_NORMAL, &mapping, false, false, pictures.colour[t]),
                    place: [0.0, 0.0],
                    mapping,
                    point: p,
                    want,
                    label: format!("{name} {projection:?} normal at {:?} n {:?} {mapping:?}", p.position, p.normal),
                });
            }
        }
        let bound = match projection {
            Projection::Planar | Projection::TriPlanar => Bound { absolute: 1e-4, relative: 0.0 },
            _ => Bound { absolute: 1e-2, relative: 0.0 },
        };
        let name = format!("{projection:?} normal maps");
        let (_, outside) = check(&h, &pictures.card, &name, &items, bound);
        within_budget(&name, outside, items.len());
    }
}

/// The card's division and arctangent are the processor's to the bit: its
/// division is IEEE's; its arctangent is correctly rounded, which the
/// processor's atan2f is all but now and then where the angle is small
/// (where they differ, which is right is printed).
#[test]
fn division_and_the_arctangent_are_the_processors() {
    let Some(h) = Harness::for_test() else { return };
    let mut rng = Pcg32::new(43, 1);
    let mut pairs: Vec<(f32, f32)> = (0..300_000)
        .map(|_| {
            let sign = |rng: &mut Pcg32| if rng.next_f32() < 0.5 { -1.0 } else { 1.0 };
            (sign(&mut rng) * log_uniform(&mut rng, 1e-6, 1e6), sign(&mut rng) * log_uniform(&mut rng, 1e-6, 1e6))
        })
        .collect();
    pairs.extend((0..=65535u32).map(|v| (v as f32, 65535.0)));
    pairs.extend((0..100_000).map(|_| (uniform(&mut rng, -20.0, 20.0), uniform(&mut rng, -20.0, 20.0))));
    pairs.extend([(0.0, 1.0), (-0.0, 1.0), (0.0, -1.0), (-0.0, -1.0), (1.0, 0.0), (-1.0, 0.0), (1.0, -0.0), (0.0, 0.0), (-0.0, -0.0), (3.0, 3.0), (-2.0, 2.0)]);
    let inputs: Vec<u32> = pairs.iter().flat_map(|(a, b)| [a.to_bits(), b.to_bits(), 0]).collect();
    let got = h.run_wgsl_with(
        &["textures"],
        "cy_zero = inp(2u); outf(0u, tex_div(inf(0u), inf(1u))); outf(1u, tex_atan2(inf(0u), inf(1u)));",
        &inputs,
        3,
        2,
        &[(16, &[0u32]), (17, &tequila_render_gpu::textures::header_tables())],
    );
    let (mut division, mut arctangent, mut processor_misrounds) = (Vec::new(), Vec::new(), 0usize);
    for (i, (a, b)) in pairs.iter().enumerate() {
        let (d, t) = (f32::from_bits(got[2 * i]), f32::from_bits(got[2 * i + 1]));
        if d.to_bits() != (a / b).to_bits() && !(d.is_nan() && (a / b).is_nan()) {
            division.push(format!("  {a:e} / {b:e}: card {d:e}, processor {:e}", a / b));
        }
        if t.to_bits() != a.atan2(*b).to_bits() {
            let correct = (*a as f64).atan2(*b as f64) as f32;
            if t.to_bits() == correct.to_bits() {
                processor_misrounds += 1;
            }
            arctangent.push(format!("  atan2({a:e}, {b:e}): card {t:e}, processor {:e}, correctly rounded {correct:e}", a.atan2(*b)));
        }
    }
    eprintln!(
        "division: {} of {} unlike the processor's; atan2: {} unlike, of which the processor's is the one not correctly rounded {processor_misrounds} times",
        division.len(),
        pairs.len(),
        arctangent.len()
    );
    for line in division.iter().take(6).chain(arctangent.iter().take(16)) {
        eprintln!("{line}");
    }
    assert!(division.is_empty(), "the card's division is not IEEE's");
    assert!(arctangent.len() * 1000 <= pairs.len(), "{} arctangents unlike the processor's", arctangent.len());
}

/// The tri-planar weights' powf is the processor's to the bit, all but
/// about once in ten thousand (its own rounding, a hair from a midpoint).
#[test]
fn the_tri_planar_power_is_the_processors() {
    let Some(h) = Harness::for_test() else { return };
    let mut rng = Pcg32::new(47, 1);
    let mut pairs: Vec<(f32, f32)> = (0..300_000)
        .map(|_| {
            let x = if rng.next_f32() < 0.1 { log_uniform(&mut rng, 1e-12, 1.0) } else { rng.next_f32() };
            let k = if rng.next_f32() < 0.4 { [1.0, 2.0, 4.0, 6.0, 8.0][(rng.next_u32() % 5) as usize] } else { uniform(&mut rng, 1.0, 9.0) };
            (x, k)
        })
        .collect();
    pairs.extend([(0.0, 6.0), (1.0, 6.0), (0.5, 1.0), (1e-30, 4.0), (0.70710677, 6.0)]);
    let inputs: Vec<u32> = pairs.iter().flat_map(|(x, k)| [x.to_bits(), k.to_bits(), 0]).collect();
    let got = h.run_wgsl_with(&["textures"], "cy_zero = inp(2u); outf(0u, tex_powf(inf(0u), inf(1u)));", &inputs, 3, 1, &[(16, &[0u32]), (17, &tequila_render_gpu::textures::header_tables())]);
    let mut unlike = Vec::new();
    for ((x, k), g) in pairs.iter().zip(&got) {
        let (g, want) = (f32::from_bits(*g), x.powf(*k));
        // Below the smallest normal float the weight is nought on the card.
        if g.to_bits() != want.to_bits() && !(want < 1.2e-38 && g == 0.0) {
            unlike.push(format!("  {x:e}^{k}: card {g:e}, processor {want:e}"));
        }
    }
    eprintln!("powf: {} of {} unlike the processor's", unlike.len(), pairs.len());
    for line in unlike.iter().take(8) {
        eprintln!("{line}");
    }
    assert!(unlike.len() * 1000 <= pairs.len(), "{} powers unlike the processor's", unlike.len());
}
