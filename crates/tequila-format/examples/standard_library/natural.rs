//! Wood, stone, ceramic, leather, paper, cork and composites.

use crate::recipe::Linear;
use crate::{base, noise, plus, preset, rgb, srgb, Preset, Recipe};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::{MaterialType, Param};

/// Terrazzo: a cement matrix between `matrix.0` and `matrix.1`, clouded,
/// with chips of stone in `chips` - two sizes, about `scale` to the metre
/// and twice that, holding `density` of the cells - and a fine speckle;
/// ground to `roughness`.
///
/// A chip is a Voronoi cell shrunk towards its middle by its own random
/// amount: the cell's shape kept, so a broken shard's straight sides and
/// corners, and every chip a different size.
fn terrazzo(name: &'static str, tags: &'static [&'static str], matrix: (Spectrum, Spectrum), chips: &[(f32, Spectrum)], scale: f32, density: f32, roughness: f32) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = matrix.0.lerp(matrix.1, 0.5);
        m.specular_roughness = roughness;
    }));
    let coords = r.node("texture_coordinate").out("Object");
    let cloud = r.node("noise_texture").w("Vector", coords).f("Scale", 25.0).f("Detail", 5.0).f("Roughness", 0.6).out("Fac");
    let mut colour = r.node("mix").f("Factor", cloud).c("A", matrix.0).c("B", matrix.1).out("Result");
    let mut solid = r.math("multiply", cloud, 0.0);
    for (size, keep_from, seed) in [(scale, 1.0 - density, 0.0f32), (scale * 2.2, 1.0 - density * 0.9, 17.3)] {
        let shifted = r.node("vector_math").set("operation", "add").w("Vector", coords).v("Vector 2", [seed, seed * 0.7, seed * 1.3]).out("Vector");
        // Broken stone's edges are straight only roughly: a fine warp nicks them.
        let nick = r.node("noise_texture").w("Vector", shifted).f("Scale", size * 5.0).f("Detail", 3.0).out("Color");
        let reach = 0.12 / size;
        let shifted = r.node("vector_math").set("operation", "multiply_add").w("Vector", nick).v("Vector 2", [reach, reach, reach]).w("Vector 3", shifted).out("Vector");
        let edge = r.node("voronoi_texture").set("feature", "distance_to_edge").w("Vector", shifted).f("Scale", size).f("Randomness", 1.0).out("Distance");
        let cell = r.node("voronoi_texture").w("Vector", shifted).f("Scale", size).f("Randomness", 1.0).add();
        let pick = r.node("separate_color").w("Color", crate::Out(cell, 1)).add();
        // Kept or not, and how far in from the cell's edge the chip stops.
        let kept = r.smooth(crate::Out(pick, 0), (keep_from, keep_from + 0.01), (0.0, 1.0));
        let margin = r.range(crate::Out(pick, 1), (0.0, 1.0), (0.05, 0.22));
        let inside = r.math("subtract", edge, margin);
        let shard = r.smooth(inside, (0.0, 0.01), (0.0, 1.0));
        let chip = r.math("multiply", shard, kept);
        let stone = r.ramp(crate::Out(pick, 2), crate::recipe::Constant, chips);
        let grain = r.node("noise_texture").w("Vector", coords).f("Scale", size * 6.0).f("Detail", 3.0).out("Fac");
        let grain = r.range(grain, (0.3, 0.7), (0.88, 1.1));
        let stone = r.mix("multiply", 1.0, stone, grain);
        colour = r.node("mix").f("Factor", chip).w("A", colour).w("B", stone).out("Result");
        solid = r.math("maximum", solid, chip);
    }
    // The speckle: sand-sized grains of the same stones, everywhere.
    let dust = r.node("voronoi_texture").w("Vector", coords).f("Scale", scale * 12.0).f("Randomness", 1.0).add();
    let dust_pick = r.node("separate_color").w("Color", crate::Out(dust, 1)).add();
    let dust_kept = r.smooth(crate::Out(dust_pick, 0), (0.35, 0.37), (0.0, 1.0));
    let dust_dot = r.smooth(crate::Out(dust, 0), (0.12, 0.2), (1.0, 0.0));
    let speck = r.math("multiply", dust_kept, dust_dot);
    let speck_stone = r.ramp(crate::Out(dust_pick, 2), crate::recipe::Constant, chips);
    // Grains that small are mostly the paler stones, half lost in the cement.
    let speck_stone = r.node("mix").f("Factor", 0.45).w("A", speck_stone).c("B", matrix.0).out("Result");
    let colour = r.node("mix").f("Factor", speck).w("A", colour).w("B", speck_stone).out("Result");
    // And finer still: a grey grit through the cement, the sand in it.
    let grit = r.node("voronoi_texture").w("Vector", coords).f("Scale", scale * 30.0).f("Randomness", 1.0).add();
    let grit_pick = r.node("separate_color").w("Color", crate::Out(grit, 1)).out("Red");
    let grit_kept = r.smooth(grit_pick, (0.5, 0.52), (0.0, 1.0));
    let grit_dot = r.smooth(crate::Out(grit, 0), (0.15, 0.25), (0.55, 0.0));
    let grit = r.math("multiply", grit_kept, grit_dot);
    let colour = r.node("mix").f("Factor", grit).w("A", colour).c("B", srgb("#8E8A88")).out("Result");
    r.to(Param::BaseColor, colour);
    // Ground flat: the stone takes a slightly better polish than the cement,
    // and the cement keeps the odd pinhole.
    let gloss = r.range(solid, (0.0, 1.0), (roughness * 1.5, roughness));
    r.to(Param::SpecularRoughness, gloss);
    let pores = r.node("voronoi_texture").w("Vector", coords).f("Scale", scale * 8.0).f("Randomness", 1.0).add();
    let pore_pick = r.node("separate_color").w("Color", crate::Out(pores, 1)).out("Red");
    let pore_kept = r.smooth(pore_pick, (0.93, 0.95), (0.0, 1.0));
    let pore_dot = r.smooth(crate::Out(pores, 0), (0.08, 0.14), (-1.0, 0.0));
    let pore = r.math("multiply", pore_kept, pore_dot);
    r.bump(pore, 5e-5);
    preset(name, "Stone", tags, MaterialType::Generic, r)
}

/// Cork pressed from crumbs `scale` to the metre, squashed to `squash` of
/// their width along y (flakes lie flat); toned by `tones` crumb by crumb;
/// gaps towards `seam`; with `lenticels` (0 to 1) of the bark's dark dashes.
///
/// What makes it read as cork rather than a mosaic, from the scans: crumbs
/// of more than one size, torn rather than cut; most of the variation
/// inside each crumb (its fibre, its mottle) rather than between crumbs; no
/// outlines, the boundaries shown only by each crumb sitting at its own
/// height and the odd gap.
fn cork(name: &'static str, tags: &'static [&'static str], scale: f32, squash: f32, tones: &[(f32, Spectrum)], seam: Spectrum, lenticels: f32) -> Preset {
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = tones[2].1;
        m.base_diffuse_roughness = 0.65;
        m.specular_roughness = 0.78;
    }));
    let coords = r.node("texture_coordinate").out("Object");
    let flat = r.node("mapping").w("Vector", coords).v("Scale", [1.0, 1.0 / squash, 1.0]).out("Vector");
    let wobble = r.node("noise_texture").w("Vector", flat).f("Scale", scale * 1.3).f("Detail", 4.0).f("Roughness", 0.6).out("Color");
    let reach = 0.65 / scale;
    let warped = r.node("vector_math").set("operation", "multiply_add").w("Vector", wobble).v("Vector 2", [reach, reach, reach]).w("Vector 3", flat).out("Vector");
    // Two sizes of crumb, the smaller filling about two fifths of the board.
    let big = r.node("voronoi_texture").w("Vector", warped).f("Scale", scale).f("Randomness", 1.0).add();
    let small = r.node("voronoi_texture").w("Vector", warped).f("Scale", scale * 2.3).f("Randomness", 1.0).add();
    let which_field = r.node("noise_texture").w("Vector", coords).f("Scale", scale * 0.7).f("Detail", 2.0).out("Fac");
    let which = r.smooth(which_field, (0.52, 0.56), (0.0, 1.0));
    let cell = r.node("mix").set("data_type", "vector").f("Factor", which).w("A", crate::Out(big, 1)).w("B", crate::Out(small, 1)).out("Result");
    let big_edge = r.node("voronoi_texture").set("feature", "distance_to_edge").w("Vector", warped).f("Scale", scale).f("Randomness", 1.0).out("Distance");
    let small_edge = r.node("voronoi_texture").set("feature", "distance_to_edge").w("Vector", warped).f("Scale", scale * 2.3).f("Randomness", 1.0).out("Distance");
    let edge = r.mix_float(which, big_edge, small_edge);
    let pick = r.node("separate_color").w("Color", cell).add();
    let tone = r.ramp(crate::Out(pick, 0), Linear, tones);
    // Each crumb's fibre, its own way: a Gabor field turned by the crumb's
    // own random colour.
    let way = r.node("vector_math").set("operation", "subtract").w("Vector", cell).v("Vector 2", [0.5, 0.5, 0.5]).out("Vector");
    let fine = r.node("mapping").w("Vector", warped).v("Scale", [scale * 0.25; 3]).out("Vector");
    let fibre = r.node("gabor_texture").set("type", "3D").w("Vector", fine).f("Scale", 14.0).f("Frequency", 3.0).f("Anisotropy", 0.85).w("Orientation 2", way).out("Value");
    let streak = r.range(fibre, (-0.5, 0.5), (0.82, 1.12));
    let mottle_field = r.node("noise_texture").w("Vector", warped).f("Scale", scale * 3.5).f("Detail", 4.0).f("Roughness", 0.6).out("Fac");
    let mottle = r.range(mottle_field, (0.3, 0.7), (0.9, 1.07));
    let shade = r.math("multiply", streak, mottle);
    let toned = r.mix("multiply", 1.0, tone, shade);
    // Gaps: only where two crumbs fall apart, shadowed rather than lined.
    let gap_field = r.node("noise_texture").w("Vector", coords).f("Scale", scale * 1.1).f("Detail", 2.0).out("Fac");
    let open = r.smooth(gap_field, (0.42, 0.56), (0.0, 1.0));
    let gap_core = r.smooth(edge, (0.0, 0.035), (1.0, 0.0));
    let gap = r.math("multiply", open, gap_core);
    let gapped = r.node("mix").f("Factor", gap).w("A", toned).c("B", seam).out("Result");
    // Pits: few and small.
    let holes = r.node("voronoi_texture").w("Vector", coords).f("Scale", scale * 4.5).f("Randomness", 1.0).add();
    let hole_pick = r.node("separate_color").w("Color", crate::Out(holes, 1)).out("Red");
    let keep = r.smooth(hole_pick, (0.78, 0.82), (0.0, 1.0));
    let hole_core = r.smooth(crate::Out(holes, 0), (0.05, 0.16), (1.0, 0.0));
    let pit = r.math("multiply", keep, hole_core);
    let mut colour = r.node("mix").f("Factor", pit).w("A", gapped).c("B", srgb("#2E1A10")).out("Result");
    // Each crumb at its own height, its edges falling away.
    let lift = r.math("multiply", crate::Out(pick, 2), 0.7);
    let dome = r.smooth(edge, (0.0, 0.18), (0.0, 0.5));
    let mut height = r.math("add", lift, dome);
    if lenticels > 0.0 {
        // The lenticels: short dark dashes, lying along the flakes.
        let along = r.node("mapping").w("Vector", warped).v("Scale", [scale * 1.2, scale * 6.0, scale * 1.2]).out("Vector");
        let dash_field = r.node("noise_texture").w("Vector", along).f("Scale", 1.0).f("Detail", 2.0).out("Fac");
        let dash = r.smooth(dash_field, (0.67, 0.74), (0.0, lenticels * 0.85));
        colour = r.node("mix").f("Factor", dash).w("A", colour).c("B", srgb("#4A2610")).out("Result");
        let sunk_dash = r.math("multiply", dash, -0.5);
        height = r.math("add", height, sunk_dash);
    }
    r.to(Param::BaseColor, colour);
    let fibre_relief = r.math("multiply", fibre, 0.12);
    let height = r.math("add", height, fibre_relief);
    let hollow = r.math("add", pit, gap);
    let sunk = r.math("multiply", hollow, -1.2);
    let height = r.math("add", height, sunk);
    r.bump(height, 3e-4);
    preset(name, "Other", tags, MaterialType::Generic, r)
}

pub fn all() -> Vec<Preset> {
    let mut out = Vec::new();

    // ---- stone ----------------------------------------------------------------
    // Terrazzo, measured against scanned floors (ambientCG Terrazzo001,
    // Terrazzo005, Poly Haven terrazzo_tiles): mostly matrix - a pale,
    // cloudy cement - with chips of crushed stone set apart in it, sharp
    // and angular, of every size down to a fine speckle, and a palette far
    // quieter than the candy colours this replaced.
    out.push(terrazzo(
        "Terrazzo",
        &["terrazzo", "stone", "chips", "honed", "floor", "white"],
        (srgb("#C9C6C8"), srgb("#DAD7D7")),
        &[(0.0, srgb("#4A4858")), (0.4, srgb("#5E5C6C")), (0.7, srgb("#7D7B88")), (0.9, srgb("#A8A6AE"))],
        260.0,
        0.36,
        0.16,
    ));
    out.push(terrazzo(
        "Terrazzo Black Fleck",
        &["terrazzo", "stone", "chips", "honed", "black", "white"],
        (srgb("#DDD6D0"), srgb("#EEE9E5")),
        &[(0.0, srgb("#262626")), (0.5, srgb("#333333")), (0.75, srgb("#B1A897")), (0.9, srgb("#8E8679"))],
        150.0,
        0.3,
        0.1,
    ));
    out.push(terrazzo(
        "Terrazzo Venetian",
        &["terrazzo", "stone", "chips", "polished", "colour", "venetian"],
        (srgb("#E3DCD0"), srgb("#EFEAE1")),
        &[(0.0, srgb("#B86A4C")), (0.22, srgb("#C99A6A")), (0.4, srgb("#7E9585")), (0.58, srgb("#3E3D3F")), (0.74, srgb("#F2EFE8")), (0.88, srgb("#9B8E80"))],
        210.0,
        0.42,
        0.12,
    ));

    // Slate: dark grey stone split along its bedding. A riven face is a
    // stair of shallow steps and ridges, a little bluer or greener from
    // place to place, with the faint sheen split slate's mica gives it.
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = srgb("#46484B");
        m.base_diffuse_roughness = 0.4;
        m.specular_roughness = 0.5;
    }));
    let layers = r.stretched([1.0, 1.0, 0.12]);
    let field = r.node("noise_texture").w("Vector", layers).f("Scale", 60.0).f("Detail", 5.0).f("Roughness", 0.6).f("Distortion", 0.3).out("Fac");
    // Steps: the field rounded down to a few levels, its slope kept a little.
    let stepped = r.math("multiply", field, 7.0);
    let floored = r.math("floor", stepped, 0.0);
    let levels = r.math("divide", floored, 7.0);
    let riven = plus(&mut r, field, 0.35, levels);
    r.bump(riven, 4e-4);
    let colour = r.ramp(field, Linear, &[(0.0, srgb("#303336")), (0.45, srgb("#44474B")), (1.0, srgb("#5C5F62"))]);
    let tint_field = noise(&mut r, 25.0, 3.0, 0.5);
    let tint = r.ramp(tint_field, Linear, &[(0.0, srgb("#4D4852")), (0.5, srgb("#47494B")), (1.0, srgb("#444C47"))]);
    let tinted = r.mix("overlay", 0.3, colour, tint);
    r.to(Param::BaseColor, tinted);
    let mica = noise(&mut r, 2500.0, 2.0, 0.5);
    let gloss = r.range(mica, (0.3, 0.7), (0.62, 0.36));
    r.to(Param::SpecularRoughness, gloss);
    out.push(preset("Slate", "Stone", &["slate", "stone", "dark", "riven"], MaterialType::Generic, r));

    // Terracotta: fired clay, unglazed. Matte with only the faintest sheen,
    // warmer and paler from place to place as the kiln fired it, a fine
    // sand of grog in the body and the odd dark fleck of iron.
    let mut r = Recipe::new(base(MaterialType::Generic, |m| {
        m.base_color = srgb("#B35E3A");
        m.base_diffuse_roughness = 0.75;
        m.specular_roughness = 0.82;
    }));
    let firing = noise(&mut r, 30.0, 4.0, 0.55);
    let mottle = noise(&mut r, 160.0, 5.0, 0.6);
    let body = r.ramp(firing, Linear, &[(0.25, srgb("#8A4229")), (0.45, srgb("#A3553A")), (0.6, srgb("#B4653F")), (0.8, srgb("#C47C58"))]);
    // Flashing: where the kiln's flame licked, the clay fired darker, in
    // soft patches.
    let flash_field = r.node("noise_texture").f("Scale", 12.0).f("Detail", 5.0).f("Roughness", 0.6).f("Distortion", 0.5).out("Fac");
    let flash = r.smooth(flash_field, (0.56, 0.7), (1.0, 0.72));
    let body = r.mix("multiply", 1.0, body, flash);
    let dusty = r.range(mottle, (0.3, 0.7), (0.86, 1.08));
    let body = r.mix("multiply", 1.0, body, dusty);
    let grog = r.node("voronoi_texture").f("Scale", 900.0).f("Randomness", 1.0).add();
    let grain = r.smooth(crate::Out(grog, 0), (0.08, 0.16), (1.0, 0.0));
    let grain_pick = r.node("separate_color").w("Color", crate::Out(grog, 1)).out("Red");
    let light = r.math("multiply", grain, grain_pick);
    let pale = r.node("mix").f("Factor", light).w("A", body).c("B", srgb("#DCA581")).out("Result");
    // A dusty bloom where salts came out of the clay as it dried.
    let bloom_field = noise(&mut r, 18.0, 4.0, 0.6);
    let bloom = r.smooth(bloom_field, (0.55, 0.75), (0.0, 0.35));
    let pale = r.node("mix").f("Factor", bloom).w("A", pale).c("B", srgb("#D8B49A")).out("Result");
    let flecks = r.node("voronoi_texture").f("Scale", 700.0).f("Randomness", 1.0).out("Distance");
    let fleck = r.smooth(flecks, (0.03, 0.06), (1.0, 0.0));
    let colour = r.node("mix").f("Factor", fleck).w("A", pale).c("B", srgb("#4A2A1C")).out("Result");
    r.to(Param::BaseColor, colour);
    let sand = noise(&mut r, 5000.0, 2.0, 0.6);
    let height = plus(&mut r, grain, 0.5, sand);
    r.bump(height, 3.5e-5);
    out.push(preset("Terracotta", "Stone", &["terracotta", "clay", "pottery", "fired"], MaterialType::Generic, r));

        // ---- leather ---------------------------------------------------------------
    // Pebbled leather as a bag or a seat has it: rounded pebbles about a
    // millimetre and a half across, a tenth of a millimetre deep, over a
    // finer grain, with the grooves between them a little darker and duller
    // than the tops. Semi-matte, with the soft
    // sheen of a hide rather than a clear coat's gloss, which made the last
    // version read as orange plastic. Wrinkles a centimetre or so apart run
    // through it, as hide has where it flexed.
    for (name, tags, colour) in [
        ("Leather Black", &["leather", "grain", "upholstery", "black"] as &'static [&'static str], srgb("#1C1A19")),
        ("Leather Tan", &["leather", "grain", "upholstery", "tan"], srgb("#9C6B3F")),
        ("Leather Oxblood", &["leather", "grain", "upholstery", "red", "burgundy"], srgb("#4A1418")),
        ("Leather Cognac", &["leather", "grain", "upholstery", "brown"], srgb("#8A4A22")),
    ] {
        let mut r = Recipe::new(base(MaterialType::Generic, |m| {
            m.base_color = colour;
            m.base_diffuse_roughness = 0.5;
            m.specular_roughness = 0.38;
            m.fuzz_weight = 0.15;
            m.fuzz_color = rgb(1.0, 1.0, 1.0);
            m.fuzz_roughness = 0.5;
        }));
        // The pebbles: packed edge to edge, each a low dome, apart only by
        // the narrow grooves between them. Two Voronoi nodes of one scale
        // see the same cells: one gives the grooves, the other the domes.
        // Seen through a gentle warp, so their outlines wander as a hide's
        // do instead of running straight from corner to corner.
        let coords = r.node("texture_coordinate").out("Object");
        let wander = r.node("noise_texture").w("Vector", coords).f("Scale", 900.0).f("Detail", 2.0).out("Color");
        let warped = r.node("vector_math").set("operation", "multiply_add").w("Vector", wander).v("Vector 2", [0.0009, 0.0009, 0.0009]).w("Vector 3", coords).out("Vector");
        let edge = r.node("voronoi_texture").set("feature", "distance_to_edge").w("Vector", warped).f("Scale", 720.0).f("Randomness", 1.0).out("Distance");
        let groove = r.smooth(edge, (0.0, 0.16), (0.0, 1.0));
        let centre = r.node("voronoi_texture").w("Vector", warped).f("Scale", 720.0).f("Randomness", 1.0).out("Distance");
        let dome = r.smooth(centre, (0.0, 0.75), (1.0, 0.45));
        let pebble = r.math("multiply", groove, dome);
        // And the finer grain the pebbles are made of.
        let fine = r.node("voronoi_texture").set("feature", "smooth_f1").f("Scale", 2600.0).f("Smoothness", 0.5).f("Randomness", 1.0).out("Distance");
        let pebble = plus(&mut r, fine, -0.18, pebble);
        // Wrinkles: thin lines where a slow noise crosses its middle.
        let field = r.node("noise_texture").f("Scale", 70.0).f("Detail", 4.0).f("Roughness", 0.55).f("Distortion", 0.4).out("Fac");
        let off = r.math("subtract", field, 0.5);
        let apart = r.math("absolute", off, 0.0);
        let wrinkle = r.smooth(apart, (0.0, 0.03), (-1.0, 0.0));
        let height = plus(&mut r, wrinkle, 0.6, pebble);
        r.bump(height, 1.0e-4);
        // Tops a little polished by use; grooves duller.
        let gloss = r.range(pebble, (0.0, 1.0), (0.55, 0.34));
        r.to(Param::SpecularRoughness, gloss);
        // Darker in the grooves and the wrinkles, and a hide's uneven dye.
        let shade = r.range(pebble, (0.0, 1.0), (0.84, 1.0));
        let dye = noise(&mut r, 40.0, 3.0, 0.5);
        let mottle = r.range(dye, (0.3, 0.7), (0.88, 1.0));
        let both = r.math("multiply", shade, mottle);
        let toned = r.node("mix").set("blend_type", "multiply").f("Factor", 1.0).c("A", colour).w("B", both).out("Result");
        r.to(Param::BaseColor, toned);
        out.push(preset(name, "Leather", tags, MaterialType::Generic, r));
    }

    // ---- paper, cork, composites ----------------------------------------------
    let mut r = Recipe::new(base(MaterialType::Diffuse, |m| {
        m.base_color = rgb(0.8966, 0.8887, 0.8596);
        m.base_diffuse_roughness = 0.35;
    }));
    let fibres = r.node("noise_texture").f("Scale", 2600.0).f("Detail", 6.0).f("Roughness", 0.6).f("Distortion", 0.6).out("Fac");
    r.bump(fibres, 6e-6);
    let mottle = noise(&mut r, 120.0, 4.0, 0.5);
    let colour = r.ramp(mottle, Linear, &[(0.0, srgb("#E8E4DA")), (1.0, srgb("#F4F1EA"))]);
    r.to(Param::BaseColor, colour);
    out.push(preset("Paper Matte", "Other", &["paper", "print", "packaging"], MaterialType::Diffuse, r));

    // Cork, measured against scanned boards (ambientCG Cork001, Cork003): the
    // crumbs differ from one another by only a few percent of lightness, all
    // one warm hue; each is a scrap of bark with its own fibre running its
    // own way; they press together with soft seams rather than outlines, and
    // only here and there leave a dark pit. The flat, outlined, two-tone
    // tiles this replaced read as a mosaic, not as cork.
    out.push(cork(
        "Cork",
        &["cork", "natural", "granulated", "board"],
        380.0,
        1.0,
        &[(0.0, srgb("#AE735B")), (0.25, srgb("#BA7B62")), (0.5, srgb("#C58066")), (0.75, srgb("#CA8A6F")), (1.0, srgb("#D29A7D"))],
        srgb("#7A4836"),
        0.0,
    ));
    // Agglomerated cork: flakes several times larger, a deeper honey, with the
    // dark dashes of the bark's lenticels through them.
    out.push(cork(
        "Cork Coarse",
        &["cork", "natural", "agglomerated", "wall", "honey"],
        150.0,
        0.55,
        &[(0.0, srgb("#A8662F")), (0.25, srgb("#BB7B43")), (0.5, srgb("#C5874C")), (0.75, srgb("#D08E52")), (1.0, srgb("#D99E5F"))],
        srgb("#6A3A1A"),
        1.0,
    ));

    out
}
