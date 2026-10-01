# lavatui

A lava lamp for your terminal. Blobs of wax warm on the heater, rise,
cool, sink, merge and split, and you can draw them in nine render
styles: smooth half-blocks, braille, halftone, a synthwave sunset,
digital rain, a topographic map and more. Next to the lamp sit a clock and a pomodoro timer.
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
  pomodoro beside it (or below it in a tall, narrow terminal).
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
| `t` | show/hide the clock |
| `T` | 12h / 24h |
| `space` | pomodoro start / pause / resume |
| `n` | skip to the next phase |
| `r` `r` | reset the pomodoro (press twice within 2 s) |
| **App** | |
| `m` | minimal mode on/off |
| `b` | status bar on/off |
| `d` | debug HUD (fps, frame time, samples) |
| `ctrl-l` | force a full redraw |
| `?` | help |
| `q` / `ctrl-c` | quit |

`esc` never quits: it only closes overlays. When help or a picker is
open, `q` closes it instead of quitting.

- **In help:** `j`/`k` or `↑`/`↓` scroll. `?`, `esc` or `q` close it.
- **In pickers:** `j`/`k` or `↑`/`↓` move (with live preview), `1`–`9`
  jump, `enter` or `space` keep, and `esc` or `q` revert. Pressing the
  opening key again keeps the choice and closes the picker. In the tiny
  inline picker, `h`/`l` and `←`/`→` move too.
- **Mouse** (opt-in, `input.mouse = true`): click or drag on the lamp to
  heat the wax there, scroll in help, and scroll or click in pickers
  (a click previews, a double-click keeps). It's off by default because mouse capture breaks
  the terminal's text selection.

This table matches the single `KEYMAP` table in `src/ui/keymap.rs`. That
table also drives the in-app help, so the help can't drift from the
actual bindings.

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

[lamp]
style = "solid"          # see Render styles
heat = 3                 # 1..=5
speed = 1.0              # 0.25 | 0.5 | 1 | 2 | 4

[theme]
palette = "lava"         # see Palettes
transparent = false      # true = never paint the background (keeps terminal transparency)

[clock]
face = "blocks"          # blocks | segment | analog | binary | words | text
show = true
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
mouse = false
```

The file is meant to be edited by hand, even while the lamp runs. A bad
value (or a style, palette or face that doesn't exist) is ignored, a value
out of range is clamped (`config: lamp.heat 99 → 5`), and the rest of the
file still applies; keys lavatui doesn't know are reported but kept. The
toast names the first problem; if there are more, all of them are printed
when you quit. A TOML syntax error is reported with its line and the lamp
starts from the defaults.

Settings from older versions load without a word: `lamp.frame` and
`lamp.lighting` are ignored (and dropped at the next save), a removed
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
