//! How long the transport kernel takes to compile with nothing cached
//! (`docs/07` §7: one generic interpreter, one call site, so the kernel does
//! not grow with every material). The driver keeps what it compiled, so the
//! kernel is salted to be a new one each time; the time is printed, and held
//! under a generous ceiling that a kernel which had doubled would break.

use tequila_render_gpu::device::{self, Needs};
use tequila_render_gpu::transport::{cold_compile_time, cold_compile_time_edited, cold_compile_time_edited_of, cold_compile_time_of, compile_time_of, GpuRenderer};
use tequila_render_gpu::variants::KernelFeatures;
use tequila_scene::parity;
use tequila_scene::snapshot::{snapshot, MeshCache};

#[test]
fn the_transport_kernel_compiles_in_reasonable_time() {
    let Some(gpu) = device::for_test(Needs::RayQueries) else { return };
    let scene = parity::scene("grey_product").expect("the parity set has it");
    let document = scene.document();
    let mut environments = tequila_scene::environment::EnvironmentCache::new();
    let environment = environments.environment(&document.environment);
    let (snapshot, _) = snapshot(&document, environment, &mut MeshCache::new());
    let renderer = GpuRenderer::new(gpu, &snapshot);
    let salt = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos() ^ d.as_secs() as u32);
    let times: Vec<f64> = (0..2).map(|i| cold_compile_time(&renderer, salt.wrapping_add(i)).as_secs_f64()).collect();
    eprintln!("the transport kernel compiles cold in {:.2} s and {:.2} s", times[0], times[1]);
    assert!(times.iter().all(|t| *t < 60.0), "the transport kernel takes {times:?} s to compile");
}

/// What each part of the material programs costs the kernel's cold compile,
/// each measured by leaving it out (its arm emptied). Run by hand:
/// `cargo test --test compile_time -- --ignored --nocapture`.
#[test]
#[ignore]
fn what_the_programs_cost_to_compile() {
    let Some(gpu) = device::for_test(Needs::RayQueries) else { return };
    let scene = parity::scene("grey_product").expect("the parity set has it");
    let document = scene.document();
    let mut environments = tequila_scene::environment::EnvironmentCache::new();
    let environment = environments.environment(&document.environment);
    let (snapshot, _) = snapshot(&document, environment, &mut MeshCache::new());
    let renderer = GpuRenderer::new(gpu, &snapshot);
    let salt = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos() ^ d.as_secs() as u32);
    let empty_arm = |source: String, arm: &str| {
        let at = source.find(arm).unwrap_or_else(|| panic!("no {arm:?}"));
        let open = at + source[at..].find('{').unwrap();
        let mut depth = 0;
        let mut close = open;
        for (i, c) in source[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = open + i;
                        break;
                    }
                }
                _ => {}
            }
        }
        format!("{}{{}}{}", &source[..open], &source[close + 1..])
    };
    let cases: Vec<(&str, Box<dyn Fn(String) -> String>)> = vec![
        ("everything", Box::new(|s| s)),
        ("no interpreter", Box::new(|s: String| s.replace("        prog_run(start, end, at);", ""))),
        ("no noise", Box::new(move |s| empty_arm(s, "            case 6u: {\n                out = cy_noise_node"))),
        ("no voronoi", Box::new(move |s| empty_arm(s, "            case 7u: {\n                let cells"))),
        ("no math", Box::new(move |s| empty_arm(s, "            case 2u: {\n                var v = cy_math"))),
        ("no vector math", Box::new(move |s| s.replace("out = cy_vector_math(settings, a[0].xyz, a[1].xyz, a[2].xyz, a[3].x);", ""))),
        ("no mix", Box::new(move |s| s.replace("out = cy_mix_node(settings, a[0].x, a[1], a[2]);", ""))),
        ("no anisotropic frame", Box::new(move |s| s.replace("f = anisotropic_frame(hit, surf, shaded.normal, shaded.drawn, material.specular_rotation);", "f = frame_of(shaded.normal);"))),
    ];
    for (i, (name, edit)) in cases.iter().enumerate() {
        let t = cold_compile_time_edited(&renderer, salt.wrapping_add(i as u32 * 7919), edit.as_ref());
        eprintln!("{name}: {:.2} s", t.as_secs_f64());
    }
}

/// What each part of glass costs the kernel's cold compile, each measured
/// by leaving it out. Run by hand:
/// `cargo test --test compile_time -- --ignored --nocapture glass`.
#[test]
#[ignore]
fn what_glass_costs_to_compile() {
    let Some(gpu) = device::for_test(Needs::RayQueries) else { return };
    let scene = parity::scene("grey_product").expect("the parity set has it");
    let document = scene.document();
    let mut environments = tequila_scene::environment::EnvironmentCache::new();
    let environment = environments.environment(&document.environment);
    let (snapshot, _) = snapshot(&document, environment, &mut MeshCache::new());
    let renderer = GpuRenderer::new(gpu, &snapshot);
    let salt = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos() ^ d.as_secs() as u32);
    let replace = |from: &'static str, to: &'static str| {
        move |s: String| {
            assert!(s.contains(from), "no {from:?}");
            s.replace(from, to)
        }
    };
    let cases: Vec<(&str, Box<dyn Fn(String) -> String>)> = vec![
        ("everything", Box::new(|s| s)),
        ("no walk through glass", Box::new(replace("    if (glass == GLASS_SOLID) {\n        return Visibility(", "    if (true) {\n        return Visibility("))),
        ("no seen weight", Box::new(replace("        if (weigh) {", "        if (false) {"))),
        ("no probe", Box::new(replace("    if (!FEATURE_GLASS || (frame.glass_flags & SCENE_TRANSMITS) == 0u) {\n        return md;", "    if (true) {\n        return md;"))),
        ("no split", Box::new(replace("    if (FEATURE_PANES && bounce == 0u && !(*st).reflected && !(*st).in_floor", "    if (false && bounce == 0u && !(*st).reflected && !(*st).in_floor"))),
        ("no whole sky", Box::new(replace("    if (!all(normal == vec3<f32>(0.0))) {\n        return env_sample_facing(normal, u);", "    if (true) {\n        return env_sample_facing(normal, u);"))),
        (
            "no transmission lobe",
            Box::new(|s: String| {
                let mut s = s;
                for arm in [
                    "        case 4u: { return transmission_eval(b.transmission, wo, wi); }\n",
                    "        case 4u: { return transmission_pdf(b.transmission, wo, wi); }\n",
                    "        case 4u: { return transmission_sample(b.transmission, wo, u, uc); }\n",
                ] {
                    assert!(s.contains(arm), "no {arm:?}");
                    s = s.replace(arm, "");
                }
                s
            }),
        ),
    ];
    for (i, (name, edit)) in cases.iter().enumerate() {
        let t = cold_compile_time_edited(&renderer, salt.wrapping_add(i as u32 * 7919), edit.as_ref());
        eprintln!("{name}: {:.2} s", t.as_secs_f64());
    }
}

/// What each node family ported after stage 1 costs the kernel's cold
/// compile, each measured by leaving its arm of the interpreter out. Run by
/// hand: `cargo test --test compile_time -- --ignored --nocapture nodes`.
#[test]
#[ignore]
fn what_the_new_nodes_cost_to_compile() {
    let Some(gpu) = device::for_test(Needs::RayQueries) else { return };
    let scene = parity::scene("grey_product").expect("the parity set has it");
    let document = scene.document();
    let mut environments = tequila_scene::environment::EnvironmentCache::new();
    let environment = environments.environment(&document.environment);
    let (snapshot, _) = snapshot(&document, environment, &mut MeshCache::new());
    let renderer = GpuRenderer::new(gpu, &snapshot);
    let salt = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos() ^ d.as_secs() as u32);
    let drop = |arm: &'static str| {
        move |s: String| {
            assert!(s.contains(arm), "no {arm:?}");
            s.replace(arm, "")
        }
    };
    let cases: Vec<(&str, Box<dyn Fn(String) -> String>)> = vec![
        ("everything", Box::new(|s| s)),
        ("no Tequila procedurals", Box::new(drop("            case 25u: { out = tq_instruction(settings, a); }\n"))),
        ("no Wave", Box::new(drop("            case 26u: { out = cy_wave(settings, a[0].xyz, a[1].x, a[2].x, a[3].x); }\n"))),
        ("no measures", Box::new(drop("            case 27u: { out = prog_attribute(settings, a[0], pt); }\n"))),
        ("no Gabor", Box::new(drop("            case 28u: { out = cy_gabor(settings, a[0].xyz, a[1].x, a[2].x, a[3].x, a[4].x, a[5].xyz, a[6].x); }\n"))),
        (
            "none of them",
            Box::new(|s: String| {
                let mut s = s;
                for arm in [
                    "            case 25u: { out = tq_instruction(settings, a); }\n",
                    "            case 26u: { out = cy_wave(settings, a[0].xyz, a[1].x, a[2].x, a[3].x); }\n",
                    "            case 27u: { out = prog_attribute(settings, a[0], pt); }\n",
                    "            case 28u: { out = cy_gabor(settings, a[0].xyz, a[1].x, a[2].x, a[3].x, a[4].x, a[5].xyz, a[6].x); }\n",
                ] {
                    assert!(s.contains(arm), "no {arm:?}");
                    s = s.replace(arm, "");
                }
                s
            }),
        ),
    ];
    for (i, (name, edit)) in cases.iter().enumerate() {
        let t = cold_compile_time_edited(&renderer, salt.wrapping_add(i as u32 * 7919), edit.as_ref());
        eprintln!("{name}: {:.2} s", t.as_secs_f64());
    }
}

/// What emission and cutouts cost the kernel's cold compile, each measured
/// by leaving it out. Run by hand:
/// `cargo test --test compile_time -- --ignored --nocapture emission`.
#[test]
#[ignore]
fn what_emission_and_cutouts_cost_to_compile() {
    let Some(gpu) = device::for_test(Needs::RayQueries) else { return };
    let scene = parity::scene("grey_product").expect("the parity set has it");
    let document = scene.document();
    let mut environments = tequila_scene::environment::EnvironmentCache::new();
    let environment = environments.environment(&document.environment);
    let (snapshot, _) = snapshot(&document, environment, &mut MeshCache::new());
    let renderer = GpuRenderer::new(gpu, &snapshot);
    let salt = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos() ^ d.as_secs() as u32);
    let replace = |pairs: &'static [(&'static str, &'static str)]| {
        move |s: String| {
            let mut s = s;
            for (from, to) in pairs {
                assert!(s.contains(from), "no {from:?}");
                s = s.replace(from, to);
            }
            s
        }
    };
    let cases: Vec<(&str, Box<dyn Fn(String) -> String>)> = vec![
        ("everything", Box::new(|s| s)),
        ("no light sample of the glowing triangles", Box::new(replace(&[("            if ((frame.glass_flags & SCENE_EMITTERS) == 0u) {\n                continue;", "            if (true) {\n                continue;")]))),
        ("no glow at a hit", Box::new(replace(&[("if (FEATURE_GLOW && material_is_emissive(material.material) && dot(st.direction, raw.geometric) <= 0.0 && lit) {", "if (false) {")]))),
        ("no cutout at a hit", Box::new(replace(&[("if (FEATURE_CUTOUTS && (frame.glass_flags & SCENE_CUTOUTS) != 0u && material.material.geometry_opacity < 0.999) {", "if (false) {")]))),
        (
            "no cutouts in the walks",
            Box::new(replace(&[
                ("        } else if (!open) {\n            through *= 1.0 - clamp(crossing_opacity(hit, direction, inst.material), 0.0, 1.0);", "        } else if (!open) {\n            return Visibility(vec3<f32>(0.0), false);"),
                ("        through = cutout_walk(from_point, normal, aim, -direction);", "        through = 0.0;"),
            ])),
        ),
        (
            "no programs at crossings",
            Box::new(replace(&[
                ("    if (!FEATURE_CUT_PROGRAMS || (scene_programs[material].flags & PROGRAM_CUTS) == 0u) {", "    if (true) {"),
                ("        let programs_cut = FEATURE_CUT_PROGRAMS && (frame.glass_flags & SCENE_CUT_PROGRAMS) != 0u;", "        let programs_cut = false;"),
            ])),
        ),
        ("no glowing volume", Box::new(replace(&[("let glow = emitted_over(medium, hit.t) * max(ior * ior, 1.0);", "let glow = vec3<f32>(0.0);")]))),
    ];
    for (i, (name, edit)) in cases.iter().enumerate() {
        let t = cold_compile_time_edited(&renderer, salt.wrapping_add(i as u32 * 7919), edit.as_ref());
        eprintln!("{name}: {:.2} s", t.as_secs_f64());
    }
}

/// What rounded edges cost the kernel's cold compile: the kernel with and
/// without the bevel's one call site, interleaved so the card's other work
/// falls on both alike, `TEQUILA_COMPILE_ROUNDS` rounds (3 by default). Run
/// by hand: `cargo test --test compile_time -- --ignored --nocapture bevel`.
#[test]
#[ignore]
fn what_rounded_edges_cost_to_compile() {
    let Some(gpu) = device::for_test(Needs::RayQueries) else { return };
    let scene = parity::scene("grey_product").expect("the parity set has it");
    let document = scene.document();
    let mut environments = tequila_scene::environment::EnvironmentCache::new();
    let environment = environments.environment(&document.environment);
    let (snapshot, _) = snapshot(&document, environment, &mut MeshCache::new());
    let renderer = GpuRenderer::new(gpu, &snapshot);
    let salt = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos() ^ d.as_secs() as u32);
    let call = "seg_rounded = bevel_normal(hit.instance_custom_data, raw.position, seg_surf.shading, base.edge_radius, base.edge_samples, &s);";
    let rounds: u32 = std::env::var("TEQUILA_COMPILE_ROUNDS").ok().and_then(|r| r.parse().ok()).unwrap_or(3);
    for round in 0..rounds {
        for (k, without) in [false, true].into_iter().enumerate() {
            let edit = move |s: String| {
                assert!(s.contains(call), "no bevel call");
                if without { s.replace(call, "") } else { s }
            };
            let t = cold_compile_time_edited(&renderer, salt.wrapping_add((round * 2 + k as u32) * 7919), &edit);
            eprintln!("round {round}, {}: {:.2} s", if without { "without rounded edges" } else { "with rounded edges" }, t.as_secs_f64());
        }
    }
}

/// What letting programs drive the transmission's weight costs the
/// kernel's cold compile: the write-back four floats longer, against the
/// kernel with the write-back as it was, interleaved, `TEQUILA_COMPILE_ROUNDS`
/// rounds (3 by default). Run by hand: `cargo test --test compile_time --
/// --ignored --nocapture transmission`.
#[test]
#[ignore]
fn what_driven_transmission_costs_to_compile() {
    let Some(gpu) = device::for_test(Needs::RayQueries) else { return };
    let scene = parity::scene("grey_product").expect("the parity set has it");
    let document = scene.document();
    let mut environments = tequila_scene::environment::EnvironmentCache::new();
    let environment = environments.environment(&document.environment);
    let (snapshot, _) = snapshot(&document, environment, &mut MeshCache::new());
    let renderer = GpuRenderer::new(gpu, &snapshot);
    let salt = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos() ^ d.as_secs() as u32);
    let pairs: &[(&str, &str)] = &[
        ("array<f32, 48>", "array<f32, 44>"),
        ("        m.transmission_color.x, m.transmission_color.y, m.transmission_color.z, m.transmission_color.w,\n", ""),
        ("vec4<f32>(f[44], f[45], f[46], f[47]), base.transmission_tint,", "base.transmission_color, base.transmission_tint,"),
    ];
    let rounds: u32 = std::env::var("TEQUILA_COMPILE_ROUNDS").ok().and_then(|r| r.parse().ok()).unwrap_or(3);
    for round in 0..rounds {
        for (k, without) in [false, true].into_iter().enumerate() {
            let edit = move |s: String| {
                let mut s = s.replace("\r\n", "\n");
                if without {
                    for (from, to) in pairs {
                        assert!(s.contains(from), "no {from:?}");
                        s = s.replace(from, to);
                    }
                }
                s
            };
            let t = cold_compile_time_edited(&renderer, salt.wrapping_add((round * 2 + k as u32) * 7919), &edit);
            eprintln!("round {round}, {}: {:.2} s", if without { "the write-back as it was" } else { "the transmission's weight written back" }, t.as_secs_f64());
        }
    }
}

/// The renderer for a scene of the parity set, or for the Headphones
/// sample ("headphones"), with the whole kernel built.
fn renderer_for(name: &str) -> Option<GpuRenderer> {
    let gpu = device::for_test(Needs::RayQueries)?;
    // A parity scene with the pictures it reads (its own cache).
    let (document, mut cache) = if name == "headphones" {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/samples/Headphones.tequila");
        (tequila_format::load(&path).expect("the Headphones sample loads").document, MeshCache::new())
    } else {
        let scene = parity::scene(name).expect("the parity set has it");
        (scene.document(), scene.cache())
    };
    let mut environments = tequila_scene::environment::EnvironmentCache::new();
    let environment = environments.environment(&document.environment);
    let (snapshot, _) = snapshot(&document, environment, &mut cache);
    Some(GpuRenderer::with_kernel(gpu, &snapshot, Some(KernelFeatures::ALL)))
}

fn salt() -> u32 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos() ^ d.as_secs() as u32)
}

/// Whether the driver drops what a variant leaves out: the kernel with no
/// part, through the pipeline constants and as constants in the source,
/// against the whole kernel; interleaved. Run by hand:
/// `cargo test --test compile_time -- --ignored --nocapture overrides`.
#[test]
#[ignore]
fn the_driver_drops_what_the_overrides_leave_out() {
    let Some(renderer) = renderer_for("grey_product") else { return };
    let rounds: usize = std::env::var("TEQUILA_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(2);
    let as_constants = |s: String| {
        let mut s = s;
        for (_, name) in KernelFeatures::NAMED {
            let from = format!("override {name}: bool = true;");
            assert!(s.contains(&from), "no {from:?}");
            s = s.replace(&from, &format!("const {name}: bool = false;"));
        }
        s.replace("override FEATURE_OPS: u32 = 0xFFFFFFFFu;", "const FEATURE_OPS: u32 = 0u;")
    };
    let salt = salt();
    for round in 0..rounds {
        let k = round as u32 * 3 * 7919;
        let all = cold_compile_time_of(&renderer, salt.wrapping_add(k), KernelFeatures::ALL);
        eprintln!("round {round}: whole kernel {:.2} s", all.as_secs_f64());
        let none = cold_compile_time_of(&renderer, salt.wrapping_add(k + 7919), KernelFeatures::NONE);
        eprintln!("round {round}: no part, by overrides {:.2} s", none.as_secs_f64());
        let constants = cold_compile_time_edited(&renderer, salt.wrapping_add(k + 2 * 7919), &as_constants);
        eprintln!("round {round}: no part, as constants in the source {:.2} s", constants.as_secs_f64());
    }
}

/// What each scene's own variant costs to compile cold against the whole
/// kernel (`docs/07` G1.6), interleaved round by round; then each built
/// again unsalted, as a second launch finds it in the driver's cache. Run
/// by hand: `cargo test --test compile_time -- --ignored --nocapture
/// variants`; `TEQUILA_ROUNDS` rounds (3), `TEQUILA_SCENES` a comma list.
#[test]
#[ignore]
fn what_the_variants_cost_to_compile() {
    let rounds: usize = std::env::var("TEQUILA_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(3);
    let names = std::env::var("TEQUILA_SCENES").unwrap_or_else(|_| "grey_product,lamp_lit,headphones,liquid_in_glass".to_string());
    let renderers: Vec<(String, GpuRenderer)> = names.split(',').filter_map(|n| renderer_for(n.trim()).map(|r| (n.trim().to_string(), r))).collect();
    for (name, r) in &renderers {
        eprintln!("{name}: its variant keeps {}", r.features.names());
    }
    let salt = salt();
    for round in 0..rounds {
        for (i, (name, r)) in renderers.iter().enumerate() {
            let k = ((round * renderers.len() + i) * 2) as u32 * 7919;
            // Alternate which goes first, so neither always has the card's
            // other user to itself.
            let (full, variant) = if (round + i) % 2 == 0 {
                let f = cold_compile_time_of(r, salt.wrapping_add(k), KernelFeatures::ALL);
                (f, cold_compile_time_of(r, salt.wrapping_add(k + 7919), r.features))
            } else {
                let v = cold_compile_time_of(r, salt.wrapping_add(k + 7919), r.features);
                (cold_compile_time_of(r, salt.wrapping_add(k), KernelFeatures::ALL), v)
            };
            eprintln!("round {round} {name}: whole kernel {:.2} s, its variant {:.2} s", full.as_secs_f64(), variant.as_secs_f64());
        }
    }
    for (name, r) in &renderers {
        let full = compile_time_of(r, KernelFeatures::ALL);
        let variant = compile_time_of(r, r.features);
        eprintln!("{name} built again, unsalted: whole kernel {:.3} s, its variant {:.3} s", full.as_secs_f64(), variant.as_secs_f64());
    }
}

/// What the parts every variant keeps cost the cold compile: the kernel
/// with no part (a grey product's), and that with one more piece left out
/// by editing its source. Run by hand: `cargo test --test compile_time --
/// --ignored --nocapture what_every_variant`.
#[test]
#[ignore]
fn what_every_variant_keeps_costs_to_compile() {
    let Some(renderer) = renderer_for("grey_product") else { return };
    let replace = |pairs: &'static [(&'static str, &'static str)]| {
        move |s: String| {
            let mut s = s;
            for (from, to) in pairs {
                assert!(s.contains(from), "no {from:?}");
                s = s.replace(from, to);
            }
            s
        }
    };
    let arms = |slot: &'static str| {
        move |s: String| {
            // Each `case N: { return ... }` arm of the stack's dispatch.
            let mut out = String::new();
            let mut dropped = 0;
            for line in s.lines() {
                if line.trim_start().starts_with(&format!("case {slot}: {{ return ")) && !line.contains("DEPTH_") {
                    dropped += 1;
                    continue;
                }
                out.push_str(line);
                out.push('\n');
            }
            assert!(dropped >= 4, "only {dropped} arms of slot {slot}");
            out
        }
    };
    let cases: Vec<(&str, Box<dyn Fn(String) -> String>)> = vec![
        ("no part", Box::new(|s| s)),
        ("no transmission arms", Box::new(arms("4u"))),
        ("no sheen arms", Box::new(arms("0u"))),
        ("no coat arms", Box::new(arms("1u"))),
        ("no metal arms", Box::new(arms("2u"))),
        ("no sky sample", Box::new(replace(&[("            if (frame.env_intensity <= 0.0) {\n                continue;", "            if (true) {\n                continue;")]))),
        ("no light sample at all", Box::new(replace(&[("        let direct = direct_light(s, bsdf, f, wo, surf, inst.instance, transmissive);", "        let direct = vec3<f32>(0.0);")]))),
        ("no meeting and escaping", Box::new(replace(&[("                let sky = meet_and_escape(&st, hit);", "                let sky = vec3<f32>(0.0);")]))),
        ("no Filter Glossy rebuild", Box::new(replace(&[("        if (roughness_floor == floor && blur > floor && openpbr_has_delta(bsdf)) {", "        if (false) {")]))),
        ("no aiming", Box::new(replace(&[("        bsdf = openpbr_aim(openpbr_build(lobes, roughness_floor, true, from_inside), wo);", "        bsdf = openpbr_build(lobes, roughness_floor, false, from_inside);")]))),
    ];
    let salt = salt();
    let rounds: usize = std::env::var("TEQUILA_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    for round in 0..rounds {
        for (i, (name, edit)) in cases.iter().enumerate() {
            let t = cold_compile_time_edited_of(&renderer, salt.wrapping_add(((round * cases.len() + i) as u32) * 7919), edit.as_ref(), KernelFeatures::NONE);
            eprintln!("round {round} {name}: {:.2} s", t.as_secs_f64());
        }
    }
}

/// What the parts of a programmed scene's variant cost: the Headphones'
/// own (its floor, programs and their instructions), less one
/// part or one instruction's arm, and with every arm. Run by hand:
/// `cargo test --test compile_time -- --ignored --nocapture programmed`.
#[test]
#[ignore]
fn what_a_programmed_variant_costs_to_compile() {
    let Some(renderer) = renderer_for("headphones") else { return };
    let own = renderer.features;
    let without = |part: KernelFeatures| own.without(part);
    let cases: Vec<(&str, KernelFeatures, Box<dyn Fn(String) -> String>)> = vec![
        ("its own variant", own, Box::new(|s| s)),
        ("no floor", without(KernelFeatures::FLOOR), Box::new(|s| s)),
        ("no programs", KernelFeatures { ops: 0, ..without(KernelFeatures::PROGRAMS) }, Box::new(|s| s)),
        ("no Noise", without(KernelFeatures::op(6)), Box::new(|s| s)),
        ("no Voronoi", without(KernelFeatures::op(7)), Box::new(|s| s)),
        ("no Math", without(KernelFeatures::op(2)), Box::new(|s| s)),
        ("no Vector Math", without(KernelFeatures::op(3)), Box::new(|s| s)),
        ("no Map Range", without(KernelFeatures::op(4)), Box::new(|s| s)),
        ("no Mix", without(KernelFeatures::op(5)), Box::new(|s| s)),
        ("no Mapping", without(KernelFeatures::op(9)), Box::new(|s| s)),
        ("every arm, its own instructions or not", KernelFeatures { ops: u32::MAX, ..own }, Box::new(|s| s)),
    ];
    let salt = salt();
    for (i, (name, kernel, edit)) in cases.iter().enumerate() {
        let t = cold_compile_time_edited_of(&renderer, salt.wrapping_add(i as u32 * 7919), edit.as_ref(), *kernel);
        eprintln!("{name}: {:.2} s", t.as_secs_f64());
    }
}

/// What of a kernel's build is naga's, on the processor, before the driver
/// sees it: joining the modules, parsing, validating, applying the
/// constants and writing SPIR-V, which every build pays, warm or cold. Run
/// by hand: `cargo test --test compile_time -- --ignored --nocapture naga`.
#[test]
#[ignore]
fn what_naga_costs_of_a_build() {
    for (label, kernel) in [("whole kernel", KernelFeatures::ALL), ("no part", KernelFeatures::NONE)] {
        let started = std::time::Instant::now();
        let source = tequila_render_gpu::shader::source("transport");
        let joined = started.elapsed();
        let module = naga::front::wgsl::parse_str(&source).expect("parses");
        let parsed = started.elapsed();
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).expect("validates");
        let validated = started.elapsed();
        let constants: naga::back::PipelineConstants = kernel.constants().into_iter().map(|(n, v)| (n.to_string(), v)).collect();
        let (module, info) = naga::back::pipeline_constants::process_overrides(&module, &info, Some((naga::ShaderStage::Compute, "transport_main")), &constants).expect("takes its constants");
        let overridden = started.elapsed();
        let spirv = naga::back::spv::write_vec(&module, &info, &naga::back::spv::Options::default(), None).expect("writes");
        let written = started.elapsed();
        eprintln!(
            "{label}: joined {:.3} s, parsed {:.3} s, validated {:.3} s, constants {:.3} s, SPIR-V {:.3} s ({} words): {:.3} s in all",
            joined.as_secs_f64(),
            (parsed - joined).as_secs_f64(),
            (validated - parsed).as_secs_f64(),
            (overridden - validated).as_secs_f64(),
            (written - overridden).as_secs_f64(),
            spirv.len(),
            written.as_secs_f64()
        );
    }
}
