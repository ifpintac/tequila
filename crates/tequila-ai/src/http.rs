//! The one place a request leaves the machine.

use std::time::Duration;

/// How long one model reply may take. Generous: a model reasoning over a
/// screenshot and thirty tools can take a minute, and cutting it off just
/// before it answers wastes the whole request.
const TIMEOUT: Duration = Duration::from_secs(300);

/// Replies carry no images back, but a verbose one can still be large.
const LIMIT: u64 = 32 * 1024 * 1024;

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        // An error status still has a body, and the body is the only place
        // the service says *why* — a wrong key, an unknown model, a quota.
        .http_status_as_error(false)
        .build()
        .into()
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
    let mut request = agent().get(url);
    for (name, value) in headers {
        // A GET has no body to describe.
        if name != "content-type" {
            request = request.header(name.as_str(), value.as_str());
        }
    }
    finish(request.call(), url)
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
