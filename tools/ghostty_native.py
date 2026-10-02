#!/usr/bin/env python3
"""Run LavaTUI in real, native Ghostty windows (macOS) and measure both
sides of a frame: what the app sent (its --trace CSV) and what Ghostty
actually showed (a screen recording, counted for distinct frames).

Each case opens its own Ghostty window (`open -na Ghostty --args ...`, so
the user's Ghostty config, shaders included, applies unless overridden
with --ghostty-arg), runs the binary with --trace/--frames/--seed 7/--demo
and a scratch config, and records:

  <case>.csv    the app's frame trace (see docs/perf/frame-trace.md)
  <case>.size   the pty size the app really got (Ghostty clamps windows
                to the screen it opens on: check this, not the request)
  <case>.mov    with --record: the main display, from 15 s in, recorded
                from inside the window, so Ghostty's Screen Recording
                permission covers it
  <case>.gpu    GPU "Device Utilization %" once a second (ioreg, no sudo)
  <case>.load   load averages at start and end

Keep the window in front while it runs: an unfocused LavaTUI drops to
10 fps (the trace's fps column shows it). Recording needs ffmpeg for the
summary. Large cell counts on a small screen: --font-size 8.

  python3 tools/ghostty_native.py --output /tmp/native --sizes 300x80 \\
      --styles synthwave solid --record
  python3 tools/ghostty_native.py --output /tmp/native --sizes 400x120 \\
      --label noshader --record --ghostty-arg=--custom-shader=
"""
import argparse
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
# in the side panel, lyrics on the lamp, art detail auto.
CONFIG = """\
[display]
fps = {fps}
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
music = "side"
cover = "side"
lyrics = "overlay"
[art]
detail = "auto"
"""

WRAPPER = """\
#!/bin/sh
# <sizefile> <movfile|-> <record secs> <binary> args...
size=$1 mov=$2 secs=$3; shift 3
sleep 0.5; stty size > "$size"
if [ "$mov" != - ]; then (sleep 15; /usr/sbin/screencapture -x -v -V "$secs" -D1 "$mov") & fi
"$@"
wait
"""


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


def run_case(args, name, cols, rows, style, wrapper):
    out = args.output
    cfg = out / f'{name}.toml'
    cfg.write_text(CONFIG.format(fps=args.fps, style=style, palette=args.palette))
    trace, size, mov = (out / f'{name}{ext}' for ext in ('.csv', '.size', '.mov'))
    for path in (trace, size, mov):
        path.unlink(missing_ok=True)
    load_start = os.getloadavg()
    command = [str(wrapper), str(size), str(mov) if args.record else '-', str(args.record_secs),
               str(args.binary.resolve()), '--config', str(cfg), '--trace', str(trace),
               '--frames', str(args.frames), '--seed', '7', '--demo']
    # Ghostty splits `-e`'s arguments on spaces again: hand it one script
    # (in --output, which must not contain spaces) holding the quoted
    # command, via /bin/sh: a lone script path gets an "allow?" prompt.
    launch = out / f'{name}.sh'
    launch.write_text('#!/bin/sh\nexec ' + shlex.join(command) + '\n')
    launch.chmod(0o755)
    subprocess.run(['open', '-na', 'Ghostty', '--args', f'--title=lavatui-native-{name}',
                    f'--window-width={cols}', f'--window-height={rows}',
                    '--quit-after-last-window-closed=true',
                    *([f'--font-size={args.font_size}'] if args.font_size else []),
                    *args.ghostty_arg, '-e', '/bin/sh', str(launch)], check=True)
    gpu = []
    deadline = time.monotonic() + args.frames / 10 + 60  # even at the unfocused 10 fps
    while not (trace.exists() and trace.stat().st_size and trace.read_text().endswith('\n')):
        if time.monotonic() > deadline:
            subprocess.run(['pkill', '-f', f'MacOS/ghostty --title=lavatui-native-{name} '])
            raise RuntimeError(f'{name}: no trace (window closed early?): {shlex.join(command)}')
        gpu.append(gpu_utilization())
        time.sleep(1)
    time.sleep(1)
    result = summarize(trace)
    result.pop('spikes')
    gpu = [g for g in gpu if g is not None]
    result.update(case=name, pty=size.read_text().strip() if size.exists() else None,
                  load_start=load_start, load_end=os.getloadavg(),
                  gpu_mean=sum(gpu) / len(gpu) if gpu else None)
    if args.record:
        # The recording starts 15 s in: a shorter run never makes one.
        until = time.monotonic() + args.record_secs + 30
        while not mov.exists() and time.monotonic() < until:
            time.sleep(1)
        time.sleep(2)
        result['presented'] = presented(mov, args.crop) if mov.exists() else None
    # Each `open -na` is its own Ghostty process, and it outlives its window.
    subprocess.run(['pkill', '-f', f'MacOS/ghostty --title=lavatui-native-{name} '])
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--binary', type=Path, default=Path('target/release/lavatui'))
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--label', default='run', help='prefix for this series')
    parser.add_argument('--sizes', nargs='+', default=['200x60', '300x80'], help='requested COLSxROWS')
    parser.add_argument('--styles', nargs='+', default=['synthwave', 'solid'])
    parser.add_argument('--palette', default='synthwave')
    parser.add_argument('--fps', type=int, default=60)
    parser.add_argument('--frames', type=int, default=3600)
    parser.add_argument('--repeat', type=int, default=1)
    parser.add_argument('--font-size', type=float, help='Ghostty font size (8 fits ~300x86 on a 1512x982 screen)')
    parser.add_argument('--record', action='store_true', help='record the screen and count presented frames')
    parser.add_argument('--record-secs', type=int, default=8)
    parser.add_argument('--crop', type=float, nargs=4, default=[0, .1, .6, .8], metavar=('X', 'Y', 'W', 'H'),
                        help='region of the screen to compare, as fractions (default: the lamp of a big window)')
    parser.add_argument('--ghostty-arg', action='append', default=[], help='extra Ghostty flag, e.g. --ghostty-arg=--custom-shader=')
    args = parser.parse_args()
    args.output = args.output.resolve()
    if any(c.isspace() for c in str(args.output)):
        parser.error('--output must not contain spaces (Ghostty re-splits its command)')
    args.output.mkdir(parents=True, exist_ok=True)
    # Run a copy from --output: a fresh Ghostty process may need macOS's
    # permission to read ~/Documents (where a checkout often lives), and
    # its prompt would hold the run.
    binary = args.output / 'lavatui'
    shutil.copy2(args.binary, binary)
    args.binary = binary
    wrapper = args.output / 'wrap.sh'
    wrapper.write_text(WRAPPER)
    wrapper.chmod(0o755)
    results = []
    for n in range(1, args.repeat + 1):
        for size in args.sizes:
            cols, rows = map(int, size.split('x'))
            for style in args.styles:
                name = f'{args.label}-{style}-{size}' + (f'-{n}' if args.repeat > 1 else '')
                result = run_case(args, name, cols, rows, style, wrapper)
                slow = sum(n for fps, n in result['fps_counts'].items() if int(fps) < args.fps)
                if slow:
                    print(f'warning: {name}: {slow} frames below {args.fps} fps (window not in front?)',
                          file=sys.stderr, flush=True)
                results.append(result)
                print(json.dumps(result), flush=True)
    (args.output / f'{args.label}.json').write_text(json.dumps(results, indent=2) + '\n')


if __name__ == '__main__':
    main()
