//! A model that Ollama runs, asked through Ollama's own API what it will be
//! given, before a request is sent to it.
//!
//! Ollama's OpenAI-compatible endpoint has no way to say how long a request
//! may be. Every model gets the context length the server gives it: the
//! model's own setting if it has one, else the server's (`OLLAMA_CONTEXT_LENGTH`,
//! or the Ollama app's Context length; left alone, 4096 tokens on a graphics
//! card under 24 GB). A longer request is not refused. Ollama cuts it to
//! half the context by dropping its start, which is where the instructions
//! and the tools are. The model then answers without either, and each
//! round is read again from the start, slowly, for nothing.
//!
//! So Ollama is asked, through its own API, what this model will get, and
//! a request that would not fit is not sent; the person is told the number
//! to set instead. The OpenAI endpoint itself cannot be asked: it is the
//! same with any server.

use crate::http::{self, Hangup, Limits};
use crate::provider::{self, Provider};
use std::time::Duration;
use tequila_format::json::{self, Value};

/// What Ollama says about the model.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Info {
    /// Ollama's version.
    pub version: String,
    pub model: String,
    /// What the model can do (`completion`, `tools`, `vision`, `thinking`),
    /// when Ollama says.
    pub capabilities: Option<Vec<String>>,
    /// The context length a request through the OpenAI endpoint gets, in
    /// tokens: what the model is loaded with.
    pub context: Option<u32>,
    /// The longest context the model itself takes.
    pub model_max: Option<u32>,
    /// The model's size in memory, and how much of it is on the graphics
    /// card, once loaded.
    pub size: Option<u64>,
    pub size_vram: Option<u64>,
}

impl Info {
    /// Whether the model can do this; None when Ollama does not say.
    pub fn can(&self, capability: &str) -> Option<bool> {
        self.capabilities.as_ref().map(|all| all.iter().any(|c| c == capability))
    }

    /// The share of the loaded model on the graphics card, 0 to 1.
    pub fn on_gpu(&self) -> Option<f64> {
        match (self.size, self.size_vram) {
            (Some(size), Some(vram)) if size > 0 => Some((vram as f64 / size as f64).min(1.0)),
            _ => None,
        }
    }
}

/// The server's root, which Ollama's own API hangs off: the address
/// without the `/v1` the OpenAI-compatible endpoint is under.
pub fn root(provider: &Provider) -> String {
    let base = provider.base_url.trim().trim_end_matches('/');
    base.strip_suffix("/v1").unwrap_or(base).to_string()
}

/// Whether a provider could be Ollama, and is worth asking: a server on
/// this computer, whatever its port; Ollama's own port; or a host named
/// for it. Ollama's API settles it; a service elsewhere is not asked
/// questions it has no answers to.
pub fn worth_asking(provider: &Provider) -> bool {
    let url = &provider.base_url;
    provider::is_local(url) || provider::port(url) == Some(11434) || provider::host(url).is_some_and(|h| h.contains("ollama"))
}

/// The version an Ollama server gives, or None when the server is not
/// Ollama (or is not there: the request itself will say so).
pub fn version(provider: &Provider, headers: &[(String, String)]) -> Option<String> {
    let wait = if provider::is_local(&provider.base_url) { Duration::from_secs(5) } else { Duration::from_secs(15) };
    let response = http::get_within(&format!("{}/api/version", root(provider)), headers, wait).ok()?;
    if response.status != 200 {
        return None;
    }
    let value = json::parse(response.body.as_bytes()).ok()?;
    value.get("version").and_then(|v| v.as_str()).filter(|v| !v.is_empty()).map(str::to_string)
}

/// Ask Ollama about the provider's model: what it can do and the context it
/// gets. A model that is not loaded is loaded first (`loading` says when),
/// as the request would have loaded it, since only a loaded model's context
/// can be read. None when the server is not Ollama.
pub fn inspect(provider: &Provider, headers: &[(String, String)], hangup: &Hangup, mut loading: impl FnMut()) -> Result<Option<Info>, String> {
    let Some(version) = version(provider, headers) else { return Ok(None) };
    let root = root(provider);
    let model = provider.model.trim().to_string();
    let quick = Limits { first: Duration::from_secs(30), silence: Duration::from_secs(30) };
    let body = format!("{{\"model\":{}}}", tequila_format::json_write::to_string(&Value::String(model.clone())));

    let shown = http::post_watched(&format!("{root}/api/show"), headers, body.clone(), quick, hangup)?;
    let show = json::parse(shown.body.as_bytes()).unwrap_or(Value::Null);
    if shown.status >= 400 {
        let said = crate::wire::error_message(&show).unwrap_or_else(|| format!("it answered {}", shown.status));
        return Err(if shown.status == 404 {
            format!("Ollama has no model called {model} ({said}). Pick one under Fetch models, or download it first: ollama pull {model}")
        } else {
            format!("Ollama could not say what {model} is: {said}")
        });
    }
    let mut info = Info { version, model: model.clone(), ..Info::default() };
    info.capabilities = show.get("capabilities").map(|c| c.as_array().iter().filter_map(|v| v.as_str()).map(str::to_string).collect());
    if let Some(model_info) = show.get("model_info") {
        let architecture = model_info.get("general.architecture").and_then(|a| a.as_str()).unwrap_or_default();
        info.model_max = model_info.get(&format!("{architecture}.context_length")).and_then(|c| c.as_f64()).map(|c| c as u32);
    }

    let mut running = loaded(&root, headers, &model)?;
    if running.is_none() {
        loading();
        let load = Limits { first: Duration::from_secs(10 * 60), silence: Duration::from_secs(60) };
        let answer = http::post_watched(&format!("{root}/api/generate"), headers, body, load, hangup)?;
        if let Some(error) = json::parse(answer.body.as_bytes()).ok().as_ref().and_then(crate::wire::error_message) {
            return Err(format!(
                "Ollama could not load {model}: {error}. A smaller model, or a shorter context length, needs less memory."
            ));
        }
        running = loaded(&root, headers, &model)?;
    }
    if let Some(entry) = running {
        let number = |key: &str| entry.get(key).and_then(|v| v.as_f64());
        info.context = number("context_length").map(|c| c as u32).filter(|c| *c > 0);
        info.size = number("size").map(|s| s as u64);
        info.size_vram = number("size_vram").map(|s| s as u64);
    }
    // Not loaded after all, or an Ollama too old to say: the model's own
    // setting is the next best answer.
    if info.context.is_none() {
        info.context = show.get("parameters").and_then(|p| p.as_str()).and_then(num_ctx);
    }
    Ok(Some(info))
}

/// The model's entry in Ollama's list of loaded models.
fn loaded(root: &str, headers: &[(String, String)], model: &str) -> Result<Option<Value>, String> {
    let response = http::get_within(&format!("{root}/api/ps"), headers, Duration::from_secs(30))?;
    let value = json::parse(response.body.as_bytes()).unwrap_or(Value::Null);
    // "qwen3" is "qwen3:latest" to Ollama.
    let tagged = if model.contains(':') { model.to_string() } else { format!("{model}:latest") };
    Ok(value
        .get("models")
        .map(|m| m.as_array())
        .unwrap_or(&[])
        .iter()
        .find(|entry| ["name", "model"].iter().any(|key| entry.get(key).and_then(|n| n.as_str()).is_some_and(|n| n == model || n == tagged)))
        .cloned())
}

/// `num_ctx` out of a model's parameters, as `/api/show` lists them.
fn num_ctx(parameters: &str) -> Option<u32> {
    parameters.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        (words.next() == Some("num_ctx")).then(|| words.next()?.parse().ok()).flatten()
    })
}

/// About how many tokens a request body is to a model: a token for every
/// four letters, every digit and every two other characters, spaces free,
/// and a picture as a few hundred to a thousand whatever its size in the
/// body. JSON costs more than prose — numbers go a digit a token, keys are
/// short — and tool answers are JSON. Measured against Ollama's own count
/// for qwen3 on November 6: the Assistant's first request is 4,843 tokens
/// there and 5,888 here; with the command catalog read, 21,076 and 22,977.
/// A little over, which is the side to be wrong on: four characters a
/// token said 18,095 for the second, and Ollama would have cut it.
pub fn estimate_tokens(body: &str) -> u32 {
    const PICTURE: u32 = 800;
    let (mut letters, mut each, mut other) = (0usize, 0usize, 0usize);
    let mut pictures = 0u32;
    let mut rest = body;
    loop {
        let next = rest.find("data:image/");
        for c in rest[..next.unwrap_or(rest.len())].chars() {
            if c.is_ascii_alphabetic() {
                letters += 1;
            } else if c.is_ascii_digit() || (c.is_alphanumeric() && !c.is_ascii()) {
                // A digit, or a letter of a script whose words are not
                // spelled in Latin letters: about a token each.
                each += 1;
            } else if !c.is_whitespace() {
                other += 1;
            }
        }
        let Some(start) = next else { break };
        pictures += 1;
        let picture = &rest[start..];
        rest = &picture[picture.find('"').unwrap_or(picture.len())..];
    }
    (letters as f64 / 4.0 + each as f64 + other as f64 / 2.0).ceil() as u32 + pictures * PICTURE
}

/// The Ollama app's Context length steps, the values to suggest.
const STEPS: [u32; 7] = [4096, 8192, 16384, 32768, 65536, 131072, 262144];

/// Room kept for the answer: reasoning, words and tool calls.
fn answer_room(max_tokens: u32) -> u32 {
    max_tokens.clamp(256, 2048)
}

/// Room for the conversation to grow, in a suggestion: as much as the
/// longest answer a tool gives. That is the command catalog
/// (`tequila_api`), about 16,000 tokens to qwen3. The guide's pages are a
/// quarter of that and are what a model is sent to first, but it may still
/// ask for the catalog: suggest only enough for the request in hand, and
/// the very next one is refused again.
pub const GROWTH: u32 = 16_000;

/// The context length to suggest for a request of `tokens`: room for it,
/// its answer and the conversation to grow, as the next of the Ollama
/// app's steps, and no more than the model takes.
pub fn suggestion(info: &Info, tokens: u32, max_tokens: u32) -> u32 {
    let wanted = tokens + answer_room(max_tokens) + GROWTH;
    let step = STEPS.iter().copied().find(|s| *s >= wanted).unwrap_or(wanted.next_multiple_of(8192));
    info.model_max.map_or(step, |max| step.min(max))
}

/// How to set Ollama's context length to `tokens`, in words.
fn how_to_set(tokens: u32) -> String {
    format!(
        "in the Ollama app, Settings, Context length; or set the environment variable OLLAMA_CONTEXT_LENGTH={tokens} and restart Ollama"
    )
}

/// For a request that fits now: whether the context leaves room to grow,
/// and if not, what to set. None when it does, or nothing is known.
pub fn room_to_grow(info: &Info, tokens: u32, max_tokens: u32) -> Option<String> {
    let context = info.context?;
    let step = suggestion(info, tokens, max_tokens);
    (context < step).then(|| {
        format!(
            "Ollama gives {} {} tokens of context: enough to start, but a longer job - several pages of the guide, pictures, the \
             command catalog - needs about 16,000 tokens more. Set it to {} or more: {}.",
            info.model,
            thousands(context),
            short(step),
            how_to_set(step),
        )
    })
}

/// Whether a request of `tokens` fits the model's context, with room for
/// the answer; if not, what to tell the person.
pub fn check_fit(info: &Info, tokens: u32, max_tokens: u32) -> Result<(), String> {
    let Some(context) = info.context else { return Ok(()) };
    let needed = tokens + answer_room(max_tokens);
    if needed <= context {
        return Ok(());
    }
    let model = &info.model;
    let size = thousands(tokens);
    if let Some(max) = info.model_max.filter(|max| needed > *max) {
        return Err(format!(
            "This request is about {size} tokens, more than {model} can take ({} at most). Choose a model with a longer context, or start a New chat.",
            thousands(max)
        ));
    }
    let step = suggestion(info, tokens, max_tokens);
    Err(format!(
        "This request is about {size} tokens, but Ollama gives {model} room for {}. It would cut the start of the request to make it fit — the \
         instructions and the tools — and the model could not work in the scene. Set Ollama's context length to {} or more: {}. Then ask \
         again: the conversation is kept. A longer context needs more memory on the graphics card.",
        thousands(context),
        short(step),
        how_to_set(step),
    ))
}

/// What to say when the model cannot use tools at all.
pub fn no_tools(info: &Info) -> String {
    format!(
        "Ollama says {} can't use tools, so it can't make changes in the scene. Choose a model that can: Ollama's library marks them \"tools\".",
        info.model
    )
}

/// A note about a model running partly on the processor, or None when
/// it is all on the graphics card.
pub fn speed_note(info: &Info) -> Option<String> {
    let share = info.on_gpu()?;
    if share >= 0.95 {
        return None;
    }
    let percent = (share * 100.0).round() as u32;
    Some(if percent == 0 {
        format!("{} runs on the processor, not the graphics card, so it answers slowly.", info.model)
    } else {
        format!(
            "Only {percent}% of {} fits in the graphics card's memory; the rest runs on the processor, which is much slower. A smaller model, or a shorter context length, fits better.",
            info.model
        )
    })
}

/// `12,345`.
pub fn thousands(n: u32) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `16k (16384)`.
fn short(tokens: u32) -> String {
    if tokens % 1024 == 0 {
        format!("{}k ({tokens})", tokens / 1024)
    } else {
        tokens.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(context: Option<u32>) -> Info {
        Info { version: "0.40.13".into(), model: "qwen3:4b".into(), context, model_max: Some(262_144), ..Info::default() }
    }

    #[test]
    fn a_request_that_fits_is_let_through() {
        assert!(check_fit(&info(Some(16_384)), 9_000, 4096).is_ok());
        assert!(check_fit(&info(None), 90_000, 4096).is_ok(), "nothing known, nothing refused");
    }

    /// The message names the number to set, both ways of setting it, and
    /// that asking again carries on.
    #[test]
    fn a_request_that_does_not_fit_says_what_to_set() {
        let message = check_fit(&info(Some(4096)), 9_700, 4096).unwrap_err();
        assert!(message.contains("about 9,700 tokens"), "{message}");
        assert!(message.contains("room for 4,096"), "{message}");
        // Room for this request, its answer, and the command catalog.
        assert!(message.contains("32k (32768)"), "{message}");
        assert!(message.contains("OLLAMA_CONTEXT_LENGTH=32768"), "{message}");
        assert!(message.contains("Settings, Context length"), "{message}");
        assert!(message.contains("conversation is kept"), "{message}");
    }

    /// A context the first request fits, but the command catalog would
    /// not, is worth a word before it is found out the slow way.
    #[test]
    fn a_context_with_no_room_to_grow_is_noted() {
        let note = room_to_grow(&info(Some(16_384)), 5_500, 4096).unwrap();
        assert!(note.contains("16,384") && note.contains("32k (32768)"), "{note}");
        assert!(room_to_grow(&info(Some(32_768)), 5_500, 4096).is_none());
        assert!(room_to_grow(&info(None), 5_500, 4096).is_none());
    }

    #[test]
    fn a_request_longer_than_the_model_can_take_says_so() {
        let mut small = info(Some(4096));
        small.model_max = Some(8192);
        let message = check_fit(&small, 9_000, 4096).unwrap_err();
        assert!(message.contains("8,192 at most"), "{message}");
    }

    /// Pictures count as pictures, not as the length of their base64.
    #[test]
    fn pictures_are_counted_by_the_picture() {
        let picture = format!("{{\"url\":\"data:image/png;base64,{}\"}}", "A".repeat(400_000));
        let tokens = estimate_tokens(&picture);
        assert!(tokens < 1000, "{tokens}");
        assert_eq!(estimate_tokens(&"word ".repeat(700)), 700);
        assert_eq!(estimate_tokens("[0.25, 1.5]"), 8, "five digits a token each, five marks a half each");
    }

    #[test]
    fn the_context_setting_is_read_from_a_models_parameters() {
        assert_eq!(num_ctx("temperature                    0.6\nnum_ctx                        8192"), Some(8192));
        assert_eq!(num_ctx("temperature 0.6"), None);
    }

    #[test]
    fn a_model_partly_on_the_processor_is_noted() {
        let mut split = info(Some(4096));
        split.size = Some(10_000);
        split.size_vram = Some(6_000);
        assert!(speed_note(&split).unwrap().contains("60%"));
        split.size_vram = Some(10_000);
        assert!(speed_note(&split).is_none());
    }

    #[test]
    fn the_root_is_the_address_without_its_v1() {
        let provider = Provider::preset("ollama").unwrap();
        assert_eq!(root(&provider), "http://localhost:11434");
        assert!(worth_asking(&provider));
        let remote = Provider { base_url: "https://api.openai.com/v1".into(), ..provider };
        assert!(!worth_asking(&remote));
    }
}
