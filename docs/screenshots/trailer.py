"""The README trailer's takes and cuts, drawn headlessly: take A (lava-0bl).

The README shows the same film filmed in native Ghostty (take C, made by
trailer_native.py from these TAKES and CUT); this draws it with pyte, for
checking the cuts and for the README loops' frame-clock method.

A tour, cut together so it plays as one calm recording: the welcome
card; all nine styles and two colour themes as a slideshow of the same
lamp, and the style picker; music (the cover coming into focus, karaoke
beside the lamp, the music controls, playlists, a song played, liked
and added: "already in ... add it again?"; help; settings turning on
wax at the top); layouts (the clock, timer and music card float out of
the side panel into corner groups, and the lamp widens); a synthwave
ending, the lamp alone. The loop dissolves back into the first frame.

Every scene state is its own take: the same seed and window, the hidden
`--frame-clock` (time moves exactly one frame a frame, so frame k is the
same wax in every take, and the clock reads each take's start time), the
`--demo` player, no status bar. The film is frames of one take, then the
next, cut at exact frame numbers (CUT): at a cut, the style, colours and
widgets change in one frame while the wax carries on unbroken. Keys are
timed by frame (capture.py's `f<n>:`), and keys that change the wax (wax
at the top, the panel emptying) are replayed at the same frames in every
later take, so the worlds stay identical at each cut. Keys a take needs
before its first frame in the film have faded by then.

    cargo build --release
    /tmp/v/bin/python docs/screenshots/trailer.py            # the GIF
    /tmp/v/bin/python docs/screenshots/trailer.py check      # + cut frames ($LAVATUI_TRAILER_CHECK,
                                                             #   default <tmp>/lavatui-trailer-check)
    /tmp/v/bin/python docs/screenshots/trailer.py survey 7 2 3  # seeds, side by side

Frames are drawn by capture.py's renderer (pyte), each part (PALETTES)
gets its own palette (ffmpeg, no dither) and gifsicle joins them. Output:
$LAVATUI_TRAILER_OUT (default <tmp>/lavatui-trailer-A.gif).
"""
import os, subprocess, sys, tempfile
from concurrent.futures import ThreadPoolExecutor

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import capture  # noqa: E402
from PIL import Image, ImageChops, ImageStat  # noqa: E402

FPS, COLS, ROWS, SEED = 10, 100, 30, 7
SCALE, COLOURS = float(os.environ.get("LAVATUI_TRAILER_SCALE", 1.0)), 128
# The loop: late night dissolves into the first morning frame (the day
# turning) instead of jumping back to it.
DISSOLVE = 8


def toml(**sections):
    """capture.py's `;`-separated TOML from {section: {key: value}}."""
    def lit(v):
        if isinstance(v, dict):
            return "{ " + ", ".join(f"{k} = {lit(x)}" for k, x in v.items()) + " }"
        return ("true" if v else "false") if isinstance(v, bool) else f'"{v}"' if isinstance(v, str) else str(v)
    return ";".join(f"[{s}];" + ";".join(f"{k}={lit(v)}" for k, v in kv.items()) for s, kv in sections.items())


OFF = dict(clock="off", pomodoro="off", music="off", lyrics="off", cover="off")


def scene(style, palette, dock=None, minimal=False, art=None):
    ui = dict(status_bar=False, mode="minimal" if minimal else "full")
    sections = dict(
        ui=ui, minimal=dict(clock="off"), clock=dict(seconds=False),
        lamp=dict(style=style), theme=dict(palette=palette),
        dock=dict(OFF, backing="soft", **(dock or {})),
    )
    if art:
        sections["art"] = dict(detail=art)
    return toml(**sections)


# The side panel is 30 columns at 100, whatever it holds, so the lamp is
# the same in every take until night's `m` lets the walls ease out.
# The tour (lava-0bl v3). Times are frames (10 a second); keys are sent
# right after the frame they name, so every take handles them identically.
ESC, ENTER, RIGHT, COMMA = "\\x1b", "\\r", "\\x1b[C", "\\x2c"


def keys(*pairs):
    return ",".join(f"f{f}:{k}" for f, k in pairs)


# Looks: eight style steps (all nine styles), two palettes, then the style
# picker previewing two styles live and cancelling back.
LOOKS = keys((18, ESC), *((40 + 12 * i, "s") for i in range(8)), (136, "p"), (148, "p"),
             (160, "S"), (168, "k"), (176, "k"), (184, ESC))
# The next song (Blob Merge: brisk, every word timed): its first line comes
# just as the music scene starts.
SONG = keys((173, "A"), (175, "n"), (177, "A"))
# The library and the menus, all in the music scene.
TOUR = keys((250, "A"), (258, "b"), (267, ENTER), (274, "j"), (278, ENTER), (287, ESC), (288, ESC),
            (291, "s"), (302, "a"), (313, ENTER), (328, ESC), (329, ESC), (330, ESC),
            (338, "?"), (347, "j"), (351, "j"), (355, "j"), (364, ESC))
# Settings › look › wax at the top: on. It changes the wax, so every take
# from here on does it at the same frames.
TOP_WAX = keys((370, COMMA), (374, ENTER), (378, "jjjj"), (382, RIGHT), (400, ESC), (401, ESC))
# Clock, timer and music card float out of the panel onto the lava, one
# at a time; the panel empties at frame 465 (the music card leaves it) and
# the lamp widens. They land in the user's groups (their anchors are set):
# clock and timer top right, music card and cover bottom right.
PANEL = keys((435, "t"), (450, "f"), (465, "a"))
SIDE = dict(clock="side", pomodoro="side")
MUSIC = dict(music="side", lyrics="side", cover="overlay")
GROUPS = dict(
    clock="side", pomodoro="side", music="side", cover="overlay", text="light",
    anchor=dict(clock="top-right", pomodoro="top-right", music="bottom-right", cover="bottom-right"),
)


def join(*ks):
    return ",".join(k for k in ks if k)


# name: (config, keys, the clock's start[, welcome])
TAKES = {
    "open": (scene("solid", "lava", SIDE), LOOKS, "07:30", True),
    **{
        f"music-{d}": (scene("synthwave", "synthwave", MUSIC, art=d), SONG, "19:30")
        for d in ["big-pixels", "medium-pixels", "small-pixels"]
    },
    "music-sharp": (scene("synthwave", "synthwave", MUSIC, art="sharp"), join(SONG, TOUR, TOP_WAX), "19:30"),
    # Side panel → groups floating on the lava, held to enjoy.
    "layout": (scene("topo", "lava", GROUPS), join(TOP_WAX, PANEL), "22:30"),
    # The same history, then everything off before it shows: the lamp alone.
    "end": (scene("synthwave", "abyss", GROUPS), join(TOP_WAX, PANEL, keys(
        (470, "t"), (472, "f"), (474, "a"), (476, "o"))), "23:30"),
}

# (take, first frame, end frame): the film.
CUT = [
    ("open", 0, 200),
    ("music-big-pixels", 200, 222),
    ("music-medium-pixels", 222, 230),
    ("music-small-pixels", 230, 238),
    ("music-sharp", 238, 415),
    ("layout", 415, 540),
    ("end", 540, 590),
]
# Film frames that share a palette (the colours on screen then).
PALETTES = [(0, 136), (136, 148), (148, 200), (200, 415), (415, 540), (540, 590 + DISSOLVE)]


def take(name, seed=SEED, frames=None):
    cfg, ks, clock, *welcome = TAKES[name]
    end = frames or max(e for _, _, e in CUT) + DISSOLVE + 5
    shot = capture.Shot(COLS, ROWS, cfg, ks, f"--seed {seed} --fps {FPS} --demo --frame-clock {clock}",
                        frames=end, welcome=bool(welcome and welcome[0]))
    snaps = []
    capture.run(shot, snaps)
    return snaps


def record(names):
    with ThreadPoolExecutor(len(names)) as ex:
        takes = dict(zip(names, ex.map(take, names)))
    print("recorded:", {n: len(t) for n, t in takes.items()}, flush=True)
    return takes


def frame(takes, name, k):
    return capture.draw(takes[name][k])


def encode(parts, out):
    """`parts`: [(frames, colours)] → one GIF, each part its own palette."""
    tmp = tempfile.mkdtemp()
    gifs = []
    for i, frames in enumerate(parts):
        d = f"{tmp}/p{i}"
        os.makedirs(d)
        for j, im in enumerate(frames):
            im.resize((round(im.width * SCALE), round(im.height * SCALE)), Image.LANCZOS).save(f"{d}/f{j:04d}.png")
        g = f"{tmp}/p{i}.gif"
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-framerate", str(FPS), "-i", f"{d}/f%04d.png",
                        "-vf", f"split[a][b];[a]palettegen=max_colors={COLOURS}:stats_mode=full[p];"
                        "[b][p]paletteuse=dither=none", "-loop", "0", g], check=True)
        gifs.append(g)
    subprocess.run(["gifsicle", "-O3", "--no-warnings", "--merge", *gifs, "-o", out], check=True)


def film(takes):
    frames = [frame(takes, n, k) for n, a, b in CUT for k in range(a, b)]
    first, (name, _, end) = frames[0], CUT[-1]
    frames += [Image.blend(frame(takes, name, end + j), first, (j + 1) / (DISSOLVE + 1)) for j in range(DISSOLVE)]
    return [frames[a:b] for a, b in PALETTES]


def check(takes, out_dir):
    """The frames either side of every cut, side by side, and how much the
    lamp changes there against an ordinary frame step."""
    os.makedirs(out_dir, exist_ok=True)
    for (a, _, k), (b, _, _) in zip(CUT, CUT[1:]):
        left, right = frame(takes, a, k - 1), frame(takes, b, k)
        pair = Image.new("RGB", (left.width * 2 + 8, left.height))
        pair.paste(left, (0, 0))
        pair.paste(right, (left.width + 8, 0))
        pair.save(f"{out_dir}/cut-{k:03d}-{a}-to-{b}.png")
    print("cut frames:", out_dir)


def survey(seeds, out):
    """The morning take (no keys), a frame every 2 s for each seed: one row each."""
    TAKES["plain"] = (TAKES["open"][0], "", "07:30")
    with ThreadPoolExecutor(len(seeds)) as ex:
        runs = list(ex.map(lambda s: take("plain", s, 24 * FPS), seeds))
    w, h = COLS * capture.CW // 3, ROWS * capture.CH // 3
    sheet = Image.new("RGB", (12 * w, len(seeds) * h))
    for r, snaps in enumerate(runs):
        for c in range(12):
            sheet.paste(capture.draw(snaps[c * 2 * FPS]).resize((w, h)), (c * w, r * h))
    sheet.save(out)
    print(out, "rows: seeds", seeds)


def main(args):
    if args[:1] == ["survey"]:
        survey([int(s) for s in args[1:]] or [SEED], os.environ.get("LAVATUI_SURVEY_OUT", "/tmp/lavatui-survey.png"))
        return
    takes = record(list(TAKES))
    out = os.environ.get("LAVATUI_TRAILER_OUT", os.path.join(tempfile.gettempdir(), "lavatui-trailer-A.gif"))
    encode(film(takes), out)
    end = max(e for _, _, e in CUT) + DISSOLVE
    print(out, os.path.getsize(out), f"{end / FPS:.1f} s")
    if args[:1] == ["check"]:
        check(takes, os.environ.get("LAVATUI_TRAILER_CHECK", os.path.join(tempfile.gettempdir(), "lavatui-trailer-check")))


if __name__ == "__main__":
    main(sys.argv[1:])
