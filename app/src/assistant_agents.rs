//! The agents the Assistant can talk to over the Agent Client Protocol, and
//! how each is found, installed, started and signed in to on this computer.
//!
//! An agent is a program somebody else makes: Claude through Anthropic's
//! Agent SDK and its `claude-agent-acp` adapter. Tequila does not ship it.
//! It uses the one already on the computer, or installs it - into Tequila's
//! own `agents` folder, with the computer's npm - when the person asks.
//! Signing in is done in the agent's own program, in a terminal window:
//! Tequila starts that program and never sees a login.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use tequila_ai::acp;
use tequila_format::json::Value;
use tequila_format::json_write::Object;

/// An agent Tequila knows how to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Claude,
}

impl Kind {
    pub const ALL: [Kind; 1] = [Kind::Claude];

    /// Its name in the settings and in a chat's file.
    pub fn id(self) -> &'static str {
        match self {
            Kind::Claude => "agent:claude",
        }
    }

    pub fn from_id(id: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|kind| kind.id() == id)
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Claude => "Claude",
        }
    }

    /// The npm package its adapter comes in.
    fn package(self) -> &'static str {
        match self {
            Kind::Claude => "@agentclientprotocol/claude-agent-acp",
        }
    }

    /// The adapter's program, as npm names it.
    fn program(self) -> &'static str {
        match self {
            Kind::Claude => "claude-agent-acp",
        }
    }
}

/// How to start an agent: the program, and what goes before its own
/// arguments (the script, for one that runs on Node).
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// Why an agent cannot be started yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Missing {
    /// The adapter is not on this computer. Whether Node - and so npm - is
    /// there to install it with.
    Adapter { node: bool },
}

/// Tequila's own folder for agents it installed.
pub fn folder() -> PathBuf {
    tequila_format::library::home_directory().join("agents")
}

/// Where programs are looked for: the `PATH` Tequila was started with, and
/// the places Node and its programs are usually put. An application opened
/// from the Finder or the Start menu has a short `PATH` that leaves most of
/// them out.
fn places() -> Vec<PathBuf> {
    let mut places: Vec<PathBuf> = std::env::var_os("PATH").map(|path| std::env::split_paths(&path).collect()).unwrap_or_default();
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    if cfg!(windows) {
        places.extend(std::env::var_os("APPDATA").map(|roaming| PathBuf::from(roaming).join("npm")));
        places.extend(std::env::var_os("ProgramFiles").map(|programs| PathBuf::from(programs).join("nodejs")));
    } else {
        places.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].map(PathBuf::from));
        if let Some(home) = &home {
            places.extend([".volta/bin", ".local/bin", ".npm-global/bin", ".bun/bin"].map(|sub| home.join(sub)));
            // nvm keeps a Node per version; the newest is as good a guess
            // as any.
            if let Ok(versions) = std::fs::read_dir(home.join(".nvm/versions/node")) {
                let mut versions: Vec<PathBuf> = versions.flatten().map(|entry| entry.path().join("bin")).collect();
                versions.sort();
                places.extend(versions.into_iter().rev());
            }
        }
    }
    let mut seen = Vec::new();
    places.retain(|place| {
        let new = !seen.contains(place);
        seen.push(place.clone());
        new
    });
    places
}

fn find(names: &[&str], places: &[PathBuf]) -> Option<PathBuf> {
    places.iter().flat_map(|place| names.iter().map(move |name| place.join(name))).find(|path| path.is_file())
}

fn node(places: &[PathBuf]) -> Option<PathBuf> {
    find(if cfg!(windows) { &["node.exe"] } else { &["node"] }, places)
}

/// A `PATH` for a program Tequila starts: the one it has, with the usual
/// places added, so the agent finds Node and whatever else it runs.
fn path_for_children(places: &[PathBuf]) -> Option<(String, String)> {
    let joined = std::env::join_paths(places).ok()?;
    Some(("PATH".to_string(), joined.to_string_lossy().into_owned()))
}

/// The agent's program on this computer: a stand-in named for a test, the
/// one Tequila installed, or one the person installed themselves.
pub fn find_agent(kind: Kind) -> Result<Found, Missing> {
    find_agent_in(kind, &folder(), &places(), std::env::var_os("TEQUILA_ASSISTANT_AGENT").map(PathBuf::from))
}

fn find_agent_in(kind: Kind, folder: &Path, places: &[PathBuf], stand_in: Option<PathBuf>) -> Result<Found, Missing> {
    let env: Vec<(String, String)> = path_for_children(places).into_iter().collect();
    // For tests and for looking at the panel without an account: any
    // program that speaks the protocol, with its arguments after it.
    if let Some(program) = stand_in {
        let args = std::env::var("TEQUILA_ASSISTANT_AGENT_ARGS").map(|args| args.split('\n').filter(|a| !a.is_empty()).map(str::to_string).collect()).unwrap_or_default();
        return Ok(Found { program, args, env });
    }
    let node = node(places);
    let script = |modules: &Path| modules.join(kind.package()).join("dist").join("index.js");
    // Tequila's own install, then a global npm install on Windows, where
    // the program on the path is a batch file around the same script.
    let mut scripts = vec![script(&folder.join("node_modules"))];
    if cfg!(windows) {
        scripts.extend(places.iter().map(|place| script(&place.join("node_modules"))));
    }
    if let (Some(node), Some(script)) = (&node, scripts.iter().find(|s| s.is_file())) {
        return Ok(Found { program: node.clone(), args: vec![script.to_string_lossy().into_owned()], env });
    }
    // One on the path: a link to its script, run with Node, or a program
    // that stands by itself (the adapter's own releases are that).
    let name = if cfg!(windows) { format!("{}.exe", kind.program()) } else { kind.program().to_string() };
    if let Some(program) = find(&[name.as_str()], places) {
        let real = program.canonicalize().unwrap_or(program);
        return match (real.extension().is_some_and(|e| e == "js" || e == "mjs"), &node) {
            (true, Some(node)) => Ok(Found { program: node.clone(), args: vec![real.to_string_lossy().into_owned()], env }),
            (true, None) => Err(Missing::Adapter { node: false }),
            (false, _) => Ok(Found { program: real, args: Vec::new(), env }),
        };
    }
    Err(Missing::Adapter { node: node.is_some() })
}

/// Install the agent's adapter into Tequila's own folder with the
/// computer's npm, on a thread of its own. The answer is what to say.
pub fn install(kind: Kind) -> Receiver<Result<String, String>> {
    let (sender, receiver) = channel();
    std::thread::spawn(move || {
        let _ = sender.send(install_now(kind, &folder(), &places()));
    });
    receiver
}

fn install_now(kind: Kind, folder: &Path, places: &[PathBuf]) -> Result<String, String> {
    let npm = find(if cfg!(windows) { &["npm.cmd"] } else { &["npm"] }, places)
        .ok_or("Node.js is not on this computer. Install it from nodejs.org, then try again.")?;
    std::fs::create_dir_all(folder).map_err(|e| format!("{} could not be made: {e}", folder.display()))?;
    let mut command = std::process::Command::new(npm);
    command.arg("install").arg("--prefix").arg(folder).arg("--no-fund").arg("--no-audit").arg(kind.package());
    if let Some((name, value)) = path_for_children(places) {
        command.env(name, value);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let output = command.output().map_err(|e| format!("npm could not be started: {e}"))?;
    if output.status.success() {
        Ok(format!("{} is installed.", kind.name()))
    } else {
        let said = String::from_utf8_lossy(&output.stderr);
        let last: Vec<&str> = said.lines().filter(|l| !l.trim().is_empty()).rev().take(4).collect();
        Err(format!("npm could not install it: {}", last.into_iter().rev().collect::<Vec<_>>().join(" ")))
    }
}

/// Start one of the agent's own sign-ins in a terminal window. The window
/// is the agent's: what is typed there goes to it, not to Tequila.
pub fn sign_in(found: &Found, way: &acp::SignIn) -> Result<(), String> {
    let Some((args, env)) = &way.terminal else {
        return Err("this sign-in is not one that runs in a terminal".into());
    };
    let mut words = vec![found.program.to_string_lossy().into_owned()];
    words.extend(found.args.iter().cloned());
    words.extend(args.iter().cloned());
    let mut env_all = found.env.clone();
    env_all.extend(env.iter().cloned());
    open_terminal(&words, &env_all)
}

/// A word as a shell takes it whole.
fn quoted(word: &str) -> String {
    format!("'{}'", word.replace('\'', "'\\''"))
}

#[cfg(target_os = "macos")]
fn open_terminal(words: &[String], env: &[(String, String)]) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    // A `.command` file is what Terminal runs when it is opened.
    let script = std::env::temp_dir().join(format!("tequila-sign-in-{}.command", std::process::id()));
    let mut text = String::from("#!/bin/sh\n");
    for (name, value) in env {
        text.push_str(&format!("export {name}={}\n", quoted(value)));
    }
    text.push_str(&words.iter().map(|w| quoted(w)).collect::<Vec<_>>().join(" "));
    text.push_str("\necho\necho 'You can close this window and go back to Tequila.'\n");
    std::fs::write(&script, text).map_err(|e| e.to_string())?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    std::process::Command::new("open").arg(&script).spawn().map(|_| ()).map_err(|e| format!("Terminal could not be opened: {e}"))
}

#[cfg(windows)]
fn open_terminal(words: &[String], env: &[(String, String)]) -> Result<(), String> {
    // `start` opens a console window of its own; `/k` leaves it open on
    // what the sign-in said.
    let line = words.iter().map(|w| format!("\"{w}\"")).collect::<Vec<_>>().join(" ");
    let mut command = std::process::Command::new("cmd");
    command.args(["/c", "start", "Sign in", "cmd", "/k", &line]);
    for (name, value) in env {
        command.env(name, value);
    }
    command.spawn().map(|_| ()).map_err(|e| format!("a console could not be opened: {e}"))
}

#[cfg(not(any(target_os = "macos", windows)))]
fn open_terminal(words: &[String], env: &[(String, String)]) -> Result<(), String> {
    let line = words.iter().map(|w| quoted(w)).collect::<Vec<_>>().join(" ");
    for terminal in ["x-terminal-emulator", "gnome-terminal", "konsole", "xterm"] {
        let mut command = std::process::Command::new(terminal);
        command.args(["-e", "sh", "-c", &format!("{line}; echo; echo 'You can close this window and go back to Tequila.'; read _")]);
        for (name, value) in env {
            command.env(name, value);
        }
        if command.spawn().is_ok() {
            return Ok(());
        }
    }
    Err(format!("No terminal could be opened. In one, run: {line}"))
}

/// What an agent is told about where it is, on top of its own instructions
/// and the ones Tequila's tools come with.
pub const INSTRUCTIONS: &str = "You are the assistant built into Tequila, a product-rendering application, and you work on the scene \
that is open in it right now. The person is a designer, not a programmer: answer in plain words, briefly, and do what they ask in the \
scene rather than describing how. Work through the tequila tools. Scene changes happen at once and the person can undo them; before \
anything that leaves the scene - importing a file, saving to a new place, rendering to a file, changing a preference, writing or running \
a plugin - Tequila asks the person itself, so call the tool and wait for its answer rather than asking first. You can read files in the \
project's folder and, when the person agrees, write there. You cannot run commands, reach the internet, or touch files anywhere else, so \
do not try. After changing how something looks, check it with tequila_screenshot. A name written as @Name in a message is a part or a \
material of the scene, and its id follows in brackets.";

/// The agent's own options for a new conversation. For Claude: its shell,
/// internet and sub-agent tools taken away, and its file-writing tools made
/// to ask every time, whatever the person's own Claude settings allow.
pub fn options(kind: Kind) -> Option<Value> {
    let words = |words: &[&str]| Value::Array(words.iter().map(|w| Value::String(w.to_string())).collect());
    match kind {
        Kind::Claude => {
            let out_of_reach = ["Bash", "BashOutput", "KillShell", "KillBash", "WebFetch", "WebSearch", "Task", "Agent", "Skill", "NotebookEdit"];
            let mut permissions = Object::new();
            permissions.set("deny", words(&out_of_reach)).set("ask", words(&["Write", "Edit", "MultiEdit"]));
            let mut settings = Object::new();
            settings.set("permissions", permissions.build());
            let mut options = Object::new();
            options.set("disallowedTools", words(&out_of_reach)).set("settings", settings.build()).set("allowDangerouslySkipPermissions", false);
            let mut claude = Object::new();
            claude.set("options", options.build());
            let mut meta = Object::new();
            meta.set("claudeCode", claude.build());
            Some(meta.build())
        }
    }
}

/// Settings of the agent's own that Tequila chooses for it, by kind and
/// value. Claude's adapter starts in a mode where Claude decides for itself
/// what needs permission; here it always asks, and Tequila answers
/// ([`tequila_ai::acp::judge`]).
pub fn pinned(kind: Kind) -> &'static [(&'static str, &'static str)] {
    match kind {
        Kind::Claude => &[("mode", "default")],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("tequila-agents-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }

    /// With neither Node nor the adapter, what is missing is said, and
    /// whether Install could work.
    #[test]
    fn a_missing_agent_says_whether_it_can_be_installed() {
        let root = scratch("missing");
        let bin = root.join("bin");
        assert_eq!(find_agent_in(Kind::Claude, &root.join("agents"), &[bin.clone()], None), Err(Missing::Adapter { node: false }));
        touch(&bin.join(if cfg!(windows) { "node.exe" } else { "node" }));
        assert_eq!(find_agent_in(Kind::Claude, &root.join("agents"), &[bin], None), Err(Missing::Adapter { node: true }));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The adapter Tequila installed is run with the computer's Node, and
    /// given a path its own programs are found on.
    #[test]
    fn the_adapter_tequila_installed_runs_on_the_computers_node() {
        let root = scratch("installed");
        let bin = root.join("bin");
        let node = bin.join(if cfg!(windows) { "node.exe" } else { "node" });
        touch(&node);
        let script = root.join("agents/node_modules/@agentclientprotocol/claude-agent-acp/dist/index.js");
        touch(&script);
        let found = find_agent_in(Kind::Claude, &root.join("agents"), &[bin.clone()], None).unwrap();
        assert_eq!((found.program, found.args), (node, vec![script.to_string_lossy().into_owned()]));
        assert_eq!(found.env[0].0, "PATH");
        assert!(found.env[0].1.contains(&*bin.to_string_lossy()));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// One the person installed themselves is found on the path: a link to
    /// its script is run with Node, a program is run as it is.
    #[cfg(unix)]
    #[test]
    fn an_adapter_the_person_installed_is_found_on_the_path() {
        let root = scratch("global");
        let bin = root.join("bin");
        let node = bin.join("node");
        touch(&node);
        let script = root.join("lib/node_modules/@agentclientprotocol/claude-agent-acp/dist/index.js");
        touch(&script);
        std::os::unix::fs::symlink(&script, bin.join("claude-agent-acp")).unwrap();
        let found = find_agent_in(Kind::Claude, &root.join("agents"), &[bin.clone()], None).unwrap();
        assert_eq!(found.program, node);
        assert_eq!(found.args, [script.canonicalize().unwrap().to_string_lossy().into_owned()]);

        std::fs::remove_file(bin.join("claude-agent-acp")).unwrap();
        touch(&bin.join("claude-agent-acp"));
        let found = find_agent_in(Kind::Claude, &root.join("agents"), &[bin.clone()], None).unwrap();
        assert_eq!((found.program, found.args.len()), (bin.join("claude-agent-acp").canonicalize().unwrap(), 0));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A stand-in named in the environment is used as it is: how the tests
    /// and the screenshots run without anybody's account.
    #[test]
    fn a_stand_in_agent_is_used_as_named() {
        let found = find_agent_in(Kind::Claude, Path::new("/nowhere"), &[], Some(PathBuf::from("/tools/stand-in"))).unwrap();
        assert_eq!(found.program, PathBuf::from("/tools/stand-in"));
    }

    #[test]
    fn claude_starts_without_a_shell_or_the_internet_and_asks_before_writing() {
        let meta = options(Kind::Claude).unwrap();
        let options = meta.get("claudeCode").unwrap().get("options").unwrap();
        let gone: Vec<&str> = options.get("disallowedTools").unwrap().as_array().iter().filter_map(Value::as_str).collect();
        for tool in ["Bash", "WebFetch", "WebSearch", "Task"] {
            assert!(gone.contains(&tool), "{tool}");
        }
        let permissions = options.get("settings").unwrap().get("permissions").unwrap();
        let ask: Vec<&str> = permissions.get("ask").unwrap().as_array().iter().filter_map(Value::as_str).collect();
        assert_eq!(ask, ["Write", "Edit", "MultiEdit"]);
        assert_eq!(options.get("allowDangerouslySkipPermissions"), Some(&Value::Bool(false)));
        assert_eq!(pinned(Kind::Claude), [("mode", "default")]);
        assert_eq!(Kind::from_id("agent:claude"), Some(Kind::Claude));
        assert_eq!(Kind::from_id("claude"), None, "a provider may be called that");
    }

    #[test]
    fn a_word_with_a_quote_in_it_stays_one_word_in_a_shell() {
        assert_eq!(quoted("/Users/o'neil/agents"), r"'/Users/o'\''neil/agents'");
    }
}
