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
    ("sobol", include_str!("wgsl/sobol.wgsl")),
    ("sampler", include_str!("wgsl/sampler.wgsl")),
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
    out
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
