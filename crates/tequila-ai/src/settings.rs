//! The providers a person has connected, remembered between sessions.

use crate::provider::{Key, Kind, Provider};
use tequila_format::json::{self, Value};
use tequila_format::json_write::{self, Object};
use std::path::Path;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    pub providers: Vec<Provider>,
    /// The id of the provider the assistant uses.
    pub active: Option<String>,
}

impl Settings {
    pub fn active(&self) -> Option<&Provider> {
        let id = self.active.as_deref()?;
        self.providers.iter().find(|p| p.id == id)
    }

    /// A fresh id for a new provider, unlike any in use.
    pub fn new_id(&self, base: &str) -> String {
        let base: String = base
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let base = if base.trim_matches('-').is_empty() { "provider".to_string() } else { base };
        (1..)
            .map(|n| if n == 1 { base.clone() } else { format!("{base}-{n}") })
            .find(|id| !self.providers.iter().any(|p| &p.id == id))
            .expect("an unbounded range has a free id")
    }

    pub fn to_json(&self) -> Result<String, String> {
        let mut list = Vec::new();
        for provider in &self.providers {
            let mut out = Object::new();
            out.set("id", provider.id.clone())
                .set("name", provider.name.clone())
                .set("kind", provider.kind.id())
                .set("base_url", provider.base_url.clone())
                .set("model", provider.model.clone())
                .set("key_header", provider.key_header.clone())
                .set("vision", provider.vision)
                .set("max_tokens", provider.max_tokens as f64);
            match &provider.key {
                Key::None => {}
                // Encrypted on the way to disk; never written as typed.
                Key::Stored(key) => {
                    out.set("key", crate::secret::protect(key)?);
                }
                Key::Env(name) => {
                    out.set("key_env", name.clone());
                }
            }
            list.push(out.build());
        }
        let mut out = Object::new();
        out.set("providers", Value::Array(list));
        out.set_some("active", self.active.clone());
        Ok(json_write::to_string(&out.build()))
    }

    /// Read settings. A provider whose key cannot be read — protected on
    /// another computer, say — is kept, without its key, and named in the
    /// returned list of complaints rather than silently dropped.
    pub fn from_json(text: &str) -> (Settings, Vec<String>) {
        let mut complaints = Vec::new();
        let Ok(value) = json::parse(text.as_bytes()) else {
            return (Settings::default(), vec!["the assistant settings could not be read".into()]);
        };
        let text_of = |v: &Value, key: &str| v.get(key).and_then(|x| x.as_str()).unwrap_or_default().to_string();
        let providers = value
            .get("providers")
            .map(|p| p.as_array())
            .unwrap_or(&[])
            .iter()
            .filter_map(|p| {
                let kind = Kind::from_id(&text_of(p, "kind"))?;
                let name = text_of(p, "name");
                let key = if let Some(stored) = p.get("key").and_then(|k| k.as_str()) {
                    match crate::secret::unprotect(stored) {
                        Ok(key) => Key::Stored(key),
                        Err(error) => {
                            complaints.push(format!("{name}: {error}; enter the key again"));
                            Key::None
                        }
                    }
                } else if let Some(env) = p.get("key_env").and_then(|k| k.as_str()) {
                    Key::Env(env.to_string())
                } else {
                    Key::None
                };
                Some(Provider {
                    id: text_of(p, "id"),
                    name,
                    kind,
                    base_url: text_of(p, "base_url"),
                    model: text_of(p, "model"),
                    key,
                    key_header: text_of(p, "key_header"),
                    vision: p.get("vision").and_then(|v| v.as_bool()).unwrap_or(true),
                    max_tokens: p.get("max_tokens").and_then(|v| v.as_f64()).map(|v| v as u32).unwrap_or(4096),
                })
            })
            .collect();
        let active = value.get("active").and_then(|a| a.as_str()).map(str::to_string);
        (Settings { providers, active }, complaints)
    }

    pub fn load(path: &Path) -> (Settings, Vec<String>) {
        match std::fs::read_to_string(path) {
            Ok(text) => Settings::from_json(&text),
            Err(_) => (Settings::default(), Vec::new()),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = self.to_json()?;
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, text).map_err(|e| format!("the assistant settings could not be saved: {e}"))?;
        std::fs::rename(&temporary, path).map_err(|e| format!("the assistant settings could not be saved: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_with_the_key_encrypted() {
        let mut claude = Provider::presets().remove(0);
        claude.key = Key::Stored("sk-ant-very-secret".into());
        let mut local = Provider::presets().remove(2);
        local.key = Key::Env("OLLAMA_KEY".into());
        let settings = Settings { providers: vec![claude, local], active: Some("claude".into()) };

        let text = settings.to_json().unwrap();
        if crate::secret::encrypted() {
            assert!(!text.contains("very-secret"), "the key was written in the clear: {text}");
        }
        let (back, complaints) = Settings::from_json(&text);
        assert!(complaints.is_empty(), "{complaints:?}");
        assert_eq!(back, settings);
        assert_eq!(back.active().unwrap().name, "Claude");
    }

    #[test]
    fn a_key_that_cannot_be_read_is_reported_not_lost_silently() {
        let text = r#"{"providers":[{"id":"x","name":"Uliverse","kind":"openai","base_url":"https://u.example/v1","model":"m","key":"dpapi:!!!!"}]}"#;
        let (settings, complaints) = Settings::from_json(text);
        assert_eq!(settings.providers.len(), 1, "the provider is kept");
        assert_eq!(settings.providers[0].key, Key::None);
        assert!(complaints[0].contains("Uliverse"), "{complaints:?}");
    }

    #[test]
    fn new_ids_do_not_collide() {
        let settings = Settings { providers: vec![Provider::presets().remove(0)], active: None };
        assert_eq!(settings.new_id("Claude"), "claude-2");
        assert_eq!(settings.new_id("Uliverse AI"), "uliverse-ai");
    }
}
