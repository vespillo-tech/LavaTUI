# Spotify Web API

LavaTUI's library features (playlist browser, add to playlist, like/unlike,
search, "more like this") talk to the Spotify Web API through
`src/spotify_web/`. Playback control of the desktop app is separate and
needs none of this.

## Setup (once)

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

Logging in opens your browser on Spotify's consent page. After you agree,
the browser shows a "connected" page and LavaTUI has the login. It is kept
in the OS credential store (macOS Keychain, Windows Credential Manager,
Secret Service on Linux), or, if there isn't one, in
`<data dir>/lavatui/spotify-tokens.json`, readable by you only. Logging out
deletes both.

## Spotify's rules (checked 2026-10-01)

Sources: developer.spotify.com docs, Web API changelog (Feb + Mar 2026),
Feb 2026 migration guide, the 2026-06-18 refresh-token blog post.

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
    these library endpoints (the player endpoints, which we don't use,
    are Premium-only).
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

The playlist browser should only offer to open or add to playlists where
`Playlist::editable_by(&me)` is true (owned or collaborative). Other
playlists come back `Forbidden`.

Error handling: 401 refreshes the token and retries once. A 429 with
`Retry-After` ≤ 3 s is waited out once; longer ones come back as
`Error::RateLimited { retry_after }`. A 5xx is retried once after 0.5 s.
Network failures are `Error::Offline`, 403 is `Forbidden` and 404 is
`NotFound`.

## Using it from the UI

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
`ArtistTracks { artist }`. They run one at a time, in order, on one worker
thread (blocking `ureq`, no async runtime).

Tests: `cargo test` covers it all with a scripted HTTP layer. Against the
real thing: `cargo test -- --ignored live_keyring` (OS keyring round trip)
and `LAVATUI_SPOTIFY_CLIENT_ID=… cargo test -- --ignored --nocapture
live_account`. The second one logs in through the browser, reads your
profile, playlists and one playlist you own, and likes then unlikes the
track playing in Spotify (putting back its liked state). It also adds that
track to a private "lavatui test" playlist, which it creates and leaves
behind. It changes nothing else.

Notes:

- On macOS, a rebuilt debug binary is a "different app" to the Keychain,
  so the first token read after a rebuild may show a Keychain prompt
  ("Always Allow" silences it until the next rebuild).
- The callback server listens on `127.0.0.1:8731` only while a login is
  pending (up to 5 minutes). It ignores anything but `/callback` and
  rejects a `state` mismatch.
