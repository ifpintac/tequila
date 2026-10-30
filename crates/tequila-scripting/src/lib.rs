//! The scripting channel: driving a running Tequila from outside it.
//!
//! `TEQUILA_SPEC.md` §1.16 asks for a Python module over the command bus, and
//! `docs/04` §8.3 asks for it to be *generated from the command catalog* so
//! that anything clickable is scriptable. This crate is the layer underneath
//! that: a transport and a protocol, with no Python in it.
//!
//! The order is deliberate. A Python binding needs somewhere to send what it
//! builds, and an automated client — a test, a batch job, an assistant driving
//! the app for someone — needs the transport and nothing else. So the
//! transport comes first and the bindings sit on it later.
//!
//! # What makes this small
//!
//! Almost all of it already existed. Every command has a stable name in
//! [`tequila_scene::command::Command::CATALOG`], every command already
//! serializes to JSON and back through `tequila_format::journal`, and that
//! round trip is exhaustively tested against the catalog. So the write half
//! of this API is the journal's codec with a socket in front of it.
//!
//! The read half is the part that did not exist. Commands only write, and a
//! script that cannot ask what is in the scene can only act blind.

pub mod bindings;
pub mod client;
pub mod document;
pub mod material;
pub mod protocol;
pub mod server;
pub mod shell;
pub mod studio_presets;
