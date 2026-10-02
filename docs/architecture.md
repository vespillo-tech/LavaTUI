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
| `src/lyrics/` | LRCLIB client, cache, LRC parser and the line syncer. |
| `src/spotify_web/` | The optional Spotify Web API client (PKCE login, library, player). |
| `src/graphics/` | Album covers as real pictures: kitty graphics, iTerm2 images and sixel, with a start-up check of what the terminal really supports. |

Slow work never runs on the frame: saving, the music player, cover
downloads, lyrics lookups and Spotify requests each have their own
worker thread, and a frame only reads their latest result.

[`CLAUDE.md`](../CLAUDE.md) (the same text as `AGENTS.md`) has the full
module map and step-by-step guides for adding a render style, a clock
face, a widget or a key.

## Platforms

The lamp, clock, timer and settings work the same everywhere; config,
cache and data go where each OS expects them. Only now playing differs:

| | macOS | Linux | Windows |
|---|---|---|---|
| Builds in CI | ✓ | ✓ | ✓ |
| Tested on real hardware | ✓ | in Docker with a stand-in MPRIS player (`tools/linux/run.sh`); not yet on a desktop | not yet |
| Now playing via | AppleScript, one long-lived `osascript` | MPRIS on the D-Bus session bus (zbus) | System Media Transport Controls |
| Players | the Spotify desktop app | any MPRIS player, Spotify first | any app in the media flyout, Spotify first |
| Play/pause, next/previous, seek | ✓ | ✓ | ✓ |
| Volume | ✓ | ✓ if the player has it | – (SMTC has no volume) |
| Shuffle / repeat | through the Web API only (Spotify's AppleScript setters do nothing) | ✓ if the player honours them (Spotify: through your account) | ✓ if the app honours them |
| Cover art | ✓ | ✓ (`https` art URLs, so Spotify) | ✓ (the session's thumbnail) |
| Like / add the playing song (library setup) | ✓ | ✓ Spotify songs | ✓ Spotify songs, once your account's player reports the same song |
| Play from the playlist browser | ✓, the rest of the playlist follows | with Premium and Spotify playing: ✓; otherwise just that song | with Premium and Spotify playing: ✓; otherwise it says so |
| Launches the player? | never | never | never |
| Permission | macOS asks once (Automation) | none | none |

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

## Pictures in the terminal

The album cover is a real picture where the terminal can show one:
the kitty graphics protocol (kitty, Ghostty) with Unicode placeholders,
sent once per track and size in chunks of at most 96 KB a frame; iTerm2
inline images (iTerm2, WezTerm) and sixel (foot, mlterm, Konsole, …),
sent once when the cover appears, moves or changes size. LavaTUI asks the
terminal once at start whether it really supports what its environment
promises, and draws the cover in text cells until it says yes. Inside
tmux, screen, zellij or Ghostex it doesn't try. Elsewhere covers are text
cells: sextants (2×3 pixels a cell), quadrants (2×2) or half blocks
(1×2), with the best two colours per cell. `LAVATUI_GRAPHICS` overrides
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
until the clock or timer changes.

Music costs about nothing: 3.4 % of a core at 80×24 with the music card
vs 3.4 % without (60 s each, solid). Spotify is asked once a second
through one `osascript` process that stays up. Lyrics are one request per
track, on their own thread, and cached. The cover's cells are worked out
once per track and size, so they add nothing per frame.

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

[`perf/frame-trace.md`](perf/frame-trace.md) covers frame-interval
tracing in real terminals; [`performance-compute.md`](performance-compute.md)
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

## The README's pictures

The screenshots are drawn from the release binary in sized ptys, with a
fixed seed and a scratch config, by `docs/screenshots/capture.py` (pyte +
Pillow). The animated demo is `docs/screenshots/demo.tape` (vhs, then
gifsicle; the tape has the exact commands and the size budget).

Pictures with music use the hidden `--demo` flag (`src/demo.rs`): a
made-up player with invented songs and artists, an abstract cover drawn
by the app and invented lyrics served by a canned LRCLIB. Nothing goes to
the network, no account is touched, and no real album art or song ends
up in a committed image.

```sh
cargo build --release
python3 -m venv /tmp/v && /tmp/v/bin/pip install pyte pillow
/tmp/v/bin/python docs/screenshots/capture.py      # all committed PNGs
vhs docs/screenshots/demo.tape && gifsicle -O3 --colors 80 -b docs/screenshots/demo.gif
```
