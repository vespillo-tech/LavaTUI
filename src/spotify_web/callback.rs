//! The one-shot loopback server Spotify redirects the browser to:
//! `http://127.0.0.1:8731/callback?code=…&state=…`. It answers the browser
//! with a small page, checks `state`, and hands back the code.

use std::io::{self, BufRead as _, BufReader, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::Error;
use super::form;

/// Matches the registered redirect URI; Spotify only accepts loopback IP
/// literals (`localhost` is rejected), over plain HTTP.
pub const ADDR: &str = "127.0.0.1:8731";
pub const PATH: &str = "/callback";

/// How long to wait for the user to finish in the browser.
pub const TIMEOUT: Duration = Duration::from_secs(300);

const POLL: Duration = Duration::from_millis(50);

/// Binds the callback port now, so a busy port fails before the browser
/// opens.
pub fn bind() -> Result<TcpListener, Error> {
    TcpListener::bind(ADDR).map_err(|e| {
        Error::Login(if e.kind() == io::ErrorKind::AddrInUse {
            format!("port {ADDR} is busy (another login in progress?)")
        } else {
            format!("can't listen on {ADDR}: {e}")
        })
    })
}

/// What one request to the server means.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The authorization code, state checked.
    Code(String),
    /// Not the callback (a favicon, a stray probe): 404 it, keep waiting.
    Ignore,
    /// The callback, but the login failed.
    Fail(Error),
}

/// Interprets a request line (`GET /callback?… HTTP/1.1`).
pub fn parse(request_line: &str, expected_state: &str) -> Outcome {
    let mut parts = request_line.split_whitespace();
    let (Some("GET"), Some(target)) = (parts.next(), parts.next()) else {
        return Outcome::Ignore;
    };
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != PATH {
        return Outcome::Ignore;
    }
    let q = form::parse_query(query);
    let get = |k: &str| q.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
    if get("state") != Some(expected_state) {
        // Checked before anything else, so a forged redirect can't even
        // make us report its error.
        return Outcome::Fail(Error::Login(
            "state mismatch (stale or forged redirect)".into(),
        ));
    }
    if let Some(err) = get("error") {
        return Outcome::Fail(Error::Login(if err == "access_denied" {
            "access denied in the browser".into()
        } else {
            err.to_owned()
        }));
    }
    match get("code") {
        Some(code) if !code.is_empty() => Outcome::Code(code.to_owned()),
        _ => Outcome::Fail(Error::Login("no code in the callback".into())),
    }
}

/// Serves until the callback arrives, `cancel` is set, or `timeout` passes.
pub fn wait(
    listener: &TcpListener,
    expected_state: &str,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<String, Error> {
    listener
        .set_nonblocking(true)
        .map_err(|e| Error::Login(e.to_string()))?;
    let deadline = Instant::now() + timeout;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Login("cancelled".into()));
        }
        if Instant::now() >= deadline {
            return Err(Error::Login("timed out waiting for the browser".into()));
        }
        match listener.accept() {
            Ok((stream, _)) => match serve(stream, expected_state) {
                Outcome::Code(code) => return Ok(code),
                Outcome::Fail(err) => return Err(err),
                Outcome::Ignore => {}
            },
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(POLL),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(Error::Login(e.to_string())),
        }
    }
}

/// Reads one request, answers it, and says what it was.
fn serve(stream: TcpStream, expected_state: &str) -> Outcome {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    // Bounded: a request line longer than this isn't ours.
    if (&mut reader).take(16 * 1024).read_line(&mut line).is_err() {
        return Outcome::Ignore;
    }
    // Drain the headers: closing with unread input makes some stacks reset
    // the connection, and the browser would show an error, not our page.
    let mut header = String::new();
    for _ in 0..100 {
        header.clear();
        match (&mut reader).take(16 * 1024).read_line(&mut header) {
            Ok(n) if n > 0 && !header.trim_end().is_empty() => {}
            _ => break,
        }
    }
    let outcome = parse(line.trim_end(), expected_state);
    let (status, body) = match &outcome {
        Outcome::Code(_) => (
            "200 OK",
            page(
                "Connected",
                "LavaTUI is now connected to Spotify. You can close this tab and go back to your terminal.",
            ),
        ),
        Outcome::Fail(err) => (
            "400 Bad Request",
            page(
                "Login failed",
                &format!("{err}. Go back to LavaTUI and try again."),
            ),
        ),
        Outcome::Ignore => ("404 Not Found", page("Not found", "Nothing here.")),
    };
    let mut stream = reader.into_inner();
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.flush();
    outcome
}

/// A small page in the lamp's colours.
fn page(title: &str, message: &str) -> String {
    let esc = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>LavaTUI · {title}</title>
<style>
html,body{{height:100%;margin:0}}
body{{display:grid;place-items:center;background:#140b1f;color:#f6e7d8;
font:16px/1.5 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}}
.blob{{width:96px;height:96px;margin:0 auto 24px;border-radius:50%;
background:radial-gradient(circle at 35% 30%,#ffd166,#ff6b35 45%,#c1121f);
box-shadow:0 0 48px #ff6b3588;animation:f 4s ease-in-out infinite}}
@keyframes f{{50%{{transform:translateY(-12px) scale(1.04,.96)}}}}
main{{text-align:center;padding:16px;max-width:32em}}
h1{{font-size:20px;letter-spacing:.08em;text-transform:lowercase;margin:0 0 8px}}
p{{opacity:.8;margin:0}}
</style></head><body><main><div class="blob"></div>
<h1>{}</h1><p>{}</p></main></body></html>"#,
        esc(title),
        esc(message)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_matching_state() {
        assert_eq!(
            parse("GET /callback?code=AQ%2Bx&state=s1 HTTP/1.1", "s1"),
            Outcome::Code("AQ+x".into())
        );
    }

    #[test]
    fn parse_rejects_state_mismatch_before_anything() {
        for line in [
            "GET /callback?code=c&state=evil HTTP/1.1",
            "GET /callback?code=c HTTP/1.1",
            "GET /callback?error=access_denied&state=evil HTTP/1.1",
        ] {
            let Outcome::Fail(Error::Login(why)) = parse(line, "s1") else {
                panic!("{line}");
            };
            assert!(why.contains("state mismatch"), "{why}");
        }
    }

    #[test]
    fn parse_reports_denial_and_missing_code() {
        assert_eq!(
            parse("GET /callback?error=access_denied&state=s HTTP/1.1", "s"),
            Outcome::Fail(Error::Login("access denied in the browser".into()))
        );
        assert_eq!(
            parse("GET /callback?state=s&code= HTTP/1.1", "s"),
            Outcome::Fail(Error::Login("no code in the callback".into()))
        );
    }

    #[test]
    fn parse_ignores_other_requests() {
        for line in [
            "GET /favicon.ico HTTP/1.1",
            "POST /callback HTTP/1.1",
            "",
            "garbage",
        ] {
            assert_eq!(parse(line, "s"), Outcome::Ignore, "{line:?}");
        }
    }

    fn get(port: u16, target: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(s, "GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    }

    #[test]
    fn serves_until_the_callback_and_answers_the_browser() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let browser = std::thread::spawn(move || {
            let favicon = get(port, "/favicon.ico");
            let callback = get(port, "/callback?code=the-code&state=s");
            (favicon, callback)
        });
        let cancel = AtomicBool::new(false);
        let code = wait(&listener, "s", &cancel, Duration::from_secs(10));
        let (favicon, callback) = browser.join().unwrap();
        assert_eq!(code, Ok("the-code".into()));
        assert!(favicon.starts_with("HTTP/1.1 404"), "{favicon}");
        assert!(callback.starts_with("HTTP/1.1 200"), "{callback}");
        assert!(callback.contains("connected to Spotify"));
    }

    #[test]
    fn state_mismatch_ends_the_wait_with_an_error_page() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let browser = std::thread::spawn(move || get(port, "/callback?code=c&state=forged"));
        let cancel = AtomicBool::new(false);
        let result = wait(&listener, "s", &cancel, Duration::from_secs(10));
        let page = browser.join().unwrap();
        assert!(matches!(result, Err(Error::Login(why)) if why.contains("state mismatch")));
        assert!(page.starts_with("HTTP/1.1 400"), "{page}");
    }

    #[test]
    fn cancel_and_timeout_end_the_wait() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let cancel = AtomicBool::new(true);
        assert_eq!(
            wait(&listener, "s", &cancel, Duration::from_secs(10)),
            Err(Error::Login("cancelled".into()))
        );
        cancel.store(false, Ordering::Relaxed);
        assert!(matches!(
            wait(&listener, "s", &cancel, Duration::from_millis(1)),
            Err(Error::Login(why)) if why.contains("timed out")
        ));
    }

    #[test]
    fn page_escapes_html() {
        assert!(page("<t>", "a & \"b\"").contains("&lt;t&gt;"));
        assert!(page("t", "a & \"b\"").contains("a &amp; &quot;b&quot;"));
    }
}
