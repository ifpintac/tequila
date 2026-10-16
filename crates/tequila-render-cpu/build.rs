//! Locate the Embree SDK and tell Cargo how to link it.
//!
//! Embree ships as a prebuilt SDK rather than as a crate. `EMBREE_DIR` points
//! at an unpacked release; failing that, the repository's own `third_party`
//! directory is searched, which is where `scripts/fetch-third-party` puts it.

use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=EMBREE_DIR");
    println!("cargo:rerun-if-changed=build.rs");

    let root = match locate() {
        Some(root) => root,
        None => {
            // A clear, actionable message: a bare "cannot find -lembree4" from
            // the linker sends people hunting in the wrong place.
            println!(
                "cargo:warning=Embree SDK not found. Set EMBREE_DIR to an unpacked \
                 Embree 4 release, or run scripts/fetch-third-party."
            );
            return;
        }
    };

    let lib = root.join("lib");
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=dylib=embree4");

    // The libraries have to sit next to the test and example binaries, or
    // every run fails at load time with an error that says nothing useful:
    // the DLLs from `bin` on Windows, the dylibs or shared objects from `lib`
    // elsewhere, where the binaries look beside themselves by their rpath
    // (`.cargo/config.toml`).
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let bin = root.join("bin");
        println!("cargo:rustc-env=TEQUILA_EMBREE_BIN={}", bin.display());
        copy_runtime_libraries(&bin, true);
    } else {
        copy_runtime_libraries(&lib, false);
    }

    println!("cargo:rustc-env=TEQUILA_EMBREE_DIR={}", root.display());
}

fn locate() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("EMBREE_DIR") {
        let path = PathBuf::from(dir);
        if path.join("lib").is_dir() {
            return Some(path);
        }
    }
    // `third_party/embree-*`, relative to the workspace root.
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent()?.parent()?;
    let third_party = workspace.join("third_party");
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(&third_party)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("embree") && for_target(n))
                && p.join("lib").is_dir()
        })
        .collect();
    // Newest version last by name, so prefer the highest.
    candidates.sort();
    candidates.pop()
}

/// Whether an SDK folder is for the platform being built for, by its name:
/// `embree-4.4.1.x64.windows`, `oidn-2.5.1.arm64.macos` and the like. By the
/// target, not the machine building it, so a checkout copied from Windows to
/// a Mac does not link the Windows SDK it still holds.
fn for_target(name: &str) -> bool {
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let os_matches = match os.as_str() {
        "windows" => name.contains("windows"),
        "macos" => name.contains("macos"),
        "linux" => name.contains("linux"),
        _ => true,
    };
    let arch_matches = match arch.as_str() {
        "x86_64" => name.contains("x64") || name.contains("x86_64"),
        "aarch64" => name.contains("arm64") || name.contains("aarch64"),
        _ => true,
    };
    os_matches && arch_matches
}

/// Whether a file is a runtime library to put beside the binaries: a DLL on
/// Windows, a dylib or shared object elsewhere.
fn runtime_library(path: &Path, windows: bool) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if windows {
        name.ends_with(".dll")
    } else {
        name.contains(".dylib") || name.contains(".so")
    }
}

/// Copy the runtime libraries beside the built binaries.
fn copy_runtime_libraries(bin: &Path, windows: bool) {
    let Ok(out_dir) = std::env::var("OUT_DIR") else {
        return;
    };
    // OUT_DIR is `target/<profile>/build/<pkg>-<hash>/out`; the binaries live
    // three levels up.
    let Some(target_dir) = Path::new(&out_dir)
        .ancestors()
        .nth(3)
        .map(Path::to_path_buf)
    else {
        return;
    };

    let Ok(entries) = std::fs::read_dir(bin) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if !runtime_library(&path, windows) {
            continue;
        }
        let Some(name) = path.file_name() else {
            continue;
        };
        for destination in [target_dir.join(name), target_dir.join("deps").join(name)] {
            if let Some(parent) = destination.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            // Only copy when missing or stale, so incremental builds stay fast.
            let stale = std::fs::metadata(&destination)
                .and_then(|d| d.modified())
                .ok()
                .zip(entry.metadata().and_then(|s| s.modified()).ok())
                .map(|(dst, src)| dst < src)
                .unwrap_or(true);
            if stale {
                let _ = std::fs::copy(&path, &destination);
            }
        }
    }
}
