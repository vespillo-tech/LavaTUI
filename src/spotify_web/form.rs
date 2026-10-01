//! Percent-encoding for query strings and `application/x-www-form-urlencoded`
//! bodies, and the reverse for the callback's query. Small enough not to be
//! worth a dependency.

use std::fmt::Write as _;

/// Percent-encodes everything but RFC 3986 unreserved characters
/// (`A-Z a-z 0-9 - . _ ~`), so the result is safe in a query value and in a
/// form body alike.
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// `k1=v1&k2=v2`, every key and value encoded.
pub fn pairs(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Decodes `%XX` escapes and `+` (a space in form encoding). A malformed
/// escape is kept as is; invalid UTF-8 is replaced.
pub fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                (Some(hi), Some(lo)) => {
                    out.push(hi << 4 | lo);
                    i += 2;
                }
                _ => out.push(b'%'),
            },
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    (b as char).to_digit(16).map(|d| d as u8)
}

/// The decoded `key=value` pairs of a query string (no leading `?`).
pub fn parse_query(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (k, v) = part.split_once('=').unwrap_or((part, ""));
            (decode(k), decode(v))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_keeps_unreserved_only() {
        assert_eq!(encode("aZ09-._~"), "aZ09-._~");
        assert_eq!(
            encode("spotify:track:1,2 x/y?&="),
            "spotify%3Atrack%3A1%2C2%20x%2Fy%3F%26%3D"
        );
        assert_eq!(encode("é"), "%C3%A9");
    }

    #[test]
    fn decode_round_trips_and_tolerates_junk() {
        for s in ["", "plain", "a b&c=d/é?%", "artist:\"Daft Punk\""] {
            assert_eq!(decode(&encode(s)), s);
        }
        assert_eq!(decode("a+b"), "a b");
        assert_eq!(decode("100%"), "100%");
        assert_eq!(decode("%zz%4"), "%zz%4");
    }

    #[test]
    fn parse_query_splits_pairs() {
        assert_eq!(
            parse_query("code=AQ%2Bx&state=s&&flag"),
            vec![
                ("code".into(), "AQ+x".into()),
                ("state".into(), "s".into()),
                ("flag".into(), String::new()),
            ]
        );
    }

    #[test]
    fn pairs_joins_encoded() {
        assert_eq!(pairs(&[("a", "1 2"), ("b:c", "")]), "a=1%202&b%3Ac=");
    }
}
