"""The README trailer: docs/screenshots/demo.gif (lava-0bl).

A day in three scenes, cut together so it plays as one calm recording:
morning focus (solid, lava, the clock and focus timer beside the lamp,
the timer starting), evening music (synthwave, the music card and karaoke
lyrics beside the lamp, the cover on it coming into focus), late night
(abyss, the clock at 23:30 beside the lamp, slow wax).

Every scene state is its own take: the same seed and window, the hidden
`--frame-clock` (time moves exactly one frame a frame, so frame k is the
same wax in every take, and the clock reads each take's start time), the
`--demo` player, no status bar, the side panel the same width in every
take (so the lamp's size never changes at a cut). The film is frames of one take, then the next, cut at exact
frame numbers: at a cut, the style, colours and widgets change in one
frame while the wax carries on unbroken. The cover's focus pull is four
takes too (big, medium, small pixels, sharp). Keys a take needs before
its first frame in the film (picking the song) have faded by then.

    cargo build --release
    /tmp/v/bin/python docs/screenshots/trailer.py            # the GIF
    /tmp/v/bin/python docs/screenshots/trailer.py check      # + cut frames, proofs
    /tmp/v/bin/python docs/screenshots/trailer.py survey 7 2 3  # seeds, side by side

Frames are drawn by capture.py's renderer (pyte), each scene's frames get
their own palette (ffmpeg, no dither) and gifsicle joins them. Output:
$LAVATUI_TRAILER_OUT (default docs/screenshots/demo.gif).
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
MUSIC = dict(music="side", lyrics="side", cover="overlay")
# The next song (Blob Merge: brisk, every word timed) from 0:00, its first
# line just before the evening starts; A turns the player keys on and off.
SONG = "4.3:A,4.5:n,4.7:A"

# name: (config, keys, the clock's start)
TAKES = {
    "morning": (scene("solid", "lava", dict(clock="side", pomodoro="side")), "2.5: ", "07:30"),
    **{
        f"evening-{d}": (scene("synthwave", "synthwave", MUSIC, art=d), SONG, "19:30")
        for d in ["big-pixels", "medium-pixels", "small-pixels", "sharp"]
    },
    # Not lamp only: dropping the panel widens the lamp, and the wax then
    # re-centres in one frame (the walls ease, the view doesn't).
    "night": (scene("solid", "abyss", dict(clock="side")), "18:-", "23:30"),
}

# (take, first frame, end frame): the film.
CUT = [
    ("morning", 0, 70),
    ("evening-big-pixels", 70, 95),
    ("evening-medium-pixels", 95, 103),
    ("evening-small-pixels", 103, 111),
    ("evening-sharp", 111, 155),
    ("night", 155, 225),
]
# Which parts share a palette (a scene's colours).
SCENES = [["morning"], [n for n in TAKES if n.startswith("evening")], ["night"]]


def take(name, seed=SEED, frames=None):
    cfg, keys, clock = TAKES[name]
    end = frames or max(e for _, _, e in CUT) + DISSOLVE + 5
    shot = capture.Shot(COLS, ROWS, cfg, keys, f"--seed {seed} --fps {FPS} --demo --frame-clock {clock}", frames=end)
    snaps = []
    capture.run(shot, snaps)
    return snaps


def record(names):
    with ThreadPoolExecutor(len(names)) as ex:
        return dict(zip(names, ex.map(take, names)))


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
    parts = []
    for names in SCENES:
        parts.append([frame(takes, n, k) for n, a, b in CUT if n in names for k in range(a, b)])
    first, (name, _, end) = parts[0][0], CUT[-1]
    parts[-1] += [Image.blend(frame(takes, name, end + j), first, (j + 1) / (DISSOLVE + 1)) for j in range(DISSOLVE)]
    return parts


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
    TAKES["plain"] = (TAKES["morning"][0], "", "07:30")
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
    out = os.environ.get("LAVATUI_TRAILER_OUT", os.path.join(capture.HERE, "demo.gif"))
    encode(film(takes), out)
    end = max(e for _, _, e in CUT) + DISSOLVE
    print(out, os.path.getsize(out), f"{end / FPS:.1f} s")
    if args[:1] == ["check"]:
        check(takes, os.path.splitext(out)[0] + "-check")


if __name__ == "__main__":
    main(sys.argv[1:])
