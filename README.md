# lavatui

A lava lamp for your terminal. Blobs of wax warm on the heater, rise,
cool, sink, merge and split, and you can draw them in nine render
styles: smooth half-blocks, braille, halftone, a synthwave sunset,
digital rain, a topographic map and more. Next to the lamp sit a clock and a pomodoro timer,
and, if you want it, what's playing in Spotify, cover art included.
Turn those off with one key and you just have the lamp.

![lavatui demo: the lamp cycling through render styles and palettes, the style picker previewing styles live, and minimal mode](docs/screenshots/demo.gif)

Built with Rust and [ratatui](https://ratatui.rs). It's one small binary
with no runtime dependencies and no Nerd Fonts.

## Gallery

**The full TUI** at 120×36: the solid style, the blocks clock and a
running pomodoro.

![lavatui at 120×36: the lamp in the solid style, the blocks clock and a running pomodoro](docs/screenshots/hero.png)

**Nine render styles.** These are all the same seed and the same moment,
in minimal mode:

![the render styles side by side](docs/screenshots/styles.png)

**Eight palettes:**

![the eight palettes side by side](docs/screenshots/palettes.png)

| **Minimal mode** (`m`): just the lamp | **Help** (`?`): the lamp keeps going behind it |
|---|---|
| ![minimal mode, braille in ultraviolet](docs/screenshots/minimal.png) | ![help overlay](docs/screenshots/help.png) |
| **Style picker** (`S`) with live preview | **16 colours** (`--color 16`) |
| ![style picker over a synthwave lamp](docs/screenshots/picker.png) | ![ascii style in 16 colours](docs/screenshots/color16.png) |

| **On the lava** (`t`, `f`): clock and pomodoro over the wax | **Clock on the lava, pomodoro beside it** |
|---|---|
| ![clock and pomodoro on the lava, solid style](docs/screenshots/overlay.png) | ![clock on the lava in braille, pomodoro in the side panel](docs/screenshots/overlay-mix.png) |

| Portrait terminal (36×56): panel moves below | Tiny terminal (26×10): lamp + clock chip |
|---|---|
| ![tall narrow terminal in abyss](docs/screenshots/portrait.png) | ![tiny terminal](docs/screenshots/tiny.png) |

## Features

- **The wax is simulated.** Heat comes from the base. Warm wax gets
  buoyant and rises, cools at the top and sinks back, and it merges, necks
  and splits on the way. The wax area is conserved, and every run is
  deterministic for a given `--seed`.
- **9 render styles** that you can cycle with `s` or pick with `S`
  (live preview): solid, outline, ascii, braille, halftone, synthwave,
  matrix, topo, chrome.
- **8 palettes**: lava, ultraviolet, abyss, toxic, synthwave, mono, paper
  (light) and ansi (your terminal's own colours).
- **Edge to edge.** The wax fills the window, with the clock and
  pomodoro beside it (or below it in a tall, narrow terminal). A
  wide, short terminal gets them side by side in a strip under the lamp,
  a very large one two columns, a cramped portrait one rows across the
  width: whichever keeps the widgets largest.
- **Widgets go where you want them.** The clock (`t`), the pomodoro
  (`f`), music (`a`), lyrics (`y`) and the album cover (`o`) each sit in
  the side panel, on the lava, or off.
  On the lava each has its own spot (centre, top, the corners, bottom):
  `l` moves the one you last put there, `L` picks another. They float
  right on the lamp with no background: the wax runs up to every stroke,
  and each word turns dark over bright wax and light over the liquid, so
  it stays readable over every style (`dock.backing = "soft"` brings
  back a soft pool of liquid behind them). They spread out without
  touching. What
  matters most right now keeps its size longest (a running pomodoro,
  then playing music, then the clock); when space runs out the rest
  shrink, then fold into one row of chips in the corner
  (`14:32 · ▸ 18:24 · ▶ Song – Artist`), never covering the lamp.
- **Now playing** (`a`, off by default): the Spotify desktop app's track
  with a small cover beside it, title, artist, album, a progress bar,
  play state and volume, beside the lamp or on the lava. It shrinks from
  that card down to `▶ title – artist`, and says calmly
  when Spotify isn't running or needs permission. `A` turns on the player
  keys: `space` play/pause, `n`/`p` next/previous, `←`/`→` seek, `↑`/`↓`
  volume, `esc` when done. It never blocks a frame: the player is polled on
  its own thread, only while the widget is shown, and covers are fetched
  and cached (`$XDG_CACHE_HOME/lavatui/art`) in the background. On Linux
  and Windows it shows any player (Spotify first); see
  [Platform support](#platform-support).
- **Your Spotify library** (needs a Client ID, `docs/spotify.md`): log in
  once (`A` then `i`, in the browser), then browse your playlists (`b`),
  open the ones you own or share and play any track in them, add the
  playing track to a playlist (`a`), and like or unlike it (`s`; a `♥` in
  the widget). With Premium, shuffle and repeat (`x` / `r`) work too,
  through the Web API. All of it runs on a worker thread; the lamp never
  waits for Spotify.
- **Album cover** (`o`, off by default): the playing track's cover as a
  widget of its own, beside the lamp or on the lava (top right by
  default), small / medium / large / as big as fits (`art.size`). In
  **kitty and Ghostty it's the real picture** (the kitty graphics
  protocol, sent once per track and size, then just cells that never
  flicker); elsewhere it's drawn in text cells: sextants (2 × 3 pixels a
  cell), quadrants (2 × 2) or half blocks (1 × 2). `O` cycles the detail
  (`auto` picks the best your terminal has). Clicking it plays / pauses.
  With the cover widget on, the music card leaves its own small cover
  out (`art.inline = false` drops that one for good).
- **Mouse**: the music widget has quiet buttons (`◂◂ ‖ ▸▸`, `♡ + ≡`) and a
  progress bar you can click to seek; pickers and the playlist browser
  click and scroll. Every button has a key.
- **Lyrics** (`y`, off by default): the playing track's words from
  [lrclib.net](https://lrclib.net), in time with the song: the current
  line bold and bright, the ones around it dim, a gentle fade from line
  to line, dots through the instrumental breaks. Five lines, three, or
  one, on the lava (bottom centre) or beside it; plain lyrics scroll with
  the song when there's no timing. Turning it on sends each track's title,
  artist, album and length to lrclib.net.
- **Clock faces**: blocks, segment, analog, binary, words and text. Each
  face comes in several sizes, and the largest one that fits is used; on
  a very large terminal the panel widens for the biggest ones.
- **Pomodoro timer** (`space`) with focus and break phases, cycle dots,
  a phase-change flash and an optional bell.
- **It looks right at any size.** It works from 1×1 to a 4K full-screen
  terminal. When space runs out, the panel collapses into a chip, and
  nothing is ever truncated or overlapping.
- **Colour fallbacks are automatic**: truecolor → 256 (perceptual,
  hue-preserving match) → 16 → `NO_COLOR`. Every style still reads in 16
  colours and in monochrome.
- **Minimal mode** (`m` / `-m`) shows just the lamp, with a tiny clock
  in the corner.
- **It's light on resources.** About 2 % of a core at 80×24 and 5 %
  at 200×60, at 60 fps. It drops to 10 fps when the terminal loses focus
  and sleeps while frozen. See [Performance](#performance).

## Install

You need Rust 1.88 or newer (`rustup update` if `cargo` says otherwise).
There's no published crate yet, so build it from a checkout of this
repository:

```sh
cargo install --path .     # puts `lavatui` in ~/.cargo/bin
lavatui
```

Or run it in place with `cargo run --release`. Use a release build: the
simulation is too slow in debug.

**Terminal:** a truecolor terminal looks best (iTerm2, kitty, WezTerm,
Alacritty, Ghostty, Windows Terminal, recent GNOME Terminal and others).
The colour depth is detected from `NO_COLOR`, `COLORTERM` and `TERM`, and
the app falls back on its own, or you can force a depth with `--color`.
Any font with Unicode block elements and braille works, so no Nerd Font
is needed. On macOS, Terminal.app has no truecolor and gets the 256-colour
path.

**Translucent terminals:** with `background-opacity < 1` and
`background-opacity-cells = true`, Ghostty draws a cell's background
see-through but its glyph opaque, so a half-block cell split across two
colours shows its lower or upper half darker. The lamp is drawn for this:
halves that look alike become one colour, and the liquid is always the
(see-through) background. In such a Ghostty (`display.cells = "auto"`
reads its config at start; apps that embed Ghostty's terminal, like
Ghostex, don't count: they don't read that config) wax cells are never split at all, so wax and
pool show no half-row streaks; it costs a little colour detail inside the
wax, which is why opaque terminals don't get it. Set
`display.cells = "translucent"` for another terminal that blends this
way (or a Ghostty configured on the command line), `"opaque"` to turn
it off. Album covers in text cells follow the same rule (one colour a
cell when translucent); kitty graphics (pixels) are unaffected. See
[Ghostty's opacity settings](https://ghostty.org/docs/config/reference#background-opacity-cells).

## Usage

```
Usage: lavatui [OPTIONS]

Options:
  -m, --minimal         Just the lamp: no panels, status bar or hints
      --fps <N>         Target render frames per second (the simulation rate is fixed separately)
      --style <NAME>    Render style for this session (e.g. solid, outline, ascii, braille, chrome)
      --palette <NAME>  Palette for this session (lava, ultraviolet, abyss, toxic, synthwave, mono, paper, ansi)
      --color <DEPTH>   Colour depth, instead of detecting it from the environment [possible values: auto, truecolor, 256, 16, none]
      --seed <U64>      Seed the wax simulation: the same seed always plays out the same lamp (default: a new seed every launch)
      --config <PATH>   Read and write settings here instead of the XDG config dir
  -h, --help            Print help
  -V, --version         Print version
```

Flags apply to the current session only and are never written back to
the config file. A setting you change in the app is saved as usual. An
unknown `--style` or `--palette` name exits with the list of valid ones.

### Keys

| Key | Action |
|---|---|
| **Lamp** | |
| `s` / `S` | next style / style picker |
| `p` / `P` | next palette / palette picker |
| `[` / `]` | heat − / + (5 steps) |
| `-` / `+` (`=`) | sim speed ×0.25 … ×4 |
| `z` | freeze / unfreeze the lamp |
| `0` | reset heat and speed |
| `R` | reseed the wax |
| **Clock & pomodoro** | |
| `c` / `C` | next clock face / face picker |
| `T` | 12h / 24h |
| `space` | pomodoro start / pause / resume |
| `n` | skip to the next phase |
| `r` `r` | reset the pomodoro (press twice within 2 s) |
| **Widgets** | |
| `t` | clock: side panel → on the lava → off |
| `f` | pomodoro: side panel → on the lava → off |
| `a` | music (now playing): side panel → on the lava → off |
| `A` | player keys on (see below) |
| `y` | lyrics: side panel → on the lava → off (looks tracks up on lrclib.net) |
| `o` | album cover: side panel → on the lava → off |
| `O` | cover detail: auto → pixels → sextant → quadrant → halfblock |
| `l` | move a widget on the lava (the last put there): centre, top, the corners, bottom |
| `L` | pick which widget on the lava `l` moves |
| **App** | |
| `m` | minimal mode on/off |
| `b` | status bar on/off |
| `d` | debug HUD (fps, frame time, samples) |
| `ctrl-l` | force a full redraw |
| `?` | help |
| `q` / `ctrl-c` | quit |

`esc` never quits: it only closes overlays. When help or a picker is
open, `q` closes it instead of quitting.

- **Player keys** (after `A`, until `esc`, `q` or `A`): `space` play /
  pause, `n` / `p` next / previous, `←` / `→` (`h` / `l`) seek 10 s,
  `↑` / `↓` (`k` / `j`, `+` / `-`) volume, `x` / `r` shuffle / repeat
  where the player supports them (Spotify's AppleScript doesn't; logged
  in with Premium they go through the Web API), `s` like / unlike, `a` add
  to playlist, `b` playlists, `i` log in to Spotify (again, twice: log
  out). They take the keyboard like an overlay, so they can reuse `space`,
  `n` and `p`; the status bar shows them while they're on.
- **In the playlist browser:** `j`/`k` move, `enter` (or `l`) opens a
  playlist you own or share (others: plays it) or plays a track in it,
  `p` plays the whole playlist, `g`/`G` and page up/down jump, `esc` (or
  `h`) goes back, `q` closes. The add-to-playlist picker lists only
  playlists you can add to; `enter` adds.
- **In help:** `j`/`k` or `↑`/`↓` scroll. `?`, `esc` or `q` close it.
- **In pickers:** `j`/`k` or `↑`/`↓` move (with live preview), `1`–`9`
  jump, `enter` or `space` keep, and `esc` or `q` revert. Pressing the
  opening key again keeps the choice and closes the picker. In the tiny
  inline picker, `h`/`l` and `←`/`→` move too.
- **Mouse** (on by default, `input.mouse = false` turns it off): click the
  music widget's buttons and progress bar, click the cover to play /
  pause, click or drag on the lamp to
  heat the wax there, scroll in help, and scroll or click in pickers and
  the playlist browser (a click picks, a double-click keeps / opens). To
  select text in the terminal while the mouse is on, hold **shift** while
  dragging (**option** in macOS Terminal and iTerm2).

This table matches the single `KEYMAP` table in `src/ui/keymap.rs`. That
table also drives the in-app help, so the help can't drift from the
actual bindings.

## Platform support

The lamp, clock, pomodoro and config work the same everywhere; config,
cache and data go where each OS expects them (`directories`). Only now
playing differs:

| | macOS | Linux | Windows |
|---|---|---|---|
| Builds (`cargo check --all-targets`) | ✓ (aarch64) | ✓ (x86_64-unknown-linux-gnu) | ✓ (x86_64-pc-windows-msvc) |
| Tested on real hardware | ✓ | not yet | not yet |
| Now playing via | AppleScript, one long-lived `osascript` | MPRIS on the D-Bus session bus (zbus) | System Media Transport Controls |
| Players | the Spotify desktop app | any MPRIS player, Spotify first | any app in the media flyout, Spotify first |
| Play/pause, next/previous, seek | ✓ | ✓ | ✓ |
| Volume | ✓ | ✓ if the player has it | – (SMTC has no volume: no readout, the keys say so) |
| Shuffle / repeat | – (no-ops in Spotify 1.2) | ✓ if the player honours them | ✓ if the app honours them |
| Cover art | ✓ | ✓ (`https` art URLs, so Spotify) | – (SMTC gives a stream, not a URL) |
| Launches the player? | never | never | never |
| Permission | macOS asks once (Automation) | none | none |

Linux players vary: Spotify has long reported its position as 0 over
MPRIS (the bar then counts from where it was first seen) and ignored
shuffle and repeat. Anything a player leaves out falls back quietly.

## Configuration

Settings are saved on their own, 1 s after a change and on quit, to:

- `$XDG_CONFIG_HOME/lavatui/config.toml` if `XDG_CONFIG_HOME` is set to an
  absolute path, else
- `~/.config/lavatui/config.toml` on Linux, or
- `~/Library/Application Support/lavatui/config.toml` on macOS,

or to the file given with `--config`. Every key is optional. Here are the
defaults:

```toml
[display]
fps = 60                 # 1..=240
color = "auto"           # auto | truecolor | 256 | 16 | none
cell_aspect = 2.0        # cell height / width; used only when the terminal doesn't report pixels
cells = "auto"           # auto | opaque | translucent: are cell backgrounds see-through?

[lamp]
style = "solid"          # see Render styles
heat = 3                 # 1..=5
speed = 1.0              # 0.25 | 0.5 | 1 | 2 | 4

[theme]
palette = "lava"         # see Palettes
transparent = false      # true = never paint the background (keeps terminal transparency)

[clock]
face = "blocks"          # blocks | segment | analog | binary | words | text
hour24 = true

[pomodoro]
focus_min = 25
short_break_min = 5
long_break_min = 15
cycles = 4               # focus phases before a long break
bell = true

[ui]
mode = "full"            # full | minimal
status_bar = true

[minimal]
clock = "corner"         # corner | off

[input]
mouse = true             # shift-drag (option-drag on macOS) still selects text

[dock]
clock = "side"           # side | overlay | off
pomodoro = "side"        # side | overlay | off
music = "off"            # side | overlay | off
lyrics = "off"           # side | overlay | off (on = lookups on lrclib.net)
cover = "off"            # side | overlay | off
# each widget's spot on the lava:
# center | top | top-right | bottom-right | bottom | bottom-left | top-left
anchor = { clock = "center", pomodoro = "center", music = "top-left", lyrics = "bottom", cover = "top-right" }
backing = "none"         # none (text floats on the lamp) | soft (a veiled pool behind)

[art]
detail = "auto"          # auto | pixels | sextant | quadrant | halfblock
size = "medium"          # small (16 cols) | medium (24) | large (34) | fill (up to 64)
inline = true            # the music card's own small cover (while the cover widget is off)

[spotify]
client_id = ""           # for the Web API library features, see docs/spotify.md
                         # ("" = off; LAVATUI_SPOTIFY_CLIENT_ID works too)
```

Music needs nothing set up: it talks to the Spotify desktop app. The
first time, macOS asks whether your terminal may control Spotify; if you
said no, the widget tells you where to change it (System Settings ›
Privacy & Security › Automation).

Lyrics are **off until you place them** (`y`): with the widget on, the
title, artist, album and length of each track you play are sent to
[lrclib.net](https://lrclib.net), a free, open lyrics database, and the
answers are kept in your cache dir (`$XDG_CACHE_HOME/lavatui/lyrics`).
Nothing is sent while it's off.

The file is meant to be edited by hand, even while the lamp runs. A bad
value (or a style, palette or face that doesn't exist) is ignored, a value
out of range is clamped (`config: lamp.heat 99 → 5`), and the rest of the
file still applies; keys lavatui doesn't know are reported but kept. The
toast names the first problem; if there are more, all of them are printed
when you quit. A TOML syntax error is reported with its line and the lamp
starts from the defaults.

Settings from older versions load without a word: a single `dock.anchor =
"top"` puts every widget there (saved per widget next time), `lamp.frame` and
`lamp.lighting` are ignored (and dropped at the next save), `clock.show =
false` becomes `dock.clock = "off"`, a removed
style (`heatmap`, `dither`, `crt`) becomes `solid`, and `minimal.clock =
"under"` means `corner`.

Saving only writes the settings you changed in the app, into the file as
it is at that moment, so your hand edits to anything else survive. Your
comments, key order and unknown keys are kept, and saves write through
symlinks, so a dotfile manager's link stays intact. If a save would
overwrite something lavatui couldn't use (an ignored or clamped value, a
broken file, bytes that aren't UTF-8), the file is first copied to
`config.toml.bak`. A file that's mid-edit and not valid TOML is left alone
until it is. A read-only config (or folder), or a path that isn't a
regular file (`/dev/null`, a fifo), is never written; a toast says so once.

## Render styles

| Style | What it looks like |
|---|---|
| `solid` | Smooth wax in half blocks, coloured by temperature, with anti-aliased edges. The default. |
| `outline` | Just the wax surface, as a thin braille contour coloured by temperature. |
| `ascii` | A classic ` .:-=+*#%@` density ramp. It gets denser toward the core and with heat. |
| `braille` | Filled wax at 2×4 dots per cell, with an engraving-like stipple that thins toward the skin. |
| `halftone` | Newsprint: a 45° screen of round dots that swell toward the hot core. |
| `synthwave` | A 1986 sunset: striped retro-sun blobs over a neon perspective grid. |
| `matrix` | Digital rain that only shows where it crosses the wax. |
| `topo` | A topographic map of the wax field, with contour lines and elevation tints. |
| `chrome` | Glossy blown glass: domes with a body shade, a specular glint and a fresnel rim. (`glass` still works as an old name.) |

## Palettes

| Palette | Mood |
|---|---|
| `lava` | The 1970s original: red-orange wax in amber oil. The default. |
| `ultraviolet` | A blacklight poster: violet to hot pink in deep indigo. |
| `abyss` | Deep sea: teal wax glowing to seafoam in navy water. |
| `toxic` | Radioactive slime: moss to acid yellow-green. |
| `synthwave` | A 1986 sunset: hot pink → coral → gold, with a cyan accent. |
| `mono` | Graphite grayscale. It suits halftone and braille. |
| `paper` | The light theme: rust-red ink on cream, for light terminals. |
| `ansi` | Uses your terminal's own 16-colour theme and default background. |

## Performance

Measured on an Apple M5 laptop under background load (load average
3–5), so treat them as rough. The real runs are from before v1.1 (the
release binary in a pty, truecolor, 60 fps, 15 s each, with the glass
frame since removed); render times are `bench_lamp` on v1.1.

| Measurement | Result |
|---|---|
| Launch → first frame → exit (`--frames 1`, 80×24) | ~30 ms (median 34, min 29). The sim starts pre-warmed. |
| CPU at 80×24, solid or braille | ~2.3 % of one core |
| CPU at 200×60, solid / braille | 4.6 % / 4.9 % |
| Output at 80×24 / 200×60, solid | ~10 KB/s / ~55 KB/s |
| Output in braille (80×24 / 200×60) | ~5 KB/s / ~21 KB/s |

Braille changes few cells per frame, so it is the cheapest to send.

With music on and Spotify playing, the cost is lost in the noise: 3.4 %
of a core at 80×24 with the music panel vs 3.4 % with music off (60 s
each, solid). Spotify is asked once a second through one `osascript`
process that stays up, two Apple events per poll (~25 ms, ~0.4 % of a
core); starting `osascript` for every poll used to cost 12.6 %.
Lyrics read the same player (so on their own they cost about the same as
music) and add nothing measurable on top of it; the lookup is one request
per track, on its own thread, and cached.

The cover widget costs nothing per frame either: at 120×36 the median
draw is 0.24 ms with music alone and 0.24–0.25 ms with the cover in any
detail (sextant, quadrant, half block, pixels), and bytes per frame are
unchanged, since the cover's cells never change between frames (its text
cells are worked out once per track and size). In pixels mode the
picture (a ≤ 400 px PNG, ~370 KB as base64) is sent once per track and
size, at most 96 KB a frame (a few frames), inside the frame's
synchronized update; `tools/kitty_check.py` shows exactly what goes out.

Here is the render time per frame at 200×60 in truecolor: the field
sampling plus the style draw (`bench_lamp`: a full-area lamp, two sim
steps per frame):

| Style | Time | | Style | Time |
|---|---|---|---|---|
| solid | 0.23 ms | | synthwave | 0.50 ms |
| outline | 0.33 ms | | matrix | 0.12 ms |
| ascii | 0.14 ms | | topo | 0.70 ms |
| braille | 0.39 ms | | chrome | 0.41 ms |
| halftone | 0.14 ms | | | |

Every style stays under 0.8 ms at 200×60, in truecolor and in 256
colours (0.16–0.76 ms there, with the dither pass). The design target
is 8 ms. At 80×24, every style takes 0.02–0.14 ms. If frames ever get
slow, adaptive quality first lowers the sample grid and then halves the
frame rate (never below 30 fps). It recovers on its own and never changes
your settings.

To reproduce:

```sh
cargo test --release -- --ignored --nocapture bench_lamp    # per style, bytes per frame
cargo test --release -- --ignored --nocapture bench_fill    # field sampler + sim step
```

## Development

```sh
cargo test                                   # unit, render-snapshot and layout-sweep tests
cargo fmt --check && cargo clippy --all-targets -- -D warnings
UPDATE_SNAPSHOTS=1 cargo test                # rewrite snapshots (review the diff)
cargo run --release -- --config /tmp/x.toml  # try settings without touching your config
```

- [`docs/design.md`](docs/design.md) is the layout and visual design
  contract: breakpoints, hide order, palettes, keymap and performance
  targets.
- [`CLAUDE.md`](CLAUDE.md) has the architecture overview (sim, field,
  render, theme, layout and app model) and step-by-step guides for
  adding a render style, a clock face or a key.

All the logic is pure and unit-tested: the simulation, layout, clock,
pomodoro and the app model. Only `ui/` and `app/mod.rs` touch the
terminal.

The screenshots are generated from the release binary with a fixed seed
and a scratch config. Rerun them after visible changes:

```sh
python3 docs/screenshots/capture.py          # the PNGs (needs pyte + pillow; see its docstring)
vhs docs/screenshots/demo.tape               # the demo GIF (needs vhs), then crop + optimise:
gifsicle -O3 --crop 6,0+984x580 -b docs/screenshots/demo.gif
```

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.
