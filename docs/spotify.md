# Spotify Web API

LavaTUI's library features (playlist browser, add to playlist, like/unlike,
search, "more like this") talk to the Spotify Web API through
`src/spotify_web/`. Now playing, playback control of the desktop app, the
cover and lyrics are separate and need none of this.

## Who can use it (check before step 1)

Spotify's development-mode rules (checked 2026-10-01; see the sources
below):

- You need **your own Spotify developer app**, and LavaTUI needs only its
  Client ID (PKCE: no Client Secret).
- **The app's owner needs an active Spotify Premium subscription**, or
  the app stops working for everyone.
- At most **5 users in total, the owner included**, each added by
  email under *User Management*. Anyone else can complete the login, but
  every API request then returns 403; the app shows "Spotify refused this
  account" and the setup page says what to fix.
- Sharing a Client ID only helps people on that list; extra Client IDs
  (up to 25 per developer) don't raise the limit for one app.
- The player endpoints (shuffle / repeat, playing a playlist through
  Spotify) also need Premium on the logged-in account and an active
  device.

## Setup (once)

The app walks you through this: press `,` and pick *music & lyrics* →
*spotify*. It opens the dashboard, copies the redirect address, takes the
Client ID by paste (and checks it), then logs in. The same steps by hand:

1. Go to <https://developer.spotify.com/dashboard>, log in, and click
   **Create app**.
2. Fill in a name (e.g. `LavaTUI`) and a description; both appear on the
   consent page you'll see when logging in.
3. **Redirect URI**: add exactly

   ```
   http://127.0.0.1:8731/callback
   ```

   (plain `http` is allowed only for loopback IPs; `localhost` is
   rejected by Spotify, so it must be `127.0.0.1`).
4. **Which API/SDKs**: tick **Web API**. Accept the terms and save.
5. Open the app's **Settings** and copy the **Client ID**. You don't
   need the Client Secret: LavaTUI uses PKCE and never asks for it.
6. Give LavaTUI the Client ID, either in `config.toml`

   ```toml
   [spotify]
   client_id = "your-client-id"
   ```

   or in the environment: `LAVATUI_SPOTIFY_CLIENT_ID=your-client-id`.
7. To let anyone else use your Client ID, add their Spotify account email
   under **User Management** (development mode allowlist, max 5 users).
   You don't need to add yourself, as the app owner.

Troubleshooting: the login works but lists say "Spotify refused this
account": the account isn't on the allowlist, or the owner has no
Premium (fix it on the dashboard, then disconnect and connect again in
the setup). The consent page complains about the redirect URI: it must be
exactly the address in step 3.

Logging in opens your browser on Spotify's consent page. After you agree,
the browser shows a "connected" page and LavaTUI has the login. It is kept
in the OS credential store (macOS Keychain, Windows Credential Manager,
Secret Service on Linux), or, if there isn't one, in
`<data dir>/lavatui/spotify-tokens.json`, readable by you only. Logging out
deletes both. `spotify.store = "file"` (the setup's *keep the login in* ›
*private file*) always uses that file; switching moves a saved login
across. `LAVATUI_SPOTIFY_TOKEN_FILE=/path/tokens.json` keeps it in that
file (0600) instead, never touching the keyring: for headless runs,
scripted screenshots, or a macOS Keychain that asks again after every
rebuild.

On macOS the Keychain is read only when the login is first needed in a
session: the first like, add, playlist browse, Web API shuffle / repeat,
or opening the Spotify setup (lava-1xk.38). A toast says first that macOS
may ask and to choose *Always Allow*; the key then runs once the login is
read (it gives up after 30 s). Until then `spotify.logged_in`, a plain
flag the app keeps in the config, is what shows "connected". The Keychain
ties *Always Allow* to the program's code signature, and an ad-hoc signed
build is a new program after every update; a Developer ID signed release
(see `docs/architecture.md`, Releases) keeps it. Elsewhere the login is
read at start, as no other store asks.

## Spotify's rules (checked 2026-10-01)

Sources: developer.spotify.com docs, Web API changelog (Feb + Mar 2026),
Feb 2026 migration guide, the 2026-06-18 refresh-token blog post.
Quota modes: <https://developer.spotify.com/documentation/web-api/concepts/quota-modes>;
Feb 2026 migration guide:
<https://developer.spotify.com/documentation/web-api/tutorials/february-2026-migration-guide>;
July 2026 changes:
<https://developer.spotify.com/documentation/web-api/references/changes/july-2026>.

- **Auth**: Authorization Code with PKCE is the recommended flow for apps
  that can't keep a secret. Verifier is 43 to 128 chars of
  `[A-Za-z0-9-._~]`, challenge `S256`. Token and refresh requests are
  form-encoded `POST https://accounts.spotify.com/api/token` with
  `client_id` in the body. Access tokens last 3600 s. A refresh **may or
  may not** return a new refresh token; keep the old one if not.
- **Refresh tokens expire 6 months after the user authorized** (new apps
  since 2026-06-18, existing apps since 2026-07-20). Refreshing does
  **not** reset the clock. An expired one gets `400 invalid_grant`, which
  means: discard it and log in again. LavaTUI then emits
  `Event::LoggedOut { expired: true }`.
- **Redirect URIs**: HTTPS, except loopback, where HTTP is fine. Loopback
  must be the IP literal (`127.0.0.1` or `[::1]`); `localhost` is not
  allowed.
- **Development mode** (every personal app; extended quota is for
  organizations with 250k+ MAU only, since 2025-05-15):
  - **The app owner needs an active Spotify Premium subscription**, or
    the app stops working (since 2026-02-11 for new apps and 2026-03-09
    for existing ones). Users other than the owner don't need Premium for
    these library endpoints (the player endpoints, used for shuffle /
    repeat and playing a playlist, are Premium-only).
  - Up to **5 users** per app, each added to the allowlist by hand.
  - Up to 25 Client IDs per developer (raised from 1 in July 2026). The
    quota is counted per account, not per Client ID.
  - Lower rate limits. A 429 carries `Retry-After` (seconds), over a
    rolling 30 s window.
- **Removed for development-mode apps (Feb 2026)**, among others:
  `GET /artists/{id}/top-tracks`, `GET /tracks` (several), `GET /users/{id}`
  and `GET /users/{id}/playlists`, browse/new releases, markets. Also
  removed are the per-type library endpoints (`PUT/DELETE /me/tracks`,
  `GET /me/tracks/contains`, the follow endpoints), now
  **`PUT /me/library`, `DELETE /me/library` and
  `GET /me/library/contains`** with `uris=spotify:track:…` (comma
  separated, at most 40). Recommendations, related artists and audio
  features were already gone (Nov 2024).
- **Playlists**: `/playlists/{id}/tracks` is now **`/playlists/{id}/items`**
  (GET, POST, PUT, DELETE), and the entry's object moved from `track` to
  `item`. The playlist count is `items.total` (`tracks` is deprecated).
  **Items are only returned for playlists the user owns or collaborates
  on**; for others you get metadata only. Add takes up to 100 URIs per
  call and returns a `snapshot_id`.
- **Search**: `limit` max 10 (was 50), default 5; `offset` up to 1000.
- **Removed fields**: `popularity` (tracks, albums, artists), `followers`,
  `available_markets`, `linked_from`, `label`, and on the user `email`,
  `country`, `product`, `explicit_content`. `external_ids` came back in
  Mar 2026.
- **Terms**: Developer Terms v10 (2025-05-15) forbid using Spotify content
  to train or feed ML/AI models.

## What LavaTUI does with that

| Feature | Endpoint | Scope |
|---|---|---|
| Who am I | `GET /me` | none |
| My playlists (all pages, 50 each) | `GET /me/playlists` | `playlist-read-private`, `playlist-read-collaborative` |
| Playlist contents (page of 50) | `GET /playlists/{id}/items` | `playlist-read-private` |
| New playlist | `POST /me/playlists` (`POST /users/{id}/playlists` is gone) | `playlist-modify-public` / `-private` |
| Add to playlist (100 per call) | `POST /playlists/{id}/items` | `playlist-modify-public`, `playlist-modify-private` |
| Liked? (40 per call) | `GET /me/library/contains` | `user-library-read` |
| Like / unlike (40 per call) | `PUT` / `DELETE /me/library` | `user-library-modify` |
| Search tracks (max 10) | `GET /search?type=track` | none |
| More like this | `GET /search?q=artist:"…"` (top tracks is gone) | none |
| Player state (shuffle / repeat / device) | `GET /me/player` (204: nothing playing) | `user-read-playback-state` |
| Shuffle / repeat | `PUT /me/player/shuffle?state=…`, `PUT /me/player/repeat?state=off\|context\|track` | `user-modify-playback-state` (Premium) |
| Play a track in a playlist | `PUT /me/player/play` `{context_uri, offset: {uri}}` | `user-modify-playback-state` (Premium) |

The player endpoints need Premium (a non-Premium account gets `403
PREMIUM_REQUIRED`, which hides shuffle / repeat until the next login).
Logins made before the playback scopes were added lack them (`403
Insufficient client scope`); log out and in again (`A`, `i` `i`, `i`).

The playlist browser should only offer to open or add to playlists where
`Playlist::editable_by(&me)` is true (owned or collaborative). Other
playlists come back `Forbidden`.

Error handling: 401 refreshes the token and retries once. A 429 with
`Retry-After` ≤ 3 s is waited out once; longer ones come back as
`Error::RateLimited { retry_after }`. A 5xx is retried once after 0.5 s.
Network failures are `Error::Offline`, 403 is `Forbidden` and 404 is
`NotFound`.

## In the app

All of it lives in the player keys (`A`, with the music widget placed):
`i` log in (browser; `i` again cancels; logged in, `i` twice logs out),
`b` the playlist browser (`/` filters it by name; `⏎` on a track plays
it in its playlist through the Web API's player when it's available,
else through the desktop app: on macOS in its playlist, on Linux the
track alone (MPRIS has no playlists), and on Windows not at all (the
media controls can't be told what to play), which the toast says), `a` add the playing track to a playlist, `s`
like / unlike (the `♥` in the widget), and `x` / `r` shuffle / repeat
through the Web API when Spotify allows them. With the mouse on, the
widget's `log in`, `♡`, `+` and `≡` do the same. See docs/design.md §4.4
and §4.6.

Like and add act on the playing track's Spotify URI (`Track::uri`, kept
apart from the player's own id, `Track::id`): macOS's AppleScript reports
it; on Linux it comes from MPRIS `xesam:url` (or an old-style trackid),
while the MPRIS object path stays the id for seeking; Windows' media
controls report none, so the model uses the Web API player's item only
while the Spotify app shows a track of the same name (lava-1xk.24). Local
files, ads, episodes and other players get no URI: like and add say
there's nothing to act on. The Web API's shuffle / repeat likewise apply
only while its player is playing the track shown (lava-1xk.27); another
player keeps its own modes and keys.

## Using it from the code

```rust
use crate::spotify_web::{SpotifyWeb, Request, Reply, Event, client_id_from_env};

let mut spotify = SpotifyWeb::new(client_id);   // starts the worker thread
spotify.is_logged_in();                         // saved login, picked up async
let url = spotify.login()?;                     // opens the browser; show `url` too
spotify.cancel_login();
spotify.logout();
let id = spotify.request(Request::MyPlaylists); // -> RequestId, never blocks
// once per frame:
while let Some(event) = spotify.poll() {
    match event {
        Event::Reply { id, result: Ok(Reply::Playlists(lists)) } => { /* … */ }
        Event::Reply { result: Err(e), .. } if e.needs_login() => { /* … */ }
        Event::LoggedIn { saved } | Event::LoginFailed(_) | Event::LoggedOut { .. } => {}
        _ => {}
    }
}
```

Requests: `Me`, `MyPlaylists`, `PlaylistTracks { playlist_id, offset }`,
`CreatePlaylist { name, public }`, `AddToPlaylist { playlist_id, uris }`,
`LibraryContains { uris }`,
`Like { uris }`, `Unlike { uris }`, `SearchTracks { query, limit, offset }`,
`ArtistTracks { artist }`, `Player`, `SetShuffle(bool)`,
`SetRepeat(Repeat)`, `Play { context_uri, offset_uri }`. They run one at a time, in order, on one worker
thread (blocking `ureq`, no async runtime).

Tests: `cargo test` covers it all with a scripted HTTP layer and a fake
account; nothing there touches a real one. The opt-in live tests
(`src/spotify_web/live_tests.rs`, all ignored by default) run against your
own account on any OS, and take everything from the environment:

| Variable | What for |
|---|---|
| `LAVATUI_SPOTIFY_CLIENT_ID` | your app's Client ID (required) |
| `LAVATUI_SPOTIFY_TOKEN_FILE` | where the login is kept (never the keyring); default a temp file per Client ID. Use one file per account. |
| `LAVATUI_LIVE_CHANGES=1` | consent to the tests that change something; without it they stop before the first change |
| `LAVATUI_TEST_PLAYLIST` | id of a playlist of yours they may add to and play (else your "lavatui test") |
| `LAVATUI_TEST_TRACK` | the `spotify:track:…` to like, unlike and add (else what your account is playing) |

- `live_token_endpoint` (network only, no account) and `live_peek`
  (read only) change nothing.
- `live_account` logs in through the browser (once per token file), reads
  your profile, playlists and one playlist you own, likes then unlikes the
  test track (putting back its liked state) and adds it to the test
  playlist, creating a private "lavatui test" if there's none. It changes
  nothing else.
- `live_library` does what the library screen does: the same like and add,
  then flips shuffle and repeat on your active device for a moment and
  puts them back (Premium).
- `live_play_in_context` and `live_player_in_a_playlist` also drive the
  macOS desktop app (AppleScript), so they are macOS only.
- `live_keyring` round-trips the OS keyring.

```sh
LAVATUI_SPOTIFY_CLIENT_ID=… LAVATUI_LIVE_CHANGES=1 \
  cargo test -- --ignored --nocapture live_account
```

Notes:

- On macOS, a rebuilt debug binary is a "different app" to the Keychain,
  so the first token read after a rebuild may show a Keychain prompt
  ("Always Allow" silences it until the next rebuild). Use
  `LAVATUI_SPOTIFY_TOKEN_FILE` or `spotify.store = "file"` while
  developing.
- The callback server listens on `127.0.0.1:8731` only while a login is
  pending (up to 5 minutes). It ignores anything but `/callback` and
  rejects a `state` mismatch.
