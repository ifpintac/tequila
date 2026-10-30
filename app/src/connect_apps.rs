//! Connecting the AI apps on this computer to Tequila in one click.
//!
//! An app that speaks MCP reaches tools on this computer through the servers
//! listed in its settings. Tequila's server is built into its command-line
//! program (`tequila-cli --mcp`). Listing it took a hand-edited settings
//! file, different for each app, and before it was built in, Python - so on
//! a work PC the Claude app had no Tequila tools at all, and asked to set up
//! materials it drove the screen with its mouse instead. Here the entry is
//! written for the person, keeping everything else in the file.

use tequila_format::json::{self, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The name Tequila's entry goes under.
const SERVER: &str = "tequila";

/// An AI app that can start Tequila's MCP server itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum App {
    Claude,
    ClaudeCode,
    Codex,
    Cursor,
    VsCode,
    Windsurf,
    GeminiCli,
    LmStudio,
}

/// How an app is told about a server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Setup {
    /// A JSON settings file listing servers under `key`. A `typed` entry
    /// says it talks over standard input and output, as VS Code's must.
    Json { key: &'static str, typed: bool },
    /// Codex's `config.toml`, a table to each server.
    Toml,
    /// A command, for a terminal where the app runs.
    Command,
}

impl App {
    pub const ALL: [App; 8] = [
        App::Claude,
        App::Cursor,
        App::VsCode,
        App::Codex,
        App::GeminiCli,
        App::Windsurf,
        App::LmStudio,
        App::ClaudeCode,
    ];

    pub fn name(self) -> &'static str {
        match self {
            App::Claude => "Claude",
            App::ClaudeCode => "Claude Code",
            App::Codex => "Codex (OpenAI)",
            App::Cursor => "Cursor",
            App::VsCode => "VS Code (GitHub Copilot)",
            App::Windsurf => "Windsurf",
            App::GeminiCli => "Gemini CLI",
            App::LmStudio => "LM Studio",
        }
    }

    /// What to do once it is connected.
    pub fn after(self) -> &'static str {
        match self {
            App::Claude => "Quit the Claude app completely (right-click its icon by the clock, Quit) and open it again.",
            App::ClaudeCode => "Paste it into a terminal where Claude Code runs.",
            App::Codex => "New Codex sessions have Tequila's tools; /mcp lists them.",
            App::Cursor => "Cursor lists Tequila in Settings, MCP; turn it on there if it asks.",
            App::VsCode => "In Copilot Chat's agent mode, Tequila's tools are in the tools list; start the server when it asks.",
            App::Windsurf => "Refresh Windsurf's MCP servers (Cascade, the hammer) or restart it.",
            App::GeminiCli => "New Gemini CLI sessions have Tequila's tools; /mcp lists them.",
            App::LmStudio => "In a chat, turn on the tequila tools (the plug), with a model that can call tools.",
        }
    }

    fn setup(self) -> Setup {
        match self {
            App::VsCode => Setup::Json { key: "servers", typed: true },
            App::Codex => Setup::Toml,
            App::ClaudeCode => Setup::Command,
            _ => Setup::Json { key: "mcpServers", typed: false },
        }
    }

    /// The settings files Tequila is listed in: for most apps one, for the
    /// Claude app every copy it keeps.
    pub fn settings(self) -> Vec<PathBuf> {
        let Some(home) = home() else { return Vec::new() };
        match self {
            App::Claude => claude_config_paths(),
            App::ClaudeCode => Vec::new(),
            App::Codex => vec![home.join(".codex").join("config.toml")],
            App::Cursor => vec![home.join(".cursor").join("mcp.json")],
            App::VsCode => vs_code_users(true).into_iter().map(|user| user.join("mcp.json")).collect(),
            App::Windsurf => vec![home.join(".codeium").join("windsurf").join("mcp_config.json")],
            App::GeminiCli => vec![home.join(".gemini").join("settings.json")],
            App::LmStudio => vec![home.join(".lmstudio").join("mcp.json")],
        }
    }

    /// Whether the app looks installed: the folder it keeps its settings in
    /// is there.
    pub fn installed(self) -> bool {
        match self {
            App::Claude => claude_config_paths().iter().any(|p| p.parent().is_some_and(Path::is_dir)),
            App::ClaudeCode => home().is_some_and(|h| h.join(".claude").is_dir()),
            App::VsCode => vs_code_users(false).iter().any(|user| user.is_dir()),
            _ => self.settings().iter().any(|p| p.parent().is_some_and(Path::is_dir)),
        }
    }

    /// The command to paste, for an app set up from a terminal.
    pub fn command(self, program: &Path) -> Option<String> {
        (self.setup() == Setup::Command).then(|| format!("claude mcp add {SERVER} -- \"{}\" --mcp", program.display()))
    }

    /// Where the app keeps Agent Skills (a folder each, with a SKILL.md),
    /// for those that read them from a folder.
    pub fn skills(self) -> Option<PathBuf> {
        let home = home()?;
        match self {
            App::ClaudeCode => Some(home.join(".claude").join("skills")),
            App::Codex => Some(home.join(".codex").join("skills")),
            _ => None,
        }
    }

    /// Whether its settings at `path` list Tequila, pointing at `program`.
    pub fn is_connected(self, path: &Path, program: &Path) -> bool {
        match self.setup() {
            Setup::Json { key, .. } => json_lists(path, key, program),
            Setup::Toml => toml_lists(path, program),
            Setup::Command => false,
        }
    }

    /// List Tequila in its settings at `path`, serving through `program`.
    pub fn connect(self, path: &Path, program: &Path) -> Result<(), String> {
        match self.setup() {
            Setup::Json { key, typed } => connect_json(path, key, typed, program),
            Setup::Toml => connect_toml(path, program),
            Setup::Command => Err(format!("{} is set up with a command, not a file", self.name())),
        }
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from)
}

/// Where the Claude app keeps its settings on this computer: every folder
/// it has made, or where it would make one. The app installed from the
/// Microsoft Store keeps its own copy, inside its package's folder.
fn claude_config_paths() -> Vec<PathBuf> {
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

/// VS Code's user settings folders that exist, the release's and the
/// Insiders build's; with `or_release`, the release's when neither does.
fn vs_code_users(or_release: bool) -> Vec<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        home().map(|h| h.join("Library").join("Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| home().map(|h| h.join(".config")))
    };
    let Some(base) = base else { return Vec::new() };
    let all: Vec<PathBuf> = ["Code", "Code - Insiders"].iter().map(|name| base.join(name).join("User")).collect();
    let found: Vec<PathBuf> = all.iter().filter(|user| user.is_dir()).cloned().collect();
    if found.is_empty() && or_release {
        all.into_iter().take(1).collect()
    } else {
        found
    }
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

/// What any other MCP app needs, in the settings most of them read.
pub fn generic_settings(program: &Path) -> String {
    let mut entry = BTreeMap::new();
    entry.insert("command".to_string(), Value::String(program.to_string_lossy().into_owned()));
    entry.insert("args".to_string(), Value::Array(vec![Value::String("--mcp".into())]));
    let servers = BTreeMap::from([(SERVER.to_string(), Value::Object(entry))]);
    tequila_format::json_write::to_string(&Value::Object(BTreeMap::from([("mcpServers".to_string(), Value::Object(servers))])))
}

fn json_lists(path: &Path, key: &str, program: &Path) -> bool {
    let Ok(bytes) = std::fs::read(path) else { return false };
    let Ok(config) = json::parse(&bytes) else { return false };
    config
        .get(key)
        .and_then(|servers| servers.get(SERVER))
        .and_then(|server| server.get("command"))
        .and_then(Value::as_str)
        .is_some_and(|command| Path::new(command) == program)
}

/// List Tequila under `key` in the JSON settings at `path`, keeping every
/// other setting and server. A file that is not JSON - one with comments,
/// say - is left alone and reported rather than replaced; the first change
/// keeps the original beside it as `.before-tequila`.
fn connect_json(path: &Path, key: &str, typed: bool, program: &Path) -> Result<(), String> {
    let mut config = match std::fs::read(path) {
        Ok(bytes) if bytes.iter().all(u8::is_ascii_whitespace) => Value::Object(BTreeMap::new()),
        Ok(bytes) => json::parse(&bytes).map_err(|e| {
            format!("{} is not plain JSON ({e}), so it was left as it is: add Tequila by hand, or fix it and try again", path.display())
        })?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Value::Object(BTreeMap::new()),
        Err(e) => return Err(format!("{} could not be read: {e}", path.display())),
    };
    let Value::Object(root) = &mut config else {
        return Err(format!("{} does not hold settings Tequila recognises, so it was left as it is", path.display()));
    };
    let servers = root.entry(key.to_string()).or_insert_with(|| Value::Object(BTreeMap::new()));
    let Value::Object(servers) = servers else {
        return Err(format!("{}: its {key} is not a list of servers, so it was left as it is", path.display()));
    };
    let mut entry = BTreeMap::new();
    if typed {
        entry.insert("type".to_string(), Value::String("stdio".into()));
    }
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
    write_keeping_original(path, tequila_format::json_write::to_string(&config).as_bytes())
}

/// The table Codex reads a server from.
const TOML_TABLE: &str = "[mcp_servers.tequila]";

fn toml_lists(path: &Path, program: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|text| text.contains(&toml_entry(program)))
}

/// Tequila's table, the path in a literal string so its backslashes stay
/// as they are.
fn toml_entry(program: &Path) -> String {
    format!("{TOML_TABLE}\ncommand = '{}'\nargs = [\"--mcp\"]\n", program.display())
}

/// List Tequila in Codex's `config.toml` at `path`: its table replaced if
/// it is there, added at the end if not, and nothing else changed.
fn connect_toml(path: &Path, program: &Path) -> Result<(), String> {
    if program.to_string_lossy().contains('\'') {
        return Err(format!("{} has a quote in it, which Codex's settings cannot hold as written here", program.display()));
    }
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{} could not be read: {e}", path.display())),
    };
    let mut kept: Vec<&str> = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let header = line.trim_start().starts_with('[');
        if header {
            inside = line.trim() == TOML_TABLE || line.trim().starts_with("[mcp_servers.tequila.");
        }
        if !inside {
            kept.push(line);
        }
    }
    let mut out = kept.join("\n").trim_end().to_string();
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(&toml_entry(program));
    write_keeping_original(path, out.as_bytes())
}

/// Write `bytes` to `path` whole, through a file beside it, keeping the
/// original beside it the first time as `<name>.before-tequila`.
fn write_keeping_original(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let backup = path.with_file_name(format!("{name}.before-tequila"));
    if path.exists() && !backup.exists() {
        let _ = std::fs::copy(path, &backup);
    }
    let partial = path.with_file_name(format!("{name}.partial"));
    std::fs::write(&partial, bytes).map_err(|e| format!("{}: {e}", partial.display()))?;
    std::fs::rename(&partial, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Tequila's skill, shipped beside the application: what an AI needs to
/// work in Tequila well, in the Agent Skills format the Claude app, Claude
/// Code, Codex and others read.
pub fn skill_file() -> Option<PathBuf> {
    crate::beside_application("Tequila-Skill.zip").or_else(|| crate::beside_application("Tequila-Claude-Skill.zip"))
}

/// The skill ZIP's `tequila` folder unpacked into `skills` - an app's skills
/// folder - over an earlier version's files. Every entry must lie in that
/// one folder; nothing is written otherwise. Answers where it went.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The skill unpacks into an app's skills folder as `tequila`, over an
    /// older copy; a ZIP with anything outside that folder, or no SKILL.md,
    /// writes nothing.
    #[test]
    fn the_skill_installs_into_a_skills_folder() {
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

    fn scratch(name: &str, file: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("tequila-connect-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder.join(file)
    }

    const PROGRAM: &str = r"C:\Users\someone\AppData\Local\Programs\Tequila\tequila-cli.exe";

    /// Freeshot's entry - the application before its rename - goes when
    /// Tequila is connected; a server of someone else's called that stays.
    #[test]
    fn connecting_replaces_freeshots_entry() {
        let path = scratch("older", "claude_desktop_config.json");
        std::fs::write(&path, r#"{"mcpServers":{"freeshot":{"command":"C:\\Users\\someone\\AppData\\Local\\Programs\\Freeshot\\freeshot-cli.exe","args":["--mcp"]}}}"#).unwrap();
        App::Claude.connect(&path, Path::new(PROGRAM)).unwrap();
        let config = json::parse(&std::fs::read(&path).unwrap()).unwrap();
        let servers = config.get("mcpServers").unwrap();
        assert!(servers.get("freeshot").is_none() && servers.get(SERVER).is_some());
        std::fs::write(&path, r#"{"mcpServers":{"freeshot":{"command":"npx","args":["freeshot-mcp"]}}}"#).unwrap();
        App::Claude.connect(&path, Path::new(PROGRAM)).unwrap();
        let config = json::parse(&std::fs::read(&path).unwrap()).unwrap();
        assert!(config.get("mcpServers").unwrap().get("freeshot").is_some(), "someone else's server went");
    }

    /// Connecting adds Tequila and keeps the person's other servers and
    /// settings - in the Claude app's file, Cursor's, Gemini CLI's - doing
    /// it twice changes nothing more, and the original is kept.
    #[test]
    fn connecting_keeps_everything_else() {
        for (app, file) in [(App::Claude, "claude_desktop_config.json"), (App::Cursor, "mcp.json"), (App::GeminiCli, "settings.json")] {
            let path = scratch(&format!("keeps-{app:?}"), file);
            std::fs::write(
                &path,
                r#"{"globalShortcut":"Ctrl+Space","mcpServers":{"files":{"command":"npx","args":["-y","@modelcontextprotocol/server-filesystem","C:/work"]}}}"#,
            )
            .unwrap();
            let program = Path::new(PROGRAM);
            assert!(!app.is_connected(&path, program));
            app.connect(&path, program).unwrap();
            assert!(app.is_connected(&path, program), "{app:?}");
            app.connect(&path, program).unwrap();
            let config = json::parse(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(config.get("globalShortcut").and_then(Value::as_str), Some("Ctrl+Space"));
            let servers = config.get("mcpServers").unwrap();
            assert_eq!(servers.get("files").and_then(|s| s.get("command")).and_then(Value::as_str), Some("npx"));
            assert_eq!(servers.get("tequila").and_then(|s| s.get("args")).unwrap().as_array()[0].as_str(), Some("--mcp"));
            assert!(path.with_file_name(format!("{file}.before-tequila")).exists(), "the original was not kept");
        }
    }

    /// VS Code lists servers under `servers`, each saying how it is
    /// reached.
    #[test]
    fn vs_code_is_listed_as_it_reads_servers() {
        let path = scratch("vscode", "mcp.json");
        std::fs::write(&path, r#"{"servers":{"github":{"type":"http","url":"https://api.githubcopilot.com/mcp/"}},"inputs":[]}"#).unwrap();
        App::VsCode.connect(&path, Path::new(PROGRAM)).unwrap();
        assert!(App::VsCode.is_connected(&path, Path::new(PROGRAM)));
        let config = json::parse(&std::fs::read(&path).unwrap()).unwrap();
        let tequila = config.get("servers").and_then(|s| s.get("tequila")).unwrap();
        assert_eq!(tequila.get("type").and_then(Value::as_str), Some("stdio"));
        assert!(config.get("servers").and_then(|s| s.get("github")).is_some() && config.get("inputs").is_some());
        assert!(config.get("mcpServers").is_none());
    }

    /// Codex's TOML gets Tequila's table at the end, or in place of an
    /// earlier one, with the rest of the file as it was.
    #[test]
    fn codex_gets_a_table_of_its_own() {
        let path = scratch("codex", "config.toml");
        App::Codex.connect(&path, Path::new(PROGRAM)).unwrap();
        assert!(App::Codex.is_connected(&path, Path::new(PROGRAM)));
        let fresh = std::fs::read_to_string(&path).unwrap();
        assert!(fresh.contains(&format!("command = '{PROGRAM}'")), "{fresh}");

        std::fs::write(
            &path,
            "model = \"gpt-5\"\n\n[mcp_servers.tequila]\ncommand = 'C:\\old\\tequila-cli.exe'\nargs = [\"--mcp\"]\n\n[mcp_servers.tequila.env]\nX = \"1\"\n\n[mcp_servers.files]\ncommand = \"npx\"\n",
        )
        .unwrap();
        App::Codex.connect(&path, Path::new(PROGRAM)).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("model = \"gpt-5\""), "{text}");
        assert!(text.contains("[mcp_servers.files]\ncommand = \"npx\""), "{text}");
        assert!(!text.contains("old") && !text.contains("X = \"1\""), "the earlier table stayed: {text}");
        assert_eq!(text.matches(TOML_TABLE).count(), 1, "{text}");
        assert!(App::Codex.is_connected(&path, Path::new(PROGRAM)));
    }

    /// No settings file yet: one is made. A broken one, or one with
    /// comments, is left alone.
    #[test]
    fn a_missing_file_is_made_and_a_broken_one_left_alone() {
        let path = scratch("missing", "mcp.json");
        let _ = std::fs::remove_file(&path);
        let program = Path::new("/Applications/Tequila.app/Contents/MacOS/tequila-cli");
        App::Cursor.connect(&path, program).unwrap();
        assert!(App::Cursor.is_connected(&path, program));

        for (name, text) in [("broken", "{ not json"), ("commented", "// my servers\n{\"servers\": {}}")] {
            let broken = scratch(name, "mcp.json");
            std::fs::write(&broken, text).unwrap();
            let error = App::VsCode.connect(&broken, program).unwrap_err();
            assert!(error.contains("left as it is"), "{error}");
            assert_eq!(std::fs::read_to_string(&broken).unwrap(), text, "a settings file it could not read was changed");
        }
    }

    /// Every app either has a settings file to write or a command to give,
    /// and the settings for any other app are what they read.
    #[test]
    fn every_app_has_a_way_in() {
        let program = Path::new(PROGRAM);
        for app in App::ALL {
            assert!(!app.settings().is_empty() || app.command(program).is_some(), "{app:?} has no way to be connected");
        }
        let generic = json::parse(generic_settings(program).as_bytes()).unwrap();
        assert_eq!(generic.get("mcpServers").and_then(|s| s.get("tequila")).and_then(|t| t.get("command")).and_then(Value::as_str), Some(PROGRAM));
    }
}
