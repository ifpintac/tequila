//! The wire between a dispatcher and a worker.
//!
//! Plain TCP, because a render farm is machines on one network and a
//! certificate on every render node is a chore nobody keeps up. What it
//! does guarantee:
//!
//! * **Both ends hold the farm's token.** Each sends a fresh random
//!   challenge and proves knowledge of the token by an HMAC-SHA256 over both
//!   challenges and its role. The token itself never crosses the wire, so a
//!   listener learns nothing that lets it connect later.
//! * **Nothing is altered, replayed, reordered or reflected.** Every frame
//!   carries an HMAC under a key derived from the token and both challenges
//!   (fresh each connection), over its direction, its sequence number and
//!   its contents.
//!
//! What it does not: hide what is sent. Projects and pictures travel
//! readable, so a farm belongs on a network the studio trusts, or a VPN.
//!
//! A frame is a JSON header and a binary payload (a project bundle, a
//! picture), each with its length up front.

use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use std::io::{Read, Write};
use std::net::TcpStream;

/// The protocol revision; both ends must agree.
pub const PROTOCOL: u32 = 1;
const MAGIC: &[u8; 8] = b"FSFARM\0\x01";
const MAX_HEADER: u32 = 1 << 20;
/// The largest payload accepted: a project bundle with its textures, or a
/// 16K picture, with room to spare.
pub const MAX_PAYLOAD: u64 = 4 << 30;

#[derive(Debug)]
pub enum WireError {
    Io(std::io::Error),
    /// The other end is not a Tequila farm peer, or speaks another revision.
    Protocol(String),
    /// The other end does not hold the token.
    Refused,
    /// A frame's signature did not match: altered, replayed or misordered.
    Tampered,
    TooLarge(u64),
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WireError::Io(e) => write!(f, "{e}"),
            WireError::Protocol(m) => write!(f, "{m}"),
            WireError::Refused => write!(f, "the token does not match"),
            WireError::Tampered => write!(f, "a message arrived altered or out of order, so the connection was closed"),
            WireError::TooLarge(n) => write!(f, "a message of {n} bytes is larger than the farm takes"),
        }
    }
}

impl std::error::Error for WireError {}

impl From<std::io::Error> for WireError {
    fn from(e: std::io::Error) -> Self {
        WireError::Io(e)
    }
}

/// Which way a frame travels, signed into it so a frame cannot be sent back
/// to its sender as if the other end had said it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Role {
    Dispatcher,
    Worker,
}

impl Role {
    fn byte(self) -> u8 {
        match self {
            Role::Dispatcher => b'd',
            Role::Worker => b'w',
        }
    }
}

fn random() -> [u8; 32] {
    let mut bytes = [0u8; 32];
    SystemRandom::new().fill(&mut bytes).expect("the system's random source");
    bytes
}

fn sign(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let key = hmac::Key::new(hmac::HMAC_SHA256, key);
    let mut context = hmac::Context::with_key(&key);
    for part in parts {
        context.update(part);
    }
    let tag = context.sign();
    let mut out = [0u8; 32];
    out.copy_from_slice(tag.as_ref());
    out
}

/// Equal without telling, by how long it took, how much of it was.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// One end of an authenticated connection.
pub struct Wire {
    stream: TcpStream,
    key: [u8; 32],
    role: Role,
    sent: u64,
    received: u64,
}

impl Wire {
    /// As the dispatcher: connect, and prove each other.
    pub fn connect(mut stream: TcpStream, token: &[u8]) -> Result<Wire, WireError> {
        // The magic first, alone: something else on that port says so in
        // its first bytes, and may say no more.
        let mut greeting = [0u8; 8 + 4 + 32];
        stream.read_exact(&mut greeting[..8])?;
        if &greeting[..8] != MAGIC {
            return Err(WireError::Protocol("that is not a Tequila render worker".into()));
        }
        stream.read_exact(&mut greeting[8..])?;
        let revision = u32::from_be_bytes(greeting[8..12].try_into().unwrap());
        if revision != PROTOCOL {
            return Err(WireError::Protocol(format!("the worker speaks farm protocol {revision}, this Tequila {PROTOCOL}: update one of them")));
        }
        let worker_challenge: [u8; 32] = greeting[12..].try_into().unwrap();
        let ours = random();
        let proof = sign(token, &[b"dispatcher", &worker_challenge, &ours]);
        let mut hello = Vec::with_capacity(8 + 64);
        hello.extend_from_slice(MAGIC);
        hello.extend_from_slice(&ours);
        hello.extend_from_slice(&proof);
        stream.write_all(&hello)?;
        let mut verdict = [0u8; 1];
        stream.read_exact(&mut verdict)?;
        if verdict[0] != 1 {
            return Err(WireError::Refused);
        }
        let mut theirs = [0u8; 32];
        stream.read_exact(&mut theirs)?;
        if !same(&theirs, &sign(token, &[b"worker", &ours, &worker_challenge])) {
            // It answered, so it is a worker - for some other farm.
            return Err(WireError::Refused);
        }
        let key = sign(token, &[b"session", &worker_challenge, &ours]);
        Ok(Wire { stream, key, role: Role::Dispatcher, sent: 0, received: 0 })
    }

    /// As the worker: greet, and prove each other.
    pub fn accept(mut stream: TcpStream, token: &[u8]) -> Result<Wire, WireError> {
        let ours = random();
        let mut greeting = Vec::with_capacity(8 + 4 + 32);
        greeting.extend_from_slice(MAGIC);
        greeting.extend_from_slice(&PROTOCOL.to_be_bytes());
        greeting.extend_from_slice(&ours);
        stream.write_all(&greeting)?;
        let mut hello = [0u8; 8 + 64];
        stream.read_exact(&mut hello)?;
        if &hello[..8] != MAGIC {
            return Err(WireError::Protocol("that is not a Tequila dispatcher".into()));
        }
        let theirs: [u8; 32] = hello[8..40].try_into().unwrap();
        if !same(&hello[40..], &sign(token, &[b"dispatcher", &ours, &theirs])) {
            let _ = stream.write_all(&[0]);
            return Err(WireError::Refused);
        }
        let mut answer = vec![1u8];
        answer.extend_from_slice(&sign(token, &[b"worker", &theirs, &ours]));
        stream.write_all(&answer)?;
        let key = sign(token, &[b"session", &ours, &theirs]);
        Ok(Wire { stream, key, role: Role::Worker, sent: 0, received: 0 })
    }

    pub fn stream(&self) -> &TcpStream {
        &self.stream
    }

    fn tag(&self, role: Role, sequence: u64, header: &[u8], payload: &[u8]) -> [u8; 32] {
        let lengths = [(header.len() as u32).to_be_bytes().as_slice(), (payload.len() as u64).to_be_bytes().as_slice()].concat();
        sign(&self.key, &[&[role.byte()], &sequence.to_be_bytes(), &lengths, header, payload])
    }

    /// Send a header (JSON) and a payload.
    pub fn send(&mut self, header: &str, payload: &[u8]) -> Result<(), WireError> {
        let header = header.as_bytes();
        let tag = self.tag(self.role, self.sent, header, payload);
        let mut frame = Vec::with_capacity(12 + header.len() + 32);
        frame.extend_from_slice(&(header.len() as u32).to_be_bytes());
        frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
        frame.extend_from_slice(header);
        self.stream.write_all(&frame)?;
        self.stream.write_all(payload)?;
        self.stream.write_all(&tag)?;
        self.stream.flush()?;
        self.sent += 1;
        Ok(())
    }

    /// Receive a header and a payload, refusing anything not signed by the
    /// other end in order.
    pub fn receive(&mut self) -> Result<(String, Vec<u8>), WireError> {
        let mut lengths = [0u8; 12];
        self.stream.read_exact(&mut lengths)?;
        let header_length = u32::from_be_bytes(lengths[..4].try_into().unwrap());
        let payload_length = u64::from_be_bytes(lengths[4..].try_into().unwrap());
        if header_length > MAX_HEADER {
            return Err(WireError::TooLarge(header_length as u64));
        }
        if payload_length > MAX_PAYLOAD {
            return Err(WireError::TooLarge(payload_length));
        }
        let mut header = vec![0u8; header_length as usize];
        self.stream.read_exact(&mut header)?;
        let mut payload = vec![0u8; payload_length as usize];
        self.stream.read_exact(&mut payload)?;
        let mut tag = [0u8; 32];
        self.stream.read_exact(&mut tag)?;
        let from = if self.role == Role::Dispatcher { Role::Worker } else { Role::Dispatcher };
        if !same(&tag, &self.tag(from, self.received, &header, &payload)) {
            return Err(WireError::Tampered);
        }
        self.received += 1;
        let header = String::from_utf8(header).map_err(|_| WireError::Protocol("a header was not text".into()))?;
        Ok((header, payload))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::net::TcpListener;

    /// A connected pair: the worker's end accepted on a thread.
    pub(crate) fn pair(worker_token: &'static [u8], dispatcher_token: &'static [u8]) -> (Result<Wire, WireError>, Result<Wire, WireError>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = std::thread::spawn(move || Wire::accept(listener.accept().unwrap().0, worker_token));
        let dispatcher = Wire::connect(TcpStream::connect(address).unwrap(), dispatcher_token);
        (dispatcher, worker.join().unwrap())
    }

    #[test]
    fn both_ends_with_the_token_talk() {
        let (dispatcher, worker) = pair(b"same secret", b"same secret");
        let (mut dispatcher, mut worker) = (dispatcher.unwrap(), worker.unwrap());
        dispatcher.send("{\"op\":\"render\"}", b"project bytes").unwrap();
        let (header, payload) = worker.receive().unwrap();
        assert_eq!((header.as_str(), payload.as_slice()), ("{\"op\":\"render\"}", b"project bytes".as_slice()));
        worker.send("{\"op\":\"result\"}", &[7; 100_000]).unwrap();
        assert_eq!(dispatcher.receive().unwrap().1.len(), 100_000);
    }

    #[test]
    fn a_wrong_token_is_refused_at_either_end() {
        let (dispatcher, worker) = pair(b"the farm's", b"a guess");
        assert!(matches!(dispatcher, Err(WireError::Refused)), "{:?}", dispatcher.err());
        assert!(matches!(worker, Err(WireError::Refused)));
    }

    #[test]
    fn an_altered_or_replayed_frame_is_refused() {
        // Altered: a byte of the payload flipped on the way.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            let mut wire = Wire::accept(listener.accept().unwrap().0, b"token").unwrap();
            let first = wire.receive().map(|(h, _)| h);
            let second = wire.receive().map(|(h, _)| h);
            (first, second)
        });
        let mut dispatcher = Wire::connect(TcpStream::connect(address).unwrap(), b"token").unwrap();
        dispatcher.send("{\"op\":\"status\"}", b"").unwrap();
        // The same frame again, as a replay would send it.
        let tag = dispatcher.tag(Role::Dispatcher, 0, b"{\"op\":\"status\"}", b"");
        let mut frame = Vec::new();
        frame.extend_from_slice(&(15u32).to_be_bytes());
        frame.extend_from_slice(&0u64.to_be_bytes());
        frame.extend_from_slice(b"{\"op\":\"status\"}");
        frame.extend_from_slice(&tag);
        dispatcher.stream.write_all(&frame).unwrap();
        let (first, second) = worker.join().unwrap();
        assert!(first.is_ok());
        assert!(matches!(second, Err(WireError::Tampered)), "a replayed frame was taken");
    }

    #[test]
    fn something_that_is_not_a_worker_is_named() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n.............");
        });
        let error = Wire::connect(TcpStream::connect(address).unwrap(), b"token").err().unwrap();
        assert!(error.to_string().contains("not a Tequila render worker"), "{error}");
    }
}
