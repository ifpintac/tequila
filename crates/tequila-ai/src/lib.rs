//! The in-app assistant: talk to Tequila in plain language, through whichever
//! AI the person has connected — Claude, OpenAI, a model running locally, or a
//! company's own.
//!
//! Everything here is independent of the window. The application supplies two
//! things: the provider to use, and a way to *carry out* a tool call. It gets
//! back the assistant's words and the tool calls to perform. Tool calls become
//! the very same scripting requests a script or the console sends, so the
//! assistant can do nothing a script cannot, every step is undoable, and every
//! step shows in the console as it happens.
//!
//! Two wire formats cover the field:
//!
//! - **Anthropic Messages** — Claude.
//! - **OpenAI Chat Completions** — OpenAI itself, and the format nearly every
//!   other service speaks: local servers such as Ollama and LM Studio,
//!   OpenRouter, Azure-style gateways, and most companies' internal models.

pub mod base64;
pub mod conversation;
pub mod http;
pub mod provider;
pub mod secret;
pub mod session;
pub mod settings;
pub mod tools;
pub mod wire;

pub use conversation::{Call, Message, Part, Role};
pub use provider::{Key, Kind, Provider};
pub use session::{Session, Turn};
pub use settings::Settings;
