//! The Connections page: every way the Assistant can be answered, in one
//! window, each with where it stands.
//!
//! Three kinds, top to bottom. **Subscriptions and agents** - Claude,
//! ChatGPT - each a program of its maker's own: whether it is on this
//! computer, whether anyone is signed in to it and what pays, with Install
//! and Sign in where they are needed and its models to choose from. **API
//! keys and this computer** - the providers Tequila's own loop talks to,
//! their keys in the system's keychain. And, folded away, the **AI apps**
//! that can use Tequila from outside.
//!
//! To say whether an agent is signed in, the agent has to be asked: it is
//! started for a moment with nothing to work on, says who it is signed in
//! as and which models it has, and is let go again. A sign-in the agent
//! carries out itself keeps it running until that is done.

use crate::assistant::{Assistant, Backend};
use crate::assistant_agents::{self as agents, Found, Missing};
use crate::theme;
use std::sync::mpsc::Receiver;
use tequila_ai::acp::{self, Agent};
use tequila_ai::Settings;

/// Where an agent stands.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Not on this computer; whether Node is, to install it with.
    Missing { node: bool },
    Installing,
    /// Started, and being asked who is signed in.
    Checking,
    SignedOut,
    /// The person is signing in, in the agent's own page or window.
    SigningIn,
    /// Signed in: what pays (`account` for a subscription, `api_key`), and
    /// the name it gave.
    SignedIn { kind: String, label: String },
    Failed(String),
}

/// One agent on the page.
pub struct Connection {
    pub kind: agents::Kind,
    pub status: Status,
    /// The agent, while it is being asked or signed in to.
    agent: Option<Agent>,
    found: Option<Found>,
    installing: Option<Receiver<Result<String, String>>>,
    /// Its ways of signing in, as it last gave them.
    ways: Vec<acp::SignIn>,
    /// Its models, once it has said.
    pub models: Option<acp::Setting>,
    /// The sign-in a connected API key goes through, tried once.
    key_way: Option<&'static str>,
}

impl Connection {
    fn new(kind: agents::Kind) -> Connection {
        Connection { kind, status: Status::Checking, agent: None, found: None, installing: None, ways: Vec::new(), models: None, key_way: None }
    }

    /// Look for the agent's program and, where it is there, start it to ask
    /// who is signed in.
    fn look(&mut self, settings: &Settings, wake: &egui::Context) {
        self.agent = None;
        self.models = None;
        self.key_way = self.kind.key_sign_in(settings);
        match agents::find_agent(self.kind) {
            Err(Missing::Adapter { node }) => self.status = Status::Missing { node },
            Ok(found) => {
                let (more_args, more_env) = agents::start_with(self.kind, settings);
                let folder = tequila_format::library::home_directory().join("assistant-workspace");
                let _ = std::fs::create_dir_all(&folder);
                let launch = acp::Launch {
                    program: found.program.clone(),
                    args: found.args.iter().cloned().chain(more_args).collect(),
                    env: found.env.iter().cloned().chain(more_env).collect(),
                    folder,
                    version: env!("CARGO_PKG_VERSION").into(),
                    ..acp::Launch::default()
                };
                let wake = wake.clone();
                match Agent::start(launch, move || wake.request_repaint()) {
                    Ok(agent) => self.watch(agent),
                    Err(error) => self.status = Status::Failed(error),
                }
                self.found = Some(found);
            }
        }
    }

    /// Take an agent that has been started, to hear where it stands.
    fn watch(&mut self, agent: Agent) {
        self.agent = Some(agent);
        self.status = Status::Checking;
    }

    /// Hear what the agent has said since last time.
    fn poll(&mut self, settings: &Settings, wake: &egui::Context) {
        if let Some(receiver) = &self.installing {
            match receiver.try_recv() {
                Ok(Ok(_)) => {
                    self.installing = None;
                    self.look(settings, wake);
                }
                Ok(Err(error)) => {
                    self.installing = None;
                    self.status = Status::Failed(error);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.installing = None;
                    self.status = Status::Failed("The install stopped before it finished.".into());
                }
            }
        }
        let Some(agent) = &mut self.agent else { return };
        let mut done = false;
        for event in agent.poll() {
            match event {
                acp::Event::Account { kind, .. } if kind == "none" => {
                    self.ways = agent.sign_ins().to_vec();
                    self.status = Status::SignedOut;
                }
                acp::Event::Account { kind, label } => self.status = Status::SignedIn { kind, label },
                acp::Event::SignIn(ways) => {
                    // A connected key signs it in without a question, once.
                    match self.key_way.take().filter(|way| ways.iter().any(|w| w.id == *way)) {
                        Some(way) => agent.authenticate(way),
                        None => self.status = Status::SignedOut,
                    }
                    self.ways = ways;
                }
                acp::Event::Ready { .. } => {
                    self.models = agent.setting("model").cloned();
                    // Open, and it never said nobody was signed in.
                    if matches!(self.status, Status::Checking | Status::SigningIn) {
                        self.status = Status::SignedIn { kind: String::new(), label: String::new() };
                    }
                    // Asked and answered: it has nothing more to do here.
                    done = !matches!(self.status, Status::SignedOut);
                }
                acp::Event::Notice(text) if self.status == Status::SigningIn => self.status = Status::Failed(text),
                acp::Event::Failed(error) => {
                    self.status = Status::Failed(error);
                    done = true;
                }
                acp::Event::Exited(said) => {
                    if matches!(self.status, Status::Checking | Status::SigningIn) {
                        let said = said.trim();
                        self.status = Status::Failed(if said.is_empty() { "Its program stopped as it started.".into() } else { format!("Its program stopped as it started: {said}") });
                    }
                    done = true;
                }
                _ => {}
            }
        }
        if done {
            self.agent = None;
        }
    }

    /// Whether it is still finding something out, and the window should
    /// keep looking.
    fn busy(&self) -> bool {
        self.installing.is_some() || matches!(self.status, Status::Checking | Status::SigningIn | Status::Installing)
    }

    /// Where it stands, in a few words.
    pub fn says(&self) -> String {
        match &self.status {
            Status::Missing { node: true } => "Not on this computer".into(),
            Status::Missing { node: false } => "Needs Node.js, which is not on this computer".into(),
            Status::Installing => "Installing\u{2026}".into(),
            Status::Checking => "Checking\u{2026}".into(),
            Status::SignedOut => "Installed, not signed in".into(),
            Status::SigningIn => "Signing in: finish in the page that opened".into(),
            Status::SignedIn { kind, label } => {
                let paid = match kind.as_str() {
                    "account" => "a subscription",
                    "api_key" => "an API key, by use",
                    _ => "",
                };
                match (paid.is_empty(), label.is_empty()) {
                    (true, _) => "Signed in".into(),
                    (false, true) => format!("Signed in \u{b7} paid for by {paid}"),
                    (false, false) => format!("Signed in \u{b7} paid for by {paid} ({label})"),
                }
            }
            Status::Failed(error) => error.clone(),
        }
    }
}

/// The agents on the page, looked at while it is open.
#[derive(Default)]
pub struct Connections {
    each: Vec<Connection>,
}

impl Connections {
    /// Hear from each agent; look for them all the first time.
    fn poll(&mut self, settings: &Settings, wake: &egui::Context) {
        if self.each.is_empty() {
            self.each = agents::Kind::ALL.into_iter().map(Connection::new).collect();
            for connection in &mut self.each {
                connection.look(settings, wake);
            }
        }
        for connection in &mut self.each {
            connection.poll(settings, wake);
        }
        if self.each.iter().any(Connection::busy) {
            wake.request_repaint_after(std::time::Duration::from_millis(200));
        }
    }

    /// The window has closed: the agents asked are let go.
    pub fn forget(&mut self) {
        self.each.clear();
    }

    /// Where each agent stands, for a script.
    pub fn status(&self) -> Vec<(String, String)> {
        self.each.iter().map(|connection| (connection.kind.name().to_string(), connection.says())).collect()
    }
}

impl Assistant {
    /// The agents: one card each, with where it stands and what to do next.
    pub(crate) fn agents_section(&mut self, ui: &mut egui::Ui) {
        let p = theme::p();
        let ctx = ui.ctx().clone();
        let mut connections = std::mem::take(&mut self.connections);
        connections.poll(&self.settings, &ctx);
        ui.label(egui::RichText::new("Subscriptions and agents").strong());
        ui.label(
            egui::RichText::new("The AI company's own program, running on this computer and signed in with your account there. Tequila starts it, and never sees the login.")
                .small()
                .color(theme::dim()),
        );
        ui.add_space(4.0);
        let chosen = self.backend();
        for connection in &mut connections.each {
            let kind = connection.kind;
            let mut look = false;
            egui::Frame::none().fill(p.card).rounding(8.0).stroke(egui::Stroke::new(1.0_f32, p.line)).inner_margin(10.0).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let active = chosen == Some(Backend::Agent(kind));
                    if ui.radio(active, "").on_hover_text("Use this one in the chat").clicked() {
                        self.choose(&Backend::Agent(kind));
                    }
                    ui.label(egui::RichText::new(kind.name()).font(theme::strong(13.5)));
                    if connection.busy() {
                        ui.add(egui::Spinner::new().size(12.0));
                    }
                    let colour = match connection.status {
                        Status::SignedIn { .. } => p.success,
                        Status::Failed(_) => p.danger,
                        _ => theme::dim(),
                    };
                    ui.add(egui::Label::new(egui::RichText::new(connection.says()).small().color(colour)).wrap());
                });
                let status = connection.status.clone();
                ui.horizontal_wrapped(|ui| match status {
                    Status::Missing { node: true } => {
                        if ui.button("Install").on_hover_text(kind.install_says()).clicked() {
                            connection.installing = Some(agents::install(kind));
                            connection.status = Status::Installing;
                        }
                    }
                    Status::Missing { node: false } => {
                        if ui.button("Get Node.js").clicked() {
                            ui.ctx().open_url(egui::OpenUrl::new_tab("https://nodejs.org"));
                        }
                        look = ui.button("Check again").clicked();
                    }
                    Status::SignedOut => {
                        for way in connection.ways.clone().iter().filter(|way| !way.id.contains("api-key")) {
                            let button = ui.button(format!("Sign in: {}", way.name));
                            let button = if way.description.is_empty() { button } else { button.on_hover_text(&way.description) };
                            if button.clicked() {
                                match (&way.terminal, &connection.found, &mut connection.agent) {
                                    // In a terminal of the agent's own; the
                                    // person says when it is done.
                                    (Some(_), Some(found), _) => {
                                        if let Err(error) = agents::sign_in(found, way) {
                                            connection.status = Status::Failed(error);
                                        }
                                    }
                                    // Carried out by the agent, which opens
                                    // its maker's page and says when.
                                    (None, _, Some(agent)) => {
                                        agent.authenticate(&way.id);
                                        connection.status = Status::SigningIn;
                                    }
                                    _ => look = true,
                                }
                            }
                        }
                        if ui.button("Add an API key\u{2026}").clicked() {
                            self.add_provider(kind.key_preset());
                        }
                        look |= ui.button("Check again").on_hover_text("After signing in").clicked();
                    }
                    Status::SigningIn => look = ui.button("Start over").clicked(),
                    Status::SignedIn { .. } => {
                        // Its models, to choose the one the chat starts with.
                        if let Some(models) = &connection.models {
                            let key = format!("{}/model", kind.id());
                            let current = self.settings.choices.get(&key).cloned().unwrap_or_else(|| models.current.clone());
                            let shown = models.choices.iter().find(|c| c.value == current).map(|c| c.name.clone()).unwrap_or(current.clone());
                            ui.label(egui::RichText::new("Model").small().color(theme::dim()));
                            egui::ComboBox::from_id_salt(("connection-model", kind.id())).selected_text(shown).show_ui(ui, |ui| {
                                for choice in &models.choices {
                                    let row = ui.selectable_label(choice.value == current, &choice.name);
                                    let row = if choice.description.is_empty() { row } else { row.on_hover_text(&choice.description) };
                                    if row.clicked() {
                                        self.settings.choices.insert(key.clone(), choice.value.clone());
                                        self.save_settings();
                                        // The chat's own agent, if it is this
                                        // one, changes with it.
                                        if let Some((running, agent)) = &mut self.agent {
                                            if *running == kind {
                                                agent.set(&models.id, &choice.value);
                                            }
                                        }
                                    }
                                }
                            });
                        }
                        look = ui.small_button("Check again").clicked();
                    }
                    Status::Failed(_) => look = ui.button("Try again").clicked(),
                    Status::Installing | Status::Checking => {}
                });
                if let (Some(caveat), true) = (kind.caveat(), matches!(connection.status, Status::SignedIn { .. } | Status::SignedOut)) {
                    ui.add(egui::Label::new(egui::RichText::new(caveat).small().color(theme::dim())).wrap());
                }
            });
            if look {
                connection.look(&self.settings, &ctx);
            }
            ui.add_space(4.0);
        }
        self.connections = connections;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Write};
    use tequila_format::json;

    /// A stand-in agent on a local socket, answering as `script` says.
    fn stand_in(mut script: impl FnMut(&json::Value, &mut dyn FnMut(&str)) + Send + 'static) -> Agent {
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

    fn settle(connection: &mut Connection, done: impl Fn(&Connection) -> bool) {
        let ctx = egui::Context::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done(connection) && std::time::Instant::now() < deadline {
            connection.poll(&Settings::default(), &ctx);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(done(connection), "{:?}", connection.status);
    }

    const OPENED: &str = r#""result":{"sessionId":"s","configOptions":[{"id":"model","name":"Model","category":"model","type":"select","currentValue":"a","options":[{"value":"a","name":"Sonnet"},{"value":"b","name":"Opus"}]}]}"#;

    /// An agent that is signed in says so, with what pays and its models,
    /// and is let go once it has.
    #[test]
    fn an_agent_signed_in_says_what_pays_and_which_models_it_has() {
        let mut connection = Connection::new(agents::Kind::Claude);
        connection.watch(stand_in(|message, write| match message.get("method").and_then(|m| m.as_str()) {
            Some("initialize") => write(&format!(r#"{{"jsonrpc":"2.0","id":{},"result":{{"protocolVersion":1}}}}"#, id_of(message))),
            Some("session/new") => {
                write(r#"{"jsonrpc":"2.0","method":"_auth/status_update","params":{"authStatus":{"kind":"api_key","label":"Anthropic API key"}}}"#);
                write(&format!(r#"{{"jsonrpc":"2.0","id":{},{OPENED}}}"#, id_of(message)));
            }
            _ => {}
        }));
        assert_eq!(connection.says(), "Checking\u{2026}");
        settle(&mut connection, |c| c.agent.is_none());
        assert_eq!(connection.says(), "Signed in \u{b7} paid for by an API key, by use (Anthropic API key)");
        let models: Vec<&str> = connection.models.as_ref().unwrap().choices.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(models, ["Sonnet", "Opus"]);
        assert!(!connection.busy());
    }

    /// One that opens a conversation and says nobody is signed in (Claude's
    /// way), and one that will not open one (Codex's), both read as signed
    /// out with their ways of signing in; and a sign-in the agent carries
    /// out itself ends signed in.
    #[test]
    fn an_agent_signed_out_offers_its_sign_ins() {
        let ways = r#""authMethods":[{"id":"console","name":"Anthropic Console","type":"terminal","args":["--cli","auth","login"]}]"#;
        let mut claude = Connection::new(agents::Kind::Claude);
        claude.watch(stand_in(move |message, write| match message.get("method").and_then(|m| m.as_str()) {
            Some("initialize") => write(&format!(r#"{{"jsonrpc":"2.0","id":{},"result":{{"protocolVersion":1,{ways}}}}}"#, id_of(message))),
            Some("session/new") => {
                write(r#"{"jsonrpc":"2.0","method":"_auth/status_update","params":{"authStatus":{"kind":"none","label":"Not logged in"}}}"#);
                write(&format!(r#"{{"jsonrpc":"2.0","id":{},{OPENED}}}"#, id_of(message)));
            }
            _ => {}
        }));
        settle(&mut claude, |c| c.models.is_some());
        assert_eq!((claude.says().as_str(), claude.ways.len()), ("Installed, not signed in", 1));
        assert!(claude.ways[0].terminal.is_some());

        let signed_in = std::sync::Arc::new(std::sync::Mutex::new(false));
        let state = signed_in.clone();
        let mut codex = Connection::new(agents::Kind::ChatGpt);
        codex.watch(stand_in(move |message, write| match message.get("method").and_then(|m| m.as_str()) {
            Some("initialize") => write(&format!(r#"{{"jsonrpc":"2.0","id":{},"result":{{"protocolVersion":1,"authMethods":[{{"id":"chat-gpt","name":"ChatGPT"}}]}}}}"#, id_of(message))),
            Some("session/new") if *state.lock().unwrap() => write(&format!(r#"{{"jsonrpc":"2.0","id":{},{OPENED}}}"#, id_of(message))),
            Some("session/new") => write(&format!(r#"{{"jsonrpc":"2.0","id":{},"error":{{"code":-32000,"message":"Authentication required"}}}}"#, id_of(message))),
            Some("authenticate") => {
                *state.lock().unwrap() = true;
                write(&format!(r#"{{"jsonrpc":"2.0","id":{},"result":{{}}}}"#, id_of(message)));
            }
            _ => {}
        }));
        settle(&mut codex, |c| c.status == Status::SignedOut);
        assert_eq!(codex.ways[0].id, "chat-gpt");
        // The agent is kept, to sign in through.
        codex.agent.as_mut().expect("still running").authenticate("chat-gpt");
        codex.status = Status::SigningIn;
        settle(&mut codex, |c| c.agent.is_none());
        assert_eq!(codex.says(), "Signed in");
    }

    /// An agent whose program ends as it starts is said, not asked forever.
    #[test]
    fn an_agent_that_will_not_start_says_so() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || drop(listener.accept()));
        let stream = std::net::TcpStream::connect(address).unwrap();
        let mut connection = Connection::new(agents::Kind::Claude);
        connection.watch(Agent::over(acp::Launch::default(), stream.try_clone().unwrap(), stream, || {}));
        settle(&mut connection, |c| c.agent.is_none());
        assert!(matches!(&connection.status, Status::Failed(text) if text.contains("stopped as it started")));
        assert!(!connection.busy());
    }

    /// The page drawn with an agent in every state.
    #[test]
    fn the_page_draws_every_state() {
        let mut assistant = Assistant::default();
        let states = [
            Status::Missing { node: true },
            Status::Missing { node: false },
            Status::Installing,
            Status::Checking,
            Status::SignedOut,
            Status::SigningIn,
            Status::SignedIn { kind: "account".into(), label: "me@example.com".into() },
            Status::Failed("npm could not install it".into()),
        ];
        let ctx = egui::Context::default();
        for status in states {
            let mut connection = Connection::new(agents::Kind::ChatGpt);
            connection.status = status;
            connection.ways = vec![acp::SignIn { id: "chat-gpt".into(), name: "ChatGPT".into(), description: String::new(), terminal: None }];
            connection.models = Some(acp::Setting {
                id: "model".into(),
                name: "Model".into(),
                category: "model".into(),
                current: "a".into(),
                choices: vec![acp::Choice { value: "a".into(), name: "6.1 Sol".into(), description: String::new(), group: String::new() }],
            });
            assistant.connections.each = vec![connection];
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| assistant.agents_section(ui));
            });
        }
        assert_eq!(assistant.connections.status().len(), 1);
    }
}
