//! Connecting the Claude desktop app to Tequila in one click.
//!
//! The Claude app reaches tools on this computer through MCP servers listed
//! in its settings file, `claude_desktop_config.json`. Tequila's server is
//! built into its command-line program (`tequila-cli --mcp`). Listing it
//! took a hand-edited JSON file and, before it was built in, Python - so on a
//! work PC the Claude app had no Tequila tools at all, and asked to set up
//! materials it drove the screen with its mouse instead. Here the entry is
//! written for the person, keeping everything else in the file.

use tequila_format::json::{self, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The name Tequila's entry goes under.
const SERVER: &str = "tequila";

/// Where the Claude app keeps its settings on this computer: every folder
/// it has made, or where it would make one. The app installed from the
/// Microsoft Store keeps its own copy, inside its package's folder.
pub fn config_paths() -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut standard = None;
    #[cfg(windows)]
    {
        if let Some(roaming) = std::env::var_os("APPDATA").map(PathBuf::from) {
            standard = Some(roaming.join("Claude"));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
            if let Ok(packages) = std::fs::read_dir(local.join("Packages")) {
                for package in packages.flatten() {
                    if package.file_name().to_string_lossy().starts_with("Claude_") {
                        let folder = package.path().join("LocalCache").join("Roaming").join("Claude");
                        if folder.is_dir() {
                            found.push(folder);
                        }
                    }
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            standard = Some(home.join("Library").join("Application Support").join("Claude"));
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            standard = Some(home.join(".config").join("Claude"));
        }
    }
    if let Some(standard) = standard {
        if standard.is_dir() || found.is_empty() {
            found.insert(0, standard);
        }
    }
    found.into_iter().map(|folder| folder.join("claude_desktop_config.json")).collect()
}

/// Tequila's command-line program, which serves the tools: beside the
/// application once installed, or the `tequila` a development build makes
/// beside it.
pub fn server_program() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let folder = exe.parent()?;
    let name = if cfg!(windows) { ["tequila-cli.exe", "tequila.exe"] } else { ["tequila-cli", "tequila"] };
    name.iter().map(|n| folder.join(n)).find(|p| p.is_file())
}

/// Whether the settings at `path` list Tequila, pointing at `program`.
pub fn is_connected(path: &Path, program: &Path) -> bool {
    let Ok(bytes) = std::fs::read(path) else { return false };
    let Ok(config) = json::parse(&bytes) else { return false };
    config
        .get("mcpServers")
        .and_then(|servers| servers.get(SERVER))
        .and_then(|server| server.get("command"))
        .and_then(Value::as_str)
        .is_some_and(|command| Path::new(command) == program)
}

/// List Tequila in the settings at `path`, serving through `program`,
/// keeping every other setting and server. A file that is not JSON is left
/// alone and reported rather than replaced; the first change keeps the
/// original beside it as `.before-tequila`.
pub fn connect(path: &Path, program: &Path) -> Result<(), String> {
    let mut config = match std::fs::read(path) {
        Ok(bytes) if bytes.iter().all(u8::is_ascii_whitespace) => Value::Object(BTreeMap::new()),
        Ok(bytes) => json::parse(&bytes).map_err(|e| {
            format!("{} is not valid JSON ({e}), so it was left as it is: fix or remove it, then try again", path.display())
        })?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Value::Object(BTreeMap::new()),
        Err(e) => return Err(format!("{} could not be read: {e}", path.display())),
    };
    let Value::Object(root) = &mut config else {
        return Err(format!("{} does not hold settings Tequila recognises, so it was left as it is", path.display()));
    };
    let servers = root.entry("mcpServers".to_string()).or_insert_with(|| Value::Object(BTreeMap::new()));
    let Value::Object(servers) = servers else {
        return Err(format!("{}: its mcpServers is not a list of servers, so it was left as it is", path.display()));
    };
    let mut entry = BTreeMap::new();
    entry.insert("command".to_string(), Value::String(program.to_string_lossy().into_owned()));
    entry.insert("args".to_string(), Value::Array(vec![Value::String("--mcp".into())]));
    servers.insert(SERVER.to_string(), Value::Object(entry));
    // Freeshot, as the application was called before, listed itself as
    // "freeshot": that entry starts a program the rename leaves behind.
    let older = servers.get("freeshot").and_then(|s| s.get("command")).and_then(Value::as_str).is_some_and(|command| {
        // The program's name, whichever system's separators the path uses:
        // Path splits only at its own system's.
        let file = command.rsplit(['/', '\\']).next().unwrap_or(command).to_ascii_lowercase();
        let program = file.strip_suffix(".exe").unwrap_or(&file);
        program == "freeshot-cli" || program == "freeshot"
    });
    if older {
        servers.remove("freeshot");
    }

    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    }
    let backup = path.with_extension("json.before-tequila");
    if path.exists() && !backup.exists() {
        let _ = std::fs::copy(path, &backup);
    }
    let partial = path.with_extension("json.partial");
    std::fs::write(&partial, tequila_format::json_write::to_string(&config)).map_err(|e| format!("{}: {e}", partial.display()))?;
    std::fs::rename(&partial, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// The Claude skill shipped beside the application, which teaches Claude to
/// work in Tequila well: upload it to a Claude account.
pub fn skill_file() -> Option<PathBuf> {
    crate::beside_application("Tequila-Claude-Skill.zip")
}

/// Claude Code's own skills folder, `~/.claude/skills`.
pub fn claude_code_skills() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(PathBuf::from(home).join(".claude").join("skills"))
}

/// The skill ZIP's `tequila` folder unpacked into `skills` - Claude Code's
/// skills folder - over an earlier version's files. Every entry must lie in
/// that one folder; nothing is written otherwise. Answers where it went.
pub fn install_skill(zip: &Path, skills: &Path) -> Result<PathBuf, String> {
    let bytes = std::fs::read(zip).map_err(|e| format!("{}: {e}", zip.display()))?;
    let archive = tequila_format::zip::ZipArchive::read(&bytes).map_err(|e| format!("{} is not the skill's ZIP: {e:?}", zip.display()))?;
    let mut files = Vec::new();
    for name in archive.names() {
        if name.ends_with('/') {
            continue;
        }
        let parts: Vec<&str> = name.split('/').collect();
        let safe = parts.len() > 1 && parts[0] == "tequila" && parts.iter().all(|p| !p.is_empty() && *p != "." && *p != ".." && !p.contains(':') && !p.contains('\\'));
        if !safe {
            return Err(format!("{} holds `{name}`, outside the skill's folder", zip.display()));
        }
        files.push((parts.iter().fold(skills.to_path_buf(), |path, part| path.join(part)), archive.get(name).unwrap_or_default()));
    }
    if !files.iter().any(|(path, _)| path == &skills.join("tequila").join("SKILL.md")) {
        return Err(format!("{} holds no SKILL.md", zip.display()));
    }
    for (path, bytes) in files {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(skills.join("tequila"))
}

/// What Claude Code's `claude mcp add` needs, to copy.
pub fn claude_code_command(program: &Path) -> String {
    format!("claude mcp add {SERVER} -- \"{}\" --mcp", program.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The skill unpacks into Claude Code's skills folder as `tequila`,
    /// over an older copy; a ZIP with anything outside that folder, or no
    /// SKILL.md, writes nothing.
    #[test]
    fn the_skill_installs_for_claude_code() {
        let root = std::env::temp_dir().join(format!("tequila-skill-install-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let skills = root.join("skills");
        std::fs::create_dir_all(skills.join("tequila")).unwrap();
        std::fs::write(skills.join("tequila").join("SKILL.md"), "old").unwrap();
        let zip = |entries: &[(&str, &str)]| {
            let mut writer = tequila_format::zip::ZipWriter::new();
            for (name, text) in entries {
                writer.add(*name, text.as_bytes().to_vec());
            }
            let path = root.join(format!("skill-{}.zip", entries.len()));
            std::fs::write(&path, writer.finish()).unwrap();
            path
        };
        let good = zip(&[("tequila/SKILL.md", "new"), ("tequila/reference/scene.md", "scene")]);
        assert_eq!(install_skill(&good, &skills).expect("installs"), skills.join("tequila"));
        assert_eq!(std::fs::read_to_string(skills.join("tequila").join("SKILL.md")).unwrap(), "new");
        assert!(skills.join("tequila").join("reference").join("scene.md").is_file());
        for bad in [
            zip(&[("tequila/SKILL.md", "x"), ("tequila/../../escape.md", "x"), ("a", "b")]),
            zip(&[("other/SKILL.md", "x")]),
            zip(&[("tequila/notes.md", "x"), ("tequila/more.md", "x"), ("tequila/x.md", "x"), ("tequila/y.md", "x")]),
        ] {
            assert!(install_skill(&bad, &skills).is_err(), "{}", bad.display());
        }
        assert!(!root.join("escape.md").exists() && !skills.join("other").exists() && !skills.join("tequila").join("notes.md").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("tequila-connect-claude-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder.join("claude_desktop_config.json")
    }

    /// Freeshot's entry - the application before its rename - goes when
    /// Tequila is connected; a server of someone else's called that stays.
    #[test]
    fn connecting_replaces_freeshots_entry() {
        let path = scratch("older");
        std::fs::write(&path, r#"{"mcpServers":{"freeshot":{"command":"C:\\Users\\someone\\AppData\\Local\\Programs\\Freeshot\\freeshot-cli.exe","args":["--mcp"]}}}"#).unwrap();
        connect(&path, Path::new(r"C:\Programs\Tequila\tequila-cli.exe")).unwrap();
        let config = json::parse(&std::fs::read(&path).unwrap()).unwrap();
        let servers = config.get("mcpServers").unwrap();
        assert!(servers.get("freeshot").is_none() && servers.get(SERVER).is_some());
        std::fs::write(&path, r#"{"mcpServers":{"freeshot":{"command":"npx","args":["freeshot-mcp"]}}}"#).unwrap();
        connect(&path, Path::new(r"C:\Programs\Tequila\tequila-cli.exe")).unwrap();
        let config = json::parse(&std::fs::read(&path).unwrap()).unwrap();
        assert!(config.get("mcpServers").unwrap().get("freeshot").is_some(), "someone else's server went");
    }

    /// Connecting adds Tequila and keeps the person's other servers and
    /// settings; doing it twice changes nothing more; the original is kept.
    #[test]
    fn connecting_keeps_everything_else() {
        let path = scratch("keeps");
        std::fs::write(
            &path,
            r#"{"globalShortcut":"Ctrl+Space","mcpServers":{"files":{"command":"npx","args":["-y","@modelcontextprotocol/server-filesystem","C:/work"]}}}"#,
        )
        .unwrap();
        let program = Path::new(r"C:\Users\someone\AppData\Local\Programs\Tequila\tequila-cli.exe");
        assert!(!is_connected(&path, program));
        connect(&path, program).unwrap();
        assert!(is_connected(&path, program));
        connect(&path, program).unwrap();
        let config = json::parse(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(config.get("globalShortcut").and_then(Value::as_str), Some("Ctrl+Space"));
        let servers = config.get("mcpServers").unwrap();
        assert_eq!(servers.get("files").and_then(|s| s.get("command")).and_then(Value::as_str), Some("npx"));
        assert_eq!(servers.get("tequila").and_then(|s| s.get("args")).unwrap().as_array()[0].as_str(), Some("--mcp"));
        assert!(path.with_extension("json.before-tequila").exists(), "the original was not kept");
    }

    /// No settings file yet: one is made. A broken one is left alone.
    #[test]
    fn a_missing_file_is_made_and_a_broken_one_left_alone() {
        let path = scratch("missing");
        let _ = std::fs::remove_file(&path);
        let program = Path::new("/Applications/Tequila.app/Contents/MacOS/tequila-cli");
        connect(&path, program).unwrap();
        assert!(is_connected(&path, program));

        let broken = scratch("broken");
        std::fs::write(&broken, "{ not json").unwrap();
        let error = connect(&broken, program).unwrap_err();
        assert!(error.contains("left as it is"), "{error}");
        assert_eq!(std::fs::read_to_string(&broken).unwrap(), "{ not json", "a broken settings file was changed");
    }
}
