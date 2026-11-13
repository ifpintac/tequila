//! The Assistant: ask for what you want in plain language, and an AI you have
//! connected — Claude, OpenAI, a local model, your company's — does it in the
//! open scene.
//!
//! One chat, answered one of two ways:
//!
//! - **An agent**, a program of its maker's own reached over the Agent
//!   Client Protocol ([`tequila_ai::acp`]): Claude, through its adapter. The
//!   agent runs its own loop and gets Tequila's tools from the MCP bridge
//!   Tequila hands it (`tequila-cli --mcp --assistant`), so what it does
//!   arrives on the scripting channel like any script's, marked as the
//!   Assistant's.
//! - **Tequila's own loop** ([`tequila_ai::session`]) for an API key or a
//!   model on this computer: the conversation is held here, and a tool call
//!   becomes a scripting request that goes through
//!   [`crate::channel::handle`].
//!
//! Either way a step is validated, undoable, journalled, and written into
//! the console with the assistant's name on it, so a person can watch it
//! work; and what leaves the scene - a file brought in or written, a render
//! saved - waits for the person to agree in the chat
//! ([`tequila_scripting::approval`]).
//!
//! This file is the Assistant's state and what moves it along each frame;
//! the panel is drawn in [`crate::assistant_panel`], and the agents are
//! found and started in [`crate::assistant_agents`].

use crate::assistant_agents::{self as agents, Found, Missing};
use crate::TequilaApp;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use tequila_ai::acp::{self, Agent, Block};
use tequila_ai::chat::{self, Asked, Chat, Item, Picture, Status, StepNews};
use tequila_ai::{session, tools, Call, Key, Kind, Part, Phase, Progress, Provider, Session, Settings};
use tequila_scripting::approval::Question;
use tequila_scripting::protocol::{parse_request, Operation, Reply, Request};
use tequila_scripting::server::Responder;

/// The longest side of a screenshot sent to a model. Larger costs tokens
/// without helping it judge a material or a composition.
const SENT_SIZE: usize = 1024;

/// The name Tequila's MCP bridge is given to an agent under.
const TOOLS: &str = "tequila";

/// How much of an earlier conversation is retold to a backend that was not
/// there for it: the end of it, which is what the next request leans on.
const RETOLD: usize = 6000;

/// A screenshot the assistant asked for, waiting for the window to draw it.
struct Shot {
    call: Call,
    path: PathBuf,
    asked: std::time::Instant,
    /// What the model is told with the picture.
    caption: String,
}

/// A plugin's command sent, and its log watched for how it went.
struct PluginWait {
    call: Call,
    plugin: String,
    command: String,
    /// How many lines the plugin had logged when it was sent.
    before: u64,
    asked: std::time::Instant,
}

/// How long a plugin's command is watched before the model is told it is
/// still going.
const PLUGIN_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// Background checks started from the providers window.
enum Check {
    Models(Receiver<Result<Vec<String>, String>>),
    Test(Receiver<Result<String, String>>),
}

/// The provider being added or edited.
struct Draft {
    /// The provider being replaced, if editing.
    original: Option<String>,
    provider: Provider,
    key_mode: KeyMode,
    /// A newly typed key. Empty with [`KeyMode::Stored`] keeps the old one.
    key_input: String,
    env_name: String,
    models: Vec<String>,
    check: Option<Check>,
    status: Option<Result<String, String>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum KeyMode {
    None,
    Stored,
    Env,
}

/// Who answers the chat.
#[derive(Clone, Debug, PartialEq)]
pub enum Backend {
    Agent(agents::Kind),
    Provider(Provider),
}

impl Backend {
    /// Its name in the settings and in a chat's file.
    pub fn id(&self) -> String {
        match self {
            Backend::Agent(kind) => kind.id().to_string(),
            Backend::Provider(provider) => provider.id.clone(),
        }
    }

    pub fn name(&self) -> String {
        match self {
            Backend::Agent(kind) => kind.name().to_string(),
            Backend::Provider(provider) => provider.name.clone(),
        }
    }
}

/// What the Assistant needs to know of the application around it to start
/// a request.
#[derive(Clone, Debug, Default)]
pub struct Around {
    /// How many steps the undo history holds.
    pub depth: usize,
    /// The open project's folder, once it has been saved somewhere.
    pub folder: Option<PathBuf>,
    /// The file that names this window's scripting channel.
    pub channel: Option<PathBuf>,
    /// Tequila's command-line program, which is also its MCP bridge.
    pub program: Option<PathBuf>,
}

/// What stands between the person and an agent that answers.
pub enum Setup {
    /// The agent's program is not on this computer; whether Node is, to
    /// install it with.
    Missing { node: bool },
    Installing(Receiver<Result<String, String>>),
    /// It wants signing in, and these are its ways. `subscription` when it
    /// found a Claude subscription signed in and was told not to use one.
    SignIn { ways: Vec<acp::SignIn>, subscription: bool },
    /// The person is signing in, in the agent's own window or browser page.
    SigningIn,
    /// Something went wrong setting it up, to read and try again.
    Failed(String),
}

/// A request of the Assistant's own bridge, kept until the person has said
/// whether it may go ahead.
struct Held {
    question: u64,
    request: Request,
    responder: Responder,
}

/// What a question in the chat is waiting to settle.
enum Waiting {
    /// The agent's own question, by its number there.
    Agent(u64),
    /// A request held here ([`Held`]).
    Held,
    /// A tool call of Tequila's own loop, kept until answered.
    Call,
}

pub enum Action {
    None,
    /// Send a message: what the person wrote, what the panel adds for the
    /// model about the things they mentioned, and the pictures attached.
    Ask { text: String, context: String, pictures: Vec<Picture> },
    Stop,
    UndoTurn,
    /// The person's answer to a question in the chat.
    Answer { question: u64, allow: bool },
    NewChat,
    OpenChat(String),
    DeleteChat(String),
    /// Attach a picture of the viewport as it is now.
    SendViewport,
    /// Into a window of its own, or back beside the viewport.
    Detach(bool),
    Close,
}

#[derive(Default)]
pub struct Assistant {
    pub open: bool,
    /// Shown in a window of its own rather than docked.
    pub detached: bool,
    pub(crate) settings: Settings,
    settings_path: Option<PathBuf>,
    /// Where chats are kept.
    pub(crate) store: Option<chat::Store>,
    /// The chat that is open.
    pub(crate) chat: Chat,
    /// Tequila's own loop, for a provider: the conversation as the model is
    /// sent it.
    session: Option<Session>,
    /// The agent that is running, and where it works.
    pub(crate) agent: Option<(agents::Kind, Agent)>,
    agent_found: Option<Found>,
    agent_folder: PathBuf,
    pub(crate) setup: Option<Setup>,
    /// Who the agent is signed in as, or what pays for it.
    pub(crate) account: Option<(String, String)>,
    /// The sign-in a connected API key goes through, for an agent that
    /// wants asking: tried once, when it says it needs signing in.
    key_way: Option<&'static str>,
    /// A message waiting for the agent to be ready for it.
    queued: Option<Vec<Block>>,
    /// The agent's program left while it had nothing to do: it is started
    /// again when next asked, not at once.
    pub(crate) rested: bool,
    /// The mark this window's Assistant gives its bridge.
    mark: String,
    held: Vec<Held>,
    /// A tool call of Tequila's own loop that the person is being asked
    /// about, and the question's number.
    call_waiting: Option<(u64, Call)>,
    waiting: HashMap<u64, Waiting>,
    next_question: u64,
    /// The window, for an agent's reader to wake.
    window: Option<egui::Context>,
    pub(crate) input: String,
    calls: VecDeque<Call>,
    results: Vec<Part>,
    shot: Option<Shot>,
    /// Tequila's count of the person's own view moves at the assistant's
    /// last screenshot, to say with the next that the view is theirs.
    view_moves: Option<u64>,
    plugin_wait: Option<PluginWait>,
    /// The undo depth when the current request began, and after the
    /// assistant's last change — for taking back everything one request did.
    turn_start: Option<usize>,
    /// The history as the request left it: its depth, and the bus's change
    /// count, which is what tells "nothing done since" from "undone by hand
    /// and as many edits made again".
    turn_end: Option<(usize, u64)>,
    providers_open: bool,
    draft: Option<Draft>,
    pub(crate) focus_input: bool,
    /// Each AI app: whether it is installed and lists Tequila, and when that
    /// was last looked at. Their settings files are read every few seconds
    /// while the window is open, not every frame.
    apps: Option<(std::time::Instant, Vec<(crate::connect_apps::App, bool, bool)>)>,
    /// What connecting an app last said.
    apps_message: Option<Result<String, String>>,
    /// When the request being worked on was asked, for the time shown.
    pub(crate) asked: Option<std::time::Instant>,
    /// The reply on its way from Tequila's own loop, as last looked at.
    pub(crate) live: Option<Progress>,
    /// When the chat was last written down.
    saved: Option<std::time::Instant>,
    /// A picture of the viewport the person asked to attach, on its way.
    viewport: Option<(PathBuf, std::time::Instant)>,
    /// What the panel keeps between frames.
    pub(crate) panel: crate::assistant_panel::State,
    /// The agents as the Connections page finds them, while it is open.
    pub(crate) connections: crate::assistant_connections::Connections,
}

impl Assistant {
    /// Load the connected providers from `path`, and find the chats kept
    /// beside it.
    pub fn load(&mut self, path: PathBuf) {
        let (settings, complaints) = Settings::load(&path);
        self.settings = settings;
        self.store = path.parent().map(|home| chat::Store::new(home.join("chats")));
        self.settings_path = Some(path);
        for complaint in complaints {
            self.chat.push(Item::Problem(complaint));
        }
    }

    pub(crate) fn save_settings(&mut self) {
        if let Some(path) = &self.settings_path {
            if let Err(error) = self.settings.save(path) {
                self.chat.push(Item::Problem(error));
            }
        }
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.focus_input = self.open;
    }

    /// Who answers: the agent or provider chosen, or the first provider
    /// connected where nothing has been.
    pub fn backend(&self) -> Option<Backend> {
        let chosen = self.settings.active.as_deref().unwrap_or_default();
        if let Some(kind) = agents::Kind::from_id(chosen) {
            return Some(Backend::Agent(kind));
        }
        self.settings.active().cloned().map(Backend::Provider)
    }

    /// Choose who answers. The chat goes on: whoever was not there for it
    /// is told how it went.
    pub(crate) fn choose(&mut self, backend: &Backend) {
        if self.working() || self.backend().as_ref() == Some(backend) {
            return;
        }
        self.settings.active = Some(backend.id());
        self.save_settings();
        self.session = None;
        self.end_agent();
        self.setup = None;
    }

    /// Choose who answers by name, for a script: `claude`, or a connected
    /// provider's name or id.
    pub fn use_named(&mut self, name: &str) -> Result<(), String> {
        let wanted = name.trim().to_lowercase();
        let backend = agents::Kind::ALL
            .into_iter()
            .find(|kind| kind.name().to_lowercase() == wanted || kind.id() == wanted)
            .map(Backend::Agent)
            .or_else(|| self.settings.providers.iter().find(|p| p.id == wanted || p.name.to_lowercase() == wanted).cloned().map(Backend::Provider))
            .ok_or_else(|| format!("nothing called {name:?} is connected to the Assistant"))?;
        if self.working() {
            return Err("the Assistant is still working on the last request; stop it first (stop=true)".into());
        }
        self.choose(&backend);
        Ok(())
    }

    fn agent_working(&self) -> bool {
        self.agent.as_ref().is_some_and(|(_, agent)| agent.phase() == acp::Phase::Working)
    }

    fn working(&self) -> bool {
        self.session.as_ref().is_some_and(Session::busy)
            || !self.calls.is_empty()
            || self.shot.is_some()
            || self.plugin_wait.is_some()
            || self.call_waiting.is_some()
            || self.agent_working()
            // Asked, and the agent still starting.
            || (self.queued.is_some() && self.setup.is_none())
    }

    /// The conversation so far, in words, for a backend that was not there
    /// for it: one chosen part-way, or an agent that no longer has it.
    fn retold(&self) -> Option<String> {
        let mut lines = Vec::new();
        // All but the message being sent now.
        for item in self.chat.items.iter().take(self.chat.items.len().saturating_sub(1)) {
            match item {
                Item::You { text, .. } => lines.push(format!("Person: {text}")),
                Item::Said(text) => lines.push(format!("Assistant: {text}")),
                Item::Step(step) if step.status == Status::Done => lines.push(format!("(Assistant did: {})", step.title)),
                _ => {}
            }
        }
        if lines.is_empty() {
            return None;
        }
        let mut told = lines.join("\n");
        if told.len() > RETOLD {
            let mut from = told.len() - RETOLD;
            while !told.is_char_boundary(from) {
                from += 1;
            }
            told = format!("\u{2026}{}", &told[from..]);
        }
        Some(format!("This conversation so far, which you were not part of. The scene may have changed since, so look before you rely on it.\n\n{told}\n\nThe person's new message follows."))
    }

    /// Start a request: called by the application, which knows what is
    /// around ([`Around`]).
    pub fn ask(&mut self, text: String, context: &str, pictures: Vec<Picture>, around: &Around) {
        if self.working() {
            return;
        }
        let Some(backend) = self.backend() else {
            self.chat.push(Item::Problem("Connect an AI first: Connections\u{2026}".into()));
            self.providers_open = true;
            return;
        };
        self.chat.push(Item::You { text: text.clone(), pictures: pictures.clone() });
        self.chat.backend = backend.id();
        let sent = if context.is_empty() { text } else { format!("{text}\n\n{context}") };
        self.turn_start = Some(around.depth);
        self.turn_end = None;
        self.asked = Some(std::time::Instant::now());
        match backend {
            Backend::Provider(provider) => self.ask_provider(provider, sent, &pictures),
            Backend::Agent(kind) => {
                let mut blocks = vec![Block::Text(sent)];
                blocks.extend(pictures.iter().map(|picture| Block::Picture(picture.png.to_vec())));
                self.ask_agent(kind, blocks, around);
            }
        }
        self.keep_chat();
    }

    fn ask_provider(&mut self, provider: Provider, sent: String, pictures: &[Picture]) {
        self.chat.model = provider.model.clone();
        let retold = if self.session.is_none() { self.retold() } else { None };
        let session = self.session.get_or_insert_with(|| Session::new(provider.clone()));
        // The conversation is kept in a neutral form, so switching provider
        // mid-conversation just sends it to the other one.
        session.provider = provider;
        let sent = match retold {
            Some(retold) => format!("{retold}\n\n{sent}"),
            None => sent,
        };
        let pictures: Vec<String> = pictures.iter().filter_map(|picture| prepare_image(&picture.png)).map(|png| tequila_ai::base64::encode(&png)).collect();
        if let Err(error) = session.ask_with(&sent, &pictures) {
            self.chat.push(Item::Problem(error));
            self.asked = None;
        }
    }

    fn ask_agent(&mut self, kind: agents::Kind, blocks: Vec<Block>, around: &Around) {
        // Nobody signed in: the message waits for that, and goes when the
        // person has signed in and tried again.
        if matches!(self.setup, Some(Setup::SignIn { .. })) {
            self.queued = Some(blocks);
            self.asked = None;
            return;
        }
        let ready = self.agent.as_ref().is_some_and(|(running, agent)| *running == kind && agent.phase() == acp::Phase::Idle);
        if ready {
            self.prompt(blocks);
            return;
        }
        self.queued = Some(blocks);
        let starting = self.agent.as_ref().is_some_and(|(running, agent)| *running == kind && agent.phase() == acp::Phase::Starting);
        if !starting && self.setup.is_none() {
            self.start_agent(kind, around);
        }
    }

    fn prompt(&mut self, blocks: Vec<Block>) {
        let Some((_, agent)) = &mut self.agent else { return };
        if blocks.iter().any(|b| matches!(b, Block::Picture(_))) && !agent.takes_pictures() {
            self.chat.push(Item::Note("This assistant does not take pictures: the ones attached were left out.".into()));
        }
        if let Err(error) = agent.prompt(&blocks) {
            self.chat.push(Item::Problem(format!("The message was not sent: {error}.")));
            self.asked = None;
        }
    }

    /// The mark this window's Assistant gives its bridge, made once.
    fn mark(&mut self) -> String {
        if self.mark.is_empty() {
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
            self.mark = format!("{nanos:x}-{:x}", std::process::id());
        }
        self.mark.clone()
    }

    /// Whether a request that came in over the scripting channel is this
    /// window's Assistant's.
    pub fn is_its(&self, agent: &str) -> bool {
        !agent.is_empty() && agent == self.mark
    }

    /// The name the console gives the Assistant's requests.
    pub fn peer(&self) -> String {
        format!("assistant \u{b7} {}", self.backend().map(|b| b.name()).unwrap_or_default())
    }

    /// Start the agent, if what it needs is here; say what is missing if not.
    pub(crate) fn start_agent(&mut self, kind: agents::Kind, around: &Around) {
        self.end_agent();
        self.account = None;
        self.rested = false;
        let found = match agents::find_agent(kind) {
            Ok(found) => found,
            Err(Missing::Adapter { node }) => {
                self.setup = Some(Setup::Missing { node });
                return;
            }
        };
        let (Some(program), Some(channel)) = (&around.program, &around.channel) else {
            let what = if around.program.is_none() {
                "Tequila's command-line program is not beside the application"
            } else {
                "the scripting channel is not open"
            };
            self.setup = Some(Setup::Failed(format!("{} cannot reach the scene from here: {what}.", kind.name())));
            return;
        };
        // An unsaved project has no folder of its own: the agent is given
        // an empty one of Tequila's, and has no files to look at.
        let folder = around.folder.clone().filter(|f| f.is_dir()).unwrap_or_else(|| {
            let folder = tequila_format::library::home_directory().join("assistant-workspace");
            let _ = std::fs::create_dir_all(&folder);
            folder
        });
        let (more_args, more_env) = agents::start_with(kind, &self.settings);
        self.key_way = kind.key_sign_in(&self.settings);
        let mut args = found.args.clone();
        args.extend(more_args);
        let mut env = found.env.clone();
        env.extend(more_env);
        // The earlier conversation, where this chat had one with this agent
        // in this folder.
        let resume = self.chat.session.clone().filter(|_| self.chat.backend == kind.id() && self.chat.folder.as_deref() == Some(folder.as_path()));
        let launch = acp::Launch {
            program: found.program.clone(),
            args,
            env,
            folder: folder.clone(),
            servers: vec![acp::McpServer {
                name: TOOLS.into(),
                command: program.clone(),
                args: vec!["--mcp".into(), "--assistant".into(), self.mark(), "--discovery".into(), channel.to_string_lossy().into_owned()],
                env: Vec::new(),
            }],
            resume,
            instructions: agents::INSTRUCTIONS.into(),
            meta: agents::options(kind),
            version: env!("CARGO_PKG_VERSION").into(),
        };
        let window = self.window.clone();
        let wake = move || {
            if let Some(window) = &window {
                window.request_repaint();
            }
        };
        match Agent::start(launch, wake) {
            Ok(agent) => {
                self.agent = Some((kind, agent));
                self.agent_found = Some(found);
                self.agent_folder = folder;
                self.setup = None;
            }
            Err(error) => self.setup = Some(Setup::Failed(error)),
        }
    }

    /// Let the agent go: its program is ended, and anything held for the
    /// person's answer is refused.
    fn end_agent(&mut self) {
        self.agent = None;
        self.refuse_held("The assistant was closed before the person answered.");
        self.waiting.clear();
    }

    /// Install the agent's program, when the person asks.
    pub(crate) fn install(&mut self, kind: agents::Kind) {
        self.setup = Some(Setup::Installing(agents::install(kind)));
    }

    /// Open one of the agent's own sign-ins, in a terminal.
    pub(crate) fn sign_in(&mut self, way: &acp::SignIn) {
        let Some(found) = &self.agent_found else { return };
        if let Err(error) = agents::sign_in(found, way) {
            self.setup = Some(Setup::Failed(error));
        }
    }

    /// Sign in by one of the ways the agent carries out itself: it opens
    /// its maker's own page in the browser, and says when that is done.
    pub(crate) fn authenticate(&mut self, way: &str) {
        if let Some((_, agent)) = &mut self.agent {
            agent.authenticate(way);
            self.setup = Some(Setup::SigningIn);
        }
    }

    /// Whether Claude may be signed in with a Claude subscription: the
    /// person's own choice, made in the sign-in card or on the Connections
    /// page. The agent is let go, to start again as chosen.
    pub(crate) fn set_claude_subscription(&mut self, on: bool) {
        if self.settings.claude_subscription == on || self.working() {
            return;
        }
        self.settings.claude_subscription = on;
        self.save_settings();
        if matches!(self.agent, Some((agents::Kind::Claude, _))) {
            self.end_agent();
        }
        if matches!(self.backend(), Some(Backend::Agent(agents::Kind::Claude))) {
            self.setup = None;
            self.rested = false;
        }
        self.connections.forget();
    }

    /// Try the agent again: after installing it, or signing in.
    pub(crate) fn retry(&mut self, around: &Around) {
        self.setup = None;
        if let Some(Backend::Agent(kind)) = self.backend() {
            self.start_agent(kind, around);
        }
    }

    pub fn stop(&mut self) {
        if let Some(session) = &mut self.session {
            session.cancel();
        }
        if let Some((_, agent)) = &mut self.agent {
            agent.cancel();
        }
        self.refuse_held("The person stopped the request.");
        self.waiting.clear();
        self.call_waiting = None;
        self.queued = None;
        self.calls.clear();
        self.results.clear();
        self.shot = None;
        self.plugin_wait = None;
        self.live = None;
        self.asked = None;
        self.chat.settle();
        self.chat.push(Item::Note("Stopped.".into()));
        self.keep_chat();
    }

    /// Whether a request is being worked on: waiting for the model, or
    /// carrying out what it asked for.
    pub fn is_working(&self) -> bool {
        self.working()
    }

    /// Whether the person has a question to answer before it goes on.
    pub fn is_asking(&self) -> bool {
        !self.waiting.is_empty()
    }

    /// What it is doing, in a few words, and the same as a word for a script.
    fn doing(&self) -> (String, &'static str) {
        if self.is_asking() {
            return ("Waiting for your answer".into(), "asking");
        }
        if self.shot.is_some() {
            return ("Looking at the scene".into(), "looking");
        }
        if self.plugin_wait.is_some() {
            return ("Running the plugin".into(), "tools");
        }
        if let Some((kind, agent)) = &self.agent {
            if agent.phase() == acp::Phase::Starting {
                return (format!("Starting {}", kind.name()), "loading");
            }
            if agent.phase() == acp::Phase::Working {
                return match self.chat.items.last() {
                    Some(Item::Thought { .. }) if self.chat.is_thinking() => ("Thinking".into(), "thinking"),
                    Some(Item::Said(_)) => ("Writing".into(), "writing"),
                    Some(Item::Step(step)) if step.status == Status::Running => (step.title.clone(), "calling"),
                    _ => ("Working".into(), "reading"),
                };
            }
        }
        if self.queued.is_some() {
            return ("Starting".into(), "loading");
        }
        let waiting = self.session.as_ref().is_some_and(Session::busy);
        match (&self.live, waiting) {
            (Some(live), true) => match &live.phase {
                Phase::Loading => ("Loading the model".into(), "loading"),
                Phase::Reading => ("Reading the request".into(), "reading"),
                Phase::Thinking => ("Thinking".into(), "thinking"),
                Phase::Writing => ("Writing".into(), "writing"),
                Phase::Calling(name) => (format!("Calling {}", name.trim_start_matches("tequila_")), "calling"),
            },
            (None, true) => ("Sending".into(), "reading"),
            (_, false) => ("Carrying out its steps".into(), "tools"),
        }
    }

    /// The line under the conversation while it works: what it is doing,
    /// how much it has written, which step.
    pub(crate) fn status_line(&self) -> String {
        let (doing, phase) = self.doing();
        let mut line = doing;
        if let Some(live) = &self.live {
            if matches!(phase, "thinking" | "writing" | "calling") && live.tokens > 0 {
                line.push_str(&format!(" · {} tokens", tequila_ai::ollama::thousands(live.tokens as u32)));
            }
        }
        let step = self.session.as_ref().map(Session::steps).unwrap_or(0);
        if step > 0 {
            line.push_str(&format!(" · step {}", step + 1));
        }
        line
    }

    /// What the Assistant is doing, for a script: whether it is working and
    /// on what, and the conversation as the panel shows it.
    pub fn status(&self) -> tequila_format::json::Value {
        use tequila_format::json::Value;
        let mut out = tequila_format::json_write::Object::new();
        out.set("open", self.open).set("working", self.working()).set("chat", self.chat.id.clone()).set("title", self.chat.title.clone());
        match self.backend() {
            Some(Backend::Provider(provider)) => {
                out.set("provider", provider.name.clone()).set("model", provider.model.clone());
            }
            Some(Backend::Agent(kind)) => {
                out.set("provider", kind.name());
                if let Some(model) = self.agent.as_ref().and_then(|(_, agent)| agent.setting("model")) {
                    out.set("model", model.current.clone());
                }
            }
            None => {}
        }
        if let Some((kind, label)) = &self.account {
            out.set("account", format!("{kind} {label}").trim().to_string());
        }
        if let Some((_, agent)) = &self.agent {
            let settings: Vec<Value> = agent.settings().iter().map(|setting| Value::String(format!("{}={}", setting.category, setting.current))).collect();
            out.set("settings", Value::Array(settings));
        }
        if self.providers_open {
            let rows: Vec<Value> = self.connections.status().into_iter().map(|(name, says)| Value::String(format!("{name}: {says}"))).collect();
            out.set("connections", Value::Array(rows));
        }
        if let Some(setup) = &self.setup {
            out.set(
                "setup",
                match setup {
                    Setup::Missing { .. } => "missing",
                    Setup::Installing(_) => "installing",
                    Setup::SignIn { .. } => "sign in",
                    Setup::SigningIn => "signing in",
                    Setup::Failed(_) => "failed",
                },
            );
        }
        if self.working() {
            let (doing, phase) = self.doing();
            out.set("doing", doing).set("phase", phase);
            out.set("step", self.session.as_ref().map(Session::steps).unwrap_or(0) as f64 + 1.0);
            out.set_some("seconds", self.asked.map(|a| (a.elapsed().as_secs_f64() * 10.0).round() / 10.0));
            if let Some(live) = &self.live {
                out.set("tokens", live.tokens as f64);
            }
        }
        let mut entries = Vec::new();
        let entry = |kind: &str, text: &str| {
            let mut item = tequila_format::json_write::Object::new();
            item.set("kind", kind).set("text", text);
            item
        };
        for item in &self.chat.items {
            match item {
                Item::You { text, pictures } => {
                    entries.push(entry("you", text).build());
                    entries.extend(pictures.iter().map(|_| entry("picture", "").build()));
                }
                Item::Said(text) => entries.push(entry("said", text).build()),
                Item::Thought { text, .. } => entries.push(entry("thought", text).build()),
                Item::Step(step) => {
                    let kind = match step.status {
                        Status::Running => "running step",
                        Status::Done => "step",
                        Status::Failed => "failed step",
                    };
                    entries.push(entry(kind, &step.title).build());
                    entries.extend(step.pictures.iter().map(|_| entry("picture", "").build()));
                }
                Item::Asked(asked) => {
                    let mut item = entry("asked", &asked.title);
                    item.set("id", asked.id).set("detail", asked.detail.clone());
                    item.set_some("answer", asked.answer);
                    entries.push(item.build());
                }
                Item::Problem(text) => entries.push(entry("problem", text).build()),
                Item::Note(text) => entries.push(entry("note", text).build()),
            }
        }
        out.set("transcript", Value::Array(entries));
        out.build()
    }

    /// Whether "undo what the assistant did" would take back only its work:
    /// something was changed, and nothing has been done by hand since.
    pub fn can_undo_turn(&self, bus: &tequila_scene::CommandBus) -> bool {
        match (self.turn_start, self.turn_end) {
            (Some(start), Some(end)) => end == (bus.depth(), bus.changes()) && bus.depth() > start,
            _ => false,
        }
    }

    /// The request's work no longer applies: another project was opened.
    pub fn forget_turn(&mut self) {
        self.turn_start = None;
        self.turn_end = None;
    }

    pub fn undo_turn(&mut self, app: &mut TequilaApp) {
        let (Some(start), true) = (self.turn_start, self.can_undo_turn(app.project.bus())) else {
            return;
        };
        let camera = app.project.camera().clone();
        while app.project.bus().depth() > start {
            let before = app.project.bus().depth();
            app.project.undo();
            if app.project.bus().depth() >= before {
                break;
            }
        }
        if *app.project.camera() != camera {
            app.camera.adopt(app.project.camera());
        }
        self.turn_end = None;
        self.chat.push(Item::Note("Took back everything that request changed.".into()));
    }

    /// Take the viewport's picture as it is now, for the message being
    /// written.
    pub fn send_viewport(&mut self, app: &mut TequilaApp) {
        if app.pending_screenshot.is_some() || self.viewport.is_some() {
            return;
        }
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let path = std::env::temp_dir().join(format!("tequila-assistant-viewport-{}-{nanos}.png", std::process::id()));
        app.pending_screenshot = Some(path.clone());
        app.screenshot_viewport = true;
        self.viewport = Some((path, std::time::Instant::now()));
    }

    /// Attach the viewport's picture once the window has written it.
    fn collect_viewport(&mut self) {
        let Some((path, asked)) = &self.viewport else { return };
        match std::fs::read(path).ok().filter(|bytes| bytes.ends_with(b"IEND\xaeB`\x82")) {
            Some(bytes) => {
                let _ = std::fs::remove_file(path);
                self.viewport = None;
                self.attach(&bytes);
            }
            None if asked.elapsed() > std::time::Duration::from_secs(10) => self.viewport = None,
            None => {}
        }
    }

    // ---- chats -----------------------------------------------------------------------

    /// Write the open chat down, if it has changed.
    fn keep_chat(&mut self) {
        if !self.chat.unsaved {
            return;
        }
        if let Some(store) = &self.store {
            if let Err(error) = store.save(&mut self.chat) {
                // Said once, not on every frame it fails.
                self.chat.items.push(Item::Problem(error));
            }
        }
        self.chat.unsaved = false;
        self.saved = Some(std::time::Instant::now());
    }

    /// Put away what belongs to the open chat, before another takes its
    /// place.
    fn close_chat(&mut self) {
        if self.working() {
            self.stop();
        }
        self.keep_chat();
        self.session = None;
        self.end_agent();
        self.queued = None;
        self.turn_start = None;
        self.turn_end = None;
        self.asked = None;
        self.live = None;
        if matches!(self.setup, Some(Setup::Failed(_))) {
            self.setup = None;
        }
    }

    pub fn new_chat(&mut self) {
        self.close_chat();
        self.chat = Chat::new();
        self.focus_input = true;
    }

    /// Open a past chat, to read and to carry on. It goes on with whoever
    /// answered it, where they are still connected.
    pub fn open_chat(&mut self, id: &str) {
        let Some(store) = self.store.clone() else { return };
        if id == self.chat.id {
            return;
        }
        match store.load(id) {
            Ok(chat) => {
                self.close_chat();
                let was = chat.backend.clone();
                let known = agents::Kind::from_id(&was).is_some() || self.settings.providers.iter().any(|p| p.id == was);
                if known && self.settings.active.as_deref() != Some(was.as_str()) {
                    self.settings.active = Some(was);
                    self.save_settings();
                    self.setup = None;
                }
                self.chat = chat;
                self.focus_input = true;
            }
            Err(error) => self.chat.push(Item::Problem(error)),
        }
    }

    pub fn delete_chat(&mut self, id: &str) {
        if id == self.chat.id {
            self.close_chat();
            self.chat = Chat::new();
        }
        if let Some(store) = &self.store {
            let _ = store.delete(id);
        }
    }

    // ---- questions the person answers ---------------------------------------------------

    fn question(&mut self, title: String, detail: String, waiting: Waiting) -> u64 {
        self.next_question += 1;
        let id = self.next_question;
        self.waiting.insert(id, waiting);
        self.chat.push(Item::Asked(Asked { id, title, detail, answer: None }));
        id
    }

    /// Keep a request of the Assistant's own until the person has said
    /// whether it may go ahead.
    pub fn hold(&mut self, request: Request, responder: Responder, question: Question) {
        // A file in the project's folder is named as it is known there.
        let folder = format!("{}{}", self.agent_folder.display(), std::path::MAIN_SEPARATOR);
        let detail = if folder.len() > 1 { question.detail.replace(&folder, "") } else { question.detail };
        let question = self.question(question.title, detail, Waiting::Held);
        self.held.push(Held { question, request, responder });
        self.open = true;
    }

    fn refuse_held(&mut self, why: &str) {
        for held in std::mem::take(&mut self.held) {
            held.responder.answer(&Reply::Failed(why.to_string()));
        }
    }

    /// The person's answer to a question in the chat: the agent is told, or
    /// the request that was held is carried out - or refused, in words the
    /// agent can pass on.
    pub fn answer(&mut self, app: &mut TequilaApp, question: u64, allow: bool) {
        if !self.chat.answer(question, allow) {
            return;
        }
        let ctx = self.window.clone().unwrap_or_default();
        match self.waiting.remove(&question) {
            Some(Waiting::Agent(permission)) => {
                if let Some((_, agent)) = &mut self.agent {
                    agent.permit(permission, allow);
                }
            }
            Some(Waiting::Held) => {
                let Some(at) = self.held.iter().position(|held| held.question == question) else { return };
                let held = self.held.remove(at);
                if !allow {
                    let reply = Reply::Failed("The person said no to this. Do not try it another way; tell them it was not done.".into());
                    crate::channel::log_reply(app, &held.request.peer, held.request.operation.summary(), &[], &reply);
                    held.responder.answer(&reply);
                    return;
                }
                match &held.request.operation {
                    Operation::Import { path, parent, .. } => crate::channel::begin_import(app, &held.request, path, *parent, held.responder, &ctx),
                    _ => held.responder.answer(&crate::channel::handle(app, &held.request)),
                }
                self.touched(app.project.bus());
            }
            Some(Waiting::Call) => {
                let Some((_, call)) = self.call_waiting.take() else { return };
                if allow {
                    self.carry_out_agreed(app, call, true);
                } else {
                    let label = step_label(&call);
                    self.result(call, "The person said no to this. Do not try it another way; tell them it was not done.".into(), None, true, label);
                }
            }
            None => {}
        }
    }

    /// One of the Assistant's requests has been carried out: what it
    /// changed is the request's, for "Undo request" to take back.
    pub fn touched(&mut self, bus: &tequila_scene::CommandBus) {
        if self.turn_start.is_some_and(|start| bus.depth() > start) {
            self.turn_end = Some((bus.depth(), bus.changes()));
        }
    }

    // ---- moving along ------------------------------------------------------------------

    /// What an agent has to say, taken into the chat.
    pub(crate) fn agent_events(&mut self, app: &mut TequilaApp) {
        let Some((kind, agent)) = &mut self.agent else { return };
        let kind = *kind;
        let events = agent.poll();
        for event in events {
            match event {
                acp::Event::Ready { resumed } => {
                    // Signed in a moment ago: the card has done its work.
                    if matches!(self.setup, Some(Setup::SigningIn)) {
                        self.setup = None;
                    }
                    let Some((_, agent)) = &mut self.agent else { continue };
                    self.chat.session = agent.session().map(str::to_string);
                    self.chat.folder = Some(self.agent_folder.clone());
                    // What Tequila chooses for the agent - that it always
                    // asks - and the model and thinking level the person
                    // chose last.
                    let pinned = agents::pinned(kind);
                    let wanted: Vec<(String, String)> = agent
                        .settings()
                        .iter()
                        .filter_map(|setting| {
                            let chosen = match pinned.iter().find(|(category, _)| *category == setting.category) {
                                Some((_, value)) => value.to_string(),
                                None => self.settings.choices.get(&format!("{}/{}", kind.id(), setting.category))?.clone(),
                            };
                            (chosen != setting.current && setting.choices.iter().any(|c| c.value == chosen)).then(|| (setting.id.clone(), chosen))
                        })
                        .collect();
                    for (setting, value) in wanted {
                        agent.set(&setting, &value);
                    }
                    // Open, and it has said nobody is signed in (Claude's
                    // way; Codex will not open at all): the ways to sign
                    // in are shown now, not when the first message fails.
                    if self.account.as_ref().is_some_and(|(kind, _)| kind == "none") && self.setup.is_none() {
                        let ways = self.agent.as_ref().map(|(_, agent)| agent.sign_ins().to_vec()).unwrap_or_default();
                        self.setup = Some(Setup::SignIn { ways, subscription: false });
                    }
                    // A message waiting goes now - unless nobody is signed
                    // in, when it waits for that.
                    let signed_out = matches!(self.setup, Some(Setup::SignIn { .. }));
                    let waiting = if signed_out { None } else { self.queued.take() };
                    if let Some(mut blocks) = waiting {
                        if !resumed {
                            if let Some(retold) = self.retold() {
                                blocks.insert(0, Block::Text(retold));
                            }
                        }
                        self.prompt(blocks);
                    }
                }
                acp::Event::SignIn(ways) => {
                    // A key the person has connected signs it in without a
                    // question, where the agent offers that way; once.
                    if let Some(way) = self.key_way.take().filter(|way| ways.iter().any(|w| w.id == *way)) {
                        if let Some((_, agent)) = &mut self.agent {
                            agent.authenticate(way);
                        }
                        continue;
                    }
                    let subscription = self.agent.as_ref().is_some_and(|(_, agent)| agent.signed_out_because == "claude_subscription_not_supported");
                    self.setup = Some(Setup::SignIn { ways, subscription });
                    self.asked = None;
                }
                acp::Event::Account { kind, label } => {
                    // Signed out while it was running: said at once. While
                    // it is still starting, what it says first may not be
                    // what it ends on (a key signs it in a moment later),
                    // so that is read when the conversation has opened.
                    let idle = self.agent.as_ref().is_some_and(|(_, agent)| agent.phase() == acp::Phase::Idle);
                    if kind == "none" && idle && self.setup.is_none() {
                        let ways = self.agent.as_ref().map(|(_, agent)| agent.sign_ins().to_vec()).unwrap_or_default();
                        self.setup = Some(Setup::SignIn { ways, subscription: false });
                    }
                    // And signed in, with a conversation open: the card that
                    // asked for it has done. (An agent can name an account
                    // and still refuse to open one - Claude, with a plan it
                    // was told not to use - and then the card stays.)
                    if kind != "none" && idle && matches!(self.setup, Some(Setup::SignIn { .. } | Setup::SigningIn)) {
                        self.setup = None;
                    }
                    self.account = Some((kind, label));
                }
                acp::Event::Said(text) => self.chat.say(&text),
                acp::Event::Thought(text) => self.chat.think(&text),
                acp::Event::Tool(tool) => self.chat.step(step_news(&tool)),
                acp::Event::Permission(permission) => {
                    self.chat.step(step_news(&permission.tool));
                    match acp::judge(&permission.tool, &self.agent_folder, TOOLS) {
                        acp::Verdict::Allow => {
                            if let Some((_, agent)) = &mut self.agent {
                                agent.permit(permission.id, true);
                            }
                        }
                        acp::Verdict::Deny(why) => {
                            if let Some((_, agent)) = &mut self.agent {
                                agent.permit(permission.id, false);
                            }
                            self.chat.step(StepNews { id: permission.tool.id.clone(), status: Some(Status::Failed), detail: Some(why), ..StepNews::default() });
                        }
                        acp::Verdict::Ask { title, detail } => {
                            self.question(title, detail, Waiting::Agent(permission.id));
                        }
                    }
                }
                acp::Event::Plan(_) | acp::Event::Settings => {}
                acp::Event::Title(title) => {
                    self.chat.title = title;
                    self.chat.unsaved = true;
                }
                acp::Event::Notice(text) => self.chat.push(Item::Note(text)),
                acp::Event::Finished(stop) => {
                    self.refuse_held("The request ended before the person answered.");
                    self.waiting.clear();
                    self.chat.settle();
                    match stop {
                        acp::Stop::Done | acp::Stop::Stopped => {}
                        acp::Stop::MaxTokens => self.chat.push(Item::Note("It ran out of room for its reply. Ask again to carry on.".into())),
                        acp::Stop::MaxSteps => self.chat.push(Item::Note("It stopped after as many steps as one request may take. Ask again to let it continue.".into())),
                        acp::Stop::Refused => self.chat.push(Item::Note("It declined this request.".into())),
                        acp::Stop::Other(why) => self.chat.push(Item::Note(format!("It stopped: {why}."))),
                    }
                    self.finish_turn(app);
                    self.asked = None;
                    self.keep_chat();
                }
                acp::Event::Failed(error) => {
                    self.refuse_held("The request failed before the person answered.");
                    self.waiting.clear();
                    self.chat.settle();
                    if self.agent.as_ref().is_some_and(|(_, agent)| agent.phase() == acp::Phase::Gone) {
                        // It could not be started at all: said on the card
                        // with Try again, not tried again every frame.
                        self.agent = None;
                        self.setup = Some(Setup::Failed(error));
                    } else {
                        self.chat.push(Item::Problem(error));
                        self.chat.push(Item::Note("The conversation is kept: ask again to carry on.".into()));
                    }
                    self.queued = None;
                    self.asked = None;
                    self.keep_chat();
                }
                acp::Event::Exited(said) => {
                    self.refuse_held("The assistant's program stopped before the person answered.");
                    self.waiting.clear();
                    let was_working = self.asked.is_some() || self.queued.is_some();
                    let opened = self.agent.as_ref().is_some_and(|(_, agent)| agent.session().is_some());
                    self.chat.settle();
                    // Left quietly while idle, it is started again when next
                    // asked; mid-request, the person is told; and one that
                    // ended before it had opened a conversation would only
                    // do so again, so that is said on the card instead.
                    if !opened {
                        let said = said.trim();
                        self.setup = Some(Setup::Failed(if said.is_empty() {
                            format!("{}'s program stopped as it started.", kind.name())
                        } else {
                            format!("{}'s program stopped as it started: {said}", kind.name())
                        }));
                    } else if was_working {
                        let said = said.trim();
                        self.chat.push(Item::Problem(if said.is_empty() {
                            format!("{}'s program stopped.", kind.name())
                        } else {
                            format!("{}'s program stopped: {said}", kind.name())
                        }));
                        self.chat.push(Item::Note("The conversation is kept: ask again to carry on.".into()));
                    }
                    self.agent = None;
                    self.rested = opened;
                    // A message that never reached it waits for Try again.
                    if opened {
                        self.queued = None;
                    }
                    self.asked = None;
                    self.keep_chat();
                }
            }
        }
    }

    /// How an install the person asked for went.
    fn installing(&mut self, around: &Around) {
        let Some(Setup::Installing(receiver)) = &self.setup else { return };
        match receiver.try_recv() {
            Ok(Ok(_)) => self.retry(around),
            Ok(Err(error)) => self.setup = Some(Setup::Failed(error)),
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.setup = Some(Setup::Failed("The install stopped before it finished.".into())),
        }
    }

    /// Move the conversation along: collect replies, carry out tool calls,
    /// send results. Called every frame.
    pub fn pump(&mut self, app: &mut TequilaApp, ctx: &egui::Context) {
        self.window = Some(ctx.clone());
        self.agent_events(app);
        if matches!(self.setup, Some(Setup::Installing(_))) {
            self.installing(&app.around());
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        // What the request has come to, and what it is doing; then the
        // session is let go of, for the rest to touch the panel.
        let (polled, live, notes) = match &mut self.session {
            Some(session) => {
                let live = session.progress();
                let polled = session.poll();
                (polled, live, session.take_notes())
            }
            None => (None, None, Vec::new()),
        };
        match polled {
            Some(Ok(turn)) => {
                // How long it reasoned, where that is known; the round's time if not.
                let seconds = live
                    .as_ref()
                    .map(|l| if l.thought_for.is_zero() { l.since.elapsed() } else { l.thought_for }.as_secs_f32())
                    .unwrap_or(0.0);
                if !turn.reasoning.trim().is_empty() {
                    self.chat.push(Item::Thought { text: turn.reasoning.trim().to_string(), seconds });
                }
                let said = !turn.text.is_empty();
                for text in turn.text {
                    self.chat.push(Item::Said(text));
                }
                self.calls.extend(turn.calls);
                if self.calls.is_empty() {
                    if !said {
                        self.chat.push(Item::Note(
                            "It finished without an answer: it may have run out of room for its reply (Connections\u{2026}, Edit, Advanced: Longest reply). Ask again to carry on.".into(),
                        ));
                    }
                    self.finish_turn(app);
                }
                self.live = None;
            }
            Some(Err(error)) => {
                self.live = None;
                if error != tequila_ai::http::STOPPED {
                    let again = error.to_lowercase().contains("ask again");
                    self.chat.push(Item::Problem(error));
                    if !again {
                        self.chat.push(Item::Note("The conversation is kept: ask again to carry on.".into()));
                    }
                }
            }
            None => self.live = live,
        }
        for note in notes {
            self.chat.push(Item::Note(note));
        }

        self.collect_screenshot();
        self.collect_viewport();
        if self.viewport.is_some() {
            ctx.request_repaint();
        }
        self.collect_plugin_run(app);
        while self.shot.is_none() && self.plugin_wait.is_none() && self.call_waiting.is_none() {
            let Some(call) = self.calls.pop_front() else { break };
            if !self.carry_out(app, call) {
                // Waiting on something outside the assistant; next frame.
                break;
            }
        }

        let answered = self.calls.is_empty() && self.shot.is_none() && self.plugin_wait.is_none() && self.call_waiting.is_none() && !self.results.is_empty();
        if answered {
            let results = std::mem::take(&mut self.results);
            if let Some(session) = &mut self.session {
                if let Err(error) = session.answer(results) {
                    self.chat.push(Item::Problem(error));
                }
            }
        }
        if self.working() {
            // Replies arrive on another thread; keep looking for them even
            // when nobody is moving the mouse.
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            // Written down now and then while it works, so a crash loses
            // seconds of a long request, not all of it.
            if self.saved.is_none_or(|at| at.elapsed() > std::time::Duration::from_secs(5)) {
                self.keep_chat();
            }
        } else {
            self.asked = None;
            self.live = None;
            self.keep_chat();
        }
    }

    /// A tool call of Tequila's own loop, as a step in the chat.
    fn stepped(&mut self, call: &Call, label: String, status: Status, detail: String) {
        self.chat.step(StepNews { id: call.id.clone(), title: Some(label), status: Some(status), detail: Some(detail), ..StepNews::default() });
    }

    fn finish_turn(&mut self, app: &TequilaApp) {
        let depth = app.project.bus().depth();
        if self.turn_start.is_some_and(|start| depth > start) {
            self.turn_end = Some((depth, app.project.bus().changes()));
        }
    }

    /// Carry out one call. False when it has to wait for a later frame.
    fn carry_out(&mut self, app: &mut TequilaApp, call: Call) -> bool {
        self.carry_out_agreed(app, call, false)
    }

    /// The same, `agreed` when the person has said yes to it already.
    fn carry_out_agreed(&mut self, app: &mut TequilaApp, call: Call, agreed: bool) -> bool {
        let label = step_label(&call);
        let arguments = match call.arguments.clone() {
            Ok(arguments) => arguments,
            Err(error) => {
                self.result(call, error, None, true, label);
                return true;
            }
        };
        let planned = tools::plan(&call.name, &arguments).map(|plan| in_project(plan, app.project.path.as_deref().and_then(std::path::Path::parent)));
        // What leaves the scene waits for the person, as it does for an
        // agent: the same questions, asked of the same request.
        if !agreed {
            let request = match &planned {
                Ok(tools::Plan::Request { json, .. } | tools::Plan::Material { json, .. } | tools::Plan::PluginRun { json, .. }) => parse_request(json).ok(),
                _ => None,
            };
            if let Some(question) = request.and_then(|request| tequila_scripting::approval::question(&request.operation, &std::env::temp_dir())) {
                self.stepped(&call, label, Status::Running, String::new());
                let folder = app.project.path.as_deref().and_then(std::path::Path::parent).map(|f| format!("{}{}", f.display(), std::path::MAIN_SEPARATOR));
                let detail = match folder {
                    Some(folder) => question.detail.replace(&folder, ""),
                    None => question.detail,
                };
                let asked = self.question(question.title, detail, Waiting::Call);
                self.call_waiting = Some((asked, call));
                return false;
            }
        }
        match planned {
            Err(error) => self.result(call, error, None, true, label),
            Ok(tools::Plan::Screenshot { viewport }) => {
                if app.pending_screenshot.is_some() {
                    // Someone else's capture is in flight; ask again next frame.
                    self.calls.push_front(call);
                    return false;
                }
                let path = std::env::temp_dir().join(format!(
                    "tequila-assistant-{}-{}.png",
                    std::process::id(),
                    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
                ));
                app.pending_screenshot = Some(path.clone());
                app.screenshot_viewport = viewport;
                // Given time to finish its picture, as the bridge's is.
                app.screenshot_settle = Some((std::time::Instant::now(), 12.0));
                self.stepped(&call, label, Status::Running, String::new());
                // Whose the view is, when the person has moved it since
                // the last look: said with the picture, as the bridge says
                // it (`channel::note_view`).
                let moves = crate::channel::view_moves_by_hand(app);
                let moved = self.view_moves.is_some_and(|before| moves > before);
                self.view_moves = Some(moves);
                let mut caption = String::from("The screenshot, as the application shows it now.");
                if moved {
                    caption.push_str(" The person has moved the view since your last look: it is where they want it, so leave it there.");
                }
                self.shot = Some(Shot { call, path, asked: std::time::Instant::now(), caption });
            }
            Ok(tools::Plan::Material { json, picture }) => {
                if self.shot.is_some() || app.pending_screenshot.is_some() {
                    // A picture is on its way already; ask again next frame.
                    self.calls.push_front(call);
                    return false;
                }
                let mut request = match parse_request(&json) {
                    Ok(request) => request,
                    Err(error) => {
                        self.result(call, error.to_string(), None, true, label);
                        return true;
                    }
                };
                let name = self.session.as_ref().map(|s| s.provider.name.clone()).unwrap_or_default();
                request.peer = format!("assistant · {name}");
                let path = std::env::temp_dir().join(format!(
                    "tequila-assistant-material-{}-{}.png",
                    std::process::id(),
                    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
                ));
                let previewing = matches!(&request.operation, tequila_scripting::protocol::Operation::Material(m) if m.action == "preview");
                if let tequila_scripting::protocol::Operation::Material(material) = &mut request.operation {
                    if previewing {
                        material.path = Some(path.display().to_string());
                    }
                }
                let answer = match crate::channel::handle(app, &request) {
                    Reply::Value(value) => tools::describe(&tools::Shape::Raw, &value),
                    Reply::Done => "done".into(),
                    Reply::Failed(message) => {
                        self.result(call, message, None, true, label);
                        return true;
                    }
                };
                // The material made or changed, shown on the ball as well.
                let shown = previewing || (picture && {
                    let made = tequila_format::json::parse(answer.as_bytes()).ok().and_then(|v| v.get("material").and_then(|m| m.as_index()));
                    request.operation = tequila_scripting::protocol::Operation::Material(tequila_scripting::protocol::MaterialRequest {
                        action: "preview".into(),
                        material: made.map(|m| m as u64),
                        path: Some(path.display().to_string()),
                        ..Default::default()
                    });
                    made.is_some() && matches!(crate::channel::handle(app, &request), Reply::Value(_))
                });
                let depth = app.project.bus().depth();
                if self.turn_start.is_some_and(|start| depth > start) {
                    self.turn_end = Some((depth, app.project.bus().changes()));
                }
                self.stepped(&call, label, if shown { Status::Running } else { Status::Done }, String::new());
                if shown {
                    self.shot = Some(Shot { call, path, asked: std::time::Instant::now(), caption: answer });
                } else {
                    self.results.push(Part::Result { id: call.id, name: call.name, text: answer, image: None, is_error: false });
                }
            }
            Ok(tools::Plan::PluginRun { json, plugin, command }) => {
                let log_total = |app: &mut TequilaApp, plugin: &str| -> u64 {
                    app.plugins.logged.get(plugin).copied().unwrap_or(0)
                };
                let before = log_total(app, &plugin);
                let mut request = match parse_request(&json) {
                    Ok(request) => request,
                    Err(error) => {
                        self.result(call, error.to_string(), None, true, label);
                        return true;
                    }
                };
                let name = self.session.as_ref().map(|s| s.provider.name.clone()).unwrap_or_default();
                request.peer = format!("assistant · {name}");
                match crate::channel::handle(app, &request) {
                    Reply::Failed(message) => self.result(call, message, None, true, label),
                    Reply::Value(value) if value.get("opened_panel").is_some() => {
                        self.result(call, tools::describe(&tools::Shape::Raw, &value), None, false, label)
                    }
                    _ => {
                        self.stepped(&call, label, Status::Running, String::new());
                        self.plugin_wait = Some(PluginWait { call, plugin, command, before, asked: std::time::Instant::now() });
                    }
                }
            }
            Ok(tools::Plan::Request { json, shape }) => {
                let mut request = match parse_request(&json) {
                    Ok(request) => request,
                    Err(error) => {
                        self.result(call, error.to_string(), None, true, label);
                        return true;
                    }
                };
                // Named, so the console shows who is acting.
                let name = self.session.as_ref().map(|s| s.provider.name.clone()).unwrap_or_default();
                request.peer = format!("assistant · {name}");
                match crate::channel::handle(app, &request) {
                    Reply::Done => self.result(call, "done".into(), None, false, label),
                    Reply::Value(value) => self.result(call, tools::describe(&shape, &value), None, false, label),
                    Reply::Failed(message) => self.result(call, message, None, true, label),
                }
                let depth = app.project.bus().depth();
                if self.turn_start.is_some_and(|start| depth > start) {
                    self.turn_end = Some((depth, app.project.bus().changes()));
                }
            }
        }
        true
    }

    fn result(&mut self, call: Call, text: String, image: Option<String>, is_error: bool, label: String) {
        let detail = if is_error { text.clone() } else { String::new() };
        if image.is_none() {
            self.stepped(&call, label, if is_error { Status::Failed } else { Status::Done }, detail);
        }
        self.results.push(Part::Result { id: call.id, name: call.name, text, image, is_error });
    }

    /// Tell the model how a plugin's command went once its log says - or,
    /// after a while, what it has logged so far.
    fn collect_plugin_run(&mut self, app: &mut TequilaApp) {
        let Some(wait) = &self.plugin_wait else { return };
        let total = app.plugins.logged.get(&wait.plugin).copied().unwrap_or(0);
        let lines: Vec<String> = app
            .plugins
            .logs
            .get(&wait.plugin)
            .map(|log| log.iter().skip(log.len().saturating_sub(total.saturating_sub(wait.before) as usize)).cloned().collect())
            .unwrap_or_default();
        let finished = format!("finished `{}`", wait.command);
        let done = lines.contains(&finished);
        let stopped = !app.plugins.hosts.contains_key(&wait.plugin);
        if !done && !stopped && wait.asked.elapsed() < PLUGIN_WAIT {
            return;
        }
        let wait = self.plugin_wait.take().expect("checked above");
        let mut out = tequila_format::json_write::Object::new();
        out.set("finished", done)
            .set("log", tequila_format::json::Value::Array(lines.into_iter().map(tequila_format::json::Value::String).collect()));
        if !done {
            out.set("note", if stopped { "the plugin is not running; tequila_plugin_list says why" } else { "still running; tequila_plugin_log shows more later" });
        }
        let text = tequila_format::json_write::to_string(&out.build());
        self.chat.step(StepNews { id: wait.call.id.clone(), status: Some(Status::Done), ..StepNews::default() });
        self.results.push(Part::Result { id: wait.call.id, name: wait.call.name, text, image: None, is_error: false });
    }

    /// Hand a finished screenshot to the model, and show it here.
    fn collect_screenshot(&mut self) {
        let Some(shot) = &self.shot else { return };
        let ready = std::fs::read(&shot.path).ok().filter(|bytes| bytes.ends_with(b"IEND\xaeB`\x82"));
        let Some(bytes) = ready else {
            if shot.asked.elapsed() > std::time::Duration::from_secs(25) {
                let shot = self.shot.take().expect("checked above");
                self.result(shot.call, "the screenshot did not arrive".into(), None, true, "screenshot".into());
            }
            return;
        };
        let shot = self.shot.take().expect("checked above");
        let _ = std::fs::remove_file(&shot.path);
        match prepare_image(&bytes) {
            Some(png) => {
                let sent = tequila_ai::base64::encode(&png);
                self.chat.step(StepNews { id: shot.call.id.clone(), status: Some(Status::Done), pictures: vec![Picture::new(png)], ..StepNews::default() });
                self.results.push(Part::Result { id: shot.call.id, name: shot.call.name, text: shot.caption, image: Some(sent), is_error: false });
            }
            None => self.result(shot.call, "the screenshot could not be read".into(), None, true, "screenshot".into()),
        }
    }


    // ---- connecting providers -------------------------------------------------------

    /// Whether the Connect an AI window is open.
    pub fn providers_open(&self) -> bool {
        self.providers_open
    }

    /// Open the Connect an AI window on a new provider from a starting
    /// point: `claude`, for an Anthropic API key.
    pub(crate) fn add_provider(&mut self, preset: &str) {
        if let Some(mut provider) = Provider::preset(preset) {
            provider.id = self.settings.new_id(&provider.name);
            self.draft = Some(Draft::of(None, provider));
            self.providers_open = true;
        }
    }

    /// Open or close the Connect an AI window.
    pub fn set_providers_open(&mut self, open: bool) {
        self.providers_open = open;
        if !open {
            self.draft = None;
            // The agents asked on the page are let go, and the chat looks
            // again at what its own still needs: it may have been installed
            // or signed in to meanwhile.
            self.connections.forget();
            if matches!(self.setup, Some(Setup::Missing { .. } | Setup::SignIn { .. } | Setup::Failed(_))) && !self.working() {
                self.setup = None;
                self.rested = false;
            }
        }
    }

    pub fn providers_window(&mut self, ctx: &egui::Context) {
        if !self.providers_open {
            return;
        }
        let mut open = true;
        let spec = crate::windows::Spec {
            kind: crate::windows::Kind::Connect,
            title: "Connections".into(),
            size: egui::vec2(580.0, 620.0),
            min_size: egui::vec2(420.0, 300.0),
            resizable: true,
            margin: 16.0,
        };
        crate::windows::show(ctx, spec, &mut open, |window| window.default_width(480.0).resizable(true), |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
                if self.draft.is_some() {
                    self.edit_draft(ui);
                    return;
                }
                self.agents_section(ui);
                ui.add_space(4.0);
                ui.separator();
                ui.label(egui::RichText::new("API keys and this computer").strong());
                ui.label(
                    egui::RichText::new("A service's API key, billed by use, or a model running here (Ollama, LM Studio). Tequila's own loop talks to these.")
                        .small()
                        .color(crate::theme::dim()),
                );
                if self.settings.providers.is_empty() {
                    ui.label(egui::RichText::new("Nothing connected yet.").color(crate::theme::dim()));
                }
                let mut remove = None;
                let mut edit = None;
                let mut use_provider = None;
                for provider in &self.settings.providers {
                    ui.horizontal(|ui| {
                        let active = self.settings.active.as_deref() == Some(provider.id.as_str());
                        if ui.radio(active, "").on_hover_text("Use this one in the chat").clicked() {
                            use_provider = Some(provider.clone());
                        }
                        ui.label(egui::RichText::new(&provider.name).strong());
                        ui.label(egui::RichText::new(format!("{} · {}", provider.kind.label(), provider.model)).color(crate::theme::dim()));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Remove").clicked() {
                                remove = Some(provider.id.clone());
                            }
                            if ui.small_button("Edit").clicked() {
                                edit = Some(provider.clone());
                            }
                        });
                    });
                }
                if let Some(id) = remove {
                    self.settings.providers.retain(|p| p.id != id);
                    if self.settings.active.as_deref() == Some(id.as_str()) {
                        self.settings.active = self.settings.providers.first().map(|p| p.id.clone());
                    }
                    self.save_settings();
                }
                if let Some(provider) = edit {
                    self.draft = Some(Draft::of(Some(provider.id.clone()), provider));
                }
                if let Some(provider) = use_provider {
                    self.choose(&Backend::Provider(provider));
                }
                ui.add_space(2.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label("Add:");
                    for preset in Provider::presets() {
                        if ui.button(&preset.name).clicked() {
                            let mut provider = preset;
                            provider.id = self.settings.new_id(&provider.name);
                            self.draft = Some(Draft::of(None, provider));
                        }
                    }
                });
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(tequila_ai::secret::kept())
                    .small()
                    .color(crate::theme::dim()),
                );
                ui.add_space(4.0);
                ui.separator();
                // The other direction: AI apps that use Tequila from
                // outside, rather than answering inside it.
                egui::CollapsingHeader::new(egui::RichText::new("AI apps that can use Tequila").strong()).id_salt("connections-apps").default_open(false).show(ui, |ui| {
                    self.apps_section(ui);
                });
            });
        });
        if !open {
            self.set_providers_open(false);
        }
    }

    /// The AI apps on this computer, connected to Tequila's tools in one
    /// click, so asked to work in Tequila they use them rather than the
    /// mouse.
    fn apps_section(&mut self, ui: &mut egui::Ui) {
        use crate::connect_apps::App;
        let dim = crate::theme::dim();
        ui.label(
            egui::RichText::new(
                "Lets an AI app work in Tequila through its tools - exact, fast and undoable - \
                 instead of clicking around the screen. Any app that speaks MCP can.",
            )
            .small()
            .color(dim),
        );
        let Some(program) = crate::connect_apps::server_program() else {
            ui.label(
                egui::RichText::new("Tequila's command-line program is not beside the application, so no app can be connected from here.")
                    .small()
                    .color(dim),
            );
            return;
        };
        let stale = self.apps.as_ref().is_none_or(|(at, _)| at.elapsed() > std::time::Duration::from_secs(3));
        if stale {
            let status = App::ALL
                .iter()
                .map(|&app| (app, app.installed(), app.settings().iter().any(|path| app.is_connected(path, &program))))
                .collect();
            self.apps = Some((std::time::Instant::now(), status));
        }
        let status = self.apps.as_ref().map(|(_, s)| s.clone()).unwrap_or_default();
        let skill = crate::connect_apps::skill_file();
        let mut row = |ui: &mut egui::Ui, app: App, connected: bool| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(app.name()).strong());
                if connected {
                    ui.colored_label(crate::theme::p().success, "Connected");
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let (Some(skills), Some(skill)) = (app.skills(), &skill) {
                        if ui.small_button("Add the skill").on_hover_text(skills.join("tequila").display().to_string()).clicked() {
                            self.apps_message = Some(crate::connect_apps::install_skill(skill, &skills).map(|folder| {
                                format!("Tequila's skill is in {}: new {} sessions use it.", folder.display(), app.name())
                            }));
                        }
                    }
                    match app.command(&program) {
                        Some(command) => {
                            if ui.small_button("Copy the command").on_hover_text(&command).clicked() {
                                ui.ctx().copy_text(command);
                                self.apps_message = Some(Ok(format!("Copied. {}", app.after())));
                            }
                        }
                        None => {
                            if ui.small_button(if connected { "Connect again" } else { "Connect" }).clicked() {
                                let failures: Vec<String> =
                                    app.settings().iter().filter_map(|path| app.connect(path, &program).err()).collect();
                                self.apps_message = Some(if failures.is_empty() {
                                    Ok(format!("{} is connected. {}", app.name(), app.after()))
                                } else {
                                    Err(failures.join("\n"))
                                });
                                self.apps = None;
                            }
                        }
                    }
                });
            });
        };
        let (here, elsewhere): (Vec<_>, Vec<_>) = status.iter().partition(|(_, installed, _)| *installed);
        for (app, _, connected) in &here {
            row(ui, *app, *connected);
        }
        if here.is_empty() {
            ui.label(egui::RichText::new("None of the apps below is on this computer yet.").small().color(dim));
        }
        if !elsewhere.is_empty() {
            egui::CollapsingHeader::new(egui::RichText::new("Other apps").small()).id_salt("other-ai-apps").show(ui, |ui| {
                for (app, _, connected) in &elsewhere {
                    row(ui, *app, *connected);
                }
            });
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("Any other MCP app:").small().color(dim));
            if ui.small_button("Copy the settings").on_hover_text("The mcpServers entry most apps read, with Tequila's program").clicked() {
                ui.ctx().copy_text(crate::connect_apps::generic_settings(&program));
                self.apps_message = Some(Ok("Copied: paste it into the app's MCP settings.".into()));
            }
        });
        ui.label(
            egui::RichText::new(
                "ChatGPT reaches MCP servers over the internet: OpenAI's Secure MCP Tunnel connects it (see the Assistant guide), \
                 or connect an OpenAI key below for Tequila's own Assistant.",
            )
            .small()
            .color(dim),
        );
        // The skill that teaches an AI to work in Tequila: what to look at
        // first, the library, colours, floor reflections, renders.
        if let Some(skill) = &skill {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new("Better results: add Tequila's skill. In the Claude app, Settings, Capabilities, Skills: upload the file.")
                        .small()
                        .color(dim),
                );
                if ui.small_button("Show the skill file").clicked() {
                    crate::library_tab::reveal(&skill.to_string_lossy());
                }
            });
        }
        match &self.apps_message {
            Some(Ok(text)) => {
                ui.add(egui::Label::new(egui::RichText::new(text).small().color(crate::theme::p().success)).wrap());
            }
            Some(Err(text)) => {
                ui.add(egui::Label::new(egui::RichText::new(text).small().color(crate::theme::p().danger)).wrap());
            }
            None => {}
        }
    }

    fn edit_draft(&mut self, ui: &mut egui::Ui) {
        let Some(draft) = &mut self.draft else { return };
        draft.poll();
        let provider = &mut draft.provider;
        egui::Grid::new("provider-form").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut provider.name);
            ui.end_row();

            ui.label("Type");
            egui::ComboBox::from_id_salt("provider-kind").selected_text(provider.kind.label()).show_ui(ui, |ui| {
                for kind in Kind::ALL {
                    if ui.selectable_label(provider.kind == kind, kind.label()).clicked() && provider.kind != kind {
                        if provider.base_url.is_empty() || provider.base_url == provider.kind.default_url() {
                            provider.base_url = kind.default_url().into();
                        }
                        provider.kind = kind;
                    }
                }
            });
            ui.end_row();

            ui.label("Address");
            ui.add(egui::TextEdit::singleline(&mut provider.base_url).desired_width(320.0).hint_text(match provider.kind {
                Kind::Anthropic => "https://api.anthropic.com",
                Kind::OpenAi => "https://…/v1",
            }));
            ui.end_row();

            ui.label("Model");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut provider.model).desired_width(220.0).hint_text("the model's id"));
                if !draft.models.is_empty() {
                    egui::ComboBox::from_id_salt("provider-models").selected_text("Choose…").show_ui(ui, |ui| {
                        for model in &draft.models {
                            if ui.selectable_label(provider.model == *model, model).clicked() {
                                provider.model = model.clone();
                            }
                        }
                    });
                }
            });
            ui.end_row();

            ui.label("Key");
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.radio_value(&mut draft.key_mode, KeyMode::Stored, "API key");
                    ui.radio_value(&mut draft.key_mode, KeyMode::Env, "Environment variable");
                    ui.radio_value(&mut draft.key_mode, KeyMode::None, "None (local server)");
                });
                match draft.key_mode {
                    KeyMode::Stored => {
                        let kept = matches!(provider.key, Key::Stored(_));
                        ui.add(
                            egui::TextEdit::singleline(&mut draft.key_input)
                                .password(true)
                                .desired_width(320.0)
                                .hint_text(if kept { "saved — type a new one to replace it" } else { "paste the API key" }),
                        );
                    }
                    KeyMode::Env => {
                        ui.add(egui::TextEdit::singleline(&mut draft.env_name).desired_width(220.0).hint_text("e.g. ANTHROPIC_API_KEY"));
                    }
                    KeyMode::None => {}
                }
                // Someone given a key and nothing else: what the key says
                // about where it belongs, filled in where that is certain.
                if draft.key_mode == KeyMode::Stored && !draft.key_input.trim().is_empty() {
                    match tequila_ai::provider::recognise_key(&draft.key_input) {
                        tequila_ai::provider::KeyLooks::Known { service, kind, base_url } => {
                            if provider.base_url.trim().is_empty() || provider.kind != kind && provider.base_url == provider.kind.default_url() {
                                provider.kind = kind;
                                provider.base_url = base_url.into();
                            }
                            ui.label(
                                egui::RichText::new(format!("A {service} key: its address is filled in. Click Fetch models to choose one."))
                                    .small()
                                    .color(crate::theme::p().success),
                            );
                        }
                        tequila_ai::provider::KeyLooks::OpenAiOrGateway if provider.base_url.trim().is_empty() => {
                            ui.label(
                                egui::RichText::new(
                                    "This could be an OpenAI key or one from a company gateway, which use the same start. \
                                     If it came from your IT team, ask them for the API address too.",
                                )
                                .small()
                                .color(crate::theme::dim()),
                            );
                            if ui.small_button("It's an OpenAI key").clicked() {
                                provider.kind = Kind::OpenAi;
                                provider.base_url = Kind::OpenAi.default_url().into();
                            }
                        }
                        tequila_ai::provider::KeyLooks::Unknown if provider.base_url.trim().is_empty() => {
                            ui.label(
                                egui::RichText::new(
                                    "This key does not say which service it belongs to, so Tequila needs the service's \
                                     address as well. Ask whoever gave you the key for \"the API base URL\" (it often ends \
                                     in /v1) and whether it is OpenAI- or Anthropic-compatible.",
                                )
                                .small()
                                .color(crate::theme::dim()),
                            );
                        }
                        _ => {}
                    }
                }
            });
            ui.end_row();
        });

        crate::field::Fold::new("Advanced").id_salt("provider-advanced").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Key header");
                ui.add(egui::TextEdit::singleline(&mut provider.key_header).desired_width(160.0).hint_text(match provider.kind {
                    Kind::Anthropic => "x-api-key",
                    Kind::OpenAi => "Authorization: Bearer",
                }))
                .on_hover_text("For gateways that expect the key in another header, such as Azure's api-key");
            });
            ui.add(crate::field::Checkbox::new(&mut provider.vision, "The model can see images (it checks its work with screenshots)"));
            ui.horizontal(|ui| {
                ui.label("Longest reply");
                ui.add(egui::DragValue::new(&mut provider.max_tokens).range(256..=64_000).suffix(" tokens"));
            });
        });

        if provider.id.starts_with("custom") || provider.id.starts_with("company") {
            ui.label(
                egui::RichText::new(
                    "For a service not listed, such as a company's own AI, ask whoever runs it for: an OpenAI-compatible (or \
                     Anthropic-compatible) API address, a model name, and an API key — and, if they use one, the name of the \
                     header the key goes in.",
                )
                .small()
                .color(crate::theme::dim()),
            );
        }

        ui.add_space(6.0);
        let busy = draft.check.is_some();
        ui.horizontal(|ui| {
            if ui.add_enabled(!busy, egui::Button::new("Fetch models")).on_hover_text("Ask the service which models it has").clicked() {
                let candidate = draft.candidate();
                let (sender, receiver) = channel();
                std::thread::spawn(move || {
                    let _ = sender.send(candidate.and_then(|p| session::fetch_models(&p)));
                });
                draft.check = Some(Check::Models(receiver));
                draft.status = None;
            }
            if ui.add_enabled(!busy, egui::Button::new("Test")).on_hover_text("Send a one-line message to check it answers").clicked() {
                let candidate = draft.candidate();
                let (sender, receiver) = channel();
                std::thread::spawn(move || {
                    let _ = sender.send(candidate.and_then(|p| session::test(&p)));
                });
                draft.check = Some(Check::Test(receiver));
                draft.status = None;
            }
            if busy {
                ui.spinner();
            }
        });
        match &draft.status {
            Some(Ok(text)) => {
                ui.colored_label(crate::theme::p().success, text);
            }
            Some(Err(text)) => {
                ui.add(egui::Label::new(egui::RichText::new(text).color(crate::theme::p().danger)).wrap());
            }
            None => {}
        }

        ui.separator();
        let (mut save, mut cancel) = (false, false);
        ui.horizontal(|ui| {
            save = ui.button(egui::RichText::new("Save").strong()).clicked();
            cancel = ui.button("Cancel").clicked();
        });
        let mut done = cancel;
        if save {
            match draft.candidate() {
                Ok(provider) => match provider.problem() {
                    Some(problem) => draft.status = Some(Err(format!("Not saved: {problem}"))),
                    None => {
                        let id = provider.id.clone();
                        match draft.original.clone().and_then(|o| self.settings.providers.iter().position(|p| p.id == o)) {
                            Some(index) => self.settings.providers[index] = provider,
                            None => self.settings.providers.push(provider),
                        }
                        if self.settings.active().is_none() {
                            self.settings.active = Some(id);
                        }
                        done = true;
                    }
                },
                Err(error) => draft.status = Some(Err(error)),
            }
        }
        if done {
            self.draft = None;
            if save {
                self.save_settings();
            }
        }
    }
}

impl Draft {
    fn of(original: Option<String>, provider: Provider) -> Draft {
        let (key_mode, env_name) = match &provider.key {
            // A server on this computer, by any of its names, needs no key.
            Key::None if tequila_ai::provider::is_local(&provider.base_url) => (KeyMode::None, String::new()),
            Key::None => (KeyMode::Stored, String::new()),
            Key::Stored(_) => (KeyMode::Stored, String::new()),
            Key::Env(name) => (KeyMode::Env, name.clone()),
        };
        Draft { original, provider, key_mode, key_input: String::new(), env_name, models: Vec::new(), check: None, status: None }
    }

    /// The provider as the form stands, key included.
    fn candidate(&self) -> Result<Provider, String> {
        let mut provider = self.provider.clone();
        provider.name = provider.name.trim().to_string();
        provider.base_url = provider.base_url.trim().to_string();
        provider.model = provider.model.trim().to_string();
        provider.key = match self.key_mode {
            KeyMode::None => Key::None,
            KeyMode::Env => {
                let name = self.env_name.trim();
                if name.is_empty() {
                    return Err("name the environment variable that holds the key".into());
                }
                Key::Env(name.to_string())
            }
            KeyMode::Stored => {
                let typed = self.key_input.trim();
                match (&self.provider.key, typed.is_empty()) {
                    (_, false) => Key::Stored(typed.to_string()),
                    (Key::Stored(old), true) => Key::Stored(old.clone()),
                    (_, true) => return Err("paste the API key".into()),
                }
            }
        };
        Ok(provider)
    }

    fn poll(&mut self) {
        let finished = match &self.check {
            Some(Check::Models(receiver)) => receiver.try_recv().ok().map(|result| match result {
                Ok(models) if models.is_empty() => Err("the service listed no models; type the model's id".to_string()),
                Ok(models) => {
                    let count = models.len();
                    self.models = models;
                    Ok(format!("{count} models found — choose one"))
                }
                Err(error) => Err(error),
            }),
            Some(Check::Test(receiver)) => receiver.try_recv().ok().map(|result| result.map(|reply| format!("Connected — it answered: {reply}"))),
            None => None,
        };
        if let Some(status) = finished {
            self.status = Some(status);
            self.check = None;
        }
    }
}

/// A file a tool names without saying where is in the project's folder:
/// left as written, it would be looked for wherever Tequila was started
/// from.
fn in_project(plan: tools::Plan, folder: Option<&std::path::Path>) -> tools::Plan {
    let (tools::Plan::Request { json, shape }, Some(folder)) = (&plan, folder) else { return plan };
    let Ok(tequila_format::json::Value::Object(mut fields)) = tequila_format::json::parse(json.as_bytes()) else { return plan };
    let op = fields.get("op").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let keys: &[&str] = match op.as_str() {
        "import" | "save" => &["path"],
        "queue" => &["folder"],
        _ => return plan,
    };
    for key in keys {
        if let Some(tequila_format::json::Value::String(path)) = fields.get_mut(*key) {
            if !std::path::Path::new(path.as_str()).is_absolute() {
                *path = folder.join(path.as_str()).to_string_lossy().into_owned();
            }
        }
    }
    tools::Plan::Request { json: tequila_format::json_write::to_string(&tequila_format::json::Value::Object(fields)), shape: shape.clone() }
}

/// A short line for the panel: what the assistant is doing, in words a
/// person who has never seen a tool's name can read.
fn step_label(call: &Call) -> String {
    let name = call.name.trim_start_matches("tequila_");
    let arguments = call.arguments.as_ref().ok();
    let word = |key: &str| arguments.and_then(|a| a.get(key)).and_then(|v| v.as_str()).map(str::to_string);
    // `material.set_type` as "Material: set type".
    let spoken = |command: &str| {
        let (about, what) = command.split_once('.').unwrap_or(("", command));
        let what = what.replace('_', " ");
        let mut about: Vec<char> = about.chars().collect();
        if let Some(first) = about.first_mut() {
            *first = first.to_ascii_uppercase();
        }
        let about: String = about.into_iter().collect();
        if about.is_empty() { what } else { format!("{about}: {what}") }
    };
    match name {
        "scene" => "Read the scene".into(),
        "nodes" => match word("filter") {
            Some(filter) => format!("Found parts named \"{filter}\""),
            None => "Read the parts".into(),
        },
        "materials" => "Read the materials".into(),
        "api" => "Read the command catalog".into(),
        "query" => format!("Read {}", word("what").unwrap_or_else(|| "the scene".into())),
        "apply" => arguments.and_then(|a| a.get("command")).and_then(|c| c.get("command")).and_then(|c| c.as_str()).map(spoken).unwrap_or_else(|| "Changed the scene".into()),
        "batch" => match arguments.and_then(|a| a.get("ops")).map(|ops| ops.as_array().len()) {
            Some(count) => format!("{count} changes as one step"),
            None => "Several changes as one step".into(),
        },
        "undo" => "Undid a step".into(),
        "redo" => "Redid a step".into(),
        "select" => "Selected parts".into(),
        "frame" => "Framed the model".into(),
        "look" => match word("view") {
            Some(view) => format!("Camera: {view} view"),
            None => "Moved the camera".into(),
        },
        "screenshot" => "Looked at the viewport".into(),
        "render" => "Rendered a quick look".into(),
        "queue" => "Render to a file".into(),
        "jobs" => "Checked the renders".into(),
        "stop" => "Stopped the render".into(),
        "ui" => match word("viewport") {
            Some(mode) => format!("Viewport: {mode}"),
            None => "Changed what is on screen".into(),
        },
        "material_guide" => "Read the material guide".into(),
        "material_preview" => "Tried a material on a ball".into(),
        "material_create" => "Made a material".into(),
        "material_update" => "Changed a material".into(),
        "material_tint" => match word("color") {
            Some(colour) => format!("Tinted a material {colour}"),
            None => "Tinted a material".into(),
        },
        "material_recipe" => "Read a material".into(),
        "material_save" => "Save a material to the library".into(),
        "studio_presets" => "Read the studios".into(),
        "studio_preset_apply" => "Applied a studio".into(),
        "studio_preset_save" => "Save a studio".into(),
        "studio_render_all" => "Render every studio".into(),
        "preferences" => "Preferences".into(),
        "library" => match word("search").or_else(|| word("category")) {
            Some(words) => format!("Looked in the library for \"{words}\""),
            None => "Looked in the library".into(),
        },
        "command" => match word("command") {
            Some(command) => format!("Looked up {command}"),
            None => "Read the commands".into(),
        },
        "add_shape" => match word("shape") {
            Some(shape) => format!("Added a {shape}"),
            None => "Added a shape".into(),
        },
        "material" => "Read a material".into(),
        "library_apply" => match (word("name"), word("color")) {
            (Some(name), Some(colour)) => format!("Library: {name}, in {colour}"),
            (Some(name), None) => format!("Library: {name}"),
            (None, _) => "Put a library material on parts".into(),
        },
        "import" => "Import a file".into(),
        "save" => "Save the project".into(),
        "plugin_guide" => "Read the plugin guide".into(),
        "plugin_list" => "Read the plugins".into(),
        "plugin_write" => "Write a plugin".into(),
        "plugin_enable" => "Turn a plugin on".into(),
        "plugin_run" => "Run a plugin".into(),
        "plugin_log" => "Read a plugin's log".into(),
        "request" => match arguments.and_then(|a| a.get("request")).and_then(|r| r.get("op")).and_then(|op| op.as_str()) {
            Some(op) => format!("Request: {}", op.replace('_', " ")),
            None => "Request".into(),
        },
        other => other.replace('_', " "),
    }
}

/// A PNG shrunk for sending to a model, as a PNG again.
pub(crate) fn prepare_image(png_bytes: &[u8]) -> Option<Vec<u8>> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).ok()?;
    if info.color_type != png::ColorType::Rgba {
        return None;
    }
    let (width, height, rgba) = crate::render_window::shrink(info.width as usize, info.height as usize, &buffer[..info.buffer_size()], SENT_SIZE);
    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&rgba).ok()?;
    }
    Some(encoded)
}

/// What an agent says of a tool call, as the chat shows it: Tequila's own
/// tools under the names a person knows them by, the agent's as it titles
/// them.
fn step_news(tool: &acp::ToolNews) -> StepNews {
    // Tequila's tool, however the agent writes it: `mcp__tequila__tequila_look`,
    // `tequila.tequila_look`, or the bare name.
    let ours = [tool.name.as_deref(), tool.title.as_deref()].into_iter().flatten().find_map(|name| {
        let from = name.rfind("tequila_")?;
        let short: String = name[from..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        (short.len() > "tequila_".len()).then(|| Call { id: String::new(), name: short, arguments: tool.input.clone().ok_or_else(String::new) })
    });
    let title = match (&ours, &tool.title) {
        // Only where there is something to title it from: news of a call
        // already listed leaves its title alone.
        (Some(call), _) if tool.title.is_some() || tool.input.is_some() => Some(step_label(call)),
        (_, title) => title.clone(),
    };
    // What it said is worth keeping when it failed; when it worked, the
    // chat has the assistant's own words for it.
    let detail = match (tool.status, &tool.text) {
        (Some(Status::Failed), Some(text)) if !text.trim().is_empty() => Some(text.trim().to_string()),
        _ => None,
    };
    StepNews { id: tool.id.clone(), title, status: tool.status, detail, pictures: tool.pictures.iter().cloned().map(Picture::new).collect() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_format::json;

    fn call(name: &str, arguments: &str) -> Call {
        Call { id: "c".into(), name: name.into(), arguments: Ok(json::parse(arguments.as_bytes()).unwrap()) }
    }

    /// The assistant's tool calls run through the real handler on a real
    /// application: they change the scene, they are undoable, and a failing
    /// one comes back as an error the model can read.
    #[test]
    fn tool_calls_change_the_scene_through_the_handler() {
        let mut app = TequilaApp::for_tests();
        let mut assistant = Assistant::default();
        let node = app.project.document().tree.iter().next().unwrap().id.0;
        let start = app.project.bus().depth();
        assistant.turn_start = Some(start);

        assistant.carry_out(&mut app, call("tequila_apply", &format!(r#"{{"command":{{"command":"scene.rename","node":{node},"name":"By the assistant"}}}}"#)));
        assistant.carry_out(&mut app, call("tequila_select", r#"{"nodes":[99999999]}"#));

        assert_eq!(app.project.document().tree.get(tequila_scene::tree::NodeId(node)).unwrap().name, "By the assistant");
        let errors: Vec<bool> = assistant.results.iter().map(|r| matches!(r, Part::Result { is_error: true, .. })).collect();
        assert_eq!(errors, [false, true]);
        assert!(app.console.text().contains("assistant"), "the console shows who acted");

        // Undo request takes back exactly what it did.
        assert!(assistant.can_undo_turn(app.project.bus()));
        assistant.undo_turn(&mut app);
        assert_eq!(app.project.bus().depth(), start);
        assert_ne!(app.project.document().tree.get(tequila_scene::tree::NodeId(node)).unwrap().name, "By the assistant");
    }

    /// A material the assistant makes goes on the part, and comes back to it
    /// with a picture of it on the ball, so it can see what it made.
    #[test]
    fn a_material_made_by_the_assistant_comes_with_its_picture() {
        let mut app = TequilaApp::for_tests();
        let mut assistant = Assistant::default();
        let part = app.project.document().tree.iter().find(|n| n.mesh().is_some()).unwrap().id;
        assistant.turn_start = Some(app.project.bus().depth());
        let recipe = r##"{"name": "Test Speckle", "type": "plastic", "patterns": [{"param": "base_color", "procedural": {"kind": "spots"}, "size": 0.004, "colors": [{"at": 0, "color": "#eeeeee"}, {"at": 1, "color": "#111111"}]}]}"##;
        assert!(assistant.carry_out(&mut app, call("tequila_material_create", &format!(r#"{{"recipe":{recipe},"nodes":[{}]}}"#, part.0))));
        let key = app.project.document().tree.get(part).unwrap().material.expect("a material on the part");
        assert_eq!(app.project.document().materials.get(key).unwrap().name, "Test Speckle");
        let shot = assistant.shot.as_ref().expect("its picture is on its way");
        assert!(shot.path.exists(), "the preview was not written");
        assert!(shot.caption.contains("Test Speckle"), "{}", shot.caption);
        let _ = std::fs::remove_file(&shot.path);
        // One step to take back.
        assert!(assistant.can_undo_turn(app.project.bus()));
    }

    /// "This leather, but dark green": a library preset put on a part, then
    /// tinted, each by its own tool through the application's channel. The
    /// leather's colour is inside its pattern, and that is where the green
    /// goes; the answer says so, and comes with the leather on the ball.
    #[test]
    fn the_assistant_tints_a_library_preset_in_its_pattern() {
        let mut app = TequilaApp::for_tests();
        let mut assistant = Assistant::default();
        let part = app.project.document().tree.iter().find(|n| n.mesh().is_some()).unwrap().id;
        assistant.turn_start = Some(app.project.bus().depth());
        assert!(assistant.carry_out(&mut app, call("tequila_library_apply", &format!(r#"{{"name":"Leather Tan","nodes":[{}]}}"#, part.0))));
        let key = app.project.document().tree.get(part).unwrap().material.expect("the preset is on the part");
        let applied = app.project.bus().depth();
        let tan = app.project.document().materials.get(key).unwrap().graph.clone();
        assert!(tan.is_connected(tequila_materials::material_type::Param::BaseColor));

        assert!(assistant.carry_out(&mut app, call("tequila_material_tint", &format!(r##"{{"material":{},"color":"#1f4d2b"}}"##, key.0))));
        let green = tequila_format::hex_colour("#1f4d2b").unwrap();
        let leather = app.project.document().materials.get(key).unwrap();
        assert_eq!(leather.name, "Leather Tan");
        assert_eq!(leather.material().base_color, green);
        let driver = leather.graph.driver_of(tequila_materials::material_type::Param::BaseColor).unwrap();
        assert!(leather.graph.node(driver).unwrap().literals.values().any(|v| *v == tequila_materials::graph::Value::Color(green)), "the green is not in the pattern");
        assert_eq!(leather.graph.topology_hash(), tan.topology_hash(), "the grain is as it was");
        assert_eq!(app.project.bus().depth(), applied + 1, "a tint is one step");
        let shot = assistant.shot.take().expect("its picture is on its way");
        assert!(shot.path.exists(), "the preview was not written");
        assert!(shot.caption.contains("\"in\": \"pattern\"") || shot.caption.contains("\"in\":\"pattern\""), "{}", shot.caption);
        let _ = std::fs::remove_file(&shot.path);

        // Already in a colour, as it goes on: a material of its own.
        assert!(assistant.carry_out(&mut app, call("tequila_library_apply", &format!(r##"{{"name":"Leather Tan","nodes":[{}],"color":"#7a1f2b"}}"##, part.0))));
        let other = app.project.document().tree.get(part).unwrap().material.unwrap();
        assert_ne!(other, key);
        assert_eq!(app.project.document().materials.get(other).unwrap().name, "Leather Tan #7a1f2b");

        // A material with no colour says so, and is an error the model reads.
        let film = r#"{"name": "Soap Film", "type": "thin_film"}"#;
        assert!(assistant.carry_out(&mut app, call("tequila_material_create", &format!(r#"{{"recipe":{film},"preview":false}}"#))));
        let film = app.project.document().materials.iter().find(|(_, d)| d.name == "Soap Film").map(|(key, _)| key).unwrap();
        assert!(assistant.carry_out(&mut app, call("tequila_material_tint", &format!(r##"{{"material":{},"color":"#1f4d2b"}}"##, film.0))));
        assert!(matches!(assistant.results.last(), Some(Part::Result { is_error: true, text, .. }) if text.contains("`Soap Film` has no colour to change")), "{:?}", assistant.results.last());
    }

    /// A plugin's command run by the assistant is answered once it has run,
    /// with what it printed and that it finished - the frames go on, the
    /// window is not held. Skipped where there is no Python.
    #[test]
    fn a_plugin_run_by_the_assistant_answers_with_its_log() {
        if crate::plugins::python().is_none() {
            eprintln!("skipped: no Python 3");
            return;
        }
        let root = std::env::temp_dir().join(format!("tequila-assistant-plugin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let folder = root.join("test.echo");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("plugin.json"), r#"{"id":"test.echo","name":"Echo","main":"main.py","commands":[{"id":"go"}]}"#).unwrap();
        std::fs::write(folder.join("main.py"), "import tequila_plugin as plugin\n\n@plugin.command('go')\ndef go(fs, values):\n    print('hello ' + str(values.get('n')))\n").unwrap();
        let mut app = TequilaApp::for_tests();
        app.plugins = crate::plugins::Plugins::load_from(vec![(root.clone(), crate::plugins::Origin::User)], root.join("enabled.json"));
        app.plugins.set_enabled("test.echo", true).unwrap();
        app.plugins.start_enabled(None);
        // Up first: a first start of Python can take longer than the run is
        // watched for.
        let ready = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while !app.plugins.logs.get("test.echo").is_some_and(|l| l.contains(&"running".to_string())) && std::time::Instant::now() < ready {
            app.plugins.poll();
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let mut assistant = Assistant::default();
        // Running a plugin's code is the person's to allow.
        assert!(!assistant.carry_out(&mut app, call("tequila_plugin_run", r#"{"id":"test.echo","command":"go","values":{"n":3}}"#)));
        let Some(Item::Asked(asked)) = assistant.chat.items.last().cloned() else { panic!("no question") };
        assert_eq!((asked.title.as_str(), asked.detail.as_str()), ("Run a plugin?", "test.echo: go"));
        assistant.answer(&mut app, asked.id, true);
        assert!(assistant.plugin_wait.is_some(), "the run was answered before it ran");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while assistant.plugin_wait.is_some() && std::time::Instant::now() < deadline {
            app.plugins.poll();
            assistant.collect_plugin_run(&mut app);
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let Some(Part::Result { text, is_error, .. }) = assistant.results.last() else { panic!("no answer") };
        let answer = tequila_format::json::parse(text.as_bytes()).expect("the answer is JSON");
        let finished = answer.get("finished").and_then(|v| v.as_bool());
        let said: Vec<&str> = answer.get("log").map(|l| l.as_array().iter().filter_map(|v| v.as_str()).collect()).unwrap_or_default();
        assert!(!is_error && finished == Some(true) && said.contains(&"hello 3"), "{text}");
        app.plugins.set_enabled("test.echo", false).unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Tequila's own loop is asked about the same things an agent is: a
    /// save to a new place waits for the person, is refused to the model in
    /// words when they say no, and is done - in the project's folder, when
    /// the model named no folder - when they say yes. A save to the
    /// project's own file, and a preset from the library, ask nobody.
    #[test]
    fn the_own_loops_tools_that_leave_the_scene_ask_first() {
        let root = std::env::temp_dir().join(format!("tequila-assistant-own-gate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let mut app = TequilaApp::for_tests();
        app.project.save(&root.join("project.tequila")).unwrap();
        let mut assistant = Assistant::default();

        assert!(!assistant.carry_out(&mut app, call("tequila_save", r#"{"path":"copy.tequila"}"#)), "it waits");
        assert!(assistant.is_working() && assistant.is_asking() && assistant.results.is_empty());
        let asked = |assistant: &Assistant| assistant.chat.items.iter().rev().find_map(|item| if let Item::Asked(asked) = item { Some(asked.clone()) } else { None }).unwrap();
        let question = asked(&assistant);
        assert_eq!((question.title.as_str(), question.detail.as_str()), ("Save the project to a new place?", "copy.tequila"));
        assistant.answer(&mut app, question.id, false);
        assert!(matches!(assistant.results.last(), Some(Part::Result { is_error: true, text, .. }) if text.contains("said no")));
        assert!(!root.join("copy.tequila").exists() && !assistant.is_asking());

        assert!(!assistant.carry_out(&mut app, call("tequila_save", r#"{"path":"copy.tequila"}"#)));
        let question = asked(&assistant);
        assistant.answer(&mut app, question.id, true);
        assert!(matches!(assistant.results.last(), Some(Part::Result { is_error: false, .. })), "{:?}", assistant.results.last());
        assert!(root.join("copy.tequila").exists(), "saved beside the project, not wherever Tequila was started");

        // Nothing asked: the project's own file, a look, a library query.
        let before = assistant.chat.items.len();
        assert!(assistant.carry_out(&mut app, call("tequila_save", "{}")));
        assert!(assistant.carry_out(&mut app, call("tequila_query", r#"{"what":"library"}"#)));
        assert!(!assistant.chat.items[before..].iter().any(|item| matches!(item, Item::Asked(_))));
        // Asked: an import, a render to a file, a preference.
        for (tool, arguments, title) in [
            ("tequila_import", r#"{"path":"model.glb"}"#, "Import a file?"),
            ("tequila_queue", r#"{"width":1920,"height":1080}"#, "Render to a file?"),
            ("tequila_preferences", r#"{"set":{"viewport.gizmo_size":150}}"#, "Change Tequila's preferences?"),
        ] {
            assert!(!assistant.carry_out(&mut app, call(tool, arguments)), "{tool}");
            let question = asked(&assistant);
            assert_eq!(question.title, title);
            assistant.answer(&mut app, question.id, false);
        }
        // Reading the preferences is free.
        assert!(assistant.carry_out(&mut app, call("tequila_preferences", "{}")));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Once the person has edited by hand, undoing "the request" would take
    /// their work too, so it is not offered.
    #[test]
    fn a_manual_edit_after_the_request_blocks_undo_request() {
        let mut app = TequilaApp::for_tests();
        let mut assistant = Assistant::default();
        let node = app.project.document().tree.iter().next().unwrap().id;
        assistant.turn_start = Some(app.project.bus().depth());
        assistant.carry_out(&mut app, call("tequila_apply", &format!(r#"{{"command":{{"command":"scene.rename","node":{},"name":"A"}}}}"#, node.0)));
        app.project.apply(tequila_scene::Command::Rename { node, name: "By hand".into() }).unwrap();
        assert!(!assistant.can_undo_turn(app.project.bus()));
    }

    /// Undone by hand and as many edits made again leaves the depth where
    /// the request left it; the edits are still the person's, not its.
    #[test]
    fn edits_made_after_undoing_the_request_by_hand_are_not_taken_back() {
        let mut app = TequilaApp::for_tests();
        let mut assistant = Assistant::default();
        let node = app.project.document().tree.iter().next().unwrap().id;
        assistant.turn_start = Some(app.project.bus().depth());
        assistant.carry_out(&mut app, call("tequila_apply", &format!(r#"{{"command":{{"command":"scene.rename","node":{},"name":"A"}}}}"#, node.0)));
        assert!(assistant.can_undo_turn(app.project.bus()));
        app.project.undo();
        app.project.apply(tequila_scene::Command::Rename { node, name: "By hand".into() }).unwrap();
        assert!(!assistant.can_undo_turn(app.project.bus()), "the button would undo the person's own edit");
    }

    /// The assistant's own screenshot says when the person has moved the
    /// view since its last one, and not at its first look.
    #[test]
    fn a_look_says_when_the_person_moved_the_view() {
        let mut app = TequilaApp::for_tests();
        let mut assistant = Assistant::default();
        let mut look = |app: &mut TequilaApp| {
            assistant.carry_out(app, call("tequila_screenshot", "{}"));
            let shot = assistant.shot.take().expect("a screenshot asked for");
            app.pending_screenshot = None;
            shot.caption
        };
        assert!(!look(&mut app).contains("moved the view"));
        assert!(!look(&mut app).contains("moved the view"));
        app.camera.zoom(2.0);
        assert!(look(&mut app).contains("The person has moved the view since your last look"));
        assert!(!look(&mut app).contains("moved the view"), "said once");
    }

    /// A script's screenshot already in flight makes the assistant's wait a
    /// frame — and the frame must end, not spin on the same call forever.
    #[test]
    fn a_screenshot_waits_for_one_already_in_flight() {
        let mut app = TequilaApp::for_tests();
        let mut assistant = Assistant::default();
        app.pending_screenshot = Some(std::env::temp_dir().join("someone-else.png"));
        assistant.calls.push_back(call("tequila_screenshot", "{}"));
        assistant.pump(&mut app, &egui::Context::default());
        assert_eq!(assistant.calls.len(), 1, "still waiting its turn");
        assert!(assistant.shot.is_none());

        app.pending_screenshot = None;
        assistant.pump(&mut app, &egui::Context::default());
        assert!(assistant.shot.is_some(), "taken once the other capture is done");
        assert!(app.pending_screenshot.is_some());
    }

    #[test]
    fn a_picture_is_shrunk_for_the_model() {
        let (w, h) = (2048u32, 1024u32);
        let mut png_bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png_bytes, w, h);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().unwrap().write_image_data(&vec![128; (w * h * 4) as usize]).unwrap();
        }
        let shrunk = prepare_image(&png_bytes).expect("prepared");
        assert!(shrunk.starts_with(b"\x89PNG"));
        let size = png::Decoder::new(std::io::Cursor::new(&shrunk)).read_info().unwrap().info().size();
        assert!(size.0 as usize <= SENT_SIZE && size.1 as usize <= SENT_SIZE, "{size:?}");
        // Attached to a message, it is what the message box holds.
        let mut assistant = Assistant::default();
        assistant.attach(&png_bytes);
        assert_eq!(assistant.panel.attached.len(), 1);
        assistant.attach(b"not a picture");
        assert_eq!(assistant.panel.attached.len(), 1, "what cannot be read is not attached");
    }

    /// The panel while a reply streams in, drawn for real: its reasoning
    /// folded under a header, and the line saying what it is doing; and
    /// the same for a script.
    #[test]
    fn a_reply_on_its_way_is_shown_as_it_comes() {
        // A server that takes the request and never answers: the request
        // stays on its way for as long as the test needs.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}/v1", listener.local_addr().unwrap().port());
        std::thread::spawn(move || {
            let held: Vec<_> = listener.incoming().take(4).collect();
            std::thread::sleep(std::time::Duration::from_secs(30));
            drop(held);
        });
        let mut assistant = Assistant::default();
        assistant.open = true;
        let provider = Provider { model: "test".into(), base_url: url, ..Provider::preset("ollama").unwrap() };
        assistant.settings = Settings { providers: vec![provider.clone()], active: Some(provider.id.clone()), ..Settings::default() };
        assistant.ask("Make the strap velvet".into(), "", Vec::new(), &Around::default());
        assistant.chat.push(Item::Thought { text: "Earlier.".into(), seconds: 3.0 });
        assistant.live = Some(Progress {
            phase: Phase::Thinking,
            since: std::time::Instant::now(),
            reasoning: "The strap is part 4.".into(),
            text: String::new(),
            tokens: 1312,
            thinking_from: Some(std::time::Instant::now()),
            thought_for: std::time::Duration::from_secs(12),
        });
        assert_eq!(assistant.status_line(), "Thinking · 1,312 tokens");
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                assistant.show(ui, false, &Around::default(), &Vec::new);
            });
        });
        let status = assistant.status();
        assert_eq!(status.get("phase").and_then(|v| v.as_str()), Some("thinking"));
        assert_eq!(status.get("tokens").and_then(|v| v.as_f64()), Some(1312.0));
        let kinds: Vec<&str> = status.get("transcript").unwrap().as_array().iter().filter_map(|e| e.get("kind").and_then(|k| k.as_str())).collect();
        assert_eq!(kinds, ["you", "thought"]);
        assistant.stop();
        assert!(!assistant.is_working(), "Stop frees the panel at once");
    }

    #[test]
    fn steps_are_labelled_for_people() {
        assert_eq!(step_label(&call("tequila_apply", r#"{"command":{"command":"material.set_type"}}"#)), "Material: set type");
        assert_eq!(step_label(&call("tequila_batch", r#"{"ops":[{},{},{}]}"#)), "3 changes as one step");
        assert_eq!(step_label(&call("tequila_look", r#"{"view":"front"}"#)), "Camera: front view");
        assert_eq!(step_label(&call("tequila_material_tint", r##"{"material":4,"color":"#1f4d2b"}"##)), "Tinted a material #1f4d2b");
        assert_eq!(step_label(&call("tequila_library_apply", r##"{"name":"Leather Tan","color":"#1f4d2b"}"##)), "Library: Leather Tan, in #1f4d2b");
        assert_eq!(step_label(&call("tequila_nodes", r#"{"filter":"grille"}"#)), "Found parts named \"grille\"");
        assert_eq!(step_label(&call("tequila_screenshot", "{}")), "Looked at the viewport");
        assert_eq!(step_label(&call("tequila_library", r#"{"search":"black leather"}"#)), "Looked in the library for \"black leather\"");
        assert_eq!(step_label(&call("tequila_library_apply", r#"{"name":"Leather Black","nodes":[4]}"#)), "Library: Leather Black");
        assert_eq!(step_label(&call("tequila_command", r#"{"command":"scene.set_shape"}"#)), "Looked up scene.set_shape");
        assert_eq!(step_label(&call("tequila_add_shape", r#"{"shape":"cylinder"}"#)), "Added a cylinder");
        assert_eq!(step_label(&call("tequila_request", r#"{"request":{"op":"studio_kit","piece":"softbox"}}"#)), "Request: studio kit");
        // One added later still reads as words.
        assert_eq!(step_label(&call("tequila_something_new", "{}")), "something new");
    }

    /// A stand-in agent on a local socket: `script` is handed each message
    /// Tequila sends and a way to write back.
    fn stand_in(mut script: impl FnMut(&json::Value, &mut dyn FnMut(&str)) + Send + 'static) -> Agent {
        use std::io::{BufRead, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut writer = stream.try_clone().unwrap();
            let mut write = move |line: &str| {
                let _ = writeln!(writer, "{line}");
            };
            for line in std::io::BufReader::new(stream).lines().map_while(Result::ok) {
                script(&json::parse(line.as_bytes()).unwrap(), &mut write);
            }
        });
        let stream = std::net::TcpStream::connect(address).unwrap();
        Agent::over(acp::Launch { folder: std::env::temp_dir(), ..acp::Launch::default() }, stream.try_clone().unwrap(), stream, || {})
    }

    fn id_of(message: &json::Value) -> String {
        tequila_format::json_write::to_string(message.get("id").unwrap_or(&json::Value::Null)).trim().to_string()
    }

    fn kinds(assistant: &Assistant) -> Vec<String> {
        let status = assistant.status();
        status.get("transcript").unwrap().as_array().iter().filter_map(|e| e.get("kind").and_then(|k| k.as_str()).map(str::to_string)).collect()
    }

    /// Pump until `done`, or ten seconds.
    fn pump_until(assistant: &mut Assistant, app: &mut TequilaApp, done: impl Fn(&Assistant) -> bool) {
        let ctx = egui::Context::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done(assistant) && std::time::Instant::now() < deadline {
            assistant.pump(app, &ctx);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(done(assistant), "{:?}", kinds(assistant));
    }

    /// A message to an agent that is still starting waits for it; its reply
    /// streams into the chat; its own tools go ahead, a file written in the
    /// project's folder asks, and a command is refused without asking.
    #[test]
    fn an_agent_answers_in_the_chat_and_is_held_to_its_reach() {
        let folder = std::env::temp_dir().join(format!("tequila-assistant-agent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let inside = folder.join("notes.txt").to_string_lossy().replace('\\', "/");
        let answers = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let heard = answers.clone();
        let agent = stand_in(move |message, write| {
            let update = |body: &str| format!(r#"{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"s","update":{body}}}}}"#);
            let ask = |id: &str, call: &str| {
                format!(r#"{{"jsonrpc":"2.0","id":"{id}","method":"session/request_permission","params":{{"sessionId":"s","toolCall":{call},"options":[{{"optionId":"yes","name":"Allow","kind":"allow_once"}},{{"optionId":"no","name":"Reject","kind":"reject_once"}}]}}}}"#)
            };
            match message.get("method").and_then(|m| m.as_str()) {
                Some("initialize") => write(&format!(r#"{{"jsonrpc":"2.0","id":{},"result":{{"protocolVersion":1,"agentCapabilities":{{"promptCapabilities":{{"image":true}}}}}}}}"#, id_of(message))),
                Some("session/new") => write(&format!(
                    r#"{{"jsonrpc":"2.0","id":{},"result":{{"sessionId":"s","configOptions":[{{"id":"model","name":"Model","category":"model","type":"select","currentValue":"a","options":[{{"value":"a","name":"Sonnet"}},{{"value":"b","name":"Opus"}}]}}]}}}}"#,
                    id_of(message)
                )),
                Some("session/prompt") => {
                    write(&update(r#"{"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"Which part?"}}"#));
                    write(&update(r#"{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Looking "}}"#));
                    write(&update(r#"{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"now."}}"#));
                    write(&ask("p1", r#"{"toolCallId":"t1","title":"mcp__tequila__tequila_nodes","name":"mcp__tequila__tequila_nodes","kind":"other","rawInput":{"filter":"grille"}}"#));
                    write(&ask("p2", r#"{"toolCallId":"t2","title":"ls","name":"Bash","kind":"execute","rawInput":{"command":"ls"}}"#));
                    write(&ask("p3", &format!(r#"{{"toolCallId":"t3","title":"Write notes.txt","name":"Write","kind":"edit","locations":[{{"path":"{inside}"}}]}}"#)));
                }
                None => {
                    let outcome = message.get("result").and_then(|r| r.get("outcome")).map(|o| o.get("optionId").and_then(|v| v.as_str()).unwrap_or("cancelled").to_string());
                    if let Some(outcome) = outcome {
                        let mut heard = heard.lock().unwrap();
                        heard.push(format!("{}={outcome}", id_of(message).trim_matches('"')));
                        if heard.len() == 3 {
                            write(&update(r#"{"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"completed"}"#));
                            write(&update(r#"{"sessionUpdate":"tool_call_update","toolCallId":"t3","status":"completed"}"#));
                            write(r#"{"jsonrpc":"2.0","id":2,"result":{"stopReason":"end_turn"}}"#);
                        }
                    }
                }
                _ => {}
            }
        });
        let mut app = TequilaApp::for_tests();
        let mut assistant = Assistant::default();
        assistant.settings.active = Some(agents::Kind::Claude.id().into());
        assistant.agent = Some((agents::Kind::Claude, agent));
        assistant.agent_folder = folder.clone();
        assert_eq!(assistant.backend(), Some(Backend::Agent(agents::Kind::Claude)));

        assistant.ask("Find the grille".into(), "(In this scene: @Grille is the part with id 12.)", Vec::new(), &app.around());
        assert!(assistant.is_working(), "asked, and waiting for the agent to be ready");
        pump_until(&mut assistant, &mut app, |a| a.is_asking());
        // Its own tool went ahead and the command was refused, both without
        // a question; the file is the one thing asked about.
        assert_eq!(kinds(&assistant), ["you", "thought", "said", "running step", "failed step", "running step", "asked"]);
        // (The agent hears them a moment after they are sent.)
        let heard_by = |count: usize| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while answers.lock().unwrap().len() < count && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            answers.lock().unwrap().clone()
        };
        assert_eq!(heard_by(2), ["p1=yes", "p2=no"]);
        assert!(matches!(&assistant.chat.items[3], Item::Step(step) if step.title == "Found parts named \"grille\""), "{:?}", assistant.chat.items[3]);
        assert!(matches!(&assistant.chat.items[4], Item::Step(step) if step.detail.contains("does not run commands")));
        let Item::Asked(asked) = assistant.chat.items[6].clone() else { panic!() };
        assert_eq!(asked.title, "Change a file in the project's folder?");
        assert!(asked.detail.ends_with("notes.txt"), "{}", asked.detail);
        assert_eq!(assistant.status().get("phase").and_then(|p| p.as_str()), Some("asking"));

        assistant.answer(&mut app, asked.id, true);
        pump_until(&mut assistant, &mut app, |a| !a.is_working());
        assert_eq!(heard_by(3), ["p1=yes", "p2=no", "p3=yes"]);
        assert_eq!(kinds(&assistant), ["you", "thought", "said", "step", "failed step", "step", "asked"]);
        assert_eq!(assistant.chat.items[2], Item::Said("Looking now.".into()));
        assert_eq!((assistant.chat.session.as_deref(), assistant.chat.backend.as_str()), (Some("s"), "agent:claude"));
        // The agent's models are what the picker offers.
        assert_eq!(assistant.answering(), "Claude Sonnet");
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// An agent that opens a conversation but says nobody is signed in
    /// shows its ways of signing in as it opens, and a message sent then
    /// waits for that instead of failing; one that says so and then signs
    /// in with a key a moment later shows no card at all.
    #[test]
    fn an_agent_nobody_is_signed_in_to_says_so_as_it_opens() {
        let opens = |account: &'static [&'static str]| {
            stand_in(move |message, write| match message.get("method").and_then(|m| m.as_str()) {
                Some("initialize") => write(&format!(
                    r#"{{"jsonrpc":"2.0","id":{},"result":{{"protocolVersion":1,"authMethods":[{{"id":"console","name":"Anthropic Console","type":"terminal","args":["--cli"]}}]}}}}"#,
                    id_of(message)
                )),
                Some("session/new") => {
                    for kind in account {
                        write(&format!(r#"{{"jsonrpc":"2.0","method":"_auth/status_update","params":{{"authStatus":{{"kind":"{kind}","label":""}}}}}}"#));
                    }
                    write(&format!(r#"{{"jsonrpc":"2.0","id":{},"result":{{"sessionId":"s"}}}}"#, id_of(message)));
                }
                _ => {}
            })
        };
        let mut app = TequilaApp::for_tests();
        let mut assistant = Assistant::default();
        assistant.settings.active = Some(agents::Kind::Claude.id().into());
        assistant.agent = Some((agents::Kind::Claude, opens(&["none"])));
        pump_until(&mut assistant, &mut app, |a| a.setup.is_some());
        assert!(matches!(&assistant.setup, Some(Setup::SignIn { ways, .. }) if ways.len() == 1));
        assistant.ask("Hello".into(), "", Vec::new(), &app.around());
        assert!(assistant.queued.is_some() && !assistant.is_working(), "kept for after signing in, not sent to fail");

        let mut assistant = Assistant::default();
        assistant.settings.active = Some(agents::Kind::ChatGpt.id().into());
        assistant.agent = Some((agents::Kind::ChatGpt, opens(&["none", "api_key"])));
        pump_until(&mut assistant, &mut app, |a| a.chat.session.is_some());
        assert!(assistant.setup.is_none(), "signed in by its key before the conversation opened: {:?}", assistant.status().get("setup"));
        assert_eq!(assistant.account.as_ref().map(|(kind, _)| kind.as_str()), Some("api_key"));
    }

    /// An agent that wants asking for a key sign-in (Gemini's) is asked,
    /// once, when the person has connected a key; the conversation opens
    /// without a card. Without a key the card shows its ways.
    #[test]
    fn a_connected_key_signs_an_agent_in_that_wants_asking() {
        let wanting = || {
            let signed_in = std::sync::Arc::new(std::sync::Mutex::new(false));
            stand_in(move |message, write| match message.get("method").and_then(|m| m.as_str()) {
                Some("initialize") => write(&format!(
                    r#"{{"jsonrpc":"2.0","id":{},"result":{{"protocolVersion":1,"authMethods":[{{"id":"oauth-personal","name":"Log in with Google"}},{{"id":"gemini-api-key","name":"Gemini API key"}}]}}}}"#,
                    id_of(message)
                )),
                Some("session/new") if *signed_in.lock().unwrap() => write(&format!(r#"{{"jsonrpc":"2.0","id":{},"result":{{"sessionId":"s"}}}}"#, id_of(message))),
                Some("session/new") => write(&format!(r#"{{"jsonrpc":"2.0","id":{},"error":{{"code":-32000,"message":"Authentication required"}}}}"#, id_of(message))),
                Some("authenticate") => {
                    assert_eq!(message.get("params").unwrap().get("methodId").and_then(|m| m.as_str()), Some("gemini-api-key"));
                    *signed_in.lock().unwrap() = true;
                    write(&format!(r#"{{"jsonrpc":"2.0","id":{},"result":{{}}}}"#, id_of(message)));
                }
                _ => {}
            })
        };
        let mut app = TequilaApp::for_tests();
        let mut assistant = Assistant::default();
        assistant.settings.active = Some(agents::Kind::Gemini.id().into());
        assistant.agent = Some((agents::Kind::Gemini, wanting()));
        assistant.key_way = Some("gemini-api-key");
        pump_until(&mut assistant, &mut app, |a| a.chat.session.is_some());
        assert!(assistant.setup.is_none() && assistant.key_way.is_none());

        let mut assistant = Assistant::default();
        assistant.settings.active = Some(agents::Kind::Gemini.id().into());
        assistant.agent = Some((agents::Kind::Gemini, wanting()));
        pump_until(&mut assistant, &mut app, |a| a.setup.is_some());
        assert!(matches!(&assistant.setup, Some(Setup::SignIn { ways, .. }) if ways.len() == 2));
    }

    /// Using a Claude subscription is off until the person turns it on,
    /// and their choice is kept: Claude's program is let go, to start again
    /// as chosen, and the setting is in the file for next time.
    #[test]
    fn the_person_turns_their_claude_subscription_on_themselves() {
        let home = std::env::temp_dir().join(format!("tequila-assistant-subscription-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let mut assistant = Assistant::default();
        assistant.load(home.join("assistant.json"));
        assistant.settings.active = Some(agents::Kind::Claude.id().into());
        assert!(!assistant.settings.claude_subscription);
        assert_eq!(agents::start_with(agents::Kind::Claude, &assistant.settings).0, ["--hide-claude-auth"]);
        assistant.agent = Some((agents::Kind::Claude, stand_in(|_, _| {})));
        assistant.setup = Some(Setup::SignIn { ways: Vec::new(), subscription: true });

        assistant.set_claude_subscription(true);
        assert!(assistant.agent.is_none() && assistant.setup.is_none(), "let go, to start again with the subscription allowed");
        assert!(agents::start_with(agents::Kind::Claude, &assistant.settings).0.is_empty());
        let (kept, _) = Settings::load(&home.join("assistant.json"));
        assert!(kept.claude_subscription, "kept for next time");

        assistant.set_claude_subscription(false);
        assert!(!Settings::load(&home.join("assistant.json")).0.claude_subscription);
        let _ = std::fs::remove_dir_all(&home);
    }

    /// Claude told not to use a subscription names the account it found and
    /// refuses to open a conversation: the card says a subscription is
    /// signed in and stays, however often the account is named again.
    #[test]
    fn a_subscription_that_is_refused_keeps_its_card() {
        let agent = stand_in(|message, write| {
            let named = r#"{"jsonrpc":"2.0","method":"_auth/status_update","params":{"authStatus":{"kind":"account","label":"me@example.com"}}}"#;
            match message.get("method").and_then(|m| m.as_str()) {
                Some("initialize") => write(&format!(r#"{{"jsonrpc":"2.0","id":{},"result":{{"protocolVersion":1,"authMethods":[{{"id":"console-login","name":"Anthropic Console","type":"terminal","args":["--cli"]}}]}}}}"#, id_of(message))),
                Some("session/new") => {
                    write(named);
                    write(&format!(
                        r#"{{"jsonrpc":"2.0","id":{},"error":{{"code":-32000,"message":"Authentication required","data":{{"reason":"claude_subscription_not_supported"}}}}}}"#,
                        id_of(message)
                    ));
                    write(named);
                }
                _ => {}
            }
        });
        let mut app = TequilaApp::for_tests();
        let mut assistant = Assistant::default();
        assistant.settings.active = Some(agents::Kind::Claude.id().into());
        assistant.agent = Some((agents::Kind::Claude, agent));
        pump_until(&mut assistant, &mut app, |a| a.setup.is_some());
        let ctx = egui::Context::default();
        for _ in 0..20 {
            assistant.pump(&mut app, &ctx);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(matches!(assistant.setup, Some(Setup::SignIn { subscription: true, .. })), "the card stays, saying a subscription is signed in");
    }

    /// An agent whose program ends before it has opened a conversation is
    /// said on the card, with Try again - not started again every frame.
    #[test]
    fn an_agent_that_ends_as_it_starts_is_said_once() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || drop(listener.accept()));
        let stream = std::net::TcpStream::connect(address).unwrap();
        let agent = Agent::over(acp::Launch::default(), stream.try_clone().unwrap(), stream, || {});
        let mut app = TequilaApp::for_tests();
        let mut assistant = Assistant::default();
        assistant.settings.active = Some(agents::Kind::Claude.id().into());
        assistant.agent = Some((agents::Kind::Claude, agent));
        assistant.ask("Hello".into(), "", Vec::new(), &app.around());
        pump_until(&mut assistant, &mut app, |a| a.setup.is_some());
        assert!(matches!(&assistant.setup, Some(Setup::Failed(text)) if text.contains("stopped as it started")));
        assert!(assistant.agent.is_none() && !assistant.is_working());
        // The panel leaves it there until the person tries again.
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                assistant.show(ui, false, &Around::default(), &Vec::new);
            });
        });
        assert!(assistant.agent.is_none() && matches!(assistant.setup, Some(Setup::Failed(_))));
    }

    /// The Assistant's own bridge, over the real channel: a scene edit goes
    /// straight through, named for the Assistant and counted as the
    /// request's own; a save to a new place waits for the person, is
    /// refused in words when they say no, and is done when they say yes.
    #[test]
    fn what_leaves_the_scene_waits_for_the_person() {
        let root = std::env::temp_dir().join(format!("tequila-assistant-gate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let discovery = root.join("channel.json");
        let mut app = TequilaApp::for_tests();
        app.channel = Some(tequila_scripting::server::Channel::start_at(&discovery).unwrap());
        let node = app.project.document().tree.iter().next().unwrap().id.0;
        let mark = app.assistant.mark();
        app.assistant.turn_start = Some(app.project.bus().depth());
        let copy = root.join("copy.tequila");

        let (said, heard) = std::sync::mpsc::channel();
        let (sent_mark, sent_copy, record) = (mark.clone(), copy.to_string_lossy().into_owned(), app.channel.as_ref().unwrap().own_file().unwrap());
        std::thread::spawn(move || {
            let mut bridge = tequila_scripting::client::Connection::connect(&record).unwrap();
            bridge.set_patience(None);
            let mut command = tequila_format::json_write::Object::new();
            command.set("command", "scene.rename").set("node", node as f64).set("name", "By the agent");
            let _ = said.send(bridge.request("apply", vec![("command", command.build()), ("agent", sent_mark.as_str().into())]).map(|_| "renamed".to_string()));
            for _ in 0..2 {
                let _ = said.send(bridge.request("save", vec![("path", sent_copy.as_str().into()), ("agent", sent_mark.as_str().into())]).map(|_| "saved".to_string()));
            }
            let _ = said.send(bridge.request("query", vec![("what", "scene".into())]).map(|_| "looked".to_string()));
        });
        let ctx = egui::Context::default();
        let serve_until = |app: &mut TequilaApp, done: &dyn Fn(&TequilaApp) -> bool| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while !done(app) && std::time::Instant::now() < deadline {
                app.serve_channel(&ctx);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(done(app), "timed out");
        };

        serve_until(&mut app, &|app| app.assistant.is_asking());
        assert_eq!(heard.recv().unwrap(), Ok("renamed".into()));
        // While a request is being served the channel is out of the
        // application's hands; where it is stays known all the same, for a
        // script's request that starts the agent.
        let taken = app.channel.take();
        assert_eq!(app.around().channel, taken.as_ref().unwrap().own_file());
        app.channel = taken;
        assert_eq!(app.project.document().tree.get(tequila_scene::tree::NodeId(node)).unwrap().name, "By the agent");
        assert!(app.console.text().contains("assistant"), "the console names the Assistant");
        assert!(app.assistant.can_undo_turn(app.project.bus()), "its change is the request's to take back");
        let question = |app: &TequilaApp| app.assistant.chat.items.iter().rev().find_map(|item| if let Item::Asked(asked) = item { Some(asked.clone()) } else { None }).unwrap();
        let asked = question(&app);
        assert_eq!((asked.title.as_str(), asked.detail.as_str(), asked.answer), ("Save the project to a new place?", copy.to_string_lossy().as_ref(), None));
        // Inside the folder the agent works in, a file is named as it is
        // known there.
        app.assistant.agent_folder = root.clone();
        assert!(heard.try_recv().is_err(), "held, not answered");
        assert!(app.assistant.open, "the panel opens on a question");

        // No: refused in words the agent can pass on, and nothing written.
        let mut assistant = std::mem::take(&mut app.assistant);
        assistant.answer(&mut app, asked.id, false);
        app.assistant = assistant;
        let refused = heard.recv().unwrap().unwrap_err();
        assert!(refused.contains("said no"), "{refused}");
        assert!(!copy.exists());

        // Asked again, and yes this time: done.
        serve_until(&mut app, &|app| app.assistant.is_asking());
        let asked = question(&app);
        assert_eq!(asked.detail, "copy.tequila");
        let mut assistant = std::mem::take(&mut app.assistant);
        assistant.answer(&mut app, asked.id, true);
        app.assistant = assistant;
        assert_eq!(heard.recv().unwrap(), Ok("saved".into()));
        assert!(copy.exists(), "the project was saved where it asked");
        // A request without the mark is any script's: answered, nobody asked.
        let looked = std::cell::RefCell::new(None);
        serve_until(&mut app, &|_| {
            let mut looked = looked.borrow_mut();
            *looked = looked.take().or_else(|| heard.try_recv().ok());
            looked.is_some()
        });
        assert_eq!(looked.into_inner(), Some(Ok("looked".into())));
        assert_eq!(app.assistant.chat.items.iter().filter(|item| matches!(item, Item::Asked(_))).count(), 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Stop answers what is held, so the agent's tool call ends instead of
    /// waiting for an answer that will not come.
    #[test]
    fn stopping_refuses_what_was_waiting_for_an_answer() {
        let root = std::env::temp_dir().join(format!("tequila-assistant-stop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let mut app = TequilaApp::for_tests();
        app.channel = Some(tequila_scripting::server::Channel::start_at(&root.join("channel.json")).unwrap());
        let mark = app.assistant.mark();
        let record = app.channel.as_ref().unwrap().own_file().unwrap();
        let (said, heard) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut bridge = tequila_scripting::client::Connection::connect(&record).unwrap();
            let _ = said.send(bridge.request("import", vec![("path", "/models/speaker.step".into()), ("agent", mark.as_str().into())]));
        });
        let ctx = egui::Context::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !app.assistant.is_asking() && std::time::Instant::now() < deadline {
            app.serve_channel(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(app.assistant.is_asking());
        app.assistant.stop();
        assert!(heard.recv().unwrap().unwrap_err().contains("stopped"));
        assert!(!app.assistant.is_asking());
        assert!(matches!(app.assistant.chat.items.first(), Some(Item::Asked(asked)) if asked.answer == Some(false)));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A chat is kept as it goes, listed, opened again with what was said,
    /// and a new one started beside it.
    #[test]
    fn chats_are_kept_and_opened_again() {
        let home = std::env::temp_dir().join(format!("tequila-assistant-chats-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let mut assistant = Assistant::default();
        assistant.load(home.join("assistant.json"));
        assistant.chat.push(Item::You { text: "Make the strap velvet".into(), pictures: Vec::new() });
        assistant.chat.push(Item::Said("Done.".into()));
        assistant.chat.backend = agents::Kind::Claude.id().into();
        let first = assistant.chat.id.clone();
        assistant.new_chat();
        assert!(assistant.chat.is_empty() && assistant.chat.id != first);
        assistant.chat.push(Item::You { text: "Light it like a studio".into(), pictures: Vec::new() });
        assistant.new_chat();

        let store = assistant.store.clone().unwrap();
        let mut titles: Vec<String> = store.list().into_iter().map(|c| c.title).collect();
        titles.sort();
        assert_eq!(titles, ["Light it like a studio", "Make the strap velvet"]);

        assistant.open_chat(&first);
        assert_eq!(assistant.chat.items.len(), 2);
        // It goes on with whoever answered it.
        assert_eq!(assistant.backend(), Some(Backend::Agent(agents::Kind::Claude)));
        // And whoever picks it up later is told how it went.
        assistant.chat.push(Item::You { text: "Now the lenses".into(), pictures: Vec::new() });
        let retold = assistant.retold().unwrap();
        assert!(retold.contains("Person: Make the strap velvet") && retold.contains("Assistant: Done.") && !retold.contains("Now the lenses"), "{retold}");

        assistant.delete_chat(&first);
        assert_eq!(store.list().len(), 1);
        assert!(assistant.chat.is_empty(), "the open chat, deleted, leaves a new one");
        let _ = std::fs::remove_dir_all(&home);
    }

    /// The whole panel drawn with everything a chat can hold - and the list
    /// of past chats - without a frame going wrong.
    #[test]
    fn the_panel_draws_every_kind_of_thing() {
        let mut assistant = Assistant::default();
        assistant.open = true;
        assistant.chat.push(Item::You { text: "Make @Grille brushed aluminium".into(), pictures: Vec::new() });
        assistant.chat.push(Item::Thought { text: "The grille is part 12.".into(), seconds: 6.0 });
        assistant.chat.push(Item::Said("The **grille** is `part 12`.\n\n- one\n- two\n\n```\ncode\n```\n# Done".into()));
        assistant.chat.step(StepNews { id: "a".into(), title: Some("nodes \"grille\"".into()), status: Some(Status::Done), ..StepNews::default() });
        assistant.chat.step(StepNews { id: "b".into(), title: Some("screenshot".into()), status: Some(Status::Running), ..StepNews::default() });
        assistant.chat.step(StepNews { id: "c".into(), title: Some("ls".into()), status: Some(Status::Failed), detail: Some("no commands".into()), ..StepNews::default() });
        assistant.chat.push(Item::Asked(Asked { id: 1, title: "Render to a file?".into(), detail: "Renders/front.png, 3840 \u{d7} 2160".into(), answer: None }));
        assistant.chat.push(Item::Asked(Asked { id: 2, title: "Import a file?".into(), detail: "a.step".into(), answer: Some(false) }));
        assistant.chat.push(Item::Problem("It stopped.".into()));
        assistant.chat.push(Item::Note("Ask again.".into()));
        assistant.setup = Some(Setup::SignIn { ways: vec![acp::SignIn { id: "console".into(), name: "Anthropic Console".into(), description: String::new(), terminal: Some((Vec::new(), Vec::new())) }], subscription: true });
        assistant.settings.active = Some(agents::Kind::Claude.id().into());
        let ctx = egui::Context::default();
        let things = || vec![crate::assistant_panel::Thing { label: "Grille".into(), kind: "part".into(), context: "@Grille is the part with id 12".into() }];
        for history in [false, true] {
            assistant.input = "Now @gr".into();
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    assistant.show(ui, true, &Around::default(), &things);
                });
            });
            assistant.panel.toggle_history();
            let _ = history;
        }
        // Nothing is beside a test's application for an agent to reach the
        // scene through, and the panel says so rather than starting one.
        assert!(assistant.agent.is_none());
    }
}
