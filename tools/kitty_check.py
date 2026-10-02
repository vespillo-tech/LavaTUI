#!/usr/bin/env python3
"""Check the cover's kitty-graphics output in a real pty (no kitty needed).

Runs the release binary as if in Ghostty (TERM_PROGRAM=ghostty, so
`art.detail = "auto"` picks pixels) with the cover and music widgets
placed, presses keys on a timeline, records every byte it writes, then
reports what a kitty-protocol terminal would have been told:

* each transmission (`a=T`): image id, cells (c × r), chunk count, and the
  PNG it carried (decoded and saved next to the report);
* in which synchronized frame (`CSI ? 2026 h … l`) each chunk went, and
  the most graphics bytes in any one frame;
* the placeholder cells (U+10EEEE) drawn, by image id (from their
  `38;2;r;g;b` colour);
* every deletion (`a=d,d=I`), and that the way out deletes ours.

It plays the terminal's part in the start-up probe (graphics/probe.rs):
when lavatui asks (`a=q`), it answers OK, then the OSC 10 fence. With
`--no-answer` it stays silent (as a terminal without kitty graphics
would): expect no transmission and no placeholder, the cover in text
cells. With `--ghostex` it runs as Ghostex's built-in terminal does
(`ZMX_SESSION`, `GHOSTEX_SESSION_ID`): expect not even a query.

Needs Spotify playing a track with a cover (it reads the live player).

    cargo build --release
    python3 tools/kitty_check.py [--out DIR] [--cols 120 --rows 36]
    python3 tools/kitty_check.py --no-answer
    python3 tools/kitty_check.py --ghostex
"""
import argparse, base64, fcntl, os, pty, re, select, signal, struct, sys, tempfile, termios, time

HERE = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.join(HERE, "..", "target", "release", "lavatui")
APC = re.compile(rb"\x1b_G([^;\x1b]*)(?:;([^\x1b]*))?\x1b\\")
SYNC = re.compile(rb"\x1b\[\?2026h(.*?)\x1b\[\?2026l", re.S)
PLACEHOLDER = "\U0010EEEE".encode()
QUERY = b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\"
ANSWER = b"\x1b_Gi=31;OK\x1b\\\x1b]10;rgb:ffff/ffff/ffff\x1b\\"
FG = re.compile(rb"38;2;(\d+);(\d+);(\d+)")


def run(args, toml, keys, resize):
    cfg = tempfile.NamedTemporaryFile("w", suffix=".toml", delete=False)
    cfg.write(toml)
    cfg.close()
    argv = [BIN, "--config", cfg.name, "--frames", str(args.frames), "--seed", "2"]
    env = dict(os.environ, TERM="xterm-256color", COLORTERM="truecolor", TERM_PROGRAM="ghostty")
    for k in ("TMUX", "STY", "NO_COLOR", "ZELLIJ", "ZMX_SESSION", "GHOSTEX_SESSION_ID", "LAVATUI_GRAPHICS"):
        env.pop(k, None)
    if args.ghostex:
        env.update(ZMX_SESSION="check", GHOSTEX_SESSION_ID="check")
    pid, fd = pty.fork()
    if pid == 0:
        os.execve(BIN, argv, env)

    def size(cols, rows):
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, cols * 9, rows * 18))

    size(args.cols, args.rows)
    out, start, answered = b"", time.time(), False
    while True:
        if not answered and QUERY in out:
            answered = True
            if not args.no_answer:
                os.write(fd, ANSWER)
        el = time.time() - start
        while keys and keys[0][0] <= el:
            os.write(fd, keys.pop(0)[1])
        while resize and resize[0][0] <= el:
            size(*resize.pop(0)[1])
            os.kill(pid, signal.SIGWINCH)
        r, _, _ = select.select([fd], [], [], 0.02)
        if r:
            try:
                data = os.read(fd, 1 << 16)
            except OSError:
                break
            if not data:
                break
            out += data
        if el > 90:
            os.kill(pid, 9)
            break
    os.waitpid(pid, 0)
    os.unlink(cfg.name)
    return out


def controls(c):
    return dict(kv.split("=", 1) for kv in c.decode().split(",") if "=" in kv)


def report(out, outdir, label):
    print(f"== {label}: {len(out)} bytes written")
    frames = [m for m in SYNC.finditer(out)]
    print(f"   synchronized frames: {len(frames)}")
    # Graphics bytes per frame, and which frames carried any.
    per = []
    for i, m in enumerate(frames):
        g = sum(len(a.group(0)) for a in APC.finditer(m.group(1)))
        if g:
            per.append((i, g))
    outside = {a.start() for a in APC.finditer(out) if not any(f.start() <= a.start() < f.end() for f in frames)}

    def frame_of(pos):
        return next((i for i, f in enumerate(frames) if f.start() <= pos < f.end()), None)

    print(f"   frames with graphics: {[i for i, _ in per]}")
    print(f"   most graphics bytes in one frame: {max((g for _, g in per), default=0)}")
    # Transmissions.
    sends, cur = [], None
    for a in APC.finditer(out):
        c = controls(a.group(1))
        if c.get("a") == "T":
            cur = {"c": c, "data": [a.group(2) or b""], "frame": frame_of(a.start())}
            sends.append(cur)
        elif c.get("a") == "d":
            where = "outside a frame: the way out" if a.start() in outside else f"frame {frame_of(a.start())}"
            print(f"   delete: {a.group(1).decode()}  ({where})")
        elif cur is not None and "m" in c:
            cur["data"].append(a.group(2) or b"")
        if cur is not None and c.get("m") == "0":
            cur = None
    for n, s in enumerate(sends):
        c = s["c"]
        png = base64.b64decode(b"".join(s["data"]))
        path = os.path.join(outdir, f"{label}-sent-{n}.png")
        with open(path, "wb") as f:
            f.write(png)
        dims = struct.unpack(">II", png[16:24]) if png[:8] == b"\x89PNG\r\n\x1a\n" else None
        print(f"   transmit #{n}: i={c['i']} c={c['c']} r={c['r']} U={c.get('U')} f={c.get('f')} "
              f"q={c.get('q')} chunks={len(s['data'])} from frame {s['frame']} png={len(png)} B {dims} -> {path}")
    # Placeholder cells by id.
    ids, by_frame = {}, {}
    for m in re.finditer(re.escape(PLACEHOLDER), out):
        f = frame_of(m.start())
        by_frame[f] = by_frame.get(f, 0) + 1
        sgr = out.rfind(b"38;2;", 0, m.start())
        fg = FG.match(out, sgr)
        if fg:
            r, g, b = map(int, fg.groups())
            ids[(r << 16) | (g << 8) | b] = ids.get((r << 16) | (g << 8) | b, 0) + 1
    print(f"   probe query sent: {'yes' if QUERY in out else 'no'}")
    print(f"   placeholder cells written, by image id: {ids}")
    print(f"   ...and by frame (cells are only rewritten when they change): {by_frame}")
    return sends, ids


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=tempfile.mkdtemp(prefix="lavatui-kitty-"))
    ap.add_argument("--cols", type=int, default=120)
    ap.add_argument("--rows", type=int, default=36)
    ap.add_argument("--frames", type=int, default=600)
    ap.add_argument("--no-answer", action="store_true", help="never answer the probe")
    ap.add_argument("--ghostex", action="store_true", help="run as in Ghostex's terminal (zmx)")
    args = ap.parse_args()
    os.makedirs(args.out, exist_ok=True)
    toml = '[lamp]\nstyle = "solid"\n[dock]\nmusic = "side"\ncover = "side"\n'
    # 0-3 s: in the panel (sent once); 3 s: o → on the lava (same size:
    # nothing re-sent); 5 s: resize (new size: sent again, old deleted);
    # 7 s: o → off (deleted); the run ends by frame count (the way out).
    keys = [(3.0, b"o"), (7.0, b"o")]
    resize = [(5.0, (160, 44))]
    out = run(args, toml, keys, resize)
    with open(os.path.join(args.out, "pty-output.bin"), "wb") as f:
        f.write(out)
    report(out, args.out, "cover")
    print(f"raw output and PNGs in {args.out}")


if __name__ == "__main__":
    main()
