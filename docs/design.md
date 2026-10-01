# LavaTUI — Layout & Visual Design Spec

Status: **contract** for `lava-xxx` (TUI shell), `lava-bdj`/`lava-y7g`
(styles), `lava-ef7` (faces) and `lava-h0f` (palettes/perf); v1.1
(`lava-9vj`) dropped the glass frame, the lighting pass and the heatmap,
dither and crt styles, and turned the panel into a widget dock (§4.6):
the clock and pomodoro each sit in the side panel, on the lava, or off. If the code and this doc disagree, fix one of them on purpose,
not by accident.

The one rule above all others: **the lamp is the hero, and the screen is
never cluttered.** At every size from 1×1 to 400×120 the app shows fewer
things rather than cramming more. Elements drop out whole. They never
truncate mid-word, wrap, overlap or overflow.

---

## 0. Vocabulary

| Term | Meaning |
|---|---|
| **cols × rows** | Terminal size in cells. |
| **visual aspect** `A` | `cols / (rows × cell_aspect)`: the true on-screen width÷height. `cell_aspect` defaults to 2.0 (see §2.3). |
| **content area** | Terminal minus the status bar row when it's shown. |
| **lamp region** | The part of the content area that belongs to the lamp. The fluid fills it edge to edge: there is no frame or silhouette. |
| **widget** | A dock widget (§4.6): the clock, the pomodoro or music (now playing). Each is placed `side`, `overlay` (on the lava) or `off`. |
| **panel** | The widgets placed `side` (clock + pomodoro by default), stacked beside the lamp (right panel) or below it (bottom panel). |
| **on the lava** | The widgets placed `overlay`, stacked over the lamp at the dock's anchor on a soft backing (§4.6). |
| **chip** | The single-line fallback for a widget with no room where it was put: ` 14:32 ` or ` ▸ 18:24 `, drawn over a corner of the lamp. |
| **toast** | A transient one-line message, e.g. the style name after pressing `s`. |

---

## 1. Responsive layout

### 1.1 Principle

Layout is a **pure function** `layout(cols, rows, settings) -> Layout`
(a set of non-overlapping `Rect`s plus a few variant choices). It has no
history and no hysteresis, so it's trivially snapshot-testable. Required
tests in `ui/`:

* For every size from 1×1 to 300×100: no rect is out of bounds, no two
  rects overlap, and every shown element meets its own minimum size.
* Snapshot tests at the mockup sizes below.

### 1.2 Tiers at a glance

These tiers are shorthand for common shapes. The authoritative rules are
in §1.3, and each one is a condition on cols and/or rows. When a terminal
is wide but short (or narrow but tall), each element follows its own rule.
For example, 160×22 gets the full hint text (it's wide) but no date line
(it's short).

| Tier | Typical size | Lamp | Clock | Pomodoro | Status bar | Hints |
|---|---|---|---|---|---|---|
| **Micro** | < 20 cols or < 8 rows | whole screen | – | – | – | – |
| **Tiny** | 20–39 × 8–13 | whole screen | chip `14:32` | chip `▸ 18:24` (replaces the clock while running or paused) | – | – |
| **Small** | 40–79 × 14–23 | the content area | chip, or right panel with M face if ≥ 60 % width remains | chip or panel: time + bar | yes: style · palette | `? help` and as many more as fit |
| **Medium** | 80–119 × 24–35 | content area left of the panel | panel, M face | panel: label, time, bar, dots | yes | all hints |
| **Large** | 120–199 × 36–55 | as Medium | panel, L face + date line | full | yes | all hints |
| **Huge** | ≥ 200 × ≥ 56 | as Medium | panel, largest face that fits + date; the panel widens up to 56 for it (§1.4) | full | yes | all hints |

Micro is `cols < 20 || rows < 8`; the other tier cuts are 40 × 14,
80 × 24 and 200 × 56 (Huge). Overlays pick their form by their own size
checks (help sheet ≥ 68 × 20, §4.3; picker sheet ≥ 80 × 16, §4.4), not by
tier alone. All of these, and the §1.3 status-bar, toast and date-line
cuts, are named constants in one place at the top of `ui/layout.rs`
(`TINY`, `SMALL`, `MEDIUM`, `HUGE`, `HELP_SHEET`, `PICKER_SHEET`, …).

At any size, a portrait shape (narrow and tall) moves the panel *below*
the lamp (§1.4).

### 1.3 Element rules (authoritative)

| Element | Shown when | Variant rules |
|---|---|---|
| **Lamp** | always (if `cols < 4` or `rows < 2`, the screen is painted `bg`, nothing else) | fills what the status bar and panel leave (§2.1) |
| **Status bar** | `rows ≥ 14 && cols ≥ 30 && status_bar_on` and not minimal mode | segments drop per §4.1 |
| **Panel** | some widget is placed `side` and the placement algorithm (§1.4) finds a slot | face variant = largest that fits the panel's inner rect |
| **Widgets on the lava** | some widget is placed `overlay`, not Micro, the lamp is ≥ 28 × 10, and their smallest forms fit (§4.6) | largest forms that fit in 60 % of the lamp's width and half its height, backing included ≤ 35 % of its area |
| **Chip** | a widget with no room where it was put (`side` with no panel, `overlay` that didn't fit), `cols ≥ 20 && rows ≥ 8` | the highest-ranked such widget: the pomodoro while one is running (`▸`) or paused (`‖`), `break` before a break's time, else the clock |
| **Date line** | in panel, `rows ≥ 36`, and the panel still fits | `thu 1 oct`, dim, lowercase |
| **Pomodoro label** `focus` / `break` | panel inner width ≥ 18 | — |
| **Cycle dots** `●●○○` | panel inner width ≥ 22 | right-aligned on the label line |
| **Toasts** | `cols ≥ 16 && rows ≥ 4` | truncated by dropping the suffix (`braille 4/9` → `braille`), never mid-word |
| **Side margin** | the status bar and side sheets only (the lamp is edge-to-edge) | 2 if cols < 120, 4 if < 200, else `round(cols × 0.03)` |

**Hide priority.** When space runs out, things go in this order (first to
go at the top). The lamp is never hidden.

1. Extended key hints (dropped in the §4.1 order until only `? help` is left, then that too)
2. Date line
3. Cycle dots, then the pomodoro phase label
4. Clock face size (XL → L → M → S → `text`). With several widgets in
   one place, the last one shrinks first (on the lava: the pomodoro's
   3 → 2 → 1 rows before the face shrinks).
5. Panel, and the widgets on the lava (→ collapse into the chip;
   nothing is lost but size)
6. Status bar
7. Clock chip (a running pomodoro chip outranks it)
8. Pomodoro chip
9. ~~Lamp~~ — never

### 1.4 Panel placement algorithm

```
panel_w   = clamp(round(cols × 0.30), 22, 36)      // incl. 1-col inner padding each side
            // cols ≥ 200: widened to face_w + 2 (max 56) when the face needs it
panel_h   = face_h + (date? 2) + 2 + 3             // face, gap, label/time/bar
                                                   // (each side widget's height,
                                                   // 2 rows between; pomodoro alone: 3)

1. A ≥ 1.0 → right panel, flush with the right edge and vertically
   centred, if the lamp keeps ≥ 60 % of cols and ≥ 24 cols.
2. A < 1.0 → bottom panel (min(36, content_cols) wide, centred, one blank
   row below the lamp) if the lamp keeps ≥ 60 % of rows and ≥ 10 rows.
3. Otherwise no panel → chip.
```

Below 200 cols the panel's inner width is at most 36 − 2 = 34. From 200
cols up the panel grows only as far as the face it holds needs, up to
56 (inner 54): blocks XL (51 × 8) shows at Huge, blocks L with seconds
(54 × 5) when the terminal is ≥ 200 cols but under 56 rows. Narrower
faces keep the 36-col panel. Face size is still tried largest first
within the §1.3 hide order (the date line goes before a smaller face).

Centring: whenever a split leaves an odd cell, the extra cell goes
right/bottom. Always do it this way, so the composition never jitters by
a cell between neighbouring sizes.

### 1.5 Mockups

Legend: `░` liquid · `█▀▄` wax (solid style, half-blocks) · ` ` app
background. Real colours come from the palette (§5). These are captured
from the running app (pty, `--seed 2 --color none`, solid style) with the
lamp's liquid drawn as `░`; the times are whenever they were taken. Real
screenshots of most of these sizes are in `docs/screenshots/` (see the
README).

**Micro — 16×6.** Lamp only. Nothing else, ever.

```
░░░░░░░░░░░░░░░░
░░░░██▄░░░░▄░░░░
░░░░▀▀▀░░▄███░░░
░░░░░▄████▀▀▀░░░
░░░░█░▀████░░░░░
████████████████
```

**Tiny — 20×8.** The lamp fills the screen, plus the clock chip in the
bottom-right. No status bar, no hints.

```
░░░░░░░░░░░░░░░░░░░░
░░░░░▄▄▄░░░░░░░░░░░░
░░░░░████░░░░▄█▄░░░░
░░░░░░▀▀░░░▄█████░░░
░░░░░░▄███▄██▀▀▀░░░░
░░░░░▄░██████░░░░░░░
░░░░░█░░▄████▄░░░░░░
█████████████ 13:46
```

The same size with a pomodoro running. The chip switches to the
pomodoro: `▸` while running, coloured by phase (accent for focus,
`wax_hot` for breaks), and `‖` in `text` while paused.

```
░░░░░░░░░░░░░░░░░░░░
░░░░░▄▄▄░░░░░░░░░░░░
░░░░░████░░░░▄█▄░░░░
░░░░░░▀▀░░░▄█████░░░
░░░░░░▄███▄██▀▀▀░░░░
░░░░░▄░██████░░░░░░░
░░░░░█░░▄████▄░░░░░░
███████████░▸ 24:58
```

**Small — 50×16.** A panel would leave the lamp only 56 % of the width,
so the clock is a chip. The status bar appears.

```
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░▄████▄░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░████████░░░░░░░░░░░░░▄████▄░░░░░░░░░░
░░████▄░░░░░░███████▀░░░░░░░░░░░░███████▄░░░░░░░░░
░██████░░░░░█████▀▀░░░░░░░░░░░░░█████████░░░░░░░░░
░██████░░░░░████▀░░░░░░░░░░░░░░░████████░░░░░░░░░░
░██████░░░░░█████░▄██████░░░░░░░░▀████▀░░░░░░░░░░░
░░▀▀▀▀███████████▀████████░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░███████████░▀▀███▀▀░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░███████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░██████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░▄█████████▄▄░░░░░░░░░░░░░░▄▄▄▄▄▄▄▄▄░░░░░░░░░
███████████████████████████████████████████ 13:46
  ● solid    s style  c clock  p palette  ? help
```

**Small, wide — 72×18.** The lamp keeps 69 % of the width, so a right
panel appears with the M blocks face. The panel is too narrow for the
cycle dots.

```
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░▄▄███▄▄░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░█████████▄░░░░░░░░░░░░▄▄▄▄▄░░░░░░░░░░░ ▄█  ▀▀█ ▄ █ █ █▀▀
░░░░░░░░░░░░██████████░░░░░░░░░░▄████████░░░░░░░░░  █  ▀▀█ ▄ ▀▀█ █▀█
░░░░░░░░░░░░▀████████▀░░░░░░░░░▄█████████░░░░░░░░░ ▀▀▀ ▀▀▀     ▀ ▀▀▀
░░░░░░░░░░░░░░░▀▀▀▀░░░░░░░░░░░░██████████░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▀████████▀░░░░░░░░░
░░░░░░░░░░░░░░░░▄███████▄░░░░░░▄████████░░░░░░░░░░ focus
░░░░░░░░░░░░░░░░██████████░░░░░████████░░░░░░░░░░░ 25:00
░░░░░░░▄▄░░░░░░░▀████████░░░░░█████████▄░░░░░░░░░░ ────────────────────
░░░░░░███░░░░░░░░░▀▀▀▀▀▀░░░░░░██████████░░░░░░░░░░
░░░░░░▀██░░░░░░░░░░░░░░░░░░░░░░████████▀░░░░░░░░░░
░░░░░░░░░░░░▄▄▄▄░░░░░░░░░░░░░░█████████░░░░░░░░░░░
░░▄▄▄▄▄████████████▄▄░░░░░░░░▄█████████████▄▄▄▄░░░
██████████████████████████████████████████████████
  ● solid · lava           s style  c clock  p palette  ␣ pomo  ? help
```

**Medium — 80×24.** The reference size. The lamp takes 56 cols, the
panel the other 24, vertically centred. Status bar on the last row with
a 2-col inset.

```
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░▄██████▄▄░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░▄███████████▄░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░▄██████████████░░░░░░░░░░░░▄▄▄██▄▄▄░░░░░░░░░░
░░░░░░░░░░░▀███████████████░░░░░░░░▄███████████▄░░░░░░░░ ▄█  ▀▀█ ▄ █ █ █▀▀
░░░░░░░░░░░░▀█████████████░░░░░░░░▄█████████████░░░░░░░░  █  ▀▀█ ▄ ▀▀█ █▀█
░░░░░░░░░░░░░░▀▀███████▀▀░░▄▄▄▄▄▄▄███████████████░░░░░░░ ▀▀▀ ▀▀▀     ▀ ▀▀▀
░░░░░░░░░░░░░░░░░░░░░░░░░░██████████████████████░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░█████████████████████▀░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░▄▄█▄▄███████████████████▀░░░░░░░░░░░ focus             ○○○○
░░░░░░░░░░░░░░░░░░█████████████████░░▀▀▀▀▀░░░░░░░░░░░░░░ 25:00
░░░░░░░░░░░░░░░░░▄████████████████▀░░░░░░░░░░░░░░░░░░░░░ ──────────────────────
░░░░░░░░░░░░░░░░░████████████████▀░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░▀█████████████▀▀░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░██████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░▀█████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░▀████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░▄████████████████████▄▄▄░░░░░░░░░░░░░░
░░░░▄▄▄▄▄▄███████████████████████████████████▄▄▄░░░░░░░░
████████████████████████████████████████████████████████
  ● solid · lava        s style  c clock  p palette  m minimal  ␣ pomo  ? help
```

**Large — 120×36.** L face (blocks ×2), date line, every hint.

```
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░▄▄▄░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░▄▄██████████▄▄░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░████████████████▄░░░░░░░░░░░░░░░░░░▄▄████████▄░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░███████████████████▄░░░░░░░░░░░░░░▄█████████████▄░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░█████████████████████▄░░░░░░░░░░░░█████████████████░░░░░░░░░░░░░░░░   ██    ██████      ██  ██  ██████
░░░░░░░░░░░░░░░░░██████████████████████░░░░░░░░░░░███████████████████░░░░░░░░░░░░░░░ ████        ██  ██  ██  ██  ██
░░░░░░░░░░░░░░░░░▀█████████████████████░░░░░░░░░░████████████████████░░░░░░░░░░░░░░░   ██    ██████      ██████  ██████
░░░░░░░░░░░░░░░░░░▀████████████████████░░░░░░░░░░████████████████████░░░░░░░░░░░░░░░   ██        ██  ██      ██  ██  ██
░░░░░░░░░░░░░░░░░░░░▀███████████████████▄░░░░░░░█████████████████████░░░░░░░░░░░░░░░ ██████  ██████          ██  ██████
░░░░░░░░░░░░░░░░░░░░░░▀▀█████████████████░░░░░░░████████████████████░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▀███████████░░░░░░░██████████████████░░░░░░░░░░░░░░░░░ thu 1 oct
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░███████████░░░░░░░▀███████████████▀░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░██████████▀░░░░░░░░▀████████████▀░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░██████████░░░░░░░░░░░▀▀██████▀▀░░░░░░░░░░░░░░░░░░░░░░ focus                         ○○○○
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░██████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 25:00
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▄██████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ ──────────────────────────────────
░░░░░░░░░░░░░░░░░░░░░░░░░▄▄▄█████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░▄███████████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░▄████████████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░█████████████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░█████████████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░█████████████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░████████████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░▀██████████████████████▄▄▄▄▄▄▄▄▄▄░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░▄███████████████████████████████████▄▄░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░▄▄▄▄▄▄▄██████████████████████████████████████████▄▄▄░░░░░░░░░░░░░░░░░
▄▄▄▄▄▄▄▄▄▄████████████████████████████████████████████████████████████▄▄▄▄░░░░░░░░░░
████████████████████████████████████████████████████████████████████████████████████
    ● solid · lava                                            s style  c clock  p palette  m minimal  ␣ pomo  ? help
```

**Wide — 160×22** (`A ≈ 3.8`). A wide wax tank with convection cells;
the right panel takes 36 cols. This terminal is short, so there's no
date line, but it's wide, so every hint shows.

```
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▄▄▄▄░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▄██████▄░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▄▄████▄▄░░░░░░░░░░░░░░░░░░░░░▄████████▄░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▄█████████▄░░░░░░░░░░░░░░░░░░░██████████░░░░░░░▄▄███████▄▄░░░▄██▄░░░▄████▄░░░░░░░░░░░░   ██    ██████      ██  ██  ██████
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░███████████▄░░░░░░░░░░░░░░░░░░███████████░░░░▄█████████████▄██████░▄███████░░░░░░░░░░░ ████        ██  ██  ██  ██  ██
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░████████████░░░░░░░░░░░░░░░░░░██████████▀░░░██████████████████████░▀███████░░░░░░░░░░░   ██    ██████      ██████  ██████
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░██████████░░░░░░░░░░░░░░░░░░░▀█████████░░░░██████████████████████░░▀█████▀░░░░░░░░░░░   ██        ██  ██      ██  ██  ██
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▀▀███▀▀░░░░░░░░░░░░░░░░░░░░░▀██████▀░░░░░▀███████████▀▀████████░░░░▀▀▀░░░░░░░░░░░░░ ██████  ██████          ██  ██████
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▄▄████▄░░░░░░░░░░░░░░▀▀▀░░░░░░░░░░▀▀▀▀▀▀▀░░░░████████░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▄███████▄░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░████████░░▄▄▄▄▄░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▄▄█████████▄▄█████████▄░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▀██████▀▄███████░░░░░░░░░░░░ focus                         ○○○○
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▄████████████████████████▄░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▄▄░░░░░██████░████████░░░░░░░░░░░░ 25:00
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░█████████████████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░█████░░░▀████░░░▀████▀░░░░░░░░░░░░░ ──────────────────────────────────
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▀▀▀▀▀▀▀▀▀▀░▀███████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░▀███▀░░░░▀██▀░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▀██████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░▄▄▄▄▄▄▄████░░░░░░░░░░░░░░░░░░░███████████▄▄░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▄▄█████▄▄▄░░░░░░░░░░░░░░░░
░░░░░░░░▄▄▄▄▄▄██████████████████▄▄░░░░░░░░░░░░░░░░▄███████████████████▄▄▄▄░░░░░░░░░░░░░░░░░░░▄▄███████████████▄▄▄░░░░░░░░░░░
▄▄██████████████████████████████████▄▄░░░░░░░▄▄▄███████████████████████████████▄▄▄▄▄▄▄▄▄▄▄███████████████████████████▄▄▄▄▄▄▄
████████████████████████████████████████████████████████████████████████████████████████████████████████████████████████████
    ● solid · lava                                                                                    s style  c clock  p palette  m minimal  ␣ pomo  ? help
```

**Ultra-tall — 34×56** (`A ≈ 0.3`). A right panel won't fit, so the
panel goes *below* the lamp, after one blank row.

```
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░▄▄░░░░░░░░░░
░░░░░░░░░░░░░░░░░░▄████████▄░░░░░░
░░░░░░▄▄▄▄▄░░░░░▄████████████░░░░░
░░░░▄███████▄░░███████████████░░░░
░░░▄██████████████████████████░░░░
░░░███████████████████████████░░░░
░░░██████████████████████████░░░░░
░░░▀████████████████████████░░░░░░
░░░░░▀█████▀▀░░▀██████████▀░░░░░░░
░░░░░░░░░░░░░░░░░▀▀███▀▀░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░▄▄█████▄░░░░░░░░░░░░░░░░░░░░░░░
░░▄█████████▄░░░░░░░░░░░░░░░░░░░░░
░░███████████▄▄▄██████▄░░░░░░░░░░░
░░███████████████████████▄░░░░░░░░
░░░▀███████▀▀█████████████▄░░░░░░░
░░░░░░░░░░░░░██████████████░░░░░░░
░░░░░░░░░░░░░█████████████▀░░░░░░░
░░░░░░░░░░░░▄█████████████░░░░░░░░
░░░░░░░░░░▄█████████████▀░░░░░░░░░
░░░░░░░░░░████████████▀░░░░░░░░░░░
░░░░░░░░░████████████▀░░░░░░░░░░░░
░░░░░░░░░▀██████████▀░░░░░░░░░░░░░
░░░░░░░░░░██████████░░░░░░░░░░░░░░
░░░░░░░░░░░████████▄░░░░░░░░░░░░░░
░░░░░░░░░░░██████████▄▄▄░░░░░░░░░░
░░░░░░░░▄▄▄██████████████▄░░░░░░░░
░░░░▄▄█████████████████████▄░░░░░░
░▄▄██████████████████████████▄▄░░░
██████████████████████████████████

 ▄█  ▀▀█ ▄ █ █ █▀▀
  █  ▀▀█ ▄ ▀▀█ █▀█
 ▀▀▀ ▀▀▀     ▀ ▀▀▀

 thu 1 oct


 focus                       ○○○○
 25:00
 ────────────────────────────────
  ● solid        s style  ? help
```

---

## 2. Lamp viewport

### 2.1 The lamp area

The lamp region *is* the tank: no frame, no silhouette, no metal. Heat
source along the bottom row, cooling at the top, the fluid edge to edge.
It takes the whole content area, less the panel when there is one (§1.4),
so a resize only ever changes its width and height (§2.2).

(Before v1.1 there was also a glass lamp silhouette with a cap, a base
and a lighting pass; both were dropped to keep the lamp simple and
uncluttered at every size.)

### 2.2 Proportions: square sim pixels

Terminal cells are about 1:2 (w:h). The renderer never samples one value
per cell and stretches it. Every style samples on a grid of **square
pixels**:

| Style family | Sub-samples per cell | Pixel grid for a cols×rows region |
|---|---|---|
| half-block (solid, ascii, halftone, synthwave, chrome) | 1 × 2 | cols × 2·rows |
| braille (braille, outline, topo) | 2 × 4 | 2·cols × 4·rows |
| cell (matrix) | 1 × 1, sample at the cell centre, aspect-corrected | cols × rows, `y` scaled by `cell_aspect` |

Each style declares its grid (`LampStyle::GRID`). `ascii` supersamples
two pixels per glyph.

The sim lives in **world units**, independent of the terminal:

* World height is always `1.0`. World width is `A_region` (the visual
  aspect of the lamp region).
* Blob radii, velocities and the heat field are in world units. A
  terminal resize changes **sampling density only**, never the physics.
  A blob that's 10 % of lamp height stays 10 % at 30 rows or 120.
* World width follows the region. On resize the walls **ease** to
  the new width over 250 ms (the sim pushes blobs, so nothing teleports),
  and total wax volume is kept at a constant **≈ 30 % of world area** by
  slowly growing/shrinking the bottom pool (no blobs pop in or out).

### 2.3 Cell aspect

`cell_aspect` = from `crossterm::terminal::window_size()` pixel fields
when they're non-zero (`(px_h/rows) / (px_w/cols)`, clamped 1.6–2.6),
otherwise `display.cell_aspect` from config (clamped 1.6–2.6 too),
otherwise **2.0**. Recompute on every resize.

### 2.4 Resolution scaling & budget

* The field is sampled at the style's native pixel grid (table above), up
  to a **budget of 400 k samples/frame**. Above that the renderer samples
  at a reduced grid and bilinearly upsamples the field (not the glyphs).
  That only happens with the 2×4 styles (braille, outline, topo) at huge
  sizes.
* Sampling culls per blob: each blob only touches pixels inside its
  influence box. Cost scales with *blob area*, not blobs × pixels.
* Blob count is set by the world, not the window: a few big, varied blobs
  rather than many equal ones. At the default heat (3) the lamp aims for
  `≈ 2.8 × A_region` blobs, clamped to 3–16. Heat scales the target by
  `1 + 0.4 × (heat − 3)` (×0.6 at heat 1, ×1.4 at heat 5), and the result
  is clamped to 2–40. The pool is a soft mound of the same wax (≈ 0.07 lamp heights
  on average, never below 0.045; heaped about 1.5× in the middle of each
  ≈ 0.9-wide mound, thinner at the walls), glowing hot where it is deep
  and cooler at its skin. It buds mostly off the mound tops, and sooner
  the deeper it gets.
* Blobs are drawn lumpy (a main bump plus slowly orbiting lobes), stretch
  and teardrop along their motion, and join the pool with a skirt that
  draws in to a neck as a bud lets go (`sim/field.rs`). Lobes need
  resolution: a blob under ~2.5 sample pixels in radius draws as one
  round bump, with lobes fading in up to 5 px, so small lamps show round
  droplets, not torn clumps. The pool is drawn at
  least 2 sample rows deep (its mean lifted to that, mounds on top).

---

## 3. Minimal mode

`m` toggles it; `--minimal` / `-m` starts in it; it's persisted in config.
The switch is instant: the next frame shows the new layout, and the sim is
untouched (same blobs, same phase).

* **Just the lamp.** No status bar, no panel, no hints, no borders. The
  lamp fills the screen.
* **Tiny optional clock** (`minimal.clock = "corner" | "off"`, default
  `corner`): the clock chip in the bottom-right corner (the panel's
  widgets have no panel here). A running pomodoro
  replaces it with `▸ 18:24` in the phase colour, even with the clock
  off; a break also says so, `▸ break 4:12`, since phase colours can be
  near twins (and are one colour in 16 / none). The old value `under`
  (under the glass lamp) loads as `corner`.
* Widgets placed on the lava (§4.6) stay: they're part of the lamp's
  picture, not chrome. `t` / `f` cycle them as usual.
* Every key still works. Toasts still appear (that's the only feedback
  minimal mode gives). `?` still opens help, and the pickers still open:
  minimal mode drops the resting chrome, not the overlays.
* Pomodoro phase changes still flash (§4.4).

**Minimal — 80×24:**

```
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░▄▄██████▄▄░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░▄███████████▄░░░░░░░░░░░░░░░░░░░░░░░▄████▄▄░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░██████████████░░░░░░░░░░░░░░░░░░░░██████████░░░░░░░░░░░░░░░░
░░░░▄████▄░░░░░░░░░░██████████████░░░░░░░░░░░░░░░░░░░████████████░░░░░░░░░░░░░░░
░░▄████████░░░░░░░░░█████████████░░░░░░░░░░░░░░░░░░░█████████████░░░░░░░░░░░░░░░
░▄██████████░░░░░░░▄███████▀▀▀▀░░░░░░░░░░░░░░░░░░░░░█████████████░░░░░░░░░░░░░░░
░███████████░░░░░░░███████░░░░░░░░░░░░░░░░░░░░░░░░░░█████████████░░░░░░░░░░░░░░░
░██████████▀░░░░░░░███████░░░░░▄▄▄▄▄▄░░░░░░░░░░░░░░░███████████▀░░░░░░░░░░░░░░░░
░█████████▀░░░░░░░░████████░▄██████████▄▄░░░░░░░░░░░▀████████▀░░░░░░░░░░░░░░░░░░
░░▀██████▀░▄▄▄▄▄░▄███████████████████████▄░░░░░░░░░░░░▀▀▀▀▀▀░░░░░░░░░░░░░░░░░░░░
░░░░▀▀▀▀░████████████████████████████████▀░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░██████████████████░▀██████████▀▀░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░██████████████████░░░░░▀▀▀▀░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░██████████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░█████████████████▀░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░███████████████▀░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░██████████████▄░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▄▄▄░░░░░░░░░░░░░░░░░░░░
░░░░░░▄▄▄▄███████████████████▄▄▄▄░░░░░░░░░░░░░░▄▄▄█████████████████▄▄▄▄▄▄▄▄░░░░░
█████████████████████████████████████████████████████████████████████████ 13:46
```

---

## 4. Chrome: status bar, toasts, help, pickers

### 4.1 Status bar

One row, the last row of the terminal. **It's a whisper, not a bar:** no
background fill, no reverse video, no separators other than spacing.
Inset by the side margin (§1.3) on each side.

```
  ● braille · lava       s style  c clock  p palette  m minimal  ␣ pomo  ? help
  └─ left ────────┘      └─ right: hints ──────────────────────────────────────┘
```

* **Left:** `●` in `accent`, then the style name in `text`, then
  `· palette` in `dim` (only if `cols ≥ 60`). When paused (`z`), `●`
  becomes `‖` and the text says `frozen`.
* **Centre:** empty, unless debug HUD (`d`) is on: `60 fps · 2.1 ms · 412k
  px` in `dim` (the whole readout turns `wax_hot` while adaptive quality
  is active or the frame takes > 80 % of its budget, §7).
* **Right:** hints in `dim`, each formatted `key label` with the key in
  `text`. The full list in display order is `s style  c clock  p palette
  m minimal  ␣ pomo  ? help`. The bar fits as many as possible while
  keeping a gap of at least 4 cols to the left segment. Hints drop in
  this order: `m`, `␣`, `p`, `c`, `s`. `? help`
  always goes last.
* The pomodoro is **not** repeated in the status bar. It lives in the
  panel or chip.

### 4.2 Toasts

* Appear centred in the **top row of the lamp region**, with a 1-cell
  `bg` pad on each side.
* Format: `‹name›  ‹i›/‹n›` for cycling (`braille  4/9`), or a short
  lowercase sentence (`press r again to reset`).
* Last 1.4 s. The final 400 ms fade `text`→`bg` in truecolor. In 256 and
  16 colour they just vanish. A new toast replaces the old one
  immediately; toasts never stack.

### 4.3 Help overlay (`?`)

The form depends on the terminal size (`ui/help/sheet.rs`):

* **≥ 68 × 20: a centred sheet**, `min(66, cols−4)` × `min(22, rows−2)`,
  with a **rounded border in `metal`**. Overlays are the only place
  borders appear. The title `keys` sits in the top border in `accent`,
  and `esc close` in the bottom-right border in `dim`. The sheet always
  has two columns: *lamp* then *clock & pomodoro* | *widgets*, *music · after A*
  (the player keys, §6.2) then *app*, with section
  headers in `dim`, keys in `accent` and labels in `text`. Labels line up
  per column at its widest key + 2; the left column takes its natural
  width (at least half) and a 2-col gutter separates them. The rows come
  straight from the keymap table (§6), so help can't drift from dispatch.
* The lamp keeps animating behind it, dimmed to 35 % (truecolor: lerp
  toward `bg`; 256/16: the sheet's rect is cleared to `bg`, the rest
  isn't dimmed).
* **Smaller (not Micro): a full-screen sheet**, one column, scrollable
  with `j/k/↑/↓`, no border: `keys` (accent) top-left and `esc close`
  (dim) top-right on the first row, the body from the third row. The
  *app* section comes first (`m ? q` lead it), then lamp, clock, widgets,
  music;
  labels line up per section. When keys are cut off, a dim scroll hint
  sits after `keys`: `↓ j/k more` (`↑` at the end, `↕` between),
  shortened to `↓ more` or `↓` to fit.
* **Micro:** the single line `? close · too small for keys` in the top row
  (only help's own keys act while it's open, so it names no others),
  clipped by dropping items from the end.
* `?`, `esc` or `q` closes it. While help is open, `q` closes help and
  does *not* quit.

80×24, captured from the app (`--color none`; the panel stays hidden while
the sheet would touch it, §8.2). Everything fits without scrolling from
80×24 up:

```
       ╭ keys ──────────────────────────────────────────────────────────╮
       │  lamp                          widgets                         │
       │  s    next style               t       clock side/lava/off     │
       │  S    style picker             f       pomodoro side/lava/off  │
       │  p    next palette             a       music side/lava/off     │
       │  P    palette picker           A       music keys              │
       │  [ ]  heat − +                 l       move lava widgets       │
       │  - +  speed                                                    │
       │  z    freeze                   music · after A                 │
       │  0    reset heat & speed       ␣       play / pause            │
       │  R    reseed wax               n p     next · previous         │
       │                                ←→ ↑↓   seek · volume           │
       │  clock & pomodoro              x r     shuffle · repeat        │
       │  c    next face                                                │
       │  C    face picker              app                             │
       │  T    12h / 24h                m       minimal                 │
       │  ␣    start / pause            ?       this help               │
       │  n    skip phase               q       quit · ctrl-c           │
       │  r r  reset pomodoro           b       status bar              │
       │                                d       debug hud               │
       │                                ctrl-l  redraw                  │
       ╰───────────────────────────────────────────────────── esc close ╯
  ● braille · lava      s style  c clock  p palette  m minimal  ␣ pomo  ? help
```

### 4.4 Pickers (`S` style, `C` face, `P` palette)

* **Live preview:** moving the cursor applies the item to the live lamp
  or clock right away. `⏎` keeps it, `esc` reverts to what was active
  when the picker opened.
* **≥ 80 × 16: a sheet**, width 26, height `items + 6` (capped at
  `rows − 2`, scrolls), inset from the side by the side margin (§1.3) and
  vertically centred above the status bar. Rounded `metal` border, title
  (`style`, `clock`, `palette`) in `accent`, cursor `▸` + name in
  `accent`, the item that was active when it opened marked with a dim `·`,
  and a `⏎ keep   esc revert` row. The style and palette pickers anchor
  right. The face picker anchors left when that keeps it clear of the
  panel, so the face it previews stays in view. The lamp stays visible
  and *un*-dimmed, because the point is to watch it change. Chrome the
  sheet would touch (panel, chip) is hidden whole (§8.2).
* **Smaller (Small tier and short windows): a bottom sheet** just above
  the status bar, `items + 3` rows, at most half the height above the
  status bar (at least 3), with no hint row. It spans only the lamp's
  columns when a panel sits to the right of the lamp (and the lamp is
  ≥ 16 cols), otherwise the full width.
* **Tiny / Micro:** an inline selector in the top row, `‹ braille ›`. Use
  `←/→` or `h/l` (also `j/k`). A name too long for the row is cut with
  `…`, the one place text is shortened rather than dropped: the
  selector must show *something* to be usable.
* Keys inside a picker: `↑↓`/`j k` move, `1`–`9` jump, `⏎`/`space` keep,
  `esc`/`q` revert. Pressing the opening key again keeps and closes.
* The status bar's right side switches to picker hints: `↑↓ preview  ⏎
  keep  esc revert`.

80×24, captured from the app (`--color none`):

```




             ⢀⣔⣶⣿⣿⣿⣷⣶⣦⢄⡀                            ╭ style ─────────────────╮
            ⣴⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣦⡀                          │                        │
           ⢸⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣝⡄          ⢀⣠⣴⣶⣶⣶⣖⢤⢄      │   solid                │
           ⠸⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣷⡇        ⢠⣶⣿⣿⣿⣿⣿⣿⣿⣿⣷⣕⢄    │   outline              │
            ⠹⣽⢿⣿⣿⣿⣿⣿⣿⣿⣿⣿⢿⡵⠁       ⣰⣽⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣝⡄   │   ascii                │
              ⠙⠻⠿⣿⣿⣿⣿⣿⠿⠝⠋  ⣠⣤⣤⣤⣤⢤⣼⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡇   │ ▸ braille ·            │
                         ⢠⣾⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡵⠁   │   halftone             │
                         ⣾⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⢿⠕⠁    │   synthwave            │
                   ⢀⣤⣶⣶⣶⣶⣽⣿⣿⣿⣿⣿⣿⣿⣿⣿⢿⣿⢿⣿⣿⣿⣿⢿⡿⠓⠁      │   matrix               │
                 ⢀⣾⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⢿⠁ ⠉⠑⠛⠓⠉⠁         │   topo                 │
                 ⢸⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⠇                 │   chrome               │
                 ⢸⣿⣿⣿⣿⣿⣿⣿⢟⢝⢿⣿⣿⣿⣿⢿⠝                  │                        │
                 ⢸⣿⣿⣿⣿⣿⣿⣿⣷⣷⣝⢝⣝⠿⠝⠁                   │ ⏎ keep   esc revert    │
                 ⠈⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡇                       │                        │
                  ⢹⣿⣿⣿⣿⣿⣿⣿⣿⣿⠁                       ╰────────────────────────╯
                   ⠻⣿⣿⣿⣿⣿⣿⣿⣿⣀⣀⣀⣀⣀⣀⣀⣀
                ⣀⣀⣠⣾⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣷⣶⣤⣀⡀
 ⣀⣀⣀⣀⣀⣠⣤⣴⣶⣶⣾⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣶⣦⣤⣀⣀
⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣶⣶⣤⣤⣤
  ● braille · lava                              ↑↓ preview  ⏎ keep  esc revert
```

### 4.5 Panel contents

From top to bottom, left-aligned in a block that is vertically centred on
the lamp:

```
 ▄█  █ █ ▄ ▀▀█ ▀▀█        ← clock face (variant by space)
  █  ▀▀█ ▄ ▀▀█ █▀▀
 ▀▀▀   ▀   ▀▀▀ ▀▀▀
                          ← (date line: thu 1 oct — Large+)

 focus             ●●○○   ← phase label (dim) · cycle dots (accent / dim)
 18:24                    ← remaining (phase colour, see below)
 ━━━━━━━━──────────────   ← progress: ━ phase colour, ─ dim
```

The phase label is `focus`, `break` or `long break`. The time and the
filled bar take the phase colour: `accent` while a focus phase runs,
`wax_hot` while a break runs, `text` while paused (with a dim ` paused`
after the time), and `dim` when idle. Idle shows `focus  25:00` with an
empty bar. The pomodoro is always present in the panel, so it's always
discoverable.

**Face variants.** Every face declares variants with fixed sizes, and the
panel uses the largest one that fits its inner rect. Anything that can't
fit falls back to `text` (`14:32`, 5×1), which always fits.

Sizes are the real ones from `src/clock` (24h, no seconds; seconds add
width only in L/XL, where the panel allows them):

| Face | S | M | L | XL |
|---|---|---|---|---|
| `blocks` (default) | — | 3×5 font, half-blocks: 17×3 | ×2: 34×5 (with seconds 54×5) | ×3: 51×8 |
| `segment` | — | 17×3 | 21×5 (with seconds 33×5) | 33×7 |
| `analog` | — | 15×8 | 23×12 | 31×16 (circle aspect-corrected) |
| `binary` | 9×4 | 12×6 | — | — |
| `words` | 16×3 | 24×2 | 21×10 (word grid) | — |
| `text` | 5×1 (`14:32`; 12h ` 2:32 pm` 8×1) | — | — | — |

Forms wider than 34 cols (blocks XL, blocks L with seconds, segment XL
with seconds 52) only show from 200 cols, where the panel widens for them
(§1.4).

The colon never blinks (motion belongs to the lamp). Seconds appear only
in L/XL variants and the `text` face's 12h/24h follows `T`.

**Phase-change flash.** When a pomodoro phase ends on its own: the
lamp's liquid tint pulses, peaking at 35 % toward `accent`, one `sin` swell over 600 ms; a
toast says `break · 5:00`, and the terminal bell sounds if
`pomodoro.bell = true` (default true). Skipping a phase with `n` only
toasts: no flash, no bell.

---

### 4.6 The widget dock

The clock, the pomodoro and music are *widgets* (`src/dock/`). Each has
a place, persisted as `dock.<name>`, cycled by its key: `t` the clock,
`f` the pomodoro (`focus`), `a` music (`audio`), each `side → overlay →
off → side`. A toast says
where it went (`clock · on the lava`), adding `· no room` when the
layout couldn't fit it there (it's in the chip meanwhile) and `· not in
minimal` for `side` in minimal mode.

* **side**: stacked in the panel (§1.4, §4.5), 2 rows apart, in
  registry order (clock, pomodoro, music). The default for the clock and
  the pomodoro, so the default screen is the v1 panel, cell for cell;
  music is `off` by default.
* **overlay**: stacked on the lava, 1 row apart, at the anchor
  (`dock.anchor`; `l` moves it: centre → top → top right → bottom
  right → bottom → bottom left → top left). Widgets line up by the
  anchor: centred, or flush left / right at the sides. Limits, so the
  lamp stays the hero: the lamp must be ≥ 28 × 10; the stack ≤ 60 % of
  the lamp's width and ≤ half its height; with its backing ≤ 35 % of the
  lamp's area; inset (≥ 5 cols / 3 rows, more on big lamps) so the
  backing never reaches the toast row or the chip's row. Forms shrink in
  the hide order first; when not even the smallest fit, the widgets go to
  the chip. Seconds are never shown on the lava (they'd be the one thing
  ticking over the wax); the date line comes along in tall terminals.
* **off**: not drawn (a running pomodoro still toasts and flashes).

**The backing.** Over the lava the widgets sit on a soft pool of liquid,
not a box: under the stack and half a row around it the lamp is veiled
82 % of the way to `liquid` (its glyphs cleared, so text never sits on
wax glyphs), and the veil fades to nothing over the next 1½ rows (a
column counts half a row), within 4 cols / 2 rows of the stack. Wax
drifting behind shows as a faint ghost and melts out at the edges. This
was picked from pty captures of all nine styles and eight palettes
against three alternatives: no backing (unreadable over ascii, matrix,
braille), a halo following the glyphs (busy around the pomodoro's short
lines) and per-row spans (ragged edges). In 256 colours the veil would
snap to cube greys (a grey box), so below truecolor the backing is plain
`liquid` wherever it's at least half strength: a crisp, slightly rounded
cutout, exact to the liquid's index.

Chrome rules still hold: an overlay sheet (help, picker) touching the
stack's backing hides the whole stack (§8.2); the face picker sits on
whichever side keeps the panel and the stack clear when it can.

A chip is only offered when it fits the lamp's width (with its pads); a
wider one (a long track name in a tiny terminal) passes to the next
widget in rank.

#### Music (now playing)

`src/dock/music.rs`, fed by `src/media/` (the player) and
`src/media/art.rs` (covers). Off by default. While it's placed (side or
on the lava) the app holds a media source, whose worker thread polls the
player (Spotify through AppleScript on macOS; MPRIS / SMTC to come); `off`
drops it, which stops the polling. The UI only ever reads the source's
latest snapshot (a short lock) once a frame, and commands are queued and
shown at once (optimistically), so the player never stalls a frame.

Forms, most preferred first (the layout keeps the first that fits; music
is last in the registry, so it shrinks first):

| form | size | shows |
|---|---|---|
| cover on top | A × (A/2 + 7), A = 32, 24 | the cover (half-block pixels, A px square), a blank row, the card |
| cover beside | 32 × 6 | a 12-col cover, 2 cols, the card |
| cover on top | A = 20, 16 | as above |
| card | ≥ 20 × 6 | title (`text`), artist, album (`dim`), a blank row, bar, status line |
| compact | ≥ 20 × 3 | title, artist, `▶ 1:23 ━━━─── 3:45` |
| line | ≤ 36 × 1 | `▶ title – artist` |

```
 ████████████████        ← the cover, A × A/2 cells (A × A px)
 ████████████████
 ████████████████

 Voices (From "The Be…   ← title, cut with … to the card's width
 Dario G                 ← artist (dim)
 Sunmachine              ← album (dim)

 ━━━━━━━━━━━━──────────  ← elapsed in text (dim while paused), rest dim
 ▶ 3:28     vol 68  5:19 ← play state + elapsed · ⇄ ↻ vol (dim) · total (dim)
```

The status line drops shuffle/repeat, then the volume, then the total
before it would crowd the elapsed time. `▶` playing, `‖` paused; the
glyph turns `accent` while the player keys are on (the one sign of the
mode besides the status bar's hints). Shuffle `⇄` and repeat `↻` show
only for players that can change them (`MediaSource::capabilities`):
Spotify's AppleScript can't (its setters are no-ops, lava-75z.9), so for
it they're neither shown nor offered. On the lava the forms line up by the
anchor (centred lines under a centred cover).

**Covers.** Fetched on a background thread when the track changes
(`https` only, ≤ 8 MB), kept on disk in `$XDG_CACHE_HOME/lavatui/art`
(else the platform cache dir; 256 newest kept), decoded (JPEG / PNG),
cropped square and shrunk to 64 px, then box-filtered to the cover's
cells each frame. Pixels go through `Theme::image`: exact in truecolor,
the nearest xterm index in 256 colours, and no cover at all in 16
colours or `NO_COLOR` (the cover forms aren't offered). Until the cover
has arrived (or if it can't be had) a quiet placeholder holds its place
(`bg` tinted 18 % toward `dim`, a dim `♪` in the middle), so nothing jumps
when it lands. On the lava the cover is opaque; the soft backing frames
it like the text.

**Without a player** the widget is one calm, dim sentence, wrapped at 20
cols beside the lamp and 30 on the lava, and it has no chip: `♪ Spotify
isn't running`, `♪ Spotify isn't installed`, the Automation permission
path (`♪ Allow control of Spotify: System Settings › Privacy & Security ›
Automation › your terminal › Spotify`), `♪ No media player support on
this platform yet`, `♪ nothing playing` (running, nothing loaded), `♪ …`
for the moment before the first answer.

**Chip:** `▶ title – artist` (≤ 32 cols, cut with `…`), rank 2 while
playing (above the clock; ties go to the pomodoro, earlier in the
registry), `‖ …` rank 1 while paused, none otherwise.

**Frozen lamp:** while music is placed, the idle loop looks at the player
at least once a second, so a track change or a pause made in Spotify
shows within a second.

---

## 5. Palettes / lamp themes

### 5.1 Roles

Every palette defines exactly these nine roles. Nothing outside `ui/`
and `render/` hard-codes a colour.

| Role | Used for |
|---|---|
| `bg` | app background: behind the chrome, around the panel |
| `liquid` | the lamp's fluid, behind the wax |
| `wax_cool` `wax_mid` `wax_hot` | 3-stop temperature gradient (cool → hot). Single-colour styles use `wax_mid`, or lerp by temperature |
| `metal` | overlay borders |
| `text` | primary text |
| `dim` | secondary text, hints, idle states |
| `accent` | **the one accent**: selection cursor, running pomodoro, `●`, toast keys |

### 5.2 The palettes

Default: **lava**. Names are lowercase in the UI. 256 = xterm index
(hand-picked; an unmixed role always uses it, blends are matched as in
§5.3). 16 =
ratatui `Color` name. `default` = terminal default (`Color::Reset`).

**lava**: the 1970s original. Red-orange wax in amber oil.

| role | hex | 256 | 16 |
|---|---|---|---|
| bg | `#0F0B0A` | 232 | default |
| liquid | `#23160C` | 233 | default |
| wax_cool | `#8E1B12` | 88 | Red |
| wax_mid | `#E2471B` | 166 | LightRed |
| wax_hot | `#FFB04A` | 215 | LightYellow |
| metal | `#6B5A4E` | 240 | DarkGray |
| text | `#E9DCCF` | 253 | default |
| dim | `#7D6E62` | 242 | DarkGray |
| accent | `#FF8A3D` | 209 | Yellow |

**ultraviolet**: blacklight poster. Violet to hot pink in deep indigo.

| role | hex | 256 | 16 |
|---|---|---|---|
| bg | `#0B0816` | 233 | default |
| liquid | `#170F2C` | 234 | default |
| wax_cool | `#4B1D8F` | 54 | Blue |
| wax_mid | `#B5179E` | 127 | Magenta |
| wax_hot | `#FF7AD9` | 212 | LightMagenta |
| metal | `#4A4166` | 239 | DarkGray |
| text | `#E4DDF5` | 254 | default |
| dim | `#776E93` | 60 | DarkGray |
| accent | `#A78BFA` | 141 | LightBlue |

**abyss**: deep sea. Teal wax glowing to seafoam in navy water.

| role | hex | 256 | 16 |
|---|---|---|---|
| bg | `#060B10` | 232 | default |
| liquid | `#0B1A24` | 234 | default |
| wax_cool | `#0B4F6C` | 24 | Blue |
| wax_mid | `#1A9BA8` | 31 | Cyan |
| wax_hot | `#A8F5E4` | 158 | LightCyan |
| metal | `#34495A` | 238 | DarkGray |
| text | `#D6E7EE` | 254 | default |
| dim | `#5F7785` | 67 | DarkGray |
| accent | `#4FD1C5` | 80 | LightCyan |

**toxic**: radioactive slime. Moss to acid yellow-green.

| role | hex | 256 | 16 |
|---|---|---|---|
| bg | `#080A06` | 232 | default |
| liquid | `#121A0B` | 233 | default |
| wax_cool | `#2F6B1F` | 22 | Green |
| wax_mid | `#7FBF2A` | 106 | LightGreen |
| wax_hot | `#E8FF6A` | 191 | LightYellow |
| metal | `#3E4A33` | 238 | DarkGray |
| text | `#E2ECD5` | 254 | default |
| dim | `#6C7A5E` | 65 | DarkGray |
| accent | `#C6FF3D` | 154 | LightGreen |

**synthwave**: 1986 sunset. Hot pink → coral → gold, with a cyan accent
as the counterpoint.

| role | hex | 256 | 16 |
|---|---|---|---|
| bg | `#0E0718` | 233 | default |
| liquid | `#1B0B2B` | 234 | default |
| wax_cool | `#FF2E88` | 198 | Magenta |
| wax_mid | `#FF7A59` | 209 | LightRed |
| wax_hot | `#FFD66B` | 221 | LightYellow |
| metal | `#4B3264` | 238 | DarkGray |
| text | `#F4E6FF` | 255 | default |
| dim | `#8A73A3` | 97 | DarkGray |
| accent | `#2DE2E6` | 44 | LightCyan |

**mono**: graphite. Grayscale only. It suits halftone/braille and
e-ink moods.

| role | hex | 256 | 16 |
|---|---|---|---|
| bg | `#0C0C0C` | 232 | default |
| liquid | `#161616` | 233 | default |
| wax_cool | `#3D3D3D` | 237 | DarkGray |
| wax_mid | `#9A9A9A` | 247 | Gray |
| wax_hot | `#F0F0F0` | 255 | White |
| metal | `#3A3A3A` | 236 | DarkGray |
| text | `#E0E0E0` | 254 | default |
| dim | `#6E6E6E` | 242 | DarkGray |
| accent | `#FFFFFF` | 231 | White |

**paper**: the light theme, for light terminals. Rust-red ink in cream,
with an ink-blue accent.

| role | hex | 256 | 16 |
|---|---|---|---|
| bg | `#F3EEE3` | 255 | default |
| liquid | `#E7DECB` | 253 | default |
| wax_cool | `#7A2617` | 88 | Red |
| wax_mid | `#C24D2C` | 130 | LightRed |
| wax_hot | `#F08A3C` | 209 | Yellow |
| metal | `#A8997E` | 137 | Gray |
| text | `#3B342C` | 236 | default |
| dim | `#8C8173` | 244 | Gray |
| accent | `#1F6F8B` | 24 | Blue |

**ansi**: uses the terminal's own 16-colour theme in every colour mode
(no hex). It's for people whose terminal theme *is* their aesthetic.
Every role uses the 16-colour column of **lava**, and `bg`/`liquid` are
always `default`, so it respects transparency.

### 5.3 Colour depth & fallbacks

Detection order, overridable with `--color=auto|truecolor|256|16|none` /
`display.color`:

1. `NO_COLOR` set (non-empty) → **none**
2. `COLORTERM` ∈ {`truecolor`, `24bit`} → **truecolor**
3. `TERM` contains `256color` → **256**
4. otherwise → **16**

| Depth | Gradient | Background | Notes |
|---|---|---|---|
| truecolor | lerp across the 3 wax stops (a 64-step ramp LUT at every depth); blended colours are rounded to multiples of 4 per channel, so sub-visible drift doesn't repaint cells | `bg` painted (unless `theme.transparent = true`) | fades, dimming, glow all on |
| 256 | blend in RGB, then match to the nearest xterm index by a hue- and lightness-weighted OKLab distance over the 6×6×6 cube and grey ramp only (the 16 system colours are themed by the terminal, so never picked); cached per 6-bit RGB bucket. **Hue guard:** a chromatic index more than 30° off the input's hue is never picked, alone or as a dither end (greys always may be), so dark orange never goes olive and brown is never dithered from red and green dots. Dark tints the cube lacks (colours that lose their hue when snapped to one index) are ordered-dithered between the two best indices with the 8×8 Bayer matrix, fixed to the lamp in screen space (`render/dither256.rs`, `Theme::dithering`). Unmixed roles use the §5.2 index | `bg` painted (index above) | toast fade → instant; help dim → cleared rect |
| 16 | 3 discrete steps; styles add glyph density (`░▒▓█`) to show temperature | always `default` | |
| none | no colour at all; temperature shown only through glyph density and shape | `default` | `accent` → bold; `dim` → plain |

Every style must stay legible in **16** and **none**. That's a snapshot
test requirement for `lava-bdj` / `lava-y7g`.

---

## 6. Keymap

Single keys only (no chords, no leader). Lowercase means *do the common
thing*. **Shift means open the picker / bigger version of the same
thing.** Every binding appears in help (§4.3). The keymap lives in one
table in `ui/keymap.rs` that drives both dispatch and the help overlay,
so they can't drift.

### 6.1 Global

| Key | Action | Notes |
|---|---|---|
| `?` | toggle help | |
| `q` | quit | closes the overlay instead when one is open |
| `ctrl-c` | quit | always, from anywhere |
| `esc` | close overlay / cancel picker | no-op otherwise: **esc never quits** (esc is muscle memory for "close this"; an accidental quit loses pomodoro state) |
| `m` | minimal mode on/off | also `--minimal` / `-m` |
| `b` | status bar on/off | full mode only (minimal toasts `no status bar in minimal · m to leave`) |
| `s` / `S` | next style / style picker | toast shows `name  i/n` |
| `c` / `C` | next clock face / face picker | |
| `p` / `P` | next palette / palette picker | |
| `t` | clock: side → on the lava → off | §4.6; toast `clock · on the lava` |
| `f` | pomodoro: side → on the lava → off | §4.6 |
| `a` | music: side → on the lava → off | §4.6; off by default |
| `A` | player keys on (§6.2) | toast `music keys · esc when done`; with music off: `music is off · a to show it` |
| `l` | move the widgets on the lava | centre → top → top right → … → top left |
| `T` | 12h / 24h | |
| `space` | pomodoro start / pause / resume | starts a focus phase if idle |
| `n` | pomodoro: skip to next phase | idle: toasts `pomodoro idle · ␣ to start` |
| `r` | pomodoro reset (press **twice** within 2 s) | first press toasts `press r again to reset`; presses < 150 ms apart count as key repeat, never as the second press |
| `[` / `]` | heat − / + (5 steps, default middle) | more heat = more, faster blobs; toast shows `heat ▮▮▮▯▯` |
| `-` / `+` (`=`) | sim speed ×0.25 · ×0.5 · ×1 · ×2 · ×4 | toast `speed ×2` |
| `0` | reset heat and speed | |
| `z` | freeze / unfreeze the lamp | frozen = zero sim cost; the clock keeps ticking |
| `R` | reseed the wax (new random seed) | toast `reseeding`; blobs melt into the pool (under 2 s), then 5 s of fast budding refill the lamp. Never a hard cut |
| `d` | debug HUD (fps, frame ms, samples) | |
| `ctrl-l` | force full redraw | |

### 6.2 In overlays

| Context | Keys |
|---|---|
| help | `j k ↑ ↓` scroll · `?` `esc` `q` close |
| picker | `j k ↑ ↓` move (live preview) · `1`–`9` jump · `⏎` `space` keep · `esc` `q` revert · opening key = keep + close |
| tiny inline picker | `h l ← →` (also `j k`) move · `⏎` keep · `esc` revert |
| player keys (`A`) | `␣` play / pause · `n` `p` next / previous · `← →` (`h l`) seek ∓ 10 s · `↑ ↓` (`k j`, `+ -`) volume ± 5 · `x` `r` shuffle / repeat (where the player can) · `esc` `q` `A` done · `?` help (ends them) |

**The player keys** are a mode, like an overlay without a sheet: `A`
turns them on and they take the keyboard until `esc`, `q` or `A`. That
keeps one global key for the whole player (instead of nine more single
keys in an already full map) and lets it reuse the obvious letters
(`␣`, `n`, `p`, arrows) that the pomodoro and lamp own outside it. The
status bar's hints become `␣ play  n p skip  ←→ seek  ↑↓ volume  esc
done` and the widget's play glyph turns `accent`. Volume toasts its new
value (`volume 65`); with no player, any key toasts why (`Spotify isn't
running`). Leaving music `off` ends the mode.

Every key not listed is ignored (no beep, no toast). Overlay keys take
precedence over global keys; global keys other than `ctrl-c` don't fire
while an overlay (or the player keys) is open.

### 6.3 Mouse

`input.mouse = false` by default, because mouse capture breaks the
terminal's native text selection. When it's on: click/drag on the lamp
= a local heat pulse (the wax there warms and rises), scroll in pickers
and help, click a picker item to preview, double-click to keep.

---

## 7. Motion & performance

| Target | Value |
|---|---|
| Sim timestep | fixed `SIM_HZ` (**120 Hz** in the scaffold, `timing::FixedStep`; sim time = real time × speed at any fps, only a > 1.5 s stall is cut short), decoupled from render; render interpolates with `alpha()` |
| Render rate | default **60 fps** (`--fps 1..=240`, `display.fps`). Lava is slow, but 60 fps keeps input feeling instant and makes the slow motion buttery |
| Wax tempo (×1, heat 3) | a blob takes **~20–40 s** to cross the lamp: slow, hypnotic, never jittery |
| Startup → first frame | **< 100 ms**. The sim starts *pre-warmed*: ~600 headless steps at launch, so frame 1 already looks alive (no 2-hour warm-up) |
| Input latency | key → visible change **≤ 1 frame** (≤ 17 ms at 60 fps). The loop blocks on `event::poll(time_to_next_frame)`; any input that changes UI state triggers an immediate redraw, without waiting for the tick |
| Frame CPU (release, 2020-era laptop) | ≤ **2 ms** at 80×24; ≤ **8 ms** at 200×60 with braille (≤ 50 % of a 60 fps budget) |
| CPU usage | ≤ **5 %** of a core at 80×24, ≤ **15 %** at 200×60 @ 60 fps |
| Output bandwidth | rely on ratatui's cell diff; ≤ ~200 KB/s at 80×24 (SSH-friendly) |
| Unfocused | on `FocusLost` (if the terminal reports it), drop to **10 fps**; back to normal on `FocusGained` |
| Frozen (`z`) | no sim steps; the loop sleeps until the clock readout changes (each minute, or each second while a face shows seconds or a pomodoro runs), input arrives or a save is due; toasts and flashes still animate |

**Resize behaviour.**

* Handle `Event::Resize` at once: recompute `layout()`, re-derive
  `cell_aspect`, and draw the new geometry on the **next frame**. Never
  draw a frame with stale geometry, and do one full clear + repaint.
* The sim's walls ease to the new width over 250 ms (§2.2).
* Coalesce resize storms: when multiple resize events arrive in one poll
  batch, only the last one counts.

**Adaptive quality** (silent; it never changes the user's choices):

1. If the moving-average frame time (EMA, τ = 0.5 s) is > 80 % of the
   frame budget for 2 s, the sampling grid drops one step (e.g. braille
   samples at half resolution and upsamples).
2. Still over budget: fps halves (60 → 30, 120 → 60), never below 30
   from adaptation alone (45 → 30). At ≤ 30 fps this step doesn't exist;
   the grid step is all there is.
3. Recovers one step at a time once the frame time is < 40 % of the
   budget *of the level it would return to* for 5 s, so it never comes
   back into a level it would immediately leave.
4. Backoff: if quality is lost again soon after a recovery, the next
   recovery waits twice as long (up to 5 min), so a borderline load never
   flaps. A workload change (resize, style, grid) resets the wait.
   Frames aren't measured while frozen.

The debug HUD shows when this is active (the whole readout turns
`wax_hot`, §4.1).

**Measured** (lava-ebq.35, `main` before v1, so with the glass frame since
removed; Apple M5 under background load 3–5, real pty runs at 60 fps):
launch → first frame ≈ 30 ms; ≈ 2.3 % of a core at 80×24 and 4.6–4.9 %
at 200×60 (solid/braille); output ≈ 10 KB/s at 80×24 and ≈ 55 KB/s at
200×60 in solid, ≈ 5 / 21 KB/s in braille. Lamp render on v1.1
(`bench_lamp`): 0.02–0.14 ms per frame at 80×24 and 0.12–0.76 ms at
200×60 for every style, truecolor or 256 colours. All within the targets
above. Details in the README.

**Music** (lava-75z.2, same machine, 10 s pty runs at 60 fps, min of 3,
Spotify playing): drawing the widget, cover included, is too small to
see; the cost is the Spotify backend's `osascript` poll, once a second
while playing (≈ 50–90 ms of CPU per run): 80×24 solid 2.5 % of a core
with music off → 11.1 % beside the lamp or on the lava; 200×50 8.1 % →
16.7 %. That is over the 5 % target at 80×24; cheaper polling is
lava-75z.11. Output is unchanged (64 KB/s at 80×24 either way; a little
less on the lava, which covers wax).

Speed changes (`-`/`+`/`0`) ease in over a fraction of a second rather
than jumping.

---

## 8. Visual design principles

1. **The lamp is the hero; everything else whispers.** Chrome uses `dim`
   and `text` only, with no fills, bars or boxes in the resting state.
2. **Hide before you cram.** Elements drop out whole, in the fixed
   priority order (§1.3). Nothing truncates mid-word, wraps or overlaps.
   Overlays follow the same rule: any panel or chip an overlay (help,
   picker or toast) would touch, with a 1-cell gap, is left out whole
   rather than drawn under or beside it, and so is a status bar, toast or
   HUD it would overlap. The HUD also hides under a toast. The one
   shortened text is the Tiny inline picker's name (§4.4).
3. **One accent colour**, used sparingly: cursor, running pomodoro,
   status `●`, help keys. If two things are accented, one of them
   shouldn't be.
4. **No borders at rest.** Separate things with whitespace. Rounded
   borders appear only on transient overlays (help, pickers).
5. **Only the lamp moves.** No spinners, no blinking colon, no animated
   chrome. The exceptions are the pomodoro bar's progress, toast fades and
   the phase-change flash, all of which carry information.
6. **Proportional compositions.** The panel and margins scale with the
   window. Odd leftover cells always go right/bottom, so nothing
   jitters.
7. **Lowercase, terse labels**: `braille`, `ultraviolet`, `focus`,
   `esc close`. No title case, no exclamation marks, no emoji.
8. **Plain Unicode only.** Block elements, box drawing, braille and
   geometric shapes that ship in every common monospace font. No Nerd
   Font glyphs required.
9. **Colour degrades, structure doesn't.** Every screen reads correctly
   in 16 colours and in NO_COLOR.
10. **Two text weights:** normal and `dim`. Bold appears only in the
    NO_COLOR fallback for accent. No italics, no underline.

---

## 9. Config surface implied by this doc

```toml
[display]
fps = 60                 # 1..=240
color = "auto"           # auto | truecolor | 256 | 16 | none
cell_aspect = 2.0        # used only when the terminal doesn't report pixels

[lamp]
style = "solid"
heat = 3                 # 1..5
speed = 1.0              # 0.25 | 0.5 | 1 | 2 | 4

[theme]
palette = "lava"
transparent = false      # true = never paint bg (the terminal's own shows)

[clock]
face = "blocks"
hour24 = true

[pomodoro]
focus_min = 25
short_break_min = 5
long_break_min = 15
cycles = 4
bell = true

[ui]
mode = "full"            # full | minimal
status_bar = true

[minimal]
clock = "corner"         # corner | off

[input]
mouse = false

[dock]
anchor = "center"        # center | top | top-right | bottom-right | bottom | bottom-left | top-left
clock = "side"           # side | overlay | off
pomodoro = "side"        # one key per widget in the registry
music = "off"

[spotify]
client_id = ""           # Web API library features (docs/spotify.md); "" = off
```

Out-of-range values are clamped rather than rejected: `fps` 1–240,
`cell_aspect` 1.6–2.6 (NaN → 2.0), `heat` 1–5, `speed` snapped to the
nearest step (≤ 0 or non-finite → 1), pomodoro minutes 1–1440, `cycles`
1–12, `spotify.client_id` trimmed (anything but letters and digits → `""`).
An empty `client_id` falls back to `LAVATUI_SPOTIFY_CLIENT_ID`. The old style name `glass` is accepted as `chrome`.

Retired in v1.1, and quietly ignored in an old file (no toast; the next
save takes them out): `lamp.frame`, `lamp.lighting`, `clock.show`
(`show = false` loads as `dock.clock = "off"` unless the file sets
`dock.clock`; the next save writes that). A `dock.<name>` for a widget
this build doesn't have is an unknown key, kept in the file. A file naming a
removed style (`heatmap`, `dither`, `crt`) gets `solid`, also without a
toast; on the command line those names are unknown, like any other.

CLI: `--minimal`/`-m`, `--fps <n>`, `--style <name>`, `--palette <name>`,
`--color <depth>`, `--seed <u64>`, `--config <path>` (use this file
instead of the XDG one), plus hidden `--frames <n>` (exit after n frames)
and `--panic-after <n>` (tests the terminal-restoring panic hook). Flags
override config for the session only. They're never written back (until
you change that setting in the app, which then saves as usual). An
unknown `--style` or `--palette` name is a usage error like any other
bad flag: exit code 2 and the list of valid names.

The file lives at `$XDG_CONFIG_HOME/lavatui/config.toml`, else the
platform config dir (`~/.config/lavatui/` on Linux, `~/Library/Application
Support/lavatui/` on macOS). It is saved 1 s after the last change and on
quit. A missing file means defaults. A bad value, including a style,
palette or face name that doesn't exist, is ignored (with a toast,
`config: ignored lamp.heat`) and the rest kept. A key that isn't a
setting is reported (`config: unknown key lamp.future_key`) but left in
the file; it needs no backup. A TOML syntax
error means all defaults (`config unreadable · using defaults`); a file
that can't be read at all (permissions, a directory) is never written
that session. Anything a save would drop is first copied to
`config.toml.bak`. Saves keep comments, key order and unknown keys,
follow symlinks to the real file, and are atomic (temp file + rename,
keeping the file's permissions).

---

## 10. Out of scope for v1 (ideas, not commitments)

* Kitty/sixel graphics backend for true-pixel wax.
* Ambient mode: auto-cycle styles/palettes every N minutes.
