use sha2::{Digest, Sha256};
use std::io::Read;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

pub trait HttpTransport: Send + Sync {
    fn send(&self, request: &HttpRequest) -> Result<HttpResponse, HttpError>;
}

#[derive(Debug, Error)]
pub enum HttpError {
    #[error("{0}")]
    Message(String),
}

pub struct UreqTransport {
    agent: ureq::Agent,
}

impl UreqTransport {
    pub fn new(timeout: Duration) -> Self {
        let agent = ureq::AgentBuilder::new().timeout(timeout).build();
        Self { agent }
    }
}

impl HttpTransport for UreqTransport {
    fn send(&self, request: &HttpRequest) -> Result<HttpResponse, HttpError> {
        let mut call = self.agent.request(&request.method, &request.url);
        for (name, value) in &request.headers {
            call = call.set(name, value);
        }
        let result = if let Some(body) = &request.body {
            call = call.set("Content-Type", "application/json");
            call.send_string(body)
        } else {
            call.call()
        };
        match result {
            Ok(response) => read_response(response.status(), response.into_reader()),
            Err(ureq::Error::Status(status, response)) => {
                read_response(status, response.into_reader())
            }
            Err(error) => Err(HttpError::Message(error.to_string())),
        }
    }
}

fn read_response(status: u16, mut reader: impl Read) -> Result<HttpResponse, HttpError> {
    let mut body = String::new();
    reader
        .read_to_string(&mut body)
        .map_err(|error| HttpError::Message(error.to_string()))?;
    Ok(HttpResponse { status, body })
}

pub fn cache_key(
    adapter_id: &str,
    method: &str,
    url: &str,
    body: &str,
    credential_fingerprint: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(adapter_id.as_bytes());
    hasher.update(b"\n");
    hasher.update(method.as_bytes());
    hasher.update(b"\n");
    hasher.update(url.as_bytes());
    hasher.update(b"\n");
    hasher.update(body.as_bytes());
    hasher.update(b"\n");
    hasher.update(credential_fingerprint.as_bytes());
    hex_encode(&hasher.finalize())
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

pub fn credential_fingerprint(headers: &[(String, String)]) -> String {
    let auth = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        .map(|(_, value)| value.as_str())
        .unwrap_or("");
    let mut hasher = Sha256::new();
    hasher.update(auth.as_bytes());
    hex_encode(&hasher.finalize())
}
