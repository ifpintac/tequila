//! Faux fur: a long, soft pile matted into locks a couple of millimetres
//! wide, combed one way in patches, with the backing showing dark in the
//! gaps between the locks.
//!
//! It began as a roughout suede with real fibres. Grown fibres made a
//! hairy nap read as fur on a shoe, so the suede is drawn in the graph
//! instead (`suede.rs`), and the grown look is kept here as fur.

use crate::recipe::Out;
use crate::suede::{curvature, noise, occlusion, product, tint};
use crate::{base, preset, srgb, Preset, Recipe};
use tequila_materials::material_type::{MaterialType, Param};

pub fn all() -> Vec<Preset> {
    vec![camel()]
}

/// Camel faux fur, shaggy and soft: fibres 2.4 mm long and 50 microns at
/// the root (bundles, as the eye sees them), laid nearly flat, gathered
/// into locks and combed one way. Under them the backing is a darker brown,
/// blotchy where it took the dye unevenly and darker where it is pressed
/// down or rubbed on an edge.
fn camel() -> Preset {
    let backing = srgb("#573722");
    let sheen = srgb("#7A583A");
    let mut r = Recipe::new(base(MaterialType::Velvet, |m| {
        m.base_color = backing;
        m.fuzz_color = sheen;
        m.fuzz_weight = 1.0;
        m.fuzz_roughness = 0.6;
        // About a thousand fibres a square centimetre: as dense as the
        // fur first looked on a 240 cm² swatch, where it met the fibre cap.
        m.fuzz_density = 1000.0;
        m.fuzz_length = 0.0024;
        m.fuzz_thickness = 0.00005;
        m.fuzz_lean = 0.95;
        m.fuzz_curl = 0.1;
        m.fuzz_clump = 0.7;
        m.fuzz_comb = 0.85;
        m.fuzz_color_variation = 0.5;
    }));
    let coords = r.node("texture_coordinate").out("Object");
    // The dye's unevenness, a hand's width and a thumb's.
    let blotch_field = noise(&mut r, coords, 8.0, 4.0, 0.6, 0.8);
    let blotch = r.smooth(blotch_field, (0.35, 0.65), (0.75, 1.12));
    let mottle_field = noise(&mut r, coords, 60.0, 5.0, 0.65, 0.0);
    let mottle = r.smooth(mottle_field, (0.3, 0.7), (0.88, 1.08));
    // Matted clumps, each its own shade.
    let clumps = r.node("voronoi_texture").w("Vector", coords).f("Scale", 650.0).add();
    let pick = r.node("separate_color").w("Color", Out(clumps, 1)).out("Red");
    let clump_tone = r.smooth(pick, (0.0, 1.0), (0.9, 1.08));
    // Fibres running every which way inside them.
    let fibres = r.node("gabor_texture").set("type", "3D").w("Vector", coords).f("Scale", 1300.0).f("Frequency", 3.0).f("Anisotropy", 0.75).out("Value");
    let fibre_tone = r.smooth(fibres, (0.3, 0.7), (0.9, 1.07));
    // Pressed-down patches: darker, smoother.
    let press_field = noise(&mut r, coords, 5.0, 3.0, 0.5, 0.0);
    let press = r.smooth(press_field, (0.58, 0.74), (0.0, 1.0));
    let pressed = r.smooth(press, (0.0, 1.0), (1.0, 0.8));
    // Rubbed edges darker; dirt and shadow down in the creases.
    let curv = curvature(&mut r, 0.004, 1.2);
    let burnish = r.smooth(curv, (0.35, 0.9), (1.0, 0.72));
    let occ = occlusion(&mut r, 0.012, 1.0);
    let crease = r.smooth(occ, (0.0, 1.0), (0.8, 1.0));

    let shade = product(&mut r, &[blotch, mottle, clump_tone, fibre_tone, pressed, burnish, crease]);
    let colour = tint(&mut r, backing, shade);
    r.to(Param::BaseColor, colour);
    let glow = product(&mut r, &[blotch, clump_tone, pressed, burnish, crease]);
    let glow = tint(&mut r, sheen, glow);
    r.to(Param::FuzzColor, glow);
    // The sheen tightens where the pile is pressed or rubbed flat.
    let held = r.smooth(press, (0.0, 1.0), (0.0, -0.18));
    let rubbed = r.smooth(curv, (0.35, 0.9), (0.0, -0.28));
    let lie = r.math("add", held, rubbed);
    let lie = r.math("add", lie, 0.8);
    r.to(Param::FuzzRoughness, lie);
    // Height: clumps as domes, the fibres and a coarse grain on top.
    let domes = r.smooth(Out(clumps, 0), (0.0, 0.6), (1.0, 0.0));
    let coarse = noise(&mut r, coords, 2600.0, 4.0, 0.7, 0.0);
    let coarse_part = r.math("multiply", coarse, 0.2);
    let fibre_part = r.math3("multiply_add", fibres, 0.25, coarse_part);
    let height = r.math3("multiply_add", domes, 0.55, fibre_part);
    r.bump(height, 8e-5);
    preset(
        "Faux Fur Camel",
        "Fabric",
        &["fur", "faux fur", "fake fur", "shaggy", "plush", "pile", "teddy", "fluffy", "soft", "cushion", "throw", "rug", "brown", "camel", "tan"],
        MaterialType::Velvet,
        r,
    )
}
