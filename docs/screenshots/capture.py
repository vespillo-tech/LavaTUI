"""Regenerate the README screenshots in docs/screenshots/.

Runs the release binary in sized ptys (fixed --seed, scratch config),
emulates the terminal with pyte and renders the last frame to PNG: block
elements, braille and box drawing are drawn as pixels, other glyphs from
a monospace font. Clock times are whatever the local time is.

    cargo build --release
    python3 -m venv /tmp/v && /tmp/v/bin/pip install pyte pillow
    /tmp/v/bin/python docs/screenshots/capture.py            # all
    /tmp/v/bin/python docs/screenshots/capture.py hero help  # some

Fonts default to macOS Menlo; set LAVATUI_SHOT_FONT to a .ttf/.ttc
elsewhere (e.g. DejaVuSansMono.ttf).
"""
import fcntl, os, pty, select, struct, sys, tempfile, termios, time
from concurrent.futures import ThreadPoolExecutor

import pyte
from PIL import Image, ImageDraw, ImageFont

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


def run(args):
    cfg = tempfile.NamedTemporaryFile("w", suffix=".toml", delete=False)
    cfg.write(args.toml.replace(";", "\n"))
    cfg.close()
    argv = [BIN, "--config", cfg.name, "--frames", str(args.frames)] + args.app
    keys = []
    for k in filter(None, args.keys.split(",")):
        t, s = k.split(":", 1)
        keys.append((float(t), s.encode().decode("unicode_escape").encode()))
    env = dict(os.environ, TERM="xterm-256color", COLORTERM="truecolor")
    env.pop("NO_COLOR", None)
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
                    stream.feed(data[:i])
                    done = True
                else:
                    stream.feed(data)
        if el > 60:
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


def mix(a, b, t):
    return tuple(round(x + (y - x) * t) for x, y in zip(a, b))


def render(screen, out, pad=0):
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
    if pad:
        framed = Image.new("RGB", (w + 2 * pad, h + 2 * pad), DEF_BG)
        framed.paste(img, (pad, pad))
        img = framed
    img = img.quantize(colors=256, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.NONE)
    img.save(out, optimize=True)


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
    def __init__(self, cols, rows, toml, keys="", args="--seed 2", frames=300):
        self.cols, self.rows, self.toml, self.keys = cols, rows, toml, keys
        self.app, self.frames, self.env = args.split(), frames, []


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
    "color16": Shot(80, 24, '[lamp];style="ascii"', args="--seed 2 --color 16"),
}
TILES = {f"style-{s}": Shot(34, 30, TILE + f'style="{s}"') for s in STYLES}
TILES |= {f"palette-{p}": Shot(34, 30, TILE + f'style="solid";[theme];palette="{p}"') for p in PALETTES}


def main(names):
    tmp = tempfile.mkdtemp()
    want = names or list(SHOTS) + ["styles", "palettes"]
    jobs = {n: s for n, s in SHOTS.items() if n in want}
    if "styles" in want:
        jobs |= {n: s for n, s in TILES.items() if n.startswith("style-")}
    if "palettes" in want:
        jobs |= {n: s for n, s in TILES.items() if n.startswith("palette-")}

    def one(item):
        name, shot = item
        out = os.path.join(HERE if name in SHOTS else tmp, name + ".png")
        render(run(shot), out)
        return out

    with ThreadPoolExecutor(6) as ex:
        for out in ex.map(one, jobs.items()):
            print(out, os.path.getsize(out))
    if "styles" in want:
        montage(os.path.join(HERE, "styles.png"), 5, [(s, f"{tmp}/style-{s}.png") for s in STYLES])
    if "palettes" in want:
        montage(os.path.join(HERE, "palettes.png"), 8, [(p, f"{tmp}/palette-{p}.png") for p in PALETTES])


if __name__ == "__main__":
    main(sys.argv[1:])
