//! The lamps on the card against `tequila_render_core::light`: each kind's
//! light sample, what a ray meets of it and the density a light sample
//! gives a direction to it (`lights.wgsl`), and choosing one from a point,
//! by the alias table or down the light hierarchy, with the chance of that
//! choice, and its light links (`lamps.wgsl`), on random lamps, points and
//! directions.
//!
//! The tolerances are `tests/lobes.rs`'s: values to 1e-4 relative (1e-6
//! absolute); what goes through the card's trigonometry - a sampled
//! direction to 2e-4 absolute, a sample's radiance and density to 2e-3
//! relative; one item in a thousand may be looser, to 5% (1e-5 absolute),
//! or fall on the other side of a threshold (a grazing cosine, a ray that
//! only just meets a panel's edge, a draw on the boundary between two lamps).

use tequila_core::math::{vec2, vec3, Vec3};
use tequila_core::rng::Pcg32;
use tequila_core::spectrum::Spectrum;
use tequila_render_core::light::{
    AreaLight, DistantLight, Light, LightKind, LightSet, PointLight, SelectionStrategy, SpotLight, Visibility,
};
use tequila_render_gpu::harness::Harness;
use tequila_render_gpu::lights::{light_record, GpuLight, LightRecords};

fn bits(values: &[f32]) -> Vec<u32> {
    values.iter().map(|v| v.to_bits()).collect()
}

fn unit_vector(rng: &mut Pcg32) -> Vec3 {
    let z = 1.0 - 2.0 * rng.next_f32();
    let r = (1.0 - z * z).max(0.0).sqrt();
    let phi = std::f32::consts::TAU * rng.next_f32();
    vec3(r * phi.cos(), r * phi.sin(), z)
}

fn point(rng: &mut Pcg32, extent: f32) -> Vec3 {
    vec3((rng.next_f32() * 2.0 - 1.0) * extent, (rng.next_f32() * 2.0 - 1.0) * extent, (rng.next_f32() * 2.0 - 1.0) * extent)
}

fn colour(rng: &mut Pcg32) -> Spectrum {
    Spectrum { r: rng.next_f32() * 50.0, g: rng.next_f32() * 50.0, b: rng.next_f32() * 50.0 }
}

/// A lamp of any kind, every parameter over its range.
fn lamp(rng: &mut Pcg32) -> Light {
    let kind = match rng.next_f32() {
        x if x < 0.5 => {
            // Two perpendicular extents, as a placed panel has.
            let a = unit_vector(rng);
            let helper = if a.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
            let b = a.cross(helper).normalize();
            LightKind::Area(AreaLight {
                position: point(rng, 2.0),
                half_u: a * (0.05 + rng.next_f32()),
                half_v: b * (0.05 + rng.next_f32()),
                radiance: colour(rng),
                disc: rng.next_f32() < 0.3,
                two_sided: rng.next_f32() < 0.3,
                spread: if rng.next_f32() < 0.5 {
                    std::f32::consts::PI
                } else {
                    (1.0 + rng.next_f32() * 179.0).to_radians()
                },
                falloff: if rng.next_f32() < 0.5 { 0.0 } else { rng.next_f32() },
                fade: if rng.next_f32() < 0.7 { 0.0 } else { rng.next_f32() * 2.0 - 1.0 },
            })
        }
        x if x < 0.65 => LightKind::Point(PointLight {
            position: point(rng, 2.0),
            intensity: colour(rng),
            radius: if rng.next_f32() < 0.5 { 0.0 } else { 0.01 + rng.next_f32() * 0.3 },
        }),
        x if x < 0.85 => {
            let outer = (5.0 + rng.next_f32() * 75.0).to_radians();
            LightKind::Spot(SpotLight {
                position: point(rng, 2.0),
                direction: unit_vector(rng),
                intensity: colour(rng),
                inner_angle: outer * rng.next_f32(),
                outer_angle: outer,
                falloff: match rng.next_f32() {
                    x if x < 0.5 => 1.0,
                    x if x < 0.6 => 0.0,
                    _ => 0.2 + rng.next_f32() * 4.0,
                },
            })
        }
        _ => LightKind::Distant(DistantLight {
            direction: unit_vector(rng),
            irradiance: colour(rng),
            half_angle: if rng.next_f32() < 0.3 { 0.0 } else { 0.001 + rng.next_f32() * 0.2 },
        }),
    };
    let mut light = Light::new(kind);
    light.enabled = rng.next_f32() < 0.95;
    light.visibility = Visibility {
        camera: rng.next_f32() < 0.5,
        diffuse: rng.next_f32() < 0.9,
        specular: rng.next_f32() < 0.9,
        shadow: rng.next_f32() < 0.9,
    };
    light
}

const LIGHT_WORDS: usize = std::mem::size_of::<GpuLight>() / 4;

/// `lights.wgsl`'s GpuLight from the first words of the item.
fn light_from_words() -> String {
    let f = |k: usize| format!("inf({k}u)");
    let u = |k: usize| format!("inp({k}u)");
    let mut parts: Vec<String> = (0..9).map(|c| format!("vec4<f32>({}, {}, {}, {})", f(4 * c), f(4 * c + 1), f(4 * c + 2), f(4 * c + 3))).collect();
    parts.push(u(36));
    parts.push(u(37));
    parts.extend((38..46).map(f));
    parts.push(f(46));
    parts.push(u(47));
    parts.push(u(48));
    parts.push(u(49));
    parts.push(u(50));
    parts.push(u(51));
    format!("GpuLight({})", parts.join(", "))
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Value,
    Flag,
    Direction,
    Sampled,
}

/// Every word within its bound, but at most one item in a thousand, which
/// may be looser or flip a flag. Words after a flag word that is off on
/// either side (`guarded_by`) are not compared, nor words the processor's
/// side marks as not to be (a NaN).
fn compare(name: &str, card: &[u32], want: &[Vec<f32>], words: &[(Kind, Option<usize>)], cases: &[String]) {
    let n = words.len();
    let mut wrong = Vec::new();
    let mut loose = Vec::new();
    for (i, w) in want.iter().enumerate() {
        let c: Vec<f32> = card[i * n..(i + 1) * n].iter().map(|b| f32::from_bits(*b)).collect();
        let mut item_loose = false;
        for (k, (kind, guard)) in words.iter().enumerate() {
            if let Some(g) = guard {
                if c[*g] == 0.0 || w[*g] == 0.0 {
                    continue;
                }
            }
            let (absolute, relative) = match kind {
                Kind::Value => (1e-6, 1e-4),
                Kind::Flag => (0.0, 0.0),
                Kind::Direction => (2e-4, 0.0),
                Kind::Sampled => (1e-6, 2e-3),
            };
            let d = (c[k] - w[k]).abs();
            if d <= absolute + relative * w[k].abs() || w[k].is_nan() {
                continue;
            }
            if *kind == Kind::Flag || d <= 1e-5 + 5e-2 * w[k].abs() {
                item_loose = true;
            } else if wrong.len() < 8 {
                wrong.push(format!("  #{i} word {k}: card {}, processor {} ({})", c[k], w[k], cases[i]));
            }
        }
        if item_loose {
            loose.push(i);
        }
    }
    assert!(wrong.is_empty(), "{name} differs:\n{}", wrong.join("\n"));
    assert!(
        loose.len() * 1000 <= want.len(),
        "{name}: {} items of {} outside the tight bound (first {:?}: {}); at most one in a thousand may be",
        loose.len(),
        want.len(),
        &loose[..loose.len().min(5)],
        loose.first().map_or(String::new(), |&i| cases[i].clone())
    );
    println!("{name}: {} of {} items looser than the tight bound", loose.len(), want.len());
}

const N: usize = 20_000;

/// Each kind's sample, meeting, density and face against the processor's.
#[test]
fn each_lamp_is_the_processors() {
    let Some(h) = Harness::for_test() else { return };
    let mut rng = Pcg32::new(41, 1);
    let (mut inputs, mut want, mut cases) = (Vec::new(), Vec::new(), Vec::new());
    let (mut sampled, mut met, mut fallback, mut ill) = (0, 0, 0, 0);
    for _ in 0..N {
        let light = lamp(&mut rng);
        // Now and then just off an area lamp's plane, where the spherical
        // rectangle gives way to sampling by area; now and then a ray aimed
        // at the lamp, so it is met.
        let from = match (&light.kind, rng.next_f32()) {
            (LightKind::Area(a), x) if x < 0.15 => a.position + a.half_u * (rng.next_f32() * 3.0 - 1.5) + a.normal() * ((rng.next_f32() - 0.5) * 2e-5),
            _ => point(&mut rng, 3.0),
        };
        let u = vec2(rng.next_f32(), rng.next_f32());
        let direction = match (&light.kind, rng.next_f32()) {
            (LightKind::Area(a), x) if x < 0.5 => {
                let target = a.position + a.half_u * (rng.next_f32() * 2.4 - 1.2) + a.half_v * (rng.next_f32() * 2.4 - 1.2);
                (target - from).try_normalize().unwrap_or(Vec3::Y)
            }
            (LightKind::Distant(d), x) if x < 0.5 => {
                let jitter = unit_vector(&mut rng) * (d.half_angle * 1.3);
                (-d.direction + jitter).normalize()
            }
            _ => unit_vector(&mut rng),
        };
        let face = (rng.next_f32() * 2.6 - 1.3, rng.next_f32() * 2.6 - 1.3);
        let record = light_record(&light);
        inputs.extend_from_slice(bytemuck::cast_slice::<GpuLight, u32>(std::slice::from_ref(&record)));
        inputs.extend(bits(&[from.x, from.y, from.z, u.x, u.y, direction.x, direction.y, direction.z, face.0, face.1]));

        let mut e = Vec::with_capacity(19);
        match light.sample(from, u) {
            Some(s) => {
                sampled += 1;
                let infinite = s.distance.is_infinite();
                e.extend([1.0, s.direction.x, s.direction.y, s.direction.z, s.radiance.r, s.radiance.g, s.radiance.b]);
                e.extend([if infinite { 0.0 } else { s.distance }, infinite as u8 as f32, s.pdf, s.is_delta as u8 as f32]);
            }
            None => e.extend([0.0; 11]),
        }
        match light.met_along(from, direction) {
            Some((radiance, t)) => {
                met += 1;
                let infinite = t.is_infinite();
                e.extend([1.0, radiance.r, radiance.g, radiance.b, if infinite { 0.0 } else { t }, infinite as u8 as f32]);
            }
            None => e.extend([0.0; 6]),
        }
        let density = light.density_along(from, direction);
        // Ill-conditioned, on both sides: a panel subtending under a
        // thousandth of a steradian (its solid angle the small difference of
        // four arc cosines and 2 pi), or one met or sampled at a grazing
        // angle, where a direction's last bits move the point on it far.
        // The processor's own f32 differs from its f64 by percents there.
        let grazing = |d: Vec3| match &light.kind {
            LightKind::Area(a) => d.dot(a.normal()).abs() < 0.02,
            _ => false,
        };
        if e[0] != 0.0 && (e[9] > 1e3 || grazing(vec3(e[1], e[2], e[3]))) {
            ill += 1;
            for word in &mut e[1..10] {
                *word = f32::NAN;
            }
        }
        if e[11] != 0.0 && grazing(direction) {
            for word in &mut e[12..16] {
                *word = f32::NAN;
            }
        }
        e.push(if density > 1e3 || grazing(direction) { f32::NAN } else { density });
        e.push(match &light.kind {
            LightKind::Area(a) => {
                if !a.disc && light.sample(from, u).is_some() && tequila_core::math::Vec3::dot(a.position - from, a.normal()).abs() < 1e-4 {
                    fallback += 1;
                }
                a.face(face.0, face.1)
            }
            _ => 0.0,
        });
        want.push(e);
        cases.push(format!("light {light:?} from {from:?} u {u:?} direction {direction:?} face {face:?}"));
    }
    let base = LIGHT_WORDS;
    let body = format!(
        "let l = {light};
         let at = vec3<f32>(inf({}u), inf({}u), inf({}u));
         let u = vec2<f32>(inf({}u), inf({}u));
         let direction = vec3<f32>(inf({}u), inf({}u), inf({}u));
         let s = light_sample(l, at, u);
         if (s.ok) {{
             outf(0u, 1.0);
             outf(1u, s.direction.x); outf(2u, s.direction.y); outf(3u, s.direction.z);
             outf(4u, s.radiance.x); outf(5u, s.radiance.y); outf(6u, s.radiance.z);
             outf(7u, select(s.distance, 0.0, s.infinite)); outf(8u, select(0.0, 1.0, s.infinite));
             outf(9u, s.pdf); outf(10u, select(0.0, 1.0, s.is_delta));
         }} else {{
             for (var k = 0u; k < 11u; k++) {{ outf(k, 0.0); }}
         }}
         let m = light_met_along(l, at, direction);
         if (m.ok) {{
             outf(11u, 1.0);
             outf(12u, m.radiance.x); outf(13u, m.radiance.y); outf(14u, m.radiance.z);
             outf(15u, select(m.t, 0.0, m.infinite)); outf(16u, select(0.0, 1.0, m.infinite));
         }} else {{
             for (var k = 11u; k < 17u; k++) {{ outf(k, 0.0); }}
         }}
         outf(17u, light_density_along(l, at, direction));
         outf(18u, select(0.0, light_face(l, inf({}u), inf({}u)), l.kind == LIGHT_AREA));",
        base,
        base + 1,
        base + 2,
        base + 3,
        base + 4,
        base + 5,
        base + 6,
        base + 7,
        base + 8,
        base + 9,
        light = light_from_words(),
    );
    let out = h.run_wgsl(&["lights"], &body, &inputs, base + 10, 19);
    use Kind::*;
    let s = Some(0);
    let m = Some(11);
    let words = [
        (Flag, None),
        (Direction, s),
        (Direction, s),
        (Direction, s),
        (Sampled, s),
        (Sampled, s),
        (Sampled, s),
        (Sampled, s),
        (Flag, s),
        (Sampled, s),
        (Flag, s),
        (Flag, None),
        (Value, m),
        (Value, m),
        (Value, m),
        (Value, m),
        (Flag, m),
        (Sampled, None),
        (Value, None),
    ];
    println!("{sampled} lamps sampled ({ill} ill-conditioned), {met} met, {fallback} sampled by area just off a rectangle's plane");
    compare("Light", &out, &want, &words, &cases);
}

/// A scene's worth of lamps: a few to a dozen, of every kind, now and then a
/// weak one beside the rest, chosen as the count and their powers say, or
/// by the strategy the test asks for.
fn lamp_set(rng: &mut Pcg32, strategy: Option<SelectionStrategy>) -> LightSet {
    let count = 1 + (rng.next_f32() * 12.0) as usize;
    let lights: Vec<Light> = (0..count)
        .map(|_| {
            let mut light = lamp(rng);
            light.enabled = true;
            if rng.next_f32() < 0.2 {
                // A weak accent.
                match &mut light.kind {
                    LightKind::Area(a) => a.radiance = a.radiance * 0.01,
                    LightKind::Point(p) => p.intensity = p.intensity * 0.01,
                    LightKind::Spot(s) => s.intensity = s.intensity * 0.01,
                    LightKind::Distant(d) => d.irradiance = d.irradiance * 0.01,
                    LightKind::Ies(_) => {}
                }
            }
            light
        })
        .collect();
    match strategy {
        Some(strategy) => LightSet::with_strategy(lights, strategy),
        None => LightSet::new(lights),
    }
}

/// Choosing a lamp from a point, and the chance of choosing each, against
/// `LightSet::select_at` and `::selection_pdf_at`.
#[test]
fn choosing_a_lamp_is_the_processors() {
    let Some(h) = Harness::for_test() else { return };
    let mut rng = Pcg32::new(43, 1);
    const SETS: usize = 24;
    const QUERIES: usize = 1500;
    let (mut hierarchies, mut chosen, mut differ) = (0, 0, 0);
    let mut total = 0;
    for set_index in 0..SETS {
        let strategy = match set_index % 3 {
            0 => None,
            1 => Some(SelectionStrategy::Hierarchy),
            _ => Some(SelectionStrategy::Alias),
        };
        let set = lamp_set(&mut rng, strategy);
        let records = LightRecords::of_set(&set);
        hierarchies += records.hierarchy as usize;
        let lights: &[u32] = bytemuck::cast_slice(&records.lights);
        let nodes_owned = if records.nodes.is_empty() { vec![Default::default()] } else { records.nodes.clone() };
        let nodes: &[u32] = bytemuck::cast_slice(&nodes_owned);
        let count = set.len();
        let (mut inputs, mut want, mut cases) = (Vec::new(), Vec::new(), Vec::new());
        for _ in 0..QUERIES {
            let at = point(&mut rng, 3.0);
            let normal = unit_vector(&mut rng);
            let two_sided = rng.next_f32() < 0.3;
            let u = rng.next_f32();
            inputs.extend(bits(&[at.x, at.y, at.z, normal.x, normal.y, normal.z, u]));
            inputs.push(two_sided as u32);
            let mut e = match set.select_at(at, normal, two_sided, u) {
                Some((id, p)) => vec![1.0, id.0 as f32, p],
                None => vec![0.0, 0.0, 0.0],
            };
            for id in 0..12 {
                e.push(if id < count { set.selection_pdf_at(at, normal, two_sided, tequila_core::ids::LightId(id as u32)) } else { 0.0 });
            }
            want.push(e);
            cases.push(format!("set {set_index} ({count} lamps, {:?}) at {at:?} normal {normal:?} two_sided {two_sided} u {u}", set.strategy()));
        }
        let body = format!(
            "let at = vec3<f32>(inf(0u), inf(1u), inf(2u));
             let normal = vec3<f32>(inf(3u), inf(4u), inf(5u));
             let u = inf(6u);
             let two_sided = inp(7u) != 0u;
             let c = lamps_select_at({count}u, {hierarchy}, at, normal, two_sided, u);
             outf(0u, select(0.0, 1.0, c.ok));
             outf(1u, select(0.0, f32(c.index), c.ok));
             outf(2u, select(0.0, c.probability, c.ok));
             for (var id = 0u; id < 12u; id++) {{
                 var p = 0.0;
                 if (id < {count}u) {{ p = lamps_selection_pdf_at({hierarchy}, at, normal, two_sided, id); }}
                 outf(3u + id, p);
             }}",
            hierarchy = records.hierarchy,
        );
        let out = h.run_wgsl_with(&["lamps"], &body, &inputs, 8, 15, &[(8, lights), (9, nodes), (10, &[0u32])]);
        // The lamp chosen: where the two sides choose apart, a draw on the
        // boundary between two lamps, both chances must still agree.
        let mut words = vec![(Kind::Flag, None), (Kind::Flag, Some(0)), (Kind::Value, Some(0))];
        words.extend((0..12).map(|_| (Kind::Value, None)));
        for (i, w) in want.iter().enumerate() {
            let c = f32::from_bits(out[i * 15 + 1]);
            if w[0] != 0.0 {
                chosen += 1;
            }
            if w[0] != 0.0 && c != w[1] {
                differ += 1;
            }
        }
        compare("LightSet::select_at", &out, &want, &words, &cases);
        total += QUERIES;
    }
    println!("{SETS} sets, {hierarchies} by the hierarchy; {chosen} of {total} draws chose a lamp, {differ} another lamp than the processor");
    assert!(hierarchies >= SETS / 3, "the hierarchy is tested");
}


/// Light linking on the card (`lamp_lights`) against `LightLinks::lights`:
/// a lamp kept to some instances, or kept from them, asked about instances
/// in its list and out of it.
#[test]
fn a_lamps_links_are_the_processors() {
    use tequila_render_core::light::LightLinks;
    let Some(h) = Harness::for_test() else { return };
    let mut rng = Pcg32::new(47, 1);
    let mut lights = Vec::new();
    for _ in 0..12 {
        let mut light = lamp(&mut rng);
        let mut these: Vec<u32> = (0..(rng.next_f32() * 9.0) as usize).map(|_| (rng.next_f32() * 40.0) as u32).collect();
        these.sort_unstable();
        these.dedup();
        light.links = match rng.next_f32() {
            x if x < 0.2 => LightLinks::All,
            x if x < 0.6 => LightLinks::Only(these.into()),
            _ => LightLinks::AllBut(these.into()),
        };
        lights.push(light);
    }
    let set = LightSet::with_strategy(lights, SelectionStrategy::Alias);
    let records = LightRecords::of_set(&set);
    let (mut inputs, mut want) = (Vec::new(), Vec::new());
    for (i, light) in set.lights().iter().enumerate() {
        for instance in 0..48u32 {
            inputs.extend([i as u32, instance]);
            want.push(light.links.lights(instance) as u32);
        }
    }
    let links = if records.links.is_empty() { vec![0] } else { records.links.clone() };
    let out = h.run_wgsl_with(
        &["lamps"],
        "out(0u, select(0u, 1u, lamp_lights(scene_lights[inp(0u)], inp(1u))));",
        &inputs,
        2,
        1,
        &[(8, bytemuck::cast_slice(&records.lights)), (9, &[0u32; 16]), (10, &links)],
    );
    assert_eq!(out, want, "the card's light links differ");
    println!("{} lamps' links asked about {} instances each: all the processor's", set.len(), 48);
}
