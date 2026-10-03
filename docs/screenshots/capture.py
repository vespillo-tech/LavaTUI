"""Regenerate the README screenshots in docs/screenshots/.

Runs the release binary in sized ptys (fixed --seed, scratch config),
emulates the terminal with pyte and renders the last frame to PNG: block
elements, braille and box drawing are drawn as pixels, other glyphs from
a monospace font. Clock times are whatever the local time is.

    cargo build --release
    python3 -m venv /tmp/v && /tmp/v/bin/pip install pyte pillow
    /tmp/v/bin/python docs/screenshots/capture.py            # all
    /tmp/v/bin/python docs/screenshots/capture.py hero help  # some
    /tmp/v/bin/python docs/screenshots/capture.py live       # needs Spotify playing
    /tmp/v/bin/python docs/screenshots/capture.py lyrics     # Spotify + lrclib.net
    /tmp/v/bin/python docs/screenshots/capture.py library    # --demo's made-up account
    /tmp/v/bin/python docs/screenshots/capture.py cover      # Spotify; text-cell covers
    /tmp/v/bin/python docs/screenshots/capture.py settings-pages  # every settings page
    /tmp/v/bin/python docs/screenshots/capture.py guide      # welcome card, music controls

Scratch configs say `[ui] welcome = false` (the card would cover every
shot) unless the shot is of the welcome card.

`music`, `music-lava` and `spotify-setup` run with the hidden `--demo`
flag: a made-up player with invented songs, original embedded covers
and invented lyrics, so they are safe to commit.

The live sets below read the real player instead:
`live` (the now-playing widget, beside the lamp and on the lava) and
`lyrics` (the lyrics widget at three sizes, on the lava over several
styles and in the side panel; it looks the playing track up on
lrclib.net) are never part of "all": they show whatever Spotify is
playing, so they're for checking the widgets, not for committing. They
land in $LAVATUI_SHOT_OUT (default: the temp dir). `cover` (the cover
widget in each cover quality, beside the lamp and on the lava, with
opaque and see-through cell backgrounds, plus `cover-qualities.png`
side by side) uses `--demo` covers and lands there too, as does `library`
(the playlist browser, a playlist's songs, the add list with its ✓ and
the "add it again?" question, over `--demo`'s made-up Spotify account;
`library-live` is the same over your real one).

pyte can't show kitty graphics, so the terminal's own variables that would
make `art.detail = "auto"` pick pixels (Ghostty's, kitty's) are dropped:
captures always show text cells. `tools/kitty_check.py` checks pixels.

Fonts default to macOS Menlo; set LAVATUI_SHOT_FONT to a .ttf/.ttc
elsewhere (e.g. DejaVuSansMono.ttf).
"""
import fcntl, os, pty, select, struct, subprocess, sys, tempfile, termios, time
from concurrent.futures import ThreadPoolExecutor

import pyte
from PIL import Image, ImageChops, ImageDraw, ImageFont, ImageStat

HERE = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.join(HERE, "..", "..", "target", "release", "lavatui")
CW, CH = 9, 18  # cell px
FONT_PATH = os.environ.get("LAVATUI_SHOT_FONT", "/System/Library/Fonts/Menlo.ttc")
FONT = ImageFont.truetype(FONT_PATH, 15, index=0)
try:
    FONT_B = ImageFont.truetype(FONT_PATH, 15, index=1)
except OSError:
    FONT_B = FONT
FALLBACKS = [ImageFont.truetype(p, 15) for p in (
    "/System/Library/Fonts/Apple Symbols.ttf",
    "/System/Library/Fonts/SFNSMono.ttf",
) if os.path.exists(p)]

DEF_FG, DEF_BG = (208, 200, 192), (16, 14, 14)
ANSI = {
    "black": (0, 0, 0), "red": (205, 49, 49), "green": (13, 188, 121), "brown": (229, 229, 16),
    "yellow": (229, 229, 16), "blue": (36, 114, 200), "magenta": (188, 63, 188), "cyan": (17, 168, 205),
    "white": (229, 229, 229), "brightblack": (102, 102, 102), "brightred": (241, 76, 76),
    "brightgreen": (35, 209, 139), "brightbrown": (245, 245, 67), "brightyellow": (245, 245, 67),
    "brightblue": (59, 142, 234), "brightmagenta": (214, 112, 214), "brightcyan": (41, 184, 219),
    "brightwhite": (255, 255, 255),
}


def colour(c, default):
    if c == "default":
        return default
    if c in ANSI:
        return ANSI[c]
    try:
        return tuple(int(c[i:i + 2], 16) for i in (0, 2, 4))
    except ValueError:
        return default


def with_welcome(toml, welcome):
    """`toml` with `[ui] welcome` set: off unless the shot is of the card."""
    flag = f"welcome={'true' if welcome else 'false'}"
    if "[ui];" in toml:
        return toml.replace("[ui];", f"[ui];{flag};", 1)
    return f"{toml};[ui];{flag}"


FRAME_END = b"\x1b[?2026l"  # the app wraps every frame in DEC 2026


class Snap:
    """The screen as one frame left it (what `draw` reads), and when."""

    def __init__(self, screen, at):
        self.columns, self.lines, self.at = screen.columns, screen.lines, at
        self.buffer = [[screen.buffer[y][x] for x in range(screen.columns)] for y in range(screen.lines)]


def run(args, snaps=None):
    """Run `args` in a pty and return its last screen. With `snaps` (a
    list), also append a `Snap` at the end of every frame, and keys may be
    timed by frame: `f42:x` sends x as soon as frame 42 has been drawn, so
    the app handles it before frame 43 in every run."""
    cfg = tempfile.NamedTemporaryFile("w", suffix=".toml", delete=False)
    cfg.write(with_welcome(args.toml, args.welcome).replace(";", "\n"))
    cfg.close()
    argv = [BIN, "--config", cfg.name, "--frames", str(args.frames)] + args.app
    keys, frame_keys = [], []
    for k in filter(None, args.keys.split(",")):
        t, s = k.split(":", 1)
        data = s.encode().decode("unicode_escape").encode()
        if t.startswith("f"):
            frame_keys.append((int(t[1:]), data))
        else:
            keys.append((float(t), data))
    frame_keys.sort(key=lambda k: k[0])
    env = dict(os.environ, TERM="xterm-256color", COLORTERM="truecolor")
    # A plain terminal: nothing from the one capture.py runs in (inside
    # Ghostex / zmx the app would draw its safe symbols).
    for k in ("NO_COLOR", "TERM_PROGRAM", "GHOSTTY_RESOURCES_DIR", "KITTY_WINDOW_ID", "TMUX", "ZMX_SESSION"):
        env.pop(k, None)
    for k in [k for k in env if k.startswith("GHOSTEX_")]:
        env.pop(k)
    for e in args.env:
        k, v = e.split("=", 1)
        if v:
            env[k] = v
        else:
            env.pop(k, None)
    pid, fd = pty.fork()
    if pid == 0:
        os.execve(BIN, argv, env)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", args.rows, args.cols, args.cols * 8, args.rows * 16))
    screen = pyte.Screen(args.cols, args.rows)
    stream = pyte.ByteStream(screen)
    start = time.time()
    done = False
    buf = b""
    while True:
        el = time.time() - start
        while keys and keys[0][0] <= el:
            os.write(fd, keys.pop(0)[1])
        r, _, _ = select.select([fd], [], [], 0.02)
        if r:
            try:
                data = os.read(fd, 65536)
            except OSError:
                break
            if not data:
                break
            if not done:
                i = data.find(b"\x1b[?1049l")
                if i >= 0:
                    data, done = data[:i], True
                if snaps is None:
                    stream.feed(data)
                else:
                    buf += data
                    while (j := buf.find(FRAME_END)) >= 0:
                        j += len(FRAME_END)
                        stream.feed(buf[:j])
                        buf = buf[j:]
                        snaps.append(Snap(screen, time.time()))
                        while frame_keys and frame_keys[0][0] < len(snaps):
                            os.write(fd, frame_keys.pop(0)[1])
        # A minute, or more for long recordings (10 fps loops and films).
        if el > max(60.0, args.frames / 10 + 15):
            os.kill(pid, 9)
            break
    os.waitpid(pid, 0)
    os.unlink(cfg.name)
    return screen


def notdef(font):
    return bytes(font.getmask("\U000F0000"))


ND = [notdef(f) for f in [FONT] + FALLBACKS]


def pick_font(ch, bold):
    for f, nd in zip([FONT_B if bold else FONT] + FALLBACKS, ND):
        if bytes(f.getmask(ch)) != nd:
            return f
    return FONT


BLOCKS = {  # (x0, y0, x1, y1) in eighths
    "▀": [(0, 0, 8, 4)], "▄": [(0, 4, 8, 8)], "█": [(0, 0, 8, 8)], "▌": [(0, 0, 4, 8)], "▐": [(4, 0, 8, 8)],
    "▔": [(0, 0, 8, 1)], "▁": [(0, 7, 8, 8)], "▂": [(0, 6, 8, 8)], "▃": [(0, 5, 8, 8)], "▅": [(0, 3, 8, 8)],
    "▆": [(0, 2, 8, 8)], "▇": [(0, 1, 8, 8)], "▉": [(0, 0, 7, 8)], "▊": [(0, 0, 6, 8)], "▋": [(0, 0, 5, 8)],
    "▍": [(0, 0, 3, 8)], "▎": [(0, 0, 2, 8)], "▏": [(0, 0, 1, 8)], "▕": [(7, 0, 8, 8)],
}
QUAD = {"▖": "0010", "▗": "0001", "▘": "1000", "▝": "0100", "▙": "1011", "▚": "1001", "▛": "1110",
        "▜": "1101", "▞": "0110", "▟": "0111"}
for k, m in QUAD.items():
    BLOCKS[k] = [r for r, b in zip([(0, 0, 4, 4), (4, 0, 8, 4), (0, 4, 4, 8), (4, 4, 8, 8)], m) if b == "1"]
BOX = {"─": (1, 1, 0, 0, 1), "━": (1, 1, 0, 0, 2), "│": (0, 0, 1, 1, 1), "╭": (0, 1, 0, 1, 1),
       "╮": (1, 0, 0, 1, 1), "╰": (0, 1, 1, 0, 1), "╯": (1, 0, 1, 0, 1)}
SHADE = {"░": 0.25, "▒": 0.5, "▓": 0.75}
# Block sextants (U+1FB00..U+1FB3B): 2 × 3 cells, bit i = cell i, left to
# right, top to bottom; every pattern but empty, full and the half blocks.
SEXT = {}
for m in range(1, 63):
    if m not in (21, 42):
        SEXT[chr(0x1FB00 + m - 1 - (m > 21) - (m > 42))] = m


def mix(a, b, t):
    return tuple(round(x + (y - x) * t) for x, y in zip(a, b))


def render(screen, out, pad=0):
    img = draw(screen)
    if pad:
        framed = Image.new("RGB", (img.width + 2 * pad, img.height + 2 * pad), DEF_BG)
        framed.paste(img, (pad, pad))
        img = framed
    img = img.quantize(colors=256, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.NONE)
    img.save(out, optimize=True)


def draw(screen):
    """`screen` as an RGB picture, CW × CH px a cell."""
    w, h = screen.columns * CW, screen.lines * CH
    img = Image.new("RGB", (w, h), DEF_BG)
    d = ImageDraw.Draw(img)
    for y in range(screen.lines):
        row = screen.buffer[y]
        for x in range(screen.columns):
            c = row[x]
            fg, bg = colour(c.fg, DEF_FG), colour(c.bg, DEF_BG)
            if c.reverse:
                fg, bg = bg, fg
            px, py = x * CW, y * CH
            ch = c.data or " "
            if ch in SHADE:
                bg = mix(bg, fg, SHADE[ch])
                ch = " "
            d.rectangle([px, py, px + CW - 1, py + CH - 1], fill=bg)
            if ch == " ":
                continue
            if ch in BLOCKS:
                for x0, y0, x1, y1 in BLOCKS[ch]:
                    d.rectangle([px + x0 * CW // 8, py + y0 * CH // 8,
                                 px + x1 * CW // 8 - 1, py + y1 * CH // 8 - 1], fill=fg)
            elif ch in SEXT:
                m = SEXT[ch]
                for i in range(6):
                    if m >> i & 1:
                        x0, y0 = px + (i % 2) * CW // 2, py + (i // 2) * CH // 3
                        x1, y1 = px + (i % 2 + 1) * CW // 2, py + (i // 2 + 1) * CH // 3
                        d.rectangle([x0, y0, x1 - 1, y1 - 1], fill=fg)
            elif ch in BOX:
                l, r, u, dn, wgt = BOX[ch]
                cx, cy = px + CW // 2, py + CH // 2
                t = wgt
                if ch in "╭╮╰╯":
                    # rounded corner: quarter arc
                    rad = CW // 2
                    if ch == "╭": bb, a0 = [cx, cy, cx + 2 * rad, cy + 2 * rad], 180
                    if ch == "╮": bb, a0 = [cx - 2 * rad, cy, cx, cy + 2 * rad], 270
                    if ch == "╰": bb, a0 = [cx, cy - 2 * rad, cx + 2 * rad, cy], 90
                    if ch == "╯": bb, a0 = [cx - 2 * rad, cy - 2 * rad, cx, cy], 0
                    d.arc(bb, a0, a0 + 90, fill=fg, width=t)
                    if ch in "╭╮": d.rectangle([cx, cy + rad, cx + t - 1, py + CH - 1], fill=fg)
                    if ch in "╰╯": d.rectangle([cx, py, cx + t - 1, cy - rad], fill=fg)
                    if ch in "╭╰": d.rectangle([cx + rad, cy, px + CW - 1, cy + t - 1], fill=fg)
                    if ch in "╮╯": d.rectangle([px, cy, cx - rad, cy + t - 1], fill=fg)
                    continue
                if l: d.rectangle([px, cy, cx, cy + t - 1], fill=fg)
                if r: d.rectangle([cx, cy, px + CW - 1, cy + t - 1], fill=fg)
                if u: d.rectangle([cx, py, cx + t - 1, cy], fill=fg)
                if dn: d.rectangle([cx, cy, cx + t - 1, py + CH - 1], fill=fg)
            elif 0x2800 <= ord(ch) <= 0x28FF:
                bits = ord(ch) - 0x2800
                dots = [(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 2), (0, 3), (1, 3)]
                for i, (dx, dy) in enumerate(dots):
                    if bits >> i & 1:
                        cx = px + (dx * 2 + 1) * CW / 4
                        cy = py + (dy * 2 + 1) * CH / 8
                        r = 1.6
                        d.ellipse([cx - r, cy - r, cx + r, cy + r], fill=fg)
            else:
                f = pick_font(ch, c.bold)
                d.text((px + CW / 2, py + CH / 2 + 1), ch, font=f, fill=fg, anchor="mm")
    return img


def montage(out, ncols, items):
    ims = [Image.open(p).convert("RGB") for _, p in items]
    w, h = ims[0].size
    lab, gap = 30, 6
    rows = (len(ims) + ncols - 1) // ncols
    m = Image.new("RGB", (ncols * w + (ncols - 1) * gap, rows * (h + lab) + (rows - 1) * gap), DEF_BG)
    d = ImageDraw.Draw(m)
    for i, (im, (name, _)) in enumerate(zip(ims, items)):
        x, y = (i % ncols) * (w + gap), (i // ncols) * (h + lab + gap)
        m.paste(im, (x, y))
        d.text((x + w / 2, y + h + lab / 2), name, font=FONT, fill=(150, 140, 130), anchor="mm")
    m = m.quantize(colors=256, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.NONE)
    m.save(out, optimize=True)


class Shot:
    def __init__(self, cols, rows, toml, keys="", args="--seed 2", frames=300, welcome=False):
        self.cols, self.rows, self.toml, self.keys = cols, rows, toml, keys
        self.app, self.frames, self.env = args.split(), frames, []
        self.welcome = welcome


STYLES = "solid outline ascii braille halftone synthwave matrix topo chrome".split()
PALETTES = "lava ultraviolet abyss toxic synthwave mono paper ansi".split()
TILE = '[ui];mode="minimal";[minimal];clock="off";[lamp];'
SHOTS = {
    "hero": Shot(120, 36, '[lamp];style="solid"', "0.5: ", frames=360),
    "minimal": Shot(80, 24, '[lamp];style="braille";[ui];mode="minimal";[theme];palette="ultraviolet"', args="--seed 3"),
    "help": Shot(100, 30, '[lamp];style="solid"', "1:?", frames=240),
    "picker": Shot(100, 30, '[lamp];style="solid";[theme];palette="synthwave"', "1:S,2:j,2.5:j", "--seed 4"),
    "portrait": Shot(36, 56, '[lamp];style="halftone";[theme];palette="abyss"', "0.5: ", "--seed 6"),
    "tiny": Shot(26, 10, '[lamp];style="solid"'),
    "overlay": Shot(100, 30, '[lamp];style="solid";[dock];clock="overlay";pomodoro="overlay"', "0.5: "),
    "overlay-mix": Shot(100, 30, '[lamp];style="braille";[theme];palette="abyss";[dock];clock="overlay"', "0.5: ", "--seed 5"),
    "color16": Shot(80, 24, '[lamp];style="ascii"', args="--seed 2 --color 16"),
    "settings": Shot(100, 30, '[lamp];style="solid"', "1:\\x2c,1.5:\\r,2:j", frames=240),
    # The music widgets, with `--demo`'s made-up songs, cover and lyrics
    # (never a real player: no real cover or song in a committed image).
    "music": Shot(
        120, 36, '[lamp];style="solid";[dock];music="side";cover="overlay";lyrics="overlay";pomodoro="off"',
        args="--seed 2 --demo", frames=480,
    ),
    "music-lava": Shot(
        120, 36,
        '[lamp];style="braille";[theme];palette="abyss";[dock];music="overlay";lyrics="overlay";clock="overlay";pomodoro="off"',
        args="--seed 5 --demo", frames=480,
    ),
    "spotify-setup": Shot(
        100, 30, '[lamp];style="solid"', "1:\\x2c,1.2:j,1.4:j,1.6:j,2:\\r,2.5:\\r",
        args="--seed 2 --demo", frames=300,
    ),
    "welcome": Shot(80, 24, '[lamp];style="solid"', args="--seed 3", frames=120, welcome=True),
}
# Live: whatever Spotify plays (never in "all", never committed).
LIVE = {
    "live-music-side": Shot(120, 36, '[lamp];style="solid";[dock];music="side"', frames=420),
    "live-music-lava": Shot(120, 36, '[lamp];style="braille";[theme];palette="abyss";[dock];music="overlay"', frames=420),
}
# The library UI over `--demo`'s made-up account. `capture.py library`.
LIBRARY = {
    "library-demo-widget": Shot(120, 36, '[lamp];style="solid";[dock];music="side"', args="--seed 2 --demo", frames=420),
    "library-demo-playlists": Shot(120, 36, '[lamp];style="solid";[dock];music="side"', "2:A,3:b,4:j", args="--seed 2 --demo", frames=420),
    "library-demo-tracks": Shot(120, 36, '[lamp];style="solid";[dock];music="side"', "2:A,3:b,4:\\r,5:j", args="--seed 2 --demo", frames=480),
    "library-demo-add": Shot(120, 36, '[lamp];style="braille";[theme];palette="abyss";[dock];music="side"', "2:A,3:a", args="--seed 5 --demo", frames=420),
    "library-demo-again": Shot(120, 36, '[lamp];style="braille";[theme];palette="abyss";[dock];music="side"', "2:A,3:a,4:\\r", args="--seed 5 --demo", frames=420),
    "library-demo-small": Shot(60, 18, '[lamp];style="solid";[dock];music="side"', "2:A,3:a", args="--seed 2 --demo", frames=420),
}
# Live, logged in to the Web API (needs LAVATUI_SPOTIFY_CLIENT_ID and a
# LAVATUI_SPOTIFY_TOKEN_FILE from a login, e.g. live_library's): the
# library UI over your real playlists. `capture.py library-live`.
LIBRARY_LIVE = {
    "library-widget": Shot(120, 36, '[lamp];style="solid";[dock];music="side"', frames=420),
    "library-playlists": Shot(120, 36, '[lamp];style="solid";[dock];music="side"', "2:A,3:b,4:j", frames=420),
    "library-tracks": Shot(120, 36, '[lamp];style="solid";[dock];music="side"', "2:A,3:b,4:\\r,5:j", frames=480),
    "library-add": Shot(120, 36, '[lamp];style="braille";[theme];palette="abyss";[dock];music="side"', "2:A,3:a", frames=420),
    "library-small": Shot(60, 18, '[lamp];style="solid";[dock];music="side"', "2:A,3:b", frames=420),
    "library-tiny": Shot(28, 10, '[lamp];style="solid";[dock];music="side"', "2:A,3:b,4:j", frames=420),
}
LYRICS = {
    f"lyrics-{c}x{r}-{style}": Shot(c, r, f'[lamp];style="{style}";[dock];lyrics="overlay"', frames=600)
    for (c, r) in [(80, 24), (120, 36), (200, 50)]
    for style in ["solid", "braille", "ascii", "halftone", "synthwave"]
}
LYRICS |= {
    f"lyrics-side-{c}x{r}": Shot(c, r, '[lamp];style="solid";[dock];lyrics="side"', frames=600)
    for (c, r) in [(80, 24), (120, 36), (200, 50)]
}
LYRICS["lyrics-with-music-200x50"] = Shot(
    200, 50, '[lamp];style="topo";[theme];palette="abyss";[dock];music="overlay";lyrics="overlay"', frames=600
)
COVER = {
    f"cover-{place}-{detail}{cells}": Shot(
        120, 36,
        f'[lamp];style="{style}";[theme];palette="{pal}";[dock];music="{place}";cover="{place}";'
        f'[art];detail="{detail}";size="large";[display];cells="{cells or "opaque"}"',
        args="--seed 2 --demo", frames=420,
    )
    for detail in ["sharp", "small-pixels", "medium-pixels", "big-pixels"]
    for (place, style, pal) in [("side", "solid", "lava"), ("overlay", "braille", "abyss")]
    # "-translucent": as Ghostty draws see-through cell backgrounds.
    for cells in ["", "translucent"]
}
COVER |= {
    "cover-fill-200x50": Shot(200, 50, '[lamp];style="solid";[dock];cover="side";[art];size="fill";detail="sharp"', args="--seed 2 --demo", frames=420),
    "cover-small-80x24": Shot(80, 24, '[lamp];style="solid";[dock];cover="overlay";[art];size="small"', args="--seed 2 --demo", frames=420),
    "cover-inline-120x36": Shot(120, 36, '[lamp];style="solid";[dock];music="side";[art];detail="medium-pixels"', args="--seed 2 --demo", frames=420),
    "cover-256-120x36": Shot(120, 36, '[lamp];style="solid";[dock];cover="side";[art];detail="big-pixels"', args="--seed 2 --demo --color 256", frames=420),
    "cover-16-80x24": Shot(80, 24, '[lamp];style="ascii";[dock];cover="side"', args="--seed 2 --demo --color 16", frames=420),
    "cover-tiny-30x10": Shot(30, 10, '[lamp];style="solid";[dock];cover="overlay"', args="--seed 2 --demo", frames=420),
}
# (`\\x2c` is `,`: the key list is comma-separated.)
# Every page of the settings screen at two sizes (`capture.py
# settings-pages`); the Spotify setup asks the Spotify app how it is, so
# these land in $LAVATUI_SHOT_OUT like the live ones.
SETTINGS_PAGES = {}
for (c, r) in [(80, 24), (40, 14)]:
    SETTINGS_PAGES[f"settings-{c}x{r}-pages"] = Shot(c, r, '[lamp];style="solid"', "1:\\x2c", frames=240)
    for i, page in enumerate(["look", "clock", "widgets", "music", "controls", "window"]):
        down = "".join(f",{1.2 + 0.2 * k:.1f}:j" for k in range(i))
        SETTINGS_PAGES[f"settings-{c}x{r}-{page}"] = Shot(
            c, r, '[lamp];style="solid"', f"1:\\x2c{down},2.6:\\r", frames=240
        )
    SETTINGS_PAGES[f"settings-{c}x{r}-spotify"] = Shot(
        c, r, '[lamp];style="solid"', "1:\\x2c,1.2:j,1.4:j,1.6:j,2:\\r,2.5:\\r", frames=300
    )
# First-run guidance (lava-1xk.9, .6): the welcome card, and the music
# controls' line after its toast has gone (reads the live player, so never
# committed). `capture.py guide`.
GUIDE = {
    f"welcome-{c}x{r}": Shot(c, r, '[lamp];style="solid"', frames=120, welcome=True)
    for (c, r) in [(80, 24), (30, 10), (20, 8)]
}
GUIDE |= {
    f"music-controls-{c}x{r}": Shot(
        c, r, '[lamp];style="solid";[ui];mode="minimal";[dock];music="side"', "1:A", frames=330
    )
    for (c, r) in [(80, 24), (30, 10)]
}
TILES = {f"style-{s}": Shot(34, 30, TILE + f'style="{s}"') for s in STYLES}
TILES |= {f"palette-{p}": Shot(34, 30, TILE + f'style="solid";[theme];palette="{p}"') for p in PALETTES}


# Loops (`capture.py loops [name ...]`): the README pictures as short GIFs
# that repeat seamlessly. Only the wax moves (a cinemagraph): the timer
# isn't started, the big clock has no seconds, the demo player is paused
# and every toast has gone before the loop starts. Each frame the app
# draws is kept (split on its DEC 2026 frame ends); the loop is the
# >= LOOP_SECS stretch whose end is most like the frame before its start,
# all inside one wall-clock minute, with its last LOOP_FADE seconds
# crossfaded into the frames that lead into the start. Sheet tiles are
# separate runs of the same seed: frame k is the same wax in each.
# Encoded with ffmpeg (one palette, no dither) + gifsicle; written to
# $LAVATUI_LOOP_OUT (default: here, as <name>.gif).
LOOP_FPS, LOOP_SECS = 10, 6.0
LOOP_FADE = float(os.environ.get("LAVATUI_LOOP_FADE", 1.0))
LOOP_RECORD = float(os.environ.get("LAVATUI_LOOP_RECORD", 54.0))
PAUSE = "3:A,3.2: ,3.4:A"  # music controls on, pause, off again


class Loop:
    def __init__(self, shot=None, tiles=None, ncols=0, scale=0.65, colours=96, clock=True):
        self.shot, self.tiles, self.ncols = shot, tiles, ncols
        self.scale, self.colours, self.clock = scale, colours, clock


def still(shot, keys=None):
    """`shot` recorded for a loop: LOOP_RECORD s at LOOP_FPS, no seconds on
    the clock, and `keys` instead of its own (None keeps them)."""
    return Shot(
        shot.cols, shot.rows, shot.toml + ";[clock];seconds=false",
        shot.keys if keys is None else keys,
        " ".join(shot.app) + f" --fps {LOOP_FPS}",
        frames=int(LOOP_RECORD * LOOP_FPS) + 5, welcome=shot.welcome,
    )


def tile(shot):
    return Shot(shot.cols, shot.rows, shot.toml, shot.keys, " ".join(shot.app) + f" --fps {LOOP_FPS}",
                frames=int(LOOP_RECORD * LOOP_FPS) + 5)


LOOPS = {
    "palettes": Loop(tiles=[(p, tile(TILES[f"palette-{p}"])) for p in PALETTES], ncols=4, clock=False),
    "styles": Loop(tiles=[(n, tile(TILES[f"style-{n}"])) for n in STYLES], ncols=5, clock=False),
    "music": Loop(still(SHOTS["music"], PAUSE)),
    "music-lava": Loop(still(SHOTS["music-lava"], PAUSE)),
    "overlay": Loop(still(SHOTS["overlay"], "")),
    "overlay-mix": Loop(still(SHOTS["overlay-mix"], "")),
    "settings": Loop(still(SHOTS["settings"])),
    "spotify-setup": Loop(still(SHOTS["spotify-setup"])),
    "help": Loop(still(SHOTS["help"])),
    "picker": Loop(still(SHOTS["picker"])),
    "minimal": Loop(still(SHOTS["minimal"])),
    "welcome": Loop(still(SHOTS["welcome"])),
    "portrait": Loop(still(SHOTS["portrait"], "")),
    "tiny": Loop(still(SHOTS["tiny"]), scale=1.0),
    "color16": Loop(still(SHOTS["color16"]), colours=16),
}


def last_key(shot):
    return max([float(k.split(":", 1)[0]) for k in filter(None, shot.keys.split(","))] or [0.0])


def record(shot, clock):
    """Every frame of `shot`, started early in a minute when a clock shows
    (so a whole loop fits before it turns)."""
    while clock and time.localtime().tm_sec > 1:
        time.sleep(0.2)
    snaps = []
    run(shot, snaps)
    return snaps


def seam(snaps, warm, clock):
    """(start, length) of the best loop in `snaps`: frames from `warm` on."""
    thumbs = [draw(s).convert("L").reduce(4) for s in snaps]
    n, c = len(snaps), int(LOOP_FADE * LOOP_FPS)
    minute = [int(s.at // 60) for s in snaps]
    best = None
    for length in range(int(LOOP_SECS * LOOP_FPS), int(LOOP_SECS * LOOP_FPS) + 11, 5):
        for i in range(max(int(warm * LOOP_FPS), c), n - length + 1):
            if clock and minute[i - c] != minute[i + length - 1]:
                continue
            cost = ImageStat.Stat(ImageChops.difference(thumbs[i - 1], thumbs[i + length - 1])).mean[0]
            if best is None or cost < best[0]:
                best = (cost, i, length)
    if best is None:
        raise SystemExit("no loop fits: record longer")
    steps = sorted(ImageStat.Stat(ImageChops.difference(a, b)).mean[0] for a, b in zip(thumbs, thumbs[1:]))
    return best[1], best[2], best[0], steps[len(steps) // 2]


def loop_frames(images, start, length):
    """The loop's frames from `images(k)` (frame k): the last LOOP_FADE s
    crossfaded into the frames just before `start`."""
    c = int(LOOP_FADE * LOOP_FPS)
    out = []
    for j in range(length):
        a = images(start + j)
        if j >= length - c:
            a = Image.blend(a, images(start + j - length), (j - (length - c) + 1) / (c + 1))
        out.append(a)
    return out


def swing_at(u, ease=0.2):
    """Where a swing is (0..1..0) at `u` (0..1) of its loop: out and back,
    at an even pace but for an `ease` share of each leg at its ends, where
    it slows to a stop (no jolt as it turns)."""
    v = 2 * u if u < 0.5 else 2 - 2 * u
    peak = 1 / (1 - ease)  # the even pace, so the leg still ends at 1

    def dist(x):  # distance after x of a leg, its pace ramping 0 → peak → 0
        if x < ease:
            return peak * x * x / (2 * ease)
        if x > 1 - ease:
            return 1 - peak * (1 - x) ** 2 / (2 * ease)
        return peak * (x - ease / 2)

    return dist(v)


def swing_frames(images, start, span, n):
    """`n` frames that drift forward through frames start..start+span and
    back again, easing to a stop at both ends; in-between times blend the
    two nearest frames."""
    out = []
    for j in range(n):
        t = start + span * swing_at(j / n)
        k = int(t)
        f = t - k
        a = images(k)
        out.append(a if f < 0.01 else Image.blend(a, images(k + 1), f))
    return out


def liveliest(snaps, warm, span, clock):
    """The start of the `span`-frame stretch (from `warm` s on, inside one
    minute when a clock shows) where the wax moves most."""
    thumbs = [draw(s).convert("L").reduce(4) for s in snaps]
    steps = [ImageStat.Stat(ImageChops.difference(a, b)).mean[0] for a, b in zip(thumbs, thumbs[1:])]
    minute = [int(s.at // 60) for s in snaps]
    best = None
    for i in range(int(warm * LOOP_FPS), len(snaps) - span - 1):
        if clock and minute[i] != minute[i + span + 1]:
            continue
        motion = sum(steps[i:i + span])
        if best is None or motion > best[0]:
            best = (motion, i)
    if best is None:
        raise SystemExit("no stretch fits: record longer")
    return best[1]


def encode(frames, out, scale, colours):
    tmp = tempfile.mkdtemp()
    for j, im in enumerate(frames):
        if scale != 1:
            im = im.resize((round(im.width * scale), round(im.height * scale)), Image.LANCZOS)
        im.save(f"{tmp}/f{j:03d}.png")
    raw = f"{tmp}/raw.gif"
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-framerate", str(LOOP_FPS), "-i", f"{tmp}/f%03d.png",
                    "-vf", f"split[a][b];[a]palettegen=max_colors={colours}:stats_mode=full[p];"
                    "[b][p]paletteuse=dither=none", "-loop", "0", raw], check=True)
    subprocess.run(["gifsicle", "-O3", raw, "-o", out], check=True)
    return frames[0].size


def make_loop(name):
    lp = LOOPS[name]
    out = os.path.join(os.environ.get("LAVATUI_LOOP_OUT", HERE), name + ".gif")
    if lp.tiles:
        with ThreadPoolExecutor(len(lp.tiles)) as ex:
            runs = list(ex.map(lambda t: record(t[1], False), lp.tiles))
        n = min(len(r) for r in runs)
        if max(len(r) for r in runs) - n > 2:
            print(f"{name}: tiles drew {[len(r) for r in runs]} frames: out of step, run it alone")
        start, length, cost, step = seam(runs[0][:n], 3.0, False)
        cache = {}

        def sheet(k):
            if k not in cache:
                ims = [draw(r[k]) for r in runs]
                w, h = ims[0].size
                lab, gap = 30, 6
                rows = (len(ims) + lp.ncols - 1) // lp.ncols
                m = Image.new("RGB", (lp.ncols * w + (lp.ncols - 1) * gap, rows * (h + lab) + (rows - 1) * gap), DEF_BG)
                d = ImageDraw.Draw(m)
                for i, (im, (label, _)) in enumerate(zip(ims, lp.tiles)):
                    x, y = (i % lp.ncols) * (w + gap), (i // lp.ncols) * (h + lab + gap)
                    m.paste(im, (x, y))
                    d.text((x + w / 2, y + h + lab / 2), label, font=FONT, fill=(150, 140, 130), anchor="mm")
                cache[k] = m
            return cache[k]

        frames = loop_frames(sheet, start, length)
    elif os.environ.get("LAVATUI_LOOP_KIND", "swing") == "swing":
        snaps = record(lp.shot, lp.clock)
        n = int(LOOP_SECS * LOOP_FPS)
        span = n // 2  # each leg at about real speed
        start = liveliest(snaps, last_key(lp.shot) + 4.5, span, lp.clock)
        cache = {}
        frames = swing_frames(lambda k: cache.setdefault(k, draw(snaps[k])), start, span, n)
        length, cost, step = n, 0.0, 0.0
    else:
        snaps = record(lp.shot, lp.clock)
        start, length, cost, step = seam(snaps, last_key(lp.shot) + 4.5, lp.clock)
        frames = loop_frames(lambda k: draw(snaps[k]), start, length)
    w, h = encode(frames, out, lp.scale, lp.colours)
    print(f"{out} {os.path.getsize(out)} B {round(w * lp.scale)}x{round(h * lp.scale)} "
          f"{length / LOOP_FPS:.1f} s from {start / LOOP_FPS:.1f} s, seam {cost:.2f} (median step {step:.2f})")
    return out


def main(names):
    if names[:1] == ["loops"]:
        want = names[1:] or list(LOOPS)
        # Sheets alone: a busy machine makes the app skip late frames,
        # and then frame k is no longer the same wax in every tile.
        for name in [n for n in want if LOOPS[n].tiles]:
            make_loop(name)
        with ThreadPoolExecutor(3) as ex:
            list(ex.map(make_loop, [n for n in want if not LOOPS[n].tiles]))
        return
    tmp = tempfile.mkdtemp()
    want = names or list(SHOTS) + ["styles", "palettes"]
    jobs = {n: s for n, s in SHOTS.items() if n in want}
    if "live" in want:
        jobs |= LIVE
    if "lyrics" in want:
        jobs |= LYRICS
    if "library" in want:
        jobs |= LIBRARY
    if "library-live" in want:
        jobs |= LIBRARY_LIVE
    if "cover" in want:
        jobs |= COVER
    if "guide" in want:
        jobs |= GUIDE
    if "settings-pages" in want:
        jobs |= SETTINGS_PAGES
    if "styles" in want:
        jobs |= {n: s for n, s in TILES.items() if n.startswith("style-")}
    if "palettes" in want:
        jobs |= {n: s for n, s in TILES.items() if n.startswith("palette-")}

    def one(item):
        name, shot = item
        out = os.path.join(HERE if name in SHOTS else tmp, name + ".png")
        if name in LIVE or name in LYRICS or name in LIBRARY or name in LIBRARY_LIVE or name in COVER or name in GUIDE or name in SETTINGS_PAGES:
            live = os.environ.get("LAVATUI_SHOT_OUT", tempfile.gettempdir())
            out = os.path.join(live, name + ".png")
        render(run(shot), out)
        return out

    with ThreadPoolExecutor(6) as ex:
        for out in ex.map(one, jobs.items()):
            print(out, os.path.getsize(out))
    if "styles" in want:
        montage(os.path.join(HERE, "styles.png"), 5, [(s, f"{tmp}/style-{s}.png") for s in STYLES])
    if "cover" in want:
        live = os.environ.get("LAVATUI_SHOT_OUT", tempfile.gettempdir())
        montage(os.path.join(live, "cover-qualities.png"), 4, [
            (f"{d}{' (see-through cells)' if c else ''}", f"{live}/cover-side-{d}{c}.png")
            for c in ["", "translucent"] for d in ["sharp", "small-pixels", "medium-pixels", "big-pixels"]
        ])
    if "palettes" in want:
        montage(os.path.join(HERE, "palettes.png"), 8, [(p, f"{tmp}/palette-{p}.png") for p in PALETTES])


if __name__ == "__main__":
    main(sys.argv[1:])
