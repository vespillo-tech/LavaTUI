//! What can go wrong, in terms the UI can act on: log in again, wait, check
//! the network, or show a message.

use std::fmt;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// No tokens: the user has to log in first.
    NotLoggedIn,
    /// The refresh token was refused (revoked, or past Spotify's six-month
    /// lifetime): the stored login is gone; log in again.
    LoginExpired,
    /// The login didn't complete: denied in the browser, timed out, state
    /// mismatch, port busy, bad code, …
    Login(String),
    /// Couldn't reach Spotify (no network, DNS, TLS, timeout).
    Offline(String),
    /// 429, still limited after the one short wait we allow ourselves.
    RateLimited { retry_after: Duration },
    /// 403: not allowed (e.g. a playlist the user neither owns nor
    /// collaborates on, or a user not on a development-mode app's allowlist).
    Forbidden(String),
    /// 404.
    NotFound(String),
    /// Any other non-success status.
    Api { status: u16, message: String },
    /// A 2xx whose body didn't parse.
    Decode(String),
}

impl Error {
    /// True if logging in again is the fix.
    pub fn needs_login(&self) -> bool {
        matches!(self, Error::NotLoggedIn | Error::LoginExpired)
    }

    /// When asking again may help: after Spotify's `Retry-After` for a
    /// rate limit (at least `pause`), after `pause` for no network or a
    /// server error. `None`: it won't (refused, not found, a bad reply).
    pub fn retry_after(&self, pause: Duration) -> Option<Duration> {
        match self {
            Error::RateLimited { retry_after } => Some((*retry_after).max(pause)),
            Error::Offline(_) => Some(pause),
            Error::Api { status, .. } if *status >= 500 => Some(pause),
            _ => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotLoggedIn => f.write_str("not logged in to Spotify"),
            Error::LoginExpired => f.write_str("Spotify login expired, log in again"),
            Error::Login(why) => write!(f, "Spotify login failed: {why}"),
            Error::Offline(why) => write!(f, "can't reach Spotify: {why}"),
            Error::RateLimited { retry_after } => write!(
                f,
                "Spotify rate limit, try again in {}s",
                retry_after.as_secs().max(1)
            ),
            // A development-mode app's user who isn't on its allowlist.
            Error::Forbidden(msg) if msg.to_ascii_lowercase().contains("registered") => f
                .write_str(
                    "Spotify refused this account: add it under User Management in your \
                     Spotify app's settings",
                ),
            Error::Forbidden(msg) => write!(f, "Spotify refused: {msg}"),
            Error::NotFound(msg) => write!(f, "not found on Spotify: {msg}"),
            Error::Api { status, message } => write!(f, "Spotify error {status}: {message}"),
            Error::Decode(why) => write!(f, "unexpected Spotify reply: {why}"),
        }
    }
}

impl std::error::Error for Error {}

/// The human message inside a Web API error body
/// (`{"error":{"status":403,"message":"…"}}`) or an accounts error body
/// (`{"error":"invalid_grant","error_description":"…"}`), else the raw body.
pub fn message(body: &str) -> String {
    #[derive(serde::Deserialize)]
    struct Wrapper {
        error: serde_json::Value,
        error_description: Option<String>,
    }
    let parsed = serde_json::from_str::<Wrapper>(body).ok();
    let msg = parsed.and_then(|w| match w.error {
        serde_json::Value::Object(o) => o.get("message")?.as_str().map(str::to_owned),
        serde_json::Value::String(code) => Some(match w.error_description {
            Some(desc) if !desc.is_empty() => desc,
            _ => code,
        }),
        _ => None,
    });
    msg.unwrap_or_else(|| {
        let body = body.trim();
        let short: String = body.chars().take(200).collect();
        if short.is_empty() {
            "no details".into()
        } else {
            short
        }
    })
}

/// The `error` code of an accounts-service error body (`invalid_grant`, …).
pub fn oauth_code(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    v.get("error")?.as_str().map(str::to_owned)
}

/// Maps a non-success Web API status to an [`Error`].
pub fn from_status(status: u16, body: &str) -> Error {
    let message = message(body);
    match status {
        403 => Error::Forbidden(message),
        404 => Error::NotFound(message),
        _ => Error::Api { status, message },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_come_out_of_either_error_shape() {
        assert_eq!(
            message(r#"{"error":{"status":403,"message":"Forbidden."}}"#),
            "Forbidden."
        );
        assert_eq!(
            message(r#"{"error":"invalid_grant","error_description":"Refresh token revoked"}"#),
            "Refresh token revoked"
        );
        assert_eq!(message(r#"{"error":"invalid_client"}"#), "invalid_client");
        assert_eq!(
            message("<html>bad gateway</html>"),
            "<html>bad gateway</html>"
        );
        assert_eq!(message(""), "no details");
        assert_eq!(
            oauth_code(r#"{"error":"invalid_grant"}"#).as_deref(),
            Some("invalid_grant")
        );
        assert_eq!(oauth_code(r#"{"error":{"status":1}}"#), None);
    }

    #[test]
    fn statuses_map_to_kinds() {
        let body = r#"{"error":{"status":0,"message":"m"}}"#;
        assert_eq!(from_status(403, body), Error::Forbidden("m".into()));
        assert_eq!(from_status(404, body), Error::NotFound("m".into()));
        assert_eq!(
            from_status(502, body),
            Error::Api {
                status: 502,
                message: "m".into()
            }
        );
        let stranger = Error::Forbidden(
            "Check settings on developer.spotify.com/dashboard, the user may not be registered."
                .into(),
        );
        assert!(
            stranger.to_string().contains("User Management"),
            "{stranger}"
        );
        assert!(Error::LoginExpired.needs_login());
        assert!(!Error::Offline("x".into()).needs_login());
    }
}
