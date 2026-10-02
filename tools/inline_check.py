#!/usr/bin/env python3
"""Check the cover's iTerm2 / sixel output in a real pty (no such terminal needed).

The sibling of kitty_check.py, for the protocols that place a picture at
the cursor. Runs the release binary with LAVATUI_GRAPHICS=iterm (or sixel)
in a pty whose size says 9 x 18 pixel cells, the cover and music widgets
placed, presses keys on a timeline, records every byte it writes, then
follows the cursor through it (a small VT parser: cursor moves, text,
DECSC/DECRC) and reports:

* each picture placed: frame, cell rect, how it was put there (cursor
  saved and restored around it) and what it carried (the PNG, saved; or
  the sixel's pixel size and colour registers);
* that every picture went inside a synchronized frame (`CSI ? 2026 h ... l`);
* the frames where text was written into a placed picture's cells, and
  whether each such frame rewrote *all* of them (a move / hide / overlay:
  correct) or only some (stale picture pixels left on screen: a bug);
* that no sentinel cell (U+10EEED) ever reached the terminal.

Needs Spotify playing a track with a cover (it reads the live player).

    cargo build --release
    python3 tools/inline_check.py --protocol iterm [--out DIR]
    python3 tools/inline_check.py --protocol sixel
"""
import argparse, base64, fcntl, os, pty, re, select, signal, struct, tempfile, termios, time, unicodedata

HERE = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.join(HERE, "..", "target", "release", "lavatui")
CELL = (9, 18)
SENTINEL = "\U0010EEED"


def run(args, toml, keys, resize):
    cfg = tempfile.NamedTemporaryFile("w", suffix=".toml", delete=False)
    cfg.write(toml)
    cfg.close()
    argv = [BIN, "--config", cfg.name, "--frames", str(args.frames), "--seed", "2"]
    env = dict(os.environ, TERM="xterm-256color", COLORTERM="truecolor", LAVATUI_GRAPHICS=args.protocol)
    for k in ("TMUX", "STY", "NO_COLOR", "TERM_PROGRAM", "KITTY_WINDOW_ID", "GHOSTTY_RESOURCES_DIR", "LC_TERMINAL"):
        env.pop(k, None)
    pid, fd = pty.fork()
    if pid == 0:
        os.execve(BIN, argv, env)

    def size(cols, rows):
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, cols * CELL[0], rows * CELL[1]))

    size(args.cols, args.rows)
    out, start = b"", time.time()
    while True:
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


class Vt:
    """Just enough of a terminal to know which cells text lands in."""

    CSI = re.compile(rb"\x1b\[([0-9;?<>=]*)([ -/]*)([@-~])")

    def __init__(self):
        self.y = self.x = 0
        self.saved = (0, 0)
        self.frame = -1          # index of the current synchronized frame
        self.in_frame = False
        self.writes = []         # (frame, y, x, char)
        self.pictures = []       # dicts

    def feed(self, data):
        i, n = 0, len(data)
        while i < n:
            b = data[i]
            if b == 0x1b and i + 1 < n:
                nxt = data[i + 1]
                if nxt == ord("["):
                    m = self.CSI.match(data, i)
                    if not m:
                        i += 1
                        continue
                    self.csi(m.group(1).decode(), m.group(3).decode())
                    i = m.end()
                elif nxt == ord("7"):
                    self.saved = (self.y, self.x)
                    i += 2
                elif nxt == ord("8"):
                    self.y, self.x = self.saved
                    i += 2
                elif nxt in b"P]_^X":
                    # DCS / OSC / APC / PM / SOS: up to ST (or BEL for OSC).
                    end = data.find(b"\x1b\\", i + 2)
                    bel = data.find(b"\x07", i + 2) if nxt == ord("]") else -1
                    if bel != -1 and (end == -1 or bel < end):
                        body, i = data[i + 2:bel], bel + 1
                    else:
                        body, i = data[i + 2:end], (end + 2 if end != -1 else n)
                    self.string(chr(nxt), body, data, i)
                else:
                    i += 2
            elif b < 0x20:
                if b == 0x0d:
                    self.x = 0
                elif b == 0x0a:
                    self.y += 1
                i += 1
            else:
                # One UTF-8 character.
                length = 1 if b < 0x80 else 2 if b < 0xE0 else 3 if b < 0xF0 else 4
                ch = data[i:i + length].decode("utf-8", "replace")
                i += length
                if unicodedata.combining(ch):
                    continue
                self.writes.append((self.frame, self.y, self.x, ch))
                self.x += 1

    def csi(self, params, final):
        if final == "H":
            parts = [int(p) if p else 1 for p in params.split(";")] if params else [1, 1]
            parts += [1] * (2 - len(parts))
            self.y, self.x = parts[0] - 1, parts[1] - 1
        elif params == "?2026" and final == "h":
            self.frame += 1
            self.in_frame = True
        elif params == "?2026" and final == "l":
            self.in_frame = False

    def string(self, kind, body, data, after):
        pic = None
        if kind == "]" and body.startswith(b"1337;File="):
            head, _, payload = body.partition(b":")
            opts = dict(kv.split("=", 1) for kv in head[len(b"1337;File="):].decode().split(";") if "=" in kv)
            pic = {"kind": "iterm", "cols": int(opts["width"]), "rows": int(opts["height"]), "opts": opts,
                   "payload": payload}
        elif kind == "P" and b"q" in body[:12]:
            m = re.match(rb'[0-9;]*q"1;1;(\d+);(\d+)', body)
            w, h = (int(m.group(1)), int(m.group(2))) if m else (0, 0)
            pic = {"kind": "sixel", "px": (w, h), "cols": -(-w // CELL[0]), "rows": -(-h // CELL[1]),
                   "registers": len(set(re.findall(rb"#(\d+);2;", body))), "bytes": len(body) + 4}
        if pic:
            # Restored right after?
            pic.update(frame=self.frame, synced=self.in_frame, y=self.y, x=self.x,
                       restored=data[after:after + 2] == b"\x1b8")
            self.pictures.append(pic)


def report(out, outdir, label):
    vt = Vt()
    vt.feed(out)
    print(f"== {label}: {len(out)} bytes, {vt.frame + 1} synchronized frames")
    sentinels = [w for w in vt.writes if w[3] == SENTINEL]
    print(f"   sentinel cells reaching the terminal: {len(sentinels)}" + ("  <-- BUG" if sentinels else ""))
    bad = 0
    for n, p in enumerate(vt.pictures):
        cells = {(p["y"] + r, p["x"] + c) for r in range(p["rows"]) for c in range(p["cols"])}
        what = ""
        if p["kind"] == "iterm":
            png = base64.b64decode(p["payload"])
            path = os.path.join(outdir, f"{label}-placed-{n}.png")
            with open(path, "wb") as f:
                f.write(png)
            dims = struct.unpack(">II", png[16:24]) if png[:8] == b"\x89PNG\r\n\x1a\n" else None
            size_ok = int(p["opts"].get("size", -1)) == len(png)
            what = f"png {len(png)} B {dims} size= {'ok' if size_ok else 'WRONG'} -> {path}"
        else:
            what = f"sixel {p['px'][0]}x{p['px'][1]} px, {p['registers']} colours, {p['bytes']} B"
        print(f"   placed #{n}: frame {p['frame']} at row {p['y']} col {p['x']}, {p['cols']}x{p['rows']} cells, "
              f"{'in a synchronized frame' if p['synced'] else 'OUTSIDE A FRAME <-- BUG'}, "
              f"cursor {'restored' if p['restored'] else 'NOT RESTORED <-- BUG'}; {what}")
        # Until the next picture: which frames wrote into its cells, and all of them?
        until = vt.pictures[n + 1]["frame"] if n + 1 < len(vt.pictures) else 1 << 30
        by_frame = {}
        for f, y, x, _ in vt.writes:
            if p["frame"] < f <= until and (y, x) in cells:
                by_frame.setdefault(f, set()).add((y, x))
        # The placing frame writes the blanks under it first: not counted.
        if not by_frame:
            print("      its cells: never written over while it was up")
        for f, hit in sorted(by_frame.items()):
            whole = hit >= cells
            covered = n + 1 < len(vt.pictures) and f == until
            ok = whole or covered
            bad += not ok
            print(f"      frame {f}: {len(hit)}/{len(cells)} of its cells rewritten"
                  f"{' (the next picture went over it)' if covered and not whole else ''}"
                  f"{'' if ok else '  <-- STALE PIXELS LEFT'}")
            break  # the first rewrite ends it
    print(f"   problems: {bad + len(sentinels) + sum(not p['synced'] or not p['restored'] for p in vt.pictures)}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--protocol", choices=["iterm", "sixel"], default="iterm")
    ap.add_argument("--out", default=None)
    ap.add_argument("--cols", type=int, default=120)
    ap.add_argument("--rows", type=int, default=36)
    ap.add_argument("--frames", type=int, default=600)
    args = ap.parse_args()
    args.out = args.out or tempfile.mkdtemp(prefix=f"lavatui-{args.protocol}-")
    os.makedirs(args.out, exist_ok=True)
    toml = '[lamp]\nstyle = "solid"\n[dock]\nmusic = "side"\ncover = "side"\n'
    # 0-3 s: in the panel (placed once); 3 s: o -> on the lava (moved:
    # old cells rewritten, placed again); 4 s: ? help over it, 4.6 s: ?
    # closed; 5 s: resize (screen cleared: placed again, at the new size);
    # 6 s: ctrl-l (placed again); 7 s: o -> off (its cells rewritten).
    keys = [(3.0, b"o"), (4.0, b"?"), (4.6, b"?"), (6.0, b"\x0c"), (7.0, b"o")]
    resize = [(5.0, (160, 44))]
    out = run(args, toml, keys, resize)
    with open(os.path.join(args.out, "pty-output.bin"), "wb") as f:
        f.write(out)
    report(out, args.out, args.protocol)
    print(f"raw output in {args.out}")


if __name__ == "__main__":
    main()
