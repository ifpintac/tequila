//! An agent reached over the Agent Client Protocol.
//!
//! The protocol (agentclientprotocol.com, version 1) is how an editor talks
//! to a coding agent it did not write: Claude through its `claude-agent-acp`
//! adapter, Codex through `codex-acp`, and others. Tequila is the client. It
//! starts the agent as a program of its own and exchanges JSON-RPC with it,
//! a message a line, over the program's standard input and output:
//!
//! 1. `initialize`, each side saying what it can do;
//! 2. `session/new` (or `session/resume`, `session/load` to carry on an
//!    earlier conversation) with the project's folder and Tequila's MCP
//!    bridge, which is where the agent gets its Tequila tools from;
//! 3. `session/prompt` for each message. While it is open the agent sends
//!    `session/update` notifications - its words and reasoning as they come,
//!    each tool call and how it went - and may ask
//!    `session/request_permission` before using a tool. Its answer ends the
//!    turn.
//!
//! Nothing here blocks. The agent's output is read on a thread of its own
//! and what it wrote is taken in by [`Agent::poll`], once a frame, which
//! hands back [`Event`]s for the panel. Signing in is the agent's own
//! business: when it says it needs it, the person is sent to its own sign-in
//! and Tequila never sees a login.

use crate::chat::Status;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use tequila_format::json::{self, Value};
use tequila_format::json_write::{self, Object};

/// The protocol's version this speaks.
const PROTOCOL: f64 = 1.0;
/// The agent's "sign in first".
const AUTH_REQUIRED: f64 = -32000.0;
/// How much of what the agent wrote to its error output is kept, to say why
/// it stopped.
const ERROR_LINES: usize = 30;

/// An MCP server the agent is to start: Tequila's bridge.
#[derive(Clone, Debug, PartialEq)]
pub struct McpServer {
    pub name: String,
    pub command: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// How to start an agent and its conversation.
#[derive(Clone, Debug, Default)]
pub struct Launch {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Added to the environment Tequila itself has.
    pub env: Vec<(String, String)>,
    /// The folder the agent works in: the open project's.
    pub folder: PathBuf,
    pub servers: Vec<McpServer>,
    /// An earlier conversation of the agent's to carry on.
    pub resume: Option<String>,
    /// What the agent is told about where it is, added to its own
    /// instructions where it allows that.
    pub instructions: String,
    /// The agent's own options for a new conversation, under `_meta`.
    pub meta: Option<Value>,
    /// Tequila's version, for the agent's records.
    pub version: String,
}

/// A way of signing in the agent offers.
#[derive(Clone, Debug, PartialEq)]
pub struct SignIn {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Run in a terminal: the agent's own program with these arguments and
    /// this environment added. `None` is a sign-in the agent carries out
    /// itself when asked ([`Agent::authenticate`]).
    pub terminal: Option<(Vec<String>, Vec<(String, String)>)>,
}

/// One of the agent's settings for the conversation: its model, how hard it
/// thinks.
#[derive(Clone, Debug, PartialEq)]
pub struct Setting {
    pub id: String,
    pub name: String,
    /// `model`, `thought_level`, `mode`, or the agent's own word.
    pub category: String,
    pub current: String,
    pub choices: Vec<Choice>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub value: String,
    pub name: String,
    pub description: String,
    /// The heading it is listed under, where the agent groups its choices.
    pub group: String,
}

impl Setting {
    /// The name of the choice in use.
    pub fn current_name(&self) -> &str {
        self.choices.iter().find(|c| c.value == self.current).map(|c| c.name.as_str()).unwrap_or(&self.current)
    }
}

/// News of a tool call: a new one, or more about one already known. What is
/// `None` has not changed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolNews {
    pub id: String,
    pub title: Option<String>,
    /// The tool's own name, where the agent gives it.
    pub name: Option<String>,
    /// `read`, `edit`, `delete`, `move`, `search`, `execute`, `think`,
    /// `fetch`, `switch_mode` or `other`.
    pub kind: Option<String>,
    pub status: Option<Status>,
    /// What it said, where it has said anything.
    pub text: Option<String>,
    /// PNG or JPEG pictures it produced.
    pub pictures: Vec<Vec<u8>>,
    /// The files it touches.
    pub paths: Vec<PathBuf>,
    /// What it was called with.
    pub input: Option<Value>,
}

/// The agent asking before it uses a tool.
#[derive(Clone, Debug, PartialEq)]
pub struct Permission {
    /// For [`Agent::permit`].
    pub id: u64,
    pub tool: ToolNews,
}

/// Why a turn ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    Done,
    /// It ran out of room for its reply.
    MaxTokens,
    /// It used as many steps as one request may.
    MaxSteps,
    Refused,
    Stopped,
    Other(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The conversation is open and a message can be sent. `resumed` when
    /// an earlier one was carried on.
    Ready { resumed: bool },
    /// The agent needs signing in first, and these are its ways.
    SignIn(Vec<SignIn>),
    /// Who the agent is signed in as, or what pays: `account` for a
    /// subscription, `api_key`, and so on, with a name to show.
    Account { kind: String, label: String },
    Said(String),
    Thought(String),
    Tool(ToolNews),
    Permission(Permission),
    /// Its plan, whole, each step with whether it is done.
    Plan(Vec<(String, Status)>),
    /// Its settings changed: read them from [`Agent::settings`].
    Settings,
    /// The agent's own title for the conversation.
    Title(String),
    /// Something worth telling the person, that is not part of the reply.
    Notice(String),
    Finished(Stop),
    /// The request failed; the conversation is still there.
    Failed(String),
    /// The agent's program ended, with what it said last.
    Exited(String),
}

/// A part of a message to the agent.
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Text(String),
    /// A PNG.
    Picture(Vec<u8>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Starting up, or opening the conversation.
    Starting,
    /// Waiting for the person to sign in.
    SignedOut,
    /// Ready for a message.
    Idle,
    /// Working on one.
    Working,
    /// The program has ended.
    Gone,
}

/// What a request of ours is, for when its answer comes.
enum Asked {
    Initialize,
    Open { resumed: bool },
    Authenticate,
    Prompt,
    Setting,
}

enum Incoming {
    Message(Value),
    /// Something that was not JSON, on the agent's output.
    Stray(String),
    Closed,
}

/// What the agent said it can do.
#[derive(Default)]
struct Abilities {
    pictures: bool,
    load: bool,
    resume: bool,
    instructions: bool,
}

/// A running agent and the conversation with it.
pub struct Agent {
    launch: Launch,
    child: Option<Child>,
    outgoing: Option<Sender<String>>,
    incoming: Receiver<Incoming>,
    errors: Arc<Mutex<VecDeque<String>>>,
    next: u64,
    asked: HashMap<u64, Asked>,
    /// The agent's questions waiting for the person: ours for each, and
    /// the agent's own id and the choices it offered.
    permissions: BTreeMap<u64, (Value, Vec<(String, String)>)>,
    next_permission: u64,
    phase: Phase,
    abilities: Abilities,
    sign_ins: Vec<SignIn>,
    session: Option<String>,
    settings: Vec<Setting>,
    /// An earlier conversation is being played back by the agent as it is
    /// loaded: the chat has it already.
    replaying: bool,
    /// The agent's name for itself.
    pub name: String,
    /// The agent's own word for why it wants signing in, where it gave
    /// one: `claude_subscription_not_supported`.
    pub signed_out_because: String,
}

/// JSON on one line, as the protocol wants it: the shared writer indents,
/// and its strings escape their own newlines, so the only ones are its own.
fn one_line(value: &Value) -> String {
    json_write::to_string(value).split('\n').map(str::trim).collect::<Vec<_>>().join(" ").trim().to_string()
}

fn word(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn text_block(text: &str) -> Value {
    let mut out = Object::new();
    out.set("type", "text").set("text", text);
    out.build()
}

impl Agent {
    /// Start the agent's program and begin opening the conversation. `wake`
    /// is called, from another thread, whenever the agent has written
    /// something: the window's cue to look.
    pub fn start(launch: Launch, wake: impl Fn() + Send + Sync + 'static) -> Result<Agent, String> {
        let mut command = Command::new(&launch.program);
        command.args(&launch.args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        for (name, value) in &launch.env {
            command.env(name, value);
        }
        if launch.folder.is_dir() {
            command.current_dir(&launch.folder);
        }
        #[cfg(windows)]
        {
            // No console window flashing up behind Tequila's.
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let mut child = command.spawn().map_err(|e| format!("{} could not be started: {e}", launch.program.display()))?;
        let (Some(stdin), Some(stdout), Some(stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take()) else {
            let _ = child.kill();
            return Err("the agent's program could not be talked to".into());
        };
        let errors = Arc::new(Mutex::new(VecDeque::new()));
        let kept = errors.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let mut kept = kept.lock().unwrap_or_else(|e| e.into_inner());
                if kept.len() == ERROR_LINES {
                    kept.pop_front();
                }
                kept.push_back(line);
            }
        });
        let mut agent = Agent::over(launch, stdout, stdin, wake);
        agent.child = Some(child);
        agent.errors = errors;
        Ok(agent)
    }

    /// The same over any two streams: what the tests talk to a stand-in
    /// agent through.
    pub fn over(launch: Launch, from_agent: impl Read + Send + 'static, to_agent: impl Write + Send + 'static, wake: impl Fn() + Send + Sync + 'static) -> Agent {
        let (lines, incoming) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(from_agent).lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                let message = match json::parse(line.as_bytes()) {
                    Ok(value) => Incoming::Message(value),
                    Err(_) => Incoming::Stray(line),
                };
                if lines.send(message).is_err() {
                    return;
                }
                wake();
            }
            let _ = lines.send(Incoming::Closed);
            wake();
        });
        // Written on a thread of its own: a message with pictures in it is
        // megabytes, and the window does not wait for the agent to read it.
        let (outgoing, writing) = channel::<String>();
        std::thread::spawn(move || {
            let mut to_agent = to_agent;
            for line in writing {
                if writeln!(to_agent, "{line}").is_err() || to_agent.flush().is_err() {
                    break;
                }
            }
        });
        let mut agent = Agent {
            launch,
            child: None,
            outgoing: Some(outgoing),
            incoming,
            errors: Arc::default(),
            next: 0,
            asked: HashMap::new(),
            permissions: BTreeMap::new(),
            next_permission: 1,
            phase: Phase::Starting,
            abilities: Abilities::default(),
            sign_ins: Vec::new(),
            session: None,
            settings: Vec::new(),
            replaying: false,
            name: String::new(),
            signed_out_because: String::new(),
        };
        agent.initialize();
        agent
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The agent's name for the conversation, once it is open.
    pub fn session(&self) -> Option<&str> {
        self.session.as_deref()
    }

    pub fn settings(&self) -> &[Setting] {
        &self.settings
    }

    /// The agent's ways of signing in.
    pub fn sign_ins(&self) -> &[SignIn] {
        &self.sign_ins
    }

    /// The first setting of a kind: `model`, `thought_level`.
    pub fn setting(&self, category: &str) -> Option<&Setting> {
        self.settings.iter().find(|s| s.category == category)
    }

    /// Whether the agent takes pictures in a message.
    pub fn takes_pictures(&self) -> bool {
        self.abilities.pictures
    }

    fn send(&mut self, message: Value) {
        if let Some(outgoing) = &self.outgoing {
            let _ = outgoing.send(one_line(&message));
        }
    }

    fn request(&mut self, method: &str, params: Value, asked: Asked) {
        let id = self.next;
        self.next += 1;
        self.asked.insert(id, asked);
        let mut out = Object::new();
        out.set("jsonrpc", "2.0").set("id", id).set("method", method).set("params", params);
        self.send(out.build());
    }

    fn notify(&mut self, method: &str, params: Value) {
        let mut out = Object::new();
        out.set("jsonrpc", "2.0").set("method", method).set("params", params);
        self.send(out.build());
    }

    fn respond(&mut self, id: Value, result: Value) {
        let mut out = Object::new();
        out.set("jsonrpc", "2.0").set("id", id).set("result", result);
        self.send(out.build());
    }

    fn refuse(&mut self, id: Value, code: f64, message: &str) {
        let mut error = Object::new();
        error.set("code", code).set("message", message);
        let mut out = Object::new();
        out.set("jsonrpc", "2.0").set("id", id).set("error", error.build());
        self.send(out.build());
    }

    fn initialize(&mut self) {
        let mut info = Object::new();
        info.set("name", "tequila").set("title", "Tequila").set("version", self.launch.version.clone());
        // Signing in happens in the agent's own program, in a terminal:
        // saying so is what makes an agent offer its sign-ins at all. Files
        // and terminals are left to the agent, which asks before it uses
        // them; nothing of Tequila's is lent to it for that.
        let mut auth = Object::new();
        auth.set("terminal", true);
        let mut capabilities = Object::new();
        capabilities.set("auth", auth.build());
        let mut params = Object::new();
        params.set("protocolVersion", PROTOCOL).set("clientCapabilities", capabilities.build()).set("clientInfo", info.build());
        self.request("initialize", params.build(), Asked::Initialize);
    }

    /// Open the conversation: a new one, or the earlier one asked for where
    /// the agent can carry one on.
    fn open(&mut self) {
        let servers: Vec<Value> = self
            .launch
            .servers
            .iter()
            .map(|server| {
                let pair = |(name, value): &(String, String)| {
                    let mut out = Object::new();
                    out.set("name", name.clone()).set("value", value.clone());
                    out.build()
                };
                let mut out = Object::new();
                out.set("name", server.name.clone())
                    .set("command", server.command.to_string_lossy().into_owned())
                    .set("args", Value::Array(server.args.iter().cloned().map(Value::String).collect()))
                    .set("env", Value::Array(server.env.iter().map(pair).collect()));
                out.build()
            })
            .collect();
        let mut params = Object::new();
        params.set("cwd", self.launch.folder.to_string_lossy().into_owned()).set("mcpServers", Value::Array(servers));
        if self.abilities.instructions && !self.launch.instructions.is_empty() {
            let mut prompt = Object::new();
            prompt.set("mode", "append").set("content", Value::Array(vec![text_block(&self.launch.instructions)]));
            params.set("systemPrompt", prompt.build());
        }
        if let Some(meta) = &self.launch.meta {
            params.set("_meta", meta.clone());
        }
        let earlier = self.launch.resume.clone().filter(|_| self.abilities.resume || self.abilities.load);
        match earlier {
            Some(id) => {
                params.set("sessionId", id);
                // Resuming does not send the conversation again; loading
                // does, and the chat has it already.
                let method = if self.abilities.resume { "session/resume" } else { "session/load" };
                self.replaying = method == "session/load";
                self.request(method, params.build(), Asked::Open { resumed: true });
            }
            None => {
                self.launch.resume = None;
                self.request("session/new", params.build(), Asked::Open { resumed: false });
            }
        }
    }

    /// Sign in by one of the ways the agent carries out itself.
    pub fn authenticate(&mut self, method: &str) {
        let mut params = Object::new();
        params.set("methodId", method);
        self.phase = Phase::Starting;
        self.request("authenticate", params.build(), Asked::Authenticate);
    }

    /// Send a message. The reply comes as events, ending in
    /// [`Event::Finished`] or [`Event::Failed`].
    pub fn prompt(&mut self, blocks: &[Block]) -> Result<(), String> {
        let Some(session) = self.session.clone().filter(|_| self.phase == Phase::Idle) else {
            return Err(match self.phase {
                Phase::Working => "it is still working on the last request".into(),
                Phase::SignedOut => "sign in first".into(),
                Phase::Gone => "the assistant's program has stopped".into(),
                _ => "the assistant is still starting".into(),
            });
        };
        let content: Vec<Value> = blocks
            .iter()
            .filter_map(|block| match block {
                Block::Text(text) => Some(text_block(text)),
                Block::Picture(png) if self.abilities.pictures => {
                    let mut out = Object::new();
                    out.set("type", "image").set("mimeType", "image/png").set("data", crate::base64::encode(png));
                    Some(out.build())
                }
                Block::Picture(_) => None,
            })
            .collect();
        let mut params = Object::new();
        params.set("sessionId", session).set("prompt", Value::Array(content));
        self.phase = Phase::Working;
        self.request("session/prompt", params.build(), Asked::Prompt);
        Ok(())
    }

    /// Stop the request being worked on. The turn is over when the agent
    /// says so, with [`Event::Finished`].
    pub fn cancel(&mut self) {
        // Every question still open is answered "cancelled", as the
        // protocol requires, or the agent goes on waiting for it.
        for (_, (id, _)) in std::mem::take(&mut self.permissions) {
            let mut outcome = Object::new();
            outcome.set("outcome", "cancelled");
            let mut result = Object::new();
            result.set("outcome", outcome.build());
            self.respond(id, result.build());
        }
        if let (Phase::Working, Some(session)) = (self.phase, self.session.clone()) {
            let mut params = Object::new();
            params.set("sessionId", session);
            self.notify("session/cancel", params.build());
        }
    }

    /// Answer one of the agent's questions. It offers its own choices: this
    /// takes "this once" of the kind asked for, or failing that "always".
    pub fn permit(&mut self, permission: u64, allow: bool) {
        let Some((id, options)) = self.permissions.remove(&permission) else { return };
        let wanted: [&str; 2] = if allow { ["allow_once", "allow_always"] } else { ["reject_once", "reject_always"] };
        let chosen = wanted.iter().find_map(|kind| options.iter().find(|(_, k)| k == kind)).map(|(option, _)| option.clone());
        let mut outcome = Object::new();
        match chosen {
            Some(option) => {
                outcome.set("outcome", "selected").set("optionId", option);
            }
            // Nothing of the kind on offer: the nearest honest answer is
            // that the question was not answered, which lets nothing happen.
            None => {
                outcome.set("outcome", "cancelled");
            }
        }
        let mut result = Object::new();
        result.set("outcome", outcome.build());
        self.respond(id, result.build());
    }

    /// Change one of the agent's settings: its model, how hard it thinks.
    pub fn set(&mut self, setting: &str, value: &str) {
        let Some(session) = self.session.clone() else { return };
        if let Some(known) = self.settings.iter_mut().find(|s| s.id == setting) {
            known.current = value.to_string();
        }
        let mut params = Object::new();
        params.set("sessionId", session).set("configId", setting).set("value", value);
        self.request("session/set_config_option", params.build(), Asked::Setting);
    }

    /// What the agent last wrote to its error output: why it stopped.
    fn last_words(&self) -> String {
        let kept = self.errors.lock().unwrap_or_else(|e| e.into_inner());
        kept.iter().rev().take(6).rev().cloned().collect::<Vec<_>>().join("\n")
    }

    /// Take in everything the agent has written since last time.
    pub fn poll(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        loop {
            match self.incoming.try_recv() {
                Ok(Incoming::Message(message)) => self.take(&message, &mut events),
                Ok(Incoming::Stray(line)) => {
                    let mut kept = self.errors.lock().unwrap_or_else(|e| e.into_inner());
                    if kept.len() == ERROR_LINES {
                        kept.pop_front();
                    }
                    kept.push_back(line);
                }
                Ok(Incoming::Closed) | Err(TryRecvError::Disconnected) => {
                    if self.phase != Phase::Gone {
                        self.phase = Phase::Gone;
                        self.permissions.clear();
                        events.push(Event::Exited(self.last_words()));
                    }
                    break;
                }
                Err(TryRecvError::Empty) => break,
            }
        }
        events
    }

    fn take(&mut self, message: &Value, events: &mut Vec<Event>) {
        let empty = Value::Object(BTreeMap::new());
        let params = message.get("params").unwrap_or(&empty);
        match (message.get("method").and_then(Value::as_str), message.get("id")) {
            (Some("session/request_permission"), Some(id)) => self.asked_permission(id.clone(), params, events),
            // Nothing else an agent may ask of a client is offered here.
            (Some(method), Some(id)) => self.refuse(id.clone(), -32601.0, &format!("Method not found: {method}")),
            (Some("session/update"), None) => {
                if let (Some(update), false) = (params.get("update"), self.replaying) {
                    self.update(update, events);
                }
            }
            // Who pays, in the words of Claude's adapter: `account` for a
            // subscription, `api_key`, `none` when nobody is signed in.
            (Some("_auth/status_update"), None) => {
                let status = params.get("authStatus").unwrap_or(params);
                events.push(Event::Account { kind: word(status, "kind"), label: word(status, "label") });
            }
            // And in the protocol's own, which says only whether.
            (Some("auth/status_update"), None) => {
                if params.get("state").and_then(Value::as_str) == Some("unauthenticated") {
                    events.push(Event::Account { kind: "none".into(), label: String::new() });
                }
            }
            (Some(_), None) => {}
            (None, Some(id)) => {
                let Some(asked) = id.as_index().and_then(|id| self.asked.remove(&(id as u64))) else { return };
                match message.get("error") {
                    Some(error) => self.failed(asked, error, events),
                    None => self.answered(asked, message.get("result").unwrap_or(&empty), events),
                }
            }
            (None, None) => {}
        }
    }

    fn answered(&mut self, asked: Asked, result: &Value, events: &mut Vec<Event>) {
        match asked {
            Asked::Initialize => {
                if result.get("protocolVersion").and_then(Value::as_f64) != Some(PROTOCOL) {
                    events.push(Event::Failed(
                        "This assistant speaks a different version of the Agent Client Protocol from Tequila's. Updating it, or Tequila, should bring them together.".into(),
                    ));
                    self.phase = Phase::Gone;
                    self.outgoing = None;
                    return;
                }
                let capabilities = result.get("agentCapabilities").cloned().unwrap_or(Value::Null);
                let flag = |value: Option<&Value>| value.is_some_and(|v| !matches!(v, Value::Null | Value::Bool(false)));
                let sessions = capabilities.get("sessionCapabilities");
                self.abilities = Abilities {
                    pictures: flag(capabilities.get("promptCapabilities").and_then(|p| p.get("image"))),
                    load: flag(capabilities.get("loadSession")),
                    resume: flag(sessions.and_then(|s| s.get("resume"))),
                    instructions: flag(sessions.and_then(|s| s.get("systemPrompt"))),
                };
                self.sign_ins = result.get("authMethods").map(Value::as_array).unwrap_or(&[]).iter().filter_map(sign_in).collect();
                if let Some(info) = result.get("agentInfo") {
                    self.name = info.get("title").or(info.get("name")).and_then(Value::as_str).unwrap_or_default().to_string();
                }
                self.open();
            }
            Asked::Open { resumed } => {
                self.replaying = false;
                self.session = result.get("sessionId").and_then(Value::as_str).map(str::to_string).or(self.launch.resume.clone());
                if self.session.is_none() {
                    events.push(Event::Failed("The assistant opened no conversation.".into()));
                    return;
                }
                self.settings = settings(result.get("configOptions"));
                self.phase = Phase::Idle;
                events.push(Event::Ready { resumed });
            }
            Asked::Authenticate => self.open(),
            Asked::Prompt => {
                self.phase = Phase::Idle;
                self.permissions.clear();
                events.push(Event::Finished(match result.get("stopReason").and_then(Value::as_str).unwrap_or_default() {
                    "end_turn" => Stop::Done,
                    "max_tokens" => Stop::MaxTokens,
                    "max_turn_requests" => Stop::MaxSteps,
                    "refusal" => Stop::Refused,
                    "cancelled" => Stop::Stopped,
                    other => Stop::Other(other.to_string()),
                }));
            }
            Asked::Setting => {
                if let Some(options) = result.get("configOptions") {
                    self.settings = settings(Some(options));
                }
                events.push(Event::Settings);
            }
        }
    }

    fn failed(&mut self, asked: Asked, error: &Value, events: &mut Vec<Event>) {
        let needs_sign_in = error.get("code").and_then(Value::as_f64) == Some(AUTH_REQUIRED);
        let mut said = word(error, "message");
        // More than the one line, where the agent sent it.
        if let Some(more) = error.get("data").and_then(|d| d.as_str().map(str::to_string).or_else(|| d.get("details").and_then(Value::as_str).map(str::to_string))) {
            if !more.is_empty() && !said.contains(&more) {
                said = if said.is_empty() { more } else { format!("{said}: {more}") };
            }
        }
        match asked {
            Asked::Open { .. } | Asked::Authenticate | Asked::Prompt if needs_sign_in => {
                self.phase = Phase::SignedOut;
                self.signed_out_because = error.get("data").and_then(|d| d.get("reason")).and_then(Value::as_str).unwrap_or_default().to_string();
                events.push(Event::SignIn(self.sign_ins.clone()));
            }
            // The earlier conversation is gone, or not this agent's: a
            // fresh one, and the person is told.
            Asked::Open { resumed: true } => {
                self.replaying = false;
                self.launch.resume = None;
                events.push(Event::Notice("The assistant no longer has the earlier conversation, so it starts from what is written here.".into()));
                self.open();
            }
            // The sign-in did not go through: said, and the ways to sign in
            // are offered again.
            Asked::Authenticate => {
                self.phase = Phase::SignedOut;
                events.push(Event::Notice(if said.is_empty() { "Signing in did not go through.".into() } else { format!("Signing in did not go through: {said}") }));
                events.push(Event::SignIn(self.sign_ins.clone()));
            }
            Asked::Initialize | Asked::Open { .. } => {
                self.phase = Phase::Gone;
                events.push(Event::Failed(if said.is_empty() { "The assistant could not be started.".into() } else { said }));
            }
            Asked::Prompt => {
                self.phase = Phase::Idle;
                self.permissions.clear();
                events.push(Event::Failed(if said.is_empty() { "The request failed.".into() } else { said }));
            }
            // The setting stays as the agent has it.
            Asked::Setting => events.push(Event::Settings),
        }
    }

    fn asked_permission(&mut self, id: Value, params: &Value, events: &mut Vec<Event>) {
        let Some(tool) = params.get("toolCall").and_then(tool_news) else {
            return self.refuse(id, -32602.0, "no tool call to ask about");
        };
        let options = params
            .get("options")
            .map(Value::as_array)
            .unwrap_or(&[])
            .iter()
            .map(|option| (word(option, "optionId"), word(option, "kind")))
            .collect();
        let ours = self.next_permission;
        self.next_permission += 1;
        self.permissions.insert(ours, (id, options));
        events.push(Event::Permission(Permission { id: ours, tool }));
    }

    fn update(&mut self, update: &Value, events: &mut Vec<Event>) {
        let said = || update.get("content").filter(|c| c.get("type").and_then(Value::as_str) == Some("text")).map(|c| word(c, "text"));
        match update.get("sessionUpdate").and_then(Value::as_str).unwrap_or_default() {
            "agent_message_chunk" => events.extend(said().filter(|t| !t.is_empty()).map(Event::Said)),
            "agent_thought_chunk" => events.extend(said().filter(|t| !t.is_empty()).map(Event::Thought)),
            "tool_call" | "tool_call_update" => events.extend(tool_news(update).map(Event::Tool)),
            "plan" => {
                let entries = update.get("entries").map(Value::as_array).unwrap_or(&[]);
                events.push(Event::Plan(
                    entries
                        .iter()
                        .map(|entry| {
                            let status = match entry.get("status").and_then(Value::as_str) {
                                Some("completed") => Status::Done,
                                _ => Status::Running,
                            };
                            (word(entry, "content"), status)
                        })
                        .collect(),
                ));
            }
            "config_option_update" => {
                self.settings = settings(update.get("configOptions"));
                events.push(Event::Settings);
            }
            "session_info_update" => events.extend(update.get("title").and_then(Value::as_str).filter(|t| !t.is_empty()).map(|t| Event::Title(t.to_string()))),
            "notice" => {
                let (title, more) = (word(update, "title"), word(update, "description"));
                events.push(Event::Notice(if more.is_empty() { title } else { format!("{title}: {more}") }));
            }
            // The agent's commands, its modes and how full its memory is:
            // nothing the panel shows. And whatever a later version adds.
            _ => {}
        }
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        // Its input closed first, which is how an agent is asked to leave;
        // then ended, in case it does not.
        self.outgoing = None;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn sign_in(method: &Value) -> Option<SignIn> {
    let id = word(method, "id");
    if id.is_empty() {
        return None;
    }
    let terminal = (method.get("type").and_then(Value::as_str) == Some("terminal")).then(|| {
        let args = method.get("args").map(Value::as_array).unwrap_or(&[]).iter().filter_map(|a| a.as_str().map(str::to_string)).collect();
        let env = match method.get("env") {
            Some(Value::Object(map)) => map.iter().filter_map(|(name, value)| Some((name.clone(), value.as_str()?.to_string()))).collect(),
            _ => Vec::new(),
        };
        (args, env)
    });
    Some(SignIn { name: word(method, "name"), description: word(method, "description"), id, terminal })
}

/// The agent's settings that offer a choice from a list. One that is on or
/// off, or of a kind added later, is left out.
fn settings(options: Option<&Value>) -> Vec<Setting> {
    let choice = |value: &Value, group: &str| {
        let chosen = word(value, "value");
        (!chosen.is_empty()).then(|| Choice { name: word(value, "name"), description: word(value, "description"), value: chosen, group: group.to_string() })
    };
    options
        .map(Value::as_array)
        .unwrap_or(&[])
        .iter()
        .filter(|option| option.get("type").and_then(Value::as_str) == Some("select"))
        .filter_map(|option| {
            let mut choices = Vec::new();
            for entry in option.get("options").map(Value::as_array).unwrap_or(&[]) {
                match entry.get("options") {
                    // Grouped under a heading.
                    Some(inner) => choices.extend(inner.as_array().iter().filter_map(|c| choice(c, &word(entry, "name")))),
                    None => choices.extend(choice(entry, "")),
                }
            }
            let id = word(option, "id");
            (!id.is_empty() && !choices.is_empty()).then(|| Setting {
                id,
                name: word(option, "name"),
                category: word(option, "category"),
                current: word(option, "currentValue"),
                choices,
            })
        })
        .collect()
}

fn tool_news(call: &Value) -> Option<ToolNews> {
    let id = word(call, "toolCallId");
    if id.is_empty() {
        return None;
    }
    let status = call.get("status").and_then(Value::as_str).map(|status| match status {
        "completed" => Status::Done,
        "failed" => Status::Failed,
        _ => Status::Running,
    });
    let mut paths: Vec<PathBuf> = call.get("locations").map(Value::as_array).unwrap_or(&[]).iter().filter_map(|l| l.get("path")?.as_str().map(PathBuf::from)).collect();
    let mut said = Vec::new();
    let mut pictures = Vec::new();
    for item in call.get("content").map(Value::as_array).unwrap_or(&[]) {
        match item.get("type").and_then(Value::as_str) {
            Some("content") => match item.get("content") {
                Some(block) if block.get("type").and_then(Value::as_str) == Some("text") => said.push(word(block, "text")),
                Some(block) if block.get("type").and_then(Value::as_str) == Some("image") => {
                    pictures.extend(block.get("data").and_then(Value::as_str).and_then(crate::base64::decode));
                }
                _ => {}
            },
            Some("diff") => {
                let path = word(item, "path");
                if !path.is_empty() && !paths.iter().any(|p| p == Path::new(&path)) {
                    paths.push(PathBuf::from(path));
                }
            }
            _ => {}
        }
    }
    // The tool's own name: the protocol's field, or where Claude's adapter
    // has long put it.
    let name = call
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| call.get("_meta")?.get("claudeCode")?.get("toolName")?.as_str())
        .map(str::to_string);
    Some(ToolNews {
        id,
        title: call.get("title").and_then(Value::as_str).map(str::to_string),
        name,
        kind: call.get("kind").and_then(Value::as_str).map(str::to_string),
        status,
        text: call.get("content").filter(|c| !matches!(c, Value::Null)).map(|_| said.join("\n")),
        pictures,
        paths,
        input: call.get("rawInput").filter(|v| !matches!(v, Value::Null)).cloned(),
    })
}

/// What Tequila says to an agent asking to use a tool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    /// The person is asked, with this as the question and what it is about.
    Ask { title: String, detail: String },
    /// Refused, and why, for the agent to read.
    Deny(String),
}

/// A path with its `.` and `..` worked out, without asking the disk.
fn tidy(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Whether `path` is inside `folder`, through whatever links: the nearest
/// part of it that exists is what the disk is asked about.
pub fn inside(path: &Path, folder: &Path) -> bool {
    let Ok(folder) = folder.canonicalize() else { return false };
    let path = tidy(&if path.is_absolute() { path.to_path_buf() } else { folder.join(path) });
    let mut exists = path.as_path();
    let mut rest = Vec::new();
    let real = loop {
        match exists.canonicalize() {
            Ok(real) => break real,
            Err(_) => match (exists.parent(), exists.file_name()) {
                (Some(parent), Some(name)) => {
                    rest.push(name.to_os_string());
                    exists = parent;
                }
                _ => return false,
            },
        }
    };
    let mut whole = real;
    whole.extend(rest.iter().rev());
    whole.starts_with(&folder)
}

/// Decide a tool call an agent asks permission for. `tools` is the name
/// Tequila's MCP bridge was given to the agent under.
///
/// The assistant may use Tequila's tools and the files in the open
/// project's folder, and nothing else on the computer. Tequila's own tools
/// are let through here because Tequila itself holds the ones that leave
/// the scene - an import, a render to a file - until the person agrees.
/// Reading in the project's folder is free; writing there asks; a command,
/// the internet, or a file anywhere else is refused.
pub fn judge(tool: &ToolNews, folder: &Path, tools: &str) -> Verdict {
    let name = tool.name.as_deref().unwrap_or_default();
    let title = tool.title.clone().unwrap_or_default();
    // Claude calls an MCP tool `mcp__<server>__<tool>`; others write the
    // server's name and the tool's with a dot, a slash or a colon between.
    let ours = [format!("mcp__{tools}__"), format!("{tools}."), format!("{tools}/"), format!("{tools}:")];
    if ours.iter().any(|prefix| name.starts_with(prefix.as_str()) || title.starts_with(prefix.as_str())) {
        return Verdict::Allow;
    }
    // Where it reaches: the files it names, and those in what it was called
    // with.
    let mut paths = tool.paths.clone();
    for key in ["file_path", "path", "notebook_path", "absolute_path"] {
        if let Some(path) = tool.input.as_ref().and_then(|i| i.get(key)).and_then(Value::as_str) {
            if !paths.iter().any(|known| known == Path::new(path)) {
                paths.push(PathBuf::from(path));
            }
        }
    }
    let outside = paths.iter().find(|path| !inside(path, folder));
    let elsewhere = |path: &Path| Verdict::Deny(format!("{} is outside the project's folder, which is as far as Tequila's assistant reaches.", path.display()));
    // A file in the folder is named as it is known there.
    let files = paths.iter().map(|p| p.strip_prefix(folder).unwrap_or(p).display().to_string()).collect::<Vec<_>>().join(", ");
    match tool.kind.as_deref().unwrap_or("other") {
        "read" | "search" => match outside {
            Some(path) => elsewhere(path),
            None => Verdict::Allow,
        },
        "edit" | "delete" | "move" => match outside {
            Some(path) => elsewhere(path),
            None if paths.is_empty() => Verdict::Ask { title: "Change a file in the project's folder?".into(), detail: title },
            None => Verdict::Ask { title: "Change a file in the project's folder?".into(), detail: files },
        },
        "think" => Verdict::Allow,
        "execute" => Verdict::Deny("Tequila's assistant does not run commands on the computer.".into()),
        "fetch" => Verdict::Deny("Tequila's assistant does not reach the internet.".into()),
        _ if outside.is_some() => elsewhere(outside.expect("checked")),
        _ => Verdict::Ask { title: "Let the assistant do this?".into(), detail: title },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};
    use std::time::{Duration, Instant};

    /// A stand-in agent on a local socket: `script` is handed each message
    /// Tequila sends and a way to write back.
    fn stand_in(launch: Launch, mut script: impl FnMut(&Value, &mut dyn FnMut(&str)) + Send + 'static) -> Agent {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut writer = stream.try_clone().unwrap();
            let mut write = move |line: &str| {
                let _ = writeln!(writer, "{line}");
            };
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                script(&json::parse(line.as_bytes()).unwrap(), &mut write);
            }
        });
        let stream = TcpStream::connect(address).unwrap();
        Agent::over(launch, stream.try_clone().unwrap(), stream, || {})
    }

    /// Events until `done` says so, or five seconds.
    fn until(agent: &mut Agent, done: impl Fn(&[Event]) -> bool) -> Vec<Event> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut events = Vec::new();
        while !done(&events) && Instant::now() < deadline {
            events.extend(agent.poll());
            std::thread::sleep(Duration::from_millis(5));
        }
        events
    }

    fn id(message: &Value) -> String {
        one_line(message.get("id").unwrap_or(&Value::Null))
    }

    const INITIALIZED: &str = r#""result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"promptCapabilities":{"image":true},"sessionCapabilities":{"resume":{},"systemPrompt":{}}},"agentInfo":{"name":"stand-in","title":"Stand-in","version":"1"},"authMethods":[{"id":"login","name":"Subscription","type":"terminal","args":["--cli","auth","login"],"env":{"A":"1"}},{"id":"key","name":"Key","description":"By key"}]}"#;
    const OPENED: &str = r#""result":{"sessionId":"s-1","configOptions":[{"id":"model","name":"Model","category":"model","type":"select","currentValue":"fast","options":[{"value":"fast","name":"Fast"},{"value":"deep","name":"Deep","description":"Slower"}]},{"id":"effort","name":"Effort","category":"thought_level","type":"select","currentValue":"medium","options":[{"group":"g","name":"Levels","options":[{"value":"low","name":"Low"},{"value":"medium","name":"Medium"}]}]},{"id":"brave","name":"Brave","type":"boolean","currentValue":true}]}"#;

    fn launch() -> Launch {
        Launch {
            folder: std::env::temp_dir(),
            servers: vec![McpServer { name: "tequila".into(), command: "/apps/tequila-cli".into(), args: vec!["--mcp".into()], env: vec![("K".into(), "V".into())] }],
            instructions: "You are in Tequila.".into(),
            version: "1.2.3".into(),
            ..Launch::default()
        }
    }

    /// The whole of a turn: the conversation opened with Tequila's bridge
    /// and instructions, a message sent with its picture, the reply and its
    /// reasoning streamed, a tool call started and finished with a picture,
    /// and the turn ended.
    #[test]
    fn a_turn_runs_from_opening_to_its_end() {
        let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
        let record = seen.clone();
        let mut agent = stand_in(launch(), move |message, write| {
            record.lock().unwrap().push(message.clone());
            match message.get("method").and_then(Value::as_str) {
                Some("initialize") => write(&format!(r#"{{"jsonrpc":"2.0","id":{},{INITIALIZED}}}"#, id(message))),
                Some("session/new") => write(&format!(r#"{{"jsonrpc":"2.0","id":{},{OPENED}}}"#, id(message))),
                Some("session/prompt") => {
                    let update = |body: &str| format!(r#"{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"s-1","update":{body}}}}}"#);
                    write(&update(r#"{"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"The strap "}}"#));
                    write(&update(r#"{"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"is part 4."}}"#));
                    write(&update(r#"{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"On it."}}"#));
                    write(&update(r#"{"sessionUpdate":"tool_call","toolCallId":"t1","title":"tequila_screenshot","name":"mcp__tequila__tequila_screenshot","kind":"other","status":"pending","rawInput":{"viewport":true}}"#));
                    write(&update(r#"{"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"completed","content":[{"type":"content","content":{"type":"image","mimeType":"image/png","data":"AQID"}},{"type":"content","content":{"type":"text","text":"Tequila viewport"}}]}"#));
                    write(&update(r#"{"sessionUpdate":"plan","entries":[{"content":"Look","priority":"high","status":"completed"},{"content":"Change","priority":"high","status":"pending"}]}"#));
                    write(&update(r#"{"sessionUpdate":"session_info_update","title":"Velvet strap"}"#));
                    write(&update(r#"{"sessionUpdate":"something_new","more":1}"#));
                    write("not json at all");
                    write(&format!(r#"{{"jsonrpc":"2.0","id":{},"result":{{"stopReason":"end_turn"}}}}"#, id(message)));
                }
                _ => {}
            }
        });
        let events = until(&mut agent, |e| e.iter().any(|e| matches!(e, Event::Ready { .. })));
        assert_eq!(events, [Event::Ready { resumed: false }]);
        assert_eq!((agent.name.as_str(), agent.session(), agent.phase()), ("Stand-in", Some("s-1"), Phase::Idle));
        // Its settings: the two it offers a list for, the grouped one
        // flattened; the on-or-off one is not shown.
        let names: Vec<(&str, &str)> = agent.settings().iter().map(|s| (s.category.as_str(), s.current_name())).collect();
        assert_eq!(names, [("model", "Fast"), ("thought_level", "Medium")]);
        assert_eq!(agent.setting("thought_level").unwrap().choices[0].group, "Levels");

        agent.prompt(&[Block::Text("Make the strap velvet".into()), Block::Picture(vec![1, 2, 3])]).unwrap();
        assert!(agent.prompt(&[Block::Text("And again".into())]).is_err(), "one request at a time");
        let events = until(&mut agent, |e| e.iter().any(|e| matches!(e, Event::Finished(_))));
        assert_eq!(
            events,
            [
                Event::Thought("The strap ".into()),
                Event::Thought("is part 4.".into()),
                Event::Said("On it.".into()),
                Event::Tool(ToolNews {
                    id: "t1".into(),
                    title: Some("tequila_screenshot".into()),
                    name: Some("mcp__tequila__tequila_screenshot".into()),
                    kind: Some("other".into()),
                    status: Some(Status::Running),
                    input: Some(json::parse(br#"{"viewport":true}"#).unwrap()),
                    ..ToolNews::default()
                }),
                Event::Tool(ToolNews { id: "t1".into(), status: Some(Status::Done), text: Some("Tequila viewport".into()), pictures: vec![vec![1, 2, 3]], ..ToolNews::default() }),
                Event::Plan(vec![("Look".into(), Status::Done), ("Change".into(), Status::Running)]),
                Event::Title("Velvet strap".into()),
                Event::Finished(Stop::Done),
            ]
        );
        assert_eq!(agent.phase(), Phase::Idle);

        // What Tequila sent: itself, the folder and its bridge, the
        // instructions, and the message with its picture.
        let seen = seen.lock().unwrap();
        let initialize = seen[0].get("params").unwrap();
        assert_eq!(initialize.get("protocolVersion").and_then(Value::as_f64), Some(1.0));
        assert_eq!(initialize.get("clientCapabilities").unwrap().get("auth").unwrap().get("terminal"), Some(&Value::Bool(true)));
        assert!(initialize.get("clientCapabilities").unwrap().get("fs").is_none(), "no files are lent to the agent");
        let open = seen[1].get("params").unwrap();
        let server = &open.get("mcpServers").unwrap().as_array()[0];
        assert_eq!(server.get("command").and_then(Value::as_str), Some("/apps/tequila-cli"));
        assert_eq!(one_line(server.get("env").unwrap()), r#"[ { "name": "K", "value": "V" } ]"#);
        assert_eq!(open.get("systemPrompt").unwrap().get("mode").and_then(Value::as_str), Some("append"));
        let prompt = seen[2].get("params").unwrap().get("prompt").unwrap().as_array();
        assert_eq!(prompt[0].get("text").and_then(Value::as_str), Some("Make the strap velvet"));
        assert_eq!((prompt[1].get("type").and_then(Value::as_str), prompt[1].get("data").and_then(Value::as_str)), (Some("image"), Some("AQID")));
    }

    /// An agent that is not signed in says so when the conversation is
    /// opened, and its ways of signing in are handed on.
    #[test]
    fn an_agent_that_needs_signing_in_says_how() {
        let mut agent = stand_in(launch(), |message, write| match message.get("method").and_then(Value::as_str) {
            Some("initialize") => write(&format!(r#"{{"jsonrpc":"2.0","id":{},{INITIALIZED}}}"#, id(message))),
            Some("session/new") => write(&format!(r#"{{"jsonrpc":"2.0","id":{},"error":{{"code":-32000,"message":"Authentication required"}}}}"#, id(message))),
            _ => {}
        });
        let events = until(&mut agent, |e| !e.is_empty());
        let Some(Event::SignIn(ways)) = events.first() else { panic!("{events:?}") };
        assert_eq!(ways.len(), 2);
        assert_eq!(ways[0].terminal, Some((vec!["--cli".to_string(), "auth".into(), "login".into()], vec![("A".to_string(), "1".to_string())])));
        assert_eq!((ways[1].name.as_str(), &ways[1].terminal), ("Key", &None));
        assert_eq!(agent.phase(), Phase::SignedOut);
        assert!(agent.prompt(&[Block::Text("Hello".into())]).is_err());
    }

    /// A sign-in the agent carries out itself - ChatGPT's, in the browser -
    /// is asked for by its name; when it is done the conversation opens,
    /// and when it fails the ways to sign in are offered again.
    #[test]
    fn signing_in_through_the_agent_opens_the_conversation_or_says_why_not() {
        let signed_in = Arc::new(Mutex::new(false));
        let state = signed_in.clone();
        let mut agent = stand_in(launch(), move |message, write| match message.get("method").and_then(Value::as_str) {
            Some("initialize") => write(&format!(r#"{{"jsonrpc":"2.0","id":{},{INITIALIZED}}}"#, id(message))),
            Some("session/new") if *state.lock().unwrap() => write(&format!(r#"{{"jsonrpc":"2.0","id":{},{OPENED}}}"#, id(message))),
            Some("session/new") => write(&format!(r#"{{"jsonrpc":"2.0","id":{},"error":{{"code":-32000,"message":"Authentication required"}}}}"#, id(message))),
            Some("authenticate") => match message.get("params").unwrap().get("methodId").and_then(Value::as_str) {
                Some("key") => {
                    *state.lock().unwrap() = true;
                    write(&format!(r#"{{"jsonrpc":"2.0","id":{},"result":{{}}}}"#, id(message)));
                }
                _ => write(&format!(r#"{{"jsonrpc":"2.0","id":{},"error":{{"code":-32603,"message":"The window was closed"}}}}"#, id(message))),
            },
            _ => {}
        });
        assert!(matches!(until(&mut agent, |e| !e.is_empty()).as_slice(), [Event::SignIn(_)]));
        agent.authenticate("login");
        assert_eq!(agent.phase(), Phase::Starting);
        let events = until(&mut agent, |e| e.len() == 2);
        assert_eq!(events[0], Event::Notice("Signing in did not go through: The window was closed".into()));
        assert!(matches!(&events[1], Event::SignIn(ways) if ways.len() == 2));
        assert_eq!(agent.phase(), Phase::SignedOut);

        agent.authenticate("key");
        assert_eq!(until(&mut agent, |e| !e.is_empty()), [Event::Ready { resumed: false }]);
        assert_eq!(agent.phase(), Phase::Idle);
    }

    /// The agent asks before a tool; the answer goes back as the choice it
    /// offered, and Stop answers what is still open as cancelled before
    /// telling the agent to stop.
    #[test]
    fn permission_is_answered_with_the_agents_own_choices_and_stop_cancels_the_rest() {
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let record = seen.clone();
        let mut agent = stand_in(launch(), move |message, write| {
            let ask = |n: u32| {
                format!(
                    r#"{{"jsonrpc":"2.0","id":"ask-{n}","method":"session/request_permission","params":{{"sessionId":"s-1","toolCall":{{"toolCallId":"w{n}","title":"Write notes.txt","kind":"edit","locations":[{{"path":"/p/notes.txt"}}]}},"options":[{{"optionId":"yes-{n}","name":"Allow","kind":"allow_once"}},{{"optionId":"always","name":"Always","kind":"allow_always"}},{{"optionId":"no-{n}","name":"Reject","kind":"reject_once"}}]}}}}"#
                )
            };
            match message.get("method").and_then(Value::as_str) {
                Some("initialize") => write(&format!(r#"{{"jsonrpc":"2.0","id":{},{INITIALIZED}}}"#, id(message))),
                Some("session/new") => write(&format!(r#"{{"jsonrpc":"2.0","id":{},{OPENED}}}"#, id(message))),
                Some("session/prompt") => {
                    record.lock().unwrap().push(format!("prompt {}", id(message)));
                    for n in 1..=3 {
                        write(&ask(n));
                    }
                    // Something a client is not expected to offer.
                    write(r#"{"jsonrpc":"2.0","id":"fs","method":"fs/read_text_file","params":{"sessionId":"s-1","path":"/etc/passwd"}}"#);
                }
                Some("session/cancel") => {
                    record.lock().unwrap().push("cancel".into());
                    write(r#"{"jsonrpc":"2.0","id":2,"result":{"stopReason":"cancelled"}}"#);
                }
                _ => record.lock().unwrap().push(one_line(message)),
            }
        });
        until(&mut agent, |e| e.iter().any(|e| matches!(e, Event::Ready { .. })));
        agent.prompt(&[Block::Text("Write it down".into())]).unwrap();
        let events = until(&mut agent, |e| e.len() == 3);
        let asks: Vec<u64> = events.iter().filter_map(|e| if let Event::Permission(p) = e { Some(p.id) } else { None }).collect();
        assert_eq!(asks.len(), 3, "{events:?}");
        let Event::Permission(first) = &events[0] else { panic!() };
        assert_eq!((first.tool.kind.as_deref(), first.tool.paths.as_slice()), (Some("edit"), [PathBuf::from("/p/notes.txt")].as_slice()));

        agent.permit(asks[0], true);
        agent.permit(asks[1], false);
        agent.cancel();
        let events = until(&mut agent, |e| e.iter().any(|e| matches!(e, Event::Finished(_))));
        assert_eq!(events, [Event::Finished(Stop::Stopped)]);
        let seen = seen.lock().unwrap();
        assert_eq!(
            *seen,
            [
                "prompt 2",
                r#"{ "error": { "code": -32601, "message": "Method not found: fs/read_text_file" }, "id": "fs", "jsonrpc": "2.0" }"#,
                r#"{ "id": "ask-1", "jsonrpc": "2.0", "result": { "outcome": { "optionId": "yes-1", "outcome": "selected" } } }"#,
                r#"{ "id": "ask-2", "jsonrpc": "2.0", "result": { "outcome": { "optionId": "no-2", "outcome": "selected" } } }"#,
                r#"{ "id": "ask-3", "jsonrpc": "2.0", "result": { "outcome": { "outcome": "cancelled" } } }"#,
                "cancel",
            ]
        );
    }

    /// A reopened chat carries on the agent's earlier conversation where
    /// the agent still has it, and starts a fresh one - saying so - where
    /// it does not.
    #[test]
    fn an_earlier_conversation_is_carried_on_or_started_afresh() {
        let mut agent = stand_in(Launch { resume: Some("old-1".into()), ..launch() }, |message, write| match message.get("method").and_then(Value::as_str) {
            Some("initialize") => write(&format!(r#"{{"jsonrpc":"2.0","id":{},{INITIALIZED}}}"#, id(message))),
            Some("session/resume") => {
                assert_eq!(message.get("params").unwrap().get("sessionId").and_then(Value::as_str), Some("old-1"));
                write(&format!(r#"{{"jsonrpc":"2.0","id":{},"result":{{}}}}"#, id(message)));
            }
            _ => {}
        });
        assert_eq!(until(&mut agent, |e| !e.is_empty()), [Event::Ready { resumed: true }]);
        assert_eq!(agent.session(), Some("old-1"));

        let mut agent = stand_in(Launch { resume: Some("old-2".into()), ..launch() }, |message, write| match message.get("method").and_then(Value::as_str) {
            Some("initialize") => write(&format!(r#"{{"jsonrpc":"2.0","id":{},{INITIALIZED}}}"#, id(message))),
            Some("session/resume") => write(&format!(r#"{{"jsonrpc":"2.0","id":{},"error":{{"code":-32002,"message":"Resource not found"}}}}"#, id(message))),
            Some("session/new") => write(&format!(r#"{{"jsonrpc":"2.0","id":{},{OPENED}}}"#, id(message))),
            _ => {}
        });
        let events = until(&mut agent, |e| e.iter().any(|e| matches!(e, Event::Ready { .. })));
        assert!(matches!(&events[0], Event::Notice(text) if text.contains("earlier conversation")), "{events:?}");
        assert_eq!(events[1], Event::Ready { resumed: false });
        assert_eq!(agent.session(), Some("s-1"));
    }

    /// An agent that can only load a conversation plays it back while it
    /// does; none of that is shown twice.
    #[test]
    fn a_conversation_played_back_while_loading_is_not_shown_again() {
        let mut agent = stand_in(Launch { resume: Some("old".into()), ..launch() }, |message, write| match message.get("method").and_then(Value::as_str) {
            Some("initialize") => write(&format!(r#"{{"jsonrpc":"2.0","id":{},"result":{{"protocolVersion":1,"agentCapabilities":{{"loadSession":true}}}}}}"#, id(message))),
            Some("session/load") => {
                write(r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"old","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Said before."}}}}"#);
                write(&format!(r#"{{"jsonrpc":"2.0","id":{},"result":{{}}}}"#, id(message)));
                write(r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"old","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Said now."}}}}"#);
            }
            _ => {}
        });
        let events = until(&mut agent, |e| e.len() == 2);
        assert_eq!(events, [Event::Ready { resumed: true }, Event::Said("Said now.".into())]);
        assert!(!agent.takes_pictures());
    }

    /// Choosing a model is sent as the agent's setting, and its answer -
    /// every setting, as they now stand - replaces what was known.
    #[test]
    fn a_setting_is_changed_and_the_agents_answer_kept() {
        let mut agent = stand_in(launch(), |message, write| match message.get("method").and_then(Value::as_str) {
            Some("initialize") => write(&format!(r#"{{"jsonrpc":"2.0","id":{},{INITIALIZED}}}"#, id(message))),
            Some("session/new") => write(&format!(r#"{{"jsonrpc":"2.0","id":{},{OPENED}}}"#, id(message))),
            Some("session/set_config_option") => {
                let params = message.get("params").unwrap();
                assert_eq!((params.get("configId").and_then(Value::as_str), params.get("value").and_then(Value::as_str)), (Some("model"), Some("deep")));
                write(&format!(
                    r#"{{"jsonrpc":"2.0","id":{},"result":{{"configOptions":[{{"id":"model","name":"Model","category":"model","type":"select","currentValue":"deep","options":[{{"value":"deep","name":"Deep"}}]}}]}}}}"#,
                    id(message)
                ));
            }
            _ => {}
        });
        until(&mut agent, |e| !e.is_empty());
        agent.set("model", "deep");
        assert_eq!(agent.setting("model").unwrap().current, "deep", "shown at once");
        assert_eq!(until(&mut agent, |e| !e.is_empty()), [Event::Settings]);
        assert_eq!(agent.settings().len(), 1);
        assert_eq!(agent.setting("model").unwrap().current_name(), "Deep");
    }

    /// Who pays is told as Claude's adapter says it, and "nobody is signed
    /// in" as the protocol says it too - once each, as they come.
    #[test]
    fn the_account_an_agent_reports_is_handed_on() {
        let mut agent = stand_in(launch(), |message, write| match message.get("method").and_then(Value::as_str) {
            Some("initialize") => write(&format!(r#"{{"jsonrpc":"2.0","id":{},{INITIALIZED}}}"#, id(message))),
            Some("session/new") => {
                write(r#"{"jsonrpc":"2.0","method":"_auth/status_update","params":{"authStatus":{"kind":"api_key","label":"sk-ant-...1234"}}}"#);
                write(r#"{"jsonrpc":"2.0","method":"auth/status_update","params":{"state":"authenticated"}}"#);
                write(r#"{"jsonrpc":"2.0","method":"auth/status_update","params":{"state":"unauthenticated"}}"#);
                write(&format!(r#"{{"jsonrpc":"2.0","id":{},{OPENED}}}"#, id(message)));
            }
            _ => {}
        });
        let events = until(&mut agent, |e| e.iter().any(|e| matches!(e, Event::Ready { .. })));
        assert_eq!(
            events,
            [
                Event::Account { kind: "api_key".into(), label: "sk-ant-...1234".into() },
                Event::Account { kind: "none".into(), label: String::new() },
                Event::Ready { resumed: false },
            ]
        );
        assert_eq!(agent.sign_ins().len(), 2);
    }

    /// The agent's program ending is told, once, with the request it was on
    /// no longer waited for.
    #[test]
    fn an_agent_that_ends_is_noticed() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || drop(listener.accept()));
        let stream = TcpStream::connect(address).unwrap();
        let mut agent = Agent::over(launch(), stream.try_clone().unwrap(), stream, || {});
        let events = until(&mut agent, |e| !e.is_empty());
        assert!(matches!(events.as_slice(), [Event::Exited(_)]), "{events:?}");
        assert_eq!(agent.phase(), Phase::Gone);
        assert!(agent.poll().is_empty(), "told once");
    }

    /// A program that is not there is an error to show, not a panic.
    #[test]
    fn a_program_that_is_missing_says_so() {
        let launch = Launch { program: "/nowhere/claude-agent-acp".into(), ..launch() };
        let error = Agent::start(launch, || {}).err().expect("it cannot start");
        assert!(error.contains("could not be started"), "{error}");
    }

    #[test]
    fn the_assistant_reaches_tequilas_tools_and_the_projects_folder_and_no_further() {
        let folder = std::env::temp_dir().join(format!("tequila-acp-reach-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(folder.join("textures")).unwrap();
        std::fs::write(folder.join("notes.txt"), "x").unwrap();
        let tool = |name: &str, kind: &str, paths: &[PathBuf]| ToolNews {
            id: "t".into(),
            title: Some(name.into()),
            name: Some(name.into()),
            kind: Some(kind.into()),
            paths: paths.to_vec(),
            ..ToolNews::default()
        };
        let judge = |tool: &ToolNews| judge(tool, &folder, "tequila");

        // Tequila's own tools, however the agent spells them.
        assert_eq!(judge(&tool("mcp__tequila__tequila_apply", "other", &[])), Verdict::Allow);
        assert_eq!(judge(&tool("tequila.tequila_import", "other", &[])), Verdict::Allow);
        // Another server's tool that only has Tequila in its name is not.
        assert!(matches!(judge(&tool("mcp__other__tequila_apply", "other", &[])), Verdict::Ask { .. }));

        // Reading in the folder is free; writing there asks, naming the file.
        assert_eq!(judge(&tool("Read", "read", &[folder.join("notes.txt")])), Verdict::Allow);
        assert_eq!(judge(&tool("Grep", "search", &[])), Verdict::Allow);
        let mut write = tool("Write", "edit", &[folder.join("textures").join("new.txt")]);
        write.input = Some(json::parse(format!(r#"{{"file_path":{:?}}}"#, folder.join("textures").join("new.txt").to_string_lossy()).as_bytes()).unwrap());
        let Verdict::Ask { detail, .. } = judge(&write) else { panic!() };
        assert_eq!(Path::new(&detail), Path::new("textures").join("new.txt"), "named once, as the folder knows it");

        // Anywhere else is refused: outright, by `..`, and as the file a
        // tool was called with rather than one it lists.
        assert!(matches!(judge(&tool("Read", "read", &[PathBuf::from("/etc/passwd")])), Verdict::Deny(_)));
        assert!(matches!(judge(&tool("Write", "edit", &[folder.join("textures/../../escaped.txt")])), Verdict::Deny(_)));
        let mut by_input = tool("Write", "edit", &[]);
        by_input.input = Some(json::parse(br#"{"file_path":"/tmp/elsewhere.txt"}"#).unwrap());
        assert!(matches!(judge(&by_input), Verdict::Deny(_)));

        // No commands, no internet.
        assert!(matches!(judge(&tool("Bash", "execute", &[])), Verdict::Deny(text) if text.contains("commands")));
        assert!(matches!(judge(&tool("WebFetch", "fetch", &[])), Verdict::Deny(text) if text.contains("internet")));
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// A link inside the folder that points out of it does not let a file
    /// elsewhere be reached through it.
    #[cfg(unix)]
    #[test]
    fn a_link_out_of_the_folder_is_followed_before_it_is_trusted() {
        let root = std::env::temp_dir().join(format!("tequila-acp-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (folder, elsewhere) = (root.join("project"), root.join("elsewhere"));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, folder.join("way-out")).unwrap();
        assert!(inside(&folder.join("new.txt"), &folder));
        assert!(inside(Path::new("sub/new.txt"), &folder), "a relative path is the folder's");
        assert!(!inside(&folder.join("way-out/secret.txt"), &folder));
        let _ = std::fs::remove_dir_all(&root);
    }
}
