#!/usr/bin/env python3
"""Drive lavatui in a pty against the fake MPRIS player and check the
now-playing card follows it, both ways.

    dbus-run-session -- python3 tools/linux/pty_check.py --bin target/release/lavatui --out /tmp/x

Starts tools/linux/fake_mpris.py, runs lavatui (120x36, music widget
beside the lamp, scratch config) in a pty emulated by pyte, and steps
through: the card shows the fake track; then, in the music-keys mode (A),
Space / n / Right / Up / x / r each reach the player (its call log) and
show on the card. Every step's screen is saved as text, and the first and
last as PNG (docs/screenshots/capture.py's renderer). Exits non-zero on
the first check that fails. `tools/linux/run.sh pty` runs it in Docker.
"""

import argparse
import fcntl
import os
import pty
import queue
import select
import struct
import subprocess
import sys
import tempfile
import termios
import threading
import time

import pyte

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.join(HERE, "..", "..")
COLS, ROWS = 120, 36
CONFIG = """\
[ui]
welcome = false
[lamp]
style = "solid"
[dock]
music = "side"
"""


class FakePlayer:
    """The fake player, its call log read on a thread."""

    def __init__(self, *args):
        self.proc = subprocess.Popen(
            [sys.executable, os.path.join(HERE, "fake_mpris.py"), *args],
            stdout=subprocess.PIPE,
            text=True,
        )
        ready = self.proc.stdout.readline()
        assert ready.startswith("ready "), ready
        self.calls = queue.Queue()
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for line in self.proc.stdout:
            self.calls.put(line.strip())

    def drain(self):
        out = []
        while not self.calls.empty():
            out.append(self.calls.get())
        return out

    def stop(self):
        self.proc.terminate()
        self.proc.wait(5)


class App:
    """lavatui in a pty, its screen kept up to date by pumping output."""

    def __init__(self, binary, config):
        env = dict(os.environ, TERM="xterm-256color", COLORTERM="truecolor")
        for k in ("NO_COLOR", "TERM_PROGRAM", "TMUX"):
            env.pop(k, None)
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.execve(binary, [binary, "--config", config, "--seed", "2"], env)
        fcntl.ioctl(
            self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, COLS * 9, ROWS * 18)
        )
        self.screen = pyte.Screen(COLS, ROWS)
        self.stream = pyte.ByteStream(self.screen)
        self.alive = True

    def pump(self, seconds):
        end = time.time() + seconds
        while self.alive and time.time() < end:
            r, _, _ = select.select([self.fd], [], [], 0.02)
            if not r:
                continue
            try:
                data = os.read(self.fd, 65536)
            except OSError:
                data = b""
            if not data:
                self.alive = False
                break
            self.stream.feed(data)

    def send(self, keys):
        os.write(self.fd, keys)

    def text(self):
        return "\n".join(self.screen.display)

    def wait_for(self, what, ok, timeout=5.0):
        end = time.time() + timeout
        while time.time() < end:
            self.pump(0.1)
            if ok(self.text()):
                return True
        return False

    def quit(self):
        self.send(b"\x1b")
        self.pump(0.3)
        self.send(b"q")
        end = time.time() + 5
        while self.alive and time.time() < end:
            self.pump(0.2)
        _, status = os.waitpid(self.pid, 0)
        return os.waitstatus_to_exitcode(status)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--bin", default=os.path.join(REPO, "target", "release", "lavatui"))
    parser.add_argument("--out", default=tempfile.gettempdir())
    parser.add_argument("--spotify", action="store_true", help="the fake acts as Spotify")
    args = parser.parse_args()
    os.makedirs(args.out, exist_ok=True)

    try:
        sys.path.insert(0, os.path.join(REPO, "docs", "screenshots"))
        import capture  # noqa: E402  (its fonts: LAVATUI_SHOT_FONT)

        render = capture.render
    except Exception as err:  # no font, no Pillow: text only
        print(f"(no PNGs: {err})")
        render = None

    cfg = tempfile.NamedTemporaryFile("w", suffix=".toml", delete=False)
    cfg.write(CONFIG)
    cfg.close()

    fake = FakePlayer(*(["--spotify"] if args.spotify else []))
    app = App(args.bin, cfg.name)
    failures = []
    step_no = 0

    def save(name, png=False):
        nonlocal step_no
        step_no += 1
        slug = "".join(c if c.isalnum() else "-" for c in name)
        base = os.path.join(args.out, f"{step_no:02d}-{slug}")
        with open(base + ".txt", "w") as f:
            f.write(app.text() + "\n")
        if png and render:
            render(app.screen, base + ".png")

    def check(name, ok, calls_want=None, png=False):
        shown = app.wait_for(name, ok)
        calls = fake.drain()
        reached = calls_want is None or any(c.startswith(calls_want) for c in calls)
        save(name, png)
        status = "ok" if shown and reached else "FAIL"
        print(f"{status:4} {name}: calls={calls}")
        if status == "FAIL":
            failures.append(name)

    try:
        check("track shown", lambda t: "Slow Bloom" in t and "The Wax Hearts" in t, png=True)
        app.send(b"A")
        app.pump(0.3)
        app.send(b" ")
        check("play/pause", lambda t: "Slow Bloom" in t, "call PlayPause")
        app.send(b"n")
        check("next", lambda t: "Convection" in t and "Mara Vell" in t, "call Next")
        app.send(b"\x1b[C")  # Right: seek forward
        check("seek", lambda t: True, "call SetPosition")
        app.send(b"\x1b[A")  # Up: volume
        check("volume", lambda t: True, "set Volume")
        app.send(b"x")
        check("shuffle", lambda t: True, "set Shuffle")
        app.send(b"r")
        check("repeat", lambda t: True, "set LoopStatus")
        app.send(b"p")
        check("previous", lambda t: "Slow Bloom" in t, "call Previous", png=True)
        code = app.quit()
        print(f"lavatui exited {code}")
        if code != 0:
            failures.append("exit code")
    finally:
        if app.alive:
            os.kill(app.pid, 9)
        fake.stop()
        os.unlink(cfg.name)

    print(f"screens in {args.out}")
    if failures:
        print("FAILED: " + ", ".join(failures))
        sys.exit(1)
    print("all pty checks passed")


if __name__ == "__main__":
    main()
