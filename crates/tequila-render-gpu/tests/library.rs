//! The whole standard library against the card (`docs/07` R3's exit: every
//! preset, card against processor): how many of its presets the card
//! renders whole - its lowering takes the program (its pictures none
//! loaded, so an image texture takes its fallback), its stack takes the
//! surface, and nothing the preset asks for is in a `*_limits` refusal - and
//! why it refuses the rest. Prints the tally; asserts only that rounding
//! edges took nothing away.

use tequila_materials::openpbr::Material;
use tequila_render_gpu::program::{self, Refusal};
use tequila_render_gpu::textures::{CardPictures, Pictures};

/// Why the card does not render `m`, a preset's surface as its program's
/// constants have it, as the processor does; `rounding` whether rounded
/// edges count among them (they did until the card took Bevel).
fn surface_refusals(m: &Material, rounding: bool) -> Vec<&'static str> {
    let glass = m.transmission_weight > 0.0;
    let mut why = Vec::new();
    if m.subsurface_weight != 0.0 {
        why.push("subsurface");
    }
    if m.thin_film_weight > 1e-4 {
        why.push("thin film");
    }
    if glass && m.volume_scattering().is_some() {
        why.push("a volume that scatters");
    }
    if glass && m.transmission_dispersion_abbe > 0.0 {
        why.push("dispersion");
    }
    if m.fuzz_density > 0.0 {
        why.push("fibres");
    }
    if rounding && m.edge_radius > 0.0 {
        why.push("rounded edges");
    }
    why
}

#[test]
fn the_whole_librarys_tally() {
    let (entries, problems) = tequila_format::library::read_directory(&tequila_format::library::standard_directory());
    assert!(!entries.is_empty(), "no presets: {problems:?}");
    let mut pictures = CardPictures::default();
    pictures.finish();
    let mut first: std::collections::BTreeMap<String, usize> = Default::default();
    let mut any: std::collections::BTreeMap<String, usize> = Default::default();
    let (mut whole_before, mut whole_now, mut rounding) = (0usize, 0usize, 0usize);
    for entry in &entries {
        let program = match entry.graph.compile() {
            Ok(program) => program,
            Err(_) => {
                *first.entry("does not compile".into()).or_default() += 1;
                continue;
            }
        };
        let m = &program.constants;
        if m.edge_radius > 0.0 {
            rounding += 1;
        }
        let mut why: Vec<String> = Vec::new();
        if let Err(refusal) = program::lower_with(&program, Pictures::On(&pictures)) {
            why.push(match refusal {
                Refusal::Node(n) => format!("the {n} node"),
                Refusal::Param(p) => format!("driving {}", p.label()),
                other => other.to_string(),
            });
        }
        let now: Vec<String> = surface_refusals(m, false).into_iter().map(String::from).collect();
        let before: Vec<String> = surface_refusals(m, true).into_iter().map(String::from).collect();
        if why.is_empty() && before.is_empty() {
            whole_before += 1;
        }
        why.extend(now);
        if why.is_empty() {
            whole_now += 1;
        } else {
            *first.entry(why[0].clone()).or_default() += 1;
            for reason in &why {
                *any.entry(reason.clone()).or_default() += 1;
            }
        }
    }
    eprintln!(
        "the whole library: {} presets, {rounding} of them rounding their edges; the card renders {whole_now} whole ({whole_before} before it rounded edges)",
        entries.len()
    );
    for (why, count) in &any {
        eprintln!("  refused, {why}: {count} ({} the first reason)", first.get(why).copied().unwrap_or(0));
    }
    if let Some(count) = first.get("does not compile") {
        eprintln!("  {count} do not compile");
    }
    assert!(whole_now >= whole_before, "rounding edges took presets off the card");
}
