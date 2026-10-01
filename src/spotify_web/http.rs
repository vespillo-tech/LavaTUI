//! The HTTP seam: a tiny blocking request/response trait the client talks
//! through, [`Ureq`] for real, a scripted mock in tests.

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: Method,
    /// Full URL, query included.
    pub url: String,
    /// `Authorization: Bearer …`, if any.
    pub bearer: Option<String>,
    pub body: Body,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    Empty,
    /// `application/x-www-form-urlencoded`.
    Form(String),
    /// `application/json`.
    Json(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    /// The `Retry-After` header, raw.
    pub retry_after: Option<String>,
    pub body: String,
}

/// A transport failure: no status line came back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportError(pub String);

pub trait Http: Send {
    fn send(&self, req: &Request) -> Result<Response, TransportError>;
}

/// The real thing: one `ureq` agent (pooled connections, rustls), every
/// status returned as a response rather than an error.
pub struct Ureq(ureq::Agent);

impl Ureq {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(5)))
            .timeout_global(Some(Duration::from_secs(20)))
            .user_agent(concat!("lavatui/", env!("CARGO_PKG_VERSION")))
            .build();
        Self(ureq::Agent::new_with_config(config))
    }
}

impl Http for Ureq {
    fn send(&self, req: &Request) -> Result<Response, TransportError> {
        let auth = req.bearer.as_ref().map(|t| format!("Bearer {t}"));
        macro_rules! with_auth {
            ($builder:expr) => {{
                let b = $builder.header("Accept", "application/json");
                match &auth {
                    Some(a) => b.header("Authorization", a),
                    None => b,
                }
            }};
        }
        let sent = match req.method {
            Method::Get => with_auth!(self.0.get(&req.url)).call(),
            Method::Delete => with_auth!(self.0.delete(&req.url)).call(),
            Method::Post | Method::Put => {
                let b = match req.method {
                    Method::Post => self.0.post(&req.url),
                    _ => self.0.put(&req.url),
                };
                let b = with_auth!(b);
                match &req.body {
                    Body::Empty => b.send_empty(),
                    Body::Form(s) => b
                        .header("Content-Type", "application/x-www-form-urlencoded")
                        .send(s.as_str()),
                    Body::Json(s) => b
                        .header("Content-Type", "application/json")
                        .send(s.as_str()),
                }
            }
        };
        let mut resp = sent.map_err(|e| TransportError(e.to_string()))?;
        let status = resp.status().as_u16();
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let body = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| TransportError(e.to_string()))?;
        Ok(Response {
            status,
            retry_after,
            body,
        })
    }
}
