//! Write the generated Python bindings and API reference:
//! `python/tequila_commands.py` and `docs/scripting-api.md`.

fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (path, text) in [("python/tequila_commands.py", tequila_scripting::bindings::python()), ("docs/scripting-api.md", tequila_scripting::bindings::reference())] {
        let target = root.join(path);
        std::fs::write(&target, text).unwrap_or_else(|e| panic!("{}: {e}", target.display()));
        println!("wrote {path}");
    }
}
