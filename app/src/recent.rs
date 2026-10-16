//! The projects opened or saved lately, for File → Open recent: kept in
//! `recent.txt` beside the user's library, one path a line, newest first.

use std::path::{Path, PathBuf};

/// How many are kept.
const KEEP: usize = 10;

/// Where the list is kept.
pub fn file() -> PathBuf {
    crate::library_tab::user_library_path()
        .parent()
        .map(|p| p.join("recent.txt"))
        .unwrap_or_else(|| std::env::temp_dir().join("tequila-recent.txt"))
}

/// The list in `file`, newest first; empty when there is none.
pub fn read(file: &Path) -> Vec<PathBuf> {
    std::fs::read_to_string(file)
        .map(|text| text.lines().map(str::trim).filter(|l| !l.is_empty()).map(PathBuf::from).collect())
        .unwrap_or_default()
}

/// The same file, however it was spelled: Windows paths ignore case.
fn same(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

/// `list` with `project` first, once, cut to the most kept.
pub fn with(list: &[PathBuf], project: &Path) -> Vec<PathBuf> {
    let mut out = vec![project.to_path_buf()];
    out.extend(list.iter().filter(|p| !same(p, project)).cloned());
    out.truncate(KEEP);
    out
}

/// `list` without `project`.
pub fn without(list: &[PathBuf], project: &Path) -> Vec<PathBuf> {
    list.iter().filter(|p| !same(p, project)).cloned().collect()
}

/// Write the list to `file`. A list that cannot be kept is only a
/// convenience lost, never an error worth interrupting anyone for.
pub fn write(file: &Path, list: &[PathBuf]) {
    let text: String = list.iter().map(|p| format!("{}\n", p.display())).collect();
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(file, text);
}

/// The name shown in the menu: the file's name, and its folder after it
/// when two in the list share a name.
pub fn label(list: &[PathBuf], project: &Path) -> String {
    let name = project.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| project.display().to_string());
    let twins = list.iter().filter(|p| p.file_stem() == project.file_stem()).count();
    match project.parent().and_then(|p| p.file_name()) {
        Some(folder) if twins > 1 => format!("{name}  ({})", folder.to_string_lossy()),
        _ => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Newest first, each once whatever its case, ten at most, and kept
    /// between sessions.
    #[test]
    fn the_list_keeps_the_latest_ten_once_each() {
        let mut list = Vec::new();
        for i in 0..12 {
            list = with(&list, Path::new(&format!("C:/work/p{i}.tequila")));
        }
        assert_eq!(list.len(), KEEP);
        assert_eq!(list[0], PathBuf::from("C:/work/p11.tequila"));
        list = with(&list, Path::new("C:/work/p5.tequila"));
        assert_eq!(list[0], PathBuf::from("C:/work/p5.tequila"));
        assert_eq!(list.iter().filter(|p| p.ends_with("p5.tequila")).count(), 1);
        if cfg!(windows) {
            list = with(&list, Path::new("C:/WORK/P5.tequila"));
            assert_eq!(list.iter().filter(|p| p.to_string_lossy().to_lowercase().ends_with("p5.tequila")).count(), 1);
        }
        list = without(&list, Path::new("C:/work/p11.tequila"));
        assert!(!list.iter().any(|p| p.ends_with("p11.tequila")));

        let file = std::env::temp_dir().join(format!("tequila-recent-test-{}.txt", std::process::id()));
        write(&file, &list);
        assert_eq!(read(&file), list);
        let _ = std::fs::remove_file(&file);
        assert!(read(&file).is_empty());

        let twins = vec![PathBuf::from("C:/a/Kettle.tequila"), PathBuf::from("C:/b/Kettle.tequila"), PathBuf::from("C:/b/Mug.tequila")];
        assert_eq!(label(&twins, &twins[0]), "Kettle  (a)");
        assert_eq!(label(&twins, &twins[2]), "Mug");
    }
}
