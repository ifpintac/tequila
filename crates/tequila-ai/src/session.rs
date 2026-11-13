//! One conversation with the assistant, and the loop that drives it.
//!
//! The loop is the usual one — ask; the model answers with words, or asks for
//! tools; the tools' results go back; repeat until it answers with words
//! alone — split so the application stays in charge. Network waits happen on
//! a thread of their own and are polled once a frame; tool calls are handed
//! to the application to carry out on its own thread, because only it can
//! touch the document.
//!
//! Replies are streamed. The panel shows what the model is doing while it
//! does it — reading the request, thinking, writing, which tool it is
//! calling — and a reply is waited for as long as it keeps coming, so a
//! slow model on a modest computer finishes instead of being cut off at an
//! arbitrary minute. A server that will not stream is asked again for the
//! whole reply.

use crate::conversation::{Call, Message, Part, Role};
use crate::http::{self, Hangup, Limits, Piece};
use crate::ollama;
use crate::provider::{self, Provider};
use crate::sse;
use crate::tools;
use crate::wire::{self, Activity};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How many rounds of tool use one request may take before the loop stops
/// and says so. Enough for real work — look, change twenty materials in a few
/// batches, check, adjust — and a hard ceiling on a model stuck in a loop,
/// which would otherwise spend the person's money until they noticed.
pub const MAX_STEPS: usize = 40;

/// How many pictures a request to a model on this computer carries: the
/// newest only. Each is read again every round, and on a modest graphics
/// card a picture is seconds of reading; the newest is the one that counts.
pub const LOCAL_IMAGES_KEPT: usize = 1;

/// What one reply asked for.
#[derive(Clone, Debug, PartialEq)]
pub struct Turn {
    pub text: Vec<String>,
    /// Tool calls to carry out, then [`Session::answer`]. None means the
    /// assistant has finished.
    pub calls: Vec<Call>,
    /// What the model reasoned on the way, where it said.
    pub reasoning: String,
}

/// What a request on its way is doing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// A server on this computer loading the model into memory.
    Loading,
    /// Sent, and being read: nothing has come back yet.
    Reading,
    Thinking,
    Writing,
    /// Writing a call to this tool.
    Calling(String),
}

/// A request on its way, as the panel shows it.
#[derive(Clone, Debug)]
pub struct Progress {
    pub phase: Phase,
    /// When this round was sent.
    pub since: Instant,
    /// The model's reasoning so far, where it sends it.
    pub reasoning: String,
    /// Its words so far.
    pub text: String,
    /// About how many tokens it has written: four characters to a token.
    pub tokens: usize,
    /// When its reasoning began, and how long it has gone on: from the
    /// first of it to the latest.
    pub thinking_from: Option<Instant>,
    pub thought_for: Duration,
}

impl Progress {
    fn new() -> Progress {
        Progress {
            phase: Phase::Reading,
            since: Instant::now(),
            reasoning: String::new(),
            text: String::new(),
            tokens: 0,
            thinking_from: None,
            thought_for: Duration::ZERO,
        }
    }
}

/// What the request thread and the session share.
#[derive(Clone)]
struct Job {
    hangup: Hangup,
    progress: Arc<Mutex<Progress>>,
    /// Things worth knowing about the model, to say once.
    notes: Arc<Mutex<Vec<String>>>,
    /// False once the provider has refused to stream.
    streams: Arc<AtomicBool>,
    limits: Limits,
}

impl Job {
    fn progress(&self) -> std::sync::MutexGuard<'_, Progress> {
        self.progress.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn phase(&self, phase: Phase) {
        self.progress().phase = phase;
    }

    fn note(&self, text: String) {
        self.notes.lock().unwrap_or_else(|e| e.into_inner()).push(text);
    }

    fn show(&self, reply: &wire::Assembler) {
        let mut progress = self.progress();
        progress.phase = match reply.activity() {
            Activity::Waiting => Phase::Reading,
            Activity::Thinking => Phase::Thinking,
            Activity::Writing => Phase::Writing,
            Activity::Calling(name) => Phase::Calling(name.clone()),
        };
        progress.reasoning = reply.reasoning();
        progress.text = reply.text();
        progress.tokens = reply.written().div_ceil(4);
        if progress.phase == Phase::Thinking {
            let from = *progress.thinking_from.get_or_insert_with(Instant::now);
            progress.thought_for = from.elapsed();
        }
    }
}

struct Pending {
    receiver: Receiver<Result<wire::Reply, String>>,
    job: Job,
}

pub struct Session {
    pub provider: Provider,
    pub messages: Vec<Message>,
    pending: Option<Pending>,
    steps: usize,
    streams: Arc<AtomicBool>,
    /// How long to wait for a reply; by default, what suits the provider's
    /// address ([`Limits::for_url`]).
    pub limits: Option<Limits>,
    /// Notes said already, and notes waiting to be said.
    noted: Vec<String>,
    unread: Vec<String>,
}

impl Session {
    pub fn new(provider: Provider) -> Session {
        Session {
            provider,
            messages: Vec::new(),
            pending: None,
            steps: 0,
            streams: Arc::new(AtomicBool::new(true)),
            limits: None,
            noted: Vec::new(),
            unread: Vec::new(),
        }
    }

    /// Waiting for the model.
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub fn steps(&self) -> usize {
        self.steps
    }

    /// What the request on its way is doing, if one is.
    pub fn progress(&self) -> Option<Progress> {
        self.pending.as_ref().map(|p| p.job.progress().clone())
    }

    /// Things worth knowing about the model — it cannot see, it runs on
    /// the processor — each said once in a conversation.
    pub fn take_notes(&mut self) -> Vec<String> {
        if let Some(pending) = &self.pending {
            let fresh = std::mem::take(&mut *pending.job.notes.lock().unwrap_or_else(|e| e.into_inner()));
            self.unread.extend(fresh);
        }
        let mut out = Vec::new();
        for note in std::mem::take(&mut self.unread) {
            if !self.noted.contains(&note) {
                self.noted.push(note.clone());
                out.push(note);
            }
        }
        out
    }

    /// Ask something new.
    pub fn ask(&mut self, text: &str) -> Result<(), String> {
        self.ask_with(text, &[])
    }

    /// Ask something new, with pictures the person attached: PNGs,
    /// base64-encoded.
    pub fn ask_with(&mut self, text: &str, pictures: &[String]) -> Result<(), String> {
        if self.busy() {
            return Err("the assistant is still working on the last request".into());
        }
        if let Some(problem) = self.provider.problem() {
            return Err(format!("{} is not set up: {problem}", self.provider.name));
        }
        self.steps = 0;
        // After a failed request the conversation already ends with the
        // person's words; add to them, since both formats expect the two
        // sides to take turns.
        match self.messages.last_mut() {
            Some(last) if last.role == Role::User => last.parts.push(Part::Text(text.to_string())),
            _ => self.messages.push(Message::user(text)),
        }
        if let Some(last) = self.messages.last_mut() {
            last.parts.extend(pictures.iter().cloned().map(Part::Image));
        }
        self.send();
        Ok(())
    }

    /// Hand back the results of the calls the last turn asked for.
    pub fn answer(&mut self, results: Vec<Part>) -> Result<(), String> {
        self.messages.push(Message { role: Role::User, parts: results });
        self.steps += 1;
        if self.steps >= MAX_STEPS {
            return Err(format!(
                "stopped after {MAX_STEPS} steps without finishing — ask again to let it continue"
            ));
        }
        self.send();
        Ok(())
    }

    /// The next turn, once it has arrived.
    pub fn poll(&mut self) -> Option<Result<Turn, String>> {
        let pending = self.pending.as_ref()?;
        let result = match pending.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err("the request ended without an answer".into()),
        };
        let pending = self.pending.take().expect("checked above");
        self.unread.extend(std::mem::take(&mut *pending.job.notes.lock().unwrap_or_else(|e| e.into_inner())));
        Some(result.map(|reply| {
            let mut parts: Vec<Part> = reply.text.iter().cloned().map(Part::Text).collect();
            parts.extend(reply.calls.iter().cloned().map(Part::Call));
            if !parts.is_empty() {
                self.messages.push(Message { role: Role::Assistant, parts });
            }
            Turn { text: reply.text, calls: reply.calls, reasoning: reply.reasoning }
        }))
    }

    /// Stop, at once. The reply's connection is closed, so the server stops
    /// writing it; calls the model asked for that were never carried out
    /// are answered as stopped, so the next request is still a valid
    /// conversation.
    pub fn cancel(&mut self) {
        if let Some(pending) = self.pending.take() {
            pending.job.hangup.hang_up();
        }
        if let Some(last) = self.messages.last() {
            if last.role == Role::Assistant {
                let stopped: Vec<Part> = last
                    .parts
                    .iter()
                    .filter_map(|p| match p {
                        Part::Call(call) => Some(Part::Result {
                            id: call.id.clone(),
                            name: call.name.clone(),
                            text: "Stopped by the person before this ran.".into(),
                            image: None,
                            is_error: true,
                        }),
                        _ => None,
                    })
                    .collect();
                if !stopped.is_empty() {
                    self.messages.push(Message { role: Role::User, parts: stopped });
                }
            }
        }
    }

    fn send(&mut self) {
        let (sender, receiver) = channel();
        let job = Job {
            hangup: Hangup::new(),
            progress: Arc::new(Mutex::new(Progress::new())),
            notes: Arc::new(Mutex::new(Vec::new())),
            streams: self.streams.clone(),
            limits: self.limits.unwrap_or_else(|| Limits::for_url(&self.provider.base_url)),
        };
        let provider = self.provider.clone();
        let messages = self.messages.clone();
        let theirs = job.clone();
        let spawned = std::thread::Builder::new().name("assistant-request".into()).spawn(move || {
            let _ = sender.send(exchange(&provider, &messages, &theirs));
        });
        let receiver = match spawned {
            Ok(_) => receiver,
            Err(error) => {
                let (sender, receiver) = channel();
                let _ = sender.send(Err(format!("the request could not be started: {error}")));
                receiver
            }
        };
        self.pending = Some(Pending { receiver, job });
    }
}

/// One round trip: the conversation out, the reply back.
fn exchange(provider: &Provider, messages: &[Message], job: &Job) -> Result<wire::Reply, String> {
    let headers = provider.headers()?;
    let ollama = if ollama::worth_asking(provider) {
        ollama::inspect(provider, &headers, &job.hangup, || job.phase(Phase::Loading)).map_err(|e| advise(provider, None, e))?
    } else {
        None
    };
    send_turn(provider, messages, job, &headers, ollama.as_ref()).map_err(|e| advise(provider, ollama.as_ref(), e))
}

fn send_turn(provider: &Provider, messages: &[Message], job: &Job, headers: &[(String, String)], ollama: Option<&ollama::Info>) -> Result<wire::Reply, String> {
    let mut options = wire::Options::plain(provider);
    // Lighter for a model on this computer; a service reads fast enough.
    if provider::is_local(&provider.base_url) {
        options.images = options.images.min(LOCAL_IMAGES_KEPT);
    }
    if let Some(info) = ollama {
        if info.can("tools") == Some(false) {
            return Err(ollama::no_tools(info));
        }
        if info.can("vision") == Some(false) && options.images > 0 {
            options.images = 0;
            job.note(format!("{} can't see images in Ollama, so it works without screenshots.", info.model));
        }
        if let Some(note) = ollama::speed_note(info) {
            job.note(note);
        }
    }
    job.phase(Phase::Reading);
    let tools = tools::all();
    let body = |stream| wire::request_body_with(provider, tools::system(), &tools, messages, wire::Options { stream, ..options });
    if let Some(info) = ollama {
        let tokens = ollama::estimate_tokens(&body(false));
        ollama::check_fit(info, tokens, provider.max_tokens)?;
        // It fits; say early if the next long tool answer will not.
        if let Some(note) = ollama::room_to_grow(info, tokens, provider.max_tokens) {
            job.note(note);
        }
    }
    let url = provider.chat_url();
    if job.streams.load(Ordering::SeqCst) {
        match streamed(provider, &url, headers, body(true), job)? {
            Some(reply) => return Ok(wire::with_written_calls(reply, &tools)),
            // Refused: ask for the whole reply, now and for the rest of
            // the conversation.
            None => job.streams.store(false, Ordering::SeqCst),
        }
    }
    let response = http::post_watched(&url, headers, body(false), job.limits.whole(), &job.hangup)?;
    reply_of(provider, response.status, &response.body).map(|reply| wire::with_written_calls(reply, &tools))
}

/// Ask for the reply as a stream of events, and put it together as it
/// comes. None when the server will not stream.
fn streamed(provider: &Provider, url: &str, headers: &[(String, String)], body: String, job: &Job) -> Result<Option<wire::Reply>, String> {
    let mut parser = sse::Parser::new();
    let mut reply = wire::Assembler::new(provider.kind);
    let mut events = Vec::new();
    let mut status = 0;
    let mut streaming = false;
    let mut whole = Vec::new();
    let outcome = http::post_streaming(url, headers, body, job.limits, &job.hangup, |piece| {
        match piece {
            Piece::Head { status: s, events: e } => {
                status = s;
                streaming = e && (200..300).contains(&s);
            }
            Piece::Body(bytes) if streaming => {
                parser.feed(bytes, &mut events);
                for event in events.drain(..) {
                    reply.event(&event)?;
                }
                // Each piece as it comes: a server can pause for minutes
                // after one, and the panel should not be a piece behind.
                job.show(&reply);
            }
            Piece::Body(bytes) => whole.extend_from_slice(bytes),
        }
        Ok(())
    });
    // An error the service sent inside the stream reads like any other.
    outcome.map_err(|message| if message == http::STOPPED { message } else { explain(status, message) })?;
    if streaming {
        parser.finish(&mut events);
        for event in events.drain(..) {
            reply.event(&event)?;
        }
        job.show(&reply);
        return reply.finish().map(Some);
    }
    let text = String::from_utf8_lossy(&whole);
    if refuses_streaming(status, &text) {
        return Ok(None);
    }
    // Asked to stream, it answered all at once: as good.
    reply_of(provider, status, &text).map(Some)
}

/// A server saying it cannot stream, rather than failing for a reason the
/// whole reply would share.
fn refuses_streaming(status: u16, body: &str) -> bool {
    matches!(status, 400 | 405 | 415 | 422 | 501) && body.to_ascii_lowercase().contains("stream")
}

fn reply_of(provider: &Provider, status: u16, body: &str) -> Result<wire::Reply, String> {
    match wire::parse_reply(provider.kind, body) {
        Ok(reply) if (200..300).contains(&status) => Ok(reply),
        Err(message) => Err(explain(status, message)),
        Ok(_) => Err(explain(status, format!("the service answered {status}"))),
    }
}

/// A failure, with what helps where something does.
fn advise(provider: &Provider, ollama: Option<&ollama::Info>, error: String) -> String {
    if error == http::STOPPED {
        return error;
    }
    let slow = ["had not started to answer", "stopped sending part-way", "took too long"].iter().any(|s| error.contains(s));
    if !slow {
        return error;
    }
    if provider::is_local(&provider.base_url) || ollama.is_some() {
        let mut out = format!(
            "{error}. A model on this computer reads the whole request before it writes a word, and the assistant's requests are long: its \
             instructions, its tools and the conversation so far. A smaller or faster model answers sooner, as does one that fits in the \
             graphics card's memory."
        );
        if let Some(note) = ollama.and_then(ollama::speed_note) {
            out.push(' ');
            out.push_str(&note);
        }
        out
    } else {
        format!("{error}. The service may be busy: try again, or choose a faster model.")
    }
}

/// An error, with what usually causes that status.
fn explain(status: u16, message: String) -> String {
    let lower = message.to_lowercase();
    let context = lower.contains("context") && ["length", "window", "overflow", "exceed", "too long", "too large"].iter().any(|w| lower.contains(w));
    let hint = if context {
        " — the request is longer than the model's context length allows. Load the model with a longer one (in LM Studio, Context Length \
         when loading it; in Ollama, its Context length setting), or start a New chat"
    } else {
        match status {
            401 | 403 => " — check the API key",
            404 => " — check the address and the model name",
            429 => " — the service is rate-limiting or out of credit; wait, or check the account",
            500..=599 => " — the service had a problem; try again shortly",
            _ => "",
        }
    };
    format!("{message}{hint}")
}

/// Ask a provider which models it offers.
pub fn fetch_models(provider: &Provider) -> Result<Vec<String>, String> {
    let response = crate::http::get(&provider.models_url(), &provider.headers()?)?;
    wire::parse_models(&response.body).map_err(|message| explain(response.status, message))
}

/// Check a provider works, with a one-line request and no tools. For a
/// model Ollama runs, also whether it can work in the scene: whether it can
/// use tools, and whether the assistant's first request fits its context.
pub fn test(provider: &Provider) -> Result<String, String> {
    if let Some(problem) = provider.problem() {
        return Err(problem);
    }
    let headers = provider.headers()?;
    let messages = [Message::user("Reply with exactly one word: ready")];
    let mut small = provider.clone();
    small.max_tokens = 32;
    let body = wire::request_body(&small, "You are being connected to Tequila.", &[], &messages);
    let response = crate::http::post(&provider.chat_url(), &headers, body)?;
    let answer = match wire::parse_reply(provider.kind, &response.body) {
        Ok(reply) if (200..300).contains(&response.status) => reply.text.join(" ").trim().to_string(),
        Err(message) => return Err(explain(response.status, message)),
        Ok(_) => return Err(explain(response.status, format!("the service answered {}", response.status))),
    };
    if ollama::worth_asking(provider) {
        if let Ok(Some(info)) = ollama::inspect(provider, &headers, &Hangup::new(), || {}) {
            if info.can("tools") == Some(false) {
                return Err(format!("It answered ({answer}), but: {}", ollama::no_tools(&info)));
            }
            let first = wire::request_body(provider, tools::system(), &tools::all(), &[Message::user("Make the strap black velvet.")]);
            let tokens = ollama::estimate_tokens(&first);
            if let Err(message) = ollama::check_fit(&info, tokens, provider.max_tokens) {
                return Err(format!("It answered ({answer}), but: {message}"));
            }
            if let Some(note) = ollama::room_to_grow(&info, tokens, provider.max_tokens) {
                return Err(format!("It answered ({answer}). {note}"));
            }
            if let Some(context) = info.context {
                return Ok(format!("{answer} — Ollama gives {} {} tokens of context", info.model, ollama::thousands(context)));
            }
        }
    }
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{Key, Kind};
    use std::collections::{HashMap, VecDeque};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    /// A canned reply: a status, a content type, and the body, sent in
    /// pieces each after a pause in milliseconds.
    #[derive(Clone)]
    struct Canned {
        status: u16,
        content_type: &'static str,
        pieces: Vec<(u64, String)>,
    }

    fn json(body: &str) -> Canned {
        Canned { status: 200, content_type: "application/json", pieces: vec![(0, body.to_string())] }
    }

    fn failing(status: u16, body: &str) -> Canned {
        Canned { status, ..json(body) }
    }

    /// Server-sent events, one `data:` each, sent together.
    fn events(data: &[&str]) -> Canned {
        paced(data.iter().map(|d| (0, *d)).collect())
    }

    /// Server-sent events, each after its pause.
    fn paced(data: Vec<(u64, &str)>) -> Canned {
        Canned { status: 200, content_type: "text/event-stream", pieces: data.into_iter().map(|(pause, d)| (pause, format!("data: {d}\n\n"))).collect() }
    }

    type Seen = Arc<Mutex<Vec<(String, String)>>>;

    /// A stand-in model service on a local port. Each path answers with its
    /// next canned reply, in order; a path with none left, or none at all,
    /// answers 404. Every request is kept: its path and its body.
    fn service(routes: Vec<(&'static str, Vec<Canned>)>) -> (String, Seen) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}/v1", listener.local_addr().unwrap().port());
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let mut queues: HashMap<&'static str, VecDeque<Canned>> = routes.into_iter().map(|(path, replies)| (path, replies.into())).collect();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request_line = String::new();
                if reader.read_line(&mut request_line).is_err() {
                    continue;
                }
                let path = request_line.split_whitespace().nth(1).unwrap_or("").to_string();
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; length];
                let _ = reader.read_exact(&mut body);
                log.lock().unwrap().push((path.clone(), String::from_utf8_lossy(&body).into_owned()));
                let canned = queues
                    .get_mut(path.as_str())
                    .and_then(VecDeque::pop_front)
                    .unwrap_or_else(|| failing(404, r#"{"error":"not found"}"#));
                // Each reply on its own thread, so a slow one holds up no
                // other request.
                std::thread::spawn(move || {
                    let mut stream = stream;
                    let reason = if canned.status == 200 { "OK" } else { "Error" };
                    let mut head = format!("HTTP/1.1 {} {reason}\r\ncontent-type: {}\r\nconnection: close\r\n", canned.status, canned.content_type);
                    if canned.content_type != "text/event-stream" {
                        let length: usize = canned.pieces.iter().map(|(_, p)| p.len()).sum();
                        head.push_str(&format!("content-length: {length}\r\n"));
                    }
                    head.push_str("\r\n");
                    let mut first = true;
                    for (pause, piece) in canned.pieces {
                        std::thread::sleep(Duration::from_millis(pause));
                        // A server sends its head with the first of the
                        // body, as Ollama does once it has read the request.
                        let out = if std::mem::take(&mut first) { format!("{head}{piece}") } else { piece };
                        if stream.write_all(out.as_bytes()).and_then(|_| stream.flush()).is_err() {
                            return;
                        }
                    }
                });
            }
        });
        (url, seen)
    }

    fn local(url: String) -> Provider {
        Provider {
            id: "t".into(),
            name: "Test".into(),
            kind: Kind::OpenAi,
            base_url: url,
            model: "test".into(),
            key: Key::None,
            key_header: String::new(),
            vision: true,
            max_tokens: 100,
        }
    }

    fn session(url: String) -> Session {
        let mut session = Session::new(local(url));
        session.limits = Some(Limits { first: Duration::from_secs(5), silence: Duration::from_secs(5) });
        session
    }

    fn wait(session: &mut Session) -> Result<Turn, String> {
        for _ in 0..1000 {
            if let Some(result) = session.poll() {
                return result;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("no answer");
    }

    fn chat(path_replies: Vec<Canned>) -> (String, Seen) {
        service(vec![("/v1/chat/completions", path_replies)])
    }

    /// The whole loop against a real socket: a tool call, its result sent
    /// back, and a final answer.
    #[test]
    fn a_request_goes_round_the_loop_until_the_model_is_done() {
        let (url, seen) = chat(vec![
            json(r#"{"choices":[{"message":{"content":"Looking.","tool_calls":[{"id":"c1","type":"function","function":{"name":"tequila_scene","arguments":"{}"}}]}}]}"#),
            json(r#"{"choices":[{"message":{"content":"The scene has 355 parts."}}]}"#),
        ]);
        let mut session = session(url);
        session.ask("How many parts?").unwrap();
        assert!(session.busy());

        let turn = wait(&mut session).unwrap();
        assert_eq!(turn.text, ["Looking."]);
        assert_eq!(turn.calls[0].name, "tequila_scene");

        session
            .answer(vec![Part::Result { id: "c1".into(), name: "tequila_scene".into(), text: "{\"parts\":355}".into(), image: None, is_error: false }])
            .unwrap();
        let turn = wait(&mut session).unwrap();
        assert!(turn.calls.is_empty(), "finished");
        assert_eq!(turn.text, ["The scene has 355 parts."]);

        let sent = seen.lock().unwrap();
        let chats: Vec<&String> = sent.iter().filter(|(p, _)| p == "/v1/chat/completions").map(|(_, b)| b).collect();
        assert!(chats[0].contains("How many parts?") && chats[0].contains("tequila_scene"));
        assert!(chats[0].contains("\"stream\": true") || chats[0].contains("\"stream\":true"), "it asked for a stream");
        assert!(chats[1].contains("\"tool_call_id\": \"c1\"") || chats[1].contains("\"tool_call_id\":\"c1\""), "{}", chats[1]);
        assert!(chats[1].contains("355"));
    }

    #[test]
    fn streamed_words_are_put_together_and_shown_as_they_come() {
        let (url, _) = chat(vec![paced(vec![
            (0, r#"{"choices":[{"delta":{"role":"assistant","content":"The scene "}}]}"#),
            (300, r#"{"choices":[{"delta":{"content":"has 355 parts."}}]}"#),
            (300, r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#),
            (0, "[DONE]"),
        ])]);
        let mut session = session(url);
        session.ask("How many parts?").unwrap();
        let mut seen_part_way = None;
        let turn = loop {
            if let Some(result) = session.poll() {
                break result.unwrap();
            }
            if let Some(progress) = session.progress() {
                if progress.phase == Phase::Writing && !progress.text.is_empty() && progress.text.len() < 20 {
                    seen_part_way = Some(progress.text.clone());
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(turn.text, ["The scene has 355 parts."]);
        assert_eq!(seen_part_way.as_deref(), Some("The scene"), "the panel saw the words as they came");
        assert!(turn.calls.is_empty());
    }

    /// Tool calls arrive in pieces, numbered, their arguments split at any
    /// character, two interleaved; they come out whole and in order.
    #[test]
    fn tool_calls_split_across_pieces_come_out_whole() {
        let (url, _) = chat(vec![events(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"tequila_nodes","arguments":""}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"fil"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"id":"call_b","type":"function","function":{"name":"tequila_materials","arguments":"{"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"ter\": \"strap\"}"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"function":{"arguments":"}"}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            "[DONE]",
        ])]);
        let mut session = session(url);
        session.ask("Find the strap").unwrap();
        let turn = wait(&mut session).unwrap();
        assert_eq!(turn.calls.len(), 2);
        assert_eq!((turn.calls[0].id.as_str(), turn.calls[0].name.as_str()), ("call_a", "tequila_nodes"));
        assert_eq!(turn.calls[0].arguments.as_ref().unwrap().get("filter").and_then(|f| f.as_str()), Some("strap"));
        assert_eq!(turn.calls[1].name, "tequila_materials");
        assert!(turn.calls[1].arguments.is_ok());
    }

    /// A call the model wrote into its words, which the server passed on
    /// as words (`wire::with_written_calls`), goes round the loop like any
    /// other: carried out, answered, and sent back as the call it was.
    #[test]
    fn a_call_written_into_the_answer_goes_round_the_loop() {
        let (url, seen) = chat(vec![
            events(&[
                r#"{"choices":[{"index":0,"delta":{"role":"assistant","content":"First, let me look at the scene."},"finish_reason":null}]}"#,
                r#"{"choices":[{"index":0,"delta":{"content":"\n\n\u003cfunction"},"finish_reason":null}]}"#,
                r#"{"choices":[{"index":0,"delta":{"content":"=tequila_scene\u003e"},"finish_reason":null}]}"#,
                r#"{"choices":[{"index":0,"delta":{"content":"\n\u003c/function\u003e"},"finish_reason":null}]}"#,
                r#"{"choices":[{"index":0,"delta":{"content":"\n\u003c/tool_call\u003e"},"finish_reason":null}]}"#,
                r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
                "[DONE]",
            ]),
            json(r#"{"choices":[{"message":{"role":"assistant","content":"There are 24 parts."},"finish_reason":"stop"}]}"#),
        ]);
        let mut session = session(url);
        session.ask("What is in the scene?").unwrap();
        let turn = wait(&mut session).unwrap();
        assert_eq!(turn.text, ["First, let me look at the scene."]);
        assert_eq!(turn.calls.len(), 1, "the turn does not end on the markup");
        assert_eq!(turn.calls[0].name, "tequila_scene");
        session
            .answer(vec![Part::Result { id: turn.calls[0].id.clone(), name: "tequila_scene".into(), text: "{\"parts\":24}".into(), image: None, is_error: false }])
            .unwrap();
        assert_eq!(wait(&mut session).unwrap().text, ["There are 24 parts."]);
        let sent = seen.lock().unwrap();
        let second = &sent.iter().filter(|(path, _)| path.ends_with("/completions")).nth(1).expect("the result was sent").1;
        assert!(second.contains("\"tool_calls\"") && second.contains("\"tool_call_id\""), "the call goes back as a call: {second}");
        assert!(!second.contains("function="), "and none of the markup: {second}");
    }

    /// Reasoning beside the answer, as Ollama sends it (`reasoning`) and as
    /// others do (`reasoning_content`), and inside it, as `<think>`."""
    #[test]
    fn reasoning_is_kept_apart_from_the_answer() {
        let (url, _) = chat(vec![
            events(&[
                r#"{"choices":[{"delta":{"role":"assistant","content":"","reasoning":"The strap "}}]}"#,
                r#"{"choices":[{"delta":{"content":"","reasoning":"is part 4."}}]}"#,
                r#"{"choices":[{"delta":{"content":"Done."}}]}"#,
                r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
            ]),
            events(&[
                r#"{"choices":[{"delta":{"reasoning_content":"Looking first."}}]}"#,
                r#"{"choices":[{"delta":{"content":"Ok."},"finish_reason":"stop"}]}"#,
            ]),
            events(&[
                r#"{"choices":[{"delta":{"content":"<think>Hmm, "}}]}"#,
                r#"{"choices":[{"delta":{"content":"velvet.</think>\n\nVelvet it is."}}]}"#,
                r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
            ]),
        ]);
        let mut session = session(url);
        session.ask("one").unwrap();
        let turn = wait(&mut session).unwrap();
        assert_eq!((turn.reasoning.as_str(), turn.text.as_slice()), ("The strap is part 4.", &["Done.".to_string()][..]));
        session.ask("two").unwrap();
        assert_eq!(wait(&mut session).unwrap().reasoning, "Looking first.");
        session.ask("three").unwrap();
        let turn = wait(&mut session).unwrap();
        assert_eq!(turn.reasoning, "Hmm, velvet.");
        assert_eq!(turn.text, ["Velvet it is."]);
        // What goes back to the model is the answer, not the reasoning.
        let said: Vec<&Part> = session.messages.iter().filter(|m| m.role == Role::Assistant).flat_map(|m| &m.parts).collect();
        assert!(said.iter().all(|p| !matches!(p, Part::Text(t) if t.contains("think") || t.contains("Hmm"))), "{said:?}");
    }

    /// A server that takes its time over the request — a model on this
    /// computer reading it — is waited for, past what silence would allow.
    #[test]
    fn a_slow_first_byte_is_still_accepted() {
        let (url, _) = chat(vec![paced(vec![(1500, r#"{"choices":[{"delta":{"content":"Here."},"finish_reason":"stop"}]}"#)])]);
        let mut session = session(url);
        session.limits = Some(Limits { first: Duration::from_secs(4), silence: Duration::from_millis(500) });
        session.ask("hello").unwrap();
        assert_eq!(wait(&mut session).unwrap().text, ["Here."]);
    }

    #[test]
    fn silence_part_way_ends_in_an_error_that_says_what_helps() {
        let (url, _) = chat(vec![paced(vec![
            (0, r#"{"choices":[{"delta":{"content":"Starting"}}]}"#),
            (5000, r#"{"choices":[{"delta":{"content":" too late"}}]}"#),
        ])]);
        let mut session = session(url);
        session.limits = Some(Limits { first: Duration::from_secs(4), silence: Duration::from_millis(500) });
        session.ask("hello").unwrap();
        let error = wait(&mut session).unwrap_err();
        assert!(error.contains("stopped sending part-way"), "{error}");
        assert!(error.contains("smaller or faster model"), "a model on this computer: {error}");
        // The conversation is kept: asking again carries on from it.
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.messages[0].role, Role::User);
    }

    /// A server that answers all at once though asked to stream is read as
    /// it is; one that refuses to stream is asked again for the whole
    /// reply, and not asked to stream again.
    #[test]
    fn a_server_that_does_not_stream_still_works() {
        let (url, seen) = chat(vec![
            failing(400, r#"{"error":{"message":"stream is not supported"}}"#),
            json(r#"{"choices":[{"message":{"content":"Whole."}}]}"#),
            json(r#"{"choices":[{"message":{"content":"Again."}}]}"#),
        ]);
        let mut session = session(url);
        session.ask("one").unwrap();
        assert_eq!(wait(&mut session).unwrap().text, ["Whole."]);
        session.ask("two").unwrap();
        assert_eq!(wait(&mut session).unwrap().text, ["Again."]);
        let sent = seen.lock().unwrap();
        let streams: Vec<bool> = sent.iter().filter(|(p, _)| p.ends_with("/completions")).map(|(_, b)| b.contains("\"stream\"")).collect();
        assert_eq!(streams, [true, false, false]);
    }

    /// Anthropic's events: a thinking block, words, and a tool call whose
    /// input arrives in pieces.
    #[test]
    fn an_anthropic_event_stream_is_put_together() {
        let listener = service(vec![(
            "/v1/messages",
            vec![events(&[
                r#"{"type":"message_start","message":{"id":"m","role":"assistant","content":[]}}"#,
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Find it first."}}"#,
                r#"{"type":"content_block_stop","index":0}"#,
                r#"{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}"#,
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"On "}}"#,
                r#"{"type":"ping"}"#,
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"it."}}"#,
                r#"{"type":"content_block_stop","index":1}"#,
                r#"{"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_1","name":"tequila_nodes","input":{}}}"#,
                r#"{"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"filter\": \"st"}}"#,
                r#"{"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"rap\"}"}}"#,
                r#"{"type":"content_block_stop","index":2}"#,
                r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
                r#"{"type":"message_stop"}"#,
            ])],
        )]);
        let (url, seen) = listener;
        let mut provider = local(url.trim_end_matches("/v1").to_string());
        provider.kind = Kind::Anthropic;
        let mut session = Session::new(provider);
        session.ask("Find the strap").unwrap();
        let turn = wait(&mut session).unwrap();
        assert_eq!(turn.reasoning, "Find it first.");
        assert_eq!(turn.text, ["On it."]);
        assert_eq!((turn.calls[0].id.as_str(), turn.calls[0].name.as_str()), ("toolu_1", "tequila_nodes"));
        assert_eq!(turn.calls[0].arguments.as_ref().unwrap().get("filter").and_then(|f| f.as_str()), Some("strap"));
        assert!(seen.lock().unwrap().iter().any(|(p, b)| p == "/v1/messages" && b.contains("\"stream\"")));
    }

    #[test]
    fn an_error_inside_a_stream_is_reported() {
        let (url, _) = chat(vec![events(&[r#"{"error":{"message":"the model crashed"}}"#])]);
        let mut session = session(url);
        session.ask("hello").unwrap();
        assert!(wait(&mut session).unwrap_err().contains("the model crashed"));
    }

    /// Stop ends a stream at once, mid-way, and the conversation can go on.
    #[test]
    fn stop_mid_stream_ends_it_at_once() {
        let (url, _) = chat(vec![
            paced(vec![(0, r#"{"choices":[{"delta":{"reasoning":"Thinking"}}]}"#), (20_000, r#"{"choices":[{"delta":{"content":"late"}}]}"#)]),
            json(r#"{"choices":[{"message":{"content":"Carrying on."}}]}"#),
        ]);
        let mut session = session(url);
        session.limits = Some(Limits { first: Duration::from_secs(60), silence: Duration::from_secs(60) });
        session.ask("hello").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while session.progress().is_none_or(|p| p.phase != Phase::Thinking) {
            assert!(Instant::now() < deadline, "the reasoning never showed");
            std::thread::sleep(Duration::from_millis(10));
        }
        let hangup = session.pending.as_ref().unwrap().job.hangup.clone();
        let stopped = Instant::now();
        session.cancel();
        assert!(!session.busy(), "stopped at once");
        assert!(hangup.is_stopped());
        assert!(stopped.elapsed() < Duration::from_millis(100));
        session.ask("go on").unwrap();
        assert_eq!(wait(&mut session).unwrap().text, ["Carrying on."]);
    }

    #[test]
    fn a_stopped_request_leaves_a_conversation_that_can_continue() {
        let (url, _) = chat(vec![json(r#"{"choices":[{"message":{"tool_calls":[{"id":"c1","function":{"name":"tequila_scene","arguments":"{}"}}]}}]}"#)]);
        let mut session = session(url);
        session.ask("Look").unwrap();
        wait(&mut session).unwrap();
        session.cancel();
        let last = session.messages.last().unwrap();
        assert_eq!(last.role, Role::User);
        assert!(matches!(&last.parts[0], Part::Result { is_error: true, .. }), "the unrun call is answered");
    }

    #[test]
    fn nothing_listening_is_explained() {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let mut session = Session::new(local(format!("http://127.0.0.1:{port}/v1")));
        session.ask("hello").unwrap();
        let error = wait(&mut session).unwrap_err();
        assert!(error.contains("127.0.0.1"), "{error}");
    }

    #[test]
    fn a_provider_that_is_not_set_up_is_refused_before_sending() {
        let mut provider = local("http://127.0.0.1:1/v1".into());
        provider.model.clear();
        assert!(Session::new(provider).ask("hi").unwrap_err().contains("model"));
    }

    // ---- Ollama -------------------------------------------------------------------

    const VERSION: &str = r#"{"version":"0.40.13"}"#;
    const SHOW: &str = r#"{"capabilities":["completion","tools","thinking"],"model_info":{"general.architecture":"qwen3","qwen3.context_length":262144},"parameters":"temperature 0.6"}"#;

    fn ps(context: u32) -> String {
        format!(r#"{{"models":[{{"name":"test:latest","model":"test:latest","size":3180247121,"size_vram":3180247121,"context_length":{context}}}]}}"#)
    }

    fn ollama(chat_replies: Vec<Canned>, show: &str, ps_replies: Vec<String>) -> (String, Seen) {
        service(vec![
            ("/api/version", vec![json(VERSION); 4]),
            ("/api/show", vec![json(show); 4]),
            ("/api/ps", ps_replies.iter().map(|p| json(p)).collect()),
            ("/api/generate", vec![json(r#"{"model":"test","done":true,"done_reason":"load"}"#)]),
            ("/v1/chat/completions", chat_replies),
        ])
    }

    /// Ollama's own API says the model gets 4,096 tokens: the assistant's
    /// request, with its tools, is longer, so it is not sent, and the
    /// person is told what to set.
    #[test]
    fn a_request_too_long_for_ollamas_context_is_not_sent() {
        let (url, seen) = ollama(vec![], SHOW, vec![ps(4096)]);
        let mut session = session(url);
        session.ask("Make the strap velvet").unwrap();
        let error = wait(&mut session).unwrap_err();
        assert!(error.contains("room for 4,096"), "{error}");
        assert!(error.contains("OLLAMA_CONTEXT_LENGTH="), "{error}");
        assert!(!seen.lock().unwrap().iter().any(|(p, _)| p.contains("/chat/")), "nothing was sent to the model");
    }

    /// With room enough, it goes; a model not yet loaded is loaded first,
    /// as Loading, and only then is its context read.
    #[test]
    fn a_request_that_fits_goes_after_the_model_is_loaded() {
        let (url, seen) = ollama(
            vec![events(&[r#"{"choices":[{"delta":{"content":"Done."},"finish_reason":"stop"}]}"#])],
            SHOW,
            vec![r#"{"models":[]}"#.into(), ps(32_768)],
        );
        let mut session = session(url);
        session.ask("Make the strap velvet").unwrap();
        assert_eq!(wait(&mut session).unwrap().text, ["Done."]);
        let paths: Vec<String> = seen.lock().unwrap().iter().map(|(p, _)| p.clone()).collect();
        assert_eq!(paths, ["/api/version", "/api/show", "/api/ps", "/api/generate", "/api/ps", "/v1/chat/completions"]);
    }

    #[test]
    fn a_model_without_tools_is_named_before_sending() {
        let show = r#"{"capabilities":["completion","vision"],"model_info":{}}"#;
        let (url, seen) = ollama(vec![], show, vec![ps(32_768)]);
        let mut session = session(url);
        session.ask("Make the strap velvet").unwrap();
        let error = wait(&mut session).unwrap_err();
        assert!(error.contains("can't use tools"), "{error}");
        assert!(!seen.lock().unwrap().iter().any(|(p, _)| p.contains("/chat/")));
    }

    /// A model Ollama says cannot see is sent no pictures, and the person
    /// is told once; a model half on the processor is noted too.
    #[test]
    fn a_blind_or_split_model_is_noted_once() {
        let half = r#"{"models":[{"name":"test:latest","size":1000,"size_vram":500,"context_length":32768}]}"#.to_string();
        let (url, seen) = ollama(
            vec![
                events(&[r#"{"choices":[{"delta":{"content":"One."},"finish_reason":"stop"}]}"#]),
                events(&[r#"{"choices":[{"delta":{"content":"Two."},"finish_reason":"stop"}]}"#]),
            ],
            SHOW,
            vec![half.clone(), half],
        );
        let mut session = session(url);
        session.messages.push(Message { role: Role::User, parts: vec![Part::Image("UE5H".into())] });
        session.messages.push(Message { role: Role::Assistant, parts: vec![Part::Text("Seen.".into())] });
        session.ask("one").unwrap();
        wait(&mut session).unwrap();
        let notes = session.take_notes();
        assert!(notes.iter().any(|n| n.contains("can't see images")), "{notes:?}");
        assert!(notes.iter().any(|n| n.contains("50%")), "{notes:?}");
        session.ask("two").unwrap();
        wait(&mut session).unwrap();
        assert!(session.take_notes().is_empty(), "said once");
        assert!(seen.lock().unwrap().iter().filter(|(p, _)| p.contains("/chat/")).all(|(_, b)| !b.contains("image_url")));
    }

    #[test]
    fn a_missing_model_is_named_with_how_to_get_it() {
        let (url, _) = service(vec![("/api/version", vec![json(VERSION)]), ("/api/show", vec![failing(404, r#"{"error":"model 'test' not found"}"#)])]);
        let mut session = session(url);
        session.ask("hello").unwrap();
        let error = wait(&mut session).unwrap_err();
        assert!(error.contains("ollama pull test"), "{error}");
    }

    /// A local server that is not Ollama answers `/api/version` with 404,
    /// and is talked to as any other.
    #[test]
    fn a_local_server_that_is_not_ollama_is_left_alone() {
        let (url, seen) = chat(vec![events(&[r#"{"choices":[{"delta":{"content":"Hi."},"finish_reason":"stop"}]}"#])]);
        let mut session = session(url);
        session.ask("hello").unwrap();
        assert_eq!(wait(&mut session).unwrap().text, ["Hi."]);
        let paths: Vec<String> = seen.lock().unwrap().iter().map(|(p, _)| p.clone()).collect();
        assert_eq!(paths, ["/api/version", "/v1/chat/completions"]);
    }

    /// The real request — the instructions, every tool, one question — is
    /// several thousand tokens: more than Ollama's 4096 default, which is
    /// what the check is for, and within 16k. (Ollama 0.40.13 counted
    /// 4,843 for qwen3 on November 6; the estimate is 5,888.)
    #[test]
    fn the_assistants_first_request_needs_more_than_ollamas_default() {
        let body = wire::request_body(&local("http://localhost:11434/v1".into()), tools::system(), &tools::all(), &[Message::user("Make the strap velvet")]);
        let tokens = ollama::estimate_tokens(&body);
        assert!((4096..14_000).contains(&tokens), "{tokens}");
    }
}
