#!/usr/bin/env python3
"""Drain sized ptys and summarize LavaTUI's opt-in CSV frame traces.

No terminal emulator is involved: this measures application/pty timings,
not Ghostty's presentation latency. Run before/after with the same seed.
Uses only Python's standard library (Unix pty support required).
"""
import argparse
import csv
from collections import Counter
import errno
import fcntl
import json
import math
import os
from pathlib import Path
import pty
import select
import signal
import struct
import termios
import time


def summarize(path):
    with path.open() as file:
        rows = list(csv.DictReader(file))
    # First frame has no preceding interval.
    samples = [r for r in rows if int(r['frame']) > 0 and not int(r['input'])]
    intervals = sorted(int(r['interval_us']) / 1000 for r in samples)
    if not intervals:
        raise RuntimeError(f'no timed frames: {path}')
    def percentile(p):
        return intervals[math.ceil(p * len(intervals)) - 1]
    spikes = []
    for i, row in enumerate(rows):
        if i == 0 or int(row['input']):
            continue
        period = 1_000_000 / int(row['fps'])
        if int(row['interval_us']) <= period * 2:
            continue
        previous = rows[i - 1]
        components = {k: int(row[k]) for k in ('tick_us', 'draw_us', 'diff_us', 'write_us', 'flush_us')}
        largest = max(components, key=components.get)
        wait = int(row['wait_end_us']) - int(row['wait_start_us'])
        # These are measured locations, not proof of OS scheduler attribution.
        cause = 'wait/deadline wake' if wait > period * 1.5 else largest
        if int(row['save_us']) > period / 2:
            cause = 'config save'
        elif int(previous['tick_us']) + int(previous['draw_us']) > period:
            cause = 'preceding frame overrun / pacing'
        spikes.append(dict(frame=int(row['frame']), interval_ms=int(row['interval_us']) / 1000,
                           location=cause, wait_us=wait, deadline_miss_us=int(row['deadline_miss_us']), **components))
    return dict(trace=str(path), frames=len(rows), seconds=(int(rows[-1]['end_us']) - int(rows[0]['end_us'])) / 1e6,
                p50_ms=percentile(.5), p99_ms=percentile(.99), max_ms=max(intervals),
                fps_counts=dict(Counter(int(r["fps"]) for r in rows)),
                bytes=sum(int(r['bytes']) for r in rows), spikes=spikes)


def run(args):
    args.output.mkdir(parents=True, exist_ok=True)
    binary = str(args.binary.resolve())
    children = {}
    env = dict(os.environ, TERM='xterm-256color', COLORTERM='truecolor')
    env.pop('NO_COLOR', None)
    for size in args.sizes:
        cols, rows = map(int, size.split('x'))
        for style in args.styles:
            name = f'{size}-{style}'
            cfg = args.output / f'{name}.toml'
            cfg.write_text('')
            trace = args.output / f'{name}.csv'
            pid, fd = pty.fork()
            if pid == 0:
                fcntl.ioctl(1, termios.TIOCSWINSZ, struct.pack('HHHH', rows, cols, cols * 8, rows * 16))
                os.execve(binary, [binary, '--seed', '7', '--fps', '60', '--style', style,
                                  '--config', str(cfg.resolve()), '--trace', str(trace.resolve())], env)
            children[fd] = dict(pid=pid, trace=trace, bytes=0, started=time.monotonic(), quit=False, changed=False, load_start=os.getloadavg(), tail=b'')
    results = []
    try:
        while children:
            now = time.monotonic()
            for fd, child in children.items():
                elapsed = now - child['started']
                if args.change and not child['changed'] and elapsed > 3:
                    os.write(fd, b']')
                    child['changed'] = True
                if not child['quit'] and elapsed >= args.seconds:
                    os.write(fd, b'q')
                    child['quit'] = True
                if elapsed > args.seconds + 15:
                    raise RuntimeError(f"child failed to exit: {child['trace']}")
            ready, _, _ = select.select(list(children), [], [], .02)
            for fd in ready:
                try:
                    data = os.read(fd, 262144)
                except OSError as err:
                    if err.errno != errno.EIO:
                        raise
                    data = b''
                if data:
                    children[fd]['bytes'] += len(data)
                    children[fd]['tail'] = (children[fd]['tail'] + data)[-2048:]
                    continue
                child = children.pop(fd)
                os.close(fd)
                _, status, usage = os.wait4(child['pid'], 0)
                if status:
                    raise RuntimeError(f"child exit {status}: {child['trace']}: {child['tail']!r}")
                result = summarize(child['trace'])
                result['pty_bytes'] = child['bytes']
                result['process_cpu_s'] = usage.ru_utime + usage.ru_stime
                result['load_start'] = child['load_start']
                result['load_end'] = os.getloadavg()
                results.append(result)
                print(json.dumps({**{k: v for k, v in result.items() if k != "spikes"}, "spike_count": len(result["spikes"])}), flush=True)
    finally:
        for fd, child in children.items():
            os.kill(child['pid'], signal.SIGTERM)
            os.close(fd)
            os.waitpid(child['pid'], 0)
    (args.output / 'summary.json').write_text(json.dumps(results, indent=2) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/release/lavatui'))
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--seconds', type=float, default=300)
    parser.add_argument('--sizes', nargs='+', default=['80x24', '160x40', '250x70'])
    parser.add_argument('--styles', nargs='+', default=['solid', 'braille'])
    parser.add_argument('--change', action='store_true', help='exercise a debounced config save after 3s')
    run(parser.parse_args())
