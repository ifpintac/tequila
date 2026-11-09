//! The listener, and how a client finds it.
//!
//! # Why a TCP socket on the loopback
//!
//! Because it is the one transport that is the same on every platform this
//! ships to. Named pipes and Unix domain sockets are each nicer on their own
//! system and different enough from each other to need two implementations
//! and two sets of bugs. Bound to `127.0.0.1`, a loopback socket is not
//! reachable from another machine, which is the boundary that matters.
//!
//! # What the token is and is not
//!
//! It is **not** a security boundary. The file it is written to is readable by
//! anything running as this user, so any process that could abuse the channel
//! could also read the token first. What it does is make connecting
//! *deliberate*: it stops a stray client that happened to guess the port from
//! driving someone's scene, and it identifies which instance a client meant.
//! The real boundary is the loopback bind.
//!
//! # Why the app answers, not the socket thread
//!
//! The document has exactly one writer — the command bus — and the bus lives
//! on the UI thread with everything else that reads the document. A request
//! is therefore parked in a queue and answered on the next frame, in arrival
//! order, which also means a scripted change lands at a frame boundary
//! exactly as a click does. A connection thread blocks on its own reply, so
//! each client sees its requests complete in order without any of them
//! needing to know about the others.
//!
//! # Operations of a session's own
//!
//! A channel can be opened with operations beyond the protocol's
//! ([`Channel::start_at_with`]): a request whose `op` is one of them is
//! queued as it came, and answered by the handler [`Channel::drain_with`]
//! is given, so an end that has more to offer - `uv_layout`, which the
//! application and the headless server answer - can offer it without every
//! end having to.

use crate::protocol::{encode_reply, parse_request, request_id, ProtocolError, Reply, Request};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use tequila_format::json::Value;

/// One request waiting for the UI thread, and where its answer goes.
struct Pending {
    request: Incoming,
    reply: Sender<String>,
}

/// What arrived: a request of the protocol's, or one of the session's own
/// operations, as it came.
enum Incoming {
    Request(Request),
    Own { id: u64, value: Value },
}

impl Incoming {
    fn id(&self) -> u64 {
        match self {
            Incoming::Request(request) => request.id,
            Incoming::Own { id, .. } => *id,
        }
    }
}

/// The channel's end of a running application.
pub struct Channel {
    pending: Arc<Mutex<VecDeque<Pending>>>,
    /// Called when a request is queued: the application's way to be woken.
    waker: Waker,
    address: SocketAddr,
    token: String,
    /// The discovery file this channel announced itself in, until it retires.
    announced: Option<std::path::PathBuf>,
}

/// Where a client looks for a running instance.
///
/// One file rather than one per process: the common case is a single open
/// application, and a client that wants to be careful can check the `pid`.
/// The last instance to start wins the file, which is also the instance a
/// person most likely just opened. When that instance closes, the file goes
/// back to the newest one still running (see [`Channel::retire`]), so a
/// second window opened and closed does not leave every client pointed at a
/// process that no longer exists.
///
/// `~/.tequila/channel.json`, or `data/channel.json` in a portable copy,
/// where `tequila-cli --mcp` and the Python module beside it look too.
pub fn discovery_path() -> std::path::PathBuf {
    if let Some(path) = std::env::var_os("TEQUILA_CHANNEL_FILE") { return path.into(); }
    tequila_format::library::home_directory().join("channel.json")
}

/// Every running instance's own discovery record, beside the shared file:
/// what the shared file is handed back from, and what a client can look
/// through when the instance the shared file names has crashed.
pub fn instances_dir(discovery: &std::path::Path) -> std::path::PathBuf {
    discovery.with_file_name("channels")
}

/// A token that is unpredictable enough to have to be read rather than
/// guessed.
///
/// Deliberately not presented as a secret — see this module's header. There is
/// no cryptographic source of randomness in this dependency set, and adding
/// one to produce a value whose file is world-readable by the same user would
/// be security theatre.
fn token() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mixed = (now as u64) ^ ((std::process::id() as u64) << 32);
    let mut rng = tequila_core::rng::Pcg32::new(mixed, 0x9E37_79B9_7F4A_7C15);
    (0..4).fold(String::new(), |mut out, _| {
        out.push_str(&format!("{:08x}", rng.next_u32()));
        out
    })
}

impl Channel {
    /// Open the channel, or explain why not.
    ///
    /// Failure is not fatal to the application: a renderer that will not start
    /// because a port was busy would be a worse tool than one that is simply
    /// not scriptable this session.
    pub fn start() -> Result<Channel, String> {
        Channel::start_with(&[])
    }

    /// As [`Channel::start`], answering the operations `own` names as well
    /// as the protocol's: the application's `uv_layout`.
    pub fn start_with(own: &'static [&'static str]) -> Result<Channel, String> {
        let mut channel = Channel::open(own)?;
        channel.announce(&discovery_path());
        Ok(channel)
    }

    /// Open the channel and announce it at `discovery` alone: a headless
    /// session a script started for itself (`tequila --serve`), which must
    /// not take the shared file from the application a person has open.
    pub fn start_at(discovery: &std::path::Path) -> Result<Channel, String> {
        Channel::start_at_with(discovery, &[])
    }

    /// As [`Channel::start_at`], answering the operations `own` names as
    /// well as the protocol's: see the module's documentation.
    pub fn start_at_with(discovery: &std::path::Path, own: &'static [&'static str]) -> Result<Channel, String> {
        let mut channel = Channel::open(own)?;
        channel.announce(discovery);
        Ok(channel)
    }

    /// Open the channel without telling anyone where it is.
    ///
    /// For tests. [`Channel::start`] writes the discovery file in the user's
    /// home, and a test that did the same pointed every client on the machine
    /// at a process that exited a second later — running the test suite
    /// quietly disconnected scripts and assistants from the Tequila that was
    /// actually open.
    fn open(own: &'static [&'static str]) -> Result<Channel, String> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|e| format!("the scripting channel could not be opened: {e}"))?;
        let address = listener
            .local_addr()
            .map_err(|e| format!("the scripting channel has no address: {e}"))?;
        let token = token();

        let pending: Arc<Mutex<VecDeque<Pending>>> = Arc::new(Mutex::new(VecDeque::new()));
        let queue = Arc::clone(&pending);
        let waker: Waker = Arc::new(Mutex::new(None));
        let wake = Arc::clone(&waker);
        let expected = token.clone();
        std::thread::Builder::new()
            .name("tequila-channel".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { continue };
                    let queue = Arc::clone(&queue);
                    let wake = Arc::clone(&wake);
                    let expected = expected.clone();
                    // A thread per connection. Clients here are scripts and
                    // test harnesses rather than a crowd, and a thread that
                    // blocks on its own reply is far easier to reason about
                    // than a shared poll loop that must not.
                    let _ = std::thread::Builder::new()
                        .name("tequila-channel-client".into())
                        .spawn(move || serve(stream, queue, wake, expected, own));
                }
            })
            .map_err(|e| format!("the scripting channel could not be started: {e}"))?;

        Ok(Channel {
            pending,
            waker,
            address,
            token,
            announced: None,
        })
    }

    fn record(&self) -> String {
        format!(
            "{{\"port\":{},\"token\":\"{}\",\"pid\":{}}}\n",
            self.address.port(),
            self.token,
            std::process::id()
        )
    }

    /// This instance's own file in [`instances_dir`]. By port, which is
    /// unique among running instances — and among channels in one process,
    /// as in the tests.
    fn own_record(&self, discovery: &std::path::Path) -> std::path::PathBuf {
        instances_dir(discovery).join(format!("{}.json", self.address.port()))
    }

    /// Write the discovery file, so a client does not have to be told the port.
    fn announce(&mut self, path: &std::path::Path) {
        let _ = std::fs::create_dir_all(instances_dir(path));
        let _ = std::fs::write(self.own_record(path), self.record());
        let _ = std::fs::write(path, self.record());
        self.announced = Some(path.to_path_buf());
    }

    /// Stop being findable: remove this instance's record and, if the shared
    /// file names this instance, hand it to the newest instance still
    /// listening, or remove it when there is none.
    ///
    /// Called when the application closes, and on drop. A crash skips it;
    /// the shared file then names a dead port, and a client falls back to
    /// [`instances_dir`] — records of dead instances fail to connect, and are
    /// cleared here the next time any instance retires.
    pub fn retire(&mut self) {
        let Some(path) = self.announced.take() else { return };
        let _ = std::fs::remove_file(self.own_record(&path));
        let ours = std::fs::read_to_string(&path).is_ok_and(|text| text == self.record());
        if !ours {
            return;
        }
        match newest_listening(&instances_dir(&path)) {
            Some(record) => {
                let _ = std::fs::write(&path, record);
            }
            None => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    pub fn port(&self) -> u16 {
        self.address.port()
    }

    /// Answer everything that arrived since the last frame.
    ///
    /// The handler is the application: it is the only thing that can touch the
    /// document, so it is the only thing that can answer. Requests are taken
    /// in arrival order and each reply is sent as it is produced, so a slow
    /// render does not hold up the answer to a query that came after it —
    /// that one simply waits for the next frame.
    /// Call `wake` whenever a request is queued, from the thread it came in
    /// on: an idle application draws its next frame - and answers - at
    /// once, rather than at its next idle repaint, which made a script or a
    /// plugin wait a fifth of a second a request.
    pub fn set_waker(&self, wake: impl Fn() + Send + 'static) {
        if let Ok(mut slot) = self.waker.lock() {
            *slot = Some(Box::new(wake));
        }
    }

    pub fn drain(&self, mut handler: impl FnMut(&Request) -> Reply) {
        self.drain_with(|arrived| match arrived {
            Arrived::Request(request) => handler(request),
            Arrived::Own(value) => {
                let op = value.get("op").and_then(|v| v.as_str()).unwrap_or_default();
                Reply::Failed(format!("`{op}` is not an operation this understands"))
            }
        })
    }

    /// As [`Channel::drain`], the session's own operations among what
    /// arrives, each as the request's JSON.
    pub fn drain_with(&self, mut handler: impl FnMut(Arrived<'_>) -> Reply) {
        self.drain_with_responders(|arrived, responder| responder.answer(&handler(arrived)));
    }

    /// The same, each request handed over with where its answer goes, to be
    /// answered at once or kept and answered on a later frame: a request the
    /// application finishes on another thread, as an import reads its model,
    /// while the window goes on drawing. The client waits as for any answer.
    pub fn drain_with_responders(&self, mut handler: impl FnMut(Arrived<'_>, Responder)) {
        // Yield back to the UI even while clients continuously enqueue work.
        for _ in 0..16 {
            let Some(Pending { request, reply }) = self.pending.lock().ok().and_then(|mut q| q.pop_front()) else {
                return;
            };
            let responder = Responder { id: request.id(), reply };
            match &request {
                Incoming::Request(request) => handler(Arrived::Request(request), responder),
                Incoming::Own { value, .. } => handler(Arrived::Own(value), responder),
            }
        }
    }
}

/// Where one request's answer goes ([`Channel::drain_with_responders`]).
/// Dropped unanswered, it tells the client the application is closing.
pub struct Responder {
    id: u64,
    reply: Sender<String>,
}

impl Responder {
    /// Send the answer to the client that asked.
    pub fn answer(self, reply: &Reply) {
        let _ = self.reply.send(encode_reply(self.id, reply));
    }
}

impl std::fmt::Debug for Responder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Responder").field("id", &self.id).finish_non_exhaustive()
    }
}

/// What [`Channel::drain_with`] hands its handler.
#[derive(Debug)]
pub enum Arrived<'a> {
    /// A request of the protocol's.
    Request(&'a Request),
    /// One of the session's own operations, as its JSON.
    Own(&'a Value),
}

impl Drop for Channel {
    fn drop(&mut self) {
        self.retire();
    }
}

/// The record of the most recently started instance whose port still
/// accepts a connection. Records whose port does not are of instances that
/// crashed, and are removed.
fn newest_listening(dir: &std::path::Path) -> Option<String> {
    let mut records: Vec<(std::time::SystemTime, std::path::PathBuf)> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, entry.path()))
        })
        .collect();
    records.sort_by_key(|a| std::cmp::Reverse(a.0));
    for (_, path) in records {
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let port = tequila_format::json::parse(text.as_bytes())
            .ok()
            .and_then(|value| value.get("port").and_then(|p| p.as_f64()))
            .map(|p| p as u16);
        let listening = port.is_some_and(|port| {
            TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, port)), std::time::Duration::from_millis(300))
                .is_ok()
        });
        if listening {
            return Some(text);
        }
        let _ = std::fs::remove_file(&path);
    }
    None
}

/// What wakes the application when a request arrives.
type Waker = Arc<Mutex<Option<Box<dyn Fn() + Send>>>>;

/// One of the session's own operations, if the line is a request for one:
/// its id, token and JSON.
fn own_operation(line: &str, own: &[&str]) -> Option<(u64, String, Value)> {
    if own.is_empty() {
        return None;
    }
    let value = tequila_format::json::parse(line.as_bytes()).ok()?;
    let op = value.get("op").and_then(|v| v.as_str())?;
    if !own.contains(&op) {
        return None;
    }
    let id = value.get("id").and_then(|v| v.as_index()).unwrap_or(0) as u64;
    let token = value.get("token").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    Some((id, token, value))
}

fn serve(stream: TcpStream, queue: Arc<Mutex<VecDeque<Pending>>>, wake: Waker, expected: String, own: &'static [&'static str]) {
    let peer = stream
        .peer_addr()
        .map(|address| address.to_string())
        .unwrap_or_else(|_| "unknown".into());
    let Ok(write_half) = stream.try_clone() else {
        return;
    };
    let mut out = write_half;
    let reader = BufReader::new(stream);
    let mismatch = || {
        Reply::Failed(
            "that token does not match this instance; read the port and \
             token from ~/.tequila/channel.json (data/channel.json beside \
             a portable copy's program)"
                .into(),
        )
    };

    for line in reader.lines() {
        let Ok(line) = line else { return };
        if line.trim().is_empty() {
            continue;
        }
        let incoming = match parse_request(&line) {
            Ok(request) if request.token != expected => Err((request.id, mismatch())),
            Ok(mut request) => {
                request.peer = peer.clone();
                Ok(Incoming::Request(request))
            }
            Err(ProtocolError(message)) => match own_operation(&line, own) {
                Some((id, token, _)) if token != expected => Err((id, mismatch())),
                Some((id, _, value)) => Ok(Incoming::Own { id, value }),
                None => Err((request_id(&line), Reply::Failed(message))),
            },
        };
        let reply = match incoming {
            Err(refused) => Some(refused),
            Ok(request) => {
                let (sender, receiver) = channel();
                let id = request.id();
                if let Ok(mut queue) = queue.lock() {
                    queue.push_back(Pending {
                        request,
                        reply: sender,
                    });
                } else {
                    return;
                }
                if let Some(wake) = wake.lock().ok().as_ref().and_then(|w| w.as_ref()) {
                    wake();
                }
                // Blocks until the UI thread has answered this one. The
                // application is never waiting on us, so the only way this
                // does not return is the application exiting — which closes
                // the sender and ends the connection, which is correct.
                match receiver.recv() {
                    Ok(encoded) => {
                        if writeln!(out, "{encoded}").is_err() || out.flush().is_err() {
                            return;
                        }
                        None
                    }
                    Err(_) => Some((id, Reply::Failed("the application is closing".into()))),
                }
            }
        };

        if let Some((id, reply)) = reply {
            if writeln!(out, "{}", encode_reply(id, &reply)).is_err() || out.flush().is_err() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_format::json::{self, Value};
    use std::io::Read;

    /// Read one field out of a reply.
    ///
    /// The shared JSON writer puts a space after each colon, so matching on
    /// `"ok":true` as text passes or fails on formatting rather than on
    /// meaning. Parsing is both shorter to read and the thing a real client
    /// would do.
    fn field_of(line: &str, key: &str) -> Value {
        json::parse(line.as_bytes())
            .unwrap_or_else(|e| panic!("a reply must be JSON: {e} in {line:?}"))
            .get(key)
            .cloned()
            .unwrap_or(Value::Null)
    }

    /// Tests never touch the discovery file that real clients read.
    #[test]
    fn a_test_channel_does_not_announce_itself() {
        let before = std::fs::read(discovery_path()).ok();
        let _channel = Channel::open(&[]).expect("opens");
        assert_eq!(std::fs::read(discovery_path()).ok(), before);
    }

    /// Two windows open, the second closes: clients must find the first
    /// again, not a port nobody is listening on.
    #[test]
    fn closing_the_newest_instance_hands_discovery_back() {
        let dir = std::env::temp_dir().join(format!("tequila-discovery-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("channel.json");

        let mut first = Channel::open(&[]).expect("opens");
        first.announce(&path);
        // Modification times order the records; keep them apart.
        std::thread::sleep(std::time::Duration::from_millis(50));
        let mut second = Channel::open(&[]).expect("opens");
        second.announce(&path);
        let port = |text: String| field_of(&text, "port").as_f64().unwrap() as u16;
        assert_eq!(port(std::fs::read_to_string(&path).unwrap()), second.port(), "the newest wins");

        second.retire();
        assert_eq!(
            port(std::fs::read_to_string(&path).expect("still there")),
            first.port(),
            "handed back to the instance still running"
        );

        // The first closing does not touch a file that is no longer its own…
        let mut third = Channel::open(&[]).expect("opens");
        third.announce(&path);
        first.retire();
        assert_eq!(port(std::fs::read_to_string(&path).unwrap()), third.port());
        // …and the last one out removes it.
        third.retire();
        assert!(!path.exists(), "no instance, no file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A request arriving wakes the application, from the connection's own
    /// thread, before the application has drained it: an idle window
    /// answers at once. Without the waker a script waited for the next idle
    /// repaint - a fifth of a second - on every request.
    #[test]
    fn a_request_wakes_the_application() {
        let channel = Channel::open(&[]).expect("opens");
        let (woken, woke) = std::sync::mpsc::channel::<()>();
        let woken = Mutex::new(woken);
        channel.set_waker(move || {
            let _ = woken.lock().map(|w| w.send(()));
        });
        let (port, token) = (channel.port(), channel.token.clone());
        let _client = std::thread::spawn(move || {
            let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).expect("connects");
            writeln!(stream, r#"{{"id":7,"token":"{token}","op":"ping"}}"#).expect("writes");
            stream.flush().expect("flushes");
            let mut line = String::new();
            let _ = BufReader::new(stream).read_line(&mut line);
        });
        woke.recv_timeout(std::time::Duration::from_secs(5)).expect("the request did not wake the application");
        channel.drain(|_| Reply::Done);
    }

    /// An answer kept and sent frames later reaches the client that asked,
    /// which waited for it, while the application went on draining other
    /// clients' requests meanwhile.
    #[test]
    fn an_answer_can_come_frames_later() {
        let channel = Channel::open(&[]).expect("opens");
        let (port, token) = (channel.port(), channel.token.clone());
        let ask = move |id: u64| {
            let token = token.clone();
            std::thread::spawn(move || {
                let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).expect("connects");
                writeln!(stream, r#"{{"id":{id},"token":"{token}","op":"ping"}}"#).expect("writes");
                stream.flush().expect("flushes");
                let mut line = String::new();
                BufReader::new(stream).read_line(&mut line).expect("an answer");
                line
            })
        };
        let slow = ask(1);
        let mut kept = None;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while kept.is_none() {
            assert!(std::time::Instant::now() < deadline, "the request never arrived");
            channel.drain_with_responders(|_, responder| kept = Some(responder));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // Another client is answered meanwhile.
        let quick = ask(2);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !quick.is_finished() {
            assert!(std::time::Instant::now() < deadline, "the second client was not answered");
            channel.drain_with(|_| Reply::Done);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(field_of(&quick.join().unwrap(), "id"), Value::Number(2.0));
        assert!(!slow.is_finished(), "the kept request was answered before its answer was sent");
        kept.unwrap().answer(&Reply::Done);
        let line = slow.join().unwrap();
        assert_eq!(field_of(&line, "id"), Value::Number(1.0), "{line}");
        assert_eq!(field_of(&line, "ok"), Value::Bool(true), "{line}");
    }

    /// The whole loop, over a real socket: connect, send, get answered by
    /// something standing in for the application.
    #[test]
    fn a_request_reaches_the_handler_and_the_answer_comes_back() {
        let channel = Channel::open(&[]).expect("opens");
        let port = channel.port();
        let token = channel.token.clone();

        let client = std::thread::spawn(move || {
            let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).expect("connects");
            writeln!(stream, r#"{{"id":42,"token":"{token}","op":"ping"}}"#).expect("writes");
            stream.flush().expect("flushes");
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).expect("reads");
            line
        });

        // Stand in for the frame loop until the request turns up.
        let mut answered = false;
        for _ in 0..200 {
            channel.drain(|request| {
                assert_eq!(request.id, 42);
                answered = true;
                Reply::Done
            });
            if answered {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(answered, "the request never reached the handler");

        let line = client.join().expect("the client finished");
        assert_eq!(field_of(&line, "ok"), Value::Bool(true), "{line}");
        assert_eq!(field_of(&line, "id"), Value::Number(42.0), "{line}");
    }

    /// An operation of the session's own reaches its handler as the JSON it
    /// came as; the same operation to a channel without it is refused.
    #[test]
    fn a_session_s_own_operation_reaches_its_own_handler() {
        let send = |port: u16, token: String| {
            std::thread::spawn(move || {
                let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).expect("connects");
                writeln!(stream, r#"{{"id":9,"token":"{token}","op":"uv_layout","node":3}}"#).expect("writes");
                stream.flush().expect("flushes");
                let mut line = String::new();
                BufReader::new(stream).read_line(&mut line).expect("reads");
                line
            })
        };
        let channel = Channel::open(&["uv_layout"]).expect("opens");
        let client = send(channel.port(), channel.token.clone());
        let mut seen = None;
        for _ in 0..200 {
            channel.drain_with(|arrived| match arrived {
                Arrived::Own(value) => {
                    seen = value.get("node").and_then(|v| v.as_index());
                    Reply::Done
                }
                Arrived::Request(_) => Reply::Failed("not this one".into()),
            });
            if seen.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(seen, Some(3));
        let line = client.join().expect("the client finished");
        assert_eq!(field_of(&line, "ok"), Value::Bool(true), "{line}");
        assert_eq!(field_of(&line, "id"), Value::Number(9.0), "{line}");

        let plain = Channel::open(&[]).expect("opens");
        let line = send(plain.port(), plain.token.clone()).join().expect("the client finished");
        assert_eq!(field_of(&line, "ok"), Value::Bool(false), "{line}");
    }

    /// A wrong token is refused before the application ever sees it.
    #[test]
    fn a_bad_token_never_reaches_the_document() {
        let channel = Channel::open(&[]).expect("opens");
        let port = channel.port();

        let client = std::thread::spawn(move || {
            let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).expect("connects");
            writeln!(stream, r#"{{"id":9,"token":"wrong","op":"ping"}}"#).expect("writes");
            stream.flush().expect("flushes");
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).expect("reads");
            line
        });

        let mut reached = false;
        for _ in 0..30 {
            channel.drain(|_| {
                reached = true;
                Reply::Done
            });
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!reached, "a request with a bad token was handed to the app");

        let line = client.join().expect("the client finished");
        assert_eq!(field_of(&line, "ok"), Value::Bool(false), "{line}");
        let message = field_of(&line, "error");
        assert!(
            message.as_str().unwrap_or_default().contains("token"),
            "the refusal does not say why: {line}"
        );
    }

    /// A malformed line is answered rather than dropped, because a client
    /// waiting forever for a reply it will never get is the worst failure
    /// this protocol can have.
    #[test]
    fn nonsense_is_answered_not_ignored() {
        let channel = Channel::open(&[]).expect("opens");
        let port = channel.port();

        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).expect("connects");
        writeln!(stream, "this is not json").expect("writes");
        stream.flush().expect("flushes");
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("sets a timeout");

        let mut buffer = [0u8; 512];
        let read = stream.read(&mut buffer).expect("reads");
        let line = String::from_utf8_lossy(&buffer[..read]).to_string();
        assert_eq!(field_of(&line, "ok"), Value::Bool(false), "{line}");
    }

    /// A request that is JSON but asks for something unreadable is refused
    /// under its own id. Answered as id 0, the Python client took it for a
    /// reply to some other request and dropped the connection.
    #[test]
    fn a_refused_request_is_answered_under_its_own_id() {
        let channel = Channel::open(&[]).expect("opens");
        let port = channel.port();

        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).expect("connects");
        writeln!(stream, r#"{{"id":17,"op":"apply","command":{{"command":"no.such_command"}}}}"#).expect("writes");
        stream.flush().expect("flushes");
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).expect("reads");
        assert_eq!(field_of(&line, "ok"), Value::Bool(false), "{line}");
        assert_eq!(field_of(&line, "id"), Value::Number(17.0), "{line}");
    }
}
