# The settings file

You don't need to edit anything by hand: press `,` in LavaTUI for the
settings screen. Its pages (look, clock & timer, widgets, music & lyrics,
controls, window) cover every everyday setting, show each change on the
lamp as you make it, and save on their own. This page is for people who
like a text file.

## Where it is

Settings are saved 1 second after a change, and when you quit, to
`config.toml` in:

| System | Folder |
|---|---|
| any, if `XDG_CONFIG_HOME` is set to a full path | `$XDG_CONFIG_HOME/lavatui/` |
| macOS | `~/Library/Application Support/lavatui/` |
| Linux | `~/.config/lavatui/` |
| Windows | `%APPDATA%\lavatui\config\` |

`lavatui --config <file>` reads and writes another file instead (handy to
try things without touching your own settings).

Command-line options (`-m`, `--style`, `--palette`, `--color`, `--fps`,
`--seed`) apply to that run only and are never written to the file. A
setting you change in the app is saved as usual.

## Every setting

Every key is optional. These are the defaults:

```toml
[display]
fps = 60                 # 1..=240
color = "auto"           # auto | truecolor | 256 | 16 | none
cell_aspect = 2.0        # cell height / width; used only when the terminal doesn't report pixels
cells = "auto"           # auto | opaque | translucent | background ("stripe fix", see below)

[lamp]
style = "solid"          # solid | outline | ascii | braille | halftone | synthwave | matrix | topo | chrome
heat = 3                 # 1..=5
speed = 1.0              # 0.25 | 0.5 | 1 | 2 | 4
top_wax = false          # a thin layer of wax under the top (see below)

[theme]
palette = "lava"         # lava | ultraviolet | abyss | toxic | synthwave | mono | paper | ansi
transparent = false      # true = never paint the background (keeps terminal transparency)

[clock]
face = "blocks"          # blocks | segment | analog | binary | words | text
hour24 = true
seconds = true           # seconds on the big clock in the side panel; false = 14:32
                         # at every size (no second hand, no seconds column)

[pomodoro]
focus_min = 25
short_break_min = 5
long_break_min = 15
cycles = 4               # focus sessions before a long break
bell = true

[ui]
mode = "full"            # full | minimal (lamp only)
status_bar = true
welcome = true           # the welcome card at start; off once dismissed (w shows it)

[minimal]
clock = "corner"         # corner | off

[input]
mouse = true             # shift-drag (option-drag in macOS Terminal and iTerm2) still selects text

[dock]
clock = "side"           # side | overlay | off  (overlay = on the lamp)
pomodoro = "side"        # side | overlay | off
music = "off"            # side | overlay | off
lyrics = "off"           # side | overlay | off (on = lookups on lrclib.net)
cover = "off"            # side | overlay | off
# each item's spot on the lamp:
# center | top | top-right | bottom-right | bottom | bottom-left | top-left
anchor = { clock = "center", pomodoro = "center", music = "top-left", lyrics = "bottom", cover = "top-right" }
backing = "none"         # none (text floats on the lamp) | soft (a veiled pool behind)
text = "auto"            # text on the lamp: auto (light or dark per letter, by the wax
                         # behind it) | light | dark (always the palette's light / dark ink)

[art]
detail = "auto"          # auto (= sharp) | sharp (the real picture where the terminal shows
                         # pictures, else the finest text) | small-pixels (pixel art, ~32
                         # squares across) | medium-pixels (~16) | big-pixels (~10); older
                         # pixels / photo / sextant / fine load as sharp, quadrant / medium as
                         # medium-pixels, halfblock / coarse as big-pixels
size = "medium"          # small (16 cols) | medium (24) | large (34) | fill (up to 64)
inline = true            # the music card's own small cover (while the cover widget is off)

[spotify]
client_id = ""           # for the Spotify library, see spotify.md
                         # ("" = off; LAVATUI_SPOTIFY_CLIENT_ID works too)
store = "system"         # where the login is kept: system (Keychain, Credential
                         # Manager, Secret Service) | file (0600, in the data folder)
logged_in = false        # kept up to date by the app: a login is saved
                         # (not a secret; lets it say "connected" without
                         # reading the login, so macOS doesn't ask at start)
```

### Styles

| Style | What it looks like |
|---|---|
| `solid` | Smooth wax in half blocks, coloured by temperature, with soft edges. The default. |
| `outline` | Just the wax surface, as a thin dotted line. |
| `ascii` | A classic ` .:-=+*#%@` ramp, denser toward the hot core. |
| `braille` | Filled wax in braille dots (2×4 a cell), thinning toward the skin. |
| `halftone` | Newsprint: a 45° screen of round dots that swell toward the core. |
| `synthwave` | A 1986 sunset: striped sun blobs over a neon grid. |
| `matrix` | Digital rain that only shows where it crosses the wax. |
| `topo` | A topographic map of the wax, with contour lines. |
| `chrome` | Glossy blown glass with a glint and a rim. (`glass` still works as an old name.) |

### Wax at the top (`lamp.top_wax`)

Real lava lamps often have a thin layer of wax resting under the top.
`top_wax = true` (settings › look › *wax at the top*) adds one: a thin,
slightly uneven layer of the coolest wax colour. A blob that rises to it
and has cooled a little can stick (small, still-warm ones bump it and
turn back; about a third of the blobs that reach the top stick). It
flattens against the layer and seeps in over a few seconds, slower than
at the hot pool; a big one gives part of itself and the rest pulls away
and sinks. Where wax joined, the layer bulges, briefly warmer, then
sags into a hanging teardrop that snaps off and falls, or spreads out
and evens. It takes its wax from the pool and gives it back, so the
lamp holds the same amount of wax. Turning it on or off fades it in or out over a
second or two. It's drawn at least 1.5 and at most 5 sample pixels deep,
so it stays a thin layer at any window size. Off by default.

### Colour themes

| Palette | Mood |
|---|---|
| `lava` | The 1970s original: red-orange wax in amber oil. The default. |
| `ultraviolet` | A blacklight poster: violet to hot pink in deep indigo. |
| `abyss` | Deep sea: teal wax glowing to seafoam in navy water. |
| `toxic` | Radioactive slime: moss to acid yellow-green. |
| `synthwave` | Hot pink → coral → gold, with a cyan accent. |
| `mono` | Graphite grey. It suits halftone and braille. |
| `paper` | The light theme: rust-red ink on cream, for light terminals. |
| `ansi` | Your terminal's own 16 colours and background. |

### Stripe fix (`display.cells`)

- `auto` (the default) picks for your terminal.
- `translucent`: for a see-through terminal that draws cell backgrounds
  see-through but characters solid (Ghostty with `background-opacity`
  below 1 and `background-opacity-cells = true`). Halves of a cell that
  look alike become one colour, so the wax shows no half-row stripes.
  `auto` reads Ghostty's config files once at start to find this.
- `background`: for terminals that draw block characters a little short
  of the cell, leaving dark lines between rows (macOS Terminal, Ghostex).
  Wax is drawn as cell background wherever it can be. `auto` picks it for
  `TERM_PROGRAM=Apple_Terminal` and Ghostex; inside tmux in Terminal,
  set it by hand.
- `opaque`: neither.

## Editing by hand

The file is meant to be edited by hand, even while the lamp runs.

- A bad value (or a style, palette or face that doesn't exist) is
  ignored and the rest of the file still applies. A value out of range is
  clamped (`config: lamp.heat 99 → 5`). A message names the first
  problem; if there are more, all of them are printed when you quit.
- A TOML syntax error is reported with its line, and the lamp starts from
  the defaults.
- Keys LavaTUI doesn't know are reported but kept.

Saving only writes the settings you changed in the app, into the file as
it is at that moment, so your hand edits survive. Comments, key order
and unknown keys are kept, and saves write through symlinks, so a dotfile
manager's link stays intact. If a save would overwrite something LavaTUI
couldn't use (an ignored or clamped value, a broken file, bytes that
aren't UTF-8), the file is first copied to `config.toml.bak`. A file
that's mid-edit and not valid TOML is left alone until it is. A
read-only file (or folder), or a path that isn't a regular file
(`/dev/null`, a fifo), is never written; a message says so once.

## Settings from older versions

They load without a word: a single `dock.anchor = "top"` puts every item
there (saved per item next time), `lamp.frame` and `lamp.lighting` are
ignored (and dropped at the next save), `clock.show = false` becomes
`dock.clock = "off"`, a removed style (`heatmap`, `dither`, `crt`)
becomes `solid`, and `minimal.clock = "under"` means `corner`.

## Environment variables

| Variable | What it does |
|---|---|
| `NO_COLOR` | no colour at all (unless `--color` says otherwise) |
| `LAVATUI_SPOTIFY_CLIENT_ID` | the Spotify Client ID, instead of `[spotify] client_id` |
| `LAVATUI_SPOTIFY_TOKEN_FILE` | keep the Spotify login in this file instead of the system password store |
| `LAVATUI_GRAPHICS` | `kitty`, `iterm`, `sixel` or `none`: how to show album covers as pictures, if your terminal isn't recognised |
| `LAVATUI_GLYPHS` | `safe` or `rich`: the set of symbols widgets use |
| `XDG_CONFIG_HOME`, `XDG_CACHE_HOME` | where settings and the cover / lyrics caches go (`lavatui/art`, `lavatui/lyrics`; trimmed automatically, cleared from settings › music & lyrics) |
