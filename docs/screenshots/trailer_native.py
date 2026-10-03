"""The README trailer filmed in native Ghostty (lava-0bl, takes B and C).

The same film as trailer.py (take A): the same takes, seed, window size
in cells, frame clock and keys, cut at the same frames. Each take runs in
a real Ghostty window (tools/ghostty_native.py's driver: one window for
all takes, recorded from inside it) with a recording-only Ghostty config,
never the user's: opaque, no title bar or padding, the user's font, and
for C the user's custom shaders.

The app runs at 60 fps on `--frame-clock`, so take A's frame n is frame
6n here (sim time n/10 either way). Between Ghostty and the app sits a
small pass-through (PASSTHRU): it hands the app's output to Ghostty and
Ghostty's input (focus reports, replies) to the app unchanged, sends the
take's keys right after the frame they belong to (A's `f<n>` → after
frame 6n+5, so the app handles it before frame 6(n+1), as in A), and logs
when each frame was handed over. The screen recording is matched to that
log by the first frame drawn on the blank window, and each film frame is
taken from the recording at its moment. Out: <label>.mp4 (60 fps) and
<label>.gif (10 fps, take A's GIF settings), and the frames either side
of every cut.

    cargo build --release
    /tmp/v/bin/python docs/screenshots/trailer_native.py record B --output /tmp/native-b
    /tmp/v/bin/python docs/screenshots/trailer_native.py record C --output /tmp/native-c
    /tmp/v/bin/python docs/screenshots/trailer_native.py film /tmp/native-b B
    /tmp/v/bin/python docs/screenshots/trailer_native.py small /tmp/native-b B --width 660

`record` opens a Ghostty window and needs it in front, hands off, for
about five minutes (`--takes` and `--secs` for a short pilot). The full
display is recorded (Ghostty's permission covers it), then cropped to
the window: the raw recordings stay in --output and are never shipped.
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path
from types import SimpleNamespace

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(ROOT / "tools"))
import capture  # noqa: E402
import trailer  # noqa: E402
import ghostty_native as gn  # noqa: E402

RATE = 60  # app frames a second; A's film runs at trailer.FPS
STEP = RATE // trailer.FPS
DELAY = 3.0  # seconds of blank window before the app starts (the recorder starts)
LEAD = (STEP / 2 - 0.5) / RATE  # see picks()

# Recording-only Ghostty configs. Font as the user's; no title bar, no
# padding, opaque. C adds the user's shaders (read from their config).
GHOSTTY = """\
font-family = "MesloLGS NF"
font-size = 16
font-thicken = true
font-thicken-strength = 1
background = #0f0b0a
foreground = #e9dccf
background-opacity = 1
macos-titlebar-style = hidden
window-padding-x = 0
window-padding-y = 0
window-padding-color = extend
window-save-state = never
confirm-close-surface = false
cursor-style-blink = false
"""
USER_GHOSTTY = Path.home() / "Library/Application Support/com.mitchellh.ghostty/config"

PASSTHRU = r'''
import os, pty, select, signal, struct, sys, termios, time, tty, fcntl
args = sys.argv[1:]
keys_arg, log_path, delay = args[0], args[1], float(args[2])
cmd = args[args.index("--") + 1:]
keys = []
for k in filter(None, keys_arg.split(",")):
    f, s = k.split(":", 1)
    keys.append((int(f), s.encode().decode("unicode_escape").encode()))
keys.sort(key=lambda k: k[0])
END = b"\x1b[?2026l"
size = fcntl.ioctl(0, termios.TIOCGWINSZ, b"\0" * 8)
old = termios.tcgetattr(0)
tty.setraw(0)
time.sleep(delay)
pid, fd = pty.fork()
if pid == 0:
    os.execvp(cmd[0], cmd)
fcntl.ioctl(fd, termios.TIOCSWINSZ, size)
log = open(log_path, "w")
frames, pending, hold = 0, b"", len(END) - 1
try:
    while True:
        r, _, _ = select.select([0, fd], [], [])
        if 0 in r:
            data = os.read(0, 65536)
            if data:
                os.write(fd, data)
        if fd in r:
            try:
                data = os.read(fd, 65536)
            except OSError:
                break
            if not data:
                break
            buf, pos = pending + data, 0
            while (i := buf.find(END, pos)) >= 0:
                os.write(1, buf[pos:i + len(END)])
                pos = i + len(END)
                log.write(f"{frames} {time.monotonic():.6f}\n")
                frames += 1
                while keys and keys[0][0] < frames:
                    os.write(fd, keys.pop(0)[1])
            rest = buf[pos:]
            # Hold back what could be the start of a frame end.
            if len(rest) > hold:
                os.write(1, rest[:-hold])
                rest = rest[-hold:]
            pending = rest
    os.write(1, pending)
finally:
    log.close()
    termios.tcsetattr(0, termios.TCSADRAIN, old)
_, status = os.waitpid(pid, 0)
sys.exit(os.waitstatus_to_exitcode(status))
'''

# Run by the driver inside the window before the first take (as its
# "place" helper, with Ghostty's Accessibility permission): bring this
# Ghostty process's window to the front and focus it (a window opened from
# another app stays behind it, and an unfocused LavaTUI drops to 10 fps),
# then print its frame and the main screen's width, in points.
FOCUS = """\
import AppKit
import ApplicationServices
let pid = pid_t(CommandLine.arguments[1])!
let app = AXUIElementCreateApplication(pid)
func windows() -> [AXUIElement] {
    var value: CFTypeRef?
    AXUIElementCopyAttributeValue(app, kAXWindowsAttribute as CFString, &value)
    return (value as? [AXUIElement]) ?? []
}
var found = windows()
for _ in 0..<50 where found.isEmpty { usleep(100_000); found = windows() }
guard let window = found.first else { print("no window"); exit(1) }
print("trusted:", AXIsProcessTrusted())
NSRunningApplication(processIdentifier: pid)?.activate(options: [.activateAllWindows])
AXUIElementSetAttributeValue(app, kAXFrontmostAttribute as CFString, kCFBooleanTrue)
AXUIElementPerformAction(window, kAXRaiseAction as CFString)
AXUIElementSetAttributeValue(window, kAXMainAttribute as CFString, kCFBooleanTrue)
AXUIElementSetAttributeValue(window, kAXFocusedAttribute as CFString, kCFBooleanTrue)
usleep(800_000)
print("front:", NSWorkspace.shared.frontmostApplication?.processIdentifier == pid)
var p: CFTypeRef?, s: CFTypeRef?
var at = CGPoint.zero, size = CGSize.zero
AXUIElementCopyAttributeValue(window, kAXPositionAttribute as CFString, &p)
AXUIElementCopyAttributeValue(window, kAXSizeAttribute as CFString, &s)
if let p { AXValueGetValue(p as! AXValue, .cgPoint, &at) }
if let s { AXValueGetValue(s as! AXValue, .cgSize, &size) }
print("frame:", at.x, at.y, size.width, size.height, CGDisplayBounds(CGMainDisplayID()).width)
"""

BOUNDS = """\
import CoreGraphics
let pid = Int(CommandLine.arguments[1])!
let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as! [[String: Any]]
var best: [String: Any]? = nil
var area = 0.0
for w in list where (w[kCGWindowOwnerPID as String] as? Int) == pid && (w[kCGWindowLayer as String] as? Int) == 0 {
    let b = w[kCGWindowBounds as String] as! [String: Any]
    let a = (b["Width"] as! Double) * (b["Height"] as! Double)
    if a > area { area = a; best = b }
}
let screen = CGDisplayBounds(CGMainDisplayID())
if let b = best { print(b["X"]!, b["Y"]!, b["Width"]!, b["Height"]!, screen.width) }
"""


def native_keys(keys):
    """A's frame-timed keys (`f<n>:x`) for 60 fps: after frame 6n+5."""
    out = []
    for k in filter(None, keys.split(",")):
        f, s = k.split(":", 1)
        out.append(f"{STEP * int(f[1:]) + STEP - 1}:{s}")
    return ",".join(out)


def needed(name):
    """A's last film frame from take `name`, + the closing dissolve."""
    last = max(b for n, _, b in trailer.CUT if n == name)
    if name == trailer.CUT[-1][0]:
        last += trailer.DISSOLVE
    return last


def ghostty_config(variant, out):
    text = GHOSTTY
    if variant == "C":
        lines = USER_GHOSTTY.read_text().splitlines()
        shaders = [ln.strip() for ln in lines if re.match(r"\s*custom-shader(-animation)?\s*=", ln)]
        text += "\n".join(shaders) + "\n"
    path = out / f"ghostty-{variant}.conf"
    path.write_text(text)
    return path


def record(variant, out, takes, secs):
    out.mkdir(parents=True, exist_ok=True)
    binary = out / "lavatui-bin"
    shutil.copy2(ROOT / "target/release/lavatui", binary)
    (out / "passthru.py").write_text(PASSTHRU)
    driver = out / "driver.sh"
    driver.write_text(gn.DRIVER)
    driver.chmod(0o755)
    lines = []
    for name in takes:
        cfg, keys, clock, *welcome = trailer.TAKES[name]
        case = out / name
        if case.exists():
            shutil.rmtree(case)
        case.mkdir()
        toml = capture.with_welcome(cfg + ';[display];cells="opaque"', bool(welcome and welcome[0]))
        (case / "config.toml").write_text(toml.replace(";", "\n"))
        frames = min(STEP * needed(name) + 2 * STEP, int(secs * RATE)) if secs else STEP * needed(name) + 2 * STEP
        wrapper = case / "run.sh"
        # The driver calls: <binary> --config C --trace T --frames N --seed S --demo
        wrapper.write_text(
            "#!/bin/sh\n"
            f"exec /usr/bin/python3 {out}/passthru.py '{native_keys(keys)}' {case}/frames.log {DELAY} "
            f"-- {binary} \"$@\" --fps {RATE} --frame-clock {clock}\n")
        wrapper.chmod(0o755)
        rec = int(DELAY + frames / RATE + 4)
        # dir binary frames record-secs shots at seed display record-at gap
        lines.append(f"{case} {wrapper} {frames} {rec} 0 0 {trailer.SEED} 1 0 0.25")
    listed = out / f"{variant}.cases"
    listed.write_text("\n".join(lines) + "\n")
    for p in (Path(f"{listed}.lock"),):
        if p.exists():
            p.rmdir()
    (out / "place.swift").write_text(FOCUS)
    subprocess.run(["swiftc", "-O", "-o", str(out / "place"), str(out / "place.swift")], check=True)
    Path(f"{listed}.place").write_text("1\n")
    conf = ghostty_config(variant, out)
    args = SimpleNamespace(font_size=None, ghostty_arg=[], opaque=True, ghostty_config=conf,
                           output=out, frames=max(int(secs * RATE) if secs else 0, 6000),
                           record=False, crop=[0, 0, 1, 1], keep_recordings=True)
    title = gn.open_window(args, f"{trailer.COLS}x{trailer.ROWS}", listed, driver)
    print(f"{variant}: window {title} open", flush=True)
    try:
        swift = out / "bounds.swift"
        swift.write_text(BOUNDS)
        bounds = None
        for _ in range(40):
            pids = subprocess.run(["pgrep", "-f", f"MacOS/ghostty --title={title} "],
                                  capture_output=True, text=True).stdout.split()
            if pids:
                got = subprocess.run(["swift", str(swift), pids[0]], capture_output=True, text=True).stdout.split()
                if len(got) == 5:
                    bounds = [float(x) for x in got]
                    break
            time.sleep(1)
        results = []
        for name in takes:
            case = dict(name=name, build="app", style="-", scene="-", round=1)
            result = gn.watch_case(args, case, title)
            slow = sum(n for fps, n in result["fps_counts"].items() if int(fps) < RATE)
            result["slow_frames"] = slow
            print(f"{variant} {name}: pty {result['pty']}, frames below {RATE} fps: {slow}", flush=True)
            results.append(result)
        (out / "results.json").write_text(json.dumps(results, indent=2))
        log = Path(f"{listed}.place.log").read_text() if Path(f"{listed}.place.log").exists() else ""
        print(f"{variant} focus helper: {' | '.join(log.split(chr(10)))}", flush=True)
        frame = re.search(r"frame: (\S+) (\S+) (\S+) (\S+) (\S+)", log)
        if frame:
            bounds = [float(v) for v in frame.groups()]
        (out / "bounds.json").write_text(json.dumps(bounds))
    finally:
        subprocess.run(["pkill", "-f", f"MacOS/ghostty --title={title} "])


# --- the film ------------------------------------------------------------

def pts_list(mov):
    """The time of every frame the recorder emitted."""
    err = subprocess.run(["ffmpeg", "-v", "info", "-i", str(mov), "-vf", "scale=32:-2,showinfo",
                          "-fps_mode", "passthrough", "-f", "null", "-"], capture_output=True, text=True).stderr
    return [float(t) for t in re.findall(r"pts_time:([\d.]+)", err)]


def video_width(mov):
    out = subprocess.run(["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries",
                          "stream=width", "-of", "csv=p=0", str(mov)], capture_output=True, text=True).stdout
    return int(out.strip())


# The window's edge: macOS draws a thin border round it (cropped off) and
# rounds its corners, where whatever is behind the window shows through.
EDGE = 4  # recorded px
CORNER = 16  # output px: filled from inside, row by row


def crop_box(out, mov):
    """The window's content in the recording's pixels, its border cut off."""
    x, y, w, h, screen = json.loads((out / "bounds.json").read_text())
    s = video_width(mov) / screen
    x, y, w, h = (round(v * s) for v in (x, y, w, h))
    return [x + EDGE, y + EDGE, w - 2 * EDGE, h - 2 * EDGE]


def fill_corners(im, r=CORNER):
    """Paint over the rounded corners: each row's pixels outside the arc
    take the colour of its first pixel inside (so bands like the wax pool
    carry on), and nothing behind the window shows."""
    w, h = im.size
    px = im.load()
    for row in range(r):
        dy = r - row - 0.5
        inset = int(r - (r * r - dy * dy) ** 0.5 + 0.999) + 2
        for y, xs in ((row, None), (h - 1 - row, None)):
            left, right = px[inset, y], px[w - 1 - inset, y]
            for x in range(inset):
                px[x, y] = left
                px[w - 1 - x, y] = right
    return im


def decode(mov, box, width):
    """Every recorded frame, cropped to `box` and scaled to `width` px:
    (raw RGB, size), streamed."""
    x, y, w, h = box
    size = (width, round(h * width / w / 2) * 2)
    proc = subprocess.Popen(["ffmpeg", "-v", "error", "-i", str(mov), "-vf",
                             f"crop={w}:{h}:{x}:{y},scale={size[0]}:{size[1]}:flags=lanczos",
                             "-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"],
                            stdout=subprocess.PIPE)
    n = size[0] * size[1] * 3
    while len(raw := proc.stdout.read(n)) == n:
        yield raw, size
    proc.wait()


def start_frame(mov, box):
    """The first recorded frame that isn't the blank window before the app."""
    from PIL import Image, ImageChops, ImageStat
    blank = None
    for n, (raw, size) in enumerate(decode(mov, box, 120)):
        im = Image.frombytes("RGB", size, raw).convert("L")
        if blank is None:
            blank = im
        elif ImageStat.Stat(ImageChops.difference(im, blank)).mean[0] > 4:
            return n
    raise SystemExit(f"{mov}: never saw the app start")


def picks(case, box, wanted):
    """For each app frame in `wanted` (in order), the recorded frame that
    shows it: the last one at or before LEAD after it was handed over (on
    the recording's clock, set by the first frame). A film frame spans STEP
    app frames, and a key's effect starts the next one, so LEAD sits in
    the middle of that window: display latency that wanders (shaders add
    some) can't pull a film frame onto its neighbour's moment."""
    mov = case / "screen.mov"
    pts = pts_list(mov)
    times = dict((int(k), float(t)) for k, t in (ln.split() for ln in (case / "frames.log").read_text().splitlines()))
    first = start_frame(mov, box)
    out = []
    n = first
    for k in wanted:
        target = pts[first] + (times[k] - times[0]) + LEAD
        while n + 1 < len(pts) and pts[n + 1] <= target:
            n += 1
        out.append(n)
    return out, dict(start_frame=first, start_at=pts[first], recorded=len(pts),
                     last_frame=max(times), last_pick=out[-1])


def film(out, label, width=900):
    from PIL import Image
    box = crop_box(out, out / trailer.CUT[0][0] / "screen.mov")
    frames_dir = out / f"{label}-frames"
    frames_dir.mkdir(exist_ok=True)
    total = STEP * (trailer.CUT[-1][2] + trailer.DISSOLVE)
    d = STEP * trailer.DISSOLVE
    enc, first, index, info, size = None, None, 0, {}, None
    for i, (name, a, b) in enumerate(trailer.CUT):
        end = b + (trailer.DISSOLVE if i == len(trailer.CUT) - 1 else 0)
        wanted = list(range(STEP * a, STEP * end))
        chosen, info[name] = picks(out / name, box, wanted)
        need = iter(chosen)
        want_n = next(need, None)
        for n, (raw, size) in enumerate(decode(out / name / "screen.mov", box, width)):
            while want_n == n:
                im = fill_corners(Image.frombytes("RGB", size, raw))
                if first is None:
                    first = im
                    enc = subprocess.Popen(
                        ["ffmpeg", "-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s",
                         f"{size[0]}x{size[1]}", "-r", str(RATE), "-i", "-", "-c:v", "libx264", "-crf", "16",
                         "-preset", "slow", "-pix_fmt", "yuv420p", str(out / f"{label}.mp4")],
                        stdin=subprocess.PIPE)
                if index >= total - d:
                    im = Image.blend(im, first, (index - (total - d) + 1) / (d + 1))
                enc.stdin.write(im.tobytes())
                if index % STEP == 0:
                    im.save(frames_dir / f"f{index // STEP:04d}.png")
                index += 1
                want_n = next(need, None)
            if want_n is None:
                break
    enc.stdin.close()
    enc.wait()
    ten = [Image.open(frames_dir / f"f{k:04d}.png").convert("RGB") for k in range(index // STEP)]
    gif = out / f"{label}.gif"
    saved, trailer.SCALE = trailer.SCALE, 1.0
    trailer.encode([ten[a:b] for a, b in trailer.PALETTES], str(gif))
    trailer.SCALE = saved
    check = out / f"{label}-check"
    check.mkdir(exist_ok=True)
    for (_, _, k), (bname, _, _) in zip(trailer.CUT, trailer.CUT[1:]):
        left, right = ten[k - 1], ten[k]
        pair = Image.new("RGB", (left.width * 2 + 8, left.height))
        pair.paste(left, (0, 0))
        pair.paste(right, (left.width + 8, 0))
        pair.save(check / f"cut-{k:03d}-to-{bname}.png")
    (out / f"{label}-film.json").write_text(json.dumps(info, indent=2))
    mp4 = out / f"{label}.mp4"
    print(f"{gif} {gif.stat().st_size} B, {mp4} {mp4.stat().st_size} B, {index / RATE:.1f} s, "
          f"{size[0]}x{size[1]}", flush=True)


def small_gif(frames_dir, out, width, colours=64, threshold=8):
    """A smaller GIF of the 10 fps film frames, to fit a size budget. A
    screen recording carries faint noise and native text and shaders
    compress far worse than take A's flat cells, so: scaled to `width`,
    `colours` per part, and a block noise gate (an 8x8 block keeps its last
    picture unless a pixel in it moved by more than `threshold`: whole
    glyphs refresh together, so nothing is left behind)."""
    import numpy as np
    from PIL import Image
    files = sorted(p for p in Path(frames_dir).iterdir() if p.suffix == ".png")
    starts = {a for a, _ in trailer.PALETTES}
    prev, frames, B = None, [], 8
    for i, f in enumerate(files):
        im = Image.open(f).convert("RGB")
        im = im.resize((width, round(im.height * width / im.width)), Image.LANCZOS)
        a = np.asarray(im).astype(np.int16)
        if prev is not None and i not in starts:
            moved = np.abs(a - prev).max(axis=2) > threshold
            h, w = moved.shape
            pad = np.zeros((-(-h // B) * B, -(-w // B) * B), bool)
            pad[:h, :w] = moved
            blocks = pad.reshape(pad.shape[0] // B, B, pad.shape[1] // B, B).any(axis=(1, 3))
            mask = np.repeat(np.repeat(blocks, B, 0), B, 1)[:h, :w, None]
            a = np.where(mask, a, prev)
        prev = a
        frames.append(Image.fromarray(a.astype(np.uint8)))
    saved = trailer.SCALE, trailer.COLOURS
    trailer.SCALE, trailer.COLOURS = 1.0, colours
    trailer.encode([frames[a:b] for a, b in trailer.PALETTES], str(out))
    trailer.SCALE, trailer.COLOURS = saved
    print(out, Path(out).stat().st_size, flush=True)


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("record")
    r.add_argument("variant", choices=["B", "C"])
    r.add_argument("--output", type=Path, required=True)
    r.add_argument("--takes", nargs="+", default=[n for n in trailer.TAKES if n != "plain"])
    r.add_argument("--secs", type=float, default=0, help="cap each take (a pilot)")
    f = sub.add_parser("film")
    f.add_argument("output", type=Path)
    f.add_argument("label")
    sm = sub.add_parser("small", help="a smaller GIF of a film's frames (B: 660, C: 560 fit 5 MB)")
    sm.add_argument("output", type=Path)
    sm.add_argument("label")
    sm.add_argument("--width", type=int, required=True)
    sm.add_argument("--colours", type=int, default=64)
    a = p.parse_args()
    if a.cmd == "record":
        if any(c.isspace() for c in str(a.output.resolve())):
            p.error("--output must not contain spaces")
        record(a.variant, a.output.resolve(), a.takes, a.secs)
    elif a.cmd == "small":
        out = a.output.resolve()
        small_gif(out / f"{a.label}-frames", out / f"{a.label}-{a.width}.gif", a.width, a.colours)
    else:
        film(a.output.resolve(), a.label)


if __name__ == "__main__":
    main()
