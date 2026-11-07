//! The one place a request leaves the machine.
//!
//! Two kinds of request. Short ones — listing models, a one-line test,
//! asking a local server what it has — wait a fixed time. A conversation
//! turn is streamed instead: it may take as long as the model keeps
//! producing, and is cut off only when the server goes quiet, so a slow
//! model on a modest computer still finishes while a stuck one is noticed.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ureq::unversioned::resolver::DefaultResolver;
use ureq::unversioned::transport::{Buffers, ConnectProxyConnector, ConnectionDetails, Connector, Either, LazyBuffers, NextTimeout, RustlsConnector, Transport};

/// How long a short request may take.
const TIMEOUT: Duration = Duration::from_secs(300);

/// How long making a connection may take. A server that is there answers
/// in moments; one that is not should be reported, not waited for.
const CONNECT: Duration = Duration::from_secs(30);

/// Replies carry no images back, but a verbose one can still be large.
const LIMIT: u64 = 32 * 1024 * 1024;

/// How often a wait looks at the clock and at Stop.
const TICK: Duration = Duration::from_millis(50);

/// What a request that was stopped ends with.
pub const STOPPED: &str = "stopped";

/// How long a streamed reply may keep the person waiting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// From sending the request to the first of the reply. A server reads
    /// the whole request — instructions, tools, the conversation so far —
    /// before it writes a word, and a model on this computer can take
    /// minutes over a long one.
    pub first: Duration,
    /// Between one piece of the reply and the next, once it has begun. A
    /// server that streams sends something every moment it is producing;
    /// some hold back a tool call until it is whole.
    pub silence: Duration,
}

impl Limits {
    /// Generous for a server on this computer, which may be reading on the
    /// processor; tighter for a service, which answers quickly or not at all.
    pub fn for_url(url: &str) -> Limits {
        if crate::provider::is_local(url) {
            Limits { first: Duration::from_secs(20 * 60), silence: Duration::from_secs(5 * 60) }
        } else {
            Limits { first: Duration::from_secs(5 * 60), silence: Duration::from_secs(3 * 60) }
        }
    }

    /// For a reply that arrives all at once, at the end: its first byte is
    /// its last, so the wait for it is the whole answer's.
    pub fn whole(self) -> Limits {
        Limits { first: self.first + self.silence * 2, silence: self.silence }
    }
}

/// Ends a request from another thread, at once — Stop. The request's
/// connection is closed, so a read waiting on it returns now, and the server
/// sees the reader gone and stops producing.
#[derive(Clone, Default)]
pub struct Hangup(Arc<HangupState>);

#[derive(Default)]
struct HangupState {
    stopped: AtomicBool,
    /// The connections opened for the request, to close.
    sockets: Mutex<Vec<TcpStream>>,
}

impl std::fmt::Debug for Hangup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hangup").field("stopped", &self.is_stopped()).finish()
    }
}

impl Hangup {
    pub fn new() -> Hangup {
        Hangup::default()
    }

    /// Stop the request.
    pub fn hang_up(&self) {
        self.0.stopped.store(true, Ordering::SeqCst);
        self.close();
    }

    pub fn is_stopped(&self) -> bool {
        self.0.stopped.load(Ordering::SeqCst)
    }

    /// Close every connection opened so far, without calling it a stop: a
    /// server gone silent for too long.
    fn close(&self) {
        let sockets = std::mem::take(&mut *self.0.sockets.lock().unwrap_or_else(|e| e.into_inner()));
        for socket in sockets {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }

    /// Let go of the connections of a request that has finished, without
    /// closing them under anyone.
    fn release(&self) {
        self.0.sockets.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    fn keep(&self, stream: &TcpStream) {
        if let Ok(copy) = stream.try_clone() {
            if self.is_stopped() {
                let _ = copy.shutdown(Shutdown::Both);
            } else {
                self.0.sockets.lock().unwrap_or_else(|e| e.into_inner()).push(copy);
            }
        }
    }
}

/// Makes the connection itself, as ureq's own would, and keeps a handle on
/// it for [`Hangup`]. Through a proxy the proxy's connector has made it
/// already, and Stop takes effect at the reply's next piece instead.
#[derive(Debug)]
struct Tcp(Hangup);

impl<In: Transport> Connector<In> for Tcp {
    type Out = Either<In, Socket>;

    fn connect(&self, details: &ConnectionDetails, chained: Option<In>) -> Result<Option<Self::Out>, ureq::Error> {
        if let Some(transport) = chained {
            return Ok(Some(Either::A(transport)));
        }
        let timeout = details.timeout.not_zero().map(|d| *d).unwrap_or(CONNECT).min(CONNECT);
        let mut failure = None;
        for address in details.addrs.iter() {
            match TcpStream::connect_timeout(address, timeout) {
                Ok(stream) => {
                    let _ = stream.set_nodelay(details.config.no_delay());
                    self.0.keep(&stream);
                    let buffers = LazyBuffers::new(details.config.input_buffer_size(), details.config.output_buffer_size());
                    return Ok(Some(Either::B(Socket { stream, buffers, read: None, write: None })));
                }
                Err(error) => failure = Some(error),
            }
        }
        Err(failure.map(ureq::Error::Io).unwrap_or(ureq::Error::ConnectionFailed))
    }
}

/// A connection, carried as ureq's own TCP transport carries one.
struct Socket {
    stream: TcpStream,
    buffers: LazyBuffers,
    /// The time limits set on the socket, to set them only when they change.
    read: Option<Duration>,
    write: Option<Duration>,
}

impl std::fmt::Debug for Socket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Socket").field("peer", &self.stream.peer_addr().ok()).finish()
    }
}

/// An I/O failure as ureq reports one: a time limit reached is a timeout.
fn io_failure(error: std::io::Error, timeout: &NextTimeout) -> ureq::Error {
    match error.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => ureq::Error::Timeout(timeout.reason),
        _ => ureq::Error::Io(error),
    }
}

impl Transport for Socket {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        let limit = timeout.not_zero().map(|d| *d);
        if limit != self.write {
            self.stream.set_write_timeout(limit)?;
            self.write = limit;
        }
        let output = &self.buffers.output()[..amount];
        self.stream.write_all(output).map_err(|e| io_failure(e, &timeout))
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        let limit = timeout.not_zero().map(|d| *d);
        if limit != self.read {
            self.stream.set_read_timeout(limit)?;
            self.read = limit;
        }
        let input = self.buffers.input_append_buf();
        let amount = self.stream.read(input).map_err(|e| io_failure(e, &timeout))?;
        self.buffers.input_appended(amount);
        Ok(amount > 0)
    }

    /// Whether a kept connection can be used again: nothing waiting on it,
    /// and not closed by the other end.
    fn is_open(&mut self) -> bool {
        if self.stream.set_nonblocking(true).is_err() {
            return false;
        }
        let open = matches!(self.stream.read(&mut [0u8; 1]), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock);
        self.stream.set_nonblocking(false).is_ok() && open
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        // An error status still has a body, and the body is the only place
        // the service says *why* — a wrong key, an unknown model, a quota.
        .http_status_as_error(false)
        .build()
        .into()
}

/// An agent whose connections `hangup` can close, and with no overall time
/// limit: the caller watches the reply instead.
fn streaming_agent(hangup: &Hangup) -> ureq::Agent {
    let config = ureq::Agent::config_builder().timeout_connect(Some(CONNECT)).http_status_as_error(false).build();
    let connector = ().chain(ConnectProxyConnector::default()).chain(Tcp(hangup.clone())).chain(RustlsConnector::default());
    ureq::Agent::with_parts(config, connector, DefaultResolver::default())
}

/// A reply: its status and its body.
pub struct Response {
    pub status: u16,
    pub body: String,
}

fn finish(result: Result<ureq::http::Response<ureq::Body>, ureq::Error>, url: &str) -> Result<Response, String> {
    let mut response = result.map_err(|e| describe(e, url))?;
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .with_config()
        .limit(LIMIT)
        .read_to_string()
        .map_err(|e| format!("the reply from {url} could not be read: {e}"))?;
    Ok(Response { status, body })
}

pub fn post(url: &str, headers: &[(String, String)], body: String) -> Result<Response, String> {
    let mut request = agent().post(url);
    for (name, value) in headers {
        request = request.header(name.as_str(), value.as_str());
    }
    finish(request.send(body), url)
}

pub fn get(url: &str, headers: &[(String, String)]) -> Result<Response, String> {
    get_within(url, headers, TIMEOUT)
}

/// A GET that may take no longer than `timeout`: a question a server
/// answers at once if it is the kind of server that knows the answer.
pub fn get_within(url: &str, headers: &[(String, String)], timeout: Duration) -> Result<Response, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(timeout)).http_status_as_error(false).build().into();
    let mut request = agent.get(url);
    for (name, value) in headers {
        // A GET has no body to describe.
        if name != "content-type" {
            request = request.header(name.as_str(), value.as_str());
        }
    }
    finish(request.call(), url)
}

/// A piece of a reply, as it arrives.
pub enum Piece<'a> {
    /// The status and whether the body is a stream of events
    /// (`text/event-stream`), before any of the body.
    Head { status: u16, events: bool },
    Body(&'a [u8]),
}

/// Why a streamed request ended early, in words; [`STOPPED`] for Stop.
fn too_slow(url: &str, limits: Limits, begun: bool) -> String {
    if begun {
        format!("{url} stopped sending part-way through its answer: nothing came for {}", duration(limits.silence))
    } else {
        format!("{url} had not started to answer after {}", duration(limits.first))
    }
}

/// `5 minutes`, `90 seconds`.
pub fn duration(d: Duration) -> String {
    let seconds = d.as_secs();
    if seconds >= 120 && seconds % 60 == 0 {
        format!("{} minutes", seconds / 60)
    } else if seconds == 60 {
        "a minute".into()
    } else {
        format!("{seconds} seconds")
    }
}

/// Post, and hand the reply on to `on` as it arrives, for as long as it
/// keeps arriving: `limits.first` for it to begin, then `limits.silence`
/// between pieces. `hangup` stops it at once. An error from `on` ends the
/// request with that error.
pub fn post_streaming(
    url: &str,
    headers: &[(String, String)],
    body: String,
    limits: Limits,
    hangup: &Hangup,
    mut on: impl FnMut(Piece) -> Result<(), String>,
) -> Result<(), String> {
    if hangup.is_stopped() {
        return Err(STOPPED.into());
    }
    enum Arrived {
        Head { status: u16, events: bool },
        Body(Vec<u8>),
        End,
        Failed(String),
    }
    let (sender, receiver) = channel();
    let agent = streaming_agent(hangup);
    let mut request = agent.post(url);
    for (name, value) in headers {
        request = request.header(name.as_str(), value.as_str());
    }
    // Compressed, a stream is held back until a compressor's block fills.
    request = request.header("accept-encoding", "identity");
    let address = url.to_string();
    // The waiting happens here, the reading on a thread of its own, so the
    // clock and Stop are looked at even while the server says nothing.
    let spawned = std::thread::Builder::new().name("assistant-reply".into()).spawn(move || {
        let response = match request.send(body) {
            Ok(response) => response,
            Err(error) => {
                let _ = sender.send(Arrived::Failed(describe(error, &address)));
                return;
            }
        };
        let status = response.status().as_u16();
        let events = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|t| t.to_ascii_lowercase().contains("text/event-stream"));
        if sender.send(Arrived::Head { status, events }).is_err() {
            return;
        }
        let mut reader = response.into_body().into_with_config().limit(LIMIT).reader();
        let mut buffer = vec![0u8; 16 * 1024];
        loop {
            let message = match reader.read(&mut buffer) {
                Ok(0) => Arrived::End,
                Ok(n) => Arrived::Body(buffer[..n].to_vec()),
                Err(error) => Arrived::Failed(format!("the reply from {address} broke off: {error}")),
            };
            let last = !matches!(message, Arrived::Body(_));
            if sender.send(message).is_err() || last {
                return;
            }
        }
    });
    if let Err(error) = spawned {
        return Err(format!("the request could not be started: {error}"));
    }

    let start = Instant::now();
    let mut last = start;
    let mut begun = false;
    let outcome = loop {
        if hangup.is_stopped() {
            break Err(STOPPED.to_string());
        }
        match receiver.recv_timeout(TICK) {
            Ok(Arrived::Head { status, events }) => {
                if let Err(error) = on(Piece::Head { status, events }) {
                    break Err(error);
                }
            }
            Ok(Arrived::Body(bytes)) => {
                begun = true;
                last = Instant::now();
                if let Err(error) = on(Piece::Body(&bytes)) {
                    break Err(error);
                }
            }
            // A socket closed by Stop reads as an error on Windows and Linux,
            // but as the end of the reply on a Mac: the flag, set before the
            // close, tells the two apart.
            Ok(Arrived::End) => break if hangup.is_stopped() { Err(STOPPED.into()) } else { Ok(()) },
            Ok(Arrived::Failed(error)) => break Err(if hangup.is_stopped() { STOPPED.into() } else { error }),
            Err(RecvTimeoutError::Timeout) => {
                let (since, limit) = if begun { (last, limits.silence) } else { (start, limits.first) };
                if since.elapsed() > limit {
                    break Err(too_slow(url, limits, begun));
                }
            }
            Err(RecvTimeoutError::Disconnected) if hangup.is_stopped() => break Err(STOPPED.into()),
            Err(RecvTimeoutError::Disconnected) => break Err(format!("the reply from {url} ended without finishing")),
        }
    };
    // Finished or not, nothing more is read: close what is still open, so a
    // server still producing stops, and a reader still waiting returns.
    if outcome.is_err() {
        hangup.close();
    } else {
        hangup.release();
    }
    outcome
}

/// Post and wait for the whole reply, as [`post_streaming`] waits: as long
/// as it keeps coming, and no longer than Stop.
pub fn post_watched(url: &str, headers: &[(String, String)], body: String, limits: Limits, hangup: &Hangup) -> Result<Response, String> {
    let mut status = 0;
    let mut bytes = Vec::new();
    post_streaming(url, headers, body, limits, hangup, |piece| {
        match piece {
            Piece::Head { status: s, .. } => status = s,
            Piece::Body(data) => bytes.extend_from_slice(data),
        }
        Ok(())
    })?;
    Ok(Response { status, body: String::from_utf8_lossy(&bytes).into_owned() })
}

/// A transport failure in words a person can act on.
fn describe(error: ureq::Error, url: &str) -> String {
    let text = error.to_string();
    let lower = text.to_lowercase();
    if lower.contains("connection refused") || lower.contains("actively refused") {
        format!("nothing answered at {url} — is the server running?")
    } else if lower.contains("dns") || lower.contains("resolve") || lower.contains("host") && lower.contains("not") {
        format!("{url} could not be found — check the address and the internet connection ({text})")
    } else if lower.contains("timeout") || lower.contains("timed out") {
        format!("{url} took too long to answer")
    } else if lower.contains("certificate") || lower.contains("tls") {
        format!("a secure connection to {url} could not be made ({text})")
    } else {
        format!("{url}: {text}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;

    /// A server that sends its reply in timed pieces: each a pause, then
    /// bytes. Takes one request.
    fn paced(pieces: Vec<(u64, &'static str)>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}/", listener.local_addr().unwrap().port());
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut seen = Vec::new();
            let mut byte = [0u8; 1];
            while !seen.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                seen.push(byte[0]);
            }
            for (pause, text) in pieces {
                std::thread::sleep(Duration::from_millis(pause));
                if stream.write_all(text.as_bytes()).is_err() {
                    return;
                }
                let _ = stream.flush();
            }
        });
        url
    }

    const HEAD: &str = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n";

    /// Short enough for a test, long enough for a busy machine's threads.
    fn quick() -> Limits {
        Limits { first: Duration::from_secs(3), silence: Duration::from_secs(1) }
    }

    fn collect(url: &str, limits: Limits, hangup: &Hangup) -> Result<String, String> {
        let mut text = String::new();
        post_streaming(url, &[], "{}".into(), limits, hangup, |piece| {
            if let Piece::Body(bytes) = piece {
                text.push_str(std::str::from_utf8(bytes).unwrap());
            }
            Ok(())
        })
        .map(|_| text)
    }

    /// A slow start is no failure: the server is reading the request.
    #[test]
    fn a_slow_first_byte_is_waited_for() {
        let url = paced(vec![(1500, HEAD), (0, "data: a\n\n"), (100, "data: b\n\n")]);
        assert_eq!(collect(&url, quick(), &Hangup::new()).unwrap(), "data: a\n\ndata: b\n\n");
    }

    /// A reply may take far longer than either limit, as long as it keeps
    /// coming.
    #[test]
    fn a_long_reply_that_keeps_coming_is_never_cut_off() {
        let pieces: Vec<(u64, &str)> = std::iter::once((0, HEAD)).chain((0..16).map(|_| (250, "x"))).collect();
        let url = paced(pieces);
        let started = Instant::now();
        assert_eq!(collect(&url, quick(), &Hangup::new()).unwrap().len(), 16);
        assert!(started.elapsed() > quick().first, "it ran past the first limit and still finished");
    }

    #[test]
    fn silence_part_way_is_an_error_that_says_so() {
        let url = paced(vec![(0, HEAD), (0, "data: a\n\n"), (6000, "data: late\n\n")]);
        let started = Instant::now();
        let error = collect(&url, quick(), &Hangup::new()).unwrap_err();
        assert!(error.contains("stopped sending part-way"), "{error}");
        assert!(started.elapsed() < Duration::from_millis(4000), "it gave up as soon as the silence ran out");
    }

    #[test]
    fn no_start_at_all_is_an_error_that_says_so() {
        let url = paced(vec![(6000, HEAD)]);
        let error = collect(&url, quick(), &Hangup::new()).unwrap_err();
        assert!(error.contains("had not started to answer"), "{error}");
    }

    /// Stop ends a request at once, even while the server says nothing, and
    /// closes the connection under it.
    #[test]
    fn a_hangup_ends_a_silent_request_at_once() {
        let url = paced(vec![(0, HEAD), (0, "data: a\n\n"), (10_000, "data: late\n\n")]);
        let hangup = Hangup::new();
        let stopper = hangup.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            stopper.hang_up();
        });
        let started = Instant::now();
        let limits = Limits { first: Duration::from_secs(60), silence: Duration::from_secs(60) };
        assert_eq!(collect(&url, limits, &hangup).unwrap_err(), STOPPED);
        assert!(started.elapsed() < Duration::from_millis(3000), "{:?}", started.elapsed());
    }

    #[test]
    fn durations_read_as_people_say_them() {
        assert_eq!(duration(Duration::from_secs(300)), "5 minutes");
        assert_eq!(duration(Duration::from_secs(60)), "a minute");
        assert_eq!(duration(Duration::from_secs(90)), "90 seconds");
    }
}
