//! How long the transport kernel takes to compile with nothing cached
//! (`docs/07` §7: one generic interpreter, one call site, so the kernel does
//! not grow with every material). The driver keeps what it compiled, so the
//! kernel is salted to be a new one each time; the time is printed, and held
//! under a generous ceiling that a kernel which had doubled would break.

use tequila_render_gpu::device::{self, Needs};
use tequila_render_gpu::transport::{cold_compile_time, cold_compile_time_edited, GpuRenderer};
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
