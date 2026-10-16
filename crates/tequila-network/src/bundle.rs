//! A project and every file it reads, as one archive a worker can render
//! from: the project with its outside paths renamed into `files/`, and the
//! files themselves (`tequila_scene::paths` knows where every path is).
//!
//! A worker writes only into the folder it unpacks into, and only names it
//! chose the shape of: a member called anything but `files/<plain name>` is
//! refused, so a bundle cannot reach anywhere else on the worker's disk.

use tequila_format::zip::{ZipArchive, ZipWriter};
use tequila_scene::SceneDocument;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const PROJECT: &str = "project.tequila";
const FILES: &str = "files/";

/// A packed project.
pub struct Bundle {
    pub bytes: Arc<Vec<u8>>,
    /// Paths the project names that could not be read, and so are not in
    /// it: the worker renders without them, as the application would.
    pub missing: Vec<String>,
}

/// A name a bundle's file may have: letters, digits, `.`, `_` and `-`,
/// not starting with a dot.
fn plain(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && name.len() <= 200 && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Pack a document. `base` is the project's folder, which relative paths
/// in it are relative to.
pub fn pack(document: &SceneDocument, preserved: &tequila_format::Preserved, base: Option<&Path>) -> Bundle {
    let mut copy = document.clone();
    let mut zip = ZipWriter::new();
    let mut missing = Vec::new();
    let mut packed: Vec<(String, String)> = Vec::new();
    tequila_scene::paths::rewrite(&mut copy, &mut |path| {
        if let Some((_, name)) = packed.iter().find(|(p, _)| p == path) {
            return Some(name.clone());
        }
        let mut source = PathBuf::from(path);
        if source.is_relative() {
            if let Some(base) = base {
                source = base.join(source);
            }
        }
        match std::fs::read(&source) {
            Ok(bytes) => {
                let stem: String = source
                    .file_name()
                    .map(|n| n.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' }).collect())
                    .unwrap_or_default();
                let name = format!("{FILES}{}-{}", packed.len(), stem.trim_start_matches('.'));
                zip.add(name.clone(), bytes);
                packed.push((path.to_string(), name.clone()));
                Some(name)
            }
            Err(_) => {
                missing.push(path.to_string());
                None
            }
        }
    });
    zip.add(PROJECT, tequila_format::write(&copy, preserved));
    Bundle { bytes: Arc::new(zip.finish()), missing }
}

/// Unpack into `folder` (made if it is not there) and read the project,
/// its paths pointing at the files there.
pub fn unpack(bytes: &[u8], folder: &Path) -> Result<SceneDocument, String> {
    let archive = ZipArchive::read(bytes).map_err(|e| format!("the job's bundle is not readable: {e}"))?;
    let project = archive.get(PROJECT).ok_or("the job's bundle has no project")?;
    std::fs::create_dir_all(folder.join(FILES)).map_err(|e| format!("{}: {e}", folder.display()))?;
    for name in archive.names() {
        if name == PROJECT {
            continue;
        }
        let Some(file) = name.strip_prefix(FILES).filter(|f| plain(f)) else {
            return Err(format!("the job's bundle holds `{name}`, which is not a name a bundle may use"));
        };
        let target = folder.join(FILES).join(file);
        std::fs::write(&target, archive.get(name).unwrap_or_default()).map_err(|e| format!("{}: {e}", target.display()))?;
    }
    let mut document = tequila_format::read(project).map_err(|e| format!("the job's project is not readable: {e}"))?.document;
    tequila_scene::paths::rewrite(&mut document, &mut |path| {
        let file = path.strip_prefix(FILES)?;
        plain(file).then(|| folder.join(FILES).join(file).to_string_lossy().into_owned())
    });
    Ok(document)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_scene::textures::TextureSource;
    use tequila_texture::texture::MapRole;

    #[test]
    fn a_project_travels_with_its_files() {
        let folder = std::env::temp_dir().join(format!("tequila-bundle-{}", std::process::id()));
        let source = folder.join("source");
        std::fs::create_dir_all(source.join("maps")).unwrap();
        std::fs::write(source.join("maps").join("wood grain.png"), b"not really a png").unwrap();
        let mut document = SceneDocument::new();
        // One relative to the project, one that is not there.
        document.textures.intern(TextureSource::new("maps/wood grain.png", MapRole::Color));
        document.environment.image = Some(source.join("gone.exr").to_string_lossy().into_owned());

        let bundle = pack(&document, &tequila_format::Preserved::default(), Some(&source));
        assert_eq!(bundle.missing.len(), 1);
        let landed = folder.join("worker");
        let unpacked = unpack(&bundle.bytes, &landed).unwrap();
        let paths = tequila_scene::paths::external(&unpacked);
        let texture = paths.iter().find(|p| p.contains("wood")).expect("the texture's path");
        assert!(texture.starts_with(&landed.to_string_lossy().into_owned()), "{texture}");
        assert_eq!(std::fs::read(texture).unwrap(), b"not really a png");
        std::fs::remove_dir_all(&folder).ok();
    }

    #[test]
    fn a_bundle_cannot_write_outside_its_folder() {
        let folder = std::env::temp_dir().join(format!("tequila-bundle-escape-{}", std::process::id()));
        for name in ["files/../../escaped.txt", "notes.txt", "files/sub/deeper.png", "files/.hidden"] {
            let mut zip = ZipWriter::new();
            zip.add(PROJECT, tequila_format::write(&SceneDocument::new(), &tequila_format::Preserved::default()));
            zip.add(name, b"x".to_vec());
            // Refused by the archive reader or by the bundle's own rule.
            assert!(unpack(&zip.finish(), &folder).is_err(), "{name} was unpacked");
        }
        assert!(!folder.parent().unwrap().join("escaped.txt").exists());
        assert!(!folder.join("notes.txt").exists() && !folder.join("files").join("sub").exists());
        std::fs::remove_dir_all(&folder).ok();
    }
}
