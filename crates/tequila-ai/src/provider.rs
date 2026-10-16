//! Which AI to talk to, and how.

/// The wire format a provider speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Anthropic's Messages API: Claude.
    Anthropic,
    /// OpenAI's Chat Completions API, and everything compatible with it.
    OpenAi,
}

impl Kind {
    pub const ALL: [Kind; 2] = [Kind::Anthropic, Kind::OpenAi];

    pub fn id(self) -> &'static str {
        match self {
            Kind::Anthropic => "anthropic",
            Kind::OpenAi => "openai",
        }
    }

    pub fn from_id(id: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Anthropic => "Anthropic (Claude)",
            Kind::OpenAi => "OpenAI-compatible",
        }
    }

    pub fn default_url(self) -> &'static str {
        match self {
            Kind::Anthropic => "https://api.anthropic.com",
            Kind::OpenAi => "https://api.openai.com/v1",
        }
    }
}

/// Where the key comes from. Never written anywhere in the clear: see
/// [`crate::secret`].
#[derive(Clone, PartialEq, Eq, Default)]
pub enum Key {
    /// No key: a local server such as Ollama.
    #[default]
    None,
    /// Typed into Tequila, and stored encrypted for this Windows account.
    Stored(String),
    /// Read from an environment variable when needed, never stored.
    Env(String),
}

// Hand-written so a key cannot end up in a log line or a panic message.
impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Key::None => f.write_str("None"),
            Key::Stored(_) => f.write_str("Stored(…)"),
            Key::Env(name) => write!(f, "Env({name})"),
        }
    }
}

impl Key {
    /// The key itself, if there is one to send.
    pub fn resolve(&self) -> Result<Option<String>, String> {
        match self {
            Key::None => Ok(None),
            Key::Stored(key) => Ok(Some(key.clone())),
            Key::Env(name) => std::env::var(name)
                .map(Some)
                .map_err(|_| format!("the environment variable {name} is not set")),
        }
    }
}

/// What a pasted key says about where it belongs, for someone who was given
/// a key and nothing else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyLooks {
    /// A prefix only this service gives its keys: its name, API type and
    /// address can be filled in.
    Known { service: &'static str, kind: Kind, base_url: &'static str },
    /// `sk-` and no more: OpenAI's older keys look like this, and so do the
    /// keys many company gateways issue (LiteLLM's among them). Which one it
    /// is cannot be told from the key, and a company key sent to OpenAI to
    /// find out would be a company key given to OpenAI.
    OpenAiOrGateway,
    /// Nothing recognisable: the address has to come from whoever gave it.
    Unknown,
}

/// See [`KeyLooks`]. Only the prefix is looked at, and nothing is sent.
pub fn recognise_key(key: &str) -> KeyLooks {
    let key = key.trim();
    const KNOWN: [(&str, &str, Kind, &str); 9] = [
        ("sk-ant-", "Anthropic (Claude)", Kind::Anthropic, "https://api.anthropic.com"),
        ("sk-proj-", "OpenAI", Kind::OpenAi, "https://api.openai.com/v1"),
        ("sk-svcacct-", "OpenAI", Kind::OpenAi, "https://api.openai.com/v1"),
        ("sk-admin-", "OpenAI", Kind::OpenAi, "https://api.openai.com/v1"),
        ("sk-or-", "OpenRouter", Kind::OpenAi, "https://openrouter.ai/api/v1"),
        ("AIza", "Google Gemini", Kind::OpenAi, "https://generativelanguage.googleapis.com/v1beta/openai"),
        ("gsk_", "Groq", Kind::OpenAi, "https://api.groq.com/openai/v1"),
        ("xai-", "xAI (Grok)", Kind::OpenAi, "https://api.x.ai/v1"),
        ("pplx-", "Perplexity", Kind::OpenAi, "https://api.perplexity.ai"),
    ];
    for (prefix, service, kind, base_url) in KNOWN {
        if key.starts_with(prefix) && key.len() > prefix.len() + 8 {
            return KeyLooks::Known { service, kind, base_url };
        }
    }
    if key.starts_with("sk-") && key.len() > 12 {
        return KeyLooks::OpenAiOrGateway;
    }
    KeyLooks::Unknown
}

#[derive(Clone, Debug, PartialEq)]
pub struct Provider {
    /// Stable, for remembering which one is active.
    pub id: String,
    /// What the person calls it: "Claude", "Uliverse".
    pub name: String,
    pub kind: Kind,
    /// For Anthropic, the API root (`https://api.anthropic.com`). For
    /// OpenAI-compatible services, the base the paths hang off, usually
    /// ending in `/v1`.
    pub base_url: String,
    pub model: String,
    pub key: Key,
    /// A different header to carry the key in, for gateways that want one —
    /// Azure's `api-key`, say. Empty means the usual one for the kind.
    pub key_header: String,
    /// Whether the model can look at images. Screenshots are what let the
    /// assistant check its own work, so this matters; a text-only model still
    /// works, blind.
    pub vision: bool,
    pub max_tokens: u32,
}

impl Provider {
    /// Starting points for the providers people are most likely to add. The
    /// model is left empty where a default would only be a guess that goes
    /// stale — "Fetch models" asks the service what it actually has.
    pub fn presets() -> Vec<Provider> {
        let make = |id: &str, name: &str, kind: Kind, url: &str, model: &str| Provider {
            id: id.into(),
            name: name.into(),
            kind,
            base_url: url.into(),
            model: model.into(),
            key: Key::None,
            key_header: String::new(),
            vision: true,
            max_tokens: 4096,
        };
        vec![
            make("claude", "Claude", Kind::Anthropic, Kind::Anthropic.default_url(), "claude-opus-5-5"),
            make("openai", "OpenAI", Kind::OpenAi, Kind::OpenAi.default_url(), ""),
            make("ollama", "Ollama (on this computer)", Kind::OpenAi, "http://localhost:11434/v1", ""),
            make("custom", "Company AI (e.g. Uliverse)", Kind::OpenAi, "", ""),
        ]
    }

    fn base(&self) -> &str {
        self.base_url.trim().trim_end_matches('/')
    }

    /// Where a conversation turn is posted.
    pub fn chat_url(&self) -> String {
        match self.kind {
            Kind::Anthropic => format!("{}/v1/messages", self.base()),
            Kind::OpenAi => format!("{}/chat/completions", self.base()),
        }
    }

    /// Where the list of models is.
    pub fn models_url(&self) -> String {
        match self.kind {
            Kind::Anthropic => format!("{}/v1/models", self.base()),
            Kind::OpenAi => format!("{}/models", self.base()),
        }
    }

    /// The headers every request carries, key included.
    pub fn headers(&self) -> Result<Vec<(String, String)>, String> {
        let mut headers = vec![("content-type".to_string(), "application/json".to_string())];
        if self.kind == Kind::Anthropic {
            headers.push(("anthropic-version".into(), "2023-06-01".into()));
        }
        if let Some(key) = self.key.resolve()? {
            let custom = self.key_header.trim();
            if !custom.is_empty() {
                headers.push((custom.to_string(), key));
            } else if self.kind == Kind::Anthropic {
                headers.push(("x-api-key".into(), key));
            } else {
                headers.push(("authorization".into(), format!("Bearer {key}")));
            }
        }
        Ok(headers)
    }

    /// What is wrong with this provider's settings, before anything is sent.
    pub fn problem(&self) -> Option<String> {
        if self.name.trim().is_empty() {
            return Some("it needs a name".into());
        }
        let url = self.base();
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return Some("the address must start with https:// (or http:// for a server on this computer)".into());
        }
        if url.starts_with("http://") && !is_local(url) {
            // A key sent in the clear to another machine is a key anyone on
            // the way can read.
            return Some("only a server on this computer may use plain http://; use https://".into());
        }
        if self.model.trim().is_empty() {
            return Some("choose a model (Fetch models lists them)".into());
        }
        None
    }
}

fn is_local(url: &str) -> bool {
    let host = url
        .trim_start_matches("http://")
        .split(['/', ':'])
        .next()
        .unwrap_or("");
    matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

#[cfg(test)]
mod tests {

    /// A key's prefix names its service where only that service uses it;
    /// a bare `sk-` is not claimed for OpenAI, since company gateways issue
    /// them too; and nothing else is guessed.
    #[test]
    fn keys_are_recognised_by_their_prefix_only_where_it_is_certain() {
        let filler = "abcdefghijklmnopqrstuvwxyz0123456789";
        let known = |key: String| match recognise_key(&key) {
            KeyLooks::Known { service, kind, base_url } => (service, kind, base_url),
            other => panic!("{key}: {other:?}"),
        };
        assert_eq!(known(format!("sk-ant-api03-{filler}")).1, Kind::Anthropic);
        assert_eq!(known(format!("sk-proj-{filler}")).2, "https://api.openai.com/v1");
        assert_eq!(known(format!("  AIza{filler}  ")).0, "Google Gemini");
        assert_eq!(known(format!("sk-or-v1-{filler}")).0, "OpenRouter");
        assert_eq!(recognise_key(&format!("sk-{filler}")), KeyLooks::OpenAiOrGateway);
        assert_eq!(recognise_key(&format!("uv-{filler}")), KeyLooks::Unknown);
        assert_eq!(recognise_key("0123456789abcdef0123456789abcdef"), KeyLooks::Unknown);
        assert_eq!(recognise_key("sk-ant-"), KeyLooks::Unknown, "a prefix alone is not a key");
    }

    use super::*;

    fn provider(kind: Kind, url: &str) -> Provider {
        Provider { base_url: url.into(), kind, model: "m".into(), ..Provider::presets().remove(0) }
    }

    #[test]
    fn urls_follow_each_format() {
        assert_eq!(provider(Kind::Anthropic, "https://api.anthropic.com/").chat_url(), "https://api.anthropic.com/v1/messages");
        assert_eq!(provider(Kind::OpenAi, "https://api.openai.com/v1").chat_url(), "https://api.openai.com/v1/chat/completions");
        assert_eq!(provider(Kind::OpenAi, "http://localhost:11434/v1").models_url(), "http://localhost:11434/v1/models");
    }

    #[test]
    fn the_key_goes_where_each_service_expects() {
        let mut claude = provider(Kind::Anthropic, "https://api.anthropic.com");
        claude.key = Key::Stored("sk-ant".into());
        let headers = claude.headers().unwrap();
        assert!(headers.contains(&("x-api-key".into(), "sk-ant".into())));
        assert!(headers.iter().any(|(k, _)| k == "anthropic-version"));

        let mut openai = provider(Kind::OpenAi, "https://api.openai.com/v1");
        openai.key = Key::Stored("sk-o".into());
        assert!(openai.headers().unwrap().contains(&("authorization".into(), "Bearer sk-o".into())));

        openai.key_header = "api-key".into();
        assert!(openai.headers().unwrap().contains(&("api-key".into(), "sk-o".into())));

        let local = provider(Kind::OpenAi, "http://localhost:11434/v1");
        assert!(!local.headers().unwrap().iter().any(|(k, _)| k == "authorization"));
    }

    /// A key is never sent in the clear to another machine.
    #[test]
    fn plain_http_is_only_for_this_computer() {
        assert!(provider(Kind::OpenAi, "http://localhost:11434/v1").problem().is_none());
        assert!(provider(Kind::OpenAi, "http://127.0.0.1:1234/v1").problem().is_none());
        assert!(provider(Kind::OpenAi, "http://ai.example.com/v1").problem().unwrap().contains("https"));
        assert!(provider(Kind::OpenAi, "ai.example.com").problem().is_some());
    }

    #[test]
    fn a_key_never_prints() {
        let text = format!("{:?}", Key::Stored("sk-secret".into()));
        assert!(!text.contains("secret"), "{text}");
    }
}
