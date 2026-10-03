# How LavaTUI works

Technical notes for contributors and the curious: how the code fits
together, what each platform supports, performance numbers and how the
README's pictures are made. The user guide is the [README](../README.md);
the layout and visual rules are in [design.md](design.md); the Spotify
Web API details are in [spotify.md](spotify.md).

## The big picture

LavaTUI is one Rust binary built on [ratatui](https://ratatui.rs) and
crossterm. Every frame it:

1. reads input until the frame is due (keys and mouse become `Action`s
   through one key table, `src/ui/keymap.rs`, which also drives the help
   screen);
2. updates the app `Model` (`src/app/model/`): settings, the wax
   simulation, the clock and timer, music, lyrics, the overlays;
3. lays the screen out (`src/ui/layout.rs`, a pure function of the
   window size and what's placed where);
4. samples the wax field at the style's resolution and draws it
   (`src/render/`), then the widgets and chrome (`src/ui/`, `src/dock/`);
5. sends the changed cells in one synchronized update.

Everything except drawing and the main loop is pure and unit-tested: the
simulation, layout, clock, pomodoro, keymap and the app model.

| Folder | What's in it |
|---|---|
| `src/sim/` | The wax: heat, buoyancy, drag, cohesion, merging and splitting, at a fixed 120 steps a second, seeded and deterministic. `field.rs` turns blobs into a density field. |
| `src/render/` | The render pipeline and the nine styles (`styles/`, one file each, all implementing `LampStyle`). |
| `src/theme/` | Palettes and colour depth. The only place colours are decided: truecolor → 256 (perceptual match, dithering for dark tints) → 16 → none. |
| `src/clock/` | Clock faces (`Face` trait) and the pomodoro state machine. |
| `src/dock/` | The widgets that sit beside the lamp or on it: clock, timer, music, lyrics, cover (`DockWidget` trait). |
| `src/ui/` | Layout, keymap, help, pickers, the settings screen, status bar. The only terminal-facing drawing code. |
| `src/app/` | The main loop (`mod.rs`) and the model (`model/`). |
| `src/config/` | `config.toml`: tolerant loading, safe saves that keep your comments. |
| `src/media/` | Now playing: one `MediaSource` trait, a backend per platform, a fake for tests and `--demo`, and the album-art loader. |
| `src/lyrics/` | LRCLIB client, cache, LRC parser, word timing (`words.rs`) and the line / word syncer. |
| `src/spotify_web/` | The optional Spotify Web API client (PKCE login, library, player). |
| `src/graphics/` | Album covers as real pictures: kitty graphics, iTerm2 images and sixel, with a start-up check of what the terminal really supports. |

Slow work never runs on the frame: saving, the music player, cover
downloads, lyrics lookups and Spotify requests each have their own
worker thread, and a frame only reads their latest result.

[`CLAUDE.md`](../CLAUDE.md) (from its project section on, the same text
as `AGENTS.md`; only the generated issue-tracker notes at the top
differ) has the full
module map and step-by-step guides for adding a render style, a clock
face, a widget or a key.

## Platforms

The lamp, clock, timer and settings work the same everywhere; config,
cache and data go where each OS expects them. Only now playing differs:

| | macOS | Linux | Windows |
|---|---|---|---|
| Builds in CI | ✓ | ✓ | ✓ |
| Tested on real hardware | ✓ | in Docker with a stand-in MPRIS player (`tools/linux/run.sh`); not yet on a desktop | not yet |
| Now playing via | AppleScript, one long-lived `osascript` for every player | MPRIS on the D-Bus session bus (zbus) | System Media Transport Controls |
| Players | the Spotify desktop app, Apple Music (not browsers, Podcasts, VLC…: see below) | any MPRIS player | any app in the media flyout |
| Which one, when several are open (`media/choice.rs`) | Spotify while it plays, else one that plays, else the one in use | same | same, and the session Windows calls current before an idle Spotify |
| Play/pause, next/previous, seek | ✓ | ✓ | ✓ |
| Volume | ✓ | ✓ if the player has it | – (SMTC has no volume) |
| Shuffle / repeat | Music: ✓ if it honours them; Spotify: through the Web API only (its AppleScript setters do nothing) | ✓ if the player honours them (Spotify: through your account) | ✓ if the app honours them |
| Cover art | ✓ (Spotify's URL; Music's picture bytes) | ✓ (`https` art URLs, so Spotify) | ✓ (the session's thumbnail) |
| Like / add the playing song (library setup) | ✓ Spotify songs | ✓ Spotify songs | ✓ Spotify songs, once your account's player reports the same song |
| Play from the playlist browser | ✓ in Spotify (while it's open), the rest of the playlist follows; another player playing is paused | with Premium and Spotify playing: ✓; otherwise just that song | with Premium and Spotify playing: ✓; otherwise it says so |
| Launches the player? | never | never | never |
| Permission | macOS asks once per player (Automation) | none | none |

Linux players vary: Spotify has long reported its position as 0 over
MPRIS (the bar then counts on from where it was first seen) and ignored
shuffle and repeat. If a player ignores shuffle or repeat, the app
notices, says so and stops offering them; for Spotify, logging in to
your account (with Premium) makes them work. Anything a player leaves
out falls back quietly. Windows' media controls don't say which Spotify
song is playing, so there the app asks Spotify's servers what your
account is playing and uses it only when it's clearly the same song: the
same title and artist, and the same length or album. They also can't be told what to play, so
the playlist browser plays through Spotify's servers there, which needs
Premium and Spotify open on a device.

When something goes wrong for a moment, it comes back by itself. A
player that misses an answer or two (Spotify can take a second over a
track change) keeps what's shown and is asked again at once; only a
third miss in a row shows the problem. A song first read with details
missing is read again for a few seconds. A cover download that fails is
retried after 1 s and 4 s, then every 30 s while the song plays; lyrics
that couldn't be fetched are asked for again every 30 s; a failed "is
this song liked?" waits 5 s (or as long as Spotify asks) before asking
again. Songs LRCLIB has no lyrics for say so, and are asked about again
the next day.

### macOS: Spotify, Apple Music, and why not every app

`media/players.rs` asks every player each poll and follows one by the
shared rule (`media/choice.rs`). Both talk to one `osascript` process
(`media/applescript.rs`): each request names its app's bundle id, the
script checks `application id … is running` first, and only then
compiles that app's part at run time (`run script`), because compiling a
`tell application` block launches the app. A player that isn't open
costs one such check. Commands go to the player followed; a Spotify URI
from the library goes to Spotify (while it's open) wherever the keys
are, and a non-Spotify player that's playing is paused first, so
Spotify takes over. Both apps' change notifications
(`com.spotify.client.PlaybackStateChanged`, `com.apple.Music.playerInfo`)
reach one listener.

Apple Music (`media/apple_music.rs`) reads state, position, shuffle
(`shuffle enabled`), repeat (`song repeat`: off is off, one or all is
on; repeat on sets all), volume and the current track: its `persistent
ID` (plus the stream title on a radio station, so each song there is a
new track), name, artist, album and length. Music has no artwork URL:
on a new track the script writes `raw data of artwork 1` to a private
temporary file, the worker reads it, deletes it and hands the bytes to
the art loader (`art::stash`, as on Windows). A song whose cover isn't
there yet is read again for a few seconds. If Music is seen to ignore a
shuffle or repeat change (`media/modes.rs`, shared with MPRIS), they
stop being offered. Lyrics and the karaoke timing use the same
`Baseline` and change events as Spotify. Its part is checked against
Music's own scripting dictionary without Music: an ignored test builds a
stand-in app carrying a copy of it
(`music_part_compiles_against_musics_dictionary`); `live_music` reads
the real app when it's open (and changes volume, play/pause, shuffle and
repeat and puts them back with `LAVATUI_LIVE_CHANGES=1`). Live on macOS
26.6 (2026-10-03, a library song loaded and paused at volume 0): the
song's details, length and cover came through; play/pause, volume,
shuffle and repeat were all honoured (~60-75 ms a request). With nothing
loaded Music accepts shuffle and repeat but doesn't change them, so
only changes made with a song loaded count when deciding whether Music
ignores them. Volume is exact except that
1 reads back as 0. Seek, next and previous weren't tried live (the same
AppleScript as Spotify's, checked against Music's dictionary).

**Other apps (browsers, Podcasts, VLC…) are not shown.** macOS's own
"Now Playing" lives in the private MediaRemote framework. Since macOS
15.4 only Apple-signed processes may read it; the known workarounds run
through such a process. Checked on macOS 26.6 (2026-10-03):

- [mediaremote-adapter](https://github.com/ungive/mediaremote-adapter)
  (BSD-3-Clause) loads a helper framework into `/usr/bin/perl`. It works
  and can send commands, but it means building and shipping an
  Objective-C framework beside the binary, and Apple has said since
  10.15 that scripting runtimes like Perl won't stay in macOS by
  default.
- `osascript` itself gets the same access: a JXA script that loads
  `MediaRemote.framework` and asks `MRNowPlayingRequest` read the
  playing app, its state, position and length in ~50 ms with no
  permission prompt, and `ObjC.bindFunction` binds
  `MRMediaRemoteSendCommand`. Nothing to install.

Both lean on undocumented, private API through a loophole Apple already
narrowed once (15.4), with no stable track ids and no documented
change events; a macOS update can break them silently. So they're not
in this release. The `osascript` route is the one worth trying first,
as an opt-in fallback for players the AppleScript backends don't cover
(lava-75z.32).

For problems that come and go, `LAVATUI_MEDIA_LOG=<file>` writes one line
per event to that file: a missed player answer, a song read without some
details, a cover or lyrics lookup that failed, a library lookup that
failed. Songs appear only as a short hash, never by name.

## Pictures in the terminal

The album cover is a real picture where the terminal can show one:
the kitty graphics protocol (kitty, Ghostty) with Unicode placeholders,
sent once per track and size in chunks of at most 96 KB a frame; iTerm2
inline images (iTerm2, WezTerm) and sixel (foot, mlterm, Konsole, …),
sent once when the cover appears, moves or changes size. LavaTUI asks the
terminal once at start whether it really supports what its environment
promises, and draws the cover in text cells until it says yes. Inside
tmux, screen, zellij or Ghostex it doesn't try. Elsewhere covers are text
cells: sextants (2×3 pixels a cell) or quadrants (2×2), with the best
two colours per cell. The small, medium and big pixels cover qualities are
pixel art: where pictures are shown, a small PNG of exactly 32, 16 or 10
flat squares across, made when the cover or the quality changes; else
square blocks of whole and half cells, about as many (fewer on a small
cover). While a picture is on its way the cover is never drawn in text
cells instead: the picture already up stays if it's the same size, else
the spot is a blank tile in the cover's colour. `LAVATUI_GRAPHICS` overrides
the choice. `tools/kitty_check.py` and `tools/inline_check.py` show the
exact bytes sent.

## Performance

Measured on an Apple M5 laptop under background load (load average 3–5),
so treat them as rough. The CPU and output numbers are from before v1.1
(release binary in a pty, truecolor, 60 fps, 15 s each); render times
are `bench_lamp` on v1.1.

| Measurement | Result |
|---|---|
| Launch → first frame → exit (`--frames 1`, 80×24) | ~30 ms. The simulation starts pre-warmed. |
| CPU at 80×24, solid or braille | ~2.3 % of one core |
| CPU at 200×60, solid / braille | 4.6 % / 4.9 % |
| Output at 80×24 / 200×60, solid | ~10 KB/s / ~55 KB/s |
| Output in braille (80×24 / 200×60) | ~5 KB/s / ~21 KB/s |

Render time per frame at 200×60 in truecolor (field sampling plus the
style's draw, two simulation steps per frame):

| Style | Time | | Style | Time |
|---|---|---|---|---|
| solid | 0.23 ms | | synthwave | 0.50 ms |
| outline | 0.33 ms | | matrix | 0.12 ms |
| ascii | 0.14 ms | | topo | 0.70 ms |
| braille | 0.39 ms | | chrome | 0.41 ms |
| halftone | 0.14 ms | | | |

Every style stays under 0.8 ms at 200×60, in truecolor and in 256
colours. The design target is 8 ms. If frames ever get slow, adaptive
quality first lowers the sample grid and then halves the frame rate
(never below 30 fps); it recovers on its own and never changes your
settings. An unfocused window drops to 10 fps, and a paused lamp sleeps
until the clock or timer changes. A paused lamp that still redraws for
music or lyrics keeps its last cells instead of drawing the lamp again
(200×60 with the demo's music, lyrics and cover beside it: 1.4–1.7 % →
0.75–0.9 % of a core; what's left is mostly ratatui's whole-screen diff,
~0.26 ms a frame). The window size ratatui asks for every frame comes
from one `ioctl` on stdout (Unix), not a `/dev/tty` opened and closed
each time (−0.1–0.4 % of a core at 200×60).

Music costs about nothing: 3.4 % of a core at 80×24 with the music card
vs 3.4 % without (60 s each, solid). Spotify is asked once a second
through one `osascript` process that stays up (about 3.5 ms of CPU a
poll: ~0.35 % of a core playing, ~0.12 % paused at one poll every 2 s),
plus a second, idle one that hears Spotify's change notifications (no
measurable CPU: under 0.01 s in 18 minutes). Asking Apple Music too, in
the same process, keeps it there: 2.7 ms of CPU a poll with Spotify
paused and Music closed (300 polls, `LAVATUI_POLLS=303 live_players`,
load average 30-50), 15-25 ms from request to answer. Lyrics are one request per track, on their own thread, and
cached. The cover's cells are worked out once per track and size, so
they add nothing per frame.

**Lyrics timing** (lava-75z.25). Measured against the Spotify app on
macOS 26 (Spotify 1.2, a heavily loaded machine, load average 70-300).
Spotify's reported position is exact: 2,055 back-to-back reads over 90 s
fit one line within ±4 ms, a read takes ~19 ms. So the truth is
Spotify's own position, read every ~30 ms, and the error is what the app
extrapolates minus that:

| | median | 99th pct | worst |
|---|---|---|---|
| before (first reading of a song kept for the whole song) | −56 ms | +168 ms | 168 ms |
| now (`Baseline`: readings bounded by request and reply, intersected) | −3 ms | 0 ms | 50 ms |

(Six minutes each, side by side, with a pause, seeks and track skips in
them.) Replaying those readings through `Baseline` with real pauses and
seeks in them: polled every 250 ms (what lava-75z.25 did while lyrics
showed; events replace it now, below) the 99th
percentile is +47 ms, once a second +131 ms; what's left is the time to
see a change, plus Spotify holding the position still 250-400 ms after a
seek while it buffers (followed at once now) and the odd poll Spotify
takes 0.7 s to answer.

**Changes made in the player** (lava-75z.29). Between polls the
position is predicted, so polling only needs to catch what changes in
the player itself; the player says so instead, and the worker polls at
once (then once more 300 ms on). Measured (the same audit, three
variants side by side for 20 minutes, Spotify played and paused in
between by hand):

| | resume shown after | pause shown after | CPU while paused |
|---|---|---|---|
| polling every 1 s (2 s paused), no events | 129 ms (lucky: up to 2 s) | 394 ms (up to 1 s) | 0.12 % |
| polling 4× a second (lava-75z.25, now gone) | 120 ms (up to 250 ms) | 13 ms (up to 250 ms) | 0.91 % |
| events | 46 ms | 25 ms | 0.12 % (+ the idle helper) |

Spotify's `com.spotify.client.PlaybackStateChanged` arrived within
13 ms of play and pause (and on track changes). Seeks made in Spotify
weren't tried (read-only), and its notification isn't known to cover
them, so a seek in the app still shows at the next poll (within 1 s,
0.5 s on average); a seek with LavaTUI's own keys shows at once (applied
as it's sent). macOS delivers distributed notifications only to a
main-thread run loop, so a JXA `osascript` helper listens
(`src/media/notify.rs`) and exits with LavaTUI. On Linux, MPRIS
`PropertiesChanged` and `Seeked` (seeks included) arrived 2-3 ms after
another app's pause, seek or track change (the fake player in Docker,
`mpris::live`). On Windows the media session's `PlaybackInfoChanged`,
`TimelinePropertiesChanged` and `MediaPropertiesChanged` and the
manager's `SessionsChanged` nudge it (lint-checked, not run). A player
whose events stream (a ticking timeline) is read at most every 250 ms.

**A few words behind** (lava-75z.30). Reported as the highlight
sometimes trailing the voice by a few words. Taken apart:

- *Word estimate*: the cause. It spread a line's words over all the
  time to the next line (up to 1.5× the song's median pace), but singers
  mostly sing a line at their own speed and then rest. Modelled on the
  43 songs of a real lyrics cache (1,651 lines; a line sung at the
  song's quick pace, the 25th percentile of its lines' seconds per
  syllable, then a rest), it trailed by 2+ words on 28 % of lines and
  4+ on one in ten. Now each line is sung at the song's 35th-percentile
  pace: 2+ words behind on 2 %; if a singer instead draws lines out, the
  highlight runs ahead (2+ words on 7 %).
- *Position*: not it. 30 minutes of real listening (13 track changes,
  a pause): steady error median −0.2 ms, 99th percentile +2 ms; pause
  and resume shown within 22-29 ms. After a track change Spotify
  announces it, then holds the new track at 0:00 for ~0.5 s: the
  position ran ~0.12 s early for ~0.95 s (lyrics early, not late). Now
  the worker re-reads every 300 ms after an event until two readings
  agree (up to 3): 20 more minutes of listening (6 track changes) had
  each settle within 150-245 ms. Spotify's position also drifts for the
  last ~2 s of a song (seen at every change; after the last line, as a
  rule). Two event races fixed too: the read a change event brought on right after
  LavaTUI's own pause or seek could briefly undo it (Spotify announces
  the pause before its state reads paused); and only the first poll of
  a burst of events got its 300 ms re-read. *Lyrics timing* was applied
  before extrapolating, so a reading in a song's first moments shifted
  it by less than set (now after; the user's was 0 anyway).
- *Other recordings*: LRCLIB lyrics are only taken within 2 s (3 s from
  search) of the track's length; a same-length version with a shifted
  intro can't be told apart (*lyrics timing* covers it).
- *Rendering*: not it. Each frame works the word out from the clock, so
  a slow or skipped frame shows a word late by at most a frame (33 ms at
  30 fps, 100 ms unfocused), never by words.

The performance info (`d`) reads the timing out in the toast row
(design.md §4.1) for reports from real listening.

One open question from the same run: at a pause, Spotify's reported
position jumped back about 0.8 s from where playing had it (every
variant saw it). Either Spotify steps back on pause, or while playing it
reports a little ahead of what's heard; if lyrics ever feel early, that
is the place to look (and *lyrics timing* moves them).

The lead on top (lines 150 ms, words 50 ms) is a
choice, not a correction. Word times are exact only when the lyrics
have enhanced-LRC word tags; LRCLIB almost never does (none of 295
synced versions of 19 popular songs), so words are usually estimated
(`src/lyrics/words.rs`, design.md §4.6 "Words").

```sh
# Read-only against the running Spotify app (plays nothing, changes nothing):
# (LAVATUI_TIMING_EVENTS=0 polls without listening for Spotify's events.)
LAVATUI_TIMING_SECS=360 LAVATUI_TIMING_CSV=/tmp/t.csv \
  cargo test --release -- --ignored --nocapture live_timing_audit
# The same readings replayed through the Baseline at a poll interval:
LAVATUI_TIMING_CSV=/tmp/t.csv LAVATUI_REPLAY_MS=1000 \
  cargo test --release -- --ignored --nocapture baseline_replay
```

Both caches stay bounded (`src/disk_cache.rs`, run on the workers after
each write; reads mark a file used): lyrics keep the 2000 most recently
used files (16 MB cap, ~2-8 MB in practice), covers the 256 most recent
within 50 MB; anything unused for 180 days goes. Settings › music &
lyrics measures and clears them on its own thread.

To reproduce:

```sh
cargo test --release -- --ignored --nocapture bench_lamp    # per style, bytes per frame
cargo test --release -- --ignored --nocapture bench_fill    # field sampler + sim step
cargo test --release -- --ignored --nocapture pop_harness   # frame-to-frame shape jumps
```

In native Ghostty at 301×86 (synthwave, music, cover and lyrics on) a
frame costs 2–3 ms and ~13 KB and the app holds 60 fps; what reaches the
screen is up to Ghostty: with custom shaders on a big window it showed
~50 distinct fps against ~56 without (`tools/ghostty_native.py --record`).

[`perf/frame-trace.md`](perf/frame-trace.md) covers frame-interval
tracing in real terminals (and that native Ghostty measurement); [`performance-compute.md`](performance-compute.md)
has the compute-side notes.

## Development

```sh
cargo run --release                          # the lamp (debug builds are too slow)
cargo test                                   # unit, snapshot and layout-sweep tests
cargo fmt --check && cargo clippy --all-targets -- -D warnings
UPDATE_SNAPSHOTS=1 cargo test                # rewrite render/layout snapshots (review the diff)
cargo run --release -- --config /tmp/x.toml  # try settings without touching yours
tools/linux/run.sh                           # build, lint, test and drive it on Linux in Docker
```

## Releases

Releases are built by [dist](https://opensource.axo.dev/cargo-dist/)
(cargo-dist). Its settings are `dist-workspace.toml` and `[profile.dist]`
in `Cargo.toml` (the release profile as is: fat LTO). It writes
`.github/workflows/release.yml`; don't edit that file: change the
settings or `.github/build-setup.yml` (our extra build steps), then run
`dist generate`. `.github/actionlint.yaml` mutes the shellcheck notes in
dist's own steps.

Pushing a `vX.Y.Z` tag runs the workflow. dist refuses a tag that
doesn't match the version in `Cargo.toml`. It builds five targets, each
on its own runner (`dist plan` lists them):

| Target | Runner |
|---|---|
| `aarch64-apple-darwin` | macos-14 |
| `x86_64-apple-darwin` | macos-15-intel |
| `x86_64-unknown-linux-gnu` | ubuntu-22.04 (glibc 2.35, so older distros run it too) |
| `aarch64-unknown-linux-gnu` | ubuntu-22.04-arm |
| `x86_64-pc-windows-msvc` | windows-2022 |

and makes, on the GitHub Release (published, not a draft; the notes are
the version's section of `CHANGELOG.md`):

- `lavatui-<target>.tar.xz` (Windows: `.zip`) with the binary, README,
  CHANGELOG and both licences, each with a `.sha256`, plus `sha256.sum`
  and `source.tar.gz`;
- `lavatui-installer.sh` (macOS/Linux) and `lavatui-installer.ps1`
  (Windows): they pick the right archive and install to `$CARGO_HOME/bin`
  (`~/.cargo/bin`), adding it to `PATH`. The README links them through
  `releases/latest/download/`;
- `lavatui.rb`, a Homebrew formula, which the `publish-homebrew-formula`
  job commits to
  [vespillo-tech/homebrew-tap](https://github.com/vespillo-tech/homebrew-tap)
  (`brew install vespillo-tech/tap/lavatui`). The tap repository must
  exist (an empty repo with a README is enough; the job writes
  `Formula/lavatui.rb`).

Pull requests only run `dist plan` (`pr-run-mode = "plan"`).
To try it locally: `dist plan`, `dist build --artifacts=local` (archive
for this machine in `target/distrib/`), `dist build --artifacts=global`
(installers and formula).

Repository secrets:

| Secret | What |
|---|---|
| `HOMEBREW_TAP_TOKEN` | a fine-grained personal access token with **Contents: read and write** on `vespillo-tech/homebrew-tap` only. Without it the formula job fails (the release itself is already up by then). |
| `MACOS_CERTIFICATE` | optional, see below: the *Developer ID Application* certificate and key, a `.p12` file as base64 (`base64 -i cert.p12`) |
| `MACOS_CERTIFICATE_PASSWORD` | the `.p12` file's password |
| `MACOS_SIGNING_IDENTITY` | e.g. `Developer ID Application: Your Name (TEAMID)` |
| `APPLE_ID` | the Apple Developer account's email, for `notarytool` |
| `APPLE_TEAM_ID` | the 10-character team ID |
| `APPLE_APP_PASSWORD` | an app-specific password for that Apple ID |

macOS builds are only ad-hoc signed unless the Apple secrets are set;
then `.github/build-setup.yml` signs the binary with the Developer ID
(hardened runtime, timestamp, identifier
`io.github.vespillo-tech.lavatui`) and notarizes it before `dist build`
packs it. dist's own `macos-sign` isn't used: it doesn't notarize, add a
timestamp or set the identifier, and it fails when its secrets are
empty. The step builds with dist's exact cargo command and signs cargo's
copy in `target/<target>/dist/deps/` (cargo copies that file over
`target/<target>/dist/lavatui` on every build, so signing the outer one
wouldn't stick); `dist build` then has nothing to rebuild and packs the
signed binary. The step checks that, and fails if cargo ever stops
working this way.

Why it matters: the Keychain ties *Always Allow* to the program's code
signature. An ad-hoc signed binary is a new program after every update,
so macOS asks again for the saved Spotify login (lava-1xk.38); releases
signed with the same Developer ID keep the answer.

## The README's pictures

The screenshots are drawn from the release binary in sized ptys, with a
fixed seed and a scratch config, by `docs/screenshots/capture.py` (pyte +
Pillow). The animated demo is `docs/screenshots/demo.tape` (vhs, then
gifsicle; the tape has the exact commands and the size budget).

Pictures with music use the hidden `--demo` flag (`src/demo.rs`): a
made-up player with invented songs and artists, original covers embedded
from `assets/demo/` and invented lyrics served by a canned LRCLIB.
The Spotify library is a made-up account too (`demo::account`, a
`spotify_web::fake::FakeWeb`, plugged in with `Library::connect_with`):
already logged in (logging out and in again needs no browser), four
invented playlists of the demo songs plus a few more invented ones
(`MORE`, no lyrics), liked songs, and add-to: "Late Night Lava" already
has the first song, so adding it shows the "add it again?" question.
Playing from the browser plays in the `FakeSource`, which knows the
playlists (`FakeSource::with_contexts`). `Library::demo` counts as set up
with no Client ID. Without `--config` the demo reads and saves no
settings at all (`Store::for_session`); with one (as `capture.py` and the
tape do), its saves keep the file's `[spotify]` section, so the demo's
Spotify page can't change a real Client ID, login store or login.
Nothing goes to the network, no account or keyring is touched, and no
real album art or song ends up in a committed image.
`capture.py library` draws the browser, a playlist, the add list and
the question over it.

```sh
cargo build --release
python3 -m venv /tmp/v && /tmp/v/bin/pip install pyte pillow
/tmp/v/bin/python docs/screenshots/capture.py      # all committed PNGs
vhs docs/screenshots/demo.tape && gifsicle -O3 --colors 80 -b docs/screenshots/demo.gif
```
