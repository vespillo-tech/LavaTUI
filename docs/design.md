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
| **on the lava** | The widgets placed `overlay`, stacked over the lamp at their anchors, floating with no background (or on a soft backing, `dock.backing`; §4.6). |
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
| **Panel** | some widget is placed `side` and an arrangement (§1.4) fits at least the most important one | column, strip, wrap or two columns (§1.4); forms by rank |
| **Widgets on the lava** | some widget is placed `overlay`, not Micro, the lamp is ≥ 28 × 10, and the most important one's smallest form fits (§4.6) | one stack per anchor; each ≤ 60 % of the lamp's width and half its height, all backings together ≤ 35 % of its area, never touching |
| **Chip row** | widgets with no room where they were put (`side` with no panel, dropped from the panel or the lava), `cols ≥ 20 && rows ≥ 8` | their chips in registry order, ` · ` apart, the lowest-ranked left out until the row fits the lamp's width (§4.6) |
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
4. Widget size (clock face XL → L → M → S → `text`, the pomodoro's
   3 → 2 → 1 rows, music's cover → card → line). With several widgets in
   one place, the lowest-ranked shrinks first (§4.6: an idle pomodoro
   0 < the clock 1 < playing music 2 < a running pomodoro 3; ties: the
   later in the registry).
5. Widgets, one at a time, lowest rank first, from the panel and the
   lava into the chip row (nothing is lost but size)
6. Status bar
7. Chips, lowest rank first
8. ~~Lamp~~ — never

### 1.4 Panel placement algorithm

```
panel_w   = clamp(round(cols × 0.30), 22, 36)      // incl. 1-col inner padding each side
            // cols ≥ 200: widened to face_w + 2 (max 56) when the face needs it
panel_h   = face_h + (date? 2) + 2 + 3             // face, gap, label/time/bar
                                                   // (each side widget's height,
                                                   // 2 rows between; pomodoro alone: 3)

1. A ≥ 1.0 → column: right panel, flush with the right edge and
   vertically centred, if the lamp keeps ≥ 60 % of cols and ≥ 24 cols.
2. A < 1.0 → column: bottom panel (min(36, content_cols) wide, centred,
   one blank row below the lamp) if the lamp keeps ≥ 60 % of rows and
   ≥ 10 rows.
3. Otherwise no panel → chip row.
```

**Flow (Dock v2).** The column is not the only arrangement any more.
Each candidate is fitted the same way (the odometer of §1.3: forms by
rank, dropping the least important widget only when not even the
smallest forms fit) and scored by (widgets dropped, then each widget's
form by rank: larger is better). The best wins; **ties go to the
column**, so wherever the column already shows everything at its best
nothing changes (the default 80 × 24, 120 × 36, 34 × 56 … are cell for
cell as before).

| arrangement | when it's a candidate | shape |
|---|---|---|
| column | always (right if A ≥ 1, below if A < 1) | as above |
| strip | wide-short: A ≥ 3 | the widgets side by side under the lamp (registry order, 3 cols apart, centred in their row's height), wrapping into rows 1 apart if needed, the block centred, one blank row above; the lamp keeps ≥ 60 % of rows and ≥ 10 |
| two columns | `cols ≥ 200`, A ≥ 1 | right of the lamp, split in registry order where the taller column is shortest, 3 cols apart; wins ties too when the single column would be taller than half the screen |
| wrap | portrait (A < 1) | rows across the whole width under the lamp, rows 2 apart; a fill form alone in its row takes up to 34 cols |

So 160 × 22 (A ≈ 3.8) now puts the clock (blocks L with seconds, which
the 36-col column couldn't hold) and the pomodoro side by side in a
strip under a full-width lamp; 250 × 70 with music on goes to two
columns (clock + pomodoro | music), a single column being taller than
half the screen; a cramped portrait (70 × 40 with music) wraps the
pomodoro and music into one row instead of shrinking music to its
compact form. Cost (`bench_layout`, release, a busy machine): ~1 µs a
frame by default; with four widgets ~20 µs on average, up to ~170 µs in
cramped sizes where little fits and every combination is tried (the
lyrics' song-sized forms, lava-uqi, took four beside the lamp from ~45
to ~140 µs there), under 1 % of a 60 fps frame.

Below 200 cols the panel's inner width is at most 36 − 2 = 34. From 200
cols up the panel grows only as far as the face it holds needs, up to
56 (inner 54): blocks XL (51 × 8) shows at Huge, blocks L with seconds
(54 × 5) when the terminal is ≥ 200 cols but under 56 rows. The
lyrics' one-row form grows it the same way (§4.6 *Lyrics*). Narrower
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
  ● solid · lava           s style  c clock  p colours  , settings  ? help
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
  ● solid · lava        s style  c clock  p colours  m lamp only  Space timer  , settings  ? help
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
    ● solid · lava                                            s style  c clock  p colours  m lamp only  Space timer  , settings  ? help
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
    ● solid · lava                                                                                    s style  c clock  p colours  m lamp only  Space timer  , settings  ? help
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

**Half-block cells are opacity-safe.** Some terminals (Ghostty with
`background-opacity < 1` and `background-opacity-cells = true`) blend a
cell's *background* over the window but keep its *glyph* opaque, so a
`▀`/`▄` cell whose halves are two shades of wax shows its background half
darker: half-row dashes. So (`render::cell::half_block`, always):

* two halves that look the same (OKLab ΔE ≤ 0.03, `theme::NEAR`) are drawn
  as one colour: `█` for wax, a space for the liquid or a style's
  backdrop (each pixel moves < ½ NEAR: invisible on opaque terminals);
* the liquid / backdrop is always the cell background, so it's as
  see-through as the rest of the window, and a split cell puts the
  liquid's half (else the darker half) behind.

With `display.cells = "translucent"` (or `"auto"` in a native Ghostty
configured as above; not in hosts that embed its terminal, such as
Ghostex, which ignore its config) two wax halves never split: they become their mean, giving up
colour detail *inside* the wax (never the silhouette) for no seams; the
256-colour dither then works a cell at a time.

**Block glyphs short of the cell.** macOS Terminal draws `█ ▀ ▄` from
the font into the bottom ~5/6 of the cell; the top sixth is always the
cell background, so wax drawn as glyphs on the dark liquid shows a dark
line along every row. With `display.cells = "background"` (or `"auto"`
with `TERM_PROGRAM=Apple_Terminal`, or in Ghostex, whose renderer leaves
the odd block glyph a hair short of the cell's side: dark ticks in moving
wax) the finished frame goes through
`render::fill_from_background`: every block glyph whose top row is
mostly ink (on a tie, whose cell is) becomes its complement in swapped
colours (`█` → a space on its colour, `▀` → `▄`, `▛` → `▗`, sextants
alike, `▓` → `░`). It looks the same on any opaque terminal; only cells
in the terminal's own colours (`Reset`) can't swap.

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
* **Wax at the top** (`lamp.top_wax`, off by default): a thin, slightly
  uneven layer of cool wax (the cool end of the wax colours) under the
  top edge, ≈ 0.01–0.022 lamp heights, drawn 1.5–5 sample pixels deep
  whatever the size, so it never grows into a slab. Rising blobs that
  touch it sometimes melt in (small ones whole, big ones give a share
  and sink); it grows hanging drips that let go and sink. Its wax comes
  from the pool and goes back (wax is conserved). Toggling fades it in
  or out over 1.5 s. It is part of the lamp, so widgets may sit over
  it like over any wax.

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
* **Starting in it** (saved, or `-m`) once the welcome card (§4.8) has
  been seen, the first frame toasts `lamp only · ? help · m shows more`:
  the one place this mode says where its keys are. On screen the user
  reads it as *lamp only*; `minimal` stays the config value and flag.
* Modes that own the keyboard still say so here: the music controls'
  guide line (§4.8) is not resting chrome.
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
  ● braille · lava       s style  c clock  p colours  m lamp only  Space timer  , settings  ? help
  └─ left ────────┘      └─ right: hints ──────────────────────────────────────┘
```

* **Left:** `●` in `accent`, then the style name in `text`, then
  `· palette` in `dim` (only if `cols ≥ 60`). When paused (`z`), `●`
  becomes `‖` and the text says `frozen`.
* **Centre:** empty, unless debug HUD (`d`) is on: `60 fps · 2.1 ms · 412k
  px` in `dim` (the whole readout turns `wax_hot` while adaptive quality
  is active or the frame takes > 80 % of its budget, §7).
* **Right:** hints in `dim`, each formatted `key label` with the key in
  `text`. The full list in display order is `s style  c clock  p colours
  m lamp only  Space timer  , settings  ? help`: familiar key names
  (`Space`, `Enter`, `Esc`, `Ctrl+C`), never `␣` / `⏎`. The bar fits as
  many as possible while keeping a gap of at least 4 cols to the left
  segment. Hints drop in this order: `m`, `Space`, `p`, `c`, `s`, `,`.
  `? help` always goes last. In a picker: `↑↓ preview  Enter save  Esc
  cancel`; in the music controls: `Space play  n p skip  ←→ seek  ↑↓
  volume  b playlists  Esc back`, `Esc back` kept longest.
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
* Plain words, no implementation terms: `beside the lamp` / `on the
  lamp` (not side panel / lava), `timer` (not pomodoro), `enlarge to see`
  (not no room), `cover quality · auto · sharp`.
* **The guide line** shares the row: while a mode owns the keys and must
  say so (§4.8), its line sits there whenever no toast does.

### 4.3 Help overlay (`?`)

The form depends on the terminal size (`ui/help/sheet.rs`):

* **≥ 68 × 20: a centred sheet**, `min(66, cols−4)` × `min(24, rows)`,
  with a **rounded border in `metal`**. Overlays are the only place
  borders appear. The title `keys` sits in the top border in `accent`,
  followed by `capital = hold Shift` in `dim` (the sheet writes `S`,
  `A`; the full-screen help spells out `Shift+S`), and `Esc close` in the
  bottom-right border in `dim`. The sheet always
  has two columns: *lamp*, *clock & timer* then *app* | *widgets*,
  *music · Shift+A · Esc back* (the music controls, §6.2) then *mouse*, with section
  headers in `dim`, keys in `accent` and labels in `text`. A key and its
  shifted picker share a row (`s S  style · choose`; also `[ ] - +  heat
  · speed`, `n r r  skip · reset timer`, `? ,  help · settings`), so the 22 rows
  inside fit every key at 80×24 (the sheet then takes the full height).
  Labels line up per column at its widest key + 2; the left column takes
  its natural width (at least half) and a 2-col gutter separates them.
  The rows come straight from the keymap table (§6), so help can't drift
  from dispatch.
* The *mouse* section (§6.3): drag warms the wax, click picks (double
  keeps), the wheel scrolls lists and help, and text selection is
  `⇧ drag`, or `⌥ drag` where the terminal says it's macOS Terminal or
  iTerm2 (`TERM_PROGRAM`).
* The lamp keeps animating behind it, dimmed to 35 % (truecolor: lerp
  toward `bg`; 256/16: the sheet's rect is cleared to `bg`, the rest
  isn't dimmed).
* **Smaller (not Micro): a full-screen sheet**, one column, scrollable
  with `j/k/↑/↓`, no border: `keys` (accent) top-left and `esc close`
  (dim) top-right on the first row, the body from the third row. The
  *app* section comes first (`m ? q` lead it), then lamp, clock, widgets,
  music, mouse; labels line up per section. Here it's **one action a
  line** with short labels (`s  next style`, `Shift+S  choose style`;
  `Row::narrow` in the keymap), and a label shows whole or not at all:
  a combined row cut to fit could name one action for two keys. Rows
  with no room are left out and the last line says `widen for all keys`.
  When keys are cut off, a dim scroll hint
  sits after `keys`: `↓ j/k more` (`↑` at the end, `↕` between),
  shortened to `↓ more` or `↓` to fit.
* **Micro:** the single line `? close · too small for keys` in the top row
  (only help's own keys act while it's open, so it names no others),
  clipped by dropping items from the end.
* `?`, `esc` or `q` closes it. While help is open, `q` closes help and
  does *not* quit.

80×24, captured from the app (`--color none`; the panel and status bar
stay hidden while the sheet would touch them, §8.2). Everything fits
without scrolling from 80×24 up:

```
       ╭ keys ─ capital = hold Shift ───────────────────────────────────╮
       │  lamp                          widgets                         │
       │  s S      style · choose       t       clock: side/lamp/off    │
       │  p P      colours · choose     f       timer: side/lamp/off    │
       │  [ ] - +  heat · speed         a       music: side/lamp/off    │
       │  z        freeze               A       music controls          │
       │  0        reset heat & speed   y       lyrics: side/lamp/off   │
       │  R        new wax pattern      o O     cover · quality         │
       │                                l L     move · select item      │
       │  clock & timer                                                 │
       │  c C      clock face · choose  music · Shift+A · Esc back      │
       │  T        12h / 24h            Space   play / pause            │
       │  Space    timer start / pause  n p     next · previous         │
       │  n r r    skip · reset timer   ←→ ↑↓   seek · volume           │
       │                                x r     shuffle · repeat        │
       │  app                           s a     like · add to playlist  │
       │  m        lamp only            b i     playlists · log in/out  │
       │  ? ,      help · settings                                      │
       │  q        quit · Ctrl+C        mouse                           │
       │  w        welcome tips         drag    warm the wax            │
       │  b        status bar           click   pick · double keeps     │
       │  d        performance info     wheel   scroll lists & help     │
       │  Ctrl+L   redraw               ⇧ drag  select text             │
       ╰───────────────────────────────────────────────────── Esc close ╯
```

### 4.4 Pickers (`S` style, `C` face, `P` palette)

* **Live preview:** moving the cursor applies the item to the live lamp
  or clock right away. `Enter` saves it, `Esc` cancels back to what was
  active when the picker opened. Every form says so itself (below), so
  minimal mode and small windows, which have no status bar, still teach
  it.
* **≥ 80 × 16: a sheet**, width 26, height `items + 6` (capped at
  `rows − 2`, scrolls), inset from the side by the side margin (§1.3) and
  vertically centred above the status bar. Rounded `metal` border, title
  (`style`, `clock`, `palette`) in `accent`, cursor `▸` + name in
  `accent`, the item that was active when it opened marked with a dim `·`,
  and an `Enter save  Esc cancel` row. The style and palette pickers anchor
  right. The face picker anchors left when that keeps it clear of the
  panel, so the face it previews stays in view. The lamp stays visible
  and *un*-dimmed, because the point is to watch it change. Chrome the
  sheet would touch (panel, chip) is hidden whole (§8.2).
* **Smaller (Small tier and short windows): a bottom sheet** just above
  the status bar, `items + 3` rows, at most half the height above the
  status bar (at least 3), with `Enter save · Esc cancel` (or `preview ·
  Enter save · Esc cancel`, or shorter, whichever fits) centred in its
  bottom border, so it costs no row. It spans only the lamp's
  columns when a panel sits to the right of the lamp (and the lamp is
  ≥ 16 cols), otherwise the full width.
* **Tiny / Micro:** an inline selector in the top row, `‹ braille ›`. Use
  `←/→` or `h/l` (also `j/k`). A name too long for the row is cut with
  `…`, the one place text is shortened rather than dropped: the
  selector must show *something* to be usable. Under it, while at least
  3 rows, the guidance in `dim`, the longest that fits:
  `preview · Enter save · Esc cancel`, `Enter save · Esc cancel`,
  `Enter ok · Esc cancel`, `Enter ok  Esc cancel`, `Enter · Esc`.
* **The face picker's preview card.** While the clock itself doesn't
  show the face being chosen (placed off, a chip, its text form, minimal
  mode, or hidden by the sheet), a small `preview` card (rounded `metal`
  border) draws it, live, in the largest part of the lamp the sheet
  leaves (clear of the toast and chip rows): the largest form of the face
  that fits, else `enlarge to preview`, else nothing. It never changes
  where the clock is placed: cancelling leaves placement and face as
  they were.
* Keys inside a picker: `↑↓`/`j k` move, `1`–`9` jump, `Enter`/`space`
  save, `Esc`/`q` cancel. Pressing the opening key again saves and closes.
* The status bar's right side switches to picker hints: `↑↓ preview
  Enter save  Esc cancel`.
* **The library sheets** (`b` playlists, `a` add to playlist, in the
  player keys; `ui/library.rs`) use the same placement, 44 wide: a sheet
  on the right, else a bottom sheet, else the inline selector. Not live:
  `⏎` chooses. Rows are `▸ name` with a dim right-hand detail (a
  playlist's count, a track's artist). Playlists the user neither owns
  nor collaborates on are dim: Spotify won't list their items to a
  development-mode app, so `⏎` plays them instead of opening them. In a
  playlist (title: its name) `⏎` plays the track in the playlist's
  context (through the Web API's player with Premium; else the desktop
  app: on macOS AppleScript's `play track … in context …`, elsewhere the
  track alone), `p` plays the playlist from the
  top, and pages of 50 load as the cursor nears the end. The add picker
  lists only owned or collaborative playlists; those that have the
  playing song already show a dim `✓` before the count (`✓ 60`).
  Choosing one of them asks first (lava-75z.24): the rows give way to
  `already in Lamplight Mix` (text) and `add it again?` (accent), a blank
  row above when there's room, hints `⏎ add again  esc cancel`; in a
  one-row bottom sheet or the inline selector, one line, the longest of
  `already in <name> · add it again?`, `in <name> · add again?`,
  `already there · add again?`, `add it again?` that fits (no `‹ ›`: it's
  a question, not a list), with `Enter add again · Esc cancel` on the
  bottom border or under it. `⏎` adds, `esc` goes back to the list, `q`
  closes. Still checking: `checking Lamplight Mix…` (dim; `150 of 400
  songs` under it on long playlists), `⏎ add anyway`. Empty lists say
  why in one dim line: `not logged in · ⏎ to log in`, `loading…`, Spotify's error.
  Keys: `j k ↑ ↓` move, `g G` / page up / down jump, `⏎` / `l` open or
  choose, `p` play all, `/` find, `esc` / `h` back (closes at the top),
  `q` close. Hints: `↑↓ move  ⏎ open  p play  / find  esc close` (`⏎
  play  p play all  / find  esc back` in a playlist, `⏎ add  / find  esc
  close`).
  *Find* (`/`, lava-75z.17): every key but `↑ ↓`, page up / down, home /
  end, `⏎`, backspace and `esc` types; rows whose name (a track's artists
  too) contain every typed word, any case, stay. The sheet keeps its size
  (sized for all the rows). What's typed shows as `/ chill▏` with a dim
  `3 of 77` in the roomy sheet's spare row above the list, on the bottom
  sheet's bottom border, and before the name in the inline selector.
  In a playlist the filter loads every page to look through. `⏎`
  chooses the highlighted match; backspace on nothing or `esc` closes
  the filter, `esc` keeping the cursor on the row it was on. A playlist
  opened from a filtered list comes back to it filtered. Hints while
  typing: `↑↓ move  ⏎ choose  esc clear`.

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
                 ⢸⣿⣿⣿⣿⣿⣿⣿⣷⣷⣝⢝⣝⠿⠝⠁                   │ Enter save  Esc cancel │
                 ⠈⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡇                       │                        │
                  ⢹⣿⣿⣿⣿⣿⣿⣿⣿⣿⠁                       ╰────────────────────────╯
                   ⠻⣿⣿⣿⣿⣿⣿⣿⣿⣀⣀⣀⣀⣀⣀⣀⣀
                ⣀⣀⣠⣾⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣷⣶⣤⣀⡀
 ⣀⣀⣀⣀⣀⣠⣤⣴⣶⣶⣾⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣶⣦⣤⣀⣀
⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣶⣶⣤⣤⣤
  ● braille · lava                              ↑↓ preview  Enter save  Esc cancel
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

The clock, the pomodoro, music, lyrics and the album cover are *widgets* (`src/dock/`). Each has a place, persisted as `dock.<name>`, cycled
by its key: `t` the clock, `f` the pomodoro (`focus`), `a` music
(`audio`), `y` lyrics, `o` the cover, each `side → overlay → off → side`. A toast says where it
went (`clock · on the lava`), adding `· no room` when the layout couldn't
fit it there (it's in the chip row meanwhile) and `· not in minimal` for
`side` in minimal mode.

* **side**: in the panel (§1.4, §4.5), in registry order (clock,
  pomodoro, music): a column 2 rows apart, or a strip / wrap / two
  columns where that suits them better (§1.4). The default for the
  clock and the pomodoro, so the default screen is the v1 panel, cell
  for cell; music is `off` by default.
* **overlay**: on the lava at the widget's own **anchor**
  (`dock.anchor.<name>`): centre, top, top right, bottom right, bottom,
  bottom left, top left. Widgets sharing an anchor stack there, 1 row
  apart, lined up by it (centred, or flush left / right at the sides);
  different anchors make separate stacks that spread across the lamp.
  Defaults: the clock and the pomodoro centre (so both on the lava stack
  as in v1.1), music top left, lyrics bottom centre, the cover top right. Limits,
  so the lamp stays the hero: the lamp ≥ 28 × 10; each stack ≤ 60 % of
  the lamp's width and ≤ half its height; their backings never touch
  each other and together cover ≤ 35 % of the lamp; inset (≥ 5 cols /
  3 rows, more on big lamps) so no backing reaches the toast row or the
  chip row. Forms shrink by rank first; when not even the smallest fit,
  the lowest-ranked widget goes to the chip row and the rest are tried
  again. Seconds are never shown on the lava; the date line comes along
  in tall terminals.
* **off**: not drawn (a running pomodoro still toasts and flashes).

**Moving them.** `l` moves one widget on the lava to its next anchor
(centre → top → top right → bottom right → bottom → bottom left → top
left): the one last put on the lava (with `t`/`f`/`a`) or picked with
`L`, else the first there. Toasts: `pomodoro · top right` (`· no room`
if it didn't fit there), `l moves clock · now centre`, `nothing on the
lava · t f a put widgets there`. Chosen over a dock picker sheet: two
keys, no new overlay, and every press says which widget moved and where.

**Ranks.** Each widget has a rank, recomputed every frame
(`DockWidget::rank`): the clock 1; the pomodoro 3 running, 2 paused,
0 idle; music 2 playing, 1 paused, 0 otherwise; lyrics and the cover the
same, but only with lines / a picture to show (a message is 0). Ties go to the earlier widget in the registry. The rank decides
everything about room: who shrinks first, who leaves for the chip row
first, which chips stay, and in the panel the score of each arrangement.

**The chip row.** Widgets with no room where they were put (side ones
with no panel, so always in minimal mode, or dropped from the panel or
the lava) show their one-line chips in a single row over the lamp's
bottom-right corner, on `bg` with a 1-cell pad, in registry order, a dim
` · ` between them: ` 14:32 · ▸ 18:24 · ▶ Deliver Me – Sarah Brightman `.
When they don't all fit the lamp's width the lowest-ranked go first; a
chip too wide for the lamp on its own is never shown. With only the
clock homeless it is the v1 corner chip, cell for cell.

**The backing** (`dock.backing`). By default there is none: the widgets
**float** on the lamp, their text part of it. Only the cells a glyph
takes change, and each keeps the lamp's colours: a letter over a half
block sits on what the cell showed (in truecolor both halves' mean), and
the faces' own half blocks are composited pixel by pixel, so wax runs
right up to every stroke. Spaces inside a widget leave the lamp showing,
glyph styles included, except a one-cell gap between two words of a
text line, whose lamp glyph is cleared (its colours stay) so `thu 1 oct`
never reads `thu#1#oct` over ascii, matrix, braille or halftone. Text is
bold; `dim` lines aren't. Nothing is drawn around the text: no veil, box
or halo.

*Adaptive contrast* (v1.5, lava-1xk.31). Legibility comes from the ink
alone, chosen per **glyph** against the colour actually displayed right
behind it: the cell's background for text (it replaces the lamp's
glyph), the lamp's pixels around a big-digit stroke. With see-through
cell backgrounds (Ghostty's `background-opacity` with
`background-opacity-cells`, read from its config; 0.75 assumed for
`display.cells = "translucent"`) a background is measured as it shows,
at that opacity over a dark desktop (captures: lava's liquid `#23160C`
shows as `#19130D`), while glyphs stay opaque; so mid wax behind text
counts darker there than on an opaque terminal. A glyph keeps its own
role ink (`text`, `accent`, `dim`) while that reads at least 4.5 : 1
(WCAG AA; 3 : 1 for big digits), or, for a quiet ink that reads less on
the palette's plain liquid (lava's `dim`, 3.6 : 1), 0.9 of that, never
below 3 : 1 (paper's 2.85 : 1 `dim` reads dark instead). Otherwise it
takes the better of the palette's light and dark inks (`text` and `bg`,
the lighter first; white and black where they are the terminal's
defaults): dark over bright wax, light over the liquid; the two cross at
≈ 3.8 : 1 on lava. Secondary lines stay unbolded either way, so they
still read as secondary. Nothing else decides a glyph's ink, so it
changes only when what's behind *it* changes, never a whole word or
line at once. Words stay coherent only where it costs nothing: a glyph
that reads about as well in light as in dark (within 1.15×, both
≥ 3.3 : 1) follows the glyph before it, and a big clock digit takes the
ink most of its cells chose wherever that still reads ≥ 3 : 1 (one that
straddles pale wax and dark liquid splits). Calm, per glyph (by cell and
character): the ink it had last frame counts 1.15× better against the
other of light / dark, its own ink comes back only at 1.08× its bar, and
a change shows once it's wanted two frames running (a backdrop line
sweeping under a glyph doesn't make it blink), at once if the ink it
has reads below 3 : 1. Each stack on the lava keeps its own memory. So
every glyph reads ≥ 3 : 1 in every frame, and text ≥ 4.5 : 1 unless
it's quiet or sits where light and dark cross. In 256 colours the same
rule runs on the indices' standard RGB; with no colour, or colours that
are the terminal's defaults (16 colours, the `ansi` palette), contrast
can't be measured and glyphs keep their own ink, bold. Album-art pixels
are drawn as they are. The repro: `cargo test --release -- --ignored
--nocapture contrast_trace` (seeded, 30 fps frames; `STYLE`,
`PALETTE`, `SEED`, `CELLS`, `CSV`).

Picked from pty captures of all nine styles and eight palettes at
80 × 24 and 160 × 40. Earlier rejected alternatives still hold: a halo
following the glyphs was busy around short lines, per-row spans ragged.

*Fixed ink* (`dock.text`, lava-1xk.41; settings › widgets › *text on
the lamp*). `auto` (the default) is the adaptive contrast above.
`light` and `dark` turn it off: every glyph on the lava, text and big
digits alike, takes the palette's light or dark ink (the same pair
adaptive contrast picks from), whatever is behind it, with no memory or
hysteresis. Dim lines stay unbolded, album art keeps its pixels, and the
soft backing's text takes the same ink. With no colour the glyphs keep
their own. The side panel is never affected: its widgets keep their role
inks on the app background.

`dock.backing = "soft"` keeps the v1.2 look: a soft pool of liquid. Under
the stack and half a row around it the lamp is veiled 82 % of the way to
`liquid` (its glyphs cleared), and the veil fades to nothing over the
next 1½ rows (a column counts half a row), within 4 cols / 2 rows of the
stack. In 256 colours the veil would snap to cube greys (a grey box), so
below truecolor it is plain `liquid` wherever it's at least half
strength. The layout reserves the soft backing's reach (`HALO`) either
way, so switching never moves a widget.

Chrome rules still hold: an overlay sheet (help, picker) touching the
stack's backing hides the whole stack (§8.2); the face picker sits on
whichever side keeps the panel and the stack clear when it can.

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
| cover beside | ≥ 32 × 6 | a 12 × 6 cover, 2 cols, the card (`art.inline`, cover widget off) |
| card | ≥ 20 × 6 | title (`text`), artist, album (`dim`), a blank row, bar, status line |
| compact | ≥ 20 × 3 | title, artist, `▶ 1:23 ━━━─── 3:45` |
| line | ≤ 36 × 1 | `▶ title – artist` |

(v1.4 moved the big cover-on-top forms to the cover widget, below; the
card keeps only the small cover beside it, and only while the cover
widget is off, so there's never two.)

```
 Voices (From "The Be…   ← title, cut with … to the card's width
 Dario G                 ← artist (dim)
 Sunmachine              ← album (dim)
 ◂◂  ‖  ▸▸      ♥  +  ≡  ← controls (dim; ♥ accent when liked)
 ━━━━━━━━━━━━──────────  ← elapsed in text (dim while paused), rest dim
 ▶ 3:28     vol 68  5:19 ← play state + elapsed · ⇄ ↻ vol (dim) · total (dim)
```

The status line drops shuffle/repeat, then the volume, then the total
before it would crowd the elapsed time. The volume shows only for players
that have one (`Capabilities::volume`; SMTC on Windows doesn't): there the
volume keys change nothing and toast `<player> has no volume control
here`. `▶` playing, `‖` paused; the
glyph turns `accent` while the player keys are on (the one sign of the
mode besides the status bar's hints). Shuffle `⇄` and repeat `↻` show
only for players that can change them (`MediaSource::capabilities`):
Spotify's AppleScript can't (its setters are no-ops, lava-75z.9), so for
it they're neither shown nor offered, unless the Web API is logged in
and Spotify lets it change them (Premium, a device playing; lava-75z.12):
then `x` / `r` go through `PUT /me/player/shuffle|repeat`, the state is
read from `GET /me/player` (on each track change and every 30 s), and a
refusal hides them again until the next login. When the Web API's
state is what's shown, `x` / `r` always go there, even if the player
offers its own. MPRIS players are taken at their word until a change
doesn't show within 1.5 s (Spotify on Linux accepts and ignores both,
lava-75z.21): then that player stops offering them and a toast says
`Spotify ignored that` and what works (logging in). On Windows (no
URI) the Web API's track counts only when title, artists and length or
album all agree, in a state read since the track began (lava-1xk.35). On the lava the forms
line up by the anchor (centred lines under a centred cover).

**Controls row** (the card's fourth row, the card forms only). Left:
previous `◂◂`, play / pause (`‖` while playing, `▶` paused: the action),
next `▸▸`. Right, with a library login: the heart (`♥` in `accent` when
liked, `♡` dim), add to playlist `+` (Spotify tracks) and the playlist
browser `≡`; logged out, a dim `log in` (`logging in…` while the browser
is open); without a Client ID nothing. All `dim`, two spaces apart, one
when that keeps them all; then the right-hand ones drop from the end and
the left group goes rather than crowd. Each is a mouse target and has a
player key (`␣ n p s a b i`). With `input.mouse = false` the row shows
only a liked `♥`. The compact form keeps the heart at the end of its
title row and its play glyph is the play / pause target; the one-line
form is key-only. Clicking the progress bar (card and compact) seeks to
that point. Hit-testing (`dock::music::hit`) uses the same `parts` /
controls geometry the widget draws with.

**The library** (`app/model/library.rs`, all network on the
`SpotifyWeb` worker, events drained once a frame): `i` logs in (the
browser opens on Spotify's consent page; `i` again cancels), and logged
in, `i` twice within 2 s logs out. Toasts say how it went (`logged in to
Spotify`, `Spotify login expired · A i to log in again`). `s` likes /
unlikes at once (the heart flips, a refusal flips it back). `b` opens
the playlist browser and `a` the add-to-playlist picker (§4.4).

**Covers.** Fetched on a background thread when the track changes
(`https` only, ≤ 8 MB), kept on disk in `$XDG_CACHE_HOME/lavatui/art`
(else the platform cache dir; 256 newest kept), decoded (JPEG / PNG),
cropped square and shrunk to 128 px (in pixels mode also kept as a
≤ 400 px PNG, base64, ready to send), all on that thread. The card's
small cover is drawn the cover widget's way (below): text cells through
`Theme::image` (exact in truecolor, the nearest xterm index in 256
colours, none in 16 colours or `NO_COLOR`, where the cover form isn't
offered), or kitty pixels. Until the cover has arrived (or if it can't be had) a quiet placeholder holds its place
(`bg` tinted 18 % toward `dim`, a dim `♪` in the middle), so nothing jumps
when it arrives. On the lava the cover is opaque, drawn as it is (with
`dock.backing = "soft"` the soft backing frames it like the text).

**Without a player** the widget is one calm, dim sentence that says
what to do, wrapped at 20 cols beside the lamp and 30 on the lava: `♪
Open Spotify to show music`, `♪ Spotify is not installed`, the Automation permission
path (`♪ Allow control of Spotify: System Settings › Privacy & Security ›
Automation › your terminal › Spotify`), `♪ No media player support on
this platform yet`, `♪ nothing playing` (running, nothing loaded), `♪ …`
for the moment before the first answer. A problem the user can fix ranks
2 (its chip outlasts the clock's) and has a chip for when the widget
has no room: `♪ open Spotify` when it isn't running, else `♪ see
Shift+A`; in the music controls the music note card (§4.8) then shows
the whole sentence. The cover and lyrics widgets say the same, for
what they show (`♪ Open Spotify to show album art`), never a bare
`nothing playing` while the player has a problem.

**Chip:** `▶ title – artist` (≤ 32 cols, cut with `…`), rank 2 while
playing (above the clock; ties go to the pomodoro, earlier in the
registry), `‖ …` rank 1 while paused, none otherwise.

**Frozen lamp:** while music is placed, the idle loop looks at the player
at least once a second, so a track change or a pause made in Spotify
shows within a second.

#### Cover

`src/dock/cover.rs` (the widget, `[art]`), `src/dock/picture.rs` (text
cells), `src/graphics.rs` (kitty). Key `o`: `off → side → overlay → off`;
off by default; top right on the lava. It reads the music widget's player
and art loader (the source is held while music, lyrics or the cover is
placed), so it doesn't need music placed.

**Size** (`art.size`): the widest it may be, in columns: `small` 16,
`medium` 24 (default), `large` 34, `fill` 64 (as big as the room allows).
Forms are fixed squares, widest first, each narrower one offered after it
(64, 56, 48, 40, 34, 28, 24, 20, 16, 12, 10 up to the size's): rows =
cols ÷ the cell aspect (cols / 2 at 2 : 1), so it's square on screen. The
layout keeps the first that fits (the lava's limits apply: ≤ 60 % × 50 %
of the lamp); below 10 × 5 it's left out (no chip: the music chip names
the track). Its rank is music's while it has a picture, so it shrinks
before music (later in the registry) and a message never pushes anything
out.

**Detail** (`art.detail`, `O` cycles it). On screen (toasts, settings)
it's the **cover quality**, named by how fine the picture is: `auto`,
`sharp`, `small pixels`, `medium pixels`, `big pixels` (in the config
`small-pixels`, `medium-pixels`, `big-pixels`). Each is its own look in every terminal
(lava-bq0: the old `photo` / `fine` / `medium` / `coarse` were the same
photo in a terminal with pictures, and the same one colour a cell where
cell backgrounds are see-through). The toast says what it comes to
here, e.g. `cover quality · auto · sharp`:

| detail | with pictures | in text cells (256 colours+) |
|---|---|---|
| `sharp` | the real picture, at the terminal's resolution | the finest text: sextants (2 × 3 pixels a cell, two colours each, U+1FB00..1FB3B) where the terminal draws them, else quadrants (2 × 2, `▘▝▀▖▌▞▛▗▚▐▜▄▙▟█`) |
| `small-pixels` | pixel art: 32 × 32 flat squares | flat square blocks, about 32 across: k columns × k half rows (`▀`, exact colours), k whole |
| `medium-pixels` | pixel art: 16 × 16 flat squares | the same, about 16 across, always bigger blocks than `small-pixels` |
| `big-pixels` | pixel art: 10 × 10 flat squares | the same, about 10 across, always bigger blocks than `medium-pixels` |
| `auto` (default) | `sharp` | `sharp` |

Pictures need a pixel protocol: kitty graphics with Unicode placeholders
(kitty, Ghostty), iTerm2 inline images (iTerm2, WezTerm, mintty, Rio) or
sixel (foot, mlterm, Konsole ≥ 22.04, Contour); any colour depth but
none. Pixel art is made with the sharp copy, on the art worker
(`Art::pixel_art`, each block a box-filtered mean, drawn ~400 px square
so the terminal's scaling keeps edges crisp; sixel scales it nearest).
In text cells a block is `round(cols / n)` columns wide (so blocks are
all one size), bumped where needed so each size's blocks are bigger than
the one before's on small covers; where cell backgrounds are see-through
(`translucent`) the side is even: whole cells, one colour each. A block
is at least two columns: one column is as fine as text gets and would
look like `sharp`. (At a 24-column cover: 12, 8 and 6 blocks across;
see-through 12, 6 and 4; at 64 columns 32, 16 and 10.)
Older names load as the nearest look: `pixels` / `photo` / `sextant` /
`fine` → `sharp`, `quadrant` / `medium` / `pixelated` → `medium-pixels`,
`halfblock` / `coarse` / `chunky` → `big-pixels` (`pixels` stays the real
picture it always meant, so it's never a pixel-art name).

Sextants (`sharp` in text) are used only in terminals
known to draw them, and **never through a multiplexer** (tmux, screen,
zellij, zmx) or with any `GHOSTEX_*` variable set, whatever
`TERM_PROGRAM` was inherited: Ghostex's built-in terminal draws them as
`?`. There `sharp` draws quadrants. A test writes real frames through the
crossterm backend under a Ghostex environment and finds no U+1FB00–1FB3B
and no U+10EEEE.

Quadrants and sextants try every split of the cell's pixels into two
groups (8 / 32) and keep the one whose two means lose least: one the
glyph's ink, the other its background. The cells are worked out once per
track, size, detail and depth and kept (pictures: once per track, size
and detail sent). Without a way to show a picture
(16 colours without pixels, `NO_COLOR`) the widget is one calm line
(wrapped like music's), `♪ covers need 256 colours`; with no track,
`♪ nothing playing`; with no cover, `♪ no cover`.

**Pixels** (kitty graphics protocol). Detected from the environment
(`TERM` `xterm-kitty` / `xterm-ghostty`, `TERM_PROGRAM` `ghostty` /
`kitty`, `KITTY_WINDOW_ID`, `GHOSTTY_RESOURCES_DIR`; never inside tmux or
screen, and not WezTerm or Konsole, which lack Unicode placeholders), so
there's no blocking terminal query. Where none was found (or confirmed)
every detail is drawn in text cells, never a guess.

**Verified before use** (lava-1xk.18). The environment can lie:
Ghostex's built-in terminal sets `TERM_PROGRAM=ghostty` but runs sessions
through its zmx multiplexer and has no kitty graphics, so placeholders
showed as `?` boxes. So zmx (`ZMX_SESSION`, `GHOSTEX_SESSION_ID`) and
zellij count as multiplexers like tmux and screen, and whatever the
environment promises is then checked with the terminal itself
(`graphics/probe.rs`; not on Windows, whose console input doesn't pass
replies on, and not when `LAVATUI_GRAPHICS` names it). At start the app
writes one query and never waits for it: kitty gets a graphics query
(`a=q`, a 1×1 image never stored); iTerm2 / sixel get XTVERSION (crossterm
swallows DA1, whose `4` would mean sixel). Each is followed by an OSC 10
fence, which nearly every terminal answers, in order. Replies arrive as
input; `app::replies` takes them out of the key stream and hands the
strings over. Kitty: `OK` → pixels; an error, the fence first, or nothing
within 1.5 s → no. iTerm2 / sixel: a name not known to speak the
protocol → no; no name → the environment is believed. Until then the
cover is drawn in text cells; a no, while a cover is shown in `auto` /
`sharp`, toasts `no photos in this terminal · covers drawn in text`. The cover is sent as a PNG (`a=T,U=1,f=100,q=2`) with
a *virtual* placement of exactly the cover's cells (`c`, `r`), in 4096-byte
base64 chunks, at most 96 KB a frame, after the frame's cells and inside
its synchronized update; meanwhile the best text cells show. From the
next frame the cover's cells are Unicode placeholders (U+10EEEE + a row
and a column diacritic) in a foreground colour that is the image id. To
ratatui they are ordinary cells, so the lamp's 60 fps diff never touches
them: nothing is re-sent, nothing flickers, and moving the cover (`l`,
`o`) or a resize at the same size is just drawing those cells again. A new
track or size is sent under the other of two ids (from the process id),
the cells switch, and the old image is deleted (`a=d,d=I`) the frame
after; turning the cover off deletes it, and every way out (exit, error,
panic) deletes both. Help leaves placeholder cells unfaded (fading their
colour would change the id).

**Pixels** (iTerm2 inline images, sixel; lava-75z.19). Detected from the
environment too, after kitty: iTerm2's protocol for `TERM_PROGRAM`
`iTerm.app` / `WezTerm` / `mintty` / `rio` or `LC_TERMINAL=iTerm2`; sixel
for `TERM` `foot*` / `mlterm*`, `MLTERM`, `KONSOLE_VERSION` ≥ 220400,
`TERMINAL_NAME=contour`; never inside tmux or screen.
`LAVATUI_GRAPHICS=kitty|iterm|sixel|none` overrides it (xterm with sixel
can't be told apart otherwise). No DA1 query. These pictures are painted
over cells at the cursor, and text written into those cells paints over
them, so: the cover draws sentinel cells where it goes; after the whole
frame is drawn the app checks they all survived (no overlay over them),
and if so the frame it's placed writes them as blanks in the cover's
mean colour, followed (same synchronized update, cursor saved / moved /
restored) by the picture; every later frame they are
`CellDiffOption::Skip`, so the lamp's redraws never touch it. When it
moves, goes, or an overlay takes its spot, its old cells are
`CellDiffOption::AlwaysUpdate`: whatever is there now is written over it.
A resize or ctrl-l (screen cleared) places it again; leaving the
alternate screen removes it on exit. Never placed on the last row (a
picture reaching the bottom could scroll the screen). iTerm2 gets the
≤ 400 px PNG as is (`width`/`height` in cells, `preserveAspectRatio=1`,
`doNotMoveCursor=1`), ~380 KB in the placing frame. Sixel is drawn at its
own pixel size, so it needs the cell size from the terminal's reported
window pixels (else text cells): decoded, scaled to fit, centred on the
mean colour, height rounded down to whole 6-pixel bands, median-cut to
256 colours and encoded on a worker thread (text cells meanwhile), ~100
KB for a 24-column cover. Byte-checked in a pty by `tools/inline_check.py`
(no such terminal was at hand to look at them).

**Mouse:** a click on the cover is play / pause (chosen over opening the
playlist browser: one obvious action, works without a Spotify login).

#### Lyrics

`src/dock/lyrics.rs`, state in `src/app/model/lyrics.rs`, lookups in
`src/lyrics/` (LRCLIB client, LRC parser, sync, disk cache, worker).
Key `y`: `off → side → overlay → off`. **Off by default, and placing it
is the opt-in**: while it's placed, each new track's title, artist,
album and length go to [lrclib.net](https://lrclib.net) (free, no key,
`User-Agent: lavatui/<version>`); the toast says so (`lyrics · on the
lava · via lrclib.net`), as do the help (`y  lyrics · lrclib.net`) and
the README. It reads the same player snapshot as music (the source is
held while either is placed) and doesn't need music placed.

**Lookups** never touch the frame: a track change sends a request to
the lyrics thread, which answers from the disk cache
(`$XDG_CACHE_HOME/lavatui/lyrics`, else the platform cache dir; synced
and instrumental answers kept 180 days, plain 7, "not found" 1 day),
else asks `/api/get` (exact title/artist/album, duration ± 2 s) and then
`/api/search` (closest version within 3 s, synced first). Network errors,
`429` and `5xx` are retried after 1 s and 4 s (a newer track cancels
them) and never cached; offline with a stale entry, the stale entry is
shown. Each frame polls for the answer (`try_recv`).

**Sync.** The position is the snapshot's, extrapolated to the frame
(`position + (now − sampled_at)` while playing). The media worker pins
it down over polls (`media/worker.rs`, `Baseline`): a reading was taken
somewhere between sending the request and getting the reply, so it bounds
when playback was where it said, and readings of the same playback
intersect down to the quickest round trip. Spotify's reported position is
exact (thousands of reads fit one line to ±4 ms), so the extrapolation
stays within a few ms of it (measured: `live_timing_audit`, docs/
architecture.md); before, the first reading of a song set it for the
whole song, 60–100 ms off. While synced lyrics are on screen the player
is polled every 250 ms instead of every second (`follow_closely`), so a
pause, resume or seek made in the player shows within about ¼ s. Lines
light up 150 ms early (the eye reads a line ahead of the voice), words
50 ms early (with the voice: a hair early reads as on time). A new
reading a little behind (< 400 ms) holds the highlight still until
playback catches up, so it never steps back a word or a line; a pause
shows where it stopped; a jump of more than 1.5 s from where the
position should be is a seek, followed at once without a fade.

**Words** (`lyrics/words.rs`). Each line's words are timed once, when
the lyrics arrive. Exact when the LRC has enhanced word tags
(`<mm:ss.xx>` before each word, an end tag after the last), which is
rare: none of 295 synced LRCLIB versions of 19 popular songs had them.
Otherwise estimated, and it is an estimate: words get time by their
syllables (vowel groups, a little more for long words; one per
character in Chinese, Japanese and Korean, where each character is a
word here), punctuation holds a word (a comma 0.5, a full stop 0.8
syllables), and the line is sung over the time to the next line less a
breath (12 %, at most 0.6 s), but no slower than 1.5× the song's own
pace (its median seconds per syllable), so a line before a long break
isn't drawn out across it. A line is never still being sung when the
next one starts. Partly tagged lines keep their tags and estimate in
between.

**Never cut off** (lava-uqi). The line being sung always shows
whole, wherever the widget is and however squeezed the screen: lines
wrap between words, never mid-word, and a form only exists if it holds
the song's longest line. Before, the side forms were the panel's width ×
5 / 3 / 1 rows whatever the song: in a crowded panel the 1-row form cut
the current line with `…`, a 3-row one cut a line needing three rows
(`coming down to try ag…`), neighbours were cut mid-word (`thought
beh…`), and at 200+ cols the two-column panel gave lyrics a 20-col
column of `…`s. The rules:

1. **Forms are sized by the song**, once, when its lyrics arrive, so
   nothing jumps from line to line. Each keeps **R** rows for the
   current line: the most rows any of the song's lines wraps onto at
   the form's width.
2. **Neighbours** (the dim lines around it) show whole when they fit in
   the rows left, else on one row cut after a word with `…` (a trailing
   `,;:` goes too: `Floating like a thought…`). Only a single word wider
   than the form is ever cut inside.
3. **Beside the lamp** the forms fill the panel's width, each offered at
   the narrowest width (≥ 20) that holds every line in one, two and
   three rows, and at 20 in as many rows as that takes. A panel wider
   than the form's width only spares rows, which the lines after the
   current one use. From 200 cols up the panel grows for the one-row
   width (up to 56, like the clock's widest faces), so on a big screen
   every line fits on one row; in the two-column panel the lyrics'
   column is at least its form's width.
4. **Room is traded by rank, as for every widget** (§4.6 *Ranks*): the
   lyrics (2 while playing) keep their size over the clock (1); against
   music and the cover (also 2) registry order decides, so the cover
   shrinks first, then lyrics (five → three → the line alone), then
   music.
5. **No cramped form:** when even the line alone doesn't fit, the
   widget leaves for the chip row (`♪ current line`, cut after a word)
   rather than showing a clipped line. It doesn't move itself onto the
   lava: where things go stays the user's choice (`y`).

Forms, most preferred first (W = the song's widest line, clamped to
20..=56; on the lava the stack limits of 60 % of the lamp's width pick
the narrower ones on small lamps):

| form | on the lava | beside the lamp | shows |
|---|---|---|---|
| five | W × (4 + R) | fill × (4 + R) | two lines back, the current line, two ahead |
| three | W, 36, 24 × (2 + R) | fill × (2 + R) | one back, the current line, one ahead |
| line | W, 36, 24 × R | fill × R | the current line alone |

```
        Cooling at the top it drifts          ← two back (dim)
                 And falls                    ← one back (dim)
 Every blob that ever broke away comes home   ← current, whole, on the R
       again to the warm pool below              (here 2) rows kept: sung
                                                 words bold `text`, the one
                                                 being sung `accent`, the
                                                 rest `dim`
         Round and round it turns             ← one ahead (dim)
                 Slow rise                    ← two ahead
```

The current line starts on the row under the ones kept for the lines
before it; when it takes fewer than R rows, the lines after it move up.

**Look: no backing needed.** Role colours only, so it reads with or
without the soft backing (none by default: the floating text's ink adapts
to the wax, §4.6 "The backing"), lined up by the anchor (centred at the
bottom). The current line is karaoke: the words sung so far bold `text`,
the word being sung bold `accent`, the words still to come `dim` (not
bold); once the line is sung, all of it bold `text`. The other lines are
`dim`. With no colour (`NO_COLOR`) the word being sung is underlined
too, so sung (bold), being sung (bold, underlined) and to come (plain)
stay apart; in 16 colours `accent` is its own colour. On the lava a
glyph that has to take the palette's light or dark ink over bright wax
loses the accent there, but keeps its weight: the bold edge still shows
how far the line has got. The highlight moves word by word, never back,
with no fade (a word lasts a few hundred ms). **Transitions**: a new
line comes in with its words `dim` and lights word by word, while the
line it replaced dims back from bright over 320 ms (truecolor and 256
blend; 16 colours switch at the half-way point); the rows step, they
don't scroll (a terminal can't move text by less than a row). A **gap**
(an empty LRC line, or the intro before the first line) is three dots
`•  •  •` that light up one by one as it passes.

**States**, each one calm dim sentence like music's: `♪ looking for
lyrics…`, `♪ no lyrics on lrclib.net for this song` (where that doesn't
fit, the shorter `♪ not on lrclib.net`), `♪ instrumental` (LRCLIB's
`instrumental` flag, or lyrics that are only an "Instrumental" note), `♪
lyrics offline`, and the player's own (`♪ Open Spotify to show lyrics`, `♪ nothing
playing`, `♪ …`). **Plain lyrics** (LRCLIB has no timing) scroll with the
track's progress, the middle line `text`, the rest `dim`, never bold (it
isn't a claim about what's being sung).

**Chip:** `♪ current line` (≤ 32 cols, cut after a word) while playing synced lyrics, `♪`
in a gap; none otherwise. **Frozen lamp:** while playing, the idle loop
also wakes at the next word, line end, line start or gap dot (and during
a fade); paused, it doesn't.

---

### 4.7 Settings screen (`,`)

Every everyday setting, in plain words, so nobody needs `config.toml`
(lava-1xk.17). `app/model/settings_screen.rs` holds what's on it and what
keys do; `ui/settings.rs` places and draws it.

* **Pages:** *look* (style, colours, heat, speed, background, colour
  range, stripe fix), *clock & timer* (face, time format,
  focus / break lengths, long break after, sound at the end), *widgets*
  (each widget beside the lamp / on the lamp / off, its position while on
  the lamp, what things on the lamp sit on), *music & lyrics* (Spotify,
  lyrics with what lrclib.net is sent, cover picture / size, small cover
  with music), *controls* (mouse), *window* (lamp only, hint line,
  smoothness, lamp-only clock). Each ends with *reset this page*, which
  asks for a second `⏎` (the Spotify Client ID is never reset). Labels
  and values are lowercase words, never config keys: `beside the lamp`,
  not `side`; `position`, not `anchor`; `photo`, not `pixels`.
* **Live, saved:** a change applies at once (the lamp, clock, widgets
  and mouse capture show it) and goes through the usual debounced save;
  the sheet says `changes save automatically` (or that a save failed).
* **Explained:** the row under the cursor (or the picked page) is
  explained in one or two sentences under the list; sentences that don't
  fit are dropped whole. A value with no room beside its label moves
  there too, as `‹ value ›`.
* **≥ 66 × 18: a centred sheet**, up to 64 × 19, rounded `metal` border,
  title `settings`, `changes save automatically` bottom-left and `esc
  close` / `esc back` bottom-right. The page list (17 wide) sits left,
  the page's title (dim) and rows right; the cursor's row is `accent`
  with `▸` and a choice shows `‹ value ›`. Not dimmed: the lamp behind it
  is the preview. Chrome it touches hides whole (§8.2).
* **Smaller: full screen**, one list at a time: the pages, or one page
  (`settings · look` top-left, `esc back` top-right), the explanation at
  the bottom. **Micro:** `settings · window too small · esc close`.
* **Keys:** `↑↓` `j k` move; on the pages `⏎` `→` `l` open one; in a page
  `← →` `h l` change the value, `⏎` `space` steps a choice on or presses a
  button, `tab` / `shift-tab` the next / previous page; `esc` (or
  backspace) goes back a level and closes at the top; `q` and `,` close.
  Mouse: a click picks a page or a row, a click on the picked row is
  `⏎`; the wheel moves. Status bar hints: `↑↓ move  ⏎ open  esc close`,
  in a page `↑↓ move  ←→ change  ⏎ choose  esc back`.
* **Spotify setup** (*music & lyrics* → *spotify*, and what a library
  key opens while there's no Client ID, or while Spotify refuses the
  logged-in account): `status` (`refused`, with what to fix, when
  Spotify refused the account), `before you start` (`Premium needed`:
  the app owner's Premium and the 5-person allowlist, read before step
  1), then four numbered steps: `1 make a spotify app` opens developer.spotify.com/dashboard in
  the browser (off the input path), `2 add this address` copies the
  redirect URI with OSC 52 (the explanation shows it too, to type),
  `3 paste the client id` is a text field (`⏎` to type, or just paste:
  bracketed paste; `⏎` saves, `esc` stops; checked: 32 hex digits, the
  problem said in words), `4 connect` logs in in the browser, waits (a
  `copy the login link` row appears), says `Connected as …` or why it
  failed (what to check), and `⏎` twice disconnects. `spotify app` says
  whether the desktop app is playing, not running or needs the
  Automation permission, with what to do. While the setup is open the
  player and the Web API client stay connected for it, even with music
  off.

### 4.8 Cards and the guide line (`ui/cards.rs`)

Guidance that comes with a moment, never resting chrome. Cards are
small, bordered like overlays (rounded `metal`, title in `accent`, a
dim note bottom-right), centred in the lamp clear of its top (toast)
and bottom (chip) rows; chrome they'd touch is left out whole (§8.2).

* **Welcome card** (first start, `ui.welcome = true`, the default). Full
  form 42 × 11: `Welcome to LavaTUI`; `s change the look`, `p change the
  colours`, `Space start a 25-minute focus timer` (the configured
  length), `? all keys and help`, `q quit`; then dim `Capital letters
  mean hold Shift.` and `Your choices save automatically.`; bottom `any
  key to start`. Small form 28 × 7 (`? help  q quit`, `s look  p
  colours`, `Space focus timer`, `Shift = capital letters`, `choices save
  themselves`). The first key (or click) dismisses it **and still does
  what it does** (`s` changes the style), `Esc` only dismisses. Dismissal
  is its own saved change, `ui.welcome = false`; `w` shows it again.
* **Too small for either** (e.g. 20 × 8): the guide line says `? help ·
  q quit · enlarge for tips` (or `? help · q quit`, `? help`) and the
  card waits: keys don't dismiss what wasn't shown, only `Esc`, `?` and
  quitting do. The card appears once the window has room.
* **Music note**: in the music controls, when the player has a problem
  and the music widget has no room to say it, the whole sentence (the
  macOS Automation path wrapped) in a card titled `music`, bottom `Esc
  back`, up to 40 wide.
* **Face preview**: §4.4.
* **The guide line**: one line in the toast row (else the lamp's top
  row), `text` on a 1-cell `bg` pad, whenever no toast is up: while the
  music controls own the keyboard `music controls · Esc back` (`music ·
  Esc back`, `Esc back` as it narrows), at every size, minimal included;
  or the waiting welcome's `? help · q quit`. The corner HUD gives way to
  it as to a toast.

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
| `,` | settings screen (§4.7) | also from help |
| `q` | quit | closes the overlay instead when one is open |
| `ctrl-c` | quit | always, from anywhere |
| `esc` | close overlay / cancel picker | no-op otherwise: **esc never quits** (esc is muscle memory for "close this"; an accidental quit loses pomodoro state) |
| `m` | minimal mode on/off | also `--minimal` / `-m` |
| `b` | status bar on/off | full mode only (minimal toasts `no status bar in lamp-only mode · m to leave`) |
| `s` / `S` | next style / style picker | toast shows `name  i/n` |
| `c` / `C` | next clock face / face picker | |
| `p` / `P` | next palette / palette picker | |
| `t` | clock: side → on the lava → off | §4.6; toast `clock · on the lamp` (`· enlarge to see` with no room there) |
| `f` | pomodoro: side → on the lava → off | §4.6 |
| `a` | music: side → on the lava → off | §4.6; off by default |
| `y` | lyrics: side → on the lava → off | §4.6; off by default (the opt-in to lrclib.net lookups) |
| `A` | music controls on (§6.2) | toast `music controls · Esc back`, then the guide line (§4.8) says it while they're on; with music off: `music is off · a to show it` |
| `l` | move a widget on the lava | the last put there (or picked with `L`): centre → top → top right → … → top left; toast `clock · top right` |
| `L` | pick the widget `l` moves | cycles through those on the lava; toast `l moves the music · now top left` |
| `T` | 12h / 24h | |
| `space` | pomodoro start / pause / resume | starts a focus phase if idle |
| `n` | pomodoro: skip to next phase | idle: toasts `timer not running · Space starts it` |
| `r` | pomodoro reset (press **twice** within 2 s) | first press toasts `press r again to reset the timer`; presses < 150 ms apart count as key repeat, never as the second press |
| `[` / `]` | heat − / + (5 steps, default middle) | more heat = more, faster blobs; toast shows `heat ▮▮▮▯▯` |
| `-` / `+` (`=`) | sim speed ×0.25 · ×0.5 · ×1 · ×2 · ×4 | toast `speed ×2` |
| `0` | reset heat and speed | |
| `z` | freeze / unfreeze the lamp | frozen = zero sim cost; the clock keeps ticking |
| `R` | a new wax pattern (new random seed) | toast `new wax pattern`; blobs melt into the pool (under 2 s), then 5 s of fast budding refill the lamp. Never a hard cut |
| `d` | debug HUD (fps, frame ms, samples) | |
| `ctrl-l` | force full redraw | |
| `w` | the welcome card again (§4.8) | |
| `esc` (nothing open) | dismisses the welcome card | otherwise nothing |

### 6.2 In overlays

| Context | Keys |
|---|---|
| help | `j k ↑ ↓` scroll · `?` `esc` `q` close |
| settings | `j k ↑ ↓` move · `← →` `h l` change · `⏎` `space` open / choose · `tab` next page · `esc` back · `q` `,` close; typing: `⏎` save, `esc` stop |
| picker | `j k ↑ ↓` move (live preview) · `1`–`9` jump · `⏎` `space` keep · `esc` `q` revert · opening key = keep + close |
| tiny inline picker | `h l ← →` (also `j k`) move · `⏎` keep · `esc` revert |
| music controls (`Shift+A`) | `Space` play / pause · `n` `p` next / previous · `← →` (`h l`) seek ∓ 10 s · `↑ ↓` (`k j`, `+ -`) volume ± 5 · `x` `r` shuffle / repeat (where the player can) · `s` like · `a` add to playlist · `b` playlists · `i` log in / out · `Esc` `q` `A` back to the lamp's keys (`q` never quits here; `Ctrl+C` does) · `?` help (ends them) |
| library sheets | `j k ↑ ↓` move · `g G` page up / down jump · `⏎` `l` open / play / add · `p` play the playlist · `esc` `h` back · `q` close |

**The player keys** are a mode, like an overlay without a sheet: `A`
turns them on and they take the keyboard until `esc`, `q` or `A`. That
keeps one global key for the whole player (instead of nine more single
keys in an already full map) and lets it reuse the obvious letters
(`Space`, `n`, `p`, arrows) that the pomodoro and lamp own outside it. On
screen they're the *music controls*, and the guide line says `music
controls · Esc back` at every size while they're on (§4.8). The
status bar's hints become `Space play  n p skip  ←→ seek  ↑↓ volume  Esc
back` and the widget's play glyph turns `accent`. Volume toasts its new
value (`volume 65`); with no player, any key toasts why (`Open Spotify
to show music`). Leaving music `off` ends the mode.

Every key not listed is ignored (no beep, no toast). Overlay keys take
precedence over global keys; global keys other than `ctrl-c` don't fire
while an overlay (or the player keys) is open.

### 6.3 Mouse

`input.mouse = true` by default (since v1.3; a file that sets it keeps
its value). Mouse capture takes the terminal's own text selection, which
then needs shift held while dragging (option in macOS Terminal and
iTerm2); help and the README say so. What it does: click the music
widget's controls and progress bar (§4.6), click/drag on the lamp = a
local heat pulse (the wax there warms and rises), scroll in pickers,
help and the library sheets, click an item to preview / pick,
double-click to keep / open. Every click has a key.

---

## 7. Motion & performance

| Target | Value |
|---|---|
| Sim timestep | fixed `SIM_HZ` (**120 Hz** in the scaffold, `timing::FixedStep`; sim time = real time × speed at any fps, only a > 1.5 s stall is cut short), decoupled from render; render interpolates with `alpha()` |
| Render rate | default **60 fps** (`--fps 1..=240`, `display.fps`). Lava is slow, but 60 fps keeps input feeling instant and makes the slow motion buttery |
| Wax tempo (×1, heat 3) | a blob takes **~20–40 s** to cross the lamp: slow, hypnotic, never jittery |
| Startup → first frame | **< 100 ms**. The sim starts *pre-warmed*: ~600 headless steps at launch, so frame 1 already looks alive (no 2-hour warm-up) |
| Input latency | key → visible change **≤ 1 frame** (≤ 17 ms at 60 fps). The loop blocks on `event::poll(time_to_next_frame)`; any input that changes UI state triggers an immediate redraw, without waiting for the tick, unless a frame started under one period ago: then it draws when that period is up (input never draws above the target fps) |
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
   Font glyphs required. (The one exception: a `sharp` cover in text
   cells uses sextants, Unicode 13, only in terminals that draw them
   themselves.)
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
cells = "auto"           # auto | opaque | translucent (see §2.2)

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
welcome = true           # the welcome card at start (§4.8); dismissing it saves false

[minimal]
clock = "corner"         # corner | off

[input]
mouse = true

[dock]
clock = "side"           # side | overlay | off
pomodoro = "side"        # one key per widget in the registry
music = "off"
lyrics = "off"
cover = "off"
# where each sits on the lava: center | top | top-right | bottom-right | bottom | bottom-left | top-left
anchor = { clock = "center", pomodoro = "center", music = "top-left", lyrics = "bottom", cover = "top-right" }

[art]
detail = "auto"          # auto | sharp | small-pixels | medium-pixels | big-pixels (§4.6 Cover; older names load)
size = "medium"          # small | medium | large | fill
inline = true            # the music card's small cover, while the cover widget is off

[spotify]
client_id = ""           # Web API library features (docs/spotify.md); "" = off
```

Out-of-range values are clamped rather than rejected: `fps` 1–240,
`cell_aspect` 1.6–2.6 (NaN → 2.0), `heat` 1–5, `speed` snapped to the
nearest step (≤ 0 or non-finite → 1), pomodoro minutes 1–1440, `cycles`
1–12, `spotify.client_id` trimmed (anything but letters and digits → `""`).
An empty `client_id` falls back to `LAVATUI_SPOTIFY_CLIENT_ID`. The old style name `glass` is accepted as `chrome`.

A single `dock.anchor = "top-left"` (before per-widget anchors) loads as
that anchor for every widget, and the next save writes it back per
widget, inline, keeping its comment.

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
