//! Authorization Code + PKCE (RFC 7636): the verifier/challenge pair, the
//! anti-CSRF `state`, and the authorize URL the browser is sent to.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

use super::form;
use super::{ACCOUNTS_BASE, REDIRECT_URI, SCOPES};

/// One login attempt's secrets. The verifier never leaves this process
/// except in the token exchange; the browser only sees the challenge.
#[derive(Debug, Clone)]
pub struct Pkce {
    pub verifier: String,
    pub state: String,
}

impl Pkce {
    /// Fresh random verifier (64 bytes → 86 base64url chars, inside the
    /// 43..=128 the spec allows) and state.
    pub fn new() -> Result<Self, getrandom::Error> {
        let mut verifier = [0u8; 64];
        let mut state = [0u8; 16];
        getrandom::fill(&mut verifier)?;
        getrandom::fill(&mut state)?;
        Ok(Self {
            verifier: URL_SAFE_NO_PAD.encode(verifier),
            state: URL_SAFE_NO_PAD.encode(state),
        })
    }

    /// `BASE64URL(SHA256(verifier))`, the `S256` method.
    pub fn challenge(&self) -> String {
        challenge(&self.verifier)
    }

    /// The `/authorize` URL for `client_id`.
    pub fn authorize_url(&self, client_id: &str) -> String {
        format!(
            "{ACCOUNTS_BASE}/authorize?{}",
            form::pairs(&[
                ("client_id", client_id),
                ("response_type", "code"),
                ("redirect_uri", REDIRECT_URI),
                ("scope", &SCOPES.join(" ")),
                ("code_challenge_method", "S256"),
                ("code_challenge", &self.challenge()),
                ("state", &self.state),
            ])
        )
    }
}

fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_matches_rfc7636_appendix_b() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn verifier_and_state_are_valid_and_fresh() {
        let a = Pkce::new().unwrap();
        let b = Pkce::new().unwrap();
        assert!(
            (43..=128).contains(&a.verifier.len()),
            "{}",
            a.verifier.len()
        );
        let allowed = |c: char| c.is_ascii_alphanumeric() || "-._~".contains(c);
        assert!(a.verifier.chars().all(allowed));
        assert!(a.state.chars().all(allowed));
        assert_ne!(a.verifier, b.verifier);
        assert_ne!(a.state, b.state);
    }

    #[test]
    fn authorize_url_carries_every_parameter() {
        let pkce = Pkce {
            verifier: "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into(),
            state: "st4te".into(),
        };
        let url = pkce.authorize_url("my-client");
        let (base, query) = url.split_once('?').unwrap();
        assert_eq!(base, "https://accounts.spotify.com/authorize");
        let q = form::parse_query(query);
        let get = |k: &str| q.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
        assert_eq!(get("client_id"), Some("my-client"));
        assert_eq!(get("response_type"), Some("code"));
        assert_eq!(get("redirect_uri"), Some("http://127.0.0.1:8731/callback"));
        assert_eq!(get("code_challenge_method"), Some("S256"));
        assert_eq!(
            get("code_challenge"),
            Some("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM")
        );
        assert_eq!(get("state"), Some("st4te"));
        let scope = get("scope").unwrap();
        for s in SCOPES {
            assert!(scope.split(' ').any(|x| x == *s), "{s} missing");
        }
    }
}
