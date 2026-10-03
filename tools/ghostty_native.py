#!/usr/bin/env python3
"""Run LavaTUI in real, native Ghostty windows (macOS) and measure both
sides of a frame: what the app sent (its --trace CSV) and what Ghostty
actually showed (a screen recording, counted for distinct frames).

ONE Ghostty window per size runs every case of that size in turn (builds
alternating, round by round): a fixed driver script (<output>/driver.sh)
reads the size's case list and runs the binary with
--config/--trace/--frames/--seed 7/--demo for each. The user's Ghostty
config, shaders included, applies unless overridden (--ghostty-arg,
--opaque, --ghostty-config). Per case, in <output>/<case>/:

  trace.csv       the app's frame trace (see docs/perf/frame-trace.md)
  size            the pty size the app really got (Ghostty clamps windows
                  to the screen it opens on: check this, not the request)
  config.toml     the scratch LavaTUI config it ran with
  screen.mov      with --record: the main display, from 15 s in, recorded
                  from inside the window, so Ghostty's Screen Recording
                  permission covers it
  shot-NN.png     with --snapshots N: the window itself (screencapture
                  -l), which works even when another app covers it
  helpers.log     the recorder's and snapshotter's own output: never on
                  LavaTUI's screen (a "Failed to save" from screencapture
                  on a full disk once showed through the lamp)
and <output>/<label>.json with one summary per case (frame intervals,
gaps, wake lateness, CPU and wakeups/s sampled from outside, GPU busy,
presented frames).

How the window is opened matters (lava-jop): the command goes in one
dashed argument, `--initial-command=/bin/sh <driver> <cases>`. With
`-e /bin/sh <script>` macOS also hands the bare paths to Ghostty as
files to open, so Ghostty asks the user to allow running them (for every
window) and then runs the script a second time in a new tab: a second,
unfocused LavaTUI. The driver also takes a lock, so a second copy exits.

Keep the window in front while it runs: an unfocused LavaTUI drops to
10 fps (the trace's fps column shows it; the summary warns). Recording
needs ffmpeg for the summary. Large cell counts on a small screen: a
font size per size, COLSxROWS@FONT.

  python3 tools/ghostty_native.py --output /tmp/native --sizes 117x43 300x86@8 \\
      --styles solid --record
  # A/B, builds alternating, with and without music, twice:
  python3 tools/ghostty_native.py --output /tmp/ab --sizes 300x86@8 \\
      --binary main=/tmp/lavatui-main --binary new=target/release/lavatui \\
      --scenes music lamp --repeat 2 --record
  # README video takes: opaque background, no shaders, window snapshots
  python3 tools/ghostty_native.py --output /tmp/take --sizes 120x36 --opaque \\
      --ghostty-arg=--custom-shader= --snapshots 20 --frames 1800
"""
import argparse
import ctypes
import json
import math
import os
import re
import shlex
import shutil
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from trace_frames import summarize  # noqa: E402

# The setup the lag was reported with (lava-h52.1): music, cover and clock
# in the side panel, lyrics on the lamp, art detail auto. Scene "lamp":
# the clock only.
CONFIG = """\
[display]
fps = {fps}
cells = "{cells}"
[lamp]
style = "{style}"
[theme]
palette = "{palette}"
[ui]
mode = "full"
welcome = false
[dock]
clock = "side"
pomodoro = "off"
music = "{music}"
cover = "{music}"
lyrics = "{lyrics}"
[art]
detail = "auto"
"""

SCENES = {'music': ('side', 'overlay'), 'lamp': ('off', 'off')}

# Runs inside the window. Its cases come in on fd 3 (the app needs the
# window's terminal as its stdin; macOS can't watch /dev/tty with kqueue).
# Helpers share LavaTUI's terminal, so all their output goes to a log.
DRIVER = """\
#!/bin/sh
# driver.sh <cases file>: one line per case, run in order in this window:
# <case dir> <binary> <frames> <record secs|0> <snapshots> <snapshot at>
cases=$1
lock=$cases.lock
if ! mkdir "$lock" 2> /dev/null; then
  echo "another copy is running these cases; this one stops"; sleep 2; exit 0
fi
# Not hosted: a Ghostex/zmx session's variables make LavaTUI think it is.
unset ZMX_SESSION
for v in $(env | sed -n 's/^\\(GHOSTEX_[A-Z_]*\\)=.*/\\1/p'); do unset "$v"; done
while read -r dir binary frames secs shots at <&3; do
  [ -f "$dir/done" ] && continue
  stty size > "$dir/size" 2> /dev/null
  date +%s > "$dir/started"
  {
    if [ "$secs" != 0 ]; then
      (sleep 15; /usr/sbin/screencapture -x -v -V "$secs" -D1 "$dir/screen.mov") &
    fi
    if [ "$shots" != 0 ]; then
      (sleep "$at"; wid=$(cat "$cases.wid")
       i=0; while [ "$i" -lt "$shots" ]; do i=$((i + 1))
         /usr/sbin/screencapture -x -o -l"$wid" "$dir/shot-$(printf %02d "$i").png"; sleep 0.25
       done) &
    fi
  } > "$dir/helpers.log" 2>&1
  "$binary" --config "$dir/config.toml" --trace "$dir/trace.csv" --frames "$frames" --seed 7 --demo
  wait
  touch "$dir/done"
  sleep 1
done 3< "$cases"
rmdir "$lock"
"""

LIBC = ctypes.CDLL('/usr/lib/libSystem.B.dylib')


class _Timebase(ctypes.Structure):
    _fields_ = [('numer', ctypes.c_uint32), ('denom', ctypes.c_uint32)]


_TB = _Timebase()
LIBC.mach_timebase_info(ctypes.byref(_TB))
TICK_NS = _TB.numer / _TB.denom


def rusage(pid):
    """CPU seconds and timer wakeups so far (proc_pid_rusage, no sudo;
    top's %CPU reads 3-4x low on this machine)."""
    buf = (ctypes.c_uint64 * 80)()
    if LIBC.proc_pid_rusage(pid, 4, ctypes.byref(buf)) != 0:
        return None
    user, system, idle_wk, intr_wk = list(buf)[2:6]
    return (user + system) * TICK_NS / 1e9, idle_wk + intr_wk


def gpu_utilization():
    out = subprocess.run(['ioreg', '-r', '-d', '1', '-c', 'IOAccelerator'],
                         capture_output=True, text=True).stdout
    found = re.search(r'"Device Utilization %"=(\d+)', out)
    return int(found.group(1)) if found else None


def presented(mov, crop):
    """Distinct frames on screen in the recording's cropped region:
    ScreenCaptureKit only emits a frame when the screen changes, and
    mpdecimate drops near-identical ones (shader noise)."""
    x, y, w, h = crop
    region = f'crop=iw*{w}:ih*{h}:iw*{x}:ih*{y},scale=iw/4:-1'

    def times(extra):
        err = subprocess.run(['ffmpeg', '-v', 'info', '-i', str(mov), '-vf', region + extra + ',showinfo',
                              '-fps_mode', 'passthrough', '-f', 'null', '-'],
                             capture_output=True, text=True).stderr
        return [float(t) for t in re.findall(r'pts_time:([\d.]+)', err)]

    frames = times('')
    distinct = times(',mpdecimate=hi=128:lo=64:frac=0.001')
    if len(distinct) < 2:
        return None
    span = frames[-1] - frames[0]
    gaps = sorted((b - a) * 1000 for a, b in zip(distinct, distinct[1:]))
    return dict(captured_fps=len(frames) / span, distinct_fps=len(distinct) / span, seconds=span,
                hold_p50_ms=gaps[len(gaps) // 2], hold_p90_ms=gaps[math.ceil(.9 * len(gaps)) - 1],
                hold_max_ms=gaps[-1], holds_over_2_frames=sum(g > 34 for g in gaps))


def parse_size(text, font_size):
    found = re.fullmatch(r'(\d+)x(\d+)(?:@([\d.]+))?', text)
    if not found:
        raise argparse.ArgumentTypeError(f'size {text!r}: COLSxROWS or COLSxROWS@FONT')
    cols, rows, font = found.groups()
    return int(cols), int(rows), float(font) if font else font_size


def plan(args):
    """The cases, per size: each round runs every style and scene with the
    builds alternating (their order flipped every other round)."""
    sizes = {}
    for size in args.sizes:
        cases = []
        for n in range(1, args.repeat + 1):
            builds = args.binaries if n % 2 else args.binaries[::-1]
            for style in args.styles:
                for scene in args.scenes:
                    for build, _ in builds:
                        name = '-'.join(p for p in (args.label, build if len(args.binaries) > 1 else '',
                                                    style, scene, size.split('@')[0],
                                                    f'r{n}' if args.repeat > 1 else '') if p)
                        cases.append(dict(name=name, build=build, style=style, scene=scene, round=n))
        sizes[size] = cases
    return sizes


def write_size(args, size, cases, driver):
    """The size's case dirs, configs and case list; returns the list."""
    listed = args.output / f"{args.label}-{size.split('@')[0]}.cases"
    lines = []
    for case in cases:
        out = args.output / case['name']
        if out.exists():
            shutil.rmtree(out)
        out.mkdir(parents=True)
        music, lyrics = SCENES[case['scene']]
        # --opaque: LavaTUI reads Ghostty's opacity from its config files,
        # not the window's flags, so it's told here.
        cells = 'opaque' if args.opaque else 'auto'
        (out / 'config.toml').write_text(CONFIG.format(fps=args.fps, cells=cells, style=case['style'],
                                                       palette=args.palette, music=music, lyrics=lyrics))
        binary = args.output / f"lavatui-{case['build']}"
        lines.append(' '.join([str(out), str(binary), str(args.frames),
                               str(args.record_secs if args.record else 0),
                               str(args.snapshots), str(args.snapshot_at)]))
    listed.write_text('\n'.join(lines) + '\n')
    lock = Path(str(listed) + '.lock')
    if lock.exists():
        lock.rmdir()
    Path(str(listed) + '.wid').unlink(missing_ok=True)
    return listed


def open_window(args, size, listed, driver):
    cols, rows, font = parse_size(size, args.font_size)
    title = f'lavatui-native-{listed.stem}'
    extra = list(args.ghostty_arg)
    if args.opaque:
        extra += ['--background-opacity=1']
    if args.ghostty_config:
        extra += ['--config-default-files=false', f'--config-file={args.ghostty_config}']
    # One dashed argument: no bare path for macOS to "open" (see the module
    # docs), so no prompt and no second copy in a new tab.
    subprocess.run(['open', '-na', 'Ghostty', '--args', f'--title={title}',
                    f'--window-width={cols}', f'--window-height={rows}',
                    '--quit-after-last-window-closed=true',
                    *([f'--font-size={font}'] if font else []), *extra,
                    f'--initial-command=/bin/sh {driver} {listed}'], check=True)
    return title


# The window's id for `screencapture -l`, by its Ghostty process (window
# names need Screen Recording permission; ids and owners don't).
WINDOW_ID = """\
import CoreGraphics
let pid = Int(CommandLine.arguments[1])!
let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as! [[String: Any]]
var best = (0, 0.0)
for w in list where (w[kCGWindowOwnerPID as String] as? Int) == pid && (w[kCGWindowLayer as String] as? Int) == 0 {
    let b = w[kCGWindowBounds as String] as! [String: Any]
    let area = (b["Width"] as! Double) * (b["Height"] as! Double)
    if area > best.1 { best = (w[kCGWindowNumber as String] as! Int, area) }
}
print(best.0)
"""


def window_id(args, title, listed):
    """Write the window's id next to its case list for the snapshots."""
    script = args.output / 'window_id.swift'
    script.write_text(WINDOW_ID)
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        out = subprocess.run(['pgrep', '-f', f'MacOS/ghostty --title={title} '], capture_output=True, text=True).stdout
        if out.split():
            wid = subprocess.run(['swift', str(script), out.split()[0]], capture_output=True, text=True).stdout.strip()
            if wid not in ('', '0'):
                Path(str(listed) + '.wid').write_text(wid + '\n')
                return
        time.sleep(1)
    print(f'warning: {title}: no window id, so no snapshots', file=sys.stderr, flush=True)


def app_pid(case_dir):
    out = subprocess.run(['pgrep', '-f', f'{case_dir}/config.toml'], capture_output=True, text=True).stdout
    pids = [int(p) for p in out.split()]
    return pids[0] if pids else None


def watch_case(args, case, title):
    """Wait for one case in the open window, sampling it from outside."""
    out = args.output / case['name']
    started, done = out / 'started', out / 'done'
    deadline = time.monotonic() + 120  # the window opening, the case before
    while not started.exists():
        if time.monotonic() > deadline:
            raise RuntimeError(f"{case['name']}: never started (window closed?)")
        time.sleep(0.5)
    load_start = os.getloadavg()
    t0, gpu, sample = time.monotonic(), [], None
    first = None
    deadline = t0 + args.frames / 10 + 60  # even at the unfocused 10 fps
    while not done.exists():
        if time.monotonic() > deadline:
            raise RuntimeError(f"{case['name']}: not done after {deadline - t0:.0f} s")
        gpu.append(gpu_utilization())
        elapsed = time.monotonic() - t0
        # CPU and wakeups over the middle of the run (from 20 s, 12 s long,
        # or the middle third of a shorter one).
        span = (20, 32) if args.frames >= 2400 else (args.frames / 180, args.frames / 90)
        if first is None and elapsed >= span[0]:
            pid = app_pid(out)
            first = (pid, time.monotonic(), rusage(pid)) if pid else False
        elif first and sample is None and elapsed >= span[1]:
            pid, at, before = first
            now = rusage(pid)
            if before and now:
                dt = time.monotonic() - at
                sample = dict(cpu_percent=100 * (now[0] - before[0]) / dt, wakeups_per_s=(now[1] - before[1]) / dt)
        time.sleep(1)
    result = summarize(out / 'trace.csv')
    result['gaps_over_2_periods'] = len(result.pop('spikes'))
    gpu = [g for g in gpu if g is not None]
    size = out / 'size'
    result.update(case=case['name'], build=case['build'], style=case['style'], scene=case['scene'],
                  round=case['round'], pty=size.read_text().strip() if size.exists() else None,
                  load_start=load_start, load_end=os.getloadavg(),
                  gpu_mean=sum(gpu) / len(gpu) if gpu else None, **(sample or {}))
    mov = out / 'screen.mov'
    if args.record:
        result['presented'] = presented(mov, args.crop) if mov.exists() else None
        if mov.exists() and not args.keep_recordings:
            mov.unlink()
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--binary', action='append', default=[], metavar='[NAME=]PATH',
                        help='build to run (repeat for an A/B: cases alternate builds); default target/release/lavatui')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--label', default='run', help='prefix for this series')
    parser.add_argument('--sizes', nargs='+', default=['200x60', '300x80'],
                        help='requested COLSxROWS, or COLSxROWS@FONT; one window each')
    parser.add_argument('--styles', nargs='+', default=['synthwave', 'solid'])
    parser.add_argument('--scenes', nargs='+', default=['music'], choices=sorted(SCENES),
                        help='music: music, cover, clock beside the lamp, lyrics on it; lamp: the clock only')
    parser.add_argument('--palette', default='synthwave')
    parser.add_argument('--fps', type=int, default=60)
    parser.add_argument('--frames', type=int, default=3600)
    parser.add_argument('--repeat', type=int, default=1, help='rounds (build order flips every other round)')
    parser.add_argument('--font-size', type=float, help='Ghostty font size for sizes without @FONT '
                        '(8 fits ~300x86 on a 1512x982 screen)')
    parser.add_argument('--record', action='store_true', help='record the screen and count presented frames')
    parser.add_argument('--record-secs', type=int, default=8)
    parser.add_argument('--keep-recordings', action='store_true')
    parser.add_argument('--crop', type=float, nargs=4, default=[0, .1, .6, .8], metavar=('X', 'Y', 'W', 'H'),
                        help='region of the screen to compare, as fractions (default: the lamp of a big window)')
    parser.add_argument('--snapshots', type=int, default=0, help='window snapshots per case (0.25 s apart)')
    parser.add_argument('--snapshot-at', type=float, default=12, help='seconds into a case for the first one')
    parser.add_argument('--ghostty-arg', action='append', default=[], help='extra Ghostty flag, e.g. --ghostty-arg=--custom-shader=')
    parser.add_argument('--opaque', action='store_true',
                        help='opaque window background and display.cells = "opaque" (README video takes)')
    parser.add_argument('--ghostty-config', type=Path,
                        help='use only this Ghostty config file (a recording-only config), not the user\'s')
    parser.add_argument('--dry-run', action='store_true', help='write the driver and case lists, open nothing')
    args = parser.parse_args()
    args.output = args.output.resolve()
    if any(c.isspace() for c in str(args.output)):
        parser.error('--output must not contain spaces (Ghostty splits its command on them)')
    for size in args.sizes:
        try:
            parse_size(size, args.font_size)
        except argparse.ArgumentTypeError as err:
            parser.error(str(err))
    args.output.mkdir(parents=True, exist_ok=True)
    binaries = []
    for spec in args.binary or ['app=target/release/lavatui']:
        name, _, path = spec.rpartition('=')
        binaries.append((name or 'app', Path(path)))
    if len({n for n, _ in binaries}) != len(binaries):
        parser.error('--binary names must differ (NAME=PATH)')
    # Run copies from --output: a fresh Ghostty process may need macOS's
    # permission to read ~/Documents (where a checkout often lives), and
    # its prompt would hold the run.
    for name, path in binaries:
        shutil.copy2(path, args.output / f'lavatui-{name}')
    args.binaries = binaries
    driver = args.output / 'driver.sh'
    driver.write_text(DRIVER)
    driver.chmod(0o755)
    results = []
    for size, cases in plan(args).items():
        listed = write_size(args, size, cases, driver)
        if args.dry_run:
            print(f'{size}: {len(cases)} cases in {listed}; window command: /bin/sh {driver} {listed}')
            continue
        title = open_window(args, size, listed, driver)
        try:
            if args.snapshots:
                window_id(args, title, listed)
            for case in cases:
                result = watch_case(args, case, title)
                slow = sum(n for fps, n in result['fps_counts'].items() if int(fps) < args.fps)
                if slow:
                    print(f"warning: {case['name']}: {slow} frames below {args.fps} fps (window not in front?)",
                          file=sys.stderr, flush=True)
                results.append(result)
                print(json.dumps(result), flush=True)
        finally:
            # The Ghostty process can outlive its window: end it with the size.
            subprocess.run(['pkill', '-f', f'MacOS/ghostty --title={title} '])
    if not args.dry_run:
        (args.output / f'{args.label}.json').write_text(json.dumps(results, indent=2) + '\n')


if __name__ == '__main__':
    main()
