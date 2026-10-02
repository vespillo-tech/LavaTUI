# Generalizability review

Reviewed 2026-10-01 at `d8185d7` (main's first-run UX changes included), on
`ghostex/ensure-universal-spotify-support`. Review bead: **lava-1xk.22**.
Source, tests, tools, README, Spotify guide and design contract were audited.
No source edits or live Spotify account mutations were made. The findings below
are filed under the release epic and remain open.

## Ranked findings

1. **P1 — lava-1xk.24: Linux/Windows cannot like or add the playing Spotify track.**
   `src/media/mpris.rs:149` uses a D-Bus object path or text identity;
   `src/media/smtc.rs:100` uses title/artist/album. The library requires
   `spotify:track:` (`src/app/model/library.rs:357,640`). Backend fixtures confirm
   these incompatible formats; library fakes use the accepted format and hide
   the integration gap. Keep backend identity for seeking and add a separate
   canonical Spotify URI. Normalize Linux `xesam:url`; Windows needs matched
   authorized Web playback identity or an explicit limitation. Test actual
   backend-shaped snapshots through like/contains/add, including non-Spotify
   and local tracks.

2. **P1 — lava-1xk.25: Windows can claim library playback succeeded without playing.**
   SMTC discards URI/context commands (`src/media/smtc.rs:331`), but
   `Music::play` returns success after queueing (`src/app/model/music.rs:146`).
   The library's desktop fallback then says “playing”
   (`src/app/model/library.rs:1106,1131`). Expose playback capabilities and show
   an actionable unsupported message; use Web playback when available. Also
   correct README's blanket playlist-continuation promise (`README.md:254`):
   Linux's context fallback opens only the selected track
   (`src/media/mpris.rs:235`).

3. **P1 — lava-1xk.26: Setup omits eligibility before sending beginners to Create app.**
   README's library introduction (`README.md:91`) and guided setup
   (`src/app/model/settings_screen.rs:110,626`) omit the developer owner's
   Premium requirement and new-app five-user allowlist. The fresh-home guide
   confirms this omission. Explain these before step 1, along with own app +
   Client ID, and distinguish OAuth success from allowlist API failures.
   `docs/spotify.md:74` also says player endpoints are unused despite the
   implementation and its own endpoint table. Bring all three surfaces into
   agreement with current rules and platform capabilities.

4. **P2 — lava-1xk.27: Spotify account modes can overwrite another player's UI.**
   `src/app/model/library.rs:625` applies Web shuffle/repeat and capabilities
   without matching the selected desktop source/track. Native-capable keys use
   fresh native state, but unsupported native toggles fall back to Spotify Web
   commands (`src/app/model/music.rs`, `player_key`). A Linux/Windows browser or
   other media session can show remote Spotify modes, or direct a toggle to that
   unrelated device. Gate Web integration by matching Spotify playback identity;
   test mixed players and switches.

5. **P2 — lava-1xk.28: Opt-in live tests assume macOS and inconsistent token files.**
   `src/spotify_web/tests.rs:899,1093,1198` use `osascript` without platform
   gating. Some honor `LAVATUI_LIVE_TOKEN_FILE`; others hardcode a shared temp
   store. Use one isolated, documented fixture with explicit test resources;
   separate macOS AppleScript tests from platform-neutral HTTP tests. These tests
   are ignored by default and were not run with a real account.

## Spotify rules checked against official documentation

A new development app's owner must maintain Premium; it permits up to five
allowlisted users. A user outside that list can authorize successfully but get
API 403 responses. Existing over-limit apps can be grandfathered.
[Quota modes](https://developer.spotify.com/documentation/web-api/concepts/quota-modes),
[February migration guide](https://developer.spotify.com/documentation/web-api/tutorials/february-2026-migration-guide).
The developer Client ID limit rose to 25 in July 2026.
[July changes](https://developer.spotify.com/documentation/web-api/references/changes/july-2026).

Web playback requires Premium and a playback device; this is distinct from
basic local desktop control.
[Start playback](https://developer.spotify.com/documentation/web-api/reference/start-a-users-playback).
The configured `http://127.0.0.1:8731/callback` meets the explicit loopback rule;
PKCE uses a Client ID without a client secret.
[Redirect URI rules](https://developer.spotify.com/documentation/web-api/concepts/redirect_uri).

The code supports basic desktop playback, cover display and lyrics without a
Client ID or Web login. These still depend on a working media backend; art and
lyrics need available content and network access. Media is off initially.
Library features require each user's developer setup; “any Spotify account”
cannot truthfully mean that every account can create and use a new developer
app. The lamp, clock and timer have no Spotify dependency.

## Verification and remaining limits

- After the requested fast-forward from main: release build, formatting and
  Clippy passed; `env -u NO_COLOR cargo test` passed **567**, ignored **17**.
  The pre-merge revision had five cover-test failures with inherited
  `NO_COLOR=1`; main's existing **lava-1xk.16** fix resolves them. All **20** music
  model tests subsequently passed with `NO_COLOR=1` still inherited.
- Release binary started without `--config` or any existing file, in separate
  temporary HOME/XDG directories: 80×24, 20×8, 1×1, 300×90, NO_COLOR, C locale
  with TERM=vt100, first setting change, and guided Spotify setup (120×36).
  All eight returned zero and restored alternate screen/cursor. Passive starts
  wrote no files; the first change saved only the scratch config. Inspected
  rendered welcome, tiny and guided-setup captures. No authorization was opened.
- Non-TTY launch exits 1 with a clear terminal requirement; help exits 0 and an
  invalid style exits 2. Core runtime paths use platform/XDG directories.
  Screenshot/performance tools explicitly require Unix PTYs; screenshot fonts
  have an override. Performance numbers identify their measured hardware.
- This is macOS PTY/emulator verification, not a claim about every terminal's
  fonts/protocols or native Linux/Windows services. Real-device media validation
  remains **lava-75z.15**. Personal fixture/identity cleanup is already tracked
  by **lava-1xk.23**; no identifying values are reproduced here.

Temporary smoke evidence: `lavatui-generalizability-6798wldi` in the system temp
folder (results JSON, ANSI, text and PNG captures). Nothing was pushed.
