//! The WGSL modules, and the step that joins one to what it includes
//! (`docs/07` R2.1).
//!
//! WGSL has no includes. A module names the ones it needs on lines of its
//! own, `//#include rng`, and [`source`] puts each included module in front
//! of it once, in the order first asked for, so a module read on its own is
//! still the file in `src/wgsl/` and the error a shader compiler gives is
//! about a line that exists there.

/// Every module, by name.
pub const MODULES: &[(&str, &str)] = &[
    ("rng", include_str!("wgsl/rng.wgsl")),
    ("features", include_str!("wgsl/features.wgsl")),
    ("sobol", include_str!("wgsl/sobol.wgsl")),
    ("sampler", include_str!("wgsl/sampler.wgsl")),
    ("scene", include_str!("wgsl/scene.wgsl")),
    ("microfacet", include_str!("wgsl/microfacet.wgsl")),
    ("fresnel", include_str!("wgsl/fresnel.wgsl")),
    ("energy", include_str!("wgsl/energy.wgsl")),
    ("lobes", include_str!("wgsl/lobes.wgsl")),
    ("sheen", include_str!("wgsl/sheen.wgsl")),
    ("openpbr", include_str!("wgsl/openpbr.wgsl")),
    ("frame", include_str!("wgsl/frame.wgsl")),
    ("nodes", include_str!("wgsl/nodes.wgsl")),
    ("textures", include_str!("wgsl/textures.wgsl")),
    ("procedural", include_str!("wgsl/procedural.wgsl")),
    ("flakes", include_str!("wgsl/flakes.wgsl")),
    ("film", include_str!("wgsl/film.wgsl")),
    ("dispersion", include_str!("wgsl/dispersion.wgsl")),
    ("program", include_str!("wgsl/program.wgsl")),
    ("environment", include_str!("wgsl/environment.wgsl")),
    ("onb", include_str!("wgsl/onb.wgsl")),
    ("lights", include_str!("wgsl/lights.wgsl")),
    ("lamps", include_str!("wgsl/lamps.wgsl")),
    ("emitters", include_str!("wgsl/emitters.wgsl")),
    ("transport", include_str!("wgsl/transport.wgsl")),
];

const INCLUDE: &str = "//#include ";

/// A module's text.
pub fn module(name: &str) -> Option<&'static str> {
    MODULES.iter().find(|(n, _)| *n == name).map(|(_, text)| *text)
}

/// `name` with everything it includes in front of it, each once.
///
/// Panics on a module that is not in [`MODULES`] or includes itself through
/// others: both are mistakes in this crate, not in anyone's scene.
pub fn source(name: &str) -> String {
    let mut done: Vec<&str> = Vec::new();
    let mut out = String::new();
    join(name, &mut done, &mut Vec::new(), &mut out);
    hoist_enables(&out)
}

/// WGSL's `enable` directives must come before anything else in a module,
/// so the ones the joined modules carry are moved to the top, each once.
fn hoist_enables(text: &str) -> String {
    let mut enables: Vec<&str> = Vec::new();
    let mut rest = String::with_capacity(text.len());
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("enable ") && trimmed.ends_with(';') {
            if !enables.contains(&trimmed) {
                enables.push(trimmed);
            }
        } else {
            rest.push_str(line);
            rest.push('\n');
        }
    }
    let mut out: String = enables.iter().map(|e| format!("{e}\n")).collect();
    out.push_str(&rest);
    out
}

/// [`source`] for a kernel that is not a module: `text` with what it
/// includes in front of it.
pub fn with_includes(text: &str) -> String {
    let mut done: Vec<&str> = Vec::new();
    let mut out = String::new();
    for line in text.lines() {
        if let Some(name) = line.trim().strip_prefix(INCLUDE) {
            let name = MODULES
                .iter()
                .map(|(n, _)| *n)
                .find(|n| *n == name.trim())
                .unwrap_or_else(|| panic!("no WGSL module {name:?}"));
            join(name, &mut done, &mut Vec::new(), &mut out);
        }
    }
    out.push_str(text);
    hoist_enables(&out)
}

fn join(name: &str, done: &mut Vec<&'static str>, path: &mut Vec<String>, out: &mut String) {
    let (key, text) = MODULES
        .iter()
        .find(|(n, _)| *n == name)
        .copied()
        .unwrap_or_else(|| panic!("no WGSL module {name:?}"));
    if done.contains(&key) {
        return;
    }
    assert!(!path.iter().any(|p| p == key), "WGSL modules include each other in a circle: {path:?} -> {key}");
    path.push(key.to_string());
    for line in text.lines() {
        if let Some(included) = line.trim().strip_prefix(INCLUDE) {
            join(included.trim(), done, path, out);
        }
    }
    path.pop();
    done.push(key);
    out.push_str(&format!("// ---- {key}.wgsl ----\n"));
    out.push_str(text);
    if !text.ends_with('\n') {
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_module_comes_after_what_it_includes_and_each_comes_once() {
        let joined = source("sampler");
        let at = |name: &str| joined.find(&format!("// ---- {name}.wgsl ----")).unwrap_or_else(|| panic!("{name} missing"));
        assert!(at("rng") < at("sobol") && at("sobol") < at("sampler"));
        assert_eq!(joined.matches("// ---- rng.wgsl ----").count(), 1);
    }
}
